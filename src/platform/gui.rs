//! Platform-neutral GUI abstraction for native AppKit (macOS) and epoll/Wayland/X11 (Linux).
//!
//! Exposes a uniform, minimal surface for windows, mouse/keyboard/touch events,
//! DPI-scaled coordinate spaces, and the top-level application loop. All rendering
//! layers and application logic program against this module.

use std::ffi::c_void;
use std::time::Instant;

use super::input::{KeyInput, Mods};

/// Stable identity of a window for the lifetime of the process.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(pub(crate) u64);

impl WindowId {
    pub const fn from_raw(raw: u64) -> Self {
        WindowId(raw)
    }

    #[allow(dead_code)]
    pub const fn raw(&self) -> u64 {
        self.0
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
    pub fn to_logical(self, scale_factor: f64) -> LogicalPosition<f64> {
        LogicalPosition::new(self.x as f64 / scale_factor, self.y as f64 / scale_factor)
    }
}

impl PhysicalPosition<f64> {
    #[allow(dead_code)]
    pub fn to_logical(self, scale_factor: f64) -> LogicalPosition<f64> {
        LogicalPosition::new(self.x / scale_factor, self.y / scale_factor)
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

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
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
    pub fn to_logical(self, scale_factor: f64) -> LogicalSize<f64> {
        LogicalSize::new(
            self.width as f64 / scale_factor,
            self.height as f64 / scale_factor,
        )
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

/// Standard mouse pointer shapes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum CursorIcon {
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

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
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

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}

/// The platform cannot do what was asked (e.g. window positions on Wayland).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Unsupported;

#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    pub position: PhysicalPosition<i32>,
    pub size: PhysicalSize<u32>,
    pub refresh_rate_millihertz: Option<u32>,
}

/// Raw platform handle to pass to GPU surface creation (Metal, Vulkan).
#[allow(dead_code)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RawHandle {
    AppKit { ns_view: *mut c_void },
    Wayland { display: *mut c_void, surface: *mut c_void },
    Xlib { display: *mut c_void, window: std::ffi::c_ulong },
    Xcb { connection: *mut c_void, window: u32 },
}

pub enum EventProxyInner<T: 'static> {
    #[cfg(target_os = "macos")]
    Native(std::sync::mpsc::Sender<T>),
    #[cfg(target_os = "linux")]
    NativeLinux(crate::platform::linux::native::app::LinuxProxy<T>),
}

/// Thread-safe capability to wake the event loop and post a user event.
pub struct EventProxy<T: 'static>(pub(crate) EventProxyInner<T>);

impl<T: 'static> EventProxy<T> {
    #[cfg(target_os = "macos")]
    pub fn from_native(sender: std::sync::mpsc::Sender<T>) -> Self {
        EventProxy(EventProxyInner::Native(sender))
    }

    #[cfg(target_os = "linux")]
    pub fn from_native_linux(sender: std::sync::mpsc::Sender<T>, event_fd: std::os::fd::RawFd) -> Self {
        EventProxy(EventProxyInner::NativeLinux(crate::platform::linux::native::app::LinuxProxy::new(sender, event_fd)))
    }

    /// Post `event` into the application's event queue from any thread.
    /// Fails only once the event loop has exited, when nobody is left to handle `event`.
    pub fn send_event(&self, event: T) -> Result<(), T> {
        match &self.0 {
            #[cfg(target_os = "macos")]
            EventProxyInner::Native(sender) => {
                let res = sender.send(event).map_err(|e| e.0);
                if res.is_ok() {
                    crate::platform::macos::native::app::wakeup_main_loop();
                }
                res
            }
            #[cfg(target_os = "linux")]
            EventProxyInner::NativeLinux(proxy) => proxy.send(event),
        }
    }
}

impl<T: 'static> Clone for EventProxy<T> {
    fn clone(&self) -> Self {
        EventProxy(match &self.0 {
            #[cfg(target_os = "macos")]
            EventProxyInner::Native(s) => EventProxyInner::Native(s.clone()),
            #[cfg(target_os = "linux")]
            EventProxyInner::NativeLinux(p) => EventProxyInner::NativeLinux(p.clone()),
        })
    }
}

pub enum WindowInner {
    #[cfg(target_os = "macos")]
    Native(crate::platform::macos::native::window::NativeWindow),
    #[cfg(target_os = "linux")]
    NativeLinux(crate::platform::linux::native::window::NativeWindow),
}

pub struct Window(pub WindowInner);

impl Window {
    #[cfg(target_os = "macos")]
    pub fn from_native(window: crate::platform::macos::native::window::NativeWindow) -> Self {
        Window(WindowInner::Native(window))
    }

    #[cfg(target_os = "linux")]
    pub fn from_native_linux(window: crate::platform::linux::native::window::NativeWindow) -> Self {
        Window(WindowInner::NativeLinux(window))
    }

    pub fn id(&self) -> WindowId {
        match &self.0 {
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.id(),
            #[cfg(target_os = "linux")]
            WindowInner::NativeLinux(w) => w.id(),
        }
    }

