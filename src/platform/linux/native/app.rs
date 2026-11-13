//! Native Linux single-epoll event loop (Milestone 3 / Phase 4.2).

use std::os::fd::RawFd;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Instant;

use parking_lot::Mutex;

use crate::platform::gui::{EventHandler, EventProxy, Flow, LoopCtx, WindowEvent, WindowId};
use super::window::{NativeWindow, WeakNativeWindow};

static PENDING_REDRAWS: Mutex<Vec<WindowId>> = Mutex::new(Vec::new());
static PENDING_EVENTS: Mutex<Vec<(WindowId, WindowEvent)>> = Mutex::new(Vec::new());
/// Weak, so dropping a window's last `NativeWindow` destroys it (TD-1).
static WINDOWS: Mutex<Vec<WeakNativeWindow>> = Mutex::new(Vec::new());
static WAKE_EVENTFD: Mutex<Option<RawFd>> = Mutex::new(None);
static DISPLAY_FD: Mutex<Option<RawFd>> = Mutex::new(None);
/// The process-wide Wayland connection's socket, once one is open.
static WAYLAND_FD: Mutex<Option<RawFd>> = Mutex::new(None);
/// The Wayland key repeat timer, opened with the connection and never closed.
static KEY_REPEAT_FD: Mutex<Option<RawFd>> = Mutex::new(None);
static EPOLL_FD: Mutex<Option<RawFd>> = Mutex::new(None);

pub fn register_window(win: &NativeWindow) {
    WINDOWS.lock().push(win.downgrade());
}

/// Replace `out` with the windows the app still holds, pruning closed ones.
fn live_windows(out: &mut Vec<NativeWindow>) {
    out.clear();
    WINDOWS.lock().retain(|weak| weak.upgrade().map(|win| out.push(win)).is_some());
}

pub fn register_display_fd(fd: RawFd) {
    *DISPLAY_FD.lock() = Some(fd);
    if let Some(epoll) = *EPOLL_FD.lock() {
        let mut ev = libc::epoll_event {
            events: (libc::EPOLLIN | libc::EPOLLERR | libc::EPOLLHUP) as u32,
            u64: 3,
        };
        unsafe {
            libc::epoll_ctl(epoll, libc::EPOLL_CTL_ADD, fd, &mut ev);
        }
    }
}

/// Watch the Wayland socket (event 5). It is opened once and never closed.
pub fn register_wayland_fd(fd: RawFd) {
    *WAYLAND_FD.lock() = Some(fd);
    if let Some(epoll) = *EPOLL_FD.lock() {
        watch_wayland(epoll, fd);
    }
}

/// Watch the Wayland key repeat timer (event 6).
pub fn register_key_repeat_fd(fd: RawFd) {
    *KEY_REPEAT_FD.lock() = Some(fd);
    if let Some(epoll) = *EPOLL_FD.lock() {
        watch_key_repeat(epoll, fd);
    }
}

fn watch_key_repeat(epoll: RawFd, fd: RawFd) {
    let mut ev = libc::epoll_event { events: libc::EPOLLIN as u32, u64: 6 };
    if unsafe { libc::epoll_ctl(epoll, libc::EPOLL_CTL_ADD, fd, &mut ev) } != 0 {
        // Unwatched, a held key would repeat only when something else wakes the loop.
        let err = std::io::Error::last_os_error();
        eprintln!("litty: cannot watch the key repeat timer ({err}); keys do not repeat");
    }
}

/// `EPOLLOUT` is on the Wayland socket's watch.
static WAYLAND_WRITABLE: AtomicBool = AtomicBool::new(false);

fn wayland_events(writable: bool) -> u32 {
    let base = libc::EPOLLIN | libc::EPOLLERR | libc::EPOLLHUP;
    (if writable { base | libc::EPOLLOUT } else { base }) as u32
}

