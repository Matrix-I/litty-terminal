//! Event translation from NSEvent to WindowEvent and KeyInput.

use std::ffi::c_void;

use crate::platform::gui::{ButtonState, MouseButton, PhysicalPosition, ScrollDelta, TouchPhase, WindowEvent, WindowId};
use crate::platform::input::{Key, KeyCode, KeyInput, KeyLocation, Mods, NamedKey, PhysicalKey};
use crate::platform::macos::objc::*;

/// Where `event` happened, in the coordinates of `view` (flipped: (0, 0) is its
/// top-left corner), so it lines up with the terminal grid, tab bar and scrollbars.
///
/// An event without a window (the pointer was over no window of ours when it was
/// generated) carries screen coordinates in `locationInWindow`; it is first moved
/// into `win`, the window it is being delivered to.
unsafe fn location_in_view(event: Id, win: Id, view: Id) -> NSPoint {
    unsafe {
        let mut loc = msg_send!(event, c"locationInWindow"; -> NSPoint);
        if view.is_null() {
            return loc;
        }
        let event_window = msg_send!(event, c"window"; -> Id);
        if event_window.is_null() && !win.is_null() {
            loc = msg_send!(win, c"convertPointFromScreen:", loc => NSPoint; -> NSPoint);
        }
        msg_send!(view, c"convertPoint:fromView:", loc => NSPoint, std::ptr::null_mut::<c_void>() => Id; -> NSPoint)
    }
}

/// Process a pointer or modifier `NSEvent` delivered to `win`, converting it to
/// one or more `WindowEvent`s and invoking `emit`.
///
/// Key events are not handled here: they go through AppKit to `LittyView`, which
/// runs them through the input method first (see `view.rs`).
pub unsafe fn process_ns_event<F: FnMut(WindowEvent)>(
    event: Id,
    win: Id,
    view: Id,
    scale: f64,
    mut emit: F,
) {
    unsafe {
        let event_type = msg_send!(event, c"type"; -> usize);
        match event_type {
            // NSLeftMouseDown = 1, NSRightMouseDown = 3, NSOtherMouseDown = 25
            1 | 3 | 25 => {
                let pt = location_in_view(event, win, view);
                let pos = PhysicalPosition::new(pt.x * scale, pt.y * scale);
                emit(WindowEvent::CursorMoved(pos));

                let button = match event_type {
                    1 => MouseButton::Left,
                    3 => MouseButton::Right,
                    _ => MouseButton::Middle,
                };
                emit(WindowEvent::MouseInput { state: ButtonState::Pressed, button });
            }
            // NSLeftMouseUp = 2, NSRightMouseUp = 4, NSOtherMouseUp = 26
            2 | 4 | 26 => {
                let pt = location_in_view(event, win, view);
                let pos = PhysicalPosition::new(pt.x * scale, pt.y * scale);
                emit(WindowEvent::CursorMoved(pos));

                let button = match event_type {
                    2 => MouseButton::Left,
                    4 => MouseButton::Right,
                    _ => MouseButton::Middle,
                };
                emit(WindowEvent::MouseInput { state: ButtonState::Released, button });
            }
            // NSMouseMoved = 5, NSLeftMouseDragged = 6, NSRightMouseDragged = 7, NSOtherMouseDragged = 27
            5 | 6 | 7 | 27 => {
                let pt = location_in_view(event, win, view);
                let pos = PhysicalPosition::new(pt.x * scale, pt.y * scale);
                emit(WindowEvent::CursorMoved(pos));
            }
            // NSScrollWheel = 22
            22 => {
                let pt = location_in_view(event, win, view);
                let pos = PhysicalPosition::new(pt.x * scale, pt.y * scale);
                emit(WindowEvent::CursorMoved(pos));

                let dy = msg_send!(event, c"scrollingDeltaY"; -> f64);
                let has_prec = msg_send!(event, c"hasPreciseScrollingDeltas"; -> bool);
                let delta = if has_prec {
                    ScrollDelta::Pixels(PhysicalPosition::new(0.0, dy * scale))
                } else {
                    ScrollDelta::Lines { y: dy as f32 }
                };
                let phase = msg_send!(event, c"phase"; -> usize);
                let momentum = msg_send!(event, c"momentumPhase"; -> usize);
                emit(WindowEvent::MouseWheel { delta, phase: scroll_phase(phase, momentum) });
            }
            // NSFlagsChanged = 12
            12 => {
                let flags = msg_send!(event, c"modifierFlags"; -> usize);
                let mods = convert_modifier_flags(flags);
                emit(WindowEvent::ModifiersChanged(mods));
            }
            _ => {}
        }
    }
}

