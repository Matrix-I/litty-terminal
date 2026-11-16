//! Keyboard twins: no Secure Keyboard Entry, Caps Lock state or beep on Linux.

/// Only read on macOS; Linux key events infer the lock state from the keys typed.
pub fn caps_lock() -> bool {
    false
}

/// No OS-wide secure keyboard entry exists on X11 or Wayland.
pub fn set_secure_input(_on: bool) {}

/// No system alert sound API; `bell-features = audio` plays a file instead.
pub fn beep() {}
