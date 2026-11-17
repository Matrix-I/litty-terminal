//! The session bus through a dynamically loaded `libdbus-1.so.3`, shared by
//! the settings portal (`portal.rs`) and desktop notifications. Nothing links
//! against libdbus: without it, the portal keeps the dark default.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::os::fd::RawFd;

use super::sym;

pub(super) const DBUS_TYPE_STRING: c_int = b's' as c_int;
pub(super) const DBUS_TYPE_UINT32: c_int = b'u' as c_int;
pub(super) const DBUS_TYPE_VARIANT: c_int = b'v' as c_int;
pub(super) const DBUS_MESSAGE_TYPE_METHOD_RETURN: c_int = 2;
pub(super) const DBUS_MESSAGE_TYPE_ERROR: c_int = 3;
/// Portals nest the value in up to two variants (`Read` vs `ReadOne`).
const MAX_VARIANT_DEPTH: usize = 4;

/// `DBusError`: two strings, a word of flag bits, one pointer of padding.
#[repr(C)]
pub(super) struct DBusError {
    name: *const c_char,
    message: *const c_char,
    bits: u32,
    padding: *mut c_void,
}

impl DBusError {
    fn new() -> Self {
        Self { name: std::ptr::null(), message: std::ptr::null(), bits: 0, padding: std::ptr::null_mut() }
    }
}

/// `DBusMessageIter` is 72 bytes on 64-bit targets; libdbus only needs
/// stack space at least that large and pointer-aligned.
#[repr(C, align(8))]
pub(super) struct DBusMessageIter([u8; 128]);

impl DBusMessageIter {
    pub(super) fn new() -> Self {
        Self([0; 128])
    }
}

pub(super) type Conn = c_void;
pub(super) type Msg = c_void;

#[allow(non_snake_case)]
pub(super) struct DBusLib {
    _handle: *mut c_void,
    pub(super) dbus_error_init: unsafe extern "C" fn(*mut DBusError),
    pub(super) dbus_error_free: unsafe extern "C" fn(*mut DBusError),
    pub(super) dbus_connection_open_private: unsafe extern "C" fn(*const c_char, *mut DBusError) -> *mut Conn,
    pub(super) dbus_bus_register: unsafe extern "C" fn(*mut Conn, *mut DBusError) -> u32,
    pub(super) dbus_bus_add_match: unsafe extern "C" fn(*mut Conn, *const c_char, *mut DBusError),
    pub(super) dbus_connection_set_exit_on_disconnect: unsafe extern "C" fn(*mut Conn, u32),
    pub(super) dbus_connection_close: unsafe extern "C" fn(*mut Conn),
    pub(super) dbus_connection_unref: unsafe extern "C" fn(*mut Conn),
    pub(super) dbus_connection_flush: unsafe extern "C" fn(*mut Conn),
    pub(super) dbus_connection_get_unix_fd: unsafe extern "C" fn(*mut Conn, *mut c_int) -> u32,
    pub(super) dbus_connection_get_is_connected: unsafe extern "C" fn(*mut Conn) -> u32,
    pub(super) dbus_connection_read_write: unsafe extern "C" fn(*mut Conn, c_int) -> u32,
    pub(super) dbus_connection_pop_message: unsafe extern "C" fn(*mut Conn) -> *mut Msg,
    pub(super) dbus_connection_send: unsafe extern "C" fn(*mut Conn, *mut Msg, *mut u32) -> u32,
    pub(super) dbus_message_new_method_call: unsafe extern "C" fn(*const c_char, *const c_char, *const c_char, *const c_char) -> *mut Msg,
    pub(super) dbus_message_unref: unsafe extern "C" fn(*mut Msg),
    pub(super) dbus_message_is_signal: unsafe extern "C" fn(*mut Msg, *const c_char, *const c_char) -> u32,
    pub(super) dbus_message_get_type: unsafe extern "C" fn(*mut Msg) -> c_int,
    pub(super) dbus_message_get_reply_serial: unsafe extern "C" fn(*mut Msg) -> u32,
    pub(super) dbus_message_iter_init: unsafe extern "C" fn(*mut Msg, *mut DBusMessageIter) -> u32,
    pub(super) dbus_message_iter_init_append: unsafe extern "C" fn(*mut Msg, *mut DBusMessageIter),
    pub(super) dbus_message_iter_append_basic: unsafe extern "C" fn(*mut DBusMessageIter, c_int, *const c_void) -> u32,
    pub(super) dbus_message_iter_get_arg_type: unsafe extern "C" fn(*mut DBusMessageIter) -> c_int,
    pub(super) dbus_message_iter_get_basic: unsafe extern "C" fn(*mut DBusMessageIter, *mut c_void),
    pub(super) dbus_message_iter_recurse: unsafe extern "C" fn(*mut DBusMessageIter, *mut DBusMessageIter),
    pub(super) dbus_message_iter_next: unsafe extern "C" fn(*mut DBusMessageIter) -> u32,
    #[cfg(test)]
    pub(super) dbus_message_new_signal: unsafe extern "C" fn(*const c_char, *const c_char, *const c_char) -> *mut Msg,
    #[cfg(test)]
    pub(super) dbus_message_new_method_return: unsafe extern "C" fn(*mut Msg) -> *mut Msg,
    #[cfg(test)]
    pub(super) dbus_message_new_error: unsafe extern "C" fn(*mut Msg, *const c_char, *const c_char) -> *mut Msg,
    #[cfg(test)]
    pub(super) dbus_message_set_serial: unsafe extern "C" fn(*mut Msg, u32),
    #[cfg(test)]
    pub(super) dbus_message_iter_open_container: unsafe extern "C" fn(*mut DBusMessageIter, c_int, *const c_char, *mut DBusMessageIter) -> u32,
    #[cfg(test)]
    pub(super) dbus_message_iter_close_container: unsafe extern "C" fn(*mut DBusMessageIter, *mut DBusMessageIter) -> u32,
}

