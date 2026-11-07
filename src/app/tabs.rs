//! Tab and split-pane lifecycle and navigation for one window.

use super::search_ui::close_search;
use super::state::State;
use super::window::refresh_tab_titles;
use super::window::spawn_pane;
use super::window::sync_size;
use super::window::tab_title;
use super::tab_tearoff::{follow_torn_window, tear_off_while_dragging};
use crate::render::renderer::tab_span;
use crate::platform::gui::WindowId;

use crate::platform;
#[cfg(target_os = "linux")]
use crate::platform::process::system_bin;
use crate::{config, split};
use crate::{next_id, Pane, Tab, Waker};

use winit::event_loop::ActiveEventLoop;

use crate::split::{Direction, PaneRect, SplitDirection};

pub(super) fn open_fresh_tab(
    s: &mut State,
    waker: &Waker,
) {
    let sz = s.window.inner_size();
    s.renderer.resize(sz.width, sz.height);
    let (cols, rows) = s.renderer.grid_size_for_tabs(s.bands_for(s.tabs.len() + 1));
    let (cw, ch) = s.renderer.cell_size();
    let tab_id = next_id();
    let pane_id = next_id();
    match spawn_pane(
        pane_id,
        cols,
        rows,
        (cw, ch),
        None,
        Some(s.config.shell_setup()),
        waker,
    ) {
        Ok(pane) => {
            let mut tab = Tab::new(tab_id, pane);
            let title = tab_title(&mut tab);
            s.tabs = vec![tab];
            s.tab_titles = vec![title];
            s.active = 0;
            s.pointer.hovered_link = None;
            s.pointer.selecting = false;
            s.pointer.drag_moved = false;
            s.pointer.last_click_time = None;
            s.pointer.last_click_cell = None;
            s.pointer.active_click_pane_id = None;
            s.cursor_blink_epoch = std::time::Instant::now();
            s.window.set_visible(true);
            s.window.focus_window();
            platform::activate_app();
            sync_size(s);
            s.renderer.invalidate();
            s.window.request_redraw();
        }
        Err(e) => eprintln!("litty: failed to spawn fresh tab: {e}"),
    }
}

pub(super) fn duplicate_active_tab(
    s: &mut State,
    waker: &Waker,
) {
    if s.tabs.is_empty() {
        open_fresh_tab(s, waker);
        return;
    }
    let cwd = s.tabs.get_mut(s.active).and_then(|t| t.active_pane_mut()?.cwd());
    let (cols, rows) = s.renderer.grid_size_for_tabs(s.bands_for(s.tabs.len() + 1));
    let (cw, ch) = s.renderer.cell_size();
    let id = next_id();
    let pane_id = next_id();
    match spawn_pane(
        pane_id,
        cols,
        rows,
        (cw, ch),
        cwd.as_deref(),
        Some(s.config.shell_setup()),
        waker,
    ) {
        Ok(pane) => {
            let mut tab = Tab::new(id, pane);
            let title = tab_title(&mut tab);
            let insert_at = (s.active + 1).min(s.tabs.len());
            s.tabs.insert(insert_at, tab);
            s.tab_titles.insert(insert_at, title);
            s.active = insert_at;
            s.window.set_visible(true);
            sync_size(s);
            s.renderer.invalidate();
            s.window.request_redraw();
        }
        Err(e) => eprintln!("litty: failed to spawn duplicate tab: {e}"),
    }
}

/// Ask before closing `panes` per `confirm-close-surface`; `true` = go ahead.
pub(super) fn confirm_close<'a>(cfg: &config::Config, mut panes: impl Iterator<Item = &'a Pane>, what: &str) -> bool {
    use config::ConfirmClose;
    if cfg.confirm_close_surface == ConfirmClose::Never {
        return true;
    }
    let running = panes.any(|p| p.pty.has_running_process());
    if !running && cfg.confirm_close_surface == ConfirmClose::Running {
        return true;
    }
    let text = if running { "A program is still running. Closing will end it." } else { "The terminal will be closed." };
    let (title, button) = match what {
        "Litty" => ("Quit Litty?".to_string(), "Quit"),
        _ => (format!("Close {what}?"), "Close"),
    };
    #[cfg(target_os = "macos")]
    return platform::confirm(&title, text, button);
    // Linux: zenity when installed; without a dialog tool, close as asked.
    #[cfg(not(target_os = "macos"))]
    std::process::Command::new(system_bin("zenity"))
        .args(["--question", "--title", &title, "--text", text, "--ok-label", button])
        .status()
        .map_or(true, |status| status.success())
}

