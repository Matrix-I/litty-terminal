//! Native X11 window backend via dynamically loaded `libX11.so.6`.

use std::ffi::{c_char, c_int, c_long, c_ulong, c_void, CStr, CString};
use std::sync::Arc;

use crate::platform::gui::{PhysicalPosition, PhysicalSize, RawHandle, WindowEvent, WindowSpec};
use super::events::{convert_linux_key, keysym_char, mods_after_key, mods_from_state, x11_button_event, HeldKeys};
use crate::platform::input::{Key, KeyLocation, Mods};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Once, OnceLock};

#[repr(C)]
pub struct XEvent {
    pub type_: c_int,
    pub pad: [c_long; 24],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct XKeyEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: c_int,
    pub display: *mut c_void,
    pub window: c_ulong,
    pub root: c_ulong,
    pub subwindow: c_ulong,
    pub time: c_ulong,
    pub x: c_int,
    pub y: c_int,
    pub x_root: c_int,
    pub y_root: c_int,
    pub state: c_int,
    pub keycode: c_int,
    pub same_screen: c_int,
}

#[repr(C)]
pub struct XButtonEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: c_int,
    pub display: *mut c_void,
    pub window: c_ulong,
    pub root: c_ulong,
    pub subwindow: c_ulong,
    pub time: c_ulong,
    pub x: c_int,
    pub y: c_int,
    pub x_root: c_int,
    pub y_root: c_int,
    pub state: c_int,
    pub button: c_int,
    pub same_screen: c_int,
}

#[repr(C)]
pub struct XMotionEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: c_int,
    pub display: *mut c_void,
    pub window: c_ulong,
    pub root: c_ulong,
    pub subwindow: c_ulong,
    pub time: c_ulong,
    pub x: c_int,
    pub y: c_int,
    pub x_root: c_int,
    pub y_root: c_int,
    pub state: c_int,
    pub is_hint: c_char,
    pub same_screen: c_int,
}

#[repr(C)]
pub struct XConfigureEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: c_int,
    pub display: *mut c_void,
    pub event: c_ulong,
    pub window: c_ulong,
    pub x: c_int,
    pub y: c_int,
    pub width: c_int,
    pub height: c_int,
    pub border_width: c_int,
    pub above: c_ulong,
    pub override_redirect: c_int,
}

#[repr(C)]
pub struct XClientMessageEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: c_int,
    pub display: *mut c_void,
    pub window: c_ulong,
    pub message_type: c_ulong,
    pub format: c_int,
    pub data: [c_long; 5],
}

#[repr(C)]
pub struct XErrorEvent {
    pub type_: c_int,
    pub display: *mut c_void,
    pub resourceid: c_ulong,
    pub serial: c_ulong,
    pub error_code: u8,
    pub request_code: u8,
    pub minor_code: u8,
}

/// `PropModeReplace` for `XChangeProperty`.
const PROP_MODE_REPLACE: c_int = 0;
/// Predefined `CARDINAL` atom.
const XA_CARDINAL: c_ulong = 6;
/// Predefined `STRING` atom.
const XA_STRING: c_ulong = 31;
/// Predefined `WM_CLASS` atom.
const XA_WM_CLASS: c_ulong = 67;
/// `WM_CLASS` value (instance and class, each NUL-terminated). Must match
/// `StartupWMClass` and `Icon` in `litty.desktop`, or the desktop cannot
/// tie the window to the launcher and shows a generic icon.
const WM_CLASS: &[u8] = b"litty\0litty\0";

type XErrorHandler = unsafe extern "C" fn(*mut c_void, *mut XErrorEvent) -> c_int;
type XIOErrorHandler = unsafe extern "C" fn(*mut c_void) -> c_int;

/// Xlib's default handler calls `exit()` on any protocol error (e.g. a
/// `BadWindow` racing teardown); log it and keep running instead.
unsafe extern "C" fn log_x_error(_display: *mut c_void, event: *mut XErrorEvent) -> c_int {
    // SAFETY: Xlib passes a valid event for the duration of the call.
    let e = unsafe { &*event };
    eprintln!(
        "litty: X11 error {} (request {}.{}, resource {:#x})",
        e.error_code, e.request_code, e.minor_code, e.resourceid
    );
    0
}

/// The server connection is gone; Xlib exits once this returns, so leave a
/// reason in the log rather than a silent exit.
unsafe extern "C" fn log_x_io_error(_display: *mut c_void) -> c_int {
    eprintln!("litty: lost the connection to the X server");
    0
}

