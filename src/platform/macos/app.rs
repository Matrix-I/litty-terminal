//! App lifecycle: Dock reopen, activation, appearance, memory release.

use std::ffi::{c_void, CStr};
use std::sync::atomic::{AtomicBool, Ordering};

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

/// Reopen a window when the Dock icon is clicked or the app is activated.
pub fn register_reopen_handler(sink: crate::platform::EventSink) {
    EVENTS.set(sink);

    extern "C" fn on_reopen(_this: Id, _cmd: Id, _app: Id, _has_visible_windows: bool) -> bool {
        send_reopen();
        true
    }
    extern "C" fn on_activate(_this: Id, _cmd: Id, _notification: Id) {
        send_reopen();
    }

    // SAFETY: the method implementations match the type encodings given.
    unsafe {
        let Some(delegate) = shared_application().and_then(|app| app_delegate(app)) else { return };
        let Some(delegate_cls) = send_id(delegate, c"class") else { return };
        let methods: [(&CStr, *const (), &CStr); 2] = [
            (c"applicationShouldHandleReopen:hasVisibleWindows:", on_reopen as *const (), c"B@:@B"),
            (c"applicationDidBecomeActive:", on_activate as *const (), c"v@:@"),
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
            msg_send!(app, c"activateIgnoringOtherApps:", true => bool);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

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
    }
}
