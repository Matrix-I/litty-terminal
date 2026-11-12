//! Native macOS font loading, glyph rasterization, ligature shaping, and color
//! emoji via `CoreText.framework` and `CoreGraphics.framework`.
//!
//! `linux.rs` is the non-macOS counterpart (dynamic `libfreetype`, `libharfbuzz`,
//! `libfontconfig`).

use std::ffi::c_void;
use std::ptr;

use parking_lot::Mutex;

use super::shaper::{FontFeature, ShapedGlyph, FONT_FEATURES};
use super::{synthesized_line_metrics, LineMetrics};
use crate::render::cg::{CGPoint, CGRect, CGSize};

#[repr(C)]
#[derive(Copy, Clone)]
struct CFRange {
    location: isize,
    length: isize,
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithCharacters(alloc: *const c_void, chars: *const u16, n: isize) -> *const c_void;
    fn CFStringCreateWithBytes(
        alloc: *const c_void,
        bytes: *const u8,
        num_bytes: isize,
        encoding: u32,
        is_external_representation: bool,
    ) -> *const c_void;
    fn CFRetain(cf: *const c_void) -> *const c_void;
    fn CFRelease(cf: *const c_void);
    fn CFEqual(cf1: *const c_void, cf2: *const c_void) -> bool;
    fn CFStringGetLength(the_string: *const c_void) -> isize;
    fn CFStringGetCharacters(the_string: *const c_void, range: CFRange, buffer: *mut u16);
    fn CFURLCreateFromFileSystemRepresentation(
        allocator: *const c_void,
        buffer: *const u8,
        buf_len: isize,
        is_directory: bool,
    ) -> *const c_void;
    fn CFArrayCreate(
        allocator: *const c_void,
        values: *const *const c_void,
        num_values: isize,
        call_backs: *const c_void,
    ) -> *const c_void;
    fn CFArrayGetCount(the_array: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(the_array: *const c_void, idx: isize) -> *const c_void;
    fn CFNumberCreate(allocator: *const c_void, the_type: isize, value_ptr: *const c_void) -> *const c_void;
    fn CFDictionaryCreate(
        alloc: *const c_void,
        keys: *const *const c_void,
        values: *const *const c_void,
        n: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> *const c_void;
    fn CFDictionaryGetValue(the_dict: *const c_void, key: *const c_void) -> *const c_void;
    fn CFAttributedStringCreate(alloc: *const c_void, s: *const c_void, attributes: *const c_void) -> *const c_void;
    static kCFTypeArrayCallBacks: [usize; 5];
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
    fn CTFontCreateWithFontDescriptor(
        descriptor: *const c_void,
        size: f64,
        matrix: *const c_void,
    ) -> *const c_void;
    fn CTFontCreateCopyWithAttributes(
        font: *const c_void,
        size: f64,
        matrix: *const c_void,
        attributes: *const c_void,
    ) -> *const c_void;
    fn CTFontDescriptorCreateCopyWithAttributes(
        original: *const c_void,
        attributes: *const c_void,
    ) -> *const c_void;
    fn CTFontManagerCreateFontDescriptorsFromURL(file_url: *const c_void) -> *const c_void;
    fn CTFontCreateForString(
        current_font: *const c_void,
        string: *const c_void,
        range: CFRange,
    ) -> *const c_void;
    fn CTFontCopyFamilyName(font: *const c_void) -> *const c_void;
    fn CTFontGetAscent(font: *const c_void) -> f64;
    fn CTFontGetDescent(font: *const c_void) -> f64;
    fn CTFontGetLeading(font: *const c_void) -> f64;
    fn CTFontGetGlyphsForCharacters(
        font: *const c_void,
        characters: *const u16,
        glyphs: *mut u16,
        count: isize,
    ) -> bool;
    fn CTFontGetAdvancesForGlyphs(
        font: *const c_void,
        orientation: u32,
        glyphs: *const u16,
        advances: *mut CGSize,
        count: isize,
    ) -> f64;
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
    fn CTLineGetGlyphRuns(line: *const c_void) -> *const c_void;
    fn CTLineDraw(line: *const c_void, ctx: *mut c_void);
    fn CTRunGetGlyphCount(run: *const c_void) -> isize;
    fn CTRunGetAttributes(run: *const c_void) -> *const c_void;
    fn CTRunGetGlyphs(run: *const c_void, range: CFRange, buffer: *mut u16);
    fn CTRunGetPositions(run: *const c_void, range: CFRange, buffer: *mut CGPoint);
    fn CTRunGetStringIndices(run: *const c_void, range: CFRange, buffer: *mut isize);
    static kCTFontAttributeName: *const c_void;
    static kCTForegroundColorFromContextAttributeName: *const c_void;
    static kCTFontFeatureSettingsAttribute: *const c_void;
    static kCTFontOpenTypeFeatureTag: *const c_void;
    static kCTFontOpenTypeFeatureValue: *const c_void;
}

const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const K_CF_NUMBER_SINT32_TYPE: isize = 3;

/// Native macOS font handle backed by `CTFontDescriptorRef` and cached
/// `CTFontRef`s for the most recently used pixel sizes.
pub struct NativeFont {
    desc: *const c_void,
    sized: Mutex<SizedFonts>,
}

/// Pixel sizes kept at once: atlases for windows on displays with different
/// scale factors share one `NativeFont`, and each asks for its own size.
const SIZED_FONTS: usize = 4;

/// Size for a glyph lookup before any sized font exists; glyph ids do not depend on it.
const LOOKUP_PX: f32 = 16.0;

/// Owned `CTFontRef`s by size bits; null slots are free. Replaced round-robin.
struct SizedFonts {
    fonts: [(u32, *const c_void); SIZED_FONTS],
    next: usize,
}

impl SizedFonts {
    const fn new() -> Self {
        SizedFonts { fonts: [(0, ptr::null()); SIZED_FONTS], next: 0 }
    }

    /// A retained font at `px`, created and cached on a miss (null on failure).
    unsafe fn get_or_create(&mut self, desc: *const c_void, px: f32) -> *const c_void {
        let bits = px.to_bits();
        if let Some(&(_, f)) = self.fonts.iter().find(|(b, f)| *b == bits && !f.is_null()) {
            return unsafe { CFRetain(f) };
        }
        let f = unsafe { CTFontCreateWithFontDescriptor(desc, px as f64, ptr::null()) };
        if f.is_null() {
            return ptr::null();
        }
        let slot = &mut self.fonts[self.next];
        if !slot.1.is_null() {
            unsafe { CFRelease(slot.1) };
        }
        *slot = (bits, f);
        self.next = (self.next + 1) % SIZED_FONTS;
        unsafe { CFRetain(f) }
    }

    /// Any cached font, retained.
    unsafe fn any(&self) -> Option<*const c_void> {
        let &(_, f) = self.fonts.iter().find(|(_, f)| !f.is_null())?;
        Some(unsafe { CFRetain(f) })
    }
}

// SAFETY: `CTFontDescriptorRef` and `CTFontRef` are immutable, thread-safe
// CoreText objects; mutable cache access is guarded by `Mutex`.
unsafe impl Send for NativeFont {}
unsafe impl Sync for NativeFont {}

impl Drop for NativeFont {
    fn drop(&mut self) {
        unsafe {
            for &(_, font) in &self.sized.lock().fonts {
                if !font.is_null() {
                    CFRelease(font);
                }
            }
            if !self.desc.is_null() {
                CFRelease(self.desc);
            }
        }
    }
}

impl NativeFont {
    /// Obtain a retained `CTFontRef` at `px` (cached for recently used sizes).
    unsafe fn font_at(&self, px: f32) -> *const c_void {
        unsafe { self.sized.lock().get_or_create(self.desc, px) }
    }

    /// The sizes of the cached fonts, sorted.
    #[cfg(test)]
    pub(super) fn cached_sizes(&self) -> Vec<f32> {
        let mut sizes: Vec<f32> =
            self.sized.lock().fonts.iter().filter(|(_, f)| !f.is_null()).map(|&(b, _)| f32::from_bits(b)).collect();
        sizes.sort_by(f32::total_cmp);
        sizes
    }

    /// A retained font for size-independent queries (cmap lookups): any cached
    /// one. Asking for a fixed size instead would take a cache slot from a size
    /// an atlas is drawing at.
    unsafe fn font_any_size(&self) -> *const c_void {
        let mut guard = self.sized.lock();
        match unsafe { guard.any() } {
            Some(f) => f,
            None => unsafe { guard.get_or_create(self.desc, LOOKUP_PX) },
        }
    }

    /// Look up the nominal glyph ID for `ch` (`0` if unmapped).
    pub fn lookup_glyph_index(&self, ch: char) -> u16 {
        unsafe {
            let f = self.font_any_size();
            if f.is_null() {
                return 0;
            }
            let mut u16_buf = [0u16; 2];
            let utf16 = ch.encode_utf16(&mut u16_buf);
            let mut glyphs = [0u16; 2];
            let ok = CTFontGetGlyphsForCharacters(f, utf16.as_ptr(), glyphs.as_mut_ptr(), utf16.len() as isize);
            CFRelease(f);
            if ok { glyphs[0] } else { 0 }
        }
    }

    /// Horizontal line metrics at `px`, or `None` if unavailable.
    pub fn horizontal_line_metrics(&self, px: f32) -> Option<LineMetrics> {
        unsafe {
            let f = self.font_at(px);
            if f.is_null() {
                return None;
            }
            let ascent = CTFontGetAscent(f) as f32;
            let descent = -(CTFontGetDescent(f) as f32);
            let line_gap = CTFontGetLeading(f).max(0.0) as f32;
            CFRelease(f);
            if ascent <= 0.0 && descent >= 0.0 {
                return None;
            }
            Some(LineMetrics { ascent, descent, line_gap, new_line_size: ascent - descent + line_gap })
        }
    }

    /// Advance width of `ch` at `px`.
    pub fn advance_width(&self, ch: char, px: f32) -> f32 {
        unsafe {
            let f = self.font_at(px);
            if f.is_null() {
                return px * 0.6;
            }
            let mut u16_buf = [0u16; 2];
            let utf16 = ch.encode_utf16(&mut u16_buf);
            let mut glyphs = [0u16; 2];
            CTFontGetGlyphsForCharacters(f, utf16.as_ptr(), glyphs.as_mut_ptr(), utf16.len() as isize);
            let adv = if glyphs[0] != 0 {
                CTFontGetAdvancesForGlyphs(f, 0, &glyphs[0], ptr::null_mut(), 1) as f32
            } else {
                px * 0.6
            };
            CFRelease(f);
            adv
        }
    }

    /// Bounding box `(xmin, ymin, width, height)` in pixels for `gid` at `px`.
    pub(super) fn glyph_bounds(&self, gid: u16, px: f32) -> (i32, i32, u32, u32) {
        unsafe {
            let f = self.font_at(px);
            if f.is_null() {
                return (0, 0, 0, 0);
            }
            let mut bbox = CGRect::default();
            CTFontGetBoundingRectsForGlyphs(f, 0, &gid, &mut bbox, 1);
            CFRelease(f);
            if bbox.size.width <= 0.01 || bbox.size.height <= 0.01 {
                return (0, 0, 0, 0);
            }
            let x0 = bbox.origin.x.floor() as i32;
            let y0 = bbox.origin.y.floor() as i32;
            let x1 = (bbox.origin.x + bbox.size.width).ceil() as i32;
            let y1 = (bbox.origin.y + bbox.size.height).ceil() as i32;
            (x0, y0, (x1 - x0).max(0) as u32, (y1 - y0).max(0) as u32)
        }
    }

    /// Rasterize character `ch` into `scratch` (`cw × chh` 8-bit grayscale).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn rasterize_char(
        &self,
        scratch: &mut [u8],
        cw: i32,
        chh: i32,
        baseline: i32,
        px: f32,
        ch: char,
        center_if_symbol: bool,
    ) {
        unsafe {
            let f = self.font_at(px);
            if f.is_null() {
                return;
            }
            let mut u16_buf = [0u16; 2];
            let utf16 = ch.encode_utf16(&mut u16_buf);
            let mut glyphs = [0u16; 2];
            if !CTFontGetGlyphsForCharacters(f, utf16.as_ptr(), glyphs.as_mut_ptr(), utf16.len() as isize)
                || glyphs[0] == 0
            {
                CFRelease(f);
                return;
            }
            let gid = glyphs[0];
            let mut bbox = CGRect::default();
            CTFontGetBoundingRectsForGlyphs(f, 0, &gid, &mut bbox, 1);

            let mut draw_font = f;
            if center_if_symbol && (bbox.size.width > cw as f64 || bbox.size.height > chh as f64) {
                let sx = cw as f64 / bbox.size.width.max(1.0);
                let sy = chh as f64 / bbox.size.height.max(1.0);
                let new_px = (px as f64 * sx.min(sy) * 0.92).max(6.0);
                let scaled = CTFontCreateCopyWithAttributes(f, new_px, ptr::null(), ptr::null());
                if !scaled.is_null() {
                    CFRelease(f);
                    draw_font = scaled;
                    CTFontGetBoundingRectsForGlyphs(draw_font, 0, &gid, &mut bbox, 1);
                }
            }

            let gx = if center_if_symbol {
                ((cw as f64 - bbox.size.width) * 0.5 - bbox.origin.x).clamp(0.0, cw as f64)
            } else {
                0.0
            };
            let gy = (chh - baseline) as f64;
            draw_glyph_gray(draw_font, gid, CGPoint { x: gx, y: gy }, scratch, cw as usize, chh as usize);
            CFRelease(draw_font);
        }
    }

    /// Rasterize glyph `gid` at `(pen_x, baseline)` into `canvas` (`w × h` 8-bit grayscale).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_indexed_glyph(
        &self,
        canvas: &mut [u8],
        w: usize,
        h: usize,
        pen_x: f32,
        baseline: i32,
        px: f32,
        gid: u16,
    ) {
        unsafe {
            let f = self.font_at(px);
            if f.is_null() {
                return;
            }
            let pos = CGPoint { x: pen_x as f64, y: (h as i32 - baseline) as f64 };
            draw_glyph_gray(f, gid, pos, canvas, w, h);
            CFRelease(f);
        }
    }

