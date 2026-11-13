//! Native Wayland windows: `xdg_toplevel`s on one process-wide connection.
//!
//! Phase 1 of the [Wayland backend plan](../../../../docs/plans/wayland-backend.md):
//! window, Vulkan surface, configure and resize, close, title and app id,
//! and frame callbacks pacing redraws. Input, scale, decorations, text
//! input, occlusion and activation are later phases; until they land the
//! backend is opt-in (`LITTY_BACKEND=wayland`).
//!
//! The connection is never closed: the clipboard (`smithay-clipboard`) keeps
//! its `wl_display` pointer for the life of the process.

use std::os::fd::AsRawFd;
use std::sync::{Arc, OnceLock, Weak};

use parking_lot::Mutex;
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::client::backend::{ObjectId, ReadEventsGuard, WaylandError};
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::protocol::{wl_output, wl_surface};
use smithay_client_toolkit::reexports::client::{Connection, EventQueue, Proxy, QueueHandle};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::shell::xdg::window::{Window, WindowConfigure, WindowDecorations, WindowHandler};
use smithay_client_toolkit::{
    delegate_compositor, delegate_output, delegate_registry, delegate_xdg_shell, delegate_xdg_window, registry_handlers,
};

use crate::platform::gui::{PhysicalSize, RawHandle, WindowEvent, WindowId, WindowSpec};

/// Matches the `.desktop` file, so the shell shows Litty's name and icon.
const APP_ID: &str = "litty";

/// Round trips a new window waits for its first configure. Compositors send
/// it in answer to the first commit, so one is normally enough; a window
/// still unconfigured after these draws once the configure arrives.
const CONFIGURE_ROUNDTRIPS: usize = 4;

/// The window system new windows open on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendChoice {
    X11,
    Wayland,
}

/// The backend `LITTY_BACKEND` names. X11 (XWayland on Wayland desktops)
/// stays the default until the Wayland backend handles input; an unknown
/// value is an error the caller reports before using X11.
pub fn choose_backend(litty_backend: Option<&str>) -> Result<BackendChoice, String> {
    match litty_backend.map(str::trim) {
        None | Some("") => Ok(BackendChoice::X11),
        Some(v) if v.eq_ignore_ascii_case("x11") => Ok(BackendChoice::X11),
        Some(v) if v.eq_ignore_ascii_case("wayland") => Ok(BackendChoice::Wayland),
        Some(v) => Err(format!("unknown LITTY_BACKEND={v:?} (expected `x11` or `wayland`); using X11")),
    }
}

/// [`choose_backend`] for this process's environment, read once.
pub fn selected_backend() -> BackendChoice {
    static CHOICE: OnceLock<BackendChoice> = OnceLock::new();
    *CHOICE.get_or_init(|| {
        let value = std::env::var("LITTY_BACKEND").ok();
        choose_backend(value.as_deref()).unwrap_or_else(|msg| {
            eprintln!("litty: {msg}");
            BackendChoice::X11
        })
    })
}

/// When a window may draw. A redraw waits for the first configure (a
/// buffer before it is a protocol error) and for the frame callback of the
/// last presented frame: the compositor withholds that callback while the
/// window cannot be seen, so a hidden window stops drawing instead of
/// blocking the event loop in a FIFO present that never completes.
#[derive(Debug, Default)]
struct FramePacer {
    configured: bool,
    /// A presented frame's callback has not arrived yet.
    in_flight: bool,
    /// A redraw was held back and is owed once the window may draw.
    wanted: bool,
}

impl FramePacer {
    /// The app asks to draw: whether it may now. A refused redraw is owed.
    fn begin(&mut self) -> bool {
        let ready = self.configured && !self.in_flight;
        self.wanted |= !ready;
        ready
    }

    /// A frame is about to be presented: whether to ask for its callback.
    /// The present's own commit carries the request, so a redraw that
    /// presents nothing leaves no callback outstanding.
    fn presenting(&mut self) -> bool {
        if !self.configured {
            return false;
        }
        self.in_flight = true;
        true
    }

