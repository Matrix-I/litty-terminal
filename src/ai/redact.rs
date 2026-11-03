//! Pattern-based redaction of text that leaves the machine for an AI provider.
//!
//! This is a bounded safety net, not a guarantee: it recognises a fixed set of
//! well-known secret shapes (private keys, AWS access key ids, GitHub tokens,
//! `Bearer` tokens and `password=`-style assignments). Secrets in any other
//! form pass through unchanged, and an assignment whose value is shorter than
//! four characters is left alone to avoid rewriting ordinary prose such as
//! `tokens: 5`.

use regex::{Captures, Regex};
use std::borrow::Cow;
use std::sync::OnceLock;

struct Rule {
    regex: Regex,
    /// Replacement template, `$n` refers to capture groups kept verbatim.
    replacement: &'static str,
}

fn rules() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| {
        let rule = |pattern: &str, replacement: &'static str| Rule {
            regex: Regex::new(pattern).expect("redaction pattern is valid"),
            replacement,
        };
        vec![
            // A key without its END marker (truncated scrollback) is redacted to the end.
            rule(
                r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*?(?:-----END [A-Z0-9 ]*PRIVATE KEY-----|\z)",
                "[REDACTED:private-key]",
            ),
            rule(r"\b(?:AKIA|ASIA|AGPA|AIDA|AROA|ANPA)[A-Z0-9]{16}\b", "[REDACTED:aws-key]"),
            rule(
                r"\b(?:gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{22,})\b",
                "[REDACTED:github-token]",
            ),
            rule(r"(?i)\b(Bearer\s+)[A-Za-z0-9._~+/=-]{8,}", "${1}[REDACTED:bearer]"),
            rule(
                concat!(
                    r#"(?i)\b([A-Za-z0-9_.-]*(?:password|passwd|secret|token|api[_-]?key|access[_-]?key)"#,
                    r#"[A-Za-z0-9_.-]*\s*[=:]\s*)("[^"\n]{4,}"|'[^'\n]{4,}'|[^\s"']{4,})"#,
                ),
                "${1}[REDACTED:secret]",
            ),
        ]
    })
}

/// `text` with every recognised secret replaced by `[REDACTED:kind]`, and how
/// many replacements were made. Unchanged text is returned without copying.
pub fn redact(text: &str) -> (Cow<'_, str>, usize) {
    let mut out = Cow::Borrowed(text);
    let mut count = 0;
    for rule in rules() {
        let mut hits = 0;
        let replaced = rule.regex.replace_all(&out, |caps: &Captures| {
            // Values already replaced by an earlier rule must not be counted twice.
            if caps.get(0).is_some_and(|m| m.as_str().contains("[REDACTED:")) {
                return caps[0].to_string();
            }
            hits += 1;
            let mut expanded = String::new();
            caps.expand(rule.replacement, &mut expanded);
            expanded
        });
        if hits > 0 {
            count += hits;
            out = Cow::Owned(replaced.into_owned());
        }
    }
    (out, count)
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
