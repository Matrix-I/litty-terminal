//! Native AppKit window & view types for macOS (replaces winit on macOS).

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use crate::platform::macos::objc::*;
use crate::platform::gui::{
    CursorIcon, LogicalPosition, Monitor, PhysicalPosition, PhysicalSize,
    RawHandle, ResizeDirection, Theme, Unsupported, WindowEvent, WindowId, WindowSpec,
};
use super::view::ensure_litty_view_class;

static DELEGATE_CLASS_REGISTERED: AtomicBool = AtomicBool::new(false);
static mut SHARED_DELEGATE: Id = std::ptr::null_mut();

/// What a window reports when it becomes or stops being the key window. Losing
/// key first releases every modifier: a modifier let go while no window of ours
/// is key (e.g. Cmd after Cmd+W hid the window) never reaches us, and would
/// otherwise stay held when the window comes back (`d` acting as Cmd+D).
/// Gaining key reports the modifiers `held` right now: one kept down across
/// Cmd+N / Cmd+W sends no flagsChanged to the window that becomes key, which
/// would otherwise type the next `n` / `w` as text.
fn focus_events(focused: bool, held: crate::platform::input::Mods) -> impl Iterator<Item = WindowEvent> {
    let mods = if focused { held } else { Default::default() };
    [WindowEvent::ModifiersChanged(mods), WindowEvent::Focused(focused)].into_iter()
}

/// Modifiers physically held right now, whichever window has key.
fn held_modifiers() -> crate::platform::input::Mods {
    let flags = class(c"NSEvent").map_or(0, |cls| unsafe { msg_send!(cls, c"modifierFlags"; -> usize) });
    super::events::convert_modifier_flags(flags)
}

pub fn ensure_window_delegate_class() {
    if DELEGATE_CLASS_REGISTERED.swap(true, Ordering::SeqCst) {
        return;
    }
    unsafe {
        let superclass = class(c"NSObject").expect("NSObject class exists");
        let cls = objc_allocateClassPair(superclass, c"LittyWindowDelegate".as_ptr(), 0);
        assert!(!cls.is_null(), "failed to allocate LittyWindowDelegate class");

        // windowDidBecomeKey:
        extern "C" fn window_did_become_key(_this: Id, _cmd: Id, notification: Id) {
            unsafe {
                let win = msg_send!(notification, c"object"; -> Id);
                if !win.is_null() {
                    let view = msg_send!(win, c"contentView"; -> Id);
                    if !view.is_null() {
                        let tag = msg_send!(view, c"tag"; -> usize);
                        if tag != 0 {
                            let id = WindowId::from_raw(tag as u64);
                            focus_events(true, held_modifiers()).for_each(|ev| super::view::emit_event(id, ev));
                        }
                    }
                }
            }
        }
        class_addMethod(cls, sel(c"windowDidBecomeKey:"), window_did_become_key as *const (), c"v@:@".as_ptr() as *const _);

        // windowDidResignKey:
        extern "C" fn window_did_resign_key(_this: Id, _cmd: Id, notification: Id) {
            unsafe {
                let win = msg_send!(notification, c"object"; -> Id);
                if !win.is_null() {
                    let view = msg_send!(win, c"contentView"; -> Id);
                    if !view.is_null() {
                        let tag = msg_send!(view, c"tag"; -> usize);
                        if tag != 0 {
                            let id = WindowId::from_raw(tag as u64);
                            focus_events(false, Default::default()).for_each(|ev| super::view::emit_event(id, ev));
                        }
                    }
                }
            }
        }
        class_addMethod(cls, sel(c"windowDidResignKey:"), window_did_resign_key as *const (), c"v@:@".as_ptr() as *const _);

        // windowDidResize:
        extern "C" fn window_did_resize(_this: Id, _cmd: Id, notification: Id) {
            unsafe {
                let win = msg_send!(notification, c"object"; -> Id);
                if !win.is_null() {
                    let view = msg_send!(win, c"contentView"; -> Id);
                    if !view.is_null() {
                        let tag = msg_send!(view, c"tag"; -> usize);
                        if tag != 0 {
                            let id = WindowId::from_raw(tag as u64);
                            let frame = msg_send!(view, c"frame"; -> NSRect);
                            let scale = msg_send!(win, c"backingScaleFactor"; -> f64);
                            let w = (frame.w * scale).round() as u32;
                            let h = (frame.h * scale).round() as u32;
                            if w > 0 && h > 0 {
                                super::view::emit_event(id, WindowEvent::Resized(PhysicalSize::new(w, h)));
                            }
                        }
                    }
                }
            }
        }
        class_addMethod(cls, sel(c"windowDidResize:"), window_did_resize as *const (), c"v@:@".as_ptr() as *const _);

        // windowDidChangeBackingProperties:
        extern "C" fn window_did_change_backing_properties(_this: Id, _cmd: Id, notification: Id) {
            unsafe {
                let win = msg_send!(notification, c"object"; -> Id);
                if !win.is_null() {
                    let view = msg_send!(win, c"contentView"; -> Id);
                    if !view.is_null() {
                        let tag = msg_send!(view, c"tag"; -> usize);
                        if tag != 0 {
                            let id = WindowId::from_raw(tag as u64);
                            let scale = msg_send!(win, c"backingScaleFactor"; -> f64);
                            super::view::emit_event(id, WindowEvent::ScaleFactorChanged(scale));
                        }
                    }
                }
            }
        }
        class_addMethod(cls, sel(c"windowDidChangeBackingProperties:"), window_did_change_backing_properties as *const (), c"v@:@".as_ptr() as *const _);

        // windowShouldClose:
        extern "C" fn window_should_close(_this: Id, _cmd: Id, sender: Id) -> bool {
            unsafe {
                let view = msg_send!(sender, c"contentView"; -> Id);
                if !view.is_null() {
                    let tag = msg_send!(view, c"tag"; -> usize);
                    if tag != 0 {
                        let id = WindowId::from_raw(tag as u64);
                        super::view::emit_event(id, WindowEvent::CloseRequested);
                    }
                }
                false
            }
        }
        class_addMethod(cls, sel(c"windowShouldClose:"), window_should_close as *const (), c"B@:@".as_ptr() as *const _);

        objc_registerClassPair(cls);

        let instance = msg_send!(cls, c"new"; -> Id);
        SHARED_DELEGATE = instance;
    }
}

