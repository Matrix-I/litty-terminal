//! Swapchain surface backed directly by a `CAMetalLayer` attached to the window's `NSView`.

use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

use super::device::{Device, Queue};
use super::ffi::*;
use super::pass::CommandEncoder;
use crate::render::backend::{SurfaceConfig, TextureFormat};

pub struct Surface {
    ns_view: Id,
    layer: ObjPtr,
}

// SAFETY: `Surface` is accessed on the main event-loop thread via `&mut Renderer`.
unsafe impl Send for Surface {}
unsafe impl Sync for Surface {}

pub struct SurfaceFrame {
    pub(super) drawable: ObjPtr,
    pub(super) texture: ObjPtr,
}

impl SurfaceFrame {
    #[inline]
    pub fn texture_ptr(&self) -> Id {
        self.texture.as_ptr()
    }
}

impl Surface {
    pub fn new(window: &Window, device: &Device) -> Self {
        let handle = window.window_handle().expect("window handle");
        let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
            panic!("expected AppKit window handle on macOS");
        };
        let ns_view = appkit.ns_view.as_ptr() as Id;
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
            // Re-attach layer to ns_view if `shrink_drawables` previously detached it.
            let current_layer = msg_send!(self.ns_view, c"layer"; -> Id);
            if current_layer != layer {
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
            if current_layer != layer {
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
            Some(SurfaceFrame {
                drawable: ObjPtr(drawable),
                texture: ObjPtr(tex),
            })
        }
    }

    /// Shrink swapchain drawables to 1×1, flush the pool, and detach the layer when idle/hidden/closed
    /// so macOS releases the ~30MB Retina IOSurface backing buffers immediately.
    pub fn shrink_drawables(&mut self, device: &Device, queue: &Queue) {
        let cfg = SurfaceConfig {
            format: TextureFormat::Bgra8Unorm,
            width: 1,
            height: 1,
            transparent: true,
        };
        self.configure(device, &cfg);
        unsafe {
            for _ in 0..2 {
                let Some(frame) = self.acquire_frame() else { break };
                let mut cmd = CommandEncoder::new(queue);
                {
                    let _pass = cmd.begin_render_pass(frame.texture_ptr(), Some([0.0, 0.0, 0.0, 0.0]));
                }
                cmd.present_and_submit_wait(frame);
            }
            // Detach layer from superlayer so CoreAnimation drops cached IOSurfaces while hidden/closed.
            msg_send!(self.layer.as_ptr(), c"removeFromSuperlayer");
        }
    }
}
