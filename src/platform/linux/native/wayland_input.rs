//! Keyboard and pointer input for Wayland windows (phase 2 of the
//! [Wayland backend plan](../../../../docs/plans/wayland-backend.md)).
//!
//! Keys go through the compositor's xkb keymap ([`super::xkb`]) and repeat
//! on the client, at the compositor's rate and delay, from a timerfd the
//! event loop watches. The pointer reports motion, buttons and scrolling, and
//! shows the window's cursor through `wp_cursor_shape_v1`, or the cursor
//! theme where the compositor lacks it.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::{Arc, Weak};

use smithay_client_toolkit::reexports::client::protocol::{wl_keyboard, wl_pointer, wl_seat};
use smithay_client_toolkit::reexports::client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use smithay_client_toolkit::seat::pointer::{
    AxisScroll, CursorIcon as ShapeIcon, PointerEvent, PointerEventKind, PointerHandler, ThemeSpec, ThemedPointer,
};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shm::{Shm, ShmHandler};

use super::events::{HeldKeys, LinuxRawEvent, convert_linux_key, convert_raw_event};
use super::wayland::{State, WindowShared};
use super::xkb::{Keymap, fallback_mods};
use crate::platform::gui::{CursorIcon, PhysicalPosition, ScrollDelta, TouchPhase, WindowEvent};
use crate::platform::input::{Key, KeyInput, Mods, NamedKey};

/// Repeat settings until the compositor sends its own (`repeat_info`).
const DEFAULT_REPEAT_RATE: u32 = 25;
const DEFAULT_REPEAT_DELAY_MS: u32 = 600;
/// A high-resolution wheel notch (`axis_value120`).
const VALUE120_PER_NOTCH: f32 = 120.0;

/// What a key event does to the repeat timer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TimerChange {
    Keep,
    /// Fire after `delay_ms`, then every `interval_ns`.
    Arm { delay_ms: u32, interval_ns: u64 },
    Disarm,
}

/// Which key repeats, decided as a terminal expects: the last repeating key
/// pressed repeats until it is released or focus leaves; modifiers neither
/// repeat nor stop a repeat (holding `a` then Shift repeats `A`).
#[derive(Debug)]
pub(super) struct KeyRepeat<K> {
    /// Keys per second; 0 turns repeat off (the compositor repeats, or nobody does).
    rate: u32,
    delay_ms: u32,
    /// The repeating key: the keyboard it came from and its evdev code.
    key: Option<(K, u32)>,
}

impl<K: PartialEq> Default for KeyRepeat<K> {
    fn default() -> Self {
        Self { rate: DEFAULT_REPEAT_RATE, delay_ms: DEFAULT_REPEAT_DELAY_MS, key: None }
    }
}

impl<K: PartialEq> KeyRepeat<K> {
    /// `wl_keyboard.repeat_info`; a new rate applies from the next press,
    /// but rate 0 (the compositor repeats from now on) stops ours at once.
    pub(super) fn set_info(&mut self, rate: i32, delay_ms: i32) -> TimerChange {
        self.rate = rate.max(0) as u32;
        self.delay_ms = delay_ms.max(0) as u32;
        if self.rate == 0 && self.key.take().is_some() { TimerChange::Disarm } else { TimerChange::Keep }
    }

    pub(super) fn press(&mut self, keyboard: K, evdev: u32, repeats: bool) -> TimerChange {
        if !repeats {
            return TimerChange::Keep;
        }
        if self.rate == 0 {
            self.key = None;
            return TimerChange::Disarm;
        }
        self.key = Some((keyboard, evdev));
        TimerChange::Arm { delay_ms: self.delay_ms, interval_ns: 1_000_000_000 / u64::from(self.rate) }
    }

    pub(super) fn release(&mut self, keyboard: &K, evdev: u32) -> TimerChange {
        match &self.key {
            Some((k, e)) if k == keyboard && *e == evdev => {
                self.key = None;
                TimerChange::Disarm
            }
            _ => TimerChange::Keep,
        }
    }

