//! Interim adapter from `winit` key events to `crate::input` types. It goes
//! away together with `winit` once the native backends feed `KeyInput` directly.

use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{
    Key as WKey, KeyCode as WKeyCode, KeyLocation as WKeyLocation, ModifiersState,
    NamedKey as WNamedKey, PhysicalKey as WPhysicalKey,
};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

use crate::input::{Key, KeyCode, KeyInput, KeyLocation, NamedKey, PhysicalKey};
use crate::keymap::Mods;

pub(super) fn mods(state: ModifiersState) -> Mods {
    Mods {
        ctrl: state.control_key(),
        alt: state.alt_key(),
        shift: state.shift_key(),
        sup: state.super_key(),
    }
}

pub(super) fn key_input(ev: &KeyEvent) -> KeyInput {
    KeyInput {
        logical: key(&ev.logical_key),
        unshifted: key(&ev.key_without_modifiers()),
        physical: physical(ev.physical_key),
        location: location(ev.location),
        text: ev.text.as_ref().map(|t| t.to_string()),
        pressed: ev.state == ElementState::Pressed,
        repeat: ev.repeat,
    }
}

fn key(k: &WKey) -> Key {
    match k {
        WKey::Character(c) => Key::Character(c.to_string()),
        WKey::Named(n) => named(*n).map_or(Key::Unidentified, Key::Named),
        _ => Key::Unidentified,
    }
}

fn named(k: WNamedKey) -> Option<NamedKey> {
    Some(match k {
        WNamedKey::Enter => NamedKey::Enter,
        WNamedKey::Tab => NamedKey::Tab,
        WNamedKey::Space => NamedKey::Space,
        WNamedKey::Backspace => NamedKey::Backspace,
        WNamedKey::Escape => NamedKey::Escape,
        WNamedKey::Delete => NamedKey::Delete,
        WNamedKey::Insert => NamedKey::Insert,
        WNamedKey::Home => NamedKey::Home,
        WNamedKey::End => NamedKey::End,
        WNamedKey::PageUp => NamedKey::PageUp,
        WNamedKey::PageDown => NamedKey::PageDown,
        WNamedKey::ArrowUp => NamedKey::ArrowUp,
        WNamedKey::ArrowDown => NamedKey::ArrowDown,
        WNamedKey::ArrowLeft => NamedKey::ArrowLeft,
        WNamedKey::ArrowRight => NamedKey::ArrowRight,
        WNamedKey::F1 => NamedKey::F1,
        WNamedKey::F2 => NamedKey::F2,
        WNamedKey::F3 => NamedKey::F3,
        WNamedKey::F4 => NamedKey::F4,
        WNamedKey::F5 => NamedKey::F5,
        WNamedKey::F6 => NamedKey::F6,
        WNamedKey::F7 => NamedKey::F7,
        WNamedKey::F8 => NamedKey::F8,
        WNamedKey::F9 => NamedKey::F9,
        WNamedKey::F10 => NamedKey::F10,
        WNamedKey::F11 => NamedKey::F11,
        WNamedKey::F12 => NamedKey::F12,
        WNamedKey::F13 => NamedKey::F13,
        WNamedKey::F14 => NamedKey::F14,
        WNamedKey::F15 => NamedKey::F15,
        WNamedKey::F16 => NamedKey::F16,
        WNamedKey::F17 => NamedKey::F17,
        WNamedKey::F18 => NamedKey::F18,
        WNamedKey::F19 => NamedKey::F19,
        WNamedKey::F20 => NamedKey::F20,
        WNamedKey::Shift => NamedKey::Shift,
        WNamedKey::Control => NamedKey::Control,
        WNamedKey::Alt => NamedKey::Alt,
        WNamedKey::Super => NamedKey::Super,
        WNamedKey::Meta => NamedKey::Meta,
        WNamedKey::Hyper => NamedKey::Hyper,
        WNamedKey::CapsLock => NamedKey::CapsLock,
        WNamedKey::NumLock => NamedKey::NumLock,
        WNamedKey::ScrollLock => NamedKey::ScrollLock,
        WNamedKey::PrintScreen => NamedKey::PrintScreen,
        WNamedKey::Pause => NamedKey::Pause,
        WNamedKey::ContextMenu => NamedKey::ContextMenu,
        WNamedKey::Clear => NamedKey::Clear,
        WNamedKey::MediaPlay => NamedKey::MediaPlay,
        WNamedKey::MediaPause => NamedKey::MediaPause,
        WNamedKey::MediaPlayPause => NamedKey::MediaPlayPause,
        WNamedKey::MediaStop => NamedKey::MediaStop,
        WNamedKey::MediaRecord => NamedKey::MediaRecord,
        WNamedKey::MediaFastForward => NamedKey::MediaFastForward,
        WNamedKey::MediaRewind => NamedKey::MediaRewind,
        WNamedKey::MediaTrackNext => NamedKey::MediaTrackNext,
        WNamedKey::MediaTrackPrevious => NamedKey::MediaTrackPrevious,
        WNamedKey::AudioVolumeDown => NamedKey::AudioVolumeDown,
        WNamedKey::AudioVolumeUp => NamedKey::AudioVolumeUp,
        WNamedKey::AudioVolumeMute => NamedKey::AudioVolumeMute,
        _ => return None,
    })
}

