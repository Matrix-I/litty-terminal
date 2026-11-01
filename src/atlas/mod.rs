//! Glyph atlas: `fontdue` rasterizes glyphs into fixed, cell-sized slots of a
//! single 512×512 `R8Unorm` texture (256 KB of VRAM). Printable ASCII is baked
//! at startup; every other codepoint is rasterized on first use.
//!
//! Like Ghostty (`font/sprite`), Powerline (`U+E0A0..=U+E0D7`), Box Drawing
//! (`U+2500..=U+257F`), Block Elements (`U+2580..=U+259F`), Geometric shapes
//! (`U+25E2..=U+25E5`), and Braille (`U+2800..=U+28FF`) are synthesized
//! procedurally to exact cell dimensions (`cell_w × cell_h`) with 4×4
//! supersampled anti-aliasing so segments and borders connect seamlessly
//! without requiring a patched Nerd Font. For other Unicode characters missing
//! from the primary font, a fallback font chain and macOS CoreText fallback
//! are used instead of drawing `.notdef` tofu boxes (`▯`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

mod fonts;
mod sprites;
pub use fonts::load_font;
use fonts::FONT_CACHE;
#[cfg(not(target_os = "macos"))]
use fonts::shaper_mmap_for;
use sprites::{draw_builtin_sprite, draw_nerd_icon_fallback, is_builtin_sprite};

/// `adjust-cell-width` / `adjust-cell-height`: grow or shrink the cell by
/// pixels or by a percentage of the font's own metric.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub enum MetricAdjust {
    #[default]
    None,
    Px(i32),
    Percent(f32),
}

impl MetricAdjust {
    /// Parse `2`, `-1` (pixels) or `10%`, `-5%`.
    pub fn parse(v: &str) -> Result<Self, String> {
        let v = v.trim();
        if let Some(p) = v.strip_suffix('%') {
            let p: f32 = p.trim().parse().ok().filter(|p: &f32| p.is_finite()).ok_or_else(|| format!("invalid percentage `{v}`"))?;
            return Ok(MetricAdjust::Percent(p));
        }
        v.parse().map(MetricAdjust::Px).map_err(|_| format!("expected pixels or a percentage, got `{v}`"))
    }

    fn apply(self, v: u32) -> u32 {
        let adjusted = match self {
            MetricAdjust::None => v as f32,
            MetricAdjust::Px(px) => v as f32 + px as f32,
            MetricAdjust::Percent(p) => v as f32 * (1.0 + p / 100.0),
        };
        adjusted.round().max(1.0) as u32
    }
}

/// Cell size adjustments (width, height), read when the atlas is built.
static CELL_ADJUST: Mutex<(MetricAdjust, MetricAdjust)> = parking_lot::const_mutex((MetricAdjust::None, MetricAdjust::None));
/// `font-thicken`: heavier stems for every rasterized glyph.
static THICKEN: AtomicBool = AtomicBool::new(false);

/// Set `adjust-cell-width`, `adjust-cell-height` and `font-thicken`; they take
/// effect when an atlas is (re)built.
pub fn set_font_tweaks(width: MetricAdjust, height: MetricAdjust, thicken: bool) {
    let mut adj = CELL_ADJUST.lock();
    if *adj != (width, height) || THICKEN.load(Ordering::Relaxed) != thicken {
        *adj = (width, height);
        THICKEN.store(thicken, Ordering::Relaxed);
        *FONT_CACHE.lock() = None;
    }
}

/// Slot 0 is always blank (space / empty cell).
pub const BLANK_SLOT: u16 = 0;

/// Slot-map key tags for the two halves of a double-width glyph (CJK, emoji).
/// Codepoints use at most 21 bits, so the high bits are free.
const WIDE_LEFT: u32 = 0x4000_0000;
const WIDE_RIGHT: u32 = 0x8000_0000;
/// Slot-map key tags for synthetic bold / italic variants.
const STYLE_BOLD: u32 = 0x1000_0000;
const STYLE_ITALIC: u32 = 0x2000_0000;
const CODEPOINT_MASK: u32 = 0x001F_FFFF;
/// Slot-map key tag for a grapheme cluster: the low bits index `Atlas::clusters`.
const CLUSTER_KEY: u32 = 0x0400_0000;

/// Glyph style requested by a cell.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct GlyphStyle {
    pub bold: bool,
    pub italic: bool,
}

pub struct FontSet {
    pub primary: fontdue::Font,
    pub fallbacks: Vec<fontdue::Font>,
    /// HarfBuzz-compatible shaper over the primary font, for ligatures and
    /// `font-feature`; its bytes stay mapped for the lifetime of this FontSet.
    shaper_mmap: Option<Arc<memmap2::Mmap>>,
    shaper_index: u32,
}

impl FontSet {
    pub fn shaper(&self) -> Option<rustybuzz::Face<'_>> {
        let mmap = self.shaper_mmap.as_ref()?;
        rustybuzz::Face::from_slice(mmap, self.shaper_index)
    }
}

/// Slot-map key tag for a shaped glyph piece: bits 5..27 index `Atlas::pieces`,
/// bits 0..5 select the cell of a glyph that spans several cells.
const GLYPH_PIECE: u32 = 0x0800_0000;
/// Shaped key for a cell whose glyph has no ink (a ligature spacer).
pub const SHAPED_BLANK: u32 = u32::MAX;

/// A glyph by index, drawn across `span` cells with its pen at cell `origin`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
struct GlyphPiece {
    gid: u16,
    span: u8,
    origin: u8,
    bold: bool,
    italic: bool,
}

/// Ligature shaping state: features, a reusable buffer, interned glyph pieces.
struct Shaper {
    features: Vec<rustybuzz::Feature>,
    buf: Option<rustybuzz::UnicodeBuffer>,
    pieces: Vec<GlyphPiece>,
    piece_ids: HashMap<GlyphPiece, u32>,
    /// Advance of one cell in font units (0 = not measured yet, -1 = unknown).
    cell_units: i32,
}

impl Shaper {
    fn new(features: Vec<rustybuzz::Feature>) -> Self {
        Shaper { features, buf: None, pieces: Vec::new(), piece_ids: HashMap::new(), cell_units: 0 }
    }

