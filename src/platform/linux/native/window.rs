//! Native Linux Window representation for Wayland and X11.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use parking_lot::Mutex;

use crate::platform::gui::{
    CursorIcon, LogicalPosition, WindowEvent, Monitor, PhysicalPosition, PhysicalSize,
    RawHandle, ResizeDirection, Theme, Unsupported, WindowId, WindowSpec,
};
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
    pub cursor_visible: bool,
    pub cursor_icon: CursorIcon,
    pub raw_handle: Option<RawHandle>,
    pub x11_window: Option<X11Window>,
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

        // Without an X11 window there is nothing to draw into: fail loudly
        // instead of running a windowless process.
        let x11_win = X11Window::create(spec).ok_or_else(|| {
            std::io::Error::other("could not open an X11 window (is libX11 installed and DISPLAY set?)")
        })?;
        let raw_handle = Some(x11_win.raw_handle());
        super::app::register_display_fd(x11_win.fd());

        // Same physical size and position `X11Window::create` used.
        let scale = x11_win.scale();
        let inner_size = PhysicalSize::new(
            (spec.inner_size.width.max(100.0) * scale).round() as u32,
            (spec.inner_size.height.max(100.0) * scale).round() as u32,
        );
        let position = spec.position.map_or(PhysicalPosition::new(100, 100), |p| {
            PhysicalPosition::new((p.x * scale).round() as i32, (p.y * scale).round() as i32)
        });
        let x11_win = Some(x11_win);

        let inner = Arc::new(Mutex::new(NativeWindowInner {
            title: spec.title.to_string(),
            inner_size,
            position,
            scale_factor: scale,
            visible: spec.visible,
            minimized: false,
            maximized: false,
            fullscreen: false,
            cursor_visible: true,
            cursor_icon: CursorIcon::Default,
            raw_handle,
            x11_window: x11_win,
        }));

        let win = Self { id, inner };
        super::app::register_window(&win);
        Ok(win)
    }

    pub fn id(&self) -> WindowId {
        self.id
    }

    /// A handle that does not keep the window (and its X11 connection) alive.
    pub fn downgrade(&self) -> WeakNativeWindow {
        WeakNativeWindow { id: self.id, inner: Arc::downgrade(&self.inner) }
    }

    pub fn request_redraw(&self) {
        super::app::request_redraw(self.id);
    }

    /// This window's X11 connection.
    pub fn x11(&self) -> Option<X11Window> {
        self.inner.lock().x11_window.clone()
    }

    /// Keep cached window state in step with an event before the app sees it.
    pub fn observe(&self, event: &WindowEvent) {
        match *event {
            WindowEvent::Resized(size) => self.inner.lock().inner_size = size,
            WindowEvent::StateChanged { maximized, fullscreen } => {
                let mut guard = self.inner.lock();
                guard.maximized = maximized;
                guard.fullscreen = fullscreen;
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

    pub fn outer_position(&self) -> Result<PhysicalPosition<i32>, Unsupported> {
        Ok(self.inner.lock().position)
    }

    pub fn set_outer_position(&self, position: LogicalPosition<f64>) {
        let scale = self.scale_factor();
        self.set_outer_position_physical(PhysicalPosition::new(
            (position.x * scale).round() as i32,
            (position.y * scale).round() as i32,
        ));
    }

    pub fn set_outer_position_physical(&self, position: PhysicalPosition<i32>) {
        let mut guard = self.inner.lock();
        guard.position = position;
        if let Some(x11) = &guard.x11_window {
            x11.move_to(position.x, position.y);
        }
    }

    pub fn set_visible(&self, visible: bool) {
        let mut guard = self.inner.lock();
        guard.visible = visible;
        if let Some(x11) = &guard.x11_window {
            x11.set_visible(visible);
        }
    }

    pub fn is_visible(&self) -> bool {
        self.inner.lock().visible
    }

    pub fn focus_window(&self) {
        if let Some(x11) = &self.inner.lock().x11_window {
            x11.focus();
        }
    }

    pub fn is_minimized(&self) -> Option<bool> {
        Some(self.inner.lock().minimized)
    }

    pub fn set_minimized(&self, min: bool) {
        let mut guard = self.inner.lock();
        guard.minimized = min;
        if min && let Some(x11) = &guard.x11_window {
            x11.iconify();
        }
    }

    pub fn is_maximized(&self) -> bool {
        self.inner.lock().maximized
    }

    pub fn set_maximized(&self, max: bool) {
        let mut guard = self.inner.lock();
        guard.maximized = max;
        if let Some(x11) = &guard.x11_window {
            x11.set_maximized(max);
        }
    }

    /// No app action calls this yet: fullscreen is entered from the window
    /// manager, and its state arrives through `observe`.
    #[allow(dead_code)]
    pub fn set_fullscreen(&self, fullscreen: bool) {
        let mut guard = self.inner.lock();
        guard.fullscreen = fullscreen;
        if let Some(x11) = &guard.x11_window {
            x11.set_fullscreen(fullscreen);
        }
    }

    pub fn is_fullscreen(&self) -> bool {
        self.inner.lock().fullscreen
    }

    /// The monitor under the window's centre (RandR); `None` when the X
    /// server has no RandR or the window has no X11 connection.
    pub fn current_monitor(&self) -> Option<Monitor> {
        let (x11, size) = {
            let guard = self.inner.lock();
            (guard.x11_window.clone()?, guard.inner_size)
        };
        x11.current_monitor(size)
    }

    pub fn set_title(&self, title: &str) {
        let mut guard = self.inner.lock();
        guard.title = title.to_string();
        if let Some(ref x11) = guard.x11_window {
            x11.set_title(title);
        }
    }

    pub fn set_cursor_icon(&self, icon: CursorIcon) {
        let mut guard = self.inner.lock();
        guard.cursor_icon = icon;
        if let Some(x11) = &guard.x11_window {
            x11.set_cursor(icon);
        }
    }

    pub fn set_cursor_visible(&self, visible: bool) {
        let mut guard = self.inner.lock();
        guard.cursor_visible = visible;
        if let Some(x11) = &guard.x11_window {
            x11.set_cursor_visible(visible);
        }
    }

    /// The desktop's light/dark preference (settings portal).
    pub fn theme(&self) -> Option<Theme> {
        super::portal::system_theme()
    }

    pub fn set_ime_allowed(&self, allowed: bool) {
        if let Some(x11) = &self.inner.lock().x11_window {
            x11.set_ime_allowed(allowed);
        }
    }

    pub fn set_ime_cursor_area(&self, position: PhysicalPosition<f64>, size: PhysicalSize<f64>) {
        if let Some(x11) = &self.inner.lock().x11_window {
            x11.set_ime_cursor_area(position, size);
        }
    }

    pub fn request_attention(&self, requested: bool) {
        if let Some(x11) = &self.inner.lock().x11_window {
            x11.request_attention(requested);
        }
    }

    pub fn drag_window(&self) -> Result<(), Unsupported> {
        let guard = self.inner.lock();
        let x11 = guard.x11_window.as_ref().ok_or(Unsupported)?;
        x11.begin_move_resize(8); // _NET_WM_MOVERESIZE_MOVE
        Ok(())
    }

    pub fn drag_resize_window(&self, direction: ResizeDirection) -> Result<(), Unsupported> {
        let guard = self.inner.lock();
        let x11 = guard.x11_window.as_ref().ok_or(Unsupported)?;
        x11.begin_move_resize(moveresize_direction(direction));
        Ok(())
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

/// Registry handle for the event loop: the window closes (and its X11
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

/// A window with no X11 connection, for tests of the loop's bookkeeping.
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
            cursor_visible: true,
            cursor_icon: CursorIcon::Default,
            raw_handle: None,
            x11_window: None,
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
