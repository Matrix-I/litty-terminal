//! Window resize: reflow main screen + scrollback to the new width, keep the
//! cursor on screen, remap prompt marks and image placements. The only place
//! the grid allocates after startup.

use super::*;

impl Grid {
    // ------------------------------------------------------------------
    // Resize (the only place the grid allocates after startup)
    // ------------------------------------------------------------------


    pub fn resize(&mut self, cols: usize, rows: usize) {
        let cols = cols.max(1);
        let rows = rows.max(1);
        if cols == self.cols && rows == self.rows {
            return;
        }
        // Reflow renumbers lines; the caller re-runs an open search.
        self.clear_search();

        // 1. Main screen + scrollback -> logical lines (soft wraps joined), tracking
        //    the main cursor as (line, offset).
        let main_cur = if self.alt_active { self.saved_main } else { self.cur };
        let total = self.history + self.rows;
        let cursor_row = self.history + main_cur.row.min(self.rows - 1);
        let remap = !self.prompts.is_empty() || self.graphics.placements.iter().any(|p| !p.alt);
        let mut row_pos: Vec<(usize, usize)> = Vec::with_capacity(if remap { total } else { 0 });

        let mut logical_cells: Vec<Cell> = Vec::with_capacity(total * self.cols);
        let mut line_spans: Vec<std::ops::Range<usize>> = Vec::with_capacity(total);
        let mut current_start = 0;
        let mut cursor = (0, 0);
        for i in 0..total {
            let p = (self.top + self.cap - self.history + i) % self.cap;
            let current_len = logical_cells.len() - current_start;
            if i == cursor_row {
                cursor = (line_spans.len(), current_len + main_cur.col);
            }
            if remap {
                row_pos.push((line_spans.len(), current_len));
            }
            logical_cells.extend_from_slice(&self.cells[p * self.cols..(p + 1) * self.cols]);
            if self.wrapped[p] && i + 1 < total {
                continue;
            }
            let keep = if line_spans.len() == cursor.0 && i >= cursor_row { cursor.1 } else { 0 };
            let current = &logical_cells[current_start..];
            let content = current.iter().rposition(|c| c.codepoint != 0 || c.bg & BG_SET != 0).map_or(0, |idx| idx + 1);
            let target_len = content.max(keep).min(current.len());
            logical_cells.truncate(current_start + target_len);
            line_spans.push(current_start..logical_cells.len());
            current_start = logical_cells.len();
        }
        // Blank lines below the cursor are not content worth keeping.
        while line_spans.len() > cursor.0 + 1 && line_spans.last().is_some_and(|r| r.is_empty()) {
            line_spans.pop();
        }

        // 2. Re-wrap every logical line to the new width.
        let mut out: Vec<Cell> = Vec::with_capacity(total * cols);
        let mut out_wrapped: Vec<bool> = Vec::with_capacity(total);
        let mut new_cursor = (0, 0);
        let mut line_rows: Vec<usize> = Vec::with_capacity(if remap { line_spans.len() + 1 } else { 0 });
        for (li, span) in line_spans.iter().enumerate() {
            let line = &logical_cells[span.clone()];
            let first_row = out_wrapped.len();
            if remap {
                line_rows.push(first_row);
            }
            out.resize(out.len() + cols, Cell::default());
            out_wrapped.push(false);
            let mut col = 0;
            let mut i = 0;
            while i < line.len() {
                let cell = line[i];
                let w = if is_wide_cp(cell.codepoint) { 2.min(cols) } else { 1 };
                if col + w > cols {
                    *out_wrapped.last_mut().unwrap() = true;
                    out.resize(out.len() + cols, Cell::default());
                    out_wrapped.push(false);
                    col = 0;
                }
                if li == cursor.0 && (i..i + w).contains(&cursor.1) {
                    new_cursor = (out_wrapped.len() - 1, col);
                }
                let base = out.len() - cols;
                out[base + col] = cell;
                if w == 2 {
                    out[base + col + 1] = Cell { codepoint: 0, ..cell };
                }
                col += w;
                i += if w == 2 && line.get(i + 1).is_some_and(|c| c.codepoint == 0) { 2 } else { 1 };
            }
            if li == cursor.0 && cursor.1 >= line.len() {
                let extra = cursor.1 - line.len();
                let row = (out_wrapped.len() - 1).max(first_row);
                new_cursor = (row, (col + extra).min(cols - 1));
            }
        }

        // Drop intermediate logical line storage before allocating new grid cells to reduce peak memory.
        drop(logical_cells);
        drop(line_spans);

        // 3. Keep the cursor on screen; everything above it becomes scrollback.
        let total_rows = out_wrapped.len().max(new_cursor.0 + 1);
        let screen_start = total_rows.saturating_sub(rows).min(new_cursor.0).max((new_cursor.0 + 1).saturating_sub(rows));
        let scrollback = scrollback_lines(cols);
        let new_hist = screen_start.min(scrollback);
        let first = screen_start - new_hist;
        let new_cap = (new_hist + rows).max(rows * 2).max(64).min(scrollback + rows);
        let mut cells = bytemuck::zeroed_vec::<Cell>(cols * new_cap);
        let mut wrapped = vec![false; new_cap];
        for (dst, src) in (first..(screen_start + rows).min(out_wrapped.len())).enumerate() {
            cells[dst * cols..(dst + 1) * cols].copy_from_slice(&out[src * cols..(src + 1) * cols]);
            wrapped[dst] = out_wrapped[src];
        }

        // 4. Move prompt marks and images from old absolute lines to the reflowed ones.
        let new_lines_pushed = self.lines_pushed.max(new_hist);
        if remap {
            line_rows.push(out_wrapped.len());
            let old_first = self.first_line();
            let map_row = |i: usize| -> Option<usize> {
                let &(li, off) = row_pos.get(i)?;
                let (start, end) = (*line_rows.get(li)?, *line_rows.get(li + 1)?);
                let row = (start + off / cols).min(end.saturating_sub(1).max(start));
                (row >= first).then(|| new_lines_pushed + row - screen_start)
            };
            let map_line = |l: usize| -> Option<usize> {
                let i = l.checked_sub(old_first)?;
                // An output end may be the line after the last one.
                if i >= total && i > 0 { map_row(total - 1).map(|r| r + i - total + 1) } else { map_row(i) }
            };
            self.prompts.retain_mut(|m| {
                let Some(prompt) = map_line(m.prompt) else { return false };
                m.prompt = prompt;
                m.output = m.output.and_then(|(start, end)| Some((map_line(start)?, end.and_then(map_line))));
                true
            });
            self.graphics.placements.retain_mut(|p| {
                if p.alt {
                    return true;
                }
                let Some(line) = map_line(p.line) else { return false };
                p.line = line;
                true
            });
            self.graphics.prune_dead_auto_ids();
        }

        let alt = if self.alt_active {
            let mut alt = bytemuck::zeroed_vec::<Cell>(cols * rows);
            let w = cols.min(self.cols);
            for r in 0..rows.min(self.rows) {
                if !self.alt.is_empty() {
                    alt[r * cols..r * cols + w].copy_from_slice(&self.alt[r * self.cols..r * self.cols + w]);
                }
            }
            alt
        } else {
            Vec::new()
        };

        self.cells = cells;
        self.wrapped = wrapped;
        self.alt = alt;
        self.cap = new_cap;
        self.scrollback = scrollback;
        self.top = new_hist;
        self.history = new_hist;
        self.lines_pushed = new_lines_pushed;
        self.display_offset = 0;
        self.scroll_fraction = 0.0;
        let main_row = new_cursor.0 - screen_start;
        if self.alt_active {
            self.saved_main.row = main_row;
            self.saved_main.col = new_cursor.1;
        } else {
            self.cur.row = main_row;
            self.cur.col = new_cursor.1;
        }
        self.cols = cols;
        self.rows = rows;
        self.clamp_cursor();
        for c in [&mut self.saved, &mut self.saved_main] {
            c.row = c.row.min(rows - 1);
            c.col = c.col.min(cols - 1);
        }
        self.tab_stops = default_tab_stops(cols);
        self.scroll_top = 0;
        self.scroll_bot = rows - 1;
        self.wrap_pending = false;
        self.selection = None;
        self.prompt_input_mark = None;
        self.dirty = BitSet512::ones(rows);
        self.all_dirty = true;
        self.extras_dirty = true;
        if !self.graphics.virtuals.is_empty() {
            self.prune_dead_virtuals();
        }
    }
}
