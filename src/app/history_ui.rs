//! Interactive state of the history search popup (`Ctrl+R`): open / closed, query
//! edits, selection and scrolling. Loading, matching and deleting history lives
//! in `terminal::history`; the renderer only sees a borrowed [`HistoryView`].

use crate::render::HistoryView;
use crate::terminal::history::{HistoryStore, MatchResult, SortMode};

#[derive(Debug)]
pub(super) struct HistorySearch {
    pub(super) is_open: bool,
    pub(super) query: String,
    pub(super) selected: usize,
    pub(super) scroll_offset: usize,
    sort_mode: SortMode,
    pub(super) filtered: Vec<MatchResult>,
    store: HistoryStore,
}

impl HistorySearch {
    pub(super) fn new(waker: crate::Waker) -> Self {
        let mut store = HistoryStore::new();
        if store.set_wake(std::sync::Arc::new(move || waker.wake())).is_err() {
            crate::platform::alert("History unavailable", "Unable to start the history worker.");
        }
        Self::with_store(store)
    }

    fn show_error(&mut self) {
        if let Some(error) = self.store.take_error() {
            crate::platform::alert("History operation failed", error);
        }
    }

    pub(super) fn poll(&mut self) {
        if self.store.poll() { self.refilter(); }
        self.show_error();
    }

    fn with_store(store: HistoryStore) -> Self {
        Self {
            is_open: false,
            query: String::new(),
            selected: 0,
            scroll_offset: 0,
            sort_mode: SortMode::Relevance,
            filtered: Vec::new(),
            store,
        }
    }

