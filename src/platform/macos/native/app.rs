//! Native macOS Application and Event Loop implementation.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Instant;

use parking_lot::Mutex;

use crate::platform::gui::{EventHandler, EventProxy, Flow, LoopCtx, WindowEvent, WindowId};
use crate::platform::macos::objc::*;
use super::events::convert_ns_event;
use super::view::{ensure_litty_view_class, set_event_callback};

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRunLoopGetMain() -> *mut c_void;
    fn CFRunLoopWakeUp(rl: *mut c_void);
}

pub fn wakeup_main_loop() {
    unsafe {
        let main_loop = CFRunLoopGetMain();
        if !main_loop.is_null() {
            CFRunLoopWakeUp(main_loop);
        }
    }
}

static PENDING_REDRAWS: Mutex<Vec<WindowId>> = Mutex::new(Vec::new());
static PENDING_EVENTS: Mutex<Vec<(WindowId, WindowEvent)>> = Mutex::new(Vec::new());

pub fn request_redraw(id: WindowId) {
    let mut guard = PENDING_REDRAWS.lock();
    if !guard.contains(&id) {
        guard.push(id);
    }
    drop(guard);
    wakeup_main_loop();
}

pub fn queue_event(id: WindowId, event: WindowEvent) {
    PENDING_EVENTS.lock().push((id, event));
    wakeup_main_loop();
}

pub struct NativeLoopCtx {
    exit_flag: AtomicBool,
    flow_mode: AtomicU8, // 0 = Wait, 1 = Poll, 2 = WaitUntil
    until_instant: Mutex<Option<Instant>>,
}

impl NativeLoopCtx {
    pub fn new() -> Self {
        Self {
            exit_flag: AtomicBool::new(false),
            flow_mode: AtomicU8::new(0),
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
            }
            Flow::Poll => {
                self.flow_mode.store(1, Ordering::SeqCst);
                *self.until_instant.lock() = None;
            }
            Flow::WaitUntil(t) => {
                self.flow_mode.store(2, Ordering::SeqCst);
                *self.until_instant.lock() = Some(t);
            }
        }
    }
}

pub fn run_native<T: 'static, H: EventHandler<T>>(make: impl FnOnce(EventProxy<T>) -> H) {
    ensure_litty_view_class();

    unsafe {
        let app = shared_application().expect("NSApplication");
        // Set activation policy to Regular (dock icon, window focus)
        msg_send!(app, c"setActivationPolicy:", 0isize => isize);
        msg_send!(app, c"finishLaunching");

        let (tx, rx): (Sender<T>, Receiver<T>) = channel();
        let proxy = EventProxy::from_native(tx);
        let mut handler = make(proxy);

        set_event_callback(|id, ev| {
            queue_event(id, ev);
        });

        let native_ctx = NativeLoopCtx::new();
        let loop_ctx = LoopCtx::from_native(&native_ctx);
        handler.resumed(&loop_ctx);

        let date_cls = class(c"NSDate").expect("NSDate");
        let default_run_loop_mode = ns_string("kCFRunLoopDefaultMode").expect("NSDefaultRunLoopMode");

        while !native_ctx.exit_flag.load(Ordering::SeqCst) {
            let pool = msg_send!(class(c"NSAutoreleasePool").unwrap(), c"new"; -> Id);

            // 1. Process user events from proxy
            while let Ok(user_ev) = rx.try_recv() {
                handler.user_event(&loop_ctx, user_ev);
            }

            // 2. Deliver pending view/window events
            let events: Vec<(WindowId, WindowEvent)> = {
                let mut guard = PENDING_EVENTS.lock();
                guard.drain(..).collect()
            };
            for (id, ev) in events {
                handler.window_event(&loop_ctx, id, ev);
            }

            handler.cycle_started(false);

            // 3. Deliver pending redraws
            let redraws: Vec<WindowId> = {
                let mut guard = PENDING_REDRAWS.lock();
                guard.drain(..).collect()
            };
            for id in redraws {
                handler.window_event(&loop_ctx, id, WindowEvent::RedrawRequested);
            }

            handler.about_to_wait(&loop_ctx);

            // 4. Wait / poll for Cocoa events
            let flow_mode = native_ctx.flow_mode.load(Ordering::SeqCst);
            let until_date = match flow_mode {
                0 => {
                    // Wait: block until event arrives
                    msg_send!(date_cls, c"distantFuture"; -> Id)
                }
                1 => {
                    // Poll: return immediately
                    msg_send!(date_cls, c"distantPast"; -> Id)
                }
                _ => {
                    // WaitUntil
                    let deadline = *native_ctx.until_instant.lock();
                    if let Some(target) = deadline {
                        let now = Instant::now();
                        if target <= now {
                            msg_send!(date_cls, c"distantPast"; -> Id)
                        } else {
                            let secs = (target - now).as_secs_f64();
                            msg_send!(date_cls, c"dateWithTimeIntervalSinceNow:", secs => f64 ; -> Id)
                        }
                    } else {
                        msg_send!(date_cls, c"distantFuture"; -> Id)
                    }
                }
            };

            // Fetch first event with until_date
            let event = msg_send!(
                app,
                c"nextEventMatchingMask:untilDate:inMode:dequeue:",
                usize::MAX => usize,
                until_date => Id,
                default_run_loop_mode => Id,
                true => bool
                ; -> Id
            );

            if !event.is_null() {
                // Dispatch event
                let win = msg_send!(event, c"window"; -> Id);
                if !win.is_null() {
                    let view = msg_send!(win, c"contentView"; -> Id);
                    if !view.is_null() {
                        let tag = msg_send!(view, c"tag"; -> usize);
                        if tag != 0 {
                            let id = WindowId::from_raw(tag as u64);
                            let scale = msg_send!(win, c"backingScaleFactor"; -> f64);
                            if let Some(wev) = convert_ns_event(event, id, scale) {
                                handler.window_event(&loop_ctx, id, wev);
                            }
                        }
                    }
                }

                msg_send!(app, c"sendEvent:", event => Id);
                msg_send!(app, c"updateWindows");

                // Drain remaining queued events without blocking
                loop {
                    let next = msg_send!(
                        app,
                        c"nextEventMatchingMask:untilDate:inMode:dequeue:",
                        usize::MAX => usize,
                        std::ptr::null_mut::<c_void>() => Id,
                        default_run_loop_mode => Id,
                        true => bool
                        ; -> Id
                    );
                    if next.is_null() {
                        break;
                    }

                    let win = msg_send!(next, c"window"; -> Id);
                    if !win.is_null() {
                        let view = msg_send!(win, c"contentView"; -> Id);
                        if !view.is_null() {
                            let tag = msg_send!(view, c"tag"; -> usize);
                            if tag != 0 {
                                let id = WindowId::from_raw(tag as u64);
                                let scale = msg_send!(win, c"backingScaleFactor"; -> f64);
                                if let Some(wev) = convert_ns_event(next, id, scale) {
                                    handler.window_event(&loop_ctx, id, wev);
                                }
                            }
                        }
                    }

                    msg_send!(app, c"sendEvent:", next => Id);
                    msg_send!(app, c"updateWindows");
                }
            }

            msg_send!(pool, c"drain");
        }
    }
}