    /// The keyboard lost focus or went away: nothing it held repeats.
    pub(super) fn stop(&mut self, keyboard: &K) -> TimerChange {
        match &self.key {
            Some((k, _)) if k == keyboard => {
                self.key = None;
                TimerChange::Disarm
            }
            _ => TimerChange::Keep,
        }
    }

    pub(super) fn current(&self) -> Option<&(K, u32)> {
        self.key.as_ref()
    }
}

/// The timerfd key repeat runs on, watched by the event loop.
pub(super) struct RepeatTimer(OwnedFd);

impl RepeatTimer {
    pub(super) fn new() -> Result<Self, String> {
        let fd = unsafe { libc::timerfd_create(libc::CLOCK_MONOTONIC, libc::TFD_NONBLOCK | libc::TFD_CLOEXEC) };
        if fd < 0 {
            return Err(format!("cannot create the key repeat timer: {}", std::io::Error::last_os_error()));
        }
        Ok(Self(unsafe { OwnedFd::from_raw_fd(fd) }))
    }

    pub(super) fn fd(&self) -> i32 {
        self.0.as_raw_fd()
    }

    pub(super) fn apply(&self, change: TimerChange) {
        let spec = match change {
            TimerChange::Keep => return,
            TimerChange::Disarm => timespec_pair(0, 0),
            // A zero delay would disarm the timer: repeat at once instead.
            TimerChange::Arm { delay_ms, interval_ns } => timespec_pair(u64::from(delay_ms.max(1)) * 1_000_000, interval_ns),
        };
        if unsafe { libc::timerfd_settime(self.0.as_raw_fd(), 0, &spec, std::ptr::null_mut()) } != 0 {
            let err = std::io::Error::last_os_error();
            eprintln!("litty: cannot set the key repeat timer ({err})");
        }
    }

    /// Clear a firing; false when the timer has not fired (a stale wakeup).
    pub(super) fn consume(&self) -> bool {
        let mut expirations = 0u64;
        let n = unsafe { libc::read(self.0.as_raw_fd(), (&raw mut expirations).cast(), 8) };
        n == 8 && expirations > 0
    }
}

fn timespec_pair(value_ns: u64, interval_ns: u64) -> libc::itimerspec {
    let ts = |ns: u64| libc::timespec { tv_sec: (ns / 1_000_000_000) as libc::time_t, tv_nsec: (ns % 1_000_000_000) as libc::c_long };
    libc::itimerspec { it_value: ts(value_ns), it_interval: ts(interval_ns) }
}

/// One `wl_keyboard`: its keymap, focus and held keys.
pub(super) struct Keyboard {
    wl: wl_keyboard::WlKeyboard,
    seat: wl_seat::WlSeat,
    /// `None` until the keymap arrives, or without libxkbcommon.
    keymap: Option<Keymap>,
    focus: Option<Weak<WindowShared>>,
    held: HeldKeys,
    /// Raw modifier mask, for keys without a keymap.
    mask: u32,
    /// Modifiers as the compositor last stated them.
    mods: Mods,
    /// Modifiers last reported to the focused window; `None` reports the next.
    reported: Option<Mods>,
}

impl Keyboard {
    fn new(wl: wl_keyboard::WlKeyboard, seat: wl_seat::WlSeat) -> Self {
        Self { wl, seat, keymap: None, focus: None, held: HeldKeys::default(), mask: 0, mods: Mods::default(), reported: None }
    }

    fn key(&self, evdev: u32, pressed: bool, repeat: bool) -> KeyInput {
        match &self.keymap {
            Some(keymap) => keymap.key(evdev, pressed, repeat),
            None => convert_linux_key(evdev, self.mask, pressed, repeat),
        }
    }

    fn repeats(&self, evdev: u32) -> bool {
        match &self.keymap {
            Some(keymap) => keymap.repeats(evdev),
            // Without a keymap, every key but the modifiers and locks repeats.
            None => !matches!(evdev, 29 | 42 | 54 | 56 | 58 | 69 | 70 | 97 | 100 | 125 | 126),
        }
    }

    fn focused(&self) -> Option<Arc<WindowShared>> {
        self.focus.as_ref()?.upgrade()
    }

