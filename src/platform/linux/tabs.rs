//! Native window tabs: macOS only. Linux has no window-server tab groups;
//! Litty's own tab bar handles tabs, so these leave every window standalone.

pub fn add_tabbed_window(_parent: &crate::platform::gui::Window, _child: &crate::platform::gui::Window) {}

pub fn set_tabbing_allowed(_window: &crate::platform::gui::Window, _allowed: bool) {}

/// `false`: no native tab took the request, so the caller uses Litty's tab bar.
pub fn select_native_tab(_window: &crate::platform::gui::Window, _step: isize, _index: Option<usize>) -> bool {
    false
}
