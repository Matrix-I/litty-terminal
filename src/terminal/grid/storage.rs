//! Scrollback storage: one flat zero-initialized `Vec<Cell>` ring of
//! `cols * cap` cells, grown on demand up to `scrollback + rows` rows.
//! Full-screen scrolling is O(1) (advance `top`, clear one row); nothing here
//! allocates except `grow_capacity`.

use super::*;

/// Number of scrollback lines kept above the visible screen by default.
pub const SCROLLBACK: usize = 5000;

/// Upper bound for a configured scrollback, whatever its byte size.
pub(super) const MAX_SCROLLBACK: usize = 1_000_000;

/// `scrollback-limit` in bytes (Ghostty semantics); `usize::MAX` = the default
/// line count. Read when a terminal is created or resized.
pub(super) static SCROLLBACK_BYTES: AtomicUsize = AtomicUsize::new(usize::MAX);

pub fn set_scrollback_limit(bytes: Option<usize>) {
    SCROLLBACK_BYTES.store(bytes.unwrap_or(usize::MAX), Ordering::Relaxed);
}

/// Scrollback lines for a terminal `cols` wide.
pub(super) fn scrollback_lines(cols: usize) -> usize {
    lines_for_bytes(SCROLLBACK_BYTES.load(Ordering::Relaxed), cols)
}

pub(super) fn lines_for_bytes(bytes: usize, cols: usize) -> usize {
    match bytes {
        usize::MAX => SCROLLBACK,
        bytes => (bytes / (cols.max(1) * std::mem::size_of::<Cell>())).min(MAX_SCROLLBACK),
    }
}

impl Grid {
    // ------------------------------------------------------------------
    // Row addressing
    // ------------------------------------------------------------------


    #[inline]
    pub(super) fn row_start(&self, r: usize) -> usize {
        if self.alt_active {
            r * self.cols
        } else {
            ((self.top + r) % self.cap) * self.cols
        }
    }

    #[inline]
    pub(super) fn buf_mut(&mut self) -> &mut [Cell] {
        if self.alt_active {
            &mut self.alt
        } else {
            &mut self.cells
        }
    }

    #[inline]
    pub(super) fn row_mut(&mut self, r: usize) -> &mut [Cell] {
        let s = self.row_start(r);
        let c = self.cols;
        &mut self.buf_mut()[s..s + c]
    }

    /// Row `r` of what should be on screen, accounting for scrollback viewing.
    #[inline]
    pub fn display_row(&self, r: usize) -> &[Cell] {
        if self.alt_active {
            let s = r * self.cols;
            &self.alt[s..s + self.cols]
        } else {
            let p = (self.top + self.cap - self.display_offset + r) % self.cap;
            let s = p * self.cols;
            &self.cells[s..s + self.cols]
        }
    }

    /// Cursor position in screen coordinates, if it should be drawn.
    /// Absolute line number of visible row `r`.
    #[inline]
    pub(super) fn abs_line(&self, r: usize) -> usize {
        if self.alt_active {
            self.lines_pushed + r
        } else {
            self.lines_pushed + r - self.display_offset
        }
    }

    /// Cells of absolute line `l`, if it is still in scrollback or on screen.
    pub(super) fn line_cells(&self, l: usize) -> Option<&[Cell]> {
        let c = self.cols;
        if self.alt_active {
            let r = l.checked_sub(self.lines_pushed).filter(|&r| r < self.rows)?;
            return Some(&self.alt[r * c..(r + 1) * c]);
        }
        let i = l.checked_sub(self.lines_pushed - self.history).filter(|&i| i < self.history + self.rows)?;
        let p = (self.top + self.cap - self.history + i) % self.cap;
        Some(&self.cells[p * c..(p + 1) * c])
    }

    /// Whether absolute line `l` soft-wraps onto the next line.
    pub(super) fn line_wrapped(&self, l: usize) -> bool {
        if self.alt_active {
            return false;
        }
        l.checked_sub(self.lines_pushed - self.history)
            .filter(|&i| i < self.history + self.rows)
            .is_some_and(|i| self.wrapped[(self.top + self.cap - self.history + i) % self.cap])
    }

