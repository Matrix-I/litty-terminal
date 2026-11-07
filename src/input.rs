//! Windowing-backend-independent keyboard types. Native backends (AppKit,
//! Wayland, X11) translate their key events into these before `keymap`
//! resolves shortcuts or encodes bytes for the shell, so `keymap` stays a pure
//! function of this module's types.

/// Logical key produced by the layout, as opposed to its physical position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    /// Layout-translated text of a character key.
    Character(String),
    Named(NamedKey),
    /// A key with no logical meaning for the terminal (dead keys, unmapped keys).
    Unidentified,
}

/// Non-text keys the terminal acts on.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NamedKey {
    Enter,
    Tab,
    Space,
    Backspace,
    Escape,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    F13,
    F14,
    F15,
    F16,
    F17,
    F18,
    F19,
    F20,
    Shift,
    Control,
    Alt,
    Super,
    Meta,
    Hyper,
    CapsLock,
    NumLock,
    ScrollLock,
    PrintScreen,
    Pause,
    ContextMenu,
    Clear,
    MediaPlay,
    MediaPause,
    MediaPlayPause,
    MediaStop,
    MediaRecord,
    MediaFastForward,
    MediaRewind,
    MediaTrackNext,
    MediaTrackPrevious,
    AudioVolumeDown,
    AudioVolumeUp,
    AudioVolumeMute,
}

/// Where a key sits on the keyboard, independent of the layout.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum KeyCode {
    KeyA,
    KeyB,
    KeyC,
    KeyD,
    KeyE,
    KeyF,
    KeyG,
    KeyH,
    KeyI,
    KeyJ,
    KeyK,
    KeyL,
    KeyM,
    KeyN,
    KeyO,
    KeyP,
    KeyQ,
    KeyR,
    KeyS,
    KeyT,
    KeyU,
    KeyV,
    KeyW,
    KeyX,
    KeyY,
    KeyZ,
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    Backquote,
    Minus,
    Equal,
    BracketLeft,
    BracketRight,
    Backslash,
    Semicolon,
    Quote,
    Comma,
    Period,
    Slash,
    Space,
    Numpad5,
    /// Any other key, identified by a backend scancode so that releases still
    /// match their presses.
    Other(u32),
}

/// A physical key, or the backend's scancode when the key has no `KeyCode`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PhysicalKey {
    Code(KeyCode),
    Unidentified(u32),
}

impl PhysicalKey {
    /// The key position, or `None` when the backend could not identify it.
    pub fn code(self) -> Option<KeyCode> {
        match self {
            PhysicalKey::Code(KeyCode::Other(_)) | PhysicalKey::Unidentified(_) => None,
            PhysicalKey::Code(code) => Some(code),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum KeyLocation {
    Standard,
    Left,
    Right,
    Numpad,
}

/// One key press, repeat or release, already translated by the backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyInput {
    /// Logical key with the active modifiers applied.
    pub logical: Key,
    /// Logical key as if no modifier but Shift-less layout state were held.
    pub unshifted: Key,
    pub physical: PhysicalKey,
    pub location: KeyLocation,
    /// Text the key inserts, if any.
    pub text: Option<String>,
    pub pressed: bool,
    pub repeat: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_code_hides_unidentified_keys() {
        assert_eq!(PhysicalKey::Code(KeyCode::KeyA).code(), Some(KeyCode::KeyA));
        assert_eq!(PhysicalKey::Code(KeyCode::Other(7)).code(), None);
        assert_eq!(PhysicalKey::Unidentified(7).code(), None);
    }

    #[test]
    fn unidentified_scancodes_compare_by_value() {
        assert_eq!(PhysicalKey::Unidentified(3), PhysicalKey::Unidentified(3));
        assert_ne!(PhysicalKey::Unidentified(3), PhysicalKey::Unidentified(4));
    }
}