    pub fn request_redraw(&self) {
        match &self.0 {
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.request_redraw(),
            #[cfg(target_os = "linux")]
            WindowInner::NativeLinux(w) => w.request_redraw(),
        }
    }

    pub fn scale_factor(&self) -> f64 {
        match &self.0 {
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.scale_factor(),
            #[cfg(target_os = "linux")]
            WindowInner::NativeLinux(w) => w.scale_factor(),
        }
    }

    pub fn inner_size(&self) -> PhysicalSize<u32> {
        match &self.0 {
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.inner_size(),
            #[cfg(target_os = "linux")]
            WindowInner::NativeLinux(w) => w.inner_size(),
        }
    }

    pub fn inner_position(&self) -> Result<PhysicalPosition<i32>, Unsupported> {
        match &self.0 {
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.inner_position(),
            #[cfg(target_os = "linux")]
            WindowInner::NativeLinux(w) => w.outer_position(),
        }
    }

    pub fn outer_position(&self) -> Result<PhysicalPosition<i32>, Unsupported> {
        match &self.0 {
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.outer_position(),
            #[cfg(target_os = "linux")]
            WindowInner::NativeLinux(w) => w.outer_position(),
        }
    }

    pub fn set_outer_position(&self, position: PhysicalPosition<i32>) {
        match &self.0 {
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => {
                w.set_outer_position(LogicalPosition::new(position.x as f64, position.y as f64));
            }
            #[cfg(target_os = "linux")]
            WindowInner::NativeLinux(w) => w.set_outer_position_physical(position),
        }
    }

    pub fn set_outer_position_logical(&self, position: LogicalPosition<f64>) {
        match &self.0 {
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => {
                w.set_outer_position(position);
            }
            #[cfg(target_os = "linux")]
            WindowInner::NativeLinux(w) => {
                w.set_outer_position(position);
            }
        }
    }

    pub fn set_visible(&self, visible: bool) {
        match &self.0 {
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => {
                w.set_visible(visible);
            }
            #[cfg(target_os = "linux")]
            WindowInner::NativeLinux(w) => {
                w.set_visible(visible);
            }
        }
    }

    pub fn is_visible(&self) -> Option<bool> {
        match &self.0 {
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => Some(w.is_visible()),
            #[cfg(target_os = "linux")]
            WindowInner::NativeLinux(w) => Some(w.is_visible()),
        }
    }

    pub fn is_minimized(&self) -> Option<bool> {
        match &self.0 {
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.is_minimized(),
            #[cfg(target_os = "linux")]
            WindowInner::NativeLinux(w) => w.is_minimized(),
        }
    }

    pub fn set_minimized(&self, minimized: bool) {
        match &self.0 {
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.set_minimized(minimized),
            #[cfg(target_os = "linux")]
            WindowInner::NativeLinux(w) => w.set_minimized(minimized),
        }
    }

    pub fn is_maximized(&self) -> bool {
        match &self.0 {
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.is_maximized(),
            #[cfg(target_os = "linux")]
            WindowInner::NativeLinux(w) => w.is_maximized(),
        }
    }

    pub fn set_maximized(&self, maximized: bool) {
        match &self.0 {
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.set_maximized(maximized),
            #[cfg(target_os = "linux")]
            WindowInner::NativeLinux(w) => w.set_maximized(maximized),
        }
    }

    pub fn is_fullscreen(&self) -> bool {
        match &self.0 {
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.is_fullscreen(),
            #[cfg(target_os = "linux")]
            WindowInner::NativeLinux(w) => w.is_fullscreen(),
        }
    }

    pub fn set_title(&self, title: &str) {
        match &self.0 {
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.set_title(title),
            #[cfg(target_os = "linux")]
            WindowInner::NativeLinux(w) => w.set_title(title),
        }
    }

    pub fn focus_window(&self) {
        match &self.0 {
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.focus_window(),
            #[cfg(target_os = "linux")]
            WindowInner::NativeLinux(w) => w.focus_window(),
        }
    }

    pub fn set_cursor(&self, icon: CursorIcon) {
        match &self.0 {
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.set_cursor(icon),
            #[cfg(target_os = "linux")]
            WindowInner::NativeLinux(w) => w.set_cursor_icon(icon),
        }
    }

    pub fn set_cursor_visible(&self, visible: bool) {
        match &self.0 {
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.set_cursor_visible(visible),
            #[cfg(target_os = "linux")]
            WindowInner::NativeLinux(w) => w.set_cursor_visible(visible),
        }
    }

    pub fn theme(&self) -> Option<Theme> {
        match &self.0 {
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.theme(),
            #[cfg(target_os = "linux")]
            WindowInner::NativeLinux(w) => w.theme(),
        }
    }

    pub fn set_ime_allowed(&self, allowed: bool) {
        match &self.0 {
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.set_ime_allowed(allowed),
            #[cfg(target_os = "linux")]
            WindowInner::NativeLinux(w) => w.set_ime_allowed(allowed),
        }
    }

