//! Pattern-based redaction of text that leaves the machine for an AI provider.
//!
//! This is a bounded safety net, not a guarantee: it recognises a fixed set of
//! well-known secret shapes (private keys, AWS access key ids, GitHub tokens,
//! `Bearer` tokens and `password=`-style assignments). Secrets in any other
//! form pass through unchanged, and an assignment whose value is shorter than
//! four characters is left alone to avoid rewriting ordinary prose such as
//! `tokens: 5`.
//!
//! Implemented as a zero-dependency linear scanner so no `regex` DFA engine or
//! heap tables are needed in the binary.

use std::borrow::Cow;

use crate::terminal::regex_lite::find_literal_cs;

type RedactPass = fn(&str) -> Option<(String, usize)>;

/// `text` with every recognised secret replaced by `[REDACTED:kind]`, and how
/// many replacements were made. Unchanged text is returned without copying.
pub fn redact(text: &str) -> (Cow<'_, str>, usize) {
    let mut out = Cow::Borrowed(text);
    let mut total_count = 0;

    let passes: [RedactPass; 5] = [
        redact_private_keys,
        redact_aws_keys,
        redact_github_tokens,
        redact_bearer_tokens,
        redact_assignments,
    ];

    for pass in passes {
        if let Some((replaced, hits)) = pass(&out) {
            total_count += hits;
            out = Cow::Owned(replaced);
        }
    }

    (out, total_count)
}

#[inline]
fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[inline]
fn at_word_start(bytes: &[u8], i: usize) -> bool {
    i == 0 || !is_word_byte(bytes[i - 1])
}

#[inline]
fn at_word_end(bytes: &[u8], i: usize) -> bool {
    i >= bytes.len() || !is_word_byte(bytes[i])
}

/// Rule 1: `-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*?(?:-----END [A-Z0-9 ]*PRIVATE KEY-----|\z)`
fn redact_private_keys(text: &str) -> Option<(String, usize)> {
    const BEGIN: &[u8] = b"-----BEGIN ";
    const PK_TAIL: &[u8] = b"PRIVATE KEY-----";
    const END: &[u8] = b"-----END ";

    let bytes = text.as_bytes();
    let mut pos = 0;
    let mut out: Option<String> = None;
    let mut last_copy = 0;
    let mut hits = 0;

    while let Some(off) = find_literal_cs(&bytes[pos..], BEGIN) {
        let start = pos + off;
        let label_start = start + BEGIN.len();
        let line_end = bytes[label_start..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(bytes.len(), |p| label_start + p);
        let Some(pk_off) = find_literal_cs(&bytes[label_start..line_end], PK_TAIL) else {
            pos = label_start;
            continue;
        };
        if !bytes[label_start..label_start + pk_off]
            .iter()
            .all(|&b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b' ')
        {
            pos = label_start;
            continue;
        }
        let body_start = label_start + pk_off + PK_TAIL.len();
        // Search for `-----END [A-Z0-9 ]*PRIVATE KEY-----` or end of text.
        let mut match_end = bytes.len();
        let mut scan = body_start;
        while let Some(end_off) = find_literal_cs(&bytes[scan..], END) {
            let end_pos = scan + end_off;
            let end_label = end_pos + END.len();
            let end_line = bytes[end_label..]
                .iter()
                .position(|&b| b == b'\n')
                .map_or(bytes.len(), |p| end_label + p);
            if let Some(end_pk) = find_literal_cs(&bytes[end_label..end_line], PK_TAIL)
                && bytes[end_label..end_label + end_pk]
                    .iter()
                    .all(|&b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b' ')
            {
                match_end = end_label + end_pk + PK_TAIL.len();
                break;
            }
            scan = end_label;
        }

        let matched = &text[start..match_end];
        if !matched.contains("[REDACTED:") {
            let buf = out.get_or_insert_with(|| String::with_capacity(text.len()));
            buf.push_str(&text[last_copy..start]);
            buf.push_str("[REDACTED:private-key]");
            last_copy = match_end;
            hits += 1;
        }
        pos = match_end;
    }

    out.map(|mut buf| {
        buf.push_str(&text[last_copy..]);
        (buf, hits)
    })
}

/// Rule 2: `\b(?:AKIA|ASIA|AGPA|AIDA|AROA|ANPA)[A-Z0-9]{16}\b`
fn redact_aws_keys(text: &str) -> Option<(String, usize)> {
    const PREFIXES: [[u8; 4]; 6] = [*b"AKIA", *b"ASIA", *b"AGPA", *b"AIDA", *b"AROA", *b"ANPA"];
    let bytes = text.as_bytes();
    if bytes.len() < 20 {
        return None;
    }
    let mut out: Option<String> = None;
    let mut last_copy = 0;
    let mut hits = 0;
    let mut i = 0;

    while i + 20 <= bytes.len() {
        if at_word_start(bytes, i)
            && PREFIXES.iter().any(|p| &bytes[i..i + 4] == p)
            && bytes[i + 4..i + 20].iter().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            && at_word_end(bytes, i + 20)
        {
            let matched = &text[i..i + 20];
            if !matched.contains("[REDACTED:") {
                let buf = out.get_or_insert_with(|| String::with_capacity(text.len()));
                buf.push_str(&text[last_copy..i]);
                buf.push_str("[REDACTED:aws-key]");
                last_copy = i + 20;
                hits += 1;
            }
            i += 20;
        } else {
            i += 1;
        }
    }

    out.map(|mut buf| {
        buf.push_str(&text[last_copy..]);
        (buf, hits)
    })
}

/// Rule 3: `\b(?:gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{22,})\b`
fn redact_github_tokens(text: &str) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    let mut out: Option<String> = None;
    let mut last_copy = 0;
    let mut hits = 0;
    let mut i = 0;

    while i < bytes.len() {
        if at_word_start(bytes, i) {
            if i + 40 <= bytes.len()
                && bytes[i] == b'g'
                && bytes[i + 1] == b'h'
                && matches!(bytes[i + 2], b'p' | b'o' | b'u' | b's' | b'r')
                && bytes[i + 3] == b'_'
            {
                let mut end = i + 4;
                while end < bytes.len() && bytes[end].is_ascii_alphanumeric() {
                    end += 1;
                }
                if end - (i + 4) >= 36 && at_word_end(bytes, end) {
                    let buf = out.get_or_insert_with(|| String::with_capacity(text.len()));
                    buf.push_str(&text[last_copy..i]);
                    buf.push_str("[REDACTED:github-token]");
                    last_copy = end;
                    hits += 1;
                    i = end;
                    continue;
                }
            } else if bytes[i..].starts_with(b"github_pat_") {
                let mut end = i + 11;
                while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                    end += 1;
                }
                if end - (i + 11) >= 22 && at_word_end(bytes, end) {
                    let buf = out.get_or_insert_with(|| String::with_capacity(text.len()));
                    buf.push_str(&text[last_copy..i]);
                    buf.push_str("[REDACTED:github-token]");
                    last_copy = end;
                    hits += 1;
                    i = end;
                    continue;
                }
            }
        }
        i += 1;
    }

    out.map(|mut buf| {
        buf.push_str(&text[last_copy..]);
        (buf, hits)
    })
}