    /// Shape a run of ASCII cells using CoreText (`CTLineCreateWithAttributedString`)
    /// and append `(glyph_id, start_cell_index)` for glyphs produced by the primary font.
    pub(super) fn shape_ascii_run(&self, px: f32, run: &[crate::terminal::grid::Cell], out: &mut Vec<ShapedGlyph>) -> bool {
        out.clear();
        unsafe {
            let f = self.font_at(px);
            if f.is_null() {
                return false;
            }
            let mut utf16 = [0u16; 256];
            let n_chars = run.len().min(utf16.len());
            for (idx, c) in run[..n_chars].iter().enumerate() {
                utf16[idx] = c.codepoint as u16;
            }
            let cf_str = CFStringCreateWithCharacters(ptr::null(), utf16.as_ptr(), n_chars as isize);
            if cf_str.is_null() {
                CFRelease(f);
                return false;
            }
            let keys = [kCTFontAttributeName];
            let values = [f];
            let attrs = CFDictionaryCreate(
                ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                &kCFTypeDictionaryKeyCallBacks as *const _ as *const c_void,
                &kCFTypeDictionaryValueCallBacks as *const _ as *const c_void,
            );
            if attrs.is_null() {
                CFRelease(cf_str);
                CFRelease(f);
                return false;
            }
            let attr_str = CFAttributedStringCreate(ptr::null(), cf_str, attrs);
            CFRelease(attrs);
            CFRelease(cf_str);
            if attr_str.is_null() {
                CFRelease(f);
                return false;
            }
            let line = CTLineCreateWithAttributedString(attr_str);
            CFRelease(attr_str);
            if line.is_null() {
                CFRelease(f);
                return false;
            }
            let runs = CTLineGetGlyphRuns(line);
            let run_count = if runs.is_null() { 0 } else { CFArrayGetCount(runs) };
            let mut glyphs = [0u16; 256];
            let mut positions = [CGPoint::default(); 256];
            let mut indices = [0isize; 256];

            for r in 0..run_count {
                let ct_run = CFArrayGetValueAtIndex(runs, r);
                if ct_run.is_null() {
                    continue;
                }
                let run_attrs = CTRunGetAttributes(ct_run);
                if !run_attrs.is_null() {
                    let run_font = CFDictionaryGetValue(run_attrs, kCTFontAttributeName);
                    if !run_font.is_null() && !CFEqual(run_font, f) {
                        continue;
                    }
                }
                let gcount = CTRunGetGlyphCount(ct_run);
                if gcount <= 0 || gcount as usize > glyphs.len() {
                    continue;
                }
                let range = CFRange { location: 0, length: gcount };
                CTRunGetGlyphs(ct_run, range, glyphs.as_mut_ptr());
                CTRunGetPositions(ct_run, range, positions.as_mut_ptr());
                CTRunGetStringIndices(ct_run, range, indices.as_mut_ptr());
                for k in 0..gcount as usize {
                    let cluster = indices[k].max(0) as usize;
                    if cluster < n_chars {
                        out.push(ShapedGlyph {
                            gid: glyphs[k],
                            cluster,
                            x_offset_px: 0.0,
                            y_offset_px: positions[k].y as f32,
                        });
                    }
                }
            }
            CFRelease(line);
            CFRelease(f);
            true
        }
    }