    /// Shape the ligature candidates of a row: runs of printable ASCII in one
    /// style that contain punctuation (where coding-font ligatures live).
    /// `out[col]` becomes a slot key for a substituted glyph, `SHAPED_BLANK`
    /// for an empty spacer, or 0 for "draw the codepoint as usual". Returns
    /// `(changed, reset_pieces)`. Allocation-free once the buffers are warm.
    pub fn shape_row(&mut self, fonts: &FontSet, px: f32, cell_w: u32, row: &[crate::grid::Cell], out: &mut Vec<u32>) -> (bool, bool) {
        let mut reset_pieces = false;
        if self.pieces.len() >= 4096 {
            self.pieces.clear();
            self.piece_ids.clear();
            reset_pieces = true;
        }
        out.clear();
        out.resize(row.len(), 0);
        let Some(face) = fonts.shaper() else { return (false, reset_pieces) };
        if self.cell_units == 0 {
            self.cell_units = face.glyph_index('M').and_then(|g| face.glyph_hor_advance(g)).map_or(-1, i32::from);
        }
        if self.cell_units <= 0 {
            return (false, reset_pieces);
        }
        let style_of = |c: &crate::grid::Cell| (c.fg & crate::grid::FG_BOLD != 0, c.fg & crate::grid::FG_ITALIC != 0);
        let shapeable = |c: &crate::grid::Cell| (0x21..0x7F).contains(&c.codepoint);
        let mut changed = false;
        let mut i = 0;
        while i < row.len() {
            if !shapeable(&row[i]) {
                i += 1;
                continue;
            }
            let style = style_of(&row[i]);
            let start = i;
            while i < row.len() && shapeable(&row[i]) && style_of(&row[i]) == style {
                i += 1;
            }
            let run = &row[start..i];
            let punct = run.iter().any(|c| !(c.codepoint as u8).is_ascii_alphanumeric());
            if run.len() < 2 || !punct {
                continue;
            }

            let mut buf = self.buf.take().unwrap_or_default();
            for c in run {
                buf.add(char::from_u32(c.codepoint).unwrap_or(' '), 0);
            }
            buf.set_direction(rustybuzz::Direction::LeftToRight);
            let glyphs = rustybuzz::shape(&face, &self.features, buf);
            let mut pen = 0i32;
            for (info, pos) in glyphs.glyph_infos().iter().zip(glyphs.glyph_positions()) {
                let rel = ((pen + self.cell_units / 2) / self.cell_units) as usize;
                pen += pos.x_advance;
                let (Some(cell), Ok(gid)) = (run.get(rel), u16::try_from(info.glyph_id)) else { continue };
                let ch = char::from_u32(cell.codepoint).unwrap_or(' ');
                let nominal = face.glyph_index(ch).map_or(0, |g| g.0);
                if gid == nominal && pos.x_offset == 0 && pos.y_offset == 0 {
                    continue;
                }
                let m = fonts.primary.metrics_indexed(gid, px);
                if m.width == 0 || m.height == 0 {
                    if out[start + rel] == 0 {
                        out[start + rel] = SHAPED_BLANK;
                        changed = true;
                    }
                    continue;
                }
                // Cells the glyph's ink covers, relative to its pen cell.
                let cw = cell_w as i32;
                let x0 = m.xmin + pos.x_offset * px as i32 / face.units_per_em().max(1);
                let first = rel as i32 + x0.div_euclid(cw);
                let last = rel as i32 + (x0 + m.width as i32 - 1).div_euclid(cw);
                let (first, last) = (first.max(0), last.min(run.len() as i32 - 1));
                let (span, origin) = (last - first + 1, rel as i32 - first);
                if !(1..=31).contains(&span) || !(0..32).contains(&origin) {
                    continue;
                }
                let piece = GlyphPiece { gid, span: span as u8, origin: origin as u8, bold: style.0, italic: style.1 };
                let next = self.pieces.len() as u32;
                let id = *self.piece_ids.entry(piece).or_insert(next);
                if id == next {
                    self.pieces.push(piece);
                }
                for k in 0..span {
                    out[start + (first + k) as usize] = GLYPH_PIECE | (id << 5) | k as u32;
                }
                changed = true;
            }
            self.buf = Some(glyphs.clear());
        }
        (changed, reset_pieces)
    }

}

/// `font-feature` values (`-calt`, `ss01`, `liga=0`), read when an atlas is built.
static FONT_FEATURES: Mutex<Vec<rustybuzz::Feature>> = parking_lot::const_mutex(Vec::new());

/// Parse `font-feature` values; unknown syntax is an error.
pub fn parse_font_feature(v: &str) -> Result<rustybuzz::Feature, String> {
    v.trim().parse::<rustybuzz::Feature>().map_err(|_| format!("invalid font feature `{v}`"))
}

pub fn set_font_features(features: &[rustybuzz::Feature]) {
    let mut feat = FONT_FEATURES.lock();
    if *feat != features {
        *feat = features.to_vec();
        *FONT_CACHE.lock() = None;
    }
}

pub struct Atlas {
    fonts: Arc<FontSet>,
    px: f32,
    pub cell_w: u32,
    pub cell_h: u32,
    baseline: i32,

    pub size: u32,
    pub slots_per_row: u32,
    n_slots: u32,
    next_free: u32,

    ascii: [u16; 128],
    map: HashMap<u32, u16>,
    slot_cp: Vec<u32>,
    slot_used: Vec<u64>,
    lru_head: u16,
    lru_tail: u16,
    lru_prev: Vec<u16>,
    lru_next: Vec<u16>,
    frame: u64,
    evicted: bool,
    grow_needed: bool,
    scratch: Vec<u8>,
    shaper: Shaper,

    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    /// Color glyphs (emoji), premultiplied RGBA in the same slot layout as `texture`.
    color_texture: wgpu::Texture,
    pub color_view: wgpu::TextureView,
    slot_color: Vec<bool>,
    color_scratch: Vec<u8>,
    wide_r8_scratch: Vec<u8>,
    wide_rgba_scratch: Vec<u8>,
    /// Grapheme clusters drawn so far, interned for their slot keys.
    clusters: Vec<Box<str>>,
    cluster_ids: HashMap<Box<str>, u32>,
}

impl Atlas {
    /// Vertical positions inside a cell, in px from its top: where the underline
    /// starts (just below the baseline) and the center of a strikethrough
    /// (half the x-height above the baseline).
    pub fn decoration_offsets(&self) -> (f32, f32) {
        let underline = self.baseline as f32 + (self.px * 0.08).round().max(1.0);
        let strike = self.baseline as f32 - (self.px * 0.26).round();
        (underline, strike)
    }

