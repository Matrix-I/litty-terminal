//! History search popup (`Ctrl+R`): query, fzf-style matches, preview, key hints.

use super::super::Instance;
use super::{Cells, DirtyRows};
use crate::terminal::history::HistorySearch;

/// Position and size of the popup `(x0, y0, width, height, visible_rows)` in a
/// `cols × total_rows` window.
pub(in crate::render::renderer) fn geometry(cols: usize, total_rows: usize) -> Option<(usize, usize, usize, usize, usize)> {
    if cols < 30 || total_rows < 6 {
        return None;
    }
    let popup_w = (cols * 85 / 100).clamp(48, 92).min(cols.saturating_sub(2));
    let popup_h = if total_rows >= 22 {
        16.min(total_rows.saturating_sub(4))
    } else {
        13.min(total_rows.saturating_sub(2))
    };
    if popup_w < 20 || popup_h < 5 {
        return None;
    }
    let x0 = (cols.saturating_sub(popup_w)) / 2;
    let y0 = (total_rows.saturating_sub(popup_h)) / 3;
    let y0 = if y0 == 0 && total_rows > popup_h + 1 { 1 } else { y0 };
    let visible_rows = if popup_h >= 13 {
        popup_h.saturating_sub(7)
    } else {
        popup_h.saturating_sub(4)
    };
    Some((x0, y0, popup_w, popup_h, visible_rows))
}
pub(in crate::render::renderer) fn build(
    cells: &mut Cells,
    geometry: (usize, usize, usize, usize, usize),
    total_rows: usize,
    search: &HistorySearch,
    dirty: &mut DirtyRows,
) {
    let cols = cells.cols;
    let (x0, y0, popup_w, popup_h, visible_rows) = geometry;

    const POPUP_BG: u32 = 0x1819_2490;       // Catppuccin deep slate (#181924) with bit 7 set
    const BORDER_FG: u32 = 0x585B_7080;      // Slate gray border
    const TITLE_FG: u32 = 0x89DC_EB80;       // Cyan title
    const BADGE_FG: u32 = 0xA6AD_C880;       // Lavender counter
    const SORT_FG: u32 = 0xCBA6_F780;        // Purple sort mode badge
    const PROMPT_FG: u32 = 0xA6E3_A180;      // Green prompt ❯
    const QUERY_FG: u32 = 0xF2F2_F780;       // Crisp white query text
    const DIM_FG: u32 = 0x6C70_8680;         // Dim placeholder
    const CURSOR_FG: u32 = 0xCBA6_F780;      // Lavender block cursor
    const SEL_BG: u32 = 0x3134_4C90;         // Selection highlight background with bit 7 set
    const SEL_ARROW_FG: u32 = 0x00BF_E380;   // Cyan selection arrow ➜
    const SEL_TEXT_FG: u32 = 0xFFFF_FF80;     // Bright white text for selected item
    const TEXT_FG: u32 = 0xBAC2_DE80;        // Soft text for unselected item
    const MATCH_FG: u32 = 0xF9E2_AF80;       // Warm gold for matched characters
    const PREVIEW_TITLE_FG: u32 = 0x89DC_EB80;
    const PREVIEW_TEXT_FG: u32 = 0xCDD6_F480;
    const FOOTER_FG: u32 = 0x7F84_9C80;      // Subdued footer text
    const KEY_FG: u32 = 0xCBA6_F780;         // Key indicator

    // 1. Fill popup bounding box with dark background
    for r in 0..popup_h {
        let dst_r = y0 + r;
        if dst_r >= total_rows {
            break;
        }
        let row_start = dst_r * cols;
        for c in 0..popup_w {
            let dst_c = x0 + c;
            if dst_c < cols {
                cells.instances[row_start + dst_c] = Instance {
                    glyph: 0,
                    fg: TEXT_FG,
                    bg: POPUP_BG,
                };
            }
        }
    }
    dirty.mark(y0, (y0 + popup_h - 1).min(total_rows.saturating_sub(1)));

    let queue = cells.queue;

    // 2. Row 0: Top border
    let r0 = y0 * cols;
    let s_tl = cells.atlas.slot(queue, 0x256D) as u32; // ╭
    let s_tr = cells.atlas.slot(queue, 0x256E) as u32; // ╮
    let s_h = cells.atlas.slot(queue, 0x2500) as u32;  // ─
    let s_v = cells.atlas.slot(queue, 0x2502) as u32;  // │

    cells.instances[r0 + x0] = Instance { glyph: s_tl, fg: BORDER_FG, bg: POPUP_BG };
    cells.instances[r0 + x0 + popup_w - 1] = Instance { glyph: s_tr, fg: BORDER_FG, bg: POPUP_BG };
    for c in 1..(popup_w - 1) {
        cells.instances[r0 + x0 + c] = Instance { glyph: s_h, fg: BORDER_FG, bg: POPUP_BG };
    }

    // Title: ╭─ History Search (fzf) ─...
    let title = " History Search (fzf) ";
    for (i, ch) in title.chars().enumerate() {
        if x0 + 2 + i < x0 + popup_w - 1 {
            let slot = cells.atlas.slot(queue, ch as u32) as u32;
            cells.instances[r0 + x0 + 2 + i] = Instance { glyph: slot, fg: TITLE_FG, bg: POPUP_BG };
        }
    }

    // Sort mode badge: [Relevance] or [Recent]
    let sort_label = format!(" [{}] ", search.sort_mode.label());
    let sort_start = x0 + 2 + title.chars().count();
    if sort_start + sort_label.chars().count() + 16 < x0 + popup_w {
        for (i, ch) in sort_label.chars().enumerate() {
            let slot = cells.atlas.slot(queue, ch as u32) as u32;
            cells.instances[r0 + sort_start + i] = Instance { glyph: slot, fg: SORT_FG, bg: POPUP_BG };
        }
    }

    let counter = format!(" [ {}/{} ] ", search.match_count(), search.total_items());
    let counter_len = counter.chars().count();
    if x0 + 2 + title.chars().count() + counter_len + 16 < x0 + popup_w {
        let start_c = x0 + popup_w - 1 - counter_len;
        for (i, ch) in counter.chars().enumerate() {
            let slot = cells.atlas.slot(queue, ch as u32) as u32;
            cells.instances[r0 + start_c + i] = Instance { glyph: slot, fg: BADGE_FG, bg: POPUP_BG };
        }
    }

    // 3. Row 1: Search input
    let r1 = (y0 + 1) * cols;
    cells.instances[r1 + x0] = Instance { glyph: s_v, fg: BORDER_FG, bg: POPUP_BG };
    cells.instances[r1 + x0 + popup_w - 1] = Instance { glyph: s_v, fg: BORDER_FG, bg: POPUP_BG };

    let s_prompt = cells.atlas.slot(queue, 0x276F) as u32; // ❯
    cells.instances[r1 + x0 + 2] = Instance { glyph: s_prompt, fg: PROMPT_FG, bg: POPUP_BG };

    let s_cursor = cells.atlas.slot(queue, 0x258C) as u32; // ▋ left half block cursor
    if search.query.is_empty() {
        let placeholder = "FZF syntax: 'exact  ^prefix  suffix$  !exclude  fuzzy...";
        for (i, ch) in placeholder.chars().enumerate() {
            if x0 + 4 + i < x0 + popup_w - 1 {
                let slot = cells.atlas.slot(queue, ch as u32) as u32;
                cells.instances[r1 + x0 + 4 + i] = Instance { glyph: slot, fg: DIM_FG, bg: POPUP_BG };
            }
        }
        cells.instances[r1 + x0 + 4] = Instance { glyph: s_cursor, fg: CURSOR_FG, bg: POPUP_BG };
    } else {
        let q_chars: Vec<char> = search.query.chars().collect();
        let max_q = popup_w.saturating_sub(6);
        let display_q = q_chars.len().min(max_q);
        for (i, &ch) in q_chars.iter().take(display_q).enumerate() {
            let slot = cells.atlas.slot(queue, ch as u32) as u32;
            cells.instances[r1 + x0 + 4 + i] = Instance { glyph: slot, fg: QUERY_FG, bg: POPUP_BG };
        }
        if x0 + 4 + display_q < x0 + popup_w - 1 {
            cells.instances[r1 + x0 + 4 + display_q] = Instance { glyph: s_cursor, fg: CURSOR_FG, bg: POPUP_BG };
        }
    }

    // 4. Row 2: Separator line
    let r2 = (y0 + 2) * cols;
    let s_ml = cells.atlas.slot(queue, 0x251C) as u32; // ├
    let s_mr = cells.atlas.slot(queue, 0x2524) as u32; // ┤
    cells.instances[r2 + x0] = Instance { glyph: s_ml, fg: BORDER_FG, bg: POPUP_BG };
    cells.instances[r2 + x0 + popup_w - 1] = Instance { glyph: s_mr, fg: BORDER_FG, bg: POPUP_BG };
    for c in 1..(popup_w - 1) {
        cells.instances[r2 + x0 + c] = Instance { glyph: s_h, fg: BORDER_FG, bg: POPUP_BG };
    }

    // 5. Result items
    let s_arrow = cells.atlas.slot(queue, 0x279C) as u32; // ➜
    let s_ellipsis = cells.atlas.slot(queue, '…' as u32) as u32;

    for row_i in 0..visible_rows {
        let dst_r = y0 + 3 + row_i;
        if dst_r >= total_rows {
            break;
        }
        let r_start = dst_r * cols;
        cells.instances[r_start + x0] = Instance { glyph: s_v, fg: BORDER_FG, bg: POPUP_BG };
        cells.instances[r_start + x0 + popup_w - 1] = Instance { glyph: s_v, fg: BORDER_FG, bg: POPUP_BG };

        let item_i = search.scroll_offset + row_i;
        if item_i < search.filtered.len() {
            let m = &search.filtered[item_i];
            let is_selected = item_i == search.selected;
            let row_bg = if is_selected { SEL_BG } else { POPUP_BG };

            // Fill interior with row background
            for c in 1..(popup_w - 1) {
                cells.instances[r_start + x0 + c] = Instance { glyph: 0, fg: TEXT_FG, bg: row_bg };
            }

            if is_selected {
                cells.instances[r_start + x0 + 2] = Instance { glyph: s_arrow, fg: SEL_ARROW_FG, bg: row_bg };
            }

            if let Some(cmd) = search.items.get(m.item_index) {
                let avail_w = popup_w.saturating_sub(6);
                let cmd_chars: Vec<char> = cmd.chars().collect();
                let display_len = cmd_chars.len().min(avail_w);
                let is_truncated = cmd_chars.len() > avail_w;

                for (ci, &ch) in cmd_chars.iter().take(display_len).enumerate() {
                    let is_match = m.match_indices.contains(&ci);
                    let fg = if is_match {
                        MATCH_FG
                    } else if is_selected {
                        SEL_TEXT_FG
                    } else {
                        TEXT_FG
                    };
                    let slot = cells.atlas.slot(queue, ch as u32) as u32;
                    cells.instances[r_start + x0 + 4 + ci] = Instance { glyph: slot, fg, bg: row_bg };
                }
                if is_truncated && avail_w > 0 {
                    cells.instances[r_start + x0 + 4 + display_len - 1] = Instance {
                        glyph: s_ellipsis,
                        fg: DIM_FG,
                        bg: row_bg,
                    };
                }
            }
        } else if search.filtered.is_empty() && row_i == 0 {
            let no_res = "  No matching commands found";
            for (ci, ch) in no_res.chars().enumerate() {
                if x0 + 4 + ci < x0 + popup_w - 1 {
                    let slot = cells.atlas.slot(queue, ch as u32) as u32;
                    cells.instances[r_start + x0 + 4 + ci] = Instance { glyph: slot, fg: DIM_FG, bg: POPUP_BG };
                }
            }
        }
    }

    // 6. Command Preview Section (if popup is tall enough)
    let has_preview = popup_h >= 13;
    if has_preview {
        let prev_sep_r = y0 + 3 + visible_rows;
        let r_sep = prev_sep_r * cols;
        cells.instances[r_sep + x0] = Instance { glyph: s_ml, fg: BORDER_FG, bg: POPUP_BG };
        cells.instances[r_sep + x0 + popup_w - 1] = Instance { glyph: s_mr, fg: BORDER_FG, bg: POPUP_BG };
        for c in 1..(popup_w - 1) {
            cells.instances[r_sep + x0 + c] = Instance { glyph: s_h, fg: BORDER_FG, bg: POPUP_BG };
        }
        let prev_title = " Preview ";
        for (i, ch) in prev_title.chars().enumerate() {
            if x0 + 2 + i < x0 + popup_w - 1 {
                let slot = cells.atlas.slot(queue, ch as u32) as u32;
                cells.instances[r_sep + x0 + 2 + i] = Instance { glyph: slot, fg: PREVIEW_TITLE_FG, bg: POPUP_BG };
            }
        }

        let preview_rows = popup_h.saturating_sub(4 + visible_rows + 1);
        let selected_cmd = search.selected_command().unwrap_or("");
        let avail_w = popup_w.saturating_sub(6);

        // Split into lines or wrap
        let mut preview_lines: Vec<String> = Vec::new();
        for raw_line in selected_cmd.lines() {
            let mut line = raw_line.to_string();
            while line.chars().count() > avail_w && avail_w > 0 {
                let part: String = line.chars().take(avail_w).collect();
                line = line.chars().skip(avail_w).collect();
                preview_lines.push(part);
            }
            preview_lines.push(line);
        }

        for p_i in 0..preview_rows {
            let cur_r = prev_sep_r + 1 + p_i;
            if cur_r >= total_rows {
                break;
            }
            let r_start = cur_r * cols;
            cells.instances[r_start + x0] = Instance { glyph: s_v, fg: BORDER_FG, bg: POPUP_BG };
            cells.instances[r_start + x0 + popup_w - 1] = Instance { glyph: s_v, fg: BORDER_FG, bg: POPUP_BG };
            if let Some(pline) = preview_lines.get(p_i) {
                for (ci, ch) in pline.chars().take(avail_w).enumerate() {
                    let slot = cells.atlas.slot(queue, ch as u32) as u32;
                    cells.instances[r_start + x0 + 3 + ci] = Instance { glyph: slot, fg: PREVIEW_TEXT_FG, bg: POPUP_BG };
                }
            }
        }
    }

    // 7. Last row: Bottom border and key hints
    let rf = (y0 + popup_h - 1) * cols;
    let s_bl = cells.atlas.slot(queue, 0x2570) as u32; // ╰
    let s_br = cells.atlas.slot(queue, 0x256F) as u32; // ╯
    cells.instances[rf + x0] = Instance { glyph: s_bl, fg: BORDER_FG, bg: POPUP_BG };
    cells.instances[rf + x0 + popup_w - 1] = Instance { glyph: s_br, fg: BORDER_FG, bg: POPUP_BG };
    for c in 1..(popup_w - 1) {
        cells.instances[rf + x0 + c] = Instance { glyph: s_h, fg: BORDER_FG, bg: POPUP_BG };
    }

    // Key hints
    let hints: &[(&str, &str)] = if popup_w >= 70 {
        &[("Enter", "Run"), ("Tab", "Insert"), ("Ctrl+R", "Sort"), ("Shift+Del", "Del"), ("Esc", "Close")]
    } else if popup_w >= 54 {
        &[("Enter", "Run"), ("Tab", "Insert"), ("Ctrl+R", "Sort"), ("Esc", "Close")]
    } else {
        &[("Enter", "Run"), ("Tab", "Edit"), ("Esc", "Close")]
    };

    let mut total_hint_len = 0;
    for (i, (key, label)) in hints.iter().enumerate() {
        if i > 0 {
            total_hint_len += 3; // " • "
        }
        total_hint_len += key.chars().count() + 2 + label.chars().count(); // "key: label"
    }
    total_hint_len += 2; // leading and trailing spaces

    if total_hint_len + 4 <= popup_w {
        let mut start_c = x0 + (popup_w - total_hint_len) / 2;
        let s_space = 0;
        cells.instances[rf + start_c] = Instance { glyph: s_space, fg: FOOTER_FG, bg: POPUP_BG };
        start_c += 1;

        for (i, (key, label)) in hints.iter().enumerate() {
            if i > 0 {
                // " • "
                cells.instances[rf + start_c] = Instance { glyph: s_space, fg: FOOTER_FG, bg: POPUP_BG };
                let s_dot = cells.atlas.slot(queue, '•' as u32) as u32;
                cells.instances[rf + start_c + 1] = Instance { glyph: s_dot, fg: DIM_FG, bg: POPUP_BG };
                cells.instances[rf + start_c + 2] = Instance { glyph: s_space, fg: FOOTER_FG, bg: POPUP_BG };
                start_c += 3;
            }
            for ch in key.chars() {
                let slot = cells.atlas.slot(queue, ch as u32) as u32;
                cells.instances[rf + start_c] = Instance { glyph: slot, fg: KEY_FG, bg: POPUP_BG };
                start_c += 1;
            }
            let s_colon = cells.atlas.slot(queue, ':' as u32) as u32;
            cells.instances[rf + start_c] = Instance { glyph: s_colon, fg: FOOTER_FG, bg: POPUP_BG };
            cells.instances[rf + start_c + 1] = Instance { glyph: s_space, fg: FOOTER_FG, bg: POPUP_BG };
            start_c += 2;
            for ch in label.chars() {
                let slot = cells.atlas.slot(queue, ch as u32) as u32;
                cells.instances[rf + start_c] = Instance { glyph: slot, fg: FOOTER_FG, bg: POPUP_BG };
                start_c += 1;
            }
        }
        cells.instances[rf + start_c] = Instance { glyph: s_space, fg: FOOTER_FG, bg: POPUP_BG };
    }
}

#[cfg(test)]
mod tests {
    use super::geometry;

    #[test]
    fn geometry_centers_and_sizes_popup() {
        assert_eq!(geometry(100, 40), Some((7, 8, 85, 16, 9)));
        assert_eq!(geometry(60, 12), Some((4, 1, 51, 10, 6)), "short window: no preview, nudged off row 0");
        assert_eq!(geometry(29, 40), None, "needs 30 columns");
        assert_eq!(geometry(100, 5), None, "needs 6 rows");
    }
}
