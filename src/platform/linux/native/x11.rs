//! Native X11 window backend via dynamically loaded `libX11.so.6`.

use std::ffi::{c_char, c_int, c_long, c_ulong, c_void, CStr, CString};
use std::sync::Arc;

use crate::platform::gui::{
    ButtonState, MouseButton, PhysicalPosition, PhysicalSize, RawHandle, WindowEvent, WindowId, WindowSpec,
};
use super::events::{convert_linux_key, mods_after_key, mods_from_state};
use crate::platform::input::{Key, Mods};
use std::sync::atomic::{AtomicU8, Ordering};

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
    pub root: c_ulong,
    pub screen: c_int,
    pub scale: f64,
    /// Last modifiers reported to the app, packed by `pack_mods`.
    pub mods: AtomicU8,
}

unsafe impl Send for X11WindowInner {}
unsafe impl Sync for X11WindowInner {}

impl Drop for X11WindowInner {
    fn drop(&mut self) {
        unsafe {
            (self.lib.XDestroyWindow)(self.display, self.window);
            (self.lib.XCloseDisplay)(self.display);
        }
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

            // Set window title
            if let Ok(c_title) = CString::new(spec.title) {
                (lib.XStoreName)(display, window, c_title.as_ptr());
            }

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
                const PROP_MODE_REPLACE: c_int = 0;
                (lib.XChangeProperty)(display, window, atom, atom, 32, PROP_MODE_REPLACE, data.as_ptr().cast(), 5);
            }

            // Map and flush to display
            (lib.XMapWindow)(display, window);
            (lib.XFlush)(display);

            let fd = (lib.XConnectionNumber)(display);

            let win = Self(Arc::new(X11WindowInner {
                lib,
                display,
                window,
                fd,
                wm_delete_window,
                root,
                screen,
                scale,
                mods: AtomicU8::new(0),
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
        if let Ok(c_title) = CString::new(title) {
            unsafe {
                (self.0.lib.XStoreName)(self.0.display, self.0.window, c_title.as_ptr());
                (self.0.lib.XFlush)(self.0.display);
            }
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
        let n = unsafe {
            (self.0.lib.XLookupString)(&mut ev, buf.as_mut_ptr(), buf.len() as c_int, &mut keysym, std::ptr::null_mut())
        };
        if n <= 0 {
            return None;
        }
        // XLookupString returns Latin-1, which maps byte-for-byte to chars.
        let text: String = buf[..n as usize].iter().map(|&b| char::from(b as u8)).collect();
        Some(text).filter(|t| t.chars().all(|c| !c.is_control()))
    }

    pub fn raw_handle(&self) -> RawHandle {
        RawHandle::Xlib {
            display: self.0.display,
            window: self.0.window,
        }
    }

    pub fn poll_events(&self, _window_id: WindowId) -> Vec<WindowEvent> {
        let mut events = Vec::new();
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
                        self.report_mods(mods_after_key(state, evdev_code, pressed), &mut events);
                        let mut key_input = convert_linux_key(evdev_code, pressed, false);
                        if let Key::Character(_) = key_input.logical {
                            // Layout-aware text: Shift (and AltGr) applied,
                            // Ctrl/Alt/Super left to the keymap.
                            const CTRL_ALT_SUPER: u32 = 4 | 8 | 64;
                            if let Some(c) = self.lookup(kev, state & !CTRL_ALT_SUPER) {
                                key_input.logical = Key::Character(c.clone());
                                key_input.text = Some(c);
                            }
                            if let Some(c) = self.lookup(kev, 0) {
                                key_input.unshifted = Key::Character(c);
                            }
                        }
                        events.push(WindowEvent::Key(key_input));
                    }
                    // ButtonPress = 4, ButtonRelease = 5
                    4 | 5 => {
                        let bev: &XButtonEvent = std::mem::transmute(&xev);
                        self.report_mods(mods_from_state(bev.state as u32), &mut events);
                        let state = if xev.type_ == 4 { ButtonState::Pressed } else { ButtonState::Released };
                        let button = match bev.button {
                            1 => MouseButton::Left,
                            2 => MouseButton::Middle,
                            3 => MouseButton::Right,
                            _ => MouseButton::Other,
                        };
                        events.push(WindowEvent::MouseInput { state, button });
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
                        events.push(WindowEvent::Focused(xev.type_ == 9));
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
        events
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
}