    pub fn set_ime_cursor_area(&self, position: PhysicalPosition<f64>, size: PhysicalSize<f64>) {
        match &self.0 {
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.set_ime_cursor_area(position, size),
            #[cfg(target_os = "linux")]
            WindowInner::NativeLinux(w) => w.set_ime_cursor_area(position, size),
        }
    }

    pub fn request_attention(&self, requested: bool) {
        match &self.0 {
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.request_attention(requested),
            #[cfg(target_os = "linux")]
            WindowInner::NativeLinux(w) => w.request_attention(requested),
        }
    }

    pub fn drag_window(&self) -> Result<(), Unsupported> {
        match &self.0 {
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.drag_window(),
            #[cfg(target_os = "linux")]
            WindowInner::NativeLinux(w) => w.drag_window(),
        }
    }

    pub fn drag_resize_window(&self, direction: ResizeDirection) -> Result<(), Unsupported> {
        match &self.0 {
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.drag_resize_window(direction),
            #[cfg(target_os = "linux")]
            WindowInner::NativeLinux(w) => w.drag_resize_window(direction),
        }
    }

    pub fn current_monitor(&self) -> Option<Monitor> {
        match &self.0 {
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.current_monitor(),
            #[cfg(target_os = "linux")]
            WindowInner::NativeLinux(w) => w.current_monitor(),
        }
    }

    pub fn raw_handle(&self) -> Option<RawHandle> {
        match &self.0 {
            #[cfg(target_os = "macos")]
            WindowInner::Native(w) => w.raw_handle(),
            #[cfg(target_os = "linux")]
            WindowInner::NativeLinux(w) => w.raw_handle(),
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

#[allow(dead_code)]
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

// ---- Event loop -------------------------------------------------------------

/// How long the loop may sleep before the next `about_to_wait`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Flow {
    Wait,
    Poll,
    WaitUntil(Instant),
}

/// What a new window looks like.
#[allow(dead_code)]
pub struct WindowSpec {
    pub title: &'static str,
    pub decorations: bool,
    pub visible: bool,
    pub transparent: bool,
    pub inner_size: LogicalSize<f64>,
    pub position: Option<LogicalPosition<f64>>,
}

pub enum LoopCtxInner<'a> {
    #[cfg(target_os = "macos")]
    Native(&'a crate::platform::macos::native::app::NativeLoopCtx),
    #[cfg(target_os = "linux")]
    NativeLinux(&'a crate::platform::linux::native::app::NativeLoopCtx),
}

/// The running event loop, as seen from a handler callback.
pub struct LoopCtx<'a>(pub(crate) LoopCtxInner<'a>);

impl<'a> LoopCtx<'a> {
    #[cfg(target_os = "macos")]
    pub fn from_native(n: &'a crate::platform::macos::native::app::NativeLoopCtx) -> Self {
        LoopCtx(LoopCtxInner::Native(n))
    }

    #[cfg(target_os = "linux")]
    pub fn from_native_linux(n: &'a crate::platform::linux::native::app::NativeLoopCtx) -> Self {
        LoopCtx(LoopCtxInner::NativeLinux(n))
    }

    pub fn exit(&self) {
        match &self.0 {
            #[cfg(target_os = "macos")]
            LoopCtxInner::Native(n) => n.exit(),
            #[cfg(target_os = "linux")]
            LoopCtxInner::NativeLinux(n) => n.exit(),
        }
    }

    pub fn set_flow(&self, flow: Flow) {
        match &self.0 {
            #[cfg(target_os = "macos")]
            LoopCtxInner::Native(n) => n.set_flow(flow),
            #[cfg(target_os = "linux")]
            LoopCtxInner::NativeLinux(n) => n.set_flow(flow),
        }
    }

    pub fn create_window(&self, spec: &WindowSpec) -> std::io::Result<Window> {
        match &self.0 {
            #[cfg(target_os = "macos")]
            LoopCtxInner::Native(_) => {
                crate::platform::macos::native::window::NativeWindow::create(spec).map(Window::from_native)
            }
            #[cfg(target_os = "linux")]
            LoopCtxInner::NativeLinux(_) => {
                crate::platform::linux::native::window::NativeWindow::create(spec).map(Window::from_native_linux)
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

/// Run the event loop until the handler exits it. `make` receives the proxy
/// other threads use to post events.
pub fn run<T: 'static, H: EventHandler<T>>(make: impl FnOnce(EventProxy<T>) -> H) {
    #[cfg(target_os = "macos")]
    {
        crate::platform::macos::native::app::run_native(make);
    }
    #[cfg(target_os = "linux")]
    {
        crate::platform::linux::native::app::run_native(make);
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

    /// `set_outer_position` takes physical pixels (the new-window cascade);
    /// the Linux backend used to scale them a second time.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_outer_position_stays_physical() {
        let native = crate::platform::linux::native::window::detached(800, 600);
        native.set_scale_factor_for_test(2.0);
        let window = Window::from_native_linux(native);
        window.set_outer_position(PhysicalPosition::new(1920, 40));
        assert_eq!(window.outer_position(), Ok(PhysicalPosition::new(1920, 40)));
    }
}
