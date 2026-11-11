//! Hyperlinks and cell extras: implicit URL / path detection under the mouse,
//! OSC 8 explicit links, SGR 58 underline colors, the interned `CellExtra`
//! table and its reclamation, and the inspector's cell description.

use super::*;

/// `extras_witness` value for a slot no scan has seen in a cell.
pub(super) const NO_WITNESS: u32 = u32::MAX;

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
        let new = self.intern_extra(CellExtra { link, underline_color: color });
        self.set_pen_extra(new);
    }

    fn set_pen_extra(&mut self, new: u8) {
        let old = self.cur.pen.extra;
        self.cur.pen.extra = new;
        if old != 0 && new != old {
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
        let hash = match &extra.link {
            Some(link) => extra_hash(link.id.as_bytes(), &[link.uri.as_bytes()], extra.underline_color),
            None => extra_hash(b"", &[], extra.underline_color),
        };
        if let Some(i) = self.find_extra(hash, |e| *e == extra) {
            return i;
        }
        match self.free_extra_slot() {
            Some(slot) => self.store_extra(slot, hash, extra),
            None => 0,
        }
    }

    /// [`Self::intern_extra`] for an OSC 8 link given as its raw parameters: the
    /// `uri` pieces vte split at ';'. Allocates only when the link is new.
    fn intern_link(&mut self, id: &[u8], uri: &[&[u8]], underline_color: u32) -> u8 {
        let hash = extra_hash(id, uri, underline_color);
        let same = |e: &CellExtra| {
            e.underline_color == underline_color
                && e.link.as_ref().is_some_and(|l| l.id.as_bytes() == id && eq_joined(l.uri.as_bytes(), uri))
        };
        if let Some(i) = self.find_extra(hash, same) {
            return i;
        }
        // ';' is ASCII, so the joined URI is UTF-8 exactly when every piece is.
        let (Ok(id), true) = (std::str::from_utf8(id), uri.iter().all(|p| std::str::from_utf8(p).is_ok())) else {
            return self.intern_extra(CellExtra { link: None, underline_color });
        };
        let Some(slot) = self.free_extra_slot() else {
            return 0;
        };
        // Checked above, so the conversion never replaces anything.
        let uri = String::from_utf8_lossy(&uri.join(&b';')).into_owned();
        let link = OscLink { id: id.into(), uri: uri.into() };
        self.store_extra(slot, hash, CellExtra { link: Some(link), underline_color })
    }

    /// The slot holding an extra `same` accepts. `hash` filters the candidates
    /// so a miss costs integer compares, not string compares.
    fn find_extra(&self, hash: u32, same: impl Fn(&CellExtra) -> bool) -> Option<u8> {
        self.extras
            .iter()
            .zip(&self.extras_hash)
            .position(|(e, &h)| h == hash && e.as_ref().is_some_and(&same))
            .map(|i| i as u8)
    }

    /// A slot to store a new extra in: `extras.len()` to append, else a slot no
    /// cell uses any more. `None` if all 255 slots are still referenced.
    fn free_extra_slot(&mut self) -> Option<usize> {
        if self.extras.len() <= u8::MAX as usize {
            return Some(self.extras.len());
        }
        match self.extras.iter().skip(1).position(Option::is_none) {
            Some(i) => Some(i + 1),
            None if self.extras_full && !self.extras_dirty => None,
            None if self.extras_proven_full() => {
                // What a scan would find, without reading the whole scrollback.
                self.extras_full = true;
                self.extras_dirty = false;
                None
            }
            None => self.collect_extras(),
        }
    }

    fn store_extra(&mut self, slot: usize, hash: u32, extra: CellExtra) -> u8 {
        if slot == self.extras.len() {
            self.extras.push(Some(extra));
        } else {
            self.extras[slot] = Some(extra);
        }
        self.extras_hash[slot] = hash;
        slot as u8
    }

    /// First ring row of the scrollback, and how many ring rows are live
    /// (scrollback + main screen). Requires `cap > 0`.
    fn live_ring_rows(&self) -> (usize, usize) {
        let start_row = (self.top + self.cap - self.history) % self.cap;
        (start_row, (self.history + self.rows).min(self.cap))
    }

    /// Every slot is still used by the live main-screen or scrollback cell the
    /// last scan recorded for it, or by a pen, so a scan would free nothing.
    /// Costs 255 cell reads instead of a scan of the whole scrollback.
    fn extras_proven_full(&self) -> bool {
        let c = self.cols;
        if c == 0 || self.cap == 0 {
            return false;
        }
        let (start_row, active_rows) = self.live_ring_rows();
        let pens = [self.cur.pen.extra, self.saved.pen.extra, self.saved_main.pen.extra];
        (1..=u8::MAX).all(|slot| {
            let idx = self.extras_witness[slot as usize];
            let ring_row = idx as usize / c;
            pens.contains(&slot)
                || (idx != NO_WITNESS
                    && ring_row < self.cap
                    && (ring_row + self.cap - start_row) % self.cap < active_rows
                    && self.cells.get(idx as usize).is_some_and(|cell| cell.extra() == slot))
        })
    }

    /// Free every extra slot no cell or saved pen refers to; returns the first free one.
    pub(super) fn collect_extras(&mut self) -> Option<usize> {
        #[cfg(test)]
        {
            self.extras_full_scans += 1;
        }
        let mut used = [false; 256];
        let mut used_count = 0usize;
        let c = self.cols;
        if c > 0 && self.cap > 0 {
            let (start_row, active_rows) = self.live_ring_rows();
            // Newest rows first: the cell recorded for each slot is then the one
            // that stays in the scrollback longest.
            for k in (0..active_rows).rev() {
                let row_start = (start_row + k) % self.cap * c;
                for (i, cell) in self.cells[row_start..row_start + c].iter().enumerate() {
                    if ((cell.fg & FG_EXTRA) | (cell.bg & BG_EXTRA)) == 0 {
                        continue;
                    }
                    let extra = cell.extra() as usize;
                    if extra != 0 && !used[extra] {
                        used[extra] = true;
                        used_count += 1;
                        self.extras_witness[extra] = u32::try_from(row_start + i).unwrap_or(NO_WITNESS);
                        if used_count >= 255 {
                            self.extras_full = true;
                            self.extras_dirty = false;
                            return None;
                        }
                    }
                }
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
        let underline_color = self.extra_at(self.cur.pen.extra).map_or(0, |e| e.underline_color);
        let uri = params.get(2..).unwrap_or_default();
        // The URI may itself contain ';', which vte splits into extra parameters.
        let len = uri.iter().map(|p| p.len() + 1).sum::<usize>().saturating_sub(1);
        let new = if len == 0 || len > MAX_LINK_LEN {
            self.intern_extra(CellExtra { link: None, underline_color })
        } else {
            let id = params[1]
                .split(|&b| b == b':')
                .find_map(|kv| kv.strip_prefix(b"id="))
                .unwrap_or_default();
            self.intern_link(id, uri, underline_color)
        };
        self.set_pen_extra(new);
    }
}

/// FNV-1a over `underline_color`, `id` and the `uri` pieces joined by ';', so a
/// link hashes the same from vte's split parameters and from a stored `OscLink`.
fn extra_hash(id: &[u8], uri: &[&[u8]], underline_color: u32) -> u32 {
    const OFFSET_BASIS: u32 = 0x811C_9DC5;
    const PRIME: u32 = 0x0100_0193;
    // 0xFF never occurs in UTF-8, so it cannot blur the id / URI boundary.
    const ID_END: &[u8] = &[0xFF];
    let separated = uri.iter().enumerate().flat_map(|(k, part)| [if k == 0 { &b""[..] } else { b";" }, part]);
    [&underline_color.to_le_bytes()[..], id, ID_END]
        .into_iter()
        .chain(separated)
        .flatten()
        .fold(OFFSET_BASIS, |h, &b| (h ^ u32::from(b)).wrapping_mul(PRIME))
}

/// `stored == parts.join(";")`, without building the joined string.
fn eq_joined(stored: &[u8], parts: &[&[u8]]) -> bool {
    let mut rest = stored;
    for (k, part) in parts.iter().enumerate() {
        if k > 0 {
            let Some(r) = rest.strip_prefix(b";") else { return false };
            rest = r;
        }
        let Some(r) = rest.strip_prefix(*part) else { return false };
        rest = r;
    }
    rest.is_empty()
}

/// What Cmd/Ctrl+Click on an OSC 8 link opens: `(target, is_path)`. `file://` URIs
/// become local paths; other URIs are only opened for a few well-known schemes, so
/// a program cannot make a click launch an arbitrary URL handler.
pub(super) fn osc8_target(uri: &str) -> Option<(String, bool)> {
    osc8_target_on(uri, local_hostname())
}

/// [`osc8_target`] for a machine named `local_host` (empty when unknown).
/// A `file://` URI names a path on its host, so one from another machine (an
/// SSH session, a log) is not a link here: the same path may hold anything.
pub(super) fn osc8_target_on(uri: &str, local_host: &str) -> Option<(String, bool)> {
    if let Some(rest) = uri.strip_prefix("file://") {
        let host = &rest[..rest.find('/')?];
        let is_local = host.is_empty()
            || host.eq_ignore_ascii_case("localhost")
            || (!local_host.is_empty() && host.eq_ignore_ascii_case(local_host));
        return is_local.then(|| parse_osc7_path(uri.as_bytes())).flatten().map(|path| (path, true));
    }
    let (scheme, _) = uri.split_once(':')?;
    const SCHEMES: [&str; 5] = ["http", "https", "ftp", "ftps", "mailto"];
    SCHEMES.iter().any(|s| s.eq_ignore_ascii_case(scheme)).then(|| (uri.to_string(), false))
}

/// This machine's hostname as `gethostname` reports it (what `ls --hyperlink`
/// and shell integrations put in `file://` URIs), or "" if it cannot be read.
fn local_hostname() -> &'static str {
    static HOST: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HOST.get_or_init(|| {
        let mut buf = [0u8; 256];
        // SAFETY: `buf` is valid for `buf.len()` bytes; gethostname NUL-terminates within it or fails.
        let ok = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0;
        let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        if ok { String::from_utf8_lossy(&buf[..len]).into_owned() } else { String::new() }
    })
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
