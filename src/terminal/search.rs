//! Find-in-terminal (`Cmd+F` / `Ctrl+Shift+F`): the search box state and regex
//! matching over a snapshot of the scrollback.
//!
//! The grid text is copied out under the lock (`Grid::search_text`) and matched
//! afterwards, so a regex over the whole scrollback never blocks the PTY reader.
//! Matches are stored back in the grid as absolute-line selections, so they stay
//! on their text while output scrolls.

use regex::{Regex, RegexBuilder};

use crate::terminal::grid::Selection;

/// Upper bound on highlighted matches (keeps memory bounded for `.` and friends).
pub const MAX_MATCHES: usize = 10_000;

/// A one-byte, one-cell run maps byte offsets directly to columns.
pub(crate) const ASCII_RUN: usize = 1usize << (usize::BITS - 1);

/// State of the search box overlay.
pub struct SearchBar {
    pub is_open: bool,
    pub query: String,
    /// Interpret the query as a regular expression (otherwise a literal string).
    pub regex: bool,
    /// The query is not a valid regular expression.
    pub invalid: bool,
    /// Pane whose grid currently holds the matches.
    pub pane_id: Option<u64>,
    /// Index of the focused match (top-to-bottom order) and the match count.
    pub current: Option<usize>,
    pub total: usize,
    pub(crate) snapshot: Option<SearchText>,
}

impl SearchBar {
    pub fn new() -> Self {
        SearchBar { is_open: false, query: String::new(), regex: true, invalid: false, pane_id: None, current: None, total: 0, snapshot: None }
    }

    /// Open the box, keeping the previous query (like most terminals and browsers).
    pub fn open(&mut self) {
        self.is_open = true;
    }

    pub fn close(&mut self) {
        self.is_open = false;
        self.snapshot = None;
        self.pane_id = None;
        self.current = None;
        self.total = 0;
    }

    pub fn append_char(&mut self, c: char) {
        if !c.is_control() {
            self.query.push(c);
        }
    }

    /// Append pasted text: only its first line, without control characters.
    pub fn append_str(&mut self, text: &str) {
        text.lines().next().unwrap_or("").chars().for_each(|c| self.append_char(c));
    }

    pub fn backspace(&mut self) {
        self.query.pop();
    }

    pub fn clear_query(&mut self) {
        self.query.clear();
    }

    pub fn toggle_regex(&mut self) {
        self.regex = !self.regex;
    }

    /// Compile the query. Smart case: case-insensitive unless the query has an
    /// uppercase letter. Returns `None` for an empty or invalid query.
    pub fn compile(&mut self) -> Option<Regex> {
        self.invalid = false;
        if self.query.is_empty() {
            return None;
        }
        let pattern = if self.regex { self.query.clone() } else { regex::escape(&self.query) };
        let re = RegexBuilder::new(&pattern)
            .case_insensitive(!self.query.chars().any(char::is_uppercase))
            .size_limit(1 << 20)
            .build();
        self.invalid = re.is_err();
        re.ok()
    }
}

/// Full grapheme text with byte-to-cell mappings from the actual grid.
/// Wide placeholders are skipped; soft wraps join and hard lines use `\n`.
#[derive(Default)]
pub struct SearchText {
    pub text: String,
    /// `(byte offset, absolute line)` where each physical line starts in `text`.
    pub rows: Vec<(usize, usize)>,
    /// Sparse `(byte offset, column, width)` mapping. The high width bit marks
    /// an ASCII run; other entries preserve one stored grapheme's width.
    pub columns: Vec<(usize, usize, usize)>,
}

impl SearchText {
    /// Retain at most 4 MiB between keystrokes; closing or changing panes drops it.
    pub(crate) fn reusable(&self) -> bool {
        const MAX_RETAINED_BYTES: usize = 4 * 1024 * 1024;
        self.text.capacity() + self.rows.capacity() * std::mem::size_of::<(usize, usize)>()
            + self.columns.capacity() * std::mem::size_of::<(usize, usize, usize)>() <= MAX_RETAINED_BYTES
    }

    /// All non-empty matches of `re`, as inclusive `(line, col)` ranges in
    /// top-to-bottom order.
    pub fn find(&self, re: &Regex) -> Vec<Selection> {
        if self.rows.is_empty() || self.columns.is_empty() || self.text.is_empty() {
            return Vec::new();
        }
        re.find_iter(&self.text)
            .filter(|m| !m.is_empty())
            .take(MAX_MATCHES)
            .map(|m| {
                let (line, col, _) = self.cell_at(m.start());
                let start = (line, col);
                let (line, col, width) = self.cell_at(m.end() - 1);
                Selection { start, end: (line, col + width - 1) }
            })
            .collect()
    }