static NEXT_WINDOW_ID: AtomicU64 = AtomicU64::new(1);

/// Native macOS Window wrapping `NSWindow` and our custom `LittyView`.
#[derive(Clone)]
pub struct NativeWindow {
    id: WindowId,
    inner: Arc<NativeWindowInner>,
}

pub struct NativeWindowInner {
    pub ns_window: Id,
    pub ns_view: Id,
}

unsafe impl Send for NativeWindowInner {}
unsafe impl Sync for NativeWindowInner {}

impl Drop for NativeWindowInner {
    fn drop(&mut self) {
        unsafe {
            if !self.ns_window.is_null() {
                let was_key = msg_send!(self.ns_window, c"isKeyWindow"; -> bool);
                msg_send!(self.ns_window, c"close");
                if was_key {
                    hand_key_to_frontmost_window(self.ns_window);
                }
                msg_send!(self.ns_window, c"release");
            }
        }
    }
}

/// A window AppKit could make key after another closes, in front-to-back order.
#[derive(Clone, Copy)]
struct KeyCandidate {
    closing: bool,
    visible: bool,
    can_become_key: bool,
    on_active_space: bool,
}

/// Index of the window that should become key once the key window closes:
/// the frontmost visible one on the current Space that can take it.
fn next_key_window(front_to_back: impl IntoIterator<Item = KeyCandidate>) -> Option<usize> {
    front_to_back
        .into_iter()
        .position(|c| !c.closing && c.visible && c.can_become_key && c.on_active_space)
}

