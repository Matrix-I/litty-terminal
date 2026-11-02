//! Non-macOS counterparts of `coretext.rs`. There is no system glyph fallback
//! (no fontconfig / FreeType); color emoji come from bitmap strikes of
//! Noto Color Emoji found through `crate::fontdb`.

use std::sync::Arc;

use super::blit_fit;
use super::fonts::shaper_mmap_for;

/// System font fallback is CoreText-only; elsewhere the fontconfig-free fallback
/// chain above is all there is.
pub(super) fn macos_coretext_rasterize(_buf: &mut [u8], _cw: u32, _ch_px: u32, _baseline: i32, _px: f32, _ch: char) -> bool {
    false
}

pub(super) fn rasterize_text_cluster(_buf: &mut [u8], _w: u32, _h: u32, _baseline: i32, _px: f32, _text: &str) -> bool {
    false
}

/// Bitmap color emoji (CBDT / sbix PNG strikes, e.g. Noto Color Emoji) on
/// other systems; vector-only (COLR) emoji fonts fall back to monochrome.
pub(super) fn rasterize_color(buf: &mut [u8], w: u32, h: u32, _px: f32, text: &str) -> bool {
    use rustybuzz::ttf_parser::{GlyphId, RasterImageFormat};
    use std::sync::OnceLock;

    static FACE: OnceLock<Option<(Arc<memmap2::Mmap>, u32)>> = OnceLock::new();
    let pair = FACE.get_or_init(|| {
        let face = crate::fontdb::find_family("Noto Color Emoji")?;
        shaper_mmap_for(&face.path, face.index)
    });
    let Some((mmap, index)) = pair.as_ref() else { return false };
    let Some(face) = rustybuzz::Face::from_slice(mmap, *index) else { return false };
    // Shaped, so ZWJ / flag / skin tone sequences become their ligature glyph.
    let mut ub = rustybuzz::UnicodeBuffer::new();
    ub.push_str(text);
    let shaped = rustybuzz::shape(&face, &[], ub);
    let Some(gid) = shaped.glyph_infos().first().map(|g| g.glyph_id).filter(|&g| g != 0) else { return false };
    let Some(img) = face.glyph_raster_image(GlyphId(gid as u16), h.min(u16::MAX as u32) as u16) else { return false };
    if img.format != RasterImageFormat::PNG {
        return false;
    }
    let Ok((sw, sh, rgba)) = crate::graphics::decode_png(img.data) else { return false };
    blit_fit(&rgba, sw, sh, buf, w, h);
    true
}
