//! X Input Method (XIM) support: input methods such as IBus and Fcitx
//! (Vietnamese Telex/VNI, CJK) and dead keys / Compose through Xlib's
//! built-in method when no server runs.
//!
//! Composition is shown by the app (`Ime::Preedit`, on-the-spot) when the
//! method supports preedit callbacks; finished text arrives as a `KeyPress`
//! with keycode 0, read with `Xutf8LookupString` and sent as `Ime::Commit`.

use std::ffi::{c_char, c_int, c_long, c_short, c_ulong, c_ushort, c_void, CStr};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::OnceLock;

use parking_lot::Mutex;

use crate::platform::gui::Ime;
use super::sym;
use super::x11::{XEvent, XKeyEvent};

type Display = c_void;
type Xim = *mut c_void;
type Xic = *mut c_void;

// Input styles (`<X11/Xlib.h>`).
const XIM_PREEDIT_CALLBACKS: c_ulong = 0x0002;
const XIM_PREEDIT_NOTHING: c_ulong = 0x0008;
const XIM_PREEDIT_NONE: c_ulong = 0x0010;
const XIM_STATUS_NOTHING: c_ulong = 0x0400;
const XIM_STATUS_NONE: c_ulong = 0x0800;
/// On-the-spot: the app draws the composition at the cursor.
const STYLE_ON_THE_SPOT: c_ulong = XIM_PREEDIT_CALLBACKS | XIM_STATUS_NOTHING;
/// Root window: the input method draws the composition itself.
const STYLE_ROOT: c_ulong = XIM_PREEDIT_NOTHING | XIM_STATUS_NOTHING;
/// No composition shown (dead keys and Compose only).
const STYLE_NONE: c_ulong = XIM_PREEDIT_NONE | XIM_STATUS_NONE;

// `Xutf8LookupString` status values.
const X_BUFFER_OVERFLOW: c_int = -1;
const X_LOOKUP_CHARS: c_int = 2;
const X_LOOKUP_BOTH: c_int = 4;

/// Commits are a word or a phrase; longer ones are retried at their size.
const LOOKUP_BUF: usize = 64;
/// Upper bound for the retry, against a broken method reporting nonsense.
const LOOKUP_MAX: usize = 4096;

#[repr(C)]
struct XIMStyles {
    count_styles: c_ushort,
    supported_styles: *mut c_ulong,
}

#[repr(C)]
struct XIMCallback {
    client_data: *mut c_void,
    callback: *const c_void,
}

#[repr(C)]
struct XPoint {
    x: c_short,
    y: c_short,
}

#[repr(C)]
struct XIMText {
    length: c_ushort,
    feedback: *mut c_ulong,
    encoding_is_wchar: c_int,
    /// `multi_byte` (locale encoding) or `wide_char` (`wchar_t`).
    string: *mut c_void,
}

#[repr(C)]
struct XIMPreeditDrawCallbackStruct {
    caret: c_int,
    chg_first: c_int,
    chg_length: c_int,
    text: *mut XIMText,
}

type XimCallbackFn = unsafe extern "C" fn(Xic, *mut c_void, *mut c_void);
type XimStartFn = unsafe extern "C" fn(Xic, *mut c_void, *mut c_void) -> c_int;
type InstantiateFn = unsafe extern "C" fn(*mut Display, *mut c_void, *mut c_void);

#[allow(non_snake_case)]
struct XimLib {
    XSupportsLocale: unsafe extern "C" fn() -> c_int,
    XSetLocaleModifiers: unsafe extern "C" fn(*const c_char) -> *mut c_char,
    XOpenIM: unsafe extern "C" fn(*mut Display, *mut c_void, *mut c_char, *mut c_char) -> Xim,
    XCloseIM: unsafe extern "C" fn(Xim) -> c_int,
    XGetIMValues: unsafe extern "C" fn(Xim, ...) -> *mut c_char,
    XSetIMValues: unsafe extern "C" fn(Xim, ...) -> *mut c_char,
    XCreateIC: unsafe extern "C" fn(Xim, ...) -> Xic,
    XDestroyIC: unsafe extern "C" fn(Xic),
    XSetICValues: unsafe extern "C" fn(Xic, ...) -> *mut c_char,
    XGetICValues: unsafe extern "C" fn(Xic, ...) -> *mut c_char,
    XSetICFocus: unsafe extern "C" fn(Xic),
    XUnsetICFocus: unsafe extern "C" fn(Xic),
    XFilterEvent: unsafe extern "C" fn(*mut XEvent, c_ulong) -> c_int,
    Xutf8LookupString: unsafe extern "C" fn(Xic, *mut XKeyEvent, *mut c_char, c_int, *mut c_ulong, *mut c_int) -> c_int,
    XVaCreateNestedList: unsafe extern "C" fn(c_int, ...) -> *mut c_void,
    XRegisterIMInstantiateCallback:
        unsafe extern "C" fn(*mut Display, *mut c_void, *mut c_char, *mut c_char, InstantiateFn, *mut c_void) -> c_int,
    XUnregisterIMInstantiateCallback:
        unsafe extern "C" fn(*mut Display, *mut c_void, *mut c_char, *mut c_char, InstantiateFn, *mut c_void) -> c_int,
    XFree: unsafe extern "C" fn(*mut c_void) -> c_int,
}

