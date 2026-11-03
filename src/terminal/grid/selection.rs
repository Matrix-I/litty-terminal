//! Grid selection, hyperlink, and search hits representations, and the `Grid`
//! methods that select text (click, word, line, block) and find in terminal.

use super::*;

/// A text selection. The public API uses visible `(row, col)` coordinates; the grid
/// stores rows as absolute line numbers so the selection stays on its text while
/// output scrolls and while the viewport moves through history.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub start: (usize, usize), // (row, col)
    pub end: (usize, usize),   // (row, col)
}

impl Selection {
    #[inline]
    pub fn ordered(self) -> ((usize, usize), (usize, usize)) {
        if self.start <= self.end {
            (self.start, self.end)
        } else {
            (self.end, self.start)
        }
    }
}

/// Search matches on one visible row (see `Grid::search_hits`).
pub struct SearchHits<'a> {
    pub line: usize,
    /// Index of the focused match within `hits`, if it is on this row.
    pub current: Option<usize>,
    pub hits: &'a [Selection],
}

impl<'a> SearchHits<'a> {
    #[allow(dead_code)]
    pub fn new(line: usize, current: Option<usize>, hits: &'a [Selection]) -> Self {
        SearchHits { line, current, hits }
    }

    #[inline]
    pub fn at(&self, col: usize) -> SearchHit {
        let pos = (self.line, col);
        match self.hits.iter().position(|m| m.start <= pos && pos <= m.end) {
            None => SearchHit::None,
            Some(i) if Some(i) == self.current => SearchHit::Current,
            Some(_) => SearchHit::Match,
        }
    }
}

/// How a cell is highlighted by find-in-terminal.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SearchHit {
    None,
    Match,
    Current,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hyperlink {
    pub row: usize,
    pub start_col: usize,
    pub end_col: usize,
    pub url: String,
    pub is_path: bool,
}

/// Unresolved hyperlink candidate extracted while holding `grid.lock()`.
/// Implicit paths are validated on the filesystem (`resolve`) outside the lock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HyperlinkCandidate {
    Resolved(Hyperlink),
    ImplicitPath {
        row: usize,
        start_col: usize,
        end_col: usize,
        token: String,
    },
}

impl HyperlinkCandidate {
    /// Validate and resolve this candidate into a [`Hyperlink`], performing any
    /// filesystem `Path::exists()` checks without holding `grid.lock()`.
    pub fn resolve(self, cwd: Option<&std::path::Path>) -> Option<Hyperlink> {
        match self {
            HyperlinkCandidate::Resolved(link) => Some(link),
            HyperlinkCandidate::ImplicitPath { row, start_col, end_col, token } => {
                let is_valid_path = if token.starts_with("~/") || token == "~" {
                    if let Ok(home) = std::env::var("HOME") {
                        let rest = token.strip_prefix("~/").unwrap_or("");
                        let full = std::path::Path::new(&home).join(rest);
                        full.exists() || !rest.is_empty()
                    } else {
                        true
                    }
                } else if token.starts_with('/') {
                    token.len() > 1 || std::path::Path::new(&token).exists()
                } else if let Some(base) = cwd {
                    base.join(&token).exists() || std::path::Path::new(&token).exists()
                } else {
                    std::path::Path::new(&token).exists()
                };

                if is_valid_path {
                    Some(Hyperlink {
                        row,
                        start_col,
                        end_col,
                        url: token,
                        is_path: true,
                    })
                } else {
                    None
                }
            }
        }
    }
}


impl Grid {
    /// Set the selection from visible `(row, col)` coordinates.
    pub fn set_selection(&mut self, sel: Option<Selection>) {
        let sel = sel.map(|s| Selection {
            start: (self.abs_line(s.start.0), s.start.1),
            end: (self.abs_line(s.end.0), s.end.1),
        });
        if self.selection != sel {
            self.selection = sel;
            self.all_dirty = true;
        }
    }

