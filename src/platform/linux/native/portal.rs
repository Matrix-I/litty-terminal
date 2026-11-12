//! System light/dark preference from the XDG desktop portal
//! (`org.freedesktop.appearance` `color-scheme`), through a dynamically
//! loaded `libdbus-1.so.3`. GNOME, KDE and the other desktops that ship a
//! settings portal publish it on X11 and Wayland alike.

use std::ffi::{c_char, c_int, c_void, CStr};
use std::os::fd::RawFd;

use parking_lot::Mutex;

use crate::platform::gui::Theme;
use super::sym;

const PORTAL_DEST: &CStr = c"org.freedesktop.portal.Desktop";
const PORTAL_PATH: &CStr = c"/org/freedesktop/portal/desktop";
const SETTINGS_IFACE: &CStr = c"org.freedesktop.portal.Settings";
const SETTING_CHANGED: &CStr = c"SettingChanged";
const APPEARANCE_NS: &CStr = c"org.freedesktop.appearance";
const COLOR_SCHEME_KEY: &CStr = c"color-scheme";
const CHANGE_MATCH: &CStr = c"type='signal',interface='org.freedesktop.portal.Settings',\
member='SettingChanged',path='/org/freedesktop/portal/desktop',arg0='org.freedesktop.appearance'";
/// The first window waits up to this long after the portal was opened for
/// its first answer, so it opens in the right theme; a later answer still
/// applies, as a theme change.
const FIRST_READ_WAIT: std::time::Duration = std::time::Duration::from_millis(100);

/// What is left of `FIRST_READ_WAIT` once `elapsed` passed since opening.
fn first_read_wait_left(elapsed: std::time::Duration) -> std::time::Duration {
    FIRST_READ_WAIT.saturating_sub(elapsed)
}

const DBUS_TYPE_STRING: c_int = b's' as c_int;
const DBUS_TYPE_UINT32: c_int = b'u' as c_int;
const DBUS_TYPE_VARIANT: c_int = b'v' as c_int;
const DBUS_MESSAGE_TYPE_METHOD_RETURN: c_int = 2;
const DBUS_MESSAGE_TYPE_ERROR: c_int = 3;
/// Portals nest the value in up to two variants (`Read` vs `ReadOne`).
const MAX_VARIANT_DEPTH: usize = 4;

/// `DBusError`: two strings, a word of flag bits, one pointer of padding.
#[repr(C)]
struct DBusError {
    name: *const c_char,
    message: *const c_char,
    bits: u32,
    padding: *mut c_void,
}

/// `DBusMessageIter` is 72 bytes on 64-bit targets; libdbus only needs
/// stack space at least that large and pointer-aligned.
#[repr(C, align(8))]
struct DBusMessageIter([u8; 128]);

impl DBusMessageIter {
    fn new() -> Self {
        Self([0; 128])
    }
}

type Conn = c_void;
type Msg = c_void;

