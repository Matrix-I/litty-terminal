//! Native window tabs: macOS only.

pub fn add_tabbed_window(_parent: &crate::platform::gui::Window, _child: &crate::platform::gui::Window) {}

pub fn set_tabbing_allowed(_window: &crate::platform::gui::Window, _allowed: bool) {}

pub fn select_native_tab(_window: &crate::platform::gui::Window, _step: isize, _index: Option<usize>) -> bool {
    false
}