    /// Report the modifiers a key leaves held before the key itself, as
    /// X11 does: compositors send `modifiers` after the key, and a Shift
    /// press must already carry Shift (Kitty's "report all keys").
    fn report_mods_for(&mut self, key: &KeyInput, win: &WindowShared) {
        let mods = mods_with_key(self.reported.unwrap_or(self.mods), key);
        if self.reported != Some(mods) {
            self.reported = Some(mods);
            deliver(win, WindowEvent::ModifiersChanged(mods));
        }
    }
}

/// Whether a key event is reported, and as a repeat: a release without a
/// reported press (held before focus came) is dropped, as on X11. Codes
/// that do not fit X11's byte keycodes (evdev + 8) are not tracked.
fn track(held: &HeldKeys, evdev: u32, pressed: bool) -> Option<bool> {
    let Some(code) = evdev.checked_add(8).and_then(|c| u8::try_from(c).ok()) else { return Some(false) };
    let was_held = held.update(code, pressed);
    (pressed || was_held).then_some(pressed && was_held)
}

/// `mods` once a modifier key is pressed or released; other keys leave them.
fn mods_with_key(mut mods: Mods, key: &KeyInput) -> Mods {
    let slot = match key.logical {
        Key::Named(NamedKey::Shift) => &mut mods.shift,
        Key::Named(NamedKey::Control) => &mut mods.ctrl,
        Key::Named(NamedKey::Alt) => &mut mods.alt,
        Key::Named(NamedKey::Super) => &mut mods.sup,
        _ => return mods,
    };
    *slot = key.pressed;
    mods
}

/// Add an event to a window's queue from a dispatch handler.
fn deliver(win: &WindowShared, event: WindowEvent) {
    win.state.lock().events.push(event);
}

impl State {
    fn keyboard_event(&mut self, keyboard: &wl_keyboard::WlKeyboard, event: wl_keyboard::Event) {
        let Some(index) = self.keyboards.iter().position(|k| k.wl == *keyboard) else { return };
        let id = keyboard.id();
        match event {
            wl_keyboard::Event::Keymap { format, fd, size } => {
                let kb = &mut self.keyboards[index];
                kb.keymap = match format {
                    WEnum::Value(wl_keyboard::KeymapFormat::XkbV1) => match Keymap::from_fd(&fd, size) {
                        Ok(keymap) => keymap,
                        Err(e) => {
                            eprintln!("litty: {e}; Wayland keys use a US layout");
                            None
                        }
                    },
                    // `no_keymap`: the compositor offers none; keys are evdev codes.
                    _ => None,
                };
            }
            wl_keyboard::Event::Enter { surface, .. } => {
                let win = self.window(&surface);
                let kb = &mut self.keyboards[index];
                kb.focus = win.as_ref().map(Arc::downgrade);
                kb.held.clear();
                kb.reported = None;
            }
            wl_keyboard::Event::Leave { .. } => {
                let kb = &mut self.keyboards[index];
                kb.focus = None;
                kb.held.clear();
                let change = self.repeat.stop(&id);
                self.apply_repeat(change);
            }
            wl_keyboard::Event::Key { key: evdev, state, .. } => {
                let kb = &mut self.keyboards[index];
                let Some(win) = kb.focused() else { return };
                let (pressed, compositor_repeat) = match state {
                    WEnum::Value(wl_keyboard::KeyState::Pressed) => (true, false),
                    WEnum::Value(wl_keyboard::KeyState::Repeated) => (true, true),
                    WEnum::Value(wl_keyboard::KeyState::Released) => (false, false),
                    // A state newer than this client knows: neither press nor release.
                    _ => return,
                };
                // A compositor repeat of a key pressed before focus came is
                // its first report: a press, as X11 would show it.
                let Some(repeat) = track(&kb.held, evdev, pressed) else { return };
                let key = kb.key(evdev, pressed, repeat);
                kb.report_mods_for(&key, &win);
                deliver(&win, WindowEvent::Key(key));
                let change = if compositor_repeat {
                    TimerChange::Keep
                } else if pressed {
                    let repeats = kb.repeats(evdev);
                    self.repeat.press(id, evdev, repeats)
                } else {
                    self.repeat.release(&id, evdev)
                };
                self.apply_repeat(change);
            }
            wl_keyboard::Event::Modifiers { mods_depressed, mods_latched, mods_locked, group, .. } => {
                let kb = &mut self.keyboards[index];
                kb.mask = mods_depressed | mods_latched | mods_locked;
                let mods = match &mut kb.keymap {
                    Some(keymap) => {
                        keymap.update_mask(mods_depressed, mods_latched, mods_locked, group);
                        keymap.mods()
                    }
                    None => fallback_mods(mods_depressed, mods_latched, mods_locked),
                };
                kb.mods = mods;
                if kb.reported != Some(mods)
                    && let Some(win) = kb.focused()
                {
                    kb.reported = Some(mods);
                    deliver(&win, WindowEvent::ModifiersChanged(mods));
                }
            }
            wl_keyboard::Event::RepeatInfo { rate, delay } => {
                let change = self.repeat.set_info(rate, delay);
                self.apply_repeat(change);
            }
            _ => {}
        }
    }