/// The window's last tab is gone: `App` closes the window, or this was the last
/// window. On macOS the app keeps running in the Dock (at 0% CPU) and a Dock
/// click reopens a fresh tab; elsewhere the process exits.
pub(super) fn close_last_window(s: &mut State) {
    s.requests.close = true;
}

pub(super) fn hide_or_exit_last_window(s: &mut State, event_loop: &ActiveEventLoop) {
    if cfg!(target_os = "macos") {
        if s.secure_input && s.focused {
            platform::set_secure_input(false);
        }
        s.history_search.close();
        close_search(s);
        s.tab_titles.clear();
        s.pointer.hovered_link = None;
        s.pointer.selecting = false;
        s.active = 0;
        s.window.set_visible(false);
        s.renderer.shrink_drawables();
        platform::release_free_memory();
    } else {
        event_loop.exit();
    }
}

pub(super) fn select_tab(s: &mut State, idx: usize) {
    if idx < s.tabs.len() && idx != s.active {
        s.active = idx;
        s.tabs[idx].bell = false;
        refresh_tab_titles(s);
        s.renderer.invalidate();
        s.window.request_redraw();
    }
}

pub(super) fn close_active_tab(s: &mut State) {
    if s.tabs.is_empty() {
        close_last_window(s);
        return;
    }
    let removed = s.tabs.remove(s.active);
    for p in &removed.panes {
        s.invalidate_pane_state(p.id);
    }
    if s.tabs.is_empty() {
        close_last_window(s);
        return;
    }
    if s.active >= s.tabs.len() {
        s.active = s.tabs.len() - 1;
    }
    sync_size(s);
    refresh_tab_titles(s);
    s.renderer.invalidate();
    s.window.request_redraw();
    platform::release_free_memory();
}

pub(super) fn close_tab_at(s: &mut State, idx: usize) {
    if idx >= s.tabs.len() {
        return;
    }
    let tab = &s.tabs[idx];
    if !confirm_close(&s.config, tab.panes.iter(), "this tab") {
        return;
    }
    let removed = s.tabs.remove(idx);
    for p in &removed.panes {
        s.invalidate_pane_state(p.id);
    }
    if s.tabs.is_empty() {
        close_last_window(s);
        return;
    }
    if s.active >= s.tabs.len() {
        s.active = s.tabs.len() - 1;
    } else if idx < s.active {
        s.active -= 1;
    }
    sync_size(s);
    refresh_tab_titles(s);
    s.renderer.invalidate();
    s.window.request_redraw();
    platform::release_free_memory();
}

pub(super) fn split_active_pane(
    s: &mut State,
    direction: SplitDirection,
    waker: &Waker,
) {
    if s.tabs.is_empty() {
        return;
    }
    let (total_cols, total_rows) = s.renderer.grid_size_for_tabs(s.bands());
    let (cw, ch) = s.renderer.cell_size();

    let tab = &mut s.tabs[s.active];
    tab.zoomed_pane_id = None;

    let target_id = tab.active_pane_id;
    let new_id = next_id();

    let cwd = tab.pane(target_id).and_then(|p| p.pty.cwd());

    tab.layout.split_pane(target_id, new_id, direction);

    let rects = tab.compute_pane_rects(total_cols, total_rows);
    let new_rect = rects.iter().find(|r| r.id == new_id).copied().unwrap_or(PaneRect {
        id: new_id,
        col: 0,
        row: 0,
        cols: total_cols.max(1),
        rows: total_rows.max(1),
        has_border_right: false,
        has_border_bottom: false,
    });

    match spawn_pane(
        new_id,
        new_rect.cols,
        new_rect.rows,
        (cw, ch),
        cwd.as_deref(),
        Some(s.config.shell_setup()),
        waker,
    ) {
        Ok(new_pane) => {
            tab.panes.push(new_pane);
            tab.active_pane_id = new_id;
            sync_size(s);
            refresh_tab_titles(s);
            s.renderer.invalidate();
            s.window.request_redraw();
        }
        Err(e) => {
            tab.layout.remove_pane(new_id);
            eprintln!("litty: failed to spawn split pane: {e}");
        }
    }
}

