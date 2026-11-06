//! Colors: configured defaults and 16-color palette (process-wide atomics),
//! SGR pen colors, OSC 4 / 104 palette overrides and OSC 10 / 11 / 12
//! dynamic colors with their xterm-style replies.

use super::*;

/// Default colors, stored as `0xRRGGBB00`. Cells store `0` to mean "default",
/// which keeps a freshly zeroed buffer valid and lets the shader apply the theme.
/// They are runtime-configurable (`background` / `foreground` config options).
pub(super) static DEFAULT_FG_RGB: AtomicU32 = AtomicU32::new(0xE6E6_E600);

pub(super) static DEFAULT_BG_RGB: AtomicU32 = AtomicU32::new(0x0000_0000);

#[inline]
pub fn default_fg() -> u32 {
    DEFAULT_FG_RGB.load(Ordering::Relaxed)
}

#[inline]
pub fn default_bg() -> u32 {
    DEFAULT_BG_RGB.load(Ordering::Relaxed)
}

/// Whether the system appearance is dark, for color scheme reports (DSR 996 / mode 2031).
pub(super) static COLOR_SCHEME_DARK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn set_color_scheme_dark(dark: bool) {
    COLOR_SCHEME_DARK.store(dark, Ordering::Relaxed);
}

pub fn color_scheme_is_dark() -> bool {
    COLOR_SCHEME_DARK.load(Ordering::Relaxed)
}

/// `CSI ? 997 ; 1 n` (dark) or `; 2 n` (light): the color scheme report.
pub fn color_scheme_report() -> &'static [u8] {
    if COLOR_SCHEME_DARK.load(Ordering::Relaxed) {
        b"\x1b[?997;1n"
    } else {
        b"\x1b[?997;2n"
    }
}

pub fn set_default_colors(fg: u32, bg: u32) {
    DEFAULT_FG_RGB.store(fg & !0xFF, Ordering::Relaxed);
    DEFAULT_BG_RGB.store(bg & !0xFF, Ordering::Relaxed);
}

/// ANSI 16-color base palette (`0xRRGGBB`).
pub(super) static PALETTE_BASE: [AtomicU32; 16] = [
    AtomicU32::new(0x121316),
    AtomicU32::new(0xF38BA8),
    AtomicU32::new(0xA6E3A1),
    AtomicU32::new(0xF5A97F),
    AtomicU32::new(0x00BFE3),
    AtomicU32::new(0xCBA6F7),
    AtomicU32::new(0x89DCEB),
    AtomicU32::new(0xD9E0EE),
    AtomicU32::new(0x585B70),
    AtomicU32::new(0xF38BA8),
    AtomicU32::new(0xA6E3A1),
    AtomicU32::new(0xF9E2AF),
    AtomicU32::new(0x74C7EC),
    AtomicU32::new(0xF5C2E7),
    AtomicU32::new(0x94E2D5),
    AtomicU32::new(0xFFFFFF),
];

pub fn set_palette(pal: &[u32; 16]) {
    for (i, &col) in pal.iter().enumerate() {
        PALETTE_BASE[i].store(col, Ordering::Relaxed);
    }
}

impl Grid {
    // ------------------------------------------------------------------
    // SGR
    // ------------------------------------------------------------------


