//! URL of the hovered link on the window's last row, blended with the default background.

use super::super::Instance;
use super::{Cells, DirtyRows, prefix};
use crate::terminal::grid::Hyperlink;

pub(in crate::render::renderer) fn build(cells: &mut Cells, total_rows: usize, link: &Hyperlink, dirty: &mut DirtyRows) {
    let cols = cells.cols;
    let max_chars = cols.saturating_sub(2).min(120);
    let url = link.url.as_str();
    let count = url.chars().count();
    let truncated = count > max_chars;
    let head = if truncated { prefix(url, max_chars.saturating_sub(3)) } else { url };
    let preview_row = total_rows - 1;
    let row_start = preview_row * cols;
    // Naturally blend with terminal background without any background box or borders:
    // bit 4 is set so is_preview=true (suppresses cursor), while bit 7 is 0 so background matches default_bg.
    const PREVIEW_BG: u32 = 16;
    const PREVIEW_FG: u32 = 0xF2F2_F780;
    let tail = if truncated { "..." } else { "" };
    let mut end_col = 0;
    for (col, ch) in head.chars().chain(tail.chars()).enumerate() {
        if col < cols && row_start + col < cells.instances.len() {
            let slot = cells.atlas.slot(cells.queue, ch as u32);
            cells.instances[row_start + col] = Instance {
                glyph: slot as u32,
                fg: PREVIEW_FG,
                bg: PREVIEW_BG,
            };
        }
        end_col = col + 1;
    }
    for col in end_col..cols {
        if row_start + col < cells.instances.len() {
            cells.instances[row_start + col] = Instance {
                glyph: 0,
                fg: PREVIEW_FG,
                bg: PREVIEW_BG,
            };
        }
    }
    dirty.mark(preview_row, preview_row);
}