#[allow(non_snake_case)]
struct DBusLib {
    _handle: *mut c_void,
    dbus_error_init: unsafe extern "C" fn(*mut DBusError),
    dbus_error_free: unsafe extern "C" fn(*mut DBusError),
    dbus_connection_open_private: unsafe extern "C" fn(*const c_char, *mut DBusError) -> *mut Conn,
    dbus_bus_register: unsafe extern "C" fn(*mut Conn, *mut DBusError) -> u32,
    dbus_bus_add_match: unsafe extern "C" fn(*mut Conn, *const c_char, *mut DBusError),
    dbus_connection_set_exit_on_disconnect: unsafe extern "C" fn(*mut Conn, u32),
    dbus_connection_close: unsafe extern "C" fn(*mut Conn),
    dbus_connection_unref: unsafe extern "C" fn(*mut Conn),
    dbus_connection_flush: unsafe extern "C" fn(*mut Conn),
    dbus_connection_get_unix_fd: unsafe extern "C" fn(*mut Conn, *mut c_int) -> u32,
    dbus_connection_get_is_connected: unsafe extern "C" fn(*mut Conn) -> u32,
    dbus_connection_read_write: unsafe extern "C" fn(*mut Conn, c_int) -> u32,
    dbus_connection_pop_message: unsafe extern "C" fn(*mut Conn) -> *mut Msg,
    dbus_connection_send: unsafe extern "C" fn(*mut Conn, *mut Msg, *mut u32) -> u32,
    dbus_message_new_method_call: unsafe extern "C" fn(*const c_char, *const c_char, *const c_char, *const c_char) -> *mut Msg,
    dbus_message_unref: unsafe extern "C" fn(*mut Msg),
    dbus_message_is_signal: unsafe extern "C" fn(*mut Msg, *const c_char, *const c_char) -> u32,
    dbus_message_get_type: unsafe extern "C" fn(*mut Msg) -> c_int,
    dbus_message_get_reply_serial: unsafe extern "C" fn(*mut Msg) -> u32,
    dbus_message_iter_init: unsafe extern "C" fn(*mut Msg, *mut DBusMessageIter) -> u32,
    dbus_message_iter_init_append: unsafe extern "C" fn(*mut Msg, *mut DBusMessageIter),
    dbus_message_iter_append_basic: unsafe extern "C" fn(*mut DBusMessageIter, c_int, *const c_void) -> u32,
    dbus_message_iter_get_arg_type: unsafe extern "C" fn(*mut DBusMessageIter) -> c_int,
    dbus_message_iter_get_basic: unsafe extern "C" fn(*mut DBusMessageIter, *mut c_void),
    dbus_message_iter_recurse: unsafe extern "C" fn(*mut DBusMessageIter, *mut DBusMessageIter),
    dbus_message_iter_next: unsafe extern "C" fn(*mut DBusMessageIter) -> u32,
    #[cfg(test)]
    dbus_message_new_signal: unsafe extern "C" fn(*const c_char, *const c_char, *const c_char) -> *mut Msg,
    #[cfg(test)]
    dbus_message_new_method_return: unsafe extern "C" fn(*mut Msg) -> *mut Msg,
    #[cfg(test)]
    dbus_message_new_error: unsafe extern "C" fn(*mut Msg, *const c_char, *const c_char) -> *mut Msg,
    #[cfg(test)]
    dbus_message_set_serial: unsafe extern "C" fn(*mut Msg, u32),
    #[cfg(test)]
    dbus_message_iter_open_container: unsafe extern "C" fn(*mut DBusMessageIter, c_int, *const c_char, *mut DBusMessageIter) -> u32,
    #[cfg(test)]
    dbus_message_iter_close_container: unsafe extern "C" fn(*mut DBusMessageIter, *mut DBusMessageIter) -> u32,
}