    /// Cell dimensions in logical points for a given font and point size.
    pub fn cell_size_pt(fonts: &FontSet, pt: f32) -> (f64, f64) {
        let font = &fonts.primary;
        let lm = font
            .horizontal_line_metrics(pt)
            .expect("font has no horizontal line metrics");
        let natural_w = font.metrics('M', pt).advance_width.round().max(1.0) as u32;
        let natural_h = (lm.ascent - lm.descent + lm.line_gap).ceil().max(1.0) as u32;
        let (adjust_w, adjust_h) = *CELL_ADJUST.lock();
        (adjust_w.apply(natural_w) as f64, adjust_h.apply(natural_h) as f64)
    }

    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, fonts: Arc<FontSet>, px: f32) -> Self {
        let font = &fonts.primary;
        let lm = font
            .horizontal_line_metrics(px)
            .expect("font has no horizontal line metrics");
        let natural_w = font.metrics('M', px).advance_width.round().max(1.0) as u32;
        let natural_h = (lm.ascent - lm.descent + lm.line_gap).ceil().max(1.0) as u32;
        let (adjust_w, adjust_h) = *CELL_ADJUST.lock();
        let cell_w = adjust_w.apply(natural_w);
        let cell_h = adjust_h.apply(natural_h);
        // Extra (or missing) height is split above and below the text.
        let baseline = (lm.ascent + lm.line_gap / 2.0 + (cell_h as f32 - natural_h as f32) / 2.0).round() as i32;

        // 512×512 default (256KB VRAM for R8, 1MB for RGBA8); grows dynamically for large fonts.
        let mut size = 512u32;
        while (size / cell_w) * (size / cell_h) < 256 && size < 4096 {
            size *= 2;
        }
        let slots_per_row = size / cell_w;
        let n_slots = (slots_per_row * (size / cell_h)).min(u16::MAX as u32);

        let (texture, view) = Self::make_texture(device, size, wgpu::TextureFormat::R8Unorm);
        let (color_texture, color_view) = Self::make_texture(device, size, wgpu::TextureFormat::Rgba8Unorm);

        let mut atlas = Atlas {
            fonts,
            px,
            cell_w,
            cell_h,
            baseline,
            size,
            slots_per_row,
            n_slots,
            next_free: 1,
            ascii: [BLANK_SLOT; 128],
            map: HashMap::with_capacity(n_slots as usize),
            slot_cp: vec![0; n_slots as usize],
            slot_used: vec![0; n_slots as usize],
            lru_head: 0,
            lru_tail: 0,
            lru_prev: vec![0; n_slots as usize],
            lru_next: vec![0; n_slots as usize],
            frame: 1,
            evicted: false,
            grow_needed: false,
            scratch: vec![0; (cell_w * cell_h) as usize],
            shaper: Shaper::new(FONT_FEATURES.lock().clone()),
            texture,
            view,
            color_texture,
            color_view,
            slot_color: vec![false; n_slots as usize],
            color_scratch: vec![0; (cell_w * cell_h * 4) as usize],
            wide_r8_scratch: vec![0; (2 * cell_w * cell_h) as usize],
            wide_rgba_scratch: vec![0; (2 * cell_w * cell_h * 4) as usize],
            clusters: Vec::new(),
            cluster_ids: HashMap::new(),
        };
        atlas.preload_ascii(queue);
        atlas
    }

    /// Dynamically grow the glyph atlas texture (`512 -> 1024 -> 2048 -> 4096`) when
    /// all slots were active in the same frame, avoiding 512×512 slot thrashing on
    /// CJK/emoji screens. Returns `true` if the texture was recreated (bind groups
    /// must be rebound).
    pub fn grow_if_needed(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> bool {
        if !std::mem::take(&mut self.grow_needed) || self.size >= 4096 {
            return false;
        }
        let new_size = (self.size * 2).min(4096);
        let slots_per_row = new_size / self.cell_w;
        let n_slots = (slots_per_row * (new_size / self.cell_h)).min(u16::MAX as u32);
        if n_slots <= self.n_slots {
            return false;
        }

        let (texture, view) = Self::make_texture(device, new_size, wgpu::TextureFormat::R8Unorm);
        let (color_texture, color_view) = Self::make_texture(device, new_size, wgpu::TextureFormat::Rgba8Unorm);

        self.size = new_size;
        self.slots_per_row = slots_per_row;
        self.n_slots = n_slots;
        self.next_free = 1;
        self.ascii = [BLANK_SLOT; 128];
        self.map.clear();
        self.map.reserve(n_slots as usize);
        self.slot_cp.clear();
        self.slot_cp.resize(n_slots as usize, 0);
        self.slot_used.clear();
        self.slot_used.resize(n_slots as usize, 0);
        self.lru_head = 0;
        self.lru_tail = 0;
        self.lru_prev.clear();
        self.lru_prev.resize(n_slots as usize, 0);
        self.lru_next.clear();
        self.lru_next.resize(n_slots as usize, 0);
        self.slot_color.clear();
        self.slot_color.resize(n_slots as usize, false);
        self.texture = texture;
        self.view = view;
        self.color_texture = color_texture;
        self.color_view = color_view;
        self.preload_ascii(queue);
        true
    }

    fn lru_push_tail(&mut self, s: u16) {
        if self.lru_head == 0 {
            self.lru_head = s;
            self.lru_tail = s;
            self.lru_prev[s as usize] = 0;
            self.lru_next[s as usize] = 0;
        } else {
            self.lru_prev[s as usize] = self.lru_tail;
            self.lru_next[s as usize] = 0;
            self.lru_next[self.lru_tail as usize] = s;
            self.lru_tail = s;
        }
    }

    fn lru_remove(&mut self, s: u16) {
        let p = self.lru_prev[s as usize];
        let n = self.lru_next[s as usize];
        if p != 0 {
            self.lru_next[p as usize] = n;
        } else if self.lru_head == s {
            self.lru_head = n;
        }
        if n != 0 {
            self.lru_prev[n as usize] = p;
        } else if self.lru_tail == s {
            self.lru_tail = p;
        }
        self.lru_prev[s as usize] = 0;
        self.lru_next[s as usize] = 0;
    }

    fn lru_touch(&mut self, s: u16) {
        if self.lru_tail == s {
            return;
        }
        self.lru_remove(s);
        self.lru_push_tail(s);
    }

    fn lru_touch_head(&mut self, s: u16) {
        if self.lru_head == s {
            return;
        }
        self.lru_remove(s);
        if self.lru_head == 0 {
            self.lru_head = s;
            self.lru_tail = s;
            self.lru_prev[s as usize] = 0;
            self.lru_next[s as usize] = 0;
        } else {
            self.lru_next[s as usize] = self.lru_head;
            self.lru_prev[s as usize] = 0;
            self.lru_prev[self.lru_head as usize] = s;
            self.lru_head = s;
        }
    }

    fn preload_ascii(&mut self, queue: &wgpu::Queue) {
        let total_slots = 95u32.min(self.n_slots);
        let rows_needed = total_slots.div_ceil(self.slots_per_row);
        let upload_height = (rows_needed * self.cell_h).min(self.size);
        let mut staging = vec![0u8; (self.size * upload_height) as usize];
        let bytes_per_row = self.size;

        for c in 33u32..=126 {
            if self.next_free >= self.n_slots {
                break;
            }
            let s = self.next_free as u16;
            self.next_free += 1;
            self.lru_push_tail(s);

            self.rasterize(c);
            let slot = s as u32;
            let sx = (slot % self.slots_per_row) * self.cell_w;
            let sy = (slot / self.slots_per_row) * self.cell_h;
            if sy + self.cell_h <= upload_height && sx + self.cell_w <= self.size {
                for y in 0..self.cell_h {
                    let src_start = (y * self.cell_w) as usize;
                    let src_end = src_start + self.cell_w as usize;
                    let dst_start = ((sy + y) * bytes_per_row + sx) as usize;
                    staging[dst_start..dst_start + self.cell_w as usize]
                        .copy_from_slice(&self.scratch[src_start..src_end]);
                }
            } else {
                self.upload(queue, s);
            }
            self.slot_color[s as usize] = false;
            self.ascii[c as usize] = s;
            self.slot_cp[s as usize] = c;
            self.slot_used[s as usize] = self.frame;
        }

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 0, y: 0, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &staging,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(upload_height),
            },
            wgpu::Extent3d { width: self.size, height: upload_height, depth_or_array_layers: 1 },
        );
    }

    /// Rebuild the atlas in place for a new pixel size (DPI or zoom change).
    pub fn set_px(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, px: f32) {
        *self = Atlas::new(device, queue, self.fonts.clone(), px);
    }

    /// Rebuild the atlas with a different font set (config reload).
    pub fn set_fonts(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, fonts: Arc<FontSet>, px: f32) {
        *self = Atlas::new(device, queue, fonts, px);
    }

    fn make_texture(device: &wgpu::Device, size: u32, format: wgpu::TextureFormat) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glyph atlas"),
            size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        (texture, view)
    }

    pub fn begin_frame(&mut self) {
        self.frame += 1;
    }

    /// True if a slot was recycled since the last call (other rows may now be stale).
    pub fn take_evicted(&mut self) -> bool {
        std::mem::take(&mut self.evicted)
    }

    /// Atlas slots for a glyph of display width `width`. Double-width glyphs are
    /// rasterized across two cells in a single pass and split into a left and a
    /// right slot; for narrow glyphs the right slot is blank.
    pub fn glyph_slots(&mut self, queue: &wgpu::Queue, cp: u32, width: usize, style: GlyphStyle) -> (u16, u16) {
        let mut key = cp;
        // Cell-exact sprites (box drawing, Powerline...) must keep connecting seamlessly.
        if !is_builtin_sprite(cp) {
            if style.bold {
                key |= STYLE_BOLD;
            }
            if style.italic {
                key |= STYLE_ITALIC;
            }
        }
        if width == 2 {
            self.wide_slots(queue, key)
        } else {
            (self.slot(queue, key), BLANK_SLOT)
        }
    }

    fn alloc_slot(&mut self, cp: u32) -> u16 {
        let s = if self.next_free < self.n_slots {
            let s = self.next_free as u16;
            self.next_free += 1;
            self.lru_push_tail(s);
            s
        } else {
            // Constant-time O(1) LRU eviction. If the oldest LRU slot was already used
            // in the current frame, all slots are active simultaneously -> flag grow_needed.
            let candidate = if self.lru_head != 0 {
                self.lru_head
            } else {
                return BLANK_SLOT;
            };
            let i = candidate as usize;
            if self.slot_used[i] == self.frame && self.size < 4096 {
                self.grow_needed = true;
            }
            self.lru_touch(candidate);
            let old_cp = self.slot_cp[i];
            if old_cp < 128 {
                self.ascii[old_cp as usize] = BLANK_SLOT;
            } else {
                self.map.remove(&old_cp);
            }
            self.evicted = true;
            candidate
        };
        if cp < 128 {
            self.ascii[cp as usize] = s;
        } else {
            self.map.insert(cp, s);
        }
        self.slot_cp[s as usize] = cp;
        self.slot_used[s as usize] = self.frame;
        s
    }

    /// Resolve both `WIDE_LEFT` and `WIDE_RIGHT` slots for a double-width key,
    /// rasterizing `2 * cell_w` only once when either half misses in the cache.
    fn wide_slots(&mut self, queue: &wgpu::Queue, base_key: u32) -> (u16, u16) {
        let k_left = base_key | WIDE_LEFT;
        let k_right = base_key | WIDE_RIGHT;
        let cached_left = self.map.get(&k_left).copied();
        let cached_right = self.map.get(&k_right).copied();
        if let (Some(sl), Some(sr)) = (cached_left, cached_right) {
            if self.slot_used[sl as usize] != self.frame {
                self.slot_used[sl as usize] = self.frame;
                self.lru_touch(sl);
            }
            if self.slot_used[sr as usize] != self.frame {
                self.slot_used[sr as usize] = self.frame;
                self.lru_touch(sr);
            }
            return (sl, sr);
        }

        let (cw, ch) = (self.cell_w as usize, self.cell_h as usize);
        let mut wide_r8 = std::mem::take(&mut self.wide_r8_scratch);
        let mut wide_rgba = std::mem::take(&mut self.wide_rgba_scratch);
        wide_r8.fill(0);
        wide_rgba.fill(0);
        let color = self.rasterize_wide(base_key, &mut wide_r8, &mut wide_rgba);

        let sl = match cached_left {
            Some(s) => {
                self.slot_used[s as usize] = self.frame;
                self.lru_touch(s);
                s
            }
            None => {
                let s = self.alloc_slot(k_left);
                if s != BLANK_SLOT {
                    if color {
                        for y in 0..ch {
                            let row = &wide_rgba[y * 2 * cw * 4..][..cw * 4];
                            self.color_scratch[y * cw * 4..(y + 1) * cw * 4].copy_from_slice(row);
                        }
                        self.scratch.fill(0);
                        self.upload(queue, s);
                        self.upload_color(queue, s);
                    } else {
                        for y in 0..ch {
                            self.scratch[y * cw..(y + 1) * cw].copy_from_slice(&wide_r8[y * 2 * cw..y * 2 * cw + cw]);
                        }
                        self.upload(queue, s);
                    }
                    self.slot_color[s as usize] = color;
                }
                s
            }
        };

        let sr = match self.map.get(&k_right).copied() {
            Some(s) => {
                self.slot_used[s as usize] = self.frame;
                self.lru_touch(s);
                s
            }
            None => {
                let s = self.alloc_slot(k_right);
                if s != BLANK_SLOT {
                    if color {
                        for y in 0..ch {
                            let row = &wide_rgba[y * 2 * cw * 4 + cw * 4..][..cw * 4];
                            self.color_scratch[y * cw * 4..(y + 1) * cw * 4].copy_from_slice(row);
                        }
                        self.scratch.fill(0);
                        self.upload(queue, s);
                        self.upload_color(queue, s);
                    } else {
                        for y in 0..ch {
                            self.scratch[y * cw..(y + 1) * cw].copy_from_slice(&wide_r8[y * 2 * cw + cw..y * 2 * cw + 2 * cw]);
                        }
                        self.upload(queue, s);
                    }
                    self.slot_color[s as usize] = color;
                }
                s
            }
        };

        self.wide_r8_scratch = wide_r8;
        self.wide_rgba_scratch = wide_rgba;
        (sl, sr)
    }

    /// Atlas slot for `cp` (optionally tagged with `WIDE_LEFT` / `WIDE_RIGHT`),
    /// rasterizing and uploading it on a cache miss.
    pub fn slot(&mut self, queue: &wgpu::Queue, cp: u32) -> u16 {
        if cp == 0 || cp == 32 {
            return BLANK_SLOT;
        }
        if cp < 128 {
            let s = self.ascii[cp as usize];
            if s != BLANK_SLOT {
                if self.slot_used[s as usize] != self.frame {
                    self.slot_used[s as usize] = self.frame;
                    self.lru_touch(s);
                }
                return s;
            }
        } else if let Some(&s) = self.map.get(&cp) {
            if self.slot_used[s as usize] != self.frame {
                self.slot_used[s as usize] = self.frame;
                self.lru_touch(s);
            }
            return s;
        }
        let s = self.alloc_slot(cp);
        if s == BLANK_SLOT {
            return BLANK_SLOT;
        }
        let color = self.rasterize(cp);
        self.upload(queue, s);
        if color {
            self.upload_color(queue, s);
        }
        self.slot_color[s as usize] = color;
        s
    }

    /// Atlas slots for a grapheme cluster (see [`Atlas::glyph_slots`]).
    pub fn cluster_slots(&mut self, queue: &wgpu::Queue, text: &str, width: usize, style: GlyphStyle) -> (u16, u16) {
        if self.clusters.len() >= 4096 {
            self.clusters.clear();
            self.cluster_ids.clear();
            self.map.retain(|&k, _| k & CLUSTER_KEY == 0);
            for i in 1..self.n_slots as usize {
                if self.slot_cp[i] & CLUSTER_KEY != 0 {
                    self.slot_cp[i] = 0;
                    self.slot_used[i] = 0;
                    self.lru_touch_head(i as u16);
                }
            }
            self.evicted = true;
        }
        let id = match self.cluster_ids.get(text) {
            Some(&id) => id,
            None if self.clusters.len() <= CODEPOINT_MASK as usize => {
                let id = self.clusters.len() as u32;
                self.clusters.push(text.into());
                self.cluster_ids.insert(text.into(), id);
                id
            }
            None => return (BLANK_SLOT, BLANK_SLOT),
        };
        let mut key = CLUSTER_KEY | id;
        if style.bold {
            key |= STYLE_BOLD;
        }
        if style.italic {
            key |= STYLE_ITALIC;
        }
        if width == 2 {
            self.wide_slots(queue, key)
        } else {
            (self.slot(queue, key), BLANK_SLOT)
        }
    }

    /// True if `slot` holds a color glyph (drawn from `color_view`).
    pub fn is_color(&self, slot: u16) -> bool {
        self.slot_color.get(slot as usize).copied().unwrap_or(false)
    }

    /// See [`Shaper::shape_row`].
    pub fn shape_row(&mut self, row: &[crate::grid::Cell], out: &mut Vec<u32>) -> bool {
        let (changed, reset_pieces) = self.shaper.shape_row(&self.fonts, self.px, self.cell_w, row, out);
        if reset_pieces {
            self.map.retain(|&k, _| k & GLYPH_PIECE == 0);
            for i in 1..self.n_slots as usize {
                if self.slot_cp[i] & GLYPH_PIECE != 0 {
                    self.slot_cp[i] = 0;
                    self.slot_used[i] = 0;
                    self.lru_touch_head(i as u16);
                }
            }
            self.evicted = true;
        }
        changed
    }

    /// Rasterize a glyph piece (see [`GLYPH_PIECE`]) into `buf`.
    fn rasterize_piece(&self, buf: &mut [u8], key: u32) {
        let Some(&p) = self.shaper.pieces.get(((key >> 5) & 0x3F_FFFF) as usize) else { return };
        let k = (key & 31) as usize;
        let (cw, ch) = (self.cell_w as usize, self.cell_h as usize);
        let w = cw * p.span as usize;
        let mut canvas = vec![0u8; w * ch];
        let (m, bmp) = self.fonts.primary.rasterize_indexed(p.gid, self.px);
        let x0 = p.origin as i32 * cw as i32 + m.xmin;
        let y0 = self.baseline - (m.ymin + m.height as i32);
        for gy in 0..m.height as i32 {
            let y = y0 + gy;
            if y < 0 || y >= ch as i32 {
                continue;
            }
            for gx in 0..m.width as i32 {
                let x = x0 + gx;
                if x < 0 || x >= w as i32 {
                    continue;
                }
                let v = stem_boost(bmp[(gy * m.width as i32 + gx) as usize]);
                let d = &mut canvas[y as usize * w + x as usize];
                *d = (*d).max(v);
            }
        }
        self.apply_style(&mut canvas, w, GlyphStyle { bold: p.bold, italic: p.italic });
        for y in 0..ch {
            buf[y * cw..(y + 1) * cw].copy_from_slice(&canvas[y * w + k * cw..y * w + (k + 1) * cw]);
        }
    }

    /// Rasterize a double-width (`2 * cell_w`) glyph or cluster once into either
    /// `wide_rgba` (returning `true` for color emoji) or `wide_r8` (returning `false`).
    fn rasterize_wide(&self, base_key: u32, wide_r8: &mut [u8], wide_rgba: &mut [u8]) -> bool {
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
    fn rasterize(&mut self, key: u32) -> bool {
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
    /// shaped with the primary font so marks are positioned, else its first
    /// character alone.
    fn rasterize_cluster_into(&self, buf: &mut [u8], w: u32, text: &str) {
        let first = text.chars().next().map_or(0, |c| c as u32);
        let shaper = self.fonts.shaper();
        let shaped = shaper.as_ref().map(|face| {
            let mut ub = rustybuzz::UnicodeBuffer::new();
            ub.push_str(text);
            (face, rustybuzz::shape(face, &[], ub))
        });
        let Some((face, shaped)) = shaped.filter(|(_, s)| s.glyph_infos().iter().all(|g| g.glyph_id != 0)) else {
            // The primary font lacks a glyph: the system's shaping and fonts, else the base alone.
            if !rasterize_text_cluster(buf, w, self.cell_h, self.baseline, self.px, text) {
                buf.fill(0);
                self.rasterize_into(buf, w, first);
            }
            return;
        };
        let scale = self.px / face.units_per_em() as f32;
        let advance: f32 = shaped.glyph_positions().iter().map(|p| p.x_advance as f32 * scale).sum();
        // Centered like single glyphs are.
        let mut pen = ((w as f32 - advance) / 2.0).max(0.0);
        let h = self.cell_h as i32;
        for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
            let (m, bmp) = self.fonts.primary.rasterize_indexed(info.glyph_id as u16, self.px);
            let x0 = (pen + pos.x_offset as f32 * scale).round() as i32 + m.xmin;
            let y0 = self.baseline - (m.ymin + m.height as i32) - (pos.y_offset as f32 * scale).round() as i32;
            for gy in 0..m.height as i32 {
                let y = y0 + gy;
                if !(0..h).contains(&y) {
                    continue;
                }
                for gx in 0..m.width as i32 {
                    let x = x0 + gx;
                    if !(0..w as i32).contains(&x) {
                        continue;
                    }
                    let v = stem_boost(bmp[(gy * m.width as i32 + gx) as usize]);
                    let d = &mut buf[y as usize * w as usize + x as usize];
                    *d = (*d).max(v);
                }
            }
            pen += pos.x_advance as f32 * scale;
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
            rasterize_with_font(
                buf,
                cw as i32,
                self.cell_h as i32,
                self.baseline,
                self.px,
                &self.fonts.primary,
                ch,
                false,
            );
            return;
        }

        // 3. Fallback fonts (e.g., Symbols Nerd Font or user/system symbol fonts).
        for fb in &self.fonts.fallbacks {
            if fb.lookup_glyph_index(ch) != 0 {
                rasterize_with_font(
                    buf,
                    cw as i32,
                    self.cell_h as i32,
                    self.baseline,
                    self.px,
                    fb,
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

        // 5. macOS CoreText system font cascade (Apple Symbols, PingFang, Supplemental, etc.).
        // Skip Private Use Area codepoints (0xE000..=0xF8FF, 0xF0000..=0x10FFFF) because
        // macOS system fonts do not map Nerd Font PUA glyphs and searching the CoreText
        // cascade list for unmapped PUA characters takes ~50ms on first encounter.
        if !(0xE000..=0xF8FF).contains(&cp) && cp < 0xF0000 {
            macos_coretext_rasterize(buf, cw, self.cell_h, self.baseline, self.px, ch);
        }
    }

    fn upload_color(&self, queue: &wgpu::Queue, slot: u16) {
        let s = slot as u32;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.color_texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: (s % self.slots_per_row) * self.cell_w,
                    y: (s / self.slots_per_row) * self.cell_h,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &self.color_scratch,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * self.cell_w),
                rows_per_image: Some(self.cell_h),
            },
            wgpu::Extent3d { width: self.cell_w, height: self.cell_h, depth_or_array_layers: 1 },
        );
    }

    fn upload(&self, queue: &wgpu::Queue, slot: u16) {
        let s = slot as u32;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: (s % self.slots_per_row) * self.cell_w,
                    y: (s / self.slots_per_row) * self.cell_h,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &self.scratch,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.cell_w),
                rows_per_image: Some(self.cell_h),
            },
            wgpu::Extent3d { width: self.cell_w, height: self.cell_h, depth_or_array_layers: 1 },
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn rasterize_with_font(
    scratch: &mut [u8],
    cw: i32,
    chh: i32,
    baseline: i32,
    mut px: f32,
    font: &fontdue::Font,
    ch: char,
    center_if_symbol: bool,
) {
    // Scale down if a fallback symbol glyph is wider/taller than the cell.
    if center_if_symbol {
        let m0 = font.metrics(ch, px);
        if m0.width as i32 > cw || m0.height as i32 > chh {
            let sx = cw as f32 / (m0.width.max(1) as f32);
            let sy = chh as f32 / (m0.height.max(1) as f32);
            px = (px * sx.min(sy) * 0.92).max(6.0);
        }
    }
    let (m, bmp) = font.rasterize(ch, px);
    if m.width == 0 || m.height == 0 {
        return;
    }
    let mut x0 = if center_if_symbol {
        (cw - m.width as i32) / 2
    } else {
        m.xmin
    };
    let mut y0 = baseline - (m.ymin + m.height as i32);
    if (m.width as i32) <= cw {
        x0 = x0.clamp(0, cw - m.width as i32);
    }
    if (m.height as i32) <= chh {
        y0 = y0.clamp(0, chh - m.height as i32);
    }
    for gy in 0..m.height as i32 {
        let y = y0 + gy;
        if y < 0 || y >= chh {
            continue;
        }
        for gx in 0..m.width as i32 {
            let x = x0 + gx;
            if x < 0 || x >= cw {
                continue;
            }
            let v = stem_boost(bmp[(gy * m.width as i32 + gx) as usize]);
            let d = &mut scratch[(y * cw + x) as usize];
            *d = (*d).max(v);
        }
    }
}

