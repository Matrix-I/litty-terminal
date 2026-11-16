//! What the renderer needs from a window. `app` implements it for the
//! platform window, so `render` does not depend on `platform`.

/// Raw native handle for GPU surface creation (Metal, Vulkan).
// Each OS reads only its own variants' fields.
#[allow(dead_code)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RawHandle {
    AppKit { ns_view: *mut std::ffi::c_void },
    Wayland { display: *mut std::ffi::c_void, surface: *mut std::ffi::c_void },
    Xlib { display: *mut std::ffi::c_void, window: std::ffi::c_ulong },
    Xcb { connection: *mut std::ffi::c_void, window: u32 },
}

pub trait RenderWindow: Send + Sync {
    /// Native handle to build the GPU surface on; `None` if the window has none.
    fn raw_handle(&self) -> Option<RawHandle>;
    // Temporary: called through the trait only once Renderer moves onto it.
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    fn scale_factor(&self) -> f64;
    /// Physical size in pixels `(width, height)`.
    // Temporary: called through the trait only once Renderer moves onto it.
    #[allow(dead_code)]
    fn inner_size(&self) -> (u32, u32);
    /// Right before a frame is presented (Wayland asks for its frame callback here).
    // Temporary: called through the trait only once Renderer moves onto it.
    #[allow(dead_code)]
    fn pre_present_notify(&self);
    /// The frame announced by `pre_present_notify` was not presented.
    // Temporary: called through the trait only once Renderer moves onto it.
    #[allow(dead_code)]
    fn present_skipped(&self);
}
