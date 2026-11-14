//! Native Wayland windows: `xdg_toplevel`s on one process-wide connection.
//!
//! Phase 1 of the [Wayland backend plan](../../../../docs/plans/wayland-backend.md):
//! window, Vulkan surface, configure and resize, close, title and app id,
//! and frame callbacks pacing redraws; keyboard and pointer input (phase 2)
//! live in [`super::wayland_input`]; scale and outputs (phase 3) here.
//! Text input (phase 5) lives in [`super::wayland_text_input`]; occlusion
//! and activation are later phases;
//! until they land the backend is opt-in (`LITTY_BACKEND=wayland`).
//!
//! Scale: a window draws its buffer at `logical size × scale` and shows it
//! at the logical size through a `wp_viewport`, so a fractional scale
//! (`wp_fractional_scale_v1`) and an integer one (sctk's, from
//! `wl_surface.preferred_buffer_scale` or the outputs) both stay sharp and
//! the buffer never has to divide by the scale. Without `wp_viewporter`
//! the window draws at scale 1 and the compositor upscales it.
//!
//! The connection is never closed: the clipboard (`smithay-clipboard`) keeps
//! its `wl_display` pointer for the life of the process.

use std::os::fd::AsRawFd;
use std::sync::{Arc, OnceLock, Weak};

use parking_lot::Mutex;
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{Mode, OutputHandler, OutputState};
use smithay_client_toolkit::reexports::client::backend::{ObjectId, ReadEventsGuard, WaylandError};
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::protocol::{wl_output, wl_seat, wl_surface};
use smithay_client_toolkit::reexports::protocols::xdg::shell::client::xdg_toplevel::ResizeEdge;
use smithay_client_toolkit::reexports::client::{delegate_noop, Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use smithay_client_toolkit::reexports::protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    wp_fractional_scale_v1::{self, WpFractionalScaleV1},
};
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::SeatState;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::shell::xdg::window::{DecorationMode, Window, WindowConfigure, WindowDecorations, WindowHandler};
use smithay_client_toolkit::shm::Shm;
use smithay_client_toolkit::{
    delegate_compositor, delegate_output, delegate_pointer, delegate_registry, delegate_seat, delegate_shm,
    delegate_xdg_shell, delegate_xdg_window, registry_handlers,
};

use super::wayland_input::{Cursor, KeyRepeat, Keyboard, Pointer, RepeatTimer};
use crate::platform::gui::{
    CursorIcon, Monitor, PhysicalPosition, PhysicalSize, RawHandle, ResizeDirection, Unsupported, WindowEvent, WindowId, WindowSpec,
};

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
    /// The compositor draws the frame (`xdg-decoration` server side).
    /// Without the protocol (GNOME) it never does.
    server_decorations: bool,
}

impl From<&WindowConfigure> for Configure {
    fn from(c: &WindowConfigure) -> Self {
        Self {
            width: c.new_size.0.map(|v| v.get()),
            height: c.new_size.1.map(|v| v.get()),
            maximized: c.is_maximized(),
            fullscreen: c.is_fullscreen(),
            activated: c.is_activated(),
            server_decorations: c.decoration_mode == DecorationMode::Server,
        }
    }
}

/// Where a window's scale comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ScaleSource {
    /// The surface has a `wp_viewport`; without one it stays at scale 1.
    viewport: bool,
    /// `wp_fractional_scale_v1.preferred_scale`, in 120ths. Wins over
    /// `integer` once sent.
    fractional: Option<u32>,
    /// sctk's integer scale: the preferred buffer scale, or the largest
    /// scale of the outputs the surface is on.
    integer: i32,
}

impl ScaleSource {
    fn factor(&self) -> f64 {
        if !self.viewport {
            return 1.0;
        }
        match self.fractional {
            Some(n) if n > 0 => f64::from(n) / 120.0,
            _ => f64::from(self.integer.max(1)),
        }
    }
}

/// Buffer pixels for a logical length, rounded half away from zero as
/// `wp_fractional_scale_v1` asks.
fn scaled(logical: u32, scale: f64) -> u32 {
    ((f64::from(logical) * scale).round() as u32).max(1)
}

