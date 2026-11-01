//! Dispatches `keymap::Action` (keybindings and menu items) to window operations.

use super::search_ui::close_search;
use super::search_ui::refresh_search;
use super::state::State;
use super::tabs::close_active_pane_or_tab;
use super::tabs::cycle_split;
use super::tabs::duplicate_active_tab;
use super::tabs::equalize_splits;
use super::tabs::navigate_split_directional;
use super::tabs::open_fresh_tab;
use super::tabs::resize_active_split;
use super::tabs::select_tab;
use super::tabs::split_active_pane;
use super::tabs::toggle_split_zoom;
use super::terminal_io::write_pane_input;
use super::window::refresh_tab_titles;
use super::window::zoom;

use crate::{config, keymap, macos, update};
use crate::Waker;

use crate::paste::paste_payload;
use crate::platform::clipboard::{copy_to_clipboard, paste_from_clipboard};
use crate::split::{Direction, SplitDirection};

/// Execute a shortcut. Returns `false` when it does not apply in the current state
/// (e.g. switching tabs with a single tab), so the key falls through to the shell.
pub(super) fn run_action(
    s: &mut State,
    action: keymap::Action,
    waker: &Waker,
) -> bool {
    use keymap::Action as A;

    // These work even with no open tab (macOS keeps running without windows).
    match action {
        A::Quit => s.quit_requested = true,
        A::Minimize => s.window.set_minimized(true),
        A::OpenConfig => config::open_config_in_editor(),
        A::ReloadConfig => s.reload_config_requested = true,
        A::NewWindow => s.new_window_requested = true,
        A::ToggleQuickTerminal => s.quick_terminal_requested = true,
        A::NewTab if s.tabs.is_empty() => open_fresh_tab(s, waker),
        A::NewTab if cfg!(target_os = "macos") && s.config.macos_native_tabs => s.native_tab_requested = true,
        A::NewTab => duplicate_active_tab(s, waker),
        A::CheckUpdate => {
            macos::show_progress_dialog("Checking for Updates…", "Checking for new releases of Litty…");
            update::start_interactive_check(waker.proxy.clone(), s.config.auto_update_channel);
        }
        _ if s.tabs.is_empty() => {}
        _ => return run_tab_action(s, action, waker),
    }
    true
}

