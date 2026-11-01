//! Compact 12-byte cells, a zero-allocation ring buffer with scrollback, and the
//! `vte::Perform` implementation that turns the PTY byte stream into grid state.
//!
//! Memory layout: the whole scrollback + screen lives in one flat `Vec<Cell>`
//! (`cols * (SCROLLBACK + rows)`), allocated with `alloc_zeroed` so the OS only
//! commits pages once history actually reaches them. Scrolling the full screen
//! is O(1): bump `top` and clear one row in place. Nothing is allocated while
//! printing or scrolling; allocation only happens on window resize.

use std::io::Write;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use vte::Params;

pub mod cell;
pub mod perform;
pub mod prompt;
pub mod selection;

#[cfg(test)]
mod perf_probe;

mod colors;
mod control;
mod images;

pub use colors::{color_scheme_is_dark, color_scheme_report, set_color_scheme_dark, set_default_colors, set_palette};
pub(crate) use images::{ImageCompletion, ImageJob};
use control::{DcsRequest, MAX_IMAGE_STRING, MAX_OSC_BYTES, parse_osc7_path};

pub use cell::*;
pub use prompt::*;
pub use selection::*;

use crate::search::SearchText;

/// Number of scrollback lines kept above the visible screen by default.
pub const SCROLLBACK: usize = 5000;
/// Upper bound for a configured scrollback, whatever its byte size.
const MAX_SCROLLBACK: usize = 1_000_000;

/// `scrollback-limit` in bytes (Ghostty semantics); `usize::MAX` = the default
/// line count. Read when a terminal is created or resized.
static SCROLLBACK_BYTES: AtomicUsize = AtomicUsize::new(usize::MAX);

pub fn set_scrollback_limit(bytes: Option<usize>) {
    SCROLLBACK_BYTES.store(bytes.unwrap_or(usize::MAX), Ordering::Relaxed);
}

/// Scrollback lines for a terminal `cols` wide.
fn scrollback_lines(cols: usize) -> usize {
    lines_for_bytes(SCROLLBACK_BYTES.load(Ordering::Relaxed), cols)
}

fn lines_for_bytes(bytes: usize, cols: usize) -> usize {
    match bytes {
        usize::MAX => SCROLLBACK,
        bytes => (bytes / (cols.max(1) * std::mem::size_of::<Cell>())).min(MAX_SCROLLBACK),
    }
}




/// `grapheme-width-method = unicode`: new terminals start in DEC mode 2027.
static GRAPHEME_DEFAULT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Set the initial state of DEC mode 2027 (grapheme clustering) for new terminals.
pub fn set_grapheme_default(on: bool) {
    GRAPHEME_DEFAULT.store(on, Ordering::Relaxed);
}



pub struct Grid {
    osc_len: Option<usize>,
    osc_discard: bool,
    osc_start: Option<std::time::Instant>,
    pub(crate) defer_images: bool,
    pub(crate) image_job: Option<ImageJob>,
    pub(crate) image_epoch: u64,
    /// AI bridge is armed only by a user submission at a known shell prompt.
    pub ai_available: bool,
    pub ai_direct: bool,
    pub ai_armed: bool,
    pub ai_track: bool,
    pub ai_command: Option<String>,
    pub ai_request: Option<String>,
    pub prompt_epoch: u64,
    pub last_exit: Option<i32>,
    pub cols: usize,
    pub rows: usize,
    /// Scrollback lines kept above the screen (`scrollback-limit`).
    scrollback: usize,
    /// Ring capacity in rows (at most `scrollback + rows`).
    cap: usize,
    /// Main screen + scrollback ring.
    cells: Vec<Cell>,
    /// Physical ring row of visible row 0.
    top: usize,
    /// Number of valid scrollback rows above `top`.
    history: usize,
    /// How many lines the user has scrolled back into history (0 = live).
    pub display_offset: usize,
    /// Fraction of a row clipped above the viewport during precise scrolling.
    pub scroll_fraction: f64,
    pub viewport_dirty: bool,

    /// Alternate screen (vim, less, htop...), no scrollback.
    alt: Vec<Cell>,
    pub alt_active: bool,

    cur: Cursor,
    saved: Cursor,
    saved_main: Cursor,
    wrap_pending: bool,
    scroll_top: usize,
    scroll_bot: usize,

    /// Per physical main-screen row: the line continues on the next row (soft wrap).
    wrapped: Vec<bool>,
    /// Horizontal tab stops, one flag per column.
    tab_stops: Vec<bool>,
    /// IRM: printing shifts the rest of the line right instead of overwriting.
    insert_mode: bool,
    /// Lines ever pushed into scrollback; anchors absolute line numbers.
    lines_pushed: usize,

    pub cursor_visible: bool,
    /// Last DECSCUSR (`CSI Ps SP q`) value: 0 = config default,
    /// 1/2 = blinking/steady block, 3/4 = underline, 5/6 = bar.
    pub cursor_shape: u8,
    pub app_cursor: bool,
    pub bracketed_paste: bool,
    pub sync_output: bool,
    /// When synchronized output (DECSET 2026) began; rendering is deferred meanwhile.
    pub sync_started: Option<std::time::Instant>,
    autowrap: bool,

    pub mouse_tracking: MouseTracking,
    /// SGR mouse encoding (DECSET 1006).
    pub mouse_sgr: bool,
    /// Report focus in/out (DECSET 1004).
    pub focus_events: bool,
    /// Translate the mouse wheel into arrow keys on the alternate screen (DECSET 1007).
    pub alt_scroll: bool,
    /// SGR mouse reports carry pixel instead of cell coordinates (DECSET 1016).
    pub mouse_sgr_pixels: bool,
    /// Report system light/dark appearance changes (DECSET 2031).
    pub color_scheme_reports: bool,
    /// Cell size in pixels, for XTWINOPS size reports.
    pub cell_px: (u16, u16),
    /// DCS request being collected (`DCS + q` XTGETTCAP or `DCS $ q` DECRQSS).
    dcs: Option<DcsRequest>,
    dcs_buf: Vec<u8>,
    dcs_start: Option<std::time::Instant>,
    /// Inline images (Kitty graphics protocol, Sixel).
    pub graphics: crate::graphics::Store,
    /// APC filter state in front of vte (which drops APC strings): 0 ground,
    /// 1 after ESC, 2 inside `ESC _`, 3 ESC inside it.
    apc_state: u8,
    apc_buf: Vec<u8>,
    apc_start: Option<std::time::Instant>,
    /// xterm `modifyOtherKeys` level (0 off, 1, 2) set by `CSI > 4 ; level m`.
    pub modify_other_keys: u8,
    /// Kitty keyboard protocol state for the main and the alternate screen.
    kitty_keyboard: [KittyKeyboard; 2],

    /// Window/tab title set via OSC 0 / OSC 2 (e.g. `user@host:~/dir` or running command).
    pub title: String,
    /// Set when the title or the reported working directory changes.
    pub title_changed: bool,
    /// Working directory reported by the shell via OSC 7 (`file://host/path`).
    pub reported_cwd: Option<String>,
    /// Text an application asked to place on the clipboard via OSC 52.
    pub clipboard_request: Option<String>,
    /// Desktop notification requested by OSC 9 / OSC 777 as `(title, body)`.
    pub notification: Option<(String, String)>,
    /// A BEL arrived since the UI last looked.
    pub bell: bool,
    /// OSC 9;4 progress as `(state, percent)`: 1 normal, 2 error, 3 indeterminate,
    /// 4 paused. `None` when no progress is shown.
    pub progress: Option<(u8, u8)>,
    /// OSC 22 mouse pointer shape (a CSS or X11 cursor name; empty = default).
    pub pointer_shape: String,
    /// OSC 133 prompts on the main screen, oldest first (at most `MAX_PROMPTS`).
    prompts: Vec<PromptMark>,
    /// OSC 133;B: absolute line and column where the command input starts.
    prompt_input_mark: Option<(usize, usize)>,

    /// Active mouse text selection (rows are absolute line numbers).
    selection: Option<Selection>,
    /// Find-in-terminal matches (absolute lines, top-to-bottom, non-overlapping).
    search_matches: Vec<Selection>,
    /// Index of the focused match in `search_matches`.
    search_current: Option<usize>,

    /// Per visible row dirty flags, plus a "everything changed" shortcut.
    pub dirty: Vec<bool>,
    pub all_dirty: bool,

    /// Bytes the terminal must send back to the shell (DSR, DA replies).
    pub response: Vec<u8>,

    /// Palette entries changed by OSC 4, as `rgb | 1` (0 = config palette). Empty
    /// until an application first sets one.
    palette_override: Vec<u32>,
    /// Default foreground, background and cursor colors set by OSC 10 / 11 / 12,
    /// as `rgb | 1` (0 = config default).
    color_override: [u32; 3],

    /// Hyperlinks and underline colors referenced by `Cell::extra` (index 0 is
    /// reserved for "none"). Slots no cell uses any more are reclaimed when full.
    extras: Vec<Option<CellExtra>>,
    extras_dirty: bool,
    extras_full: bool,
    /// Multi-codepoint graphemes cells refer to (see `CLUSTER_FLAG`), interned.
    clusters: Vec<Box<str>>,
    cluster_ids: std::collections::HashMap<Box<str>, u32>,
    /// Reused buffer for building a cluster.
    cluster_buf: String,
    /// Collect at most once per history-sized batch of cluster-bearing evictions.
    cluster_evictions: usize,
    cluster_marks: Vec<bool>,
    cluster_remap: Vec<u32>,
    /// DEC mode 2027: emoji sequences (ZWJ, flags, skin tones, VS16) share one cell pair.
    grapheme_mode: bool,
    /// The last character joined a ZWJ to a cluster: a pictograph after it joins too.
    joiner: bool,
}

thread_local! {
    static BIDI_SCRATCH: std::cell::RefCell<(Vec<char>, Vec<u8>, Vec<u16>)> =
        const { std::cell::RefCell::new((Vec::new(), Vec::new(), Vec::new())) };
}

impl Grid {
    pub fn new(cols: usize, rows: usize) -> Self {
        let cols = cols.max(1);
        let rows = rows.max(1);
        let scrollback = scrollback_lines(cols);
        let max_cap = scrollback + rows;
        // Start lean: allocate only 2x rows (minimum 64) instead of the full 5000+ scrollback ring.
        // The ring buffer doubles dynamically on demand up to max_cap when lines scroll off screen.
        let cap = (rows * 2).max(64).min(max_cap);
        Grid {
            ai_available: false,
            ai_direct: false,
            ai_armed: false,
            ai_track: false,
            ai_command: None,
            ai_request: None,
            prompt_epoch: 0,
            last_exit: None,
            cols,
            rows,
            scrollback,
            cap,
            cells: bytemuck::zeroed_vec(cols * cap),
            top: 0,
            history: 0,
            display_offset: 0,
            scroll_fraction: 0.0,
            viewport_dirty: false,
            alt: Vec::new(),
            alt_active: false,
            cur: Cursor::default(),
            saved: Cursor::default(),
            saved_main: Cursor::default(),
            wrap_pending: false,
            scroll_top: 0,
            scroll_bot: rows - 1,
            wrapped: vec![false; cap],
            tab_stops: default_tab_stops(cols),
            insert_mode: false,
            lines_pushed: 0,
            cursor_visible: true,
            cursor_shape: 0,
            app_cursor: false,
            bracketed_paste: false,
            sync_output: false,
            sync_started: None,
            autowrap: true,
            mouse_tracking: MouseTracking::Off,
            mouse_sgr: false,
            focus_events: false,
            alt_scroll: true,
            mouse_sgr_pixels: false,
            color_scheme_reports: false,
            cell_px: (0, 0),
            osc_len: None,
            osc_discard: false,
            osc_start: None,
            defer_images: false,
            image_job: None,
            image_epoch: 0,
            dcs: None,
            dcs_buf: Vec::new(),
            dcs_start: None,
            graphics: crate::graphics::Store::default(),
            apc_state: 0,
            apc_buf: Vec::new(),
            apc_start: None,
            modify_other_keys: 0,
            kitty_keyboard: [KittyKeyboard::default(); 2],
            title: String::new(),
            title_changed: false,
            reported_cwd: None,
            clipboard_request: None,
            notification: None,
            bell: false,
            progress: None,
            pointer_shape: String::new(),
            prompt_input_mark: None,
            prompts: Vec::new(),
            selection: None,
            search_matches: Vec::new(),
            search_current: None,
            dirty: vec![true; rows],
            all_dirty: true,
            response: Vec::with_capacity(256),
            extras: vec![None],
            extras_dirty: true,
            extras_full: false,
            clusters: Vec::new(),
            cluster_ids: std::collections::HashMap::new(),
            cluster_buf: String::new(),
            cluster_evictions: 0,
            cluster_marks: Vec::new(),
            cluster_remap: Vec::new(),
            grapheme_mode: GRAPHEME_DEFAULT.load(Ordering::Relaxed),
            joiner: false,
            palette_override: Vec::new(),
            color_override: [0; 3],
        }
    }

