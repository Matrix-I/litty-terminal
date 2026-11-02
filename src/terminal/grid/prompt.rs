//! Shell prompt mark tracking for OSC 133 semantic prompt jump and output selection,
//! and the prompt-input helpers used by the AI bridge and click-to-move-cursor.

use super::*;

/// Prompts remembered for jump-to-prompt and output selection.
pub const MAX_PROMPTS: usize = 1000;

/// Shell-integration state for the AI bridge (OSC 778 `init` / `yo`, OSC 133 `C`).
/// The grid only records it; the AI module and the app decide what it means.
#[derive(Default)]
pub struct AiTrack {
    /// AI bridge is armed only by a user submission at a known shell prompt.
    pub available: bool,
    pub direct: bool,
    pub armed: bool,
    pub track: bool,
    pub command: Option<String>,
    pub request: Option<String>,
}

/// One shell prompt from OSC 133 marks, as absolute lines.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PromptMark {
    /// Line of the prompt (OSC 133;A).
    pub prompt: usize,
    /// First output line (OSC 133;C) and the line after the last (OSC 133;D).
    pub output: Option<(usize, Option<usize>)>,
}

impl Grid {
    pub fn prompt_ready(&self) -> bool {
        self.prompt_input_mark.is_some() && !self.alt_active
    }

    /// Rows between the start of the current prompt (`133;A`) and the cursor, so a
    /// multi-line prompt can be erased as a whole. Zero without a current prompt.
    pub fn prompt_rows_above_cursor(&self) -> usize {
        self.prompts
            .last()
            .filter(|p| p.output.is_none())
            .map_or(0, |p| self.cursor_line().saturating_sub(p.prompt))
    }

    /// Whole editable input, including text after the cursor and soft-wrapped rows.
    pub fn full_prompt_input(&self) -> String {
        let Some((start, col)) = self.prompt_input_mark else {
            return String::new();
        };
        let mut text = String::new();
        let mut line = start;
        let cursor = self.lines_pushed + self.cur.row;
        while let Some(cells) = self.line_cells(line) {
            let first = if line == start { col } else { 0 };
            for cell in cells.iter().skip(first).filter(|c| c.codepoint != 0) {
                self.push_cell_text(cell, &mut text);
            }
            let wrapped = self.line_wrapped(line);
            if !wrapped {
                if line >= cursor {
                    break;
                }
                let trimmed = text.trim_end_matches(' ').len();
                text.truncate(trimmed);
                text.push('\n');
            }
            line += 1;
            if line >= self.lines_pushed + self.rows || text.len() > 65536 {
                break;
            }
        }
        text.trim_end().to_owned()
    }

    /// A bounded copy of recent logical lines, excluding the current prompt.
    pub fn extract_scrollback_lines(&self, max_lines: usize) -> String {
        if self.alt_active || max_lines == 0 {
            return String::new();
        }
        let end = self
            .prompts
            .last()
            .map(|p| p.prompt)
            .unwrap_or(self.lines_pushed + self.cur.row);
        let end = end.min(self.lines_pushed + self.rows);
        let mut start = end;
        let mut count = 0;
        let limit = max_lines.min(1000);
        while start > self.first_line() && count < limit {
            start -= 1;
            if start == self.first_line() || !self.line_wrapped(start - 1) {
                count += 1;
            }
            // Bound even a pathological single wrapped logical line.
            if end - start >= 4000 {
                break;
            }
        }
        let mut out = String::new();
        for line in start..end {
            if let Some(cells) = self.line_cells(line) {
                let before = out.len();
                for cell in cells.iter().filter(|c| c.codepoint != 0) {
                    self.push_cell_text(cell, &mut out);
                }
                if !self.line_wrapped(line) {
                    let trimmed = out[before..].trim_end_matches(' ').len();
                    out.truncate(before + trimmed);
                    out.push('\n');
                }
            }
            if out.len() > 256 * 1024 {
                break;
            }
        }
        crate::terminal::text::strip_controls(&out)
    }

    /// Extract text typed by user on the active prompt line (before the cursor).
    pub fn current_prompt_input(&self) -> String {
        if self.cur.row >= self.rows || self.cols == 0 {
            return String::new();
        }
        let (cmd_start, _) = self.prompt_command_range();
        let r = self.display_row(self.cur.row);
        let end_col = self.cur.col.min(r.len());
        if cmd_start >= end_col {
            return String::new();
        }
        let mut text = String::new();
        for c in &r[cmd_start..end_col] {
            if c.codepoint != 0 {
                self.push_cell_text(c, &mut text);
            }
        }
        text.trim().to_string()
    }

