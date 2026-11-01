//! One pane row → cell instances: glyph slots (shaped, bidi, clusters, wide),
//! selection / find highlights, decorations, split divider flags.

use super::{Instance, Renderer, COLOR_GLYPH, UNFOCUSED};
use crate::atlas::GlyphStyle;
use crate::grid::{char_width, Grid, Hyperlink, SearchHit, BG_EXTRA, BG_SET, BG_STRIKE, BG_UNDERLINE_MASK, FG_BOLD, FG_EXTRA, FG_ITALIC, FG_SET, UNDERLINE_SINGLE};
use crate::split::PaneRect;

impl Renderer {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn build_pane_row(
        &mut self,
        g: &Grid,
        src_row: usize,
        instance_start: Option<usize>,
        dst_row: usize,
        dst_col: usize,
        p_cols: usize,
        window_cols: usize,
        rect: &PaneRect,
        is_active: bool,
        active_rect: Option<&PaneRect>,
        hovered_link: Option<&Hyperlink>,
        pane_defaults: Option<(u32, u32)>,
    ) {
        let (sel_bg, sel_fg) = self.selection_colors;
        const MATCH_BG: u32 = 0x5C4D_2080; // Dim amber for every find match
        const MATCH_FG: u32 = 0xF9E2_AF80;
        const CURRENT_BG: u32 = 0xF9A8_2580; // Bright orange for the focused match
        const CURRENT_FG: u32 = 0x1E1E_2E80;
        let row = g.display_row(src_row);
        let hits = g.search_hits(src_row);
        let start = instance_start.unwrap_or(dst_row * window_cols + dst_col);
        let is_link_row = hovered_link.is_some_and(|l| l.row == src_row);
        let is_last_row = src_row + 1 == rect.rows;

        let is_border_right_active = is_active
            || active_rect.is_some_and(|ar| {
                ar.col == rect.col + rect.cols
                    && ar.row < rect.row + rect.rows
                    && ar.row + ar.rows > rect.row
            });

        let is_border_bottom_active = is_active
            || active_rect.is_some_and(|ar| {
                ar.row == rect.row + rect.rows
                    && ar.col < rect.col + rect.cols
                    && ar.col + ar.cols > rect.col
            });

        // Right-to-left text: cells are drawn in visual order (`col` is the
        // logical column, `v` where it is drawn) and Arabic letters joined.
        let (mut chars, mut levels, mut order) =
            (std::mem::take(&mut self.bidi_chars), std::mem::take(&mut self.bidi_levels), std::mem::take(&mut self.bidi_order));
        let bidi = g.bidi_order(src_row, &mut chars, &mut levels, &mut order);
        // Ligatures / `font-feature` substitutions for this row (not mixed with bidi).
        let mut shaped = std::mem::take(&mut self.shaped);
        let any_shaped = !bidi && self.atlas.shape_row(row, &mut shaped);
        let out = &mut self.instances[start..start + p_cols];
        // Right half of a double-width glyph, drawn in the placeholder cell after it.
        let mut wide_right: Option<u16> = None;
        for (v, dst) in out.iter_mut().enumerate().take(row.len()) {
            let col = if bidi { order[v] as usize } else { v };
            let cell = &row[col];
            let shaped_key = if any_shaped { shaped[col] } else { 0 };
            // Arabic contextual form from the logical neighbours.
            let form = if bidi {
                let c = chars[col];
                let joined = crate::bidi::arabic_form(col.checked_sub(1).map(|i| chars[i]), c, chars.get(col + 1).copied());
                (joined != c).then_some(joined)
            } else {
                None
            };
            let slot = match (cell.codepoint, wide_right.take()) {
                _ if shaped_key == crate::atlas::SHAPED_BLANK => crate::atlas::BLANK_SLOT,
                _ if shaped_key != 0 => self.atlas.slot(&self.queue, shaped_key),
                (0, Some(right)) => right,
                // Kitty unicode placeholder: the image is drawn over it.
                (cp, _) if cp & crate::grid::PH_FLAG != 0 => crate::atlas::BLANK_SLOT,
                (cp, _) if cp & crate::grid::CLUSTER_FLAG != 0 => {
                    let width = if crate::grid::is_wide_cp(cp) { 2 } else { 1 };
                    let style = GlyphStyle { bold: cell.fg & FG_BOLD != 0, italic: cell.fg & FG_ITALIC != 0 };
                    let text = g.cluster(cp).unwrap_or("");
                    // A joined Arabic letter keeps its marks.
                    let joined: String;
                    let text = match form {
                        Some(f) => {
                            joined = std::iter::once(f).chain(text.chars().skip(1)).collect();
                            joined.as_str()
                        }
                        None => text,
                    };
                    let (left, right) = self.atlas.cluster_slots(&self.queue, text, width, style);
                    if width == 2 {
                        wide_right = Some(right);
                    }
                    left
                }
                (cp, _) => {
                    let cp = form.map_or(cp, |f| f as u32);
                    let width = char::from_u32(cp).map_or(1, char_width);
                    let style = GlyphStyle { bold: cell.fg & FG_BOLD != 0, italic: cell.fg & FG_ITALIC != 0 };
                    let (left, right) = self.atlas.glyph_slots(&self.queue, cp, width, style);
                    if width == 2 {
                        wide_right = Some(right);
                    }
                    left
                }
            };
            // Underline style + strikethrough; a hovered link gets at least a single underline.
            let hovered = is_link_row && hovered_link.is_some_and(|l| col >= l.start_col && col <= l.end_col);
            let deco = match cell.bg & (BG_UNDERLINE_MASK | BG_STRIKE) {
                d if hovered && d & BG_UNDERLINE_MASK == 0 => d | UNDERLINE_SINGLE as u32,
                d => d,
            };

            // Divider flags (bit 0: right, bit 1: bottom, bit 2: active) packed into high 16 bits of glyph
            let mut div_flags = 0u32;
            if rect.has_border_right && v + 1 == rect.cols {
                div_flags |= 1;
                if is_border_right_active {
                    div_flags |= 4;
                }
            }
            if rect.has_border_bottom && is_last_row && g.scroll_fraction == 0.0 {
                div_flags |= 2;
                if is_border_bottom_active {
                    div_flags |= 4;
                }
            }
            let mut glyph_with_div = (slot as u32) | (div_flags << 16);
            if let Some(c) = g.underline_color(cell).filter(|_| deco & BG_UNDERLINE_MASK != 0) {
                let rgb444 = ((c >> 20) & 0xF00) | ((c >> 16) & 0xF0) | ((c >> 12) & 0xF);
                glyph_with_div |= (1 << 31) | (rgb444 << 19);
            }

            let hit = hits.at(col);
            if g.is_selected(src_row, col) {
                *dst = Instance { glyph: glyph_with_div, fg: sel_fg, bg: sel_bg | deco };
            } else if hit != SearchHit::None {
                let (fg, bg) = if hit == SearchHit::Current { (CURRENT_FG, CURRENT_BG) } else { (MATCH_FG, MATCH_BG) };
                *dst = Instance { glyph: glyph_with_div, fg, bg: bg | deco };
            } else {
                let mut fg = cell.fg & !FG_EXTRA;
                let mut bg = (cell.bg & !(BG_EXTRA | BG_UNDERLINE_MASK | BG_STRIKE)) | deco;
                // This pane's OSC 10 / 11 defaults differ from the shader's.
                if let Some((pane_fg, pane_bg)) = pane_defaults {
                    if fg & FG_SET == 0 {
                        fg |= pane_fg | FG_SET;
                    }
                    if bg & BG_SET == 0 {
                        bg |= pane_bg | BG_SET;
                    }
                }
                *dst = Instance { glyph: glyph_with_div, fg, bg };
            }
            if self.atlas.is_color(slot) {
                dst.fg |= COLOR_GLYPH;
            }
            if !is_active {
                dst.bg |= UNFOCUSED;
            }
        }
        self.shaped = shaped;
        (self.bidi_chars, self.bidi_levels, self.bidi_order) = (chars, levels, order);
    }
}
