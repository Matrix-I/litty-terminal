//! Startup work that needs no window, started on threads while the platform
//! event loop starts (on macOS, AppKit's launch alone takes ~80 ms of the main
//! thread). Each step fills a process-wide cache while holding its mutex, so the
//! first window either takes the warm result or waits for the step in progress
//! instead of repeating it.

use crate::render::atlas;

/// Start loading the configured font set and, on macOS, creating the GPU
/// device (which compiles the built-in shaders).
pub(super) fn start() {
    // The font settings are process-wide and set here, on the main thread,
    // before the first window sets them again from its own config read; the
    // thread only loads. Read errors are reported by that second read.
    let cfg = crate::config::Config::load_quiet();
    super::window::apply_font_settings(&cfg);
    let families = cfg.font_family;
    spawn("litty-prewarm-font", move || {
        let _ = atlas::load_font(&families);
    });
    // Linux is left out: a Vulkan driver may use Xlib while it starts, and the
    // main thread opens the display at the same time.
    #[cfg(target_os = "macos")]
    spawn("litty-prewarm-gpu", crate::render::renderer::Renderer::prewarm_device);
}

fn spawn(name: &str, work: impl FnOnce() + Send + 'static) {
    // Without the thread the first window does the same work itself, as before.
    if let Err(e) = std::thread::Builder::new().name(name.into()).spawn(work) {
        eprintln!("litty: cannot start {name}: {e}");
    }
}