/// Make the frontmost remaining window key after `closing`, the key window,
/// was closed by us (`windowShouldClose:` defers closing to the app). AppKit
/// leaves the app with no key window then, so keystrokes and Cmd+W reach no
/// window until the user clicks one or reactivates the app. Where AppKit did
/// pick one (e.g. a sibling native tab), it is left alone.
unsafe fn hand_key_to_frontmost_window(closing: Id) {
    // Drop can run outside the event loop's pools (e.g. at exit).
    autoreleasepool(|| unsafe {
        let Some(app) = shared_application() else { return };
        if let Some(key) = send_id(app, c"keyWindow")
            && key != closing
        {
            return;
        }
        let Some(windows) = send_id(app, c"orderedWindows") else { return };
        let count = msg_send!(windows, c"count"; -> usize);
        let window_at = |i: usize| msg_send!(windows, c"objectAtIndex:", i => usize; -> Id);
        let candidates = (0..count).map(|i| {
            let w = window_at(i);
            KeyCandidate {
                closing: w == closing,
                visible: msg_send!(w, c"isVisible"; -> bool),
                can_become_key: msg_send!(w, c"canBecomeKeyWindow"; -> bool),
                on_active_space: msg_send!(w, c"isOnActiveSpace"; -> bool),
            }
        });
        if let Some(i) = next_key_window(candidates) {
            msg_send!(window_at(i), c"makeKeyWindow");
        }
    })
}

fn primary_screen_height() -> f64 {
    unsafe {
        if let Some(screen_cls) = class(c"NSScreen") {
            let screens = msg_send!(screen_cls, c"screens"; -> Id);
            if !screens.is_null() {
                let count = msg_send!(screens, c"count"; -> usize);
                if count > 0 {
                    let primary = msg_send!(screens, c"objectAtIndex:", 0usize => usize; -> Id);
                    if !primary.is_null() {
                        let frame = msg_send!(primary, c"frame"; -> NSRect);
                        return frame.h;
                    }
                }
            }
        }
        900.0
    }
}

impl NativeWindow {
    pub fn new(id: WindowId, ns_window: Id, ns_view: Id) -> Self {
        Self {
            id,
            inner: Arc::new(NativeWindowInner { ns_window, ns_view }),
        }
    }

    /// Create an NSWindow and embed our custom LittyView.
    pub fn create(spec: &WindowSpec) -> Result<Self, std::io::Error> {
        ensure_litty_view_class();
        ensure_window_delegate_class();

        let id = allocate_window_id();

        unsafe {
            let width = spec.inner_size.width;
            let height = spec.inner_size.height;
            let screen_h = primary_screen_height();
            let (x, y) = if let Some(pos) = spec.position {
                (pos.x, (screen_h - pos.y - height).max(0.0))
            } else {
                (100.0, (screen_h - 100.0 - height).max(0.0))
            };
            let rect = NSRect { x, y, w: width, h: height };

            // NSWindowStyleMask: Titled (1) | Closable (2) | Miniaturizable (4) | Resizable (8)
            let style_mask: usize = if spec.decorations {
                1 | 2 | 4 | 8
            } else {
                0
            };

            let win_cls = class(c"NSWindow").ok_or_else(|| std::io::Error::other("no NSWindow class"))?;
            let ns_window = msg_send!(
                win_cls,
                c"alloc" ; -> Id
            );
            let ns_window = msg_send!(
                ns_window,
                c"initWithContentRect:styleMask:backing:defer:",
                rect => NSRect,
                style_mask => usize,
                2usize => usize, // NSBackingStoreBuffered
                false => bool
                ; -> Id
            );

            if ns_window.is_null() {
                return Err(std::io::Error::other("failed to create NSWindow"));
            }

            // Do NOT let AppKit auto-release NSWindow when closed (Rust manages lifetime)
            msg_send!(ns_window, c"setReleasedWhenClosed:", false => bool);

            // Disallow native AppKit tab auto-grouping by default
            msg_send!(ns_window, c"setTabbingMode:", 2isize => isize); // NSWindowTabbingModeDisallowed

            // Create LittyView
            let view_cls = class(c"LittyView").ok_or_else(|| std::io::Error::other("no LittyView class"))?;
            let ns_view = msg_send!(view_cls, c"alloc" ; -> Id);
            let ns_view = msg_send!(ns_view, c"initWithFrame:", rect => NSRect ; -> Id);

            // Tag view with WindowId so delegate callbacks know which window fired
            let tag: usize = id.raw() as usize;
            msg_send!(ns_view, c"setTag:", tag => usize);

            // Make view layer-backed
            msg_send!(ns_view, c"setWantsLayer:", true => bool);

            // Set content view (retains ns_view, so release our alloc retain)
            msg_send!(ns_window, c"setContentView:", ns_view => Id);
            msg_send!(ns_view, c"release");

            // Attach window delegate for focus, resize, close notifications
            if !SHARED_DELEGATE.is_null() {
                msg_send!(ns_window, c"setDelegate:", SHARED_DELEGATE => Id);
            }

            // Add tracking area for mouse move, hover, and enter/leave tracking
            if let Some(tracking_cls) = class(c"NSTrackingArea") {
                // NSTrackingMouseEnteredAndExited (0x01) | NSTrackingMouseMoved (0x02) | NSTrackingActiveAlways (0x80) | NSTrackingInVisibleRect (0x200)
                let opts: usize = 0x01 | 0x02 | 0x80 | 0x200;
                let zero_rect = NSRect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
                let area = msg_send!(tracking_cls, c"alloc"; -> Id);
                let area = msg_send!(
                    area,
                    c"initWithRect:options:owner:userInfo:",
                    zero_rect => NSRect,
                    opts => usize,
                    ns_view => Id,
                    std::ptr::null_mut::<c_void>() => Id
                    ; -> Id
                );
                if !area.is_null() {
                    msg_send!(ns_view, c"addTrackingArea:", area => Id);
                    msg_send!(area, c"release");
                }
            }

            // Title
            if let Some(ns_title) = ns_string(spec.title) {
                msg_send!(ns_window, c"setTitle:", ns_title => Id);
            }

            // Window configuration
            msg_send!(ns_window, c"setAcceptsMouseMovedEvents:", true => bool);

            if spec.position.is_none() {
                center_on_main_screen(ns_window);
            }

            if spec.visible {
                msg_send!(ns_window, c"makeKeyAndOrderFront:", std::ptr::null_mut::<c_void>() => Id);
            }

            Ok(NativeWindow::new(id, ns_window, ns_view))
        }
    }