pub(super) fn close_active_pane_or_tab(s: &mut State) {
    if s.tabs.is_empty() {
        close_last_window(s);
        return;
    }
    let Some(pane) = s.tabs[s.active].active_pane() else {
        s.tabs.remove(s.active);
        if s.tabs.is_empty() {
            close_last_window(s);
        } else if s.active >= s.tabs.len() {
            s.active = s.tabs.len() - 1;
        }
        return;
    };
    if !confirm_close(&s.config, std::iter::once(pane), "this terminal") {
        return;
    }
    let (total_cols, total_rows) = s.renderer.grid_size_for_tabs(s.bands());
    let tab = &mut s.tabs[s.active];
    if tab.panes.len() > 1 {
        let closing_id = tab.active_pane_id;
        let rects = tab.compute_pane_rects(total_cols, total_rows);

        let mut ids = Vec::new();
        tab.layout.all_pane_ids(&mut ids);
        let cur_idx = ids.iter().position(|&id| id == closing_id);
        let prev_in_order = cur_idx.and_then(|idx| {
            if idx > 0 {
                Some(ids[idx - 1])
            } else if ids.len() > 1 {
                Some(ids[1])
            } else {
                None
            }
        });

        let neighbor = split::find_directional_neighbor(&rects, closing_id, Direction::Up)
            .or_else(|| split::find_directional_neighbor(&rects, closing_id, Direction::Left))
            .or(prev_in_order)
            .or_else(|| split::find_directional_neighbor(&rects, closing_id, Direction::Right))
            .or_else(|| split::find_directional_neighbor(&rects, closing_id, Direction::Down));

        tab.panes.retain(|p| p.id != closing_id);
        tab.layout.remove_pane(closing_id);
        if tab.zoomed_pane_id == Some(closing_id) {
            tab.zoomed_pane_id = None;
        }
        if let Some(nid) = neighbor.filter(|id| tab.panes.iter().any(|p| p.id == *id)) {
            tab.active_pane_id = nid;
        } else if let Some(first_pane) = tab.panes.first() {
            tab.active_pane_id = first_pane.id;
        }
        s.invalidate_pane_state(closing_id);
        sync_size(s);
        refresh_tab_titles(s);
        s.renderer.invalidate();
        s.window.request_redraw();
        platform::release_free_memory();
    } else {
        close_active_tab(s);
    }
}

pub(super) fn navigate_split_directional(s: &mut State, dir: Direction) -> bool {
    if s.tabs.is_empty() {
        return false;
    }
    let (total_cols, total_rows) = s.renderer.grid_size_for_tabs(s.bands());
    let tab = &mut s.tabs[s.active];
    if tab.panes.len() <= 1 {
        return false;
    }
    if tab.zoomed_pane_id.is_some() {
        tab.zoomed_pane_id = None;
    }
    let rects = tab.compute_pane_rects(total_cols, total_rows);
    if let Some(next_id) = split::find_directional_neighbor(&rects, tab.active_pane_id, dir) {
        tab.active_pane_id = next_id;
        refresh_tab_titles(s);
        s.renderer.invalidate();
        s.window.request_redraw();
        true
    } else {
        false
    }
}

pub(super) fn cycle_split(s: &mut State, forward: bool) {
    if s.tabs.is_empty() {
        return;
    }
    let tab = &mut s.tabs[s.active];
    if tab.panes.len() <= 1 {
        return;
    }
    if tab.zoomed_pane_id.is_some() {
        tab.zoomed_pane_id = None;
    }
    let mut ids = Vec::new();
    tab.layout.all_pane_ids(&mut ids);
    if ids.is_empty() {
        return;
    }
    let cur_pos = ids.iter().position(|&id| id == tab.active_pane_id).unwrap_or(0);
    let next_pos = if forward {
        (cur_pos + 1) % ids.len()
    } else {
        (cur_pos + ids.len() - 1) % ids.len()
    };
    tab.active_pane_id = ids[next_pos];
    refresh_tab_titles(s);
    s.renderer.invalidate();
    s.window.request_redraw();
}