/// Slight gamma/stem darkening for fontdue grayscale coverage so Retina stems
/// match Ghostty/CoreText weight while keeping 0 -> 0 and 255 -> 255 exact.
#[inline]
fn stem_boost(v: u8) -> u8 {
    let gamma = if THICKEN.load(Ordering::Relaxed) { 0.55 } else { 0.78 };
    match v {
        0 | 255 => v,
        _ => ((v as f32 / 255.0).powf(gamma) * 255.0 + 0.5) as u8,
    }
}

// ============================================================================
// macOS CoreText + CoreGraphics System Fallback Rasterizer
// ============================================================================

/// System font fallback is CoreText-only; elsewhere the fontconfig-free fallback
/// chain above is all there is.
#[cfg(not(target_os = "macos"))]
fn macos_coretext_rasterize(_buf: &mut [u8], _cw: u32, _ch_px: u32, _baseline: i32, _px: f32, _ch: char) -> bool {
    false
}

#[cfg(target_os = "macos")]
unsafe fn cached_ct_base_font(is_emoji: bool, size: f64) -> *const std::ffi::c_void { unsafe {
    use std::ffi::c_void;
    use std::ptr;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCharacters(alloc: *const c_void, chars: *const u16, n: isize) -> *const c_void;
        fn CFRetain(cf: *const c_void) -> *const c_void;
        fn CFRelease(cf: *const c_void);
    }
    #[link(name = "CoreText", kind = "framework")]
    unsafe extern "C" {
        fn CTFontCreateWithName(name: *const c_void, size: f64, matrix: *const c_void) -> *const c_void;
    }

    static BASE_FONT_CACHE: Mutex<[Option<(u64, usize)>; 2]> = parking_lot::const_mutex([None, None]);
    let slot_idx = usize::from(is_emoji);
    let size_bits = size.to_bits();
    {
        let cache = BASE_FONT_CACHE.lock();
        if let Some((cached_bits, ptr_val)) = cache[slot_idx]
            && cached_bits == size_bits && ptr_val != 0 {
                return CFRetain(ptr_val as *const c_void);
            }
    }

    let name_str = if is_emoji { "Apple Color Emoji" } else { "Menlo" };
    let units: Vec<u16> = name_str.encode_utf16().collect();
    let cf_name = CFStringCreateWithCharacters(ptr::null(), units.as_ptr(), units.len() as isize);
    if cf_name.is_null() {
        return ptr::null();
    }
    let font = CTFontCreateWithName(cf_name, size, ptr::null());
    CFRelease(cf_name);
    if font.is_null() {
        return ptr::null();
    }

    let mut cache = BASE_FONT_CACHE.lock();
    if let Some((_, old_ptr)) = cache[slot_idx].replace((size_bits, font as usize))
        && old_ptr != 0 {
            CFRelease(old_ptr as *const c_void);
        }
    CFRetain(font)
}}

