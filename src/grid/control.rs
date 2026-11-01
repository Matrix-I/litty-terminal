//! Control strings: DCS requests (XTGETTCAP, DECRQSS) and their replies,
//! OSC payload decoders, and the size limits for OSC / DCS / APC strings.

use super::*;

// Includes OSC 52 command/target fields in addition to its 1 MiB base64 payload.
pub(super) const MAX_OSC_BYTES: usize = (1 << 20) + 64;

impl Grid {
    /// XTGETTCAP: answer one hex-encoded terminfo capability name.
    pub(super) fn reply_termcap(&mut self, hex_name: &[u8]) {
        if hex_name.is_empty() || !hex_name.len().is_multiple_of(2) || !hex_name.iter().all(|b| b.is_ascii_hexdigit()) {
            return;
        }
        let name = decode_hex(hex_name);
        let value = name.as_deref().and_then(termcap);
        match value {
            Some(v) => {
                let _ = write!(self.response, "\x1bP1+r{}", std::str::from_utf8(hex_name).unwrap_or(""));
                if !v.is_empty() {
                    self.response.push(b'=');
                    for b in v.bytes() {
                        let _ = write!(self.response, "{b:02X}");
                    }
                }
            }
            None => {
                let _ = write!(self.response, "\x1bP0+r{}", std::str::from_utf8(hex_name).unwrap_or(""));
            }
        }
        self.response.extend_from_slice(b"\x1b\\");
    }

    /// DECRQSS: report the current SGR, scroll region or cursor style.
    pub(super) fn reply_decrqss(&mut self, request: &[u8]) {
        let body = match request {
            b"m" => Some(format!("{}m", self.sgr_report())),
            b"r" => Some(format!("{};{}r", self.scroll_top + 1, self.scroll_bot + 1)),
            b" q" => Some(format!("{} q", self.cursor_shape.max(1))),
            b"\"q" => Some("0\"q".to_string()),
            _ => None,
        };
        let _ = match body {
            Some(b) => write!(self.response, "\x1bP1$r{b}\x1b\\"),
            None => write!(self.response, "\x1bP0$r\x1b\\"),
        };
    }

    /// The pen as an SGR parameter list (`0;1;38;2;r;g;b`).
    pub(super) fn sgr_report(&self) -> String {
        use std::fmt::Write as _;
        let p = self.cur.pen;
        let mut out = String::from("0");
        for (bit, code) in [(ATTR_BOLD, 1), (ATTR_DIM, 2), (ATTR_ITALIC, 3), (ATTR_INVERSE, 7), (ATTR_HIDDEN, 8), (ATTR_STRIKE, 9)] {
            if p.attrs & bit != 0 {
                let _ = write!(out, ";{code}");
            }
        }
        if p.underline != 0 {
            let _ = write!(out, ";4:{}", p.underline);
        }
        for (color, base) in [(p.fg, 38), (p.bg, 48)] {
            if color != 0 {
                let _ = write!(out, ";{base};2;{};{};{}", color >> 24, (color >> 16) & 0xFF, (color >> 8) & 0xFF);
            }
        }
        out
    }
}

/// A DCS request Litty answers.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) enum DcsRequest {
    XtGetTcap,
    Decrqss,
    /// Sixel image; `true` = background stays transparent (DCS P2 = 1).
    Sixel(bool),
}

/// Largest APC (Kitty graphics chunk) or sixel string accepted, in bytes.
pub(super) const MAX_IMAGE_STRING: usize = 64 << 20;

/// Decode an XTGETTCAP hex string (`544E` → `TN`).
pub(super) fn decode_hex(hex: &[u8]) -> Option<String> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    let bytes: Option<Vec<u8>> = hex
        .chunks(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).ok()?, 16).ok())
        .collect();
    String::from_utf8(bytes?).ok()
}

/// Terminfo capabilities reported through XTGETTCAP; "" marks a boolean.
pub(super) fn termcap(name: &str) -> Option<&'static str> {
    Some(match name {
        "TN" | "name" => "xterm-256color",
        "Co" | "colors" => "256",
        "RGB" => "8",
        "Tc" | "Su" | "fullkbd" => "",
        "Smulx" => "\x1b[4:%p1%dm",
        "Setulc" => "\x1b[58:2::%p1%{65536}%/%d:%p1%{256}%/%{255}%&%d:%p1%{255}%&%dm",
        "Ss" => "\x1b[%p1%d q",
        "Se" => "\x1b[2 q",
        "Ms" => "\x1b]52;%p1%s;%p2%s\x07",
        "Sync" => "\x1b[?2026%?%p1%{1}%-%tl%eh%;",
        "smxx" => "\x1b[9m",
        "rmxx" => "\x1b[29m",
        "BE" => "\x1b[?2004h",
        "BD" => "\x1b[?2004l",
        "fe" => "\x1b[?1004h",
        "fd" => "\x1b[?1004l",
        _ => return None,
    })
}

/// OSC parameters joined back with `;` (vte splits on it), as lossy UTF-8 without
/// control characters, cut to at most `max` characters.
pub(super) fn lossy_join(parts: &[&[u8]], max: usize) -> String {
    let joined = parts.join(&b';');
    String::from_utf8_lossy(&joined).chars().filter(|c| !c.is_control()).take(max).collect()
}

/// Extract the path from an OSC 7 payload (`file://host/percent%20encoded/path`).
pub(super) fn parse_osc7_path(payload: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(payload).ok()?;
    let rest = s.strip_prefix("file://")?;
    let path = &rest[rest.find('/')?..];
    let mut out = Vec::with_capacity(path.len());
    let bytes = path.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(b) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).ok()
}

/// Standard base64 decoder for OSC 52 payloads (capped at 1 MiB of input).
pub(super) fn decode_base64(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() > 1 << 20 {
        return None;
    }
    let mut out = Vec::with_capacity(data.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0;
    for &c in data {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' => continue,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}
