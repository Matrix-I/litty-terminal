//! Window look and behavior: vibrancy, blur, color space, title-bar zoom, Metal drawables.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

use crate::platform::gui::Window;

use crate::platform::{ColorSpace, WindowLook};

use super::objc::*;

/// Configure CAMetalLayer maximumDrawableCount = 2 (double buffering)
/// to minimize latency and save ~10 MB of framebuffer RAM per window.
pub fn configure_layer_drawables(window: &Window) {
    let Some((ns_view, _)) = ns_window_of(window) else { return };
    unsafe {
        let mut target_layer = None;
        if let Some(layer) = send_id(ns_view, c"layer") {
            if responds_to(layer, c"setMaximumDrawableCount:") {
                target_layer = Some(layer);
            } else if let Some(sublayers) = send_id(layer, c"sublayers") {
                let count = msg_send!(sublayers, c"count"; -> usize);
                for i in 0..count {
                    let sub = msg_send!(sublayers, c"objectAtIndex:", i => usize; -> Id);
                    if responds_to(sub, c"setMaximumDrawableCount:") {
                        target_layer = Some(sub);
                        break;
                    }
                }
            }
        }
        if let Some(layer) = target_layer {
            msg_send!(layer, c"setMaximumDrawableCount:", 2 => usize);
        }
    }
}

pub fn apply_window_config(window: &Window, look: &WindowLook) {
    let Some((ns_view, ns_window)) = ns_window_of(window) else { return };
    // SAFETY: every message below is sent to live AppKit objects with the
    // signature documented by Apple for that selector.
    unsafe {
        apply_color_space(ns_view, ns_window, look.colorspace);

        // Seamless titlebar (Ghostty-style): transparent, no separator line.
        msg_send!(ns_window, c"setTitlebarAppearsTransparent:", true => bool);
        if responds_to(ns_window, c"setTitlebarSeparatorStyle:") {
            // NSTitlebarSeparatorStyleNone = 1
            msg_send!(ns_window, c"setTitlebarSeparatorStyle:", 1 => i64);
        }

        let channel = |shift: u32| ((look.background >> shift) & 0xFF) as f64 / 255.0;
        let (r, g, b) = (channel(24), channel(16), channel(8));
        let a = look.background_opacity as f64;

        // Dark/light appearance so title text and window controls contrast cleanly.
        let luminance = 0.299 * r + 0.587 * g + 0.114 * b;
        let name = if luminance > 0.5 { "NSAppearanceNameAqua" } else { "NSAppearanceNameDarkAqua" };
        if let (Some(cls), Some(name)) = (class(c"NSAppearance"), ns_string(name)) {
            let appearance = msg_send!(cls, c"appearanceNamed:", name => Id; -> Id);
            if !appearance.is_null() {
                msg_send!(ns_window, c"setAppearance:", appearance => Id);
            }
        }

        // Window background exactly matching the terminal background.
        if let Some(cls) = class(c"NSColor") {
            let color = msg_send!(
                cls, c"colorWithSRGBRed:green:blue:alpha:", r => f64, g => f64, b => f64, a => f64; -> Id
            );
            if !color.is_null() {
                msg_send!(ns_window, c"setBackgroundColor:", color => Id);
            }
        }

        msg_send!(ns_window, c"setOpaque:", look.background_opacity >= 1.0 => bool);

        if look.background_blur > 0 && look.background_opacity < 1.0 {
            setup_visual_effect_view(ns_window);
        } else {
            remove_visual_effect_view(ns_window);
        }

        install_header_double_click_hook(ns_window);
    }
}

/// Remove any saved zoom frame associated with `window` when the window closes.
pub fn cleanup_window(window: &Window) {
    let Some((_, ns_window)) = ns_window_of(window) else { return };
    if let Some(map) = SAVED_FRAMES.lock().as_mut() {
        map.remove(&(ns_window as usize));
    }
}

static HOOK_INSTALLED: AtomicBool = AtomicBool::new(false);

static ORIGINAL_SEND_EVENT: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

static ORIGINAL_ZOOM: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