#[cfg(target_os = "macos")]
pub(crate) fn macos_coretext_rasterize(
    buf: &mut [u8],
    cw: u32,
    ch_px: u32,
    baseline: i32,
    px: f32,
    ch: char,
) -> bool {
    use std::ffi::c_void;
    use std::ptr;

    #[repr(C)]
    #[derive(Copy, Clone)]
    struct CFRange {
        location: isize,
        length: isize,
    }

    #[repr(C)]
    #[derive(Copy, Clone, Default)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Copy, Clone, Default)]
    struct CGSize {
        width: f64,
        height: f64,
    }

    #[repr(C)]
    #[derive(Copy, Clone, Default)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCharacters(
            alloc: *const c_void,
            chars: *const u16,
            num_chars: isize,
        ) -> *const c_void;
        fn CFStringGetLength(the_string: *const c_void) -> isize;
        fn CFStringGetCharacters(the_string: *const c_void, range: CFRange, buffer: *mut u16);
        fn CFRelease(cf: *const c_void);
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGColorSpaceCreateDeviceGray() -> *mut c_void;
        fn CGColorSpaceRelease(space: *mut c_void);
        fn CGBitmapContextCreate(
            data: *mut c_void,
            width: usize,
            height: usize,
            bits_per_component: usize,
            bytes_per_row: usize,
            space: *mut c_void,
            bitmap_info: u32,
        ) -> *mut c_void;
        fn CGContextRelease(c: *mut c_void);
        fn CGContextSetGrayFillColor(c: *mut c_void, gray: f64, alpha: f64);
        fn CGContextSetAllowsAntialiasing(c: *mut c_void, allows: bool);
        fn CGContextSetShouldAntialias(c: *mut c_void, should: bool);
        fn CGContextSetAllowsFontSmoothing(c: *mut c_void, allows: bool);
        fn CGContextSetShouldSmoothFonts(c: *mut c_void, should: bool);
    }

    #[link(name = "CoreText", kind = "framework")]
    unsafe extern "C" {
        fn CTFontCreateWithName(
            name: *const c_void,
            size: f64,
            matrix: *const c_void,
        ) -> *const c_void;
        fn CTFontCreateForString(
            current_font: *const c_void,
            string: *const c_void,
            range: CFRange,
        ) -> *const c_void;
        fn CTFontCopyFamilyName(font: *const c_void) -> *const c_void;
        fn CTFontGetGlyphsForCharacters(
            font: *const c_void,
            characters: *const u16,
            glyphs: *mut u16,
            count: isize,
        ) -> bool;
        fn CTFontGetBoundingRectsForGlyphs(
            font: *const c_void,
            orientation: u32,
            glyphs: *const u16,
            bounding_rects: *mut CGRect,
            count: isize,
        ) -> CGRect;
        fn CTFontDrawGlyphs(
            font: *const c_void,
            glyphs: *const u16,
            positions: *const CGPoint,
            count: usize,
            context: *mut c_void,
        );
    }

    unsafe {
        let mut u16_buf = [0u16; 2];
        let utf16 = ch.encode_utf16(&mut u16_buf);
        let cf_str = CFStringCreateWithCharacters(ptr::null(), utf16.as_ptr(), utf16.len() as isize);
        if cf_str.is_null() {
            return false;
        }

        let base_font = cached_ct_base_font(false, px as f64);
        if base_font.is_null() {
            CFRelease(cf_str);
            return false;
        }

        let fb_font = CTFontCreateForString(
            base_font,
            cf_str,
            CFRange { location: 0, length: utf16.len() as isize },
        );
        CFRelease(base_font);
        CFRelease(cf_str);
        if fb_font.is_null() {
            return false;
        }

        // Ignore macOS ".LastResort" fallback font so we never draw LastResort boxes.
        let fam = CTFontCopyFamilyName(fb_font);
        if !fam.is_null() {
            let len = CFStringGetLength(fam);
            let mut fbuf = [0u16; 32];
            let n = len.min(32);
            CFStringGetCharacters(fam, CFRange { location: 0, length: n }, fbuf.as_mut_ptr());
            CFRelease(fam);
            let fam_str = String::from_utf16_lossy(&fbuf[..n as usize]);
            if fam_str.contains("LastResort") {
                CFRelease(fb_font);
                return false;
            }
        }

        let mut glyphs = [0u16; 2];
        let ok = CTFontGetGlyphsForCharacters(
            fb_font,
            utf16.as_ptr(),
            glyphs.as_mut_ptr(),
            utf16.len() as isize,
        );
        if !ok || glyphs[0] == 0 {
            CFRelease(fb_font);
            return false;
        }

        let mut bbox = CGRect::default();
        CTFontGetBoundingRectsForGlyphs(fb_font, 0, &glyphs[0], &mut bbox, 1);

        // If the fallback glyph is wider than a single cell, create a scaled font so it fits.
        let mut draw_font = fb_font;
        if bbox.size.width > cw as f64 * 1.02 && bbox.size.width > 1.0 {
            let scale = (cw as f64 / bbox.size.width) * 0.92;
            let fam = CTFontCopyFamilyName(fb_font);
            if !fam.is_null() {
                let scaled = CTFontCreateWithName(fam, px as f64 * scale, ptr::null());
                CFRelease(fam);
                if !scaled.is_null() {
                    let mut g2 = [0u16; 2];
                    if CTFontGetGlyphsForCharacters(
                        scaled,
                        utf16.as_ptr(),
                        g2.as_mut_ptr(),
                        utf16.len() as isize,
                    ) && g2[0] != 0
                    {
                        CFRelease(fb_font);
                        draw_font = scaled;
                        glyphs[0] = g2[0];
                        CTFontGetBoundingRectsForGlyphs(draw_font, 0, &glyphs[0], &mut bbox, 1);
                    } else {
                        CFRelease(scaled);
                    }
                }
            }
        }

        let cs = CGColorSpaceCreateDeviceGray();
        let ctx = CGBitmapContextCreate(
            buf.as_mut_ptr() as *mut c_void,
            cw as usize,
            ch_px as usize,
            8,
            cw as usize,
            cs,
            0, // kCGImageAlphaNone
        );
        CGColorSpaceRelease(cs);
        if ctx.is_null() {
            CFRelease(draw_font);
            return false;
        }

        CGContextSetAllowsAntialiasing(ctx, true);
        CGContextSetShouldAntialias(ctx, true);
        CGContextSetAllowsFontSmoothing(ctx, true);
        CGContextSetShouldSmoothFonts(ctx, true);
        CGContextSetGrayFillColor(ctx, 1.0, 1.0);

        let gx = ((cw as f64 - bbox.size.width) * 0.5 - bbox.origin.x).clamp(0.0, cw as f64);
        let gy = (ch_px as i32 - baseline) as f64;
        let pos = CGPoint { x: gx, y: gy };
        CTFontDrawGlyphs(draw_font, &glyphs[0], &pos, 1, ctx);

        CGContextRelease(ctx);
        CFRelease(draw_font);
        true
    }
}