/// `xkb_keysym_to_utf8` from libxkbcommon, when installed: maps every
/// legacy keysym (Cyrillic, Greek, Latin-2, ...) to text.
fn xkb_keysym_to_utf8() -> Option<unsafe extern "C" fn(u32, *mut c_char, usize) -> c_int> {
    static SYM: OnceLock<Option<unsafe extern "C" fn(u32, *mut c_char, usize) -> c_int>> = OnceLock::new();
    *SYM.get_or_init(|| unsafe {
        let handle = libc::dlopen(c"libxkbcommon.so.0".as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
        if handle.is_null() {
            return None;
        }
        let p = libc::dlsym(handle, c"xkb_keysym_to_utf8".as_ptr());
        (!p.is_null()).then(|| std::mem::transmute_copy(&p))
    })
}

/// Text a keysym produces, or `None` for function keys and control codes.
fn keysym_text(keysym: c_ulong) -> Option<String> {
    let keysym = u32::try_from(keysym).ok()?;
    let text = match xkb_keysym_to_utf8() {
        Some(to_utf8) => {
            let mut buf = [0u8; 8];
            // Returns bytes written including the NUL, 0 when there is no text.
            let n = unsafe { to_utf8(keysym, buf.as_mut_ptr().cast(), buf.len()) };
            if n <= 1 {
                return None;
            }
            std::str::from_utf8(&buf[..n as usize - 1]).ok()?.to_owned()
        }
        None => keysym_char(keysym)?.to_string(),
    };
    Some(text).filter(|t| t.chars().all(|c| !c.is_control()))
}

#[allow(non_snake_case)]
pub struct X11Lib {
    _handle: *mut c_void,
    pub XOpenDisplay: unsafe extern "C" fn(*const c_char) -> *mut c_void,
    pub XCloseDisplay: unsafe extern "C" fn(*mut c_void) -> c_int,
    pub XDefaultScreen: unsafe extern "C" fn(*mut c_void) -> c_int,
    pub XRootWindow: unsafe extern "C" fn(*mut c_void, c_int) -> c_ulong,
    pub XCreateSimpleWindow: unsafe extern "C" fn(*mut c_void, c_ulong, c_int, c_int, c_int, c_int, c_int, c_ulong, c_ulong) -> c_ulong,
    pub XDestroyWindow: unsafe extern "C" fn(*mut c_void, c_ulong) -> c_int,
    pub XMapWindow: unsafe extern "C" fn(*mut c_void, c_ulong) -> c_int,
    pub XMapRaised: unsafe extern "C" fn(*mut c_void, c_ulong) -> c_int,
    pub XWithdrawWindow: unsafe extern "C" fn(*mut c_void, c_ulong, c_int) -> c_int,
    pub XMoveWindow: unsafe extern "C" fn(*mut c_void, c_ulong, c_int, c_int) -> c_int,
    pub XRaiseWindow: unsafe extern "C" fn(*mut c_void, c_ulong) -> c_int,
    pub XSetErrorHandler: unsafe extern "C" fn(Option<XErrorHandler>) -> Option<XErrorHandler>,
    pub XSetIOErrorHandler: unsafe extern "C" fn(Option<XIOErrorHandler>) -> Option<XIOErrorHandler>,
    pub XkbSetDetectableAutoRepeat: unsafe extern "C" fn(*mut c_void, c_int, *mut c_int) -> c_int,
    pub XSelectInput: unsafe extern "C" fn(*mut c_void, c_ulong, c_long) -> c_int,
    pub XFlush: unsafe extern "C" fn(*mut c_void) -> c_int,
    pub XConnectionNumber: unsafe extern "C" fn(*mut c_void) -> c_int,
    pub XPending: unsafe extern "C" fn(*mut c_void) -> c_int,
    pub XNextEvent: unsafe extern "C" fn(*mut c_void, *mut XEvent) -> c_int,
    pub XStoreName: unsafe extern "C" fn(*mut c_void, c_ulong, *const c_char) -> c_int,
    pub XInternAtom: unsafe extern "C" fn(*mut c_void, *const c_char, c_int) -> c_ulong,
    pub XSetWMProtocols: unsafe extern "C" fn(*mut c_void, c_ulong, *mut c_ulong, c_int) -> c_int,
    pub XChangeProperty: unsafe extern "C" fn(*mut c_void, c_ulong, c_ulong, c_ulong, c_int, c_int, *const u8, c_int) -> c_int,
    pub XSendEvent: unsafe extern "C" fn(*mut c_void, c_ulong, c_int, c_long, *mut XEvent) -> c_int,
    pub XIconifyWindow: unsafe extern "C" fn(*mut c_void, c_ulong, c_int) -> c_int,
    pub XUngrabPointer: unsafe extern "C" fn(*mut c_void, c_ulong) -> c_int,
    pub XResourceManagerString: unsafe extern "C" fn(*mut c_void) -> *const c_char,
    pub XLookupString: unsafe extern "C" fn(*mut XKeyEvent, *mut c_char, c_int, *mut c_ulong, *mut c_void) -> c_int,
    pub XRefreshKeyboardMapping: unsafe extern "C" fn(*mut XEvent) -> c_int,
    #[allow(clippy::type_complexity)]
    pub XQueryPointer: unsafe extern "C" fn(
        *mut c_void, c_ulong, *mut c_ulong, *mut c_ulong,
        *mut c_int, *mut c_int, *mut c_int, *mut c_int, *mut std::ffi::c_uint,
    ) -> c_int,
}

unsafe impl Send for X11Lib {}
unsafe impl Sync for X11Lib {}

impl X11Lib {
    pub fn load() -> Option<Self> {
        let names = [c"libX11.so.6", c"libX11.so"];
        let mut handle = std::ptr::null_mut();
        for name in &names {
            handle = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL) };
            if !handle.is_null() {
                break;
            }
        }
        if handle.is_null() {
            return None;
        }

        unsafe fn sym<T>(h: *mut c_void, s: &CStr) -> Option<T> {
            let p = unsafe { libc::dlsym(h, s.as_ptr()) };
            if p.is_null() { None } else { Some(unsafe { std::mem::transmute_copy(&p) }) }
        }

        unsafe {
            Some(Self {
                _handle: handle,
                XOpenDisplay: sym(handle, c"XOpenDisplay")?,
                XCloseDisplay: sym(handle, c"XCloseDisplay")?,
                XDefaultScreen: sym(handle, c"XDefaultScreen")?,
                XRootWindow: sym(handle, c"XRootWindow")?,
                XCreateSimpleWindow: sym(handle, c"XCreateSimpleWindow")?,
                XDestroyWindow: sym(handle, c"XDestroyWindow")?,
                XMapWindow: sym(handle, c"XMapWindow")?,
                XMapRaised: sym(handle, c"XMapRaised")?,
                XWithdrawWindow: sym(handle, c"XWithdrawWindow")?,
                XMoveWindow: sym(handle, c"XMoveWindow")?,
                XRaiseWindow: sym(handle, c"XRaiseWindow")?,
                XSetErrorHandler: sym(handle, c"XSetErrorHandler")?,
                XSetIOErrorHandler: sym(handle, c"XSetIOErrorHandler")?,
                XkbSetDetectableAutoRepeat: sym(handle, c"XkbSetDetectableAutoRepeat")?,
                XSelectInput: sym(handle, c"XSelectInput")?,
                XFlush: sym(handle, c"XFlush")?,
                XConnectionNumber: sym(handle, c"XConnectionNumber")?,
                XPending: sym(handle, c"XPending")?,
                XNextEvent: sym(handle, c"XNextEvent")?,
                XStoreName: sym(handle, c"XStoreName")?,
                XInternAtom: sym(handle, c"XInternAtom")?,
                XSetWMProtocols: sym(handle, c"XSetWMProtocols")?,
                XChangeProperty: sym(handle, c"XChangeProperty")?,
                XSendEvent: sym(handle, c"XSendEvent")?,
                XIconifyWindow: sym(handle, c"XIconifyWindow")?,
                XUngrabPointer: sym(handle, c"XUngrabPointer")?,
                XQueryPointer: sym(handle, c"XQueryPointer")?,
                XResourceManagerString: sym(handle, c"XResourceManagerString")?,
                XLookupString: sym(handle, c"XLookupString")?,
                XRefreshKeyboardMapping: sym(handle, c"XRefreshKeyboardMapping")?,
            })
        }
    }
}