/// Window state last reported to the app.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Reported {
    /// Surface size in logical pixels: the configured one, or the
    /// requested one before that. The viewport shows the buffer at it.
    logical: (u32, u32),
    source: ScaleSource,
    /// The buffer size (`logical × scale`) and the scale the app was told.
    size: PhysicalSize<u32>,
    scale: f64,
    maximized: bool,
    fullscreen: bool,
    activated: bool,
    server_decorations: bool,
}

impl Reported {
    /// A window of `logical` size at scale 1 until the compositor says
    /// more, framed as it asked (`server_decorations`) until then too.
    fn new(logical: (u32, u32), viewport: bool, server_decorations: bool) -> Self {
        Self {
            logical,
            source: ScaleSource { viewport, fractional: None, integer: 1 },
            size: PhysicalSize::new(logical.0, logical.1),
            scale: 1.0,
            maximized: false,
            fullscreen: false,
            activated: false,
            server_decorations,
        }
    }

    /// The compositor's preferred fractional scale, in 120ths.
    fn set_fractional(&mut self, scale_120: u32, out: &mut Vec<WindowEvent>) {
        self.source.fractional = Some(scale_120);
        self.rescale(out, false);
    }

    /// sctk's integer scale for the surface.
    fn set_integer(&mut self, factor: i32, out: &mut Vec<WindowEvent>) {
        self.source.integer = factor;
        self.rescale(out, false);
    }

    /// A new scale is one event, which carries the new buffer size along
    /// (`NativeWindow::observe` reads it back): a `Resized` before it would
    /// size the terminal for the old font first, sending the shell a
    /// wrong SIGWINCH. `quiet` leaves a new size to another event that
    /// carries it the same way.
    fn rescale(&mut self, out: &mut Vec<WindowEvent>, quiet: bool) {
        let scale = self.source.factor();
        let size = PhysicalSize::new(scaled(self.logical.0, scale), scaled(self.logical.1, scale));
        let resized = size != self.size;
        self.size = size;
        if scale != self.scale {
            self.scale = scale;
            out.push(WindowEvent::ScaleFactorChanged(scale));
        } else if resized && !quiet {
            out.push(WindowEvent::Resized(size));
        }
    }

    /// Apply a configure (sizes in logical pixels), appending an event for
    /// each change. A frame handed over or back carries the new size too
    /// (sway resizes as it floats or tiles a window), so the terminal is
    /// laid out once, for the right title band.
    fn apply(&mut self, c: Configure, out: &mut Vec<WindowEvent>) {
        self.logical = (c.width.unwrap_or(self.logical.0), c.height.unwrap_or(self.logical.1));
        let decorations = c.server_decorations != self.server_decorations;
        self.rescale(out, decorations);
        if (c.maximized, c.fullscreen) != (self.maximized, self.fullscreen) {
            (self.maximized, self.fullscreen) = (c.maximized, c.fullscreen);
            out.push(WindowEvent::StateChanged { maximized: c.maximized, fullscreen: c.fullscreen });
        }
        if c.activated != self.activated {
            self.activated = c.activated;
            out.push(WindowEvent::Focused(c.activated));
        }
        if decorations {
            self.server_decorations = c.server_decorations;
            out.push(WindowEvent::ServerDecorations(c.server_decorations));
        }
    }
}

/// Per-window state the dispatch handlers update. Lock order: the dispatch
/// queue first, then this.
pub(super) struct WindowShared {
    id: WindowId,
    pub(super) state: Mutex<SharedState>,
}

pub(super) struct SharedState {
    /// Events for the app, drained by `poll_events`.
    pub(super) events: Vec<WindowEvent>,
    pacer: FramePacer,
    reported: Reported,
    /// Shown whenever a pointer enters the window.
    pub(super) cursor: Cursor,
    /// Outputs the surface is on, the latest entered last.
    outputs: Vec<wl_output::WlOutput>,
    /// The viewport destination last committed.
    viewport_size: Option<(u32, u32)>,
    /// The seat and serial of the last button press on the window: a move
    /// or resize the compositor starts must name the press it follows.
    pub(super) ime_allowed: bool,
    pub(super) ime_rect: (PhysicalPosition<f64>, PhysicalSize<f64>),
    pub(super) grab: Option<(wl_seat::WlSeat, u32)>,
}

impl SharedState {
    /// Buffer pixels per logical pixel: pointer positions arrive logical.
    pub(super) fn scale(&self) -> f64 {
        self.reported.scale
    }
}

