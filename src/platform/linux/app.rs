//! App lifecycle twins: all no-ops except `system_dark`.

pub fn release_free_memory() {}

pub fn register_reopen_handler(_proxy: winit::event_loop::EventLoopProxy<crate::AppEvent>) {}

pub fn hide_app() {}

pub fn system_dark() -> bool {
    true
}

pub fn set_dock_icon() {}

pub fn activate_app() {}