    /// Select everything in scrollback and on screen, from the first to the last
    /// line with text (`Cmd+A`).
    pub fn select_all(&mut self) {
        let first = if self.alt_active { self.lines_pushed } else { self.lines_pushed - self.history };
        let has_text = |l: &usize| self.line_cells(*l).is_some_and(|r| r.iter().any(|c| !Self::is_blank(c.codepoint)));
        let lines = first..self.lines_pushed + self.rows;
        let sel = lines.clone().find(has_text).zip(lines.rev().find(has_text)).map(|(top, bottom)| Selection {
            start: (top, 0),
            end: (bottom, self.cols - 1),
        });
        if self.selection != sel {
            self.selection = sel;
            self.all_dirty = true;
        }
    }

    /// Shift+Click: move the selection end nearest to visible `(row, col)` there,
    /// keeping the other end as the anchor. Returns `false` without a selection.
    pub fn extend_selection(&mut self, row: usize, col: usize) -> bool {
        let Some(sel) = self.selection else {
            return false;
        };
        let pos = (self.abs_line(row), col);
        let (a, b) = sel.ordered();
        let offset = |p: (usize, usize)| p.0 * self.cols + p.1;
        let anchor = if offset(pos).abs_diff(offset(a)) <= offset(pos).abs_diff(offset(b)) { b } else { a };
        self.selection = Some(Selection { start: anchor, end: pos });
        self.all_dirty = true;
        true
    }

    /// Dragging after Shift+Click: move the selection end, keeping its anchor
    /// (which may be in scrollback, off screen).
    pub fn set_selection_end(&mut self, row: usize, col: usize) {
        let end = (self.abs_line(row), col);
        if let Some(sel) = self.selection.as_mut().filter(|s| s.end != end) {
            sel.end = end;
            self.all_dirty = true;
        }
    }

    #[inline]
    pub fn is_selected(&self, row: usize, col: usize) -> bool {
        let Some(sel) = self.selection else {
            return false;
        };
        let (start, end) = sel.ordered();
        let pos = (self.abs_line(row), col);
        pos >= start && pos <= end
    }

    #[inline]
    pub(super) fn is_blank(codepoint: u32) -> bool {
        codepoint == 0 || codepoint == b' ' as u32
    }

    #[inline]
    pub(super) fn trim_col_range(row: &[Cell], start: usize, end_exclusive: usize) -> Option<(usize, usize)> {
        let end_exclusive = end_exclusive.min(row.len());
        if start >= end_exclusive {
            return None;
        }
        let slice = &row[start..end_exclusive];
        let first = slice.iter().position(|c| !Self::is_blank(c.codepoint))?;
        let last = slice.iter().rposition(|c| !Self::is_blank(c.codepoint))?;
        Some((start + first, start + last))
    }

