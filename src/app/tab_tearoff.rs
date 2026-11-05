//! Dragging a tab out of the tab bar: dropped far enough away, it moves to a
//! window of its own.

use winit::dpi::{LogicalPosition, LogicalSize};
use winit::window::WindowId;

use super::state::State;
use super::window::{refresh_tab_titles, sync_size};
use crate::{ai, Tab};

/// How far the pointer must be from the tab bar, in cell heights, to tear a tab off.
const TEAR_OFF_CELLS: f64 = 2.0;
/// Distance from the new window's top edge to the pointer, in logical pixels.
const GRAB_FROM_TOP_LOGICAL: f64 = 14.0;

/// A tab on its way from one window to a new one, with its per-pane window state.
pub(super) struct TornTab {
    pub(super) tab: Tab,
    pub(super) ai: Vec<(u64, ai::PaneState)>,
    pub(super) from: WindowId,
    /// Where the tab was in that window, for putting it back.
    pub(super) index: usize,
    /// Inner size of the window the tab came from, so its grids keep their size
    /// (in logical pixels, so it also holds on a display with another scale).
    pub(super) size: LogicalSize<f64>,
    /// Font size of that window, which may have been zoomed.
    pub(super) font_pt: f32,
    /// Where to put the new window; `None` where the platform gives no window positions.
    pub(super) position: Option<LogicalPosition<f64>>,
}

/// Whether a pointer at `pointer` (window pixels, may be outside the window) has left
/// the band of `bar_height` at the top of a `window_width` wide window by more than `margin`.
fn is_torn_off(pointer: (f64, f64), window_width: f64, bar_height: f64, margin: f64) -> bool {
    let (x, y) = pointer;
    y < -margin || y > bar_height + margin || x < -margin || x > window_width + margin
}

/// Top-left corner for a window `width` pixels wide so the pointer lands on its
/// top edge, in the middle. Inputs are physical pixels at `scale`; the result is in
/// logical pixels so it stays right when the window lands on a display with another
/// scale (macOS positions windows in points).
fn new_window_origin(pointer_screen: (i32, i32), width: u32, scale: f64) -> LogicalPosition<f64> {
    let x = (f64::from(pointer_screen.0) - f64::from(width) / 2.0) / scale;
    let y = f64::from(pointer_screen.1) / scale - GRAB_FROM_TOP_LOGICAL;
    LogicalPosition::new(x, y)
}

/// Index of the held tab. The animation's index alone can go stale when tabs are
/// closed while the button is down, so it must agree with the tab found by id.
fn held_tab_index(tab_ids: impl Iterator<Item = u64>, held_id: u64, animated: Option<usize>) -> Option<usize> {
    let index = tab_ids.into_iter().position(|id| id == held_id)?;
    (animated == Some(index)).then_some(index)
}

/// Pointer position on the screen; `None` where the platform gives no window positions (Wayland).
fn pointer_on_screen(s: &State) -> Option<(i32, i32)> {
    let origin = s.window.inner_position().ok()?;
    let pointer = s.pointer.mouse_pos;
    Some((origin.x + pointer.0.round() as i32, origin.y + pointer.1.round() as i32))
}

/// The pointer moved while a tab is held. Once it is far from the tab bar the tab
/// leaves for a window of its own, which then follows the pointer until the button
/// is released. Returns whether the tab left; it does not where windows cannot be
/// positioned, and the tab is torn off on release instead.
pub(super) fn tear_off_while_dragging(s: &mut State) -> bool {
    if pointer_on_screen(s).is_none() || !detach_held_tab(s) {
        return false;
    }
    if let Some(drag) = s.pointer.tab_drag.as_mut() {
        drag.mark_detached();
    }
    true
}

/// The pointer moved after the tab left: ask `App` to move its new window under the pointer.
pub(super) fn follow_torn_window(s: &mut State) {
    let Some(target) = s.pointer.tab_drag.as_ref().and_then(|drag| drag.follower()) else { return };
    let Some(screen) = pointer_on_screen(s) else { return };
    let origin = new_window_origin(screen, s.window.inner_size().width, s.window.scale_factor());
    s.requests.move_torn = Some((target, origin));
}

/// The mouse button of a tab drag was released. If the pointer is far from the
/// tab bar and the tab has not left yet, it leaves now; `App` opens it in a new window.
/// Returns whether that happened.
pub(super) fn tear_off_if_dragged_out(s: &mut State) -> bool {
    detach_held_tab(s)
}

