//! The window API the rest of the crate programs against. Everything outside
//! this module sees only these types; the `winit` window behind them is the
//! single thing the native AppKit / Wayland / X11 backends replace.

use std::ffi::c_void;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi as wdpi;
use winit::event as ww_event;
use winit::event::{KeyEvent, StartCause};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
#[cfg(not(target_os = "macos"))]
use winit::event_loop::EventLoop;
use winit::keyboard::{
    Key as WKey, KeyCode as WKeyCode, KeyLocation as WKeyLocation, ModifiersState,
    NamedKey as WNamedKey, PhysicalKey as WPhysicalKey,
};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::window as ww;

use super::input::{Key, KeyCode, KeyInput, KeyLocation, Mods, NamedKey, PhysicalKey};

/// Stable identity of a window for the lifetime of the process.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct WindowId(u64);

impl WindowId {
    #[allow(dead_code)]
    pub const fn from_raw(id: u64) -> Self {
        WindowId(id)
    }

    #[allow(dead_code)]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

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

pub enum EventProxyInner<T: 'static> {
    Winit(winit::event_loop::EventLoopProxy<T>),
    #[cfg(target_os = "macos")]
    Native(std::sync::mpsc::Sender<T>),
}

/// Posts events from any thread to the thread that runs the event loop.
pub struct EventProxy<T: 'static>(pub(crate) EventProxyInner<T>);

impl<T: 'static> EventProxy<T> {
    pub fn new(proxy: winit::event_loop::EventLoopProxy<T>) -> Self {
        EventProxy(EventProxyInner::Winit(proxy))
    }

    #[cfg(target_os = "macos")]
    pub fn from_native(sender: std::sync::mpsc::Sender<T>) -> Self {
        EventProxy(EventProxyInner::Native(sender))
    }

    /// Fails only once the event loop has exited, when nobody is left to handle `event`.
    pub fn send_event(&self, event: T) -> Result<(), T> {
        match &self.0 {
            EventProxyInner::Winit(proxy) => proxy.send_event(event).map_err(|e| e.0),
            #[cfg(target_os = "macos")]
            EventProxyInner::Native(sender) => {
                let res = sender.send(event).map_err(|e| e.0);
                crate::platform::macos::native::app::wakeup_main_loop();
                res
            }
        }
    }
}

impl<T: 'static> Clone for EventProxy<T> {
    fn clone(&self) -> Self {
        EventProxy(match &self.0 {
            EventProxyInner::Winit(p) => EventProxyInner::Winit(p.clone()),
            #[cfg(target_os = "macos")]
            EventProxyInner::Native(s) => EventProxyInner::Native(s.clone()),
        })
    }
}

pub enum WindowInner {
    Winit(ww::Window),
    #[cfg(target_os = "macos")]
    Native(crate::platform::macos::native::window::NativeWindow),
}

pub struct Window(pub WindowInner);

impl Window {
    fn from_winit(window: ww::Window) -> Self {
        Window(WindowInner::Winit(window))
    }

    #[cfg(target_os = "macos")]
    pub fn from_native(window: crate::platform::macos::native::window::NativeWindow) -> Self {
        Window(WindowInner::Native(window))
    }