    fn apply_repeat(&self, change: TimerChange) {
        if let Some(timer) = &self.repeat_timer {
            timer.apply(change);
        }
    }

    /// The repeat timer fired: the held key repeats into the focused window.
    pub(super) fn key_repeat_due(&mut self) {
        let Some(timer) = &self.repeat_timer else { return };
        if !timer.consume() {
            return;
        }
        let Some((id, evdev)) = self.repeat.current().cloned() else { return };
        let target = self.keyboards.iter().find(|k| k.wl.id() == id).and_then(|kb| Some((kb, kb.focused()?)));
        match target {
            Some((kb, win)) => deliver(&win, WindowEvent::Key(kb.key(evdev, true, true))),
            None => {
                let change = self.repeat.stop(&id);
                self.apply_repeat(change);
            }
        }
    }

    fn remove_keyboards(&mut self, seat: &wl_seat::WlSeat) {
        let mut removed = Vec::new();
        self.keyboards.retain(|kb| {
            let keep = kb.seat != *seat;
            if !keep {
                if kb.wl.version() >= 3 {
                    kb.wl.release();
                }
                removed.push(kb.wl.id());
            }
            keep
        });
        for id in removed {
            let change = self.repeat.stop(&id);
            self.apply_repeat(change);
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for State {
    fn event(
        state: &mut Self,
        keyboard: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.keyboard_event(keyboard, event);
    }
}

/// Touchpad scrolling as one gesture: `Started` on its first movement,
/// `Ended` when the compositor says the fingers stopped.
#[derive(Debug, Default)]
pub(super) struct ScrollGesture {
    active: bool,
}

impl ScrollGesture {
    /// Events for one frame's vertical scroll. Wheels scroll in notches
    /// (high-resolution `value120` where sent); touchpads and other
    /// continuous sources scroll in pixels. Wayland's axis points down,
    /// Litty's up.
    pub(super) fn events(&mut self, v: &AxisScroll, source: Option<wl_pointer::AxisSource>, out: &mut Vec<WindowEvent>) {
        let notches = if v.value120 != 0 {
            Some(v.value120 as f32 / VALUE120_PER_NOTCH)
        } else if v.discrete != 0 {
            Some(v.discrete as f32)
        } else {
            None
        };
        let wheel = matches!(source, Some(wl_pointer::AxisSource::Wheel | wl_pointer::AxisSource::WheelTilt));
        match notches {
            Some(n) => out.push(WindowEvent::MouseWheel { delta: ScrollDelta::Lines { y: -n }, phase: TouchPhase::Moved }),
            // A wheel's pixel amount without notches belongs to a frame
            // whose notches were already counted (or follow); counting it
            // too would scroll twice.
            None if wheel => {}
            None if v.absolute != 0.0 => {
                // Only fingers end a gesture (`axis_stop`); other continuous
                // sources (trackpoints) just move.
                let phase = if source != Some(wl_pointer::AxisSource::Finger) || std::mem::replace(&mut self.active, true) {
                    TouchPhase::Moved
                } else {
                    TouchPhase::Started
                };
                out.push(WindowEvent::MouseWheel { delta: ScrollDelta::Pixels(PhysicalPosition::new(0.0, -v.absolute)), phase });
            }
            None => {}
        }
        if v.stop {
            self.end(out);
        }
    }

    /// The fingers lifted, or the pointer left mid-gesture.
    pub(super) fn end(&mut self, out: &mut Vec<WindowEvent>) {
        if std::mem::take(&mut self.active) {
            out.push(WindowEvent::MouseWheel { delta: ScrollDelta::Pixels(PhysicalPosition::new(0.0, 0.0)), phase: TouchPhase::Ended });
        }
    }
}

/// The cursor a window asks for while the pointer is over it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Cursor {
    pub icon: CursorIcon,
    pub visible: bool,
}

impl Default for Cursor {
    fn default() -> Self {
        Self { icon: CursorIcon::Default, visible: true }
    }
}

/// The CSS cursor name for `wp_cursor_shape_v1` and cursor themes.
fn shape_icon(icon: CursorIcon) -> ShapeIcon {
    use CursorIcon::*;
    match icon {
        Default => ShapeIcon::Default,
        ContextMenu => ShapeIcon::ContextMenu,
        Help => ShapeIcon::Help,
        Pointer => ShapeIcon::Pointer,
        Progress => ShapeIcon::Progress,
        Wait => ShapeIcon::Wait,
        Cell => ShapeIcon::Cell,
        Crosshair => ShapeIcon::Crosshair,
        Text => ShapeIcon::Text,
        VerticalText => ShapeIcon::VerticalText,
        Alias => ShapeIcon::Alias,
        Copy => ShapeIcon::Copy,
        Move => ShapeIcon::Move,
        NoDrop => ShapeIcon::NoDrop,
        NotAllowed => ShapeIcon::NotAllowed,
        Grab => ShapeIcon::Grab,
        Grabbing => ShapeIcon::Grabbing,
        EResize => ShapeIcon::EResize,
        NResize => ShapeIcon::NResize,
        NeResize => ShapeIcon::NeResize,
        NwResize => ShapeIcon::NwResize,
        SResize => ShapeIcon::SResize,
        SeResize => ShapeIcon::SeResize,
        SwResize => ShapeIcon::SwResize,
        WResize => ShapeIcon::WResize,
        EwResize => ShapeIcon::EwResize,
        NsResize => ShapeIcon::NsResize,
        NeswResize => ShapeIcon::NeswResize,
        NwseResize => ShapeIcon::NwseResize,
        ColResize => ShapeIcon::ColResize,
        RowResize => ShapeIcon::RowResize,
        AllScroll => ShapeIcon::AllScroll,
        ZoomIn => ShapeIcon::ZoomIn,
        ZoomOut => ShapeIcon::ZoomOut,
    }
}

/// One `wl_pointer` and the window it is over.
pub(super) struct Pointer {
    themed: ThemedPointer,
    seat: wl_seat::WlSeat,
    over: Option<Weak<WindowShared>>,
    scroll: ScrollGesture,
}

impl Pointer {
    pub(super) fn is_over(&self, win: &Arc<WindowShared>) -> bool {
        self.over.as_ref().is_some_and(|w| w.as_ptr() == Arc::as_ptr(win))
    }

    /// Show `cursor`. A theme without the icon falls back to the default
    /// arrow; a failure is reported once rather than per motion.
    pub(super) fn show(&self, conn: &Connection, cursor: Cursor) {
        let result = if cursor.visible {
            self.themed
                .set_cursor(conn, shape_icon(cursor.icon))
                .or_else(|_| self.themed.set_cursor(conn, ShapeIcon::Default))
        } else {
            self.themed.hide_cursor()
        };
        if let Err(e) = result {
            static WARNED: std::sync::Once = std::sync::Once::new();
            WARNED.call_once(|| eprintln!("litty: cannot set the Wayland cursor: {e}"));
        }
    }
}

impl PointerHandler for State {
    fn pointer_frame(&mut self, conn: &Connection, _: &QueueHandle<Self>, pointer: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        let Some(index) = self.pointers.iter().position(|p| p.themed.pointer() == pointer) else { return };
        for event in events {
            let Some(win) = self.window(&event.surface) else { continue };
            let p = &mut self.pointers[index];
            let (x, y) = event.position;
            // Logical and physical pixels agree until scaling lands (phase 3).
            let moved = || convert_raw_event(LinuxRawEvent::Motion { x, y }, 1.0);
            if let PointerEventKind::Enter { .. } = event.kind {
                p.over = Some(Arc::downgrade(&win));
                // The cursor is per enter: the compositor forgets it on
                // leave. A theme may load from disk: no window lock held.
                let cursor = win.state.lock().cursor;
                p.show(conn, cursor);
            }
            let mut st = win.state.lock();
            match event.kind {
                PointerEventKind::Enter { .. } => st.events.extend(moved()),
                PointerEventKind::Leave { .. } => {
                    p.over = None;
                    p.scroll.end(&mut st.events);
                }
                PointerEventKind::Motion { .. } => st.events.extend(moved()),
                PointerEventKind::Press { button, .. } | PointerEventKind::Release { button, .. } => {
                    let pressed = matches!(event.kind, PointerEventKind::Press { .. });
                    st.events.extend(convert_raw_event(LinuxRawEvent::Button { button, pressed }, 1.0));
                }
                // Litty scrolls vertically only, as on X11.
                PointerEventKind::Axis { ref vertical, source, .. } => p.scroll.events(vertical, source, &mut st.events),
            }
        }
    }
}

impl State {
    fn remove_pointers(&mut self, seat: &wl_seat::WlSeat) {
        // Dropping a `ThemedPointer` releases the pointer and its cursor surface.
        self.pointers.retain(|p| p.seat != *seat);
    }
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seats
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        match capability {
            Capability::Keyboard => {
                let wl = seat.get_keyboard(qh, ());
                self.keyboards.push(Keyboard::new(wl, seat));
            }
            Capability::Pointer => {
                let surface = self.compositor.create_surface(qh);
                match self.seats.get_pointer_with_theme(qh, &seat, self.shm.wl_shm(), surface.clone(), ThemeSpec::System) {
                    Ok(themed) => self.pointers.push(Pointer { themed, seat, over: None, scroll: ScrollGesture::default() }),
                    Err(e) => {
                        surface.destroy();
                        eprintln!("litty: cannot use the Wayland pointer: {e}");
                    }
                }
            }
            _ => {}
        }
    }

    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        match capability {
            Capability::Keyboard => self.remove_keyboards(&seat),
            Capability::Pointer => self.remove_pointers(&seat),
            _ => {}
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        self.remove_keyboards(&seat);
        self.remove_pointers(&seat);
    }
}

impl ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: u32 = 30;
    const B: u32 = 48;
    const SHIFT: u32 = 42;

