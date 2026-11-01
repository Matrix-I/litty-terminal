//! Input-method composition (preedit, commit, candidate window placement).

use super::search_ui::refresh_search;
use super::state::State;
use super::terminal_io::write_pane_input;

use winit::event::Ime;

pub(super) fn handle_ime(s: &mut State, ime: Ime) {
    match ime {
        Ime::Preedit(text, _) => {
            if s.ime_preedit != text {
                s.ime_preedit = text;
                s.cursor_blink_epoch = std::time::Instant::now();
                s.window.request_redraw();
            }
        }
        Ime::Commit(text) => {
            s.ime_preedit.clear();
            if s.tabs.is_empty() || text.is_empty() {
                return;
            }
            if s.history_search.is_open {
                for c in text.chars() {
                    s.history_search.append_char(c);
                }
                s.renderer.invalidate();
            } else if s.search.is_open {
                text.chars().for_each(|c| s.search.append_char(c));
                refresh_search(s);
            } else if let Some(pane) = s.tabs[s.active].active_pane() {
                pane.grid.lock().reset_display();
                write_pane_input(pane,&mut s.ai,text.as_bytes());
            }
            s.cursor_blink_epoch = std::time::Instant::now();
            s.window.request_redraw();
        }
        Ime::Enabled => {}
        Ime::Disabled => {
            if !s.ime_preedit.is_empty() {
                s.ime_preedit.clear();
                s.window.request_redraw();
            }
        }
    }
}
