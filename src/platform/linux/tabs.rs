//! Native window tabs: macOS only.

pub fn add_tabbed_window(_parent: &winit::window::Window, _child: &winit::window::Window) {}

pub fn set_tabbing_allowed(_window: &winit::window::Window, _allowed: bool) {}

pub fn select_native_tab(_window: &winit::window::Window, _step: isize, _index: Option<usize>) -> bool {
    false
}
