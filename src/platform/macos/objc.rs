//! Objective-C runtime access: ids, selectors, message sends.

use std::ffi::{c_void, CStr};

use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

pub(super) type Id = *mut c_void;

#[repr(C)]
#[derive(Copy, Clone)]
pub(super) struct NSRect {
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) w: f64,
    pub(super) h: f64,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub(super) struct NSPoint {
    pub(super) x: f64,
    pub(super) y: f64,
}

#[link(name = "objc", kind = "dylib")]
unsafe extern "C" {
    pub(super) fn objc_getClass(name: *const std::ffi::c_char) -> Id;
    pub(super) fn sel_registerName(name: *const std::ffi::c_char) -> Id;
    pub(super) fn objc_msgSend();
    /// Required on x86_64 for methods returning a struct larger than 16 bytes
    /// (e.g. `NSRect`): the caller passes the return buffer in `%rdi`. arm64
    /// has no such entry point; it returns small structs in registers.
    #[cfg(target_arch = "x86_64")]
    pub(super) fn objc_msgSend_stret();
    pub(super) fn class_addMethod(cls: Id, name: Id, imp: *const (), types: *const std::ffi::c_char) -> bool;
    pub(super) fn class_getInstanceMethod(cls: Id, name: Id) -> Id;
    pub(super) fn method_setImplementation(m: Id, imp: *const ()) -> *const ();
    pub(super) fn method_getImplementation(m: Id) -> *const ();
    pub(super) fn objc_allocateClassPair(superclass: Id, name: *const std::ffi::c_char, extra: usize) -> Id;
    pub(super) fn objc_registerClassPair(cls: Id);
}

pub(super) fn sel(name: &CStr) -> Id {
    // SAFETY: registering a selector name is always valid.
    unsafe { sel_registerName(name.as_ptr()) }
}

pub(super) fn class(name: &CStr) -> Option<Id> {
    // SAFETY: looking up a class by name is always valid.
    let cls = unsafe { objc_getClass(name.as_ptr()) };
    (!cls.is_null()).then_some(cls)
}

/// Whether a method returning `T` must go through `objc_msgSend_stret` (Apple
/// x86_64 System V ABI: aggregates over 16 bytes are returned in memory).
#[cfg(any(target_arch = "x86_64", test))]
pub(super) const fn returns_in_memory<T>() -> bool {
    cfg!(target_arch = "x86_64") && std::mem::size_of::<T>() > 16
}

/// Send an Objective-C message with an explicit C signature:
/// `msg_send!(receiver, c"selector:", arg: Type, ... => ReturnType)`.
/// Struct returns that the ABI passes in memory are routed to
/// `objc_msgSend_stret` on x86_64 (CWE-686).
/// Must be used inside `unsafe`: the signature has to match the method.
#[cfg(target_arch = "x86_64")]
macro_rules! msg_send {
    ($recv:expr_2021, $sel:expr_2021 $(, $arg:expr_2021 => $ty:ty)* ; -> $ret:ty) => {{
        if returns_in_memory::<$ret>() {
            let f: unsafe extern "C" fn(*mut $ret, Id, Id $(, $ty)*) =
                std::mem::transmute(objc_msgSend_stret as *const ());
            let mut out = std::mem::MaybeUninit::<$ret>::uninit();
            f(out.as_mut_ptr(), $recv, sel($sel) $(, $arg)*);
            out.assume_init()
        } else {
            let f: unsafe extern "C" fn(Id, Id $(, $ty)*) -> $ret =
                std::mem::transmute(objc_msgSend as *const ());
            f($recv, sel($sel) $(, $arg)*)
        }
    }};
    ($recv:expr_2021, $sel:expr_2021 $(, $arg:expr_2021 => $ty:ty)*) => {
        msg_send!($recv, $sel $(, $arg => $ty)* ; -> ())
    };
}