    /// Returns the column range `(cmd_start_col, cmd_end_col)` for the active command line on `self.cur.row`.
    pub fn prompt_command_range(&self) -> (usize, usize) {
        if self.cur.row >= self.rows || self.cols == 0 {
            return (0, 0);
        }
        let r = self.display_row(self.cur.row);
        let cur_col = self.cur.col.min(self.cols);
        let cmd_start = match self.prompt_input_mark {
            // Shell integration (OSC 133;B) marked exactly where the input begins.
            Some((line, col)) if line == self.abs_line(self.cur.row) && col <= cur_col => col,
            _ => self.guess_command_start(r, cur_col),
        };

        let last_non_blank = r[cmd_start..]
            .iter()
            .rposition(|c| !Self::is_blank(c.codepoint))
            .map(|idx| cmd_start + idx + 1)
            .unwrap_or(cmd_start);

        let cmd_end = last_non_blank.max(cur_col).min(self.cols);
        (cmd_start, cmd_end)
    }

    /// Guess where the command input starts on a prompt line from common prompt
    /// glyphs (Powerline separators, `$`, `%`, `❯`, ...) when no OSC 133 mark exists.
    pub(super) fn guess_command_start(&self, r: &[Cell], cur_col: usize) -> usize {
        // 1. Check for Powerline segment separator U+E0B0 ('') before or at cursor
        let mut pl_end = None;
        if let Some(last_pl) = r.iter().take(cur_col + 1).rposition(|c| c.codepoint == 0xE0B0) {
            let mut sc = last_pl + 1;
            while sc < self.cols && Self::is_blank(r[sc].codepoint) {
                sc += 1;
            }
            if sc < self.cols
                && matches!(
                    char::from_u32(r[sc].codepoint),
                    Some('$' | '%' | '#' | '❯' | '>' | '➜')
                )
            {
                sc += 1;
                while sc < self.cols && Self::is_blank(r[sc].codepoint) {
                    sc += 1;
                }
            }
            pl_end = Some(sc.min(cur_col));
        }

        // 2. Search for standard prompt signs using exact column-level matching
        let mut sign_end = None;
        const PROMPT_SIGNS: &[&[char]] = &[
            &['❯', ' '],
            &['➜', ' '],
            &['%', ' '],
            &['$', ' '],
            &['#', ' '],
            &['>', ' '],
            &[':', ' '],
            &['λ', ' '],
            &['»', ' '],
            &['›', ' '],
            &['>', '>', '>', ' '],
        ];

        let search_limit = (cur_col + 1).min(r.len());
        for &sign in PROMPT_SIGNS {
            let sign_len = sign.len();
            if search_limit >= sign_len {
                for i in (0..=search_limit - sign_len).rev() {
                    let matches = sign.iter().enumerate().all(|(k, &ch)| {
                        r[i + k].codepoint == ch as u32
                    });
                    if matches {
                        let end = i + sign_len;
                        if sign_end.is_none_or(|cur| end > cur) {
                            sign_end = Some(end);
                        }
                        break;
                    }
                }
            }
        }

        if sign_end.is_none() {
            for &sym in &['❯', '➜', '%', '$', '#', '>', 'λ', '»', '›'] {
                if let Some(pos) = r[..search_limit].iter().rposition(|c| c.codepoint == sym as u32) {
                    let end = pos + 1;
                    if end <= cur_col {
                        let mut sc = end;
                        while sc < self.cols && Self::is_blank(r[sc].codepoint) {
                            sc += 1;
                        }
                        sign_end = Some(sc.min(cur_col));
                        break;
                    }
                }
            }
        }

        pl_end.or(sign_end).unwrap_or(0).min(cur_col)
    }

    /// Calculate the arrow-key escape sequences needed to move the shell cursor to `(pane_row, pane_col)`.
    /// Returns `None` if the click is outside the active command prompt line or no movement is needed.
    pub fn calculate_cursor_movement_to(&self, pane_row: usize, pane_col: usize) -> Option<Vec<u8>> {
        if self.alt_active || self.display_offset != 0 {
            return None;
        }
        if pane_row != self.cur.row || self.cur.row >= self.rows || self.cols == 0 {
            return None;
        }

        let r = self.display_row(self.cur.row);
        let cur_col = self.cur.col.min(self.cols);
        let (cmd_start, cmd_end) = self.prompt_command_range();

        let mut target_col = pane_col.clamp(cmd_start, cmd_end);

        // If target_col lands on a wide-character dummy cell (codepoint == 0 following a width-2 character), snap to character start
        if target_col > cmd_start
            && target_col < self.cols
            && r[target_col].codepoint == 0
            && is_wide_cp(r[target_col - 1].codepoint)
        {
            target_col -= 1;
        }

        if target_col == cur_col {
            return None;
        }

        let seq_left = if self.modes.app_cursor { b"\x1bOD" } else { b"\x1b[D" };
        let seq_right = if self.modes.app_cursor { b"\x1bOC" } else { b"\x1b[C" };

        if target_col < cur_col {
            let mut count = 0;
            for c in &r[target_col..cur_col] {
                if c.codepoint != 0 {
                    count += 1;
                }
            }
            let count = count.max(1);
            let mut bytes = Vec::with_capacity(count * seq_left.len());
            for _ in 0..count {
                bytes.extend_from_slice(seq_left);
            }
            Some(bytes)
        } else {
            let mut count = 0;
            for c in &r[cur_col..target_col] {
                if c.codepoint != 0 {
                    count += 1;
                }
            }
            let count = count.max(1);
            let mut bytes = Vec::with_capacity(count * seq_right.len());
            for _ in 0..count {
                bytes.extend_from_slice(seq_right);
            }
            Some(bytes)
        }
    }