    /// Shape and draw a grapheme cluster (`text`) using the primary font if all
    /// codepoints are supported by the primary font; returns `false` if any
    /// character falls back to a different font.
    pub(super) fn rasterize_cluster_primary(
        &self,
        buf: &mut [u8],
        w: u32,
        h: u32,
        baseline: i32,
        px: f32,
        text: &str,
    ) -> bool {
        let mut u16_buf = [0u16; 32];
        let mut n = 0usize;
        for ch in text.chars() {
            if self.lookup_glyph_index(ch) == 0 {
                return false;
            }
            let slice = ch.encode_utf16(&mut u16_buf[n..]);
            n += slice.len();
            if n + 2 > u16_buf.len() {
                break;
            }
        }
        if n == 0 {
            return false;
        }
        unsafe {
            let f = self.font_at(px);
            if f.is_null() {
                return false;
            }
            let cf_str = CFStringCreateWithCharacters(ptr::null(), u16_buf.as_ptr(), n as isize);
            if cf_str.is_null() {
                CFRelease(f);
                return false;
            }
            let keys = [kCTFontAttributeName, kCTForegroundColorFromContextAttributeName];
            let values = [f, kCFBooleanTrue];
            let attrs = CFDictionaryCreate(
                ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                2,
                &kCFTypeDictionaryKeyCallBacks as *const _ as *const c_void,
                &kCFTypeDictionaryValueCallBacks as *const _ as *const c_void,
            );
            CFRelease(f);
            if attrs.is_null() {
                CFRelease(cf_str);
                return false;
            }
            let attributed = CFAttributedStringCreate(ptr::null(), cf_str, attrs);
            CFRelease(attrs);
            CFRelease(cf_str);
            if attributed.is_null() {
                return false;
            }
            let line = CTLineCreateWithAttributedString(attributed);
            CFRelease(attributed);
            if line.is_null() {
                return false;
            }
            const GLYPH_PATH_BOUNDS: usize = 1 << 3;
            let b = CTLineGetBoundsWithOptions(line, GLYPH_PATH_BOUNDS);
            let cs = CGColorSpaceCreateDeviceGray();
            let ctx = CGBitmapContextCreate(
                buf.as_mut_ptr() as *mut c_void,
                w as usize,
                h as usize,
                8,
                w as usize,
                cs,
                0,
            );
            CGColorSpaceRelease(cs);
            if ctx.is_null() {
                CFRelease(line);
                return false;
            }
            CGContextSetAllowsAntialiasing(ctx, true);
            CGContextSetShouldAntialias(ctx, true);
            CGContextSetAllowsFontSmoothing(ctx, true);
            CGContextSetShouldSmoothFonts(ctx, true);
            CGContextSetGrayFillColor(ctx, 1.0, 1.0);
            let x = ((w as f64 - b.size.width) * 0.5 - b.origin.x).max(0.0);
            let y = (h as i32 - baseline) as f64;
            CGContextSetTextPosition(ctx, x, y);
            CTLineDraw(line, ctx);
            CGContextRelease(ctx);
            CFRelease(line);
            for v in buf.iter_mut() {
                *v = super::raster::stem_boost(*v);
            }
            buf.iter().any(|&v| v != 0)
        }
    }
}

