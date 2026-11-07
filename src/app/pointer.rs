//! Mouse pointer: scrollbar drag, application mouse reporting, link hover.

use super::state::State;
use super::terminal_io::write_pty;
use super::window::refresh_tab_titles;

use crate::terminal::grid;
use crate::render::renderer;
use crate::mouse;
use crate::HoveredPaneLink;

use winit::window::CursorIcon;

/// Pane under the mouse pointer and the pane-relative `(row, col)` cell.
/// Pane whose visible scrollbar (shown while scrolled back) is under the mouse.
pub(super) fn scrollbar_at_mouse(s: &State) -> Option<u64> {
    let bands = s.bands();
    let (cols, rows) = s.renderer.grid_size_for_tabs(bands);
    let tab = s.tabs.get(s.active)?;
    let (mx, my) = s.pointer.mouse_pos;
    tab.compute_pane_rects(cols, rows).into_iter().find_map(|r| {
        let (right, top, bottom) = s.renderer.scrollbar_track(&r, bands);
        let over = mx <= right && mx >= right - renderer::SCROLLBAR_WIDTH && my >= top && my < bottom;
        let scrolled = tab.pane(r.id).is_some_and(|p| p.grid.lock().scroll_position().1 > 0);
        (over && scrolled).then_some(r.id)
    })
}

/// Scroll the dragged pane so the thumb is centered on the mouse.
pub(super) fn drag_scrollbar(s: &mut State, pane_id: u64) {
    let bands = s.bands();
    let (cols, rows) = s.renderer.grid_size_for_tabs(bands);
    let Some(tab) = s.tabs.get(s.active) else { return };
    let Some(rect) = tab.compute_pane_rects(cols, rows).into_iter().find(|r| r.id == pane_id) else { return };
    let Some(pane) = tab.pane(pane_id) else { return };
    let (_, top, bottom) = s.renderer.scrollbar_track(&rect, bands);
    let mut g = pane.grid.lock();
    let (history, offset) = g.scroll_position();
    let track_height = (bottom - top).max(0.0);
    let (_, height) = renderer::scrollbar_thumb(track_height, g.rows, history, offset);
    let frac = ((s.pointer.mouse_pos.1 - top - height / 2.0) / (track_height - height).max(1.0)).clamp(0.0, 1.0);
    let new_offset = ((1.0 - frac) * history as f64).round() as usize;
    g.scroll_display(new_offset as isize - offset as isize);
    drop(g);
    s.window.request_redraw();
}

pub(super) fn pane_cell_at_mouse(s: &State) -> Option<(u64, usize, usize)> {
    let bands = s.bands();
    let (cols, rows) = s.renderer.grid_size_for_tabs(bands);
    let (row, col) = s.renderer.pixel_to_cell(s.pointer.mouse_pos.0, s.pointer.mouse_pos.1, bands, cols, rows)?;
    let tab = s.tabs.get(s.active)?;
    tab.compute_pane_rects(cols, rows)
        .into_iter()
        .find(|r| col >= r.col && col < r.col + r.cols && row >= r.row && row < r.row + r.rows)
        .map(|r| {
            // Right-to-left rows are drawn reordered: hit-test the logical cell.
            let top = s.renderer.cell_origin_px(r.row, r.col, bands).1;
            let col = col - r.col;
            let (row, col) = tab.pane(r.id).map_or((row - r.row, col), |p| {
                let g = p.grid.lock();
                let row = g.viewport_row(s.pointer.mouse_pos.1 - top, s.renderer.cell_size().1 as f64);
                (row, g.logical_col(row, col))
            });
            (r.id, row, col)
        })
}