    // ------------------------------------------------------------------
    // Row addressing
    // ------------------------------------------------------------------

    #[inline]
    fn row_start(&self, r: usize) -> usize {
        if self.alt_active {
            r * self.cols
        } else {
            ((self.top + r) % self.cap) * self.cols
        }
    }

    #[inline]
    fn buf_mut(&mut self) -> &mut [Cell] {
        if self.alt_active {
            &mut self.alt
        } else {
            &mut self.cells
        }
    }

    #[inline]
    fn row_mut(&mut self, r: usize) -> &mut [Cell] {
        let s = self.row_start(r);
        let c = self.cols;
        &mut self.buf_mut()[s..s + c]
    }

    /// Row `r` of what should be on screen, accounting for scrollback viewing.
    #[inline]
    pub fn display_row(&self, r: usize) -> &[Cell] {
        if self.alt_active {
            let s = r * self.cols;
            &self.alt[s..s + self.cols]
        } else {
            let p = (self.top + self.cap - self.display_offset + r) % self.cap;
            let s = p * self.cols;
            &self.cells[s..s + self.cols]
        }
    }

    /// Cursor position in screen coordinates, if it should be drawn.
    /// Absolute line number of visible row `r`.
    #[inline]
    fn abs_line(&self, r: usize) -> usize {
        if self.alt_active {
            self.lines_pushed + r
        } else {
            self.lines_pushed + r - self.display_offset
        }
    }

    /// Cells of absolute line `l`, if it is still in scrollback or on screen.
    fn line_cells(&self, l: usize) -> Option<&[Cell]> {
        let c = self.cols;
        if self.alt_active {
            let r = l.checked_sub(self.lines_pushed).filter(|&r| r < self.rows)?;
            return Some(&self.alt[r * c..(r + 1) * c]);
        }
        let i = l.checked_sub(self.lines_pushed - self.history).filter(|&i| i < self.history + self.rows)?;
        let p = (self.top + self.cap - self.history + i) % self.cap;
        Some(&self.cells[p * c..(p + 1) * c])
    }

    /// Whether absolute line `l` soft-wraps onto the next line.
    fn line_wrapped(&self, l: usize) -> bool {
        if self.alt_active {
            return false;
        }
        l.checked_sub(self.lines_pushed - self.history)
            .filter(|&i| i < self.history + self.rows)
            .is_some_and(|i| self.wrapped[(self.top + self.cap - self.history + i) % self.cap])
    }

    #[inline]
    fn set_wrapped(&mut self, r: usize, on: bool) {
        if !self.alt_active {
            let p = (self.top + r) % self.cap;
            self.wrapped[p] = on;
        }
    }

    pub fn cursor_pos(&self) -> Option<(usize, usize)> {
        if !self.cursor_visible {
            return None;
        }
        let off = if self.alt_active { 0 } else { self.display_offset };
        let r = self.cur.row + off;
        (r < self.rows).then(|| (self.cur.col.min(self.cols - 1), r))
    }

    /// Scroll the viewport through history (positive = towards older lines).
    /// The selection stays attached to its text.
    pub fn scroll_display(&mut self, delta: isize) {
        if self.alt_active {
            return;
        }
        let was_fractional = self.scroll_fraction != 0.0;
        self.scroll_fraction = 0.0;
        let new = (self.display_offset as isize + delta).clamp(0, self.history as isize) as usize;
        if new != self.display_offset || was_fractional {
            self.display_offset = new;
            self.all_dirty = true;
        }
    }

    /// Precise scrolling changes the viewport on every pixel, not only at row boundaries.
    pub fn scroll_pixels(&mut self, pixels: f64, cell_height: f64) -> bool {
        if self.alt_active || !pixels.is_finite() || !cell_height.is_finite() || cell_height <= 0.0 {
            return false;
        }
        let current = self.display_offset as f64 - self.scroll_fraction;
        let next = (current + pixels / cell_height).clamp(0.0, self.history as f64);
        let nearest = next.round();
        let next = if (next - nearest).abs() < 1e-10 { nearest } else { next };
        let offset = next.ceil() as usize;
        let fraction = offset as f64 - next;
        if offset == self.display_offset && (fraction - self.scroll_fraction).abs() < 1e-10 {
            return false;
        }
        self.all_dirty |= offset != self.display_offset;
        self.display_offset = offset;
        self.scroll_fraction = fraction;
        self.viewport_dirty = true;
        true
    }

    /// Hit-test a row after applying the same pixel shift as the renderer.
    pub fn viewport_row(&self, y: f64, cell_height: f64) -> usize {
        ((y / cell_height.max(1.0) + self.scroll_fraction).max(0.0).floor() as usize)
            .min(self.rows.saturating_sub(usize::from(self.scroll_fraction == 0.0)))
    }

    pub fn reset_display(&mut self) {
        if self.display_offset != 0 {
            self.display_offset = 0;
            self.scroll_fraction = 0.0;
            self.all_dirty = true;
        }
        if self.selection.is_some() {
            self.selection = None;
            self.all_dirty = true;
        }
    }

    pub fn clear_history(&mut self) {
        self.image_epoch = self.image_epoch.wrapping_add(1);
        self.prompts.clear();
        self.graphics.prune(self.lines_pushed);
        self.history = 0;
        self.display_offset = 0;
        self.scroll_fraction = 0.0;
        self.selection = None;
        self.clear_search();
        self.trim_extras();
        if !self.clusters.is_empty() {
            self.gc_clusters();
        }
        if !self.graphics.virtuals.is_empty() {
            self.prune_dead_virtuals();
        }
        self.all_dirty = true;
    }

    /// Clear both scrollback history and visible screen rows, placing cursor at top-left.
    /// Provides instant 0ms visual feedback on Cmd+K without waiting for shell roundtrip.
    pub fn clear_screen_and_history(&mut self) {
        self.clear_history();
        let rows = self.rows;
        self.clear_rows(0, rows);
        self.graphics.delete_lines(self.lines_pushed..self.lines_pushed + rows, self.alt_active);
        self.graphics.virtuals.clear();
        self.graphics.prune_dead_auto_ids();
        self.cur.row = 0;
        self.cur.col = 0;
        self.wrap_pending = false;
        self.clusters.clear();
        self.cluster_ids.clear();
        self.trim_extras();
        self.apc_state = 0;
        self.apc_buf.clear();
        self.apc_buf.shrink_to_fit();
        self.apc_start = None;
        self.dcs = None;
        self.dcs_buf.clear();
        self.dcs_buf.shrink_to_fit();
        self.dcs_start = None;
        self.all_dirty = true;
    }

    /// Set the selection from visible `(row, col)` coordinates.
    pub fn set_selection(&mut self, sel: Option<Selection>) {
        let sel = sel.map(|s| Selection {
            start: (self.abs_line(s.start.0), s.start.1),
            end: (self.abs_line(s.end.0), s.end.1),
        });
        if self.selection != sel {
            self.selection = sel;
            self.all_dirty = true;
        }
    }

    /// Select everything in scrollback and on screen, from the first to the last
    /// line with text (`Cmd+A`).
    pub fn select_all(&mut self) {
        let first = if self.alt_active { self.lines_pushed } else { self.lines_pushed - self.history };
        let has_text = |l: &usize| self.line_cells(*l).is_some_and(|r| r.iter().any(|c| !Self::is_blank(c.codepoint)));
        let lines = first..self.lines_pushed + self.rows;
        let sel = lines.clone().find(has_text).zip(lines.rev().find(has_text)).map(|(top, bottom)| Selection {
            start: (top, 0),
            end: (bottom, self.cols - 1),
        });
        if self.selection != sel {
            self.selection = sel;
            self.all_dirty = true;
        }
    }

    /// Shift+Click: move the selection end nearest to visible `(row, col)` there,
    /// keeping the other end as the anchor. Returns `false` without a selection.
    pub fn extend_selection(&mut self, row: usize, col: usize) -> bool {
        let Some(sel) = self.selection else {
            return false;
        };
        let pos = (self.abs_line(row), col);
        let (a, b) = sel.ordered();
        let offset = |p: (usize, usize)| p.0 * self.cols + p.1;
        let anchor = if offset(pos).abs_diff(offset(a)) <= offset(pos).abs_diff(offset(b)) { b } else { a };
        self.selection = Some(Selection { start: anchor, end: pos });
        self.all_dirty = true;
        true
    }

    /// Dragging after Shift+Click: move the selection end, keeping its anchor
    /// (which may be in scrollback, off screen).
    pub fn set_selection_end(&mut self, row: usize, col: usize) {
        let end = (self.abs_line(row), col);
        if let Some(sel) = self.selection.as_mut().filter(|s| s.end != end) {
            sel.end = end;
            self.all_dirty = true;
        }
    }

    #[inline]
    pub fn is_selected(&self, row: usize, col: usize) -> bool {
        let Some(sel) = self.selection else {
            return false;
        };
        let (start, end) = sel.ordered();
        let pos = (self.abs_line(row), col);
        pos >= start && pos <= end
    }