impl DBusLib {
    pub(super) fn load() -> Option<Self> {
        unsafe {
            let handle = libc::dlopen(c"libdbus-1.so.3".as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
            if handle.is_null() {
                return None;
            }
            Some(Self {
                _handle: handle,
                dbus_error_init: sym(handle, c"dbus_error_init")?,
                dbus_error_free: sym(handle, c"dbus_error_free")?,
                dbus_connection_open_private: sym(handle, c"dbus_connection_open_private")?,
                dbus_bus_register: sym(handle, c"dbus_bus_register")?,
                dbus_bus_add_match: sym(handle, c"dbus_bus_add_match")?,
                dbus_connection_set_exit_on_disconnect: sym(handle, c"dbus_connection_set_exit_on_disconnect")?,
                dbus_connection_close: sym(handle, c"dbus_connection_close")?,
                dbus_connection_unref: sym(handle, c"dbus_connection_unref")?,
                dbus_connection_flush: sym(handle, c"dbus_connection_flush")?,
                dbus_connection_get_unix_fd: sym(handle, c"dbus_connection_get_unix_fd")?,
                dbus_connection_get_is_connected: sym(handle, c"dbus_connection_get_is_connected")?,
                dbus_connection_read_write: sym(handle, c"dbus_connection_read_write")?,
                dbus_connection_pop_message: sym(handle, c"dbus_connection_pop_message")?,
                dbus_connection_send: sym(handle, c"dbus_connection_send")?,
                dbus_message_new_method_call: sym(handle, c"dbus_message_new_method_call")?,
                dbus_message_unref: sym(handle, c"dbus_message_unref")?,
                dbus_message_is_signal: sym(handle, c"dbus_message_is_signal")?,
                dbus_message_get_type: sym(handle, c"dbus_message_get_type")?,
                dbus_message_get_reply_serial: sym(handle, c"dbus_message_get_reply_serial")?,
                dbus_message_iter_init: sym(handle, c"dbus_message_iter_init")?,
                dbus_message_iter_init_append: sym(handle, c"dbus_message_iter_init_append")?,
                dbus_message_iter_append_basic: sym(handle, c"dbus_message_iter_append_basic")?,
                dbus_message_iter_get_arg_type: sym(handle, c"dbus_message_iter_get_arg_type")?,
                dbus_message_iter_get_basic: sym(handle, c"dbus_message_iter_get_basic")?,
                dbus_message_iter_recurse: sym(handle, c"dbus_message_iter_recurse")?,
                dbus_message_iter_next: sym(handle, c"dbus_message_iter_next")?,
                #[cfg(test)]
                dbus_message_new_signal: sym(handle, c"dbus_message_new_signal")?,
                #[cfg(test)]
                dbus_message_new_method_return: sym(handle, c"dbus_message_new_method_return")?,
                #[cfg(test)]
                dbus_message_new_error: sym(handle, c"dbus_message_new_error")?,
                #[cfg(test)]
                dbus_message_set_serial: sym(handle, c"dbus_message_set_serial")?,
                #[cfg(test)]
                dbus_message_iter_open_container: sym(handle, c"dbus_message_iter_open_container")?,
                #[cfg(test)]
                dbus_message_iter_close_container: sym(handle, c"dbus_message_iter_close_container")?,
            })
        }
    }

    /// The `u32` at `iter`, looking through nested variants.
    ///
    /// # Safety
    /// `iter` must be a valid iterator into a live message.
    pub(super) unsafe fn uint_in_variants(&self, iter: &mut DBusMessageIter) -> Option<u32> {
        let mut current = DBusMessageIter::new();
        unsafe {
            std::ptr::copy_nonoverlapping(iter, &mut current, 1);
            for _ in 0..MAX_VARIANT_DEPTH {
                match (self.dbus_message_iter_get_arg_type)(&mut current) {
                    DBUS_TYPE_UINT32 => {
                        let mut value = 0u32;
                        (self.dbus_message_iter_get_basic)(&mut current, (&mut value as *mut u32).cast());
                        return Some(value);
                    }
                    DBUS_TYPE_VARIANT => {
                        let mut inner = DBusMessageIter::new();
                        (self.dbus_message_iter_recurse)(&mut current, &mut inner);
                        current = inner;
                    }
                    _ => return None,
                }
            }
        }
        None
    }

    /// The string at `iter`, if it holds one.
    ///
    /// # Safety
    /// `iter` must be a valid iterator into a live message; the string
    /// lives as long as the message.
    pub(super) unsafe fn string_at<'a>(&self, iter: &mut DBusMessageIter) -> Option<&'a CStr> {
        unsafe {
            if (self.dbus_message_iter_get_arg_type)(iter) != DBUS_TYPE_STRING {
                return None;
            }
            let mut ptr: *const c_char = std::ptr::null();
            (self.dbus_message_iter_get_basic)(iter, (&mut ptr as *mut *const c_char).cast());
            (!ptr.is_null()).then(|| CStr::from_ptr(ptr))
        }
    }
}

/// The session bus address: `DBUS_SESSION_BUS_ADDRESS`, else the systemd
/// user bus socket `$XDG_RUNTIME_DIR/bus` when it exists. `None` otherwise:
/// libdbus would then autolaunch a bus (`dbus-launch` through X11), starting
/// a daemon just to ask for a theme, as over `ssh -X`.
fn session_bus_address(
    env_address: Option<std::ffi::OsString>,
    runtime_dir: Option<std::ffi::OsString>,
    exists: impl Fn(&std::path::Path) -> bool,
) -> Option<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    let address = match env_address.filter(|a| !a.is_empty()) {
        Some(address) => address,
        None => {
            let socket = std::path::Path::new(&runtime_dir.filter(|d| !d.is_empty())?).join("bus");
            if !exists(&socket) {
                return None;
            }
            let mut address = std::ffi::OsString::from("unix:path=");
            address.push(socket);
            address
        }
    };
    std::ffi::CString::new(address.as_bytes()).ok()
}