// ============================================================================
// Color emoji
// ============================================================================

/// Codepoints drawn from the color emoji font when they are double-width
/// (emoji presentation): pictographs, symbols, flags.
fn is_emoji(cp: u32) -> bool {
    matches!(cp, 0x231A..=0x23FF | 0x2600..=0x27BF | 0x2B00..=0x2BFF | 0x1F000..=0x1FAFF)
}

/// A double-width cluster that is an emoji: an emoji base, or emoji
/// presentation (VS16, ZWJ, keycap) unless VS15 asks for text.
fn is_emoji_text(text: &str) -> bool {
    let first = text.chars().next().map_or(0, |c| c as u32);
    !text.contains('\u{FE0E}') && (is_emoji(first) || text.contains(['\u{FE0F}', '\u{200D}', '\u{20E3}']))
}

/// Rasterize `text` (one emoji, possibly a ZWJ / flag / skin tone sequence)
/// from the color emoji font into `buf`, a zeroed `w × h` premultiplied RGBA
/// canvas, scaled to fit and centered.
#[cfg(target_os = "macos")]
fn rasterize_color(buf: &mut [u8], w: u32, h: u32, px: f32, text: &str) -> bool {
    coretext_line(buf, w, h, px, text, None)
}

/// A grapheme cluster as text (an Indic conjunct, a base with marks) through
/// CoreText's shaping and font cascade into a `w × h` coverage canvas, on the
/// baseline; for clusters the primary font cannot shape.
#[cfg(target_os = "macos")]
fn rasterize_text_cluster(buf: &mut [u8], w: u32, h: u32, baseline: i32, px: f32, text: &str) -> bool {
    coretext_line(buf, w, h, px, text, Some(baseline))
}