pub(super) fn toggle_split_zoom(s: &mut State) {
    if s.tabs.is_empty() {
        return;
    }
    let tab = &mut s.tabs[s.active];
    if tab.panes.len() <= 1 {
        return;
    }
    if tab.zoomed_pane_id.is_some() {
        tab.zoomed_pane_id = None;
    } else {
        tab.zoomed_pane_id = Some(tab.active_pane_id);
    }
    sync_size(s);
    refresh_tab_titles(s);
    s.renderer.invalidate();
    s.window.request_redraw();
}

pub(super) fn equalize_splits(s: &mut State) {
    if s.tabs.is_empty() {
        return;
    }
    let tab = &mut s.tabs[s.active];
    if tab.panes.len() <= 1 {
        return;
    }
    tab.layout.equalize();
    sync_size(s);
    s.renderer.invalidate();
    s.window.request_redraw();
}

pub(super) fn resize_active_split(s: &mut State, delta: f32) {
    if s.tabs.is_empty() {
        return;
    }
    let tab = &mut s.tabs[s.active];
    if tab.panes.len() <= 1 {
        return;
    }
    if tab.layout.adjust_ratio(tab.active_pane_id, delta) {
        sync_size(s);
        s.renderer.invalidate();
        s.window.request_redraw();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tab_active_pane_empty_panes_does_not_panic() {
        use crate::split::SplitNode;
        let mut tab = Tab {
            id: 1,
            panes: Vec::new(),
            layout: SplitNode::new_leaf(1),
            active_pane_id: 1,
            zoomed_pane_id: None,
            bell: false,
        };
        assert!(tab.active_pane().is_none());
        assert!(tab.active_pane_mut().is_none());
    }
}

/// Move the item at `from` to `to`, shifting those in between by one.
fn rotate_range<T>(items: &mut [T], from: usize, to: usize) {
    if from < to {
        items[from..=to].rotate_left(1);
    } else {
        items[to..=from].rotate_right(1);
    }
}

/// Move a tab without changing the identity of the active session.
fn move_tab<T>(tabs: &mut [T], active: &mut usize, from: usize, to: usize) -> bool {
    if from >= tabs.len() || to >= tabs.len() || from == to {
        return false;
    }
    rotate_range(tabs, from, to);
    *active = if *active == from {
        to
    } else if from < *active && *active <= to {
        *active - 1
    } else if to <= *active && *active < from {
        *active + 1
    } else {
        *active
    };
    true
}

/// A drag follows the session ID even if tabs are inserted or closed meanwhile.
pub(super) struct TabDrag {
    id: u64,
    /// Pointer x at the press; `None` once the drag threshold has been passed.
    start_x: Option<f64>,
    /// Columns between the tab's left edge and the pointer when it was grabbed.
    grab_col: f64,
    detach: Detach,
    /// Where the last swap happened, so the opposite swap needs a real retreat.
    swap: Option<Swap>,
}

/// A swap of the dragged tab: its left column then, and the direction it moved (+1 right).
#[derive(Clone, Copy, PartialEq, Debug)]
struct Swap {
    left: i32,
    dir: i32,
}

/// Whether the held tab has left the tab bar for a window of its own.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Detach {
    No,
    /// Taken out of this window; `App` has not opened its window yet.
    Pending,
    /// Held in this window, whose events keep arriving here while the pointer moves.
    Following(WindowId),
}

impl TabDrag {
    fn new(id: u64, x: f64, grab_col: f64) -> Self {
        Self { id, start_x: Some(x), grab_col, detach: Detach::No, swap: None }
    }

    /// The drag continues in a window whose tab bar the tab entered, held `grab_col` columns
    /// from the tab's left edge. The press threshold was passed long ago.
    pub(super) fn resume(mut self, grab_col: f64) -> Self {
        self.start_x = None;
        self.grab_col = grab_col;
        self.detach = Detach::No;
        self.swap = None;
        self
    }

