//! Modal alerts and the update progress window.

use std::ffi::c_void;
use std::sync::atomic::{AtomicPtr, Ordering};

use super::app::activate_app;
use super::objc::*;

/// Ask with a modal alert; `true` when `button` (the default) is chosen.
/// Without AppKit available, answers `true`.
pub fn confirm(title: &str, text: &str, button: &str) -> bool {
    // SAFETY: standard NSAlert calls on an object we allocated; `runModal`
    // spins a nested run loop, during which winit queues its events.
    unsafe {
        let Some(cls) = class(c"NSAlert") else { return true };
        let Some(alert) = send_id(cls, c"alloc").and_then(|a| send_id(a, c"init")) else { return true };
        for (selector, value) in [(c"setMessageText:", title), (c"setInformativeText:", text)] {
            if let Some(s) = ns_string(value) {
                msg_send!(alert, selector, s => Id);
            }
        }
        for label in [button, "Cancel"] {
            if let Some(s) = ns_string(label) {
                msg_send!(alert, c"addButtonWithTitle:", s => Id);
            }
        }
        activate_app();
        // NSAlertFirstButtonReturn
        let answer = msg_send!(alert, c"runModal"; -> isize);
        msg_send!(alert, c"release");
        answer == 1000
    }
}

/// Present a modal alert with arbitrary button labels. Returns the index of
/// the selected button (0 = first button, 1 = second, ...).
pub fn alert_choice(title: &str, text: &str, buttons: &[&str]) -> usize {
    unsafe {
        let Some(cls) = class(c"NSAlert") else { return 0 };
        let Some(alert) = send_id(cls, c"alloc").and_then(|a| send_id(a, c"init")) else { return 0 };
        for (selector, value) in [(c"setMessageText:", title), (c"setInformativeText:", text)] {
            if let Some(s) = ns_string(value) {
                msg_send!(alert, selector, s => Id);
            }
        }
        for &label in buttons {
            if let Some(s) = ns_string(label) {
                msg_send!(alert, c"addButtonWithTitle:", s => Id);
            }
        }
        activate_app();
        let answer = msg_send!(alert, c"runModal"; -> isize);
        msg_send!(alert, c"release");
        if answer >= 1000 {
            (answer - 1000) as usize
        } else {
            0
        }
    }
}

/// Present an informational modal alert with an "OK" button.
pub fn alert(title: &str, text: &str) {
    let _ = alert_choice(title, text, &["OK"]);
}

static ACTIVE_PROGRESS_WINDOW: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

static PROGRESS_DELEGATE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

extern "C" fn progress_window_will_close(_this: *mut c_void, _cmd: *mut c_void, _notification: *mut c_void) {
    let win = ACTIVE_PROGRESS_WINDOW.swap(std::ptr::null_mut(), Ordering::SeqCst);
    if !win.is_null() {
        unsafe {
            msg_send!(win, c"release");
        }
    }
}

fn progress_delegate() -> Option<Id> {
    let cur = PROGRESS_DELEGATE.load(Ordering::SeqCst);
    if !cur.is_null() {
        return Some(cur);
    }
    unsafe {
        let name = c"LittyProgressDelegate";
        let mut cls = objc_getClass(name.as_ptr());
        if cls.is_null() {
            let base = class(c"NSObject")?;
            cls = objc_allocateClassPair(base, name.as_ptr(), 0);
            if cls.is_null() {
                return None;
            }
            class_addMethod(cls, sel(c"windowWillClose:"), progress_window_will_close as *const (), c"v@:@".as_ptr());
            objc_registerClassPair(cls);
        }
        let obj = send_id(cls, c"alloc").and_then(|o| send_id(o, c"init"))?;
        PROGRESS_DELEGATE.store(obj, Ordering::SeqCst);
        Some(obj)
    }
}

