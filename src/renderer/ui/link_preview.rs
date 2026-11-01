//! URL of the hovered link on the window's last row, blended with the default background.

use super::super::Instance;
use super::{Cells, DirtyRows};
use crate::grid::Hyperlink;

pub(in crate::renderer) fn build(cells: &mut Cells, total_rows: usize, link: &Hyperlink, dirty: &mut DirtyRows) {
    let cols = cells.cols;
    let max_chars = cols.saturating_sub(2).min(120);
    let display_url = if link.url.chars().count() > max_chars {
        format!("{}...", link.url.chars().take(max_chars.saturating_sub(3)).collect::<String>())
    } else {
        link.url.clone()
    };
    let preview_row = total_rows - 1;
    let row_start = preview_row * cols;
    // Naturally blend with terminal background without any background box or borders:
    // bit 4 is set so is_preview=true (suppresses cursor), while bit 7 is 0 so background matches default_bg.
    const PREVIEW_BG: u32 = 16;
    const PREVIEW_FG: u32 = 0xF2F2_F780;
    for (col, ch) in display_url.chars().enumerate() {
        if col < cols && row_start + col < cells.instances.len() {
            let slot = cells.atlas.slot(cells.queue, ch as u32);
            cells.instances[row_start + col] = Instance {
                glyph: slot as u32,
                fg: PREVIEW_FG,
                bg: PREVIEW_BG,
            };
        }
    }
    let end_col = display_url.chars().count();
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