    /// Escape hatch for backend-specific code that has no equivalent yet (macOS native tabs).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(crate) fn winit(&self) -> &ww::Window {
        match &self.0 {
            WindowInner::Winit(w) => w,
            #[cfg(target_os = "macos")]
            WindowInner::Native(_) => panic!("winit requested on native window"),
        }
    }

    pub fn id(&self) -> WindowId {
        match &self.0 {
            WindowInner::Winit(w) => w.id().into(),
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.id(),
        }
    }

    pub fn request_redraw(&self) {
        match &self.0 {
            WindowInner::Winit(w) => w.request_redraw(),
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.request_redraw(),
        }
    }

    pub fn scale_factor(&self) -> f64 {
        match &self.0 {
            WindowInner::Winit(w) => w.scale_factor(),
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.scale_factor(),
        }
    }

    pub fn inner_size(&self) -> PhysicalSize<u32> {
        match &self.0 {
            WindowInner::Winit(w) => {
                let s = w.inner_size();
                PhysicalSize::new(s.width, s.height)
            }
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.inner_size(),
        }
    }

    pub fn request_inner_size(&self, size: PhysicalSize<u32>) {
        match &self.0 {
            WindowInner::Winit(w) => {
                let _ = w.request_inner_size(wdpi::PhysicalSize::new(size.width, size.height));
            }
            #[cfg(target_os = "macos")]
            WindowInner::Native(_) => {}
        }
    }

    pub fn inner_position(&self) -> Result<PhysicalPosition<i32>, Unsupported> {
        match &self.0 {
            WindowInner::Winit(w) => {
                let p = w.inner_position().map_err(|_| Unsupported)?;
                Ok(PhysicalPosition::new(p.x, p.y))
            }
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.outer_position(),
        }
    }

    pub fn outer_position(&self) -> Result<PhysicalPosition<i32>, Unsupported> {
        match &self.0 {
            WindowInner::Winit(w) => {
                let p = w.outer_position().map_err(|_| Unsupported)?;
                Ok(PhysicalPosition::new(p.x, p.y))
            }
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.outer_position(),
        }
    }

    pub fn set_outer_position(&self, position: PhysicalPosition<i32>) {
        match &self.0 {
            WindowInner::Winit(w) => {
                w.set_outer_position(wdpi::PhysicalPosition::new(position.x, position.y));
            }
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => {
                w.set_outer_position(LogicalPosition::new(position.x as f64, position.y as f64));
            }
        }
    }

    pub fn set_outer_position_logical(&self, position: LogicalPosition<f64>) {
        match &self.0 {
            WindowInner::Winit(w) => {
                w.set_outer_position(wdpi::LogicalPosition::new(position.x, position.y));
            }
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => {
                w.set_outer_position(position);
            }
        }
    }

    pub fn set_visible(&self, visible: bool) {
        match &self.0 {
            WindowInner::Winit(w) => w.set_visible(visible),
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => {
                if visible {
                    w.focus_window();
                }
            }
        }
    }

    pub fn is_visible(&self) -> Option<bool> {
        match &self.0 {
            WindowInner::Winit(w) => w.is_visible(),
            #[cfg(target_os = "macos")]
            WindowInner::Native(_) => Some(true),
        }
    }

    pub fn is_minimized(&self) -> Option<bool> {
        match &self.0 {
            WindowInner::Winit(w) => w.is_minimized(),
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.is_minimized(),
        }
    }

    pub fn set_minimized(&self, minimized: bool) {
        match &self.0 {
            WindowInner::Winit(w) => w.set_minimized(minimized),
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.set_minimized(minimized),
        }
    }

    pub fn is_maximized(&self) -> bool {
        match &self.0 {
            WindowInner::Winit(w) => w.is_maximized(),
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.is_maximized(),
        }
    }

    pub fn set_maximized(&self, maximized: bool) {
        match &self.0 {
            WindowInner::Winit(w) => w.set_maximized(maximized),
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.set_maximized(maximized),
        }
    }

    pub fn is_fullscreen(&self) -> bool {
        match &self.0 {
            WindowInner::Winit(w) => w.fullscreen().is_some(),
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.is_fullscreen(),
        }
    }

    pub fn set_title(&self, title: &str) {
        match &self.0 {
            WindowInner::Winit(w) => w.set_title(title),
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.set_title(title),
        }
    }

    pub fn focus_window(&self) {
        match &self.0 {
            WindowInner::Winit(w) => w.focus_window(),
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.focus_window(),
        }
    }

    pub fn set_cursor(&self, icon: CursorIcon) {
        match &self.0 {
            WindowInner::Winit(w) => w.set_cursor(ww::CursorIcon::from(icon)),
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.set_cursor(icon),
        }
    }

    pub fn set_cursor_visible(&self, visible: bool) {
        match &self.0 {
            WindowInner::Winit(w) => w.set_cursor_visible(visible),
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.set_cursor_visible(visible),
        }
    }

    pub fn theme(&self) -> Option<Theme> {
        match &self.0 {
            WindowInner::Winit(w) => w.theme().map(Theme::from),
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.theme(),
        }
    }

    pub fn set_ime_allowed(&self, allowed: bool) {
        match &self.0 {
            WindowInner::Winit(w) => w.set_ime_allowed(allowed),
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.set_ime_allowed(allowed),
        }
    }

    pub fn set_ime_cursor_area(&self, position: PhysicalPosition<f64>, size: PhysicalSize<f64>) {
        match &self.0 {
            WindowInner::Winit(w) => {
                w.set_ime_cursor_area(
                    wdpi::PhysicalPosition::new(position.x, position.y),
                    wdpi::PhysicalSize::new(size.width, size.height),
                );
            }
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.set_ime_cursor_area(position, size),
        }
    }

    pub fn request_attention(&self, requested: bool) {
        match &self.0 {
            WindowInner::Winit(w) => {
                w.request_user_attention(requested.then_some(ww::UserAttentionType::Informational));
            }
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.request_attention(requested),
        }
    }

    pub fn drag_window(&self) -> Result<(), Unsupported> {
        match &self.0 {
            WindowInner::Winit(w) => w.drag_window().map_err(|_| Unsupported),
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.drag_window(),
        }
    }

    pub fn drag_resize_window(&self, direction: ResizeDirection) -> Result<(), Unsupported> {
        match &self.0 {
            WindowInner::Winit(w) => w.drag_resize_window(direction.into()).map_err(|_| Unsupported),
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.drag_resize_window(direction),
        }
    }

    pub fn current_monitor(&self) -> Option<Monitor> {
        match &self.0 {
            WindowInner::Winit(w) => {
                let m = w.current_monitor()?;
                let (p, s) = (m.position(), m.size());
                Some(Monitor {
                    position: PhysicalPosition::new(p.x, p.y),
                    size: PhysicalSize::new(s.width, s.height),
                    refresh_rate_millihertz: m.refresh_rate_millihertz(),
                })
            }
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.current_monitor(),
        }
    }

    pub fn raw_handle(&self) -> Option<RawHandle> {
        match &self.0 {
            WindowInner::Winit(w) => {
                use winit::raw_window_handle::{
                    HasDisplayHandle, HasWindowHandle, RawDisplayHandle as D, RawWindowHandle as W,
                };
                let window = w.window_handle().ok()?.as_raw();
                let display = w.display_handle().ok()?.as_raw();
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
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.raw_handle(),
        }
    }
}