/// Take the held tab out of this window if the pointer is far enough from the tab bar,
/// and queue it for a new window.
fn detach_held_tab(s: &mut State) -> bool {
    if s.quick || s.tabs.len() < 2 {
        return false;
    }
    let Some(held_id) = s.pointer.tab_drag.as_ref().map(|drag| drag.id()) else { return false };
    let Some(index) = held_tab_index(s.tabs.iter().map(|t| t.id), held_id, s.tab_anim.dragged()) else { return false };
    let pointer = s.pointer.mouse_pos;
    let size = s.window.inner_size();
    let scale = s.window.scale_factor();
    let cell_h = f64::from(s.renderer.cell_size().1);
    if !is_torn_off(pointer, f64::from(size.width), s.renderer.tab_bar_height(), TEAR_OFF_CELLS * cell_h) {
        return false;
    }
    let position = pointer_on_screen(s).map(|screen| new_window_origin(screen, size.width, scale));

    let tab = s.tabs.remove(index);
    // The animation's index would otherwise point at the neighbour that slid into the gap.
    s.tab_anim.end_drag();
    let mut ai = Vec::new();
    for pane in &tab.panes {
        ai.extend(s.ai.remove(&pane.id).map(|state| (pane.id, state)));
        s.invalidate_pane_state(pane.id);
    }
    if s.active >= s.tabs.len() {
        s.active = s.tabs.len() - 1;
    } else if index < s.active {
        s.active -= 1;
    }
    sync_size(s);
    refresh_tab_titles(s);
    s.renderer.invalidate();
    s.window.request_redraw();
    s.requests.tear_off = Some(TornTab {
        tab,
        ai,
        from: s.window.id(),
        index,
        size: size.to_logical(scale),
        font_pt: s.font_pt,
        position,
    });
    true
}

/// Put a torn-off tab back where it was in `s` when its new window could not be opened.
pub(super) fn restore_tab(s: &mut State, torn: TornTab) {
    for (id, state) in torn.ai {
        s.ai.insert(id, state);
    }
    let at = torn.index.min(s.tabs.len());
    s.tabs.insert(at, torn.tab);
    s.active = at;
    sync_size(s);
    refresh_tab_titles(s);
    s.renderer.invalidate();
    s.window.request_redraw();
}

#[cfg(test)]
mod tests {
    use super::{held_tab_index, is_torn_off, new_window_origin};

    #[test]
    fn the_held_tab_is_found_by_id_and_must_match_the_animation() {
        let ids = [10, 20, 30];
        assert_eq!(held_tab_index(ids.into_iter(), 20, Some(1)), Some(1));
        // Tab 10 was closed while 20 was held: the animation still says index 1,
        // which is now tab 30, so nothing may be torn off.
        assert_eq!(held_tab_index([20, 30].into_iter(), 20, Some(1)), None);
        assert_eq!(held_tab_index([30].into_iter(), 20, Some(0)), None, "the held tab is gone");
        assert_eq!(held_tab_index(ids.into_iter(), 20, None), None, "no drag was started");
    }

    #[test]
    fn staying_near_the_tab_bar_keeps_the_tab() {
        let (width, bar, margin) = (960.0, 24.0, 40.0);
        for pointer in [(10.0, 10.0), (500.0, 24.0), (500.0, 24.0 + 40.0), (500.0, -40.0), (-40.0, 10.0), (width + 40.0, 10.0)] {
            assert!(!is_torn_off(pointer, width, bar, margin), "{pointer:?}");
        }
    }

    #[test]
    fn leaving_the_band_on_any_side_tears_the_tab_off() {
        let (width, bar, margin) = (960.0, 24.0, 40.0);
        for pointer in [(500.0, 24.0 + 40.5), (500.0, 600.0), (500.0, -41.0), (-41.0, 10.0), (width + 41.0, 10.0), (-300.0, -300.0)] {
            assert!(is_torn_off(pointer, width, bar, margin), "{pointer:?}");
        }
    }

    #[test]
    fn new_window_is_centred_on_the_pointer_just_below_its_top_edge() {
        let at = new_window_origin((1000, 500), 960, 1.0);
        assert_eq!((at.x, at.y), (520.0, 486.0));
        let at = new_window_origin((1000, 500), 1920, 2.0);
        assert_eq!((at.x, at.y), (20.0, 236.0), "logical pixels: physical input divided by the scale");
        let at = new_window_origin((100, 5), 960, 1.0);
        assert_eq!((at.x, at.y), (-380.0, -9.0), "positions may be off-screen; the window manager adjusts them");
    }
}