    /// The announced frame was not presented, so its callback request was
    /// never committed; it rides along with the next present instead.
    fn present_skipped(&mut self) {
        self.in_flight = false;
    }

    /// The frame callback arrived: whether a held redraw is now due.
    fn frame_done(&mut self) -> bool {
        self.in_flight = false;
        std::mem::take(&mut self.wanted)
    }

    /// A configure arrived; the window always redraws for one (first map,
    /// new size or state).
    fn configure(&mut self) {
        self.configured = true;
    }
}

/// What a configure tells the app about, in the form tests can build.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Configure {
    /// Surface size the compositor asks for, per axis; `None` leaves it to us.
    width: Option<u32>,
    height: Option<u32>,
    maximized: bool,
    fullscreen: bool,
    activated: bool,
}

impl From<&WindowConfigure> for Configure {
    fn from(c: &WindowConfigure) -> Self {
        Self {
            width: c.new_size.0.map(|v| v.get()),
            height: c.new_size.1.map(|v| v.get()),
            maximized: c.is_maximized(),
            fullscreen: c.is_fullscreen(),
            activated: c.is_activated(),
        }
    }
}

/// Window state last reported to the app.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Reported {
    size: PhysicalSize<u32>,
    maximized: bool,
    fullscreen: bool,
    activated: bool,
}

impl Reported {
    /// Apply a configure, appending an event for each change. Scale is 1
    /// until fractional scaling lands, so surface and buffer sizes agree.
    fn apply(&mut self, c: Configure, out: &mut Vec<WindowEvent>) {
        let size = PhysicalSize::new(c.width.unwrap_or(self.size.width), c.height.unwrap_or(self.size.height));
        if size != self.size {
            self.size = size;
            out.push(WindowEvent::Resized(size));
        }
        if (c.maximized, c.fullscreen) != (self.maximized, self.fullscreen) {
            (self.maximized, self.fullscreen) = (c.maximized, c.fullscreen);
            out.push(WindowEvent::StateChanged { maximized: c.maximized, fullscreen: c.fullscreen });
        }
        if c.activated != self.activated {
            self.activated = c.activated;
            out.push(WindowEvent::Focused(c.activated));
        }
    }
}

/// Per-window state the dispatch handlers update.
struct WindowShared {
    id: WindowId,
    state: Mutex<SharedState>,
}

struct SharedState {
    /// Events for the app, drained by `poll_events`.
    events: Vec<WindowEvent>,
    pacer: FramePacer,
    reported: Reported,
}

/// Registry, outputs and the windows the handlers route events to.
struct State {
    registry: RegistryState,
    outputs: OutputState,
    windows: Vec<(ObjectId, Weak<WindowShared>)>,
}

impl State {
    /// The live window whose `wl_surface` is `surface`, pruning closed ones.
    fn window(&mut self, surface: &wl_surface::WlSurface) -> Option<Arc<WindowShared>> {
        self.windows.retain(|(_, w)| w.strong_count() > 0);
        let id = surface.id();
        self.windows.iter().find(|(s, _)| *s == id).and_then(|(_, w)| w.upgrade())
    }
}

struct Client {
    conn: Connection,
    qh: QueueHandle<State>,
    compositor: CompositorState,
    xdg: XdgShell,
    /// Dispatched only by the event loop (and a new window's first round
    /// trips); handlers never take this lock again.
    queue: Mutex<(EventQueue<State>, State)>,
}

static CLIENT: OnceLock<Result<Client, String>> = OnceLock::new();

fn client() -> Result<&'static Client, String> {
    CLIENT.get_or_init(connect).as_ref().map_err(Clone::clone)
}

/// The connection, once a Wayland window has opened it.
fn connected() -> Option<&'static Client> {
    CLIENT.get()?.as_ref().ok()
}