#[cfg(not(target_os = "macos"))]
fn rasterize_text_cluster(_buf: &mut [u8], _w: u32, _h: u32, _baseline: i32, _px: f32, _text: &str) -> bool {
    false
}

/// Draw `text` as one CoreText line: color emoji (RGBA, scaled to fit and
/// centered) when `baseline` is `None`, else text coverage (one byte per px)
/// from Menlo and the system cascade, centered horizontally on the baseline.
#[cfg(target_os = "macos")]
fn coretext_line(buf: &mut [u8], w: u32, h: u32, px: f32, text: &str, baseline: Option<i32>) -> bool {
    use std::ffi::c_void;
    use std::ptr;

    #[repr(C)]
    #[derive(Copy, Clone, Default)]
    struct CGPoint {
        x: f64,
        y: f64,
    }
    #[repr(C)]
    #[derive(Copy, Clone, Default)]
    struct CGSize {
        width: f64,
        height: f64,
    }
    #[repr(C)]
    #[derive(Copy, Clone, Default)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCharacters(alloc: *const c_void, chars: *const u16, n: isize) -> *const c_void;
        fn CFDictionaryCreate(
            alloc: *const c_void,
            keys: *const *const c_void,
            values: *const *const c_void,
            n: isize,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> *const c_void;
        fn CFAttributedStringCreate(alloc: *const c_void, s: *const c_void, attributes: *const c_void) -> *const c_void;
        fn CFRelease(cf: *const c_void);
        static kCFTypeDictionaryKeyCallBacks: [usize; 6];
        static kCFTypeDictionaryValueCallBacks: [usize; 5];
        static kCFBooleanTrue: *const c_void;
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGColorSpaceCreateDeviceRGB() -> *mut c_void;
        fn CGColorSpaceCreateDeviceGray() -> *mut c_void;
        fn CGContextSetGrayFillColor(c: *mut c_void, gray: f64, alpha: f64);
        fn CGColorSpaceRelease(space: *mut c_void);
        fn CGBitmapContextCreate(
            data: *mut c_void,
            width: usize,
            height: usize,
            bits_per_component: usize,
            bytes_per_row: usize,
            space: *mut c_void,
            bitmap_info: u32,
        ) -> *mut c_void;
        fn CGContextRelease(c: *mut c_void);
        fn CGContextSetTextPosition(c: *mut c_void, x: f64, y: f64);
    }
    #[link(name = "CoreText", kind = "framework")]
    unsafe extern "C" {
        fn CTLineCreateWithAttributedString(s: *const c_void) -> *const c_void;
        fn CTLineGetBoundsWithOptions(line: *const c_void, options: usize) -> CGRect;
        fn CTLineDraw(line: *const c_void, ctx: *mut c_void);
        static kCTFontAttributeName: *const c_void;
        static kCTForegroundColorFromContextAttributeName: *const c_void;
    }
    /// `kCGImageAlphaPremultipliedLast`: RGBA bytes.
    const PREMULTIPLIED_LAST: u32 = 1;
    /// `kCTLineBoundsUseGlyphPathBounds`: the ink, not the typographic box.
    const GLYPH_PATH_BOUNDS: usize = 1 << 3;

    let units: Vec<u16> = text.encode_utf16().collect();
    let is_emoji_font = baseline.is_none();
    // SAFETY: every CoreFoundation object created here is released; the
    // context draws into `buf`, which outlives it and is `w * h * 4` bytes.
    unsafe {
        let cf_text = CFStringCreateWithCharacters(ptr::null(), units.as_ptr(), units.len() as isize);
        // A whole sequence shaped as one line, so ZWJ / flag / skin tone ligatures apply.
        let line_at = |size: f64| -> *const c_void {
            let font = cached_ct_base_font(is_emoji_font, size);
            if font.is_null() {
                return ptr::null();
            }
            // Text is drawn in the context's fill color (white coverage).
            let keys = [kCTFontAttributeName, kCTForegroundColorFromContextAttributeName];
            let values = [font, kCFBooleanTrue];
            let attrs = CFDictionaryCreate(
                ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1 + baseline.is_some() as isize,
                &kCFTypeDictionaryKeyCallBacks as *const _ as *const c_void,
                &kCFTypeDictionaryValueCallBacks as *const _ as *const c_void,
            );
            CFRelease(font);
            if attrs.is_null() {
                return ptr::null();
            }
            let attributed = CFAttributedStringCreate(ptr::null(), cf_text, attrs);
            CFRelease(attrs);
            if attributed.is_null() {
                return ptr::null();
            }
            let line = CTLineCreateWithAttributedString(attributed);
            CFRelease(attributed);
            line
        };
        let mut line = if cf_text.is_null() { ptr::null() } else { line_at(px as f64) };
        if !line.is_null() {
            let b = CTLineGetBoundsWithOptions(line, GLYPH_PATH_BOUNDS);
            // Shrink to fit the cells (emoji are wider and taller than text at the same size).
            let scale = match baseline {
                None => (w as f64 / b.size.width.max(1.0)).min(h as f64 / b.size.height.max(1.0)) * 0.95,
                Some(_) => w as f64 / b.size.width.max(1.0),
            };
            if scale < 1.0 {
                let smaller = line_at(px as f64 * scale);
                if !smaller.is_null() {
                    CFRelease(line);
                    line = smaller;
                }
            }
        }
        if !cf_text.is_null() {
            CFRelease(cf_text);
        }
        if line.is_null() {
            return false;
        }
        let b = CTLineGetBoundsWithOptions(line, GLYPH_PATH_BOUNDS);
        let (cs, bpp, info) = match baseline {
            None => (CGColorSpaceCreateDeviceRGB(), 4, PREMULTIPLIED_LAST),
            Some(_) => (CGColorSpaceCreateDeviceGray(), 1, 0),
        };
        let ctx = CGBitmapContextCreate(buf.as_mut_ptr() as *mut c_void, w as usize, h as usize, 8, w as usize * bpp, cs, info);
        CGColorSpaceRelease(cs);
        if ctx.is_null() {
            CFRelease(line);
            return false;
        }
        CGContextSetGrayFillColor(ctx, 1.0, 1.0);
        // Centered in the canvas (CoreGraphics has its origin at the bottom left),
        // text on the cell's baseline.
        let x = (w as f64 - b.size.width) / 2.0 - b.origin.x;
        let y = match baseline {
            None => (h as f64 - b.size.height) / 2.0 - b.origin.y,
            Some(base) => (h as i32 - base) as f64,
        };
        CGContextSetTextPosition(ctx, x, y);
        CTLineDraw(line, ctx);
        CGContextRelease(ctx);
        CFRelease(line);
    }
    match baseline {
        None => buf.as_chunks::<4>().0.iter().any(|p| p[3] != 0),
        Some(_) => buf.iter().any(|&v| v != 0),
    }
}

