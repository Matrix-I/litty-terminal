//! Native NSView subclass (`LittyView`) implementing `NSTextInputClient`
//! for full Vietnamese Telex/VNI and CJK IME support.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::platform::macos::objc::*;
use crate::platform::gui::{Ime, WindowEvent, WindowId};

static CLASS_REGISTERED: AtomicBool = AtomicBool::new(false);

#[repr(C)]
#[derive(Copy, Clone)]
pub struct NSRange {
    pub location: usize,
    pub length: usize,
}

pub const NS_NOT_FOUND: usize = usize::MAX;

static mut GLOBAL_EVENT_SINK: Option<Box<dyn Fn(WindowId, WindowEvent) + Send + Sync>> = None;

pub fn set_event_callback<F>(cb: F)
where
    F: Fn(WindowId, WindowEvent) + Send + Sync + 'static,
{
    unsafe {
        GLOBAL_EVENT_SINK = Some(Box::new(cb));
    }
}

pub(super) fn emit_event(id: WindowId, event: WindowEvent) {
    unsafe {
        if let Some(ref sink) = GLOBAL_EVENT_SINK {
            sink(id, event);
        }
    }
}

/// Register `LittyView` subclass of `NSView` conforming to `NSTextInputClient`.
pub fn ensure_litty_view_class() {
    if CLASS_REGISTERED.swap(true, Ordering::SeqCst) {
        return;
    }

    unsafe {
        let superclass = class(c"NSView").expect("NSView class exists");
        let cls = objc_allocateClassPair(superclass, c"LittyView".as_ptr(), 0);
        assert!(!cls.is_null(), "failed to allocate LittyView class");

        // acceptsFirstResponder -> YES
        extern "C" fn accepts_first_responder(_this: Id, _cmd: Id) -> bool {
            true
        }
        class_addMethod(cls, sel(c"acceptsFirstResponder"), accepts_first_responder as *const (), c"B@:".as_ptr() as *const _);

        // wantsUpdateLayer -> YES
        extern "C" fn wants_update_layer(_this: Id, _cmd: Id) -> bool {
            true
        }
        class_addMethod(cls, sel(c"wantsUpdateLayer"), wants_update_layer as *const (), c"B@:".as_ptr() as *const _);

        // isFlipped -> YES
        extern "C" fn is_flipped(_this: Id, _cmd: Id) -> bool {
            true
        }
        class_addMethod(cls, sel(c"isFlipped"), is_flipped as *const (), c"B@:".as_ptr() as *const _);

        // ---------------- NSTextInputClient protocol ----------------

        // - (BOOL)hasMarkedText
        extern "C" fn has_marked_text(_this: Id, _cmd: Id) -> bool {
            false
        }
        class_addMethod(cls, sel(c"hasMarkedText"), has_marked_text as *const (), c"B@:".as_ptr() as *const _);

        // - (NSRange)markedRange
        extern "C" fn marked_range(_this: Id, _cmd: Id) -> NSRange {
            NSRange { location: NS_NOT_FOUND, length: 0 }
        }
        class_addMethod(cls, sel(c"markedRange"), marked_range as *const (), c"{NSRange=QQ}@:".as_ptr() as *const _);

        // - (NSRange)selectedRange
        extern "C" fn selected_range(_this: Id, _cmd: Id) -> NSRange {
            NSRange { location: NS_NOT_FOUND, length: 0 }
        }
        class_addMethod(cls, sel(c"selectedRange"), selected_range as *const (), c"{NSRange=QQ}@:".as_ptr() as *const _);

        // - (void)setMarkedText:(id)string selectedRange:(NSRange)selRange replacementRange:(NSRange)replacementRange
        extern "C" fn set_marked_text(this: Id, _cmd: Id, string: Id, _sel_range: NSRange, _replacement_range: NSRange) {
            unsafe {
                let text = if responds_to(string, c"string") {
                    let str_obj = msg_send!(string, c"string"; -> Id);
                    ns_string_to_string(str_obj)
                } else {
                    ns_string_to_string(string)
                };
                if let Some(text) = text {
                    let tag = msg_send!(this, c"tag"; -> usize);
                    let id = WindowId::from_raw(tag as u64);
                    emit_event(id, WindowEvent::Ime(Ime::Preedit(text)));
                }
            }
        }
        class_addMethod(cls, sel(c"setMarkedText:selectedRange:replacementRange:"), set_marked_text as *const (), c"v@:@{NSRange=QQ}{NSRange=QQ}".as_ptr() as *const _);

        // - (void)unmarkText
        extern "C" fn unmark_text(this: Id, _cmd: Id) {
            unsafe {
                let tag = msg_send!(this, c"tag"; -> usize);
                let id = WindowId::from_raw(tag as u64);
                emit_event(id, WindowEvent::Ime(Ime::Preedit(String::new())));
            }
        }
        class_addMethod(cls, sel(c"unmarkText"), unmark_text as *const (), c"v@:".as_ptr() as *const _);

        // - (void)insertText:(id)string replacementRange:(NSRange)replacementRange
        extern "C" fn insert_text(this: Id, _cmd: Id, string: Id, _replacement_range: NSRange) {
            unsafe {
                let text = if responds_to(string, c"string") {
                    let str_obj = msg_send!(string, c"string"; -> Id);
                    ns_string_to_string(str_obj)
                } else {
                    ns_string_to_string(string)
                };
                if let Some(text) = text {
                    let tag = msg_send!(this, c"tag"; -> usize);
                    let id = WindowId::from_raw(tag as u64);
                    emit_event(id, WindowEvent::Ime(Ime::Commit(text)));
                }
            }
        }
        class_addMethod(cls, sel(c"insertText:replacementRange:"), insert_text as *const (), c"v@:@{NSRange=QQ}".as_ptr() as *const _);

        objc_registerClassPair(cls);
    }
}