/// The `xdg_toplevel` edge a resize starts from.
fn resize_edge(direction: ResizeDirection) -> ResizeEdge {
    match direction {
        ResizeDirection::North => ResizeEdge::Top,
        ResizeDirection::South => ResizeEdge::Bottom,
        ResizeDirection::East => ResizeEdge::Right,
        ResizeDirection::West => ResizeEdge::Left,
        ResizeDirection::NorthEast => ResizeEdge::TopRight,
        ResizeDirection::NorthWest => ResizeEdge::TopLeft,
        ResizeDirection::SouthEast => ResizeEdge::BottomRight,
        ResizeDirection::SouthWest => ResizeEdge::BottomLeft,
    }
}

/// The output a window is on: the latest it entered that `describe` still
/// knows, or before any enter the only output there is.
fn pick_output<O, I>(entered: &[O], all: &[O], describe: impl Fn(&O) -> Option<I>) -> Option<I> {
    match entered.iter().rev().find_map(&describe) {
        Some(info) => Some(info),
        None if entered.is_empty() && all.len() == 1 => describe(&all[0]),
        None => None,
    }
}

/// The monitor an output's current mode describes. The position is the
/// output's place in the compositor's (logical) space; the size is the
/// mode's, in physical pixels.
fn monitor(position: (i32, i32), modes: &[Mode]) -> Option<Monitor> {
    let mode = modes.iter().find(|m| m.current)?;
    let (width, height) = mode.dimensions;
    Some(Monitor {
        position: PhysicalPosition::new(position.0, position.1),
        size: PhysicalSize::new(width.max(0) as u32, height.max(0) as u32),
        // Zero means a virtual output with no rate.
        refresh_rate_millihertz: u32::try_from(mode.refresh_rate).ok().filter(|&r| r > 0),
    })
}

/// Globals, input devices and the windows the handlers route events to.
pub(super) struct State {
    registry: RegistryState,
    outputs: OutputState,
    pub(super) compositor: CompositorState,
    pub(super) shm: Shm,
    pub(super) seats: SeatState,
    pub(super) keyboards: Vec<Keyboard>,
    pub(super) pointers: Vec<Pointer>,
    pub(super) repeat: KeyRepeat<ObjectId>,
    /// `None` when no timerfd could be made: keys then do not repeat.
    pub(super) repeat_timer: Option<RepeatTimer>,
    windows: Vec<(ObjectId, Weak<WindowShared>)>,
    pub(super) text_inputs: Vec<super::wayland_text_input::TextInput>,
    pub(super) text_manager: Option<super::wayland_text_input::Manager>,
}

impl State {
    /// The live window whose `wl_surface` is `surface`, pruning closed ones.
    pub(super) fn window(&mut self, surface: &wl_surface::WlSurface) -> Option<Arc<WindowShared>> {
        self.windows.retain(|(_, w)| w.strong_count() > 0);
        let id = surface.id();
        self.windows.iter().find(|(s, _)| *s == id).and_then(|(_, w)| w.upgrade())
    }
}

struct Client {
    conn: Connection,
    qh: QueueHandle<State>,
    xdg: XdgShell,
    viewporter: Option<WpViewporter>,
    fractional: Option<WpFractionalScaleManagerV1>,
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
    let shm = Shm::bind(&globals, &qh).map_err(|e| format!("no wl_shm: {e}"))?;
    // Both optional: without them the window draws at an integer scale, or 1.
    let viewporter = globals.bind(&qh, 1..=1, ()).ok();
    let fractional = globals.bind(&qh, 1..=1, ()).ok();
    let repeat_timer = RepeatTimer::new().map_err(|e| eprintln!("litty: {e}; keys do not repeat")).ok();
    let mut state = State {
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
        seats: SeatState::new(&globals, &qh),
        compositor,
        shm,
        keyboards: Vec::new(),
        pointers: Vec::new(),
        repeat: KeyRepeat::default(),
        repeat_timer,
        windows: Vec::new(),
        text_inputs: Vec::new(),
        text_manager: globals.bind(&qh, 1..=1, ()).ok(),
    };
    for seat in state.seats.seats() {
        state.add_text_input(&qh, seat);
    }
    super::app::register_wayland_fd(conn.backend().poll_fd().as_raw_fd());
    if let Some(timer) = &state.repeat_timer {
        super::app::register_key_repeat_fd(timer.fd());
    }
    Ok(Client { conn, qh, xdg, viewporter, fractional, queue: Mutex::new((queue, state)) })
}

