//! Tab strip on the top row: close button, centered title, shortcut badge.

use super::super::Instance;
use super::Cells;

/// Tabs beyond this count are never shifted: the shader holds one shift per tab.
pub const MAX_SHIFTED_TABS: usize = 16;

/// Everything the tab strip draws from, borrowed for one frame.
#[derive(Clone, Copy)]
pub struct TabBarView<'a> {
    pub titles: &'a [String],
    pub active: usize,
    /// Per-tab column shift from its equal-width slot (a dragged or settling tab).
    pub shifts: &'a [i32],
    /// Tab drawn above the others while it is dragged.
    pub dragged: Option<usize>,
    /// The shifts changed since the last frame, so the strip must be rebuilt.
    pub dirty: bool,
}

impl<'a> TabBarView<'a> {
    pub fn new(titles: &'a [String], active: usize) -> Self {
        Self { titles, active, shifts: &[], dragged: None, dirty: false }
    }

    /// Column shift of tab `index`; zero when there are too many tabs to shift.
    pub fn shift(&self, index: usize) -> i32 {
        if self.titles.len() > MAX_SHIFTED_TABS {
            return 0;
        }
        self.shifts.get(index).copied().unwrap_or(0)
    }
}

pub(in crate::render::renderer) fn build(cells: &mut Cells, view: TabBarView) {
    let cols = cells.cols;
    const INACTIVE_FG: u32 = 0x8E90_9980;
    const BAR_BG: u32 = 0x1C1D_2080;

    for dst in &mut cells.instances[0..cols] {
        *dst = Instance { glyph: 0, fg: INACTIVE_FG, bg: BAR_BG };
    }

    let total = available_columns(cols);
    let n = view.titles.len().max(1);
    for i in draw_order(view.titles.len(), view.dragged) {
        draw_tab(cells, view, total, n, i);
    }
}

/// Tab indices in painting order: the dragged tab last, so it covers its neighbours.
fn draw_order(count: usize, dragged: Option<usize>) -> impl Iterator<Item = usize> {
    let dragged = dragged.filter(|&d| d < count);
    (0..count).filter(move |&i| Some(i) != dragged).chain(dragged)
}

fn draw_tab(cells: &mut Cells, view: TabBarView, total: usize, n: usize, i: usize) {
    const ACTIVE_FG: u32 = 0xF2F2_F780;
    const ACTIVE_BADGE_FG: u32 = 0xA6A8_B380;
    const INACTIVE_FG: u32 = 0x8E90_9980;
    const INACTIVE_BADGE_FG: u32 = 0x686A_7380;
    const BAR_BG: u32 = 0x1C1D_2080;

    let Some((c0, c1)) = shifted_span(total, n, i, view.shift(i)) else { return };
    // A tab drawn over a neighbour must not show the neighbour's glyphs.
    for dst in &mut cells.instances[c0..c1] {
        *dst = Instance { glyph: 0, fg: INACTIVE_FG, bg: BAR_BG };
    }
    let span = c1.saturating_sub(c0);
    if span < 4 {
        return;
    }
    let Some(title) = view.titles.get(i) else { return };
    let (fg, badge_fg) = if i == view.active {
        (ACTIVE_FG, ACTIVE_BADGE_FG)
    } else {
        (INACTIVE_FG, INACTIVE_BADGE_FG)
    };

    // Left-aligned close button ('✕' at c0 + 1)
    let left_reserve = if shows_close(span) {
        let close_slot = cells.atlas.slot(cells.queue, '✕' as u32);
        cells.instances[c0 + 1] = Instance { glyph: close_slot as u32, fg: badge_fg, bg: BAR_BG };
        3
    } else {
        1
    };

    // Right-aligned shortcut badge (`⌘1`..`⌘9` on macOS, `^1`..`^9` on Linux).
    let badge_reserve = if i < 9 && span >= 10 {
        #[cfg(target_os = "macos")]
        let mod_ch = '⌘';
        #[cfg(not(target_os = "macos"))]
        let mod_ch = '^';
        let digit = (b'1' + i as u8) as char;
        let bx = c1 - 3;
        let s0 = cells.atlas.slot(cells.queue, mod_ch as u32);
        let s1 = cells.atlas.slot(cells.queue, digit as u32);
        cells.instances[bx] = Instance { glyph: s0 as u32, fg: badge_fg, bg: BAR_BG };
        cells.instances[bx + 1] = Instance { glyph: s1 as u32, fg: badge_fg, bg: BAR_BG };
        4
    } else {
        1
    };

    let avail = span.saturating_sub(left_reserve + badge_reserve);
    if avail == 0 {
        return;
    }
    let label = tab_label(title);
    let count = label.chars().count();
    let truncated = count > avail;
    let shown = count.min(avail);
    let start_col = c0 + left_reserve + (avail.saturating_sub(shown)) / 2;
    for (k, ch) in label.chars().take(shown).enumerate() {
        let ch = if truncated && k + 1 == shown { '…' } else { ch };
        let x = start_col + k;
        if x < c1.saturating_sub(badge_reserve) && x < cells.cols {
            let slot = cells.atlas.slot(cells.queue, ch as u32);
            cells.instances[x] = Instance { glyph: slot as u32, fg, bg: BAR_BG };
        }
    }
}