pub struct X11WindowInner {
    pub lib: Arc<X11Lib>,
    pub display: *mut c_void,
    pub window: c_ulong,
    pub fd: i32,
    pub wm_delete_window: c_ulong,
    /// `_NET_WM_NAME` and `UTF8_STRING`, interned once: `set_title` runs on
    /// every OSC title change and interning is a server round trip.
    pub net_wm_name: c_ulong,
    pub utf8_string: c_ulong,
    pub root: c_ulong,
    pub screen: c_int,
    pub scale: f64,
    /// Last modifiers reported to the app, packed by `pack_mods`.
    pub mods: AtomicU8,
    /// Keys held down, to flag auto-repeat presses.
    pub held: HeldKeys,
}

unsafe impl Send for X11WindowInner {}
unsafe impl Sync for X11WindowInner {}

impl Drop for X11WindowInner {
    fn drop(&mut self) {
        super::app::deregister_display_fd(self.fd);
        unsafe {
            (self.lib.XDestroyWindow)(self.display, self.window);
            (self.lib.XCloseDisplay)(self.display);
        }
    }
}

/// Publish `title` as `WM_NAME` and as `_NET_WM_NAME`. EWMH window
/// managers read the latter as UTF-8; `WM_NAME` is Latin-1, so non-ASCII
/// titles show garbled when it is the only one set.
///
/// # Safety
/// `display` and `window` must be live handles from `lib`.
unsafe fn store_title(lib: &X11Lib, display: *mut c_void, window: c_ulong, net_wm_name: c_ulong, utf8_string: c_ulong, title: &str) {
    let Ok(c_title) = CString::new(title) else {
        return;
    };
    let Ok(len) = c_int::try_from(title.len()) else {
        return;
    };
    unsafe {
        (lib.XStoreName)(display, window, c_title.as_ptr());
        (lib.XChangeProperty)(display, window, net_wm_name, utf8_string, 8, PROP_MODE_REPLACE, title.as_ptr(), len);
    }
}