/// Show a modeless progress dialog with an indeterminate progress bar.
/// Must be called on the main thread.
pub fn show_progress_dialog(title: &str, text: &str) {
    unsafe {
        close_progress_dialog();

        let Some(win_cls) = class(c"NSWindow") else { return };
        let Some(alloc) = send_id(win_cls, c"alloc") else { return };
        let frame = NSRect { x: 0.0, y: 0.0, w: 360.0, h: 90.0 };
        // NSWindowStyleMaskTitled (1), NSBackingStoreBuffered (2), defer: false (0)
        let win = msg_send!(
            alloc,
            c"initWithContentRect:styleMask:backing:defer:",
            frame => NSRect, 1u64 => u64, 2u64 => u64, false => bool;
            -> Id
        );
        if win.is_null() {
            return;
        }

        if let Some(s) = ns_string("Litty") {
            msg_send!(win, c"setTitle:", s => Id);
        }
        msg_send!(win, c"setReleasedWhenClosed:", false => bool);
        if let Some(del) = progress_delegate() {
            msg_send!(win, c"setDelegate:", del => Id);
        }

        let Some(content) = send_id(win, c"contentView") else {
            msg_send!(win, c"release");
            return;
        };

        // App icon on the left
        let app_icon = shared_application().and_then(|app| send_id(app, c"applicationIconImage"));
        if let Some(iv_cls) = class(c"NSImageView")
            && let Some(alloc_iv) = send_id(iv_cls, c"alloc") {
                let iv_frame = NSRect { x: 18.0, y: 20.0, w: 48.0, h: 48.0 };
                let iv = msg_send!(alloc_iv, c"initWithFrame:", iv_frame => NSRect; -> Id);
                if !iv.is_null() {
                    if let Some(img) = app_icon {
                        msg_send!(iv, c"setImage:", img => Id);
                    }
                    msg_send!(content, c"addSubview:", iv => Id);
                    msg_send!(iv, c"release");
                }
            }

        // Title label
        if let Some(tf_cls) = class(c"NSTextField")
            && let Some(alloc_tf) = send_id(tf_cls, c"alloc") {
                let tf_frame = NSRect { x: 78.0, y: 55.0, w: 265.0, h: 20.0 };
                let tf = msg_send!(alloc_tf, c"initWithFrame:", tf_frame => NSRect; -> Id);
                if !tf.is_null() {
                    if let Some(s) = ns_string(title) {
                        msg_send!(tf, c"setStringValue:", s => Id);
                    }
                    msg_send!(tf, c"setBezeled:", false => bool);
                    msg_send!(tf, c"setDrawsBackground:", false => bool);
                    msg_send!(tf, c"setEditable:", false => bool);
                    msg_send!(tf, c"setSelectable:", false => bool);
                    if let Some(font_cls) = class(c"NSFont") {
                        let bold_font: Id = msg_send!(font_cls, c"boldSystemFontOfSize:", 13.0f64 => f64; -> Id);
                        if !bold_font.is_null() {
                            msg_send!(tf, c"setFont:", bold_font => Id);
                        }
                    }
                    msg_send!(content, c"addSubview:", tf => Id);
                    msg_send!(tf, c"release");
                }
            }

        // Informational text label
        if let Some(tf_cls) = class(c"NSTextField")
            && let Some(alloc_tf) = send_id(tf_cls, c"alloc") {
                let tf_frame = NSRect { x: 78.0, y: 35.0, w: 265.0, h: 18.0 };
                let tf = msg_send!(alloc_tf, c"initWithFrame:", tf_frame => NSRect; -> Id);
                if !tf.is_null() {
                    if let Some(s) = ns_string(text) {
                        msg_send!(tf, c"setStringValue:", s => Id);
                    }
                    msg_send!(tf, c"setBezeled:", false => bool);
                    msg_send!(tf, c"setDrawsBackground:", false => bool);
                    msg_send!(tf, c"setEditable:", false => bool);
                    msg_send!(tf, c"setSelectable:", false => bool);
                    if let Some(font_cls) = class(c"NSFont") {
                        let sys_font: Id = msg_send!(font_cls, c"systemFontOfSize:", 11.0f64 => f64; -> Id);
                        if !sys_font.is_null() {
                            msg_send!(tf, c"setFont:", sys_font => Id);
                        }
                    }
                    msg_send!(content, c"addSubview:", tf => Id);
                    msg_send!(tf, c"release");
                }
            }

        // Indeterminate progress bar
        if let Some(ind_cls) = class(c"NSProgressIndicator")
            && let Some(alloc_ind) = send_id(ind_cls, c"alloc") {
                let frame = NSRect { x: 78.0, y: 15.0, w: 265.0, h: 14.0 };
                let ind = msg_send!(alloc_ind, c"initWithFrame:", frame => NSRect; -> Id);
                if !ind.is_null() {
                    msg_send!(ind, c"setStyle:", 1u64 => u64); // NSProgressIndicatorStyleBar = 1
                    msg_send!(ind, c"setIndeterminate:", true => bool);
                    msg_send!(ind, c"startAnimation:", std::ptr::null_mut() => Id);
                    msg_send!(content, c"addSubview:", ind => Id);
                    msg_send!(ind, c"release");
                }
            }

        msg_send!(win, c"center");
        msg_send!(win, c"makeKeyAndOrderFront:", std::ptr::null_mut() => Id);
        activate_app();

        ACTIVE_PROGRESS_WINDOW.store(win, Ordering::SeqCst);
    }
}

/// Close the active modeless progress dialog if one is shown.
/// Must be called on the main thread.
pub fn close_progress_dialog() {
    unsafe {
        let win = ACTIVE_PROGRESS_WINDOW.swap(std::ptr::null_mut(), Ordering::SeqCst);
        if !win.is_null() {
            msg_send!(win, c"setDelegate:", std::ptr::null_mut() => Id);
            msg_send!(win, c"orderOut:", std::ptr::null_mut() => Id);
            msg_send!(win, c"close");
            msg_send!(win, c"release");
        }
    }
}