unsafe fn draw_glyph_gray(
    font: *const c_void,
    gid: u16,
    pos: CGPoint,
    canvas: &mut [u8],
    w: usize,
    h: usize,
) {
    unsafe {
        let cs = CGColorSpaceCreateDeviceGray();
        let ctx = CGBitmapContextCreate(
            canvas.as_mut_ptr() as *mut c_void,
            w,
            h,
            8,
            w,
            cs,
            0,
        );
        CGColorSpaceRelease(cs);
        if ctx.is_null() {
            return;
        }
        CGContextSetAllowsAntialiasing(ctx, true);
        CGContextSetShouldAntialias(ctx, true);
        CGContextSetAllowsFontSmoothing(ctx, true);
        CGContextSetShouldSmoothFonts(ctx, true);
        CGContextSetGrayFillColor(ctx, 1.0, 1.0);
        CTFontDrawGlyphs(font, &gid, &pos, 1, ctx);
        CGContextRelease(ctx);
        for v in canvas.iter_mut() {
            *v = super::raster::stem_boost(*v);
        }
    }
}

/// Load a `NativeFont` from `path` at `collection_index`. When `substitutions` is
/// `true`, attaches `FONT_FEATURES` (`kCTFontFeatureSettingsAttribute`) to the descriptor.
pub(super) fn parse_font(path: &str, collection_index: u32, substitutions: bool) -> Option<NativeFont> {
    unsafe {
        let url = CFURLCreateFromFileSystemRepresentation(
            ptr::null(),
            path.as_ptr(),
            path.len() as isize,
            false,
        );
        if url.is_null() {
            return None;
        }
        let arr = CTFontManagerCreateFontDescriptorsFromURL(url);
        CFRelease(url);
        if arr.is_null() {
            return None;
        }
        let count = CFArrayGetCount(arr);
        if (collection_index as isize) >= count {
            CFRelease(arr);
            return None;
        }
        let base_desc = CFArrayGetValueAtIndex(arr, collection_index as isize);
        if base_desc.is_null() {
            CFRelease(arr);
            return None;
        }
        CFRetain(base_desc);
        CFRelease(arr);

        let desc = if substitutions {
            let features = FONT_FEATURES.lock();
            if features.is_empty() {
                base_desc
            } else {
                let with_feat = apply_features_to_descriptor(base_desc, &features);
                CFRelease(base_desc);
                with_feat
            }
        } else {
            base_desc
        };
        if desc.is_null() {
            return None;
        }
        Some(NativeFont { desc, sized: Mutex::new(SizedFonts::new()) })
    }
}