fn watch_wayland(epoll: RawFd, fd: RawFd) {
    let mut ev = libc::epoll_event { events: wayland_events(WAYLAND_WRITABLE.load(Ordering::Relaxed)), u64: 5 };
    if unsafe { libc::epoll_ctl(epoll, libc::EPOLL_CTL_ADD, fd, &mut ev) } != 0 {
        // Unwatched, its events would only be read while something else wakes the loop.
        let err = std::io::Error::last_os_error();
        eprintln!("litty: cannot watch the Wayland connection ({err})");
        std::process::exit(1);
    }
}

/// Also wake when the Wayland socket can take more requests (`writable`),
/// while a flush could not send them all. Only a change costs a syscall.
pub fn watch_wayland_writable(writable: bool) {
    if WAYLAND_WRITABLE.swap(writable, Ordering::Relaxed) == writable {
        return;
    }
    let (Some(epoll), Some(fd)) = (*EPOLL_FD.lock(), *WAYLAND_FD.lock()) else { return };
    let mut ev = libc::epoll_event { events: wayland_events(writable), u64: 5 };
    if unsafe { libc::epoll_ctl(epoll, libc::EPOLL_CTL_MOD, fd, &mut ev) } != 0 {
        let err = std::io::Error::last_os_error();
        eprintln!("litty: cannot watch the Wayland connection ({err})");
        std::process::exit(1);
    }
}

/// Stop watching a display connection that is about to close.
pub fn deregister_display_fd(fd: RawFd) {
    let mut display = DISPLAY_FD.lock();
    if *display == Some(fd) {
        *display = None;
    }
    if let Some(epoll) = *EPOLL_FD.lock() {
        unsafe {
            libc::epoll_ctl(epoll, libc::EPOLL_CTL_DEL, fd, std::ptr::null_mut());
        }
    }
}

pub fn wakeup_main_loop() {
    if let Some(fd) = *WAKE_EVENTFD.lock() {
        let val: u64 = 1;
        unsafe {
            libc::write(fd, &val as *const _ as *const std::ffi::c_void, 8);
        }
    }
}

pub fn request_redraw(id: WindowId) {
    let mut guard = PENDING_REDRAWS.lock();
    if !guard.contains(&id) {
        guard.push(id);
    }
    drop(guard);
    wakeup_main_loop();
}

#[allow(dead_code)]
pub fn queue_event(id: WindowId, event: WindowEvent) {
    PENDING_EVENTS.lock().push((id, event));
    wakeup_main_loop();
}

pub struct LinuxProxy<T: 'static> {
    sender: Sender<T>,
    event_fd: RawFd,
}

impl<T: 'static> Clone for LinuxProxy<T> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            event_fd: self.event_fd,
        }
    }
}

impl<T: 'static> LinuxProxy<T> {
    pub fn new(sender: Sender<T>, event_fd: RawFd) -> Self {
        Self { sender, event_fd }
    }

    pub fn send(&self, event: T) -> Result<(), T> {
        let res = self.sender.send(event).map_err(|e| e.0);
        if res.is_ok() {
            let val: u64 = 1;
            unsafe {
                libc::write(self.event_fd, &val as *const _ as *const std::ffi::c_void, 8);
            }
        }
        res
    }
}

pub struct NativeLoopCtx {
    pub(crate) exit_flag: AtomicBool,
    pub(crate) flow_mode: AtomicU8, // 0 = Wait, 1 = Poll, 2 = WaitUntil
    pub(crate) timerfd: RawFd,
    pub(crate) until_instant: Mutex<Option<Instant>>,
}

impl NativeLoopCtx {
    pub fn new(timerfd: RawFd) -> Self {
        Self {
            exit_flag: AtomicBool::new(false),
            flow_mode: AtomicU8::new(0),
            timerfd,
            until_instant: Mutex::new(None),
        }
    }

    pub fn exit(&self) {
        self.exit_flag.store(true, Ordering::SeqCst);
        wakeup_main_loop();
    }

