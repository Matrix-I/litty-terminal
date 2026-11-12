//! Native X11 window backend via dynamically loaded `libX11.so.6`.

use std::ffi::{c_char, c_int, c_long, c_ulong, c_void, CStr, CString};
use std::sync::Arc;

use crate::platform::gui::{CursorIcon, Ime, PhysicalPosition, PhysicalSize, RawHandle, WindowEvent, WindowSpec};
use super::sym;
use super::xim::XimContext;
use super::events::{wm_state_event, WmStateAtoms, convert_linux_key, keysym_char, mods_after_key, mods_from_state, x11_button_event, HeldKeys};
use crate::platform::input::{Key, KeyLocation, Mods};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
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
pub struct XPropertyEvent {
    pub type_: c_int,
    pub serial: c_ulong,
    pub send_event: c_int,
    pub display: *mut c_void,
    pub window: c_ulong,
    pub atom: c_ulong,
    pub time: c_ulong,
    pub state: c_int,
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
/// Predefined `ATOM` atom (property type of `_NET_WM_STATE`).
const XA_ATOM: c_ulong = 4;
/// `_NET_WM_STATE` holds a handful of atoms; more than this is not a real WM.
const WM_STATE_MAX_ATOMS: c_long = 64;

// X core cursor font shapes (`<X11/cursorfont.h>`). Xlib draws them from the
// desktop's cursor theme when libXcursor is installed.
const XC_NUM_GLYPHS: u32 = 154;
const XC_X_CURSOR: u32 = 0;
const XC_BOTTOM_LEFT_CORNER: u32 = 12;
const XC_BOTTOM_RIGHT_CORNER: u32 = 14;
const XC_BOTTOM_SIDE: u32 = 16;
const XC_CROSSHAIR: u32 = 34;
const XC_FLEUR: u32 = 52;
const XC_HAND1: u32 = 58;
const XC_HAND2: u32 = 60;
const XC_LEFT_PTR: u32 = 68;
const XC_LEFT_SIDE: u32 = 70;
const XC_PLUS: u32 = 90;
const XC_QUESTION_ARROW: u32 = 92;
const XC_RIGHT_SIDE: u32 = 96;
const XC_SB_H_DOUBLE_ARROW: u32 = 108;
const XC_SB_V_DOUBLE_ARROW: u32 = 116;
const XC_TOP_LEFT_CORNER: u32 = 134;
const XC_TOP_RIGHT_CORNER: u32 = 136;
const XC_TOP_SIDE: u32 = 138;
const XC_WATCH: u32 = 150;
const XC_XTERM: u32 = 152;
/// Shapes are even numbers below `XC_NUM_GLYPHS`, one cache slot each.
const CURSOR_SLOTS: usize = (XC_NUM_GLYPHS / 2) as usize;
/// `NorthWestGravity`: on resize the X server keeps the window's pixels
/// anchored top-left instead of discarding them (`ForgetGravity`, the
/// default), so the old frame stays up until the new one is presented.
const NORTH_WEST_GRAVITY: c_int = 1;
/// `CWBitGravity` value mask bit for `XChangeWindowAttributes`.
const CW_BIT_GRAVITY: c_ulong = 1 << 4;

/// Events every window selects: KeyPress (1), KeyRelease (2), ButtonPress
/// (4), ButtonRelease (8), PointerMotion (64), Exposure (32768),
/// StructureNotify (131072), FocusChange (2097152) and PropertyChange
/// (4194304) for the WM's `_NET_WM_STATE`.
const BASE_EVENT_MASK: c_long = 1 | 2 | 4 | 8 | 64 | 32768 | 131072 | 2097152 | 4194304;

/// Pseudo shape for the invisible cursor (outside the cursor font).
const BLANK_CURSOR: u32 = XC_NUM_GLYPHS;

/// Xlib `XColor`, only passed to `XCreatePixmapCursor`.
#[repr(C)]
#[derive(Default)]
pub struct XColor {
    pixel: c_ulong,
    red: u16,
    green: u16,
    blue: u16,
    flags: c_char,
    pad: c_char,
}

/// Xlib `XSetWindowAttributes`; only the fields named by the value mask are read.
#[repr(C)]
#[derive(Default)]
pub struct XSetWindowAttributes {
    background_pixmap: c_ulong,
    background_pixel: c_ulong,
    border_pixmap: c_ulong,
    border_pixel: c_ulong,
    bit_gravity: c_int,
    win_gravity: c_int,
    backing_store: c_int,
    backing_planes: c_ulong,
    backing_pixel: c_ulong,
    save_under: c_int,
    event_mask: c_long,
    do_not_propagate_mask: c_long,
    override_redirect: c_int,
    colormap: c_ulong,
    cursor: c_ulong,
}

/// Cursors this window created; 0 = not yet created.
pub struct CursorCache {
    font: [c_ulong; CURSOR_SLOTS],
    blank: c_ulong,
}

/// Core cursor font shape for a CSS cursor name; the core font has no
/// diagonal double arrows, so those use the matching corner.
fn font_cursor_shape(icon: CursorIcon) -> u32 {
    use CursorIcon::*;
    match icon {
        Default | ContextMenu | Alias | Copy | ZoomIn | ZoomOut => XC_LEFT_PTR,
        Help => XC_QUESTION_ARROW,
        Pointer => XC_HAND2,
        Progress | Wait => XC_WATCH,
        Cell => XC_PLUS,
        Crosshair => XC_CROSSHAIR,
        Text | VerticalText => XC_XTERM,
        Move | AllScroll | Grabbing => XC_FLEUR,
        Grab => XC_HAND1,
        NoDrop | NotAllowed => XC_X_CURSOR,
        EResize => XC_RIGHT_SIDE,
        WResize => XC_LEFT_SIDE,
        NResize => XC_TOP_SIDE,
        SResize => XC_BOTTOM_SIDE,
        NeResize => XC_TOP_RIGHT_CORNER,
        NwResize | NwseResize => XC_TOP_LEFT_CORNER,
        SeResize => XC_BOTTOM_RIGHT_CORNER,
        SwResize | NeswResize => XC_BOTTOM_LEFT_CORNER,
        EwResize | ColResize => XC_SB_H_DOUBLE_ARROW,
        NsResize | RowResize => XC_SB_V_DOUBLE_ARROW,
    }
}

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
    pub XGetWindowProperty: unsafe extern "C" fn(
        *mut c_void, c_ulong, c_ulong, c_long, c_long, c_int, c_ulong,
        *mut c_ulong, *mut c_int, *mut c_ulong, *mut c_ulong, *mut *mut u8,
    ) -> c_int,
    pub XFree: unsafe extern "C" fn(*mut c_void) -> c_int,
    pub XCreateFontCursor: unsafe extern "C" fn(*mut c_void, std::ffi::c_uint) -> c_ulong,
    pub XDefineCursor: unsafe extern "C" fn(*mut c_void, c_ulong, c_ulong) -> c_int,
    pub XFreeCursor: unsafe extern "C" fn(*mut c_void, c_ulong) -> c_int,
    pub XCreateBitmapFromData: unsafe extern "C" fn(*mut c_void, c_ulong, *const c_char, std::ffi::c_uint, std::ffi::c_uint) -> c_ulong,
    #[allow(clippy::type_complexity)]
    pub XCreatePixmapCursor: unsafe extern "C" fn(
        *mut c_void, c_ulong, c_ulong, *mut XColor, *mut XColor, std::ffi::c_uint, std::ffi::c_uint,
    ) -> c_ulong,
    pub XFreePixmap: unsafe extern "C" fn(*mut c_void, c_ulong) -> c_int,
    pub XChangeWindowAttributes: unsafe extern "C" fn(*mut c_void, c_ulong, c_ulong, *mut XSetWindowAttributes) -> c_int,
    #[allow(clippy::type_complexity)]
    pub XTranslateCoordinates: unsafe extern "C" fn(
        *mut c_void, c_ulong, c_ulong, c_int, c_int, *mut c_int, *mut c_int, *mut c_ulong,
    ) -> c_int,
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
                XTranslateCoordinates: sym(handle, c"XTranslateCoordinates")?,
                XResourceManagerString: sym(handle, c"XResourceManagerString")?,
                XLookupString: sym(handle, c"XLookupString")?,
                XRefreshKeyboardMapping: sym(handle, c"XRefreshKeyboardMapping")?,
                XGetWindowProperty: sym(handle, c"XGetWindowProperty")?,
                XFree: sym(handle, c"XFree")?,
                XCreateFontCursor: sym(handle, c"XCreateFontCursor")?,
                XDefineCursor: sym(handle, c"XDefineCursor")?,
                XFreeCursor: sym(handle, c"XFreeCursor")?,
                XCreateBitmapFromData: sym(handle, c"XCreateBitmapFromData")?,
                XCreatePixmapCursor: sym(handle, c"XCreatePixmapCursor")?,
                XFreePixmap: sym(handle, c"XFreePixmap")?,
                XChangeWindowAttributes: sym(handle, c"XChangeWindowAttributes")?,
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
    /// `_NET_WM_STATE`, whose changes report WM-driven maximize/fullscreen.
    pub net_wm_state: c_ulong,
    pub wm_state_atoms: WmStateAtoms,
    pub root: c_ulong,
    pub screen: c_int,
    pub scale: f64,
    /// Last modifiers reported to the app, packed by `pack_mods`.
    pub mods: AtomicU8,
    /// Keys held down, to flag auto-repeat presses.
    pub held: HeldKeys,
    pub cursors: parking_lot::Mutex<CursorCache>,
    /// Shape + 1 of the cursor defined on the window (`BLANK_CURSOR` when
    /// hidden), 0 for none.
    pub defined_cursor: AtomicU32,
    /// Shape the app asked for, shown again when the pointer is unhidden.
    pub wanted_cursor: AtomicU32,
    pub cursor_hidden: AtomicBool,
    /// Input method connection; `None` only while the window is dropped.
    pub xim: parking_lot::Mutex<Option<XimContext>>,
}

