//! Window look and behavior: macOS only.

pub fn configure_layer_drawables(_window: &winit::window::Window) {}

pub fn apply_window_config(_window: &winit::window::Window, _cfg: &crate::config::Config) {}

pub fn cleanup_window(_window: &winit::window::Window) {}

/// macOS shows the cwd as the title-bar proxy icon; Linux has no equivalent.
/// Called only from macOS-gated code in `app`; kept for API parity.
#[allow(dead_code)]
pub fn set_window_represented_filename(_window: &winit::window::Window, _path: &str) {}