    #[inline]
    pub(super) fn set_wrapped(&mut self, r: usize, on: bool) {
        if !self.alt_active {
            let p = (self.top + r) % self.cap;
            self.wrapped[p] = on;
        }
    }

    pub(super) fn clear_cols(&mut self, r: usize, c0: usize, c1: usize) {
        let b = self.blank();
        let c1 = c1.min(self.cols);
        if c0 < c1 {
            let had_extra = self.extras.len() > 1
                && self.row_mut(r)[c0..c1]
                    .iter()
                    .any(|c| ((c.fg & FG_EXTRA) | (c.bg & BG_EXTRA)) != 0);
            if had_extra {
                self.extras_dirty = true;
            }
            self.row_mut(r)[c0..c1].fill(b);
        }
        if c1 == self.cols {
            // The wrap marker lives in the last column; erasing it ends the wrap.
            self.set_wrapped(r, false);
        }
        self.mark(r);
    }

    pub(super) fn clear_rows(&mut self, r0: usize, r1: usize) {
        for r in r0..r1.min(self.rows) {
            self.clear_cols(r, 0, self.cols);
        }
    }

    pub(super) fn copy_row(&mut self, src: usize, dst: usize) {
        let s = self.row_start(src);
        let d = self.row_start(dst);
        let c = self.cols;
        let check_extras = self.extras.len() > 1;
        let had_extra = check_extras && {
            let buf = self.buf_mut();
            buf[d..d + c].iter().any(|cell| ((cell.fg & FG_EXTRA) | (cell.bg & BG_EXTRA)) != 0)
        };
        if had_extra {
            self.extras_dirty = true;
        }
        self.buf_mut().copy_within(s..s + c, d);
        if !self.alt_active {
            self.wrapped[d / c] = self.wrapped[s / c];
        }
        self.mark(dst);
    }

    pub(super) fn grow_capacity(&mut self) {
        let max_cap = self.scrollback + self.rows;
        if self.cap >= max_cap {
            return;
        }
        let new_cap = (self.cap * 4).max(512).min(max_cap);
        let mut new_cells = bytemuck::zeroed_vec::<Cell>(self.cols * new_cap);
        let mut new_wrapped = vec![false; new_cap];
        let n_lines = self.history + self.rows;
        let w = self.cols;
        for i in 0..n_lines {
            let op = (self.top + self.cap - self.history + i) % self.cap;
            let src = op * self.cols;
            new_cells[i * self.cols..i * self.cols + w].copy_from_slice(&self.cells[src..src + w]);
            new_wrapped[i] = self.wrapped[op];
        }
        self.cells = new_cells;
        self.wrapped = new_wrapped;
        self.cap = new_cap;
        self.top = self.history;
    }

