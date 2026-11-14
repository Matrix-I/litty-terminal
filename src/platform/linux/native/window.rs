//! Native Linux window, over an X11 or a Wayland backend.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use parking_lot::Mutex;

use crate::platform::gui::{
    CursorIcon, LogicalPosition, WindowEvent, Monitor, PhysicalPosition, PhysicalSize,
    RawHandle, ResizeDirection, Theme, Unsupported, WindowId, WindowSpec,
};
use super::wayland::{BackendChoice, WaylandWindow};
use super::x11::X11Window;

static NEXT_WINDOW_ID: AtomicU64 = AtomicU64::new(1);

pub fn allocate_window_id() -> WindowId {
    WindowId::from_raw(NEXT_WINDOW_ID.fetch_add(1, Ordering::Relaxed))
}

pub struct NativeWindowInner {
    pub title: String,
    pub inner_size: PhysicalSize<u32>,
    pub position: PhysicalPosition<i32>,
    pub scale_factor: f64,
    pub visible: bool,
    pub minimized: bool,
    pub maximized: bool,
    pub fullscreen: bool,
    /// The window system draws the frame; otherwise the app does.
    pub server_decorations: bool,
    pub cursor_visible: bool,
    pub cursor_icon: CursorIcon,
    pub raw_handle: Option<RawHandle>,
    /// `None` only for the windowless test double.
    pub backend: Option<Backend>,
}

/// The window system connection behind a `NativeWindow`.
#[derive(Clone)]
pub enum Backend {
    X11(X11Window),
    Wayland(WaylandWindow),
}

impl Backend {
    /// Append the events this window's connection has queued.
    pub fn poll_events(&self, events: &mut Vec<WindowEvent>) {
        match self {
            Backend::X11(x11) => x11.poll_events(events),
            Backend::Wayland(wl) => wl.poll_events(events),
        }
    }

    /// Events are queued in the X11 connection. Wayland's queue is checked
    /// once for all windows (`wayland::prepare_wait`).
    pub fn pending(&self) -> bool {
        match self {
            Backend::X11(x11) => x11.pending(),
            Backend::Wayland(_) => false,
        }
    }

    fn x11(&self) -> Option<&X11Window> {
        match self {
            Backend::X11(x11) => Some(x11),
            Backend::Wayland(_) => None,
        }
    }
}

unsafe impl Send for NativeWindowInner {}
unsafe impl Sync for NativeWindowInner {}

#[derive(Clone)]
pub struct NativeWindow {
    id: WindowId,
    inner: Arc<Mutex<NativeWindowInner>>,
}

impl NativeWindow {
    pub fn create(spec: &WindowSpec) -> Result<Self, std::io::Error> {
        let id = allocate_window_id();

        let wayland = match super::wayland::selected_backend() {
            BackendChoice::Wayland => WaylandWindow::create(spec, id)
                .inspect_err(|e| eprintln!("litty: {e}; opening an X11 window instead"))
                .ok(),
            BackendChoice::X11 => None,
        };
        let (backend, inner_size, position, scale, server_decorations) = match wayland {
            // Wayland never reveals the position.
            Some(wl) => {
                let decorations = wl.server_decorations();
                (Backend::Wayland(wl.clone()), wl.inner_size(), PhysicalPosition::new(0, 0), wl.scale_factor(), decorations)
            }
            None => {
                // Without an X11 window there is nothing to draw into: fail
                // loudly instead of running a windowless process.
                let x11_win = X11Window::create(spec).ok_or_else(|| {
                    std::io::Error::other("could not open an X11 window (is libX11 installed and DISPLAY set?)")
                })?;
                super::app::register_display_fd(x11_win.fd());
                // Same physical size and position `X11Window::create` used.
                let scale = x11_win.scale();
                let inner_size = PhysicalSize::new(
                    (spec.inner_size.width.max(100.0) * scale).round() as u32,
                    (spec.inner_size.height.max(100.0) * scale).round() as u32,
                );
                let position = x11_win.origin();
                // X11 window managers frame a window unless the spec's Motif
                // hints ask them not to.
                (Backend::X11(x11_win), inner_size, position, scale, spec.decorations)
            }
        };
        let raw_handle = Some(match &backend {
            Backend::X11(x11) => x11.raw_handle(),
            Backend::Wayland(wl) => wl.raw_handle(),
        });

        let inner = Arc::new(Mutex::new(NativeWindowInner {
            title: spec.title.to_string(),
            inner_size,
            position,
            scale_factor: scale,
            visible: spec.visible,
            minimized: false,
            maximized: false,
            fullscreen: false,
            server_decorations,
            cursor_visible: true,
            cursor_icon: CursorIcon::Default,
            raw_handle,
            backend: Some(backend),
        }));

        let win = Self { id, inner };
        super::app::register_window(&win);
        Ok(win)
    }