impl DBusLib {
    fn load() -> Option<Self> {
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
    unsafe fn uint_in_variants(&self, iter: &mut DBusMessageIter) -> Option<u32> {
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
    unsafe fn string_at<'a>(&self, iter: &mut DBusMessageIter) -> Option<&'a CStr> {
        unsafe {
            if (self.dbus_message_iter_get_arg_type)(iter) != DBUS_TYPE_STRING {
                return None;
            }
            let mut ptr: *const c_char = std::ptr::null();
            (self.dbus_message_iter_get_basic)(iter, (&mut ptr as *mut *const c_char).cast());
            (!ptr.is_null()).then(|| CStr::from_ptr(ptr))
        }
    }

    /// `color-scheme` from a `Read`/`ReadOne` reply.
    ///
    /// # Safety
    /// `reply` must be a live message.
    unsafe fn scheme_in_reply(&self, reply: *mut Msg) -> Option<u32> {
        let mut iter = DBusMessageIter::new();
        unsafe {
            if (self.dbus_message_iter_init)(reply, &mut iter) == 0 {
                return None;
            }
            self.uint_in_variants(&mut iter)
        }
    }

    /// `color-scheme` from a `SettingChanged(namespace, key, value)` signal,
    /// or `None` for any other setting.
    ///
    /// # Safety
    /// `signal` must be a live message.
    unsafe fn scheme_in_signal(&self, signal: *mut Msg) -> Option<u32> {
        let mut iter = DBusMessageIter::new();
        unsafe {
            if (self.dbus_message_is_signal)(signal, SETTINGS_IFACE.as_ptr(), SETTING_CHANGED.as_ptr()) == 0
                || (self.dbus_message_iter_init)(signal, &mut iter) == 0
                || self.string_at(&mut iter)? != APPEARANCE_NS
                || (self.dbus_message_iter_next)(&mut iter) == 0
                || self.string_at(&mut iter)? != COLOR_SCHEME_KEY
                || (self.dbus_message_iter_next)(&mut iter) == 0
            {
                return None;
            }
            self.uint_in_variants(&mut iter)
        }
    }

    /// What `msg` means for the read awaiting the reply `pending`.
    ///
    /// # Safety
    /// `msg` must be a live message.
    unsafe fn classify(&self, msg: *mut Msg, pending: Option<u32>) -> Incoming {
        unsafe {
            let kind = (self.dbus_message_get_type)(msg);
            let answers_read = matches!(kind, DBUS_MESSAGE_TYPE_METHOD_RETURN | DBUS_MESSAGE_TYPE_ERROR)
                && pending == Some((self.dbus_message_get_reply_serial)(msg));
            if answers_read {
                return match (kind, self.scheme_in_reply(msg)) {
                    (DBUS_MESSAGE_TYPE_METHOD_RETURN, Some(scheme)) => Incoming::Answer(scheme),
                    _ => Incoming::ReadFailed,
                };
            }
            self.scheme_in_signal(msg).map_or(Incoming::Other, Incoming::Changed)
        }
    }
}

/// A message from the bus, as far as the theme is concerned.
#[derive(Debug, PartialEq, Eq)]
enum Incoming {
    /// The read's answer.
    Answer(u32),
    /// A `SettingChanged` signal for `color-scheme`.
    Changed(u32),
    /// The read failed: an error reply, or a value that is no `u32`.
    ReadFailed,
    Other,
}

/// The portal's two methods for one setting: `ReadOne` (portal version 2),
/// then the deprecated `Read` older portals still answer.
const READ_METHODS: [&CStr; 2] = [c"ReadOne", c"Read"];

/// Theme for a portal `color-scheme` value: 1 prefers dark, 2 prefers
/// light. 0 is no preference, where the app keeps its own (dark) default,
/// as the portal specification asks; unknown future values likewise.
fn theme_from_scheme(scheme: u32) -> Option<Theme> {
    match scheme {
        1 => Some(Theme::Dark),
        2 => Some(Theme::Light),
        _ => None,
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

struct Portal {
    lib: DBusLib,
    conn: *mut Conn,
    fd: RawFd,
    theme: Option<Theme>,
    /// The read awaiting its reply: the call's serial and its index in
    /// `READ_METHODS`.
    pending: Option<(u32, usize)>,
    /// When the first read was sent (`FIRST_READ_WAIT`).
    opened_at: std::time::Instant,
}

// The connection is only used under `PORTAL`'s lock.
unsafe impl Send for Portal {}

impl Portal {
    fn open() -> Option<Self> {
        let address = session_bus_address(
            std::env::var_os("DBUS_SESSION_BUS_ADDRESS"),
            std::env::var_os("XDG_RUNTIME_DIR"),
            |p| p.exists(),
        )?;
        let lib = DBusLib::load()?;
        unsafe {
            let mut err = DBusError { name: std::ptr::null(), message: std::ptr::null(), bits: 0, padding: std::ptr::null_mut() };
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
            // Subscribe before reading, so a switch in between is not lost.
            // No error out-parameter: the match is sent without waiting for the reply.
            (lib.dbus_bus_add_match)(conn, CHANGE_MATCH.as_ptr(), std::ptr::null_mut());
            let mut portal = Self { lib, conn, fd, theme: None, pending: None, opened_at: std::time::Instant::now() };
            // Not waited for here: the answer comes while the app starts up.
            portal.send_read(0);
            Some(portal)
        }
    }

    /// Ask for `color-scheme` with `READ_METHODS[method]`, without waiting:
    /// the reply arrives through `dispatch`.
    fn send_read(&mut self, method: usize) {
        self.pending = None;
        let Some(name) = READ_METHODS.get(method) else { return };
        let lib = &self.lib;
        unsafe {
            let msg = (lib.dbus_message_new_method_call)(
                PORTAL_DEST.as_ptr(), PORTAL_PATH.as_ptr(), SETTINGS_IFACE.as_ptr(), name.as_ptr(),
            );
            if msg.is_null() {
                return;
            }
            let mut args = DBusMessageIter::new();
            (lib.dbus_message_iter_init_append)(msg, &mut args);
            for arg in [APPEARANCE_NS, COLOR_SCHEME_KEY] {
                let ptr = arg.as_ptr();
                (lib.dbus_message_iter_append_basic)(&mut args, DBUS_TYPE_STRING, (&ptr as *const *const c_char).cast());
            }
            let mut serial = 0u32;
            if (lib.dbus_connection_send)(self.conn, msg, &mut serial) != 0 {
                self.pending = Some((serial, method));
            }
            (lib.dbus_message_unref)(msg);
            (lib.dbus_connection_flush)(self.conn);
        }
    }

    /// Take an answer already sent, then wait for one for what is left of
    /// `FIRST_READ_WAIT`.
    fn wait_for_first_answer(&mut self) {
        if self.pending.is_some() {
            unsafe { (self.lib.dbus_connection_read_write)(self.conn, 0) };
            self.drain();
        }
        let left = first_read_wait_left(self.opened_at.elapsed());
        if !left.is_zero() {
            self.wait_for_read(left);
        }
    }

    /// Process bus traffic until the pending read is answered or `limit`
    /// passed; a missing portal answers with an error at once, so this
    /// only bounds a hung or slow-starting one.
    fn wait_for_read(&mut self, limit: std::time::Duration) {
        let deadline = std::time::Instant::now() + limit;
        while self.pending.is_some() {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                break;
            }
            let millis = c_int::try_from(left.as_millis().max(1)).unwrap_or(c_int::MAX);
            if unsafe { (self.lib.dbus_connection_read_write)(self.conn, millis) } == 0 {
                break;
            }
            self.drain();
        }
    }

    /// Handle every queued message; the new theme when it changed.
    fn drain(&mut self) -> Option<Theme> {
        let mut changed = None;
        loop {
            let msg = unsafe { (self.lib.dbus_connection_pop_message)(self.conn) };
            if msg.is_null() {
                return changed;
            }
            let incoming = unsafe { self.lib.classify(msg, self.pending.map(|(serial, _)| serial)) };
            unsafe { (self.lib.dbus_message_unref)(msg) };
            match incoming {
                Incoming::Answer(scheme) | Incoming::Changed(scheme) => {
                    if matches!(incoming, Incoming::Answer(_)) {
                        self.pending = None;
                    }
                    let theme = theme_from_scheme(scheme);
                    if theme != self.theme {
                        self.theme = theme;
                        changed = Some(theme.unwrap_or(Theme::Dark));
                    }
                }
                Incoming::ReadFailed => {
                    let next = self.pending.map_or(READ_METHODS.len(), |(_, method)| method + 1);
                    self.send_read(next);
                }
                Incoming::Other => {}
            }
        }
    }

    /// Read what the bus sent; `Err` once the connection is gone.
    fn dispatch(&mut self) -> Result<Option<Theme>, Disconnected> {
        unsafe { (self.lib.dbus_connection_read_write)(self.conn, 0) };
        let changed = self.drain();
        if unsafe { (self.lib.dbus_connection_get_is_connected)(self.conn) } == 0 {
            return Err(Disconnected);
        }
        Ok(changed)
    }
}

impl Drop for Portal {
    fn drop(&mut self) {
        unsafe {
            (self.lib.dbus_connection_close)(self.conn);
            (self.lib.dbus_connection_unref)(self.conn);
        }
    }
}

/// The session bus connection closed.
#[derive(Debug, PartialEq, Eq)]
pub struct Disconnected;

enum State {
    Unopened,
    Open(Box<Portal>),
    /// No portal, or its bus went away: the last theme it reported stays.
    Closed(Option<Theme>),
}

static PORTAL: Mutex<State> = Mutex::new(State::Unopened);

fn with_portal<R>(f: impl FnOnce(&mut Portal) -> R) -> Option<R> {
    let mut state = PORTAL.lock();
    if matches!(*state, State::Unopened) {
        *state = Portal::open().map_or(State::Closed(None), |p| State::Open(Box::new(p)));
    }
    match &mut *state {
        State::Open(portal) => Some(f(portal)),
        _ => None,
    }
}

/// The desktop's light/dark preference; `None` without a settings portal,
/// before its first answer, or when it states no preference.
pub fn system_theme() -> Option<Theme> {
    with_portal(|p| {
        p.wait_for_first_answer();
        p.theme
    })
    .unwrap_or_else(|| match &*PORTAL.lock() {
        State::Closed(theme) => *theme,
        _ => None,
    })
}

/// The bus socket to watch for preference changes.
pub fn watch_fd() -> Option<RawFd> {
    with_portal(|p| p.fd)
}

/// Handle bus traffic after `watch_fd` became readable: the new theme when
/// the preference changed (dark again when it no longer states one).
/// `Err` once the bus is gone: the caller stops watching the socket, then
/// calls `close`.
pub fn dispatch() -> Result<Option<Theme>, Disconnected> {
    match &mut *PORTAL.lock() {
        State::Open(portal) => portal.dispatch(),
        _ => Err(Disconnected),
    }
}

/// Forget the connection state, so the next call opens the portal again.
#[cfg(test)]
pub fn reopen_for_test() {
    *PORTAL.lock() = State::Unopened;
}

/// Drop the connection (and its socket), keeping the last known theme.
pub fn close() {
    let mut state = PORTAL.lock();
    let theme = match &*state {
        State::Open(portal) => portal.theme,
        State::Closed(theme) => *theme,
        State::Unopened => None,
    };
    *state = State::Closed(theme);
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    #[test]
    fn color_scheme_values_map_to_themes() {
        assert_eq!(theme_from_scheme(1), Some(Theme::Dark));
        assert_eq!(theme_from_scheme(2), Some(Theme::Light));
        // No preference keeps the app's dark default (the portal-gtk
        // default on non-GNOME desktops, and GNOME's "Default" appearance).
        assert_eq!(theme_from_scheme(0), None);
        assert_eq!(theme_from_scheme(3), None);
    }

    #[test]
    fn the_bus_address_never_needs_autolaunch() {
        let os = |s: &str| Some(std::ffi::OsString::from(s));
        let yes = |_: &std::path::Path| true;
        let no = |_: &std::path::Path| false;
        assert_eq!(session_bus_address(os("unix:path=/x"), os("/run/user/1"), no).as_deref(), Some(c"unix:path=/x"));
        assert_eq!(session_bus_address(None, os("/run/user/1"), yes).as_deref(), Some(c"unix:path=/run/user/1/bus"));
        assert_eq!(session_bus_address(os(""), os("/run/user/1"), yes).as_deref(), Some(c"unix:path=/run/user/1/bus"));
        // No address and no user bus socket: no portal rather than autolaunch.
        assert_eq!(session_bus_address(None, os("/run/user/1"), no), None);
        assert_eq!(session_bus_address(None, None, yes), None);
    }

    fn lib() -> Option<DBusLib> {
        let lib = DBusLib::load();
        if lib.is_none() {
            eprintln!("libdbus-1 not installed; skipping");
        }
        lib
    }

    /// Append a `u32` wrapped in `depth` variants.
    unsafe fn append_scheme(lib: &DBusLib, iter: &mut DBusMessageIter, depth: usize, value: u32) {
        unsafe {
            if depth == 0 {
                (lib.dbus_message_iter_append_basic)(iter, DBUS_TYPE_UINT32, (&value as *const u32).cast());
                return;
            }
            let sig = if depth == 1 { c"u" } else { c"v" };
            let mut sub = DBusMessageIter::new();
            (lib.dbus_message_iter_open_container)(iter, DBUS_TYPE_VARIANT, sig.as_ptr(), &mut sub);
            append_scheme(lib, &mut sub, depth - 1, value);
            (lib.dbus_message_iter_close_container)(iter, &mut sub);
        }
    }

    unsafe fn append_str(lib: &DBusLib, iter: &mut DBusMessageIter, s: &CStr) {
        let ptr = s.as_ptr();
        unsafe { (lib.dbus_message_iter_append_basic)(iter, DBUS_TYPE_STRING, (&ptr as *const *const c_char).cast()) };
    }

    unsafe fn setting_changed(lib: &DBusLib, namespace: &CStr, key: &CStr, value: u32) -> *mut Msg {
        unsafe {
            let msg = (lib.dbus_message_new_signal)(PORTAL_PATH.as_ptr(), SETTINGS_IFACE.as_ptr(), SETTING_CHANGED.as_ptr());
            let mut iter = DBusMessageIter::new();
            (lib.dbus_message_iter_init_append)(msg, &mut iter);
            append_str(lib, &mut iter, namespace);
            append_str(lib, &mut iter, key);
            append_scheme(lib, &mut iter, 1, value);
            msg
        }
    }

    /// `ReadOne` replies `v(u)`, the older `Read` replies `v(v(u))`.
    #[test]
    fn replies_are_read_through_one_or_two_variants() {
        let Some(lib) = lib() else { return };
        unsafe {
            let call = (lib.dbus_message_new_method_call)(
                PORTAL_DEST.as_ptr(), PORTAL_PATH.as_ptr(), SETTINGS_IFACE.as_ptr(), c"Read".as_ptr(),
            );
            // A reply needs the serial the bus would have given the call.
            (lib.dbus_message_set_serial)(call, 1);
            for (depth, value) in [(1, 1), (2, 2)] {
                let reply = (lib.dbus_message_new_method_return)(call);
                let mut iter = DBusMessageIter::new();
                (lib.dbus_message_iter_init_append)(reply, &mut iter);
                append_scheme(&lib, &mut iter, depth, value);
                assert_eq!(lib.scheme_in_reply(reply), Some(value), "depth {depth}");
                (lib.dbus_message_unref)(reply);
            }
            let empty = (lib.dbus_message_new_method_return)(call);
            assert_eq!(lib.scheme_in_reply(empty), None);
            (lib.dbus_message_unref)(empty);
            (lib.dbus_message_unref)(call);
        }
    }

    /// Only `org.freedesktop.appearance` `color-scheme` changes the theme;
    /// the portal also signals accent colours, fonts and other namespaces.
    #[test]
    fn only_color_scheme_signals_are_read() {
        let Some(lib) = lib() else { return };
        unsafe {
            let scheme = setting_changed(&lib, APPEARANCE_NS, COLOR_SCHEME_KEY, 1);
            assert_eq!(lib.scheme_in_signal(scheme), Some(1));
            (lib.dbus_message_unref)(scheme);

            let accent = setting_changed(&lib, APPEARANCE_NS, c"contrast", 1);
            assert_eq!(lib.scheme_in_signal(accent), None);
            (lib.dbus_message_unref)(accent);

            let other = setting_changed(&lib, c"org.gnome.desktop.interface", COLOR_SCHEME_KEY, 1);
            assert_eq!(lib.scheme_in_signal(other), None);
            (lib.dbus_message_unref)(other);
        }
    }

    /// A method call with the serial the bus would have given it.
    unsafe fn read_call(lib: &DBusLib, serial: u32) -> *mut Msg {
        unsafe {
            let call = (lib.dbus_message_new_method_call)(
                PORTAL_DEST.as_ptr(), PORTAL_PATH.as_ptr(), SETTINGS_IFACE.as_ptr(), c"ReadOne".as_ptr(),
            );
            (lib.dbus_message_set_serial)(call, serial);
            call
        }
    }

    /// Only the reply to the pending read answers it; an error reply (no
    /// `ReadOne` on an old portal) or a malformed value fails the read, so
    /// the next method is tried.
    #[test]
    fn replies_answer_only_the_pending_read() {
        let Some(lib) = lib() else { return };
        unsafe {
            let call = read_call(&lib, 7);
            let reply = (lib.dbus_message_new_method_return)(call);
            let mut iter = DBusMessageIter::new();
            (lib.dbus_message_iter_init_append)(reply, &mut iter);
            append_scheme(&lib, &mut iter, 1, 2);
            assert_eq!(lib.classify(reply, Some(7)), Incoming::Answer(2));
            assert_eq!(lib.classify(reply, Some(8)), Incoming::Other, "a reply to another call");
            assert_eq!(lib.classify(reply, None), Incoming::Other);
            (lib.dbus_message_unref)(reply);

            let error = (lib.dbus_message_new_error)(call, c"org.freedesktop.DBus.Error.UnknownMethod".as_ptr(), c"no".as_ptr());
            assert_eq!(lib.classify(error, Some(7)), Incoming::ReadFailed);
            (lib.dbus_message_unref)(error);

            let wrong = (lib.dbus_message_new_method_return)(call);
            let mut iter = DBusMessageIter::new();
            (lib.dbus_message_iter_init_append)(wrong, &mut iter);
            append_str(&lib, &mut iter, c"dark");
            assert_eq!(lib.classify(wrong, Some(7)), Incoming::ReadFailed);
            (lib.dbus_message_unref)(wrong);
            (lib.dbus_message_unref)(call);

            let signal = setting_changed(&lib, APPEARANCE_NS, COLOR_SCHEME_KEY, 1);
            assert_eq!(lib.classify(signal, Some(7)), Incoming::Changed(1));
            (lib.dbus_message_unref)(signal);
        }
    }

    /// Tests that swap the process-wide `PORTAL` state run one at a time.
    pub(in crate::platform::linux::native) static STATE_TESTS: Mutex<()> = Mutex::new(());

    /// A bus that went away keeps the theme it last reported instead of
    /// flipping the windows back to dark.
    #[test]
    fn closing_keeps_the_last_theme() {
        let _serial = STATE_TESTS.lock();
        let saved = std::mem::replace(&mut *PORTAL.lock(), State::Closed(Some(Theme::Light)));
        close();
        assert_eq!(system_theme(), Some(Theme::Light));
        assert_eq!(dispatch(), Err(Disconnected));
        *PORTAL.lock() = saved;
    }

    /// A window that asks after the first wait ran out still gets the
    /// answer already on the socket, instead of opening in the dark default
    /// and flipping when the event loop reads it.
    #[test]
    fn a_late_first_read_takes_the_answer_already_sent() {
        let _serial = STATE_TESTS.lock();
        reopen_for_test();
        let answered = system_theme();
        reopen_for_test();
        if watch_fd().is_none() {
            eprintln!("no settings portal bus; skipping");
            return;
        }
        std::thread::sleep(FIRST_READ_WAIT * 3);
        assert_eq!(system_theme(), answered);
        reopen_for_test();
    }

    /// The first window waits for the portal's answer only for what is
    /// left of `FIRST_READ_WAIT` since the loop opened the portal, so the
    /// wait overlaps startup instead of adding to it.
    #[test]
    fn the_first_read_wait_counts_from_opening_the_portal() {
        use std::time::Duration;
        assert_eq!(first_read_wait_left(Duration::ZERO), FIRST_READ_WAIT);
        assert_eq!(first_read_wait_left(Duration::from_millis(30)), FIRST_READ_WAIT - Duration::from_millis(30));
        assert_eq!(first_read_wait_left(FIRST_READ_WAIT), Duration::ZERO);
        assert_eq!(first_read_wait_left(Duration::from_secs(5)), Duration::ZERO);
    }
}
