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
pub mod simd;

#[cfg(test)]
mod perf_probe;
#[cfg(test)]
mod ascii_run_tests;

mod colors;
mod clusters;
mod control;
mod images;
mod links;
mod modes;
mod resize;
mod storage;
mod viewport;
mod visual;
mod width;

pub use simd::BitSet512;
pub use width::{char_width, is_wide_cp};
use width::is_pictographic;
pub use modes::{TerminalModes, set_grapheme_default};
use modes::GRAPHEME_DEFAULT;
use clusters::ClusterTable;
pub use storage::set_scrollback_limit;
use storage::scrollback_lines;

pub use colors::{color_scheme_is_dark, color_scheme_report, set_color_scheme_dark, set_default_colors, set_palette};
pub(crate) use images::{ImageCompletion, ImageJob};
use control::{DcsRequest, MAX_IMAGE_STRING, MAX_OSC_BYTES, parse_osc7_path};

pub use cell::*;
pub use prompt::*;
pub use selection::*;

use crate::terminal::search::SearchText;






pub struct Grid {
    osc_len: Option<usize>,
    osc_discard: bool,
    osc_start: Option<std::time::Instant>,
    pub(crate) defer_images: bool,
    pub(crate) image_job: Option<ImageJob>,
    pub(crate) image_epoch: u64,
    /// AI bridge state (see `AiTrack`).
    pub ai: AiTrack,
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
    /// Horizontal tab stops, one bit per column (up to 512 columns on stack).
    tab_stops: BitSet512,
    /// Lines ever pushed into scrollback; anchors absolute line numbers.
    lines_pushed: usize,

    /// DECSET / DECRST and keyboard protocol modes (reset together by RIS).
    pub modes: TerminalModes,

    /// Cell size in pixels, for XTWINOPS size reports.
    pub cell_px: (u16, u16),
    /// DCS request being collected (`DCS + q` XTGETTCAP or `DCS $ q` DECRQSS).
    dcs: Option<DcsRequest>,
    dcs_buf: Vec<u8>,
    dcs_start: Option<std::time::Instant>,
    /// Inline images (Kitty graphics protocol, Sixel).
    pub graphics: crate::terminal::graphics::Store,
    /// APC filter state in front of vte (which drops APC strings): 0 ground,
    /// 1 after ESC, 2 inside `ESC _`, 3 ESC inside it.
    apc_state: u8,
    apc_buf: Vec<u8>,
    apc_start: Option<std::time::Instant>,

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

