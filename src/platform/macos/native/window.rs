//! Native AppKit window & view types for macOS (replaces winit on macOS).

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::platform::macos::objc::*;
use crate::platform::gui::{
    CursorIcon, LogicalPosition, Monitor, PhysicalPosition, PhysicalSize,
    RawHandle, ResizeDirection, Theme, Unsupported, WindowId, WindowSpec,
};
use super::view::ensure_litty_view_class;

static NEXT_WINDOW_ID: AtomicU64 = AtomicU64::new(1);

/// Native macOS Window wrapping `NSWindow` and our custom `LittyView`.
#[derive(Clone)]
pub struct NativeWindow {
    id: WindowId,
    inner: Arc<NativeWindowInner>,
}

pub struct NativeWindowInner {
    pub ns_window: Id,
    pub ns_view: Id,
}

unsafe impl Send for NativeWindowInner {}
unsafe impl Sync for NativeWindowInner {}

impl NativeWindow {
    pub fn new(id: WindowId, ns_window: Id, ns_view: Id) -> Self {
        Self {
            id,
            inner: Arc::new(NativeWindowInner { ns_window, ns_view }),
        }
    }

    /// Create an NSWindow and embed our custom LittyView.
    pub fn create(spec: &WindowSpec) -> Result<Self, std::io::Error> {
        ensure_litty_view_class();

        let id = allocate_window_id();

        unsafe {
            let width = spec.inner_size.width;
            let height = spec.inner_size.height;
            let rect = NSRect { x: 100.0, y: 100.0, w: width, h: height };

            // NSWindowStyleMask: Titled (1) | Closable (2) | Miniaturizable (4) | Resizable (8) | FullSizeContentView (32768)
            let style_mask: usize = 1 | 2 | 4 | 8 | (1 << 15);

            let win_cls = class(c"NSWindow").ok_or_else(|| std::io::Error::other("no NSWindow class"))?;
            let ns_window = msg_send!(
                win_cls,
                c"alloc" ; -> Id
            );
            let ns_window = msg_send!(
                ns_window,
                c"initWithContentRect:styleMask:backing:defer:",
                rect => NSRect,
                style_mask => usize,
                2usize => usize, // NSBackingStoreBuffered
                false => bool
                ; -> Id
            );

            if ns_window.is_null() {
                return Err(std::io::Error::other("failed to create NSWindow"));
            }

            // Create LittyView
            let view_cls = class(c"LittyView").ok_or_else(|| std::io::Error::other("no LittyView class"))?;
            let ns_view = msg_send!(view_cls, c"alloc" ; -> Id);
            let ns_view = msg_send!(ns_view, c"initWithFrame:", rect => NSRect ; -> Id);

            // Tag view with WindowId so delegate callbacks know which window fired
            let tag: usize = id.raw() as usize;
            msg_send!(ns_view, c"setTag:", tag => usize);

            // Make view layer-backed
            msg_send!(ns_view, c"setWantsLayer:", true => bool);

            // Set content view
            msg_send!(ns_window, c"setContentView:", ns_view => Id);

            // Title
            if let Some(ns_title) = ns_string(spec.title) {
                msg_send!(ns_window, c"setTitle:", ns_title => Id);
            }

            // Window configuration
            msg_send!(ns_window, c"setTitlebarAppearsTransparent:", true => bool);
            msg_send!(ns_window, c"setAcceptsMouseMovedEvents:", true => bool);

            if spec.visible {
                msg_send!(ns_window, c"makeKeyAndOrderFront:", std::ptr::null_mut::<c_void>() => Id);
            }

            Ok(NativeWindow::new(id, ns_window, ns_view))
        }
    }

    pub fn id(&self) -> WindowId {
        self.id
    }

    #[allow(dead_code)]
    pub fn ns_window(&self) -> Id {
        self.inner.ns_window
    }

    #[allow(dead_code)]
    pub fn ns_view(&self) -> Id {
        self.inner.ns_view
    }

    pub fn request_redraw(&self) {
        super::app::request_redraw(self.id());
        unsafe {
            msg_send!(self.inner.ns_view, c"setNeedsDisplay:", true => bool);
        }
    }

    pub fn scale_factor(&self) -> f64 {
        unsafe {
            msg_send!(self.inner.ns_window, c"backingScaleFactor"; -> f64)
        }
    }

    pub fn inner_size(&self) -> PhysicalSize<u32> {
        unsafe {
            let frame = msg_send!(self.inner.ns_view, c"frame"; -> NSRect);
            let scale = self.scale_factor();
            PhysicalSize::new((frame.w * scale).round() as u32, (frame.h * scale).round() as u32)
        }
    }

    pub fn outer_position(&self) -> Result<PhysicalPosition<i32>, Unsupported> {
        unsafe {
            let frame = msg_send!(self.inner.ns_window, c"frame"; -> NSRect);
            let scale = self.scale_factor();
            Ok(PhysicalPosition::new((frame.x * scale).round() as i32, (frame.y * scale).round() as i32))
        }
    }

