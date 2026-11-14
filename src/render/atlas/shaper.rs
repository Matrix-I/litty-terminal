//! Ligature and `font-feature` shaping via the platform's native shaping engine
//! (CoreText on macOS, HarfBuzz on Linux): runs of printable ASCII that contain
//! punctuation become interned multi-cell glyph pieces.

use std::collections::HashMap;

use parking_lot::Mutex;

use super::fonts::FONT_CACHE;
use super::run_cache::RunCache;
use super::{FontSet, GLYPH_PIECE, SHAPED_BLANK};

/// An OpenType feature tag (`calt`, `liga`, `ss01`, ...) and value (`0` = off, `1` = on).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct FontFeature {
    pub tag: [u8; 4],
    pub value: u32,
}

/// Output of shaping one glyph in an ASCII run.
#[derive(Copy, Clone, Debug)]
pub(super) struct ShapedGlyph {
    pub(super) gid: u16,
    pub(super) cluster: usize,
    pub(super) x_offset_px: f32,
    pub(super) y_offset_px: f32,
}

/// A glyph by index, drawn across `span` cells with its pen at cell `origin`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct GlyphPiece {
    pub(super) gid: u16,
    pub(super) span: u8,
    pub(super) origin: u8,
    pub(super) bold: bool,
    pub(super) italic: bool,
}

/// Ligature shaping state: reusable shaped-glyph buffer, interned glyph pieces
/// and the outputs of recently shaped runs.
pub(super) struct Shaper {
    shaped_buf: Vec<ShapedGlyph>,
    pub(super) pieces: Vec<GlyphPiece>,
    pub(super) piece_ids: HashMap<GlyphPiece, u32>,
    runs: RunCache,
    /// Calls into the platform shaper, so tests can tell cache hits from misses.
    #[cfg(test)]
    pub(super) platform_shapes: u64,
}

/// Interned pieces before their ids start over (they are 22-bit slot keys).
const MAX_PIECES: usize = 4096;

impl Shaper {
    pub(super) fn new(_features: Vec<FontFeature>) -> Self {
        Shaper {
            shaped_buf: Vec::with_capacity(64),
            pieces: Vec::new(),
            piece_ids: HashMap::new(),
            runs: RunCache::new(),
            #[cfg(test)]
            platform_shapes: 0,
        }
    }

    /// Shape the ligature candidates of a row: runs of printable ASCII in one
    /// style that contain punctuation (where coding-font ligatures live).
    /// `out[col]` becomes a slot key for a substituted glyph, `SHAPED_BLANK`
    /// for an empty spacer, or 0 for "draw the codepoint as usual". Returns
    /// `(changed, reset_pieces)`. Allocation-free once the buffers are warm.
    pub fn shape_row(
        &mut self,
        fonts: &FontSet,
        px: f32,
        cell_w: u32,
        row: &[crate::terminal::grid::Cell],
        out: &mut Vec<u32>,
    ) -> (bool, bool) {
        let mut reset_pieces = false;
        if self.pieces.len() >= MAX_PIECES {
            self.pieces.clear();
            self.piece_ids.clear();
            // Cached outputs name the old piece ids.
            self.runs.clear();
            reset_pieces = true;
        }
        self.runs.set_shaped_at(px, cell_w);
        out.clear();
        out.resize(row.len(), 0);
        if cell_w == 0 {
            return (false, reset_pieces);
        }
        let style_of = |c: &crate::terminal::grid::Cell| {
            (c.fg & crate::terminal::grid::FG_BOLD != 0, c.fg & crate::terminal::grid::FG_ITALIC != 0)
        };
        let shapeable = |c: &crate::terminal::grid::Cell| (0x21..0x7F).contains(&c.codepoint);
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
            let style_byte = u8::from(style.0) | u8::from(style.1) << 1;
            let hash = self.runs.load(style_byte, run.iter().map(|c| c.codepoint as u8));
            let run_out = &mut out[start..i];
            if !self.runs.get_into(hash, run_out) {
                self.shape_run(fonts, px, cell_w, run, style, run_out);
                self.runs.insert(hash, run_out);
            }
            changed |= run_out.iter().any(|&v| v != 0);
        }
        (changed, reset_pieces)
    }

    /// Shape one run with the platform shaper into `out` (one word per cell, all 0 on entry).
    fn shape_run(
        &mut self,
        fonts: &FontSet,
        px: f32,
        cell_w: u32,
        run: &[crate::terminal::grid::Cell],
        style: (bool, bool),
        out: &mut [u32],
    ) {
        #[cfg(test)]
        {
            self.platform_shapes += 1;
        }
        if !fonts.primary.shape_ascii_run(px, run, &mut self.shaped_buf) {
            return;
        }
        let n_shaped = self.shaped_buf.len();
        for idx in 0..n_shaped {
            let sg = self.shaped_buf[idx];
            let cluster_end = (idx + 1..n_shaped)
                .map(|j| self.shaped_buf[j].cluster)
                .find(|&c| c > sg.cluster)
                .unwrap_or(run.len())
                .min(run.len());
            let cluster_len = cluster_end.saturating_sub(sg.cluster).max(1);
            let Some(cell) = run.get(sg.cluster) else { continue };
            let ch = char::from_u32(cell.codepoint).unwrap_or(' ');
            let nominal = fonts.primary.lookup_glyph_index(ch);
            if sg.gid == nominal
                && cluster_len == 1
                && sg.x_offset_px.abs() < 0.1
                && sg.y_offset_px.abs() < 0.1
            {
                continue;
            }
            let (xmin, _ymin, bw, bh) = fonts.primary.glyph_bounds(sg.gid, px);
            if bw == 0 || bh == 0 {
                for slot in &mut out[sg.cluster..cluster_end] {
                    if *slot == 0 {
                        *slot = SHAPED_BLANK;
                    }
                }
                continue;
            }
            let cw = cell_w as i32;
            let rel = if xmin < 0 && cluster_len > 1 {
                cluster_end - 1
            } else {
                sg.cluster
            };
            let x0 = xmin + sg.x_offset_px.round() as i32;
            let first = (rel as i32 + x0.div_euclid(cw)).min(sg.cluster as i32);
            let last = (rel as i32 + (x0 + bw as i32 - 1).div_euclid(cw)).max(cluster_end as i32 - 1);
            let (first, last) = (first.max(0), last.min(run.len() as i32 - 1));
            let (span, origin) = (last - first + 1, rel as i32 - first);
            if !(1..=31).contains(&span) || !(0..32).contains(&origin) {
                continue;
            }
            let piece = GlyphPiece {
                gid: sg.gid,
                span: span as u8,
                origin: origin as u8,
                bold: style.0,
                italic: style.1,
            };
            let next = self.pieces.len() as u32;
            let id = *self.piece_ids.entry(piece).or_insert(next);
            if id == next {
                self.pieces.push(piece);
            }
            for k in 0..span {
                out[(first + k) as usize] = GLYPH_PIECE | (id << 5) | k as u32;
            }
        }
    }

    #[cfg(test)]
    pub(super) fn cached_runs(&self) -> usize {
        self.runs.len()
    }
}