#[cfg(not(target_arch = "x86_64"))]
macro_rules! msg_send {
    ($recv:expr_2021, $sel:expr_2021 $(, $arg:expr_2021 => $ty:ty)* ; -> $ret:ty) => {{
        let f: unsafe extern "C" fn(Id, Id $(, $ty)*) -> $ret =
            std::mem::transmute(objc_msgSend as *const ());
        f($recv, sel($sel) $(, $arg)*)
    }};
    ($recv:expr_2021, $sel:expr_2021 $(, $arg:expr_2021 => $ty:ty)*) => {
        msg_send!($recv, $sel $(, $arg => $ty)* ; -> ())
    };
}

pub(super) use msg_send;

/// Send a message that returns an object; `None` for nil.
pub(super) unsafe fn send_id(receiver: Id, selector: &CStr) -> Option<Id> { unsafe {
    let obj = msg_send!(receiver, selector; -> Id);
    (!obj.is_null()).then_some(obj)
}}

pub(super) unsafe fn responds_to(obj: Id, selector: &CStr) -> bool { unsafe {
    msg_send!(obj, c"respondsToSelector:", sel(selector) => Id; -> bool)
}}

pub(super) unsafe fn ns_string(s: &str) -> Option<Id> { unsafe {
    let c = std::ffi::CString::new(s).ok()?;
    let obj = msg_send!(class(c"NSString")?, c"stringWithUTF8String:", c.as_ptr() => *const std::ffi::c_char; -> Id);
    (!obj.is_null()).then_some(obj)
}}

pub(super) unsafe fn ns_string_to_string(ns_str: Id) -> Option<String> { unsafe {
    let utf8_ptr = msg_send!(ns_str, c"UTF8String"; -> *const std::ffi::c_char);
    if utf8_ptr.is_null() {
        return None;
    }
    std::ffi::CStr::from_ptr(utf8_ptr).to_str().ok().map(|s| s.to_string())
}}

pub(super) unsafe fn shared_application() -> Option<Id> { unsafe {
    send_id(class(c"NSApplication")?, c"sharedApplication")
}}

/// Call `f` for each element of an `NSArray`.
pub(super) unsafe fn for_each_in_array(array: Id, mut f: impl FnMut(Id)) { unsafe {
    let count = msg_send!(array, c"count"; -> usize);
    for i in 0..count {
        let obj = msg_send!(array, c"objectAtIndex:", i => usize; -> Id);
        if !obj.is_null() {
            f(obj);
        }
    }
}}

pub(super) fn ns_window_of(window: &Window) -> Option<(Id, Id)> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else { return None };
    let ns_view = appkit.ns_view.as_ptr() as Id;
    // SAFETY: `ns_view` is the live NSView backing this winit window.
    let ns_window = unsafe { send_id(ns_view, c"window")? };
    Some((ns_view, ns_window))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_structs_over_16_bytes_use_stret_on_x86_64() {
        assert_eq!(std::mem::size_of::<NSRect>(), 32);
        assert_eq!(returns_in_memory::<NSRect>(), cfg!(target_arch = "x86_64"));
        assert!(!returns_in_memory::<NSPoint>());
        assert!(!returns_in_memory::<Id>());
        assert!(!returns_in_memory::<bool>());
    }

    #[test]
    fn struct_return_roundtrips_through_a_real_objc_call() {
        // NSValue links via Foundation, which winit/AppKit already load.
        let rect = NSRect { x: 1.0, y: 2.0, w: 300.0, h: 400.0 };
        // SAFETY: valueWithRect: / rectValue have exactly these signatures.
        let got = unsafe {
            let value = msg_send!(class(c"NSValue").unwrap(), c"valueWithRect:", rect => NSRect; -> Id);
            assert!(!value.is_null());
            msg_send!(value, c"rectValue"; -> NSRect)
        };
        assert_eq!((got.x, got.y, got.w, got.h), (1.0, 2.0, 300.0, 400.0));
    }
}
