//! Find box (`Cmd+F` / `Ctrl+Shift+F`) in the top-right corner, below the tab bar.

use super::super::Instance;
use super::{Cells, DirtyRows};
use crate::atlas::GlyphStyle;
use crate::grid::char_width;
use crate::search::SearchBar;

/// Find box in the top-right corner, below the tab bar: `(x0, y0, width)`, 3 rows tall.
pub(in crate::renderer) fn geometry(cols: usize, top_rows: usize, total_rows: usize) -> Option<(usize, usize, usize)> {
    let w = 46.min(cols.saturating_sub(2));
    (w >= 24 && total_rows >= top_rows + 4).then(|| (cols - w - 1, top_rows, w))
}

pub(in crate::renderer) fn build(
    cells: &mut Cells,
    top_rows: usize,
    total_rows: usize,
    search: &SearchBar,
    dirty: &mut DirtyRows,
) {
    let cols = cells.cols;
    let Some((x0, y0, w)) = geometry(cols, top_rows, total_rows) else {
        return;
    };
    const BOX_BG: u32 = 0x1819_2490;
    const BORDER_FG: u32 = 0x585B_7080;
    const TITLE_FG: u32 = 0x89DC_EB80;
    const PROMPT_FG: u32 = 0xA6E3_A180;
    const QUERY_FG: u32 = 0xF2F2_F780;
    const DIM_FG: u32 = 0x6C70_8680;
    const CURSOR_FG: u32 = 0xCBA6_F780;
    const COUNT_FG: u32 = 0xA6AD_C880;
    const ERROR_FG: u32 = 0xF38B_A880;
    const MODE_ON_FG: u32 = 0xCBA6_F780;

    let queue = cells.queue;
    let atlas = &mut *cells.atlas;
    let instances = &mut *cells.instances;
    // Draws `ch` at a box-relative cell (both halves of a wide glyph).
    let mut put = |row: usize, col: usize, ch: char, fg: u32| {
        let width = char_width(ch).max(1);
        let (left, right) = atlas.glyph_slots(queue, ch as u32, width, GlyphStyle::default());
        let i = (y0 + row) * cols + x0 + col;
        instances[i] = Instance { glyph: left as u32, fg, bg: BOX_BG };
        if width == 2 {
            instances[i + 1] = Instance { glyph: right as u32, fg, bg: BOX_BG };
        }
    };

    // Frame: ╭─ Find ───╮ / │ … │ / ╰─ ^R regex ─╯
    for c in 0..w {
        let (top, mid, bot) = match c {
            0 => ('╭', '│', '╰'),
            _ if c == w - 1 => ('╮', '│', '╯'),
            _ => ('─', ' ', '─'),
        };
        put(0, c, top, BORDER_FG);
        put(1, c, mid, BORDER_FG);
        put(2, c, bot, BORDER_FG);
    }
    for (i, ch) in " Find ".chars().enumerate() {
        put(0, 2 + i, ch, TITLE_FG);
    }
    let hint = " ↵ ↑  ⇧↵ ↓  ^R regex  esc ";
    let hint_len = hint.chars().count();
    if hint_len + 3 < w {
        for (i, ch) in hint.chars().enumerate() {
            put(2, w - 2 - hint_len + i, ch, DIM_FG);
        }
    }

    // Right side of the input row: match counter and the `.*` regex badge.
    let (status, status_fg) = if search.invalid {
        ("invalid regex".to_string(), ERROR_FG)
    } else if search.query.is_empty() {
        (String::new(), COUNT_FG)
    } else if search.total == 0 {
        ("no matches".to_string(), ERROR_FG)
    } else {
        let cur = search.current.map_or(0, |c| c + 1);
        let more = if search.total >= crate::search::MAX_MATCHES { "+" } else { "" };
        (format!("{cur}/{}{more}", search.total), COUNT_FG)
    };
    put(1, w - 4, '.', if search.regex { MODE_ON_FG } else { DIM_FG });
    put(1, w - 3, '*', if search.regex { MODE_ON_FG } else { DIM_FG });
    let status_len = status.chars().count();
    let status_x = w - 5 - status_len;
    for (i, ch) in status.chars().enumerate() {
        put(1, status_x + i, ch, status_fg);
    }

    // Query, showing its tail when it does not fit, followed by the cursor.
    put(1, 2, '❯', PROMPT_FG);
    let room = status_x.saturating_sub(6);
    let chars: Vec<char> = search.query.chars().collect();
    let mut col = 4;
    if chars.is_empty() {
        let placeholder = if search.regex { "Search (regex)" } else { "Search (literal)" };
        for (i, ch) in placeholder.chars().take(room).enumerate() {
            put(1, 5 + i, ch, DIM_FG);
        }
    } else {
        let total: usize = chars.iter().map(|&c| char_width(c).max(1)).sum();
        // Keep the tail visible; reserve a cell for the leading ellipsis.
        let budget = if total > room { room.saturating_sub(1) } else { room };
        let (mut start, mut shown) = (chars.len(), 0);
        while start > 0 && shown + char_width(chars[start - 1]).max(1) <= budget {
            start -= 1;
            shown += char_width(chars[start]).max(1);
        }
        if start > 0 {
            put(1, col, '…', DIM_FG);
            col += 1;
        }
        for &ch in &chars[start..] {
            put(1, col, ch, QUERY_FG);
            col += char_width(ch).max(1);
        }
    }
    put(1, col, '▌', CURSOR_FG);

    dirty.mark(y0, y0 + 2);
}

#[cfg(test)]
mod tests {
    use super::geometry;

    #[test]
    fn geometry_needs_room_for_the_box() {
        assert_eq!(geometry(80, 1, 30), Some((33, 1, 46)));
        assert_eq!(geometry(25, 1, 30), None, "narrower than 24 columns");
        assert_eq!(geometry(80, 1, 4), None, "box needs 3 rows below the tab bar");
    }
}
