//! Ligature and `font-feature` shaping with rustybuzz: runs of printable ASCII
//! that contain punctuation become interned multi-cell glyph pieces.

use std::collections::HashMap;

use parking_lot::Mutex;

use super::fonts::FONT_CACHE;
use super::{FontSet, GLYPH_PIECE, SHAPED_BLANK};

/// A glyph by index, drawn across `span` cells with its pen at cell `origin`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct GlyphPiece {
    pub(super) gid: u16,
    pub(super) span: u8,
    pub(super) origin: u8,
    pub(super) bold: bool,
    pub(super) italic: bool,
}

/// Ligature shaping state: features, a reusable buffer, interned glyph pieces.
pub(super) struct Shaper {
    features: Vec<rustybuzz::Feature>,
    buf: Option<rustybuzz::UnicodeBuffer>,
    pub(super) pieces: Vec<GlyphPiece>,
    pub(super) piece_ids: HashMap<GlyphPiece, u32>,
    /// Advance of one cell in font units (0 = not measured yet, -1 = unknown).
    cell_units: i32,
}

impl Shaper {
    pub(super) fn new(features: Vec<rustybuzz::Feature>) -> Self {
        Shaper { features, buf: None, pieces: Vec::new(), piece_ids: HashMap::new(), cell_units: 0 }
    }

    /// Shape the ligature candidates of a row: runs of printable ASCII in one
    /// style that contain punctuation (where coding-font ligatures live).
    /// `out[col]` becomes a slot key for a substituted glyph, `SHAPED_BLANK`
    /// for an empty spacer, or 0 for "draw the codepoint as usual". Returns
    /// `(changed, reset_pieces)`. Allocation-free once the buffers are warm.
    pub fn shape_row(&mut self, fonts: &FontSet, px: f32, cell_w: u32, row: &[crate::grid::Cell], out: &mut Vec<u32>) -> (bool, bool) {
        let mut reset_pieces = false;
        if self.pieces.len() >= 4096 {
            self.pieces.clear();
            self.piece_ids.clear();
            reset_pieces = true;
        }
        out.clear();
        out.resize(row.len(), 0);
        let Some(face) = fonts.shaper() else { return (false, reset_pieces) };
        if self.cell_units == 0 {
            self.cell_units = face.glyph_index('M').and_then(|g| face.glyph_hor_advance(g)).map_or(-1, i32::from);
        }
        if self.cell_units <= 0 {
            return (false, reset_pieces);
        }
        let style_of = |c: &crate::grid::Cell| (c.fg & crate::grid::FG_BOLD != 0, c.fg & crate::grid::FG_ITALIC != 0);
        let shapeable = |c: &crate::grid::Cell| (0x21..0x7F).contains(&c.codepoint);
        let mut changed = false;
        let mut i = 0;
        while i < row.len() {
            if !shapeable(&row[i]) {
                i += 1;
                continue;
            }
            let style = style_of(&row[i]);
            let start = i;
            while i < row.len() && shapeable(&row[i]) && style_of(&row[i]) == style {
                i += 1;
            }
            let run = &row[start..i];
            let punct = run.iter().any(|c| !(c.codepoint as u8).is_ascii_alphanumeric());
            if run.len() < 2 || !punct {
                continue;
            }

            let mut buf = self.buf.take().unwrap_or_default();
            for c in run {
                buf.add(char::from_u32(c.codepoint).unwrap_or(' '), 0);
            }
            buf.set_direction(rustybuzz::Direction::LeftToRight);
            let glyphs = rustybuzz::shape(&face, &self.features, buf);
            let mut pen = 0i32;
            for (info, pos) in glyphs.glyph_infos().iter().zip(glyphs.glyph_positions()) {
                let rel = ((pen + self.cell_units / 2) / self.cell_units) as usize;
                pen += pos.x_advance;
                let (Some(cell), Ok(gid)) = (run.get(rel), u16::try_from(info.glyph_id)) else { continue };
                let ch = char::from_u32(cell.codepoint).unwrap_or(' ');
                let nominal = face.glyph_index(ch).map_or(0, |g| g.0);
                if gid == nominal && pos.x_offset == 0 && pos.y_offset == 0 {
                    continue;
                }
                let m = fonts.primary.metrics_indexed(gid, px);
                if m.width == 0 || m.height == 0 {
                    if out[start + rel] == 0 {
                        out[start + rel] = SHAPED_BLANK;
                        changed = true;
                    }
                    continue;
                }
                // Cells the glyph's ink covers, relative to its pen cell.
                let cw = cell_w as i32;
                let x0 = m.xmin + pos.x_offset * px as i32 / face.units_per_em().max(1);
                let first = rel as i32 + x0.div_euclid(cw);
                let last = rel as i32 + (x0 + m.width as i32 - 1).div_euclid(cw);
                let (first, last) = (first.max(0), last.min(run.len() as i32 - 1));
                let (span, origin) = (last - first + 1, rel as i32 - first);
                if !(1..=31).contains(&span) || !(0..32).contains(&origin) {
                    continue;
                }
                let piece = GlyphPiece { gid, span: span as u8, origin: origin as u8, bold: style.0, italic: style.1 };
                let next = self.pieces.len() as u32;
                let id = *self.piece_ids.entry(piece).or_insert(next);
                if id == next {
                    self.pieces.push(piece);
                }
                for k in 0..span {
                    out[start + (first + k) as usize] = GLYPH_PIECE | (id << 5) | k as u32;
                }
                changed = true;
            }
            self.buf = Some(glyphs.clear());
        }
        (changed, reset_pieces)
    }

}

/// `font-feature` values (`-calt`, `ss01`, `liga=0`), read when an atlas is built.
pub(super) static FONT_FEATURES: Mutex<Vec<rustybuzz::Feature>> = parking_lot::const_mutex(Vec::new());

/// Parse `font-feature` values; unknown syntax is an error.
pub fn parse_font_feature(v: &str) -> Result<rustybuzz::Feature, String> {
    v.trim().parse::<rustybuzz::Feature>().map_err(|_| format!("invalid font feature `{v}`"))
}

pub fn set_font_features(features: &[rustybuzz::Feature]) {
    let mut feat = FONT_FEATURES.lock();
    if *feat != features {
        *feat = features.to_vec();
        *FONT_CACHE.lock() = None;
    }
}
