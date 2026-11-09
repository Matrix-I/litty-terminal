//! Swapchain surface backed directly by a `CAMetalLayer` attached to the window's `NSView`.

use crate::platform::gui::{RawHandle, Window};

use super::device::{Device, Queue};
use super::ffi::*;
use super::pass::{CommandEncoder, InFlight};
use crate::render::backend::{SurfaceConfig, TextureFormat};

pub struct Surface {
    ns_view: Id,
    layer: ObjPtr,
    in_flight: InFlight,
}

// SAFETY: `Surface` is accessed on the main event-loop thread via `&mut Renderer`.
unsafe impl Send for Surface {}
unsafe impl Sync for Surface {}

pub struct SurfaceFrame {
    pub(super) drawable: ObjPtr,
    pub(super) texture: ObjPtr,
    pub(super) in_flight: InFlight,
}

impl SurfaceFrame {
    #[inline]
    pub fn texture_ptr(&self) -> Id {
        self.texture.as_ptr()
    }
}

impl Surface {
    pub fn new(window: &Window, device: &Device) -> Self {
        let Some(RawHandle::AppKit { ns_view }) = window.raw_handle() else {
            panic!("expected AppKit window handle on macOS");
        };
        let ns_view = ns_view as Id;
        unsafe {
            let cls = class(c"CAMetalLayer").expect("CAMetalLayer");
            let layer = msg_send!(cls, c"new"; -> Id);
            assert!(!layer.is_null(), "CAMetalLayer allocation failed");

            msg_send!(layer, c"setDevice:", device.raw.as_ptr() => Id);
            msg_send!(layer, c"setPixelFormat:", MTL_PIXEL_FORMAT_BGRA8_UNORM => usize);
            msg_send!(layer, c"setFramebufferOnly:", true => bool);
            msg_send!(layer, c"setMaximumDrawableCount:", 2usize => usize);
            msg_send!(layer, c"setAllowsNextDrawableTimeout:", true => bool);
            msg_send!(layer, c"setPresentsWithTransaction:", false => bool);

            let scale = window.scale_factor();
            msg_send!(layer, c"setContentsScale:", scale => f64);

            msg_send!(ns_view, c"setWantsLayer:", true => bool);
            msg_send!(ns_view, c"setLayer:", layer => Id);

            Self {
                ns_view,
                layer: ObjPtr(layer),
                in_flight: InFlight::default(),
            }
        }
    }

    #[inline]
    pub fn default_format(&self) -> TextureFormat {
        TextureFormat::Bgra8Unorm
    }

    pub fn configure(&mut self, _device: &Device, config: &SurfaceConfig) {
        unsafe {
            let layer = self.layer.as_ptr();
            let current_layer = msg_send!(self.ns_view, c"layer"; -> Id);
            let superlayer = msg_send!(layer, c"superlayer"; -> Id);
            if current_layer != layer || superlayer.is_null() {
                msg_send!(self.ns_view, c"setLayer:", std::ptr::null_mut::<std::ffi::c_void>() => Id);
                msg_send!(self.ns_view, c"setWantsLayer:", true => bool);
                msg_send!(self.ns_view, c"setLayer:", layer => Id);
            }
            msg_send!(layer, c"setPixelFormat:", mtl_pixel_format(config.format) => usize);
            msg_send!(layer, c"setOpaque:", !config.transparent => bool);
            let size = CGSize {
                width: config.width.max(1) as f64,
                height: config.height.max(1) as f64,
            };
            msg_send!(layer, c"setDrawableSize:", size => CGSize);
        }
    }

    pub fn acquire_frame(&mut self) -> Option<SurfaceFrame> {
        unsafe {
            let layer = self.layer.as_ptr();
            let current_layer = msg_send!(self.ns_view, c"layer"; -> Id);
            let superlayer = msg_send!(layer, c"superlayer"; -> Id);
            if current_layer != layer || superlayer.is_null() {
                msg_send!(self.ns_view, c"setLayer:", std::ptr::null_mut::<std::ffi::c_void>() => Id);
                msg_send!(self.ns_view, c"setWantsLayer:", true => bool);
                msg_send!(self.ns_view, c"setLayer:", layer => Id);
            }
            let drawable = msg_send!(layer, c"nextDrawable"; -> Id);
            if drawable.is_null() {
                return None;
            }
            msg_send!(drawable, c"retain"; -> Id);
            let tex = msg_send!(drawable, c"texture"; -> Id);
            if tex.is_null() {
                msg_send!(drawable, c"release");
                return None;
            }
            msg_send!(tex, c"retain"; -> Id);
            // The caller writes instance buffers next: the previous frame's
            // GPU work must have finished reading them (TD-5). Waiting after
            // `nextDrawable` overlaps this with the vsync wait.
            self.in_flight.wait();
            Some(SurfaceFrame {
                drawable: ObjPtr(drawable),
                texture: ObjPtr(tex),
                in_flight: self.in_flight.clone(),
            })
        }
    }

    /// Block until the GPU has finished the last submitted frame; `false` if it
    /// failed. Benchmarks only: the render loop itself never waits here.
    pub fn wait_gpu_idle(&self, _timeout: std::time::Duration) -> bool {
        // Metal ends every command buffer (completed or failed), so no timeout is needed.
        self.in_flight.wait_completed()
    }

    /// Shrink swapchain drawables to 1×1 and flush the pool when idle/hidden/closed
    /// so macOS releases the ~30MB Retina IOSurface backing buffers immediately.
    pub fn shrink_drawables(&mut self, device: &Device, queue: &Queue) {
        let cfg = SurfaceConfig {
            format: TextureFormat::Bgra8Unorm,
            width: 1,
            height: 1,
            transparent: true,
        };
        self.configure(device, &cfg);
        for _ in 0..2 {
            let Some(frame) = self.acquire_frame() else { break };
            let mut cmd = CommandEncoder::new(queue);
            {
                let _pass = cmd.begin_render_pass(frame.texture_ptr(), Some([0.0, 0.0, 0.0, 0.0]));
            }
            cmd.present_and_submit_wait(frame);
        }
    }
}

impl Drop for Surface {
    /// Detach the layer so it dies with this surface: AppKit keeps a closed
    /// `NSWindow`, and through it this view, alive (as `NSApp`'s previous key
    /// window), and the view's layer would pin its last full-size drawable.
    fn drop(&mut self) {
        unsafe {
            if msg_send!(self.ns_view, c"layer"; -> Id) == self.layer.as_ptr() {
                msg_send!(self.ns_view, c"setLayer:", std::ptr::null_mut::<std::ffi::c_void>() => Id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[repr(C)]
    struct NSRect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    }

    /// A closed window's view outlives its surface (AppKit keeps the window
    /// alive), so the surface must take its layer, and the layer's last
    /// full-size drawable, with it.
    #[test]
    fn dropping_the_surface_detaches_its_layer_from_the_view() {
        unsafe {
            let rect = NSRect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 };
            let view = msg_send!(class(c"NSView").unwrap(), c"alloc"; -> Id);
            let view = msg_send!(view, c"initWithFrame:", rect => NSRect; -> Id);
            let layer = msg_send!(class(c"CAMetalLayer").unwrap(), c"new"; -> Id);
            msg_send!(view, c"setWantsLayer:", true => bool);
            msg_send!(view, c"setLayer:", layer => Id);
            let surface = Surface { ns_view: view, layer: ObjPtr(layer), in_flight: InFlight::default() };

            drop(surface);

            assert!(msg_send!(view, c"layer"; -> Id).is_null(), "the view no longer holds the layer");
            msg_send!(view, c"release");
        }
    }
}
