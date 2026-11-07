//! Native macOS Application and Event Loop implementation.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Instant;

use parking_lot::Mutex;

use crate::platform::gui::{EventHandler, EventProxy, Flow, LoopCtx, WindowEvent, WindowId};
use crate::platform::macos::objc::*;
use super::events::{convert_key_event, process_ns_event, MODIFIER_COMMAND};
use super::view::{ensure_litty_view_class, set_event_callback};

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRunLoopGetMain() -> *mut c_void;
    fn CFRunLoopWakeUp(rl: *mut c_void);
}

pub fn wakeup_main_loop() {
    unsafe {
        if let Some(app) = shared_application()
            && let Some(event_cls) = class(c"NSEvent") {
            let dummy = msg_send!(
                event_cls,
                c"otherEventWithType:location:modifierFlags:timestamp:windowNumber:context:subtype:data1:data2:",
                15usize => usize, // NSEventTypeApplicationDefined
                NSPoint { x: 0.0, y: 0.0 } => NSPoint,
                0usize => usize,
                0.0f64 => f64,
                0isize => isize,
                std::ptr::null_mut::<c_void>() => Id,
                0isize => isize,
                0isize => isize,
                0isize => isize
                ; -> Id
            );
            if !dummy.is_null() {
                msg_send!(app, c"postEvent:atStart:", dummy => Id, false => bool);
            }
        }
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

/// NSEventType values used by the loop.
const NS_EVENT_LEFT_MOUSE_DOWN: usize = 1;
const NS_EVENT_RIGHT_MOUSE_DRAGGED: usize = 7;
const NS_EVENT_OTHER_MOUSE_DRAGGED: usize = 27;
const NS_EVENT_KEY_DOWN: usize = 10;
const NS_EVENT_KEY_UP: usize = 11;
const NS_EVENT_APPLICATION_DEFINED: usize = 15;

/// Hand `WindowEvent`s queued by view and window-delegate callbacks to `handler`,
/// keeping the queue's buffer so steady input does not allocate.
fn deliver_pending_events<T: 'static, H: EventHandler<T>>(handler: &mut H, loop_ctx: &LoopCtx) {
    let mut batch = std::mem::take(&mut *PENDING_EVENTS.lock());
    if batch.is_empty() {
        return;
    }
    for (id, ev) in batch.drain(..) {
        handler.window_event(loop_ctx, id, ev);
    }
    let mut queue = PENDING_EVENTS.lock();
    if queue.is_empty() {
        *queue = batch;
    }
}

/// The window an event belongs to: its own, or for a pointer event generated over
/// no window of ours (e.g. `mouseMoved` outside the key window), the window under it.
unsafe fn event_window(app: Id, event: Id, event_type: usize) -> Id {
    unsafe {
        let win = msg_send!(event, c"window"; -> Id);
        // LeftMouseDown (1) through RightMouseDragged (7) include MouseMoved (5).
        let is_pointer = (NS_EVENT_LEFT_MOUSE_DOWN..=NS_EVENT_RIGHT_MOUSE_DRAGGED).contains(&event_type)
            || event_type == NS_EVENT_OTHER_MOUSE_DRAGGED;
        if !win.is_null() || !is_pointer {
            return win;
        }
        let Some(win_cls) = class(c"NSWindow") else { return win };
        // Windowless events carry screen coordinates in `locationInWindow`.
        let loc = msg_send!(event, c"locationInWindow"; -> NSPoint);
        let number = msg_send!(win_cls, c"windowNumberAtPoint:belowWindowWithWindowNumber:", loc => NSPoint, 0isize => isize; -> isize);
        if number <= 0 {
            return win;
        }
        msg_send!(app, c"windowWithWindowNumber:", number => isize; -> Id)
    }
}

/// The id of `win` when it is one of ours (its content view is a tagged `LittyView`).
unsafe fn litty_window_id(win: Id) -> Option<WindowId> {
    unsafe {
        if win.is_null() {
            return None;
        }
        let view = msg_send!(win, c"contentView"; -> Id);
        if view.is_null() {
            return None;
        }
        let tag = msg_send!(view, c"tag"; -> usize);
        (tag != 0).then(|| WindowId::from_raw(tag as u64))
    }
}

/// Deliver one `NSEvent`: pointer and modifier events go straight to `handler`;
/// every event then goes through AppKit, which runs key events through the menu's
/// key equivalents and `LittyView`'s input method. Whatever the view and delegate
/// queued is delivered before the next event, so input keeps its order.
unsafe fn dispatch_event<T: 'static, H: EventHandler<T>>(app: Id, event: Id, handler: &mut H, loop_ctx: &LoopCtx) {
    unsafe {
        let event_type = msg_send!(event, c"type"; -> usize);
        if event_type == NS_EVENT_APPLICATION_DEFINED {
            // Wake-up only: the loop then delivers redraws and user events.
            return;
        }
        if event_type == NS_EVENT_KEY_UP && msg_send!(event, c"modifierFlags"; -> usize) & MODIFIER_COMMAND != 0 {
            // NSApplication never forwards a Cmd key-up to the key window.
            let win = msg_send!(event, c"window"; -> Id);
            if let Some(id) = litty_window_id(win) {
                handler.window_event(loop_ctx, id, WindowEvent::Key(convert_key_event(event, false)));
            }
            return;
        }
        if event_type != NS_EVENT_KEY_DOWN && event_type != NS_EVENT_KEY_UP {
            let win = event_window(app, event, event_type);
            if let Some(id) = litty_window_id(win) {
                let view = msg_send!(win, c"contentView"; -> Id);
                let scale = msg_send!(win, c"backingScaleFactor"; -> f64);
                process_ns_event(event, win, view, scale, |wev| {
                    handler.window_event(loop_ctx, id, wev);
                });
            }
        }
        msg_send!(app, c"sendEvent:", event => Id);
        msg_send!(app, c"updateWindows");
        deliver_pending_events(handler, loop_ctx);
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
        // Bring the first window to the front with keyboard focus, as winit did:
        // launched from a shell, the app is otherwise left behind it.
        crate::platform::macos::app::activate_app();

        let date_cls = class(c"NSDate").expect("NSDate");
        let default_run_loop_mode = ns_string("kCFRunLoopDefaultMode").expect("NSDefaultRunLoopMode");

        while !native_ctx.exit_flag.load(Ordering::SeqCst) {
            let pool = msg_send!(class(c"NSAutoreleasePool").unwrap(), c"new"; -> Id);

            // 1. Process user events from proxy
            while let Ok(user_ev) = rx.try_recv() {
                handler.user_event(&loop_ctx, user_ev);
            }

            // 2. Deliver pending view/window events
            deliver_pending_events(&mut handler, &loop_ctx);

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

            // Block (per the flow mode) for the first event, then drain the queue.
            let mut until = until_date;
            loop {
                let event = msg_send!(
                    app,
                    c"nextEventMatchingMask:untilDate:inMode:dequeue:",
                    usize::MAX => usize,
                    until => Id,
                    default_run_loop_mode => Id,
                    true => bool
                    ; -> Id
                );
                if event.is_null() {
                    break;
                }
                until = std::ptr::null_mut();
                dispatch_event(app, event, &mut handler, &loop_ctx);
            }

            msg_send!(pool, c"drain");
        }
    }
}
