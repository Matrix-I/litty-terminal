//! Linux native event conversions for Wayland and X11 protocols.

use crate::platform::gui::{
    ButtonState, MouseButton, PhysicalPosition, PhysicalSize, ScrollDelta, TouchPhase,
    WindowEvent,
};
use crate::platform::input::{Key, KeyCode, KeyInput, KeyLocation, Mods, NamedKey, PhysicalKey};

#[allow(dead_code)]
#[derive(Copy, Clone, Debug)]
pub enum LinuxRawEvent {
    Close,
    Configure { width: u32, height: u32, scale: f64 },
    Focus(bool),
    Motion { x: f64, y: f64 },
    Button { button: u32, pressed: bool },
    Axis { vertical: f64, horizontal: f64 },
    Key { keycode: u32, pressed: bool, repeat: bool },
    Modifiers { ctrl: bool, alt: bool, shift: bool, sup: bool },
}

#[allow(dead_code)]
pub fn convert_raw_event(ev: LinuxRawEvent, scale: f64) -> Option<WindowEvent> {
    match ev {
        LinuxRawEvent::Close => Some(WindowEvent::CloseRequested),
        LinuxRawEvent::Configure { width, height, scale: new_scale } => {
            if (new_scale - scale).abs() > 0.001 {
                Some(WindowEvent::ScaleFactorChanged(new_scale))
            } else {
                Some(WindowEvent::Resized(PhysicalSize::new(width, height)))
            }
        }
        LinuxRawEvent::Focus(focused) => Some(WindowEvent::Focused(focused)),
        LinuxRawEvent::Motion { x, y } => {
            Some(WindowEvent::CursorMoved(PhysicalPosition::new(x * scale, y * scale)))
        }
        LinuxRawEvent::Button { button, pressed } => {
            let state = if pressed { ButtonState::Pressed } else { ButtonState::Released };
            let mouse_button = match button {
                1 | 0x110 => MouseButton::Left,    // BTN_LEFT
                2 | 0x112 => MouseButton::Middle,  // BTN_MIDDLE
                3 | 0x111 => MouseButton::Right,   // BTN_RIGHT
                _ => MouseButton::Other,
            };
            Some(WindowEvent::MouseInput { state, button: mouse_button })
        }
        LinuxRawEvent::Axis { vertical, .. } => {
            let delta = ScrollDelta::Lines { y: -vertical as f32 };
            Some(WindowEvent::MouseWheel { delta, phase: TouchPhase::Moved })
        }
        LinuxRawEvent::Key { keycode, pressed, repeat } => {
            Some(WindowEvent::Key(convert_linux_key(keycode, pressed, repeat)))
        }
        LinuxRawEvent::Modifiers { ctrl, alt, shift, sup } => {
            Some(WindowEvent::ModifiersChanged(Mods { ctrl, alt, shift, sup }))
        }
    }
}

/// Modifiers in an X11 core event `state` mask (Shift, Control, Mod1 = Alt,
/// Mod4 = Super).
pub fn mods_from_state(state: u32) -> Mods {
    Mods {
        shift: state & 1 != 0,
        ctrl: state & 4 != 0,
        alt: state & 8 != 0,
        sup: state & 64 != 0,
    }
}

/// Modifiers once a key event is applied. X11 reports `state` as it was
/// before the event, so a modifier key's own press or release is folded in.
pub fn mods_after_key(state: u32, evdev: u32, pressed: bool) -> Mods {
    let mut m = mods_from_state(state);
    let slot = match evdev {
        29 | 97 => &mut m.ctrl,   // KEY_LEFTCTRL, KEY_RIGHTCTRL
        42 | 54 => &mut m.shift,  // KEY_LEFTSHIFT, KEY_RIGHTSHIFT
        56 | 100 => &mut m.alt,   // KEY_LEFTALT, KEY_RIGHTALT
        125 | 126 => &mut m.sup,  // KEY_LEFTMETA, KEY_RIGHTMETA
        _ => return m,
    };
    *slot = pressed;
    m
}