    /// Scroll rows `t..=b` up by `n`. When the region is the whole main screen
    /// and `to_history` is set, this is an O(1) ring rotation into scrollback.
    pub(super) fn scroll_up_in(&mut self, t: usize, b: usize, n: usize, to_history: bool) {
        if t > b || b >= self.rows {
            return;
        }
        let n = n.min(b - t + 1);
        if n == 0 {
            return;
        }
        if to_history && !self.alt_active && t == 0 && b == self.rows - 1 {
            let mut evicted_had_ph = false;

            for _ in 0..n {
                if self.history + self.rows >= self.cap && self.cap < self.scrollback + self.rows {
                    self.grow_capacity();
                }
                // A rows-only resize keeps the ring, so it may hold more rows than
                // the scrollback limit allows.
                let max_history = (self.cap - self.rows).min(self.scrollback);
                if self.history == max_history {
                    let evicted_row = (self.top + self.cap - self.history) % self.cap;
                    let start = evicted_row * self.cols;
                    let end = start + self.cols;
                    if !self.graphics.virtuals.is_empty() && self.cells[start..end].iter().any(|c| c.codepoint & PH_FLAG != 0) {
                        evicted_had_ph = true;
                    }
                    if self.clusters.texts.len() >= 4096 && self.cells[start..end].iter().any(|c| c.codepoint & (CLUSTER_FLAG | PH_FLAG) == CLUSTER_FLAG) {
                        self.clusters.evictions = self.clusters.evictions.saturating_add(1);
                    }
                    if self.extras.len() > 1 && self.cells[start..end].iter().any(|c| ((c.fg & FG_EXTRA) | (c.bg & BG_EXTRA)) != 0) {
                        self.extras_dirty = true;
                    }
                }
                self.top = (self.top + 1) % self.cap;
                self.lines_pushed += 1;
                if self.history < max_history {
                    self.history += 1;
                }
                self.clear_cols(self.rows - 1, 0, self.cols);
            }
            if self.display_offset > 0 {
                // Keep the user's scrolled-back view anchored on the same lines.
                self.display_offset = (self.display_offset + n).min(self.history);
            }
            if !self.graphics.placements.is_empty() {
                self.graphics.prune(self.lines_pushed - self.history);
            }
            if evicted_had_ph {
                self.prune_dead_virtuals();
            }
            // Amortize a history scan across a history-sized batch, even when
            // every interned cluster is still live. MAX_CLUSTERS also triggers
            // demand collection so dead IDs never permanently exhaust the pool.
            if self.clusters.evictions >= self.history.max(self.rows).max(256) {
                self.gc_clusters();
            }
            self.all_dirty = true;
        } else {
            for r in t..(b + 1 - n) {
                self.copy_row(r + n, r);
            }
            self.clear_rows(b + 1 - n, b + 1);
            if self.alt_active && !self.graphics.placements.is_empty() {
                self.graphics.scroll_alt(self.lines_pushed + t, self.lines_pushed + b, n);
            }
        }
    }

    pub(super) fn scroll_down_in(&mut self, t: usize, b: usize, n: usize) {
        if t > b || b >= self.rows {
            return;
        }
        let n = n.min(b - t + 1);
        for r in (t + n..=b).rev() {
            self.copy_row(r - n, r);
        }
        self.clear_rows(t, t + n);
        if self.alt_active && !self.graphics.placements.is_empty() {
            self.graphics.scroll_alt_down(self.lines_pushed + t, self.lines_pushed + b, n);
        }
    }

    /// Release physical OS pages backing scrollback rows outside the visible screen
    /// (`top .. top + rows` in the ring) using `madvise` (`MADV_FREE` on macOS,
    /// `MADV_DONTNEED` on Linux) so clearing scrollback immediately frees dirty RAM
    /// without reallocating `self.cells`.
    pub(super) fn release_scrollback_pages(&mut self) {
        if self.cols == 0 || self.cap <= self.rows || self.cells.is_empty() {
            return;
        }
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if page_size <= 0 {
            return;
        }
        let page = page_size as usize;
        let base = self.cells.as_mut_ptr() as usize;
        let cell_bytes = std::mem::size_of::<Cell>();
        let row_bytes = self.cols * cell_bytes;

        let advise_range = |start_row: usize, end_row: usize| {
            if start_row >= end_row {
                return;
            }
            let raw_start = base + start_row * row_bytes;
            let raw_end = base + end_row * row_bytes;
            let aligned_start = (raw_start + page - 1) & !(page - 1);
            let aligned_end = raw_end & !(page - 1);
            if aligned_start < aligned_end {
                #[cfg(target_os = "macos")]
                let advice = libc::MADV_FREE;
                #[cfg(not(target_os = "macos"))]
                let advice = libc::MADV_DONTNEED;
                // SAFETY: `[aligned_start, aligned_end)` is page-aligned and strictly within
                // `self.cells`'s heap allocation outside the active visible rows.
                unsafe {
                    let _ = libc::madvise(aligned_start as *mut libc::c_void, aligned_end - aligned_start, advice);
                }
            }
        };

        if self.top + self.rows <= self.cap {
            advise_range(0, self.top);
            advise_range(self.top + self.rows, self.cap);
        } else {
            let wrap_end = (self.top + self.rows) - self.cap;
            advise_range(wrap_end, self.top);
        }
    }
}
