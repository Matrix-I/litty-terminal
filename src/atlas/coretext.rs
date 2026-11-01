//! macOS system rasterizers through CoreText: single-glyph fallback for
//! codepoints the configured fonts lack, color emoji, and grapheme clusters
//! the primary font cannot shape. `linux.rs` is the non-macOS counterpart.

use std::ffi::c_void;
use std::ptr;

use parking_lot::Mutex;

#[repr(C)]
#[derive(Copy, Clone)]
struct CFRange {
    location: isize,
    length: isize,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithCharacters(alloc: *const c_void, chars: *const u16, n: isize) -> *const c_void;
    fn CFRetain(cf: *const c_void) -> *const c_void;
    fn CFRelease(cf: *const c_void);
    fn CFStringGetLength(the_string: *const c_void) -> isize;
    fn CFStringGetCharacters(the_string: *const c_void, range: CFRange, buffer: *mut u16);
    fn CFDictionaryCreate(
        alloc: *const c_void,
        keys: *const *const c_void,
        values: *const *const c_void,
        n: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> *const c_void;
    fn CFAttributedStringCreate(alloc: *const c_void, s: *const c_void, attributes: *const c_void) -> *const c_void;
    static kCFTypeDictionaryKeyCallBacks: [usize; 6];
    static kCFTypeDictionaryValueCallBacks: [usize; 5];
    static kCFBooleanTrue: *const c_void;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGColorSpaceCreateDeviceGray() -> *mut c_void;
    fn CGColorSpaceRelease(space: *mut c_void);
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: *mut c_void,
        bitmap_info: u32,
    ) -> *mut c_void;
    fn CGContextRelease(c: *mut c_void);
    fn CGContextSetGrayFillColor(c: *mut c_void, gray: f64, alpha: f64);
    fn CGContextSetAllowsAntialiasing(c: *mut c_void, allows: bool);
    fn CGContextSetShouldAntialias(c: *mut c_void, should: bool);
    fn CGContextSetAllowsFontSmoothing(c: *mut c_void, allows: bool);
    fn CGContextSetShouldSmoothFonts(c: *mut c_void, should: bool);
    fn CGColorSpaceCreateDeviceRGB() -> *mut c_void;
    fn CGContextSetTextPosition(c: *mut c_void, x: f64, y: f64);
}

#[link(name = "CoreText", kind = "framework")]
unsafe extern "C" {
    fn CTFontCreateWithName(name: *const c_void, size: f64, matrix: *const c_void) -> *const c_void;
    fn CTFontCreateForString(
        current_font: *const c_void,
        string: *const c_void,
        range: CFRange,
    ) -> *const c_void;
    fn CTFontCopyFamilyName(font: *const c_void) -> *const c_void;
    fn CTFontGetGlyphsForCharacters(
        font: *const c_void,
        characters: *const u16,
        glyphs: *mut u16,
        count: isize,
    ) -> bool;
    fn CTFontGetBoundingRectsForGlyphs(
        font: *const c_void,
        orientation: u32,
        glyphs: *const u16,
        bounding_rects: *mut CGRect,
        count: isize,
    ) -> CGRect;
    fn CTFontDrawGlyphs(
        font: *const c_void,
        glyphs: *const u16,
        positions: *const CGPoint,
        count: usize,
        context: *mut c_void,
    );
    fn CTLineCreateWithAttributedString(s: *const c_void) -> *const c_void;
    fn CTLineGetBoundsWithOptions(line: *const c_void, options: usize) -> CGRect;
    fn CTLineDraw(line: *const c_void, ctx: *mut c_void);
    static kCTFontAttributeName: *const c_void;
    static kCTForegroundColorFromContextAttributeName: *const c_void;
}

unsafe fn cached_ct_base_font(is_emoji: bool, size: f64) -> *const std::ffi::c_void { unsafe {


    static BASE_FONT_CACHE: Mutex<[Option<(u64, usize)>; 2]> = parking_lot::const_mutex([None, None]);
    let slot_idx = usize::from(is_emoji);
    let size_bits = size.to_bits();
    {
        let cache = BASE_FONT_CACHE.lock();
        if let Some((cached_bits, ptr_val)) = cache[slot_idx]
            && cached_bits == size_bits && ptr_val != 0 {
                return CFRetain(ptr_val as *const c_void);
            }
    }

    let name_str = if is_emoji { "Apple Color Emoji" } else { "Menlo" };
    let units: Vec<u16> = name_str.encode_utf16().collect();
    let cf_name = CFStringCreateWithCharacters(ptr::null(), units.as_ptr(), units.len() as isize);
    if cf_name.is_null() {
        return ptr::null();
    }
    let font = CTFontCreateWithName(cf_name, size, ptr::null());
    CFRelease(cf_name);
    if font.is_null() {
        return ptr::null();
    }

    let mut cache = BASE_FONT_CACHE.lock();
    if let Some((_, old_ptr)) = cache[slot_idx].replace((size_bits, font as usize))
        && old_ptr != 0 {
            CFRelease(old_ptr as *const c_void);
        }
    CFRetain(font)
}}