    /// Smart double-click selection on `row`:
    /// - On a Powerline prompt line (` ~/path   main ± `), selects `~/path`
    ///   (or the command after the last `` if present and clicked).
    /// - On a command line starting with a shell prompt (`$ cd ...`, `% ...`, `# ...`, `❯ ...`),
    ///   selects the full command line excluding the leading `$ ` prompt (`cd ...`).
    /// - On any other line, selects the trimmed line content.
    pub fn select_smart_line(&mut self, row: usize, col: usize) -> String {
        if row >= self.rows + usize::from(self.scroll_fraction > 0.0) || self.cols == 0 {
            return String::new();
        }
        let r = self.display_row(row);

        // Check for Powerline segment separator U+E0B0 ('')
        let first_pl = r.iter().position(|c| c.codepoint == 0xE0B0);
        let last_pl = r.iter().rposition(|c| c.codepoint == 0xE0B0);
        if let (Some(f_pl), Some(l_pl)) = (first_pl, last_pl) {
            let after_pl = Self::trim_col_range(r, l_pl + 1, self.cols);
            let before_pl = Self::trim_col_range(r, 0, f_pl);
            let chosen = match (before_pl, after_pl) {
                (Some(b), Some(a)) => {
                    if col >= f_pl {
                        Some(a)
                    } else {
                        Some(b)
                    }
                }
                (Some(b), None) => Some(b),
                (None, Some(a)) => Some(a),
                (None, None) => None,
            };
            if let Some(( mut sc, ec)) = chosen {
                if matches!(
                    char::from_u32(r[sc].codepoint),
                    Some('$' | '%' | '#' | '❯' | '>' | '➜')
                ) && sc < ec
                    && Self::is_blank(r[sc + 1].codepoint)
                {
                    sc += 1;
                    while sc <= ec && Self::is_blank(r[sc].codepoint) {
                        sc += 1;
                    }
                }
                if sc <= ec {
                    self.set_selection(Some(Selection {
                        start: (row, sc),
                        end: (row, ec),
                    }));
                    return self.selected_text().unwrap_or_default();
                }
            }
        }

        let Some((mut start_col, end_col)) = Self::trim_col_range(r, 0, self.cols) else {
            self.set_selection(None);
            return String::new();
        };

        // Strip leading shell prompt symbol (`$ `, `% `, `# `, `❯ `, `> `, `➜ `)
        if matches!(
            char::from_u32(r[start_col].codepoint),
            Some('$' | '%' | '#' | '❯' | '>' | '➜')
        ) && start_col < end_col
            && Self::is_blank(r[start_col + 1].codepoint)
        {
            start_col += 1;
            while start_col <= end_col && Self::is_blank(r[start_col].codepoint) {
                start_col += 1;
            }
        }

        if start_col > end_col {
            self.set_selection(None);
            return String::new();
        }

        self.set_selection(Some(Selection {
            start: (row, start_col),
            end: (row, end_col),
        }));
        self.selected_text().unwrap_or_default()
    }

    /// Columns of the run of non-blank cells around `(row, col)`; the placeholder
    /// cell of a wide glyph belongs to the glyph.
    pub(super) fn block_range(&self, row: usize, col: usize) -> Option<(usize, usize)> {
        if row >= self.rows + usize::from(self.scroll_fraction > 0.0) || col >= self.cols {
            return None;
        }
        let r = self.display_row(row);
        let filled = |c: usize| {
            !Self::is_blank(r[c].codepoint) || (c > 0 && r[c].codepoint == 0 && is_wide_cp(r[c - 1].codepoint))
        };
        if !filled(col) {
            return None;
        }
        let mut sc = col;
        while sc > 0 && filled(sc - 1) {
            sc -= 1;
        }
        let mut ec = col;
        while ec + 1 < r.len() && filled(ec + 1) {
            ec += 1;
        }
        Some((sc, ec))
    }

    /// Double-click: select the contiguous block of text at `(row, col)` and return
    /// it. On a blank cell, fall back to the smart line selection.
    pub fn select_block(&mut self, row: usize, col: usize) -> String {
        let Some((sc, ec)) = self.block_range(row, col) else {
            return self.select_smart_line(row, col);
        };
        self.set_selection(Some(Selection { start: (row, sc), end: (row, ec) }));
        self.selected_text().unwrap_or_default()
    }

    /// Whether the selection is exactly the block at `(row, col)`, so a further
    /// double-click there widens it to the line.
    pub fn is_block_selected(&self, row: usize, col: usize) -> bool {
        let (Some(sel), Some((sc, ec))) = (self.selection, self.block_range(row, col)) else {
            return false;
        };
        let line = self.abs_line(row);
        sel.ordered() == ((line, sc), (line, ec))
    }

    /// Select the entire visible line `row` (up to the last non-blank character)
    /// and return its text.
    pub fn select_line(&mut self, row: usize) -> String {
        if row >= self.rows + usize::from(self.scroll_fraction > 0.0) {
            return String::new();
        }
        let r = self.display_row(row);
        let last_non_blank = r
            .iter()
            .rposition(|c| c.codepoint != 0 && c.codepoint != b' ' as u32);
        let Some(end_col) = last_non_blank else {
            self.set_selection(None);
            return String::new();
        };
        self.set_selection(Some(Selection {
            start: (row, 0),
            end: (row, end_col),
        }));
        self.selected_text().unwrap_or_default()
    }

