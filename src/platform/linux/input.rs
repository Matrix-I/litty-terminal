//! Keyboard twins: no Secure Keyboard Entry, Caps Lock state or beep on Linux.

pub fn caps_lock() -> bool {
    false
}

pub fn set_secure_input(_on: bool) {}

pub fn beep() {}