    #[inline]
    fn is_path_char(ch: char) -> bool {
        if ('\u{E000}'..='\u{F8FF}').contains(&ch)
            || ('\u{F0000}'..='\u{FFFFD}').contains(&ch)
            || ('\u{100000}'..='\u{10FFFD}').contains(&ch)
        {
            return false;
        }
        ch.is_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | '~' | '+' | '@' | '%' | '=')
    }

    /// Extract an explicit OSC 8 hyperlink, URL, or unresolved file path candidate
    /// containing `(row, col)` without performing any blocking filesystem `stat()` calls.
    pub fn find_hyperlink_candidate_at(
        &self,
        row: usize,
        col: usize,
    ) -> Option<HyperlinkCandidate> {
        if row >= self.rows + usize::from(self.scroll_fraction > 0.0) || self.cols == 0 || col >= self.cols {
            return None;
        }
        let r = self.display_row(row);

        // 1. An explicit OSC 8 hyperlink wins over anything detected in the text.
        if let Some(link) = self.cell_link(&r[col]) {
            let same = |c: &Cell| self.cell_link(c) == Some(link);
            let start = r[..col].iter().rposition(|c| !same(c)).map_or(0, |i| i + 1);
            let end = r[col..].iter().position(|c| !same(c)).map_or(self.cols, |i| col + i);
            let (url, is_path) = osc8_target(&link.uri)?;
            return Some(HyperlinkCandidate::Resolved(Hyperlink {
                row,
                start_col: start,
                end_col: end - 1,
                url,
                is_path,
            }));
        }

        let chars: Vec<char> = r.iter().map(|c| self.cell_char(c)).collect();

        // 2. Try URL matching
        let mut idx = 0;
        while idx < chars.len() {
            let starts_with = |pat: &str| pat.chars().enumerate().all(|(k, p)| chars.get(idx + k) == Some(&p));
            let is_https = starts_with("https://");
            let is_http = !is_https && starts_with("http://");
            if is_https || is_http {
                let start = idx;
                let mut end = if is_https { idx + 8 } else { idx + 7 };
                while end < chars.len() {
                    let ch = chars[end];
                    if ch.is_ascii_whitespace()
                        || ch == '\0'
                        || ('\u{E000}'..='\u{F8FF}').contains(&ch)
                        || matches!(ch, '<' | '>' | '"' | '{' | '}' | '|' | '\\' | '^' | '`')
                    {
                        break;
                    }
                    end += 1;
                }
                let end = trim_trailing_punctuation(&chars, start, end);
                if col >= start && col < end && end > start + 7 {
                    let url_str: String = chars[start..end].iter().collect();
                    return Some(HyperlinkCandidate::Resolved(Hyperlink {
                        row,
                        start_col: start,
                        end_col: end - 1,
                        url: url_str,
                        is_path: false,
                    }));
                }
                idx = end.max(idx + 1);
            } else {
                idx += 1;
            }
        }

        // 3. Try file/directory path matching at `col`
        if !Self::is_path_char(chars[col]) {
            return None;
        }
        let mut start = col;
        while start > 0 && Self::is_path_char(chars[start - 1]) {
            start -= 1;
        }
        let mut end = col + 1;
        while end < chars.len() && Self::is_path_char(chars[end]) {
            end += 1;
        }

        let end = trim_trailing_punctuation(&chars, start, end);

        if start >= end || end - start < 2 {
            return None;
        }

        let token_str: String = chars[start..end].iter().collect();

        // Must look like a path: contain '/' or start with '~'
        if !token_str.contains('/') && token_str != "~" {
            return None;
        }

        Some(HyperlinkCandidate::ImplicitPath {
            row,
            start_col: start,
            end_col: end - 1,
            token: token_str,
        })
    }

    /// Detect a URL (http:// or https://) or a file/directory path containing `(row, col)`.
    pub fn find_hyperlink_at(
        &self,
        row: usize,
        col: usize,
        cwd: Option<&std::path::Path>,
    ) -> Option<Hyperlink> {
        self.find_hyperlink_candidate_at(row, col)?.resolve(cwd)
    }

    #[inline]
    fn is_blank(codepoint: u32) -> bool {
        codepoint == 0 || codepoint == b' ' as u32
    }

    #[inline]
    fn trim_col_range(row: &[Cell], start: usize, end_exclusive: usize) -> Option<(usize, usize)> {
        let end_exclusive = end_exclusive.min(row.len());
        if start >= end_exclusive {
            return None;
        }
        let slice = &row[start..end_exclusive];
        let first = slice.iter().position(|c| !Self::is_blank(c.codepoint))?;
        let last = slice.iter().rposition(|c| !Self::is_blank(c.codepoint))?;
        Some((start + first, start + last))
    }

    /// Smart double-click selection on `row`:
    /// - On a Powerline prompt line (` ~/path   main ± `), selects `~/path`
    ///   (or the command after the last `` if present and clicked).
    /// - On a command line starting with a shell prompt (`$ cd ...`, `% ...`, `# ...`, `❯ ...`),
    ///   selects the full command line excluding the leading `$ ` prompt (`cd ...`).
    /// - On any other line, selects the trimmed line content.
    pub fn select_smart_line(&mut self, row: usize, col: usize) -> String {
        if row >= self.rows + usize::from(self.scroll_fraction > 0.0) || self.cols == 0 {
            return String::new();
        }
        let r = self.display_row(row);

        // Check for Powerline segment separator U+E0B0 ('')
        let first_pl = r.iter().position(|c| c.codepoint == 0xE0B0);
        let last_pl = r.iter().rposition(|c| c.codepoint == 0xE0B0);
        if let (Some(f_pl), Some(l_pl)) = (first_pl, last_pl) {
            let after_pl = Self::trim_col_range(r, l_pl + 1, self.cols);
            let before_pl = Self::trim_col_range(r, 0, f_pl);
            let chosen = match (before_pl, after_pl) {
                (Some(b), Some(a)) => {
                    if col >= f_pl {
                        Some(a)
                    } else {
                        Some(b)
                    }
                }
                (Some(b), None) => Some(b),
                (None, Some(a)) => Some(a),
                (None, None) => None,
            };
            if let Some(( mut sc, ec)) = chosen {
                if matches!(
                    char::from_u32(r[sc].codepoint),
                    Some('$' | '%' | '#' | '❯' | '>' | '➜')
                ) && sc < ec
                    && Self::is_blank(r[sc + 1].codepoint)
                {
                    sc += 1;
                    while sc <= ec && Self::is_blank(r[sc].codepoint) {
                        sc += 1;
                    }
                }
                if sc <= ec {
                    self.set_selection(Some(Selection {
                        start: (row, sc),
                        end: (row, ec),
                    }));
                    return self.selected_text().unwrap_or_default();
                }
            }
        }

        let Some((mut start_col, end_col)) = Self::trim_col_range(r, 0, self.cols) else {
            self.set_selection(None);
            return String::new();
        };

        // Strip leading shell prompt symbol (`$ `, `% `, `# `, `❯ `, `> `, `➜ `)
        if matches!(
            char::from_u32(r[start_col].codepoint),
            Some('$' | '%' | '#' | '❯' | '>' | '➜')
        ) && start_col < end_col
            && Self::is_blank(r[start_col + 1].codepoint)
        {
            start_col += 1;
            while start_col <= end_col && Self::is_blank(r[start_col].codepoint) {
                start_col += 1;
            }
        }

        if start_col > end_col {
            self.set_selection(None);
            return String::new();
        }

        self.set_selection(Some(Selection {
            start: (row, start_col),
            end: (row, end_col),
        }));
        self.selected_text().unwrap_or_default()
    }

    /// Columns of the run of non-blank cells around `(row, col)`; the placeholder
    /// cell of a wide glyph belongs to the glyph.
    fn block_range(&self, row: usize, col: usize) -> Option<(usize, usize)> {
        if row >= self.rows + usize::from(self.scroll_fraction > 0.0) || col >= self.cols {
            return None;
        }
        let r = self.display_row(row);
        let filled = |c: usize| {
            !Self::is_blank(r[c].codepoint) || (c > 0 && r[c].codepoint == 0 && is_wide_cp(r[c - 1].codepoint))
        };
        if !filled(col) {
            return None;
        }
        let mut sc = col;
        while sc > 0 && filled(sc - 1) {
            sc -= 1;
        }
        let mut ec = col;
        while ec + 1 < r.len() && filled(ec + 1) {
            ec += 1;
        }
        Some((sc, ec))
    }

    /// Double-click: select the contiguous block of text at `(row, col)` and return
    /// it. On a blank cell, fall back to the smart line selection.
    pub fn select_block(&mut self, row: usize, col: usize) -> String {
        let Some((sc, ec)) = self.block_range(row, col) else {
            return self.select_smart_line(row, col);
        };
        self.set_selection(Some(Selection { start: (row, sc), end: (row, ec) }));
        self.selected_text().unwrap_or_default()
    }

    /// Whether the selection is exactly the block at `(row, col)`, so a further
    /// double-click there widens it to the line.
    pub fn is_block_selected(&self, row: usize, col: usize) -> bool {
        let (Some(sel), Some((sc, ec))) = (self.selection, self.block_range(row, col)) else {
            return false;
        };
        let line = self.abs_line(row);
        sel.ordered() == ((line, sc), (line, ec))
    }

    /// Select the entire visible line `row` (up to the last non-blank character)
    /// and return its text.
    pub fn select_line(&mut self, row: usize) -> String {
        if row >= self.rows + usize::from(self.scroll_fraction > 0.0) {
            return String::new();
        }
        let r = self.display_row(row);
        let last_non_blank = r
            .iter()
            .rposition(|c| c.codepoint != 0 && c.codepoint != b' ' as u32);
        let Some(end_col) = last_non_blank else {
            self.set_selection(None);
            return String::new();
        };
        self.set_selection(Some(Selection {
            start: (row, 0),
            end: (row, end_col),
        }));
        self.selected_text().unwrap_or_default()
    }

    /// Extract the currently selected text. Soft-wrapped lines are joined without
    /// a newline, so a long command or URL copies as one line.
    pub fn selected_text(&self) -> Option<String> {
        let sel = self.selection?;
        let ((l0, c0), (l1, c1)) = sel.ordered();
        let last_col = self.cols.saturating_sub(1);
        let mut out = String::new();
        for l in l0..=l1 {
            let Some(row) = self.line_cells(l) else { continue };
            let sc = if l == l0 { c0.min(last_col) } else { 0 };
            let ec = if l == l1 { c1.min(last_col) } else { last_col };
            let mut line = String::new();
            if sc <= ec {
                let mut skip_next = false;
                for cell in &row[sc..=ec] {
                    if skip_next {
                        skip_next = false;
                        continue;
                    }
                    skip_next = is_wide_cp(cell.codepoint);
                    if cell.codepoint == 0 {
                        line.push(' ');
                    } else {
                        self.push_cell_text(cell, &mut line);
                    }
                }
            }
            let wraps = l < l1 && self.line_wrapped(l);
            if wraps {
                out.push_str(&line);
            } else if l < l1 || ec == last_col {
                out.push_str(line.trim_end_matches(' '));
            } else {
                out.push_str(&line);
            }
            if l < l1 && !wraps {
                out.push('\n');
            }
        }
        let trimmed = out.trim_end_matches(' ').to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    }

    // ------------------------------------------------------------------
    // Find in terminal
    // ------------------------------------------------------------------

    /// Snapshot of the scrollback and screen for `SearchText::find`, so matching
    /// runs without holding the grid lock.
    pub fn search_text(&self) -> SearchText {
        let first = if self.alt_active { self.lines_pushed } else { self.lines_pushed - self.history };
        let last = self.lines_pushed + self.rows;
        let mut text = String::new();
        let mut rows = Vec::with_capacity(last - first);
        let mut columns = Vec::new();
        for l in first..last {
            let Some(cells) = self.line_cells(l) else { continue };
            let row_start = text.len();
            rows.push((row_start, l));
            let mut skip_next = false;
            for (col, cell) in cells.iter().enumerate() {
                if std::mem::take(&mut skip_next) { continue; }
                skip_next = is_wide_cp(cell.codepoint);
                let width = if skip_next { 2 } else { 1 };
                columns.push((text.len(), col, width));
                self.push_cell_text(cell, &mut text);
            }
            if !self.line_wrapped(l) {
                text.truncate(text.trim_end_matches(' ').len().max(row_start));
                while columns.last().is_some_and(|&(offset, _, _)| offset >= text.len()) { columns.pop(); }
                let col = columns.last().filter(|&&(offset, _, _)| offset >= row_start).map_or(0, |&(_, col, width)| col + width);
                columns.push((text.len(), col, 1));
                text.push('\n');
            }
        }
        SearchText { text, rows, columns }
    }

    /// Replace the search matches and focus the one closest to the bottom of the
    /// viewport (searching upwards from what the user is looking at).
    pub fn set_search_matches(&mut self, matches: Vec<Selection>) {
        let bottom = if self.rows > 0 { self.abs_line(self.rows - 1) } else { 0 };
        self.search_current = match matches.partition_point(|m| m.start.0 <= bottom) {
            0 if matches.is_empty() => None,
            0 => Some(0),
            n => Some(n - 1),
        };
        self.search_matches = matches;
        self.all_dirty = true;
        self.reveal_search_current();
    }

    pub fn clear_search(&mut self) {
        if !self.search_matches.is_empty() {
            self.search_matches = Vec::new();
            self.all_dirty = true;
        }
        self.search_current = None;
    }

    /// Focus the previous (`up`, older) or next match, wrapping around.
    pub fn search_step(&mut self, up: bool) {
        let n = self.search_matches.len();
        let Some(cur) = self.search_current.filter(|_| n > 0) else { return };
        self.search_current = Some(if up { (cur + n - 1) % n } else { (cur + 1) % n });
        self.all_dirty = true;
        self.reveal_search_current();
    }

    /// `(focused index, match count)`.
    pub fn search_status(&self) -> (Option<usize>, usize) {
        (self.search_current, self.search_matches.len())
    }

    /// Scroll the viewport so the focused match is visible (centered when it was not).
    fn reveal_search_current(&mut self) {
        let Some(m) = self.search_current.and_then(|i| self.search_matches.get(i)) else { return };
        if self.alt_active {
            return;
        }
        let line = m.start.0;
        let top = self.lines_pushed - self.display_offset;
        if line < top || line >= top + self.rows {
            let offset = (self.lines_pushed + self.rows / 2).saturating_sub(line).min(self.history);
            if offset != self.display_offset || self.scroll_fraction != 0.0 {
                self.display_offset = offset;
                self.scroll_fraction = 0.0;
                self.all_dirty = true;
            }
        }
    }

    /// Matches intersecting visible row `row`, for per-cell highlighting.
    pub fn search_hits(&self, row: usize) -> SearchHits<'_> {
        let line = self.abs_line(row);
        let lo = self.search_matches.partition_point(|m| m.end.0 < line);
        let hi = self.search_matches.partition_point(|m| m.start.0 <= line).max(lo);
        SearchHits {
            line,
            current: self.search_current.and_then(|c| c.checked_sub(lo)),
            hits: &self.search_matches[lo..hi],
        }
    }

    // ------------------------------------------------------------------
    // Primitive edits (all in place, no allocation)
    // ------------------------------------------------------------------

    /// The cell the last printed character went into (cursor row), if any.
    fn previous_cell(&mut self) -> Option<(usize, usize)> {
        let row = self.cur.row;
        let col = if self.wrap_pending { self.cur.col } else { self.cur.col.checked_sub(1)? };
        let r = self.row_mut(row);
        // Step back over the placeholder half of a double-width character.
        if r[col].codepoint == 0 && col > 0 && is_wide_cp(r[col - 1].codepoint) {
            return Some((row, col - 1));
        }
        Some((row, col))
    }

    /// Visual order of display row `r` (see [`crate::bidi::visual_order`]),
    /// with `chars` and `levels` as scratch space. False when the row has no
    /// right-to-left text, or on the alternate screen: full-screen
    /// applications lay out their own bidirectional text.
    pub fn bidi_order(&self, r: usize, chars: &mut Vec<char>, levels: &mut Vec<u8>, order: &mut Vec<u16>) -> bool {
        order.clear();
        let row = self.display_row(r);
        // Nothing below the Hebrew block is right-to-left.
        if self.alt_active
            || !row.iter().any(|c| {
                let cp = c.codepoint;
                if cp & (CLUSTER_FLAG | PH_FLAG) == 0 {
                    cp >= 0x0590
                } else if cp & (CLUSTER_FLAG | PH_FLAG) == CLUSTER_FLAG {
                    self.cluster(cp).is_some_and(|text| text.chars().any(|ch| ch >= '\u{0590}'))
                } else {
                    false
                }
            })
        {
            return false;
        }
        chars.clear();
        chars.extend(row.iter().map(|c| if c.codepoint == 0 { '\0' } else { self.cell_char(c) }));
        crate::bidi::visual_order(chars, levels, order)
    }


    /// The logical column drawn at visual column `col` of display row `r`.
    pub fn logical_col(&self, r: usize, col: usize) -> usize {
        if r >= self.rows + usize::from(self.scroll_fraction > 0.0) {
            return col;
        }
        BIDI_SCRATCH.with(|scratch| {
            let (chars, levels, order) = &mut *scratch.borrow_mut();
            if self.bidi_order(r, chars, levels, order) {
                order.get(col).map_or(col, |&l| l as usize)
            } else {
                col
            }
        })
    }

    /// The visual column the cursor at logical column `col` of display row
    /// `r` is drawn at. After left-to-right text the cursor stays right after
    /// that text, even when the cell under it starts a right-to-left run
    /// (drawn at the run's right end): typing `echo '` before a right-to-left
    /// autosuggestion keeps the cursor next to the quote.
    pub fn cursor_visual_col(&self, r: usize, col: usize) -> usize {
        if r >= self.rows + usize::from(self.scroll_fraction > 0.0) {
            return col;
        }
        BIDI_SCRATCH.with(|scratch| {
            let (chars, levels, order) = &mut *scratch.borrow_mut();
            if self.bidi_order(r, chars, levels, order) {
                let visual = |l: usize| order.iter().position(|&o| o as usize == l);
                if let Some(prev) = col.checked_sub(1).filter(|&p| levels.get(p) == Some(&0))
                    && let Some(v) = visual(prev).map(|v| v + 1).filter(|&v| v < order.len()) {
                        return v;
                    }
                visual(col).unwrap_or(col)
            } else {
                col
            }
        })
    }

    /// The grapheme cluster text of a cell's `codepoint`, if it holds one.
    pub fn cluster(&self, cp: u32) -> Option<&str> {
        if cp & (CLUSTER_FLAG | PH_FLAG) != CLUSTER_FLAG {
            return None;
        }
        self.clusters.get((cp & CLUSTER_ID) as usize).map(|s| &**s)
    }

    /// A cell's character for searching and link detection: a cluster's first
    /// one, blank for empty cells.
    pub fn cell_char(&self, cell: &Cell) -> char {
        match self.cluster(cell.codepoint) {
            Some(text) => text.chars().next().unwrap_or(' '),
            None => char::from_u32(cell.codepoint).filter(|&c| c != '\0').unwrap_or(' '),
        }
    }

    /// Append a cell's text (the whole cluster) to `out`.
    pub fn push_cell_text(&self, cell: &Cell, out: &mut String) {
        match self.cluster(cell.codepoint) {
            Some(text) => out.push_str(text),
            None => out.push(self.cell_char(cell)),
        }
    }

    /// Intern a grapheme cluster string and return its ID (`0..MAX_CLUSTERS`),
    /// garbage-collecting unreferenced clusters if `MAX_CLUSTERS` is reached.
    fn intern_cluster(&mut self, text: &str) -> Option<u32> {
        if text.len() > MAX_CLUSTER_BYTES {
            return None;
        }
        if let Some(&id) = self.cluster_ids.get(text) {
            return Some(id);
        }
        if self.clusters.len() >= MAX_CLUSTERS {
            self.gc_clusters();
            if let Some(&id) = self.cluster_ids.get(text) {
                return Some(id);
            }
            if self.clusters.len() >= MAX_CLUSTERS {
                return None;
            }
        }
        let id = self.clusters.len() as u32;
        let boxed: Box<str> = text.into();
        self.clusters.push(boxed.clone());
        self.cluster_ids.insert(boxed, id);
        Some(id)
    }

    #[cfg(test)]
    pub(crate) fn cluster_count(&self) -> usize {
        self.clusters.len()
    }

    #[cfg(test)]
    pub(crate) fn seed_dead_clusters_for_bench(&mut self, count: usize) {
        self.clusters.clear();
        self.cluster_ids.clear();
        self.clusters.reserve(count);
        self.cluster_ids.reserve(count);
        for i in 0..count {
            let s: Box<str> = format!("dead_cluster_{i}").into_boxed_str();
            self.clusters.push(s.clone());
            self.cluster_ids.insert(s, i as u32);
        }
    }

    /// Garbage-collect unreferenced grapheme clusters in `self.clusters` and
    /// `self.cluster_ids`, compacting the table and remapping `CLUSTER_ID` on all
    /// active cells (`0..history + rows` and `alt`).
    pub(crate) fn gc_clusters(&mut self) {
        let total = self.clusters.len();
        if total == 0 {
            return;
        }
        self.cluster_evictions = 0;
        let mut used = std::mem::take(&mut self.cluster_marks);
        used.clear();
        used.resize(total, false);
        let mut used_count = 0usize;

        let mut mark_slice = |slice: &[Cell]| {
            for cell in slice {
                let cp = cell.codepoint;
                if cp & (CLUSTER_FLAG | PH_FLAG) == CLUSTER_FLAG {
                    let id = (cp & CLUSTER_ID) as usize;
                    if let Some(u) = used.get_mut(id)
                        && !*u {
                            *u = true;
                            used_count += 1;
                        }
                }
            }
        };

        let c = self.cols;
        if c > 0 && self.cap > 0 {
            let active_rows = (self.history + self.rows).min(self.cap);
            let start_row = (self.top + self.cap - self.history) % self.cap;
            if active_rows == self.cap {
                mark_slice(&self.cells);
            } else if start_row + active_rows <= self.cap {
                let s = start_row * c;
                let e = (start_row + active_rows) * c;
                mark_slice(&self.cells[s..e]);
            } else {
                let s1 = start_row * c;
                let s2 = (start_row + active_rows - self.cap) * c;
                mark_slice(&self.cells[s1..]);
                mark_slice(&self.cells[..s2]);
            }
        }
        if !self.alt.is_empty() {
            mark_slice(&self.alt);
        }

        if used_count == 0 {
            self.clusters.clear();
            self.cluster_ids.clear();
            self.cluster_marks = used;
            return;
        }
        if used_count == total {
            self.cluster_marks = used;
            return;
        }

        let mut remap = std::mem::take(&mut self.cluster_remap);
        remap.clear();
        remap.resize(total, 0);
        self.cluster_ids.clear();
        let mut old_id = 0;
        let mut new_id = 0;
        self.clusters.retain(|text| {
            let live = used[old_id];
            if live {
                remap[old_id] = new_id;
                self.cluster_ids.insert(text.clone(), new_id);
                new_id += 1;
            }
            old_id += 1;
            live
        });

        let remap_slice = |slice: &mut [Cell]| {
            for cell in slice {
                let cp = cell.codepoint;
                if cp & (CLUSTER_FLAG | PH_FLAG) == CLUSTER_FLAG {
                    let old_id = (cp & CLUSTER_ID) as usize;
                    if let Some(&new_id) = remap.get(old_id) {
                        cell.codepoint = (cp & !CLUSTER_ID) | new_id;
                    }
                }
            }
        };

        if c > 0 && self.cap > 0 {
            let active_rows = (self.history + self.rows).min(self.cap);
            let start_row = (self.top + self.cap - self.history) % self.cap;
            if active_rows == self.cap {
                remap_slice(&mut self.cells);
            } else if start_row + active_rows <= self.cap {
                let s = start_row * c;
                let e = (start_row + active_rows) * c;
                remap_slice(&mut self.cells[s..e]);
            } else {
                let s1 = start_row * c;
                let s2 = (start_row + active_rows - self.cap) * c;
                remap_slice(&mut self.cells[s1..]);
                remap_slice(&mut self.cells[..s2]);
            }
        }
        if !self.alt.is_empty() {
            remap_slice(&mut self.alt);
        }
        self.cluster_marks = used;
        self.cluster_remap = remap;
    }

    /// Append `c` to the grapheme in cell `(row, col)`, turning it into a
    /// cluster. `widen` makes a narrow one double-width (emoji presentation)
    /// when the cell after it is where the cursor is. False if it could not.
    fn append_to_cell(&mut self, row: usize, col: usize, c: char, widen: bool) -> bool {
        let cp = self.row_mut(row)[col].codepoint;
        let mut text = std::mem::take(&mut self.cluster_buf);
        text.clear();
        match self.cluster(cp) {
            Some(s) => text.push_str(s),
            None => match char::from_u32(cp).filter(|&b| b != '\0') {
                Some(base) => text.push(base),
                None => {
                    self.cluster_buf = text;
                    return false;
                }
            },
        }
        text.push(c);
        let id = self.intern_cluster(text.as_str());
        self.cluster_buf = text;
        let Some(id) = id else { return false };
        let was_wide = is_wide_cp(cp);
        let widen = widen && !was_wide && !self.wrap_pending && self.cur.col == col + 1 && col + 1 < self.cols;
        let r = self.row_mut(row);
        r[col].codepoint = CLUSTER_FLAG | if was_wide || widen { CLUSTER_WIDE } else { 0 } | id;
        if widen {
            // The right half takes the cell the cursor is on.
            r[col + 1] = Cell { codepoint: 0, ..r[col] };
            if col + 2 >= self.cols {
                self.cur.col = self.cols - 1;
                self.wrap_pending = self.autowrap;
            } else {
                self.cur.col = col + 2;
            }
        }
        self.mark(row);
        true
    }

    /// Whether the shell's line editor owns the cursor line (OSC 133 B..C on
    /// the main screen). It counts columns codepoint by codepoint with its own
    /// width table, so cells must follow that count there.
    fn in_line_editor(&self) -> bool {
        self.prompt_input_mark.is_some() && !self.alt_active
    }

    /// A double-width cell for `c`, a pictograph that is narrow by default
    /// (🏳, 🌡, 👁): zsh counts every codepoint in U+1F300..U+1F6FF as two
    /// columns. None if the cluster table is full.
    fn wide_pictograph(&mut self, c: char) -> Option<u32> {
        let mut buf = [0u8; 4];
        let text: &str = c.encode_utf8(&mut buf);
        let id = self.intern_cluster(text)?;
        Some(CLUSTER_FLAG | CLUSTER_WIDE | id)
    }

    /// The cell value and width `c` is printed with, or None when it joins
    /// the previous cell (combining mark, grapheme cluster).
    fn char_cell(&mut self, c: char) -> Option<(u32, usize)> {
        if self.grapheme_mode && !c.is_ascii() && self.extend_grapheme(c) {
            return None;
        }
        let mut w = char_width(c);
        if w == 0 {
            self.combine_with_previous(c);
            return None;
        }
        let mut codepoint = if c == PLACEHOLDER { PH_FLAG } else { c as u32 };
        if w == 1 && (0x1F300..=0x1F6FF).contains(&codepoint) && self.cols >= 2 && self.in_line_editor()
            && let Some(wide) = self.wide_pictograph(c) {
                (codepoint, w) = (wide, 2);
            }
        if w == 2 && self.cols < 2 {
            w = 1;
        }
        Some((codepoint, w))
    }

    /// Write a `w`-column character at the cursor and advance it.
    #[inline(always)]
    fn put_char(&mut self, codepoint: u32, w: usize) {
        if self.wrap_pending {
            self.wrap_pending = false;
            if self.autowrap {
                self.set_wrapped(self.cur.row, true);
                self.cur.col = 0;
                self.linefeed();
            }
        }
        if w == 2 && self.cur.col + 1 >= self.cols {
            if !self.autowrap {
                return;
            }
            self.clear_cols(self.cur.row, self.cur.col, self.cols);
            self.set_wrapped(self.cur.row, true);
            self.cur.col = 0;
            self.linefeed();
        }
        let (fg, bg) = self.colors();
        let (row, col) = (self.cur.row, self.cur.col);
        let blank = self.blank();
        let cols = self.cols;
        let insert = self.insert_mode;
        let prev_extra = {
            let r = self.row_mut(row);
            if insert && col + w < cols {
                r.copy_within(col..cols - w, col + w);
            }
            let p_prev = if col > 0 && r[col].codepoint == 0 && is_wide_cp(r[col - 1].codepoint) {
                let e = r[col - 1].extra();
                r[col - 1] = blank;
                e
            } else {
                0
            };
            let last = col + w - 1;
            let p_next = if last + 1 < cols && r[last + 1].codepoint == 0 && is_wide_cp(r[last].codepoint) {
                let e = r[last + 1].extra();
                r[last + 1] = blank;
                e
            } else {
                0
            };
            let p_curr = r[col].extra();
            r[col] = Cell { codepoint, fg, bg };
            let p_w2 = if w == 2 {
                let e = r[col + 1].extra();
                r[col + 1] = Cell { codepoint: 0, fg, bg };
                e
            } else {
                0
            };
            p_prev != 0 || p_next != 0 || p_curr != 0 || p_w2 != 0
        };
        if prev_extra {
            self.extras_dirty = true;
        }
        if col + w >= cols {
            self.set_wrapped(row, false);
        }
        self.mark(row);
        let next = col + w;
        if next >= self.cols {
            self.cur.col = self.cols - 1;
            self.wrap_pending = self.autowrap;
        } else {
            self.cur.col = next;
        }
    }

    /// Mode 2027: join `c` to the grapheme before the cursor when it continues
    /// it (UAX #29 emoji rules: ZWJ sequences, VS16, skin tones, flags, tags).
    fn extend_grapheme(&mut self, c: char) -> bool {
        let joiner = std::mem::take(&mut self.joiner);
        let cp = c as u32;
        let is_ri = |cp: u32| (0x1F1E6..=0x1F1FF).contains(&cp);
        // Most characters never continue a grapheme: rule them out before
        // looking at the previous cell.
        let may_join = matches!(cp, 0x200D | 0xFE0F | 0xE0020..=0xE007F | 0x1F3FB..=0x1F3FF)
            || is_ri(cp)
            || (joiner && is_pictographic(cp));
        if !may_join {
            return false;
        }
        let Some((row, col)) = self.previous_cell() else { return false };
        let prev = self.row_mut(row)[col].codepoint;
        if prev == 0 || prev & PH_FLAG != 0 {
            return false;
        }
        let base = self.cluster(prev).and_then(|t| t.chars().next()).map_or(prev, |b| b as u32);
        let keycap = matches!(base, 0x23 | 0x2A | 0x30..=0x39);
        let joins = match cp {
            0x200D | 0xFE0F | 0xE0020..=0xE007F => true,
            0x1F3FB..=0x1F3FF => is_pictographic(base),
            // A flag is two regional indicators; a third starts the next flag.
            _ if is_ri(cp) => is_ri(prev),
            _ => joiner && is_pictographic(cp),
        };
        // While the shell edits the command line (OSC 133 B..C) its line editor
        // counts columns with the system wcwidth, codepoint by codepoint: only
        // zero-width ones may join, and nothing widens, or its cursor drifts.
        let line_editor = self.in_line_editor();
        if !joins || (line_editor && char_width(c) > 0) {
            return false;
        }
        let widen = !line_editor && ((cp == 0xFE0F && (is_pictographic(base) || keycap)) || is_ri(cp));
        if !self.append_to_cell(row, col, c, widen) {
            return false;
        }
        self.joiner = cp == 0x200D;
        true
    }

    /// Fold a combining mark (e.g. the NFD accents in "Việt") into the previously
    /// printed character: its precomposed form when there is one, else a cluster.
    fn combine_with_previous(&mut self, mark: char) {
        let Some((row, col)) = self.previous_cell() else { return };
        let r = self.row_mut(row);
        let cp = r[col].codepoint;
        if cp & PH_FLAG != 0 {
            // Placeholder diacritics: row, then column, then the image id's high byte.
            let n = (cp >> 27) & 3;
            if let (true, Ok(i)) = (n < 3, crate::graphics::PLACEHOLDER_DIACRITICS.binary_search(&(mark as u32))) {
                let shift = [9, 0, 18][n as usize];
                r[col].codepoint = (cp & !(3 << 27)) | (i as u32) << shift | (n + 1) << 27;
                self.mark(row);
            }
            return;
        }
        if self.cluster(cp).is_none() {
            let Some(base) = char::from_u32(cp).filter(|&c| c != '\0') else { return };
            if let Some(composed) = unicode_normalization::char::compose(base, mark) {
                self.row_mut(row)[col].codepoint = composed as u32;
                self.mark(row);
                return;
            }
        }
        self.append_to_cell(row, col, mark, false);
    }

    #[inline]
    fn mark(&mut self, r: usize) {
        if let Some(d) = self.dirty.get_mut(r) {
            *d = true;
        }
    }

    #[inline]
    fn blank(&self) -> Cell {
        Cell { codepoint: 0, fg: 0, bg: self.cur.pen.bg }
    }

    fn clear_cols(&mut self, r: usize, c0: usize, c1: usize) {
        let b = self.blank();
        let c1 = c1.min(self.cols);
        if c0 < c1 {
            let had_extra = self.row_mut(r)[c0..c1]
                .iter()
                .any(|c| ((c.fg & FG_EXTRA) | (c.bg & BG_EXTRA)) != 0);
            if had_extra {
                self.extras_dirty = true;
            }
            self.row_mut(r)[c0..c1].fill(b);
        }
        if c1 == self.cols {
            // The wrap marker lives in the last column; erasing it ends the wrap.
            self.set_wrapped(r, false);
        }
        self.mark(r);
    }

    fn clear_rows(&mut self, r0: usize, r1: usize) {
        for r in r0..r1.min(self.rows) {
            self.clear_cols(r, 0, self.cols);
        }
    }

    fn copy_row(&mut self, src: usize, dst: usize) {
        let s = self.row_start(src);
        let d = self.row_start(dst);
        let c = self.cols;
        let buf = self.buf_mut();
        if buf[d..d + c].iter().any(|cell| ((cell.fg & FG_EXTRA) | (cell.bg & BG_EXTRA)) != 0) {
            self.extras_dirty = true;
        }
        self.buf_mut().copy_within(s..s + c, d);
        if !self.alt_active {
            self.wrapped[d / c] = self.wrapped[s / c];
        }
        self.mark(dst);
    }

    fn grow_capacity(&mut self) {
        let max_cap = self.scrollback + self.rows;
        if self.cap >= max_cap {
            return;
        }
        let new_cap = (self.cap * 4).max(512).min(max_cap);
        let mut new_cells = bytemuck::zeroed_vec::<Cell>(self.cols * new_cap);
        let mut new_wrapped = vec![false; new_cap];
        let n_lines = self.history + self.rows;
        let w = self.cols;
        for i in 0..n_lines {
            let op = (self.top + self.cap - self.history + i) % self.cap;
            let src = op * self.cols;
            new_cells[i * self.cols..i * self.cols + w].copy_from_slice(&self.cells[src..src + w]);
            new_wrapped[i] = self.wrapped[op];
        }
        self.cells = new_cells;
        self.wrapped = new_wrapped;
        self.cap = new_cap;
        self.top = self.history;
    }

    /// Scroll rows `t..=b` up by `n`. When the region is the whole main screen
    /// and `to_history` is set, this is an O(1) ring rotation into scrollback.
    fn scroll_up_in(&mut self, t: usize, b: usize, n: usize, to_history: bool) {
        if t > b || b >= self.rows {
            return;
        }
        let n = n.min(b - t + 1);
        if n == 0 {
            return;
        }
        if to_history && !self.alt_active && t == 0 && b == self.rows - 1 {
            let mut evicted_had_ph = false;

            for _ in 0..n {
                if self.history + self.rows >= self.cap && self.cap < self.scrollback + self.rows {
                    self.grow_capacity();
                }
                if self.history == self.cap - self.rows {
                    let evicted_row = (self.top + self.cap - self.history) % self.cap;
                    let start = evicted_row * self.cols;
                    let end = start + self.cols;
                    if !self.graphics.virtuals.is_empty() && self.cells[start..end].iter().any(|c| c.codepoint & PH_FLAG != 0) {
                        evicted_had_ph = true;
                    }
                    if self.clusters.len() >= 4096 && self.cells[start..end].iter().any(|c| c.codepoint & (CLUSTER_FLAG | PH_FLAG) == CLUSTER_FLAG) {
                        self.cluster_evictions = self.cluster_evictions.saturating_add(1);
                    }
                    if self.cells[start..end].iter().any(|c| ((c.fg & FG_EXTRA) | (c.bg & BG_EXTRA)) != 0) {
                        self.extras_dirty = true;
                    }
                }
                self.top = (self.top + 1) % self.cap;
                self.lines_pushed += 1;
                if self.history < self.cap - self.rows {
                    self.history += 1;
                }
                self.clear_cols(self.rows - 1, 0, self.cols);
            }
            if self.display_offset > 0 {
                // Keep the user's scrolled-back view anchored on the same lines.
                self.display_offset = (self.display_offset + n).min(self.history);
            }
            if !self.graphics.placements.is_empty() {
                self.graphics.prune(self.lines_pushed - self.history);
            }
            if evicted_had_ph {
                self.prune_dead_virtuals();
            }
            // Amortize a history scan across a history-sized batch, even when
            // every interned cluster is still live. MAX_CLUSTERS also triggers
            // demand collection so dead IDs never permanently exhaust the pool.
            if self.cluster_evictions >= self.history.max(self.rows).max(256) {
                self.gc_clusters();
            }
            self.all_dirty = true;
        } else {
            for r in t..(b + 1 - n) {
                self.copy_row(r + n, r);
            }
            self.clear_rows(b + 1 - n, b + 1);
            if self.alt_active && !self.graphics.placements.is_empty() {
                self.graphics.scroll_alt(self.lines_pushed + t, self.lines_pushed + b, n);
            }
        }
    }

    fn scroll_down_in(&mut self, t: usize, b: usize, n: usize) {
        if t > b || b >= self.rows {
            return;
        }
        let n = n.min(b - t + 1);
        for r in (t + n..=b).rev() {
            self.copy_row(r - n, r);
        }
        self.clear_rows(t, t + n);
        if self.alt_active && !self.graphics.placements.is_empty() {
            self.graphics.scroll_alt_down(self.lines_pushed + t, self.lines_pushed + b, n);
        }
    }

    /// Feed one byte from the PTY. APC strings (the Kitty graphics protocol)
    /// are handled here, because vte drops them; everything else goes to `parser`.
    #[inline]
    pub fn feed_byte(&mut self, parser: &mut vte::Parser, b: u8) {
        if let Some(len) = self.osc_len {
            match b {
                0x07 | 0x18 | 0x1a => {
                    self.osc_len = None;
                    self.osc_start = None;
                    if !self.osc_discard {
                        // VTE invokes OscEnd even for CAN/SUB. Suppress that
                        // callback so cancellation cannot commit a prefix.
                        self.osc_discard = b != 0x07;
                        parser.advance(self, b);
                    }
                    self.osc_discard = false;
                    return;
                }
                0x1b => {
                    // Existing ESC handling forwards ESC + the next byte to
                    // VTE, preserving ST and transitions to another sequence.
                    self.osc_len = None;
                    self.osc_start = None;
                    self.osc_discard = false;
                }
                _ => {
                    if !self.osc_discard {
                        let timed_out = len & 0x03ff == 0 && self.osc_start.is_some_and(|t| t.elapsed() > std::time::Duration::from_secs(5));
                        if len >= MAX_OSC_BYTES || timed_out {
                            // Cancel before dispatch, then consume the rest of
                            // the string. A partial OSC 52 must never succeed.
                            self.osc_discard = true;
                            parser.advance(self, 0x18);
                        } else {
                            self.osc_len = Some(len + 1);
                            parser.advance(self, b);
                        }
                    }
                    return;
                }
            }
        }

        if self.apc_state >= 2
            && (self.apc_buf.len() & 0x03ff == 0)
            && self.apc_start.is_some_and(|t| t.elapsed() > std::time::Duration::from_secs(5))
        {
            self.apc_state = 0;
            self.apc_buf.clear();
            self.apc_buf.shrink_to_fit();
            self.apc_start = None;
        }

        match (self.apc_state, b) {
            (0, 0x1b) => self.apc_state = 1,
            (0, _) => parser.advance(self, b),
            (1, b']') => {
                self.apc_state = 0;
                self.apc_start = None;
                parser.advance(self, 0x1b);
                parser.advance(self, b']');
                self.osc_len = Some(0);
                self.osc_discard = false;
                self.osc_start = Some(std::time::Instant::now());
            }
            (1, b'_') => {
                self.apc_state = 2;
                self.apc_buf.clear();
                self.apc_start = Some(std::time::Instant::now());
            }
            (1, _) => {
                self.apc_state = 0;
                self.apc_start = None;
                parser.advance(self, 0x1b);
                self.feed_byte(parser, b);
            }
            (2, 0x1b) => self.apc_state = 3,
            // CAN / SUB abort the string.
            (2, 0x18 | 0x1a) => {
                self.apc_state = 0;
                self.apc_buf.clear();
                self.apc_buf.shrink_to_fit();
                self.apc_start = None;
            }
            (2, _) => {
                if self.apc_buf.len() < MAX_IMAGE_STRING {
                    self.apc_buf.push(b);
                } else {
                    self.apc_state = 0;
                    self.apc_buf.clear();
                    self.apc_buf.shrink_to_fit();
                    self.apc_start = None;
                }
            }
            (_, b'\\') => {
                self.apc_state = 0;
                self.apc_start = None;
                self.apc_dispatch();
            }
            // Any other ESC sequence ends the string unfinished.
            (_, _) => {
                self.apc_state = 0;
                self.apc_buf.clear();
                self.apc_buf.shrink_to_fit();
                self.apc_start = None;
                parser.advance(self, 0x1b);
                self.feed_byte(parser, b);
            }
        }
    }

    fn linefeed(&mut self) {
        if self.cur.row == self.scroll_bot {
            self.scroll_up_in(self.scroll_top, self.scroll_bot, 1, true);
        } else if self.cur.row + 1 < self.rows {
            self.cur.row += 1;
        }
    }

    fn reverse_index(&mut self) {
        if self.cur.row == self.scroll_top {
            self.scroll_down_in(self.scroll_top, self.scroll_bot, 1);
        } else if self.cur.row > 0 {
            self.cur.row -= 1;
        }
    }

    /// Resolve the current pen into packed (fg, bg) cell colors.
    fn colors(&self) -> (u32, u32) {
        let p = self.cur.pen;
        let (mut fg, mut bg) = (p.fg, p.bg);
        if p.attrs & ATTR_INVERSE != 0 {
            let f = if fg == 0 { self.fg_color() } else { fg & !0xFF };
            let b = if bg == 0 { self.bg_color() } else { bg & !0xFF };
            fg = b | FG_SET;
            bg = f | BG_SET;
        }
        if p.attrs & ATTR_DIM != 0 {
            let rgb = if fg == 0 { self.fg_color() } else { fg & !0xFF };
            fg = ((rgb >> 1) & 0x7F7F_7F00) | FG_SET;
        }
        if p.attrs & ATTR_HIDDEN != 0 {
            fg = if bg == 0 { self.bg_color() | FG_SET } else { (bg & !0xFF) | FG_SET };
        }
        bg |= p.underline as u32;
        if p.attrs & ATTR_STRIKE != 0 {
            bg |= BG_STRIKE;
        }
        if p.attrs & ATTR_BOLD != 0 {
            fg |= FG_BOLD;
        }
        if p.attrs & ATTR_ITALIC != 0 {
            fg |= FG_ITALIC;
        }
        with_extra(fg, bg, p.extra)
    }

    fn tab_forward(&mut self, n: usize) {
        for _ in 0..n {
            let next = (self.cur.col + 1..self.cols).find(|&c| self.tab_stops[c]);
            self.cur.col = next.unwrap_or(self.cols - 1);
        }
    }

    fn tab_backward(&mut self, n: usize) {
        for _ in 0..n {
            let prev = (1..self.cur.col).rev().find(|&c| self.tab_stops[c]);
            self.cur.col = prev.unwrap_or(0);
        }
    }

    /// Move to a 1-based row parameter, honoring origin mode (DECOM).
    fn goto_row(&mut self, row_param: usize) {
        if self.cur.origin_mode {
            self.cur.row = (self.scroll_top + row_param - 1).min(self.scroll_bot);
        } else {
            self.cur.row = (row_param - 1).min(self.rows - 1);
        }
    }

    fn clamp_cursor(&mut self) {
        self.cur.row = self.cur.row.min(self.rows - 1);
        self.cur.col = self.cur.col.min(self.cols - 1);
    }

    fn set_alt(&mut self, on: bool, save_cursor: bool) {
        if on == self.alt_active {
            return;
        }
        if on {
            if save_cursor {
                self.saved_main = self.cur;
            }
            self.alt_active = true;
            self.cur.row = 0;
            self.cur.col = 0;
            self.alt = bytemuck::zeroed_vec(self.cols * self.rows);
            self.display_offset = 0;
            self.scroll_fraction = 0.0;
        } else {
            self.alt_active = false;
            if self.alt.iter().any(|cell| ((cell.fg & FG_EXTRA) | (cell.bg & BG_EXTRA)) != 0) {
                self.extras_dirty = true;
            }
            self.alt = Vec::new();
            if save_cursor {
                self.cur = self.saved_main;
                self.clamp_cursor();
            }
            self.graphics.placements.retain(|p| !p.alt);
            self.graphics.virtuals.retain(|v| !v.alt);
            self.graphics.prune_dead_auto_ids();
        }
        self.wrap_pending = false;
        self.selection = None;
        self.all_dirty = true;
    }

    fn full_reset(&mut self) {
        self.set_alt(false, false);
        self.image_epoch = self.image_epoch.wrapping_add(1);
        self.graphics.clear();
        self.cur = Cursor::default();
        self.saved = Cursor::default();
        self.scroll_top = 0;
        self.scroll_bot = self.rows - 1;
        self.wrap_pending = false;
        self.cursor_visible = true;
        self.cursor_shape = 0;
        self.app_cursor = false;
        self.bracketed_paste = false;
        self.autowrap = true;
        self.insert_mode = false;
        self.sync_output = false;
        self.sync_started = None;
        self.mouse_tracking = MouseTracking::Off;
        self.mouse_sgr = false;
        self.focus_events = false;
        self.alt_scroll = true;
        self.mouse_sgr_pixels = false;
        self.color_scheme_reports = false;
        self.grapheme_mode = GRAPHEME_DEFAULT.load(Ordering::Relaxed);
        self.joiner = false;
        self.kitty_keyboard = [KittyKeyboard::default(); 2];
        self.modify_other_keys = 0;
        self.progress = None;
        self.pointer_shape.clear();
        self.title_changed = true;
        self.palette_override = Vec::new();
        self.color_override = [0; 3];
        self.tab_stops = default_tab_stops(self.cols);
        self.selection = None;
        self.clusters.clear();
        self.cluster_ids.clear();
        self.clear_rows(0, self.rows);
        self.trim_extras();
        self.apc_state = 0;
        self.apc_buf.clear();
        self.apc_buf.shrink_to_fit();
        self.apc_start = None;
        self.dcs = None;
        self.dcs_buf.clear();
        self.dcs_buf.shrink_to_fit();
        self.dcs_start = None;
        self.all_dirty = true;
    }

    // ------------------------------------------------------------------
    // Resize (the only place the grid allocates after startup)
    // ------------------------------------------------------------------

    pub fn resize(&mut self, cols: usize, rows: usize) {
        let cols = cols.max(1);
        let rows = rows.max(1);
        if cols == self.cols && rows == self.rows {
            return;
        }
        // Reflow renumbers lines; the caller re-runs an open search.
        self.clear_search();

        // 1. Main screen + scrollback -> logical lines (soft wraps joined), tracking
        //    the main cursor as (line, offset).
        let main_cur = if self.alt_active { self.saved_main } else { self.cur };
        let total = self.history + self.rows;
        let cursor_row = self.history + main_cur.row.min(self.rows - 1);
        let remap = !self.prompts.is_empty() || self.graphics.placements.iter().any(|p| !p.alt);
        let mut row_pos: Vec<(usize, usize)> = Vec::with_capacity(if remap { total } else { 0 });

        let mut logical_cells: Vec<Cell> = Vec::with_capacity(total * self.cols);
        let mut line_spans: Vec<std::ops::Range<usize>> = Vec::with_capacity(total);
        let mut current_start = 0;
        let mut cursor = (0, 0);
        for i in 0..total {
            let p = (self.top + self.cap - self.history + i) % self.cap;
            let current_len = logical_cells.len() - current_start;
            if i == cursor_row {
                cursor = (line_spans.len(), current_len + main_cur.col);
            }
            if remap {
                row_pos.push((line_spans.len(), current_len));
            }
            logical_cells.extend_from_slice(&self.cells[p * self.cols..(p + 1) * self.cols]);
            if self.wrapped[p] && i + 1 < total {
                continue;
            }
            let keep = if line_spans.len() == cursor.0 && i >= cursor_row { cursor.1 } else { 0 };
            let current = &logical_cells[current_start..];
            let content = current.iter().rposition(|c| c.codepoint != 0 || c.bg & BG_SET != 0).map_or(0, |idx| idx + 1);
            let target_len = content.max(keep).min(current.len());
            logical_cells.truncate(current_start + target_len);
            line_spans.push(current_start..logical_cells.len());
            current_start = logical_cells.len();
        }
        // Blank lines below the cursor are not content worth keeping.
        while line_spans.len() > cursor.0 + 1 && line_spans.last().is_some_and(|r| r.is_empty()) {
            line_spans.pop();
        }

        // 2. Re-wrap every logical line to the new width.
        let mut out: Vec<Cell> = Vec::with_capacity(total * cols);
        let mut out_wrapped: Vec<bool> = Vec::with_capacity(total);
        let mut new_cursor = (0, 0);
        let mut line_rows: Vec<usize> = Vec::with_capacity(if remap { line_spans.len() + 1 } else { 0 });
        for (li, span) in line_spans.iter().enumerate() {
            let line = &logical_cells[span.clone()];
            let first_row = out_wrapped.len();
            if remap {
                line_rows.push(first_row);
            }
            out.resize(out.len() + cols, Cell::default());
            out_wrapped.push(false);
            let mut col = 0;
            let mut i = 0;
            while i < line.len() {
                let cell = line[i];
                let w = if is_wide_cp(cell.codepoint) { 2.min(cols) } else { 1 };
                if col + w > cols {
                    *out_wrapped.last_mut().unwrap() = true;
                    out.resize(out.len() + cols, Cell::default());
                    out_wrapped.push(false);
                    col = 0;
                }
                if li == cursor.0 && (i..i + w).contains(&cursor.1) {
                    new_cursor = (out_wrapped.len() - 1, col);
                }
                let base = out.len() - cols;
                out[base + col] = cell;
                if w == 2 {
                    out[base + col + 1] = Cell { codepoint: 0, ..cell };
                }
                col += w;
                i += if w == 2 && line.get(i + 1).is_some_and(|c| c.codepoint == 0) { 2 } else { 1 };
            }
            if li == cursor.0 && cursor.1 >= line.len() {
                let extra = cursor.1 - line.len();
                let row = (out_wrapped.len() - 1).max(first_row);
                new_cursor = (row, (col + extra).min(cols - 1));
            }
        }

        // Drop intermediate logical line storage before allocating new grid cells to reduce peak memory.
        drop(logical_cells);
        drop(line_spans);

        // 3. Keep the cursor on screen; everything above it becomes scrollback.
        let total_rows = out_wrapped.len().max(new_cursor.0 + 1);
        let screen_start = total_rows.saturating_sub(rows).min(new_cursor.0).max((new_cursor.0 + 1).saturating_sub(rows));
        let scrollback = scrollback_lines(cols);
        let new_hist = screen_start.min(scrollback);
        let first = screen_start - new_hist;
        let new_cap = (new_hist + rows).max(rows * 2).max(64).min(scrollback + rows);
        let mut cells = bytemuck::zeroed_vec::<Cell>(cols * new_cap);
        let mut wrapped = vec![false; new_cap];
        for (dst, src) in (first..(screen_start + rows).min(out_wrapped.len())).enumerate() {
            cells[dst * cols..(dst + 1) * cols].copy_from_slice(&out[src * cols..(src + 1) * cols]);
            wrapped[dst] = out_wrapped[src];
        }

        // 4. Move prompt marks and images from old absolute lines to the reflowed ones.
        let new_lines_pushed = self.lines_pushed.max(new_hist);
        if remap {
            line_rows.push(out_wrapped.len());
            let old_first = self.first_line();
            let map_row = |i: usize| -> Option<usize> {
                let &(li, off) = row_pos.get(i)?;
                let (start, end) = (*line_rows.get(li)?, *line_rows.get(li + 1)?);
                let row = (start + off / cols).min(end.saturating_sub(1).max(start));
                (row >= first).then(|| new_lines_pushed + row - screen_start)
            };
            let map_line = |l: usize| -> Option<usize> {
                let i = l.checked_sub(old_first)?;
                // An output end may be the line after the last one.
                if i >= total && i > 0 { map_row(total - 1).map(|r| r + i - total + 1) } else { map_row(i) }
            };
            self.prompts.retain_mut(|m| {
                let Some(prompt) = map_line(m.prompt) else { return false };
                m.prompt = prompt;
                m.output = m.output.and_then(|(start, end)| Some((map_line(start)?, end.and_then(map_line))));
                true
            });
            self.graphics.placements.retain_mut(|p| {
                if p.alt {
                    return true;
                }
                let Some(line) = map_line(p.line) else { return false };
                p.line = line;
                true
            });
            self.graphics.prune_dead_auto_ids();
        }

        let alt = if self.alt_active {
            let mut alt = bytemuck::zeroed_vec::<Cell>(cols * rows);
            let w = cols.min(self.cols);
            for r in 0..rows.min(self.rows) {
                if !self.alt.is_empty() {
                    alt[r * cols..r * cols + w].copy_from_slice(&self.alt[r * self.cols..r * self.cols + w]);
                }
            }
            alt
        } else {
            Vec::new()
        };

        self.cells = cells;
        self.wrapped = wrapped;
        self.alt = alt;
        self.cap = new_cap;
        self.scrollback = scrollback;
        self.top = new_hist;
        self.history = new_hist;
        self.lines_pushed = new_lines_pushed;
        self.display_offset = 0;
        self.scroll_fraction = 0.0;
        let main_row = new_cursor.0 - screen_start;
        if self.alt_active {
            self.saved_main.row = main_row;
            self.saved_main.col = new_cursor.1;
        } else {
            self.cur.row = main_row;
            self.cur.col = new_cursor.1;
        }
        self.cols = cols;
        self.rows = rows;
        self.clamp_cursor();
        for c in [&mut self.saved, &mut self.saved_main] {
            c.row = c.row.min(rows - 1);
            c.col = c.col.min(cols - 1);
        }
        self.tab_stops = default_tab_stops(cols);
        self.scroll_top = 0;
        self.scroll_bot = rows - 1;
        self.wrap_pending = false;
        self.selection = None;
        self.prompt_input_mark = None;
        self.dirty = vec![true; rows];
        self.all_dirty = true;
        self.extras_dirty = true;
        if !self.graphics.virtuals.is_empty() {
            self.prune_dead_virtuals();
        }
    }

    /// Scrollback length and how far the view is scrolled back, in lines.
    pub fn scroll_position(&self) -> (usize, usize) {
        (self.history, self.display_offset)
    }

    /// First absolute line still in scrollback.
    fn first_line(&self) -> usize {
        self.lines_pushed - self.history
    }

    /// Scroll so absolute line `line` is at the top of the viewport (clamped).
    fn scroll_to_line(&mut self, line: usize) {
        let offset = self.lines_pushed.saturating_sub(line).min(self.history);
        if self.display_offset != offset || self.scroll_fraction != 0.0 {
            self.display_offset = offset;
            self.scroll_fraction = 0.0;
            self.all_dirty = true;
        }
    }

    /// Active Kitty keyboard protocol flags (`keymap::KITTY_*`) of the current screen.
    pub fn kitty_keyboard_flags(&self) -> u8 {
        self.kitty_keyboard[self.alt_active as usize].flags
    }

    // ------------------------------------------------------------------
    // Extra cell attributes (OSC 8 hyperlinks, SGR 58 underline colors)
    // ------------------------------------------------------------------

    fn extra_at(&self, idx: u8) -> Option<&CellExtra> {
        self.extras.get(idx as usize)?.as_ref()
    }

    /// Point the pen at `extra` with its underline color replaced.
    fn set_underline_color(&mut self, extra: u8, color: u32) {
        let link = self.extra_at(extra).and_then(|e| e.link.clone());
        let old = self.cur.pen.extra;
        self.cur.pen.extra = self.intern_extra(CellExtra { link, underline_color: color });
        if old != 0 && self.cur.pen.extra != old {
            self.extras_dirty = true;
        }
    }

    /// Point the pen at the current extra with its hyperlink replaced.
    fn set_link(&mut self, link: Option<OscLink>) {
        let underline_color = self.extra_at(self.cur.pen.extra).map_or(0, |e| e.underline_color);
        let old = self.cur.pen.extra;
        self.cur.pen.extra = self.intern_extra(CellExtra { link, underline_color });
        if old != 0 && self.cur.pen.extra != old {
            self.extras_dirty = true;
        }
    }

    /// Index of `extra` in the table, adding it if needed. Runs only when an escape
    /// sequence changes the pen, never per printed character. Returns 0 (plain text)
    /// if all 255 slots are still referenced by cells.
    fn intern_extra(&mut self, extra: CellExtra) -> u8 {
        if extra == CellExtra::default() {
            return 0;
        }
        if let Some(i) = self.extras.iter().position(|e| e.as_ref() == Some(&extra)) {
            return i as u8;
        }
        if self.extras.len() <= u8::MAX as usize {
            self.extras.push(Some(extra));
            return (self.extras.len() - 1) as u8;
        }
        let free = match self.extras.iter().skip(1).position(Option::is_none) {
            Some(i) => Some(i + 1),
            None => {
                if self.extras_full && !self.extras_dirty {
                    None
                } else {
                    self.collect_extras()
                }
            }
        };
        match free {
            Some(i) => {
                self.extras[i] = Some(extra);
                i as u8
            }
            None => 0,
        }
    }

    /// Free every extra slot no cell or saved pen refers to; returns the first free one.
    fn collect_extras(&mut self) -> Option<usize> {
        let mut used = [false; 256];
        let mut used_count = 0usize;
        let c = self.cols;
        if c > 0 && self.cap > 0 {
            let active_rows = (self.history + self.rows).min(self.cap);
            let start_row = (self.top + self.cap - self.history) % self.cap;

            let mut check_slice = |slice: &[Cell]| -> bool {
                for cell in slice {
                    if ((cell.fg & FG_EXTRA) | (cell.bg & BG_EXTRA)) != 0 {
                        let extra = cell.extra() as usize;
                        if extra != 0 && !used[extra] {
                            used[extra] = true;
                            used_count += 1;
                            if used_count >= 255 {
                                return true;
                            }
                        }
                    }
                }
                false
            };

            let all_in_use = if active_rows == self.cap {
                check_slice(&self.cells)
            } else if start_row + active_rows <= self.cap {
                let s = start_row * c;
                let e = (start_row + active_rows) * c;
                check_slice(&self.cells[s..e])
            } else {
                let s1 = start_row * c;
                let s2 = (start_row + active_rows - self.cap) * c;
                check_slice(&self.cells[s1..]) || check_slice(&self.cells[..s2])
            };

            if all_in_use {
                self.extras_full = true;
                self.extras_dirty = false;
                return None;
            }
        }

        for cell in &self.alt {
            if ((cell.fg & FG_EXTRA) | (cell.bg & BG_EXTRA)) != 0 {
                let extra = cell.extra() as usize;
                if extra != 0 && !used[extra] {
                    used[extra] = true;
                    used_count += 1;
                    if used_count >= 255 {
                        self.extras_full = true;
                        self.extras_dirty = false;
                        return None;
                    }
                }
            }
        }
        for c in [&self.cur, &self.saved, &self.saved_main] {
            let extra = c.pen.extra as usize;
            if extra != 0 && !used[extra] {
                used[extra] = true;
                used_count += 1;
                if used_count >= 255 {
                    self.extras_full = true;
                    self.extras_dirty = false;
                    return None;
                }
            }
        }

        for (slot, &is_used) in self.extras.iter_mut().zip(used.iter()).skip(1) {
            if !is_used {
                *slot = None;
            }
        }
        self.extras_dirty = false;
        let free = self.extras.iter().skip(1).position(Option::is_none).map(|i| i + 1);
        self.extras_full = free.is_none();
        free
    }

    /// Reclaim unused extra slots and shrink trailing empty entries.
    fn trim_extras(&mut self) {
        self.collect_extras();
        while self.extras.len() > 1 && self.extras.last().is_some_and(Option::is_none) {
            self.extras.pop();
        }
        self.extras_dirty = false;
        self.extras_full = false;
    }

    /// Explicit underline color of a cell as `0xRRGGBB00`, if SGR 58 set one.
    pub fn underline_color(&self, cell: &Cell) -> Option<u32> {
        match cell.extra() {
            0 => None,
            i => self.extra_at(i).map(|e| e.underline_color).filter(|&c| c != 0).map(|c| c & !0xFF),
        }
    }

    /// One-line description of the visible cell `(row, col)` for the inspector.
    pub fn describe_cell(&self, row: usize, col: usize) -> Option<String> {
        use std::fmt::Write as _;
        let cell = *self.display_row(row).get(col)?;
        let mut out = match (self.cluster(cell.codepoint), char::from_u32(cell.codepoint).filter(|_| cell.codepoint != 0)) {
            (Some(text), _) => {
                let cps: Vec<String> = text.chars().map(|c| format!("U+{:04X}", c as u32)).collect();
                format!("{} {text:?}", cps.join(" "))
            }
            (None, Some(c)) => format!("U+{:04X} {c:?}", cell.codepoint),
            (None, None) if cell.codepoint & PH_FLAG != 0 => "image placeholder".to_string(),
            (None, None) => "empty".to_string(),
        };
        let color = |v: u32, set: u32| if v & set != 0 { format!("#{:06x}", v >> 8) } else { "default".to_string() };
        let _ = write!(out, "  fg {}  bg {}", color(cell.fg, FG_SET), color(cell.bg, BG_SET));
        if cell.fg & FG_BOLD != 0 {
            out.push_str("  bold");
        }
        if cell.fg & FG_ITALIC != 0 {
            out.push_str("  italic");
        }
        let underline = ["", "underline", "double-underline", "curly-underline", "dotted-underline", "dashed-underline"];
        if let Some(name) = underline.get((cell.bg & BG_UNDERLINE_MASK) as usize).filter(|n| !n.is_empty()) {
            let _ = write!(out, "  {name}");
            if let Some(c) = self.underline_color(&cell) {
                let _ = write!(out, " #{:06x}", c >> 8);
            }
        }
        if cell.bg & BG_STRIKE != 0 {
            out.push_str("  strike");
        }
        if let Some(link) = self.cell_link(&cell) {
            let _ = write!(out, "  link {}", link.uri);
        }
        Some(out)
    }

    /// The OSC 8 link of a cell as `(id, uri)`.
    fn cell_link(&self, cell: &Cell) -> Option<&OscLink> {
        match cell.extra() {
            0 => None,
            i => self.extra_at(i)?.link.as_ref(),
        }
    }

    /// OSC 8: `8 ; params ; URI` starts a hyperlink, an empty URI ends it.
    fn osc8(&mut self, params: &[&[u8]]) {
        let uri = params.get(2..).unwrap_or_default();
        // The URI may itself contain ';', which vte splits into extra parameters.
        let len = uri.iter().map(|p| p.len() + 1).sum::<usize>().saturating_sub(1);
        let link = if len == 0 || len > MAX_LINK_LEN {
            None
        } else {
            let uri = uri.join(&b';');
            let id = params[1]
                .split(|&b| b == b':')
                .find_map(|kv| kv.strip_prefix(b"id="))
                .unwrap_or_default();
            match (std::str::from_utf8(&uri), std::str::from_utf8(id)) {
                (Ok(uri), Ok(id)) => Some(OscLink { id: id.into(), uri: uri.into() }),
                _ => None,
            }
        };
        self.set_link(link);
    }
}