// ---- Events -----------------------------------------------------------------

#[allow(dead_code)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Other,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ButtonState {
    Pressed,
    Released,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum ScrollDelta {
    /// Wheel notches (positive `y` scrolls up).
    Lines { y: f32 },
    Pixels(PhysicalPosition<f64>),
}

#[allow(dead_code)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TouchPhase {
    Started,
    Moved,
    Ended,
    Cancelled,
}

/// Composed text input (Vietnamese Telex/VNI, CJK).
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ime {
    Enabled,
    Preedit(String),
    Commit(String),
    Disabled,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub enum WindowEvent {
    CloseRequested,
    ModifiersChanged(Mods),
    Resized(PhysicalSize<u32>),
    ScaleFactorChanged(f64),
    Key(KeyInput),
    Ime(Ime),
    DroppedFile(std::path::PathBuf),
    ThemeChanged(Theme),
    Occluded(bool),
    CursorMoved(PhysicalPosition<f64>),
    MouseInput { state: ButtonState, button: MouseButton },
    MouseWheel { delta: ScrollDelta, phase: TouchPhase },
    Focused(bool),
    RedrawRequested,
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
fn convert_event(event: ww_event::WindowEvent) -> Option<WindowEvent> {
    use ww_event::WindowEvent as E;
    Some(match event {
        E::CloseRequested => WindowEvent::CloseRequested,
        E::ModifiersChanged(m) => WindowEvent::ModifiersChanged(convert_mods(m.state())),
        E::Resized(s) => WindowEvent::Resized(PhysicalSize::new(s.width, s.height)),
        E::ScaleFactorChanged { scale_factor, .. } => WindowEvent::ScaleFactorChanged(scale_factor),
        E::KeyboardInput { event, .. } => WindowEvent::Key(convert_key_event(&event)),
        E::Ime(ime) => WindowEvent::Ime(match ime {
            ww_event::Ime::Enabled => Ime::Enabled,
            ww_event::Ime::Preedit(text, _) => Ime::Preedit(text),
            ww_event::Ime::Commit(text) => Ime::Commit(text),
            ww_event::Ime::Disabled => Ime::Disabled,
        }),
        E::DroppedFile(path) => WindowEvent::DroppedFile(path),
        E::ThemeChanged(t) => WindowEvent::ThemeChanged(t.into()),
        E::Occluded(o) => WindowEvent::Occluded(o),
        E::CursorMoved { position, .. } => WindowEvent::CursorMoved(position.into()),
        E::MouseInput { state, button, .. } => WindowEvent::MouseInput {
            state: match state {
                ww_event::ElementState::Pressed => ButtonState::Pressed,
                ww_event::ElementState::Released => ButtonState::Released,
            },
            button: match button {
                ww_event::MouseButton::Left => MouseButton::Left,
                ww_event::MouseButton::Right => MouseButton::Right,
                ww_event::MouseButton::Middle => MouseButton::Middle,
                _ => MouseButton::Other,
            },
        },
        E::MouseWheel { delta, phase, .. } => WindowEvent::MouseWheel {
            delta: match delta {
                ww_event::MouseScrollDelta::LineDelta(_, y) => ScrollDelta::Lines { y },
                ww_event::MouseScrollDelta::PixelDelta(p) => ScrollDelta::Pixels(p.into()),
            },
            phase: match phase {
                ww_event::TouchPhase::Started => TouchPhase::Started,
                ww_event::TouchPhase::Moved => TouchPhase::Moved,
                ww_event::TouchPhase::Ended => TouchPhase::Ended,
                ww_event::TouchPhase::Cancelled => TouchPhase::Cancelled,
            },
        },
        E::Focused(f) => WindowEvent::Focused(f),
        E::RedrawRequested => WindowEvent::RedrawRequested,
        _ => return None,
    })
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
fn convert_mods(state: ModifiersState) -> Mods {
    Mods {
        ctrl: state.control_key(),
        alt: state.alt_key(),
        shift: state.shift_key(),
        sup: state.super_key(),
    }
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
fn convert_key_event(ev: &KeyEvent) -> KeyInput {
    KeyInput {
        logical: convert_key(&ev.logical_key),
        unshifted: convert_key(&ev.key_without_modifiers()),
        physical: physical(ev.physical_key),
        location: location(ev.location),
        text: ev.text.as_ref().map(|t| t.to_string()),
        pressed: ev.state == ww_event::ElementState::Pressed,
        repeat: ev.repeat,
    }
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
fn convert_key(k: &WKey) -> Key {
    match k {
        WKey::Character(c) => Key::Character(c.to_string()),
        WKey::Named(n) => named(*n).map_or(Key::Unidentified, Key::Named),
        _ => Key::Unidentified,
    }
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
fn named(k: WNamedKey) -> Option<NamedKey> {
    Some(match k {
        WNamedKey::Enter => NamedKey::Enter,
        WNamedKey::Tab => NamedKey::Tab,
        WNamedKey::Space => NamedKey::Space,
        WNamedKey::Backspace => NamedKey::Backspace,
        WNamedKey::Escape => NamedKey::Escape,
        WNamedKey::Delete => NamedKey::Delete,
        WNamedKey::Insert => NamedKey::Insert,
        WNamedKey::Home => NamedKey::Home,
        WNamedKey::End => NamedKey::End,
        WNamedKey::PageUp => NamedKey::PageUp,
        WNamedKey::PageDown => NamedKey::PageDown,
        WNamedKey::ArrowUp => NamedKey::ArrowUp,
        WNamedKey::ArrowDown => NamedKey::ArrowDown,
        WNamedKey::ArrowLeft => NamedKey::ArrowLeft,
        WNamedKey::ArrowRight => NamedKey::ArrowRight,
        WNamedKey::F1 => NamedKey::F1,
        WNamedKey::F2 => NamedKey::F2,
        WNamedKey::F3 => NamedKey::F3,
        WNamedKey::F4 => NamedKey::F4,
        WNamedKey::F5 => NamedKey::F5,
        WNamedKey::F6 => NamedKey::F6,
        WNamedKey::F7 => NamedKey::F7,
        WNamedKey::F8 => NamedKey::F8,
        WNamedKey::F9 => NamedKey::F9,
        WNamedKey::F10 => NamedKey::F10,
        WNamedKey::F11 => NamedKey::F11,
        WNamedKey::F12 => NamedKey::F12,
        WNamedKey::F13 => NamedKey::F13,
        WNamedKey::F14 => NamedKey::F14,
        WNamedKey::F15 => NamedKey::F15,
        WNamedKey::F16 => NamedKey::F16,
        WNamedKey::F17 => NamedKey::F17,
        WNamedKey::F18 => NamedKey::F18,
        WNamedKey::F19 => NamedKey::F19,
        WNamedKey::F20 => NamedKey::F20,
        WNamedKey::Shift => NamedKey::Shift,
        WNamedKey::Control => NamedKey::Control,
        WNamedKey::Alt => NamedKey::Alt,
        WNamedKey::Super => NamedKey::Super,
        WNamedKey::Meta => NamedKey::Meta,
        WNamedKey::Hyper => NamedKey::Hyper,
        WNamedKey::CapsLock => NamedKey::CapsLock,
        WNamedKey::NumLock => NamedKey::NumLock,
        WNamedKey::ScrollLock => NamedKey::ScrollLock,
        WNamedKey::PrintScreen => NamedKey::PrintScreen,
        WNamedKey::Pause => NamedKey::Pause,
        WNamedKey::ContextMenu => NamedKey::ContextMenu,
        WNamedKey::Clear => NamedKey::Clear,
        WNamedKey::MediaPlay => NamedKey::MediaPlay,
        WNamedKey::MediaPause => NamedKey::MediaPause,
        WNamedKey::MediaPlayPause => NamedKey::MediaPlayPause,
        WNamedKey::MediaStop => NamedKey::MediaStop,
        WNamedKey::MediaRecord => NamedKey::MediaRecord,
        WNamedKey::MediaFastForward => NamedKey::MediaFastForward,
        WNamedKey::MediaRewind => NamedKey::MediaRewind,
        WNamedKey::MediaTrackNext => NamedKey::MediaTrackNext,
        WNamedKey::MediaTrackPrevious => NamedKey::MediaTrackPrevious,
        WNamedKey::AudioVolumeDown => NamedKey::AudioVolumeDown,
        WNamedKey::AudioVolumeUp => NamedKey::AudioVolumeUp,
        WNamedKey::AudioVolumeMute => NamedKey::AudioVolumeMute,
        _ => return None,
    })
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
fn physical(k: WPhysicalKey) -> PhysicalKey {
    match k {
        WPhysicalKey::Code(c) => PhysicalKey::Code(code(c)),
        // The native scancode is opaque; one shared value is enough because a
        // release only needs to find its press among at most 16 consumed keys.
        WPhysicalKey::Unidentified(_) => PhysicalKey::Unidentified(0),
    }
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
fn code(c: WKeyCode) -> KeyCode {
    match c {
        WKeyCode::KeyA => KeyCode::KeyA,
        WKeyCode::KeyB => KeyCode::KeyB,
        WKeyCode::KeyC => KeyCode::KeyC,
        WKeyCode::KeyD => KeyCode::KeyD,
        WKeyCode::KeyE => KeyCode::KeyE,
        WKeyCode::KeyF => KeyCode::KeyF,
        WKeyCode::KeyG => KeyCode::KeyG,
        WKeyCode::KeyH => KeyCode::KeyH,
        WKeyCode::KeyI => KeyCode::KeyI,
        WKeyCode::KeyJ => KeyCode::KeyJ,
        WKeyCode::KeyK => KeyCode::KeyK,
        WKeyCode::KeyL => KeyCode::KeyL,
        WKeyCode::KeyM => KeyCode::KeyM,
        WKeyCode::KeyN => KeyCode::KeyN,
        WKeyCode::KeyO => KeyCode::KeyO,
        WKeyCode::KeyP => KeyCode::KeyP,
        WKeyCode::KeyQ => KeyCode::KeyQ,
        WKeyCode::KeyR => KeyCode::KeyR,
        WKeyCode::KeyS => KeyCode::KeyS,
        WKeyCode::KeyT => KeyCode::KeyT,
        WKeyCode::KeyU => KeyCode::KeyU,
        WKeyCode::KeyV => KeyCode::KeyV,
        WKeyCode::KeyW => KeyCode::KeyW,
        WKeyCode::KeyX => KeyCode::KeyX,
        WKeyCode::KeyY => KeyCode::KeyY,
        WKeyCode::KeyZ => KeyCode::KeyZ,
        WKeyCode::Digit0 => KeyCode::Digit0,
        WKeyCode::Digit1 => KeyCode::Digit1,
        WKeyCode::Digit2 => KeyCode::Digit2,
        WKeyCode::Digit3 => KeyCode::Digit3,
        WKeyCode::Digit4 => KeyCode::Digit4,
        WKeyCode::Digit5 => KeyCode::Digit5,
        WKeyCode::Digit6 => KeyCode::Digit6,
        WKeyCode::Digit7 => KeyCode::Digit7,
        WKeyCode::Digit8 => KeyCode::Digit8,
        WKeyCode::Digit9 => KeyCode::Digit9,
        WKeyCode::Backquote => KeyCode::Backquote,
        WKeyCode::Minus => KeyCode::Minus,
        WKeyCode::Equal => KeyCode::Equal,
        WKeyCode::BracketLeft => KeyCode::BracketLeft,
        WKeyCode::BracketRight => KeyCode::BracketRight,
        WKeyCode::Backslash => KeyCode::Backslash,
        WKeyCode::Semicolon => KeyCode::Semicolon,
        WKeyCode::Quote => KeyCode::Quote,
        WKeyCode::Comma => KeyCode::Comma,
        WKeyCode::Period => KeyCode::Period,
        WKeyCode::Slash => KeyCode::Slash,
        WKeyCode::Space => KeyCode::Space,
        WKeyCode::Numpad5 => KeyCode::Numpad5,
        other => KeyCode::Other(other as u32),
    }
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
fn location(l: WKeyLocation) -> KeyLocation {
    match l {
        WKeyLocation::Standard => KeyLocation::Standard,
        WKeyLocation::Left => KeyLocation::Left,
        WKeyLocation::Right => KeyLocation::Right,
        WKeyLocation::Numpad => KeyLocation::Numpad,
    }
}

// ---- Event loop -------------------------------------------------------------

/// How long the loop may sleep before the next `about_to_wait`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Flow {
    Wait,
    Poll,
    WaitUntil(Instant),
}

/// What a new window looks like.
pub struct WindowSpec {
    pub title: &'static str,
    pub decorations: bool,
    pub always_on_top: bool,
    pub visible: bool,
    pub transparent: bool,
    pub inner_size: LogicalSize<f64>,
    pub position: Option<LogicalPosition<f64>>,
}

pub enum LoopCtxInner<'a> {
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Winit(&'a ActiveEventLoop),
    #[cfg(target_os = "macos")]
    Native(&'a crate::platform::macos::native::app::NativeLoopCtx),
}

/// The running event loop, as seen from a handler callback.
pub struct LoopCtx<'a>(pub(crate) LoopCtxInner<'a>);

impl<'a> LoopCtx<'a> {
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub fn from_winit(w: &'a ActiveEventLoop) -> Self {
        LoopCtx(LoopCtxInner::Winit(w))
    }

    #[cfg(target_os = "macos")]
    pub fn from_native(n: &'a crate::platform::macos::native::app::NativeLoopCtx) -> Self {
        LoopCtx(LoopCtxInner::Native(n))
    }

    pub fn exit(&self) {
        match &self.0 {
            LoopCtxInner::Winit(w) => w.exit(),
            #[cfg(target_os = "macos")]
            LoopCtxInner::Native(n) => n.exit(),
        }
    }

    pub fn set_flow(&self, flow: Flow) {
        match &self.0 {
            LoopCtxInner::Winit(w) => {
                w.set_control_flow(match flow {
                    Flow::Wait => ControlFlow::Wait,
                    Flow::Poll => ControlFlow::Poll,
                    Flow::WaitUntil(t) => ControlFlow::WaitUntil(t),
                });
            }
            #[cfg(target_os = "macos")]
            LoopCtxInner::Native(n) => n.set_flow(flow),
        }
    }

    pub fn create_window(&self, spec: &WindowSpec) -> std::io::Result<Window> {
        match &self.0 {
            LoopCtxInner::Winit(w) => {
                let attrs = ww::Window::default_attributes()
                    .with_title(spec.title)
                    .with_decorations(spec.decorations)
                    .with_window_level(if spec.always_on_top { ww::WindowLevel::AlwaysOnTop } else { ww::WindowLevel::Normal })
                    .with_visible(spec.visible)
                    .with_transparent(spec.transparent)
                    .with_inner_size(wdpi::LogicalSize::new(spec.inner_size.width, spec.inner_size.height));
                let attrs = match spec.position {
                    Some(p) => attrs.with_position(wdpi::LogicalPosition::new(p.x, p.y)),
                    None => attrs,
                };
                // Wayland app_id / X11 WM_CLASS must match `litty.desktop` and the
                // hicolor icon name, or the shell shows a generic icon.
                #[cfg(target_os = "linux")]
                let attrs = {
                    use winit::platform::wayland::WindowAttributesExtWayland;
                    attrs.with_name("litty", "litty")
                };
                // Native tabs group under one identifier. Created hidden so AppKit
                // cannot merge a New Window into a tab of the focused one before
                // tabbing is turned off for it; the caller shows it afterwards.
                #[cfg(target_os = "macos")]
                let attrs = {
                    use winit::platform::macos::WindowAttributesExtMacOS;
                    attrs.with_tabbing_identifier("litty").with_visible(false)
                };
                w.create_window(attrs).map(Window::from_winit).map_err(std::io::Error::other)
            }
            #[cfg(target_os = "macos")]
            LoopCtxInner::Native(_) => {
                crate::platform::macos::native::window::NativeWindow::create(spec).map(Window::from_native)
            }
        }
    }
}

/// Callbacks of the application, in the order the loop produces them.
pub trait EventHandler<T: 'static> {
    /// A loop iteration starts; `timer_fired` when it was woken by `Flow::WaitUntil`.
    fn cycle_started(&mut self, timer_fired: bool);
    fn resumed(&mut self, ctx: &LoopCtx);
    fn user_event(&mut self, ctx: &LoopCtx, event: T);
    fn window_event(&mut self, ctx: &LoopCtx, id: WindowId, event: WindowEvent);
    fn about_to_wait(&mut self, ctx: &LoopCtx);
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
struct Driver<H>(H);

impl<T: 'static, H: EventHandler<T>> ApplicationHandler<T> for Driver<H> {
    fn new_events(&mut self, _: &ActiveEventLoop, cause: StartCause) {
        self.0.cycle_started(matches!(cause, StartCause::ResumeTimeReached { .. }));
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.0.resumed(&LoopCtx::from_winit(event_loop));
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: T) {
        self.0.user_event(&LoopCtx::from_winit(event_loop), event);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: ww::WindowId, event: ww_event::WindowEvent) {
        if let Some(event) = convert_event(event) {
            self.0.window_event(&LoopCtx::from_winit(event_loop), id.into(), event);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.0.about_to_wait(&LoopCtx::from_winit(event_loop));
    }
}

/// Run the event loop until the handler exits it. `make` receives the proxy
/// other threads use to post events.
pub fn run<T: 'static, H: EventHandler<T>>(make: impl FnOnce(EventProxy<T>) -> H) {
    #[cfg(target_os = "macos")]
    {
        crate::platform::macos::native::app::run_native(make);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let event_loop = EventLoop::<T>::with_user_event().build().expect("event loop");
        event_loop.set_control_flow(ControlFlow::Wait);
        let mut driver = Driver(make(EventProxy::new(event_loop.create_proxy())));
        event_loop.run_app(&mut driver).expect("run app");
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


    #[test]
    fn named_keys_map_one_to_one() {
        assert_eq!(named(WNamedKey::Enter), Some(NamedKey::Enter));
        assert_eq!(named(WNamedKey::ArrowLeft), Some(NamedKey::ArrowLeft));
        assert_eq!(named(WNamedKey::F20), Some(NamedKey::F20));
        assert_eq!(named(WNamedKey::BrowserBack), None);
    }

    #[test]
    fn unmapped_physical_keys_keep_distinct_codes() {
        let a = physical(WPhysicalKey::Code(WKeyCode::F5));
        let b = physical(WPhysicalKey::Code(WKeyCode::F6));
        assert_ne!(a, b);
        assert_eq!(physical(WPhysicalKey::Code(WKeyCode::KeyA)), PhysicalKey::Code(KeyCode::KeyA));
    }

    #[test]
    fn modifiers_translate() {
        let m = convert_mods(ModifiersState::CONTROL | ModifiersState::SHIFT);
        assert_eq!(m, Mods { ctrl: true, shift: true, alt: false, sup: false });
    }

    #[test]
    fn dead_and_unknown_keys_are_unidentified() {
        assert_eq!(convert_key(&WKey::Dead(Some('`'))), Key::Unidentified);
        assert_eq!(convert_key(&WKey::Character("a".into())), Key::Character("a".into()));
    }
}