    /// Session ID of the held tab.
    pub(super) fn id(&self) -> u64 {
        self.id
    }

    /// The tab left this window; its new window is moved with the pointer from now on.
    pub(super) fn mark_detached(&mut self) {
        self.detach = Detach::Pending;
    }

    pub(super) fn is_detached(&self) -> bool {
        self.detach != Detach::No
    }

    /// `App` opened the window that now holds the tab.
    pub(super) fn follow(&mut self, window: WindowId) {
        if self.detach == Detach::Pending {
            self.detach = Detach::Following(window);
        }
    }

    /// The window to move under the pointer, once it exists.
    pub(super) fn follower(&self) -> Option<WindowId> {
        match self.detach {
            Detach::Following(window) => Some(window),
            Detach::No | Detach::Pending => None,
        }
    }

    fn update(&mut self, x: f64, scale: f64) -> bool {
        const DRAG_THRESHOLD: f64 = 5.0;
        if self.start_x.is_some_and(|start| (x - start).abs() < DRAG_THRESHOLD * scale) {
            return false;
        }
        self.start_x = None;
        true
    }
}

/// Left column of a dragged tab `width` columns wide: it follows the pointer
/// (which holds it `grab_col` columns from its left edge) but stays inside the strip.
fn dragged_left_col(pointer_col: f64, grab_col: f64, width: usize, total: usize) -> i32 {
    let max_left = total.saturating_sub(width) as f64;
    (pointer_col - grab_col).round().clamp(0.0, max_left) as i32
}

/// Share of its own width a dragged tab must overlap a neighbour's slot before they swap.
const SWAP_OVERLAP: f64 = 0.2;
/// Share of its own width the tab must retreat after a swap before it may swap back.
/// A swap moves the tab by a whole slot, so right after it the tab overlaps the slot it
/// just left by far more than `SWAP_OVERLAP`; without this margin it would swap straight back.
const SWAP_RETREAT: f64 = 0.1;

/// Slot a tab dragged from `from` (left edge at column `left`) belongs in: it moves past
/// every neighbour whose slot it overlaps by at least `SWAP_OVERLAP` of its own width,
/// except that undoing the `last` swap needs a retreat of `SWAP_RETREAT`.
fn reorder_target(left: i32, from: usize, n: usize, total: usize, last: Option<Swap>) -> usize {
    let (c0, c1) = tab_span(total, n, from);
    let width = (c1 - c0) as f64;
    let (l, right) = (f64::from(left), f64::from(left) + width);
    let need = SWAP_OVERLAP * width;
    let mut to = from;
    for j in from + 1..n {
        let (j0, _) = tab_span(total, n, j);
        if right - j0 as f64 >= need { to = j } else { break }
    }
    if to == from {
        for j in (0..from).rev() {
            let (_, j1) = tab_span(total, n, j);
            if j1 as f64 - l >= need { to = j } else { break }
        }
    }
    let dir = (to as i32 - from as i32).signum();
    // Columns the tab has moved back against the direction of the last swap.
    let retreat = last.filter(|swap| swap.dir == -dir).map(|swap| f64::from((swap.left - left) * swap.dir));
    if retreat.is_some_and(|columns| columns < SWAP_RETREAT * width) {
        return from;
    }
    to
}

/// Start tracking a press on tab `index`; it becomes a drag once the pointer moves.
pub(super) fn begin_tab_drag(s: &mut State, index: usize) {
    let total = s.renderer.tab_cols_total();
    let (c0, _) = tab_span(total, s.tabs.len(), index);
    let grab_col = s.renderer.column_at(s.pointer.mouse_pos.0) - c0 as f64;
    s.pointer.tab_drag = Some(TabDrag::new(s.tabs[index].id, s.pointer.mouse_pos.0, grab_col));
}

/// Finish a drag: the tab slides from where it was dropped into its slot.
/// Returns whether a press on a tab was being tracked.
pub(super) fn end_tab_drag(s: &mut State) -> bool {
    let tracked = s.pointer.tab_drag.take().is_some();
    if s.tab_anim.dragged().is_some() {
        s.tab_anim.end_drag();
        s.window.request_redraw();
    }
    if tracked {
        // The drag may have changed which tab is active; the title bar and window
        // title would otherwise stay stale until the next click.
        refresh_tab_titles(s);
    }
    tracked
}