/// Display width of a character: 0 (combining / zero-width), 1, or 2 (CJK / emoji),
/// following the Unicode East Asian Width and emoji presentation tables.
pub fn char_width(c: char) -> usize {
    use unicode_width::UnicodeWidthChar;
    // Regional indicators have emoji presentation: two columns each, as in
    // zsh, kitty and Ghostty (a flag pair joins into one two-column cluster).
    if (0x1F1E6..=0x1F1FF).contains(&(c as u32)) {
        return 2;
    }
    let w = c.width().unwrap_or(0).min(2);
    #[cfg(target_os = "macos")]
    if w == 1 && is_spacing_mark_range(c as u32) && macos_wcwidth(c) == Some(0) {
        return 0;
    }
    w
}

/// Scripts with spacing combining marks (Indic, Southeast Asian, Brahmic).
#[cfg(target_os = "macos")]
fn is_spacing_mark_range(cp: u32) -> bool {
    matches!(cp, 0x0900..=0x0DFF | 0x0F00..=0x109F | 0x1700..=0x1CFF | 0xA800..=0xABFF | 0x11000..=0x11FFF)
}

/// What macOS's own `wcwidth` says. Shells (zsh, bash, readline) count with it, so
/// for spacing marks such as Devanagari U+093F, which it treats as zero-width,
/// disagreeing would leave the line editor's cursor columns off.
#[cfg(target_os = "macos")]
fn macos_wcwidth(c: char) -> Option<usize> {
    use std::sync::OnceLock;
    unsafe extern "C" {
        fn wcwidth_l(wc: libc::wchar_t, loc: libc::locale_t) -> libc::c_int;
    }
    static LOCALE: OnceLock<usize> = OnceLock::new();
    // SAFETY: newlocale gets a valid NUL-terminated name and a null base; the
    // locale is never freed, so the pointer stays valid for wcwidth_l.
    let loc = *LOCALE.get_or_init(|| unsafe {
        libc::newlocale(libc::LC_CTYPE_MASK, c"en_US.UTF-8".as_ptr(), std::ptr::null_mut()) as usize
    });
    if loc == 0 {
        return None;
    }
    // SAFETY: loc is a live locale from newlocale.
    let w = unsafe { wcwidth_l(c as libc::wchar_t, loc as libc::locale_t) };
    usize::try_from(w).ok()
}

