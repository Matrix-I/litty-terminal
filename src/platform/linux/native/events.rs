//! Linux native event conversions for Wayland and X11 protocols.

use crate::platform::gui::{
    ButtonState, MouseButton, PhysicalPosition, PhysicalSize, ScrollDelta, TouchPhase,
    WindowEvent,
};
use crate::platform::input::{Key, KeyCode, KeyInput, KeyLocation, Mods, NamedKey, PhysicalKey};
use std::ffi::{c_int, c_ulong};
use std::sync::atomic::{AtomicU64, Ordering};

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
            Some(WindowEvent::Key(convert_linux_key(keycode, 0, pressed, repeat)))
        }
        LinuxRawEvent::Modifiers { ctrl, alt, shift, sup } => {
            Some(WindowEvent::ModifiersChanged(Mods { ctrl, alt, shift, sup }))
        }
    }
}

/// Atoms of the `_NET_WM_STATE` values the app tracks.
#[derive(Copy, Clone, Debug)]
pub struct WmStateAtoms {
    pub max_vert: c_ulong,
    pub max_horz: c_ulong,
    pub fullscreen: c_ulong,
    pub hidden: c_ulong,
}

/// Whether the window manager lists the window as hidden: EWMH sets it for a
/// window that would not show even on the current workspace (minimized).
/// Some managers (mutter) minimize without unmapping, so this is all they
/// report.
pub fn wm_state_hidden(atoms: &[c_ulong], names: WmStateAtoms) -> bool {
    atoms.contains(&names.hidden)
}

/// `VisibilityNotify` state `VisibilityFullyObscured`.
pub const VISIBILITY_FULLY_OBSCURED: c_int = 2;

/// What makes a window invisible, from the three sources X11 offers: no
/// single one is reliable (`VisibilityNotify` is mostly `Unobscured` under
/// a compositor, `_NET_WM_STATE_HIDDEN` is EWMH only, and an unmapped
/// window may still be listed), so any of them hides the window.
#[derive(Debug, Default)]
pub struct OcclusionTracker {
    unmapped: bool,
    hidden: bool,
    obscured: bool,
    /// The state last handed to the app; the app assumes visible.
    reported: bool,
}

impl OcclusionTracker {
    fn changed(&mut self) -> Option<WindowEvent> {
        let occluded = self.unmapped || self.hidden || self.obscured;
        (std::mem::replace(&mut self.reported, occluded) != occluded).then_some(WindowEvent::Occluded(occluded))
    }

    /// `MapNotify` / `UnmapNotify`. An unmapped window gets no visibility
    /// events, so a stale `FullyObscured` is dropped with it.
    pub fn set_mapped(&mut self, mapped: bool) -> Option<WindowEvent> {
        self.unmapped = !mapped;
        self.obscured = false;
        self.changed()
    }

    /// `_NET_WM_STATE` listed (or no longer lists) `_NET_WM_STATE_HIDDEN`.
    pub fn set_hidden(&mut self, hidden: bool) -> Option<WindowEvent> {
        self.hidden = hidden;
        self.changed()
    }

    /// `VisibilityNotify` state.
    pub fn set_visibility(&mut self, state: c_int) -> Option<WindowEvent> {
        self.obscured = state == VISIBILITY_FULLY_OBSCURED;
        self.changed()
    }
}