    pub fn id(&self) -> WindowId {
        self.id
    }

    #[allow(dead_code)]
    pub fn ns_window(&self) -> Id {
        self.inner.ns_window
    }

    #[allow(dead_code)]
    pub fn ns_view(&self) -> Id {
        self.inner.ns_view
    }

    pub fn request_redraw(&self) {
        super::app::request_redraw(self.id());
        unsafe {
            msg_send!(self.inner.ns_view, c"setNeedsDisplay:", true => bool);
        }
    }

    pub fn scale_factor(&self) -> f64 {
        unsafe {
            msg_send!(self.inner.ns_window, c"backingScaleFactor"; -> f64)
        }
    }

    pub fn inner_size(&self) -> PhysicalSize<u32> {
        unsafe {
            let frame = msg_send!(self.inner.ns_view, c"frame"; -> NSRect);
            let scale = self.scale_factor();
            PhysicalSize::new((frame.w * scale).round() as u32, (frame.h * scale).round() as u32)
        }
    }

    pub fn outer_position(&self) -> Result<PhysicalPosition<i32>, Unsupported> {
        unsafe {
            let frame = msg_send!(self.inner.ns_window, c"frame"; -> NSRect);
            Ok(top_left_on_screen(frame, primary_screen_height(), self.scale_factor()))
        }
    }

    /// Top-left corner of the content view (below the title bar) on the screen,
    /// the origin pointer positions are relative to.
    pub fn inner_position(&self) -> Result<PhysicalPosition<i32>, Unsupported> {
        unsafe {
            let frame = msg_send!(self.inner.ns_window, c"frame"; -> NSRect);
            let content = msg_send!(self.inner.ns_window, c"contentRectForFrameRect:", frame => NSRect; -> NSRect);
            Ok(top_left_on_screen(content, primary_screen_height(), self.scale_factor()))
        }
    }

    pub fn set_outer_position(&self, position: LogicalPosition<f64>) {
        unsafe {
            let frame = msg_send!(self.inner.ns_window, c"frame"; -> NSRect);
            let screen_h = primary_screen_height();
            let cocoa_y = screen_h - position.y - frame.h;
            msg_send!(self.inner.ns_window, c"setFrameOrigin:", NSPoint { x: position.x, y: cocoa_y } => NSPoint);
        }
    }

    /// Move the top-left to `position` in physical pixels, the unit
    /// `outer_position` reports; AppKit frames are in points.
    pub fn set_outer_position_physical(&self, position: PhysicalPosition<i32>) {
        self.set_outer_position(position.to_logical(self.scale_factor()));
    }

