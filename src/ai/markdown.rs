//! Render a small, safe Markdown subset. Never pass model escape sequences through.
pub const BASE: &str = "\x1b[3;36m";
pub use crate::terminal::text::strip_controls as clean;
pub fn render(text: &str) -> String {
    let text = clean(text);
    let mut out = String::from(BASE);
    let (mut fence, mut code, mut bold, mut italic, mut strike) =
        (false, false, false, false, false);
    for line in text.split_inclusive('\n') {
        if line.trim_start().starts_with("```") {
            fence = !fence;
            out.push_str(if fence { "\x1b[0;37m" } else { BASE });
            out.push('\n');
            continue;
        }
        if fence {
            out.push_str(line);
            continue;
        }
        let mut s = line;
        while !s.is_empty() {
            let (token, style) = if s.starts_with('`') {
                code = !code;
                ("`", if code { "\x1b[0;37m" } else { BASE })
            } else if !code && s.starts_with("**") {
                bold = !bold;
                ("**", if bold { "\x1b[1m" } else { "\x1b[22m" })
            } else if !code && s.starts_with("~~") {
                strike = !strike;
                ("~~", if strike { "\x1b[9m" } else { "\x1b[29m" })
            } else if !code && s.starts_with('*') {
                italic = !italic;
                ("*", if italic { "\x1b[23m" } else { "\x1b[3m" })
            } else {
                let c = s.chars().next().unwrap();
                out.push(c);
                s = &s[c.len_utf8()..];
                continue;
            };
            out.push_str(style);
            s = &s[token.len()..];
        }
    }
    out.push_str("\x1b[0m");
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn styles_and_sanitizes() {
        let r = render("**bold** *italic* `code` ~~gone~~\n```sh\necho hi\n```\n\x1b]52;c;bad\x07");
        for s in [
            "\x1b[1mbold\x1b[22m",
            "\x1b[23mitalic\x1b[3m",
            "\x1b[0;37mcode",
            "\x1b[9mgone",
        ] {
            assert!(r.contains(s), "{r:?}");
        }
        assert!(!r.contains("\x1b]52"));
        assert!(r.ends_with("\x1b[0m"));
    }
}