static SAVED_FRAMES: parking_lot::Mutex<Option<std::collections::HashMap<usize, NSRect>>> =
    parking_lot::Mutex::new(None);

fn default_unzoomed_frame(vis: NSRect) -> NSRect {
    let w = (vis.w * 0.70).round().max(600.0).min(vis.w);
    let h = (vis.h * 0.70).round().max(400.0).min(vis.h);
    let x = vis.x + ((vis.w - w) / 2.0).round();
    let y = vis.y + ((vis.h - h) / 2.0).round();
    NSRect { x, y, w, h }
}

unsafe fn toggle_window_zoom(this: Id) { unsafe {
    let screen = send_id(this, c"screen").or_else(|| {
        let sc_cls = class(c"NSScreen")?;
        send_id(sc_cls, c"mainScreen")
    });
    let Some(screen) = screen else { return };

    let visible_frame = msg_send!(screen, c"visibleFrame"; -> NSRect);
    let curr_frame = msg_send!(this, c"frame"; -> NSRect);

    let is_at_visible = (curr_frame.x - visible_frame.x).abs() < 5.0
        && (curr_frame.y - visible_frame.y).abs() < 5.0
        && (curr_frame.w - visible_frame.w).abs() < 5.0
        && (curr_frame.h - visible_frame.h).abs() < 5.0;
    let is_appkit_zoomed = msg_send!(this, c"isZoomed"; -> bool);
    let is_zoomed = is_at_visible || is_appkit_zoomed;

    let mut lock = SAVED_FRAMES.lock();
    let map = lock.get_or_insert_with(std::collections::HashMap::new);
    let key = this as usize;

    if is_zoomed {
        let target = if let Some(saved) = map.remove(&key) {
            let is_saved_zoomed =
                (saved.w - visible_frame.w).abs() < 5.0 && (saved.h - visible_frame.h).abs() < 5.0;
            if saved.w > 100.0 && saved.h > 100.0 && !is_saved_zoomed {
                saved
            } else {
                default_unzoomed_frame(visible_frame)
            }
        } else {
            default_unzoomed_frame(visible_frame)
        };
        msg_send!(this, c"setFrame:display:animate:", target => NSRect, true => bool, true => bool);
    } else {
        map.insert(key, curr_frame);
        msg_send!(this, c"setFrame:display:animate:", visible_frame => NSRect, true => bool, true => bool);
    }
}}

unsafe fn is_in_window_header(this: Id, event: Id) -> bool { unsafe {
    let loc = msg_send!(event, c"locationInWindow"; -> NSPoint);
    let frame = msg_send!(this, c"frame"; -> NSRect);

    // Don't intercept when in native full screen mode (NSWindowStyleMaskFullScreen = 1 << 14)
    let style_mask = msg_send!(this, c"styleMask"; -> u64);
    if (style_mask & (1 << 14)) != 0 {
        return false;
    }

    // Left traffic-light buttons (close, minimize, zoom) are within x < 75.0
    if loc.x < 75.0 || loc.x > frame.w {
        return false;
    }

    let content_rect = msg_send!(this, c"contentRectForFrameRect:", frame => NSRect; -> NSRect);
    let header_bottom = if frame.h > content_rect.h {
        content_rect.h - 2.0
    } else {
        frame.h - 38.0
    };

    loc.y >= header_bottom && loc.y <= frame.h + 10.0
}}

extern "C" fn custom_zoom(this: Id, _cmd: Id, _sender: Id) {
    unsafe {
        toggle_window_zoom(this);
    }
}