/// Pointer moved while a tab is held: follow it, and swap with a neighbour when
/// the dragged tab's centre crosses into that neighbour's slot.
pub(super) fn drag_tab(s: &mut State) {
    let Some(drag) = s.pointer.tab_drag.as_mut() else { return };
    let x = s.pointer.mouse_pos.0;
    if !drag.update(x, s.window.scale_factor()) {
        return;
    }
    if drag.is_detached() {
        follow_torn_window(s);
        return;
    }
    let (id, grab_col) = (drag.id, drag.grab_col);
    let Some(from) = s.tabs.iter().position(|tab| tab.id == id) else {
        end_tab_drag(s);
        return;
    };
    s.tab_anim.begin_drag(from);
    if tear_off_while_dragging(s) {
        return;
    }
    let (n, total) = (s.tabs.len(), s.renderer.tab_cols_total());
    let pointer_col = s.renderer.column_at(x);
    let (c0, c1) = tab_span(total, n, from);
    let mut left = dragged_left_col(pointer_col, grab_col, c1 - c0, total);

    let last = s.pointer.tab_drag.as_ref().and_then(|drag| drag.swap);
    let to = reorder_target(left, from, n, total, last);
    let mut slot = c0;
    if to != from && move_tab(&mut s.tabs, &mut s.active, from, to) {
        // Titles are cached in tab order; moving them avoids querying PTY state.
        if from.max(to) < s.tab_titles.len() {
            rotate_range(&mut s.tab_titles, from, to);
        }
        s.tab_anim.moved(from, to, total);
        refresh_tab_titles(s);
        if let Some(drag) = s.pointer.tab_drag.as_mut() {
            drag.swap = Some(Swap { left, dir: (to as i32 - from as i32).signum() });
        }
        let (n0, n1) = tab_span(total, n, to);
        slot = n0;
        // The new slot can be a column wider, so the clamp must use its width.
        left = dragged_left_col(pointer_col, grab_col, n1 - n0, total);
    }
    s.tab_anim.set_dragged_shift(left - slot as i32);
    s.window.request_redraw();
}

#[cfg(test)]
mod reorder_tests {
    use super::{dragged_left_col, move_tab, reorder_target, Swap, TabDrag};

    #[test]
    fn a_twenty_percent_overlap_swaps_with_the_neighbour() {
        // Four tabs of 20 columns in an 80-column strip.
        assert_eq!(reorder_target(20, 1, 4, 80, None), 1, "exactly in its own slot");
        assert_eq!(reorder_target(23, 1, 4, 80, None), 1, "3 of 20 columns into the next slot");
        assert_eq!(reorder_target(24, 1, 4, 80, None), 2, "4 of 20 columns into the next slot");
        assert_eq!(reorder_target(17, 1, 4, 80, None), 1, "3 of 20 columns into the previous slot");
        assert_eq!(reorder_target(16, 1, 4, 80, None), 0, "4 of 20 columns into the previous slot");
        assert_eq!(reorder_target(60, 0, 4, 80, None), 3, "a long drag passes several tabs at once");
    }

    #[test]
    fn swapping_back_needs_a_retreat_so_the_tab_does_not_flap() {
        let swap = Some(Swap { left: 24, dir: 1 });
        // Just swapped right at left=24: the tab now sits at index 2 but over slot 1.
        assert_eq!(reorder_target(24, 2, 4, 80, swap), 2, "no retreat yet");
        assert_eq!(reorder_target(23, 2, 4, 80, swap), 2, "1 column back is not enough");
        assert_eq!(reorder_target(22, 2, 4, 80, swap), 1, "2 columns (10%) back swaps again");
        assert_eq!(reorder_target(30, 2, 4, 80, swap), 2, "moving on to the right keeps the slot");
    }