// Function pointers into libX11, which stays loaded for the process.
unsafe impl Send for XimLib {}
unsafe impl Sync for XimLib {}

fn xim_lib() -> Option<&'static XimLib> {
    static LIB: OnceLock<Option<XimLib>> = OnceLock::new();
    LIB.get_or_init(|| unsafe {
        let handle = [c"libX11.so.6", c"libX11.so"]
            .iter()
            .map(|name| libc::dlopen(name.as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL))
            .find(|h| !h.is_null())?;
        Some(XimLib {
            XSupportsLocale: sym(handle, c"XSupportsLocale")?,
            XSetLocaleModifiers: sym(handle, c"XSetLocaleModifiers")?,
            XOpenIM: sym(handle, c"XOpenIM")?,
            XCloseIM: sym(handle, c"XCloseIM")?,
            XGetIMValues: sym(handle, c"XGetIMValues")?,
            XSetIMValues: sym(handle, c"XSetIMValues")?,
            XCreateIC: sym(handle, c"XCreateIC")?,
            XDestroyIC: sym(handle, c"XDestroyIC")?,
            XSetICValues: sym(handle, c"XSetICValues")?,
            XGetICValues: sym(handle, c"XGetICValues")?,
            XSetICFocus: sym(handle, c"XSetICFocus")?,
            XUnsetICFocus: sym(handle, c"XUnsetICFocus")?,
            XFilterEvent: sym(handle, c"XFilterEvent")?,
            Xutf8LookupString: sym(handle, c"Xutf8LookupString")?,
            XVaCreateNestedList: sym(handle, c"XVaCreateNestedList")?,
            XRegisterIMInstantiateCallback: sym(handle, c"XRegisterIMInstantiateCallback")?,
            XUnregisterIMInstantiateCallback: sym(handle, c"XUnregisterIMInstantiateCallback")?,
            XFree: sym(handle, c"XFree")?,
        })
    })
    .as_ref()
}

/// Xlib picks input methods and decodes their text by `LC_CTYPE`, which a
/// Rust program never sets. Use the user's locale when it is UTF-8 (the
/// preedit text arrives in it), `C.UTF-8` otherwise.
fn init_locale(lib: &XimLib) {
    static LOCALE: std::sync::Once = std::sync::Once::new();
    LOCALE.call_once(|| unsafe {
        let user = libc::setlocale(libc::LC_CTYPE, c"".as_ptr());
        let codeset = libc::nl_langinfo(libc::CODESET);
        let utf8 = !codeset.is_null() && is_utf8_codeset(CStr::from_ptr(codeset));
        if user.is_null() || !utf8 || (lib.XSupportsLocale)() == 0 {
            libc::setlocale(libc::LC_CTYPE, c"C.UTF-8".as_ptr());
        }
    });
}

fn is_utf8_codeset(codeset: &CStr) -> bool {
    codeset.to_bytes().eq_ignore_ascii_case(b"UTF-8") || codeset.to_bytes().eq_ignore_ascii_case(b"utf8")
}

/// The best style the method offers: on-the-spot, then root-window, then
/// none (Xlib's built-in method only supports the last).
fn choose_style(supported: &[c_ulong]) -> Option<c_ulong> {
    [STYLE_ON_THE_SPOT, STYLE_ROOT, STYLE_NONE].into_iter().find(|s| supported.contains(s))
}

/// Apply a preedit draw: replace `len` characters at `first` with `text`.
/// Out-of-range positions from a misbehaving method are clamped.
fn apply_preedit_draw(preedit: &mut Vec<char>, first: c_int, len: c_int, text: Option<&[char]>) {
    let start = usize::try_from(first).unwrap_or(0).min(preedit.len());
    let end = start.saturating_add(usize::try_from(len).unwrap_or(0)).min(preedit.len());
    preedit.splice(start..end, text.unwrap_or(&[]).iter().copied());
}

/// The candidate window goes below the cursor cell: XIM spots are baseline
/// points, so use the cell's bottom-left corner, clamped to X's 16 bits.
fn spot_for_cell(x: f64, y: f64, height: f64) -> (c_short, c_short) {
    let clamp = |v: f64| v.round().clamp(c_short::MIN as f64, c_short::MAX as f64) as c_short;
    (clamp(x), clamp(y + height))
}

/// The spot to send to the method, if it changed since it was last sent and
/// the method can use it now (`ready`: a focused context that draws).
fn spot_to_send(
    ready: bool,
    spot: Option<(c_short, c_short)>,
    sent: Option<(c_short, c_short)>,
) -> Option<(c_short, c_short)> {
    spot.filter(|&s| ready && sent != Some(s))
}

/// Retries of a server that was announced but could not be connected to;
/// beyond this the window stays on Xlib's built-in method until the server
/// leaves or another one starts.
const MAX_REARMS: u32 = 3;