    #[test]
    fn a_held_key_repeats_at_the_compositors_rate_and_delay() {
        let mut r = KeyRepeat::<u8>::default();
        assert_eq!(r.set_info(40, 300), TimerChange::Keep);
        assert_eq!(r.press(1, A, true), TimerChange::Arm { delay_ms: 300, interval_ns: 25_000_000 });
        assert_eq!(r.current(), Some(&(1, A)));
        assert_eq!(r.release(&1, A), TimerChange::Disarm);
        assert_eq!(r.current(), None);
    }

    /// Repeat follows the newest key; releasing an older one leaves it.
    #[test]
    fn the_last_pressed_key_repeats() {
        let mut r = KeyRepeat::<u8>::default();
        r.press(1, A, true);
        assert!(matches!(r.press(1, B, true), TimerChange::Arm { .. }));
        assert_eq!(r.release(&1, A), TimerChange::Keep);
        assert_eq!(r.current(), Some(&(1, B)));
    }

    /// Holding `a` then pressing Shift keeps repeating (now `A`).
    #[test]
    fn modifiers_neither_repeat_nor_stop_a_repeat() {
        let mut r = KeyRepeat::<u8>::default();
        r.press(1, A, true);
        assert_eq!(r.press(1, SHIFT, false), TimerChange::Keep);
        assert_eq!(r.release(&1, SHIFT), TimerChange::Keep);
        assert_eq!(r.current(), Some(&(1, A)));
    }

