//! macOS-specific window vibrancy, blur, color space, Dock and app lifecycle
//! integration. Talks to the Objective-C runtime directly (no third-party crates).
//! Every function has a no-op twin on other platforms so callers need no `cfg`.

#[cfg(target_os = "macos")]
mod app;
#[cfg(target_os = "macos")]
mod dialog;
#[cfg(target_os = "macos")]
mod hotkey;
#[cfg(target_os = "macos")]
mod input;
#[cfg(target_os = "macos")]
mod menu;
#[cfg(target_os = "macos")]
mod native_clipboard;
#[cfg(target_os = "macos")]
mod objc;
#[cfg(target_os = "macos")]
mod tabs;
#[cfg(target_os = "macos")]
mod window;

#[cfg(target_os = "macos")]
pub use self::{app::*, dialog::*, hotkey::*, input::*, menu::*, native_clipboard::*, tabs::*, window::*};

#[cfg(not(target_os = "macos"))]
pub use stub::*;

#[cfg(not(target_os = "macos"))]
#[allow(dead_code)]
mod stub {
    pub fn release_free_memory() {}
    pub fn configure_layer_drawables(_window: &winit::window::Window) {}
    pub fn apply_macos_window_config(_window: &winit::window::Window, _cfg: &crate::config::Config) {}
    pub fn cleanup_window(_window: &winit::window::Window) {}
    pub fn register_reopen_handler(_proxy: winit::event_loop::EventLoopProxy<crate::AppEvent>) {}
    pub fn install_menu() {}
    pub fn set_global_hotkey(_trigger: Option<&crate::keymap::Trigger>) {}
    pub fn hide_app() {}
    pub fn add_tabbed_window(_parent: &winit::window::Window, _child: &winit::window::Window) {}
    pub fn set_tabbing_allowed(_window: &winit::window::Window, _allowed: bool) {}
    pub fn select_native_tab(_window: &winit::window::Window, _step: isize, _index: Option<usize>) -> bool {
        false
    }
    pub fn caps_lock() -> bool {
        false
    }
    pub fn system_dark() -> bool {
        true
    }
    pub fn set_secure_input_menu_state(_on: bool) {}
    pub fn set_dock_icon() {}
    pub fn activate_app() {}
    pub fn set_window_represented_filename(_window: &winit::window::Window, _path: &str) {}
    pub fn set_secure_input(_on: bool) {}
    pub fn beep() {}
    pub fn show_progress_dialog(_title: &str, _text: &str) {}
    pub fn close_progress_dialog() {}
    pub fn alert_choice(title: &str, text: &str, buttons: &[&str]) -> usize {
        let zenity_bin = if std::path::Path::new("/usr/bin/zenity").exists() { "/usr/bin/zenity" } else { "zenity" };
        let mut cmd = std::process::Command::new(zenity_bin);
        cmd.args(["--question", "--title", title, "--text", text]);
        if let Some(first) = buttons.first() {
            cmd.args(["--ok-label", first]);
        }
        if let Some(second) = buttons.get(1) {
            cmd.args(["--cancel-label", second]);
        }
        match cmd.status() {
            Ok(status) if status.success() => 0,
            _ => 1,
        }
    }
    pub fn confirm(title: &str, text: &str, button: &str) -> bool {
        alert_choice(title, text, &[button, "Cancel"]) == 0
    }
    pub fn alert(title: &str, text: &str) {
        let zenity_bin = if std::path::Path::new("/usr/bin/zenity").exists() { "/usr/bin/zenity" } else { "zenity" };
        let _ = std::process::Command::new(zenity_bin)
            .args(["--info", "--title", title, "--text", text])
            .status();
    }
    pub fn clipboard_string() -> Option<String> {
        None
    }
    pub fn clipboard_write_string(_s: &str) -> bool {
        false
    }
}