/// Registrations Xlib may refuse in a row before the waiting contexts stop
/// asking: a refusal that lasts would otherwise be retried on every event.
const MAX_REGISTER_FAILURES: u32 = 3;

/// Xlib's input method code keeps process-wide state without locks of its
/// own (the locale modifiers `XOpenIM` reads, the instantiate-callback list
/// `XFilterEvent` walks), and every window has its own display on its own
/// thread in tests. Every Xlib IM call runs under this lock; the callbacks
/// Xlib runs inside those calls never take it.
static XLIB_IM: Mutex<Watch> = Mutex::new(Watch::new());

/// Bumped by the instantiate callback: a method server registered.
static SERVERS_SEEN: AtomicU64 = AtomicU64::new(0);

/// Identifies a context in `Watch`.
static NEXT_CONTEXT_ID: AtomicU64 = AtomicU64::new(0);

/// Contexts waiting for a method server, and the one instantiate
/// registration made for all of them. Xlib walks a single process-wide
/// callback list from whichever display filters the `XIM_SERVERS` change,
/// so one registration, on a waiting context's display, wakes every
/// waiting context through `SERVERS_SEEN`. Displays are kept as addresses.
#[derive(Debug, Default, PartialEq)]
struct Watch {
    waiting: Vec<(u64, usize)>,
    registered: Option<(u64, usize)>,
    /// Registrations refused in a row (`MAX_REGISTER_FAILURES`).
    failures: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum WatchChange {
    Register(usize),
    Unregister(usize),
}

impl Watch {
    const fn new() -> Self {
        Self { waiting: Vec::new(), registered: None, failures: 0 }
    }

    /// Context `id` on `display` waits for a server.
    fn add(&mut self, id: u64, display: usize) -> Vec<WatchChange> {
        if !self.waiting.iter().any(|&(w, _)| w == id) {
            self.waiting.push((id, display));
        }
        if self.registered.is_some() {
            return Vec::new();
        }
        self.registered = Some((id, display));
        vec![WatchChange::Register(display)]
    }

    /// Context `id` stops waiting (connected or dropped). A registration on
    /// its display moves to the next waiting context, whose display lives on.
    fn remove(&mut self, id: u64) -> Vec<WatchChange> {
        self.waiting.retain(|&(w, _)| w != id);
        let Some((_, display)) = self.registered.filter(|&(owner, _)| owner == id) else { return Vec::new() };
        let mut changes = vec![WatchChange::Unregister(display)];
        self.registered = self.waiting.first().copied();
        if let Some((_, next)) = self.registered {
            changes.push(WatchChange::Register(next));
        }
        changes
    }

    /// Replace the registration with a fresh one on the same display: libX11
    /// does not run a registration's callback twice.
    fn rearm(&self) -> Vec<WatchChange> {
        self.registered
            .map(|(_, display)| vec![WatchChange::Unregister(display), WatchChange::Register(display)])
            .unwrap_or_default()
    }

    /// Register again for the contexts already waiting after Xlib refused
    /// the last registration, until it has refused too many in a row.
    fn retry(&mut self) -> Vec<WatchChange> {
        if self.registered.is_some() || self.failures >= MAX_REGISTER_FAILURES {
            return Vec::new();
        }
        self.registered = self.waiting.first().copied();
        self.registered.map(|(_, display)| vec![WatchChange::Register(display)]).unwrap_or_default()
    }

    /// Xlib refused the registration; `true` when this was the last retry.
    fn register_failed(&mut self) -> bool {
        self.registered = None;
        self.failures = self.failures.saturating_add(1);
        self.failures == MAX_REGISTER_FAILURES
    }

    fn register_succeeded(&mut self) {
        self.failures = 0;
    }

    /// Make the registrations `changes` asks for.
    fn apply(&mut self, lib: &XimLib, changes: Vec<WatchChange>) {
        let null = std::ptr::null_mut::<c_void>();
        for change in changes {
            unsafe {
                // Registrations are keyed by the modifiers current at the
                // call, so both sides must use the same ones.
                (lib.XSetLocaleModifiers)(c"".as_ptr());
                match change {
                    WatchChange::Register(display) => {
                        let display = display as *mut Display;
                        if (lib.XRegisterIMInstantiateCallback)(display, null, null.cast(), null.cast(), im_instantiated, null) == 0 {
                            if self.register_failed() {
                                eprintln!("litty: cannot watch for an X input method server");
                            }
                        } else {
                            self.register_succeeded();
                        }
                    }
                    WatchChange::Unregister(display) => {
                        let display = display as *mut Display;
                        if (lib.XUnregisterIMInstantiateCallback)(display, null, null.cast(), null.cast(), im_instantiated, null) == 0 {
                            eprintln!("litty: X input method watch was not removed");
                        }
                    }
                }
            }
        }
    }
}

/// What the input method callbacks report back to the window's event loop.
#[derive(Default)]
pub struct ImeShared {
    preedit: Mutex<Vec<char>>,
    events: Mutex<Vec<Ime>>,
    /// The method's server went away (`XNDestroyCallback`).
    destroyed: AtomicBool,
}

impl ImeShared {
    fn push_preedit(&self, preedit: &[char]) {
        self.events.lock().push(Ime::Preedit(preedit.iter().collect()));
    }

