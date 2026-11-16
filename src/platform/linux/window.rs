//! Window look and behavior: macOS only. On Linux the Vulkan swapchain's
//! composite alpha carries `background-opacity`, blur is macOS vibrancy
//! (documented as such), and the swapchain stays sRGB (`window-colorspace` is
//! macOS-only).

pub fn apply_window_config(_window: &crate::platform::gui::Window, _look: &crate::platform::WindowLook) {}

/// Nothing to release: no native window state is attached on Linux.
pub fn cleanup_window(_window: &crate::platform::gui::Window) {}

/// macOS shows the cwd as the title-bar proxy icon; Linux has no equivalent.
/// Called only from macOS-gated code in `app`; kept for API parity.
#[allow(dead_code)]
pub fn set_window_represented_filename(_window: &crate::platform::gui::Window, _path: &str) {}