/// `NSEventPhase` bits.
const PHASE_BEGAN: usize = 1;
const PHASE_ENDED: usize = 8;
const PHASE_CANCELLED: usize = 16;
const PHASE_MAY_BEGIN: usize = 32;

/// Gesture phase of a scroll event from its `phase` and `momentumPhase`
/// (as winit mapped them), so a new gesture drops the previous one's
/// fractional remainder.
fn scroll_phase(phase: usize, momentum: usize) -> TouchPhase {
    let map = |p: usize| {
        if p & (PHASE_MAY_BEGIN | PHASE_BEGAN) != 0 {
            Some(TouchPhase::Started)
        } else if p & (PHASE_ENDED | PHASE_CANCELLED) != 0 {
            Some(TouchPhase::Ended)
        } else {
            None
        }
    };
    map(momentum).or_else(|| map(phase)).unwrap_or(TouchPhase::Moved)
}

#[allow(dead_code)]
pub unsafe fn convert_ns_event(event: Id, _id: WindowId, scale: f64) -> Option<WindowEvent> {
    let mut result = None;
    unsafe {
        process_ns_event(event, std::ptr::null_mut(), std::ptr::null_mut(), scale, |ev| {
            result = Some(ev);
        });
    }
    result
}

/// `NSEventModifierFlagCommand`.
pub(super) const MODIFIER_COMMAND: usize = 1 << 20;

/// What a key press becomes once the input method has seen it.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum KeyDownOutcome {
    /// An ordinary key: deliver it as `WindowEvent::Key`.
    Key,
    /// The input method replaced the key with other text, without composing.
    Commit(String),
    /// The input method used the key (composition started, went on or was committed).
    Nothing,
}

/// Decide what a key press turns into after `interpretKeyEvents:`.
///
/// `composing_before` / `composing_after`: whether there was marked (preedit) text
/// before and after the input method handled the key. `inserted`: text inserted
/// while nothing was being composed. `characters`: the key event's own text.
/// `command`: the input method asked for an editing command (`insertNewline:`,
/// `deleteBackward:`, `moveLeft:` ...), meaning the key is not text.
pub(super) fn key_down_outcome(
    composing_before: bool,
    composing_after: bool,
    inserted: Option<String>,
    characters: Option<&str>,
    command: bool,
) -> KeyDownOutcome {
    if composing_after {
        return KeyDownOutcome::Nothing;
    }
    if composing_before {
        // The composition was committed (or cancelled) by this key; its text
        // already went out as `Ime::Commit`. A key the input method passes on
        // after committing (Enter in some layouts) still reaches the terminal.
        return if command { KeyDownOutcome::Key } else { KeyDownOutcome::Nothing };
    }
    match inserted {
        Some(text) if Some(text.as_str()) != characters => KeyDownOutcome::Commit(text),
        _ => KeyDownOutcome::Key,
    }
}

pub fn convert_modifier_flags(flags: usize) -> Mods {
    Mods {
        // NSEventModifierFlagControl = 1 << 18
        ctrl: (flags & (1 << 18)) != 0,
        // NSEventModifierFlagOption = 1 << 19
        alt: (flags & (1 << 19)) != 0,
        // NSEventModifierFlagShift = 1 << 17
        shift: (flags & (1 << 17)) != 0,
        // NSEventModifierFlagCommand = 1 << 20
        sup: (flags & (1 << 20)) != 0,
    }
}

