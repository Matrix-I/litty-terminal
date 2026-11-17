//! Desktop notifications through `org.freedesktop.Notifications` on the
//! session bus, so a click can select the pane that sent it. The bus opens
//! with the first notification; without one, `show` reports `Unsupported`
//! and the caller uses `notify-send`.

use std::collections::VecDeque;
use std::ffi::{CStr, CString};
use std::io;
use std::os::fd::RawFd;

use parking_lot::Mutex;

use super::dbus::{
    self, Conn, DBusLib, DBusMessageIter, Msg, DBUS_MESSAGE_TYPE_ERROR, DBUS_MESSAGE_TYPE_METHOD_RETURN,
    DBUS_MESSAGE_TYPE_SIGNAL, DBUS_TYPE_ARRAY, DBUS_TYPE_DICT_ENTRY, DBUS_TYPE_INT32, DBUS_TYPE_STRING,
    DBUS_TYPE_UINT32, DBUS_TYPE_VARIANT,
};

const NOTIFY_DEST: &CStr = c"org.freedesktop.Notifications";
const NOTIFY_PATH: &CStr = c"/org/freedesktop/Notifications";
const NOTIFY_IFACE: &CStr = c"org.freedesktop.Notifications";
/// Only the notification server's own broadcasts are routed here.
const SIGNAL_MATCH: &CStr = c"type='signal',sender='org.freedesktop.Notifications',\
interface='org.freedesktop.Notifications',path='/org/freedesktop/Notifications'";
const APP_NAME: &CStr = c"Litty";
/// The desktop entry (`resources/linux/litty.desktop`) gives the icon and name.
const DESKTOP_ENTRY: &CStr = c"litty";
/// The action servers invoke when the notification itself is clicked.
const DEFAULT_ACTION: &CStr = c"default";
/// Notifications remembered at once, both waiting for their `Notify` reply
/// and while shown; the oldest is forgotten first, losing only its click.
const MAX_TRACKED: usize = 32;

/// The pane a notification's click selects.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Route {
    origin: String,
    pane: u64,
}

/// A `Notify` call awaiting its reply; the text is kept for the fallback.
struct Sent {
    serial: u32,
    route: Route,
    title: String,
    body: String,
}

/// A notification the server shows, with the token it sent for it.
struct Live {
    id: u32,
    route: Route,
    token: Option<String>,
}

/// A message from the bus, as far as notifications are concerned.
#[derive(Debug, PartialEq, Eq)]
enum Incoming {
    /// `Notify` answered with the notification's id.
    Shown { serial: u32, id: u32 },
    /// `Notify` answered with an error (or no id).
    Failed { serial: u32 },
    /// `ActivationToken`: sent just before `ActionInvoked`.
    Token { id: u32, token: String },
    Invoked { id: u32, action: String },
    Closed { id: u32 },
    Other,
}

/// What the event loop does after reading the bus.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Effect {
    /// The server refused the notification: show it with `notify-send`.
    Fallback { title: String, body: String },
    /// The user clicked it: select `pane`, focusing with `token`.
    Click { origin: String, pane: u64, token: Option<String> },
}

/// Which notifications this process sent, and what their events mean.
#[derive(Default)]
struct Tracker {
    sent: VecDeque<Sent>,
    live: VecDeque<Live>,
}

impl Tracker {
    fn sent(&mut self, serial: u32, route: Route, title: String, body: String) {
        if self.sent.len() == MAX_TRACKED {
            self.sent.pop_front();
        }
        self.sent.push_back(Sent { serial, route, title, body });
    }

    /// The bus went away: every call it never answered falls back.
    fn abandon(&mut self, effects: &mut Vec<Effect>) {
        effects.extend(self.sent.drain(..).map(|sent| Effect::Fallback { title: sent.title, body: sent.body }));
        self.live.clear();
    }

    fn take_sent(&mut self, serial: u32) -> Option<Sent> {
        let at = self.sent.iter().position(|s| s.serial == serial)?;
        self.sent.remove(at)
    }

    fn take_live(&mut self, id: u32) -> Option<Live> {
        let at = self.live.iter().position(|l| l.id == id)?;
        self.live.remove(at)
    }

