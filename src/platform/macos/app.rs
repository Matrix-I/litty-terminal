//! App lifecycle: Dock reopen, activation, appearance, memory release.

use std::ffi::{c_void, CStr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::objc::*;

unsafe extern "C" {
    fn malloc_zone_pressure_relief(zone: *mut c_void, goal: usize) -> usize;
}

/// Return freed heap pages to the OS (keeps idle RSS low after closing tabs).
pub fn release_free_memory() {
    unsafe extern "C" {
        fn mi_collect(force: bool);
    }
    unsafe {
        mi_collect(true);
        malloc_zone_pressure_relief(std::ptr::null_mut(), 0);
    }
}

/// Where Dock, menu and appearance callbacks deliver their events.
pub(super) static EVENTS: crate::platform::SinkSlot = crate::platform::SinkSlot::new();

fn send_reopen() {
    EVENTS.emit(crate::platform::PlatformEvent::Reopen);
}

/// Reopen a window for an explicit Dock click, preserving passive activation.
pub fn register_reopen_handler(sink: crate::platform::EventSink) {
    EVENTS.set(sink);

    extern "C" fn on_reopen(_this: Id, _cmd: Id, _app: Id, _has_visible_windows: bool) -> bool {
        if !crate::platform::consume_notification_reopen() { send_reopen(); }
        true
    }
    extern "C" fn on_activate(_this: Id, _cmd: Id, _notification: Id) {
        // Activation can follow an exact-pane/new-window request. A generic
        // reopen here would immediately select a different window, so only
        // re-check a Light/Dark switch made while another app was in front.
        EVENTS.emit(crate::platform::PlatformEvent::SystemAppearance { dark: system_dark() });
        // ...but finish restoring the exact window a click asked for.
        finish_deferred_restore();
    }
    extern "C" fn on_will_terminate(_this: Id, _cmd: Id, _notification: Id) {
        // A quit Apple event (Dock Quit, logout, `terminate`) exits inside
        // `terminate:` and never returns to the event loop's own shutdown.
        // Withdrawal is idempotent, so the Cmd+Q path may run it again.
        crate::platform::shutdown_notifications();
    }

    // SAFETY: the method implementations match the type encodings given.
    unsafe {
        let Some(app) = shared_application() else { return };
        let Some(delegate) = app_delegate(app) else { return };
        let Some(delegate_cls) = send_id(delegate, c"class") else { return };
        let methods: [(&CStr, *const (), &CStr); 3] = [
            (c"applicationShouldHandleReopen:hasVisibleWindows:", on_reopen as *const (), c"B@:@B"),
            (c"applicationDidBecomeActive:", on_activate as *const (), c"v@:@"),
            (c"applicationWillTerminate:", on_will_terminate as *const (), c"v@:@"),
        ];
        for (name, imp, types) in methods {
            let selector = sel(name);
            if !class_addMethod(delegate_cls, selector, imp, types.as_ptr()) {
                let m = class_getInstanceMethod(delegate_cls, selector);
                if !m.is_null() {
                    method_setImplementation(m, imp);
                }
            }
        }
        // AppKit subscribes the delegate to the notifications behind
        // `applicationDidBecomeActive:` / `applicationWillTerminate:` only
        // inside `setDelegate:`. The methods were added after that call, so
        // set the delegate again (via nil, in case the same object is ignored).
        // NSApp holds it weakly; its owner keeps it alive in between.
        msg_send!(app, c"setDelegate:", std::ptr::null_mut::<c_void>() => Id);
        msg_send!(app, c"setDelegate:", delegate => Id);
    }
    observe_system_appearance();
}

/// NSApp's delegate, installing an empty `LittyAppDelegate` when there is none:
/// the native event loop never sets one, and without it AppKit has nowhere to
/// deliver Dock clicks (`applicationShouldHandleReopen:`) or activation.
unsafe fn app_delegate(app: Id) -> Option<Id> {
    unsafe {
        if let Some(delegate) = send_id(app, c"delegate") {
            return Some(delegate);
        }
        let name = c"LittyAppDelegate";
        let mut cls = objc_getClass(name.as_ptr());
        if cls.is_null() {
            cls = objc_allocateClassPair(class(c"NSObject")?, name.as_ptr(), 0);
            if cls.is_null() {
                return None;
            }
            objc_registerClassPair(cls);
        }
        // NSApp holds its delegate weakly: this +1 instance lives for the process.
        let delegate = send_id(cls, c"alloc").and_then(|d| send_id(d, c"init"))?;
        msg_send!(app, c"setDelegate:", delegate => Id);
        Some(delegate)
    }
}

/// Windows carry their own NSAppearance (matching the terminal background),
/// so winit's ThemeChanged never fires. The application's appearance still
/// follows the system: watch it with KVO (the change is reported after it
/// took effect, so `system_dark` already sees the new value).
fn observe_system_appearance() {
    static OBSERVER_SETUP: AtomicBool = AtomicBool::new(false);
    if OBSERVER_SETUP.swap(true, Ordering::SeqCst) {
        return;
    }

    extern "C" fn on_change(_this: Id, _cmd: Id, _path: Id, _object: Id, _change: Id, _context: *mut c_void) {
        EVENTS.emit(crate::platform::PlatformEvent::SystemAppearance { dark: system_dark() });
    }
    // SAFETY: the observer class is created once with a method matching its
    // type encoding; the observer instance lives for the whole process.
    unsafe {
        let Some(app) = shared_application() else { return };
        let name = c"LittyAppearanceObserver";
        let mut cls = objc_getClass(name.as_ptr());
        if cls.is_null() {
            let Some(base) = class(c"NSObject") else { return };
            cls = objc_allocateClassPair(base, name.as_ptr(), 0);
            if cls.is_null() {
                return;
            }
            let selector = sel(c"observeValueForKeyPath:ofObject:change:context:");
            class_addMethod(cls, selector, on_change as *const (), c"v@:@@@^v".as_ptr());
            objc_registerClassPair(cls);
        }
        let Some(observer) = send_id(cls, c"alloc").and_then(|o| send_id(o, c"init")) else { return };
        let Some(key) = ns_string("effectiveAppearance") else { return };
        // NSKeyValueObservingOptionNew = 1
        msg_send!(
            app,
            c"addObserver:forKeyPath:options:context:",
            observer => Id,
            key => Id,
            1 => usize,
            std::ptr::null_mut::<c_void>() => *mut c_void
        );
    }
}

/// The system appearance is Dark: the application's effective appearance
/// (Litty never sets one on NSApp, only on its windows).
pub fn system_dark() -> bool {
    // SAFETY: NSApp and its appearance are live; the name is read while
    // the autoreleased objects are still alive.
    unsafe {
        let name = shared_application()
            .and_then(|app| send_id(app, c"effectiveAppearance"))
            .and_then(|appearance| send_id(appearance, c"name"))
            .map(|name| msg_send!(name, c"UTF8String"; -> *const std::ffi::c_char))
            .filter(|p| !p.is_null());
        name.is_none_or(|p| CStr::from_ptr(p).to_bytes().windows(4).any(|w| w == b"Dark"))
    }
}

pub fn set_dock_icon() {
    // SAFETY: standard NSApplication / NSImage calls on live objects.
    unsafe {
        let Some(app) = shared_application() else { return };

        // Inside a .app bundle the Dock already reads Litty.icns from the bundle;
        // skipping setApplicationIconImage saves ~8 MB of resident bitmap memory.
        let bundle_path = class(c"NSBundle")
            .and_then(|cls| send_id(cls, c"mainBundle"))
            .and_then(|bundle| send_id(bundle, c"bundlePath"))
            .map(|path| msg_send!(path, c"UTF8String"; -> *const std::ffi::c_char))
            .filter(|p| !p.is_null());
        if let Some(path) = bundle_path
            && CStr::from_ptr(path).to_string_lossy().ends_with(".app") {
                return;
            }

        const ICON_BYTES: &[u8] = include_bytes!("../../../resources/icon.png");
        let Some(data_cls) = class(c"NSData") else { return };
        let ns_data = msg_send!(
            data_cls, c"dataWithBytes:length:", ICON_BYTES.as_ptr() => *const u8, ICON_BYTES.len() => usize; -> Id
        );
        if ns_data.is_null() {
            return;
        }
        let Some(alloc) = class(c"NSImage").and_then(|cls| send_id(cls, c"alloc")) else { return };
        let image = msg_send!(alloc, c"initWithData:", ns_data => Id; -> Id);
        if image.is_null() {
            return;
        }
        msg_send!(app, c"setApplicationIconImage:", image => Id);
        msg_send!(image, c"release");
    }
}

pub fn activate_app() {
    // SAFETY: standard NSApplication call.
    unsafe {
        if let Some(app) = shared_application() {
            if responds_to(app, c"activate") {
                msg_send!(app, c"activate");
            } else {
                // Compatibility for macOS 11–13; the selector was replaced in 14.
                msg_send!(app, c"activateIgnoringOtherApps:", true => bool);
            }
        }
    }
}

/// How long a minimized window waits for this app to become active before it
/// is restored anyway. A click forwarded from another Litty process may never
/// activate this one (macOS 14+ cooperative activation); the observed gap
/// between a notification click and activation is about 0.55 s.
const RESTORE_WAIT: Duration = Duration::from_millis(1500);

/// The one minimized window waiting for activation to be restored.
struct DeferredRestore<W> {
    pending: Option<(W, Instant)>,
}

impl<W> DeferredRestore<W> {
    const fn new() -> Self { Self { pending: None } }

    /// Hold `window` until activation or `deadline`; returns the window it replaces.
    fn hold(&mut self, window: W, deadline: Instant) -> Option<W> {
        self.pending.replace((window, deadline)).map(|(old, _)| old)
    }

    /// Take the held window on activation, with whether it is still due at
    /// `now` (past its deadline the scheduled fallback restores it instead).
    fn take(&mut self, now: Instant) -> Option<(W, bool)> {
        self.pending.take().map(|(window, deadline)| (window, now < deadline))
    }

    /// Drop the held window if `is_it` matches it (the window is going away).
    fn forget(&mut self, is_it: impl FnOnce(&W) -> bool) -> Option<W> {
        if self.pending.as_ref().is_some_and(|(window, _)| is_it(window)) {
            self.pending.take().map(|(window, _)| window)
        } else {
            None
        }
    }
}

/// Whether restoring must wait for activation: only a minimized window while
/// another app is active returns to a Space macOS would not switch to.
fn defers_restore(app_active: bool, minimized: bool) -> bool {
    !app_active && minimized
}

/// A window retained while it waits; released when dropped.
struct Retained(Id);

// SAFETY: created, used and dropped only on the main thread (AppKit callbacks
// and the event loop); the Mutex below only satisfies `static`.
unsafe impl Send for Retained {}

impl Drop for Retained {
    fn drop(&mut self) {
        // SAFETY: balances the `retain` taken in `restore_and_focus`.
        unsafe { msg_send!(self.0, c"release") }
    }
}

static DEFERRED_RESTORE: Mutex<DeferredRestore<Retained>> = Mutex::new(DeferredRestore::new());

fn deferred_restore() -> std::sync::MutexGuard<'static, DeferredRestore<Retained>> {
    DEFERRED_RESTORE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Restore `window` from the Dock and bring it forward.
///
/// While another app is active, a restored window returns to its own Space,
/// but macOS switches to that Space only when the window is already there as
/// activation completes. Restoring first plays the animation on the current
/// Space and leaves the window behind on the other one. Like the Dock, wait
/// for activation, with a timed fallback in case activation never comes.
pub(super) fn restore_and_focus(window: Id) {
    // SAFETY: `window` is a live NSWindow; standard NSWindow/NSApplication calls.
    unsafe {
        let active = shared_application().is_some_and(|app| msg_send!(app, c"isActive"; -> bool));
        if defers_restore(active, msg_send!(window, c"isMiniaturized"; -> bool)) {
            let held = Retained(msg_send!(window, c"retain"; -> Id));
            let replaced = deferred_restore().hold(held, Instant::now() + RESTORE_WAIT);
            // Superseded by this newer click: do not restore it later on its
            // own. Cancel before scheduling: the cancel matches every request
            // for that window, so for the same window it would drop the new one.
            if let Some(old) = replaced {
                cancel_restore_fallback(old.0);
            }
            msg_send!(
                window,
                c"performSelector:withObject:afterDelay:",
                sel(c"deminiaturize:") => Id,
                std::ptr::null_mut::<c_void>() => Id,
                RESTORE_WAIT.as_secs_f64() => f64
            );
            return;
        }
        msg_send!(window, c"deminiaturize:", std::ptr::null_mut::<c_void>() => Id);
        msg_send!(window, c"makeKeyAndOrderFront:", std::ptr::null_mut::<c_void>() => Id);
    }
}

/// On activation, restore the window `restore_and_focus` held back, if due.
fn finish_deferred_restore() {
    let Some((window, due)) = deferred_restore().take(Instant::now()) else { return };
    if !due {
        return;
    }
    // SAFETY: `window` is retained by `Retained`; standard NSObject/NSWindow calls.
    unsafe {
        // Restored meanwhile (a closing window was already forgotten).
        if !msg_send!(window.0, c"isMiniaturized"; -> bool) {
            return;
        }
        cancel_restore_fallback(window.0);
        msg_send!(window.0, c"deminiaturize:", std::ptr::null_mut::<c_void>() => Id);
        msg_send!(window.0, c"makeKeyAndOrderFront:", std::ptr::null_mut::<c_void>() => Id);
    }
}

/// Cancel the timed `deminiaturize:` that `restore_and_focus` scheduled.
fn cancel_restore_fallback(window: Id) {
    // SAFETY: `window` is a live NSWindow; standard NSObject class method.
    unsafe {
        if let Some(object) = class(c"NSObject") {
            msg_send!(
                object,
                c"cancelPreviousPerformRequestsWithTarget:selector:object:",
                window => Id,
                sel(c"deminiaturize:") => Id,
                std::ptr::null_mut::<c_void>() => Id
            );
        }
    }
}

/// `window` is closing: neither the fallback nor activation may restore it.
pub(super) fn forget_deferred_restore(window: Id) {
    cancel_restore_fallback(window);
    let forgotten = deferred_restore().forget(|held| held.0 == window);
    drop(forgotten);
}

/// Wayland launcher tokens do not apply to AppKit.
pub fn capture_startup_activation() { super::activation::capture_start_time(); }

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {
        static NSApplicationDidBecomeActiveNotification: Id;
    }

    #[test]
    fn dock_reopen_reaches_the_sink_without_a_preexisting_app_delegate() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        register_reopen_handler(Arc::new(move |e| log.lock().unwrap().push(e)));

        // SAFETY: NSApp is live; the delegate method has the signature called here.
        unsafe {
            let app = shared_application().expect("NSApplication");
            let delegate = send_id(app, c"delegate").expect("NSApp has a delegate after registration");
            assert!(responds_to(delegate, c"applicationShouldHandleReopen:hasVisibleWindows:"));
            assert!(responds_to(delegate, c"applicationDidBecomeActive:"));
            // A quit Apple event (Dock Quit, logout, `terminate`) runs
            // `terminate:`, which exits before the event loop returns, so
            // the delegate is the only place to withdraw notifications.
            assert!(responds_to(delegate, c"applicationWillTerminate:"));
            let handled = msg_send!(
                delegate,
                c"applicationShouldHandleReopen:hasVisibleWindows:",
                app => Id,
                false => bool
                ; -> bool
            );
            assert!(handled);
        }
        assert!(seen.lock().unwrap().contains(&crate::platform::PlatformEvent::Reopen));
        seen.lock().unwrap().clear();
        // Becoming active is not a Dock reopen: it must not redirect the exact
        // pane/window selected by a notification or an explicit reuse request.
        // AppKit wires `application*:` notification methods to the delegate
        // only in `setDelegate:`, so post the real notification instead of
        // calling the method: a method added after `setDelegate:` never runs.
        // Activation takes a window held back by `restore_and_focus`, so a
        // later activation cannot restore it again. A past deadline keeps the
        // stand-in object from receiving window messages.
        // SAFETY: a plain NSObject balances the retain `Retained` releases.
        let stand_in = unsafe { send_id(class(c"NSObject").expect("NSObject"), c"new").expect("NSObject") };
        let expired = Instant::now().checked_sub(RESTORE_WAIT).expect("monotonic clock past RESTORE_WAIT");
        drop(deferred_restore().hold(Retained(stand_in), expired));
        // SAFETY: AppKit's notification name is a live NSString constant.
        unsafe {
            let app = shared_application().expect("NSApplication");
            let center = send_id(class(c"NSNotificationCenter").expect("NSNotificationCenter"), c"defaultCenter")
                .expect("default center");
            msg_send!(center, c"postNotificationName:object:", NSApplicationDidBecomeActiveNotification => Id, app => Id);
        }
        // It must still re-check a Light/Dark switch made while another app
        // was in front, without touching any window.
        assert_eq!(
            *seen.lock().unwrap(),
            [crate::platform::PlatformEvent::SystemAppearance { dark: system_dark() }],
            "passive activation re-checks the appearance and must not redirect focus"
        );
        assert!(deferred_restore().pending.is_none(), "activation must take the held restore");

    }

    #[test]
    fn a_deferred_restore_runs_once_when_activation_arrives_in_time() {
        let start = Instant::now();
        let mut deferred = DeferredRestore::new();
        assert!(deferred.hold("window", start + RESTORE_WAIT).is_none());
        assert_eq!(deferred.take(start + RESTORE_WAIT / 2), Some(("window", true)));
        // A later activation (Cmd+Tab, Dock) must not pull the window forward again.
        assert_eq!(deferred.take(start + RESTORE_WAIT / 2), None);
    }

    #[test]
    fn a_deferred_restore_is_left_to_the_fallback_after_its_deadline() {
        let start = Instant::now();
        let mut deferred = DeferredRestore::new();
        deferred.hold("window", start + RESTORE_WAIT);
        assert_eq!(deferred.take(start + RESTORE_WAIT), Some(("window", false)));
        assert_eq!(deferred.take(start + RESTORE_WAIT), None);
    }

    #[test]
    fn only_a_minimized_window_of_an_inactive_app_waits_for_activation() {
        assert!(defers_restore(false, true));
        assert!(!defers_restore(true, true), "an active app restores at once");
        assert!(!defers_restore(false, false), "an open window needs no restore");
        assert!(!defers_restore(true, false));
    }

    #[test]
    fn a_closing_window_is_forgotten_but_another_one_is_kept() {
        let start = Instant::now();
        let mut deferred = DeferredRestore::new();
        deferred.hold("window", start + RESTORE_WAIT);
        assert_eq!(deferred.forget(|held| *held == "other"), None);
        assert_eq!(deferred.forget(|held| *held == "window"), Some("window"));
        assert_eq!(deferred.take(start), None);
    }

    #[test]
    fn a_newer_deferred_restore_replaces_the_pending_one() {
        let start = Instant::now();
        let mut deferred = DeferredRestore::new();
        deferred.hold("first", start + RESTORE_WAIT);
        assert_eq!(deferred.hold("second", start + RESTORE_WAIT), Some("first"));
        assert_eq!(deferred.take(start), Some(("second", true)));
    }
}
