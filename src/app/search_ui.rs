//! Find box and history search popup key handling.

use super::state::State;
use super::terminal_io::write_pane_input;

use crate::keymap;

use winit::keyboard::{Key, NamedKey};

use crate::paste::history_insert_payload;

/// Keys while the history search popup is open. Returns `false` for keys the popup
/// does not consume (e.g. `Cmd+V`), which then go through normal shortcut handling.
pub(super) fn handle_history_search_key(s: &mut State, key: &Key, m: keymap::Mods) -> bool {
    let visible_rows = s.renderer.history_popup_geometry(&s.history_search.view()).map_or(10, |g| g.4);
    let hs = &mut s.history_search;
    match key {
        Key::Named(NamedKey::Escape) => hs.close(),
        Key::Named(k @ (NamedKey::Enter | NamedKey::Tab)) => {
            let cmd = hs.selected_command().map(str::to_string);
            hs.close();
            if let (Some(cmd), Some(tab)) = (cmd, s.tabs.get(s.active)) {
                // Enter runs it, Tab leaves it for editing.
                if let Some(pane) = tab.active_pane() {
                    let bracketed = pane.grid.lock().modes.bracketed_paste;
                    write_pane_input(pane,&mut s.ai,&history_insert_payload(&cmd, *k == NamedKey::Enter, bracketed));
                }
            }
        }
        Key::Named(NamedKey::Delete) => {
            hs.delete_selected();
        }
        Key::Named(NamedKey::ArrowDown) => hs.select_next(visible_rows),
        Key::Named(NamedKey::ArrowUp) => hs.select_prev(),
        Key::Named(NamedKey::PageDown) => hs.page_down(visible_rows),
        Key::Named(NamedKey::PageUp) => hs.page_up(visible_rows),
        Key::Named(NamedKey::Home) => hs.select_first(),
        Key::Named(NamedKey::End) => hs.select_last(visible_rows),
        Key::Named(NamedKey::Backspace) => hs.backspace(),
        Key::Character(ch) if m.ctrl => match ch.as_str() {
            // fzf bindings: Ctrl+R toggles relevance/recency sorting, Ctrl+D deletes.
            "r" => hs.toggle_sort(),
            "d" => {
                hs.delete_selected();
            }
            "n" | "j" => hs.select_next(visible_rows),
            "p" | "k" => hs.select_prev(),
            "c" | "g" => hs.close(),
            "u" | "w" => hs.clear_query(),
            _ => return true,
        },
        Key::Character(ch) if m.alt && ch.eq_ignore_ascii_case("s") => hs.toggle_sort(),
        Key::Character(ch) if !m.sup && !m.alt => ch.chars().for_each(|c| hs.append_char(c)),
        Key::Character(_) => return false,
        _ => return true,
    }
    s.renderer.invalidate();
    s.window.request_redraw();
    true
}

/// Keys while the find box is open. Returns `false` for keys it does not consume
/// (e.g. `Cmd+V`, `Ctrl+Shift+V`, `Cmd+1`), which go through shortcut handling.
pub(super) fn handle_search_key(s: &mut State, key: &Key, m: keymap::Mods) -> bool {
    let plain_ctrl = m.ctrl && !m.shift && !m.alt && !m.sup;
    match key {
        Key::Named(NamedKey::Escape) => close_search(s),
        // Enter searches upwards (older output), Shift+Enter downwards.
        Key::Named(NamedKey::Enter) => search_step(s, !m.shift),
        Key::Named(NamedKey::ArrowUp) => search_step(s, true),
        Key::Named(NamedKey::ArrowDown) => search_step(s, false),
        Key::Named(NamedKey::Backspace) => {
            if m.ctrl || m.alt || m.sup {
                s.search.clear_query();
            } else {
                s.search.backspace();
            }
            refresh_search(s);
        }
        Key::Named(NamedKey::Space) if !m.ctrl && !m.sup => {
            s.search.append_char(' ');
            refresh_search(s);
        }
        // Cmd+G / Cmd+Shift+G (Super on Linux): find next / previous.
        Key::Character(ch) if m.sup && !m.ctrl && !m.alt && ch.eq_ignore_ascii_case("g") => search_step(s, !m.shift),
        Key::Character(ch) if plain_ctrl || (m.alt && !m.ctrl && !m.sup && ch.eq_ignore_ascii_case("r")) => {
            match ch.to_lowercase().as_str() {
                "r" => {
                    s.search.toggle_regex();
                    refresh_search(s);
                }
                "u" | "w" => {
                    s.search.clear_query();
                    refresh_search(s);
                }
                "c" | "g" => close_search(s),
                "p" => search_step(s, true),
                "n" => search_step(s, false),
                _ => {}
            }
        }
        Key::Character(ch) if !m.sup && !m.ctrl => {
            ch.chars().for_each(|c| s.search.append_char(c));
            refresh_search(s);
        }
        Key::Character(_) => return false,
        _ => {}
    }
    true
}

/// Re-run the find query on the active pane, moving the matches off a pane that
/// lost focus.
pub(super) fn refresh_search(s: &mut State) {
    let Some(tab) = s.tabs.get(s.active) else { return };
    let Some(pane) = tab.active_pane() else { return };
    if let Some(old) = s.search.pane_id.filter(|&id| id != pane.id)
        && let Some(p) = s.tabs.iter().flat_map(|t| &t.panes).find(|p| p.id == old) {
            p.grid.lock().clear_search();
        }
    if s.search.pane_id != Some(pane.id) { s.search.snapshot = None; }
    s.search.pane_id = Some(pane.id);
    // Copy the text under the lock, match outside it.
    let matches = match s.search.compile() {
        Some(re) => {
            let snapshot = s.search.snapshot.take();
            let text = {
                let grid = pane.grid.lock();
                match snapshot { Some(snapshot) => grid.search_text_reusing(snapshot), None => grid.search_text() }
            };
            let matches = text.find(&re);
            if text.reusable() { s.search.snapshot = Some(text); }
            matches
        }
        None => Vec::new(),
    };
    {
        let mut g = pane.grid.lock();
        if matches.is_empty() {
            g.clear_search();
        } else {
            g.set_search_matches(matches);
        }
        (s.search.current, s.search.total) = g.search_status();
    }
    s.renderer.invalidate();
    s.window.request_redraw();
}

pub(super) fn search_step(s: &mut State, up: bool) {
    if let Some(tab) = s.tabs.get(s.active)
        && let Some(pane) = tab.active_pane() {
            pane.grid.lock().search_step(up);
            s.window.request_redraw();
        }
}

pub(super) fn close_search(s: &mut State) {
    if let Some(id) = s.search.pane_id
        && let Some(p) = s.tabs.iter().flat_map(|t| &t.panes).find(|p| p.id == id) {
            p.grid.lock().clear_search();
        }
    s.search.close();
    s.renderer.invalidate();
    s.window.request_redraw();
}
