//! Keyboard: Secure Keyboard Entry, Caps Lock state, system beep.

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn EnableSecureEventInput() -> i32;
    fn DisableSecureEventInput() -> i32;
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    fn NSBeep();
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceFlagsState(state_id: i32) -> u64;
}

static SECURE_INPUT_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Secure Keyboard Entry: other processes cannot observe keystrokes. Calls
/// are reference-counted by macOS, so every enable needs one disable.
pub fn set_secure_input(on: bool) {
    use std::sync::atomic::Ordering;
    if SECURE_INPUT_ACTIVE.swap(on, Ordering::SeqCst) == on {
        return;
    }
    // SAFETY: plain Carbon calls without arguments.
    unsafe {
        if on {
            EnableSecureEventInput();
        } else {
            DisableSecureEventInput();
        }
    }
}

/// The system alert sound.
pub fn beep() {
    // SAFETY: plain AppKit call without arguments.
    unsafe { NSBeep() }
}

/// Caps Lock is on: the hardware state (kCGEventSourceStateHIDSystemState,
/// kCGEventFlagMaskAlphaShift), which inside the app is not tied to how far
/// AppKit's event queue has got.
pub fn caps_lock() -> bool {
    // SAFETY: plain C call without pointers.
    unsafe { CGEventSourceFlagsState(1) & 0x1_0000 != 0 }
}