fn connect() -> Result<Client, String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("cannot connect to the Wayland compositor: {e}"))?;
    let (globals, queue) = registry_queue_init::<State>(&conn).map_err(|e| format!("cannot list Wayland globals: {e}"))?;
    let qh = queue.handle();
    let compositor = CompositorState::bind(&globals, &qh).map_err(|e| format!("no wl_compositor: {e}"))?;
    let xdg = XdgShell::bind(&globals, &qh).map_err(|e| format!("no xdg_wm_base: {e}"))?;
    let state = State { registry: RegistryState::new(&globals), outputs: OutputState::new(&globals, &qh), windows: Vec::new() };
    super::app::register_wayland_fd(conn.backend().poll_fd().as_raw_fd());
    Ok(Client { conn, qh, compositor, xdg, queue: Mutex::new((queue, state)) })
}

/// Xlib exits the process when its connection breaks; do the same, saying why.
fn connection_lost(err: impl std::fmt::Display) -> ! {
    eprintln!("litty: lost the connection to the Wayland compositor: {err}");
    std::process::exit(1)
}

/// Run the handlers for every event read so far.
pub fn dispatch() {
    let Some(client) = connected() else { return };
    let mut guard = client.queue.lock();
    let (queue, state) = &mut *guard;
    if let Err(e) = queue.dispatch_pending(state) {
        connection_lost(e);
    }
}

/// What the event loop does with the Wayland socket while it waits.
pub enum Wait {
    /// No Wayland connection.
    Disconnected,
    /// Events are queued already: dispatch them before blocking.
    Busy,
    /// Wait for the socket, then hand the guard to [`read`] (dropping it cancels).
    Read(ReadEventsGuard),
}