unsafe fn apply_features_to_descriptor(desc: *const c_void, features: &[FontFeature]) -> *const c_void {
    unsafe {
        let mut dicts: Vec<*const c_void> = Vec::with_capacity(features.len());
        for f in features {
            let tag_str = CFStringCreateWithBytes(
                ptr::null(),
                f.tag.as_ptr(),
                4,
                K_CF_STRING_ENCODING_UTF8,
                false,
            );
            let val_i32 = f.value as i32;
            let val_num = CFNumberCreate(ptr::null(), K_CF_NUMBER_SINT32_TYPE, &val_i32 as *const i32 as *const c_void);
            if !tag_str.is_null() && !val_num.is_null() {
                let keys = [kCTFontOpenTypeFeatureTag, kCTFontOpenTypeFeatureValue];
                let vals = [tag_str, val_num];
                let d = CFDictionaryCreate(
                    ptr::null(),
                    keys.as_ptr(),
                    vals.as_ptr(),
                    2,
                    &kCFTypeDictionaryKeyCallBacks as *const _ as *const c_void,
                    &kCFTypeDictionaryValueCallBacks as *const _ as *const c_void,
                );
                if !d.is_null() {
                    dicts.push(d);
                }
            }
            if !tag_str.is_null() {
                CFRelease(tag_str);
            }
            if !val_num.is_null() {
                CFRelease(val_num);
            }
        }
        let feat_array = CFArrayCreate(
            ptr::null(),
            dicts.as_ptr(),
            dicts.len() as isize,
            &kCFTypeArrayCallBacks as *const _ as *const c_void,
        );
        for d in dicts {
            CFRelease(d);
        }
        if feat_array.is_null() {
            return CFRetain(desc);
        }
        let attr_keys = [kCTFontFeatureSettingsAttribute];
        let attr_vals = [feat_array];
        let attrs = CFDictionaryCreate(
            ptr::null(),
            attr_keys.as_ptr(),
            attr_vals.as_ptr(),
            1,
            &kCFTypeDictionaryKeyCallBacks as *const _ as *const c_void,
            &kCFTypeDictionaryValueCallBacks as *const _ as *const c_void,
        );
        CFRelease(feat_array);
        if attrs.is_null() {
            return CFRetain(desc);
        }
        let copy = CTFontDescriptorCreateCopyWithAttributes(desc, attrs);
        CFRelease(attrs);
        if copy.is_null() { CFRetain(desc) } else { copy }
    }
}

