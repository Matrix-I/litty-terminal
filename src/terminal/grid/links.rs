//! Hyperlinks and cell extras: implicit URL / path detection under the mouse,
//! OSC 8 explicit links, SGR 58 underline colors, the interned `CellExtra`
//! table and its reclamation, and the inspector's cell description.

use super::*;

impl Grid {
    #[inline]
    pub(super) fn is_path_char(ch: char) -> bool {
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

    // ------------------------------------------------------------------
    // Extra cell attributes (OSC 8 hyperlinks, SGR 58 underline colors)
    // ------------------------------------------------------------------


    pub(super) fn extra_at(&self, idx: u8) -> Option<&CellExtra> {
        self.extras.get(idx as usize)?.as_ref()
    }

    /// Point the pen at `extra` with its underline color replaced.
    pub(super) fn set_underline_color(&mut self, extra: u8, color: u32) {
        let link = self.extra_at(extra).and_then(|e| e.link.clone());
        let old = self.cur.pen.extra;
        self.cur.pen.extra = self.intern_extra(CellExtra { link, underline_color: color });
        if old != 0 && self.cur.pen.extra != old {
            self.extras_dirty = true;
        }
    }

    /// Point the pen at the current extra with its hyperlink replaced.
    pub(super) fn set_link(&mut self, link: Option<OscLink>) {
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
    pub(super) fn intern_extra(&mut self, extra: CellExtra) -> u8 {
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
    pub(super) fn collect_extras(&mut self) -> Option<usize> {
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
    pub(super) fn trim_extras(&mut self) {
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
    pub(super) fn cell_link(&self, cell: &Cell) -> Option<&OscLink> {
        match cell.extra() {
            0 => None,
            i => self.extra_at(i)?.link.as_ref(),
        }
    }

    /// OSC 8: `8 ; params ; URI` starts a hyperlink, an empty URI ends it.
    pub(super) fn osc8(&mut self, params: &[&[u8]]) {
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

/// What Cmd/Ctrl+Click on an OSC 8 link opens: `(target, is_path)`. `file://` URIs
/// become local paths; other URIs are only opened for a few well-known schemes, so
/// a program cannot make a click launch an arbitrary URL handler.
pub(super) fn osc8_target(uri: &str) -> Option<(String, bool)> {
    if uri.starts_with("file://") {
        return parse_osc7_path(uri.as_bytes()).map(|path| (path, true));
    }
    let (scheme, _) = uri.split_once(':')?;
    const SCHEMES: [&str; 5] = ["http", "https", "ftp", "ftps", "mailto"];
    SCHEMES.iter().any(|s| s.eq_ignore_ascii_case(scheme)).then(|| (uri.to_string(), false))
}

/// Drop trailing sentence punctuation from a detected link `chars[start..end]`,
/// keeping a closing `)` / `]` that has a matching opener inside the link.
pub(super) fn trim_trailing_punctuation(chars: &[char], start: usize, mut end: usize) -> usize {
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