    /// Extract the currently selected text. Soft-wrapped lines are joined without
    /// a newline, so a long command or URL copies as one line.
    pub fn selected_text(&self) -> Option<String> {
        let sel = self.selection?;
        let ((l0, c0), (l1, c1)) = sel.ordered();
        let last_col = self.cols.saturating_sub(1);
        let mut out = String::new();
        for l in l0..=l1 {
            let Some(row) = self.line_cells(l) else { continue };
            let sc = if l == l0 { c0.min(last_col) } else { 0 };
            let ec = if l == l1 { c1.min(last_col) } else { last_col };
            let mut line = String::new();
            if sc <= ec {
                let mut skip_next = false;
                for cell in &row[sc..=ec] {
                    if skip_next {
                        skip_next = false;
                        continue;
                    }
                    skip_next = is_wide_cp(cell.codepoint);
                    if cell.codepoint == 0 {
                        line.push(' ');
                    } else {
                        self.push_cell_text(cell, &mut line);
                    }
                }
            }
            let wraps = l < l1 && self.line_wrapped(l);
            if wraps {
                out.push_str(&line);
            } else if l < l1 || ec == last_col {
                out.push_str(line.trim_end_matches(' '));
            } else {
                out.push_str(&line);
            }
            if l < l1 && !wraps {
                out.push('\n');
            }
        }
        let trimmed = out.trim_end_matches(' ').to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    }

    // ------------------------------------------------------------------
    // Find in terminal
    // ------------------------------------------------------------------

    /// Snapshot of the scrollback and screen for `SearchText::find`, so matching
    /// runs without holding the grid lock.
    pub fn search_text(&self) -> SearchText {
        self.search_text_reusing(SearchText::default())
    }

    pub(crate) fn search_text_reusing(&self, snapshot: SearchText) -> SearchText {
        let first = if self.alt_active { self.lines_pushed } else { self.lines_pushed - self.history };
        let last = self.lines_pushed + self.rows;
        let SearchText { mut text, mut rows, mut columns } = snapshot;
        text.clear();
        rows.clear();
        columns.clear();
        rows.reserve(last - first);
        for l in first..last {
            let Some(cells) = self.line_cells(l) else { continue };
            let row_start = text.len();
            rows.push((row_start, l));
            let mut skip_next = false;
            let mut ascii_run = false;
            let wrapped = self.line_wrapped(l);
            let end = if wrapped { cells.len() } else {
                cells.iter().rposition(|cell| !Self::is_blank(cell.codepoint)).map_or(0, |col| col + 1)
            };
            for (col, cell) in cells[..end].iter().enumerate() {
                if std::mem::take(&mut skip_next) { continue; }
                skip_next = is_wide_cp(cell.codepoint);
                let width = if skip_next { 2 } else { 1 };
                let ascii = cell.codepoint < 128 && width == 1;
                if !ascii || !ascii_run {
                    columns.push((text.len(), col, width | if ascii { crate::terminal::search::ASCII_RUN } else { 0 }));
                }
                ascii_run = ascii;
                self.push_cell_text(cell, &mut text);
            }
            if !self.line_wrapped(l) {
                text.truncate(text.trim_end_matches(' ').len().max(row_start));
                while columns.last().is_some_and(|&(offset, _, _)| offset >= text.len()) { columns.pop(); }
                let col = columns.last().filter(|&&(offset, _, _)| offset >= row_start).map_or(0, |&(offset, col, width)| {
                    if width & crate::terminal::search::ASCII_RUN != 0 { col + text.len() - offset }
                    else { col + width }
                });
                columns.push((text.len(), col, 1));
                text.push('\n');
            }
        }
        SearchText { text, rows, columns }
    }

