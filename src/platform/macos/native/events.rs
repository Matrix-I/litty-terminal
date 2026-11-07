//! Event translation from NSEvent to WindowEvent and KeyInput.

use std::ffi::c_void;

use crate::platform::gui::{ButtonState, MouseButton, PhysicalPosition, ScrollDelta, TouchPhase, WindowEvent, WindowId};
use crate::platform::input::{Key, KeyCode, KeyInput, KeyLocation, Mods, NamedKey, PhysicalKey};
use crate::platform::macos::objc::*;

/// Process an `NSEvent`, converting it to one or more `WindowEvent`s and invoking `emit`.
///
/// Mouse positions are converted from window coordinates into view coordinates
/// (`LittyView` is flipped, placing (0, 0) at the top-left corner), ensuring
/// pixel-perfect alignment with the terminal grid, tab bar, and scrollbars.
pub unsafe fn process_ns_event<F: FnMut(WindowEvent)>(
    event: Id,
    view: Id,
    scale: f64,
    mut emit: F,
) {
    unsafe {
        let event_type = msg_send!(event, c"type"; -> usize);
        match event_type {
            // NSLeftMouseDown = 1, NSRightMouseDown = 3, NSOtherMouseDown = 25
            1 | 3 | 25 => {
                let loc = msg_send!(event, c"locationInWindow"; -> NSPoint);
                let pt = if !view.is_null() {
                    msg_send!(view, c"convertPoint:fromView:", loc => NSPoint, std::ptr::null_mut::<c_void>() => Id; -> NSPoint)
                } else {
                    loc
                };
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
                let loc = msg_send!(event, c"locationInWindow"; -> NSPoint);
                let pt = if !view.is_null() {
                    msg_send!(view, c"convertPoint:fromView:", loc => NSPoint, std::ptr::null_mut::<c_void>() => Id; -> NSPoint)
                } else {
                    loc
                };
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
                let loc = msg_send!(event, c"locationInWindow"; -> NSPoint);
                let pt = if !view.is_null() {
                    msg_send!(view, c"convertPoint:fromView:", loc => NSPoint, std::ptr::null_mut::<c_void>() => Id; -> NSPoint)
                } else {
                    loc
                };
                let pos = PhysicalPosition::new(pt.x * scale, pt.y * scale);
                emit(WindowEvent::CursorMoved(pos));
            }
            // NSScrollWheel = 22
            22 => {
                let loc = msg_send!(event, c"locationInWindow"; -> NSPoint);
                let pt = if !view.is_null() {
                    msg_send!(view, c"convertPoint:fromView:", loc => NSPoint, std::ptr::null_mut::<c_void>() => Id; -> NSPoint)
                } else {
                    loc
                };
                let pos = PhysicalPosition::new(pt.x * scale, pt.y * scale);
                emit(WindowEvent::CursorMoved(pos));

                let dy = msg_send!(event, c"scrollingDeltaY"; -> f64);
                let has_prec = msg_send!(event, c"hasPreciseScrollingDeltas"; -> bool);
                let delta = if has_prec {
                    ScrollDelta::Pixels(PhysicalPosition::new(0.0, dy * scale))
                } else {
                    ScrollDelta::Lines { y: dy as f32 }
                };
                emit(WindowEvent::MouseWheel { delta, phase: TouchPhase::Moved });
            }
            // NSKeyDown = 10, NSKeyUp = 11
            10 | 11 => {
                let pressed = event_type == 10;
                let key_input = convert_key_event(event, pressed);
                emit(WindowEvent::Key(key_input));
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

#[allow(dead_code)]
pub unsafe fn convert_ns_event(event: Id, _id: WindowId, scale: f64) -> Option<WindowEvent> {
    let mut result = None;
    unsafe {
        process_ns_event(event, std::ptr::null_mut(), scale, |ev| {
            result = Some(ev);
        });
    }
    result
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

unsafe fn convert_key_event(event: Id, pressed: bool) -> KeyInput {
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
}