/// Bitmap color emoji (CBDT / sbix PNG strikes, e.g. Noto Color Emoji) on
/// other systems; vector-only (COLR) emoji fonts fall back to monochrome.
#[cfg(not(target_os = "macos"))]
fn rasterize_color(buf: &mut [u8], w: u32, h: u32, _px: f32, text: &str) -> bool {
    use rustybuzz::ttf_parser::{GlyphId, RasterImageFormat};
    use std::sync::OnceLock;

    static FACE: OnceLock<Option<(Arc<memmap2::Mmap>, u32)>> = OnceLock::new();
    let pair = FACE.get_or_init(|| {
        let face = crate::fontdb::find_family("Noto Color Emoji")?;
        shaper_mmap_for(&face.path, face.index)
    });
    let Some((mmap, index)) = pair.as_ref() else { return false };
    let Some(face) = rustybuzz::Face::from_slice(mmap, *index) else { return false };
    // Shaped, so ZWJ / flag / skin tone sequences become their ligature glyph.
    let mut ub = rustybuzz::UnicodeBuffer::new();
    ub.push_str(text);
    let shaped = rustybuzz::shape(&face, &[], ub);
    let Some(gid) = shaped.glyph_infos().first().map(|g| g.glyph_id).filter(|&g| g != 0) else { return false };
    let Some(img) = face.glyph_raster_image(GlyphId(gid as u16), h.min(u16::MAX as u32) as u16) else { return false };
    if img.format != RasterImageFormat::PNG {
        return false;
    }
    let Ok((sw, sh, rgba)) = crate::graphics::decode_png(img.data) else { return false };
    blit_fit(&rgba, sw, sh, buf, w, h);
    true
}

/// Scale straight-alpha RGBA `src` to fit `dst` (aspect kept, centered, area
/// averaged), writing premultiplied RGBA.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn blit_fit(src: &[u8], sw: u32, sh: u32, dst: &mut [u8], dw: u32, dh: u32) {
    let scale = (dw as f32 / sw as f32).min(dh as f32 / sh as f32) * 0.95;
    let (tw, th) = ((sw as f32 * scale).round().max(1.0) as u32, (sh as f32 * scale).round().max(1.0) as u32);
    let (ox, oy) = ((dw.saturating_sub(tw)) / 2, (dh.saturating_sub(th)) / 2);
    for ty in 0..th.min(dh) {
        let (y0, y1) = ((ty as f32 / scale) as u32, (((ty + 1) as f32 / scale) as u32).clamp(1, sh));
        for tx in 0..tw.min(dw) {
            let (x0, x1) = ((tx as f32 / scale) as u32, (((tx + 1) as f32 / scale) as u32).clamp(1, sw));
            let mut acc = [0u32; 4];
            let mut n = 0;
            for y in y0..y1.max(y0 + 1).min(sh) {
                for x in x0..x1.max(x0 + 1).min(sw) {
                    let p = &src[((y * sw + x) * 4) as usize..][..4];
                    let a = p[3] as u32;
                    acc[0] += p[0] as u32 * a / 255;
                    acc[1] += p[1] as u32 * a / 255;
                    acc[2] += p[2] as u32 * a / 255;
                    acc[3] += a;
                    n += 1;
                }
            }
            let n = n.max(1);
            let d = &mut dst[(((oy + ty) * dw + ox + tx) * 4) as usize..][..4];
            for (d, a) in d.iter_mut().zip(acc) {
                *d = (a / n) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests;