pub(crate) fn macos_coretext_rasterize(
    buf: &mut [u8],
    cw: u32,
    ch_px: u32,
    baseline: i32,
    px: f32,
    ch: char,
) -> bool {




    unsafe {
        let mut u16_buf = [0u16; 2];
        let utf16 = ch.encode_utf16(&mut u16_buf);
        let cf_str = CFStringCreateWithCharacters(ptr::null(), utf16.as_ptr(), utf16.len() as isize);
        if cf_str.is_null() {
            return false;
        }

        let base_font = cached_ct_base_font(false, px as f64);
        if base_font.is_null() {
            CFRelease(cf_str);
            return false;
        }

        let fb_font = CTFontCreateForString(
            base_font,
            cf_str,
            CFRange { location: 0, length: utf16.len() as isize },
        );
        CFRelease(base_font);
        CFRelease(cf_str);
        if fb_font.is_null() {
            return false;
        }

        // Ignore macOS ".LastResort" fallback font so we never draw LastResort boxes.
        let fam = CTFontCopyFamilyName(fb_font);
        if !fam.is_null() {
            let len = CFStringGetLength(fam);
            let mut fbuf = [0u16; 32];
            let n = len.min(32);
            CFStringGetCharacters(fam, CFRange { location: 0, length: n }, fbuf.as_mut_ptr());
            CFRelease(fam);
            let fam_str = String::from_utf16_lossy(&fbuf[..n as usize]);
            if fam_str.contains("LastResort") {
                CFRelease(fb_font);
                return false;
            }
        }

        let mut glyphs = [0u16; 2];
        let ok = CTFontGetGlyphsForCharacters(
            fb_font,
            utf16.as_ptr(),
            glyphs.as_mut_ptr(),
            utf16.len() as isize,
        );
        if !ok || glyphs[0] == 0 {
            CFRelease(fb_font);
            return false;
        }

        let mut bbox = CGRect::default();
        CTFontGetBoundingRectsForGlyphs(fb_font, 0, &glyphs[0], &mut bbox, 1);

        // If the fallback glyph is wider than a single cell, create a scaled font so it fits.
        let mut draw_font = fb_font;
        if bbox.size.width > cw as f64 * 1.02 && bbox.size.width > 1.0 {
            let scale = (cw as f64 / bbox.size.width) * 0.92;
            let fam = CTFontCopyFamilyName(fb_font);
            if !fam.is_null() {
                let scaled = CTFontCreateWithName(fam, px as f64 * scale, ptr::null());
                CFRelease(fam);
                if !scaled.is_null() {
                    let mut g2 = [0u16; 2];
                    if CTFontGetGlyphsForCharacters(
                        scaled,
                        utf16.as_ptr(),
                        g2.as_mut_ptr(),
                        utf16.len() as isize,
                    ) && g2[0] != 0
                    {
                        CFRelease(fb_font);
                        draw_font = scaled;
                        glyphs[0] = g2[0];
                        CTFontGetBoundingRectsForGlyphs(draw_font, 0, &glyphs[0], &mut bbox, 1);
                    } else {
                        CFRelease(scaled);
                    }
                }
            }
        }

        let cs = CGColorSpaceCreateDeviceGray();
        let ctx = CGBitmapContextCreate(
            buf.as_mut_ptr() as *mut c_void,
            cw as usize,
            ch_px as usize,
            8,
            cw as usize,
            cs,
            0, // kCGImageAlphaNone
        );
        CGColorSpaceRelease(cs);
        if ctx.is_null() {
            CFRelease(draw_font);
            return false;
        }

        CGContextSetAllowsAntialiasing(ctx, true);
        CGContextSetShouldAntialias(ctx, true);
        CGContextSetAllowsFontSmoothing(ctx, true);
        CGContextSetShouldSmoothFonts(ctx, true);
        CGContextSetGrayFillColor(ctx, 1.0, 1.0);

        let gx = ((cw as f64 - bbox.size.width) * 0.5 - bbox.origin.x).clamp(0.0, cw as f64);
        let gy = (ch_px as i32 - baseline) as f64;
        let pos = CGPoint { x: gx, y: gy };
        CTFontDrawGlyphs(draw_font, &glyphs[0], &pos, 1, ctx);

        CGContextRelease(ctx);
        CFRelease(draw_font);
        true
    }
}

/// Rasterize `text` (one emoji, possibly a ZWJ / flag / skin tone sequence)
/// from the color emoji font into `buf`, a zeroed `w × h` premultiplied RGBA
/// canvas, scaled to fit and centered.
pub(super) fn rasterize_color(buf: &mut [u8], w: u32, h: u32, px: f32, text: &str) -> bool {
    coretext_line(buf, w, h, px, text, None)
}

