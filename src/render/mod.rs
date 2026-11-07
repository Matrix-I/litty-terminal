//! GPU rendering: direct Metal / Vulkan pipelines and the single instanced draw
//! call, the glyph atlas and shaper, font discovery, post effects, inline
//! images and frame timing.
//!
//! Layer rule (enforced by the `layering_tests` module): uses only `render`
//! and `terminal`, plus the tolerated edges listed there.

pub(crate) mod atlas;
pub(crate) mod backend;
pub(crate) mod fontdb;
pub(crate) mod frame_metrics;
pub(crate) mod image_layer;
pub(crate) mod postfx;
pub(crate) mod renderer;

/// Everything the renderer draws of the history search popup, borrowed from
/// whoever owns the interactive state.
#[derive(Debug, Clone, Copy)]
pub struct HistoryView<'a> {
    pub is_open: bool,
    pub query: &'a str,
    pub selected: usize,
    pub scroll_offset: usize,
    pub sort_mode: crate::terminal::history::SortMode,
    /// Commands, newest first; `filtered[i].item_index` indexes this.
    pub items: &'a std::collections::VecDeque<String>,
    pub filtered: &'a [crate::terminal::history::MatchResult],
}

impl<'a> HistoryView<'a> {
    /// Number of results for the current query.
    pub fn match_count(&self) -> usize {
        self.filtered.len()
    }

    /// Number of commands in history.
    pub fn total_items(&self) -> usize {
        self.items.len()
    }

    /// The highlighted command, if any.
    pub fn selected_command(&self) -> Option<&'a str> {
        let item = self.filtered.get(self.selected)?;
        self.items.get(item.item_index).map(String::as_str)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneRect {
    pub id: u64,
    pub col: usize,
    pub row: usize,
    pub cols: usize,
    pub rows: usize,
    pub has_border_right: bool,
    pub has_border_bottom: bool,
}
