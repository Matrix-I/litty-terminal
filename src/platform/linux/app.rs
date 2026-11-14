//! App lifecycle twins: no Dock or reopen on Linux; the event sink carries
//! system appearance changes from the settings portal.

use crate::platform::gui::Theme;

/// Where the event loop delivers `SystemAppearance` changes.
pub(crate) static EVENTS: crate::platform::SinkSlot = crate::platform::SinkSlot::new();

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

pub fn set_dock_icon() {}

pub fn activate_app() {}

/// Capture launcher activation before the app starts worker threads.
pub fn capture_startup_activation() { super::native::wayland_activation::capture_startup_activation(); }