/// Forward a mouse event to the application in the pane under the pointer if it
/// tracks the mouse (DECSET 1000/1002/1003). Returns `true` when the application
/// owns the mouse, so the event must not drive selection or scrollback. Holding
/// Shift bypasses reporting, as in other terminals; Cmd/Super stays free for links.
pub(super) fn report_mouse(s: &mut State, button: mouse::Button, action: mouse::Action) -> bool {
    use crate::terminal::grid::MouseTracking;
    if s.mods.shift || s.mods.sup {
        return false;
    }
    let Some((pane_id, row, col)) = pane_cell_at_mouse(s) else { return false };
    let tab = &mut s.tabs[s.active];
    let Some(pane) = tab.pane(pane_id) else { return false };
    let (tracking, sgr, pixels) = {
        let g = pane.grid.lock();
        (g.modes.mouse_tracking, g.modes.mouse_sgr, g.modes.mouse_sgr_pixels)
    };
    if tracking == MouseTracking::Off {
        return false;
    }
    // SGR-Pixels (DECSET 1016): the same report with pane-relative pixel coordinates.
    let (row, col) = if pixels {
        let (cw, ch) = s.renderer.cell_size();
        let (dx, dy) = s.renderer.offset_in_cell(s.pointer.mouse_pos.0, s.pointer.mouse_pos.1, s.bands());
        (row * ch as usize + dy, col * cw as usize + dx)
    } else {
        (row, col)
    };
    let tab = &mut s.tabs[s.active];
    let Some(pane) = tab.pane(pane_id) else { return false };
    let wanted = match action {
        mouse::Action::Press | mouse::Action::Release => true,
        mouse::Action::Motion => {
            let moved = s.pointer.last_mouse_report != Some((pane_id, row, col));
            moved
                && (tracking == MouseTracking::Motion
                    || (tracking == MouseTracking::Drag && button != mouse::Button::None))
        }
    };
    if wanted {
        let mods = mouse::Mods { shift: false, alt: s.mods.alt, ctrl: s.mods.ctrl };
        if let Some((bytes, len)) = mouse::encode(button, action, mods, row, col, sgr || pixels) {
            write_pty(pane, &bytes[..len]);
        }
        s.pointer.last_mouse_report = Some((pane_id, row, col));
    }
    if action == mouse::Action::Press && tab.active_pane_id != pane_id {
        tab.active_pane_id = pane_id;
        refresh_tab_titles(s);
        s.renderer.invalidate();
        s.window.request_redraw();
    }
    true
}

/// Alternate scroll (DECSET 1007): on the alternate screen without mouse tracking,
/// the wheel sends arrow keys so `less`, `man` and editors scroll naturally.
pub(super) fn alt_screen_scroll(s: &mut State, lines: isize) -> bool {
    let Some(tab) = s.tabs.get(s.active) else { return false };
    let Some(pane) = tab.active_pane() else { return false };
    let (alt, enabled, app_cursor) = {
        let g = pane.grid.lock();
        (g.alt_active, g.modes.alt_scroll, g.modes.app_cursor)
    };
    if !alt || !enabled {
        return false;
    }
    let key: &[u8] = match (lines > 0, app_cursor) {
        (true, true) => b"\x1bOA",
        (true, false) => b"\x1b[A",
        (false, true) => b"\x1bOB",
        (false, false) => b"\x1b[B",
    };
    let count = lines.unsigned_abs().min(20);
    write_pty(pane, &key.repeat(count));
    true
}

pub(super) fn matches_cached_hover_candidate(
    cached: &grid::Hyperlink,
    row: usize,
    col: usize,
    cand: &grid::HyperlinkCandidate,
) -> bool {
    cached.row == row
        && col >= cached.start_col
        && col <= cached.end_col
        && match cand {
            grid::HyperlinkCandidate::Resolved(l) => {
                l.start_col == cached.start_col
                    && l.end_col == cached.end_col
                    && l.url == cached.url
                    && l.is_path == cached.is_path
            }
            grid::HyperlinkCandidate::ImplicitPath {
                start_col,
                end_col,
                token,
                ..
            } => {
                *start_col == cached.start_col
                    && *end_col == cached.end_col
                    && *token == cached.url
                    && cached.is_path
            }
        }
}