/// Scale factor from the `Xft.dpi` X resource (96 dpi = 1.0), as GNOME,
/// KDE and XWayland's native scaling publish it; 1.0 when unset or invalid.
fn scale_from_resources(resources: &str) -> f64 {
    resources
        .lines()
        .find_map(|line| line.strip_prefix("Xft.dpi:"))
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|dpi| dpi.is_finite() && *dpi > 0.0)
        .map_or(1.0, |dpi| dpi / 96.0)
}

#[derive(Clone)]
pub struct X11Window(pub Arc<X11WindowInner>);

impl X11Window {
    pub fn create(spec: &WindowSpec) -> Option<Self> {
        let lib = Arc::new(X11Lib::load()?);
        static ERROR_HANDLERS: Once = Once::new();
        ERROR_HANDLERS.call_once(|| unsafe {
            (lib.XSetErrorHandler)(Some(log_x_error));
            (lib.XSetIOErrorHandler)(Some(log_x_io_error));
        });
        unsafe {
            let display = (lib.XOpenDisplay)(std::ptr::null());
            if display.is_null() {
                return None;
            }
            let screen = (lib.XDefaultScreen)(display);
            let root = (lib.XRootWindow)(display, screen);

            let resources = (lib.XResourceManagerString)(display);
            let scale = if resources.is_null() {
                1.0
            } else {
                scale_from_resources(&CStr::from_ptr(resources).to_string_lossy())
            };

            // The spec is in logical pixels; X11 windows are physical.
            let width = (spec.inner_size.width.max(100.0) * scale).round() as c_int;
            let height = (spec.inner_size.height.max(100.0) * scale).round() as c_int;
            let x = spec.position.map_or(100, |p| (p.x * scale).round() as c_int);
            let y = spec.position.map_or(100, |p| (p.y * scale).round() as c_int);

            let window = (lib.XCreateSimpleWindow)(display, root, x, y, width, height, 0, 0, 0);
            if window == 0 {
                (lib.XCloseDisplay)(display);
                return None;
            }

            // Select inputs:
            // KeyPress (1), KeyRelease (2), ButtonPress (4), ButtonRelease (8), PointerMotion (64)
            // Exposure (32768), StructureNotify (131072), FocusChange (2097152)
            let mask = 1 | 2 | 4 | 8 | 64 | 32768 | 131072 | 2097152;
            (lib.XSelectInput)(display, window, mask);

            let net_wm_name = (lib.XInternAtom)(display, c"_NET_WM_NAME".as_ptr(), 0);
            let utf8_string = (lib.XInternAtom)(display, c"UTF8_STRING".as_ptr(), 0);
            store_title(&lib, display, window, net_wm_name, utf8_string, spec.title);

            // Enable graceful close via WM_DELETE_WINDOW
            let wm_delete_window = (lib.XInternAtom)(display, c"WM_DELETE_WINDOW".as_ptr(), 0);
            let mut protocols = [wm_delete_window];
            (lib.XSetWMProtocols)(display, window, protocols.as_mut_ptr(), 1);

            // Without server decorations the app draws its own title bar
            // (CSD); leaving them on shows two title bars.
            if !spec.decorations {
                let hints = c"_MOTIF_WM_HINTS".as_ptr();
                let atom = (lib.XInternAtom)(display, hints, 0);
                // flags = MWM_HINTS_DECORATIONS, decorations = 0
                let data: [c_long; 5] = [2, 0, 0, 0, 0];
                (lib.XChangeProperty)(display, window, atom, atom, 32, PROP_MODE_REPLACE, data.as_ptr().cast(), 5);
            }

            // `_NET_WM_PID`: lets window managers and tools (xdotool) map
            // the window to this process, as winit did.
            let pid_atom = (lib.XInternAtom)(display, c"_NET_WM_PID".as_ptr(), 0);
            let pid = [c_long::from(libc::getpid())];
            (lib.XChangeProperty)(display, window, pid_atom, XA_CARDINAL, 32, PROP_MODE_REPLACE, pid.as_ptr().cast(), 1);

            (lib.XChangeProperty)(
                display, window, XA_WM_CLASS, XA_STRING, 8, PROP_MODE_REPLACE,
                WM_CLASS.as_ptr(), WM_CLASS.len() as c_int,
            );

            // Auto-repeat as press, press, ..., release (no fake releases),
            // so repeats are detectable and Kitty sees no spurious releases.
            let mut supported = 0;
            (lib.XkbSetDetectableAutoRepeat)(display, 1, &mut supported);

            if spec.visible {
                (lib.XMapWindow)(display, window);
            }
            (lib.XFlush)(display);

            let fd = (lib.XConnectionNumber)(display);

            let win = Self(Arc::new(X11WindowInner {
                lib,
                display,
                window,
                fd,
                wm_delete_window,
                net_wm_name,
                utf8_string,
                root,
                screen,
                scale,
                mods: AtomicU8::new(0),
                held: HeldKeys::default(),
            }));

            Some(win)
        }
    }