/// Tab bar label for a window title: drops a leading `user@host:` (or `user:`)
/// prompt prefix so narrow tabs show only the path. The window title keeps the
/// full string.
fn tab_label(title: &str) -> &str {
    if let Some((prefix, rest)) = title.split_once(':')
        && !prefix.is_empty()
            && !prefix.chars().any(char::is_whitespace)
            && (rest.starts_with('~') || (rest.starts_with('/') && !rest.starts_with("//")))
        {
            return rest;
        }
    title
}

#[cfg(test)]
mod tests {
    use super::{draw_order, shifted_span, slot_at, span, tab_label, TabBarView};

    #[test]
    fn test_tab_label_strips_user_host_prefix() {
        assert_eq!(tab_label("linh@mac:~/Desktop/personal/fastdb"), "~/Desktop/personal/fastdb");
        assert_eq!(tab_label("linh:~/src"), "~/src");
        assert_eq!(tab_label("root@box:/etc [Z]"), "/etc [Z]");
        assert_eq!(tab_label("~/src"), "~/src");
        assert_eq!(tab_label("vim: notes.md"), "vim: notes.md");
        assert_eq!(tab_label("shell"), "shell");
        assert_eq!(tab_label("http://x"), "http://x");
    }

    #[test]
    fn slot_at_matches_span_for_every_column() {
        for total in [1, 7, 11, 80, 118] {
            for n in 1..=9 {
                for col in 0..total {
                    let i = slot_at(total, n, col);
                    let (c0, c1) = span(total, n, i);
                    assert!(i < n, "total {total} n {n} col {col}");
                    assert!(c0 <= col && col < c1 || c0 == c1, "total {total} n {n} col {col} -> {i} {c0}..{c1}");
                }
            }
        }
        assert_eq!(slot_at(10, 3, 99), 2, "columns past the strip clamp to the last tab");
    }

    #[test]
    fn shifted_span_moves_and_rejects_out_of_range() {
        assert_eq!(shifted_span(80, 4, 1, 0), Some((20, 40)));
        assert_eq!(shifted_span(80, 4, 1, 15), Some((35, 55)));
        assert_eq!(shifted_span(80, 4, 1, -20), Some((0, 20)));
        assert_eq!(shifted_span(80, 4, 1, -21), None);
        assert_eq!(shifted_span(80, 4, 3, 1), None);
    }

    #[test]
    fn dragged_tab_is_drawn_last() {
        assert_eq!(draw_order(4, None).collect::<Vec<_>>(), [0, 1, 2, 3]);
        assert_eq!(draw_order(4, Some(1)).collect::<Vec<_>>(), [0, 2, 3, 1]);
        assert_eq!(draw_order(4, Some(9)).collect::<Vec<_>>(), [0, 1, 2, 3], "stale index is ignored");
    }

    #[test]
    fn view_ignores_shifts_when_too_many_tabs() {
        let few: Vec<String> = vec![String::new(); 3];
        let many: Vec<String> = vec![String::new(); 17];
        let shifts = [4, -2, 7];
        let view = TabBarView { shifts: &shifts, ..TabBarView::new(&few, 0) };
        assert_eq!([view.shift(0), view.shift(1), view.shift(2), view.shift(3)], [4, -2, 7, 0]);
        let view = TabBarView { shifts: &shifts, ..TabBarView::new(&many, 0) };
        assert_eq!(view.shift(0), 0);
    }
}

/// The two final columns are reserved for New Tab, even in narrow windows.
pub fn available_columns(cols: usize) -> usize { cols.saturating_sub(2).max(1) }

/// Columns `c0..c1` of tab `index` when `count` tabs share `available` columns equally.
pub fn span(available: usize, count: usize, index: usize) -> (usize, usize) {
    let count = count.max(1);
    (index * available / count, (index + 1) * available / count)
}

/// Index of the tab whose slot holds column `col` (the last tab for columns past the strip).
pub fn slot_at(available: usize, count: usize, col: usize) -> usize {
    let count = count.max(1);
    let mut i = (col * count / available.max(1)).min(count - 1);
    let (_, next_start) = span(available, count, i);
    if col >= next_start && i + 1 < count {
        i += 1;
    }
    i
}

/// Slot of tab `index` moved by `shift` columns, or `None` when it would leave the strip.
pub fn shifted_span(available: usize, count: usize, index: usize, shift: i32) -> Option<(usize, usize)> {
    let (c0, c1) = span(available, count, index);
    let c0 = usize::try_from(c0 as i64 + i64::from(shift)).ok()?;
    let c1 = usize::try_from(c1 as i64 + i64::from(shift)).ok()?;
    (c1 <= available).then_some((c0, c1))
}

pub(in crate::render::renderer) fn shows_close(width: usize) -> bool { width >= 6 }

#[cfg(test)]
#[test]
fn shared_geometry_preserves_thresholds_and_uneven_spans() {
    assert_eq!(available_columns(0), 1);
    assert_eq!(available_columns(82), 80);
    assert_eq!(span(11, 3, 0), (0, 3));
    assert_eq!(span(11, 3, 1), (3, 7));
    assert_eq!(span(11, 3, 2), (7, 11));
    assert_eq!(span(10, 0, 0), (0, 10));
    for width in [0, 4, 5, 6, 9, 10] { assert_eq!(shows_close(width), width >= 6); }
}