/// Rule 4: `(?i)\b(Bearer\s+)[A-Za-z0-9._~+/=-]{8,}`
fn redact_bearer_tokens(text: &str) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    let mut out: Option<String> = None;
    let mut last_copy = 0;
    let mut hits = 0;
    let mut i = 0;

    let is_bearer_tok = |b: u8| {
        b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'~' | b'+' | b'/' | b'=' | b'-')
    };

    while i + 15 <= bytes.len() {
        if at_word_start(bytes, i) && bytes[i..i + 6].eq_ignore_ascii_case(b"bearer") && bytes[i + 6].is_ascii_whitespace() {
            let mut ws_end = i + 6;
            while ws_end < bytes.len() && bytes[ws_end].is_ascii_whitespace() {
                ws_end += 1;
            }
            let mut tok_end = ws_end;
            while tok_end < bytes.len() && is_bearer_tok(bytes[tok_end]) {
                tok_end += 1;
            }
            if tok_end - ws_end >= 8 {
                let matched = &text[i..tok_end];
                if !matched.contains("[REDACTED:") {
                    let buf = out.get_or_insert_with(|| String::with_capacity(text.len()));
                    buf.push_str(&text[last_copy..ws_end]);
                    buf.push_str("[REDACTED:bearer]");
                    last_copy = tok_end;
                    hits += 1;
                }
                i = tok_end;
                continue;
            }
        }
        i += 1;
    }

    out.map(|mut buf| {
        buf.push_str(&text[last_copy..]);
        (buf, hits)
    })
}

/// Rule 5: `(?i)\b([A-Za-z0-9_.-]*(?:password|passwd|secret|token|api[_-]?key|access[_-]?key)[A-Za-z0-9_.-]*\s*[=:]\s*)("[^"\n]{4,}"|'[^'\n]{4,}'|[^\s"']{4,})`
fn redact_assignments(text: &str) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    let mut out: Option<String> = None;
    let mut last_copy = 0;
    let mut hits = 0;
    let mut i = 0;

    let is_ident_char = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-');

    while i < bytes.len() {
        if at_word_start(bytes, i) && is_ident_char(bytes[i]) {
            let key_start = i;
            let mut key_end = i;
            while key_end < bytes.len() && is_ident_char(bytes[key_end]) {
                key_end += 1;
            }
            if has_secret_keyword(&bytes[key_start..key_end]) {
                let mut sep = key_end;
                while sep < bytes.len() && bytes[sep].is_ascii_whitespace() {
                    sep += 1;
                }
                if sep < bytes.len() && matches!(bytes[sep], b'=' | b':') {
                    sep += 1;
                    while sep < bytes.len() && bytes[sep].is_ascii_whitespace() {
                        sep += 1;
                    }
                    if let Some(val_end) = parse_assignment_value(text, sep) {
                        let full_match = &text[key_start..val_end];
                        if !full_match.contains("[REDACTED:") {
                            let buf = out.get_or_insert_with(|| String::with_capacity(text.len()));
                            buf.push_str(&text[last_copy..sep]);
                            buf.push_str("[REDACTED:secret]");
                            last_copy = val_end;
                            hits += 1;
                        }
                        i = val_end;
                        continue;
                    }
                }
            }
            i = key_end;
        } else {
            i += 1;
        }
    }

    out.map(|mut buf| {
        buf.push_str(&text[last_copy..]);
        (buf, hits)
    })
}