/// A grapheme cluster as text (an Indic conjunct, a base with marks) through
/// CoreText's shaping and font cascade into a `w × h` coverage canvas, on the
/// baseline; for clusters the primary font cannot shape.
pub(super) fn rasterize_text_cluster(buf: &mut [u8], w: u32, h: u32, baseline: i32, px: f32, text: &str) -> bool {
    coretext_line(buf, w, h, px, text, Some(baseline))
}

/// Draw `text` as one CoreText line: color emoji (RGBA, scaled to fit and
/// centered) when `baseline` is `None`, else text coverage (one byte per px)
/// from Menlo and the system cascade, centered horizontally on the baseline.
fn coretext_line(buf: &mut [u8], w: u32, h: u32, px: f32, text: &str, baseline: Option<i32>) -> bool {

    /// `kCGImageAlphaPremultipliedLast`: RGBA bytes.
    const PREMULTIPLIED_LAST: u32 = 1;
    /// `kCTLineBoundsUseGlyphPathBounds`: the ink, not the typographic box.
    const GLYPH_PATH_BOUNDS: usize = 1 << 3;

    let units: Vec<u16> = text.encode_utf16().collect();
    let is_emoji_font = baseline.is_none();
    // SAFETY: every CoreFoundation object created here is released; the
    // context draws into `buf`, which outlives it and is `w * h * 4` bytes.
    unsafe {
        let cf_text = CFStringCreateWithCharacters(ptr::null(), units.as_ptr(), units.len() as isize);
        // A whole sequence shaped as one line, so ZWJ / flag / skin tone ligatures apply.
        let line_at = |size: f64| -> *const c_void {
            let font = cached_ct_base_font(is_emoji_font, size);
            if font.is_null() {
                return ptr::null();
            }
            // Text is drawn in the context's fill color (white coverage).
            let keys = [kCTFontAttributeName, kCTForegroundColorFromContextAttributeName];
            let values = [font, kCFBooleanTrue];
            let attrs = CFDictionaryCreate(
                ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1 + baseline.is_some() as isize,
                &kCFTypeDictionaryKeyCallBacks as *const _ as *const c_void,
                &kCFTypeDictionaryValueCallBacks as *const _ as *const c_void,
            );
            CFRelease(font);
            if attrs.is_null() {
                return ptr::null();
            }
            let attributed = CFAttributedStringCreate(ptr::null(), cf_text, attrs);
            CFRelease(attrs);
            if attributed.is_null() {
                return ptr::null();
            }
            let line = CTLineCreateWithAttributedString(attributed);
            CFRelease(attributed);
            line
        };
        let mut line = if cf_text.is_null() { ptr::null() } else { line_at(px as f64) };
        if !line.is_null() {
            let b = CTLineGetBoundsWithOptions(line, GLYPH_PATH_BOUNDS);
            // Shrink to fit the cells (emoji are wider and taller than text at the same size).
            let scale = match baseline {
                None => (w as f64 / b.size.width.max(1.0)).min(h as f64 / b.size.height.max(1.0)) * 0.95,
                Some(_) => w as f64 / b.size.width.max(1.0),
            };
            if scale < 1.0 {
                let smaller = line_at(px as f64 * scale);
                if !smaller.is_null() {
                    CFRelease(line);
                    line = smaller;
                }
            }
        }
        if !cf_text.is_null() {
            CFRelease(cf_text);
        }
        if line.is_null() {
            return false;
        }
        let b = CTLineGetBoundsWithOptions(line, GLYPH_PATH_BOUNDS);
        let (cs, bpp, info) = match baseline {
            None => (CGColorSpaceCreateDeviceRGB(), 4, PREMULTIPLIED_LAST),
            Some(_) => (CGColorSpaceCreateDeviceGray(), 1, 0),
        };
        let ctx = CGBitmapContextCreate(buf.as_mut_ptr() as *mut c_void, w as usize, h as usize, 8, w as usize * bpp, cs, info);
        CGColorSpaceRelease(cs);
        if ctx.is_null() {
            CFRelease(line);
            return false;
        }
        CGContextSetGrayFillColor(ctx, 1.0, 1.0);
        // Centered in the canvas (CoreGraphics has its origin at the bottom left),
        // text on the cell's baseline.
        let x = (w as f64 - b.size.width) / 2.0 - b.origin.x;
        let y = match baseline {
            None => (h as f64 - b.size.height) / 2.0 - b.origin.y,
            Some(base) => (h as i32 - base) as f64,
        };
        CGContextSetTextPosition(ctx, x, y);
        CTLineDraw(line, ctx);
        CGContextRelease(ctx);
        CFRelease(line);
    }
    match baseline {
        None => buf.as_chunks::<4>().0.iter().any(|p| p[3] != 0),
        Some(_) => buf.iter().any(|&v| v != 0),
    }
}

