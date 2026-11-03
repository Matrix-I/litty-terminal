//! Quick Terminal global hotkey through Carbon EventHotKey.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

use super::app::EVENTS;

#[repr(C)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

#[repr(C)]
struct EventHotKeyId {
    signature: u32,
    id: u32,
}

type EventHandler = extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetApplicationEventTarget() -> *mut c_void;
    fn InstallEventHandler(
        target: *mut c_void,
        handler: EventHandler,
        count: u32,
        types: *const EventTypeSpec,
        user_data: *mut c_void,
        out: *mut *mut c_void,
    ) -> i32;
    fn RegisterEventHotKey(
        code: u32,
        modifiers: u32,
        id: EventHotKeyId,
        target: *mut c_void,
        options: u32,
        out: *mut *mut c_void,
    ) -> i32;
    fn UnregisterEventHotKey(hotkey: *mut c_void) -> i32;
}

/// The registered Quick Terminal hotkey (`EventHotKeyRef`).
static HOTKEY: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

static HOTKEY_HANDLER: AtomicBool = AtomicBool::new(false);

/// Register (or, with `None`, remove) the system-wide Quick Terminal hotkey.
/// Needs [`register_reopen_handler`] first, which provides the event sink.
pub fn set_global_hotkey(hotkey: Option<crate::platform::Hotkey>) {
    extern "C" fn on_hotkey(_call: *mut c_void, _event: *mut c_void, _data: *mut c_void) -> i32 {
        EVENTS.emit(crate::platform::PlatformEvent::ToggleQuickTerminal);
        0
    }
    // SAFETY: Carbon event APIs on the main thread; the handler and the
    // hotkey stay registered until replaced here.
    unsafe {
        let old = HOTKEY.swap(std::ptr::null_mut(), Ordering::Relaxed);
        if !old.is_null() {
            UnregisterEventHotKey(old);
        }
        let Some(t) = hotkey else { return };
        let code = t.code;
        if !HOTKEY_HANDLER.swap(true, Ordering::Relaxed) {
            // kEventClassKeyboard / kEventHotKeyPressed
            let spec = EventTypeSpec { event_class: u32::from_be_bytes(*b"keyb"), event_kind: 5 };
            let target = GetApplicationEventTarget();
            InstallEventHandler(target, on_hotkey, 1, &spec, std::ptr::null_mut(), std::ptr::null_mut());
        }
        // cmdKey, shiftKey, optionKey, controlKey
        let m = t.mods;
        let mods = 0x100 * m.sup as u32 + 0x200 * m.shift as u32 + 0x800 * m.alt as u32 + 0x1000 * m.ctrl as u32;
        let id = EventHotKeyId { signature: u32::from_be_bytes(*b"LTTY"), id: 1 };
        let mut hotkey = std::ptr::null_mut();
        if RegisterEventHotKey(code, mods, id, GetApplicationEventTarget(), 0, &mut hotkey) == 0 {
            HOTKEY.store(hotkey, Ordering::Relaxed);
        } else {
            eprintln!("litty: the Quick Terminal hotkey is taken by another application");
        }
    }
}

/// The control socket is Linux-only: on macOS the Carbon hotkey and the Dock
/// cover the Quick Terminal, and `litty +toggle-quick-terminal` exits early.
pub fn listen_for_toggle(_sink: crate::platform::EventSink) {}