    pub fn drain_into(&self, out: &mut Vec<crate::platform::gui::WindowEvent>) {
        let mut events = self.events.lock();
        out.extend(events.drain(..).map(crate::platform::gui::WindowEvent::Ime));
    }
}

/// `client_data` is the window's `ImeShared`, alive as long as its IC.
unsafe fn shared<'a>(client_data: *mut c_void) -> &'a ImeShared {
    unsafe { &*client_data.cast::<ImeShared>() }
}

unsafe extern "C" fn preedit_start(_ic: Xic, client_data: *mut c_void, _call: *mut c_void) -> c_int {
    let s = unsafe { shared(client_data) };
    s.preedit.lock().clear();
    -1 // no length limit
}

unsafe extern "C" fn preedit_done(_ic: Xic, client_data: *mut c_void, _call: *mut c_void) {
    let s = unsafe { shared(client_data) };
    let mut preedit = s.preedit.lock();
    preedit.clear();
    s.push_preedit(&preedit);
}

unsafe extern "C" fn preedit_draw(_ic: Xic, client_data: *mut c_void, call: *mut c_void) {
    let s = unsafe { shared(client_data) };
    if call.is_null() {
        return;
    }
    let draw = unsafe { &*call.cast::<XIMPreeditDrawCallbackStruct>() };
    let inserted: Option<Vec<char>> = unsafe { draw.text.as_ref() }.and_then(|text| {
        if text.string.is_null() {
            return None;
        }
        Some(if text.encoding_is_wchar != 0 {
            let wide = unsafe { std::slice::from_raw_parts(text.string.cast::<libc::wchar_t>(), usize::from(text.length)) };
            wide.iter().map(|&c| u32::try_from(c).ok().and_then(char::from_u32).unwrap_or('\u{FFFD}')).collect()
        } else {
            unsafe { CStr::from_ptr(text.string.cast()) }.to_string_lossy().chars().collect()
        })
    });
    // A text with no string only restyles the range: the characters stay.
    if draw.text.is_null() || inserted.is_some() {
        let mut preedit = s.preedit.lock();
        apply_preedit_draw(&mut preedit, draw.chg_first, draw.chg_length, inserted.as_deref());
        s.push_preedit(&preedit);
    }
}

unsafe extern "C" fn preedit_caret(_ic: Xic, _client_data: *mut c_void, _call: *mut c_void) {}

unsafe extern "C" fn im_destroyed(_im: Xim, client_data: *mut c_void, _call: *mut c_void) {
    let s = unsafe { shared(client_data) };
    s.destroyed.store(true, Relaxed);
}

/// Runs inside an Xlib call made under `XLIB_IM`: only touch the counter.
unsafe extern "C" fn im_instantiated(_display: *mut Display, _client_data: *mut c_void, _call: *mut c_void) {
    SERVERS_SEEN.fetch_add(1, Relaxed);
}

/// One window's input method connection.
///
/// Methods that reach Xlib take `XLIB_IM` and first react to what the
/// callbacks reported (`after_callbacks`), so a context the server freed is
/// never used.
pub struct XimContext {
    id: u64,
    display: *mut Display,
    window: c_ulong,
    im: Xim,
    ic: Xic,
    style: c_ulong,
    /// Boxed: its address is the callbacks' `client_data`.
    shared: Box<ImeShared>,
    /// While waiting for a method server: `SERVERS_SEEN` when last checked.
    waiting: Option<u64>,
    /// A new method was opened since the last `take_reopened`.
    reopened: bool,
    allowed: bool,
    focused: bool,
    /// Where the candidate window goes, and what the method last received.
    spot: Option<(c_short, c_short)>,
    sent_spot: Option<(c_short, c_short)>,
    /// Reconnect attempts since this context started waiting (`MAX_REARMS`).
    rearms: u32,
}

impl XimContext {
    /// Connect `window` to the input method named by `XMODIFIERS`, or to
    /// Xlib's built-in one (dead keys, Compose) when that is not running.
    ///
    /// # Safety
    /// `display` and `window` must stay live until the context is dropped.
    pub unsafe fn new(display: *mut Display, window: c_ulong) -> Self {
        let mut ctx = Self {
            id: NEXT_CONTEXT_ID.fetch_add(1, Relaxed),
            display,
            window,
            im: std::ptr::null_mut(),
            ic: std::ptr::null_mut(),
            style: 0,
            shared: Box::default(),
            waiting: None,
            reopened: false,
            allowed: true,
            focused: false,
            spot: None,
            sent_spot: None,
            rearms: 0,
        };
        let Some(lib) = xim_lib() else { return ctx };
        let mut watch = XLIB_IM.lock();
        init_locale(lib);
        unsafe {
            // The method `XMODIFIERS` names; while its server is not running,
            // Xlib's own (dead keys, Compose) until it starts.
            if !ctx.open(lib, &watch, c"") {
                if !ctx.open(lib, &watch, c"@im=none") {
                    eprintln!("litty: no X input method available; composed input and dead keys are off");
                }
                ctx.start_waiting(lib, &mut watch);
            }
        }
        ctx
    }