    /// What the renderer draws.
    pub(super) fn view(&self) -> HistoryView<'_> {
        HistoryView {
            is_open: self.is_open,
            query: &self.query,
            selected: self.selected,
            scroll_offset: self.scroll_offset,
            sort_mode: self.sort_mode,
            items: &self.store.items,
            filtered: &self.filtered,
        }
    }

    /// Open or reopen the history search popup.
    ///
    /// If `initial_query` is provided (e.g. from the current shell prompt buffer),
    /// the search query is pre-populated and filtered immediately, matching fzf's `--query`.
    pub(super) fn open(&mut self, initial_query: Option<&str>) {
        self.store.reload_if_changed();
        self.show_error();
        self.is_open = true;
        self.query = initial_query.unwrap_or("").trim().to_string();
        self.selected = 0;
        self.scroll_offset = 0;
        self.refilter();
    }

    /// Close the history search popup.
    pub(super) fn close(&mut self) {
        self.is_open = false;
        self.query.clear();
        self.filtered = Vec::new();
    }

    /// Toggle visibility of the history search popup.
    #[allow(dead_code)]
    pub(super) fn toggle(&mut self) {
        if self.is_open {
            self.close();
        } else {
            self.open(None);
        }
    }

    /// Toggle sorting between Relevance (score) and Recency (chronological).
    pub(super) fn toggle_sort(&mut self) {
        self.sort_mode = match self.sort_mode {
            SortMode::Relevance => SortMode::Recency,
            SortMode::Recency => SortMode::Relevance,
        };
        self.refilter();
    }

    /// Record a newly executed command from the active shell session.
    pub(super) fn add_session_command(&mut self, cmd: &str) {
        self.store.add_session_command(cmd);
    }

    /// Delete the currently selected command from memory and disk history.
    pub(super) fn delete_selected(&mut self) -> Option<String> {
        let item_idx = self.filtered.get(self.selected)?.item_index;
        let result = self.store.remove(item_idx);
        self.show_error();
        let removed_cmd = result?;
        self.refilter();
        if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len().saturating_sub(1);
        }
        Some(removed_cmd)
    }

    /// Append a character to the search query and update results.
    pub(super) fn append_char(&mut self, c: char) {
        self.query.push(c);
        self.selected = 0;
        self.scroll_offset = 0;
        self.refilter();
    }

    /// Remove the last character of the search query.
    pub(super) fn backspace(&mut self) {
        if self.query.pop().is_some() {
            self.selected = 0;
            self.scroll_offset = 0;
            self.refilter();
        }
    }

    /// Clear the entire search query.
    pub(super) fn clear_query(&mut self) {
        if !self.query.is_empty() {
            self.query.clear();
            self.selected = 0;
            self.scroll_offset = 0;
            self.refilter();
        }
    }

    /// Select the next matching result item.
    pub(super) fn select_next(&mut self, visible_rows: usize) {
        if self.filtered.is_empty() {
            return;
        }
        if self.selected + 1 < self.filtered.len() {
            self.selected += 1;
            if visible_rows > 0 && self.selected >= self.scroll_offset + visible_rows {
                self.scroll_offset = self.selected + 1 - visible_rows;
            }
        }
    }

    /// Select the previous matching result item.
    pub(super) fn select_prev(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            if self.selected < self.scroll_offset {
                self.scroll_offset = self.selected;
            }
        }
    }

    /// Jump forward by a page of visible results.
    pub(super) fn page_down(&mut self, visible_rows: usize) {
        if self.filtered.is_empty() {
            return;
        }
        let step = visible_rows.max(1);
        self.selected = (self.selected + step).min(self.filtered.len() - 1);
        if visible_rows > 0 && self.selected >= self.scroll_offset + visible_rows {
            self.scroll_offset = self.selected + 1 - visible_rows;
        }
    }

    /// Jump backward by a page of visible results.
    pub(super) fn page_up(&mut self, visible_rows: usize) {
        let step = visible_rows.max(1);
        self.selected = self.selected.saturating_sub(step);
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        }
    }

    /// Jump to the first (top) result item.
    pub(super) fn select_first(&mut self) {
        self.selected = 0;
        self.scroll_offset = 0;
    }

    /// Jump to the last result item.
    pub(super) fn select_last(&mut self, visible_rows: usize) {
        if self.filtered.is_empty() {
            return;
        }
        self.selected = self.filtered.len() - 1;
        if visible_rows > 0 && self.selected >= self.scroll_offset + visible_rows {
            self.scroll_offset = self.selected + 1 - visible_rows;
        }
    }

    /// Return the currently selected command string, if any.
    pub(super) fn selected_command(&self) -> Option<&str> {
        self.view().selected_command()
    }

    /// Recompute matching results against the current query and keep the selection in range.
    fn refilter(&mut self) {
        self.filtered = self.store.filter(&self.query, self.sort_mode);
        if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len().saturating_sub(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search(items: &[&str]) -> HistorySearch {
        // Recorded oldest first so `items` ends up newest first, in the given order.
        let mut store = HistoryStore::isolated();
        for cmd in items.iter().rev() {
            store.add_session_command(cmd);
        }
        let mut s = HistorySearch::with_store(store);
        s.is_open = true;
        s.refilter();
        s
    }

    #[test]
    fn selection_follows_the_filtered_results() {
        let s = search(&["secret_token", "ls -la"]);
        assert_eq!(s.selected_command(), Some("secret_token"));
        assert_eq!(s.view().match_count(), 2);
        assert_eq!(s.view().total_items(), 2);
    }

    #[test]
    fn deleting_the_selection_keeps_the_next_result_selected() {
        let mut s = search(&["litty_test_unique_token_074", "ls -la"]);
        assert_eq!(s.delete_selected(), Some("litty_test_unique_token_074".into()));
        assert_eq!(s.view().match_count(), 1);
        assert_eq!(s.selected_command(), Some("ls -la"));
        // Deleting the last remaining result leaves nothing selected.
        s.select_last(5);
        s.delete_selected();
        assert_eq!(s.selected_command(), None);
        assert_eq!(s.delete_selected(), None);
    }

    #[test]
    fn navigation_scrolls_with_the_selection() {
        let mut s = search(&["cmd 1", "cmd 2", "cmd 3", "cmd 4"]);
        assert_eq!(s.filtered.len(), 4);
        assert_eq!(s.selected, 0);

        s.select_next(2);
        assert_eq!(s.selected, 1);
        s.select_next(2);
        assert_eq!(s.selected, 2);
        assert_eq!(s.scroll_offset, 1);

        s.select_prev();
        assert_eq!(s.selected, 1);
        assert_eq!(s.scroll_offset, 1);

        s.select_prev();
        assert_eq!(s.selected, 0);
        assert_eq!(s.scroll_offset, 0);
    }

    #[test]
    fn paging_and_jumps_clamp_to_the_results() {
        let mut s = search(&["a", "b", "c", "d", "e"]);
        s.page_down(2);
        assert_eq!((s.selected, s.scroll_offset), (2, 1));
        s.page_down(10);
        assert_eq!((s.selected, s.scroll_offset), (4, 1));
        s.page_up(3);
        assert_eq!(s.selected, 1);
        s.select_last(2);
        assert_eq!((s.selected, s.scroll_offset), (4, 3));
        s.select_first();
        assert_eq!((s.selected, s.scroll_offset), (0, 0));
    }

    #[test]
    fn typing_filters_and_resets_the_selection() {
        let mut s = search(&["git status", "ls -la", "git stash"]);
        s.select_next(5);
        s.append_char('g');
        s.append_char('i');
        assert_eq!((s.selected, s.scroll_offset), (0, 0));
        assert_eq!(s.filtered.len(), 2);
        s.backspace();
        s.backspace();
        s.backspace(); // already empty: no change
        assert_eq!(s.filtered.len(), 3);
        s.append_char('z');
        assert!(s.filtered.is_empty());
        assert_eq!(s.selected_command(), None);
        s.clear_query();
        assert_eq!(s.filtered.len(), 3);
    }

    #[test]
    fn sort_toggle_switches_between_relevance_and_recency() {
        let mut s = search(&["xa", "a", "axxx"]);
        s.append_char('a');
        let relevance: Vec<_> = s.filtered.iter().map(|m| m.item_index).collect();
        s.toggle_sort();
        let recency: Vec<_> = s.filtered.iter().map(|m| m.item_index).collect();
        assert_eq!(recency, [0, 1, 2]);
        assert_ne!(relevance, recency, "relevance ranks the exact short match first");
        s.toggle_sort();
        assert_eq!(s.filtered.iter().map(|m| m.item_index).collect::<Vec<_>>(), relevance);
    }

    #[test]
    fn close_discards_the_query_and_results() {
        let mut s = search(&["ls"]);
        s.append_char('l');
        s.close();
        assert!(!s.is_open && s.query.is_empty() && s.filtered.is_empty());
        assert!(!s.view().is_open);
    }

    #[test]
    fn selection_is_clamped_when_the_results_shrink() {
        let mut s = search(&["aa", "ab", "ac"]);
        s.select_last(5);
        assert_eq!(s.selected, 2);
        s.append_char('b'); // resets to the top, one match left
        assert_eq!((s.selected, s.filtered.len()), (0, 1));
    }
}
