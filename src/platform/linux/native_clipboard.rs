//! Wayland clipboard through Litty's own connection (`smithay-clipboard`).
//!
//! The `wl-copy` / `wl-paste` subprocess fallback (platform::clipboard) needs a
//! temporary window on compositors without data-control (GNOME), which flashes
//! in the dock on every copy and paste. On X11 these return `None` / `false`
//! and the subprocess fallback is used.

use parking_lot::Mutex;
use smithay_clipboard::Clipboard;
use std::sync::{Arc, OnceLock, mpsc};
use std::time::Duration;

static CLIPBOARD: OnceLock<Arc<Mutex<Clipboard>>> = OnceLock::new();

/// Attach to the window's `wl_display`. No-op on X11 and after the first call.
pub fn init_clipboard(window: &crate::platform::gui::Window) {
    use crate::platform::gui::RawHandle;
    if CLIPBOARD.get().is_some() {
        return;
    }
    if let Some(RawHandle::Wayland { display, .. }) = window.raw_handle() {
        // SAFETY: the native Wayland backend keeps its connection alive in a
        // process-lifetime static, even after the last window closes.
        let clipboard = unsafe { Clipboard::new_with_dnd(display, super::native::wayland::receive_file_drop) };
        let _ = CLIPBOARD.set(Arc::new(Mutex::new(clipboard)));
    }
}

/// Bounded load: `None` means "unavailable, use the subprocess fallback"; an
/// empty selection is `Some("")` so no helper window flashes for it.
fn load(primary: bool) -> Option<String> {
    let clipboard = CLIPBOARD.get()?.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("litty-wl-clipboard".into())
        .stack_size(128 * 1024)
        .spawn(move || {
            let Some(guard) = clipboard.try_lock() else { return };
            let result = if primary { guard.load_primary() } else { guard.load() };
            let _ = tx.send(result.unwrap_or_default());
        })
        .ok()?;
    rx.recv_timeout(Duration::from_millis(500)).ok()
}

fn store(text: &str, primary: bool) -> bool {
    let Some(clipboard) = CLIPBOARD.get() else { return false };
    let Some(guard) = clipboard.try_lock() else { return false };
    if primary {
        guard.store_primary(text);
    } else {
        guard.store(text);
    }
    true
}

pub fn clipboard_string() -> Option<String> {
    load(false)
}

pub fn clipboard_write_string(s: &str) -> bool {
    store(s, false)
}

pub fn primary_string() -> Option<String> {
    load(true)
}

pub fn primary_write_string(s: &str) -> bool {
    store(s, true)
}
