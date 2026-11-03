//! Key events: shortcuts first, then Kitty / legacy encoding to the PTY. Pure key -> bytes lives in `crate::keymap`.

use super::actions::run_action;
use super::ai_bridge::handle_ai_requests;
use super::search_ui::handle_history_search_key;
use super::search_ui::handle_search_key;
use super::state::State;
use super::terminal_io::write_pane_input;
use super::terminal_io::write_pty;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::terminal::grid::Grid;
use crate::{ai, keymap};
use crate::platform;
use crate::{next_id, Waker};

use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, NamedKey, PhysicalKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

pub(super) fn handle_key(
    s: &mut State,
    ev: &KeyEvent,
    waker: &Waker,
) {
    let m = keymap::Mods {
        ctrl: s.mods.control_key(),
        alt: s.mods.alt_key(),
        shift: s.mods.shift_key(),
        sup: s.mods.super_key(),
    };
    if ev.state != ElementState::Pressed {
        handle_key_release(s, ev, m);
        return;
    }
    s.locks = keymap::infer_locks(s.locks, &ev.logical_key, ev.location, m.shift);

    let is_modifier = keymap::is_modifier_key(&ev.logical_key);
    if !is_modifier && s.config.mouse_hide_while_typing && !s.pointer.mouse_hidden {
        s.window.set_cursor_visible(false);
        s.pointer.mouse_hidden = true;
    }
    s.cursor_blink_epoch = std::time::Instant::now();

    if s.history_search.is_open && handle_history_search_key(s, &ev.logical_key, m) {
        consume_key(s, ev);
        return;
    }

    let alt_screen = s.tabs.get(s.active).and_then(|t| t.active_pane()).is_some_and(|p| p.grid.lock().alt_active);
    if s.search.is_open {
        // Keys belong to the search box; shortcuts it does not consume still work,
        // but nothing reaches the shell.
        consume_key(s, ev);
        if !handle_search_key(s, &ev.logical_key, m)
            && let Some(action) = keymap::resolve(keymap::PLATFORM, &ev.logical_key, m, alt_screen) {
                run_action(s, action, waker);
            }
        return;
    }
    // User `keybind` entries come first; the last matching one wins.
    let unshifted_key = ev.key_without_modifiers();
    let user_binding = s
        .config
        .keybinds
        .iter()
        .rev()
        .find(|(t, _)| keymap::trigger_matches(t, &unshifted_key, m))
        .map(|(_, b)| b.clone());
    let builtin = match user_binding {
        Some(keymap::Binding::Action(action)) => {
            run_action(s, action, waker);
            consume_key(s, ev);
            return;
        }
        Some(keymap::Binding::Text(bytes)) => {
            if let Some(tab) = s.tabs.get(s.active)
                && let Some(pane) = tab.active_pane() {
                    write_pane_input(pane,&mut s.ai,&bytes);
                }
            consume_key(s, ev);
            return;
        }
        Some(keymap::Binding::Ignore) => {
            consume_key(s, ev);
            return;
        }
        Some(keymap::Binding::Unbind) => None,
        None => keymap::resolve(keymap::PLATFORM, &ev.logical_key, m, alt_screen),
    };
    if let Some(action) = builtin
        && run_action(s, action, waker) {
            consume_key(s, ev);
            return;
        }
    if s.tabs.is_empty() {
        consume_key(s, ev);
        return;
    }

    let Some(pane) = s.tabs[s.active].active_pane() else {
        consume_key(s, ev);
        return;
    };
    let pane_id = pane.id;
    if s.ai.get(&pane_id).is_some_and(|a| a.busy) {
        let cancel = keymap::ai_cancel(&ev.logical_key, m);
        if cancel {
            let ai = s.ai.get_mut(&pane_id).unwrap();
            ai.cancel_request();
            if ai.authorized.take().is_some() {
                // A canceled bridge must not leave input waiting for a worker
                // that was never started (including a malformed OSC payload).
                {
                    let mut g = pane.grid.lock();
                    g.ai.armed = false;
                    g.ai.request = None;
                }
                ai.submitted = None;
                ai.result = Some(ai::Outcome {
                    session: ai.session.take().unwrap_or_default(),
                    command: None,
                    text: "Cancelled".into(),
                    error: true,
                    literal: false,
                });
            }
        } else {
            let (app_cursor, kitty_flags, modify_other_keys) = {
                let g = pane.grid.lock();
                (g.modes.app_cursor, g.kitty_keyboard_flags(), g.modes.modify_other_keys)
            };
            if let Some(bytes) =
                encode_terminal_key(ev, m, s.locks, app_cursor, kitty_flags, modify_other_keys)
            {
                s.ai.get_mut(&pane_id).unwrap().enqueue(&bytes);
            }
        }
        consume_key(s, ev);
        if cancel {
            handle_ai_requests(s, waker);
            s.window.request_redraw();
        }
        return;
    }
    let is_enter =
        ev.logical_key == Key::Named(NamedKey::Enter) && !m.alt && !m.ctrl && !m.shift && !m.sup;
    if is_enter {
        let trigger = s.config.ai.trigger().to_owned();
        let trigger = trigger.as_str();
        let decision = inspect_enter(&mut pane.grid.lock(), s.ai.contains_key(&pane_id), trigger);
        let EnterDecision { input, epoch, intercept } = decision;
        if let Some(Intercept { query, direct }) = intercept {
            let ai = s.ai.entry(pane_id).or_default();
            ai.submitted(&input, trigger);
            ai.awaiting = false;
            ai.authorized = Some(query);
            ai.generation = next_id();
            ai.cancel = Arc::new(AtomicBool::new(false));
            ai.busy = true;
            ai.prompt_epoch = epoch;
            if direct {
                // Clear Readline/Reedline's buffer and submit an empty line. The
                // natural language is never parsed or executed by the shell.
                ai.display_query = Some(input.clone());
                s.history_search.add_session_command(&input);
                write_pty(pane, b"\x05\x15\r");
                consume_key(s, ev);
                return;
            }
        } else if let Some(ai) = s.ai.get_mut(&pane_id) {
            if input.trim().is_empty() {
                ai.submitted("", trigger);
            }
            ai.prompt_epoch = epoch;
        }
    } else if m.ctrl
        && matches!(&ev.logical_key,Key::Character(ch) if ch.eq_ignore_ascii_case("c"))
        && let Some(ai) = s.ai.get_mut(&pane_id)
    {
        ai.suggestion = None;
        ai.awaiting = false;
        ai.executed = None;
        pane.grid.lock().ai.track = false;
    }

    let (app_cursor, kitty_flags, modify_other_keys, ctrl_u_bytes) = {
        let mut g = pane.grid.lock();
        let app_cursor = g.modes.app_cursor;
        let kitty_flags = g.kitty_keyboard_flags();
        let modify_other_keys = g.modes.modify_other_keys;

        // A lone modifier (Cmd before C, Shift before a click) keeps the selection
        // and the scrollback position; only real keys snap back to the prompt.
        if !is_modifier {
            g.reset_display();
        }

        let ctrl_u_bytes = match &ev.logical_key {
            // Ctrl+U at a prompt: delete back to the prompt only (vim keeps its half-page scroll).
            Key::Character(ch) if m == (keymap::Mods { ctrl: true, ..Default::default() }) && ch.eq_ignore_ascii_case("u") => {
                if !g.alt_active {
                    g.calculate_delete_before_cursor()
                } else {
                    None
                }
            }
            Key::Named(NamedKey::Enter) => {
                let cmd = if g.alt_active { String::new() } else { g.current_prompt_input() };
                if !cmd.is_empty() {
                    s.history_search.add_session_command(&cmd);
                }
                None
            }
            _ => None,
        };

        (app_cursor, kitty_flags, modify_other_keys, ctrl_u_bytes)
    };

    // Unbound Cmd/Super combinations only reach applications that speak the Kitty
    // protocol (which can encode Super); a shell would just see the bare key.
    if m.sup && !is_modifier && kitty_flags == 0 {
        consume_key(s, ev);
        return;
    }

    if let Some(bytes) = ctrl_u_bytes {
        write_pty(pane, &bytes);
        s.window.request_redraw();
        return;
    }

    let bytes=encode_terminal_key(ev,m,s.locks,app_cursor,kitty_flags,modify_other_keys);
    if let Some(bytes) = &bytes {
        write_pty(pane, bytes);
    }
    if s.inspector.shown {
        s.inspector.last_key = describe_key(&ev.logical_key, m, bytes.as_deref());
        s.window.request_redraw();
    }
}