    /// Cell `(line, col)` of the char starting at byte `offset`.
    fn cell_at(&self, offset: usize) -> (usize, usize, usize) {
        let row = self.rows.partition_point(|&(start, _)| start <= offset).saturating_sub(1);
        let cell = self.columns.partition_point(|&(start, _, _)| start <= offset).saturating_sub(1);
        let (_, col, width) = self.columns[cell];
        let linear = width & ASCII_RUN != 0;
        let col = if linear { col + offset - self.columns[cell].0 } else { col };
        (self.rows[row].1, col, width & !ASCII_RUN)
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::grid::Grid;

    fn feed(g: &mut Grid, s: &str) {
        let mut p = vte::Parser::new();
        for b in s.bytes() {
            p.advance(g, b);
        }
    }

    fn search(g: &Grid, query: &str, regex: bool) -> Vec<Selection> {
        let mut bar = SearchBar::new();
        bar.regex = regex;
        bar.query = query.to_string();
        bar.compile().map(|re| g.search_text().find(&re)).unwrap_or_default()
    }

    #[test]
    fn grapheme_search_uses_stored_widths_and_full_cluster_text() {
        for cluster in ["❤️", "#️⃣", "👨‍👩‍👧", "🇻🇳", "👍🏽"] {
            let mut g = Grid::new(40, 2);
            feed(&mut g, &format!("{cluster}x"));
            let after = search(&g, "x", false);
            assert_eq!((after[0].start, after[0].end), ((0, 2), (0, 2)), "after {cluster}");
            let whole = search(&g, cluster, false);
            assert_eq!((whole[0].start, whole[0].end), ((0, 0), (0, 1)), "whole {cluster}");
        }
        let mut g = Grid::new(4, 3);
        feed(&mut g, "❤️xyzz");
        let across = search(&g, "yzz", false);
        assert_eq!((across[0].start, across[0].end), ((0, 3), (1, 1)));
        let variation = search(&g, "\u{fe0f}", false);
        assert_eq!((variation[0].start, variation[0].end), ((0, 0), (0, 1)), "partial cluster still highlights its cell pair");
    }

    #[test]
    fn finds_regex_matches_with_cell_columns() {
        let mut g = Grid::new(20, 4);
        feed(&mut g, "foo bar\r\nbaz foo42\r\n");
        let m = search(&g, r"foo\d*", true);
        assert_eq!(m.len(), 2);
        assert_eq!((m[0].start, m[0].end), ((0, 0), (0, 2)));
        assert_eq!((m[1].start, m[1].end), ((1, 4), (1, 8)));
    }

    #[test]
    fn literal_mode_escapes_and_smart_case_applies() {
        let mut g = Grid::new(20, 4);
        feed(&mut g, "a.b axb A.B\r\n");
        assert_eq!(search(&g, "a.b", true).len(), 3);
        assert_eq!(search(&g, "a.b", false).len(), 2, "literal, case-insensitive");
        assert_eq!(search(&g, "A.B", false).len(), 1, "uppercase makes it case-sensitive");
    }

    #[test]
    fn invalid_regex_is_reported() {
        let mut bar = SearchBar::new();
        bar.query = "(".into();
        assert!(bar.compile().is_none());
        assert!(bar.invalid);
        bar.toggle_regex();
        assert!(bar.compile().is_some());
        assert!(!bar.invalid);
    }

    #[test]
    fn wide_chars_and_soft_wraps_map_to_cells() {
        let mut g = Grid::new(6, 4);
        // "日本x" occupies 5 cells; "abcdefgh" soft-wraps onto the next row.
        feed(&mut g, "日本x\r\nabcdefgh\r\n");
        let m = search(&g, "x", true);
        assert_eq!((m[0].start, m[0].end), ((0, 4), (0, 4)));
        let m = search(&g, "本", true);
        assert_eq!((m[0].start, m[0].end), ((0, 2), (0, 3)));
        let m = search(&g, "efg", true);
        assert_eq!((m[0].start, m[0].end), ((1, 4), (2, 0)), "match spans the wrap");
    }

    #[test]
    fn paste_keeps_only_the_first_line() {
        let mut bar = SearchBar::new();
        bar.append_str("ab\x1bc\nnext");
        assert_eq!(bar.query, "abc");
    }

    #[test]
    fn search_on_empty_grid_does_not_panic() {
        let empty_search_text = SearchText {
            text: String::new(),
            rows: Vec::new(),
            columns: Vec::new(),
        };
        let re = Regex::new("foo").unwrap();
        let matches = empty_search_text.find(&re);
        assert!(matches.is_empty());

        let mut g = Grid::new(0, 0);
        let m = search(&g, "test", true);
        assert!(m.is_empty());
        g.set_search_matches(m);
    }

    #[test]
    fn zero_width_chars_map_to_correct_columns() {
        let mut g = Grid::new(10, 2);
        // 'e' + combining acute accent U+0301 + "target"
        // 'e' is at col 0, accent is zero-width, 't' of "target" starts at col 1
        feed(&mut g, "e\u{0301}target\r\n");
        let m = search(&g, "target", true);
        assert_eq!(m.len(), 1);
        assert_eq!((m[0].start, m[0].end), ((0, 1), (0, 6)));
    }
}

#[cfg(test)]
#[test]
fn ascii_snapshot_mapping_is_sparse() {
    let mut grid = crate::terminal::grid::Grid::new(160, 24);
    let mut parser = vte::Parser::new();
    for _ in 0..5024 {
        for byte in "x".repeat(160).bytes().chain(b"\r\n".iter().copied()) { parser.advance(&mut grid, byte); }
    }
    let snapshot = grid.search_text();
    assert!(snapshot.columns.len() <= snapshot.rows.len() * 2);
    let re = Regex::new("x").unwrap();
    assert_eq!(snapshot.find(&re).len(), MAX_MATCHES);
}

#[cfg(test)]
#[test]
fn search_snapshot_reuse_is_bounded_and_close_releases_buffers() {
    let mut grid = crate::terminal::grid::Grid::new(80, 24);
    let mut parser = vte::Parser::new();
    for byte in b"hello \xe7\x95\x8c" { parser.advance(&mut grid, *byte); }
    let snapshot = grid.search_text();
    let expected = snapshot.text.clone();
    assert!(snapshot.reusable());
    let mut reused = None;
    assert_eq!(crate::test_alloc::allocations(|| { reused = Some(grid.search_text_reusing(snapshot)); }), 0);
    assert_eq!(reused.as_ref().unwrap().text, expected);
    let mut bar = SearchBar::new();
    bar.snapshot = reused;
    bar.close();
    assert!(bar.snapshot.is_none());
    let oversized = SearchText { text: String::with_capacity(4 * 1024 * 1024 + 1), ..SearchText::default() };
    assert!(!oversized.reusable());
}