    /// A compositor that takes over repeat mid-hold (rate 0) stops ours,
    /// or the key would repeat twice as fast.
    #[test]
    fn rate_zero_mid_hold_stops_the_repeat() {
        let mut r = KeyRepeat::<u8>::default();
        r.press(1, A, true);
        assert_eq!(r.set_info(30, 400), TimerChange::Keep, "a new rate waits for the next press");
        assert_eq!(r.current(), Some(&(1, A)));
        assert_eq!(r.set_info(0, 0), TimerChange::Disarm);
        assert_eq!(r.current(), None);
    }

    /// Keys held before focus came: their release is not reported, nor is
    /// a press reported as a repeat; a compositor repeat of such a key is
    /// its first press.
    #[test]
    fn releases_without_a_reported_press_are_dropped() {
        let held = HeldKeys::default();
        assert_eq!(track(&held, A, false), None, "release of a key pressed elsewhere");
        assert_eq!(track(&held, A, true), Some(false), "first press (or first compositor repeat)");
        assert_eq!(track(&held, A, true), Some(true), "then repeats");
        assert_eq!(track(&held, A, false), Some(false));
        assert_eq!(track(&held, u32::MAX, true), Some(false), "untrackable codes never overflow");
    }

    fn named(name: NamedKey, pressed: bool) -> KeyInput {
        KeyInput {
            logical: Key::Named(name),
            unshifted: Key::Named(name),
            physical: crate::platform::input::PhysicalKey::Code(crate::platform::input::KeyCode::Other(0)),
            location: crate::platform::input::KeyLocation::Left,
            text: None,
            pressed,
            repeat: false,
        }
    }