/// Smallest codepoint `char_width` reports as double-width (Hangul Jamo).
const FIRST_WIDE: u32 = 0x1100;

/// Whether a cell's codepoint is a double-width character. Called for every
/// printed character, so ASCII and other narrow text skip the width tables.
#[inline]
pub fn is_wide_cp(cp: u32) -> bool {
    if cp & (CLUSTER_FLAG | PH_FLAG) == CLUSTER_FLAG {
        return cp & CLUSTER_WIDE != 0;
    }
    // Fast-reject common narrow ranges above FIRST_WIDE (0x1100):
    // Vietnamese Latin Extended Additional (0x1EA0..=0x1EF9),
    // Box Drawing and Block Elements (0x2500..=0x259F).
    if (0x1EA0..=0x1EF9).contains(&cp) || (0x2500..=0x259F).contains(&cp) {
        return false;
    }
    cp >= FIRST_WIDE && char::from_u32(cp).is_some_and(|c| char_width(c) == 2)
}

/// Extended pictographic (emoji) codepoints that join a ZWJ sequence.
fn is_pictographic(cp: u32) -> bool {
    matches!(cp, 0x00A9 | 0x00AE | 0x203C | 0x2049 | 0x2122 | 0x2139 | 0x2194..=0x21AA | 0x231A..=0x23FF
        | 0x24C2 | 0x25AA..=0x25FE | 0x2600..=0x27BF | 0x2934 | 0x2935 | 0x2B05..=0x2B55 | 0x3030 | 0x303D
        | 0x3297 | 0x3299 | 0x1F000..=0x1FAFF)
}

