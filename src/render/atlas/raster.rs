//! Glyph rasterization into atlas slots: primary font + fallbacks, synthetic
//! bold / italic, shaped ligature pieces, clusters, wide and color glyphs, and
//! the texture uploads. Sprites, CoreText / FreeType and color emoji are called
//! from here.

use std::sync::atomic::Ordering;

use super::sprites::{draw_builtin_sprite, draw_nerd_icon_fallback};
use super::system::{macos_coretext_rasterize, rasterize_color, rasterize_text_cluster};
use super::{
    is_emoji, is_emoji_text, Atlas, GlyphStyle, CLUSTER_KEY, CODEPOINT_MASK, GLYPH_PIECE, SHAPED_BLANK, STYLE_BOLD,
    STYLE_ITALIC, THICKEN, WIDE_LEFT, WIDE_RIGHT,
};

impl Atlas {
    /// Rasterize a glyph piece (see [`GLYPH_PIECE`]) into `buf`.
    fn rasterize_piece(&self, buf: &mut [u8], key: u32) {
        let Some(&p) = self.shaper.pieces.get(((key >> 5) & 0x3F_FFFF) as usize) else { return };
        let k = (key & 31) as usize;
        let (cw, ch) = (self.cell_w as usize, self.cell_h as usize);
        let w = cw * p.span as usize;
        let mut canvas = vec![0u8; w * ch];
        let pen_x = (p.origin as usize * cw) as f32;
        self.fonts.primary.draw_indexed_glyph(&mut canvas, w, ch, pen_x, self.baseline, self.px, p.gid);
        self.apply_style(&mut canvas, w, GlyphStyle { bold: p.bold, italic: p.italic });
        for y in 0..ch {
            buf[y * cw..(y + 1) * cw].copy_from_slice(&canvas[y * w + k * cw..y * w + (k + 1) * cw]);
        }
    }

    /// Rasterize a double-width (`2 * cell_w`) glyph or cluster once into either
    /// `wide_rgba` (returning `true` for color emoji) or `wide_r8` (returning `false`).
    pub(super) fn rasterize_wide(&self, base_key: u32, wide_r8: &mut [u8], wide_rgba: &mut [u8]) -> bool {
        let mut cp = base_key & CODEPOINT_MASK;
        let cluster = if base_key & CLUSTER_KEY != 0 { self.clusters.get(cp as usize).cloned() } else { None };
        if cluster.is_none() && (0x1F1E6..=0x1F1FF).contains(&cp) {
            cp = 'A' as u32 + (cp - 0x1F1E6);
        }
        let emoji = cluster.as_deref().map_or(is_emoji(cp), is_emoji_text);
        if emoji {
            let mut units = [0u8; 4];
            let text = match cluster.as_deref() {
                Some(t) => t,
                None => char::from_u32(cp).unwrap_or(' ').encode_utf8(&mut units),
            };
            if rasterize_color(wide_rgba, 2 * self.cell_w, self.cell_h, self.px, text) {
                return true;
            }
        }
        let style = GlyphStyle { bold: base_key & STYLE_BOLD != 0, italic: base_key & STYLE_ITALIC != 0 };
        match cluster.as_deref() {
            Some(text) => self.rasterize_cluster_into(wide_r8, 2 * self.cell_w, text),
            None => self.rasterize_into(wide_r8, 2 * self.cell_w, cp),
        }
        self.apply_style(wide_r8, 2 * self.cell_w as usize, style);
        false
    }