/// Window state from a `_NET_WM_STATE` atom list. Maximized needs both axes:
/// a half-screen tile is vertical only and keeps its free edge resizable.
pub fn wm_state_event(atoms: &[c_ulong], names: WmStateAtoms) -> WindowEvent {
    let has = |atom| atoms.contains(&atom);
    WindowEvent::StateChanged {
        maximized: has(names.max_vert) && has(names.max_horz),
        fullscreen: has(names.fullscreen),
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

/// X11 `state` bit for Num Lock (`Mod2` on every common X keymap).
pub const NUM_LOCK_MASK: u32 = 16;

/// Logical key of a keypad key: digits and `.` when `digits`, navigation keys
/// otherwise; `None` for keys outside the keypad.
fn keypad_logical(evdev: u32, digits: bool) -> Option<Key> {
    let (digit, nav) = match evdev {
        79 => ("1", NamedKey::End),
        80 => ("2", NamedKey::ArrowDown),
        81 => ("3", NamedKey::PageDown),
        75 => ("4", NamedKey::ArrowLeft),
        76 => ("5", NamedKey::Clear),
        77 => ("6", NamedKey::ArrowRight),
        71 => ("7", NamedKey::Home),
        72 => ("8", NamedKey::ArrowUp),
        73 => ("9", NamedKey::PageUp),
        82 => ("0", NamedKey::Insert),
        83 => (".", NamedKey::Delete),
        55 => return Some(Key::Character("*".into())),
        74 => return Some(Key::Character("-".into())),
        78 => return Some(Key::Character("+".into())),
        98 => return Some(Key::Character("/".into())),
        117 => return Some(Key::Character("=".into())),
        121 => return Some(Key::Character(",".into())),
        96 => return Some(Key::Named(NamedKey::Enter)),
        _ => return None,
    };
    Some(if digits { Key::Character(digit.into()) } else { Key::Named(nav) })
}

/// A keypad key event, or `None` for keys outside the keypad. With Num Lock
/// on the keypad types digits; Shift inverts that, as in the X core keymap.
fn keypad_key(evdev: u32, state: u32, pressed: bool, repeat: bool) -> Option<KeyInput> {
    let num_lock = state & NUM_LOCK_MASK != 0;
    let logical = keypad_logical(evdev, num_lock != (state & 1 != 0))?;
    let unshifted = keypad_logical(evdev, num_lock)?;
    let text = match &logical {
        Key::Character(s) => Some(s.clone()),
        _ => None,
    };
    let code = if evdev == 76 { KeyCode::Numpad5 } else { KeyCode::Other(evdev) };
    Some(KeyInput {
        logical,
        unshifted,
        physical: PhysicalKey::Code(code),
        location: KeyLocation::Numpad,
        text,
        pressed,
        repeat,
    })
}

/// Key event for an evdev code under the X11 modifier `state` (which only
/// matters for the keypad).
#[allow(dead_code)]
pub fn convert_linux_key(keycode: u32, state: u32, pressed: bool, repeat: bool) -> KeyInput {
    let evdev = keycode;
    if let Some(key) = keypad_key(evdev, state, pressed, repeat) {
        return key;
    }
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
        69 => Some(NamedKey::NumLock),
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

/// Pointer event for an X11 core button. Buttons 4/5 are one wheel notch
/// up/down on press; 6/7 (horizontal wheel) have no `ScrollDelta` axis, and
/// no wheel button produces a release.
pub fn x11_button_event(button: u32, pressed: bool) -> Option<WindowEvent> {
    let notch = match button {
        4 => 1.0,
        5 => -1.0,
        6 | 7 => return None,
        _ => {
            let state = if pressed { ButtonState::Pressed } else { ButtonState::Released };
            let button = match button {
                1 => MouseButton::Left,
                2 => MouseButton::Middle,
                3 => MouseButton::Right,
                _ => MouseButton::Other,
            };
            return Some(WindowEvent::MouseInput { state, button });
        }
    };
    pressed.then_some(WindowEvent::MouseWheel { delta: ScrollDelta::Lines { y: notch }, phase: TouchPhase::Moved })
}

/// Character for keysyms with a fixed Unicode mapping: Latin-1 and the
/// `0x0100_0000 + code point` range. Other legacy keysyms need xkbcommon.
pub fn keysym_char(keysym: u32) -> Option<char> {
    match keysym {
        0x20..=0x7e | 0xa0..=0xff => char::from_u32(keysym),
        0x0100_00a0..=0x0110_ffff => char::from_u32(keysym - 0x0100_0000),
        _ => None,
    }
}

/// X11 keycodes currently held, to tell auto-repeat presses (detectable
/// auto-repeat sends press, press, ..., release) from fresh ones.
#[derive(Default)]
pub struct HeldKeys([AtomicU64; 4]);

impl HeldKeys {
    /// Record a key event; true when the key was already held: a press is
    /// then an auto-repeat, a release matches a reported press.
    pub fn update(&self, keycode: u8, pressed: bool) -> bool {
        let (word, bit) = (&self.0[usize::from(keycode >> 6)], 1u64 << (keycode & 63));
        let before = if pressed {
            word.fetch_or(bit, Ordering::Relaxed)
        } else {
            word.fetch_and(!bit, Ordering::Relaxed)
        };
        before & bit != 0
    }

    /// Forget every key: releases are not delivered while unfocused.
    pub fn clear(&self) {
        for word in &self.0 {
            word.store(0, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WM: WmStateAtoms = WmStateAtoms { max_vert: 301, max_horz: 302, fullscreen: 303, hidden: 304 };

    fn wm_state(atoms: &[std::ffi::c_ulong]) -> (bool, bool) {
        match wm_state_event(atoms, WM) {
            WindowEvent::StateChanged { maximized, fullscreen } => (maximized, fullscreen),
            other => panic!("expected StateChanged, got {other:?}"),
        }
    }

    /// The window manager's `_NET_WM_STATE` is the truth about maximize,
    /// whoever changed it (title-bar drag, Super+Up, tiling).
    #[test]
    fn wm_state_reports_maximized_only_on_both_axes() {
        assert_eq!(wm_state(&[301, 302]), (true, false));
        assert_eq!(wm_state(&[999, 302, 301]), (true, false), "other atoms are ignored");
        // A half-screen tile is vertical only and keeps its free edge resizable.
        assert_eq!(wm_state(&[301]), (false, false));
        assert_eq!(wm_state(&[303]), (false, true));
    }

    /// An empty or deleted property means a plain, restored window.
    #[test]
    fn empty_wm_state_is_a_restored_window() {
        assert_eq!(wm_state(&[]), (false, false));
    }

    fn occluded(event: Option<WindowEvent>) -> Option<bool> {
        event.map(|e| match e {
            WindowEvent::Occluded(o) => o,
            other => panic!("expected Occluded, got {other:?}"),
        })
    }

    #[test]
    fn wm_state_hidden_is_read_from_its_atom() {
        assert!(wm_state_hidden(&[301, 304], WM));
        assert!(!wm_state_hidden(&[301, 302, 303], WM));
        assert!(!wm_state_hidden(&[], WM));
    }

    #[test]
    fn unmapping_and_mapping_report_occlusion_once_each() {
        let mut t = OcclusionTracker::default();
        assert_eq!(occluded(t.set_mapped(true)), None, "already visible: nothing to report");
        assert_eq!(occluded(t.set_mapped(false)), Some(true));
        assert_eq!(occluded(t.set_mapped(false)), None);
        assert_eq!(occluded(t.set_mapped(true)), Some(false));
    }

    #[test]
    fn fully_obscured_hides_the_window_and_partial_does_not() {
        let mut t = OcclusionTracker::default();
        assert_eq!(occluded(t.set_visibility(1)), None, "partly obscured still draws");
        assert_eq!(occluded(t.set_visibility(VISIBILITY_FULLY_OBSCURED)), Some(true));
        assert_eq!(occluded(t.set_visibility(1)), Some(false));
        assert_eq!(occluded(t.set_visibility(0)), None);
    }

    /// Any source keeps the window occluded: a compositor that says
    /// `Unobscured` does not undo a minimize.
    #[test]
    fn the_window_stays_occluded_until_every_source_clears() {
        let mut t = OcclusionTracker::default();
        assert_eq!(occluded(t.set_hidden(true)), Some(true));
        assert_eq!(occluded(t.set_visibility(0)), None);
        assert_eq!(occluded(t.set_mapped(false)), None);
        assert_eq!(occluded(t.set_mapped(true)), None, "still listed as hidden");
        assert_eq!(occluded(t.set_hidden(false)), Some(false));
    }

    #[test]
    fn unmapping_forgets_a_stale_obscured_state() {
        let mut t = OcclusionTracker::default();
        assert_eq!(occluded(t.set_visibility(VISIBILITY_FULLY_OBSCURED)), Some(true));
        assert_eq!(occluded(t.set_mapped(false)), None);
        // After the window is shown again, no visibility event has arrived yet.
        assert_eq!(occluded(t.set_mapped(true)), Some(false));
    }

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

    #[test]
    fn wheel_buttons_scroll_one_line_per_press() {
        let notch = |button| match x11_button_event(button, true) {
            Some(WindowEvent::MouseWheel { delta: ScrollDelta::Lines { y }, phase: TouchPhase::Moved }) => Some(y),
            _ => None,
        };
        assert_eq!(notch(4), Some(1.0));
        assert_eq!(notch(5), Some(-1.0));
        for button in 4..=7 {
            assert!(x11_button_event(button, false).is_none(), "button {button} release");
        }
        assert!(x11_button_event(6, true).is_none());
        assert!(matches!(
            x11_button_event(1, true),
            Some(WindowEvent::MouseInput { state: ButtonState::Pressed, button: MouseButton::Left })
        ));
        assert!(matches!(
            x11_button_event(3, false),
            Some(WindowEvent::MouseInput { state: ButtonState::Released, button: MouseButton::Right })
        ));
    }

    #[test]
    fn keysyms_map_to_unicode_beyond_latin1() {
        assert_eq!(keysym_char(0x61), Some('a'));
        assert_eq!(keysym_char(0xe9), Some('é'));
        assert_eq!(keysym_char(0x0100_1ea1), Some('ạ')); // Vietnamese layout
        assert_eq!(keysym_char(0x0100_0111), Some('đ'));
        assert_eq!(keysym_char(0xff0d), None); // Return
        assert_eq!(keysym_char(0x0100_d800), None); // surrogate
    }

    /// Num Lock picks digits or navigation keys, Shift inverts it (as X
    /// does), and every keypad key reports the numpad location (TD-11).
    #[test]
    fn keypad_follows_num_lock() {
        let key = |evdev, state| convert_linux_key(evdev, state, true, false);
        let kp7 = key(71, NUM_LOCK_MASK);
        assert_eq!((kp7.logical, kp7.text.as_deref(), kp7.location), (Key::Character("7".into()), Some("7"), KeyLocation::Numpad));
        let home = key(71, 0);
        assert_eq!((home.logical, home.text, home.location), (Key::Named(NamedKey::Home), None, KeyLocation::Numpad));
        let shifted = key(71, NUM_LOCK_MASK | SHIFT);
        assert_eq!((shifted.logical, shifted.unshifted), (Key::Named(NamedKey::Home), Key::Character("7".into())));
        assert_eq!(key(71, SHIFT).logical, Key::Character("7".into()));
        assert_eq!(key(76, 0).logical, Key::Named(NamedKey::Clear));
        assert_eq!(key(76, NUM_LOCK_MASK).physical, PhysicalKey::Code(KeyCode::Numpad5));
        assert_eq!(key(83, NUM_LOCK_MASK).logical, Key::Character(".".into()));
        assert_eq!(key(83, 0).logical, Key::Named(NamedKey::Delete));
        for (evdev, op) in [(55, "*"), (74, "-"), (78, "+"), (98, "/"), (117, "="), (121, ",")] {
            let k = key(evdev, 0);
            assert_eq!((k.logical, k.text.as_deref(), k.location), (Key::Character(op.into()), Some(op), KeyLocation::Numpad), "evdev {evdev}");
        }
        let enter = key(96, NUM_LOCK_MASK);
        assert_eq!((enter.logical, enter.text, enter.location), (Key::Named(NamedKey::Enter), None, KeyLocation::Numpad));
        let num_lock = key(69, 0);
        assert_eq!((num_lock.logical, num_lock.location), (Key::Named(NamedKey::NumLock), KeyLocation::Standard));
        // Main-block keys are unaffected by Num Lock.
        assert_eq!(key(8, NUM_LOCK_MASK).location, KeyLocation::Standard);
        assert_eq!(key(102, NUM_LOCK_MASK).logical, Key::Named(NamedKey::Home));
    }

    #[test]
    fn held_keys_mark_auto_repeat() {
        let held = HeldKeys::default();
        assert!(!held.update(38, true));
        assert!(held.update(38, true), "second press without release is a repeat");
        assert!(!held.update(255, true));
        assert!(held.update(38, false), "release of a held key");
        assert!(!held.update(38, false), "release without a press");
        assert!(!held.update(38, true), "press after release is fresh");
        held.clear();
        assert!(!held.update(38, true), "focus loss forgets held keys");
        // Highest keycodes share the last word without aliasing.
        assert!(!held.update(255, true) && held.update(255, true));
        assert!(!held.update(254, true));
    }
}