    /// Replace the search matches and focus the one closest to the bottom of the
    /// viewport (searching upwards from what the user is looking at).
    pub fn set_search_matches(&mut self, matches: Vec<Selection>) {
        let bottom = if self.rows > 0 { self.abs_line(self.rows - 1) } else { 0 };
        self.search_current = match matches.partition_point(|m| m.start.0 <= bottom) {
            0 if matches.is_empty() => None,
            0 => Some(0),
            n => Some(n - 1),
        };
        self.search_matches = matches;
        self.all_dirty = true;
        self.reveal_search_current();
    }

    pub fn clear_search(&mut self) {
        if !self.search_matches.is_empty() {
            self.search_matches = Vec::new();
            self.all_dirty = true;
        }
        self.search_current = None;
    }

    /// Focus the previous (`up`, older) or next match, wrapping around.
    pub fn search_step(&mut self, up: bool) {
        let n = self.search_matches.len();
        let Some(cur) = self.search_current.filter(|_| n > 0) else { return };
        self.search_current = Some(if up { (cur + n - 1) % n } else { (cur + 1) % n });
        self.all_dirty = true;
        self.reveal_search_current();
    }

    /// `(focused index, match count)`.
    pub fn search_status(&self) -> (Option<usize>, usize) {
        (self.search_current, self.search_matches.len())
    }

    /// Scroll the viewport so the focused match is visible (centered when it was not).
    pub(super) fn reveal_search_current(&mut self) {
        let Some(m) = self.search_current.and_then(|i| self.search_matches.get(i)) else { return };
        if self.alt_active {
            return;
        }
        let line = m.start.0;
        let top = self.lines_pushed - self.display_offset;
        if line < top || line >= top + self.rows {
            let offset = (self.lines_pushed + self.rows / 2).saturating_sub(line).min(self.history);
            if offset != self.display_offset || self.scroll_fraction != 0.0 {
                self.display_offset = offset;
                self.scroll_fraction = 0.0;
                self.all_dirty = true;
            }
        }
    }

    /// Matches intersecting visible row `row`, for per-cell highlighting.
    pub fn search_hits(&self, row: usize) -> SearchHits<'_> {
        let line = self.abs_line(row);
        let lo = self.search_matches.partition_point(|m| m.end.0 < line);
        let hi = self.search_matches.partition_point(|m| m.start.0 <= line).max(lo);
        SearchHits {
            line,
            current: self.search_current.and_then(|c| c.checked_sub(lo)),
            hits: &self.search_matches[lo..hi],
        }
    }
}

#[cfg(test)]
impl Grid {
    pub(super) fn dense_search_text(&self) -> SearchText {
        let first = if self.alt_active { self.lines_pushed } else { self.lines_pushed - self.history };
        let last = self.lines_pushed + self.rows;
        let mut text = String::new();
        let mut rows = Vec::with_capacity(last - first);
        let mut columns = Vec::new();
        for l in first..last {
            let Some(cells) = self.line_cells(l) else { continue };
            let row_start = text.len();
            rows.push((row_start, l));
            let mut skip_next = false;
            for (col, cell) in cells.iter().enumerate() {
                if std::mem::take(&mut skip_next) { continue; }
                skip_next = is_wide_cp(cell.codepoint);
                let width = if skip_next { 2 } else { 1 };
                columns.push((text.len(), col, width));
                self.push_cell_text(cell, &mut text);
            }
            if !self.line_wrapped(l) {
                text.truncate(text.trim_end_matches(' ').len().max(row_start));
                while columns.last().is_some_and(|&(offset, _, _)| offset >= text.len()) { columns.pop(); }
                let col = columns.last().filter(|&&(offset, _, _)| offset >= row_start).map_or(0, |&(_, col, width)| col + width);
                columns.push((text.len(), col, 1));
                text.push('\n');
            }
        }
        SearchText { text, rows, columns }
    }

}
