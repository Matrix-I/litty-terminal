//! IME composition (Telex, Pinyin, ...) drawn underlined at the cursor.

use super::super::Instance;
use super::{Cells, DirtyRows};
use crate::render::atlas::GlyphStyle;
use crate::terminal::grid::{char_width, BG_UNDERLINE};

/// Draws `preedit` at `cursor` and returns the cursor moved to the end of the composition.
pub(in crate::render::renderer) fn build(
    cells: &mut Cells,
    preedit: &str,
    cursor: (usize, usize),
    dirty: &mut DirtyRows,
) -> (usize, usize) {
    let cols = cells.cols;
    let (cx, cy) = cursor;
    let row_start = cy * cols;
    let mut col = cx;
    for ch in preedit.chars() {
        let w = char_width(ch);
        if w == 0 {
            continue;
        }
        if col + w > cols {
            break;
        }
        let (left, right) = cells.atlas.glyph_slots(cells.queue, ch as u32, w, GlyphStyle::default());
        let idx = row_start + col;
        if idx < cells.instances.len() {
            cells.instances[idx] = Instance { glyph: left as u32, fg: 0, bg: BG_UNDERLINE };
        }
        if w == 2 && idx + 1 < cells.instances.len() {
            cells.instances[idx + 1] = Instance { glyph: right as u32, fg: 0, bg: BG_UNDERLINE };
        }
        col += w;
    }
    dirty.mark(cy, cy);
    (col.min(cols.saturating_sub(1)), cy)
}