pub(super) fn update_hovered_link(s: &mut State) {
    #[cfg(target_os = "macos")]
    let cmd_pressed = s.mods.sup;
    #[cfg(not(target_os = "macos"))]
    let cmd_pressed = s.mods.ctrl || s.mods.sup;

    if !cmd_pressed && s.pointer.hovered_link.is_none() {
        return;
    }

    if s.tabs.is_empty() {
        if s.pointer.hovered_link.is_some() {
            s.pointer.hovered_link = None;
            s.window.set_cursor(CursorIcon::Default);
        }
        return;
    }

    let bands = s.bands();
    let (cols, rows) = s.renderer.grid_size_for_tabs(bands);

    let new_link = if cmd_pressed && !s.pointer.selecting {
        if let Some((cell_row, cell_col)) = s.renderer.pixel_to_cell(s.pointer.mouse_pos.0, s.pointer.mouse_pos.1, bands, cols, rows) {
            let rects = s.tabs[s.active].compute_pane_rects(cols, rows);
            if let Some(target) = rects.iter().find(|r| {
                cell_col >= r.col
                    && cell_col < r.col + r.cols
                    && cell_row >= r.row
                    && cell_row < r.row + r.rows
            }) {
                if let Some(pane) = s.tabs[s.active].pane_mut(target.id) {
                    let top = s.renderer.cell_origin_px(target.row, target.col, bands).1;
                    let (pane_row, pane_col, candidate) = {
                        let g = pane.grid.lock();
                        let pane_row = g.viewport_row(s.pointer.mouse_pos.1 - top, s.renderer.cell_size().1 as f64);
                        let lcol = g.logical_col(pane_row, cell_col - target.col);
                        (pane_row, lcol, g.find_hyperlink_candidate_at(pane_row, lcol))
                    };
                    match candidate {
                        Some(cand) => {
                            if let Some(existing) = s.pointer.hovered_link.as_ref().filter(|h| {
                                h.pane_id == target.id
                                    && matches_cached_hover_candidate(&h.link, pane_row, pane_col, &cand)
                            }) {
                                Some(existing.clone())
                            } else {
                                let cwd = pane.cwd();
                                let cwd_path = cwd.as_deref().map(std::path::Path::new);
                                cand.resolve(cwd_path).map(|link| HoveredPaneLink {
                                    pane_id: target.id,
                                    link,
                                })
                            }
                        }
                        None => None,
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    if s.pointer.hovered_link != new_link {
        s.pointer.hovered_link = new_link;
        if s.pointer.hovered_link.is_some() {
            s.window.set_cursor(CursorIcon::Pointer);
        } else {
            s.window.set_cursor(s.pointer.pointer_icon);
        }
        s.renderer.invalidate();
        s.window.request_redraw();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_cached_hover_candidate_invalidated_when_row_overwritten() {
        let mut g = crate::terminal::grid::Grid::new(80, 24);
        let mut p = vte::Parser::new();
        for &b in b"Open ./Cargo.toml now" {
            g.feed_byte(&mut p, b);
        }
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let cand1 = g.find_hyperlink_candidate_at(0, 6).expect("initial candidate");
        let resolved1 = cand1.clone().resolve(Some(manifest_dir)).expect("resolved ./Cargo.toml");
        assert!(super::matches_cached_hover_candidate(&resolved1, 0, 10, &cand1));

        // Overwrite row 0 in place with a different path of the same length
        for &b in b"\rOpen ./src/tab.rs now" {
            g.feed_byte(&mut p, b);
        }
        let cand2 = g.find_hyperlink_candidate_at(0, 10).expect("new candidate after overwrite");
        assert!(
            !super::matches_cached_hover_candidate(&resolved1, 0, 10, &cand2),
            "cached link must not match after underlying terminal row text changes"
        );
        let resolved2 = cand2.resolve(Some(manifest_dir)).expect("resolved ./src/tab.rs");
        assert_eq!(resolved2.url, "./src/tab.rs");

        // Overwrite row 0 with plain non-link text
        for &b in b"\rPlain text only here " {
            g.feed_byte(&mut p, b);
        }
        assert!(g.find_hyperlink_candidate_at(0, 10).is_none());
    }
}
