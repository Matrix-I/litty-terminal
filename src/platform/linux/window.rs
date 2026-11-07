//! Window look and behavior: macOS only.

pub fn configure_layer_drawables(_window: &crate::platform::gui::Window) {}

pub fn apply_window_config(_window: &crate::platform::gui::Window, _look: &crate::platform::WindowLook) {}

pub fn cleanup_window(_window: &crate::platform::gui::Window) {}

/// macOS shows the cwd as the title-bar proxy icon; Linux has no equivalent.
/// Called only from macOS-gated code in `app`; kept for API parity.
#[allow(dead_code)]
pub fn set_window_represented_filename(_window: &crate::platform::gui::Window, _path: &str) {}