/// The clipboard queue hands off a drop on its own data device: GNOME
/// routes DnD to the last bound device, so binding another would compete.
pub fn receive_file_drop(conn: Connection, offer: smithay_clipboard::DndOffer) {
    let Some(client) = connected() else { offer.destroy(); return; };
    let target = client.queue.lock().1.window(&offer.surface);
    super::wayland_data::receive_drop(conn, offer, target.as_ref().map(Arc::downgrade).unwrap_or_default());
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

/// The key repeat timer fired: queue the repeated key for the next poll.
/// Events already read run first, so a release that arrived with the timer
/// stops the repeat rather than trailing it.
pub fn key_repeat_due() {
    let Some(client) = connected() else { return };
    let mut guard = client.queue.lock();
    let (queue, state) = &mut *guard;
    if let Err(e) = queue.dispatch_pending(state) {
        connection_lost(e);
    }
    state.key_repeat_due();
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
    /// Declared before `window`, so it goes before the surface does.
    scaling: SurfaceScaling,
    window: Window,
    shared: Arc<WindowShared>,
    client: &'static Client,
}

/// A surface's scaling objects. `fractional` exists only with `viewport`:
/// a fractional scale can only be shown through one.
struct SurfaceScaling {
    viewport: Option<WpViewport>,
    fractional: Option<WpFractionalScaleV1>,
}

impl Drop for SurfaceScaling {
    fn drop(&mut self) {
        if let Some(fractional) = self.fractional.take() {
            fractional.destroy();
        }
        if let Some(viewport) = self.viewport.take() {
            viewport.destroy();
        }
    }
}

impl WaylandWindow {
    pub fn create(spec: &WindowSpec, id: WindowId) -> Result<Self, String> {
        let client = client()?;
        let mut guard = client.queue.lock();
        let (queue, state) = &mut *guard;
        let surface = state.compositor.create_surface(&client.qh);
        // Litty draws its own title bar when the spec turns decorations off.
        let decorations = if spec.decorations { WindowDecorations::RequestServer } else { WindowDecorations::RequestClient };
        let window = client.xdg.create_window(surface, decorations, &client.qh);
        window.set_title(spec.title);
        window.set_app_id(APP_ID);
        // The spec is in logical pixels; the scale arrives later (on the
        // first configure's round trips, or once the surface is shown).
        let logical = (spec.inner_size.width.max(100.0).round() as u32, spec.inner_size.height.max(100.0).round() as u32);
        let shared = Arc::new(WindowShared {
            id,
            state: Mutex::new(SharedState {
                events: Vec::new(),
                pacer: FramePacer::default(),
                reported: Reported::new(logical, client.viewporter.is_some(), spec.decorations),
                cursor: Cursor::default(),
                outputs: Vec::new(),
                viewport_size: None,
                grab: None,
                ime_allowed: false,
                ime_rect: (PhysicalPosition::new(0.0, 0.0), PhysicalSize::new(1.0, 1.0)),
            }),
        });
        let surface = window.wl_surface();
        let viewport = client.viewporter.as_ref().map(|v| v.get_viewport(surface, &client.qh, ()));
        let fractional = match (&viewport, &client.fractional) {
            (Some(_), Some(manager)) => Some(manager.get_fractional_scale(surface, &client.qh, Arc::downgrade(&shared))),
            _ => None,
        };
        let scaling = SurfaceScaling { viewport, fractional };

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
        // Input the round trips dispatched waits in the window's queue; the
        // loop must poll it before it blocks.
        super::app::wakeup_main_loop();
        Ok(Self(Arc::new(WaylandWindowInner { scaling, window, shared, client })))
    }

    pub fn set_ime_allowed(&self, allowed: bool) {
        let mut guard = self.0.client.queue.lock();
        self.0.shared.state.lock().ime_allowed = allowed;
        for input in &mut guard.1.text_inputs {
            input.update(&self.0.shared);
        }
    }

    pub fn set_ime_cursor_area(&self, position: PhysicalPosition<f64>, size: PhysicalSize<f64>) {
        let mut guard = self.0.client.queue.lock();
        self.0.shared.state.lock().ime_rect = (position, size);
        for input in &mut guard.1.text_inputs {
            input.update(&self.0.shared);
        }
    }

    /// The buffer size: the configured (or requested) size times the scale.
    pub fn inner_size(&self) -> PhysicalSize<u32> {
        self.0.shared.state.lock().reported.size
    }

    pub fn scale_factor(&self) -> f64 {
        self.0.shared.state.lock().reported.scale
    }

    /// Whether the compositor draws the frame (as configured so far).
    pub fn server_decorations(&self) -> bool {
        self.0.shared.state.lock().reported.server_decorations
    }

    /// `xdg_toplevel.move` for the last press on the window. The
    /// compositor ignores it unless the button is still down.
    pub fn drag_window(&self) -> Result<(), Unsupported> {
        let (seat, serial) = self.0.shared.state.lock().grab.clone().ok_or(Unsupported)?;
        self.0.window.move_(&seat, serial);
        Ok(())
    }

    /// `xdg_toplevel.resize` from `direction`'s edge, for the last press.
    pub fn drag_resize_window(&self, direction: ResizeDirection) -> Result<(), Unsupported> {
        let (seat, serial) = self.0.shared.state.lock().grab.clone().ok_or(Unsupported)?;
        self.0.window.resize(&seat, serial, resize_edge(direction));
        Ok(())
    }

    /// The output the window entered last that the compositor still
    /// describes. A surface enters outputs only once shown, so before that
    /// the only output is the answer; with several there is none yet.
    pub fn current_monitor(&self) -> Option<Monitor> {
        let guard = self.0.client.queue.lock();
        let entered = self.0.shared.state.lock().outputs.clone();
        let all: Vec<_> = guard.1.outputs.outputs().collect();
        let info = pick_output(&entered, &all, |o| guard.1.outputs.info(o))?;
        monitor(info.logical_position.unwrap_or(info.location), &info.modes)
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
    /// callback and set the viewport to the window's logical size, both
    /// sent by the present's commit. A buffer still at the old size is
    /// stretched to it for that frame.
    pub fn pre_present_notify(&self) {
        let (presenting, destination) = {
            let mut st = self.0.shared.state.lock();
            let logical = st.reported.logical;
            let presenting = st.pacer.presenting();
            let changed = presenting && st.viewport_size != Some(logical);
            if changed {
                st.viewport_size = Some(logical);
            }
            (presenting, changed.then_some(logical))
        };
        if !presenting {
            return;
        }
        if let (Some(viewport), Some((w, h))) = (&self.0.scaling.viewport, destination) {
            viewport.set_destination(i32::try_from(w).unwrap_or(i32::MAX), i32::try_from(h).unwrap_or(i32::MAX));
        }
        let surface = self.0.window.wl_surface();
        surface.frame(&self.0.client.qh, surface.clone());
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

    /// Show `icon` (or nothing) while a pointer is over this window. Motion
    /// repeats the same request, which costs nothing here.
    pub fn set_cursor(&self, icon: CursorIcon, visible: bool) {
        let cursor = Cursor { icon, visible };
        if std::mem::replace(&mut self.0.shared.state.lock().cursor, cursor) == cursor {
            return;
        }
        let guard = self.0.client.queue.lock();
        for pointer in guard.1.pointers.iter().filter(|p| p.is_over(&self.0.shared)) {
            pointer.show(&self.0.client.conn, cursor);
        }
    }
}

impl CompositorHandler for State {
    /// The integer scale; a fractional one, where sent, takes precedence.
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, factor: i32) {
        let Some(win) = self.window(surface) else { return };
        {
            let mut st = win.state.lock();
            let SharedState { events, reported, .. } = &mut *st;
            reported.set_integer(factor, events);
        }
        super::app::request_redraw(win.id);
    }

    // Rotated outputs are drawn upright by the compositor.
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, _time: u32) {
        let Some(win) = self.window(surface) else { return };
        let due = win.state.lock().pacer.frame_done();
        if due {
            super::app::request_redraw(win.id);
        }
    }

    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, output: &wl_output::WlOutput) {
        let Some(win) = self.window(surface) else { return };
        let outputs = &mut win.state.lock().outputs;
        outputs.retain(|o| o != output);
        outputs.push(output.clone());
    }

    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, output: &wl_output::WlOutput) {
        let Some(win) = self.window(surface) else { return };
        win.state.lock().outputs.retain(|o| o != output);
    }
}

