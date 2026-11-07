//! The window API the rest of the crate programs against. Everything outside
//! this module sees only these types; the `winit` window behind them is the
//! single thing the native AppKit / Wayland / X11 backends replace.

use std::ffi::c_void;

use winit::dpi as wdpi;
use winit::window as ww;

/// Stable identity of a window for the lifetime of the process.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct WindowId(u64);

impl From<ww::WindowId> for WindowId {
    fn from(id: ww::WindowId) -> Self {
        WindowId(id.into())
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct PhysicalPosition<T> {
    pub x: T,
    pub y: T,
}

impl<T> PhysicalPosition<T> {
    pub const fn new(x: T, y: T) -> Self {
        Self { x, y }
    }
}

impl PhysicalPosition<i32> {
    pub fn to_logical(self, scale: f64) -> LogicalPosition<f64> {
        LogicalPosition::new(self.x as f64 / scale, self.y as f64 / scale)
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct PhysicalSize<T> {
    pub width: T,
    pub height: T,
}

impl<T> PhysicalSize<T> {
    pub const fn new(width: T, height: T) -> Self {
        Self { width, height }
    }
}

impl PhysicalSize<u32> {
    pub fn to_logical(self, scale: f64) -> LogicalSize<f64> {
        LogicalSize::new(self.width as f64 / scale, self.height as f64 / scale)
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct LogicalPosition<T> {
    pub x: T,
    pub y: T,
}

impl<T> LogicalPosition<T> {
    pub const fn new(x: T, y: T) -> Self {
        Self { x, y }
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct LogicalSize<T> {
    pub width: T,
    pub height: T,
}

impl<T> LogicalSize<T> {
    pub const fn new(width: T, height: T) -> Self {
        Self { width, height }
    }
}

impl From<LogicalSize<f64>> for wdpi::LogicalSize<f64> {
    fn from(s: LogicalSize<f64>) -> Self {
        wdpi::LogicalSize::new(s.width, s.height)
    }
}

impl From<LogicalPosition<f64>> for wdpi::LogicalPosition<f64> {
    fn from(p: LogicalPosition<f64>) -> Self {
        wdpi::LogicalPosition::new(p.x, p.y)
    }
}

impl From<wdpi::PhysicalPosition<f64>> for PhysicalPosition<f64> {
    fn from(p: wdpi::PhysicalPosition<f64>) -> Self {
        PhysicalPosition::new(p.x, p.y)
    }
}

/// Mouse pointer shapes, named after the CSS `cursor` values.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum CursorIcon {
    #[default]
    Default,
    ContextMenu,
    Help,
    Pointer,
    Progress,
    Wait,
    Cell,
    Crosshair,
    Text,
    VerticalText,
    Alias,
    Copy,
    Move,
    NoDrop,
    NotAllowed,
    Grab,
    Grabbing,
    EResize,
    NResize,
    NeResize,
    NwResize,
    SResize,
    SeResize,
    SwResize,
    WResize,
    EwResize,
    NsResize,
    NeswResize,
    NwseResize,
    ColResize,
    RowResize,
    AllScroll,
    ZoomIn,
    ZoomOut,
}

impl std::str::FromStr for CursorIcon {
    type Err = ();

    /// Parse a CSS cursor name.
    fn from_str(name: &str) -> Result<Self, ()> {
        Ok(match name {
            "default" => CursorIcon::Default,
            "context-menu" => CursorIcon::ContextMenu,
            "help" => CursorIcon::Help,
            "pointer" => CursorIcon::Pointer,
            "progress" => CursorIcon::Progress,
            "wait" => CursorIcon::Wait,
            "cell" => CursorIcon::Cell,
            "crosshair" => CursorIcon::Crosshair,
            "text" => CursorIcon::Text,
            "vertical-text" => CursorIcon::VerticalText,
            "alias" => CursorIcon::Alias,
            "copy" => CursorIcon::Copy,
            "move" => CursorIcon::Move,
            "no-drop" => CursorIcon::NoDrop,
            "not-allowed" => CursorIcon::NotAllowed,
            "grab" => CursorIcon::Grab,
            "grabbing" => CursorIcon::Grabbing,
            "e-resize" => CursorIcon::EResize,
            "n-resize" => CursorIcon::NResize,
            "ne-resize" => CursorIcon::NeResize,
            "nw-resize" => CursorIcon::NwResize,
            "s-resize" => CursorIcon::SResize,
            "se-resize" => CursorIcon::SeResize,
            "sw-resize" => CursorIcon::SwResize,
            "w-resize" => CursorIcon::WResize,
            "ew-resize" => CursorIcon::EwResize,
            "ns-resize" => CursorIcon::NsResize,
            "nesw-resize" => CursorIcon::NeswResize,
            "nwse-resize" => CursorIcon::NwseResize,
            "col-resize" => CursorIcon::ColResize,
            "row-resize" => CursorIcon::RowResize,
            "all-scroll" => CursorIcon::AllScroll,
            "zoom-in" => CursorIcon::ZoomIn,
            "zoom-out" => CursorIcon::ZoomOut,
            _ => return Err(()),
        })
    }
}

impl From<CursorIcon> for ww::CursorIcon {
    fn from(icon: CursorIcon) -> Self {
        match icon {
            CursorIcon::Default => ww::CursorIcon::Default,
            CursorIcon::ContextMenu => ww::CursorIcon::ContextMenu,
            CursorIcon::Help => ww::CursorIcon::Help,
            CursorIcon::Pointer => ww::CursorIcon::Pointer,
            CursorIcon::Progress => ww::CursorIcon::Progress,
            CursorIcon::Wait => ww::CursorIcon::Wait,
            CursorIcon::Cell => ww::CursorIcon::Cell,
            CursorIcon::Crosshair => ww::CursorIcon::Crosshair,
            CursorIcon::Text => ww::CursorIcon::Text,
            CursorIcon::VerticalText => ww::CursorIcon::VerticalText,
            CursorIcon::Alias => ww::CursorIcon::Alias,
            CursorIcon::Copy => ww::CursorIcon::Copy,
            CursorIcon::Move => ww::CursorIcon::Move,
            CursorIcon::NoDrop => ww::CursorIcon::NoDrop,
            CursorIcon::NotAllowed => ww::CursorIcon::NotAllowed,
            CursorIcon::Grab => ww::CursorIcon::Grab,
            CursorIcon::Grabbing => ww::CursorIcon::Grabbing,
            CursorIcon::EResize => ww::CursorIcon::EResize,
            CursorIcon::NResize => ww::CursorIcon::NResize,
            CursorIcon::NeResize => ww::CursorIcon::NeResize,
            CursorIcon::NwResize => ww::CursorIcon::NwResize,
            CursorIcon::SResize => ww::CursorIcon::SResize,
            CursorIcon::SeResize => ww::CursorIcon::SeResize,
            CursorIcon::SwResize => ww::CursorIcon::SwResize,
            CursorIcon::WResize => ww::CursorIcon::WResize,
            CursorIcon::EwResize => ww::CursorIcon::EwResize,
            CursorIcon::NsResize => ww::CursorIcon::NsResize,
            CursorIcon::NeswResize => ww::CursorIcon::NeswResize,
            CursorIcon::NwseResize => ww::CursorIcon::NwseResize,
            CursorIcon::ColResize => ww::CursorIcon::ColResize,
            CursorIcon::RowResize => ww::CursorIcon::RowResize,
            CursorIcon::AllScroll => ww::CursorIcon::AllScroll,
            CursorIcon::ZoomIn => ww::CursorIcon::ZoomIn,
            CursorIcon::ZoomOut => ww::CursorIcon::ZoomOut,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ResizeDirection {
    East,
    North,
    NorthEast,
    NorthWest,
    South,
    SouthEast,
    SouthWest,
    West,
}

impl From<ResizeDirection> for ww::ResizeDirection {
    fn from(d: ResizeDirection) -> Self {
        match d {
            ResizeDirection::East => ww::ResizeDirection::East,
            ResizeDirection::North => ww::ResizeDirection::North,
            ResizeDirection::NorthEast => ww::ResizeDirection::NorthEast,
            ResizeDirection::NorthWest => ww::ResizeDirection::NorthWest,
            ResizeDirection::South => ww::ResizeDirection::South,
            ResizeDirection::SouthEast => ww::ResizeDirection::SouthEast,
            ResizeDirection::SouthWest => ww::ResizeDirection::SouthWest,
            ResizeDirection::West => ww::ResizeDirection::West,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}

impl From<ww::Theme> for Theme {
    fn from(t: ww::Theme) -> Self {
        match t {
            ww::Theme::Light => Theme::Light,
            ww::Theme::Dark => Theme::Dark,
        }
    }
}

/// The platform cannot do what was asked (e.g. window positions on Wayland).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Unsupported;

/// Geometry of the monitor a window is on.
pub struct Monitor {
    pub position: PhysicalPosition<i32>,
    pub size: PhysicalSize<u32>,
    pub refresh_rate_millihertz: Option<u32>,
}

/// Native handles a GPU backend creates its surface from. Each variant is
/// read by one platform's backend only.
pub enum RawHandle {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    AppKit { ns_view: *mut c_void },
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Wayland { display: *mut c_void, surface: *mut c_void },
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Xlib { display: *mut c_void, window: std::ffi::c_ulong },
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Xcb { connection: *mut c_void, window: u32 },
}

pub struct Window(ww::Window);

impl Window {
    pub fn from_winit(window: ww::Window) -> Self {
        Window(window)
    }

    /// Escape hatch for backend-specific code that has no equivalent yet (macOS native tabs).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(crate) fn winit(&self) -> &ww::Window {
        &self.0
    }

    pub fn id(&self) -> WindowId {
        self.0.id().into()
    }

    pub fn request_redraw(&self) {
        self.0.request_redraw();
    }

    pub fn scale_factor(&self) -> f64 {
        self.0.scale_factor()
    }

    pub fn inner_size(&self) -> PhysicalSize<u32> {
        let s = self.0.inner_size();
        PhysicalSize::new(s.width, s.height)
    }

    pub fn request_inner_size(&self, size: PhysicalSize<u32>) {
        let _ = self.0.request_inner_size(wdpi::PhysicalSize::new(size.width, size.height));
    }

    pub fn inner_position(&self) -> Result<PhysicalPosition<i32>, Unsupported> {
        let p = self.0.inner_position().map_err(|_| Unsupported)?;
        Ok(PhysicalPosition::new(p.x, p.y))
    }

    pub fn outer_position(&self) -> Result<PhysicalPosition<i32>, Unsupported> {
        let p = self.0.outer_position().map_err(|_| Unsupported)?;
        Ok(PhysicalPosition::new(p.x, p.y))
    }

    pub fn set_outer_position(&self, position: PhysicalPosition<i32>) {
        self.0.set_outer_position(wdpi::PhysicalPosition::new(position.x, position.y));
    }

    pub fn set_outer_position_logical(&self, position: LogicalPosition<f64>) {
        self.0.set_outer_position(wdpi::LogicalPosition::new(position.x, position.y));
    }

    pub fn set_visible(&self, visible: bool) {
        self.0.set_visible(visible);
    }

    pub fn is_visible(&self) -> Option<bool> {
        self.0.is_visible()
    }

    pub fn is_minimized(&self) -> Option<bool> {
        self.0.is_minimized()
    }

    pub fn set_minimized(&self, minimized: bool) {
        self.0.set_minimized(minimized);
    }

    pub fn is_maximized(&self) -> bool {
        self.0.is_maximized()
    }

    pub fn set_maximized(&self, maximized: bool) {
        self.0.set_maximized(maximized);
    }

    pub fn is_fullscreen(&self) -> bool {
        self.0.fullscreen().is_some()
    }

    pub fn set_title(&self, title: &str) {
        self.0.set_title(title);
    }

    pub fn focus_window(&self) {
        self.0.focus_window();
    }

    pub fn set_cursor(&self, icon: CursorIcon) {
        self.0.set_cursor(ww::CursorIcon::from(icon));
    }

    pub fn set_cursor_visible(&self, visible: bool) {
        self.0.set_cursor_visible(visible);
    }

    pub fn theme(&self) -> Option<Theme> {
        self.0.theme().map(Theme::from)
    }

    pub fn set_ime_allowed(&self, allowed: bool) {
        self.0.set_ime_allowed(allowed);
    }

    pub fn set_ime_cursor_area(&self, position: PhysicalPosition<f64>, size: PhysicalSize<f64>) {
        self.0.set_ime_cursor_area(
            wdpi::PhysicalPosition::new(position.x, position.y),
            wdpi::PhysicalSize::new(size.width, size.height),
        );
    }

    pub fn request_attention(&self, requested: bool) {
        self.0.request_user_attention(requested.then_some(ww::UserAttentionType::Informational));
    }

    pub fn drag_window(&self) -> Result<(), Unsupported> {
        self.0.drag_window().map_err(|_| Unsupported)
    }

    pub fn drag_resize_window(&self, direction: ResizeDirection) -> Result<(), Unsupported> {
        self.0.drag_resize_window(direction.into()).map_err(|_| Unsupported)
    }

    pub fn current_monitor(&self) -> Option<Monitor> {
        let m = self.0.current_monitor()?;
        let (p, s) = (m.position(), m.size());
        Some(Monitor {
            position: PhysicalPosition::new(p.x, p.y),
            size: PhysicalSize::new(s.width, s.height),
            refresh_rate_millihertz: m.refresh_rate_millihertz(),
        })
    }

    /// The native view or surface handles, `None` for an unsupported window system.
    pub fn raw_handle(&self) -> Option<RawHandle> {
        use winit::raw_window_handle::{
            HasDisplayHandle, HasWindowHandle, RawDisplayHandle as D, RawWindowHandle as W,
        };
        let window = self.0.window_handle().ok()?.as_raw();
        let display = self.0.display_handle().ok()?.as_raw();
        let null = std::ptr::null_mut();
        Some(match (window, display) {
            (W::AppKit(w), _) => RawHandle::AppKit { ns_view: w.ns_view.as_ptr() },
            (W::Wayland(w), D::Wayland(d)) => {
                RawHandle::Wayland { display: d.display.as_ptr(), surface: w.surface.as_ptr() }
            }
            (W::Xlib(w), D::Xlib(d)) => RawHandle::Xlib {
                display: d.display.map_or(null, |p| p.as_ptr()),
                window: w.window,
            },
            (W::Xcb(w), D::Xcb(d)) => RawHandle::Xcb {
                connection: d.connection.map_or(null, |p| p.as_ptr()),
                window: w.window.get(),
            },
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_position_converts_to_logical_by_scale() {
        assert_eq!(PhysicalPosition::new(200, -100).to_logical(2.0), LogicalPosition::new(100.0, -50.0));
    }

    #[test]
    fn cursor_and_resize_enums_map_to_winit_equivalents() {
        assert_eq!(ww::CursorIcon::from(CursorIcon::SwResize), ww::CursorIcon::SwResize);
        assert_eq!(ww::CursorIcon::from(CursorIcon::Pointer), ww::CursorIcon::Pointer);
        assert!(matches!(ww::ResizeDirection::from(ResizeDirection::NorthWest), ww::ResizeDirection::NorthWest));
    }

    #[test]
    fn css_names_parse_to_the_cursor_they_name() {
        assert_eq!("text".parse(), Ok(CursorIcon::Text));
        assert_eq!("nesw-resize".parse(), Ok(CursorIcon::NeswResize));
        assert_eq!("zoom-out".parse(), Ok(CursorIcon::ZoomOut));
        assert_eq!("not-a-cursor".parse::<CursorIcon>(), Err(()));
    }

    #[test]
    fn physical_size_converts_to_logical_by_scale() {
        assert_eq!(PhysicalSize::new(200u32, 100).to_logical(2.0), LogicalSize::new(100.0, 50.0));
    }

    #[test]
    fn theme_maps_both_ways() {
        assert_eq!(Theme::from(ww::Theme::Dark), Theme::Dark);
        assert_eq!(Theme::from(ww::Theme::Light), Theme::Light);
    }
}
