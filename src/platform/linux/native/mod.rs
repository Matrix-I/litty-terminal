//! Native Linux windowing and unified epoll event loop (Milestone 3 / Phase 4.2).

pub mod app;
pub mod events;
pub mod monitor;
pub mod portal;
pub mod wayland;
mod wayland_input;
mod wayland_data;
mod wayland_visibility;
pub(super) mod wayland_activation;
mod wayland_text_input;
pub mod window;
pub mod x11;
pub mod xim;
pub mod xkb;

#[allow(unused_imports)]
pub use app::run_native;
#[allow(unused_imports)]
pub use window::NativeWindow;

/// The function `name` in the library `handle` from `dlopen`, as the
/// function pointer type `T`; `None` when the library lacks it.
///
/// # Safety
/// `T` must be the `unsafe extern "C" fn` type the symbol really has.
unsafe fn sym<T>(handle: *mut std::ffi::c_void, name: &std::ffi::CStr) -> Option<T> {
    const { assert!(size_of::<T>() == size_of::<*mut std::ffi::c_void>()) };
    let p = unsafe { libc::dlsym(handle, name.as_ptr()) };
    (!p.is_null()).then(|| unsafe { std::mem::transmute_copy(&p) })
}

#[cfg(test)]
mod tests {
    use super::sym;

    #[test]
    fn sym_finds_present_functions_only() {
        let process = unsafe { libc::dlopen(std::ptr::null(), libc::RTLD_LAZY) };
        assert!(!process.is_null());
        let getpid: Option<unsafe extern "C" fn() -> libc::pid_t> = unsafe { sym(process, c"getpid") };
        assert_eq!(getpid.map(|f| unsafe { f() }), Some(std::process::id() as libc::pid_t));
        let missing: Option<unsafe extern "C" fn()> = unsafe { sym(process, c"litty_no_such_symbol") };
        assert!(missing.is_none());
    }
}