/// Send queued requests and prepare to read, right before the loop blocks.
/// The guard makes a reader on another thread (Mesa's WSI, the clipboard)
/// wait for this one instead of taking events from under it.
pub fn prepare_wait() -> Wait {
    let Some(client) = connected() else { return Wait::Disconnected };
    // A full socket buffer: wake when the compositor drains it, not spin.
    let blocked = match client.conn.flush() {
        Ok(()) => false,
        Err(WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => true,
        Err(e) => connection_lost(e),
    };
    super::app::watch_wayland_writable(blocked);
    match client.conn.prepare_read() {
        Some(guard) => Wait::Read(guard),
        None => Wait::Busy,
    }
}

/// The socket is readable: read its events for the next [`dispatch`].
pub fn read(guard: ReadEventsGuard) {
    match guard.read() {
        Ok(_) => {}
        Err(WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
        Err(e) => connection_lost(e),
    }
}

/// A native Wayland toplevel. Dropping the last clone destroys it.
#[derive(Clone)]
pub struct WaylandWindow(Arc<WaylandWindowInner>);

struct WaylandWindowInner {
    window: Window,
    shared: Arc<WindowShared>,
    client: &'static Client,
}

impl WaylandWindow {
    pub fn create(spec: &WindowSpec, id: WindowId) -> Result<Self, String> {
        let client = client()?;
        let surface = client.compositor.create_surface(&client.qh);
        // Litty draws its own title bar when the spec turns decorations off.
        let decorations = if spec.decorations { WindowDecorations::RequestServer } else { WindowDecorations::RequestClient };
        let window = client.xdg.create_window(surface, decorations, &client.qh);
        window.set_title(spec.title);
        window.set_app_id(APP_ID);
        // Logical and physical pixels agree until scaling lands (phase 3).
        let size = PhysicalSize::new(
            spec.inner_size.width.max(100.0).round() as u32,
            spec.inner_size.height.max(100.0).round() as u32,
        );
        let shared = Arc::new(WindowShared {
            id,
            state: Mutex::new(SharedState {
                events: Vec::new(),
                pacer: FramePacer::default(),
                reported: Reported { size, maximized: false, fullscreen: false, activated: false },
            }),
        });

        let mut guard = client.queue.lock();
        let (queue, state) = &mut *guard;
        state.windows.push((window.wl_surface().id(), Arc::downgrade(&shared)));
        // The first commit, without a buffer, asks for the first configure.
        window.commit();
        for _ in 0..CONFIGURE_ROUNDTRIPS {
            if shared.state.lock().pacer.configured {
                break;
            }
            if let Err(e) = queue.roundtrip(state) {
                connection_lost(e);
            }
        }
        drop(guard);
        Ok(Self(Arc::new(WaylandWindowInner { window, shared, client })))
    }

    /// The size the compositor configured, or the requested one before that.
    pub fn inner_size(&self) -> PhysicalSize<u32> {
        self.0.shared.state.lock().reported.size
    }

    pub fn raw_handle(&self) -> RawHandle {
        RawHandle::Wayland {
            display: self.0.client.conn.backend().display_ptr().cast(),
            surface: self.0.window.wl_surface().id().as_ptr().cast(),
        }
    }

    /// Append the events the handlers queued for this window.
    pub fn poll_events(&self, out: &mut Vec<WindowEvent>) {
        out.append(&mut self.0.shared.state.lock().events);
    }

    /// Whether a requested redraw may run now; a refused one runs after the
    /// next frame callback or configure.
    pub fn may_redraw(&self) -> bool {
        self.0.shared.state.lock().pacer.begin()
    }

    /// Called right before the frame is presented: ask for its frame
    /// callback, which the present's commit sends to the compositor.
    pub fn pre_present_notify(&self) {
        if self.0.shared.state.lock().pacer.presenting() {
            let surface = self.0.window.wl_surface();
            surface.frame(&self.0.client.qh, surface.clone());
        }
    }

    pub fn present_skipped(&self) {
        self.0.shared.state.lock().pacer.present_skipped();
    }

    /// Takes effect at once: `xdg_toplevel.set_title` is not double-buffered.
    pub fn set_title(&self, title: &str) {
        self.0.window.set_title(title);
    }

    pub fn set_maximized(&self, maximized: bool) {
        if maximized {
            self.0.window.set_maximized();
        } else {
            self.0.window.unset_maximized();
        }
    }

    pub fn set_fullscreen(&self, fullscreen: bool) {
        if fullscreen {
            self.0.window.set_fullscreen(None);
        } else {
            self.0.window.unset_fullscreen();
        }
    }

    pub fn iconify(&self) {
        self.0.window.set_minimized();
    }
}

impl CompositorHandler for State {
    // Buffer scale and output transforms are phase 3; the window draws at scale 1.
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}

    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, _time: u32) {
        let Some(win) = self.window(surface) else { return };
        let due = win.state.lock().pacer.frame_done();
        if due {
            super::app::request_redraw(win.id);
        }
    }

    // Monitor lookup is phase 3.
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}

    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }

    // Outputs are tracked by `OutputState`; nothing reads them before phase 3.
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl WindowHandler for State {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, window: &Window) {
        if let Some(win) = self.window(window.wl_surface()) {
            win.state.lock().events.push(WindowEvent::CloseRequested);
            super::app::wakeup_main_loop();
        }
    }

    /// sctk has acked the configure; the next commit (the redraw's present)
    /// applies it.
    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, window: &Window, configure: WindowConfigure, _serial: u32) {
        let Some(win) = self.window(window.wl_surface()) else { return };
        {
            let mut st = win.state.lock();
            let SharedState { events, pacer, reported } = &mut *st;
            reported.apply(Configure::from(&configure), events);
            pacer.configure();
        }
        super::app::request_redraw(win.id);
    }
}

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }

    registry_handlers![OutputState];
}