    pub(super) fn sgr(&mut self, params: &Params) {
        if params.is_empty() {
            let extra = self.cur.pen.extra;
            self.cur.pen = Pen::default();
            if extra != 0 {
                self.set_underline_color(extra, 0);
            }
            return;
        }
        // Moved out for the loop so palette lookups do not borrow `self`.
        let pal = std::mem::take(&mut self.palette_override);
        let palette = |i: u8| palette_with(&pal, i);
        let mut it = params.iter();
        while let Some(sp) = it.next() {
            let pen = &mut self.cur.pen;
            match sp[0] {
                0 => {
                    // SGR 0 resets the underline color but not the OSC 8 hyperlink.
                    let extra = pen.extra;
                    *pen = Pen::default();
                    if extra != 0 {
                        self.set_underline_color(extra, 0);
                    }
                }
                1 => pen.attrs |= ATTR_BOLD,
                2 => pen.attrs |= ATTR_DIM,
                3 => pen.attrs |= ATTR_ITALIC,
                // `4:0` .. `4:5`: none, single, double, curly, dotted, dashed.
                4 => pen.underline = sp.get(1).map_or(UNDERLINE_SINGLE, |&s| s.min(UNDERLINE_DASHED as u16) as u8),
                7 => pen.attrs |= ATTR_INVERSE,
                8 => pen.attrs |= ATTR_HIDDEN,
                9 => pen.attrs |= ATTR_STRIKE,
                21 => pen.underline = UNDERLINE_DOUBLE,
                22 => pen.attrs &= !(ATTR_BOLD | ATTR_DIM),
                23 => pen.attrs &= !ATTR_ITALIC,
                24 => pen.underline = 0,
                27 => pen.attrs &= !ATTR_INVERSE,
                28 => pen.attrs &= !ATTR_HIDDEN,
                29 => pen.attrs &= !ATTR_STRIKE,
                n @ 30..=37 => pen.fg = palette((n - 30) as u8) | FG_SET,
                38 => {
                    if let Some(c) = ext_color(sp, &mut it, palette) {
                        self.cur.pen.fg = c | FG_SET;
                    }
                }
                39 => pen.fg = 0,
                n @ 40..=47 => pen.bg = palette((n - 40) as u8) | BG_SET,
                48 => {
                    if let Some(c) = ext_color(sp, &mut it, palette) {
                        self.cur.pen.bg = c | BG_SET;
                    }
                }
                49 => pen.bg = 0,
                58 => {
                    if let Some(c) = ext_color(sp, &mut it, palette) {
                        let extra = self.cur.pen.extra;
                        self.set_underline_color(extra, c | 1);
                    }
                }
                59 => {
                    let extra = pen.extra;
                    if extra != 0 {
                        self.set_underline_color(extra, 0);
                    }
                }
                n @ 90..=97 => pen.fg = palette((n - 90 + 8) as u8) | FG_SET,
                n @ 100..=107 => pen.bg = palette((n - 100 + 8) as u8) | BG_SET,
                _ => {}
            }
        }
        self.palette_override = pal;
    }

    /// Apply flat semicolon-separated SGR parameters parsed directly from a
    /// `\x1b[...m` sequence without going through `vte::Parser`.
    #[inline]
    pub(super) fn sgr_flat(&mut self, params: &[u16]) {
        if params.is_empty() {
            let extra = self.cur.pen.extra;
            self.cur.pen = Pen::default();
            if extra != 0 {
                self.set_underline_color(extra, 0);
            }
            return;
        }
        let pal = std::mem::take(&mut self.palette_override);
        let palette = |i: u8| palette_with(&pal, i);
        let mut idx = 0;
        while idx < params.len() {
            let code = params[idx];
            idx += 1;
            let pen = &mut self.cur.pen;
            match code {
                0 => {
                    let extra = pen.extra;
                    *pen = Pen::default();
                    if extra != 0 {
                        self.set_underline_color(extra, 0);
                    }
                }
                1 => pen.attrs |= ATTR_BOLD,
                2 => pen.attrs |= ATTR_DIM,
                3 => pen.attrs |= ATTR_ITALIC,
                4 => pen.underline = UNDERLINE_SINGLE,
                7 => pen.attrs |= ATTR_INVERSE,
                8 => pen.attrs |= ATTR_HIDDEN,
                9 => pen.attrs |= ATTR_STRIKE,
                21 => pen.underline = UNDERLINE_DOUBLE,
                22 => pen.attrs &= !(ATTR_BOLD | ATTR_DIM),
                23 => pen.attrs &= !ATTR_ITALIC,
                24 => pen.underline = 0,
                27 => pen.attrs &= !ATTR_INVERSE,
                28 => pen.attrs &= !ATTR_HIDDEN,
                29 => pen.attrs &= !ATTR_STRIKE,
                n @ 30..=37 => pen.fg = palette((n - 30) as u8) | FG_SET,
                38 => {
                    if let Some(c) = ext_color_flat(params, &mut idx, palette) {
                        self.cur.pen.fg = c | FG_SET;
                    }
                }
                39 => pen.fg = 0,
                n @ 40..=47 => pen.bg = palette((n - 40) as u8) | BG_SET,
                48 => {
                    if let Some(c) = ext_color_flat(params, &mut idx, palette) {
                        self.cur.pen.bg = c | BG_SET;
                    }
                }
                49 => pen.bg = 0,
                58 => {
                    if let Some(c) = ext_color_flat(params, &mut idx, palette) {
                        let extra = self.cur.pen.extra;
                        self.set_underline_color(extra, c | 1);
                    }
                }
                59 => {
                    let extra = pen.extra;
                    if extra != 0 {
                        self.set_underline_color(extra, 0);
                    }
                }
                n @ 90..=97 => pen.fg = palette((n - 90 + 8) as u8) | FG_SET,
                n @ 100..=107 => pen.bg = palette((n - 100 + 8) as u8) | BG_SET,
                _ => {}
            }
        }
        self.palette_override = pal;
    }