/// An Enter at a prompt that the AI bridge takes over.
#[derive(Debug, PartialEq, Eq)]
struct Intercept {
    query: String,
    /// The shell never sees the text: the buffer is cleared and an empty line submitted.
    direct: bool,
}

struct EnterDecision {
    input: String,
    epoch: u64,
    intercept: Option<Intercept>,
}

/// Longest query that may still travel through the shell's own line editor.
const MAX_SHELL_MEDIATED_QUERY: usize = 4096;

/// Inspect the prompt and update the grid's AI state in one critical section, so
/// the request, the epoch and the input always describe the same prompt. Taking
/// `&mut Grid` makes that the only way to call it; the caller drops the lock
/// before touching UI state, history or the PTY.
fn inspect_enter(g: &mut Grid, has_ai_state: bool, trigger: &str) -> EnterDecision {
    let input = g.full_prompt_input();
    let epoch = g.prompt_epoch;
    let query = (g.prompt_ready() && g.ai.available)
        .then(|| ai::query(&input, trigger))
        .flatten();
    let intercept = query.map(|query| {
        let direct = g.ai.direct
            || input.starts_with(char::is_whitespace)
            || query.len() > MAX_SHELL_MEDIATED_QUERY;
        g.ai.track = false;
        if direct {
            g.ai.request = Some(query.into());
        } else {
            g.ai.armed = true;
        }
        Intercept { query: query.into(), direct }
    });
    if intercept.is_none() && has_ai_state && input.trim().is_empty() {
        g.ai.track = false;
    }
    EnterDecision { input, epoch, intercept }
}