    pub fn scale(&self) -> f64 {
        self.0.scale
    }

    pub fn fd(&self) -> i32 {
        self.0.fd
    }

    pub fn pending(&self) -> bool {
        unsafe { (self.0.lib.XPending)(self.0.display) > 0 }
    }

    pub fn set_title(&self, title: &str) {
        let w = &self.0;
        unsafe {
            store_title(&w.lib, w.display, w.window, w.net_wm_name, w.utf8_string, title);
            (w.lib.XFlush)(w.display);
        }
    }

    /// Send an EWMH client message about this window to the root window.
    fn send_wm_message(&self, message_type: &CStr, data: [c_long; 5]) {
        // SubstructureNotifyMask | SubstructureRedirectMask
        const MASK: c_long = (1 << 19) | (1 << 20);
        unsafe {
            let lib = &self.0.lib;
            let mut xev: XEvent = std::mem::zeroed();
            let cm = &mut *(&mut xev as *mut XEvent).cast::<XClientMessageEvent>();
            cm.type_ = 33; // ClientMessage
            cm.send_event = 1;
            cm.display = self.0.display;
            cm.window = self.0.window;
            cm.message_type = (lib.XInternAtom)(self.0.display, message_type.as_ptr(), 0);
            cm.format = 32;
            cm.data = data;
            (lib.XSendEvent)(self.0.display, self.0.root, 0, MASK, &mut xev);
            (lib.XFlush)(self.0.display);
        }
    }

    /// Hand an interactive move (`direction == 8`) or edge resize to the
    /// window manager (`_NET_WM_MOVERESIZE`), as the left button is down.
    pub fn begin_move_resize(&self, direction: c_long) {
        unsafe {
            let lib = &self.0.lib;
            let (mut root_ret, mut child) = (0, 0);
            let (mut rx, mut ry, mut wx, mut wy) = (0, 0, 0, 0);
            let mut mask = 0;
            (lib.XQueryPointer)(
                self.0.display, self.0.window, &mut root_ret, &mut child,
                &mut rx, &mut ry, &mut wx, &mut wy, &mut mask,
            );
            // The press holds an implicit pointer grab the WM needs.
            (lib.XUngrabPointer)(self.0.display, 0);
            self.send_wm_message(c"_NET_WM_MOVERESIZE", [rx as c_long, ry as c_long, direction, 1, 1]);
        }
    }