unsafe impl Send for X11WindowInner {}
unsafe impl Sync for X11WindowInner {}

impl Drop for X11WindowInner {
    fn drop(&mut self) {
        super::app::deregister_display_fd(self.fd);
        // The input context belongs to the window and the display.
        drop(self.xim.get_mut().take());
        unsafe {
            let cache = self.cursors.get_mut();
            for &cursor in cache.font.iter().chain([&cache.blank]).filter(|&&c| c != 0) {
                (self.lib.XFreeCursor)(self.display, cursor);
            }
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

/// `_NET_WM_STATE` client message data adding or removing up to two state
/// atoms (0 for none), from a normal application (source 1).
fn wm_state_request(add: bool, first: c_long, second: c_long) -> [c_long; 5] {
    // _NET_WM_STATE_REMOVE = 0, _NET_WM_STATE_ADD = 1
    [c_long::from(add), first, second, 1, 0]
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

            // Plus whatever the input method needs to see.
            let mut xim = XimContext::new(display, window);
            (lib.XSelectInput)(display, window, BASE_EVENT_MASK | xim.filter_events());

            // Keep the last frame while the window is resized; the default
            // `ForgetGravity` blanks it until Vulkan presents the next one.
            let mut attrs = XSetWindowAttributes { bit_gravity: NORTH_WEST_GRAVITY, ..Default::default() };
            (lib.XChangeWindowAttributes)(display, window, CW_BIT_GRAVITY, &mut attrs);

            let net_wm_name = (lib.XInternAtom)(display, c"_NET_WM_NAME".as_ptr(), 0);
            let utf8_string = (lib.XInternAtom)(display, c"UTF8_STRING".as_ptr(), 0);
            store_title(&lib, display, window, net_wm_name, utf8_string, spec.title);

            // Enable graceful close via WM_DELETE_WINDOW
            let wm_delete_window = (lib.XInternAtom)(display, c"WM_DELETE_WINDOW".as_ptr(), 0);
            let mut protocols = [wm_delete_window];
            let atom = |name: &CStr| (lib.XInternAtom)(display, name.as_ptr(), 0);
            let net_wm_state = atom(c"_NET_WM_STATE");
            let wm_state_atoms = WmStateAtoms {
                max_vert: atom(c"_NET_WM_STATE_MAXIMIZED_VERT"),
                max_horz: atom(c"_NET_WM_STATE_MAXIMIZED_HORZ"),
                fullscreen: atom(c"_NET_WM_STATE_FULLSCREEN"),
            };
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
                net_wm_state,
                wm_state_atoms,
                root,
                screen,
                scale,
                mods: AtomicU8::new(0),
                held: HeldKeys::default(),
                cursors: parking_lot::Mutex::new(CursorCache { font: [0; CURSOR_SLOTS], blank: 0 }),
                defined_cursor: AtomicU32::new(0),
                wanted_cursor: AtomicU32::new(XC_LEFT_PTR),
                cursor_hidden: AtomicBool::new(false),
                xim: parking_lot::Mutex::new(Some(xim)),
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

    /// Ask the window manager to enter or leave fullscreen
    /// (`_NET_WM_STATE_FULLSCREEN`); the resulting state comes back as
    /// `StateChanged`.
    pub fn set_fullscreen(&self, fullscreen: bool) {
        let atom = self.0.wm_state_atoms.fullscreen as c_long;
        self.send_wm_message(c"_NET_WM_STATE", wm_state_request(fullscreen, atom, 0));
    }

    /// The monitor under the window's centre, `None` without RandR.
    /// `size` is the window's current inner size.
    pub fn current_monitor(&self, size: PhysicalSize<u32>) -> Option<crate::platform::gui::Monitor> {
        let (mut x, mut y, mut child) = (0, 0, 0);
        let half = |v: u32| c_int::try_from(v / 2).unwrap_or(c_int::MAX);
        unsafe {
            let ok = (self.0.lib.XTranslateCoordinates)(
                self.0.display, self.0.window, self.0.root, half(size.width), half(size.height), &mut x, &mut y, &mut child,
            );
            if ok == 0 {
                return None;
            }
            super::monitor::monitor_for_point(self.0.display, self.0.root, PhysicalPosition::new(x, y))
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

    /// Run `f` on the input method, then select the events a method it
    /// reconnected to needs: every call into it may reconnect, not only
    /// filtering. Reading the new mask can itself reconnect, hence the loop.
    fn with_xim<R>(&self, f: impl FnOnce(&mut XimContext) -> R) -> Option<R> {
        let mut guard = self.0.xim.lock();
        let xim = guard.as_mut()?;
        let result = f(xim);
        while xim.take_reopened() {
            let mask = BASE_EVENT_MASK | xim.filter_events();
            unsafe { (self.0.lib.XSelectInput)(self.0.display, self.0.window, mask) };
        }
        Some(result)
    }

    /// Let the input method compose text while the app allows it.
    pub fn set_ime_allowed(&self, allowed: bool) {
        self.with_xim(|xim| xim.set_allowed(allowed));
    }

    /// Place the input method's candidate window under the text cursor cell.
    pub fn set_ime_cursor_area(&self, position: PhysicalPosition<f64>, size: PhysicalSize<f64>) {
        self.with_xim(|xim| xim.set_cursor_area(position.x, position.y, size.height));
    }

    /// Give `xev` to the input method; `true` when it consumed the event.
    /// Preedit changes its callbacks made are queued on `events`.
    fn filter_for_ime(&self, xev: &mut XEvent, events: &mut Vec<WindowEvent>) -> bool {
        self.with_xim(|xim| {
            let consumed = xim.filter(xev);
            xim.shared().drain_into(events);
            consumed
        })
        .unwrap_or(false)
    }

    /// Text the input method committed with a keycode-0 `KeyPress`.
    fn ime_commit(&self, kev: &XKeyEvent) -> Option<String> {
        let mut key = *kev;
        self.with_xim(|xim| xim.committed_text(&mut key)).flatten()
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

    /// Current `_NET_WM_STATE` as a `StateChanged` event; a missing or
    /// unreadable property reads as a plain, restored window.
    fn read_wm_state(&self) -> WindowEvent {
        let (mut actual_type, mut format, mut count, mut after) = (0, 0, 0, 0);
        let mut data: *mut u8 = std::ptr::null_mut();
        let status = unsafe {
            (self.0.lib.XGetWindowProperty)(
                self.0.display, self.0.window, self.0.net_wm_state, 0, WM_STATE_MAX_ATOMS, 0, XA_ATOM,
                &mut actual_type, &mut format, &mut count, &mut after, &mut data,
            )
        };
        // Format-32 properties come back as an array of C longs.
        let atoms: &[c_ulong] = if status == 0 && !data.is_null() && actual_type == XA_ATOM && format == 32 {
            unsafe { std::slice::from_raw_parts(data.cast(), count as usize) }
        } else {
            &[]
        };
        let event = wm_state_event(atoms, self.0.wm_state_atoms);
        if !data.is_null() {
            unsafe { (self.0.lib.XFree)(data.cast()) };
        }
        event
    }

    /// Show `icon` over this window; while the pointer is hidden it shows
    /// once unhidden. Pointer motion repeats the same icon, so an unchanged
    /// one costs no X request.
    pub fn set_cursor(&self, icon: CursorIcon) {
        let shape = font_cursor_shape(icon);
        self.0.wanted_cursor.store(shape, Ordering::Relaxed);
        if !self.0.cursor_hidden.load(Ordering::Relaxed) {
            self.define_cursor(shape);
        }
    }

    /// Hide the pointer over this window (`mouse_hide_while_typing`) or
    /// bring back the last requested icon.
    pub fn set_cursor_visible(&self, visible: bool) {
        self.0.cursor_hidden.store(!visible, Ordering::Relaxed);
        let shape = if visible { self.0.wanted_cursor.load(Ordering::Relaxed) } else { BLANK_CURSOR };
        self.define_cursor(shape);
    }

    fn define_cursor(&self, shape: u32) {
        if self.0.defined_cursor.swap(shape + 1, Ordering::Relaxed) == shape + 1 {
            return;
        }
        let cursor = self.cached_cursor(shape);
        if cursor == 0 {
            // Nothing defined: a later call retries.
            self.0.defined_cursor.store(0, Ordering::Relaxed);
            return;
        }
        unsafe {
            (self.0.lib.XDefineCursor)(self.0.display, self.0.window, cursor);
            (self.0.lib.XFlush)(self.0.display);
        }
    }

    /// The cursor for `shape`, created on first use; 0 if X refused it.
    fn cached_cursor(&self, shape: u32) -> c_ulong {
        let mut cache = self.0.cursors.lock();
        let slot = if shape == BLANK_CURSOR { &mut cache.blank } else { &mut cache.font[(shape / 2) as usize] };
        if *slot == 0 {
            *slot = if shape == BLANK_CURSOR { self.create_blank_cursor() } else {
                unsafe { (self.0.lib.XCreateFontCursor)(self.0.display, shape) }
            };
        }
        *slot
    }

    /// A 1x1 cursor whose mask hides every pixel.
    fn create_blank_cursor(&self) -> c_ulong {
        let lib = &self.0.lib;
        let bits: [c_char; 1] = [0];
        unsafe {
            let pixmap = (lib.XCreateBitmapFromData)(self.0.display, self.0.window, bits.as_ptr(), 1, 1);
            if pixmap == 0 {
                return 0;
            }
            let mut color = XColor::default();
            let color_ptr: *mut XColor = &mut color;
            let cursor = (lib.XCreatePixmapCursor)(self.0.display, pixmap, pixmap, color_ptr, color_ptr, 0, 0);
            (lib.XFreePixmap)(self.0.display, pixmap);
            cursor
        }
    }

    #[cfg(test)]
    fn defined_cursor_shape(&self) -> Option<u32> {
        self.0.defined_cursor.load(Ordering::Relaxed).checked_sub(1)
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
                if self.filter_for_ime(&mut xev, events) {
                    continue;
                }

                match xev.type_ {
                    // KeyPress = 2, KeyRelease = 3
                    2 | 3 => {
                        let kev: &XKeyEvent = std::mem::transmute(&xev);
                        // Keycode 0 is no key: Xlib queues it to deliver
                        // text the input method composed.
                        if kev.keycode == 0 {
                            if let Some(text) = self.ime_commit(kev) {
                                events.push(WindowEvent::Ime(Ime::Commit(text)));
                            }
                            continue;
                        }
                        let pressed = xev.type_ == 2;
                        // X11 keycodes on Linux are evdev scancode + 8
                        let evdev_code = (kev.keycode as u32).saturating_sub(8);
                        let state = kev.state as u32;
                        self.report_mods(mods_after_key(state, evdev_code, pressed), events);
                        let was_held = self.0.held.update(kev.keycode as u8, pressed);
                        // No press was reported (the input method consumed
                        // it, or it came before focus): nothing to release.
                        if !pressed && !was_held {
                            continue;
                        }
                        let repeat = pressed && was_held;
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
                        self.with_xim(|xim| xim.set_focused(xev.type_ == 9));
                        events.push(WindowEvent::Focused(xev.type_ == 9));
                    }
                    // PropertyNotify = 28: the WM changed `_NET_WM_STATE`
                    // (maximize via title-bar drag, Super+Up, tiling).
                    28 => {
                        let pev: &XPropertyEvent = std::mem::transmute(&xev);
                        if pev.atom == self.0.net_wm_state {
                            events.push(self.read_wm_state());
                        }
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

    /// The resize edges must show resize arrows, not the default pointer.
    #[test]
    fn resize_edges_map_to_their_side_and_corner_cursors() {
        use crate::platform::gui::CursorIcon::*;
        assert_eq!(font_cursor_shape(NResize), XC_TOP_SIDE);
        assert_eq!(font_cursor_shape(SResize), XC_BOTTOM_SIDE);
        assert_eq!(font_cursor_shape(EResize), XC_RIGHT_SIDE);
        assert_eq!(font_cursor_shape(WResize), XC_LEFT_SIDE);
        assert_eq!(font_cursor_shape(NeResize), XC_TOP_RIGHT_CORNER);
        assert_eq!(font_cursor_shape(NwResize), XC_TOP_LEFT_CORNER);
        assert_eq!(font_cursor_shape(SeResize), XC_BOTTOM_RIGHT_CORNER);
        assert_eq!(font_cursor_shape(SwResize), XC_BOTTOM_LEFT_CORNER);
    }

    #[test]
    fn common_cursors_map_to_their_x_shapes() {
        use crate::platform::gui::CursorIcon::*;
        assert_eq!(font_cursor_shape(Default), XC_LEFT_PTR);
        assert_eq!(font_cursor_shape(Text), XC_XTERM);
        assert_eq!(font_cursor_shape(Pointer), XC_HAND2);
    }

    fn hidden_test_window(title: &'static str) -> Option<X11Window> {
        if std::env::var_os("DISPLAY").is_none() {
            eprintln!("no X display; skipping");
            return None;
        }
        let spec = WindowSpec {
            title,
            decorations: true,
            visible: false,
            transparent: false,
            inner_size: crate::platform::gui::LogicalSize::new(200.0, 100.0),
            position: None,
        };
        let win = X11Window::create(&spec);
        if win.is_none() {
            eprintln!("X display unavailable; skipping");
        }
        win
    }

    /// `mouse_hide_while_typing`: hiding defines a blank cursor, showing
    /// brings back the icon the app asked for.
    #[test]
    fn hiding_the_cursor_defines_a_blank_one_until_shown() {
        use crate::platform::gui::CursorIcon;
        let Some(win) = hidden_test_window("litty-hide-cursor-test") else { return };
        win.set_cursor(CursorIcon::Text);
        win.set_cursor_visible(false);
        assert_eq!(win.defined_cursor_shape(), Some(BLANK_CURSOR));
        win.set_cursor_visible(true);
        assert_eq!(win.defined_cursor_shape(), Some(XC_XTERM));
    }

    /// Hover updates while typing must not reveal the hidden pointer; the
    /// latest icon shows once the pointer moves again.
    #[test]
    fn icon_changes_while_hidden_wait_for_show() {
        use crate::platform::gui::CursorIcon;
        let Some(win) = hidden_test_window("litty-hidden-icon-test") else { return };
        win.set_cursor_visible(false);
        win.set_cursor(CursorIcon::Pointer);
        assert_eq!(win.defined_cursor_shape(), Some(BLANK_CURSOR));
        win.set_cursor_visible(true);
        assert_eq!(win.defined_cursor_shape(), Some(XC_HAND2));
    }

    /// Resizing must keep the last frame instead of blanking the window
    /// until the next one is presented (`ForgetGravity` flickers).
    #[test]
    fn new_window_keeps_its_contents_while_resized() {
        #[repr(C)]
        struct XWindowAttributes {
            x: c_int, y: c_int, width: c_int, height: c_int, border_width: c_int, depth: c_int,
            visual: *mut c_void, root: c_ulong, class: c_int, bit_gravity: c_int, win_gravity: c_int,
            backing_store: c_int, backing_planes: c_ulong, backing_pixel: c_ulong, save_under: c_int,
            colormap: c_ulong, map_installed: c_int, map_state: c_int, all_event_masks: c_long,
            your_event_mask: c_long, do_not_propagate_mask: c_long, override_redirect: c_int, screen: *mut c_void,
        }
        let Some(win) = hidden_test_window("litty-gravity-test") else { return };
        let inner = &win.0;
        let gravity = unsafe {
            let h = libc::dlopen(c"libX11.so.6".as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
            assert!(!h.is_null());
            let get_sym = libc::dlsym(h, c"XGetWindowAttributes".as_ptr());
            assert!(!get_sym.is_null(), "XGetWindowAttributes missing");
            let get: unsafe extern "C" fn(*mut c_void, c_ulong, *mut XWindowAttributes) -> c_int =
                std::mem::transmute(get_sym);
            let mut attrs: XWindowAttributes = std::mem::zeroed();
            assert_ne!(get(inner.display, inner.window, &mut attrs), 0);
            attrs.bit_gravity
        };
        assert_eq!(gravity, NORTH_WEST_GRAVITY);
    }

    /// Setting a cursor must reach the X server: the window's cursor is
    /// the requested shape, and a repeat costs no new cursor.
    #[test]
    fn set_cursor_defines_the_window_cursor() {
        use crate::platform::gui::CursorIcon;
        if std::env::var_os("DISPLAY").is_none() {
            eprintln!("no X display; skipping");
            return;
        }
        let spec = WindowSpec {
            title: "litty-cursor-test",
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
        assert_eq!(win.defined_cursor_shape(), None);
        win.set_cursor(CursorIcon::EResize);
        assert_eq!(win.defined_cursor_shape(), Some(XC_RIGHT_SIDE));
        let cursor = win.cached_cursor(XC_RIGHT_SIDE);
        assert_ne!(cursor, 0, "XCreateFontCursor failed");
        win.set_cursor(CursorIcon::Text);
        win.set_cursor(CursorIcon::EResize);
        assert_eq!(win.cached_cursor(XC_RIGHT_SIDE), cursor, "cursors are created once");
        assert_eq!(win.defined_cursor_shape(), Some(XC_RIGHT_SIDE));
    }

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

    #[test]
    fn wm_state_requests_add_or_remove_the_atoms() {
        assert_eq!(wm_state_request(true, 303, 0), [1, 303, 0, 1, 0]);
        assert_eq!(wm_state_request(false, 303, 0), [0, 303, 0, 1, 0]);
    }

    /// The monitor is read from RandR: it holds the window, and its refresh
    /// rate comes from the mode (it was hard-coded to 1920x1080 at 60 Hz).
    #[test]
    fn current_monitor_holds_the_window() {
        let Some(win) = hidden_test_window("litty-monitor-test") else { return };
        let Some(monitor) = win.current_monitor(PhysicalSize::new(200, 100)) else {
            eprintln!("no RandR on this X server; skipping");
            return;
        };
        // `hidden_test_window` leaves the window at the default (100, 100).
        let (x, y) = (100 + 100, 100 + 50);
        assert!(monitor.position.x <= x && x < monitor.position.x + monitor.size.width as i32, "{monitor:?}");
        assert!(monitor.position.y <= y && y < monitor.position.y + monitor.size.height as i32, "{monitor:?}");
        assert!(monitor.refresh_rate_millihertz.is_some_and(|hz| hz > 0), "{monitor:?}");
    }

    /// Needs a window manager and shows a fullscreen window for a moment,
    /// so it only runs on request: `cargo test fullscreen -- --ignored`.
    #[test]
    #[ignore]
    fn set_fullscreen_is_reported_back_by_the_window_manager() {
        let Some(win) = hidden_test_window("litty-fullscreen-test") else { return };
        win.set_visible(true);
        let wait_for = |want: bool| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut events = Vec::new();
            while std::time::Instant::now() < deadline {
                win.poll_events(&mut events);
                if events.iter().any(|e| matches!(e, WindowEvent::StateChanged { fullscreen, .. } if *fullscreen == want)) {
                    return true;
                }
                events.clear();
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            false
        };
        // Give the WM time to manage the new window before asking.
        std::thread::sleep(std::time::Duration::from_millis(300));
        win.set_fullscreen(true);
        assert!(wait_for(true), "window manager never reported fullscreen");
        win.set_fullscreen(false);
        assert!(wait_for(false), "window manager never left fullscreen");
    }

    /// Every window gets an input context: the one `XMODIFIERS` names, or
    /// Xlib's built-in one for dead keys and Compose.
    #[test]
    fn new_window_has_an_input_context() {
        let Some(win) = hidden_test_window("litty-xim-test") else { return };
        assert!(win.0.xim.lock().as_ref().is_some_and(XimContext::has_context));
    }

    /// Every call into the input method may reconnect it, not only
    /// filtering: the new context's events must be selected right away, or
    /// the first key reaches it without the events it asked for.
    #[test]
    fn a_reconnect_outside_filtering_selects_the_new_methods_events() {
        let Some(win) = hidden_test_window("litty-xim-reselect-test") else { return };
        let reopened = |win: &X11Window| win.0.xim.lock().as_mut().is_some_and(XimContext::take_reopened);
        win.0.xim.lock().as_mut().unwrap().mark_reopened();
        win.set_ime_allowed(true);
        assert!(!reopened(&win), "set_ime_allowed left the reconnect unselected");
        win.0.xim.lock().as_mut().unwrap().mark_reopened();
        let key: XKeyEvent = unsafe { std::mem::zeroed() };
        win.ime_commit(&key);
        assert!(!reopened(&win), "ime_commit left the reconnect unselected");
    }

    /// Keycode 0 carries input-method text, never a key: without composed
    /// text it is dropped instead of reaching the keymap as a bogus key.
    #[test]
    fn keycode_zero_presses_are_not_keys() {
        let Some(win) = hidden_test_window("litty-keycode-zero-test") else { return };
        assert_keycode_zero_is_not_a_key(&win);
    }

    /// A keycode-0 press another client sends is not looked up. Xlib's
    /// built-in method would read a compose-tree entry it never initialised;
    /// the first window's tree sits in fresh zeroed pages, so the second
    /// window's, on reused heap, is the one that crashes.
    #[test]
    fn a_window_after_a_closed_one_still_looks_up_text() {
        let Some(first) = hidden_test_window("litty-xim-first") else { return };
        drop(first);
        let Some(win) = hidden_test_window("litty-xim-second") else { return };
        assert_keycode_zero_is_not_a_key(&win);
    }

    /// A synthetic keycode-0 press reaches `Xutf8LookupString` and is no key;
    /// the press of keycode 38 after it is.
    fn assert_keycode_zero_is_not_a_key(win: &X11Window) {
        let lib = &win.0.lib;
        let send = |keycode: c_int| unsafe {
            let mut xev: XEvent = std::mem::zeroed();
            let key = &mut *(&mut xev as *mut XEvent).cast::<XKeyEvent>();
            key.type_ = 2; // KeyPress
            key.display = win.0.display;
            key.window = win.0.window;
            key.root = win.0.root;
            key.keycode = keycode;
            key.same_screen = 1;
            (lib.XSendEvent)(win.0.display, win.0.window, 1, 1, &mut xev); // KeyPressMask
        };
        // evdev KEY_A (30) + 8 marks the end of what was sent.
        send(0);
        send(38);
        unsafe { (lib.XFlush)(win.0.display) };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut events = Vec::new();
        while !events.iter().any(|e| matches!(e, WindowEvent::Key(_))) && std::time::Instant::now() < deadline {
            win.poll_events(&mut events);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let keys: Vec<_> = events.iter().filter_map(|e| match e { WindowEvent::Key(k) => Some(k.physical), _ => None }).collect();
        assert_eq!(keys.len(), 1, "{events:?}");
    }
}