    /// Default foreground as `0xRRGGBB00`: OSC 10 override, else the config color.
    pub fn fg_color(&self) -> u32 {
        match self.color_override[0] {
            0 => default_fg(),
            c => c & !0xFF,
        }
    }

    /// Default background as `0xRRGGBB00`: OSC 11 override, else the config color.
    pub fn bg_color(&self) -> u32 {
        match self.color_override[1] {
            0 => default_bg(),
            c => c & !0xFF,
        }
    }

    /// Cursor color set by OSC 12, as `0xRRGGBB00`.
    pub fn cursor_color(&self) -> Option<u32> {
        (self.color_override[2] != 0).then(|| self.color_override[2] & !0xFF)
    }

    /// OSC 4: `4 ; index ; spec ; index ; spec ...`, where `spec` may be `?`.
    pub(super) fn osc4(&mut self, params: &[&[u8]], st: &str) {
        let mut pairs: Vec<(u32, u32)> = Vec::new();
        for pair in params[1..].chunks(2) {
            let Some(i) = std::str::from_utf8(pair[0]).ok().and_then(|s| s.parse::<u8>().ok()) else { continue };
            match pair.get(1) {
                Some(&b"?") => {
                    let c = palette_with(&self.palette_override, i);
                    self.reply_color(&format!("4;{i}"), c, st);
                }
                Some(spec) => {
                    if let Some(c) = parse_x11_color(spec) {
                        let old = palette_with(&self.palette_override, i) & !0xFF;
                        let new = c & !0xFF;
                        if self.palette_override.is_empty() {
                            self.palette_override = vec![0; 256];
                        }
                        self.palette_override[i as usize] = c | 1;
                        if old != new {
                            if let Some(entry) = pairs.iter_mut().find(|(o, _)| *o == old) {
                                entry.1 = new;
                            } else {
                                pairs.push((old, new));
                            }
                        }
                    }
                }
                None => {}
            }
        }
        self.recolor_pairs(&pairs);
    }

    /// OSC 104 [; index ...]: reset palette entries (all without indices) in a
    /// single pass over active cells without sequential color aliasing.
    pub(super) fn osc104(&mut self, params: &[&[u8]]) {
        let all = params.len() < 2 || params[1].is_empty();
        if !self.palette_override.is_empty() {
            let mut pairs: Vec<(u32, u32)> = Vec::new();
            let mut collect_reset = |i: u8, overrides: &mut [u32]| {
                let raw = overrides[i as usize];
                if raw != 0 {
                    overrides[i as usize] = 0;
                    let old = raw & !0xFF;
                    let new = palette(i) & !0xFF;
                    if old != new && !pairs.iter().any(|&(o, _)| o == old) {
                        pairs.push((old, new));
                    }
                }
            };
            if all {
                for i in 0..=255u8 {
                    collect_reset(i, &mut self.palette_override);
                }
            } else {
                for i in params[1..].iter().filter_map(|p| std::str::from_utf8(p).ok()?.parse::<u8>().ok()) {
                    collect_reset(i, &mut self.palette_override);
                }
            }
            self.recolor_pairs(&pairs);
        }
        if all {
            self.palette_override = Vec::new();
        }
    }