pub(super) fn line_metrics(font: &NativeFont, px: f32) -> LineMetrics {
    font.horizontal_line_metrics(px).unwrap_or_else(|| synthesized_line_metrics(px))
}

unsafe fn cached_ct_base_font(is_emoji: bool, size: f64) -> *const c_void {
    unsafe {
        static BASE_FONT_CACHE: Mutex<[Option<(u64, usize)>; 2]> = parking_lot::const_mutex([None, None]);
        let slot_idx = usize::from(is_emoji);
        let size_bits = size.to_bits();
        {
            let cache = BASE_FONT_CACHE.lock();
            if let Some((cached_bits, ptr_val)) = cache[slot_idx]
                && cached_bits == size_bits
                && ptr_val != 0
            {
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
            && old_ptr != 0
        {
            CFRelease(old_ptr as *const c_void);
        }
        CFRetain(font)
    }
}

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

fn coretext_line(buf: &mut [u8], w: u32, h: u32, px: f32, text: &str, baseline: Option<i32>) -> bool {
    const PREMULTIPLIED_LAST: u32 = 1;
    const GLYPH_PATH_BOUNDS: usize = 1 << 3;

    let units: Vec<u16> = text.encode_utf16().collect();
    let is_emoji_font = baseline.is_none();
    unsafe {
        let cf_text = CFStringCreateWithCharacters(ptr::null(), units.as_ptr(), units.len() as isize);
        let line_at = |size: f64| -> *const c_void {
            let font = cached_ct_base_font(is_emoji_font, size);
            if font.is_null() {
                return ptr::null();
            }
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
