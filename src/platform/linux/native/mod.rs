//! Native Linux windowing and unified epoll event loop (Milestone 3 / Phase 4.2).

pub mod app;
pub mod events;
pub mod window;

#[allow(unused_imports)]
pub use app::run_native;
#[allow(unused_imports)]
pub use window::NativeWindow;