impl Dispatch<WpFractionalScaleV1, Weak<WindowShared>> for State {
    fn event(
        _: &mut Self,
        _: &WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        window: &Weak<WindowShared>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let wp_fractional_scale_v1::Event::PreferredScale { scale } = event else { return };
        let Some(win) = window.upgrade() else { return };
        {
            let mut st = win.state.lock();
            let SharedState { events, reported, .. } = &mut *st;
            reported.set_fractional(scale, events);
        }
        super::app::request_redraw(win.id);
    }
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }

    // `OutputState` keeps each output's modes for `current_monitor`, and
    // sctk recomputes the integer scale of the surfaces on it.
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
            let SharedState { events, pacer, reported, .. } = &mut *st;
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

    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(State);
delegate_output!(State);
delegate_seat!(State);
delegate_pointer!(State);
delegate_shm!(State);
delegate_xdg_shell!(State);
delegate_xdg_window!(State);
delegate_registry!(State);
// No events: the manager and viewport objects only take requests.
delegate_noop!(State: WpViewporter);
delegate_noop!(State: WpViewport);
delegate_noop!(State: WpFractionalScaleManagerV1);

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
        Reported::new((w, h), true, false)
    }

    fn configure(width: Option<u32>, height: Option<u32>) -> Configure {
        Configure { width, height, maximized: false, fullscreen: false, activated: false, server_decorations: false }
    }

    /// Asked for its own frame, a window that the compositor frames anyway
    /// (sway, tiled) hears so once, and again when the frame is handed back.
    #[test]
    fn forced_server_decorations_are_reported_on_change() {
        let mut r = reported(800, 600);
        let mut out = Vec::new();
        let framed = Configure { server_decorations: true, ..configure(None, None) };
        r.apply(framed, &mut out);
        assert!(matches!(out[..], [WindowEvent::ServerDecorations(true)]), "{out:?}");
        out.clear();
        r.apply(framed, &mut out);
        assert!(out.is_empty(), "{out:?}");
        r.apply(configure(None, None), &mut out);
        assert!(matches!(out[..], [WindowEvent::ServerDecorations(false)]), "{out:?}");
    }

    /// Floating a tiled window on sway hands the frame back and resizes it
    /// in one configure: one event, carrying the size, not a `Resized`
    /// that would lay the terminal out for the old title band first.
    #[test]
    fn a_frame_change_carries_the_new_size() {
        let mut r = reported(1024, 640);
        let mut out = Vec::new();
        r.apply(Configure { server_decorations: true, ..configure(Some(800), Some(500)) }, &mut out);
        assert!(matches!(out[..], [WindowEvent::ServerDecorations(true)]), "{out:?}");
        assert_eq!(r.size, PhysicalSize::new(800, 500));
    }

    /// Each of Litty's resize edges starts the compositor's resize from the
    /// same edge; a swapped axis would grow the window the wrong way.
    #[test]
    fn resize_directions_map_to_their_edges() {
        use ResizeDirection::*;
        let pairs = [
            (North, ResizeEdge::Top),
            (South, ResizeEdge::Bottom),
            (East, ResizeEdge::Right),
            (West, ResizeEdge::Left),
            (NorthEast, ResizeEdge::TopRight),
            (NorthWest, ResizeEdge::TopLeft),
            (SouthEast, ResizeEdge::BottomRight),
            (SouthWest, ResizeEdge::BottomLeft),
        ];
        for (direction, edge) in pairs {
            assert_eq!(resize_edge(direction), edge, "{direction:?}");
        }
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
        let max = Configure { width: Some(1920), height: Some(1080), maximized: true, activated: true, ..configure(None, None) };
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

    fn sizes_and_scales(out: &[WindowEvent]) -> Vec<String> {
        out.iter().map(|e| format!("{e:?}")).collect()
    }

    /// At 125% the buffer has more pixels for the same logical window. The
    /// scale change alone is reported: a `Resized` first would lay the
    /// terminal out for the old font.
    #[test]
    fn a_fractional_scale_draws_more_pixels_at_the_same_size() {
        let mut r = reported(800, 600);
        let mut out = Vec::new();
        r.set_fractional(150, &mut out);
        assert!(
            matches!(out[..], [WindowEvent::ScaleFactorChanged(s)] if s == 1.25),
            "{:?}",
            sizes_and_scales(&out)
        );
        assert_eq!((r.size, r.scale, r.logical), (PhysicalSize::new(1000, 750), 1.25, (800, 600)));
        out.clear();
        r.set_fractional(150, &mut out);
        assert!(out.is_empty(), "the same scale again reports nothing: {out:?}");
    }

    /// A configure is logical: at a scale its buffer is that much larger.
    /// Halves round away from zero (819 × 1.25 = 1023.75, 101 × 1.5 = 151.5).
    #[test]
    fn configured_sizes_are_scaled_and_rounded() {
        let mut r = reported(800, 600);
        let mut out = Vec::new();
        r.set_fractional(150, &mut out);
        out.clear();
        r.apply(configure(Some(819), Some(512)), &mut out);
        assert!(matches!(out[..], [WindowEvent::Resized(PhysicalSize { width: 1024, height: 640 })]), "{out:?}");
        r.set_fractional(180, &mut out);
        out.clear();
        r.apply(configure(Some(101), None), &mut out);
        assert!(matches!(out[..], [WindowEvent::Resized(PhysicalSize { width: 152, height: 768 })]), "{out:?}");
    }

    /// Without fractional scaling the integer scale applies; once a
    /// fractional one is sent it wins, and later integer changes are moot.
    #[test]
    fn a_fractional_scale_wins_over_the_integer_one() {
        let mut r = reported(800, 600);
        let mut out = Vec::new();
        r.set_integer(2, &mut out);
        assert!(matches!(out[..], [WindowEvent::ScaleFactorChanged(s)] if s == 2.0), "{out:?}");
        assert_eq!(r.size, PhysicalSize::new(1600, 1200));
        out.clear();
        r.set_fractional(180, &mut out);
        assert_eq!(r.scale, 1.5);
        out.clear();
        r.set_integer(3, &mut out);
        assert!(out.is_empty(), "{out:?}");
        assert_eq!(r.size, PhysicalSize::new(1200, 900));
    }

    /// With no viewport a buffer cannot be shown smaller than it is, so the
    /// window stays at scale 1 and the compositor upscales it.
    #[test]
    fn without_a_viewport_the_scale_stays_one() {
        let mut r = Reported::new((800, 600), false, false);
        let mut out = Vec::new();
        r.set_fractional(240, &mut out);
        r.set_integer(2, &mut out);
        assert!(out.is_empty(), "{out:?}");
        assert_eq!((r.size, r.scale), (PhysicalSize::new(800, 600), 1.0));
    }

    /// The latest entered output wins; one that is gone is skipped. Before
    /// any enter a single output is the answer, but of two neither is.
    #[test]
    fn the_window_is_on_the_output_it_entered_last() {
        let known = |o: &&str| (*o != "gone").then(|| o.to_string());
        assert_eq!(pick_output(&["a", "b"], &["a", "b"], known).as_deref(), Some("b"));
        assert_eq!(pick_output(&["a", "gone"], &["a"], known).as_deref(), Some("a"));
        assert_eq!(pick_output(&[], &["a"], known).as_deref(), Some("a"));
        assert_eq!(pick_output(&[], &["a", "b"], known), None);
        assert_eq!(pick_output(&["gone"], &["a"], known), None, "it entered one, just not one still known");
    }

    fn mode(dimensions: (i32, i32), refresh_rate: i32, current: bool) -> Mode {
        Mode { dimensions, refresh_rate, current, preferred: false }
    }

    /// The monitor is the output's current mode, not its first or preferred
    /// one; a rate of 0 (virtual outputs) is no rate.
    #[test]
    fn the_monitor_is_the_current_mode() {
        let modes = [mode((3840, 2160), 60_000, false), mode((2560, 1440), 143_912, true)];
        let m = monitor((1920, 0), &modes).expect("a current mode");
        assert_eq!(m.position, PhysicalPosition::new(1920, 0));
        assert_eq!(m.size, PhysicalSize::new(2560, 1440));
        assert_eq!(m.refresh_rate_millihertz, Some(143_912));
        let virtual_output = monitor((0, 0), &[mode((1024, 640), 0, true)]).expect("a current mode");
        assert_eq!(virtual_output.refresh_rate_millihertz, None);
        assert!(monitor((0, 0), &[mode((800, 600), 60_000, false)]).is_none());
    }
}
