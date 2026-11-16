//! Real windows for tests in layers that may not name `platform`.
#![cfg(target_os = "linux")]

use crate::platform::gui::{LogicalSize, Window, WindowSpec};
use crate::platform::linux::native::window::NativeWindow;
use crate::render::window::RenderWindow;

/// A visible, decorated native window (X11 or Wayland, as `NativeWindow`
/// picks) of `width` x `height` logical pixels; `None` when none can be made.
pub fn native_window(title: &'static str, width: f64, height: f64) -> Option<Box<dyn RenderWindow>> {
    let spec = WindowSpec {
        title,
        decorations: true,
        visible: true,
        transparent: false,
        inner_size: LogicalSize::new(width, height),
        position: None,
    };
    let window = Window::from_native_linux(NativeWindow::create(&spec).ok()?);
    Some(Box::new(window))
}