    pub fn set_flow(&self, flow: Flow) {
        match flow {
            Flow::Wait => {
                self.flow_mode.store(0, Ordering::SeqCst);
                *self.until_instant.lock() = None;
                self.disarm_timer();
            }
            Flow::Poll => {
                self.flow_mode.store(1, Ordering::SeqCst);
                *self.until_instant.lock() = None;
                self.disarm_timer();
            }
            Flow::WaitUntil(t) => {
                self.flow_mode.store(2, Ordering::SeqCst);
                *self.until_instant.lock() = Some(t);
                self.arm_timer(t);
            }
        }
    }

    fn disarm_timer(&self) {
        unsafe {
            let zero = libc::itimerspec {
                it_interval: libc::timespec { tv_sec: 0, tv_nsec: 0 },
                it_value: libc::timespec { tv_sec: 0, tv_nsec: 0 },
            };
            libc::timerfd_settime(self.timerfd, 0, &zero, std::ptr::null_mut());
        }
    }

    fn arm_timer(&self, target: Instant) {
        let now = Instant::now();
        let dur = target.saturating_duration_since(now);
        let sec = dur.as_secs() as libc::time_t;
        let nsec = dur.subsec_nanos() as libc::c_long;
        // Avoid 0 sec and 0 nsec which disarms the timer
        let nsec = if sec == 0 && nsec == 0 { 1 } else { nsec };

        unsafe {
            let spec = libc::itimerspec {
                it_interval: libc::timespec { tv_sec: 0, tv_nsec: 0 },
                it_value: libc::timespec { tv_sec: sec, tv_nsec: nsec },
            };
            libc::timerfd_settime(self.timerfd, 0, &spec, std::ptr::null_mut());
        }
    }
}