pub(super) fn run_tab_action(
    s: &mut State,
    action: keymap::Action,
    waker: &Waker,
) -> bool {
    use keymap::Action as A;
    let tabs = s.tabs.len();
    let panes = s.tabs[s.active].panes.len();
    match action {
        A::ClearScreen => {
            if let Some(pane) = s.tabs[s.active].active_pane() {
                pane.grid.lock().clear_screen_and_history();
                write_pane_input(pane,&mut s.ai,b"\x0c");
                s.window.request_redraw();
            }
        }
        A::Find => {
            s.history_search.close();
            s.search.open();
            refresh_search(s);
        }
        A::HistorySearch => {
            if s.search.is_open {
                close_search(s);
            }
            let initial = s.tabs[s.active].active_pane().map(|p| p.grid.lock().current_prompt_input());
            let initial_str = initial.as_deref().unwrap_or("");
            s.history_search.open((!initial_str.is_empty()).then_some(initial_str));
            s.renderer.invalidate();
            s.window.request_redraw();
        }
        A::CloseTab => close_active_pane_or_tab(s),
        A::SplitRight => split_active_pane(s, SplitDirection::Vertical, waker),
        A::SplitDown => split_active_pane(s, SplitDirection::Horizontal, waker),
        A::EqualizeSplits => equalize_splits(s),
        A::ToggleSplitZoom => toggle_split_zoom(s),
        A::JumpToPrompt(delta) => {
            let Some(pane) = s.tabs[s.active].active_pane() else {
                return false;
            };
            if !pane.grid.lock().jump_to_prompt(delta) {
                return false;
            }
            s.window.request_redraw();
        }
        A::SelectLastOutput => {
            let text = s.tabs[s.active].active_pane().and_then(|p| p.grid.lock().select_last_output());
            match text {
                Some(ref t) => copy_to_clipboard(t),
                None => return false,
            }
            s.window.request_redraw();
        }
        A::ToggleInspector => {
            s.inspector = !s.inspector;
            s.renderer.invalidate();
            s.window.request_redraw();
        }
        A::ToggleSecureInput => {
            s.secure_input = !s.secure_input;
            if s.focused {
                macos::set_secure_input(s.secure_input);
            }
            macos::set_secure_input_menu_state(s.secure_input);
            // Force the window title (and its 🔒) to be rewritten.
            s.tab_titles.clear();
            refresh_tab_titles(s);
        }
        A::SelectAll => {
            if let Some(pane) = s.tabs[s.active].active_pane() {
                pane.grid.lock().select_all();
                s.window.request_redraw();
            }
        }
        A::Copy => {
            if let Some(text) = s.tabs[s.active].active_pane().and_then(|p| p.grid.lock().selected_text()) {
                copy_to_clipboard(&text);
            }
        }
        A::Paste if s.search.is_open => {
            if let Some(text) = paste_from_clipboard() {
                s.search.append_str(&text);
                refresh_search(s);
            }
        }
        A::Paste => {
            if let Some(text) = paste_from_clipboard().filter(|t| !t.is_empty())
                && let Some(pane) = s.tabs[s.active].active_pane() {
                    let bracketed = {
                        let mut g = pane.grid.lock();
                        g.reset_display();
                        g.bracketed_paste
                    };
                    let has_newlines = text.contains('\n') || text.contains('\r');
                    if !bracketed && has_newlines {
                        let line_count = text.lines().count().max(2);
                        let preview = text
                            .lines()
                            .take(3)
                            .map(|l| if l.len() > 60 { format!("{}…", &l[..60]) } else { l.to_string() })
                            .collect::<Vec<_>>()
                            .join("\n");
                        let msg = format!(
                            "Pasting {line_count} lines into a shell without bracketed paste may execute commands immediately.\n\nPreview:\n{preview}\n\nDo you want to paste anyway?"
                        );
                        if !macos::confirm("Confirm Multi-Line Paste", &msg, "Paste") {
                            return true;
                        }
                    }
                    write_pane_input(pane,&mut s.ai,&paste_payload(&text, bracketed));
                    s.window.request_redraw();
                }
        }
        // A single in-window tab: move between native macOS tabs, if any.
        A::PrevTab if tabs < 2 => return macos::select_native_tab(&s.window, -1, None),
        A::NextTab if tabs < 2 => return macos::select_native_tab(&s.window, 1, None),
        A::SelectTab(n) if tabs < 2 => return macos::select_native_tab(&s.window, 0, Some(n)),
        A::PrevTab => select_tab(s, (s.active + tabs - 1) % tabs),
        A::NextTab => select_tab(s, (s.active + 1) % tabs),
        A::SelectTab(n) => select_tab(s, if n == 8 { tabs - 1 } else { n }),
        A::PrevSplit | A::NextSplit if panes < 2 => return false,
        A::PrevSplit => cycle_split(s, false),
        A::NextSplit => cycle_split(s, true),
        A::Navigate(dir) => {
            // At the left/right edge, move to the neighbouring tab instead.
            if !navigate_split_directional(s, dir) && tabs > 1 {
                match dir {
                    Direction::Left => select_tab(s, (s.active + tabs - 1) % tabs),
                    Direction::Right => select_tab(s, (s.active + 1) % tabs),
                    _ => {}
                }
            }
        }
        A::ResizeSplit(delta) => resize_active_split(s, delta),
        A::ZoomIn => zoom(s, 1.0),
        A::ZoomOut => zoom(s, -1.0),
        A::ZoomReset => zoom(s, 0.0),
        A::ScrollPageUp | A::ScrollPageDown => {
            if let Some(pane) = s.tabs[s.active].active_pane() {
                let mut g = pane.grid.lock();
                let page = g.rows.saturating_sub(1).max(1) as isize;
                g.scroll_display(if action == A::ScrollPageUp { page } else { -page });
                drop(g);
                s.window.request_redraw();
            }
        }
        A::Quit | A::Minimize | A::OpenConfig | A::ReloadConfig | A::NewTab | A::NewWindow | A::CheckUpdate | A::ToggleQuickTerminal => {}
    }
    true
}
