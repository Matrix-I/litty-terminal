//! Terminal inspector panel (`Opt+Cmd+I` / `Ctrl+Shift+I`) in the bottom-right corner.

use super::super::Instance;
use super::{Cells, DirtyRows};
use crate::atlas::GlyphStyle;
use crate::terminal::grid::char_width;

/// Inspector panel in the bottom-right corner: a frame around `lines`,
/// the first of which is the title.
pub(in crate::renderer) fn build(
    cells: &mut Cells,
    top_rows: usize,
    total_rows: usize,
    lines: &[String],
    dirty: &mut DirtyRows,
) {
    let cols = cells.cols;
    const BOX_BG: u32 = 0x1819_2490;
    const BORDER_FG: u32 = 0x585B_7080;
    const TITLE_FG: u32 = 0x89DC_EB80;
    const LABEL_FG: u32 = 0xA6AD_C880;
    const VALUE_FG: u32 = 0xF2F2_F780;
    const LABEL_COLS: usize = 10;
    let Some((title, body)) = lines.split_first() else { return };
    let width = body.iter().map(|l| l.chars().count()).max().unwrap_or(0).max(title.chars().count() + 4) + 4;
    let w = width.min(cols);
    let h = body.len() + 2;
    if w < 8 || h + top_rows > total_rows {
        return;
    }
    let (x0, y0) = (cols - w, total_rows - h);

    let queue = cells.queue;
    let atlas = &mut *cells.atlas;
    let instances = &mut *cells.instances;
    let mut put = |row: usize, col: usize, ch: char, fg: u32| {
        if col >= w {
            return;
        }
        let width = char_width(ch).max(1);
        if col + width > w {
            return;
        }
        let (left, right) = atlas.glyph_slots(queue, ch as u32, width, GlyphStyle::default());
        let i = (y0 + row) * cols + x0 + col;
        instances[i] = Instance { glyph: left as u32, fg, bg: BOX_BG };
        if width == 2 {
            instances[i + 1] = Instance { glyph: right as u32, fg, bg: BOX_BG };
        }
    };
    for row in 0..h {
        for c in 0..w {
            let ch = match (row, c) {
                (0, 0) => '╭',
                (0, _) if c == w - 1 => '╮',
                (_, 0) if row == h - 1 => '╰',
                (_, _) if row == h - 1 && c == w - 1 => '╯',
                (0, _) => '─',
                (_, _) if row == h - 1 => '─',
                (_, 0) => '│',
                (_, _) if c == w - 1 => '│',
                _ => ' ',
            };
            put(row, c, ch, BORDER_FG);
        }
    }
    for (i, ch) in format!(" {title} ").chars().enumerate() {
        put(0, 2 + i, ch, TITLE_FG);
    }
    for (r, line) in body.iter().enumerate() {
        let mut col = 2;
        for (i, ch) in line.chars().enumerate() {
            if col + 2 >= w {
                break;
            }
            put(r + 1, col, ch, if i < LABEL_COLS { LABEL_FG } else { VALUE_FG });
            col += char_width(ch).max(1);
        }
    }
    dirty.mark(y0, y0 + h - 1);
}
