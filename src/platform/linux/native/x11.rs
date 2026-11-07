//! Native X11 window backend via dynamically loaded `libX11.so.6`.

use std::ffi::{c_char, c_int, c_long, c_ulong, c_void, CStr, CString};
use std::sync::Arc;
use parking_lot::Mutex;

use crate::platform::gui::{
    ButtonState, MouseButton, PhysicalPosition, PhysicalSize, RawHandle, WindowEvent, WindowId, WindowSpec,
};
use super::events::convert_linux_key;

#[repr(C)]
pub struct XEvent {
    pub type_: c_int,
    pub pad: [c_long; 24],
}

#[repr(C)]
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
    pub ConnectionNumber: unsafe extern "C" fn(*mut c_void) -> c_int,
    pub XPending: unsafe extern "C" fn(*mut c_void) -> c_int,
    pub XNextEvent: unsafe extern "C" fn(*mut c_void, *mut XEvent) -> c_int,
    pub XStoreName: unsafe extern "C" fn(*mut c_void, c_ulong, *const c_char) -> c_int,
    pub XInternAtom: unsafe extern "C" fn(*mut c_void, *const c_char, c_int) -> c_ulong,
    pub XSetWMProtocols: unsafe extern "C" fn(*mut c_void, c_ulong, *mut c_ulong, c_int) -> c_int,
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
                ConnectionNumber: sym(handle, c"ConnectionNumber")?,
                XPending: sym(handle, c"XPending")?,
                XNextEvent: sym(handle, c"XNextEvent")?,
                XStoreName: sym(handle, c"XStoreName")?,
                XInternAtom: sym(handle, c"XInternAtom")?,
                XSetWMProtocols: sym(handle, c"XSetWMProtocols")?,
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

#[derive(Clone)]
pub struct X11Window(pub Arc<X11WindowInner>);

static ACTIVE_X11: Mutex<Option<X11Window>> = Mutex::new(None);

pub fn get_active_x11() -> Option<X11Window> {
    ACTIVE_X11.lock().clone()
}

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

            let width = spec.inner_size.width.max(100.0) as c_int;
            let height = spec.inner_size.height.max(100.0) as c_int;
            let x = spec.position.map_or(100, |p| p.x as c_int);
            let y = spec.position.map_or(100, |p| p.y as c_int);

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

            // Map and flush to display
            (lib.XMapWindow)(display, window);
            (lib.XFlush)(display);

            let fd = (lib.ConnectionNumber)(display);

            let win = Self(Arc::new(X11WindowInner {
                lib,
                display,
                window,
                fd,
                wm_delete_window,
            }));

            *ACTIVE_X11.lock() = Some(win.clone());

            Some(win)
        }
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
                        let key_input = convert_linux_key(evdev_code, pressed, false);
                        events.push(WindowEvent::Key(key_input));
                    }
                    // ButtonPress = 4, ButtonRelease = 5
                    4 | 5 => {
                        let bev: &XButtonEvent = std::mem::transmute(&xev);
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