    /// Per visible row dirty flags (up to 512 rows on stack), plus a "everything changed" shortcut.
    pub dirty: BitSet512,
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
    /// `links::extra_hash` of each `extras` slot's content; meaningless for
    /// empty slots.
    extras_hash: [u32; 256],
    extras_dirty: bool,
    extras_full: bool,
    /// Per slot, the `cells` index where the last scan found it in use, or
    /// `NO_WITNESS`. Only a hint: a slot counts as used while that cell is live
    /// and still refers to it, so a stale entry just means a scan.
    extras_witness: [u32; 256],
    /// Times `collect_extras` walked the cells; tests bound it.
    #[cfg(test)]
    extras_full_scans: usize,
    /// Grapheme cluster table (see `ClusterTable`).
    clusters: ClusterTable,
    /// The last character joined a ZWJ to a cluster: a pictograph after it joins too.
    joiner: bool,
    /// True when the vte parser is known to be in its ground state with no
    /// partial UTF-8 pending (vte does not expose this). Set by the dispatch
    /// callbacks, cleared by ESC and any non-ASCII byte.
    ground: bool,
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
            ai: AiTrack::default(),
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
            lines_pushed: 0,
            modes: TerminalModes {
                cursor_visible: true,
                cursor_shape: 0,
                app_cursor: false,
                bracketed_paste: false,
                sync_output: false,
                sync_started: None,
                autowrap: true,
                insert_mode: false,
                mouse_tracking: MouseTracking::Off,
                mouse_sgr: false,
                focus_events: false,
                alt_scroll: true,
                mouse_sgr_pixels: false,
                color_scheme_reports: false,
                modify_other_keys: 0,
                kitty_keyboard: [KittyKeyboard::default(); 2],
                grapheme_mode: GRAPHEME_DEFAULT.load(Ordering::Relaxed),
            },
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
            graphics: crate::terminal::graphics::Store::default(),
            apc_state: 0,
            apc_buf: Vec::new(),
            apc_start: None,
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
            dirty: BitSet512::ones(rows),
            all_dirty: true,
            response: Vec::with_capacity(256),
            extras: vec![None],
            extras_hash: [0; 256],
            extras_dirty: true,
            extras_full: false,
            extras_witness: [links::NO_WITNESS; 256],
            #[cfg(test)]
            extras_full_scans: 0,
            clusters: ClusterTable::default(),
            joiner: false,
            ground: false,
            palette_override: Vec::new(),
            color_override: [0; 3],
        }
    }

    pub fn cursor_pos(&self) -> Option<(usize, usize)> {
        if !self.modes.cursor_visible {
            return None;
        }
        let off = if self.alt_active { 0 } else { self.display_offset };
        let r = self.cur.row + off;
        (r < self.rows).then(|| (self.cur.col.min(self.cols - 1), r))
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


    /// Append `c` to the grapheme in cell `(row, col)`, turning it into a
    /// cluster. `widen` makes a narrow one double-width (emoji presentation)
    /// when the cell after it is where the cursor is. False if it could not.
    fn append_to_cell(&mut self, row: usize, col: usize, c: char, widen: bool) -> bool {
        let cp = self.row_mut(row)[col].codepoint;
        let mut text = std::mem::take(&mut self.clusters.buf);
        text.clear();
        match self.cluster(cp) {
            Some(s) => text.push_str(s),
            None => match char::from_u32(cp).filter(|&b| b != '\0') {
                Some(base) => text.push(base),
                None => {
                    self.clusters.buf = text;
                    return false;
                }
            },
        }
        text.push(c);
        let id = self.intern_cluster(text.as_str());
        self.clusters.buf = text;
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
                self.wrap_pending = self.modes.autowrap;
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
        if self.modes.grapheme_mode && !c.is_ascii() && self.extend_grapheme(c) {
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
            if self.modes.autowrap {
                self.set_wrapped(self.cur.row, true);
                self.cur.col = 0;
                self.linefeed();
            }
        }
        if w == 2 && self.cur.col + 1 >= self.cols {
            if !self.modes.autowrap {
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
        let insert = self.modes.insert_mode;
        let prev_extra = {
            let check_extras = self.extras.len() > 1;
            let r = self.row_mut(row);
            if insert && col + w < cols {
                r.copy_within(col..cols - w, col + w);
            }
            let p_prev = if col > 0 && r[col].codepoint == 0 && is_wide_cp(r[col - 1].codepoint) {
                let e = if check_extras { r[col - 1].extra() } else { 0 };
                r[col - 1] = blank;
                e
            } else {
                0
            };
            let last = col + w - 1;
            let p_next = if last + 1 < cols && r[last + 1].codepoint == 0 && is_wide_cp(r[last].codepoint) {
                let e = if check_extras { r[last + 1].extra() } else { 0 };
                r[last + 1] = blank;
                e
            } else {
                0
            };
            let p_curr = if check_extras { r[col].extra() } else { 0 };
            r[col] = Cell { codepoint, fg, bg };
            let p_w2 = if w == 2 {
                let e = if check_extras { r[col + 1].extra() } else { 0 };
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
            self.wrap_pending = self.modes.autowrap;
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
            if let (true, Ok(i)) = (n < 3, crate::terminal::graphics::PLACEHOLDER_DIACRITICS.binary_search(&(mark as u32))) {
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
        if r < self.rows {
            self.dirty.set(r, true);
        }
    }

    #[inline]
    fn blank(&self) -> Cell {
        Cell { codepoint: 0, fg: 0, bg: self.cur.pen.bg }
    }

    /// Feed a prefix of `bytes` and return how many were consumed (at least 1).
    /// Behaves exactly like calling `feed_byte` on those bytes, but a run of
    /// printable ASCII in the parser's ground state (together with inline `\r`,
    /// `\n`, `\x1b[...m` SGR sequences and complete UTF-8 characters) is
    /// written in one batch.
    #[inline]
    pub fn feed_bytes(&mut self, parser: &mut vte::Parser, bytes: &[u8]) -> usize {
        if self.ground
            && self.osc_len.is_none()
            && self.apc_state == 0
            && !self.joiner
            && !self.cur.line_drawing
            && !self.modes.insert_mode
            && self.modes.autowrap
        {
            let mut offset = 0;
            let mut csi_params = [0u16; 16];
            while offset < bytes.len() {
                let rem = &bytes[offset..];
                if rem.len() >= MIN_ASCII_RUN
                    && rem[0].wrapping_sub(0x20) < 0x5f
                    && rem[1].wrapping_sub(0x20) < 0x5f
                    && rem[2].wrapping_sub(0x20) < 0x5f
                    && rem[3].wrapping_sub(0x20) < 0x5f
                {
                    let run = simd::printable_ascii_prefix(rem);
                    self.print_ascii_run(&rem[..run]);
                    offset += run;
                } else if rem.starts_with(b"\r\n") && offset > 0 {
                    self.cur.col = 0;
                    self.wrap_pending = false;
                    self.linefeed();
                    offset += 2;
                } else if rem[0] == b'\n' && offset > 0 {
                    self.linefeed();
                    self.wrap_pending = false;
                    offset += 1;
                } else if rem[0] == b'\r' && offset > 0 {
                    self.cur.col = 0;
                    self.wrap_pending = false;
                    offset += 1;
                } else if let Some((consumed, n_params, action)) = simd::try_parse_simple_csi(rem, &mut csi_params) {
                    self.dispatch_simple_csi(&csi_params[..n_params], action);
                    offset += consumed;
                } else if rem[0].wrapping_sub(0x20) < 0x5f {
                    // Too short to batch, e.g. the space between two words.
                    <Self as vte::Perform>::print(self, rem[0] as char);
                    offset += 1;
                } else if let Some(c) = printable_utf8_char(rem) {
                    // What vte's UTF-8 decoder would hand to `print` in ground state.
                    <Self as vte::Perform>::print(self, c);
                    offset += c.len_utf8();
                    if self.joiner {
                        // The loop was entered with no pending ZWJ; keep that
                        // true for the rest of the batch.
                        break;
                    }
                } else {
                    break;
                }
            }
            if offset > 0 {
                return offset;
            }
        }
        self.feed_byte(parser, bytes[0]);
        1
    }

    #[inline]
    fn dispatch_simple_csi(&mut self, p: &[u16], action: u8) {
        if action == b'm' {
            self.sgr_flat(p);
            return;
        }
        let raw = |i: usize| p.get(i).copied().unwrap_or(0) as usize;
        let arg = |i: usize, d: usize| match raw(i) {
            0 => d,
            v => v,
        };
        let (rows, cols) = (self.rows, self.cols);
        self.wrap_pending = false;
        match action {
            b'A' => self.cursor_up(arg(0, 1)),
            b'B' => self.cursor_down(arg(0, 1)),
            b'C' => self.cur.col = (self.cur.col + arg(0, 1)).min(cols - 1),
            b'D' => self.cur.col = self.cur.col.saturating_sub(arg(0, 1)),
            b'G' => self.cur.col = (arg(0, 1) - 1).min(cols - 1),
            b'H' | b'f' => {
                self.goto_row(arg(0, 1));
                self.cur.col = (arg(1, 1) - 1).min(cols - 1);
            }
            b'J' => {
                let (r, c) = (self.cur.row, self.cur.col);
                match raw(0) {
                    0 => {
                        self.clear_cols(r, c, cols);
                        self.clear_rows(r + 1, rows);
                    }
                    1 => {
                        self.clear_rows(0, r);
                        self.clear_cols(r, 0, c + 1);
                    }
                    2 => {
                        self.clear_rows(0, rows);
                        let top = self.lines_pushed;
                        self.graphics.delete_lines(top..top + rows, self.alt_active);
                    }
                    3 => {
                        self.history = 0;
                        self.display_offset = 0;
                        self.scroll_fraction = 0.0;
                        self.graphics.prune(self.lines_pushed);
                        self.release_scrollback_pages();
                        self.all_dirty = true;
                    }
                    _ => {}
                }
            }
            b'K' => {
                let (r, c) = (self.cur.row, self.cur.col);
                match raw(0) {
                    0 => self.clear_cols(r, c, cols),
                    1 => self.clear_cols(r, 0, c + 1),
                    2 => self.clear_cols(r, 0, cols),
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// Same result as `put_char(b, 1)` for every byte of `run`, with the
    /// per-cell bookkeeping done once per row segment. Requires autowrap on,
    /// insert mode off and a run of printable ASCII.
    fn print_ascii_run(&mut self, run: &[u8]) {
        let mut rest = run;
        let (fg, bg) = self.colors();
        let blank = self.blank();
        let check_extras = self.extras.len() > 1;
        while !rest.is_empty() {
            if self.wrap_pending {
                self.wrap_pending = false;
                self.set_wrapped(self.cur.row, true);
                self.cur.col = 0;
                self.linefeed();
            }
            let (row, col, cols) = (self.cur.row, self.cur.col, self.cols);
            let n = rest.len().min(cols - col);
            let (seg, tail) = rest.split_at(n);
            let last = col + n - 1;
            let r = self.row_mut(row);
            let mut prev_extra = simd::fixup_wide_edges(r, col, last, cols, blank, check_extras);
            prev_extra |= simd::write_ascii_cells(&mut r[col..=last], seg, fg, bg, check_extras);
            if prev_extra {
                self.extras_dirty = true;
            }
            if col + n >= cols {
                self.set_wrapped(row, false);
            }
            self.mark(row);
            if col + n >= cols {
                self.cur.col = cols - 1;
                self.wrap_pending = true;
            } else {
                self.cur.col = col + n;
            }
            rest = tail;
        }
    }

    /// Feed one byte from the PTY. APC strings (the Kitty graphics protocol)
    /// are handled here, because vte drops them; everything else goes to `parser`.
    #[inline]
    pub fn feed_byte(&mut self, parser: &mut vte::Parser, b: u8) {
        if b == 0x1b || b >= 0x80 {
            self.ground = false;
        }
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
            let next = self.tab_stops.next_set(self.cur.col, self.cols);
            self.cur.col = next.unwrap_or(self.cols - 1);
        }
    }

    fn tab_backward(&mut self, n: usize) {
        for _ in 0..n {
            let prev = self.tab_stops.prev_set(self.cur.col, 1);
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

    /// CUU / CPL: stop at the top margin when starting at or below it, as xterm does.
    fn cursor_up(&mut self, n: usize) {
        let top = if self.cur.row >= self.scroll_top { self.scroll_top } else { 0 };
        self.cur.row = self.cur.row.saturating_sub(n).max(top);
    }

    /// CUD / CNL / VPR: stop at the bottom margin when starting at or above it.
    fn cursor_down(&mut self, n: usize) {
        let bot = if self.cur.row <= self.scroll_bot { self.scroll_bot } else { self.rows - 1 };
        self.cur.row = (self.cur.row + n).min(bot);
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
        self.modes.cursor_visible = true;
        self.modes.cursor_shape = 0;
        self.modes.app_cursor = false;
        self.modes.bracketed_paste = false;
        self.modes.autowrap = true;
        self.modes.insert_mode = false;
        self.modes.sync_output = false;
        self.modes.sync_started = None;
        self.modes.mouse_tracking = MouseTracking::Off;
        self.modes.mouse_sgr = false;
        self.modes.focus_events = false;
        self.modes.alt_scroll = true;
        self.modes.mouse_sgr_pixels = false;
        self.modes.color_scheme_reports = false;
        self.modes.grapheme_mode = GRAPHEME_DEFAULT.load(Ordering::Relaxed);
        self.joiner = false;
        self.modes.kitty_keyboard = [KittyKeyboard::default(); 2];
        self.modes.modify_other_keys = 0;
        self.progress = None;
        self.pointer_shape.clear();
        self.title_changed = true;
        self.palette_override = Vec::new();
        self.color_override = [0; 3];
        self.tab_stops = default_tab_stops(self.cols);
        self.selection = None;
        self.clusters.texts.clear();
        self.clusters.ids.clear();
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
        // Drop the history length with the pages; releasing them alone leaves blank lines.
        self.clear_history();
    }

}

fn default_tab_stops(cols: usize) -> BitSet512 {
    BitSet512::default_tabs(cols)
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

/// Shortest printable-ASCII run worth batching; shorter ones cost more to set up.
const MIN_ASCII_RUN: usize = 4;

/// The complete UTF-8 character at the start of `bytes`, if it is one vte
/// would print (U+00A0 and up; C1 controls are left to vte). `None` for
/// ASCII, invalid or truncated sequences, which go through vte instead.
#[inline]
fn printable_utf8_char(bytes: &[u8]) -> Option<char> {
    // Well-formed sequences per the Unicode Standard, table 3-7: the second
    // byte's range excludes overlong forms, surrogates and values > U+10FFFF.
    let cont = |i: usize, lo: u8, hi: u8| bytes.get(i).filter(|b| (lo..=hi).contains(*b)).map(|&b| u32::from(b & 0x3F));
    let b0 = u32::from(bytes[0]);
    let cp = match bytes[0] {
        0xC2..=0xDF => (b0 & 0x1F) << 6 | cont(1, 0x80, 0xBF)?,
        0xE0..=0xEF => {
            let (lo, hi) = match bytes[0] {
                0xE0 => (0xA0, 0xBF),
                0xED => (0x80, 0x9F),
                _ => (0x80, 0xBF),
            };
            (b0 & 0x0F) << 12 | cont(1, lo, hi)? << 6 | cont(2, 0x80, 0xBF)?
        }
        0xF0..=0xF4 => {
            let (lo, hi) = match bytes[0] {
                0xF0 => (0x90, 0xBF),
                0xF4 => (0x80, 0x8F),
                _ => (0x80, 0xBF),
            };
            (b0 & 0x07) << 18 | cont(1, lo, hi)? << 12 | cont(2, 0x80, 0xBF)? << 6 | cont(3, 0x80, 0xBF)?
        }
        _ => return None,
    };
    char::from_u32(cp).filter(|&c| c >= '\u{A0}')
}