    /// Event types the method needs selected on the window (0 without one).
    pub fn filter_events(&mut self) -> c_long {
        let Some(lib) = xim_lib() else { return 0 };
        let mut watch = XLIB_IM.lock();
        self.after_callbacks(lib, &mut watch);
        let mut mask: c_long = 0;
        if !self.ic.is_null() {
            unsafe {
                (lib.XGetICValues)(self.ic, c"filterEvents".as_ptr(), &mut mask as *mut c_long, std::ptr::null_mut::<c_void>());
            }
        }
        mask
    }

    /// Open a method under `modifiers` and make it the window's, replacing
    /// the current one. `false` leaves the current one in place.
    unsafe fn open(&mut self, lib: &XimLib, _locked: &Watch, modifiers: &CStr) -> bool {
        unsafe {
            (lib.XSetLocaleModifiers)(modifiers.as_ptr());
            let im = (lib.XOpenIM)(self.display, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
            if im.is_null() {
                return false;
            }
            let Some(style) = self.query_style(lib, im) else {
                (lib.XCloseIM)(im);
                return false;
            };
            let ic = self.create_ic(lib, im, style);
            if ic.is_null() {
                (lib.XCloseIM)(im);
                return false;
            }
            self.close(lib, _locked);
            let destroy = XIMCallback {
                client_data: (&*self.shared as *const ImeShared).cast_mut().cast(),
                callback: im_destroyed as *const c_void,
            };
            (lib.XSetIMValues)(im, c"destroyCallback".as_ptr(), &destroy as *const XIMCallback, std::ptr::null_mut::<c_void>());
            self.im = im;
            self.ic = ic;
            self.style = style;
            // The new context has no spot yet; `filter` sends it.
            self.sent_spot = None;
            if self.allowed && self.focused {
                (lib.XSetICFocus)(ic);
            }
            true
        }
    }

    /// Free the current method and context, if any.
    fn close(&mut self, lib: &XimLib, _locked: &Watch) {
        unsafe {
            if !self.ic.is_null() {
                (lib.XDestroyIC)(std::mem::replace(&mut self.ic, std::ptr::null_mut()));
            }
            if !self.im.is_null() {
                (lib.XCloseIM)(std::mem::replace(&mut self.im, std::ptr::null_mut()));
            }
        }
        // Closing it ourselves is not the server going away.
        self.shared.destroyed.store(false, Relaxed);
    }

    unsafe fn query_style(&self, lib: &XimLib, im: Xim) -> Option<c_ulong> {
        let mut styles: *mut XIMStyles = std::ptr::null_mut();
        unsafe {
            let failed = (lib.XGetIMValues)(
                im, c"queryInputStyle".as_ptr(), &mut styles as *mut *mut XIMStyles, std::ptr::null_mut::<c_void>(),
            );
            if !failed.is_null() || styles.is_null() {
                return None;
            }
            let list = &*styles;
            let supported = if list.supported_styles.is_null() {
                &[][..]
            } else {
                std::slice::from_raw_parts(list.supported_styles, usize::from(list.count_styles))
            };
            let style = choose_style(supported);
            (lib.XFree)(styles.cast());
            style
        }
    }

    unsafe fn create_ic(&self, lib: &XimLib, im: Xim, style: c_ulong) -> Xic {
        let data = (&*self.shared as *const ImeShared).cast_mut().cast::<c_void>();
        let start = XIMCallback { client_data: data, callback: preedit_start as XimStartFn as *const c_void };
        let done = XIMCallback { client_data: data, callback: preedit_done as XimCallbackFn as *const c_void };
        let draw = XIMCallback { client_data: data, callback: preedit_draw as XimCallbackFn as *const c_void };
        let caret = XIMCallback { client_data: data, callback: preedit_caret as XimCallbackFn as *const c_void };
        let end = std::ptr::null_mut::<c_void>();
        unsafe {
            if style != STYLE_ON_THE_SPOT {
                return (lib.XCreateIC)(
                    im,
                    c"inputStyle".as_ptr(), style,
                    c"clientWindow".as_ptr(), self.window,
                    c"focusWindow".as_ptr(), self.window,
                    end,
                );
            }
            let attrs = (lib.XVaCreateNestedList)(
                0,
                c"preeditStartCallback".as_ptr(), &start as *const XIMCallback,
                c"preeditDoneCallback".as_ptr(), &done as *const XIMCallback,
                c"preeditDrawCallback".as_ptr(), &draw as *const XIMCallback,
                c"preeditCaretCallback".as_ptr(), &caret as *const XIMCallback,
                end,
            );
            if attrs.is_null() {
                return std::ptr::null_mut();
            }
            let ic = (lib.XCreateIC)(
                im,
                c"inputStyle".as_ptr(), style,
                c"clientWindow".as_ptr(), self.window,
                c"focusWindow".as_ptr(), self.window,
                c"preeditAttributes".as_ptr(), attrs,
                end,
            );
            (lib.XFree)(attrs);
            ic
        }
    }

    /// Give `event` to the input method first; `true` when it consumed it.
    /// Reconnects when a method server appears after the last one left.
    pub fn filter(&mut self, event: &mut XEvent) -> bool {
        let Some(lib) = xim_lib() else { return false };
        let mut watch = XLIB_IM.lock();
        self.after_callbacks(lib, &mut watch);
        // Before the key it may compose, so the candidate window opens there.
        self.flush_spot(lib, &watch);
        let consumed = unsafe { (lib.XFilterEvent)(event, 0) != 0 };
        self.after_callbacks(lib, &mut watch);
        consumed
    }

    /// The server went away: Xlib already freed the method and its
    /// contexts, so drop the pointers. `true` when that happened.
    fn forget_dead_im(&mut self) -> bool {
        if !self.shared.destroyed.swap(false, Relaxed) {
            return false;
        }
        self.im = std::ptr::null_mut();
        self.ic = std::ptr::null_mut();
        self.sent_spot = None;
        let had_preedit = !std::mem::take(&mut *self.shared.preedit.lock()).is_empty();
        if had_preedit {
            self.shared.events.lock().push(Ime::Disabled);
        }
        true
    }

    /// React to the destroy and instantiate callbacks Xlib ran: fall back to
    /// Xlib's own method when the server left, connect when one appeared.
    fn after_callbacks(&mut self, lib: &XimLib, watch: &mut Watch) {
        if self.forget_dead_im() {
            unsafe { self.open(lib, watch, c"@im=none") };
            self.reopened = true;
            self.start_waiting(lib, watch);
        }
        let Some(seen) = self.waiting else { return };
        let changes = watch.retry();
        watch.apply(lib, changes);
        let now = SERVERS_SEEN.load(Relaxed);
        if now == seen {
            return;
        }
        self.waiting = Some(now);
        if unsafe { self.open(lib, watch, c"") } {
            self.reopened = true;
            self.stop_waiting(lib, watch);
        } else {
            self.rearm(lib, watch);
        }
    }

    /// The server was announced but did not take this window. libX11 runs a
    /// registration's callback only once (until some method is destroyed),
    /// so register again for the next announcement, a bounded number of
    /// times: registering while a server runs announces it at once.
    fn rearm(&mut self, lib: &XimLib, watch: &mut Watch) {
        if self.rearms == MAX_REARMS {
            return;
        }
        self.rearms += 1;
        if self.rearms == MAX_REARMS {
            eprintln!("litty: X input method server is running but refused this window; using dead keys and Compose only");
        }
        let changes = watch.rearm();
        watch.apply(lib, changes);
    }

    fn start_waiting(&mut self, lib: &XimLib, watch: &mut Watch) {
        if self.waiting.is_some() {
            return;
        }
        // Read first: registering while a server runs calls back at once.
        self.waiting = Some(SERVERS_SEEN.load(Relaxed));
        self.rearms = 0;
        let changes = watch.add(self.id, self.display as usize);
        watch.apply(lib, changes);
    }

    fn stop_waiting(&mut self, lib: &XimLib, watch: &mut Watch) {
        if self.waiting.take().is_none() {
            return;
        }
        let changes = watch.remove(self.id);
        watch.apply(lib, changes);
    }

    #[cfg(test)]
    pub fn has_context(&self) -> bool {
        !self.ic.is_null()
    }

    /// Pretend a method was just reconnected.
    #[cfg(test)]
    pub fn mark_reopened(&mut self) {
        self.reopened = true;
    }

    /// Whether a method was reconnected since the last call; its
    /// `filter_events` must then be selected on the window again.
    pub fn take_reopened(&mut self) -> bool {
        std::mem::take(&mut self.reopened)
    }

    /// Preedit changes the callbacks queued.
    pub fn shared(&self) -> &ImeShared {
        &self.shared
    }

    /// Text the method committed: a `KeyPress` with keycode 0 that Xlib
    /// queued after composing (IBus/Fcitx commit, dead key, Compose).
    ///
    /// Xlib queues those as received (`send_event` false). One another client
    /// sent is never looked up: Xlib's built-in method treats every keycode-0
    /// press as composed and, with nothing composed yet, reads a compose-tree
    /// entry it never initialised (libX11 1.8 `imLcLkup.c`). The sender
    /// controls every field of such an event, so none can be trusted; dead
    /// keys typed with `XSendEvent` therefore do not compose.
    pub fn committed_text(&mut self, key: &mut XKeyEvent) -> Option<String> {
        let lib = xim_lib().filter(|_| key.keycode == 0 && key.send_event == 0)?;
        let mut watch = XLIB_IM.lock();
        self.after_callbacks(lib, &mut watch);
        if self.ic.is_null() {
            return None;
        }
        let mut buf = vec![0u8; LOOKUP_BUF];
        loop {
            let mut keysym: c_ulong = 0;
            let mut status: c_int = 0;
            let len = buf.len() as c_int;
            let n = unsafe { (lib.Xutf8LookupString)(self.ic, key, buf.as_mut_ptr().cast(), len, &mut keysym, &mut status) };
            match status {
                X_BUFFER_OVERFLOW if (n as usize) <= LOOKUP_MAX && n as usize > buf.len() => buf.resize(n as usize, 0),
                X_LOOKUP_CHARS | X_LOOKUP_BOTH => {
                    buf.truncate(usize::try_from(n).unwrap_or(0));
                    return String::from_utf8(buf).ok().filter(|t| !t.is_empty());
                }
                _ => return None,
            }
        }
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        self.sync_focus();
    }

    pub fn set_allowed(&mut self, allowed: bool) {
        self.allowed = allowed;
        self.sync_focus();
    }

    fn sync_focus(&mut self) {
        let Some(lib) = xim_lib() else { return };
        let mut watch = XLIB_IM.lock();
        self.after_callbacks(lib, &mut watch);
        if self.ic.is_null() {
            return;
        }
        unsafe {
            if self.allowed && self.focused {
                (lib.XSetICFocus)(self.ic);
            } else {
                (lib.XUnsetICFocus)(self.ic);
            }
        }
    }

    /// Place the method's candidate window under the cursor cell. Only
    /// stored: this runs while rendering, and sending it is a round trip to
    /// the method, so `filter` sends it before the next event.
    pub fn set_cursor_area(&mut self, x: f64, y: f64, height: f64) {
        self.spot = Some(spot_for_cell(x, y, height));
    }

    fn flush_spot(&mut self, lib: &XimLib, _locked: &Watch) {
        let ready = self.focused && self.allowed && !self.ic.is_null() && self.style != STYLE_NONE;
        let Some((x, y)) = spot_to_send(ready, self.spot, self.sent_spot) else { return };
        let point = XPoint { x, y };
        unsafe {
            let attrs = (lib.XVaCreateNestedList)(0, c"spotLocation".as_ptr(), &point as *const XPoint, std::ptr::null_mut::<c_void>());
            if attrs.is_null() {
                return;
            }
            (lib.XSetICValues)(self.ic, c"preeditAttributes".as_ptr(), attrs, std::ptr::null_mut::<c_void>());
            (lib.XFree)(attrs);
        }
        self.sent_spot = Some((x, y));
    }
}

impl Drop for XimContext {
    fn drop(&mut self) {
        let Some(lib) = xim_lib() else { return };
        let mut watch = XLIB_IM.lock();
        // A context the server freed must not be destroyed again.
        self.forget_dead_im();
        self.stop_waiting(lib, &mut watch);
        self.close(lib, &watch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_the_spot_is_preferred_then_root_window_then_none() {
        assert_eq!(choose_style(&[STYLE_NONE, STYLE_ROOT, STYLE_ON_THE_SPOT]), Some(STYLE_ON_THE_SPOT));
        assert_eq!(choose_style(&[STYLE_NONE, STYLE_ROOT]), Some(STYLE_ROOT));
        // Xlib's built-in method (dead keys, Compose) offers only this one.
        assert_eq!(choose_style(&[STYLE_NONE]), Some(STYLE_NONE));
        // Over-the-spot needs a font set Litty does not provide.
        assert_eq!(choose_style(&[0x0004 | XIM_STATUS_NOTHING]), None);
    }

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    /// Telex: `t`, `ie`, `ê`, `ế` are drawn as replacements of the preedit.
    #[test]
    fn preedit_draws_replace_the_changed_range() {
        let mut p = Vec::new();
        apply_preedit_draw(&mut p, 0, 0, Some(&chars("vie")));
        assert_eq!(p, chars("vie"));
        apply_preedit_draw(&mut p, 1, 2, Some(&chars("iê")));
        assert_eq!(p, chars("viê"));
        apply_preedit_draw(&mut p, 2, 1, Some(&chars("ế")));
        assert_eq!(p, chars("viế"));
        // No text: the range is deleted.
        apply_preedit_draw(&mut p, 1, 2, None);
        assert_eq!(p, chars("v"));
    }

    #[test]
    fn out_of_range_preedit_draws_are_clamped() {
        let mut p = chars("ab");
        apply_preedit_draw(&mut p, 5, 3, Some(&chars("c")));
        assert_eq!(p, chars("abc"));
        apply_preedit_draw(&mut p, -1, 100, None);
        assert_eq!(p, chars(""));
    }

    #[test]
    fn the_spot_is_the_cell_bottom_left_within_x_coordinates() {
        assert_eq!(spot_for_cell(80.0, 40.0, 20.0), (80, 60));
        assert_eq!(spot_for_cell(1e9, -1e9, 0.0), (c_short::MAX, c_short::MIN));
    }

    #[test]
    fn the_spot_is_sent_once_and_only_to_a_ready_context() {
        assert_eq!(spot_to_send(true, Some((1, 2)), None), Some((1, 2)));
        assert_eq!(spot_to_send(true, Some((1, 2)), Some((1, 2))), None, "unchanged");
        assert_eq!(spot_to_send(true, Some((3, 2)), Some((1, 2))), Some((3, 2)), "moved");
        assert_eq!(spot_to_send(false, Some((1, 2)), None), None, "unfocused or no context");
        assert_eq!(spot_to_send(true, None, None), None, "no cursor yet");
    }

    use WatchChange::{Register, Unregister};

    #[test]
    fn one_registration_serves_every_waiting_context() {
        let mut w = Watch::new();
        assert_eq!(w.add(1, 10), [Register(10)]);
        assert_eq!(w.add(2, 20), [], "already registered");
        assert_eq!(w.add(1, 10), [], "adding twice is a no-op");
        assert_eq!(w.waiting, [(1, 10), (2, 20)]);
        // A context that is not the owner leaves without touching Xlib.
        assert_eq!(w.remove(2), []);
        assert_eq!(w.remove(1), [Unregister(10)]);
        assert_eq!(w, Watch::new());
        assert_eq!(w.remove(1), [], "removing twice is a no-op");
    }

    #[test]
    fn the_registration_moves_when_its_owner_stops_waiting() {
        let mut w = Watch::new();
        w.add(1, 10);
        w.add(2, 20);
        w.add(3, 30);
        // The owner's display may close next: re-register on a live one.
        assert_eq!(w.remove(1), [Unregister(10), Register(20)]);
        assert_eq!(w.registered, Some((2, 20)));
        assert_eq!(w.remove(3), []);
        assert_eq!(w.remove(2), [Unregister(20)]);
        assert_eq!(w.registered, None);
        // The next waiting context registers again.
        assert_eq!(w.add(4, 40), [Register(40)]);
    }

    #[test]
    fn rearming_replaces_the_registration_on_its_display() {
        let mut w = Watch::new();
        assert_eq!(w.rearm(), [], "nothing registered");
        w.add(1, 10);
        w.add(2, 20);
        assert_eq!(w.rearm(), [Unregister(10), Register(10)]);
        assert_eq!(w.registered, Some((1, 10)), "same owner");
        assert_eq!(w.waiting, [(1, 10), (2, 20)]);
    }

    /// A registration Xlib refused is made again for the contexts already
    /// waiting, not only when one is added or removed; a lasting refusal is
    /// given up on instead of being retried on every event.
    #[test]
    fn a_refused_registration_is_retried_a_few_times() {
        let mut w = Watch::new();
        assert_eq!(w.retry(), [], "nobody waits");
        w.add(1, 10);
        w.add(2, 20);
        assert_eq!(w.retry(), [], "already registered");
        for attempt in 1..MAX_REGISTER_FAILURES {
            assert!(!w.register_failed(), "attempt {attempt}");
            assert_eq!(w.registered, None);
            assert_eq!(w.retry(), [Register(10)], "attempt {attempt}");
        }
        assert!(w.register_failed(), "the last failure gives up");
        assert_eq!(w.retry(), []);
        assert_eq!(w.waiting, [(1, 10), (2, 20)], "contexts still wait");
        // A working registration starts the count again.
        w.register_succeeded();
        assert_eq!(w.retry(), [Register(10)]);
    }

    #[test]
    fn utf8_codesets_are_recognised() {
        assert!(is_utf8_codeset(c"UTF-8"));
        assert!(is_utf8_codeset(c"utf8"));
        assert!(!is_utf8_codeset(c"ISO-8859-1"));
        assert!(!is_utf8_codeset(c"ANSI_X3.4-1968"));
    }

    /// The preedit callbacks turn draws into `Ime::Preedit` events.
    #[test]
    fn preedit_callbacks_queue_preedit_events() {
        let shared = ImeShared::default();
        let data = (&shared as *const ImeShared).cast_mut().cast();
        let mut text = c"vi".to_owned().into_bytes_with_nul();
        let mut xim_text = XIMText { length: 2, feedback: std::ptr::null_mut(), encoding_is_wchar: 0, string: text.as_mut_ptr().cast() };
        let mut draw = XIMPreeditDrawCallbackStruct { caret: 2, chg_first: 0, chg_length: 0, text: &mut xim_text };
        unsafe {
            assert_eq!(preedit_start(std::ptr::null_mut(), data, std::ptr::null_mut()), -1);
            preedit_draw(std::ptr::null_mut(), data, (&mut draw as *mut XIMPreeditDrawCallbackStruct).cast());
            // A restyle (no string) changes nothing and reports nothing.
            (*draw.text).string = std::ptr::null_mut();
            preedit_draw(std::ptr::null_mut(), data, (&mut draw as *mut XIMPreeditDrawCallbackStruct).cast());
            preedit_done(std::ptr::null_mut(), data, std::ptr::null_mut());
        }
        let mut out = Vec::new();
        shared.drain_into(&mut out);
        let got: Vec<Ime> = out
            .into_iter()
            .map(|e| match e {
                crate::platform::gui::WindowEvent::Ime(ime) => ime,
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(got, [Ime::Preedit("vi".into()), Ime::Preedit(String::new())]);
    }
}