    #[test]
    fn sweeping_the_pointer_one_way_swaps_each_slot_once() {
        let (n, total) = (4, 80);
        for (range, dir) in [((0..60).collect::<Vec<i32>>(), 1), ((0..60).rev().collect(), -1)] {
            let mut at = if dir == 1 { 0 } else { 3 };
            let mut last = None;
            let mut swaps = 0;
            for left in range {
                let to = reorder_target(left, at, n, total, last);
                if to != at {
                    last = Some(Swap { left, dir: (to as i32 - at as i32).signum() });
                    at = to;
                    swaps += 1;
                }
            }
            assert_eq!(swaps, 3, "dir={dir}");
            assert_eq!(at, if dir == 1 { 3 } else { 0 });
        }
    }

    #[test]
    fn a_resumed_drag_is_attached_and_active_at_once() {
        let mut drag = TabDrag::new(7, 100.0, 3.0);
        drag.mark_detached();
        assert!(drag.is_detached());
        let mut drag = drag.resume(9.0);
        assert!(!drag.is_detached(), "the tab sits in a tab bar again");
        assert_eq!((drag.id(), drag.grab_col), (7, 9.0));
        assert!(drag.update(100.0, 1.0), "no press threshold is applied again");
    }

    #[test]
    fn dragged_tab_follows_the_pointer_and_stays_inside_the_strip() {
        // Grabbed 3 columns from its left edge, 20 columns wide, 80-column strip.
        assert_eq!(dragged_left_col(33.0, 3.0, 20, 80), 30);
        assert_eq!(dragged_left_col(33.4, 3.0, 20, 80), 30, "rounds to a whole column");
        assert_eq!(dragged_left_col(33.6, 3.0, 20, 80), 31);
        assert_eq!(dragged_left_col(-40.0, 3.0, 20, 80), 0, "clamped at the left edge");
        assert_eq!(dragged_left_col(500.0, 3.0, 20, 80), 60, "clamped so the tab ends at the strip");
        assert_eq!(dragged_left_col(10.0, 0.0, 90, 80), 0, "a tab wider than the strip stays at 0");
        // Slots of an 81-column strip with two tabs are 40 and 41 wide: the clamp
        // must use the width of the slot the tab now occupies.
        assert_eq!(dragged_left_col(200.0, 0.0, 40, 81), 41);
        assert_eq!(dragged_left_col(200.0, 0.0, 41, 81), 40);
    }

    #[test]
    fn a_drag_starts_attached_and_follows_only_after_it_is_detached() {
        let mut drag = TabDrag::new(7, 0.0, 0.0);
        assert!(!drag.is_detached());
        assert_eq!(drag.follower(), None);
        drag.mark_detached();
        assert!(drag.is_detached());
        assert_eq!(drag.follower(), None, "its window does not exist yet");
    }

    #[test]
    fn drag_threshold_scales_and_stays_started_when_returning_to_origin() {
        for scale in [1.0, 2.0] {
            for direction in [-1.0, 1.0] {
                let mut drag = TabDrag::new(42, 100.0, 0.0);
                assert!(!drag.update(100.0 + direction * 4.0 * scale, scale));
                assert!(drag.update(100.0 + direction * 5.0 * scale, scale));
                assert!(drag.update(100.0, scale));
                assert_eq!(drag.id, 42);
            }
        }
    }

    #[test]
    fn reordering_preserves_order_and_active_identity_in_both_directions() {
        for from in 0..4 {
            for to in 0..4 {
                for selected in 0..4 {
                    let mut tabs = [0, 1, 2, 3];
                    let mut active = selected;
                    assert_eq!(move_tab(&mut tabs, &mut active, from, to), from != to);
                    let mut expected = vec![0, 1, 2, 3];
                    let moved = expected.remove(from);
                    expected.insert(to, moved);
                    assert_eq!(tabs.as_slice(), expected);
                    assert_eq!(tabs[active], selected);
                }
            }
        }
    }

    #[test]
    fn invalid_and_empty_moves_leave_state_unchanged() {
        let mut tabs = [1, 2];
        let mut active = 1;
        assert!(!move_tab(&mut tabs, &mut active, 2, 0));
        assert!(!move_tab(&mut tabs, &mut active, 0, 2));
        assert_eq!(tabs, [1, 2]);
        assert_eq!(active, 1);
        assert!(!move_tab::<u8>(&mut [], &mut 0, 0, 0));
    }
}
