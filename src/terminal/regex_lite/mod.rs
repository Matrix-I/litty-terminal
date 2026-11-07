//! Zero-dependency regular expressions for find-in-terminal, replacing the
//! `regex` crate (~900 KB binary + large heap DFA caches).
//!
//! Semantics follow the `regex` crate: leftmost-first matches (earlier
//! alternation branches and lazy quantifiers win), and non-overlapping
//! iteration that skips an empty match right after the previous match.
//! Differences, kept for size: `^`/`$` always match at line breaks, `\d` is
//! ASCII, and Unicode (`\w`, `\s`, `\b`, case folding) comes from `std`
//! tables (see [`syntax`]).
//!
//! Supported syntax: `.`, `^`, `$`, `\b`, `\B`, `\z`, `\d`/`\D`, `\w`/`\W`,
//! `\s`/`\S`, classes `[a-z0-9_]` / `[^...]`, flags `(?i)` / `(?s)`, groups
//! `(...)` / `(?:...)` / `(?i:...)`, alternation `|`, and greedy or lazy
//! `*`, `+`, `?`, `{m}`, `{m,}`, `{m,n}`.
//!
//! Matching is a single-pass PikeVM ([`exec`]) that only starts threads where
//! a SIMD prefilter ([`prefilter`], [`simd`]) finds a possible match start.
//! Patterns without metacharacters skip the NFA entirely.

mod backtrack;
mod compile;
mod exec;
mod prefilter;
mod simd;
mod syntax;

use std::ops::Range;

use compile::Prog;
use exec::Scratch;
pub use simd::find_literal_cs;
use simd::LiteralFinder;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match<'t> {
    text: &'t str,
    start: usize,
    end: usize,
}

impl<'t> Match<'t> {
    #[inline]
    pub fn start(&self) -> usize {
        self.start
    }

    #[inline]
    pub fn end(&self) -> usize {
        self.end
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    #[inline]
    pub fn as_str(&self) -> &'t str {
        &self.text[self.start..self.end]
    }

    #[inline]
    pub fn range(&self) -> Range<usize> {
        self.start..self.end
    }
}

#[derive(Clone, Debug)]
enum Engine {
    /// Plain substring (ASCII case-insensitive when built so).
    Literal(LiteralFinder),
    Nfa(Prog),
}

#[derive(Clone, Debug)]
pub struct Regex {
    engine: Engine,
}

impl Regex {
    /// Compile a regular expression with default flags (case-sensitive).
    pub fn new(pattern: &str) -> Result<Self, &'static str> {
        Self::with_options(pattern, false, true)
    }

    /// Compile `pattern` as a regex (`regex_mode`) or as a literal string,
    /// optionally case-insensitive.
    pub fn with_options(pattern: &str, case_insensitive: bool, regex_mode: bool) -> Result<Self, &'static str> {
        let literal = if regex_mode { unescape_literal(pattern) } else { Some(pattern.to_string()) };
        // The SIMD substring engine folds ASCII only; other case-insensitive
        // literals go through the NFA, which folds Unicode.
        let engine = match literal {
            Some(lit) if !case_insensitive || lit.is_ascii() => Engine::Literal(LiteralFinder::new(lit.as_bytes(), case_insensitive)),
            Some(lit) => Engine::Nfa(Prog::compile(&syntax::literal(&lit, case_insensitive))?),
            None => Engine::Nfa(Prog::compile(&syntax::parse(pattern, case_insensitive)?)?),
        };
        Ok(Self { engine })
    }

    /// Find the first match in `text` starting at or after `pos`.
    pub fn find_at<'t>(&self, text: &'t str, pos: usize) -> Option<Match<'t>> {
        self.find_at_with(text, pos, &mut None)
    }

    /// `find_at` reusing `scratch`, which is filled on first use by an NFA program.
    fn find_at_with<'t>(&self, text: &'t str, pos: usize, scratch: &mut Option<Scratch>) -> Option<Match<'t>> {
        if pos > text.len() {
            return None;
        }
        let (start, end) = match &self.engine {
            Engine::Literal(finder) => {
                let start = pos + finder.find(&text.as_bytes()[pos..])?;
                (start, start + finder.needle().len())
            }
            Engine::Nfa(prog) => prog.find_at(text, pos, scratch.get_or_insert_with(|| Scratch::new(prog)))?,
        };
        Some(Match { text, start, end })
    }

    /// Iterate over all non-overlapping matches in `text`.
    pub fn find_iter<'r, 't>(&'r self, text: &'t str) -> Matches<'r, 't> {
        Matches { re: self, text, last_end: 0, last_match_end: None, scratch: None }
    }
}

pub struct Matches<'r, 't> {
    re: &'r Regex,
    text: &'t str,
    last_end: usize,
    last_match_end: Option<usize>,
    scratch: Option<Scratch>,
}

impl<'t> Iterator for Matches<'_, 't> {
    type Item = Match<'t>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.last_end > self.text.len() {
                return None;
            }
            let m = self.re.find_at_with(self.text, self.last_end, &mut self.scratch)?;
            if m.is_empty() {
                let ch_len = self.text[m.end..].chars().next().map_or(1, char::len_utf8);
                self.last_end = m.end + ch_len;
                // An empty match right after the previous match is skipped.
                if self.last_match_end == Some(m.end) {
                    continue;
                }
                self.last_match_end = Some(m.end);
                return Some(m);
            }
            self.last_end = m.end;
            self.last_match_end = Some(m.end);
            return Some(m);
        }
    }
}

/// The literal text of `pattern` when it has no active regex metacharacters.
fn unescape_literal(pattern: &str) -> Option<String> {
    const META: &[char] = &['.', '*', '+', '?', '(', ')', '[', ']', '{', '}', '^', '$', '|'];
    let mut out = String::with_capacity(pattern.len());
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        match c {
            _ if META.contains(&c) => return None,
            // Escaped punctuation is literal; class escapes (`\d`, `\b`, ...) need the NFA.
            '\\' => match chars.next()? {
                esc if META.contains(&esc) || matches!(esc, '\\' | '/' | '-') => out.push(esc),
                _ => return None,
            },
            _ => out.push(c),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests;