pub fn run_native<T: 'static, H: EventHandler<T>>(make: impl FnOnce(EventProxy<T>) -> H) {
    unsafe {
        let epoll_fd = libc::epoll_create1(libc::EPOLL_CLOEXEC);
        if epoll_fd < 0 {
            panic!("failed to create epoll instance");
        }
        *EPOLL_FD.lock() = Some(epoll_fd);

        let event_fd = libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC);
        if event_fd < 0 {
            libc::close(epoll_fd);
            panic!("failed to create eventfd");
        }

        let timer_fd = libc::timerfd_create(libc::CLOCK_MONOTONIC, libc::TFD_NONBLOCK | libc::TFD_CLOEXEC);
        if timer_fd < 0 {
            libc::close(event_fd);
            libc::close(epoll_fd);
            panic!("failed to create timerfd");
        }

        // Register event_fd (user event 1)
        let mut ev_eventfd = libc::epoll_event {
            events: libc::EPOLLIN as u32,
            u64: 1,
        };
        libc::epoll_ctl(epoll_fd, libc::EPOLL_CTL_ADD, event_fd, &mut ev_eventfd);

        // Register timer_fd (timer event 2)
        let mut ev_timerfd = libc::epoll_event {
            events: libc::EPOLLIN as u32,
            u64: 2,
        };
        libc::epoll_ctl(epoll_fd, libc::EPOLL_CTL_ADD, timer_fd, &mut ev_timerfd);

        // If a display_fd was already registered, add it to epoll (display event 3)
        if let Some(dfd) = *DISPLAY_FD.lock() {
            let mut ev_display = libc::epoll_event {
                events: (libc::EPOLLIN | libc::EPOLLERR | libc::EPOLLHUP) as u32,
                u64: 3,
            };
            libc::epoll_ctl(epoll_fd, libc::EPOLL_CTL_ADD, dfd, &mut ev_display);
        }
        if let Some(wfd) = *WAYLAND_FD.lock() {
            watch_wayland(epoll_fd, wfd);
        }
        if let Some(rfd) = *KEY_REPEAT_FD.lock() {
            watch_key_repeat(epoll_fd, rfd);
        }

        *WAKE_EVENTFD.lock() = Some(event_fd);

        // Settings portal bus socket: opened before the first window, so
        // its theme answer arrives while the app starts up.
        let portal_fd = watch_portal(epoll_fd);

        let (tx, rx): (Sender<T>, Receiver<T>) = channel();
        let proxy = EventProxy::from_native_linux(tx, event_fd);
        let mut handler = make(proxy);

        let native_ctx = NativeLoopCtx::new(timer_fd);
        let loop_ctx = LoopCtx::from_native_linux(&native_ctx);
        handler.resumed(&loop_ctx);

        let mut events: [libc::epoll_event; 32] = std::mem::zeroed();
        let mut timer_fired = false;
        // Reused every cycle: the idle loop must not allocate (TD-10).
        let mut wins: Vec<NativeWindow> = Vec::new();
        let mut window_events: Vec<WindowEvent> = Vec::new();
        let mut pending: Vec<(WindowId, WindowEvent)> = Vec::new();
        let mut redraws: Vec<WindowId> = Vec::new();

        while !native_ctx.exit_flag.load(Ordering::SeqCst) {
            // 1. Process user events from proxy
            while let Ok(user_ev) = rx.try_recv() {
                handler.user_event(&loop_ctx, user_ev);
            }

            // 2. Poll each window's connection: its own for X11, the
            //    shared one (dispatched once here) for Wayland
            super::wayland::dispatch();
            live_windows(&mut wins);
            for win in &wins {
                let Some(backend) = win.backend() else { continue };
                backend.poll_events(&mut window_events);
                for ev in window_events.drain(..) {
                    win.observe(&ev);
                    handler.window_event(&loop_ctx, win.id(), ev);
                }
            }
            // Release our references so a window the app just closed drops now.
            wins.clear();

            // 3. Deliver pending window events
            std::mem::swap(&mut *PENDING_EVENTS.lock(), &mut pending);
            for (id, ev) in pending.drain(..) {
                handler.window_event(&loop_ctx, id, ev);
            }

            handler.cycle_started(timer_fired);
            timer_fired = false;

            // 4. Deliver pending redraws
            std::mem::swap(&mut *PENDING_REDRAWS.lock(), &mut redraws);
            for id in redraws.drain(..) {
                handler.window_event(&loop_ctx, id, WindowEvent::RedrawRequested);
            }

            handler.about_to_wait(&loop_ctx);

            // 5. Determine epoll_wait timeout
            let flow_mode = native_ctx.flow_mode.load(Ordering::SeqCst);
            live_windows(&mut wins);
            let has_x11_pending = wins.iter().any(|w| w.backend().is_some_and(|b| b.pending()));
            wins.clear();
            // Prepared last, right before blocking; dropped (cancelled) unless read.
            let mut wayland = super::wayland::prepare_wait();
            let wayland_busy = matches!(wayland, super::wayland::Wait::Busy);
            let timeout = if has_x11_pending || wayland_busy || flow_mode == 1 {
                0 // Non-blocking poll if events are pending or poll requested
            } else {
                -1 // Block on epoll (timerfd handles WaitUntil)
            };

            let n = libc::epoll_wait(epoll_fd, events.as_mut_ptr(), events.len() as i32, timeout);
            let mut key_repeat_due = false;
            if n > 0 {
                for ev in events.iter().take(n as usize) {
                    let tag = ev.u64;
                    if tag == 1 {
                        // eventfd readable: drain 8 bytes
                        let mut buf: u64 = 0;
                        libc::read(event_fd, &mut buf as *mut _ as *mut std::ffi::c_void, 8);
                    } else if tag == 2 {
                        // timerfd readable: drain 8 bytes
                        let mut buf: u64 = 0;
                        libc::read(timer_fd, &mut buf as *mut _ as *mut std::ffi::c_void, 8);
                        timer_fired = true;
                    } else if tag == 3 {
                        // X11 display socket readable: events will be drained on the next loop cycle
                    } else if tag == 4 {
                        portal_readable(epoll_fd, portal_fd);
                    } else if tag == 5 {
                        // Wayland socket readable (or writable again: the read then
                        // finds nothing, and the next cycle flushes): read now,
                        // dispatched on the next cycle.
                        if let super::wayland::Wait::Read(guard) =
                            std::mem::replace(&mut wayland, super::wayland::Wait::Disconnected)
                        {
                            super::wayland::read(guard);
                        }
                    } else if tag == 6 {
                        key_repeat_due = true;
                    }
                }
            }
            // After the socket read above, so a release read in the same
            // wakeup cancels the repeat instead of following it.
            if key_repeat_due {
                super::wayland::key_repeat_due();
            }
        }

        // Cleanup
        *WAKE_EVENTFD.lock() = None;
        *EPOLL_FD.lock() = None;
        libc::close(timer_fd);
        libc::close(event_fd);
        libc::close(epoll_fd);
    }
}

