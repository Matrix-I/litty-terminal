//! Image store: decoded images, regular and virtual placements, and the
//! memory and object-count bounds that evict the oldest of them. Scrolling
//! moves placements here; the GPU never sees anything but [`ImageDraw`].

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use super::animation::Image;
use super::kitty::KittyCommand;
use super::{MAX_IMAGES, MAX_PLACEMENTS, MAX_VIRTUALS, STORE_LIMIT};

/// One displayed copy of an image.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    pub image_id: u32,
    pub placement_id: u32,
    /// Absolute line and column of the top-left cell.
    pub line: usize,
    pub col: usize,
    /// Cells covered.
    pub cols: usize,
    pub rows: usize,
    /// Source rectangle in image pixels: x, y, width, height.
    pub src: [u32; 4],
    pub z: i32,
    /// Shown on the alternate screen (cleared when it is left).
    pub alt: bool,
    /// Stretch to `cols` x `rows` cells (given explicitly) instead of drawing
    /// at the image's own pixel size.
    pub fit: bool,
}

/// What the renderer needs to draw one placement.
pub struct ImageDraw {
    pub key: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
    /// Source rectangle in image pixels: x, y, width, height.
    pub src: [f32; 4],
    /// Top-left cell relative to the visible screen (may be negative: partly
    /// scrolled off the top).
    pub row: isize,
    pub col: usize,
    pub cols: usize,
    pub rows: usize,
    pub fit: bool,
    pub z: i32,
    /// Exact destination in px relative to the top-left cell (x, y, width,
    /// height), overriding `fit` (unicode placeholders cover parts of cells).
    pub px: Option<[f32; 4]>,
}

/// A Kitty virtual placement (`U=1`): drawn wherever the text holds U+10EEEE
/// placeholder cells naming its image (and placement) id.
#[derive(Clone, Debug, PartialEq)]
pub struct VirtualPlacement {
    pub image_id: u32,
    pub placement_id: u32,
    /// Size of the box the image is fitted into (keeping its aspect ratio), in cells.
    pub cols: usize,
    pub rows: usize,
    pub z: i32,
    pub alt: bool,
}

