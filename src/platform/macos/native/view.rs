//! Native NSView subclass (`LittyView`) implementing `NSTextInputClient`
//! for full Vietnamese Telex/VNI and CJK IME support.

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::platform::macos::objc::*;
use crate::platform::gui::{Ime, WindowEvent, WindowId};
use super::events::{convert_key_event, key_down_outcome, KeyDownOutcome, MODIFIER_COMMAND};

static CLASS_REGISTERED: AtomicBool = AtomicBool::new(false);
static TAG_KEY: u8 = 0;
/// Length (UTF-16 units) of the view's marked text; 0 when nothing is being composed.
static MARKED_LEN_KEY: u8 = 0;


/// What the input method did while `keyDown:` ran `interpretKeyEvents:`.
#[derive(Default)]
struct KeyDownState {
    /// `keyDown:` is running; `insertText:` outside it always comes from the input method.
    active: bool,
    /// There was marked text when the key arrived.
    composing_before: bool,
    /// Text inserted while nothing was being composed.
    inserted: Option<String>,
    /// `doCommandBySelector:` was called.
    command: bool,
    /// The input method committed a composition (started within this key or before it).
    committed: bool,
}

thread_local! {
    // AppKit sends every NSResponder / NSTextInputClient message on the main thread.
    static KEY_DOWN: RefCell<KeyDownState> = RefCell::new(KeyDownState::default());
}

unsafe fn view_window_id(view: Id) -> WindowId {
    unsafe { WindowId::from_raw(msg_send!(view, c"tag"; -> usize) as u64) }
}

unsafe fn marked_len(view: Id) -> usize {
    unsafe { objc_getAssociatedObject(view, &MARKED_LEN_KEY as *const u8 as *const c_void) as usize }
}

unsafe fn set_marked_len(view: Id, len: usize) {
    // OBJC_ASSOCIATION_ASSIGN (0): the value is a plain integer, not an object.
    unsafe { objc_setAssociatedObject(view, &MARKED_LEN_KEY as *const u8 as *const c_void, len as Id, 0) }
}

/// The text of an `NSString` or `NSAttributedString` passed by the input method.
unsafe fn input_text(string: Id) -> Option<String> {
    unsafe {
        if string.is_null() {
            return None;
        }
        if responds_to(string, c"string") {
            ns_string_to_string(msg_send!(string, c"string"; -> Id))
        } else {
            ns_string_to_string(string)
        }
    }
}