    pub fn is_visible(&self) -> bool {
        unsafe {
            msg_send!(self.inner.ns_window, c"isVisible"; -> bool)
        }
    }

    pub fn set_visible(&self, visible: bool) {
        unsafe {
            if visible {
                msg_send!(self.inner.ns_window, c"makeKeyAndOrderFront:", std::ptr::null_mut::<c_void>() => Id);
            } else {
                msg_send!(self.inner.ns_window, c"orderOut:", std::ptr::null_mut::<c_void>() => Id);
            }
        }
    }

    pub fn is_minimized(&self) -> Option<bool> {
        unsafe {
            let is_mini = msg_send!(self.inner.ns_window, c"isMiniaturized"; -> bool);
            Some(is_mini)
        }
    }

    pub fn set_minimized(&self, minimized: bool) {
        unsafe {
            if minimized {
                msg_send!(self.inner.ns_window, c"miniaturize:", std::ptr::null_mut::<c_void>() => Id);
            } else {
                msg_send!(self.inner.ns_window, c"deminiaturize:", std::ptr::null_mut::<c_void>() => Id);
            }
        }
    }

    pub fn is_maximized(&self) -> bool {
        unsafe {
            msg_send!(self.inner.ns_window, c"isZoomed"; -> bool)
        }
    }

    pub fn set_maximized(&self, _maximized: bool) {
        unsafe {
            msg_send!(self.inner.ns_window, c"zoom:", std::ptr::null_mut::<c_void>() => Id);
        }
    }

    pub fn is_fullscreen(&self) -> bool {
        unsafe {
            let style = msg_send!(self.inner.ns_window, c"styleMask"; -> usize);
            (style & (1 << 14)) != 0 // NSWindowStyleMaskFullScreen
        }
    }

    pub fn set_title(&self, title: &str) {
        unsafe {
            if let Some(ns_title) = ns_string(title) {
                msg_send!(self.inner.ns_window, c"setTitle:", ns_title => Id);
            }
        }
    }

    pub fn focus_window(&self) {
        unsafe {
            msg_send!(self.inner.ns_window, c"makeKeyAndOrderFront:", std::ptr::null_mut::<c_void>() => Id);
        }
    }

    pub fn set_cursor(&self, icon: CursorIcon) {
        unsafe {
            let sel_name = match icon {
                CursorIcon::Pointer => c"pointingHandCursor",
                CursorIcon::Text => c"IBeamCursor",
                CursorIcon::Crosshair => c"crosshairCursor",
                CursorIcon::ColResize | CursorIcon::EwResize => c"resizeLeftRightCursor",
                CursorIcon::RowResize | CursorIcon::NsResize => c"resizeUpDownCursor",
                _ => c"arrowCursor",
            };
            if let Some(cursor_cls) = class(c"NSCursor")
                && let Some(cursor) = send_id(cursor_cls, sel_name) {
                msg_send!(cursor, c"set");
            }
        }
    }

    /// Hide the pointer until it next moves, or show it again.
    ///
    /// `NSCursor hide`/`unhide` keep one process-wide counter, while the app
    /// tracks "hidden" per window: a window closed or left after hiding never
    /// balanced it, and the pointer stayed hidden over every later window.
    /// `setHiddenUntilMouseMoves:` has no counter, and AppKit shows the pointer
    /// on the next movement whichever window it is over.
    pub fn set_cursor_visible(&self, visible: bool) {
        unsafe {
            if let Some(cursor_cls) = class(c"NSCursor") {
                msg_send!(cursor_cls, c"setHiddenUntilMouseMoves:", !visible => bool);
            }
        }
    }

    pub fn theme(&self) -> Option<Theme> {
        Some(if crate::platform::system_dark() { Theme::Dark } else { Theme::Light })
    }

    pub fn set_ime_allowed(&self, _allowed: bool) {}

    pub fn set_ime_cursor_area(&self, _position: PhysicalPosition<f64>, _size: PhysicalSize<f64>) {}

    pub fn request_attention(&self, requested: bool) {
        unsafe {
            if let Some(app) = shared_application()
                && requested {
                msg_send!(app, c"requestUserAttention:", 0usize => usize); // NSInformationalRequest
            }
        }
    }