    pub fn set_maximized(&self, maximized: bool) {
        let atom = |name: &CStr| unsafe { (self.0.lib.XInternAtom)(self.0.display, name.as_ptr(), 0) as c_long };
        let vert = atom(c"_NET_WM_STATE_MAXIMIZED_VERT");
        let horz = atom(c"_NET_WM_STATE_MAXIMIZED_HORZ");
        self.send_wm_message(c"_NET_WM_STATE", [c_long::from(maximized), vert, horz, 1, 0]);
    }

    /// Show (mapped and raised) or withdraw the window.
    pub fn set_visible(&self, visible: bool) {
        unsafe {
            if visible {
                (self.0.lib.XMapRaised)(self.0.display, self.0.window);
            } else {
                (self.0.lib.XWithdrawWindow)(self.0.display, self.0.window, self.0.screen);
            }
            (self.0.lib.XFlush)(self.0.display);
        }
    }

    /// Move the window's top-left corner to physical `(x, y)`.
    pub fn move_to(&self, x: i32, y: i32) {
        unsafe {
            (self.0.lib.XMoveWindow)(self.0.display, self.0.window, x, y);
            (self.0.lib.XFlush)(self.0.display);
        }
    }

    /// Raise and ask the window manager to activate the window.
    pub fn focus(&self) {
        unsafe {
            (self.0.lib.XRaiseWindow)(self.0.display, self.0.window);
        }
        // Source 1: a normal application request; time 0 = CurrentTime.
        self.send_wm_message(c"_NET_ACTIVE_WINDOW", [1, 0, 0, 0, 0]);
    }

    /// Set or clear the urgency (`_NET_WM_STATE_DEMANDS_ATTENTION`) hint.
    pub fn request_attention(&self, requested: bool) {
        let atom = unsafe {
            (self.0.lib.XInternAtom)(self.0.display, c"_NET_WM_STATE_DEMANDS_ATTENTION".as_ptr(), 0) as c_long
        };
        self.send_wm_message(c"_NET_WM_STATE", [c_long::from(requested), atom, 0, 1, 0]);
    }

    pub fn iconify(&self) {
        unsafe {
            (self.0.lib.XIconifyWindow)(self.0.display, self.0.window, self.0.screen);
            (self.0.lib.XFlush)(self.0.display);
        }
    }

    /// Queue `ModifiersChanged` when the held modifiers differ from the last report.
    fn report_mods(&self, mods: Mods, events: &mut Vec<WindowEvent>) {
        let packed = u8::from(mods.ctrl) | u8::from(mods.alt) << 1 | u8::from(mods.shift) << 2 | u8::from(mods.sup) << 3;
        if self.0.mods.swap(packed, Ordering::Relaxed) != packed {
            events.push(WindowEvent::ModifiersChanged(mods));
        }
    }

    /// Text the key produces under `state`, through the X keyboard layout.
    fn lookup(&self, kev: &XKeyEvent, state: u32) -> Option<String> {
        let mut ev = XKeyEvent { state: state as c_int, ..*kev };
        let mut buf = [0 as c_char; 16];
        let mut keysym: c_ulong = 0;
        unsafe {
            (self.0.lib.XLookupString)(&mut ev, buf.as_mut_ptr(), buf.len() as c_int, &mut keysym, std::ptr::null_mut());
        }
        // The returned bytes are Latin-1 only; the keysym covers every layout.
        keysym_text(keysym)
    }

    pub fn raw_handle(&self) -> RawHandle {
        RawHandle::Xlib {
            display: self.0.display,
            window: self.0.window,
        }
    }