fn physical(k: WPhysicalKey) -> PhysicalKey {
    match k {
        WPhysicalKey::Code(c) => PhysicalKey::Code(code(c)),
        // The native scancode is opaque; one shared value is enough because a
        // release only needs to find its press among at most 16 consumed keys.
        WPhysicalKey::Unidentified(_) => PhysicalKey::Unidentified(0),
    }
}

fn code(c: WKeyCode) -> KeyCode {
    match c {
        WKeyCode::KeyA => KeyCode::KeyA,
        WKeyCode::KeyB => KeyCode::KeyB,
        WKeyCode::KeyC => KeyCode::KeyC,
        WKeyCode::KeyD => KeyCode::KeyD,
        WKeyCode::KeyE => KeyCode::KeyE,
        WKeyCode::KeyF => KeyCode::KeyF,
        WKeyCode::KeyG => KeyCode::KeyG,
        WKeyCode::KeyH => KeyCode::KeyH,
        WKeyCode::KeyI => KeyCode::KeyI,
        WKeyCode::KeyJ => KeyCode::KeyJ,
        WKeyCode::KeyK => KeyCode::KeyK,
        WKeyCode::KeyL => KeyCode::KeyL,
        WKeyCode::KeyM => KeyCode::KeyM,
        WKeyCode::KeyN => KeyCode::KeyN,
        WKeyCode::KeyO => KeyCode::KeyO,
        WKeyCode::KeyP => KeyCode::KeyP,
        WKeyCode::KeyQ => KeyCode::KeyQ,
        WKeyCode::KeyR => KeyCode::KeyR,
        WKeyCode::KeyS => KeyCode::KeyS,
        WKeyCode::KeyT => KeyCode::KeyT,
        WKeyCode::KeyU => KeyCode::KeyU,
        WKeyCode::KeyV => KeyCode::KeyV,
        WKeyCode::KeyW => KeyCode::KeyW,
        WKeyCode::KeyX => KeyCode::KeyX,
        WKeyCode::KeyY => KeyCode::KeyY,
        WKeyCode::KeyZ => KeyCode::KeyZ,
        WKeyCode::Digit0 => KeyCode::Digit0,
        WKeyCode::Digit1 => KeyCode::Digit1,
        WKeyCode::Digit2 => KeyCode::Digit2,
        WKeyCode::Digit3 => KeyCode::Digit3,
        WKeyCode::Digit4 => KeyCode::Digit4,
        WKeyCode::Digit5 => KeyCode::Digit5,
        WKeyCode::Digit6 => KeyCode::Digit6,
        WKeyCode::Digit7 => KeyCode::Digit7,
        WKeyCode::Digit8 => KeyCode::Digit8,
        WKeyCode::Digit9 => KeyCode::Digit9,
        WKeyCode::Backquote => KeyCode::Backquote,
        WKeyCode::Minus => KeyCode::Minus,
        WKeyCode::Equal => KeyCode::Equal,
        WKeyCode::BracketLeft => KeyCode::BracketLeft,
        WKeyCode::BracketRight => KeyCode::BracketRight,
        WKeyCode::Backslash => KeyCode::Backslash,
        WKeyCode::Semicolon => KeyCode::Semicolon,
        WKeyCode::Quote => KeyCode::Quote,
        WKeyCode::Comma => KeyCode::Comma,
        WKeyCode::Period => KeyCode::Period,
        WKeyCode::Slash => KeyCode::Slash,
        WKeyCode::Space => KeyCode::Space,
        WKeyCode::Numpad5 => KeyCode::Numpad5,
        other => KeyCode::Other(other as u32),
    }
}

fn location(l: WKeyLocation) -> KeyLocation {
    match l {
        WKeyLocation::Standard => KeyLocation::Standard,
        WKeyLocation::Left => KeyLocation::Left,
        WKeyLocation::Right => KeyLocation::Right,
        WKeyLocation::Numpad => KeyLocation::Numpad,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_keys_map_one_to_one() {
        assert_eq!(named(WNamedKey::Enter), Some(NamedKey::Enter));
        assert_eq!(named(WNamedKey::ArrowLeft), Some(NamedKey::ArrowLeft));
        assert_eq!(named(WNamedKey::F20), Some(NamedKey::F20));
        assert_eq!(named(WNamedKey::BrowserBack), None);
    }

    #[test]
    fn unmapped_physical_keys_keep_distinct_codes() {
        let a = physical(WPhysicalKey::Code(WKeyCode::F5));
        let b = physical(WPhysicalKey::Code(WKeyCode::F6));
        assert_ne!(a, b);
        assert_eq!(physical(WPhysicalKey::Code(WKeyCode::KeyA)), PhysicalKey::Code(KeyCode::KeyA));
    }

    #[test]
    fn modifiers_translate() {
        let m = mods(ModifiersState::CONTROL | ModifiersState::SHIFT);
        assert_eq!(m, Mods { ctrl: true, shift: true, alt: false, sup: false });
    }

    #[test]
    fn dead_and_unknown_keys_are_unidentified() {
        assert_eq!(key(&WKey::Dead(Some('`'))), Key::Unidentified);
        assert_eq!(key(&WKey::Character("a".into())), Key::Character("a".into()));
    }
}
