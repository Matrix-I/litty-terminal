//! Inline images: Kitty graphics APC dispatch, Sixel completion, placements,
//! Unicode placeholders (virtual placements) and per-frame visible draws.
//! Decoding runs off the grid lock (`ImageJob::decode`, see `pty::feed_output`).

use super::*;
use super::colors::palette_with;

/// A single image operation, completed by the PTY reader before parsing more
/// bytes. No grid reference is retained while decoding or accessing files.
pub(crate) enum ImageJob {
    Sixel(Vec<u8>, bool),
    Kitty(crate::terminal::graphics::KittyCommand, Vec<u8>),
}

pub(crate) enum ImageCompletion {
    Sixel(Option<(u32, u32, Vec<u8>)>),
    Kitty(crate::terminal::graphics::KittyCommand, crate::terminal::graphics::DecodedImage),
}

impl ImageJob {
    pub(crate) fn decode(self) -> ImageCompletion {
        match self {
            Self::Sixel(data, transparent) => ImageCompletion::Sixel(crate::terminal::graphics::decode_sixel(&data, transparent)),
            Self::Kitty(cmd, data) => {
                let decoded = crate::terminal::graphics::decode_kitty(&cmd, &data);
                ImageCompletion::Kitty(cmd, decoded)
            }
        }
    }
}

impl Grid {
    pub(super) fn apc_dispatch(&mut self) {
        let body = std::mem::take(&mut self.apc_buf);
        if let Some((cmd, payload)) = crate::terminal::graphics::parse_kitty(&body) {
            match self.graphics.prepare_kitty(cmd.clone(), payload) {
                crate::terminal::graphics::KittyPreparation::Complete(outcome, reply) => self.apply_kitty(&cmd, outcome, reply),
                crate::terminal::graphics::KittyPreparation::Decode(cmd, data) => self.queue_image(ImageJob::Kitty(cmd, data)),
            }
        }
        // Keep the buffer's capacity only for ordinary-sized chunks.
        self.apc_buf = if body.capacity() > 65536 { Vec::new() } else { body };
    }

    pub(super) fn queue_image(&mut self, job: ImageJob) {
        if self.defer_images {
            debug_assert!(self.image_job.is_none());
            self.image_job = Some(job);
        } else {
            self.complete_image(job.decode());
        }
    }

    pub(crate) fn complete_image(&mut self, result: ImageCompletion) {
        match result {
            ImageCompletion::Sixel(Some((w, h, rgba))) => {
                let id = self.graphics.insert(0, w, h, rgba);
                self.place_image(id, [0, 0, w, h], 0, 0, 0, 0, false, true);
            }
            ImageCompletion::Sixel(None) => {},
            ImageCompletion::Kitty(cmd, decoded) => {
                let (outcome, reply) = self.graphics.finish_kitty(cmd.clone(), decoded);
                self.apply_kitty(&cmd, outcome, reply);
            }
        }
    }

    pub(super) fn apply_kitty(&mut self, cmd: &crate::terminal::graphics::KittyCommand, outcome: crate::terminal::graphics::KittyOutcome, reply: Option<String>) {
        if let crate::terminal::graphics::KittyOutcome::Place { id } = outcome {
            if cmd.unicode {
                self.place_virtual(id, cmd.cols, cmd.rows, cmd.z, cmd.placement_id);
            } else {
                self.place_image(id, [cmd.x, cmd.y, cmd.src_w, cmd.src_h], cmd.cols, cmd.rows, cmd.z, cmd.placement_id, cmd.no_cursor_move, false);
            }
        }
        if let Some(r) = reply {
            self.response.extend_from_slice(b"\x1b_G");
            self.response.extend_from_slice(r.as_bytes());
            self.response.extend_from_slice(b"\x1b\\");
        }
    }