    /// Repaint text already drawn with overridden palette colors in a single pass
    /// over active cells (`0..history + rows` + `alt`), avoiding both $O(256 \times \text{cap} \times \text{cols})$
    /// full-buffer scans and sequential swap aliasing.
    pub(super) fn recolor_pairs(&mut self, pairs: &[(u32, u32)]) {
        if pairs.is_empty() {
            return;
        }
        let apply_slice = |slice: &mut [Cell]| {
            if let &[(old, new)] = pairs {
                for cell in slice {
                    if cell.fg & FG_SET != 0 && cell.fg & !0xFF == old {
                        cell.fg = new | (cell.fg & 0xFF);
                    }
                    if cell.bg & BG_SET != 0 && cell.bg & !0xFF == old {
                        cell.bg = new | (cell.bg & 0xFF);
                    }
                }
            } else {
                for cell in slice {
                    if cell.fg & FG_SET != 0 {
                        let fg_rgb = cell.fg & !0xFF;
                        if let Some(&(_, new)) = pairs.iter().find(|&&(old, _)| old == fg_rgb) {
                            cell.fg = new | (cell.fg & 0xFF);
                        }
                    }
                    if cell.bg & BG_SET != 0 {
                        let bg_rgb = cell.bg & !0xFF;
                        if let Some(&(_, new)) = pairs.iter().find(|&&(old, _)| old == bg_rgb) {
                            cell.bg = new | (cell.bg & 0xFF);
                        }
                    }
                }
            }
        };

        let c = self.cols;
        if c > 0 && self.cap > 0 {
            let active_rows = (self.history + self.rows).min(self.cap);
            let start_row = (self.top + self.cap - self.history) % self.cap;
            if active_rows == self.cap {
                apply_slice(&mut self.cells);
            } else if start_row + active_rows <= self.cap {
                let s = start_row * c;
                let e = (start_row + active_rows) * c;
                apply_slice(&mut self.cells[s..e]);
            } else {
                let s1 = start_row * c;
                let s2 = (start_row + active_rows - self.cap) * c;
                apply_slice(&mut self.cells[s1..]);
                apply_slice(&mut self.cells[..s2]);
            }
        }
        if !self.alt.is_empty() {
            apply_slice(&mut self.alt);
        }
        self.all_dirty = true;
    }

    /// OSC 10 / 11 / 12 set or query the default fg, bg and cursor color. Extra
    /// parameters continue with the next code (`10;?;?` queries fg and bg).
    pub(super) fn osc_dynamic_color(&mut self, first: usize, specs: &[&[u8]], st: &str) {
        for (slot, spec) in (first..3).zip(specs) {
            if *spec == b"?" {
                let c = match slot {
                    0 => self.fg_color(),
                    1 => self.bg_color(),
                    _ => self.cursor_color().unwrap_or_else(default_fg),
                };
                self.reply_color(&(10 + slot).to_string(), c, st);
            } else if let Some(c) = parse_x11_color(spec) {
                self.color_override[slot] = c | 1;
                self.all_dirty = true;
            }
        }
    }

    /// Answer a color query as xterm does: `OSC code ; rgb:rrrr/gggg/bbbb ST`.
    pub(super) fn reply_color(&mut self, code: &str, c: u32, st: &str) {
        let (r, g, b) = ((c >> 24) & 0xFF, (c >> 16) & 0xFF, (c >> 8) & 0xFF);
        let _ = write!(self.response, "\x1b]{code};rgb:{r:02x}{r:02x}/{g:02x}{g:02x}/{b:02x}{b:02x}{st}");
    }
}

/// Parse `38;5;n`, `38;2;r;g;b` from flat semicolon-separated parameters.
#[inline]
fn ext_color_flat(params: &[u16], idx: &mut usize, palette: impl Fn(u8) -> u32) -> Option<u32> {
    let kind = *params.get(*idx)?;
    *idx += 1;
    match kind {
        5 => {
            let n = *params.get(*idx)?;
            *idx += 1;
            Some(palette(n as u8))
        }
        2 => {
            let r = *params.get(*idx)?;
            *idx += 1;
            let g = *params.get(*idx)?;
            *idx += 1;
            let b = *params.get(*idx)?;
            *idx += 1;
            Some(rgb(r, g, b))
        }
        _ => None,
    }
}