pub(super) unsafe fn convert_key_event(event: Id, pressed: bool) -> KeyInput {
    unsafe {
        let key_code = msg_send!(event, c"keyCode"; -> u16);
        let is_repeat = if pressed {
            msg_send!(event, c"isARepeat"; -> bool)
        } else {
            false
        };

        let characters_obj = msg_send!(event, c"characters"; -> Id);
        let text = ns_string_to_string(characters_obj);

        let characters_ignoring_modifiers_obj = msg_send!(event, c"charactersIgnoringModifiers"; -> Id);
        let unshifted_text = ns_string_to_string(characters_ignoring_modifiers_obj);

        let (logical, named) = match key_code {
        36 => (Key::Named(NamedKey::Enter), Some(NamedKey::Enter)),
        48 => (Key::Named(NamedKey::Tab), Some(NamedKey::Tab)),
        49 => (Key::Named(NamedKey::Space), Some(NamedKey::Space)),
        51 => (Key::Named(NamedKey::Backspace), Some(NamedKey::Backspace)),
        53 => (Key::Named(NamedKey::Escape), Some(NamedKey::Escape)),
        117 => (Key::Named(NamedKey::Delete), Some(NamedKey::Delete)),
        115 => (Key::Named(NamedKey::Home), Some(NamedKey::Home)),
        119 => (Key::Named(NamedKey::End), Some(NamedKey::End)),
        116 => (Key::Named(NamedKey::PageUp), Some(NamedKey::PageUp)),
        121 => (Key::Named(NamedKey::PageDown), Some(NamedKey::PageDown)),
        126 => (Key::Named(NamedKey::ArrowUp), Some(NamedKey::ArrowUp)),
        125 => (Key::Named(NamedKey::ArrowDown), Some(NamedKey::ArrowDown)),
        123 => (Key::Named(NamedKey::ArrowLeft), Some(NamedKey::ArrowLeft)),
        124 => (Key::Named(NamedKey::ArrowRight), Some(NamedKey::ArrowRight)),
        122 => (Key::Named(NamedKey::F1), Some(NamedKey::F1)),
        120 => (Key::Named(NamedKey::F2), Some(NamedKey::F2)),
        99 => (Key::Named(NamedKey::F3), Some(NamedKey::F3)),
        118 => (Key::Named(NamedKey::F4), Some(NamedKey::F4)),
        96 => (Key::Named(NamedKey::F5), Some(NamedKey::F5)),
        97 => (Key::Named(NamedKey::F6), Some(NamedKey::F6)),
        98 => (Key::Named(NamedKey::F7), Some(NamedKey::F7)),
        100 => (Key::Named(NamedKey::F8), Some(NamedKey::F8)),
        101 => (Key::Named(NamedKey::F9), Some(NamedKey::F9)),
        109 => (Key::Named(NamedKey::F10), Some(NamedKey::F10)),
        103 => (Key::Named(NamedKey::F11), Some(NamedKey::F11)),
        111 => (Key::Named(NamedKey::F12), Some(NamedKey::F12)),
        _ => {
            if let Some(ref s) = text {
                if !s.is_empty() {
                    (Key::Character(s.clone()), None)
                } else {
                    (Key::Unidentified, None)
                }
            } else {
                (Key::Unidentified, None)
            }
        }
    };

    let unshifted = if let Some(n) = named {
        Key::Named(n)
    } else if let Some(ref s) = unshifted_text {
        if !s.is_empty() {
            Key::Character(s.clone())
        } else {
            logical.clone()
        }
    } else {
        logical.clone()
    };

    let physical = match key_code {
        0 => PhysicalKey::Code(KeyCode::KeyA),
        11 => PhysicalKey::Code(KeyCode::KeyB),
        8 => PhysicalKey::Code(KeyCode::KeyC),
        2 => PhysicalKey::Code(KeyCode::KeyD),
        14 => PhysicalKey::Code(KeyCode::KeyE),
        3 => PhysicalKey::Code(KeyCode::KeyF),
        5 => PhysicalKey::Code(KeyCode::KeyG),
        4 => PhysicalKey::Code(KeyCode::KeyH),
        34 => PhysicalKey::Code(KeyCode::KeyI),
        38 => PhysicalKey::Code(KeyCode::KeyJ),
        40 => PhysicalKey::Code(KeyCode::KeyK),
        37 => PhysicalKey::Code(KeyCode::KeyL),
        46 => PhysicalKey::Code(KeyCode::KeyM),
        45 => PhysicalKey::Code(KeyCode::KeyN),
        31 => PhysicalKey::Code(KeyCode::KeyO),
        35 => PhysicalKey::Code(KeyCode::KeyP),
        12 => PhysicalKey::Code(KeyCode::KeyQ),
        15 => PhysicalKey::Code(KeyCode::KeyR),
        1 => PhysicalKey::Code(KeyCode::KeyS),
        17 => PhysicalKey::Code(KeyCode::KeyT),
        32 => PhysicalKey::Code(KeyCode::KeyU),
        9 => PhysicalKey::Code(KeyCode::KeyV),
        13 => PhysicalKey::Code(KeyCode::KeyW),
        7 => PhysicalKey::Code(KeyCode::KeyX),
        16 => PhysicalKey::Code(KeyCode::KeyY),
        6 => PhysicalKey::Code(KeyCode::KeyZ),
        18 => PhysicalKey::Code(KeyCode::Digit1),
        19 => PhysicalKey::Code(KeyCode::Digit2),
        20 => PhysicalKey::Code(KeyCode::Digit3),
        21 => PhysicalKey::Code(KeyCode::Digit4),
        23 => PhysicalKey::Code(KeyCode::Digit5),
        22 => PhysicalKey::Code(KeyCode::Digit6),
        26 => PhysicalKey::Code(KeyCode::Digit7),
        28 => PhysicalKey::Code(KeyCode::Digit8),
        25 => PhysicalKey::Code(KeyCode::Digit9),
        29 => PhysicalKey::Code(KeyCode::Digit0),
        49 => PhysicalKey::Code(KeyCode::Space),
        other => PhysicalKey::Code(KeyCode::Other(other as u32)),
    };

        KeyInput {
            logical,
            unshifted,
            physical,
            location: KeyLocation::Standard,
            text,
            pressed,
            repeat: is_repeat,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_key_is_delivered_as_a_key() {
        assert_eq!(key_down_outcome(false, false, Some("a".into()), Some("a"), false), KeyDownOutcome::Key);
        assert_eq!(key_down_outcome(false, false, None, Some("\r"), true), KeyDownOutcome::Key, "Enter: insertNewline:");
        assert_eq!(key_down_outcome(false, false, None, Some("\u{f702}"), true), KeyDownOutcome::Key, "arrow: moveLeft:");
        assert_eq!(key_down_outcome(false, false, None, None, false), KeyDownOutcome::Key, "nothing asked for");
    }

    #[test]
    fn a_key_used_by_the_input_method_is_not_delivered() {
        // Telex: "a" starts a composition, "a" again turns it into "â".
        assert_eq!(key_down_outcome(false, true, None, Some("a"), false), KeyDownOutcome::Nothing);
        assert_eq!(key_down_outcome(true, true, None, Some("a"), false), KeyDownOutcome::Nothing);
        // Space commits "â" (sent as `Ime::Commit` by `insertText:`).
        assert_eq!(key_down_outcome(true, false, None, Some(" "), false), KeyDownOutcome::Nothing);
        // Backspace removing the last composed letter.
        assert_eq!(key_down_outcome(true, false, None, Some("\u{7f}"), false), KeyDownOutcome::Nothing);
    }

    #[test]
    fn a_command_after_a_commit_still_reaches_the_terminal() {
        assert_eq!(key_down_outcome(true, false, None, Some("\r"), true), KeyDownOutcome::Key);
    }

    #[test]
    fn text_replaced_without_composing_is_committed() {
        assert_eq!(
            key_down_outcome(false, false, Some("ê".into()), Some("e"), false),
            KeyDownOutcome::Commit("ê".into())
        );
    }

    #[test]
    fn test_convert_modifier_flags() {
        // NSEventModifierFlagControl = 1 << 18
        // NSEventModifierFlagOption = 1 << 19
        // NSEventModifierFlagShift = 1 << 17
        // NSEventModifierFlagCommand = 1 << 20
        let mods_none = convert_modifier_flags(0);
        assert!(!mods_none.ctrl && !mods_none.alt && !mods_none.shift && !mods_none.sup);

        let mods_cmd_shift = convert_modifier_flags((1 << 20) | (1 << 17));
        assert!(mods_cmd_shift.sup);
        assert!(mods_cmd_shift.shift);
        assert!(!mods_cmd_shift.ctrl);
        assert!(!mods_cmd_shift.alt);

        let mods_all = convert_modifier_flags((1 << 18) | (1 << 19) | (1 << 17) | (1 << 20));
        assert!(mods_all.ctrl && mods_all.alt && mods_all.shift && mods_all.sup);
    }

    #[test]
    fn scroll_phase_follows_gesture_then_momentum() {
        const CHANGED: usize = 4;
        assert_eq!(scroll_phase(0, 0), TouchPhase::Moved, "mouse wheel without phases");
        assert_eq!(scroll_phase(PHASE_MAY_BEGIN, 0), TouchPhase::Started);
        assert_eq!(scroll_phase(PHASE_BEGAN, 0), TouchPhase::Started);
        assert_eq!(scroll_phase(CHANGED, 0), TouchPhase::Moved);
        assert_eq!(scroll_phase(PHASE_ENDED, 0), TouchPhase::Ended);
        assert_eq!(scroll_phase(PHASE_CANCELLED, 0), TouchPhase::Ended);
        assert_eq!(scroll_phase(0, PHASE_BEGAN), TouchPhase::Started, "momentum start");
        assert_eq!(scroll_phase(0, CHANGED), TouchPhase::Moved);
        assert_eq!(scroll_phase(0, PHASE_ENDED), TouchPhase::Ended);
    }
}