    fn on(&mut self, incoming: Incoming) -> Option<Effect> {
        match incoming {
            Incoming::Shown { serial, id } => {
                let sent = self.take_sent(serial)?;
                if self.live.len() == MAX_TRACKED {
                    self.live.pop_front();
                }
                self.live.push_back(Live { id, route: sent.route, token: None });
                None
            }
            Incoming::Failed { serial } => {
                let sent = self.take_sent(serial)?;
                Some(Effect::Fallback { title: sent.title, body: sent.body })
            }
            Incoming::Token { id, token } => {
                if let Some(live) = self.live.iter_mut().find(|l| l.id == id) {
                    live.token = Some(token);
                }
                None
            }
            Incoming::Invoked { id, action } => {
                if action.as_bytes() != DEFAULT_ACTION.to_bytes() {
                    return None;
                }
                let live = self.take_live(id)?;
                Some(Effect::Click { origin: live.route.origin, pane: live.route.pane, token: live.token })
            }
            Incoming::Closed { id } => {
                self.take_live(id);
                None
            }
            Incoming::Other => None,
        }
    }
}

impl DBusLib {
    unsafe fn append_str(&self, iter: &mut DBusMessageIter, s: &CStr) -> bool {
        let ptr = s.as_ptr();
        unsafe { (self.dbus_message_iter_append_basic)(iter, DBUS_TYPE_STRING, (&ptr as *const *const std::ffi::c_char).cast()) != 0 }
    }

    unsafe fn append_u32(&self, iter: &mut DBusMessageIter, value: u32) -> bool {
        unsafe { (self.dbus_message_iter_append_basic)(iter, DBUS_TYPE_UINT32, (&value as *const u32).cast()) != 0 }
    }

    unsafe fn append_i32(&self, iter: &mut DBusMessageIter, value: i32) -> bool {
        unsafe { (self.dbus_message_iter_append_basic)(iter, DBUS_TYPE_INT32, (&value as *const i32).cast()) != 0 }
    }

    /// `as`: the default action, labelled for servers that show buttons.
    unsafe fn append_actions(&self, args: &mut DBusMessageIter) -> bool {
        let mut actions = DBusMessageIter::new();
        unsafe {
            (self.dbus_message_iter_open_container)(args, DBUS_TYPE_ARRAY, c"s".as_ptr(), &mut actions) != 0
                && self.append_str(&mut actions, DEFAULT_ACTION)
                && self.append_str(&mut actions, c"Open")
                && (self.dbus_message_iter_close_container)(args, &mut actions) != 0
        }
    }

    /// `a{sv}`: `desktop-entry`, so the server shows Litty's icon and name.
    unsafe fn append_hints(&self, args: &mut DBusMessageIter) -> bool {
        let (mut hints, mut entry, mut value) = (DBusMessageIter::new(), DBusMessageIter::new(), DBusMessageIter::new());
        unsafe {
            (self.dbus_message_iter_open_container)(args, DBUS_TYPE_ARRAY, c"{sv}".as_ptr(), &mut hints) != 0
                && (self.dbus_message_iter_open_container)(&mut hints, DBUS_TYPE_DICT_ENTRY, std::ptr::null(), &mut entry) != 0
                && self.append_str(&mut entry, c"desktop-entry")
                && (self.dbus_message_iter_open_container)(&mut entry, DBUS_TYPE_VARIANT, c"s".as_ptr(), &mut value) != 0
                && self.append_str(&mut value, DESKTOP_ENTRY)
                && (self.dbus_message_iter_close_container)(&mut entry, &mut value) != 0
                && (self.dbus_message_iter_close_container)(&mut hints, &mut entry) != 0
                && (self.dbus_message_iter_close_container)(args, &mut hints) != 0
        }
    }

    /// A `Notify` call for `title` and `body`; null when libdbus runs out
    /// of memory (its only reason to refuse an append).
    unsafe fn notify_call(&self, title: &CStr, body: &CStr) -> *mut Msg {
        unsafe {
            let msg = (self.dbus_message_new_method_call)(
                NOTIFY_DEST.as_ptr(), NOTIFY_PATH.as_ptr(), NOTIFY_IFACE.as_ptr(), c"Notify".as_ptr(),
            );
            if msg.is_null() {
                return msg;
            }
            let mut args = DBusMessageIter::new();
            (self.dbus_message_iter_init_append)(msg, &mut args);
            let complete = self.append_str(&mut args, APP_NAME)
                && self.append_u32(&mut args, 0) // replaces_id: a new notification
                && self.append_str(&mut args, c"") // app_icon: the desktop entry's
                && self.append_str(&mut args, title)
                && self.append_str(&mut args, body)
                && self.append_actions(&mut args)
                && self.append_hints(&mut args)
                && self.append_i32(&mut args, -1); // expire_timeout: the server's default
            if !complete {
                (self.dbus_message_unref)(msg);
                return std::ptr::null_mut();
            }
            msg
        }
    }