/// Parse `38;5;n`, `38;2;r;g;b` and their colon sub-parameter forms.
pub(super) fn ext_color<'a>(sp: &[u16], it: &mut impl Iterator<Item = &'a [u16]>, palette: impl Fn(u8) -> u32) -> Option<u32> {
    if sp.len() > 1 {
        match sp[1] {
            5 => sp.get(2).map(|&n| palette(n as u8)),
            2 => {
                let o = if sp.len() >= 6 { 3 } else { 2 };
                Some(rgb(*sp.get(o)?, *sp.get(o + 1)?, *sp.get(o + 2)?))
            }
            _ => None,
        }
    } else {
        match it.next()?[0] {
            5 => Some(palette(it.next()?[0] as u8)),
            2 => {
                let r = it.next()?[0];
                let g = it.next()?[0];
                let b = it.next()?[0];
                Some(rgb(r, g, b))
            }
            _ => None,
        }
    }
}

#[inline]
pub(super) fn rgb(r: u16, g: u16, b: u16) -> u32 {
    ((r.min(255) as u32) << 24) | ((g.min(255) as u32) << 16) | ((b.min(255) as u32) << 8)
}

/// xterm 256-color palette, returned as `0xRRGGBB00`.
pub(super) fn palette(i: u8) -> u32 {
    match i {
        0..=15 => PALETTE_BASE[i as usize].load(Ordering::Relaxed) << 8,
        16..=231 => {
            let i = i - 16;
            let lv = |v: u8| if v == 0 { 0u16 } else { 55 + 40 * v as u16 };
            rgb(lv(i / 36), lv((i / 6) % 6), lv(i % 6))
        }
        _ => {
            let v = 8 + 10 * (i - 232) as u16;
            rgb(v, v, v)
        }
    }
}

/// `palette(i)` with the OSC 4 overrides of one terminal applied.
#[inline]
pub(super) fn palette_with(overrides: &[u32], i: u8) -> u32 {
    match overrides.get(i as usize) {
        Some(&c) if c != 0 => c & !0xFF,
        _ => palette(i),
    }
}

/// Parse an X11 color spec as used by OSC 4 / 10 / 11 / 12: `rgb:R/G/B` with 1-4
/// hex digits per channel (scaled to 8 bits), `#RGB`, `#RRGGBB`, `#RRRGGGBBB`,
/// `#RRRRGGGGBBBB` (most significant bits, as in `XParseColor`), or a color name
/// such as `slate blue`. Returns `0xRRGGBB00`.
pub(super) fn parse_x11_color(spec: &[u8]) -> Option<u32> {
    let spec = std::str::from_utf8(spec).ok()?;
    let hex = |s: &str| (!s.is_empty() && s.len() <= 4).then(|| u32::from_str_radix(s, 16).ok()).flatten();
    let (r, g, b) = if let Some(rest) = spec.strip_prefix("rgb:") {
        let mut parts = rest.split('/');
        let mut channel = || {
            let p = parts.next()?;
            let max = (1u32 << (4 * p.len())) - 1;
            Some((hex(p)? * 255 + max / 2) / max)
        };
        let rgb = (channel()?, channel()?, channel()?);
        if parts.next().is_some() {
            return None;
        }
        rgb
    } else if let Some(digits) = spec.strip_prefix('#') {
        if digits.is_empty() || digits.len() % 3 != 0 || digits.len() > 12 {
            return None;
        }
        let n = digits.len() / 3;
        let channel = |i: usize| {
            let v = hex(&digits[i * n..(i + 1) * n])?;
            Some(if n == 1 { v << 4 } else { v >> (4 * (n - 2)) })
        };
        (channel(0)?, channel(1)?, channel(2)?)
    } else {
        return crate::terminal::x11colors::lookup(spec).map(|rgb| rgb << 8);
    };
    Some((r << 24) | (g << 16) | (b << 8))
}
