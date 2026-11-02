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

/// Send an Objective-C message with an explicit C signature:
/// `msg_send!(receiver, c"selector:", arg: Type, ... => ReturnType)`.
/// Must be used inside `unsafe`: the signature has to match the method.
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