/// Watch the settings portal's bus socket (event 4). A socket epoll
/// refuses would never be read: close the portal instead, keeping the theme
/// it already read, so its signals do not pile up unread.
fn watch_portal(epoll_fd: RawFd) -> Option<RawFd> {
    let fd = super::portal::watch_fd()?;
    let mut ev = libc::epoll_event { events: libc::EPOLLIN as u32, u64: 4 };
    if unsafe { libc::epoll_ctl(epoll_fd, libc::EPOLL_CTL_ADD, fd, &mut ev) } == 0 {
        return Some(fd);
    }
    let err = std::io::Error::last_os_error();
    eprintln!("litty: cannot watch the settings portal ({err}); light/dark switches are not followed");
    super::portal::close();
    None
}

/// Read the settings portal's bus traffic and report a light/dark switch;
/// stop watching a bus that went away, or `EPOLLHUP` would spin the loop.
fn portal_readable(epoll_fd: RawFd, portal_fd: Option<RawFd>) {
    match super::portal::dispatch() {
        Ok(Some(theme)) => crate::platform::linux::EVENTS.emit(crate::platform::PlatformEvent::SystemAppearance {
            dark: theme == crate::platform::gui::Theme::Dark,
        }),
        Ok(None) => {}
        Err(super::portal::Disconnected) => {
            // Deregister before the socket closes, so a reused fd number is never removed.
            if let Some(fd) = portal_fd {
                unsafe { libc::epoll_ctl(epoll_fd, libc::EPOLL_CTL_DEL, fd, std::ptr::null_mut()) };
            }
            super::portal::close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::linux::native::window::detached;

    /// A bus socket epoll refuses is never read: the portal is closed, so
    /// its signals do not pile up unread and the failure is not silent.
    #[test]
    fn a_portal_socket_epoll_refuses_closes_the_portal() {
        let _serial = super::super::portal::tests::STATE_TESTS.lock();
        super::super::portal::reopen_for_test();
        if super::super::portal::watch_fd().is_none() {
            eprintln!("no settings portal bus; skipping");
            return;
        }
        assert_eq!(watch_portal(-1), None);
        assert_eq!(super::super::portal::dispatch(), Err(super::super::portal::Disconnected));
        super::super::portal::reopen_for_test();
    }

    /// The registry must not keep a window alive: its strong reference kept
    /// closed X11 windows mapped and their connections open (TD-1).
    #[test]
    fn dropped_windows_leave_the_registry() {
        let (kept, dropped) = (detached(10, 10), detached(10, 10));
        register_window(&kept);
        register_window(&dropped);
        let dropped_id = dropped.id();
        drop(dropped);
        let mut live = Vec::new();
        live_windows(&mut live);
        let ids: Vec<_> = live.iter().map(NativeWindow::id).collect();
        assert!(ids.contains(&kept.id()));
        assert!(!ids.contains(&dropped_id), "a window the app dropped is still polled");
        assert!(WINDOWS.lock().iter().all(|w| w.upgrade().is_some()), "dead entry not pruned");
    }
}