/// Combining marks that number the rows / columns of unicode placeholder
/// cells (index = value), from Kitty's `rowcolumn-diacritics.txt`.
pub const PLACEHOLDER_DIACRITICS: [u32; 297] = [
    0x0305, 0x030D, 0x030E, 0x0310, 0x0312, 0x033D, 0x033E, 0x033F, 0x0346, 0x034A, 0x034B, 0x034C,
    0x0350, 0x0351, 0x0352, 0x0357, 0x035B, 0x0363, 0x0364, 0x0365, 0x0366, 0x0367, 0x0368, 0x0369,
    0x036A, 0x036B, 0x036C, 0x036D, 0x036E, 0x036F, 0x0483, 0x0484, 0x0485, 0x0486, 0x0487, 0x0592,
    0x0593, 0x0594, 0x0595, 0x0597, 0x0598, 0x0599, 0x059C, 0x059D, 0x059E, 0x059F, 0x05A0, 0x05A1,
    0x05A8, 0x05A9, 0x05AB, 0x05AC, 0x05AF, 0x05C4, 0x0610, 0x0611, 0x0612, 0x0613, 0x0614, 0x0615,
    0x0616, 0x0617, 0x0657, 0x0658, 0x0659, 0x065A, 0x065B, 0x065D, 0x065E, 0x06D6, 0x06D7, 0x06D8,
    0x06D9, 0x06DA, 0x06DB, 0x06DC, 0x06DF, 0x06E0, 0x06E1, 0x06E2, 0x06E4, 0x06E7, 0x06E8, 0x06EB,
    0x06EC, 0x0730, 0x0732, 0x0733, 0x0735, 0x0736, 0x073A, 0x073D, 0x073F, 0x0740, 0x0741, 0x0743,
    0x0745, 0x0747, 0x0749, 0x074A, 0x07EB, 0x07EC, 0x07ED, 0x07EE, 0x07EF, 0x07F0, 0x07F1, 0x07F3,
    0x0816, 0x0817, 0x0818, 0x0819, 0x081B, 0x081C, 0x081D, 0x081E, 0x081F, 0x0820, 0x0821, 0x0822,
    0x0823, 0x0825, 0x0826, 0x0827, 0x0829, 0x082A, 0x082B, 0x082C, 0x082D, 0x0951, 0x0953, 0x0954,
    0x0F82, 0x0F83, 0x0F86, 0x0F87, 0x135D, 0x135E, 0x135F, 0x17DD, 0x193A, 0x1A17, 0x1A75, 0x1A76,
    0x1A77, 0x1A78, 0x1A79, 0x1A7A, 0x1A7B, 0x1A7C, 0x1B6B, 0x1B6D, 0x1B6E, 0x1B6F, 0x1B70, 0x1B71,
    0x1B72, 0x1B73, 0x1CD0, 0x1CD1, 0x1CD2, 0x1CDA, 0x1CDB, 0x1CE0, 0x1DC0, 0x1DC1, 0x1DC3, 0x1DC4,
    0x1DC5, 0x1DC6, 0x1DC7, 0x1DC8, 0x1DC9, 0x1DCB, 0x1DCC, 0x1DD1, 0x1DD2, 0x1DD3, 0x1DD4, 0x1DD5,
    0x1DD6, 0x1DD7, 0x1DD8, 0x1DD9, 0x1DDA, 0x1DDB, 0x1DDC, 0x1DDD, 0x1DDE, 0x1DDF, 0x1DE0, 0x1DE1,
    0x1DE2, 0x1DE3, 0x1DE4, 0x1DE5, 0x1DE6, 0x1DFE, 0x20D0, 0x20D1, 0x20D4, 0x20D5, 0x20D6, 0x20D7,
    0x20DB, 0x20DC, 0x20E1, 0x20E7, 0x20E9, 0x20F0, 0x2CEF, 0x2CF0, 0x2CF1, 0x2DE0, 0x2DE1, 0x2DE2,
    0x2DE3, 0x2DE4, 0x2DE5, 0x2DE6, 0x2DE7, 0x2DE8, 0x2DE9, 0x2DEA, 0x2DEB, 0x2DEC, 0x2DED, 0x2DEE,
    0x2DEF, 0x2DF0, 0x2DF1, 0x2DF2, 0x2DF3, 0x2DF4, 0x2DF5, 0x2DF6, 0x2DF7, 0x2DF8, 0x2DF9, 0x2DFA,
    0x2DFB, 0x2DFC, 0x2DFD, 0x2DFE, 0x2DFF, 0xA66F, 0xA67C, 0xA67D, 0xA6F0, 0xA6F1, 0xA8E0, 0xA8E1,
    0xA8E2, 0xA8E3, 0xA8E4, 0xA8E5, 0xA8E6, 0xA8E7, 0xA8E8, 0xA8E9, 0xA8EA, 0xA8EB, 0xA8EC, 0xA8ED,
    0xA8EE, 0xA8EF, 0xA8F0, 0xA8F1, 0xAAB0, 0xAAB2, 0xAAB3, 0xAAB7, 0xAAB8, 0xAABE, 0xAABF, 0xAAC1,
    0xFE20, 0xFE21, 0xFE22, 0xFE23, 0xFE24, 0xFE25, 0xFE26, 0x10A0F, 0x10A38, 0x1D185, 0x1D186,
    0x1D187, 0x1D188, 0x1D189, 0x1D1AA, 0x1D1AB, 0x1D1AC, 0x1D1AD, 0x1D242, 0x1D243, 0x1D244,
];

/// Kitty: placements with a z-index below this are drawn under cells with a
/// non-default background too, not only under the text.
pub const Z_BELOW_BG: i32 = -1_073_741_824;

#[derive(Default)]
pub struct Store {
    pub(super) images: HashMap<u32, Image>,
    /// Insertion order of image ids, for eviction.
    pub(super) order: Vec<u32>,
    pub placements: Vec<Placement>,
    pub virtuals: Vec<VirtualPlacement>,
    pub(super) bytes: usize,
    /// Ids handed out for images transmitted without one (sixel, kitty `I=`).
    pub(super) next_auto_id: u32,
    /// A chunked Kitty transmission in progress (`m=1`), with last activity timestamp.
    pub(super) pending: Option<(KittyCommand, Vec<u8>, Instant)>,
}