delegate_compositor!(State);
delegate_output!(State);
delegate_xdg_shell!(State);
delegate_xdg_window!(State);
delegate_registry!(State);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_defaults_to_x11_and_accepts_both_names() {
        assert_eq!(choose_backend(None), Ok(BackendChoice::X11));
        assert_eq!(choose_backend(Some("")), Ok(BackendChoice::X11));
        assert_eq!(choose_backend(Some("x11")), Ok(BackendChoice::X11));
        assert_eq!(choose_backend(Some("Wayland")), Ok(BackendChoice::Wayland));
        assert_eq!(choose_backend(Some(" wayland\n")), Ok(BackendChoice::Wayland));
    }

    /// A typo must be reported, not silently ignored: it names the value.
    #[test]
    fn unknown_backend_names_the_value() {
        let err = choose_backend(Some("waylnd")).unwrap_err();
        assert!(err.contains("\"waylnd\""), "{err}");
    }

    /// Before the first configure a buffer is a protocol error: no drawing,
    /// and the redraw is owed rather than lost.
    #[test]
    fn no_redraw_before_the_first_configure() {
        let mut p = FramePacer::default();
        assert!(!p.begin());
        assert!(!p.presenting(), "no frame callback for a surface that cannot draw");
        p.configure();
        assert!(p.begin());
    }

    /// While a frame's callback is outstanding further redraws wait, and
    /// exactly one is replayed when the callback arrives.
    #[test]
    fn redraws_wait_for_the_frame_callback() {
        let mut p = FramePacer::default();
        p.configure();
        assert!(p.begin());
        assert!(p.presenting());
        assert!(!p.begin());
        assert!(!p.begin());
        assert!(p.frame_done(), "the held redraw is due");
        assert!(!p.frame_done(), "and owed only once");
        assert!(p.begin());
    }

    /// A redraw that presents nothing requests no callback, so it cannot
    /// leave the window waiting forever for one.
    #[test]
    fn a_redraw_without_a_present_does_not_block_the_next() {
        let mut p = FramePacer::default();
        p.configure();
        assert!(p.begin());
        assert!(p.begin());
    }

    /// A callback with no redraw held back schedules none: an idle window
    /// stays idle (0% CPU) after its last frame.
    #[test]
    fn a_frame_callback_with_nothing_held_is_not_a_redraw() {
        let mut p = FramePacer::default();
        p.configure();
        assert!(p.begin());
        assert!(p.presenting());
        assert!(!p.frame_done());
    }

    /// A failed submit or present commits nothing, so no callback comes:
    /// the window must draw again rather than wait forever.
    #[test]
    fn a_skipped_present_does_not_wait_for_a_callback() {
        let mut p = FramePacer::default();
        p.configure();
        assert!(p.begin());
        assert!(p.presenting());
        p.present_skipped();
        assert!(p.begin());
    }

    fn reported(w: u32, h: u32) -> Reported {
        Reported { size: PhysicalSize::new(w, h), maximized: false, fullscreen: false, activated: false }
    }

    fn configure(width: Option<u32>, height: Option<u32>) -> Configure {
        Configure { width, height, maximized: false, fullscreen: false, activated: false }
    }

    /// No size from the compositor keeps the window's own; nothing changes,
    /// nothing is reported.
    #[test]
    fn unsized_configure_keeps_the_requested_size() {
        let mut r = reported(968, 620);
        let mut out = Vec::new();
        r.apply(configure(None, None), &mut out);
        assert!(out.is_empty(), "{out:?}");
        assert_eq!(r.size, PhysicalSize::new(968, 620));
    }

    /// Each axis is taken separately: a tiling compositor may fix one only.
    #[test]
    fn configure_resizes_per_axis() {
        let mut r = reported(968, 620);
        let mut out = Vec::new();
        r.apply(configure(Some(1200), None), &mut out);
        assert!(matches!(out[..], [WindowEvent::Resized(PhysicalSize { width: 1200, height: 620 })]), "{out:?}");
    }

    /// Maximize from the compositor (a title-bar double-click, Super+Up)
    /// reaches the app as a state change, and activation as focus.
    #[test]
    fn configure_reports_state_and_focus_changes_once() {
        let mut r = reported(800, 600);
        let mut out = Vec::new();
        let max = Configure { width: Some(1920), height: Some(1080), maximized: true, fullscreen: false, activated: true };
        r.apply(max, &mut out);
        assert!(
            matches!(
                out[..],
                [
                    WindowEvent::Resized(PhysicalSize { width: 1920, height: 1080 }),
                    WindowEvent::StateChanged { maximized: true, fullscreen: false },
                    WindowEvent::Focused(true),
                ]
            ),
            "{out:?}"
        );
        out.clear();
        r.apply(max, &mut out);
        assert!(out.is_empty(), "a repeated configure reports nothing: {out:?}");
    }
}