    /// The modifiers sent before a modifier key include its own change,
    /// as X11 reports them (`mods_after_key`).
    #[test]
    fn a_modifier_key_carries_its_own_state() {
        let shift = Mods { shift: true, ..Mods::default() };
        assert_eq!(mods_with_key(Mods::default(), &named(NamedKey::Shift, true)), shift);
        assert_eq!(mods_with_key(shift, &named(NamedKey::Shift, false)), Mods::default());
        assert_eq!(mods_with_key(shift, &named(NamedKey::Control, true)), Mods { ctrl: true, ..shift });
        assert_eq!(mods_with_key(shift, &named(NamedKey::Enter, true)), shift, "other keys leave them");
    }

    /// A rate of 0 means no client repeat (from version 10 the compositor
    /// sends repeated keys itself).
    #[test]
    fn rate_zero_turns_repeat_off() {
        let mut r = KeyRepeat::<u8>::default();
        r.set_info(0, 500);
        assert_eq!(r.press(1, A, true), TimerChange::Disarm);
        assert_eq!(r.current(), None);
    }

    #[test]
    fn losing_focus_stops_only_that_keyboards_repeat() {
        let mut r = KeyRepeat::<u8>::default();
        r.press(1, A, true);
        assert_eq!(r.stop(&2), TimerChange::Keep);
        assert_eq!(r.stop(&1), TimerChange::Disarm);
        assert_eq!(r.current(), None);
    }

    #[test]
    fn the_repeat_timer_fires_after_the_delay() {
        let Ok(timer) = RepeatTimer::new() else { return };
        assert!(!timer.consume(), "an unarmed timer has not fired");
        timer.apply(TimerChange::Arm { delay_ms: 1, interval_ns: 1_000_000_000 });
        let mut pfd = libc::pollfd { fd: timer.fd(), events: libc::POLLIN, revents: 0 };
        assert_eq!(unsafe { libc::poll(&mut pfd, 1, 5_000) }, 1, "fired within the timeout");
        assert!(timer.consume());
        timer.apply(TimerChange::Disarm);
        assert!(!timer.consume());
    }

    fn wheel(value120: i32, discrete: i32, absolute: f64) -> AxisScroll {
        AxisScroll { absolute, discrete, value120, ..AxisScroll::default() }
    }

