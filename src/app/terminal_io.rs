//! PTY writes and draining terminal-originated requests (title, cwd, bell, OSC 52, notifications).

use super::state::State;
use super::window::refresh_tab_titles;

use crate::{ai, macos};
use crate::Pane;

use winit::window::CursorIcon;

use crate::platform::clipboard::copy_to_clipboard;
use crate::platform::process::{play_sound, show_notification};

/// Surface rejected input instead of silently dropping it. Enqueues are
/// atomic, so a failed paste is reported as a whole rather than truncated.
pub(super) fn write_pty(pane: &Pane, bytes: &[u8]) {
    if let Err(error) = pane.pty.write(bytes) {
        eprintln!("litty: {error}");
        let mut g = pane.grid.lock();
        g.notification = Some(("Litty input".into(), error.to_string()));
        g.bell = true;
    }
}

/// Queue every user text source while thinking; terminal protocol replies bypass this.
pub(super) fn write_pane_input(
    pane: &Pane,
    states: &mut std::collections::BTreeMap<u64, ai::PaneState>,
    bytes: &[u8],
) {
    if let Some(ai) = states.get_mut(&pane.id).filter(|a| a.busy) {
        ai.enqueue(bytes);
    } else {
        write_pty(pane, bytes);
    }
}

/// Service requests raised by the PTY parser outside the grid lock: clipboard,
/// notifications, the bell, and mouse pointer shape.
pub(super) fn handle_terminal_requests(s: &mut State) {
    let mut titles_dirty = false;
    let mut attention = false;
    let mut ring = false;
    let mut audio = false;
    let mut border = false;
    for (i, tab) in s.tabs.iter_mut().enumerate() {
        let in_front = s.focused && i == s.active;
        for pane in &tab.panes {
            let (notification, bell, clip) = {
                let mut g = pane.grid.lock();
                (g.notification.take(), std::mem::take(&mut g.bell), g.clipboard_request.take())
            };
            if let Some(text) = clip {
                let is_active_pane = in_front && pane.id == tab.active_pane_id;
                if is_active_pane && s.config.clipboard_write {
                    copy_to_clipboard(&text);
                }
            }
            if let Some((title, body)) = notification {
                // At most one per second, so a runaway loop cannot flood the desktop.
                if s.last_notification.is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(1)) {
                    s.last_notification = Some(std::time::Instant::now());
                    let title = if title.is_empty() { "Litty".to_string() } else { title };
                    show_notification(&title, &body);
                }
            }
            if bell {
                ring |= s.config.bell_features.system;
                audio |= s.config.bell_features.audio;
                border |= s.config.bell_features.border;
                if !in_front && s.config.bell_features.title {
                    titles_dirty |= !tab.bell;
                    tab.bell = true;
                }
                attention |= !s.focused && s.config.bell_features.attention;
            }
        }
    }
    if attention {
        s.window.request_user_attention(Some(winit::window::UserAttentionType::Informational));
    }
    if ring {
        macos::beep();
    }
    if audio
        && let Some(path) = &s.config.bell_audio_path {
            play_sound(path, s.config.bell_audio_volume);
        }
    if border {
        s.renderer.set_bell_border(true);
        s.window.request_redraw();
    }
    if titles_dirty {
        refresh_tab_titles(s);
    }

    let shape = s.tabs.get(s.active).and_then(|t| t.active_pane().map(|p| pointer_icon(&p.grid.lock().pointer_shape)));
    if let Some(icon) = shape.filter(|&icon| icon != s.pointer.pointer_icon) {
        s.pointer.pointer_icon = icon;
        if s.pointer.hovered_link.is_none() {
            s.window.set_cursor(icon);
        }
    }
}

/// Map an OSC 22 pointer name (CSS, or the X11 names xterm uses) to a cursor.
pub(super) fn pointer_icon(name: &str) -> CursorIcon {
    let css = match name {
        "" | "left_ptr" | "arrow" => "default",
        "xterm" | "ibeam" => "text",
        "hand" | "hand1" | "hand2" => "pointer",
        "watch" => "wait",
        "question_arrow" => "help",
        "fleur" => "move",
        "cross" | "tcross" => "crosshair",
        "sb_h_double_arrow" => "ew-resize",
        "sb_v_double_arrow" => "ns-resize",
        other => other,
    };
    css.parse().unwrap_or(CursorIcon::Default)
}
