//! App lifecycle twins: no Dock or reopen on Linux; the event sink carries
//! system appearance changes from the settings portal.

use crate::platform::gui::Theme;

/// Where the event loop delivers `SystemAppearance` changes.
pub(crate) static EVENTS: crate::platform::SinkSlot = crate::platform::SinkSlot::new();

/// Not wired on Linux yet. mimalloc is the global allocator here too, so
/// `mi_collect(true)` as on macOS could return freed pages after tabs close;
/// tracked in `docs/plans/0.14.0.md` pending an RSS measurement.
pub fn release_free_memory() {}

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

/// Capture launcher activation before the app starts worker threads.
pub fn capture_startup_activation() { super::native::wayland_activation::capture_startup_activation(); }