/// Encode queued input with the same protocol and modifiers as immediate input.
pub(super) fn encode_terminal_key(
    ev: &KeyEvent,
    m: keymap::Mods,
    locks: u8,
    app_cursor: bool,
    kitty_flags: u8,
    modify_other_keys: u8,
) -> Option<Vec<u8>> {
    if kitty_flags != 0 {
        let kind = if ev.repeat {
            keymap::KeyKind::Repeat
        } else {
            keymap::KeyKind::Press
        };
        let unshifted = ev.key_without_modifiers();
        keymap::encode_kitty(
            keymap::PLATFORM,
            &kitty_key(ev, &unshifted, kind, locks),
            m,
            kitty_flags,
            app_cursor,
        )
    } else if modify_other_keys != 0 {
        let text = ev.text.as_deref();
        keymap::encode_modify_other_keys(
            keymap::PLATFORM,
            &ev.logical_key,
            text,
            m,
            modify_other_keys,
            app_cursor,
        )
    } else {
        keymap::encode(
            keymap::PLATFORM,
            &ev.logical_key,
            ev.text.as_deref(),
            m,
            app_cursor,
        )
    }
}

/// `Ctrl+Shift+A → "\e[97;6u"` for the inspector.
pub(super) fn describe_key(key: &Key, m: keymap::Mods, bytes: Option<&[u8]>) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for (on, name) in [(m.ctrl, "Ctrl+"), (m.alt, "Alt+"), (m.shift, "Shift+"), (m.sup, "Super+")] {
        if on {
            out.push_str(name);
        }
    }
    match key {
        Key::Character(c) => out.push_str(c),
        other => {
            let _ = write!(out, "{other:?}");
        }
    }
    out.push_str(" → ");
    match bytes {
        None => out.push_str("(nothing)"),
        Some(bytes) => {
            out.push('"');
            for &b in bytes {
                match b {
                    0x1b => out.push_str("\\e"),
                    0x20..=0x7e => out.push(b as char),
                    _ => {
                        let _ = write!(out, "\\x{b:02x}");
                    }
                }
            }
            out.push('"');
        }
    }
    out
}

/// Remember that Litty handled this key press itself.
pub(super) fn consume_key(s: &mut State, ev: &KeyEvent) {
    if !s.consumed_keys.contains(&ev.physical_key) {
        // A release can be lost (e.g. focus moved away mid-press); never grow unbounded.
        if s.consumed_keys.len() >= 16 {
            s.consumed_keys.clear();
        }
        s.consumed_keys.push(ev.physical_key);
    }
}

/// Key releases only matter to applications that asked for Kitty event reporting.
pub(super) fn handle_key_release(s: &mut State, ev: &KeyEvent, m: keymap::Mods) {
    if let Some(i) = s.consumed_keys.iter().position(|k| *k == ev.physical_key) {
        s.consumed_keys.swap_remove(i);
        return;
    }
    if s.tabs.get(s.active).and_then(|t|t.active_pane()).is_some_and(|p|s.ai.get(&p.id).is_some_and(|a|a.busy)) {return;}
    if s.tabs.is_empty() || s.search.is_open || s.history_search.is_open {
        return;
    }
    let Some(pane) = s.tabs[s.active].active_pane() else { return; };
    let (app_cursor, kitty_flags) = {
        let g = pane.grid.lock();
        (g.modes.app_cursor, g.kitty_keyboard_flags())
    };
    if kitty_flags & keymap::KITTY_REPORT_EVENTS == 0 {
        return;
    }
    let unshifted = ev.key_without_modifiers();
    let key = kitty_key(ev, &unshifted, keymap::KeyKind::Release, s.locks);
    if let Some(bytes) = keymap::encode_kitty(keymap::PLATFORM, &key, m, kitty_flags, app_cursor) {
        write_pty(pane, &bytes);
    }
}

