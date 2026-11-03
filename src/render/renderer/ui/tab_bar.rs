//! Tab strip on the top row: close button, centered title, shortcut badge.

use super::super::Instance;
use super::Cells;

pub(in crate::render::renderer) fn build(cells: &mut Cells, titles: &[String], active_idx: usize) {
    let cols = cells.cols;
    const ACTIVE_FG: u32 = 0xF2F2_F780;
    const ACTIVE_BADGE_FG: u32 = 0xA6A8_B380;
    const INACTIVE_FG: u32 = 0x8E90_9980;
    const INACTIVE_BADGE_FG: u32 = 0x686A_7380;
    const BAR_BG: u32 = 0x1C1D_2080;

    for dst in &mut cells.instances[0..cols] {
        *dst = Instance { glyph: 0, fg: INACTIVE_FG, bg: BAR_BG };
    }

    let tab_cols_total = available_columns(cols);
    let n = titles.len().max(1);
    for (i, title) in titles.iter().enumerate() {
        let (c0, c1) = span(tab_cols_total, n, i);
        let span = c1.saturating_sub(c0);
        if span < 4 {
            continue;
        }
        let (fg, badge_fg) = if i == active_idx {
            (ACTIVE_FG, ACTIVE_BADGE_FG)
        } else {
            (INACTIVE_FG, INACTIVE_BADGE_FG)
        };

        // Left-aligned close button ('✕' at c0 + 1)
        let left_reserve = if shows_close(span) {
            let close_slot = cells.atlas.slot(cells.queue, '✕' as u32);
            cells.instances[c0 + 1] = Instance { glyph: close_slot as u32, fg: badge_fg, bg: BAR_BG };
            3
        } else {
            1
        };

        // Right-aligned shortcut badge (`⌘1`..`⌘9` on macOS, `^1`..`^9` on Linux).
        let badge_reserve = if i < 9 && span >= 10 {
            #[cfg(target_os = "macos")]
            let mod_ch = '⌘';
            #[cfg(not(target_os = "macos"))]
            let mod_ch = '^';
            let digit = (b'1' + i as u8) as char;
            let bx = c1 - 3;
            let s0 = cells.atlas.slot(cells.queue, mod_ch as u32);
            let s1 = cells.atlas.slot(cells.queue, digit as u32);
            cells.instances[bx] = Instance { glyph: s0 as u32, fg: badge_fg, bg: BAR_BG };
            cells.instances[bx + 1] = Instance { glyph: s1 as u32, fg: badge_fg, bg: BAR_BG };
            4
        } else {
            1
        };

        let avail = span.saturating_sub(left_reserve + badge_reserve);
        if avail == 0 {
            continue;
        }
        let label = tab_label(title);
        let count = label.chars().count();
        let truncated = count > avail;
        let shown = count.min(avail);
        let start_col = c0 + left_reserve + (avail.saturating_sub(shown)) / 2;
        for (k, ch) in label.chars().take(shown).enumerate() {
            let ch = if truncated && k + 1 == shown { '…' } else { ch };
            let x = start_col + k;
            if x < c1.saturating_sub(badge_reserve) && x < cols {
                let slot = cells.atlas.slot(cells.queue, ch as u32);
                cells.instances[x] = Instance { glyph: slot as u32, fg, bg: BAR_BG };
            }
        }
    }
}

/// Tab bar label for a window title: drops a leading `user@host:` (or `user:`)
/// prompt prefix so narrow tabs show only the path. The window title keeps the
/// full string.
fn tab_label(title: &str) -> &str {
    if let Some((prefix, rest)) = title.split_once(':')
        && !prefix.is_empty()
            && !prefix.chars().any(char::is_whitespace)
            && (rest.starts_with('~') || (rest.starts_with('/') && !rest.starts_with("//")))
        {
            return rest;
        }
    title
}

#[cfg(test)]
mod tests {
    use super::tab_label;

    #[test]
    fn test_tab_label_strips_user_host_prefix() {
        assert_eq!(tab_label("linh@mac:~/Desktop/personal/fastdb"), "~/Desktop/personal/fastdb");
        assert_eq!(tab_label("linh:~/src"), "~/src");
        assert_eq!(tab_label("root@box:/etc [Z]"), "/etc [Z]");
        assert_eq!(tab_label("~/src"), "~/src");
        assert_eq!(tab_label("vim: notes.md"), "vim: notes.md");
        assert_eq!(tab_label("shell"), "shell");
        assert_eq!(tab_label("http://x"), "http://x");
    }
}

/// The two final columns are reserved for New Tab, even in narrow windows.
pub(in crate::render::renderer) fn available_columns(cols: usize) -> usize { cols.saturating_sub(2).max(1) }

pub(in crate::render::renderer) fn span(available: usize, count: usize, index: usize) -> (usize, usize) {
    let count = count.max(1);
    (index * available / count, (index + 1) * available / count)
}

pub(in crate::render::renderer) fn shows_close(width: usize) -> bool { width >= 6 }

#[cfg(test)]
#[test]
fn shared_geometry_preserves_thresholds_and_uneven_spans() {
    assert_eq!(available_columns(0), 1);
    assert_eq!(available_columns(82), 80);
    assert_eq!(span(11, 3, 0), (0, 3));
    assert_eq!(span(11, 3, 1), (3, 7));
    assert_eq!(span(11, 3, 2), (7, 11));
    assert_eq!(span(10, 0, 0), (0, 10));
    for width in [0, 4, 5, 6, 9, 10] { assert_eq!(shows_close(width), width >= 6); }
}