    pub fn id(&self) -> WindowId {
        self.id
    }

    /// A handle that does not keep the window (and its connection) alive.
    pub fn downgrade(&self) -> WeakNativeWindow {
        WeakNativeWindow { id: self.id, inner: Arc::downgrade(&self.inner) }
    }

    pub fn request_redraw(&self) {
        super::app::request_redraw(self.id);
    }

    /// This window's window system connection.
    pub fn backend(&self) -> Option<Backend> {
        self.inner.lock().backend.clone()
    }

    fn x11(&self) -> Option<X11Window> {
        self.inner.lock().backend.as_ref()?.x11().cloned()
    }

    fn wayland(&self) -> Option<WaylandWindow> {
        match self.inner.lock().backend.as_ref()? {
            Backend::Wayland(wl) => Some(wl.clone()),
            Backend::X11(_) => None,
        }
    }

    /// Whether a requested redraw may run now. A Wayland window waits for
    /// its first configure and its last frame's callback, and replays the
    /// redraw itself once they arrive.
    pub fn may_redraw(&self) -> bool {
        self.wayland().is_none_or(|wl| wl.may_redraw())
    }

    /// Called right before a frame is presented (frame callback on Wayland).
    pub fn pre_present_notify(&self) {
        if let Some(wl) = self.wayland() {
            wl.pre_present_notify();
        }
    }

    /// The frame announced by `pre_present_notify` never reached the window.
    pub fn present_skipped(&self) {
        if let Some(wl) = self.wayland() {
            wl.present_skipped();
        }
    }

    /// Keep cached window state in step with an event before the app sees it.
    pub fn observe(&self, event: &WindowEvent) {
        match *event {
            WindowEvent::Resized(size) => self.inner.lock().inner_size = size,
            WindowEvent::ScaleFactorChanged(scale) => {
                let mut guard = self.inner.lock();
                guard.scale_factor = scale;
                // A Wayland scale change resizes the buffer without a
                // `Resized` of its own; the app re-reads the size next.
                if let Some(Backend::Wayland(wl)) = &guard.backend {
                    guard.inner_size = wl.inner_size();
                }
            }
            WindowEvent::StateChanged { maximized, fullscreen } => {
                let mut guard = self.inner.lock();
                guard.maximized = maximized;
                guard.fullscreen = fullscreen;
            }
            WindowEvent::ServerDecorations(server) => {
                let mut guard = self.inner.lock();
                guard.server_decorations = server;
                // Like a scale change, it may resize the window without a
                // `Resized` of its own.
                if let Some(Backend::Wayland(wl)) = &guard.backend {
                    guard.inner_size = wl.inner_size();
                }
            }
            _ => {}
        }
    }

    pub fn scale_factor(&self) -> f64 {
        self.inner.lock().scale_factor
    }

    pub fn inner_size(&self) -> PhysicalSize<u32> {
        self.inner.lock().inner_size
    }

    /// Wayland clients cannot know where their window is.
    pub fn outer_position(&self) -> Result<PhysicalPosition<i32>, Unsupported> {
        let guard = self.inner.lock();
        match guard.backend {
            Some(Backend::Wayland(_)) => Err(Unsupported),
            _ => Ok(guard.position),
        }
    }