fn has_secret_keyword(ident: &[u8]) -> bool {
    let mut lower = [0u8; 128];
    let len = ident.len().min(lower.len());
    for (dst, &src) in lower[..len].iter_mut().zip(&ident[..len]) {
        *dst = src.to_ascii_lowercase();
    }
    let s = &lower[..len];
    for kw in [b"password".as_slice(), b"passwd", b"secret", b"token", b"apikey", b"api_key", b"api-key", b"accesskey", b"access_key", b"access-key"] {
        if find_literal_cs(s, kw).is_some() {
            return true;
        }
    }
    false
}

fn parse_assignment_value(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    if start >= bytes.len() {
        return None;
    }
    let first = bytes[start];
    if first == b'"' || first == b'\'' {
        let mut end = start + 1;
        let mut chars = 0usize;
        while end < bytes.len() && bytes[end] != first && bytes[end] != b'\n' {
            let step = text[end..].chars().next().map_or(1, char::len_utf8);
            end += step;
            chars += 1;
        }
        if end < bytes.len() && bytes[end] == first && chars >= 4 {
            return Some(end + 1);
        }
        None
    } else {
        let mut end = start;
        let mut chars = 0usize;
        while end < bytes.len() && !bytes[end].is_ascii_whitespace() && bytes[end] != b'"' && bytes[end] != b'\'' {
            let step = text[end..].chars().next().map_or(1, char::len_utf8);
            end += step;
            chars += 1;
        }
        (chars >= 4).then_some(end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(text: &str) -> (String, usize) {
        let (out, n) = redact(text);
        (out.into_owned(), n)
    }

    #[test]
    fn ordinary_text_is_returned_unchanged_without_copying() {
        for text in [
            "ls -la /tmp",
            "the password prompt appeared",
            "tokens: 5",
            "error: missing file",
            "git push origin main",
            "export PATH=/usr/bin",
            "Bearer",
            "",
        ] {
            let (out, n) = redact(text);
            assert_eq!(n, 0, "{text}");
            assert!(matches!(out, Cow::Borrowed(_)), "{text}");
        }
    }

    #[test]
    fn aws_and_github_tokens_are_replaced() {
        let (out, n) = clean("key AKIAIOSFODNN7EXAMPLE and ghp_abcdefghijklmnopqrstuvwxyz0123456789 end");
        assert_eq!(out, "key [REDACTED:aws-key] and [REDACTED:github-token] end");
        assert_eq!(n, 2);
        let (out, _) = clean("github_pat_11ABCDEFG0123456789_abcdefghij");
        assert_eq!(out, "[REDACTED:github-token]");
    }

    #[test]
    fn bearer_tokens_keep_the_scheme_word() {
        let (out, n) = clean("curl -H 'Authorization: Bearer eyJhbGciOi.abc-def_123'");
        assert_eq!(out, "curl -H 'Authorization: Bearer [REDACTED:bearer]'");
        assert_eq!(n, 1);
    }

    #[test]
    fn assignments_keep_the_name_and_drop_the_value() {
        let (out, n) = clean("password=LITTY_FAKE_PROBE_074\nAPI_KEY: \"abc def ghi\"\nmy-token = 's3cr3t!'");
        assert_eq!(
            out,
            "password=[REDACTED:secret]\nAPI_KEY: [REDACTED:secret]\nmy-token = [REDACTED:secret]"
        );
        assert_eq!(n, 3);
    }

    #[test]
    fn short_assignment_values_are_left_alone() {
        assert_eq!(clean("password=abc"), ("password=abc".into(), 0));
    }

    #[test]
    fn multiline_private_keys_are_replaced_as_one_block() {
        let text = "before\n-----BEGIN RSA PRIVATE KEY-----\nMIIEow\nIBAAKC\n-----END RSA PRIVATE KEY-----\nafter";
        let (out, n) = clean(text);
        assert_eq!(out, "before\n[REDACTED:private-key]\nafter");
        assert_eq!(n, 1);
    }

    #[test]
    fn truncated_private_keys_are_replaced_to_the_end() {
        let (out, n) = clean("x\n-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1r");
        assert_eq!(out, "x\n[REDACTED:private-key]");
        assert_eq!(n, 1);
    }

    #[test]
    fn redaction_is_idempotent() {
        let (once, _) = clean("password=hunter22hunter AKIAIOSFODNN7EXAMPLE");
        let (twice, n) = clean(&once);
        assert_eq!(once, twice);
        assert_eq!(n, 0);
    }
}
