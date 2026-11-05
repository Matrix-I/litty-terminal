//! Pure byte payloads for pasting into the PTY.
//!
//! `paste_payload` strips `ESC` so pasted text cannot terminate bracketed
//! paste (`\x1b[201~`) early.

/// Build the bytes sent to the PTY for a clipboard paste.
///
/// ESC (and the 8-bit C1 CSI, U+009B) is stripped so clipboard content can never
/// smuggle in `\x1b[201~` to terminate bracketed paste early and execute the rest
/// as typed input, nor inject escape sequences into programs without bracketed paste.
/// Quote a path for a POSIX shell: bare when it only has safe characters,
/// otherwise in single quotes (with `'` written as `'\''`).
pub(crate) fn shell_quote(s: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "_@%+=:,./-~".contains(c);
    if !s.is_empty() && s.chars().all(safe) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub(crate) fn paste_payload(text: &str, bracketed: bool) -> Vec<u8> {
    let mut payload = Vec::with_capacity(text.len() + 12);
    if bracketed {
        payload.extend_from_slice(b"\x1b[200~");
    }
    let mut buf = [0u8; 4];
    for ch in text.chars().filter(|&c| c != '\x1b' && c != '\u{9b}') {
        payload.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
    }
    if bracketed {
        payload.extend_from_slice(b"\x1b[201~");
    }
    payload
}

/// Most characters kept per preview line in the multi-line paste confirmation.
const PREVIEW_LINE_CHARS: usize = 60;
/// Most lines shown in the multi-line paste confirmation.
const PREVIEW_LINES: usize = 3;

/// First lines of `text` for the multi-line paste confirmation, each cut to
/// [`PREVIEW_LINE_CHARS`] characters (never inside a multibyte character).
pub(crate) fn paste_preview(text: &str) -> String {
    text.lines()
        .take(PREVIEW_LINES)
        .map(|l| match l.char_indices().nth(PREVIEW_LINE_CHARS) {
            Some((cut, _)) => format!("{}…", &l[..cut]),
            None => l.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Bytes that put a history entry on the shell's command line. Ctrl+E Ctrl+U
/// clears the current line first; `run` appends Enter.
///
/// Multi-line entries and control characters are sanitized: when `bracketed` is
/// true, the command is wrapped in `\x1b[200~...\x1b[201~` (with ESC, C1 CSI,
/// and non-newline C0/DEL control bytes stripped so a crafted entry cannot break
/// out of bracketed paste); when `bracketed` is false and `run` is false (Tab /
/// edit), newlines are flattened to spaces so the shell never executes lines
/// prematurely.
pub(crate) fn history_insert_payload(cmd: &str, run: bool, bracketed: bool) -> Vec<u8> {
    let mut payload = Vec::with_capacity(cmd.len() + 16);
    payload.extend_from_slice(b"\x05\x15");
    if bracketed {
        payload.extend_from_slice(b"\x1b[200~");
    }
    let mut buf = [0u8; 4];
    for ch in cmd.chars() {
        match ch {
            '\n' | '\r' if !bracketed && !run => payload.push(b' '),
            '\n' | '\t' => payload.push(ch as u8),
            c if c.is_control() || c == '\u{9b}' => {}
            c => payload.extend_from_slice(c.encode_utf8(&mut buf).as_bytes()),
        }
    }
    if bracketed {
        payload.extend_from_slice(b"\x1b[201~");
    }
    if run {
        payload.push(b'\r');
    }
    payload
}

#[cfg(test)]
mod tests {
    use super::{paste_payload, paste_preview, shell_quote};

    #[test]
    fn preview_cuts_on_a_char_boundary_not_a_byte_offset() {
        // 'ệ' is 3 bytes and straddles byte 60 (bytes 59..62).
        let text = format!("{}ệ and more text after it\nline2", "A".repeat(59));
        let preview = paste_preview(&text);
        assert_eq!(preview, format!("{}ệ…\nline2", "A".repeat(59)));
    }

    #[test]
    fn preview_truncates_long_lines_by_characters() {
        let text = format!("{}\nshort", "ệ".repeat(80));
        let preview = paste_preview(&text);
        assert_eq!(preview, format!("{}…\nshort", "ệ".repeat(60)));
    }

    #[test]
    fn preview_keeps_only_three_lines_and_short_lines_verbatim() {
        assert_eq!(paste_preview("a\nb\nc\nd\ne"), "a\nb\nc");
        assert_eq!(paste_preview(&"x".repeat(60)), "x".repeat(60));
        assert_eq!(paste_preview(&"x".repeat(61)), format!("{}…", "x".repeat(60)));
    }

    #[test]
    fn dropped_paths_are_shell_quoted() {
        assert_eq!(shell_quote("/Users/me/notes.txt"), "/Users/me/notes.txt");
        assert_eq!(shell_quote("/tmp/My File (1).png"), "'/tmp/My File (1).png'");
        assert_eq!(shell_quote("/tmp/it's"), "'/tmp/it'\\''s'");
        assert_eq!(shell_quote("/tmp/$(rm -rf ~)"), "'/tmp/$(rm -rf ~)'");
    }

    #[test]
    fn bracketed_paste_wraps_plain_text() {
        assert_eq!(paste_payload("ls -la\n", true), b"\x1b[200~ls -la\n\x1b[201~");
        assert_eq!(paste_payload("ls -la\n", false), b"ls -la\n");
    }

    #[test]
    fn bracketed_paste_cannot_be_terminated_early() {
        let evil = "echo hi\x1b[201~rm -rf ~\n";
        let out = paste_payload(evil, true);
        assert_eq!(out, b"\x1b[200~echo hi[201~rm -rf ~\n\x1b[201~");
        // Exactly one start and one end marker: the injected terminator is neutralized.
        let end_markers = out.windows(6).filter(|w| *w == b"\x1b[201~").count();
        assert_eq!(end_markers, 1);
    }

    #[test]
    fn paste_strips_escape_and_c1_csi_but_keeps_unicode() {
        assert_eq!(paste_payload("Việt\x1b[31m\u{9b}2Jnam", false), "Việt[31m2Jnam".as_bytes());
    }
}