extern "C" fn custom_send_event(this: Id, cmd: Id, event: Id) {
    if !event.is_null() {
        let is_header_double_click = unsafe {
            // NSEventTypeLeftMouseDown = 1
            let ev_type = msg_send!(event, c"type"; -> usize);
            if ev_type == 1 {
                let click_count = msg_send!(event, c"clickCount"; -> isize);
                if click_count == 2 {
                    let modifiers = msg_send!(event, c"modifierFlags"; -> usize);
                    // Ignore if Command (1 << 20) or Control (1 << 18) is held
                    if (modifiers & ((1 << 20) | (1 << 18))) == 0 {
                        is_in_window_header(this, event)
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            }
        };

        if is_header_double_click {
            unsafe {
                toggle_window_zoom(this);
            }
            return;
        }
    }

    let orig = ORIGINAL_SEND_EVENT.load(Ordering::Relaxed);
    if !orig.is_null() {
        let f: extern "C" fn(Id, Id, Id) = unsafe { std::mem::transmute(orig) };
        f(this, cmd, event);
    }
}

unsafe fn install_header_double_click_hook(ns_window: Id) { unsafe {
    if HOOK_INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }

    let Some(window_cls) = send_id(ns_window, c"class") else { return };

    // Swizzle sendEvent:
    let send_event_sel = sel(c"sendEvent:");
    let send_event_m = class_getInstanceMethod(window_cls, send_event_sel);
    if !send_event_m.is_null() {
        let orig_imp = method_getImplementation(send_event_m);
        let added = class_addMethod(
            window_cls,
            send_event_sel,
            custom_send_event as *const (),
            c"v@:@".as_ptr(),
        );
        let prev = if added {
            orig_imp
        } else {
            method_setImplementation(send_event_m, custom_send_event as *const ())
        };
        ORIGINAL_SEND_EVENT.store(prev as *mut c_void, Ordering::SeqCst);
    }

    // Swizzle zoom:
    let zoom_sel = sel(c"zoom:");
    let zoom_m = class_getInstanceMethod(window_cls, zoom_sel);
    if !zoom_m.is_null() {
        let orig_imp = method_getImplementation(zoom_m);
        let added = class_addMethod(
            window_cls,
            zoom_sel,
            custom_zoom as *const (),
            c"v@:@".as_ptr(),
        );
        let prev = if added {
            orig_imp
        } else {
            method_setImplementation(zoom_m, custom_zoom as *const ())
        };
        ORIGINAL_ZOOM.store(prev as *mut c_void, Ordering::SeqCst);
    }
}}

/// Set the window and CAMetalLayer color space so Metal outputs sRGB or Display P3.
unsafe fn apply_color_space(ns_view: Id, ns_window: Id, space: ColorSpace) { unsafe {
    let Some(cls) = class(c"NSColorSpace") else { return };
    let ctor = match space {
        ColorSpace::DisplayP3 => c"displayP3ColorSpace",
        ColorSpace::Srgb => c"sRGBColorSpace",
    };
    let Some(cs) = send_id(cls, ctor) else { return };
    msg_send!(ns_window, c"setColorSpace:", cs => Id);

    let Some(cg_cs) = send_id(cs, c"CGColorSpace") else { return };
    if let Some(layer) = send_id(ns_view, c"layer") {
        apply_layer_colorspace(layer, cg_cs);
    }
    if let Some(content_view) = send_id(ns_window, c"contentView").filter(|&v| v != ns_view)
        && let Some(layer) = send_id(content_view, c"layer") {
            apply_layer_colorspace(layer, cg_cs);
        }
    if let Some(subviews) = send_id(ns_view, c"subviews") {
        for_each_in_array(subviews, |sv| {
            if let Some(layer) = send_id(sv, c"layer") {
                apply_layer_colorspace(layer, cg_cs);
            }
        });
    }
}}

unsafe fn apply_layer_colorspace(layer: Id, cg_cs: Id) { unsafe {
    for setter in [c"setColorspace:", c"setColorSpace:"] {
        if responds_to(layer, setter) {
            msg_send!(layer, setter, cg_cs => Id);
        }
    }
    if let Some(sublayers) = send_id(layer, c"sublayers") {
        for_each_in_array(sublayers, |sub| apply_layer_colorspace(sub, cg_cs));
    }
}}

/// Background blur: an NSVisualEffectView behind the window content.
unsafe fn setup_visual_effect_view(ns_window: Id) { unsafe {
    let Some(content_view) = send_id(ns_window, c"contentView") else { return };
    // Avoid duplicate effect views if config is re-applied to the same window
    if let Some(effect_cls) = class(c"NSVisualEffectView") {
        let mut already_has_blur = false;
        if let Some(subviews) = send_id(content_view, c"subviews") {
            for_each_in_array(subviews, |subview| {
                if msg_send!(subview, c"isKindOfClass:", effect_cls => Id; -> bool) {
                    already_has_blur = true;
                }
            });
        }
        if already_has_blur {
            return;
        }
    }
    let Some(alloc) = class(c"NSVisualEffectView").and_then(|cls| send_id(cls, c"alloc")) else { return };

    let bounds = msg_send!(content_view, c"bounds"; -> NSRect);
    let effect_view = msg_send!(alloc, c"initWithFrame:", bounds => NSRect; -> Id);
    if effect_view.is_null() {
        return;
    }
    // NSViewWidthSizable (2) | NSViewHeightSizable (16)
    msg_send!(effect_view, c"setAutoresizingMask:", 18 => u64);
    // NSVisualEffectMaterialUnderWindowBackground
    msg_send!(effect_view, c"setMaterial:", 11 => i64);
    // NSVisualEffectBlendingModeBehindWindow
    msg_send!(effect_view, c"setBlendingMode:", 0 => i64);
    // NSVisualEffectStateActive
    msg_send!(effect_view, c"setState:", 1 => i64);
    // [contentView addSubview:effect_view positioned:NSWindowBelow relativeTo:nil]
    msg_send!(
        content_view, c"addSubview:positioned:relativeTo:",
        effect_view => Id, -1 => i64, std::ptr::null_mut() => Id
    );
    msg_send!(effect_view, c"release");
}}

/// Remove any attached NSVisualEffectView when blur is disabled or opacity is 1.0.
unsafe fn remove_visual_effect_view(ns_window: Id) { unsafe {
    let Some(content_view) = send_id(ns_window, c"contentView") else { return };
    let Some(effect_cls) = class(c"NSVisualEffectView") else { return };
    let mut to_remove: Vec<Id> = Vec::new();
    if let Some(subviews) = send_id(content_view, c"subviews") {
        for_each_in_array(subviews, |subview| {
            if msg_send!(subview, c"isKindOfClass:", effect_cls => Id; -> bool) {
                to_remove.push(subview);
            }
        });
    }
    for view in to_remove {
        msg_send!(view, c"removeFromSuperview");
    }
}}

/// Show the shell's working directory as the window's proxy icon.
pub fn set_window_represented_filename(window: &Window, path: &str) {
    let Some((_, ns_window)) = ns_window_of(window) else { return };
    // SAFETY: NSWindow.setRepresentedFilename: takes an NSString.
    unsafe {
        if let Some(path) = ns_string(path) {
            msg_send!(ns_window, c"setRepresentedFilename:", path => Id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_unzoomed_frame() {
        let vis = NSRect { x: 0.0, y: 25.0, w: 1512.0, h: 957.0 };
        let unzoomed = default_unzoomed_frame(vis);
        assert!(unzoomed.w >= 600.0 && unzoomed.w <= vis.w);
        assert!(unzoomed.h >= 400.0 && unzoomed.h <= vis.h);
        assert!(unzoomed.x >= vis.x);
        assert!(unzoomed.y >= vis.y);
    }

    #[test]
    fn test_saved_frames_cycle() {
        let mut lock = SAVED_FRAMES.lock();
        let map = lock.get_or_insert_with(std::collections::HashMap::new);
        let test_id = 0x12345usize;
        let original_frame = NSRect { x: 100.0, y: 100.0, w: 900.0, h: 600.0 };

        // Save frame on zoom
        map.insert(test_id, original_frame);
        assert_eq!(map.len(), 1);

        // Pop frame on unzoom
        let restored = map.remove(&test_id).unwrap();
        assert_eq!(restored.x, 100.0);
        assert_eq!(restored.y, 100.0);
        assert_eq!(restored.w, 900.0);
        assert_eq!(restored.h, 600.0);
        assert!(map.get(&test_id).is_none());
    }
}