    pub fn set_outer_position(&self, position: LogicalPosition<f64>) {
        let scale = self.scale_factor();
        self.set_outer_position_physical(PhysicalPosition::new(
            (position.x * scale).round() as i32,
            (position.y * scale).round() as i32,
        ));
    }

    /// No-op on Wayland: clients cannot place their own windows.
    pub fn set_outer_position_physical(&self, position: PhysicalPosition<i32>) {
        let mut guard = self.inner.lock();
        guard.position = position;
        if let Some(x11) = guard.backend.as_ref().and_then(Backend::x11) {
            x11.move_to(position.x, position.y);
        }
    }

    /// No-op on Wayland: a toplevel cannot be hidden without destroying it
    /// (the app only ever shows its windows).
    pub fn set_visible(&self, visible: bool) {
        let mut guard = self.inner.lock();
        guard.visible = visible;
        if let Some(x11) = guard.backend.as_ref().and_then(Backend::x11) {
            x11.set_visible(visible);
        }
    }

    pub fn is_visible(&self) -> bool {
        self.inner.lock().visible
    }

    /// Activation is subject to the compositor's focus policy on Wayland.
    pub fn focus_window(&self) {
        if let Some(x11) = self.x11() {
            x11.focus();
        } else if let Some(wl) = self.wayland() {
            wl.focus();
        }
    }

    pub fn is_minimized(&self) -> Option<bool> {
        Some(self.inner.lock().minimized)
    }

    pub fn set_minimized(&self, min: bool) {
        let mut guard = self.inner.lock();
        guard.minimized = min;
        if !min {
            return;
        }
        match &guard.backend {
            Some(Backend::X11(x11)) => x11.iconify(),
            Some(Backend::Wayland(wl)) => wl.iconify(),
            None => {}
        }
    }

    pub fn is_maximized(&self) -> bool {
        self.inner.lock().maximized
    }

    pub fn set_maximized(&self, max: bool) {
        let mut guard = self.inner.lock();
        guard.maximized = max;
        match &guard.backend {
            Some(Backend::X11(x11)) => x11.set_maximized(max),
            Some(Backend::Wayland(wl)) => wl.set_maximized(max),
            None => {}
        }
    }

    /// Ask the window manager for fullscreen. No app action calls this
    /// yet. The state is only recorded when the window manager reports it
    /// through `observe`: it may refuse or ignore the request.
    #[allow(dead_code)]
    pub fn set_fullscreen(&self, fullscreen: bool) {
        match self.backend() {
            Some(Backend::X11(x11)) => x11.set_fullscreen(fullscreen),
            Some(Backend::Wayland(wl)) => wl.set_fullscreen(fullscreen),
            None => {}
        }
    }

    pub fn is_fullscreen(&self) -> bool {
        self.inner.lock().fullscreen
    }

    pub fn server_decorations(&self) -> bool {
        self.inner.lock().server_decorations
    }

    /// The monitor under the window's centre (RandR; `None` when the X
    /// server has none), or on Wayland the output the window entered last.
    pub fn current_monitor(&self) -> Option<Monitor> {
        let (backend, size) = {
            let guard = self.inner.lock();
            (guard.backend.clone()?, guard.inner_size)
        };
        match backend {
            Backend::X11(x11) => x11.current_monitor(size),
            Backend::Wayland(wl) => wl.current_monitor(),
        }
    }

    pub fn set_title(&self, title: &str) {
        let mut guard = self.inner.lock();
        guard.title = title.to_string();
        match &guard.backend {
            Some(Backend::X11(x11)) => x11.set_title(title),
            Some(Backend::Wayland(wl)) => wl.set_title(title),
            None => {}
        }
    }

