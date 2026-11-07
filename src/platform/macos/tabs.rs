//! Native window tabs.

use crate::platform::gui::Window;

use super::objc::*;

/// Attach `child` as a native tab of `parent`, right after it.
pub fn add_tabbed_window(parent: &Window, child: &Window) {
    set_tabbing_allowed(parent, true);
    set_tabbing_allowed(child, true);
    let (Some((_, p)), Some((_, c))) = (ns_window_of(parent), ns_window_of(child)) else { return };
    // SAFETY: both are live NSWindows; 1 = NSWindowAbove.
    unsafe { msg_send!(p, c"addTabbedWindow:ordered:", c => Id, 1 => isize) };
}

/// Let `window` join native tab groups (`true`), or keep it a separate
/// window even where AppKit would merge it into a tab of another Litty
/// window (`false`). Takes effect the next time the window is ordered in.
pub fn set_tabbing_allowed(window: &Window, allowed: bool) {
    let Some((_, w)) = ns_window_of(window) else { return };
    // NSWindowTabbingMode: 0 = automatic, 2 = disallowed.
    let mode: isize = if allowed { 0 } else { 2 };
    // SAFETY: live NSWindow.
    unsafe { msg_send!(w, c"setTabbingMode:", mode => isize) };
}

/// Move between native tabs of `window`: -1 previous, 1 next, or index
/// `n` (8 = last). Returns `false` when the window has no other tabs.
pub fn select_native_tab(window: &Window, step: isize, index: Option<usize>) -> bool {
    use winit::platform::macos::WindowExtMacOS;
    let window = window.winit();
    let n = window.num_tabs();
    if n < 2 {
        return false;
    }
    match (index, step) {
        (Some(8), _) => window.select_tab_at_index(n - 1),
        (Some(i), _) if i < n => window.select_tab_at_index(i),
        (Some(_), _) => return false,
        (None, s) if s < 0 => window.select_previous_tab(),
        (None, _) => window.select_next_tab(),
    }
    true
}
