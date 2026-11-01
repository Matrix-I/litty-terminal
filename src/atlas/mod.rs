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

#[cfg(target_os = "macos")]
mod coretext;
mod fonts;
#[cfg(not(target_os = "macos"))]
mod linux;
mod raster;
mod shaper;
mod sprites;
pub use fonts::load_font;
use fonts::FONT_CACHE;
pub use shaper::{parse_font_feature, set_font_features};
use shaper::{Shaper, FONT_FEATURES};
use sprites::is_builtin_sprite;

/// The platform's system rasterizers (CoreText on macOS, bitmap emoji only elsewhere).
#[cfg(target_os = "macos")]
use coretext as system;
#[cfg(not(target_os = "macos"))]
use linux as system;

/// Called by the CoreText benchmark in `app::latency_benchmarks`.
#[cfg(all(test, target_os = "macos"))]
pub(crate) use coretext::macos_coretext_rasterize;

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
