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

use crate::{ai, keymap, macos};
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
    if !is_modifier && s.config.mouse_hide_while_typing && !s.mouse_hidden {
        s.window.set_cursor_visible(false);
        s.mouse_hidden = true;
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
                    g.ai_armed = false;
                    g.ai_request = None;
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
                (g.app_cursor, g.kitty_keyboard_flags(), g.modify_other_keys)
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
        let (input, ready, epoch) = {
            let g = pane.grid.lock();
            (g.full_prompt_input(), g.prompt_ready(), g.prompt_epoch)
        };
        if ready
            && pane.grid.lock().ai_available
            && let Some(query) = ai::query(&input)
        {
            let ai = s.ai.entry(pane_id).or_default();
            ai.submitted(&input);
            ai.awaiting = false;
            ai.authorized = Some(query.into());
            ai.generation = next_id();
            ai.cancel = Arc::new(AtomicBool::new(false));
            ai.busy = true;
            ai.prompt_epoch = epoch;
            let direct = pane.grid.lock().ai_direct
                || input.starts_with(char::is_whitespace)
                || query.len() > 4096;
            if direct {
                // Clear Readline/Reedline's buffer and submit an empty line. The
                // natural language is never parsed or executed by the shell.
                {
                    let mut g = pane.grid.lock();
                    g.ai_track = false;
                    g.ai_request = Some(query.into());
                }
                ai.display_query = Some(input.clone());
                s.history_search.add_session_command(&input);
                write_pty(pane, b"\x05\x15\r");
                consume_key(s, ev);
                return;
            }
            {
                let mut g = pane.grid.lock();
                g.ai_track = false;
                g.ai_armed = true;
            }
        } else if let Some(ai) = s.ai.get_mut(&pane_id) {
            if input.trim().is_empty() {
                ai.submitted("");
                pane.grid.lock().ai_track = false;
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
        pane.grid.lock().ai_track = false;
    }

    let (app_cursor, kitty_flags, modify_other_keys, ctrl_u_bytes) = {
        let mut g = pane.grid.lock();
        let app_cursor = g.app_cursor;
        let kitty_flags = g.kitty_keyboard_flags();
        let modify_other_keys = g.modify_other_keys;

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
        (g.app_cursor, g.kitty_keyboard_flags())
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
            if macos::caps_lock() { keymap::KITTY_CAPS_LOCK } else { 0 }
        } else {
            inferred
        },
    }
}