/// The session bus address from this process's environment.
pub(super) fn session_address() -> Option<CString> {
    session_bus_address(
        std::env::var_os("DBUS_SESSION_BUS_ADDRESS"),
        std::env::var_os("XDG_RUNTIME_DIR"),
        |p| p.exists(),
    )
}

/// A private connection to the session bus at `address`, registered on the
/// bus, and its socket; `None` when the bus cannot be reached.
pub(super) fn open_session(lib: &DBusLib, address: &CStr) -> Option<(*mut Conn, RawFd)> {
    unsafe {
        let mut err = DBusError::new();
        (lib.dbus_error_init)(&mut err);
        let conn = (lib.dbus_connection_open_private)(address.as_ptr(), &mut err);
        if conn.is_null() {
            (lib.dbus_error_free)(&mut err);
            return None;
        }
        // libdbus calls `_exit` when a bus connection drops unless told not to.
        (lib.dbus_connection_set_exit_on_disconnect)(conn, 0);
        if (lib.dbus_bus_register)(conn, &mut err) == 0 {
            (lib.dbus_error_free)(&mut err);
            (lib.dbus_connection_close)(conn);
            (lib.dbus_connection_unref)(conn);
            return None;
        }
        let mut fd: c_int = -1;
        if (lib.dbus_connection_get_unix_fd)(conn, &mut fd) == 0 {
            (lib.dbus_connection_close)(conn);
            (lib.dbus_connection_unref)(conn);
            return None;
        }
        Some((conn, fd))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bus_address_never_needs_autolaunch() {
        let os = |s: &str| Some(std::ffi::OsString::from(s));
        let yes = |_: &std::path::Path| true;
        let no = |_: &std::path::Path| false;
        assert_eq!(session_bus_address(os("unix:path=/x"), os("/run/user/1"), no).as_deref(), Some(c"unix:path=/x"));
        assert_eq!(session_bus_address(None, os("/run/user/1"), yes).as_deref(), Some(c"unix:path=/run/user/1/bus"));
        assert_eq!(session_bus_address(os(""), os("/run/user/1"), yes).as_deref(), Some(c"unix:path=/run/user/1/bus"));
        // No address and no user bus socket: no bus rather than autolaunch.
        assert_eq!(session_bus_address(None, os("/run/user/1"), no), None);
        assert_eq!(session_bus_address(None, None, yes), None);
    }

    /// An address nothing listens on is no connection, not a hang or abort.
    #[test]
    fn an_unreachable_bus_opens_nothing() {
        let Some(lib) = DBusLib::load() else {
            eprintln!("libdbus-1 not installed; skipping");
            return;
        };
        assert!(open_session(&lib, c"unix:path=/nonexistent/litty-test-bus").is_none());
    }

    /// A reachable session bus gives a registered connection with a socket.
    #[test]
    fn the_session_bus_opens_with_a_socket() {
        let (Some(lib), Some(address)) = (DBusLib::load(), session_address()) else {
            eprintln!("no libdbus-1 or session bus; skipping");
            return;
        };
        let (conn, fd) = open_session(&lib, &address).expect("session bus reachable");
        assert!(fd >= 0);
        unsafe {
            (lib.dbus_connection_close)(conn);
            (lib.dbus_connection_unref)(conn);
        }
    }
}