    pub fn drag_window(&self) -> Result<(), Unsupported> {
        unsafe {
            if let Some(app) = shared_application() {
                let event = msg_send!(app, c"currentEvent"; -> Id);
                if !event.is_null() {
                    msg_send!(self.inner.ns_window, c"performWindowDragWithEvent:", event => Id);
                    return Ok(());
                }
            }
        }
        Err(Unsupported)
    }

    pub fn drag_resize_window(&self, _direction: ResizeDirection) -> Result<(), Unsupported> {
        Err(Unsupported)
    }

    pub fn current_monitor(&self) -> Option<Monitor> {
        unsafe {
            let screen = send_id(self.inner.ns_window, c"screen")?;
            let frame = msg_send!(screen, c"frame"; -> NSRect);
            let scale = self.scale_factor();
            Some(Monitor {
                position: PhysicalPosition::new((frame.x * scale) as i32, (frame.y * scale) as i32),
                size: PhysicalSize::new((frame.w * scale) as u32, (frame.h * scale) as u32),
                refresh_rate_millihertz: None,
            })
        }
    }

    pub fn raw_handle(&self) -> Option<RawHandle> {
        Some(RawHandle::AppKit { ns_view: self.inner.ns_view })
    }
}

/// Top-left corner of `rect` (Cocoa screen coordinates: points, origin at the
/// bottom-left of the primary screen of height `screen_h`) in physical pixels
/// from the top-left of the primary screen.
fn top_left_on_screen(rect: NSRect, screen_h: f64, scale: f64) -> PhysicalPosition<i32> {
    let top = screen_h - (rect.y + rect.h);
    PhysicalPosition::new((rect.x * scale).round() as i32, (top * scale).round() as i32)
}

/// Center `ns_window` on the screen the user is working on: `mainScreen` is the
/// screen of the key window, or the one with the active menu bar when no window
/// is key (app launch, Dock reopen). AppKit's own `center` sits above center.
unsafe fn center_on_main_screen(ns_window: Id) { unsafe {
    let Some(screen) = class(c"NSScreen").and_then(|cls| send_id(cls, c"mainScreen")) else { return };
    let visible = msg_send!(screen, c"visibleFrame"; -> NSRect);
    let frame = msg_send!(ns_window, c"frame"; -> NSRect);
    let origin = centered_origin(visible, frame.w, frame.h);
    msg_send!(ns_window, c"setFrameOrigin:", origin => NSPoint);
}}

/// Bottom-left origin (Cocoa coordinates) that centers a `w` x `h` pt window
/// frame in `visible`. A frame larger than `visible` is pinned to its top-left
/// so the title bar and traffic lights stay reachable.
fn centered_origin(visible: NSRect, w: f64, h: f64) -> NSPoint {
    let x = visible.x + ((visible.w - w) / 2.0).round().max(0.0);
    let top = visible.y + visible.h;
    let y = (visible.y + ((visible.h - h) / 2.0).round()).min(top - h);
    NSPoint { x, y }
}