    /// The `u32` at `iter`, if it holds one.
    ///
    /// # Safety
    /// `iter` must be a valid iterator into a live message.
    unsafe fn u32_at(&self, iter: &mut DBusMessageIter) -> Option<u32> {
        unsafe {
            if (self.dbus_message_iter_get_arg_type)(iter) != DBUS_TYPE_UINT32 {
                return None;
            }
            let mut value = 0u32;
            (self.dbus_message_iter_get_basic)(iter, (&mut value as *mut u32).cast());
            Some(value)
        }
    }

    /// What `msg` means for the notifications this connection sent.
    ///
    /// # Safety
    /// `msg` must be a live message.
    unsafe fn notification_incoming(&self, msg: *mut Msg) -> Incoming {
        unsafe {
            let kind = (self.dbus_message_get_type)(msg);
            if kind == DBUS_MESSAGE_TYPE_SIGNAL {
                return self.notification_signal(msg);
            }
            if !matches!(kind, DBUS_MESSAGE_TYPE_METHOD_RETURN | DBUS_MESSAGE_TYPE_ERROR) {
                return Incoming::Other;
            }
            let serial = (self.dbus_message_get_reply_serial)(msg);
            let mut iter = DBusMessageIter::new();
            if kind == DBUS_MESSAGE_TYPE_METHOD_RETURN
                && (self.dbus_message_iter_init)(msg, &mut iter) != 0
                && let Some(id) = self.u32_at(&mut iter)
            {
                return Incoming::Shown { serial, id };
            }
            Incoming::Failed { serial }
        }
    }

    /// `ActionInvoked(u, s)`, `ActivationToken(u, s)` or `NotificationClosed(u, u)`.
    ///
    /// # Safety
    /// `msg` must be a live signal.
    unsafe fn notification_signal(&self, msg: *mut Msg) -> Incoming {
        unsafe {
            let is = |member: &CStr| (self.dbus_message_is_signal)(msg, NOTIFY_IFACE.as_ptr(), member.as_ptr()) != 0;
            let mut iter = DBusMessageIter::new();
            if (self.dbus_message_iter_init)(msg, &mut iter) == 0 {
                return Incoming::Other;
            }
            let Some(id) = self.u32_at(&mut iter) else { return Incoming::Other };
            if is(c"NotificationClosed") {
                return Incoming::Closed { id };
            }
            if (self.dbus_message_iter_next)(&mut iter) == 0 {
                return Incoming::Other;
            }
            let Some(text) = self.string_at(&mut iter).and_then(|s| s.to_str().ok()) else {
                return Incoming::Other;
            };
            if is(c"ActionInvoked") {
                Incoming::Invoked { id, action: text.to_owned() }
            } else if is(c"ActivationToken") {
                Incoming::Token { id, token: text.to_owned() }
            } else {
                Incoming::Other
            }
        }
    }
}

struct Bus {
    lib: DBusLib,
    conn: *mut Conn,
    fd: RawFd,
    tracker: Tracker,
}

// The connection is only used under `BUS`'s lock.
unsafe impl Send for Bus {}

impl Bus {
    fn open() -> Option<Self> {
        let address = dbus::session_address()?;
        let lib = DBusLib::load()?;
        let (conn, fd) = dbus::open_session(&lib, &address)?;
        // No error out-parameter: the match is sent without waiting for the reply.
        unsafe { (lib.dbus_bus_add_match)(conn, SIGNAL_MATCH.as_ptr(), std::ptr::null_mut()) };
        Some(Self { lib, conn, fd, tracker: Tracker::default() })
    }

    /// Send `Notify` without waiting; the reply arrives through `dispatch`.
    fn send(&mut self, route: Route, title: &str, body: &str, c_title: &CStr, c_body: &CStr) -> io::Result<()> {
        let lib = &self.lib;
        let mut serial = 0u32;
        unsafe {
            let msg = lib.notify_call(c_title, c_body);
            if msg.is_null() {
                return Err(unavailable("notification message could not be built"));
            }
            let sent = (lib.dbus_connection_send)(self.conn, msg, &mut serial) != 0;
            (lib.dbus_message_unref)(msg);
            if !sent {
                return Err(unavailable("notification bus refused the message"));
            }
            // Flushing may also read incoming messages into libdbus's queue
            // without waking epoll; they are popped with the `Notify` reply,
            // which always follows on the socket.
            (lib.dbus_connection_flush)(self.conn);
        }
        self.tracker.sent(serial, route, title.to_owned(), body.to_owned());
        Ok(())
    }