    pub fn set_cursor_icon(&self, icon: CursorIcon) {
        let mut guard = self.inner.lock();
        guard.cursor_icon = icon;
        match &guard.backend {
            Some(Backend::X11(x11)) => x11.set_cursor(icon),
            Some(Backend::Wayland(wl)) => {
                // The Wayland queue lock may wait on a dispatch; don't hold the window meanwhile.
                let (wl, visible) = (wl.clone(), guard.cursor_visible);
                drop(guard);
                wl.set_cursor(icon, visible);
            }
            None => {}
        }
    }

    pub fn set_cursor_visible(&self, visible: bool) {
        let mut guard = self.inner.lock();
        guard.cursor_visible = visible;
        match &guard.backend {
            Some(Backend::X11(x11)) => x11.set_cursor_visible(visible),
            Some(Backend::Wayland(wl)) => {
                let (wl, icon) = (wl.clone(), guard.cursor_icon);
                drop(guard);
                wl.set_cursor(icon, visible);
            }
            None => {}
        }
    }

    /// The desktop's light/dark preference (settings portal).
    pub fn theme(&self) -> Option<Theme> {
        super::portal::system_theme()
    }

    /// Outside the window lock: reaching the input method may reconnect
    /// it, a round trip to its server.
    pub fn set_ime_allowed(&self, allowed: bool) {
        if let Some(x11) = self.x11() {
            x11.set_ime_allowed(allowed);
        } else if let Some(wl) = self.wayland() {
            wl.set_ime_allowed(allowed);
        }
    }

    /// Position the input method candidate window at the terminal cursor.
    pub fn set_ime_cursor_area(&self, position: PhysicalPosition<f64>, size: PhysicalSize<f64>) {
        if let Some(wl) = self.wayland() {
            wl.set_ime_cursor_area(position, size);
        } else if let Some(x11) = self.x11() {
            x11.set_ime_cursor_area(position, size);
        }
    }

    /// Best-effort attention request; Wayland cannot revoke compositor urgency.
    pub fn request_attention(&self, requested: bool) {
        if let Some(x11) = self.x11() {
            x11.request_attention(requested);
        } else if let Some(wl) = self.wayland() {
            wl.request_attention(requested);
        }
    }

    /// Hand the window to the window manager to move with the pointer, as
    /// the left button is down on Litty's own title bar.
    pub fn drag_window(&self) -> Result<(), Unsupported> {
        match self.backend().ok_or(Unsupported)? {
            Backend::X11(x11) => {
                x11.begin_move_resize(8); // _NET_WM_MOVERESIZE_MOVE
                Ok(())
            }
            Backend::Wayland(wl) => wl.drag_window(),
        }
    }

    /// Hand the window to the window manager to resize from `direction`'s
    /// edge, as the left button is down on Litty's own resize border.
    pub fn drag_resize_window(&self, direction: ResizeDirection) -> Result<(), Unsupported> {
        match self.backend().ok_or(Unsupported)? {
            Backend::X11(x11) => {
                x11.begin_move_resize(moveresize_direction(direction));
                Ok(())
            }
            Backend::Wayland(wl) => wl.drag_resize_window(direction),
        }
    }

    pub fn raw_handle(&self) -> Option<RawHandle> {
        self.inner.lock().raw_handle
    }

    #[cfg(test)]
    pub(crate) fn set_scale_factor_for_test(&self, scale: f64) {
        self.inner.lock().scale_factor = scale;
    }

    #[allow(dead_code)]
    pub fn set_raw_handle(&self, handle: RawHandle) {
        self.inner.lock().raw_handle = Some(handle);
    }
}

/// Registry handle for the event loop: the window closes (an X11 window's
/// connection with it) once the app drops its last `NativeWindow`.
pub struct WeakNativeWindow {
    id: WindowId,
    inner: Weak<Mutex<NativeWindowInner>>,
}

impl WeakNativeWindow {
    pub fn upgrade(&self) -> Option<NativeWindow> {
        Some(NativeWindow { id: self.id, inner: self.inner.upgrade()? })
    }
}

