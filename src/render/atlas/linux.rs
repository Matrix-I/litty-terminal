//! Native Linux font loading, glyph rasterization, HarfBuzz shaping, and
//! Fontconfig fallback via dynamically loaded `libfreetype.so.6`,
//! `libharfbuzz.so.0`, and `libfontconfig.so.1`.
//!
//! `coretext.rs` is the macOS counterpart.

use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void, CString};
use std::ptr;
use std::sync::OnceLock;

use parking_lot::Mutex;

use super::blit_fit;
use super::shaper::{FontFeature, ShapedGlyph, FONT_FEATURES};
use super::{synthesized_line_metrics, LineMetrics};

const FT_LOAD_DEFAULT: i32 = 0;
const FT_LOAD_RENDER: i32 = 1 << 2;
const FT_LOAD_COLOR: i32 = 1 << 20;
const FT_PIXEL_MODE_GRAY: u8 = 2;
const FT_PIXEL_MODE_BGRA: u8 = 7;

#[repr(C)]
struct FT_FaceRec {
    num_faces: c_long,
    face_index: c_long,
    face_flags: c_long,
    style_flags: c_long,
    num_glyphs: c_long,
    family_name: *const c_char,
    style_name: *const c_char,
    num_fixed_sizes: c_int,
    available_sizes: *const FT_Bitmap_Size,
    num_charmaps: c_int,
    charmaps: *const *const c_void,
    generic: [usize; 2],
    bbox: [c_long; 4],
    units_per_em: u16,
    ascender: i16,
    descender: i16,
    height: i16,
    max_advance_width: i16,
    max_advance_height: i16,
    underline_position: i16,
    underline_thickness: i16,
    glyph: *mut FT_GlyphSlotRec,
    size: *mut FT_SizeRec,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct FT_Bitmap_Size {
    height: i16,
    width: i16,
    size: c_long,
    x_ppem: c_long,
    y_ppem: c_long,
}

#[repr(C)]
struct FT_SizeRec {
    face: *mut FT_FaceRec,
    generic: [usize; 2],
    metrics: FT_Size_Metrics,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct FT_Size_Metrics {
    x_ppem: u16,
    y_ppem: u16,
    x_scale: c_long,
    y_scale: c_long,
    ascender: c_long,
    descender: c_long,
    height: c_long,
    max_advance: c_long,
}

#[repr(C)]
struct FT_GlyphSlotRec {
    library: *mut c_void,
    face: *mut FT_FaceRec,
    next: *mut FT_GlyphSlotRec,
    glyph_index: c_uint,
    generic: [usize; 2],
    metrics: [c_long; 8],
    linear_hori_advance: c_long,
    linear_vert_advance: c_long,
    advance_x: c_long,
    advance_y: c_long,
    format: c_uint,
    bitmap: FT_Bitmap,
    bitmap_left: c_int,
    bitmap_top: c_int,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct FT_Bitmap {
    rows: c_uint,
    width: c_uint,
    pitch: c_int,
    buffer: *const u8,
    num_grays: u16,
    pixel_mode: u8,
    palette_mode: u8,
    palette: *mut c_void,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct hb_feature_t {
    tag: u32,
    value: u32,
    start: c_uint,
    end: c_uint,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct hb_glyph_info_t {
    codepoint: u32,
    mask: u32,
    cluster: u32,
    var1: u32,
    var2: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct hb_glyph_position_t {
    x_advance: i32,
    y_advance: i32,
    x_offset: i32,
    y_offset: i32,
    var: u32,
}

struct FtApi {
    init: unsafe extern "C" fn(*mut *mut c_void) -> c_int,
    done_lib: unsafe extern "C" fn(*mut c_void) -> c_int,
    new_face: unsafe extern "C" fn(*mut c_void, *const c_char, c_long, *mut *mut FT_FaceRec) -> c_int,
    done_face: unsafe extern "C" fn(*mut FT_FaceRec) -> c_int,
    set_pixel_sizes: unsafe extern "C" fn(*mut FT_FaceRec, c_uint, c_uint) -> c_int,
    select_size: unsafe extern "C" fn(*mut FT_FaceRec, c_int) -> c_int,
    get_char_index: unsafe extern "C" fn(*mut FT_FaceRec, c_ulong) -> c_uint,
    load_glyph: unsafe extern "C" fn(*mut FT_FaceRec, c_uint, i32) -> c_int,
}

struct HbApi {
    blob_create_from_file: unsafe extern "C" fn(*const c_char) -> *mut c_void,
    blob_destroy: unsafe extern "C" fn(*mut c_void),
    face_create: unsafe extern "C" fn(*mut c_void, c_uint) -> *mut c_void,
    face_destroy: unsafe extern "C" fn(*mut c_void),
    face_get_upem: unsafe extern "C" fn(*mut c_void) -> c_uint,
    font_create: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    font_destroy: unsafe extern "C" fn(*mut c_void),
    buffer_create: unsafe extern "C" fn() -> *mut c_void,
    buffer_destroy: unsafe extern "C" fn(*mut c_void),
    buffer_clear_contents: unsafe extern "C" fn(*mut c_void),
    buffer_add_utf8: unsafe extern "C" fn(*mut c_void, *const c_char, c_int, c_uint, c_int),
    buffer_guess_segment_properties: unsafe extern "C" fn(*mut c_void),
    shape: unsafe extern "C" fn(*mut c_void, *mut c_void, *const hb_feature_t, c_uint),
    buffer_get_glyph_infos: unsafe extern "C" fn(*mut c_void, *mut c_uint) -> *const hb_glyph_info_t,
    buffer_get_glyph_positions: unsafe extern "C" fn(*mut c_void, *mut c_uint) -> *const hb_glyph_position_t,
}

struct FcApi {
    pattern_create: unsafe extern "C" fn() -> *mut c_void,
    pattern_destroy: unsafe extern "C" fn(*mut c_void),
    pattern_add_char_set: unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_void) -> c_int,
    pattern_add_string: unsafe extern "C" fn(*mut c_void, *const c_char, *const u8) -> c_int,
    pattern_get_string: unsafe extern "C" fn(*mut c_void, *const c_char, c_int, *mut *const u8) -> c_int,
    charset_create: unsafe extern "C" fn() -> *mut c_void,
    charset_destroy: unsafe extern "C" fn(*mut c_void),
    charset_add_char: unsafe extern "C" fn(*mut c_void, u32) -> c_int,
    config_substitute: unsafe extern "C" fn(*mut c_void, *mut c_void, c_int) -> c_int,
    default_substitute: unsafe extern "C" fn(*mut c_void),
    font_match: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_int) -> *mut c_void,
}

unsafe fn load_sym<T: Copy>(lib: *mut c_void, name: &[u8]) -> Option<T> {
    let p = unsafe { libc::dlsym(lib, name.as_ptr() as *const c_char) };
    if p.is_null() {
        None
    } else {
        Some(unsafe { core::mem::transmute_copy(&p) })
    }
}

fn ft_api() -> Option<&'static FtApi> {
    static API: OnceLock<Option<FtApi>> = OnceLock::new();
    API.get_or_init(|| unsafe {
        let lib = libc::dlopen(c"libfreetype.so.6".as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
        if lib.is_null() {
            return None;
        }
        Some(FtApi {
            init: load_sym(lib, b"FT_Init_FreeType\0")?,
            done_lib: load_sym(lib, b"FT_Done_FreeType\0")?,
            new_face: load_sym(lib, b"FT_New_Face\0")?,
            done_face: load_sym(lib, b"FT_Done_Face\0")?,
            set_pixel_sizes: load_sym(lib, b"FT_Set_Pixel_Sizes\0")?,
            select_size: load_sym(lib, b"FT_Select_Size\0")?,
            get_char_index: load_sym(lib, b"FT_Get_Char_Index\0")?,
            load_glyph: load_sym(lib, b"FT_Load_Glyph\0")?,
        })
    })
    .as_ref()
}

fn hb_api() -> Option<&'static HbApi> {
    static API: OnceLock<Option<HbApi>> = OnceLock::new();
    API.get_or_init(|| unsafe {
        let lib = libc::dlopen(c"libharfbuzz.so.0".as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
        if lib.is_null() {
            return None;
        }
        Some(HbApi {
            blob_create_from_file: load_sym(lib, b"hb_blob_create_from_file\0")?,
            blob_destroy: load_sym(lib, b"hb_blob_destroy\0")?,
            face_create: load_sym(lib, b"hb_face_create\0")?,
            face_destroy: load_sym(lib, b"hb_face_destroy\0")?,
            face_get_upem: load_sym(lib, b"hb_face_get_upem\0")?,
            font_create: load_sym(lib, b"hb_font_create\0")?,
            font_destroy: load_sym(lib, b"hb_font_destroy\0")?,
            buffer_create: load_sym(lib, b"hb_buffer_create\0")?,
            buffer_destroy: load_sym(lib, b"hb_buffer_destroy\0")?,
            buffer_clear_contents: load_sym(lib, b"hb_buffer_clear_contents\0")?,
            buffer_add_utf8: load_sym(lib, b"hb_buffer_add_utf8\0")?,
            buffer_guess_segment_properties: load_sym(lib, b"hb_buffer_guess_segment_properties\0")?,
            shape: load_sym(lib, b"hb_shape\0")?,
            buffer_get_glyph_infos: load_sym(lib, b"hb_buffer_get_glyph_infos\0")?,
            buffer_get_glyph_positions: load_sym(lib, b"hb_buffer_get_glyph_positions\0")?,
        })
    })
    .as_ref()
}

fn fc_api() -> Option<&'static FcApi> {
    static API: OnceLock<Option<FcApi>> = OnceLock::new();
    API.get_or_init(|| unsafe {
        let lib = libc::dlopen(c"libfontconfig.so.1".as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
        if lib.is_null() {
            return None;
        }
        Some(FcApi {
            pattern_create: load_sym(lib, b"FcPatternCreate\0")?,
            pattern_destroy: load_sym(lib, b"FcPatternDestroy\0")?,
            pattern_add_char_set: load_sym(lib, b"FcPatternAddCharSet\0")?,
            pattern_add_string: load_sym(lib, b"FcPatternAddString\0")?,
            pattern_get_string: load_sym(lib, b"FcPatternGetString\0")?,
            charset_create: load_sym(lib, b"FcCharSetCreate\0")?,
            charset_destroy: load_sym(lib, b"FcCharSetDestroy\0")?,
            charset_add_char: load_sym(lib, b"FcCharSetAddChar\0")?,
            config_substitute: load_sym(lib, b"FcConfigSubstitute\0")?,
            default_substitute: load_sym(lib, b"FcDefaultSubstitute\0")?,
            font_match: load_sym(lib, b"FcFontMatch\0")?,
        })
    })
    .as_ref()
}

struct FtInner {
    lib: *mut c_void,
    face: *mut FT_FaceRec,
    cur_px: u32,
}

struct HbInner {
    face: *mut c_void,
    font: *mut c_void,
    buf: *mut c_void,
    upem: u32,
    features: Vec<hb_feature_t>,
}

/// Native Linux font handle backed by FreeType (`FT_Face`) and HarfBuzz (`hb_font_t`).
pub struct NativeFont {
    ft: Mutex<FtInner>,
    hb: Option<Mutex<HbInner>>,
}

// SAFETY: Every `NativeFont` owns its dedicated `FT_Library`/`FT_Face` and
// `hb_font_t`/`hb_buffer_t` behind a `Mutex`, satisfying FreeType's per-library
// thread safety requirement.
unsafe impl Send for NativeFont {}
unsafe impl Sync for NativeFont {}

impl Drop for NativeFont {
    fn drop(&mut self) {
        unsafe {
            if let Some(hb_mutex) = &self.hb {
                let hb = hb_mutex.lock();
                if let Some(api) = hb_api() {
                    if !hb.buf.is_null() {
                        (api.buffer_destroy)(hb.buf);
                    }
                    if !hb.font.is_null() {
                        (api.font_destroy)(hb.font);
                    }
                    if !hb.face.is_null() {
                        (api.face_destroy)(hb.face);
                    }
                }
            }
            let ft = self.ft.lock();
            if let Some(api) = ft_api() {
                if !ft.face.is_null() {
                    (api.done_face)(ft.face);
                }
                if !ft.lib.is_null() {
                    (api.done_lib)(ft.lib);
                }
            }
        }
    }
}

impl NativeFont {
    unsafe fn ensure_size(ft: &mut FtInner, api: &FtApi, px: f32) {
        let target = px.round().max(1.0) as u32;
        if ft.cur_px != target {
            unsafe {
                if (api.set_pixel_sizes)(ft.face, 0, target) != 0 && (*ft.face).num_fixed_sizes > 0 {
                    let _ = (api.select_size)(ft.face, 0);
                }
            }
            ft.cur_px = target;
        }
    }

    pub fn lookup_glyph_index(&self, ch: char) -> u16 {
        let Some(api) = ft_api() else { return 0 };
        let ft = self.ft.lock();
        unsafe { (api.get_char_index)(ft.face, ch as c_ulong) as u16 }
    }

    pub fn horizontal_line_metrics(&self, px: f32) -> Option<LineMetrics> {
        let api = ft_api()?;
        let mut ft = self.ft.lock();
        unsafe {
            Self::ensure_size(&mut ft, api, px);
            let upem = (*ft.face).units_per_em;
            if upem == 0 {
                return None;
            }
            let scale = px / upem as f32;
            let ascent = (*ft.face).ascender as f32 * scale;
            let descent = (*ft.face).descender as f32 * scale;
            let height = (*ft.face).height as f32 * scale;
            let line_gap = (height - (ascent - descent)).max(0.0);
            if ascent <= 0.0 && descent >= 0.0 {
                return None;
            }
            Some(LineMetrics { ascent, descent, line_gap, new_line_size: ascent - descent + line_gap })
        }
    }

    pub fn advance_width(&self, ch: char, px: f32) -> f32 {
        let Some(api) = ft_api() else { return px * 0.6 };
        let mut ft = self.ft.lock();
        unsafe {
            Self::ensure_size(&mut ft, api, px);
            let gid = (api.get_char_index)(ft.face, ch as c_ulong);
            if gid == 0 || (api.load_glyph)(ft.face, gid, FT_LOAD_DEFAULT) != 0 {
                return px * 0.6;
            }
            let slot = (*ft.face).glyph;
            ((*slot).advance_x as f32) / 64.0
        }
    }

    pub(super) fn glyph_bounds(&self, gid: u16, px: f32) -> (i32, i32, u32, u32) {
        let Some(api) = ft_api() else { return (0, 0, 0, 0) };
        let mut ft = self.ft.lock();
        unsafe {
            Self::ensure_size(&mut ft, api, px);
            if (api.load_glyph)(ft.face, gid as c_uint, FT_LOAD_RENDER) != 0 {
                return (0, 0, 0, 0);
            }
            let slot = (*ft.face).glyph;
            let w = (*slot).bitmap.width;
            let h = (*slot).bitmap.rows;
            let xmin = (*slot).bitmap_left;
            let ymin = (*slot).bitmap_top - h as i32;
            (xmin, ymin, w, h)
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn rasterize_char(
        &self,
        scratch: &mut [u8],
        cw: i32,
        chh: i32,
        baseline: i32,
        mut px: f32,
        ch: char,
        center_if_symbol: bool,
    ) {
        let Some(api) = ft_api() else { return };
        let mut ft = self.ft.lock();
        unsafe {
            let gid = (api.get_char_index)(ft.face, ch as c_ulong);
            if gid == 0 {
                return;
            }
            Self::ensure_size(&mut ft, api, px);
            if (api.load_glyph)(ft.face, gid, FT_LOAD_RENDER) != 0 {
                return;
            }
            if center_if_symbol {
                let slot = (*ft.face).glyph;
                let bw = (*slot).bitmap.width as i32;
                let bh = (*slot).bitmap.rows as i32;
                if bw > cw || bh > chh {
                    let sx = cw as f32 / bw.max(1) as f32;
                    let sy = chh as f32 / bh.max(1) as f32;
                    px = (px * sx.min(sy) * 0.92).max(6.0);
                    Self::ensure_size(&mut ft, api, px);
                    if (api.load_glyph)(ft.face, gid, FT_LOAD_RENDER) != 0 {
                        return;
                    }
                }
            }
            let slot = (*ft.face).glyph;
            let bw = (*slot).bitmap.width as i32;
            let bh = (*slot).bitmap.rows as i32;
            if bw == 0 || bh == 0 || (*slot).bitmap.buffer.is_null() {
                return;
            }
            let mut x0 = if center_if_symbol { (cw - bw) / 2 } else { (*slot).bitmap_left };
            let mut y0 = baseline - (*slot).bitmap_top;
            if bw <= cw {
                x0 = x0.clamp(0, cw - bw);
            }
            if bh <= chh {
                y0 = y0.clamp(0, chh - bh);
            }
            blit_ft_bitmap(&(*slot).bitmap, scratch, cw, chh, x0, y0);
        }
    }

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
        let Some(api) = ft_api() else { return };
        let mut ft = self.ft.lock();
        unsafe {
            Self::ensure_size(&mut ft, api, px);
            if (api.load_glyph)(ft.face, gid as c_uint, FT_LOAD_RENDER) != 0 {
                return;
            }
            let slot = (*ft.face).glyph;
            if (*slot).bitmap.buffer.is_null() {
                return;
            }
            let x0 = pen_x.round() as i32 + (*slot).bitmap_left;
            let y0 = baseline - (*slot).bitmap_top;
            blit_ft_bitmap(&(*slot).bitmap, canvas, w as i32, h as i32, x0, y0);
        }
    }

    pub(super) fn shape_ascii_run(&self, px: f32, run: &[crate::terminal::grid::Cell], out: &mut Vec<ShapedGlyph>) -> bool {
        out.clear();
        let (Some(hb_mutex), Some(api)) = (&self.hb, hb_api()) else { return false };
        let hb = hb_mutex.lock();
        let mut bytes = [0u8; 256];
        let n = run.len().min(bytes.len());
        for (idx, c) in run[..n].iter().enumerate() {
            bytes[idx] = c.codepoint as u8;
        }
        unsafe {
            (api.buffer_clear_contents)(hb.buf);
            (api.buffer_add_utf8)(hb.buf, bytes.as_ptr() as *const c_char, n as c_int, 0, n as c_int);
            (api.buffer_guess_segment_properties)(hb.buf);
            (api.shape)(hb.font, hb.buf, hb.features.as_ptr(), hb.features.len() as c_uint);
            let mut len = 0u32;
            let infos = (api.buffer_get_glyph_infos)(hb.buf, &mut len);
            let mut pos_len = 0u32;
            let positions = (api.buffer_get_glyph_positions)(hb.buf, &mut pos_len);
            if infos.is_null() || positions.is_null() {
                return false;
            }
            let count = len.min(pos_len) as usize;
            let scale = px / hb.upem.max(1) as f32;
            for k in 0..count {
                let info = *infos.add(k);
                let pos = *positions.add(k);
                if let Ok(gid) = u16::try_from(info.codepoint) {
                    out.push(ShapedGlyph {
                        gid,
                        cluster: (info.cluster as usize).min(n.saturating_sub(1)),
                        x_offset_px: pos.x_offset as f32 * scale,
                        y_offset_px: pos.y_offset as f32 * scale,
                    });
                }
            }
            true
        }
    }

    pub(super) fn rasterize_cluster_primary(
        &self,
        buf: &mut [u8],
        w: u32,
        h: u32,
        baseline: i32,
        px: f32,
        text: &str,
    ) -> bool {
        let (Some(hb_mutex), Some(api)) = (&self.hb, hb_api()) else { return false };
        let mut shaped_list: Vec<(u16, f32, f32, f32)> = Vec::new();
        let mut total_advance = 0.0f32;
        {
            let hb = hb_mutex.lock();
            unsafe {
                (api.buffer_clear_contents)(hb.buf);
                (api.buffer_add_utf8)(
                    hb.buf,
                    text.as_ptr() as *const c_char,
                    text.len() as c_int,
                    0,
                    text.len() as c_int,
                );
                (api.buffer_guess_segment_properties)(hb.buf);
                (api.shape)(hb.font, hb.buf, ptr::null(), 0);
                let mut len = 0u32;
                let infos = (api.buffer_get_glyph_infos)(hb.buf, &mut len);
                let mut pos_len = 0u32;
                let positions = (api.buffer_get_glyph_positions)(hb.buf, &mut pos_len);
                let count = len.min(pos_len) as usize;
                if count == 0 || infos.is_null() || positions.is_null() {
                    return false;
                }
                let scale = px / hb.upem.max(1) as f32;
                for k in 0..count {
                    let info = *infos.add(k);
                    if info.codepoint == 0 {
                        return false;
                    }
                    let pos = *positions.add(k);
                    let x_adv = pos.x_advance as f32 * scale;
                    total_advance += x_adv;
                    shaped_list.push((
                        info.codepoint as u16,
                        pos.x_offset as f32 * scale,
                        pos.y_offset as f32 * scale,
                        x_adv,
                    ));
                }
            }
        }
        let mut pen = ((w as f32 - total_advance) * 0.5).max(0.0);
        for (gid, x_off, y_off, x_adv) in shaped_list {
            self.draw_indexed_glyph(
                buf,
                w as usize,
                h as usize,
                pen + x_off,
                baseline - y_off.round() as i32,
                px,
                gid,
            );
            pen += x_adv;
        }
        buf.iter().any(|&v| v != 0)
    }
}

unsafe fn blit_ft_bitmap(bmp: &FT_Bitmap, dst: &mut [u8], cw: i32, chh: i32, x0: i32, y0: i32) {
    if bmp.pixel_mode != FT_PIXEL_MODE_GRAY || bmp.buffer.is_null() {
        return;
    }
    let bw = bmp.width as i32;
    let bh = bmp.rows as i32;
    let pitch = bmp.pitch;
    for gy in 0..bh {
        let y = y0 + gy;
        if y < 0 || y >= chh {
            continue;
        }
        let row_ptr = unsafe {
            if pitch >= 0 {
                bmp.buffer.add((gy * pitch) as usize)
            } else {
                bmp.buffer.add(((bh - 1 - gy) * (-pitch)) as usize)
            }
        };
        for gx in 0..bw {
            let x = x0 + gx;
            if x < 0 || x >= cw {
                continue;
            }
            let cov = unsafe { *row_ptr.add(gx as usize) };
            let v = super::raster::stem_boost(cov);
            let d = &mut dst[(y * cw + x) as usize];
            *d = (*d).max(v);
        }
    }
}

pub(super) fn parse_font(path: &str, collection_index: u32, substitutions: bool) -> Option<NativeFont> {
    let ft = ft_api()?;
    let c_path = CString::new(path).ok()?;
    unsafe {
        let mut lib: *mut c_void = ptr::null_mut();
        if (ft.init)(&mut lib) != 0 || lib.is_null() {
            return None;
        }
        let mut face: *mut FT_FaceRec = ptr::null_mut();
        if (ft.new_face)(lib, c_path.as_ptr(), collection_index as c_long, &mut face) != 0 || face.is_null() {
            return None;
        }

        let hb = if substitutions && let Some(hb_api) = hb_api() {
            let blob = (hb_api.blob_create_from_file)(c_path.as_ptr());
            if !blob.is_null() {
                let hb_face = (hb_api.face_create)(blob, collection_index as c_uint);
                (hb_api.blob_destroy)(blob);
                let upem = (hb_api.face_get_upem)(hb_face).max(1);
                let hb_font = (hb_api.font_create)(hb_face);
                let hb_buf = (hb_api.buffer_create)();
                let feats: Vec<hb_feature_t> = FONT_FEATURES
                    .lock()
                    .iter()
                    .map(|f: &FontFeature| hb_feature_t {
                        tag: u32::from_be_bytes(f.tag),
                        value: f.value,
                        start: 0,
                        end: c_uint::MAX,
                    })
                    .collect();
                Some(Mutex::new(HbInner { face: hb_face, font: hb_font, buf: hb_buf, upem, features: feats }))
            } else {
                None
            }
        } else {
            None
        };

        Some(NativeFont { ft: Mutex::new(FtInner { lib, face, cur_px: 0 }), hb })
    }
}

pub(super) fn line_metrics(font: &NativeFont, px: f32) -> LineMetrics {
    font.horizontal_line_metrics(px).unwrap_or_else(|| synthesized_line_metrics(px))
}

/// System fallback via Fontconfig (`libfontconfig.so.1`) + FreeType when a
/// codepoint is missing from the configured primary and fallback fonts.
pub(super) fn macos_coretext_rasterize(
    buf: &mut [u8],
    cw: u32,
    ch_px: u32,
    baseline: i32,
    px: f32,
    ch: char,
) -> bool {
    let Some(fc) = fc_api() else { return false };
    unsafe {
        let pat = (fc.pattern_create)();
        if pat.is_null() {
            return false;
        }
        let cs = (fc.charset_create)();
        if cs.is_null() {
            (fc.pattern_destroy)(pat);
            return false;
        }
        (fc.charset_add_char)(cs, ch as u32);
        (fc.pattern_add_char_set)(pat, c"charset".as_ptr(), cs);
        let _ = (fc.pattern_add_string)(pat, c"family".as_ptr(), c"monospace".as_ptr().cast());
        (fc.config_substitute)(ptr::null_mut(), pat, 0);
        (fc.default_substitute)(pat);
        let mut res = 0;
        let matched = (fc.font_match)(ptr::null_mut(), pat, &mut res);
        (fc.charset_destroy)(cs);
        (fc.pattern_destroy)(pat);
        if matched.is_null() {
            return false;
        }
        let mut file_ptr: *const u8 = ptr::null();
        let ok = (fc.pattern_get_string)(matched, c"file".as_ptr(), 0, &mut file_ptr) == 0 && !file_ptr.is_null();
        let path_opt = if ok {
            std::ffi::CStr::from_ptr(file_ptr as *const c_char).to_str().ok().map(String::from)
        } else {
            None
        };
        (fc.pattern_destroy)(matched);
        let Some(path) = path_opt else { return false };
        let Some(font) = parse_font(&path, 0, false) else { return false };
        if font.lookup_glyph_index(ch) == 0 {
            return false;
        }
        font.rasterize_char(buf, cw as i32, ch_px as i32, baseline, px, ch, true);
        buf.iter().any(|&v| v != 0)
    }
}

pub(super) fn rasterize_text_cluster(_buf: &mut [u8], _w: u32, _h: u32, _baseline: i32, _px: f32, _text: &str) -> bool {
    false
}

/// Bitmap color emoji (`FT_LOAD_COLOR` BGRA strikes from Noto Color Emoji via FreeType + HarfBuzz).
pub(super) fn rasterize_color(buf: &mut [u8], w: u32, h: u32, px: f32, text: &str) -> bool {
    static EMOJI_FONT: OnceLock<Option<NativeFont>> = OnceLock::new();
    let Some(font) = EMOJI_FONT
        .get_or_init(|| {
            let face = crate::render::fontdb::find_family("Noto Color Emoji")?;
            parse_font(&face.path, face.index, true)
        })
        .as_ref()
    else {
        return false;
    };
    let Some(ft_api) = ft_api() else { return false };
    let gid = if let (Some(hb_mutex), Some(hb_api)) = (&font.hb, hb_api()) {
        let hb = hb_mutex.lock();
        unsafe {
            (hb_api.buffer_clear_contents)(hb.buf);
            (hb_api.buffer_add_utf8)(
                hb.buf,
                text.as_ptr() as *const c_char,
                text.len() as c_int,
                0,
                text.len() as c_int,
            );
            (hb_api.buffer_guess_segment_properties)(hb.buf);
            (hb_api.shape)(hb.font, hb.buf, ptr::null(), 0);
            let mut len = 0u32;
            let infos = (hb_api.buffer_get_glyph_infos)(hb.buf, &mut len);
            if len > 0 && !infos.is_null() { (*infos).codepoint } else { 0 }
        }
    } else {
        text.chars().next().map_or(0, |ch| font.lookup_glyph_index(ch) as u32)
    };
    if gid == 0 {
        return false;
    }
    let mut ft = font.ft.lock();
    unsafe {
        NativeFont::ensure_size(&mut ft, ft_api, px);
        if (ft_api.load_glyph)(ft.face, gid as c_uint, FT_LOAD_RENDER | FT_LOAD_COLOR) != 0 {
            return false;
        }
        let slot = (*ft.face).glyph;
        let bmp = &(*slot).bitmap;
        if bmp.pixel_mode != FT_PIXEL_MODE_BGRA || bmp.width == 0 || bmp.rows == 0 || bmp.buffer.is_null() {
            return false;
        }
        let (sw, sh) = (bmp.width, bmp.rows);
        let pitch = bmp.pitch.unsigned_abs() as usize;
        let mut rgba = vec![0u8; (sw * sh * 4) as usize];
        for y in 0..sh as usize {
            let src_row = core::slice::from_raw_parts(bmp.buffer.add(y * pitch), (sw as usize) * 4);
            let dst_row = &mut rgba[y * (sw as usize) * 4..(y + 1) * (sw as usize) * 4];
            for (d, s) in dst_row.as_chunks_mut::<4>().0.iter_mut().zip(src_row.as_chunks::<4>().0) {
                let a = s[3] as u32;
                if a > 0 && a < 255 {
                    d[0] = ((s[2] as u32 * 255 + a / 2) / a).min(255) as u8;
                    d[1] = ((s[1] as u32 * 255 + a / 2) / a).min(255) as u8;
                    d[2] = ((s[0] as u32 * 255 + a / 2) / a).min(255) as u8;
                    d[3] = s[3];
                } else {
                    *d = [s[2], s[1], s[0], s[3]];
                }
            }
        }
        blit_fit(&rgba, sw, sh, buf, w, h);
        true
    }
}