impl Store {
    pub fn clear(&mut self) {
        *self = Store { next_auto_id: self.next_auto_id, ..Store::default() };
    }

    /// Store an image under `id` (0 = pick one), replacing an older one.
    pub fn insert(&mut self, id: u32, width: u32, height: u32, rgba: Vec<u8>) -> u32 {
        let is_auto = id == 0;
        let id = if is_auto { self.auto_id() } else { id };
        self.remove_image(id, false);
        // Reject a single root frame above the store limit up front without
        // wiping existing images or converting `Vec<u8>` into `Arc<[u8]>`.
        if rgba.len() > STORE_LIMIT {
            return id;
        }
        self.bytes += rgba.len();
        let mut img = Image::new(width, height, rgba);
        img.auto = is_auto;
        self.images.insert(id, img);
        self.order.push(id);
        self.evict();
        id
    }

    /// Enforce both pixel bytes and object count, including transmit-only
    /// images. Removing an image also releases its frames and placements.
    pub(super) fn evict(&mut self) {
        while (self.bytes > STORE_LIMIT || self.images.len() > MAX_IMAGES) && !self.order.is_empty() {
            let oldest = self.order[0];
            self.remove_image(oldest, true);
        }
    }

    /// When an animation shown in lines `top..top + rows` (or through
    /// placeholders) next changes frame.
    pub fn next_frame_at(&self, now: Instant, top: usize, rows: usize, alt: bool) -> Option<Instant> {
        let on_screen = |p: &Placement| p.alt == alt && p.line < top + rows && p.line + p.rows > top;
        let shown = |id: &u32| {
            self.placements.iter().any(|p| p.image_id == *id && on_screen(p)) || self.virtuals.iter().any(|v| v.image_id == *id)
        };
        self.images.iter().filter(|(id, img)| img.anim.started.is_some() && shown(id)).filter_map(|(_, img)| img.frame_at(now).1).min()
    }

    pub(super) fn auto_id(&mut self) -> u32 {
        loop {
            // Kitty reserves small ids for applications; count down from the top.
            if self.next_auto_id <= (1 << 24) {
                self.next_auto_id = u32::MAX;
            } else {
                self.next_auto_id -= 1;
            }
            if !self.images.contains_key(&self.next_auto_id) {
                return self.next_auto_id;
            }
        }
    }

    pub(super) fn remove_image(&mut self, id: u32, with_placements: bool) {
        if let Some(img) = self.images.remove(&id) {
            self.bytes -= img.bytes();
            self.order.retain(|&i| i != id);
        }
        if with_placements {
            self.placements.retain(|p| p.image_id != id);
            self.virtuals.retain(|v| v.image_id != id);
        }
    }

    pub fn image(&self, id: u32) -> Option<&Image> {
        self.images.get(&id)
    }

    /// Add a placement, replacing one with the same image and placement id.
    pub fn place(&mut self, p: Placement) {
        if p.placement_id != 0 {
            self.placements.retain(|q| !(q.image_id == p.image_id && q.placement_id == p.placement_id));
        }
        if self.placements.len() >= MAX_PLACEMENTS {
            // Drop the oldest in one batch so a flood stays amortized O(1).
            self.placements.drain(..MAX_PLACEMENTS / 8);
        }
        self.placements.push(p);
    }

    /// Add a virtual placement, replacing one with the same image and placement id.
    pub fn place_virtual(&mut self, v: VirtualPlacement) {
        self.virtuals.retain(|q| !(q.image_id == v.image_id && q.placement_id == v.placement_id));
        if self.virtuals.len() >= MAX_VIRTUALS {
            self.virtuals.drain(..MAX_VIRTUALS / 8);
        }
        self.virtuals.push(v);
    }

    /// The virtual placement placeholder cells with these ids refer to: the
    /// exact placement, else the newest one of the image.
    pub fn virtual_for(&self, image_id: u32, placement_id: u32) -> Option<&VirtualPlacement> {
        let mut of_image = self.virtuals.iter().rev().filter(|v| v.image_id == image_id);
        of_image.clone().find(|v| v.placement_id == placement_id).or_else(|| of_image.next())
    }