pub(super) fn kitty_key<'a>(ev: &'a KeyEvent, unshifted: &'a Key, kind: keymap::KeyKind, inferred: u8) -> keymap::KittyKey<'a> {
    keymap::KittyKey {
        key: &ev.logical_key,
        unshifted,
        physical: match ev.physical_key {
            PhysicalKey::Code(code) => Some(code),
            PhysicalKey::Unidentified(_) => None,
        },
        location: ev.location,
        text: ev.text.as_deref(),
        kind,
        // winit has no lock state: macOS reads Caps Lock from AppKit (and has no
        // Num Lock), elsewhere it is inferred from the keys typed.
        locks: if cfg!(target_os = "macos") {
            if platform::caps_lock() { keymap::KITTY_CAPS_LOCK } else { 0 }
        } else {
            inferred
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::grid::Grid;

    fn feed(g: &mut Grid, s: &str) {
        let mut p = vte::Parser::new();
        for b in s.bytes() {
            g.feed_byte(&mut p, b);
        }
    }

    fn prompt_with(input: &str) -> Grid {
        let mut g = Grid::new(80, 8);
        feed(&mut g, &format!("\x1b]133;A\x07$ \x1b]133;B\x07{input}"));
        g.ai.available = true;
        g.ai.track = true;
        g
    }

    #[test]
    fn a_renamed_trigger_replaces_yo() {
        let mut g = prompt_with("ask list files");
        let d = inspect_enter(&mut g, true, "ask");
        assert_eq!(d.intercept.map(|i| i.query), Some("list files".into()));
        let mut g = prompt_with("yo generator");
        assert_eq!(inspect_enter(&mut g, true, "ask").intercept, None);
        assert!(!g.ai.armed && g.ai.request.is_none());
    }

    #[test]
    fn a_disabled_trigger_never_intercepts() {
        for line in ["yo list files", "ask list files"] {
            let mut g = prompt_with(line);
            assert_eq!(inspect_enter(&mut g, true, "").intercept, None, "{line}");
            assert!(!g.ai.armed && g.ai.request.is_none());
        }
    }

    #[test]
    fn enter_shell_mediated_arms_bridge_and_stops_tracking() {
        let mut g = prompt_with("yo list files");
        let epoch = g.prompt_epoch;
        let d = inspect_enter(&mut g, true, "yo");
        assert_eq!(d.epoch, epoch);
        assert_eq!(d.input, "yo list files");
        assert_eq!(d.intercept, Some(Intercept { query: "list files".into(), direct: false }));
        assert!(g.ai.armed && !g.ai.track && g.ai.request.is_none());
    }

    #[test]
    fn enter_direct_mode_queues_request_for_the_inspected_query() {
        let mut g = prompt_with("yo old query");
        g.ai.direct = true;
        let d = inspect_enter(&mut g, true, "yo");
        assert_eq!(d.intercept, Some(Intercept { query: "old query".into(), direct: true }));
        assert_eq!(g.ai.request.as_deref(), Some("old query"));
        assert!(!g.ai.armed && !g.ai.track);
    }

    #[test]
    fn enter_leading_whitespace_forces_direct() {
        let mut g = prompt_with(" yo hi");
        let d = inspect_enter(&mut g, true, "yo");
        assert!(d.intercept.is_some_and(|i| i.direct));
        assert_eq!(g.ai.request.as_deref(), Some("hi"));
    }

    #[test]
    fn enter_request_and_epoch_come_from_the_same_prompt() {
        // The grid is only reachable through `&mut Grid`, so a second prompt
        // cannot be parsed between inspection and mutation.
        let mut g = prompt_with("yo old query");
        g.ai.direct = true;
        let d = inspect_enter(&mut g, true, "yo");
        feed(&mut g, "\r\n\x1b]133;D;0\x07\x1b]133;A\x07$ \x1b]133;B\x07different command");
        assert_ne!(d.epoch, g.prompt_epoch);
        assert_eq!(g.ai.request.as_deref(), Some("old query"));
        assert_eq!(d.intercept.map(|i| i.query), Some("old query".into()));
    }

    #[test]
    fn enter_ordinary_command_is_not_intercepted() {
        let mut g = prompt_with("ls -la");
        let d = inspect_enter(&mut g, true, "yo");
        assert_eq!(d.intercept, None);
        assert_eq!(d.input, "ls -la");
        assert!(g.ai.track && !g.ai.armed && g.ai.request.is_none());
    }

    #[test]
    fn enter_empty_stops_tracking_only_with_ai_state() {
        let mut g = prompt_with("");
        inspect_enter(&mut g, false, "yo");
        assert!(g.ai.track);
        inspect_enter(&mut g, true, "yo");
        assert!(!g.ai.track);
    }

    #[test]
    fn enter_unavailable_or_alt_screen_is_not_intercepted() {
        let mut g = prompt_with("yo hi");
        g.ai.available = false;
        assert_eq!(inspect_enter(&mut g, true, "yo").intercept, None);
        let mut g = prompt_with("yo hi");
        feed(&mut g, "\x1b[?1049h");
        assert_eq!(inspect_enter(&mut g, true, "yo").intercept, None);
        assert!(!g.ai.armed && g.ai.request.is_none());
    }
}
