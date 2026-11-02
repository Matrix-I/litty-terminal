//! The viewport over scrollback: whole-row and pixel-precise scrolling,
//! hit-testing rows, and clearing history / the screen (Cmd+K).

use super::*;

impl Grid {
    /// Scroll the viewport through history (positive = towards older lines).
    /// The selection stays attached to its text.
    pub fn scroll_display(&mut self, delta: isize) {
        if self.alt_active {
            return;
        }
        let was_fractional = self.scroll_fraction != 0.0;
        self.scroll_fraction = 0.0;
        let new = (self.display_offset as isize + delta).clamp(0, self.history as isize) as usize;
        if new != self.display_offset || was_fractional {
            self.display_offset = new;
            self.all_dirty = true;
        }
    }

    /// Precise scrolling changes the viewport on every pixel, not only at row boundaries.
    pub fn scroll_pixels(&mut self, pixels: f64, cell_height: f64) -> bool {
        if self.alt_active || !pixels.is_finite() || !cell_height.is_finite() || cell_height <= 0.0 {
            return false;
        }
        let current = self.display_offset as f64 - self.scroll_fraction;
        let next = (current + pixels / cell_height).clamp(0.0, self.history as f64);
        let nearest = next.round();
        let next = if (next - nearest).abs() < 1e-10 { nearest } else { next };
        let offset = next.ceil() as usize;
        let fraction = offset as f64 - next;
        if offset == self.display_offset && (fraction - self.scroll_fraction).abs() < 1e-10 {
            return false;
        }
        self.all_dirty |= offset != self.display_offset;
        self.display_offset = offset;
        self.scroll_fraction = fraction;
        self.viewport_dirty = true;
        true
    }

    /// Hit-test a row after applying the same pixel shift as the renderer.
    pub fn viewport_row(&self, y: f64, cell_height: f64) -> usize {
        ((y / cell_height.max(1.0) + self.scroll_fraction).max(0.0).floor() as usize)
            .min(self.rows.saturating_sub(usize::from(self.scroll_fraction == 0.0)))
    }

    pub fn reset_display(&mut self) {
        if self.display_offset != 0 {
            self.display_offset = 0;
            self.scroll_fraction = 0.0;
            self.all_dirty = true;
        }
        if self.selection.is_some() {
            self.selection = None;
            self.all_dirty = true;
        }
    }

    pub fn clear_history(&mut self) {
        self.image_epoch = self.image_epoch.wrapping_add(1);
        self.prompts.clear();
        self.graphics.prune(self.lines_pushed);
        self.history = 0;
        self.display_offset = 0;
        self.scroll_fraction = 0.0;
        self.selection = None;
        self.clear_search();
        self.trim_extras();
        if !self.clusters.texts.is_empty() {
            self.gc_clusters();
        } else {
            self.clusters.exhausted = false;
        }
        if !self.graphics.virtuals.is_empty() {
            self.prune_dead_virtuals();
        }
        self.all_dirty = true;
    }

    /// Clear both scrollback history and visible screen rows, placing cursor at top-left.
    /// Provides instant 0ms visual feedback on Cmd+K without waiting for shell roundtrip.
    pub fn clear_screen_and_history(&mut self) {
        self.clear_history();
        let rows = self.rows;
        self.clear_rows(0, rows);
        self.graphics.delete_lines(self.lines_pushed..self.lines_pushed + rows, self.alt_active);
        self.graphics.virtuals.clear();
        self.graphics.prune_dead_auto_ids();
        self.cur.row = 0;
        self.cur.col = 0;
        self.wrap_pending = false;
        self.clusters.texts.clear();
        self.clusters.ids.clear();
        self.trim_extras();
        self.apc_state = 0;
        self.apc_buf.clear();
        self.apc_buf.shrink_to_fit();
        self.apc_start = None;
        self.dcs = None;
        self.dcs_buf.clear();
        self.dcs_buf.shrink_to_fit();
        self.dcs_start = None;
        self.all_dirty = true;
    }

    /// Scrollback length and how far the view is scrolled back, in lines.
    pub fn scroll_position(&self) -> (usize, usize) {
        (self.history, self.display_offset)
    }

    /// First absolute line still in scrollback.
    pub(super) fn first_line(&self) -> usize {
        self.lines_pushed - self.history
    }

    /// Scroll so absolute line `line` is at the top of the viewport (clamped).
    pub(super) fn scroll_to_line(&mut self, line: usize) {
        let offset = self.lines_pushed.saturating_sub(line).min(self.history);
        if self.display_offset != offset || self.scroll_fraction != 0.0 {
            self.display_offset = offset;
            self.scroll_fraction = 0.0;
            self.all_dirty = true;
        }
    }
}