    /// Rasterize `key` into `scratch`, or into `color_scratch` (returning true)
    /// for a color emoji.
    pub(super) fn rasterize(&mut self, key: u32) -> bool {
        if key & GLYPH_PIECE != 0 && key != SHAPED_BLANK {
            let mut buf = std::mem::take(&mut self.scratch);
            buf.fill(0);
            self.rasterize_piece(&mut buf, key);
            self.scratch = buf;
            return false;
        }
        let half = key & (WIDE_LEFT | WIDE_RIGHT);
        let mut cp = key & CODEPOINT_MASK;
        let cluster = if key & CLUSTER_KEY != 0 { self.clusters.get(cp as usize).cloned() } else { None };
        // A lone regional indicator (a flag half the shell's line editor
        // prints on its own) has no emoji glyph: show its letter.
        if cluster.is_none() && (0x1F1E6..=0x1F1FF).contains(&cp) {
            cp = 'A' as u32 + (cp - 0x1F1E6);
        }
        // Wide emoji come from the system's color emoji font when there is one.
        let emoji = cluster.as_deref().map_or(is_emoji(cp), is_emoji_text);
        // So does an emoji sequence squeezed into one cell (❤️ at a zsh prompt).
        if half == 0 && emoji && cluster.is_some() {
            let text = cluster.as_deref().unwrap_or_default();
            let mut buf = std::mem::take(&mut self.color_scratch);
            buf.fill(0);
            let drawn = rasterize_color(&mut buf, self.cell_w, self.cell_h, self.px, text);
            self.color_scratch = buf;
            if drawn {
                self.scratch.fill(0);
                return true;
            }
        }
        if half != 0 && emoji {
            let (cw, ch) = (self.cell_w as usize, self.cell_h as usize);
            let mut wide = vec![0u8; 2 * cw * ch * 4];
            let mut units = [0u8; 4];
            let text = match cluster.as_deref() {
                Some(t) => t,
                None => char::from_u32(cp).unwrap_or(' ').encode_utf8(&mut units),
            };
            if rasterize_color(&mut wide, 2 * self.cell_w, self.cell_h, self.px, text) {
                let x0 = if half == WIDE_RIGHT { cw * 4 } else { 0 };
                for y in 0..ch {
                    let row = &wide[y * 2 * cw * 4 + x0..][..cw * 4];
                    self.color_scratch[y * cw * 4..(y + 1) * cw * 4].copy_from_slice(row);
                }
                self.scratch.fill(0);
                return true;
            }
        }
        let style = GlyphStyle { bold: key & STYLE_BOLD != 0, italic: key & STYLE_ITALIC != 0 };
        let mut buf = std::mem::take(&mut self.scratch);
        if half == 0 {
            buf.fill(0);
            match cluster.as_deref() {
                Some(text) => self.rasterize_cluster_into(&mut buf, self.cell_w, text),
                None => self.rasterize_into(&mut buf, self.cell_w, cp),
            }
            self.apply_style(&mut buf, self.cell_w as usize, style);
        } else {
            // Rasterize across two cells, then keep the requested half.
            let (cw, ch) = (self.cell_w as usize, self.cell_h as usize);
            let mut wide = vec![0u8; 2 * cw * ch];
            match cluster.as_deref() {
                Some(text) => self.rasterize_cluster_into(&mut wide, 2 * self.cell_w, text),
                None => self.rasterize_into(&mut wide, 2 * self.cell_w, cp),
            }
            self.apply_style(&mut wide, 2 * cw, style);
            let x0 = if half == WIDE_RIGHT { cw } else { 0 };
            for y in 0..ch {
                buf[y * cw..(y + 1) * cw].copy_from_slice(&wide[y * 2 * cw + x0..y * 2 * cw + x0 + cw]);
            }
        }
        self.scratch = buf;
        false
    }

    /// Rasterize a grapheme cluster (base + marks) into a `w × cell_h` canvas:
    /// shaped with the primary font so marks are positioned, else system shaping,
    /// else its first character alone.
    fn rasterize_cluster_into(&self, buf: &mut [u8], w: u32, text: &str) {
        if self.fonts.primary.rasterize_cluster_primary(buf, w, self.cell_h, self.baseline, self.px, text) {
            return;
        }
        if !rasterize_text_cluster(buf, w, self.cell_h, self.baseline, self.px, text) {
            buf.fill(0);
            let first = text.chars().next().map_or(0, |c| c as u32);
            self.rasterize_into(buf, w, first);
        }
    }

    /// Synthesize bold (horizontal dilation) and italic (shear around the baseline)
    /// on a `w × cell_h` coverage canvas, so any font works without bold/italic faces.
    fn apply_style(&self, buf: &mut [u8], w: usize, style: GlyphStyle) {
        let h = self.cell_h as usize;
        if style.italic {
            let mut row = vec![0u8; w];
            for y in 0..h {
                // ~12° slant: rows above the baseline move right, rows below move left.
                let shift = ((self.baseline - y as i32) as f32 * 0.21).round() as isize;
                let src = &buf[y * w..(y + 1) * w];
                row.fill(0);
                for (x, &v) in src.iter().enumerate() {
                    let nx = x as isize + shift;
                    if (0..w as isize).contains(&nx) {
                        row[nx as usize] = v;
                    }
                }
                buf[y * w..(y + 1) * w].copy_from_slice(&row);
            }
        }
        if style.bold {
            let stroke = ((self.px / 16.0).round() as usize).max(1);
            for y in 0..h {
                let line = &mut buf[y * w..(y + 1) * w];
                for x in (1..w).rev() {
                    let lo = x.saturating_sub(stroke);
                    let m = line[lo..x].iter().copied().max().unwrap_or(0);
                    line[x] = line[x].max(m);
                }
            }
        }
    }

