//! Escaping for values written into a curl config (`-K -`), where a quote or
//! newline would otherwise end the value and start another directive.

/// Quote `s` as a curl config string: `\\`, `"`, `\n` and `\r` are escaped.
pub(crate) fn quote(s: &str) -> String {
    let extra = s.bytes().filter(|b| matches!(b, b'\\' | b'"' | b'\n' | b'\r')).count();
    let mut out = String::with_capacity(s.len() + extra + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::quote;

    #[test]
    fn curl_config_escapes() {
        assert_eq!(quote("a\"\\\nb"), "\"a\\\"\\\\\\nb\"");
    }

    #[test]
    fn curl_quote_matches_original_with_one_allocation() {
        for input in [String::new(), "x".repeat(500_000), "\\\"\n\rx".repeat(100_000), "@/tmp/secret\té😀".into()] {
            let expected = format!("\"{}\"", input.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\r', "\\r"));
            assert_eq!(crate::test_alloc::allocations(|| assert_eq!(quote(&input), expected)), 1);
        }
    }
}
