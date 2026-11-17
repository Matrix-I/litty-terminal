//! App lifecycle twins: no Dock or reopen on Linux; the event sink carries
//! system appearance changes from the settings portal.

use crate::platform::gui::Theme;

/// Where the event loop delivers `SystemAppearance` changes.
pub(crate) static EVENTS: crate::platform::SinkSlot = crate::platform::SinkSlot::new();

/// Return freed heap pages to the OS (keeps idle RSS low after closing tabs).
/// Linux uses the system allocator (mimalloc is macOS-only), so glibc keeps
/// freed pages mapped until `malloc_trim` hands them back.
pub fn release_free_memory() {
    trim_heap();
}

/// Whether glibc returned any freed memory to the OS.
#[cfg(target_env = "gnu")]
fn trim_heap() -> bool {
    // SAFETY: `malloc_trim` only inspects and shrinks glibc's own arenas.
    unsafe { libc::malloc_trim(0) == 1 }
}

/// Other C libraries have no `malloc_trim`; their allocator decides alone.
#[cfg(not(target_env = "gnu"))]
fn trim_heap() -> bool {
    false
}

/// Linux has no Dock reopen; the sink still receives appearance changes.
pub fn register_reopen_handler(sink: crate::platform::EventSink) {
    EVENTS.set(sink);
}

/// Dark unless the desktop prefers light (no settings portal keeps the
/// dark default).
pub fn system_dark() -> bool {
    super::native::portal::system_theme() != Some(Theme::Light)
}

/// No Dock: the window icon comes from the desktop entry.
pub fn set_dock_icon() {}

/// No app object to activate: X11 and Wayland raise individual windows
/// (`xdg_activation` on Wayland), never the whole application.
pub fn activate_app() {}

/// Capture launcher activation and the start time that names this
/// instance's activation socket, before the app starts worker threads.
pub fn capture_startup_activation() {
    super::activation::capture_start_time();
    super::native::wayland_activation::capture_startup_activation();
}

#[cfg(all(test, target_env = "gnu"))]
mod tests {
    use super::trim_heap;

    #[test]
    fn trimming_returns_freed_heap_pages() {
        // Small blocks stay on the heap (below glibc's mmap threshold), so
        // freeing them leaves whole pages that only a trim gives back.
        let blocks: Vec<Vec<u8>> = (0..4096).map(|i| vec![i as u8; 16 * 1024]).collect();
        drop(blocks);
        assert!(trim_heap(), "64 MB of freed heap blocks should release pages");
    }
}