    /// Calculate bytes to delete all text on the command line before the cursor.
    /// If at a shell prompt, returns the exact number of backspaces needed to clear
    /// from the prompt end up to the cursor without affecting the prompt.
    /// In alternate screen mode, returns `Ctrl+U` (`\x15`).
    pub fn calculate_delete_before_cursor(&self) -> Option<Vec<u8>> {
        if self.alt_active {
            return Some(vec![0x15]); // Ctrl+U
        }
        if self.cur.row >= self.rows || self.cols == 0 {
            return None;
        }
        let (cmd_start, _) = self.prompt_command_range();
        let cur_col = self.cur.col.min(self.cols);
        if cur_col <= cmd_start {
            return None;
        }
        let r = self.display_row(self.cur.row);
        let mut count = 0;
        for c in &r[cmd_start..cur_col] {
            if c.codepoint != 0 {
                count += 1;
            }
        }
        if count == 0 {
            return None;
        }
        Some(vec![0x7f; count])
    }

    /// Calculate bytes to delete all text on the command line after the cursor.
    /// Sends `Ctrl+K` (`\x0b`, standard kill-line).
    #[allow(dead_code)]
    pub fn calculate_delete_after_cursor(&self) -> Option<Vec<u8>> {
        if self.alt_active {
            return Some(vec![0x0b]);
        }
        if self.cur.row >= self.rows || self.cols == 0 {
            return Some(vec![0x0b]);
        }
        let (_, cmd_end) = self.prompt_command_range();
        let cur_col = self.cur.col.min(self.cols);
        if cur_col >= cmd_end {
            return None;
        }
        Some(vec![0x0b])
    }

    /// Absolute line of the cursor (independent of the viewport).
    pub(super) fn cursor_line(&self) -> usize {
        self.lines_pushed + self.cur.row
    }

    /// Track OSC 133 `A` (prompt), `C` (output starts) and `D` (output ends).
    pub(super) fn record_prompt_mark(&mut self, kind: Option<u8>) {
        let line = self.cursor_line();
        match kind {
            Some(b'A') => {
                if self.prompts.last().is_some_and(|p| p.prompt == line) {
                    return;
                }
                if self.prompts.len() == MAX_PROMPTS {
                    self.prompts.remove(0);
                }
                self.prompts.push(PromptMark { prompt: line, output: None });
            }
            Some(b'C') => {
                if let Some(p) = self.prompts.last_mut() {
                    p.output = Some((line, None));
                }
            }
            Some(b'D') => {
                // Output that did not end in a newline still owns the cursor line.
                let end = line + usize::from(self.cur.col > 0);
                if let Some(PromptMark { output: Some((_, e @ None)), .. }) = self.prompts.last_mut() {
                    *e = Some(end);
                }
            }
            _ => {}
        }
    }

    /// Jump to the previous (`delta < 0`) or next prompt relative to the top of
    /// the viewport. Returns `false` when there is none in that direction.
    pub fn jump_to_prompt(&mut self, delta: isize) -> bool {
        if self.alt_active {
            return false;
        }
        let top = self.lines_pushed - self.display_offset;
        let first = self.first_line();
        let mut lines = self.prompts.iter().map(|p| p.prompt).filter(|&l| l >= first);
        let target = if delta < 0 { lines.rfind(|&l| l < top) } else { lines.find(|&l| l > top) };
        match target {
            Some(line) => {
                self.scroll_to_line(line);
                true
            }
            // Past the last prompt: back to the live screen.
            None if delta > 0 && self.display_offset != 0 => {
                self.scroll_to_line(self.lines_pushed);
                true
            }
            None => false,
        }
    }

    /// Select the output of the most recent finished command (OSC 133 C..D) and
    /// return its text, scrolling it into view.
    pub fn select_last_output(&mut self) -> Option<String> {
        let first = self.first_line();
        let (start, end) = self.prompts.iter().rev().find_map(|p| match p.output {
            Some((s, Some(e))) if e > s && s >= first => Some((s, e)),
            _ => None,
        })?;
        self.selection = Some(Selection { start: (start, 0), end: (end - 1, self.cols - 1) });
        let top = self.lines_pushed - self.display_offset;
        if start < top || start >= top + self.rows {
            self.scroll_to_line(start);
        }
        self.all_dirty = true;
        self.selected_text()
    }
}