pub fn allocate_window_id() -> WindowId {
    let val = NEXT_WINDOW_ID.fetch_add(1, Ordering::Relaxed);
    unsafe { std::mem::transmute(val) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn losing_key_releases_held_modifiers() {
        // Cmd+W hides the window before Cmd is released; that release reaches no
        // window of ours, so the app must not keep treating Cmd as held.
        let events: Vec<_> = focus_events(false, Default::default()).collect();
        assert!(
            matches!(events.as_slice(), [WindowEvent::ModifiersChanged(m), WindowEvent::Focused(false)] if *m == crate::platform::input::Mods::default()),
            "{events:?}"
        );
    }

    const OPEN: KeyCandidate = KeyCandidate { closing: false, visible: true, can_become_key: true, on_active_space: true };
    const CLOSING: KeyCandidate = KeyCandidate { closing: true, ..OPEN };

    #[test]
    fn closing_the_key_window_hands_key_to_the_frontmost_remaining_one() {
        // Front to back: the closing window, then two open ones.
        assert_eq!(next_key_window([CLOSING, OPEN, OPEN]), Some(1));
    }

    #[test]
    fn hidden_and_non_key_windows_are_skipped() {
        let hidden = KeyCandidate { visible: false, ..OPEN };
        let panel = KeyCandidate { can_become_key: false, ..OPEN };
        assert_eq!(next_key_window([hidden, panel, OPEN]), Some(2));
    }

    #[test]
    fn windows_on_other_spaces_are_skipped() {
        // Keystrokes must not go to a terminal the user cannot see.
        let elsewhere = KeyCandidate { on_active_space: false, ..OPEN };
        assert_eq!(next_key_window([CLOSING, elsewhere, OPEN]), Some(2));
        assert_eq!(next_key_window([CLOSING, elsewhere]), None);
    }

    #[test]
    fn closing_the_last_window_hands_key_to_none() {
        assert_eq!(next_key_window([CLOSING]), None);
        assert_eq!(next_key_window([]), None);
    }

    const CMD: crate::platform::input::Mods = crate::platform::input::Mods { ctrl: false, alt: false, shift: false, sup: true };

    #[test]
    fn gaining_key_reports_the_modifiers_still_held() {
        // Cmd held through Cmd+N / Cmd+W: the window that becomes key gets no
        // flagsChanged, so without this the next `n` / `w` is typed as text.
        let events: Vec<_> = focus_events(true, CMD).collect();
        assert!(
            matches!(events.as_slice(), [WindowEvent::ModifiersChanged(m), WindowEvent::Focused(true)] if *m == CMD),
            "{events:?}"
        );
    }

    #[test]
    fn losing_key_ignores_the_modifiers_still_held() {
        let events: Vec<_> = focus_events(false, CMD).collect();
        assert!(
            matches!(events.as_slice(), [WindowEvent::ModifiersChanged(m), WindowEvent::Focused(false)] if *m == crate::platform::input::Mods::default()),
            "{events:?}"
        );
    }

    #[test]
    fn positions_are_measured_from_the_top_left_of_the_primary_screen() {
        let frame = NSRect { x: 100.0, y: 323.0, w: 1295.0, h: 778.0 };
        let at = top_left_on_screen(frame, 1117.0, 2.0);
        assert_eq!((at.x, at.y), (200, 32));
    }

    #[test]
    fn a_new_window_is_centered_in_the_visible_frame() {
        // 1512x982 pt screen, 25 pt menu bar on top, 70 pt Dock at the bottom.
        let visible = NSRect { x: 0.0, y: 70.0, w: 1512.0, h: 887.0 };
        let at = centered_origin(visible, 960.0, 628.0);
        assert_eq!((at.x, at.y), (276.0, 200.0));
    }

    #[test]
    fn centering_uses_the_offset_of_a_secondary_screen() {
        // A screen to the left of and below the primary one has negative origins.
        let visible = NSRect { x: -1920.0, y: -300.0, w: 1920.0, h: 1080.0 };
        let at = centered_origin(visible, 960.0, 600.0);
        assert_eq!((at.x, at.y), (-1440.0, -60.0));
    }

    #[test]
    fn a_window_larger_than_the_screen_keeps_its_title_bar_on_screen() {
        let visible = NSRect { x: 0.0, y: 70.0, w: 800.0, h: 500.0 };
        let at = centered_origin(visible, 960.0, 628.0);
        // Left edge and top edge are pinned to the visible frame.
        assert_eq!(at.x, 0.0);
        assert_eq!(at.y + 628.0, 570.0);
    }

    #[test]
    fn centered_origins_are_whole_points() {
        let visible = NSRect { x: 0.0, y: 0.0, w: 1001.0, h: 701.0 };
        let at = centered_origin(visible, 500.0, 300.0);
        assert_eq!((at.x, at.y), (251.0, 201.0));
    }

    #[test]
    fn the_content_area_starts_below_the_title_bar() {
        // Same window as above with a 28 pt title bar: the content rect is 28 pt shorter.
        let frame = NSRect { x: 100.0, y: 323.0, w: 1295.0, h: 778.0 };
        let content = NSRect { h: 750.0, ..frame };
        let outer = top_left_on_screen(frame, 1117.0, 2.0);
        let inner = top_left_on_screen(content, 1117.0, 2.0);
        assert_eq!(inner.x, outer.x);
        assert_eq!(inner.y - outer.y, 56, "28 pt at scale 2");
    }
}