    fn dispatch(&mut self, effects: &mut Vec<Effect>) -> Result<(), Disconnected> {
        unsafe { (self.lib.dbus_connection_read_write)(self.conn, 0) };
        loop {
            let msg = unsafe { (self.lib.dbus_connection_pop_message)(self.conn) };
            if msg.is_null() {
                break;
            }
            let incoming = unsafe { self.lib.notification_incoming(msg) };
            unsafe { (self.lib.dbus_message_unref)(msg) };
            effects.extend(self.tracker.on(incoming));
        }
        if unsafe { (self.lib.dbus_connection_get_is_connected)(self.conn) } == 0 {
            return Err(Disconnected);
        }
        Ok(())
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        unsafe {
            (self.lib.dbus_connection_close)(self.conn);
            (self.lib.dbus_connection_unref)(self.conn);
        }
    }
}

fn unavailable(why: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, why)
}

/// The notification bus connection closed.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Disconnected;

enum State {
    Unopened,
    Open(Box<Bus>),
    /// No bus, no watch, or the bus went away: `notify-send` from now on.
    Closed,
}

static BUS: Mutex<State> = Mutex::new(State::Unopened);

/// Show a notification whose click selects `pane` in the instance `origin`.
/// `Unsupported` when the session bus cannot carry it (the caller falls back
/// to `notify-send`); `InvalidInput` for text libdbus cannot take.
pub(in crate::platform::linux) fn show(origin: &str, pane: u64, title: &str, body: &str) -> io::Result<()> {
    let text = |s: &str| CString::new(s).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "notification contains NUL"));
    let (c_title, c_body) = (text(title)?, text(body)?);
    let mut state = BUS.lock();
    if matches!(*state, State::Unopened) {
        *state = match Bus::open() {
            Some(bus) if super::app::watch_notification_bus(bus.fd) => State::Open(Box::new(bus)),
            _ => State::Closed,
        };
    }
    let State::Open(bus) = &mut *state else {
        return Err(unavailable("no notification bus"));
    };
    bus.send(Route { origin: origin.to_owned(), pane }, title, body, &c_title, &c_body)
}

/// The bus socket the event loop watches, while the bus is open.
pub(super) fn watch_fd() -> Option<RawFd> {
    match &*BUS.lock() {
        State::Open(bus) => Some(bus.fd),
        _ => None,
    }
}

/// Read what the bus sent, adding what to do to `effects`. `Err` once the
/// bus is gone: the caller stops watching the socket, then calls `close`.
pub(super) fn dispatch(effects: &mut Vec<Effect>) -> Result<(), Disconnected> {
    match &mut *BUS.lock() {
        State::Open(bus) => bus.dispatch(effects),
        _ => Err(Disconnected),
    }
}

