//! OS integration that does not depend on window or terminal state:
//! detached child processes, notifications, URL / file opening, clipboard.

pub(crate) mod clipboard;
pub(crate) mod gui;
pub(crate) mod input;
pub(crate) mod process;
mod types;

#[cfg(target_os = "macos")]
pub(crate) mod macos;
#[cfg(target_os = "macos")]
use macos as imp;

#[cfg(not(target_os = "macos"))]
pub(crate) mod linux;
#[cfg(not(target_os = "macos"))]
use linux as imp;

// The cross-platform API, listed by name (never `imp::*`): a function missing
// on one platform fails that platform's build instead of silently vanishing.
pub(crate) use imp::{
    activate_app, add_tabbed_window, alert, alert_choice, apply_window_config, beep, caps_lock,
    cleanup_window, clipboard_string, clipboard_write_string, init_clipboard, primary_string, primary_write_string, close_progress_dialog,
    configure_layer_drawables, confirm, install_menu,
    register_reopen_handler, release_free_memory, select_native_tab, set_dock_icon,
    set_secure_input, set_secure_input_menu_state, set_tabbing_allowed,
    show_progress_dialog, system_dark,
};
// Only called from macOS-gated code (see the cfg inventory in
// docs/plans/2026-11-01-layered-layout.md); the Linux twin keeps the API symmetric.
#[cfg_attr(not(target_os = "macos"), allow(unused_imports))]
pub(crate) use imp::set_window_represented_filename;
pub(crate) use types::{ColorSpace, EventSink, MenuCommand, PlatformEvent, WindowLook};
// Only the macOS backend delivers events from OS callbacks through a slot.
#[cfg_attr(not(target_os = "macos"), allow(unused_imports))]
pub(crate) use types::SinkSlot;