/// `font-feature` values (`-calt`, `ss01`, `liga=0`), read when an atlas is built.
pub(super) static FONT_FEATURES: Mutex<Vec<FontFeature>> = Mutex::new(Vec::new());

/// Parse `font-feature` values (`-calt`, `+calt`, `calt`, `ss01`, `liga=0`, `calt=on`);
/// unknown syntax is an error.
pub fn parse_font_feature(v: &str) -> Result<FontFeature, String> {
    let orig = v;
    let s = v.trim().trim_matches(|c| c == '"' || c == '\'');
    if s.is_empty() {
        return Err(format!("invalid font feature `{orig}`"));
    }
    let (prefix_val, rest) = if let Some(r) = s.strip_prefix('-') {
        (Some(0u32), r.trim())
    } else if let Some(r) = s.strip_prefix('+') {
        (Some(1u32), r.trim())
    } else {
        (None, s)
    };
    let (tag_part, explicit_val) = if let Some((l, r)) = rest.split_once('=') {
        (l.trim(), Some(r.trim()))
    } else if let Some((l, r)) = rest.split_once(char::is_whitespace) {
        (l.trim(), Some(r.trim()))
    } else {
        (rest, None)
    };
    if prefix_val.is_some() && explicit_val.is_some() {
        return Err(format!("invalid font feature `{orig}`"));
    }
    let tag_str = tag_part.split('[').next().unwrap_or(tag_part).trim();
    if tag_str.is_empty() || tag_str.len() > 4 || !tag_str.bytes().all(|b| (0x21..=0x7E).contains(&b)) {
        return Err(format!("invalid font feature `{orig}`"));
    }
    let mut tag = [b' '; 4];
    tag[..tag_str.len()].copy_from_slice(tag_str.as_bytes());
    let value = match (prefix_val, explicit_val) {
        (Some(v), None) => v,
        (None, None) => 1,
        (None, Some("on" | "true")) => 1,
        (None, Some("off" | "false")) => 0,
        (None, Some(num)) => num.parse::<u32>().map_err(|_| format!("invalid font feature `{orig}`"))?,
        _ => return Err(format!("invalid font feature `{orig}`")),
    };
    Ok(FontFeature { tag, value })
}

pub fn set_font_features(features: &[FontFeature]) {
    let mut feat = FONT_FEATURES.lock();
    if *feat == features {
        return;
    }
    *feat = features.to_vec();
    // `load_font` takes FONT_CACHE then FONT_FEATURES (in `parse_font`):
    // release ours first so the two never wait on each other. A font cached
    // before this point is dropped here; one cached after sees the new features.
    drop(feat);
    *FONT_CACHE.lock() = None;
}
