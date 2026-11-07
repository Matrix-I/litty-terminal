//! Native Linux Window representation for Wayland and X11.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use parking_lot::Mutex;

use crate::platform::gui::{
    CursorIcon, LogicalPosition, Monitor, PhysicalPosition, PhysicalSize,
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
        let scale = 1.0;
        let inner_size = PhysicalSize::new(
            (spec.inner_size.width * scale).round() as u32,
            (spec.inner_size.height * scale).round() as u32,
        );
        let position = spec.position.map_or(PhysicalPosition::new(100, 100), |p| {
            PhysicalPosition::new(p.x.round() as i32, p.y.round() as i32)
        });

        // Try creating native X11 window connection
        let x11_win = X11Window::create(spec);
        let raw_handle = x11_win.as_ref().map(|x| x.raw_handle());
        if let Some(ref x) = x11_win {
            super::app::register_display_fd(x.fd());
        }

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
        super::app::register_window(win.clone());
        Ok(win)
    }

    pub fn id(&self) -> WindowId {
        self.id
    }

    pub fn request_redraw(&self) {
        super::app::request_redraw(self.id);
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
        let mut guard = self.inner.lock();
        guard.position = PhysicalPosition::new(position.x.round() as i32, position.y.round() as i32);
    }

    pub fn set_visible(&self, visible: bool) {
        self.inner.lock().visible = visible;
    }

    pub fn is_visible(&self) -> bool {
        self.inner.lock().visible
    }

    pub fn focus_window(&self) {}

    pub fn is_minimized(&self) -> Option<bool> {
        Some(self.inner.lock().minimized)
    }

    pub fn set_minimized(&self, min: bool) {
        self.inner.lock().minimized = min;
    }

    pub fn is_maximized(&self) -> bool {
        self.inner.lock().maximized
    }

    pub fn set_maximized(&self, max: bool) {
        self.inner.lock().maximized = max;
    }

    #[allow(dead_code)]
    pub fn set_fullscreen(&self, fullscreen: bool) {
        self.inner.lock().fullscreen = fullscreen;
    }

    pub fn is_fullscreen(&self) -> bool {
        self.inner.lock().fullscreen
    }

    pub fn current_monitor(&self) -> Option<Monitor> {
        let guard = self.inner.lock();
        Some(Monitor {
            position: guard.position,
            size: PhysicalSize::new(1920, 1080),
            refresh_rate_millihertz: Some(60000),
        })
    }

    pub fn set_title(&self, title: &str) {
        let mut guard = self.inner.lock();
        guard.title = title.to_string();
        if let Some(ref x11) = guard.x11_window {
            x11.set_title(title);
        }
    }

    pub fn set_cursor_icon(&self, icon: CursorIcon) {
        self.inner.lock().cursor_icon = icon;
    }

    pub fn set_cursor_visible(&self, visible: bool) {
        self.inner.lock().cursor_visible = visible;
    }

    pub fn theme(&self) -> Option<Theme> {
        Some(Theme::Dark)
    }

    pub fn set_ime_allowed(&self, _allowed: bool) {}

    pub fn set_ime_cursor_area(&self, _position: PhysicalPosition<f64>, _size: PhysicalSize<f64>) {}

    pub fn request_attention(&self, _requested: bool) {}

    pub fn drag_window(&self) -> Result<(), Unsupported> {
        Ok(())
    }

    pub fn drag_resize_window(&self, _direction: ResizeDirection) -> Result<(), Unsupported> {
        Ok(())
    }

    pub fn raw_handle(&self) -> Option<RawHandle> {
        self.inner.lock().raw_handle
    }

    #[allow(dead_code)]
    pub fn set_raw_handle(&self, handle: RawHandle) {
        self.inner.lock().raw_handle = Some(handle);
    }
}