    /// Rasterize `cp` into `buf`, a zeroed `cw × cell_h` canvas.
    fn rasterize_into(&self, buf: &mut [u8], cw: u32, cp: u32) {
        let Some(ch) = char::from_u32(cp) else { return };

        // 1. Ghostty-style built-in cell-exact sprite glyphs:
        //    Powerline, Box Drawing, Block Elements, Geometric triangles, Braille.
        if draw_builtin_sprite(buf, cw, self.cell_h, cp) {
            return;
        }

        // 2. Primary font (only if the font actually contains a glyph for `ch`).
        if self.fonts.primary.lookup_glyph_index(ch) != 0 {
            self.fonts.primary.rasterize_char(
                buf,
                cw as i32,
                self.cell_h as i32,
                self.baseline,
                self.px,
                ch,
                false,
            );
            return;
        }

        // 3. Fallback fonts (e.g., Symbols Nerd Font or user/system symbol fonts).
        for fb in &self.fonts.fallbacks {
            if fb.lookup_glyph_index(ch) != 0 {
                fb.rasterize_char(
                    buf,
                    cw as i32,
                    self.cell_h as i32,
                    self.baseline,
                    self.px,
                    ch,
                    true,
                );
                return;
            }
        }

        // 4. Built-in Nerd Font PUA icon fallbacks when no Nerd Font is installed.
        if draw_nerd_icon_fallback(buf, cw, self.cell_h, cp) {
            return;
        }

        // 5. System font cascade (CoreText on macOS, Fontconfig on Linux).
        // Skip Private Use Area codepoints (0xE000..=0xF8FF, 0xF0000..=0x10FFFF) because
        // system fonts do not map Nerd Font PUA glyphs and searching the cascade list
        // for unmapped PUA characters takes ~50ms on first encounter.
        if !(0xE000..=0xF8FF).contains(&cp) && cp < 0xF0000 {
            macos_coretext_rasterize(buf, cw, self.cell_h, self.baseline, self.px, ch);
        }
    }

    pub(super) fn upload_color(&mut self, queue: &crate::render::backend::Queue, slot: u16) {
        self.ensure_color_texture();
        let s = slot as u32;
        let x = (s % self.slots_per_row) * self.cell_w;
        let y = (s / self.slots_per_row) * self.cell_h;
        self.color_texture.write_rect(
            queue,
            x,
            y,
            self.cell_w,
            self.cell_h,
            &self.color_scratch,
            4 * self.cell_w,
        );
    }

    pub(super) fn upload(&self, queue: &crate::render::backend::Queue, slot: u16) {
        let s = slot as u32;
        let x = (s % self.slots_per_row) * self.cell_w;
        let y = (s / self.slots_per_row) * self.cell_h;
        self.texture.write_rect(
            queue,
            x,
            y,
            self.cell_w,
            self.cell_h,
            &self.scratch,
            self.cell_w,
        );
    }
}

/// Slight gamma/stem darkening for grayscale coverage so Retina stems match
/// Ghostty/CoreText weight while keeping 0 -> 0 and 255 -> 255 exact.
#[path = "stem_tables.rs"]
mod stem_tables;

#[inline]
pub(super) fn stem_boost(v: u8) -> u8 {
    let table = if THICKEN.load(Ordering::Relaxed) { &stem_tables::THICK } else { &stem_tables::NORMAL };
    table[v as usize]
}

#[cfg(test)]
#[test]
fn stem_tables_equal_original_f32_formula() {
    for (table, gamma) in [(&stem_tables::NORMAL, 0.78f32), (&stem_tables::THICK, 0.55f32)] {
        for (v, &actual) in table.iter().enumerate() {
            let expected = ((v as f32 / 255.0).powf(gamma) * 255.0 + 0.5) as u8;
            assert_eq!(actual, expected, "gamma={gamma} input={v}");
        }
    }
}