    /// Append this connection's queued events to `events`.
    pub fn poll_events(&self, events: &mut Vec<WindowEvent>) {
        unsafe {
            while (self.0.lib.XPending)(self.0.display) > 0 {
                let mut xev: XEvent = std::mem::zeroed();
                (self.0.lib.XNextEvent)(self.0.display, &mut xev);

                match xev.type_ {
                    // KeyPress = 2, KeyRelease = 3
                    2 | 3 => {
                        let kev: &XKeyEvent = std::mem::transmute(&xev);
                        let pressed = xev.type_ == 2;
                        // X11 keycodes on Linux are evdev scancode + 8
                        let evdev_code = (kev.keycode as u32).saturating_sub(8);
                        let state = kev.state as u32;
                        self.report_mods(mods_after_key(state, evdev_code, pressed), events);
                        let repeat = self.0.held.update(kev.keycode as u8, pressed);
                        let mut key_input = convert_linux_key(evdev_code, state, pressed, repeat);
                        // Keys outside the evdev table (other layouts' extra
                        // keys, remapped keycodes) still produce layout text.
                        if !matches!(key_input.logical, Key::Named(_)) {
                            // Layout-aware text: Shift (and AltGr) applied,
                            // Ctrl/Alt/Super left to the keymap. This keeps
                            // a layout's own keypad symbols (KP_Separator).
                            const CTRL_ALT_SUPER: u32 = 4 | 8 | 64;
                            if let Some(c) = self.lookup(kev, state & !CTRL_ALT_SUPER) {
                                key_input.logical = Key::Character(c.clone());
                                key_input.text = Some(c);
                            }
                            // A keypad key's base key is its Num Lock one,
                            // not the navigation key `state` 0 would give.
                            if key_input.location != KeyLocation::Numpad
                                && let Some(c) = self.lookup(kev, 0)
                            {
                                key_input.unshifted = Key::Character(c);
                            }
                        }
                        events.push(WindowEvent::Key(key_input));
                    }
                    // ButtonPress = 4, ButtonRelease = 5
                    4 | 5 => {
                        let bev: &XButtonEvent = std::mem::transmute(&xev);
                        self.report_mods(mods_from_state(bev.state as u32), events);
                        if let Some(ev) = x11_button_event(bev.button as u32, xev.type_ == 4) {
                            events.push(ev);
                        }
                    }
                    // MotionNotify = 6
                    6 => {
                        let mev: &XMotionEvent = std::mem::transmute(&xev);
                        events.push(WindowEvent::CursorMoved(PhysicalPosition::new(mev.x as f64, mev.y as f64)));
                    }
                    // Expose = 12
                    12 => {
                        events.push(WindowEvent::RedrawRequested);
                    }
                    // ConfigureNotify = 22
                    22 => {
                        let cev: &XConfigureEvent = std::mem::transmute(&xev);
                        events.push(WindowEvent::Resized(PhysicalSize::new(cev.width as u32, cev.height as u32)));
                    }
                    // FocusIn = 9, FocusOut = 10
                    9 | 10 => {
                        // Keys released while unfocused never report a release.
                        self.0.held.clear();
                        events.push(WindowEvent::Focused(xev.type_ == 9));
                    }
                    // MappingNotify = 34: the layout changed; without this
                    // `XLookupString` keeps resolving keysyms from the old map.
                    34 => {
                        (self.0.lib.XRefreshKeyboardMapping)(&mut xev);
                    }
                    // ClientMessage = 33
                    33 => {
                        let cmev: &XClientMessageEvent = std::mem::transmute(&xev);
                        if cmev.data[0] as c_ulong == self.0.wm_delete_window {
                            events.push(WindowEvent::CloseRequested);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_comes_from_xft_dpi() {
        assert_eq!(scale_from_resources("Xcursor.size:\t24\nXft.dpi:\t192\n"), 2.0);
        assert_eq!(scale_from_resources("Xft.dpi: 144"), 1.5);
        assert_eq!(scale_from_resources("Xft.antialias:\t1\n"), 1.0);
        assert_eq!(scale_from_resources("Xft.dpi:\tjunk\n"), 1.0);
        assert_eq!(scale_from_resources("Xft.dpi:\t0\n"), 1.0);
    }

    /// Every symbol `X11Lib` resolves must be a real libX11 export: one
    /// missing name (e.g. the `ConnectionNumber` macro) makes `load` return
    /// `None` and the app runs with no window at all.
    #[test]
    fn loads_all_symbols_when_libx11_is_installed() {
        let present = [c"libX11.so.6", c"libX11.so"].iter().any(|name| unsafe {
            let h = libc::dlopen(name.as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
            !h.is_null()
        });
        if !present {
            eprintln!("libX11 not installed; skipping");
            return;
        }
        assert!(X11Lib::load().is_some(), "libX11 is installed but a symbol failed to resolve");
    }

    /// The desktop shows `litty.desktop`'s icon only for windows whose
    /// `WM_CLASS` matches its `StartupWMClass` (GNOME, KDE, XWayland).
    #[test]
    fn new_window_has_the_desktop_entry_wm_class() {
        #[repr(C)]
        struct XClassHint {
            res_name: *mut c_char,
            res_class: *mut c_char,
        }
        if std::env::var_os("DISPLAY").is_none() {
            eprintln!("no X display; skipping");
            return;
        }
        let spec = WindowSpec {
            title: "litty-wm-class-test",
            decorations: true,
            visible: false,
            transparent: false,
            inner_size: crate::platform::gui::LogicalSize::new(200.0, 100.0),
            position: None,
        };
        let Some(win) = X11Window::create(&spec) else {
            eprintln!("X display unavailable; skipping");
            return;
        };
        let expected = include_str!("../../../../resources/linux/litty.desktop")
            .lines()
            .find_map(|l| l.strip_prefix("StartupWMClass="))
            .expect("litty.desktop sets StartupWMClass");
        let inner = &win.0;
        let (name, class) = unsafe {
            let h = libc::dlopen(c"libX11.so.6".as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
            assert!(!h.is_null());
            let get: unsafe extern "C" fn(*mut c_void, c_ulong, *mut XClassHint) -> c_int =
                std::mem::transmute(libc::dlsym(h, c"XGetClassHint".as_ptr()));
            let free: unsafe extern "C" fn(*mut c_void) -> c_int =
                std::mem::transmute(libc::dlsym(h, c"XFree".as_ptr()));
            let mut hint = XClassHint { res_name: std::ptr::null_mut(), res_class: std::ptr::null_mut() };
            let found = get(inner.display, inner.window, &mut hint) != 0;
            let read = |p: *mut c_char| {
                if p.is_null() {
                    return None;
                }
                let s = CStr::from_ptr(p).to_string_lossy().into_owned();
                free(p.cast());
                Some(s)
            };
            let (name, class) = (read(hint.res_name), read(hint.res_class));
            assert!(found, "window has no WM_CLASS");
            (name, class)
        };
        assert_eq!(name.as_deref(), Some(expected));
        assert_eq!(class.as_deref(), Some(expected));
    }

    /// `_NET_WM_NAME` of `win` as a `UTF8_STRING`, or `None` when unset.
    fn net_wm_name(win: &X11Window) -> Option<String> {
        type GetProperty = unsafe extern "C" fn(
            *mut c_void, c_ulong, c_ulong, c_long, c_long, c_int, c_ulong,
            *mut c_ulong, *mut c_int, *mut c_ulong, *mut c_ulong, *mut *mut u8,
        ) -> c_int;
        let inner = &win.0;
        unsafe {
            let h = libc::dlopen(c"libX11.so.6".as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
            assert!(!h.is_null());
            let get: GetProperty = std::mem::transmute(libc::dlsym(h, c"XGetWindowProperty".as_ptr()));
            let free: unsafe extern "C" fn(*mut c_void) -> c_int =
                std::mem::transmute(libc::dlsym(h, c"XFree".as_ptr()));
            let name = (inner.lib.XInternAtom)(inner.display, c"_NET_WM_NAME".as_ptr(), 0);
            let utf8 = (inner.lib.XInternAtom)(inner.display, c"UTF8_STRING".as_ptr(), 0);
            let (mut actual_type, mut format, mut items, mut after) = (0, 0, 0, 0);
            let mut data: *mut u8 = std::ptr::null_mut();
            let status = get(
                inner.display, inner.window, name, 0, 1024, 0, utf8,
                &mut actual_type, &mut format, &mut items, &mut after, &mut data,
            );
            assert_eq!(status, 0, "XGetWindowProperty failed");
            if data.is_null() {
                return None;
            }
            let bytes = std::slice::from_raw_parts(data, items as usize).to_vec();
            free(data.cast());
            assert_eq!(actual_type, utf8, "_NET_WM_NAME is not a UTF8_STRING");
            assert_eq!(format, 8);
            Some(String::from_utf8(bytes).expect("_NET_WM_NAME is valid UTF-8"))
        }
    }

    /// Window managers read non-ASCII titles from `_NET_WM_NAME`
    /// (`UTF8_STRING`); `WM_NAME` alone is Latin-1 and garbles them.
    #[test]
    fn titles_are_published_as_utf8_net_wm_name() {
        if std::env::var_os("DISPLAY").is_none() {
            eprintln!("no X display; skipping");
            return;
        }
        let spec = WindowSpec {
            title: "litty — Tiếng Việt ✓",
            decorations: true,
            visible: false,
            transparent: false,
            inner_size: crate::platform::gui::LogicalSize::new(200.0, 100.0),
            position: None,
        };
        let Some(win) = X11Window::create(&spec) else {
            eprintln!("X display unavailable; skipping");
            return;
        };
        assert_eq!(net_wm_name(&win).as_deref(), Some("litty — Tiếng Việt ✓"));

        win.set_title("vim ~/日本語.txt");
        assert_eq!(net_wm_name(&win).as_deref(), Some("vim ~/日本語.txt"));
    }
}