#[allow(dead_code)]
pub fn convert_linux_key(keycode: u32, pressed: bool, repeat: bool) -> KeyInput {
    let evdev = keycode;
    let code = match evdev {
        1 => KeyCode::Other(1), // ESC
        2 => KeyCode::Digit1,
        3 => KeyCode::Digit2,
        4 => KeyCode::Digit3,
        5 => KeyCode::Digit4,
        6 => KeyCode::Digit5,
        7 => KeyCode::Digit6,
        8 => KeyCode::Digit7,
        9 => KeyCode::Digit8,
        10 => KeyCode::Digit9,
        11 => KeyCode::Digit0,
        12 => KeyCode::Minus,
        13 => KeyCode::Equal,
        14 => KeyCode::Other(14), // Backspace
        15 => KeyCode::Other(15), // Tab
        16 => KeyCode::KeyQ,
        17 => KeyCode::KeyW,
        18 => KeyCode::KeyE,
        19 => KeyCode::KeyR,
        20 => KeyCode::KeyT,
        21 => KeyCode::KeyY,
        22 => KeyCode::KeyU,
        23 => KeyCode::KeyI,
        24 => KeyCode::KeyO,
        25 => KeyCode::KeyP,
        26 => KeyCode::BracketLeft,
        27 => KeyCode::BracketRight,
        28 => KeyCode::Other(28), // Enter
        30 => KeyCode::KeyA,
        31 => KeyCode::KeyS,
        32 => KeyCode::KeyD,
        33 => KeyCode::KeyF,
        34 => KeyCode::KeyG,
        35 => KeyCode::KeyH,
        36 => KeyCode::KeyJ,
        37 => KeyCode::KeyK,
        38 => KeyCode::KeyL,
        39 => KeyCode::Semicolon,
        40 => KeyCode::Quote,
        41 => KeyCode::Backquote,
        43 => KeyCode::Backslash,
        44 => KeyCode::KeyZ,
        45 => KeyCode::KeyX,
        46 => KeyCode::KeyC,
        47 => KeyCode::KeyV,
        48 => KeyCode::KeyB,
        49 => KeyCode::KeyN,
        50 => KeyCode::KeyM,
        51 => KeyCode::Comma,
        52 => KeyCode::Period,
        53 => KeyCode::Slash,
        57 => KeyCode::Space,
        102 => KeyCode::Other(102), // Home
        103 => KeyCode::Other(103), // Up
        104 => KeyCode::Other(104), // PageUp
        105 => KeyCode::Other(105), // Left
        106 => KeyCode::Other(106), // Right
        107 => KeyCode::Other(107), // End
        108 => KeyCode::Other(108), // Down
        109 => KeyCode::Other(109), // PageDown
        110 => KeyCode::Other(110), // Insert
        111 => KeyCode::Other(111), // Delete
        _ => KeyCode::Other(evdev),
    };

    let named = match evdev {
        1 => Some(NamedKey::Escape),
        14 => Some(NamedKey::Backspace),
        15 => Some(NamedKey::Tab),
        28 => Some(NamedKey::Enter),
        57 => Some(NamedKey::Space),
        102 => Some(NamedKey::Home),
        103 => Some(NamedKey::ArrowUp),
        104 => Some(NamedKey::PageUp),
        105 => Some(NamedKey::ArrowLeft),
        106 => Some(NamedKey::ArrowRight),
        107 => Some(NamedKey::End),
        108 => Some(NamedKey::ArrowDown),
        109 => Some(NamedKey::PageDown),
        110 => Some(NamedKey::Insert),
        111 => Some(NamedKey::Delete),
        _ => None,
    };

    let logical = if let Some(n) = named {
        Key::Named(n)
    } else {
        match code {
            KeyCode::KeyA => Key::Character("a".into()),
            KeyCode::KeyB => Key::Character("b".into()),
            KeyCode::KeyC => Key::Character("c".into()),
            KeyCode::KeyD => Key::Character("d".into()),
            KeyCode::KeyE => Key::Character("e".into()),
            KeyCode::KeyF => Key::Character("f".into()),
            KeyCode::KeyG => Key::Character("g".into()),
            KeyCode::KeyH => Key::Character("h".into()),
            KeyCode::KeyI => Key::Character("i".into()),
            KeyCode::KeyJ => Key::Character("j".into()),
            KeyCode::KeyK => Key::Character("k".into()),
            KeyCode::KeyL => Key::Character("l".into()),
            KeyCode::KeyM => Key::Character("m".into()),
            KeyCode::KeyN => Key::Character("n".into()),
            KeyCode::KeyO => Key::Character("o".into()),
            KeyCode::KeyP => Key::Character("p".into()),
            KeyCode::KeyQ => Key::Character("q".into()),
            KeyCode::KeyR => Key::Character("r".into()),
            KeyCode::KeyS => Key::Character("s".into()),
            KeyCode::KeyT => Key::Character("t".into()),
            KeyCode::KeyU => Key::Character("u".into()),
            KeyCode::KeyV => Key::Character("v".into()),
            KeyCode::KeyW => Key::Character("w".into()),
            KeyCode::KeyX => Key::Character("x".into()),
            KeyCode::KeyY => Key::Character("y".into()),
            KeyCode::KeyZ => Key::Character("z".into()),
            KeyCode::Digit1 => Key::Character("1".into()),
            KeyCode::Digit2 => Key::Character("2".into()),
            KeyCode::Digit3 => Key::Character("3".into()),
            KeyCode::Digit4 => Key::Character("4".into()),
            KeyCode::Digit5 => Key::Character("5".into()),
            KeyCode::Digit6 => Key::Character("6".into()),
            KeyCode::Digit7 => Key::Character("7".into()),
            KeyCode::Digit8 => Key::Character("8".into()),
            KeyCode::Digit9 => Key::Character("9".into()),
            KeyCode::Digit0 => Key::Character("0".into()),
            KeyCode::Minus => Key::Character("-".into()),
            KeyCode::Equal => Key::Character("=".into()),
            KeyCode::BracketLeft => Key::Character("[".into()),
            KeyCode::BracketRight => Key::Character("]".into()),
            KeyCode::Backslash => Key::Character("\\".into()),
            KeyCode::Semicolon => Key::Character(";".into()),
            KeyCode::Quote => Key::Character("'".into()),
            KeyCode::Backquote => Key::Character("`".into()),
            KeyCode::Comma => Key::Character(",".into()),
            KeyCode::Period => Key::Character(".".into()),
            KeyCode::Slash => Key::Character("/".into()),
            _ => Key::Unidentified,
        }
    };

    let text = match &logical {
        Key::Character(s) => Some(s.clone()),
        _ => None,
    };

    KeyInput {
        logical: logical.clone(),
        unshifted: logical,
        physical: PhysicalKey::Code(code),
        location: KeyLocation::Standard,
        text,
        pressed,
        repeat,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHIFT: u32 = 1;
    const CTRL: u32 = 4;

    #[test]
    fn mods_come_from_x11_state() {
        // T (evdev 20) pressed while Ctrl+Shift are held.
        assert_eq!(mods_after_key(CTRL | SHIFT, 20, true), Mods { ctrl: true, shift: true, ..Mods::default() });
        assert_eq!(mods_after_key(0, 20, true), Mods::default());
        assert_eq!(mods_after_key(8, 20, true), Mods { alt: true, ..Mods::default() });
        assert_eq!(mods_after_key(64, 20, true), Mods { sup: true, ..Mods::default() });
    }

    /// X11 `state` is the mask *before* the event, so a modifier key's own
    /// press or release must be applied on top of it.
    #[test]
    fn modifier_key_press_and_release_apply_immediately() {
        assert_eq!(mods_after_key(0, 29, true), Mods { ctrl: true, ..Mods::default() }); // LCtrl down
        assert_eq!(mods_after_key(CTRL, 42, true), Mods { ctrl: true, shift: true, ..Mods::default() }); // LShift down
        assert_eq!(mods_after_key(CTRL | SHIFT, 97, false), Mods { shift: true, ..Mods::default() }); // RCtrl up
        assert_eq!(mods_after_key(SHIFT, 54, false), Mods::default()); // RShift up
    }
}