    fn lines(events: &[WindowEvent]) -> Vec<f32> {
        events
            .iter()
            .map(|e| match e {
                WindowEvent::MouseWheel { delta: ScrollDelta::Lines { y }, phase: TouchPhase::Moved } => *y,
                other => panic!("expected a wheel notch, got {other:?}"),
            })
            .collect()
    }

    /// Wayland's axis points down, Litty's up; `value120` counts fractions
    /// of a notch on high-resolution wheels.
    #[test]
    fn wheel_notches_scroll_lines_upward_positive() {
        let mut g = ScrollGesture::default();
        let mut out = Vec::new();
        g.events(&wheel(-120, -1, -15.0), Some(wl_pointer::AxisSource::Wheel), &mut out);
        g.events(&wheel(60, 0, 7.5), Some(wl_pointer::AxisSource::Wheel), &mut out);
        g.events(&wheel(0, 2, 30.0), Some(wl_pointer::AxisSource::Wheel), &mut out);
        assert_eq!(lines(&out), [1.0, -0.5, -2.0]);
    }

    /// A wheel frame with pixels but no notch is not scrolled twice.
    #[test]
    fn wheel_pixels_without_a_notch_are_ignored() {
        let mut out = Vec::new();
        ScrollGesture::default().events(&wheel(0, 0, 15.0), Some(wl_pointer::AxisSource::Wheel), &mut out);
        assert!(out.is_empty(), "{out:?}");
    }

    /// A trackpoint never sends `axis_stop`: its scrolling must not open a
    /// gesture that later ones never restart.
    #[test]
    fn continuous_scrolling_without_stop_just_moves() {
        let mut g = ScrollGesture::default();
        let mut out = Vec::new();
        g.events(&wheel(0, 0, 3.0), Some(wl_pointer::AxisSource::Continuous), &mut out);
        assert!(matches!(out[..], [WindowEvent::MouseWheel { phase: TouchPhase::Moved, .. }]), "{out:?}");
        out.clear();
        g.events(&wheel(0, 0, 3.0), Some(wl_pointer::AxisSource::Finger), &mut out);
        assert!(matches!(out[..], [WindowEvent::MouseWheel { phase: TouchPhase::Started, .. }]), "{out:?}");
        out.clear();
        g.end(&mut out);
        assert!(matches!(out[..], [WindowEvent::MouseWheel { phase: TouchPhase::Ended, .. }]), "leaving ends it: {out:?}");
    }

    #[test]
    fn touchpad_scrolling_is_one_gesture_in_pixels() {
        let mut g = ScrollGesture::default();
        let mut out = Vec::new();
        let finger = Some(wl_pointer::AxisSource::Finger);
        g.events(&wheel(0, 0, 4.0), finger, &mut out);
        g.events(&wheel(0, 0, 6.0), finger, &mut out);
        g.events(&AxisScroll { stop: true, ..AxisScroll::default() }, finger, &mut out);
        g.events(&AxisScroll { stop: true, ..AxisScroll::default() }, finger, &mut out);
        let seen: Vec<_> = out
            .iter()
            .map(|e| match e {
                WindowEvent::MouseWheel { delta: ScrollDelta::Pixels(p), phase } => (p.y, *phase),
                other => panic!("expected a pixel scroll, got {other:?}"),
            })
            .collect();
        assert_eq!(seen, [(-4.0, TouchPhase::Started), (-6.0, TouchPhase::Moved), (0.0, TouchPhase::Ended)], "one Ended only");
    }

    /// Every icon maps to the cursor of the same CSS name.
    #[test]
    fn cursor_icons_map_to_their_css_names() {
        use CursorIcon::*;
        for icon in [
            Default, ContextMenu, Help, Pointer, Progress, Wait, Cell, Crosshair, Text, VerticalText, Alias, Copy, Move,
            NoDrop, NotAllowed, Grab, Grabbing, EResize, NResize, NeResize, NwResize, SResize, SeResize, SwResize, WResize,
            EwResize, NsResize, NeswResize, NwseResize, ColResize, RowResize, AllScroll, ZoomIn, ZoomOut,
        ] {
            assert_eq!(shape_icon(icon).name().parse::<CursorIcon>(), Ok(icon), "{icon:?}");
        }
    }
}