unsafe fn utf16_len(string: Id) -> usize {
    unsafe {
        if string.is_null() {
            return 0;
        }
        msg_send!(string, c"length"; -> usize)
    }
}

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

        // acceptsFirstMouse: -> YES (click-through on inactive window)
        extern "C" fn accepts_first_mouse(_this: Id, _cmd: Id, _event: Id) -> bool {
            true
        }
        class_addMethod(cls, sel(c"acceptsFirstMouse:"), accepts_first_mouse as *const (), c"B@:@".as_ptr() as *const _);

        // keyDown: runs the key through the input method (Telex, VNI, CJK, dead keys)
        // first; only a key it does not use reaches the terminal as `WindowEvent::Key`.
        extern "C" fn key_down(this: Id, _cmd: Id, event: Id) {
            unsafe {
                let id = view_window_id(this);
                let flags = msg_send!(event, c"modifierFlags"; -> usize);
                // Cmd shortcuts are never text: the menu has already had its chance.
                if flags & MODIFIER_COMMAND != 0 {
                    emit_event(id, WindowEvent::Key(convert_key_event(event, true)));
                    return;
                }
                let composing_before = marked_len(this) > 0;
                KEY_DOWN.with(|k| {
                    *k.borrow_mut() = KeyDownState { active: true, composing_before, ..KeyDownState::default() };
                });
                if let Some(array_cls) = class(c"NSArray") {
                    let events = msg_send!(array_cls, c"arrayWithObject:", event => Id; -> Id);
                    msg_send!(this, c"interpretKeyEvents:", events => Id);
                }
                let state = KEY_DOWN.with(|k| std::mem::take(&mut *k.borrow_mut()));
                let characters = ns_string_to_string(msg_send!(event, c"characters"; -> Id));
                match key_down_outcome(
                    // A composition committed by this very key counts as one going on.
                    state.composing_before || state.committed,
                    marked_len(this) > 0,
                    state.inserted,
                    characters.as_deref(),
                    state.command,
                ) {
                    KeyDownOutcome::Key => emit_event(id, WindowEvent::Key(convert_key_event(event, true))),
                    KeyDownOutcome::Commit(text) => emit_event(id, WindowEvent::Ime(Ime::Commit(text))),
                    KeyDownOutcome::Nothing => {}
                }
            }
        }
        class_addMethod(cls, sel(c"keyDown:"), key_down as *const (), c"v@:@".as_ptr() as *const _);

        extern "C" fn key_up(this: Id, _cmd: Id, event: Id) {
            unsafe {
                emit_event(view_window_id(this), WindowEvent::Key(convert_key_event(event, false)));
            }
        }
        class_addMethod(cls, sel(c"keyUp:"), key_up as *const (), c"v@:@".as_ptr() as *const _);

        // - (void)doCommandBySelector:(SEL)selector: the key is an editing command, not
        // text. Implemented so NSResponder does not NSBeep() for unhandled commands.
        extern "C" fn do_command_by_selector(_this: Id, _cmd: Id, _selector: Id) {
            KEY_DOWN.with(|k| k.borrow_mut().command = true);
        }
        class_addMethod(cls, sel(c"doCommandBySelector:"), do_command_by_selector as *const (), c"v@::".as_ptr() as *const _);

        // Mouse responder stubs so AppKit delivers drags and clicks without beeping
        extern "C" fn mouse_event_stub(_this: Id, _cmd: Id, _event: Id) {}
        for sel_name in &[
            c"mouseDown:",
            c"mouseUp:",
            c"mouseMoved:",
            c"mouseDragged:",
            c"rightMouseDown:",
            c"rightMouseUp:",
            c"rightMouseDragged:",
            c"otherMouseDown:",
            c"otherMouseUp:",
            c"otherMouseDragged:",
            c"scrollWheel:",
        ] {
            class_addMethod(cls, sel(sel_name), mouse_event_stub as *const (), c"v@:@".as_ptr() as *const _);
        }

        // tag and setTag:
        extern "C" fn view_tag(this: Id, _cmd: Id) -> usize {
            unsafe {
                objc_getAssociatedObject(this, &TAG_KEY as *const u8 as *const std::ffi::c_void) as usize
            }
        }
        class_addMethod(cls, sel(c"tag"), view_tag as *const (), c"q@:".as_ptr() as *const _);

        extern "C" fn view_set_tag(this: Id, _cmd: Id, tag: usize) {
            unsafe {
                objc_setAssociatedObject(this, &TAG_KEY as *const u8 as *const std::ffi::c_void, tag as Id, 0);
            }
        }
        class_addMethod(cls, sel(c"setTag:"), view_set_tag as *const (), c"v@:q".as_ptr() as *const _);

        // ---------------- NSTextInputClient protocol ----------------

        // - (BOOL)hasMarkedText
        extern "C" fn has_marked_text(this: Id, _cmd: Id) -> bool {
            unsafe { marked_len(this) > 0 }
        }
        class_addMethod(cls, sel(c"hasMarkedText"), has_marked_text as *const (), c"B@:".as_ptr() as *const _);

        // - (NSRange)markedRange
        extern "C" fn marked_range(this: Id, _cmd: Id) -> NSRange {
            match unsafe { marked_len(this) } {
                0 => NSRange { location: NS_NOT_FOUND, length: 0 },
                length => NSRange { location: 0, length },
            }
        }
        class_addMethod(cls, sel(c"markedRange"), marked_range as *const (), c"{_NSRange=QQ}@:".as_ptr() as *const _);

        // - (NSRange)selectedRange: the caret sits at the end of the marked text.
        extern "C" fn selected_range(this: Id, _cmd: Id) -> NSRange {
            match unsafe { marked_len(this) } {
                0 => NSRange { location: NS_NOT_FOUND, length: 0 },
                length => NSRange { location: length, length: 0 },
            }
        }
        class_addMethod(cls, sel(c"selectedRange"), selected_range as *const (), c"{_NSRange=QQ}@:".as_ptr() as *const _);

        // - (void)setMarkedText:(id)string selectedRange:(NSRange)selRange replacementRange:(NSRange)replacementRange
        extern "C" fn set_marked_text(this: Id, _cmd: Id, string: Id, _sel_range: NSRange, _replacement_range: NSRange) {
            unsafe {
                let text = input_text(string).unwrap_or_default();
                set_marked_len(this, if text.is_empty() { 0 } else { utf16_len(string) });
                emit_event(view_window_id(this), WindowEvent::Ime(Ime::Preedit(text)));
            }
        }
        class_addMethod(cls, sel(c"setMarkedText:selectedRange:replacementRange:"), set_marked_text as *const (), c"v@:@{_NSRange=QQ}{_NSRange=QQ}".as_ptr() as *const _);

        // - (void)unmarkText
        extern "C" fn unmark_text(this: Id, _cmd: Id) {
            unsafe {
                if marked_len(this) > 0 {
                    set_marked_len(this, 0);
                    emit_event(view_window_id(this), WindowEvent::Ime(Ime::Preedit(String::new())));
                }
            }
        }
        class_addMethod(cls, sel(c"unmarkText"), unmark_text as *const (), c"v@:".as_ptr() as *const _);

        // - (void)insertText:(id)string replacementRange:(NSRange)replacementRange
        extern "C" fn insert_text(this: Id, _cmd: Id, string: Id, _replacement_range: NSRange) {
            unsafe {
                let composing = marked_len(this) > 0;
                set_marked_len(this, 0);
                let Some(text) = input_text(string) else { return };
                // Typed text with no composition going on: `keyDown:` decides whether
                // it is the key's own text (a plain key) or a replacement.
                let plain = KEY_DOWN.with(|k| {
                    let mut k = k.borrow_mut();
                    let plain = k.active && !k.composing_before && !composing;
                    if plain {
                        k.inserted.get_or_insert_with(String::new).push_str(&text);
                    } else {
                        k.committed = true;
                    }
                    plain
                });
                if !plain {
                    emit_event(view_window_id(this), WindowEvent::Ime(Ime::Commit(text)));
                }
            }
        }
        class_addMethod(cls, sel(c"insertText:replacementRange:"), insert_text as *const (), c"v@:@{_NSRange=QQ}".as_ptr() as *const _);

        // - (NSArray *)validAttributesForMarkedText
        extern "C" fn valid_attributes(_this: Id, _cmd: Id) -> Id {
            unsafe {
                let arr_cls = class(c"NSArray").unwrap();
                msg_send!(arr_cls, c"array"; -> Id)
            }
        }
        class_addMethod(cls, sel(c"validAttributesForMarkedText"), valid_attributes as *const (), c"@@:".as_ptr() as *const _);

        // - (NSAttributedString *)attributedSubstringForProposedRange:(NSRange)range actualRange:(NSRangePointer)actualRange
        extern "C" fn attributed_substring(_this: Id, _cmd: Id, _range: NSRange, _actual: *mut c_void) -> Id {
            std::ptr::null_mut()
        }
        class_addMethod(cls, sel(c"attributedSubstringForProposedRange:actualRange:"), attributed_substring as *const (), c"@@:{_NSRange=QQ}^v".as_ptr() as *const _);

        // - (NSUInteger)characterIndexForPoint:(NSPoint)point
        extern "C" fn character_index_for_point(_this: Id, _cmd: Id, _point: NSPoint) -> usize {
            NS_NOT_FOUND
        }
        class_addMethod(cls, sel(c"characterIndexForPoint:"), character_index_for_point as *const (), c"Q@:{CGPoint=dd}".as_ptr() as *const _);

        // - (NSRect)firstRectForCharacterRange:(NSRange)range actualRange:(NSRangePointer)actualRange
        extern "C" fn first_rect_for_character_range(this: Id, _cmd: Id, _range: NSRange, _actual: *mut c_void) -> NSRect {
            unsafe {
                let win = msg_send!(this, c"window"; -> Id);
                if !win.is_null() {
                    msg_send!(win, c"frame"; -> NSRect)
                } else {
                    NSRect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }
                }
            }
        }
        class_addMethod(cls, sel(c"firstRectForCharacterRange:actualRange:"), first_rect_for_character_range as *const (), c"{CGRect={CGPoint=dd}{CGSize=dd}}@:{_NSRange=QQ}^v".as_ptr() as *const _);

        // Declaring the protocol is what makes `-[NSView inputContext]` create an
        // NSTextInputContext, so `interpretKeyEvents:` reaches the input method.
        let protocol = objc_getProtocol(c"NSTextInputClient".as_ptr());
        if !protocol.is_null() {
            class_addProtocol(cls, protocol);
        }

        objc_registerClassPair(cls);
    }
}