/// Drop the connection; later notifications use `notify-send`. Calls the
/// server never answered become `Fallback` effects, so they are still shown.
pub(super) fn close(effects: &mut Vec<Effect>) {
    let mut state = BUS.lock();
    if let State::Open(bus) = &mut *state {
        bus.tracker.abandon(effects);
    }
    *state = State::Closed;
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::c_char;

    const ORIGIN: &str = "0000000000000001-1-00000000000000000000000000000001";

    fn route(pane: u64) -> Route {
        Route { origin: ORIGIN.to_owned(), pane }
    }

    #[test]
    fn a_click_selects_the_pane_with_the_servers_token() {
        let mut tracker = Tracker::default();
        tracker.sent(5, route(9), "title".into(), "body".into());
        assert_eq!(tracker.on(Incoming::Shown { serial: 5, id: 77 }), None);
        assert_eq!(tracker.on(Incoming::Token { id: 77, token: "tok".into() }), None);
        assert_eq!(
            tracker.on(Incoming::Invoked { id: 77, action: "default".into() }),
            Some(Effect::Click { origin: ORIGIN.into(), pane: 9, token: Some("tok".into()) })
        );
        assert_eq!(tracker.on(Incoming::Invoked { id: 77, action: "default".into() }), None, "one click per notification");
    }

    #[test]
    fn a_click_without_a_token_still_selects_the_pane() {
        let mut tracker = Tracker::default();
        tracker.sent(5, route(9), "t".into(), "b".into());
        tracker.on(Incoming::Shown { serial: 5, id: 77 });
        assert_eq!(
            tracker.on(Incoming::Invoked { id: 77, action: "default".into() }),
            Some(Effect::Click { origin: ORIGIN.into(), pane: 9, token: None })
        );
    }

    #[test]
    fn a_refused_notify_falls_back_to_the_helper_once() {
        let mut tracker = Tracker::default();
        tracker.sent(5, route(9), "title".into(), "body".into());
        assert_eq!(tracker.on(Incoming::Failed { serial: 5 }), Some(Effect::Fallback { title: "title".into(), body: "body".into() }));
        assert_eq!(tracker.on(Incoming::Failed { serial: 5 }), None);
    }

    #[test]
    fn unrelated_messages_change_nothing() {
        let mut tracker = Tracker::default();
        // Replies to other calls (AddMatch) and signals for other clients.
        assert_eq!(tracker.on(Incoming::Failed { serial: 1 }), None);
        assert_eq!(tracker.on(Incoming::Shown { serial: 1, id: 3 }), None);
        assert_eq!(tracker.on(Incoming::Invoked { id: 3, action: "default".into() }), None);
        assert_eq!(tracker.on(Incoming::Token { id: 3, token: "t".into() }), None);
        assert_eq!(tracker.on(Incoming::Other), None);
        tracker.sent(5, route(9), "t".into(), "b".into());
        tracker.on(Incoming::Shown { serial: 5, id: 77 });
        assert_eq!(tracker.on(Incoming::Invoked { id: 77, action: "snooze".into() }), None, "only the default action clicks");
        assert_eq!(tracker.on(Incoming::Closed { id: 77 }), None);
        assert_eq!(tracker.on(Incoming::Invoked { id: 77, action: "default".into() }), None, "closed is forgotten");
    }

    #[test]
    fn a_lost_bus_falls_back_for_unanswered_calls_only() {
        let mut tracker = Tracker::default();
        tracker.sent(5, route(9), "shown".into(), "b".into());
        tracker.on(Incoming::Shown { serial: 5, id: 77 });
        tracker.sent(6, route(9), "waiting".into(), "b".into());
        let mut effects = Vec::new();
        tracker.abandon(&mut effects);
        assert_eq!(effects, [Effect::Fallback { title: "waiting".into(), body: "b".into() }]);
        assert_eq!(tracker.on(Incoming::Invoked { id: 77, action: "default".into() }), None, "nothing tracked after");
    }

    #[test]
    fn tracking_is_bounded() {
        let mut tracker = Tracker::default();
        for serial in 0..=MAX_TRACKED as u32 {
            tracker.sent(serial, route(1), String::new(), String::new());
        }
        assert_eq!(tracker.on(Incoming::Failed { serial: 0 }), None, "the oldest unanswered call was forgotten");
        for serial in 1..=MAX_TRACKED as u32 {
            tracker.on(Incoming::Shown { serial, id: serial + 100 });
        }
        tracker.sent(99, route(2), String::new(), String::new());
        tracker.on(Incoming::Shown { serial: 99, id: 999 });
        assert_eq!(tracker.on(Incoming::Invoked { id: 101, action: "default".into() }), None, "the oldest shown one was forgotten");
        assert!(tracker.on(Incoming::Invoked { id: 999, action: "default".into() }).is_some());
    }

    #[test]
    fn without_an_event_loop_notifications_are_unsupported() {
        // No unit test runs the event loop, so the bus socket cannot be
        // watched and nothing is left open.
        *BUS.lock() = State::Unopened;
        assert_eq!(show(ORIGIN, 1, "t", "b").unwrap_err().kind(), io::ErrorKind::Unsupported);
        assert!(watch_fd().is_none());
        assert_eq!(show(ORIGIN, 1, "t\0", "b").unwrap_err().kind(), io::ErrorKind::InvalidInput);
        *BUS.lock() = State::Unopened;
    }

    fn lib() -> Option<DBusLib> {
        let lib = DBusLib::load();
        if lib.is_none() {
            eprintln!("libdbus-1 not installed; skipping");
        }
        lib
    }

    unsafe fn append_text(lib: &DBusLib, iter: &mut DBusMessageIter, s: &CStr) {
        let ptr = s.as_ptr();
        unsafe { (lib.dbus_message_iter_append_basic)(iter, DBUS_TYPE_STRING, (&ptr as *const *const c_char).cast()) };
    }

    unsafe fn signal(lib: &DBusLib, member: &CStr, id: u32, text: Option<&CStr>) -> *mut Msg {
        unsafe {
            let msg = (lib.dbus_message_new_signal)(NOTIFY_PATH.as_ptr(), NOTIFY_IFACE.as_ptr(), member.as_ptr());
            let mut iter = DBusMessageIter::new();
            (lib.dbus_message_iter_init_append)(msg, &mut iter);
            (lib.dbus_message_iter_append_basic)(&mut iter, DBUS_TYPE_UINT32, (&id as *const u32).cast());
            match text {
                Some(text) => append_text(lib, &mut iter, text),
                None => {
                    let reason = 2u32;
                    (lib.dbus_message_iter_append_basic)(&mut iter, DBUS_TYPE_UINT32, (&reason as *const u32).cast());
                }
            }
            msg
        }
    }

    /// The call matches the specification's `Notify(susssasa{sv}i)` with
    /// one default action, so servers accept it and send `ActionInvoked`.
    #[test]
    fn notify_calls_have_the_spec_signature_and_a_default_action() {
        let Some(lib) = lib() else { return };
        unsafe {
            let msg = lib.notify_call(c"title", c"body");
            assert!(!msg.is_null());
            assert_eq!(CStr::from_ptr((lib.dbus_message_get_signature)(msg)), c"susssasa{sv}i");
            let mut iter = DBusMessageIter::new();
            (lib.dbus_message_iter_init)(msg, &mut iter);
            assert_eq!(lib.string_at(&mut iter), Some(APP_NAME));
            for _ in 0..5 {
                (lib.dbus_message_iter_next)(&mut iter);
            }
            let mut actions = DBusMessageIter::new();
            (lib.dbus_message_iter_recurse)(&mut iter, &mut actions);
            assert_eq!(lib.string_at(&mut actions), Some(DEFAULT_ACTION));
            (lib.dbus_message_iter_next)(&mut actions);
            assert_eq!(lib.string_at(&mut actions), Some(c"Open"));
            (lib.dbus_message_unref)(msg);
        }
    }

    #[test]
    fn replies_and_signals_are_classified() {
        let Some(lib) = lib() else { return };
        unsafe {
            let call = lib.notify_call(c"t", c"b");
            (lib.dbus_message_set_serial)(call, 7);
            let reply = (lib.dbus_message_new_method_return)(call);
            let mut iter = DBusMessageIter::new();
            (lib.dbus_message_iter_init_append)(reply, &mut iter);
            let id = 42u32;
            (lib.dbus_message_iter_append_basic)(&mut iter, DBUS_TYPE_UINT32, (&id as *const u32).cast());
            assert_eq!(lib.notification_incoming(reply), Incoming::Shown { serial: 7, id: 42 });
            (lib.dbus_message_unref)(reply);

            let empty = (lib.dbus_message_new_method_return)(call);
            assert_eq!(lib.notification_incoming(empty), Incoming::Failed { serial: 7 });
            (lib.dbus_message_unref)(empty);

            let error = (lib.dbus_message_new_error)(call, c"org.freedesktop.DBus.Error.ServiceUnknown".as_ptr(), c"no".as_ptr());
            assert_eq!(lib.notification_incoming(error), Incoming::Failed { serial: 7 });
            (lib.dbus_message_unref)(error);
            (lib.dbus_message_unref)(call);

            for (member, text, expected) in [
                (c"ActionInvoked", Some(c"default"), Incoming::Invoked { id: 42, action: "default".into() }),
                (c"ActivationToken", Some(c"tok"), Incoming::Token { id: 42, token: "tok".into() }),
                (c"NotificationClosed", None, Incoming::Closed { id: 42 }),
            ] {
                let msg = signal(&lib, member, 42, text);
                assert_eq!(lib.notification_incoming(msg), expected, "{member:?}");
                (lib.dbus_message_unref)(msg);
            }
            let other = (lib.dbus_message_new_signal)(NOTIFY_PATH.as_ptr(), c"org.example.Other".as_ptr(), c"ActionInvoked".as_ptr());
            let mut iter = DBusMessageIter::new();
            (lib.dbus_message_iter_init_append)(other, &mut iter);
            (lib.dbus_message_iter_append_basic)(&mut iter, DBUS_TYPE_UINT32, (&id as *const u32).cast());
            append_text(&lib, &mut iter, c"default");
            assert_eq!(lib.notification_incoming(other), Incoming::Other);
            (lib.dbus_message_unref)(other);
        }
    }
}
