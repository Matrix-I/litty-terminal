//! Linux client-side decorations: the tab strip is the title bar, so the window
//! has no native frame. This module holds the pieces a native frame would give
//! for free: resize edges and the window-button actions.

use super::state::State;
use super::window::{refresh_tab_titles, sync_size};
use crate::render::renderer::WindowControl;

use crate::platform::gui::{CursorIcon, ResizeDirection};

/// Width in logical pixels of the invisible resize border.
const BORDER_LOGICAL: f64 = 5.0;

/// Which edge or corner of a `width` x `height` window the point `(x, y)` is
/// on, within `border` pixels of the outline.
pub(super) fn resize_edge(width: f64, height: f64, x: f64, y: f64, border: f64) -> Option<ResizeDirection> {
    if x < 0.0 || y < 0.0 || x >= width || y >= height {
        return None;
    }
    let west = x < border;
    let east = x >= width - border;
    let north = y < border;
    let south = y >= height - border;
    Some(match (north, south, west, east) {
        (true, _, true, _) => ResizeDirection::NorthWest,
        (true, _, _, true) => ResizeDirection::NorthEast,
        (_, true, true, _) => ResizeDirection::SouthWest,
        (_, true, _, true) => ResizeDirection::SouthEast,
        (true, ..) => ResizeDirection::North,
        (_, true, ..) => ResizeDirection::South,
        (_, _, true, _) => ResizeDirection::West,
        (_, _, _, true) => ResizeDirection::East,
        _ => return None,
    })
}

/// Cursor shown over a resize edge.
pub(super) fn resize_cursor(direction: ResizeDirection) -> CursorIcon {
    match direction {
        ResizeDirection::North => CursorIcon::NResize,
        ResizeDirection::South => CursorIcon::SResize,
        ResizeDirection::East => CursorIcon::EResize,
        ResizeDirection::West => CursorIcon::WResize,
        ResizeDirection::NorthEast => CursorIcon::NeResize,
        ResizeDirection::NorthWest => CursorIcon::NwResize,
        ResizeDirection::SouthEast => CursorIcon::SeResize,
        ResizeDirection::SouthWest => CursorIcon::SwResize,
    }
}

/// Resize edge under the pointer, when this window draws its own frame and can be resized.
pub(super) fn edge_at_pointer(s: &State) -> Option<ResizeDirection> {
    if !s.csd || s.window.is_maximized() || s.window.is_fullscreen() {
        return None;
    }
    let size = s.window.inner_size();
    let border = (BORDER_LOGICAL * s.window.scale_factor()).round();
    resize_edge(f64::from(size.width), f64::from(size.height), s.pointer.mouse_pos.0, s.pointer.mouse_pos.1, border)
}

/// Start a resize when the press landed on a window edge. Returns whether it did.
pub(super) fn begin_edge_resize(s: &State) -> bool {
    edge_at_pointer(s).is_some_and(|direction| s.window.drag_resize_window(direction).is_ok())
}

/// Show the resize cursor over an edge; returns whether the pointer is on one.
pub(super) fn update_edge_cursor(s: &mut State) -> bool {
    match edge_at_pointer(s) {
        Some(direction) => {
            s.window.set_cursor(resize_cursor(direction));
            true
        }
        None => false,
    }
}

/// Logical radius of the rounded window corners.
const CORNER_LOGICAL: f64 = 10.0;

/// Round the corners, except where the window fills the screen.
pub(super) fn update_corner_radius(s: &mut State) {
    let full = s.window.is_maximized() || s.window.is_fullscreen();
    let radius = if s.csd && !full { CORNER_LOGICAL * s.window.scale_factor() } else { 0.0 };
    s.renderer.set_corner_radius(radius as f32);
}

/// The window system took the frame over (`server` true) or handed it back,
/// maybe resizing the window as it did: Litty's title bar, rounded corners
/// and resize edges follow, and the terminal is laid out once for the new
/// size and band.
pub(super) fn set_server_decorations(s: &mut State, server: bool) {
    let size = s.window.inner_size();
    if size.width > 0 && size.height > 0 {
        s.renderer.resize(size.width, size.height);
    }
    let csd = !server;
    if s.csd != csd {
        s.csd = csd;
        s.renderer.set_titlebar(csd);
        if std::mem::take(&mut s.pointer.on_edge) {
            s.window.set_cursor(s.pointer.pointer_icon);
        }
        refresh_tab_titles(s);
    }
    update_corner_radius(s);
    sync_size(s);
    s.window.request_redraw();
}

pub(super) fn toggle_maximized(s: &State) {
    s.window.set_maximized(!s.window.is_maximized());
}

/// Run a window button. `Close` is left to the caller, which owns the confirm dialog.
pub(super) fn run_control(s: &State, control: WindowControl) {
    match control {
        WindowControl::Minimize => s.window.set_minimized(true),
        WindowControl::Maximize => toggle_maximized(s),
        WindowControl::Close => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ResizeDirection::*;

    #[test]
    fn edges_and_corners_are_detected() {
        let at = |x, y| resize_edge(800.0, 600.0, x, y, 5.0);
        assert_eq!(at(400.0, 2.0), Some(North));
        assert_eq!(at(400.0, 598.0), Some(South));
        assert_eq!(at(2.0, 300.0), Some(West));
        assert_eq!(at(798.0, 300.0), Some(East));
        assert_eq!(at(1.0, 1.0), Some(NorthWest));
        assert_eq!(at(799.0, 1.0), Some(NorthEast));
        assert_eq!(at(1.0, 599.0), Some(SouthWest));
        assert_eq!(at(799.0, 599.0), Some(SouthEast));
    }

    #[test]
    fn interior_and_outside_are_not_edges() {
        assert_eq!(resize_edge(800.0, 600.0, 400.0, 300.0, 5.0), None);
        assert_eq!(resize_edge(800.0, 600.0, 5.0, 5.0, 5.0), None, "border is exclusive");
        assert_eq!(resize_edge(800.0, 600.0, -1.0, 300.0, 5.0), None);
        assert_eq!(resize_edge(800.0, 600.0, 800.0, 300.0, 5.0), None);
    }

    #[test]
    fn every_direction_has_a_resize_cursor() {
        assert_eq!(resize_cursor(NorthWest), CursorIcon::NwResize);
        assert_eq!(resize_cursor(East), CursorIcon::EResize);
    }
}