/// `_NET_WM_MOVERESIZE` edge codes.
fn moveresize_direction(direction: ResizeDirection) -> std::ffi::c_long {
    match direction {
        ResizeDirection::NorthWest => 0,
        ResizeDirection::North => 1,
        ResizeDirection::NorthEast => 2,
        ResizeDirection::East => 3,
        ResizeDirection::SouthEast => 4,
        ResizeDirection::South => 5,
        ResizeDirection::SouthWest => 6,
        ResizeDirection::West => 7,
    }
}

/// A window with no window system connection, for tests of the loop's bookkeeping.
#[cfg(test)]
pub(crate) fn detached(width: u32, height: u32) -> NativeWindow {
    NativeWindow {
        id: allocate_window_id(),
        inner: Arc::new(Mutex::new(NativeWindowInner {
            title: String::new(),
            inner_size: PhysicalSize::new(width, height),
            position: PhysicalPosition::new(0, 0),
            scale_factor: 1.0,
            visible: true,
            minimized: false,
            maximized: false,
            fullscreen: false,
            server_decorations: false,
            cursor_visible: true,
            cursor_icon: CursorIcon::Default,
            raw_handle: None,
            backend: None,
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::gui::WindowEvent;

    /// The app re-reads `inner_size()` on focus changes; a stale size makes
    /// the renderer draw the old layout over the resized swapchain.
    #[test]
    fn resized_event_updates_inner_size() {
        let win = detached(968, 620);
        win.observe(&WindowEvent::Resized(PhysicalSize::new(1920, 1080)));
        assert_eq!(win.inner_size(), PhysicalSize::new(1920, 1080));
    }

    /// The window manager restoring a window the app maximized (a title-bar
    /// drag on GNOME) must clear the flag, or the resize edges stay off.
    #[test]
    fn wm_state_event_overrides_the_requested_state() {
        let win = detached(800, 600);
        win.set_maximized(true);
        assert!(win.is_maximized());
        win.observe(&WindowEvent::StateChanged { maximized: false, fullscreen: false });
        assert!(!win.is_maximized());
        win.observe(&WindowEvent::StateChanged { maximized: true, fullscreen: true });
        assert!(win.is_maximized());
        assert!(win.is_fullscreen());
    }

    /// Fullscreen is the window manager's to grant: a refused or ignored
    /// request (tiling WM, unmapped window) must not report fullscreen, or
    /// the next toggle asks to leave a state the window never entered.
    #[test]
    fn requesting_fullscreen_waits_for_the_window_manager() {
        let win = detached(800, 600);
        win.set_fullscreen(true);
        assert!(!win.is_fullscreen());
        win.observe(&WindowEvent::StateChanged { maximized: false, fullscreen: true });
        assert!(win.is_fullscreen());
    }

    /// A compositor that takes the frame over (sway, for tiled windows)
    /// must reach the cached flag the app reads when it lays out.
    #[test]
    fn decoration_event_updates_the_flag() {
        let win = detached(800, 600);
        assert!(!win.server_decorations());
        win.observe(&WindowEvent::ServerDecorations(true));
        assert!(win.server_decorations());
        win.observe(&WindowEvent::ServerDecorations(false));
        assert!(!win.server_decorations());
    }

    /// A Wayland output scale change reaches the cached scale before the
    /// app reads it to size its font.
    #[test]
    fn scale_event_updates_the_scale_factor() {
        let win = detached(1000, 750);
        win.observe(&WindowEvent::ScaleFactorChanged(1.25));
        assert_eq!(win.scale_factor(), 1.25);
    }

    /// Cascade positions are physical pixels already; scaling them again
    /// placed new windows twice as far away on a HiDPI screen.
    #[test]
    fn physical_positions_are_not_scaled_again() {
        let win = detached(800, 600);
        win.set_scale_factor_for_test(2.0);
        win.set_outer_position_physical(PhysicalPosition::new(1920, 40));
        assert_eq!(win.outer_position(), Ok(PhysicalPosition::new(1920, 40)));
        win.set_outer_position(LogicalPosition::new(100.0, 50.0));
        assert_eq!(win.outer_position(), Ok(PhysicalPosition::new(200, 100)));
    }
}
