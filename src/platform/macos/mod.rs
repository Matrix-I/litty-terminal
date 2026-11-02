//! macOS-specific window vibrancy, blur, color space, Dock and app lifecycle
//! integration. Talks to the Objective-C runtime directly (no third-party crates).
//! The Linux twins of every public function live in `platform::linux`.

mod app;
mod dialog;
mod hotkey;
mod input;
mod menu;
mod native_clipboard;
mod objc;
mod tabs;
mod window;

pub use self::{app::*, dialog::*, hotkey::*, input::*, menu::*, native_clipboard::*, tabs::*, window::*};