    /// Show image `id` at the cursor. `src` is x, y, width, height in image
    /// pixels (0 = to the edge); `cols` / `rows` 0 = the image's own size in
    /// cells. Kitty moves the cursor to after the image's last column on its
    /// last row; Sixel (`sixel`) to the line below it, same column.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn place_image(&mut self, id: u32, src: [u32; 4], cols: u32, rows: u32, z: i32, placement_id: u32, no_move: bool, sixel: bool) {
        let Some(img) = self.graphics.image(id) else { return };
        let (iw, ih) = (img.width, img.height);
        let x = src[0].min(iw);
        let y = src[1].min(ih);
        let w = if src[2] == 0 { iw - x } else { src[2].min(iw - x) };
        let h = if src[3] == 0 { ih - y } else { src[3].min(ih - y) };
        if w == 0 || h == 0 {
            return;
        }
        let (cw, ch) = self.image_cell_px();
        let fit = cols > 0 || rows > 0;
        let cols = if cols > 0 { cols } else { w.div_ceil(cw) }.clamp(1, 1000) as usize;
        let rows = if rows > 0 { rows } else { h.div_ceil(ch) }.clamp(1, 1000) as usize;
        let start_col = self.cur.col;
        let line = self.lines_pushed + self.cur.row;
        self.graphics.place(crate::terminal::graphics::Placement {
            image_id: id,
            placement_id,
            line,
            col: start_col,
            cols,
            rows,
            src: [x, y, w, h],
            z,
            alt: self.alt_active,
            fit,
        });
        if !no_move {
            // Scrolling here keeps the image's absolute line right.
            let down = if sixel { rows } else { rows - 1 };
            for _ in 0..down {
                self.linefeed();
            }
            self.cur.col = if sixel { start_col } else { (start_col + cols).min(self.cols - 1) };
            self.wrap_pending = false;
        }
        self.graphics.prune(self.lines_pushed - self.history);
        self.all_dirty = true;
    }

    /// Cell size in px for image layout (a guess before the first resize reports it).
    pub(super) fn image_cell_px(&self) -> (u32, u32) {
        match self.cell_px {
            (0, _) | (_, 0) => (8, 16),
            (cw, ch) => (cw as u32, ch as u32),
        }
    }

    /// Kitty `U=1`: remember a virtual placement; placeholder cells show it.
    /// `cols` / `rows` 0 = the image's own size in cells.
    pub(super) fn place_virtual(&mut self, id: u32, cols: u32, rows: u32, z: i32, placement_id: u32) {
        let Some(img) = self.graphics.image(id) else { return };
        let (cw, ch) = self.image_cell_px();
        let cols = if cols > 0 { cols } else { img.width.div_ceil(cw) }.clamp(1, 1000) as usize;
        let rows = if rows > 0 { rows } else { img.height.div_ceil(ch) }.clamp(1, 1000) as usize;
        self.graphics.place_virtual(crate::terminal::graphics::VirtualPlacement {
            image_id: id,
            placement_id,
            cols,
            rows,
            z,
            alt: self.alt_active,
        });
        self.all_dirty = true;
    }

    /// Prune any virtual placements whose placeholder cells have been evicted
    /// from scrollback or overwritten, freeing any associated auto-id images.
    pub fn prune_dead_virtuals(&mut self) {
        if self.graphics.virtuals.is_empty() {
            return;
        }
        let mut referenced = std::collections::HashSet::new();
        // Check valid main screen and scrollback rows
        for r in 0..(self.history + self.rows) {
            let physical_row = (self.top + self.cap - self.history + r) % self.cap;
            let start = physical_row * self.cols;
            let end = start + self.cols;
            for cell in &self.cells[start..end] {
                if cell.codepoint & PH_FLAG != 0
                    && let Some((id, pid)) = self.placeholder_ids(cell) {
                        referenced.insert((id, pid));
                    }
            }
        }
        // Check alt screen if active
        if self.alt_active {
            for cell in &self.alt {
                if cell.codepoint & PH_FLAG != 0
                    && let Some((id, pid)) = self.placeholder_ids(cell) {
                        referenced.insert((id, pid));
                    }
            }
        }
        self.graphics.virtuals.retain(|v| {
            referenced.iter().any(|&(id, pid)| id == v.image_id && (pid == v.placement_id || pid == 0 || v.placement_id == 0))
        });
        self.graphics.prune_dead_auto_ids();
    }

    /// When an animated image on this terminal next changes frame.
    pub fn next_image_frame(&self, now: std::time::Instant) -> Option<std::time::Instant> {
        let top = if self.alt_active { self.lines_pushed } else { self.lines_pushed - self.display_offset };
        self.graphics.next_frame_at(now, top, self.rows, self.alt_active)
    }

    /// Images to draw on the visible screen (see [`crate::terminal::graphics::ImageDraw`]).
    pub fn visible_images(&self, out: &mut Vec<crate::terminal::graphics::ImageDraw>) {
        if !self.graphics.placements.is_empty() {
            let top = if self.alt_active { self.lines_pushed } else { self.lines_pushed - self.display_offset };
            self.graphics.visible(top, self.rows + usize::from(self.scroll_fraction > 0.0), self.alt_active, std::time::Instant::now(), out);
        }
        if !self.graphics.virtuals.is_empty() {
            for r in 0..self.rows + usize::from(self.scroll_fraction > 0.0) {
                self.placeholder_images(r, out);
            }
        }
    }

    /// The image of a placeholder cell: `(image id, placement id)`. The id is
    /// the text color (24-bit, or a 256-color index) plus the high byte from
    /// the third diacritic; the placement id is the underline color.
    pub(super) fn placeholder_ids(&self, cell: &Cell) -> Option<(u32, u32)> {
        if cell.fg & FG_SET == 0 {
            return None;
        }
        let rgb = cell.fg & !0xFF;
        let msb = if (cell.codepoint >> 27) & 3 == 3 { (cell.codepoint >> 18) & 0xFF } else { 0 };
        let pid = self.extra_at(cell.extra()).map_or(0, |e| e.underline_color >> 8);
        let id = (rgb >> 8) | msb << 24;
        if self.graphics.virtual_for(id, pid).is_some() {
            return Some((id, pid));
        }
        let index = (0..=255u8).find(|&i| palette_with(&self.palette_override, i) == rgb)? as u32;
        let id = index | msb << 24;
        self.graphics.virtual_for(id, pid).map(|_| (id, pid))
    }

    /// Image pieces for the placeholder cells of visible row `r`. A cell
    /// without a column diacritic continues the one to its left, and one
    /// without any diacritics also keeps its row.
    pub(super) fn placeholder_images(&self, r: usize, out: &mut Vec<crate::terminal::graphics::ImageDraw>) {
        // Run in progress: (image id, placement id, image row, image column, screen column, length).
        let mut run: Option<(u32, u32, u32, u32, usize, usize)> = None;
        let mut prev: Option<(u32, u32, u32, u32)> = None;
        for (c, cell) in self.display_row(r).iter().enumerate() {
            let cp = cell.codepoint;
            let ids = if cp & PH_FLAG != 0 { self.placeholder_ids(cell) } else { None };
            let Some((id, pid)) = ids else {
                if let Some(done) = run.take() {
                    self.placeholder_draw(r, done, out);
                }
                prev = None;
                continue;
            };
            let follows = |p: &(u32, u32, u32, u32)| p.0 == id && p.1 == pid;
            let (row, col) = match (cp >> 27) & 3 {
                0 => prev.filter(follows).map_or((0, 0), |p| (p.2, p.3 + 1)),
                1 => {
                    let row = (cp >> 9) & 0x1FF;
                    (row, prev.filter(|p| follows(p) && p.2 == row).map_or(0, |p| p.3 + 1))
                }
                _ => ((cp >> 9) & 0x1FF, cp & 0x1FF),
            };
            prev = Some((id, pid, row, col));
            match &mut run {
                Some(cur) if cur.0 == id && cur.1 == pid && cur.2 == row && cur.3 + cur.5 as u32 == col => cur.5 += 1,
                _ => {
                    if let Some(done) = run.replace((id, pid, row, col, c, 1)) {
                        self.placeholder_draw(r, done, out);
                    }
                }
            }
        }
        if let Some(done) = run {
            self.placeholder_draw(r, done, out);
        }
    }

    /// Draw a run of placeholder cells: the part of the image (fitted into the
    /// placement's box, aspect ratio kept, centered) that the cells cover.
    pub(super) fn placeholder_draw(
        &self,
        r: usize,
        (id, pid, row, col, screen_col, len): (u32, u32, u32, u32, usize, usize),
        out: &mut Vec<crate::terminal::graphics::ImageDraw>,
    ) {
        let Some(vp) = self.graphics.virtual_for(id, pid) else { return };
        let Some(img) = self.graphics.image(id) else { return };
        let (key, rgba) = img.shown(std::time::Instant::now());
        let (cw, ch) = self.image_cell_px();
        let (cw, ch) = (cw as f32, ch as f32);
        let (iw, ih) = (img.width as f32, img.height as f32);
        let (bw, bh) = (vp.cols as f32 * cw, vp.rows as f32 * ch);
        let s = (bw / iw).min(bh / ih);
        let (ox, oy) = ((bw - iw * s) / 2.0, (bh - ih * s) / 2.0);
        // The run's rectangle in the box, clipped to the image.
        let (cx, cy) = (col as f32 * cw, row as f32 * ch);
        let x0 = cx.max(ox);
        let x1 = (cx + len as f32 * cw).min(ox + iw * s);
        let y0 = cy.max(oy);
        let y1 = (cy + ch).min(oy + ih * s);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        out.push(crate::terminal::graphics::ImageDraw {
            key,
            width: img.width,
            height: img.height,
            rgba: rgba.clone(),
            src: [(x0 - ox) / s, (y0 - oy) / s, (x1 - x0) / s, (y1 - y0) / s],
            row: r as isize,
            col: screen_col,
            cols: len,
            rows: 1,
            fit: true,
            z: vp.z,
            px: Some([x0 - cx, y0 - cy, x1 - x0, y1 - y0]),
        });
    }
}