/// What Cmd/Ctrl+Click on an OSC 8 link opens: `(target, is_path)`. `file://` URIs
/// become local paths; other URIs are only opened for a few well-known schemes, so
/// a program cannot make a click launch an arbitrary URL handler.
fn osc8_target(uri: &str) -> Option<(String, bool)> {
    if uri.starts_with("file://") {
        return parse_osc7_path(uri.as_bytes()).map(|path| (path, true));
    }
    let (scheme, _) = uri.split_once(':')?;
    const SCHEMES: [&str; 5] = ["http", "https", "ftp", "ftps", "mailto"];
    SCHEMES.iter().any(|s| s.eq_ignore_ascii_case(scheme)).then(|| (uri.to_string(), false))
}

/// Drop trailing sentence punctuation from a detected link `chars[start..end]`,
/// keeping a closing `)` / `]` that has a matching opener inside the link.
fn trim_trailing_punctuation(chars: &[char], start: usize, mut end: usize) -> usize {
    while end > start {
        let unbalanced = |close: char, open: char| chars[end - 1] == close && !chars[start..end].contains(&open);
        if matches!(chars[end - 1], '.' | ',' | ';' | ':' | '!' | '?' | '\'') || unbalanced(')', '(') || unbalanced(']', '[') {
            end -= 1;
        } else {
            break;
        }
    }
    end
}

fn default_tab_stops(cols: usize) -> Vec<bool> {
    (0..cols).map(|c| c > 0 && c % 8 == 0).collect()
}

/// DEC Special Graphics (ESC ( 0) used by tmux, mc, etc. for box drawing.
fn dec_line_drawing(c: char) -> char {
    match c {
        '`' => '◆', 'a' => '▒', 'f' => '°', 'g' => '±', 'j' => '┘', 'k' => '┐', 'l' => '┌',
        'm' => '└', 'n' => '┼', 'o' => '⎺', 'p' => '⎻', 'q' => '─', 'r' => '⎼', 's' => '⎽',
        't' => '├', 'u' => '┤', 'v' => '┴', 'w' => '┬', 'x' => '│', 'y' => '≤', 'z' => '≥',
        '{' => 'π', '|' => '≠', '}' => '£', '~' => '·',
        _ => c,
    }
}

#[cfg(test)]
mod tests;
