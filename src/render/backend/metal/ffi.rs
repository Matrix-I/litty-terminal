//! Objective-C runtime helpers, reference-counted `ObjPtr`, and Metal C structs/constants.

use std::ffi::{c_char, c_void, CStr, CString};

use crate::render::backend::TextureFormat;

pub type Id = *mut c_void;

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub(super) struct CGSize {
    pub width: f64,
    pub height: f64,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub(super) struct MTLOrigin {
    pub x: usize,
    pub y: usize,
    pub z: usize,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub(super) struct MTLSize {
    pub width: usize,
    pub height: usize,
    pub depth: usize,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub(super) struct MTLRegion {
    pub origin: MTLOrigin,
    pub size: MTLSize,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub(super) struct MTLClearColor {
    pub red: f64,
    pub green: f64,
    pub blue: f64,
    pub alpha: f64,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub(super) struct MTLScissorRect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

#[link(name = "objc", kind = "dylib")]
unsafe extern "C" {
    pub(super) fn objc_getClass(name: *const c_char) -> Id;
    pub(super) fn sel_registerName(name: *const c_char) -> Id;
    pub(super) fn objc_msgSend();
}

#[link(name = "Metal", kind = "framework")]
unsafe extern "C" {
    pub(super) fn MTLCreateSystemDefaultDevice() -> Id;
}

#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {}

#[inline]
pub(super) fn sel(name: &CStr) -> Id {
    // SAFETY: registering a static C selector name is always valid.
    unsafe { sel_registerName(name.as_ptr()) }
}

#[inline]
pub(super) fn class(name: &CStr) -> Option<Id> {
    // SAFETY: looking up an Objective-C class by name is always valid.
    let cls = unsafe { objc_getClass(name.as_ptr()) };
    (!cls.is_null()).then_some(cls)
}

macro_rules! msg_send {
    ($recv:expr_2021, $sel:expr_2021 $(, $arg:expr_2021 => $ty:ty)* ; -> $ret:ty) => {{
        let f: unsafe extern "C" fn(
            $crate::render::backend::metal::ffi::Id,
            $crate::render::backend::metal::ffi::Id
            $(, $ty)*
        ) -> $ret = std::mem::transmute($crate::render::backend::metal::ffi::objc_msgSend as *const ());
        f($recv, $crate::render::backend::metal::ffi::sel($sel) $(, $arg)*)
    }};
    ($recv:expr_2021, $sel:expr_2021 $(, $arg:expr_2021 => $ty:ty)*) => {
        $crate::render::backend::metal::ffi::msg_send!($recv, $sel $(, $arg => $ty)* ; -> ())
    };
}

pub(super) use msg_send;

#[inline]
pub(super) unsafe fn send_id(receiver: Id, selector: &CStr) -> Option<Id> {
    unsafe {
        let obj = msg_send!(receiver, selector; -> Id);
        (!obj.is_null()).then_some(obj)
    }
}

#[inline]
pub(super) unsafe fn ns_string(s: &str) -> Option<Id> {
    unsafe {
        let c = CString::new(s).ok()?;
        let obj = msg_send!(
            class(c"NSString")?,
            c"stringWithUTF8String:",
            c.as_ptr() => *const c_char;
            -> Id
        );
        (!obj.is_null()).then_some(obj)
    }
}

pub(super) unsafe fn ns_error_string(err: Id) -> String {
    unsafe {
        if err.is_null() {
            return "unknown Metal error".to_string();
        }
        let Some(desc) = send_id(err, c"localizedDescription") else {
            return "unknown Metal error".to_string();
        };
        let utf8 = msg_send!(desc, c"UTF8String"; -> *const c_char);
        if utf8.is_null() {
            return "unknown Metal error".to_string();
        }
        CStr::from_ptr(utf8).to_string_lossy().into_owned()
    }
}

/// Objective-C object pointer that calls `[obj release]` on drop.
pub struct ObjPtr(pub(super) Id);

// SAFETY: Metal device, command queue, pipeline states, textures, samplers, and buffers
// are thread-safe reference-counted Objective-C objects.
unsafe impl Send for ObjPtr {}
unsafe impl Sync for ObjPtr {}

impl ObjPtr {
    #[inline]
    pub const fn as_ptr(&self) -> Id {
        self.0
    }
}

impl Clone for ObjPtr {
    fn clone(&self) -> Self {
        if !self.0.is_null() {
            unsafe {
                msg_send!(self.0, c"retain"; -> Id);
            }
        }
        Self(self.0)
    }
}

impl Drop for ObjPtr {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                msg_send!(self.0, c"release");
            }
        }
    }
}

// Metal constants
pub(super) const MTL_PIXEL_FORMAT_R8_UNORM: usize = 10;
pub(super) const MTL_PIXEL_FORMAT_RGBA8_UNORM: usize = 70;
pub(super) const MTL_PIXEL_FORMAT_BGRA8_UNORM: usize = 80;

pub(super) const MTL_TEXTURE_USAGE_SHADER_READ: usize = 1;
pub(super) const MTL_TEXTURE_USAGE_RENDER_TARGET: usize = 4;

pub(super) const MTL_STORAGE_MODE_SHARED: usize = 0;

pub(super) const MTL_LOAD_ACTION_LOAD: usize = 1;
pub(super) const MTL_LOAD_ACTION_CLEAR: usize = 2;
pub(super) const MTL_STORE_ACTION_STORE: usize = 1;

pub(super) const MTL_PRIMITIVE_TYPE_TRIANGLE: usize = 3;
pub(super) const MTL_PRIMITIVE_TYPE_TRIANGLE_STRIP: usize = 4;

pub(super) const MTL_BLEND_FACTOR_ZERO: usize = 0;
pub(super) const MTL_BLEND_FACTOR_ONE: usize = 1;
pub(super) const MTL_BLEND_FACTOR_SRC_ALPHA: usize = 4;
pub(super) const MTL_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA: usize = 5;
pub(super) const MTL_BLEND_OPERATION_ADD: usize = 0;

pub(super) const MTL_SAMPLER_MIN_MAG_FILTER_NEAREST: usize = 0;
pub(super) const MTL_SAMPLER_MIN_MAG_FILTER_LINEAR: usize = 1;

#[inline]
pub(super) const fn mtl_pixel_format(format: TextureFormat) -> usize {
    match format {
        TextureFormat::R8Unorm => MTL_PIXEL_FORMAT_R8_UNORM,
        TextureFormat::Rgba8Unorm => MTL_PIXEL_FORMAT_RGBA8_UNORM,
        TextureFormat::Bgra8Unorm => MTL_PIXEL_FORMAT_BGRA8_UNORM,
    }
}
