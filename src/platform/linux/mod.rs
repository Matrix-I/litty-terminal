//! Linux (X11 / Wayland) side of the platform API. Every public function has a
//! namesake in `platform::macos`; most are explicit no-ops because the feature
//! is macOS-only (Dock, native tabs, menu bar, Secure Keyboard Entry).

mod app;
mod dialog;
mod hotkey;
mod input;
mod menu;
mod native_clipboard;
mod tabs;
mod window;

pub use self::{app::*, dialog::*, hotkey::*, input::*, menu::*, native_clipboard::*, tabs::*, window::*};
