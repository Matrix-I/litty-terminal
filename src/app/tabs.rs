//! Tab and split-pane lifecycle and navigation for one window.

use super::window::tab_title;
use super::window::sync_size;
use super::window::spawn_pane;
use super::window::refresh_tab_titles;
use super::close_search;
use super::state::State;

#[cfg(target_os = "macos")]
use crate::macos;
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
    let (cols, rows) = s.renderer.grid_size_for_tabs(false);
    let (cw, ch) = s.renderer.cell_size();
    let tab_id = next_id();
    let pane_id = next_id();
    match spawn_pane(
        pane_id,
        cols,
        rows,
        (cw, ch),
        None,
        Some((s.config.shell_integration, s.config.shell_integration_features)),
        waker,
    ) {
        Ok(pane) => {
            let mut tab = Tab::new(tab_id, pane);
            let title = tab_title(&mut tab);
            s.tabs = vec![tab];
            s.tab_titles = vec![title];
            s.active = 0;
            s.hovered_link = None;
            s.selecting = false;
            s.drag_moved = false;
            s.last_click_time = None;
            s.last_click_cell = None;
            s.active_click_pane_id = None;
            s.cursor_blink_epoch = std::time::Instant::now();
            s.window.set_visible(true);
            s.window.focus_window();
            #[cfg(target_os = "macos")]
            macos::activate_app();
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
    let (cols, rows) = s.renderer.grid_size_for_tabs(true);
    let (cw, ch) = s.renderer.cell_size();
    let id = next_id();
    let pane_id = next_id();
    match spawn_pane(
        pane_id,
        cols,
        rows,
        (cw, ch),
        cwd.as_deref(),
        Some((s.config.shell_integration, s.config.shell_integration_features)),
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
    return macos::confirm(&title, text, button);
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
    s.close_requested = true;
}

pub(super) fn hide_or_exit_last_window(s: &mut State, event_loop: &ActiveEventLoop) {
    if cfg!(target_os = "macos") {
        if s.secure_input && s.focused {
            #[cfg(target_os = "macos")]
            macos::set_secure_input(false);
        }
        s.history_search.close();
        close_search(s);
        s.tab_titles.clear();
        s.hovered_link = None;
        s.selecting = false;
        s.active = 0;
        s.window.set_visible(false);
        s.renderer.shrink_drawables();
        #[cfg(target_os = "macos")]
        macos::release_free_memory();
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
    #[cfg(target_os = "macos")]
    macos::release_free_memory();
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
    #[cfg(target_os = "macos")]
    macos::release_free_memory();
}

pub(super) fn split_active_pane(
    s: &mut State,
    direction: SplitDirection,
    waker: &Waker,
) {
    if s.tabs.is_empty() {
        return;
    }
    let (total_cols, total_rows) = s.renderer.grid_size_for_tabs(s.tabs.len() > 1);
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
        Some((s.config.shell_integration, s.config.shell_integration_features)),
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
    let (total_cols, total_rows) = s.renderer.grid_size_for_tabs(s.tabs.len() > 1);
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
        #[cfg(target_os = "macos")]
        macos::release_free_memory();
    } else {
        close_active_tab(s);
    }
}

pub(super) fn navigate_split_directional(s: &mut State, dir: Direction) -> bool {
    if s.tabs.is_empty() {
        return false;
    }
    let (total_cols, total_rows) = s.renderer.grid_size_for_tabs(s.tabs.len() > 1);
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
