//! `platform::gui::Window` as the `render` layer's window.

use crate::platform::gui::{RawHandle as PlatformHandle, Window};
use crate::render::window::{RawHandle, RenderWindow};

/// The same handle, as the `render` layer's type.
pub(crate) fn render_handle(handle: PlatformHandle) -> RawHandle {
    match handle {
        PlatformHandle::AppKit { ns_view } => RawHandle::AppKit { ns_view },
        PlatformHandle::Wayland { display, surface } => RawHandle::Wayland { display, surface },
        PlatformHandle::Xlib { display, window } => RawHandle::Xlib { display, window },
        PlatformHandle::Xcb { connection, window } => RawHandle::Xcb { connection, window },
    }
}

impl RenderWindow for Window {
    fn raw_handle(&self) -> Option<RawHandle> {
        Window::raw_handle(self).map(render_handle)
    }

    fn scale_factor(&self) -> f64 {
        Window::scale_factor(self)
    }

    fn inner_size(&self) -> (u32, u32) {
        let size = Window::inner_size(self);
        (size.width, size.height)
    }

    fn pre_present_notify(&self) {
        Window::pre_present_notify(self);
    }

    fn present_skipped(&self) {
        Window::present_skipped(self);
    }
}

#[cfg(test)]
mod tests {
    use super::render_handle;
    use crate::platform::gui::RawHandle as P;
    use crate::render::window::RawHandle as R;

    fn ptr(n: usize) -> *mut std::ffi::c_void {
        n as *mut std::ffi::c_void
    }

    #[test]
    fn render_handle_keeps_every_variant_and_field() {
        assert_eq!(render_handle(P::AppKit { ns_view: ptr(1) }), R::AppKit { ns_view: ptr(1) });
        assert_eq!(
            render_handle(P::Wayland { display: ptr(2), surface: ptr(3) }),
            R::Wayland { display: ptr(2), surface: ptr(3) },
        );
        assert_eq!(render_handle(P::Xlib { display: ptr(4), window: 5 }), R::Xlib { display: ptr(4), window: 5 });
        assert_eq!(render_handle(P::Xcb { connection: ptr(6), window: 7 }), R::Xcb { connection: ptr(6), window: 7 });
    }
}