    /// Prune any auto-assigned images that have no remaining regular or
    /// virtual placements.
    pub fn prune_dead_auto_ids(&mut self) {
        let dead: Vec<u32> = self
            .images
            .iter()
            .filter(|(_, img)| img.auto)
            .map(|(&id, _)| id)
            .filter(|&id| !self.placements.iter().any(|p| p.image_id == id) && !self.virtuals.iter().any(|v| v.image_id == id))
            .collect();
        for id in dead {
            self.remove_image(id, false);
        }
    }

    /// Drop placements that scrolled out of the scrollback, and images no
    /// placement shows that were only ever meant for display.
    pub fn prune(&mut self, first_line: usize) {
        self.placements.retain(|p| p.alt || p.line + p.rows > first_line);
        self.prune_dead_auto_ids();
    }

    /// Placements visible in `top..top + rows` on the current screen.
    pub fn visible(&self, top: usize, rows: usize, alt: bool, now: Instant, out: &mut Vec<ImageDraw>) {
        for p in self.placements.iter().filter(|p| p.alt == alt) {
            if p.line >= top + rows || p.line + p.rows <= top {
                continue;
            }
            let Some(img) = self.images.get(&p.image_id) else { continue };
            let (key, rgba) = img.shown(now);
            out.push(ImageDraw {
                key,
                width: img.width,
                height: img.height,
                rgba: rgba.clone(),
                src: p.src.map(|v| v as f32),
                row: p.line as isize - top as isize,
                col: p.col,
                cols: p.cols,
                rows: p.rows,
                fit: p.fit,
                z: p.z,
                px: None,
            });
        }
    }

    /// Remove the placements covering any of `lines` (screen clears).
    pub fn delete_lines(&mut self, lines: std::ops::Range<usize>, alt: bool) {
        self.placements.retain(|p| p.alt != alt || p.line + p.rows <= lines.start || p.line >= lines.end);
        self.prune_dead_auto_ids();
    }

    /// Scroll alternate-screen placements in region `top_line..=bot_line` up by `n` lines.
    /// Placements scrolled out of the region are pruned, and auto-id images are freed.
    pub fn scroll_alt(&mut self, top_line: usize, bot_line: usize, n: usize) {
        if n == 0 {
            return;
        }
        self.placements.retain_mut(|p| {
            if !p.alt {
                return true;
            }
            let p_end = p.line + p.rows;
            if p.line > bot_line || p_end <= top_line {
                return true;
            }
            if p.line < top_line + n {
                if p_end <= top_line + n {
                    // Completely scrolled off the top of the region
                    return false;
                }
                let dy = (top_line + n) - p.line;
                let cell_h = p.src[3] as f32 / p.rows as f32;
                let clipped_px = (dy as f32 * cell_h).round() as u32;
                p.src[1] += clipped_px;
                p.src[3] = p.src[3].saturating_sub(clipped_px);
                p.rows -= dy;
                p.line = top_line;
                p.rows > 0 && p.src[3] > 0
            } else {
                p.line -= n;
                true
            }
        });
        self.prune_dead_auto_ids();
    }

    /// Scroll alternate-screen placements in region `top_line..=bot_line` down by `n` lines.
    pub fn scroll_alt_down(&mut self, top_line: usize, bot_line: usize, n: usize) {
        if n == 0 {
            return;
        }
        self.placements.retain_mut(|p| {
            if !p.alt {
                return true;
            }
            let p_end = p.line + p.rows;
            if p.line > bot_line || p_end <= top_line {
                return true;
            }
            if p.line + n > bot_line {
                // Scrolled entirely off the bottom
                return false;
            }
            p.line += n;
            if p.line + p.rows > bot_line + 1 {
                let overflow = (p.line + p.rows) - (bot_line + 1);
                let cell_h = p.src[3] as f32 / p.rows as f32;
                let clipped_px = (overflow as f32 * cell_h).round() as u32;
                p.src[3] = p.src[3].saturating_sub(clipped_px);
                p.rows -= overflow;
            }
            p.rows > 0 && p.src[3] > 0
        });
        self.prune_dead_auto_ids();
    }
}