    pub fn set_outer_position(&self, position: LogicalPosition<f64>) {
        unsafe {
            let mut frame = msg_send!(self.inner.ns_window, c"frame"; -> NSRect);
            frame.x = position.x;
            frame.y = position.y;
            msg_send!(self.inner.ns_window, c"setFrameOrigin:", NSPoint { x: frame.x, y: frame.y } => NSPoint);
        }
    }

    pub fn is_minimized(&self) -> Option<bool> {
        unsafe {
            let is_mini = msg_send!(self.inner.ns_window, c"isMiniaturized"; -> bool);
            Some(is_mini)
        }
    }

    pub fn set_minimized(&self, minimized: bool) {
        unsafe {
            if minimized {
                msg_send!(self.inner.ns_window, c"miniaturize:", std::ptr::null_mut::<c_void>() => Id);
            } else {
                msg_send!(self.inner.ns_window, c"deminiaturize:", std::ptr::null_mut::<c_void>() => Id);
            }
        }
    }

    pub fn is_maximized(&self) -> bool {
        unsafe {
            msg_send!(self.inner.ns_window, c"isZoomed"; -> bool)
        }
    }

    pub fn set_maximized(&self, _maximized: bool) {
        unsafe {
            msg_send!(self.inner.ns_window, c"zoom:", std::ptr::null_mut::<c_void>() => Id);
        }
    }

    pub fn is_fullscreen(&self) -> bool {
        unsafe {
            let style = msg_send!(self.inner.ns_window, c"styleMask"; -> usize);
            (style & (1 << 14)) != 0 // NSWindowStyleMaskFullScreen
        }
    }

    pub fn set_title(&self, title: &str) {
        unsafe {
            if let Some(ns_title) = ns_string(title) {
                msg_send!(self.inner.ns_window, c"setTitle:", ns_title => Id);
            }
        }
    }

    pub fn focus_window(&self) {
        unsafe {
            msg_send!(self.inner.ns_window, c"makeKeyAndOrderFront:", std::ptr::null_mut::<c_void>() => Id);
        }
    }

    pub fn set_cursor(&self, icon: CursorIcon) {
        unsafe {
            let sel_name = match icon {
                CursorIcon::Pointer => c"pointingHandCursor",
                CursorIcon::Text => c"IBeamCursor",
                CursorIcon::Crosshair => c"crosshairCursor",
                CursorIcon::ColResize | CursorIcon::EwResize => c"resizeLeftRightCursor",
                CursorIcon::RowResize | CursorIcon::NsResize => c"resizeUpDownCursor",
                _ => c"arrowCursor",
            };
            if let Some(cursor_cls) = class(c"NSCursor")
                && let Some(cursor) = send_id(cursor_cls, sel_name) {
                msg_send!(cursor, c"set");
            }
        }
    }

    pub fn set_cursor_visible(&self, visible: bool) {
        unsafe {
            if let Some(cursor_cls) = class(c"NSCursor") {
                let sel = if visible { c"unhide" } else { c"hide" };
                msg_send!(cursor_cls, sel);
            }
        }
    }

    pub fn theme(&self) -> Option<Theme> {
        Some(if crate::platform::system_dark() { Theme::Dark } else { Theme::Light })
    }

    pub fn set_ime_allowed(&self, _allowed: bool) {}

    pub fn set_ime_cursor_area(&self, _position: PhysicalPosition<f64>, _size: PhysicalSize<f64>) {}

    pub fn request_attention(&self, requested: bool) {
        unsafe {
            if let Some(app) = shared_application()
                && requested {
                msg_send!(app, c"requestUserAttention:", 0usize => usize); // NSInformationalRequest
            }
        }
    }

    pub fn drag_window(&self) -> Result<(), Unsupported> {
        unsafe {
            let event = msg_send!(self.inner.ns_window, c"currentEvent"; -> Id);
            if !event.is_null() {
                msg_send!(self.inner.ns_window, c"performWindowDragWithEvent:", event => Id);
                return Ok(());
            }
        }
        Err(Unsupported)
    }

    pub fn drag_resize_window(&self, _direction: ResizeDirection) -> Result<(), Unsupported> {
        Err(Unsupported)
    }

    pub fn current_monitor(&self) -> Option<Monitor> {
        unsafe {
            let screen = send_id(self.inner.ns_window, c"screen")?;
            let frame = msg_send!(screen, c"frame"; -> NSRect);
            let scale = self.scale_factor();
            Some(Monitor {
                position: PhysicalPosition::new((frame.x * scale) as i32, (frame.y * scale) as i32),
                size: PhysicalSize::new((frame.w * scale) as u32, (frame.h * scale) as u32),
                refresh_rate_millihertz: None,
            })
        }
    }

    pub fn raw_handle(&self) -> Option<RawHandle> {
        Some(RawHandle::AppKit { ns_view: self.inner.ns_view })
    }
}

pub fn allocate_window_id() -> WindowId {
    let val = NEXT_WINDOW_ID.fetch_add(1, Ordering::Relaxed);
    unsafe { std::mem::transmute(val) }
}
