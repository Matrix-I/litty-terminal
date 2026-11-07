//! Zero-dependency, SIMD-accelerated literal matcher + bounded Thompson NFA
//! regular expression engine replacing the heavy `regex` crate (~900 KB binary +
//! large heap DFA caches).
//!
//! Design:
//! 1. **Literal / Smart-Case Fast Path**: When a pattern contains no active regex
//!    metacharacters (or is compiled via [`Regex::new_literal`]), matching uses
//!    architecture SIMD (`NEON` on `aarch64`, `SSE2` on `x86_64`) first-byte
//!    vectorized scanning with zero heap allocation during search.
//! 2. **Bounded PikeVM / Thompson NFA**: Supports `.`, `^`, `$`, `\b`, `\z`,
//!    `\d`/`\D`, `\w`/`\W`, `\s`/`\S`, character classes `[a-z0-9_]` / `[^...]`,
//!    inline flags `(?i)` / `(?s)`, non-capturing `(?:...)` and capturing `(...)`
//!    groups, alternation `|`, and greedy/non-greedy quantifiers `*`, `+`, `?`,
//!    `{m}`, `{m,}`, `{m,n}`. State sets use fixed stack bitsets during `find_iter`
//!    when state count `<= 256` (zero heap allocation per scan).

use std::ops::Range;

/// Maximum compiled NFA instructions to bound memory against pathological patterns.
const MAX_INSTS: usize = 2048;

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
    /// Pure literal substring match (case-sensitive or ASCII case-insensitive).
    Literal {
        needle: Vec<u8>,
        case_insensitive: bool,
    },
    /// Compiled Thompson NFA bytecode.
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

    /// Compile with explicit `case_insensitive` and `regex_syntax` options.
    pub fn with_options(pattern: &str, case_insensitive: bool, regex_mode: bool) -> Result<Self, &'static str> {
        if !regex_mode {
            return Ok(Self {
                engine: Engine::Literal {
                    needle: if case_insensitive {
                        pattern.bytes().map(|b| b.to_ascii_lowercase()).collect()
                    } else {
                        pattern.as_bytes().to_vec()
                    },
                    case_insensitive,
                },
            });
        }

        // If the regex pattern is a plain literal (no metacharacters), use the SIMD literal engine!
        if let Some(lit) = try_extract_literal(pattern, case_insensitive) {
            return Ok(Self {
                engine: Engine::Literal {
                    needle: lit,
                    case_insensitive,
                },
            });
        }

        let prog = Compiler::compile(pattern, case_insensitive)?;
        Ok(Self {
            engine: Engine::Nfa(prog),
        })
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
        match &self.engine {
            Engine::Literal { needle, case_insensitive } => {
                if needle.is_empty() {
                    return Some(Match { text, start: pos, end: pos });
                }
                let hay = &text.as_bytes()[pos..];
                let off = if *case_insensitive {
                    find_literal_ci(hay, needle)?
                } else {
                    find_literal_cs(hay, needle)?
                };
                let start = pos + off;
                Some(Match {
                    text,
                    start,
                    end: start + needle.len(),
                })
            }
            Engine::Nfa(prog) => prog.find_at(text, pos, scratch.get_or_insert_with(|| Scratch::new(prog.insts.len()))),
        }
    }

    /// Iterate over all non-overlapping matches in `text`.
    pub fn find_iter<'r, 't>(&'r self, text: &'t str) -> Matches<'r, 't> {
        Matches {
            re: self,
            text,
            last_end: 0,
            last_match_end: None,
            scratch: None,
        }
    }
}

pub struct Matches<'r, 't> {
    re: &'r Regex,
    text: &'t str,
    last_end: usize,
    last_match_end: Option<usize>,
    scratch: Option<Scratch>,
}

impl<'r, 't> Iterator for Matches<'r, 't> {
    type Item = Match<'t>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.last_end > self.text.len() {
                return None;
            }
            let m = self.re.find_at_with(self.text, self.last_end, &mut self.scratch)?;
            if m.start == m.end {
                // Empty match: advance by one UTF-8 character if it duplicates the end of the previous non-empty match.
                if self.last_match_end == Some(m.end) {
                    if m.end >= self.text.len() {
                        return None;
                    }
                    let ch_len = self.text[m.end..].chars().next().map_or(1, char::len_utf8);
                    self.last_end = m.end + ch_len;
                    continue;
                }
                let ch_len = self.text[m.end..].chars().next().map_or(1, char::len_utf8);
                self.last_end = m.end + ch_len;
                self.last_match_end = Some(m.end);
                return Some(m);
            }
            self.last_end = m.end;
            self.last_match_end = Some(m.end);
            return Some(m);
        }
    }
}

/// If `pattern` has no active regex metacharacters, return its literal byte sequence.
fn try_extract_literal(pattern: &str, case_insensitive: bool) -> Option<Vec<u8>> {
    // Any non-ASCII character with case_insensitive stays literal if ASCII-only folding is used.
    let mut out = Vec::with_capacity(pattern.len());
    let bytes = pattern.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'.' | b'*' | b'+' | b'?' | b'(' | b')' | b'[' | b']' | b'{' | b'}' | b'^' | b'$' | b'|' => {
                return None;
            }
            b'\\' => {
                i += 1;
                if i >= bytes.len() {
                    return None;
                }
                let esc = bytes[i];
                // Metacharacter escapes become literal bytes; class escapes (`\d`, `\s`, `\w`, `\b`, `\z`) use NFA.
                if matches!(esc, b'.' | b'*' | b'+' | b'?' | b'(' | b')' | b'[' | b']' | b'{' | b'}' | b'^' | b'$' | b'|' | b'\\' | b'/' | b'-') {
                    out.push(if case_insensitive { esc.to_ascii_lowercase() } else { esc });
                } else {
                    return None;
                }
            }
            _ => {
                out.push(if case_insensitive { b.to_ascii_lowercase() } else { b });
            }
        }
        i += 1;
    }
    Some(out)
}

// ------------------------------------------------------------------
// SIMD Literal Substring Search (Case-Sensitive & Case-Insensitive)
// ------------------------------------------------------------------

#[inline]
pub fn find_literal_cs(hay: &[u8], needle: &[u8]) -> Option<usize> {
    let n = needle.len();
    if n == 0 {
        return Some(0);
    }
    if hay.len() < n {
        return None;
    }
    let first = needle[0];
    let last = needle[n - 1];
    let limit = hay.len() - n + 1;

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: NEON is mandatory on aarch64.
        unsafe { find_literal_cs_neon(hay, needle, first, last, limit) }
    }
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: SSE2 is mandatory on x86_64.
        unsafe { find_literal_cs_sse2(hay, needle, first, last, limit) }
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        find_literal_cs_scalar(hay, needle, first, last, limit)
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn find_literal_cs_neon(hay: &[u8], needle: &[u8], first: u8, last: u8, limit: usize) -> Option<usize> {
    use std::arch::aarch64::*;
    let n = needle.len();
    let ptr = hay.as_ptr();
    let v_first = vdupq_n_u8(first);
    let v_last = vdupq_n_u8(last);
    let mut i = 0;

    // Fast-path for 4-byte needle like "ERR_"
    let needle_u32 = if n == 4 {
        Some(u32::from_ne_bytes([needle[0], needle[1], needle[2], needle[3]]))
    } else {
        None
    };

    while i + 16 <= limit {
        // SAFETY: `i + 16 <= limit` and `limit = hay.len() - n + 1`, so `i + n - 1 + 16 <= hay.len()`.
        unsafe {
            let b_first = vld1q_u8(ptr.add(i));
            let b_last = vld1q_u8(ptr.add(i + n - 1));
            let eq = vandq_u8(vceqq_u8(b_first, v_first), vceqq_u8(b_last, v_last));
            if vmaxvq_u8(eq) != 0 {
                // Check lanes that matched
                // Shift bits into a 64-bit mask using vshrn_n_u16
                let narrow1 = vshrn_n_u16(vreinterpretq_u16_u8(eq), 4);
                let _narrow2 = vget_lane_u64(vreinterpret_u64_u8(narrow1), 0);
                // If any lane matched, scan those positions
                for k in 0..16 {
                    let pos = i + k;
                    if let Some(target) = needle_u32 {
                        let val = std::ptr::read_unaligned(ptr.add(pos) as *const u32);
                        if val == target {
                            return Some(pos);
                        }
                    } else if *ptr.add(pos) == first
                        && *ptr.add(pos + n - 1) == last
                        && hay.get_unchecked(pos..pos + n) == needle
                    {
                        return Some(pos);
                    }
                }
            }
        }
        i += 16;
    }

    find_literal_cs_scalar(&hay[i..], needle, first, last, limit - i).map(|off| i + off)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn find_literal_cs_sse2(hay: &[u8], needle: &[u8], first: u8, last: u8, limit: usize) -> Option<usize> {
    use std::arch::x86_64::*;
    let n = needle.len();
    let ptr = hay.as_ptr();
    let v_first = _mm_set1_epi8(first as i8);
    let v_last = _mm_set1_epi8(last as i8);
    let mut i = 0;

    while i + 16 <= limit {
        // SAFETY: `i + 16 <= limit` and `i + n - 1 + 16 <= hay.len()`.
        unsafe {
            let b_first = _mm_loadu_si128(ptr.add(i) as *const __m128i);
            let b_last = _mm_loadu_si128(ptr.add(i + n - 1) as *const __m128i);
            let eq = _mm_and_si128(_mm_cmpeq_epi8(b_first, v_first), _mm_cmpeq_epi8(b_last, v_last));
            let mut mask = _mm_movemask_epi8(eq) as u32;
            while mask != 0 {
                let k = mask.trailing_zeros() as usize;
                let pos = i + k;
                if &hay[pos..pos + n] == needle {
                    return Some(pos);
                }
                mask &= mask - 1;
            }
        }
        i += 16;
    }

    find_literal_cs_scalar(&hay[i..], needle, first, last, limit - i).map(|off| i + off)
}

#[inline]
fn find_literal_cs_scalar(hay: &[u8], needle: &[u8], first: u8, last: u8, limit: usize) -> Option<usize> {
    let n = needle.len();
    for i in 0..limit {
        if hay[i] == first && hay[i + n - 1] == last && &hay[i..i + n] == needle {
            return Some(i);
        }
    }
    None
}

#[inline]
pub fn find_literal_ci(hay: &[u8], needle_lower: &[u8]) -> Option<usize> {
    let n = needle_lower.len();
    if n == 0 {
        return Some(0);
    }
    if hay.len() < n {
        return None;
    }
    let first_lo = needle_lower[0];
    let first_hi = first_lo.to_ascii_uppercase();
    let last_lo = needle_lower[n - 1];
    let last_hi = last_lo.to_ascii_uppercase();
    let limit = hay.len() - n + 1;

    for i in 0..limit {
        let b0 = hay[i];
        let bn = hay[i + n - 1];
        if (b0 == first_lo || b0 == first_hi)
            && (bn == last_lo || bn == last_hi)
            && hay[i..i + n].iter().zip(needle_lower).all(|(&h, &nl)| h.to_ascii_lowercase() == nl)
        {
            return Some(i);
        }
    }
    None
}

// ------------------------------------------------------------------
// Bounded Thompson NFA / PikeVM Engine
// ------------------------------------------------------------------

#[derive(Clone, Debug)]
enum AssertKind {
    StartLine,   // ^
    EndLine,     // $
    EndText,     // \z
    WordBound,   // \b
}

#[derive(Clone, Debug)]
enum ClassItem {
    Byte(u8),
    Range(u8, u8),
    Char(char),
    CharRange(char, char),
    Digit,
    Word,
    Space,
}

#[derive(Clone, Debug)]
enum Inst {
    Byte { b: u8, ci: bool, next: usize },
    Char { c: char, ci: bool, next: usize },
    Any { dot_all: bool, next: usize },
    Class { items: Vec<ClassItem>, negated: bool, ci: bool, next: usize },
    Assert { kind: AssertKind, next: usize },
    Split { out1: usize, out2: usize },
    Jump { target: usize },
    Match,
}

#[derive(Clone, Debug)]
struct Prog {
    insts: Vec<Inst>,
    start: usize,
    /// Optional literal prefix bytes to skip non-matching regions via SIMD.
    literal_prefix: Vec<u8>,
    prefix_ci: bool,
    anchored_start: bool,
    /// Every path from `start` to `Match` is the same chain of consuming
    /// instructions (no `Split`, `Jump` or `Assert`), so one pass decides a match.
    linear: bool,
}

/// Per-scan NFA working memory, sized once for a program and reused across
/// every `find_at` of that scan.
struct Scratch {
    curr: Vec<bool>,
    next: Vec<bool>,
    curr_list: Vec<usize>,
    next_list: Vec<usize>,
    /// Epsilon-closure work stack; each instruction is visited once and pushes
    /// at most two successors, so `2 * n_insts + 1` never reallocates.
    stack: Vec<usize>,
}

impl Scratch {
    fn new(n_insts: usize) -> Self {
        Scratch {
            curr: vec![false; n_insts],
            next: vec![false; n_insts],
            curr_list: Vec::with_capacity(n_insts),
            next_list: Vec::with_capacity(n_insts),
            stack: Vec::with_capacity(2 * n_insts + 1),
        }
    }
}

/// `true` when following `next` links from `start` only meets consuming
/// instructions until `Match`.
fn is_linear(insts: &[Inst], start: usize) -> bool {
    let mut pc = start;
    for _ in 0..=insts.len() {
        pc = match &insts[pc] {
            Inst::Byte { next, .. } | Inst::Char { next, .. } | Inst::Any { next, .. } | Inst::Class { next, .. } => *next,
            Inst::Match => return true,
            Inst::Split { .. } | Inst::Jump { .. } | Inst::Assert { .. } => return false,
        };
    }
    false
}

/// One consuming step shared by the NFA and the linear fast path: the next pc
/// when `inst` accepts the character at `idx`.
#[inline(always)]
fn step_inst(inst: &Inst, text: &str, idx: usize) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let b0 = bytes[idx];
    match inst {
        Inst::Byte { b, ci, next } => {
            let ok = if *ci { b0.eq_ignore_ascii_case(b) } else { b0 == *b };
            if ok { Some((*next, 1)) } else { None }
        }
        Inst::Class { items, negated, ci, next } if b0 < 0x80 => {
            // Fast-path ASCII for class
            let ok = eval_class_ascii(items, b0, *ci) != *negated;
            if ok { Some((*next, 1)) } else { None }
        }
        _ => {
            let (ch, step) = if b0 < 0x80 {
                (b0 as char, 1)
            } else {
                let c = text[idx..].chars().next().unwrap_or('\u{FFFD}');
                (c, c.len_utf8())
            };
            let target = match inst {
                Inst::Char { c, ci, next } => {
                    let ok = if *ci { ch.eq_ignore_ascii_case(c) } else { ch == *c };
                    if ok { Some(*next) } else { None }
                }
                Inst::Any { dot_all, next } => (*dot_all || ch != '\n').then_some(*next),
                Inst::Class { items, negated, ci, next } => (eval_class(items, ch, *ci) != *negated).then_some(*next),
                _ => None,
            }?;
            Some((target, step))
        }
    }
}

#[inline(always)]
fn eval_class_ascii(items: &[ClassItem], b: u8, ci: bool) -> bool {
    let b_cmp = if ci { b.to_ascii_lowercase() } else { b };
    for item in items {
        let matched = match *item {
            ClassItem::Byte(tb) => {
                let target = if ci { tb.to_ascii_lowercase() } else { tb };
                b_cmp == target
            }
            ClassItem::Range(lo, hi) => {
                if ci {
                    let lo = lo.to_ascii_lowercase();
                    let hi = hi.to_ascii_lowercase();
                    b_cmp >= lo && b_cmp <= hi
                } else {
                    b >= lo && b <= hi
                }
            }
            ClassItem::Char(c) => (c as u32) < 0x80 && b_cmp == (if ci { c.to_ascii_lowercase() } else { c } as u8),
            ClassItem::CharRange(lo, hi) => (b as char) >= lo && (b as char) <= hi,
            ClassItem::Digit => b.is_ascii_digit(),
            ClassItem::Word => b.is_ascii_alphanumeric() || b == b'_',
            ClassItem::Space => b.is_ascii_whitespace(),
        };
        if matched {
            return true;
        }
    }
    false
}

impl Prog {
    /// End of the match that starts at `pos`, for a `linear` program.
    #[inline(always)]
    fn match_linear(&self, text: &str, pos: usize) -> Option<usize> {
        let (mut pc, mut idx) = (self.start, pos);
        let bytes = text.as_bytes();
        let insts = &self.insts;
        loop {
            match unsafe { insts.get_unchecked(pc) } {
                Inst::Match => return Some(idx),
                Inst::Byte { b, ci, next } => {
                    if idx >= bytes.len() { return None; }
                    let b0 = unsafe { *bytes.get_unchecked(idx) };
                    let ok = if *ci { b0.eq_ignore_ascii_case(b) } else { b0 == *b };
                    if !ok { return None; }
                    pc = *next;
                    idx += 1;
                }
                Inst::Class { items, negated, ci, next } => {
                    if idx >= bytes.len() { return None; }
                    let b0 = unsafe { *bytes.get_unchecked(idx) };
                    if b0 < 0x80 {
                        let ok = eval_class_ascii(items, b0, *ci) != *negated;
                        if !ok { return None; }
                        pc = *next;
                        idx += 1;
                    } else {
                        let (next_pc, step) = step_inst(&insts[pc], text, idx)?;
                        pc = next_pc;
                        idx += step;
                    }
                }
                other => {
                    if idx >= text.len() { return None; }
                    let (next, step) = step_inst(other, text, idx)?;
                    pc = next;
                    idx += step;
                }
            }
        }
    }

    fn find_at<'t>(&self, text: &'t str, start_pos: usize, scratch: &mut Scratch) -> Option<Match<'t>> {
        let bytes = text.as_bytes();
        let Scratch { curr, next, curr_list, next_list, stack } = scratch;
        let mut pos = start_pos;

        loop {
            if pos > bytes.len() {
                return None;
            }
            if self.anchored_start && pos > 0 {
                return None;
            }
            // Fast skip using literal prefix when available.
            if !self.literal_prefix.is_empty() {
                if pos < bytes.len() {
                    let rem = &bytes[pos..];
                    let off = if self.prefix_ci {
                        find_literal_ci(rem, &self.literal_prefix)
                    } else {
                        find_literal_cs(rem, &self.literal_prefix)
                    };
                    let o = off?;
                    pos += o;
                    // Ensure `pos` lands on a valid UTF-8 char boundary.
                    while pos < bytes.len() && !text.is_char_boundary(pos) {
                        pos += 1;
                    }
                } else {
                    return None;
                }
            }

            if self.linear {
                if let Some(end) = self.match_linear(text, pos) {
                    return Some(Match { text, start: pos, end });
                }
                if pos >= bytes.len() {
                    return None;
                }
                pos += text[pos..].chars().next().map_or(1, char::len_utf8);
                continue;
            }

            curr.fill(false);
            curr_list.clear();
            let mut best_end: Option<usize> = None;

            if self.add_state(self.start, text, pos, curr, curr_list, stack) {
                best_end = Some(pos);
            }

            let mut idx = pos;
            while idx < bytes.len() && !curr_list.is_empty() {
                let step = if bytes[idx] < 0x80 { 1 } else { text[idx..].chars().next().map_or(1, char::len_utf8) };
                let next_idx = idx + step;

                next.fill(false);
                next_list.clear();
                let mut matched_here = false;

                for &pc in curr_list.iter() {
                    if let Some((target, _)) = step_inst(&self.insts[pc], text, idx)
                        && self.add_state(target, text, next_idx, next, next_list, stack)
                    {
                        matched_here = true;
                    }
                }

                if matched_here {
                    best_end = Some(next_idx);
                }

                std::mem::swap(curr, next);
                std::mem::swap(curr_list, next_list);
                idx = next_idx;
            }

            if let Some(end) = best_end {
                return Some(Match { text, start: pos, end });
            }

            if pos >= bytes.len() {
                return None;
            }
            let step = text[pos..].chars().next().map_or(1, char::len_utf8);
            pos += step;
        }
    }

    /// Epsilon-closure from `pc` at byte offset `idx` in `text`. Returns `true` if `Inst::Match` is reached.
    fn add_state(&self, pc: usize, text: &str, idx: usize, visited: &mut [bool], list: &mut Vec<usize>, stack: &mut Vec<usize>) -> bool {
        stack.clear();
        stack.push(pc);
        let mut reached_match = false;

        while let Some(cur) = stack.pop() {
            if visited[cur] {
                continue;
            }
            visited[cur] = true;
            match &self.insts[cur] {
                Inst::Match => {
                    reached_match = true;
                }
                Inst::Jump { target } => stack.push(*target),
                Inst::Split { out1, out2 } => {
                    stack.push(*out2);
                    stack.push(*out1);
                }
                Inst::Assert { kind, next } => {
                    if eval_assert(kind, text.as_bytes(), idx) {
                        stack.push(*next);
                    }
                }
                _ => {
                    list.push(cur);
                }
            }
        }
        reached_match
    }
}

#[inline]
fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[inline]
fn eval_assert(kind: &AssertKind, bytes: &[u8], idx: usize) -> bool {
    match kind {
        AssertKind::StartLine => idx == 0 || bytes.get(idx.wrapping_sub(1)) == Some(&b'\n'),
        AssertKind::EndLine => idx == bytes.len() || bytes.get(idx) == Some(&b'\n'),
        AssertKind::EndText => idx == bytes.len(),
        AssertKind::WordBound => {
            let prev = idx > 0 && is_word_byte(bytes[idx - 1]);
            let next = idx < bytes.len() && is_word_byte(bytes[idx]);
            prev != next
        }
    }
}

#[inline]
fn eval_class(items: &[ClassItem], ch: char, ci: bool) -> bool {
    let ch_cmp = if ci { ch.to_ascii_lowercase() } else { ch };
    for item in items {
        let matched = match *item {
            ClassItem::Byte(b) => {
                let target = if ci { b.to_ascii_lowercase() as char } else { b as char };
                ch_cmp == target
            }
            ClassItem::Range(lo, hi) => {
                if ci {
                    let c_lo = ch.to_ascii_lowercase();
                    let c_hi = ch.to_ascii_uppercase();
                    (c_lo as u32 >= lo as u32 && c_lo as u32 <= hi as u32)
                        || (c_hi as u32 >= lo as u32 && c_hi as u32 <= hi as u32)
                } else {
                    ch as u32 >= lo as u32 && ch as u32 <= hi as u32
                }
            }
            ClassItem::Char(c) => {
                let target = if ci { c.to_ascii_lowercase() } else { c };
                ch_cmp == target
            }
            ClassItem::CharRange(lo, hi) => ch_cmp >= lo && ch_cmp <= hi,
            ClassItem::Digit => ch.is_ascii_digit(),
            ClassItem::Word => ch.is_ascii_alphanumeric() || ch == '_',
            ClassItem::Space => ch.is_ascii_whitespace(),
        };
        if matched {
            return true;
        }
    }
    false
}

// ------------------------------------------------------------------
// Recursive-Descent AST + Thompson NFA Compiler
// ------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Ast {
    Empty,
    Byte(u8, bool),
    Char(char, bool),
    Any(bool),
    Class { items: Vec<ClassItem>, negated: bool, ci: bool },
    Assert(AssertKind),
    Concat(Vec<Ast>),
    Alt(Box<Ast>, Box<Ast>),
    Rep { sub: Box<Ast>, min: usize, max: Option<usize>, greedy: bool },
}

struct Parser<'a> {
    src: &'a str,
    chars: std::str::CharIndices<'a>,
    peeked: Option<(usize, char)>,
    ci: bool,
    dot_all: bool,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str, ci: bool) -> Self {
        let mut chars = src.char_indices();
        let peeked = chars.next();
        Self { src, chars, peeked, ci, dot_all: false }
    }

    fn peek(&self) -> Option<char> {
        self.peeked.map(|(_, c)| c)
    }

    fn bump(&mut self) -> Option<char> {
        let (_, c) = self.peeked?;
        self.peeked = self.chars.next();
        Some(c)
    }

    fn parse_expr(&mut self) -> Result<Ast, &'static str> {
        let mut left = self.parse_concat()?;
        while self.peek() == Some('|') {
            self.bump();
            let right = self.parse_concat()?;
            left = Ast::Alt(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_concat(&mut self) -> Result<Ast, &'static str> {
        let mut seq = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            if let Some(node) = self.parse_rep()? {
                seq.push(node);
            }
        }
        Ok(match seq.len() {
            0 => Ast::Empty,
            1 => seq.pop().unwrap(),
            _ => Ast::Concat(seq),
        })
    }

    fn parse_rep(&mut self) -> Result<Option<Ast>, &'static str> {
        let Some(atom) = self.parse_atom()? else {
            return Ok(None);
        };
        let (min, max) = match self.peek() {
            Some('*') => {
                self.bump();
                (0, None)
            }
            Some('+') => {
                self.bump();
                (1, None)
            }
            Some('?') => {
                self.bump();
                (0, Some(1))
            }
            Some('{') => {
                self.bump();
                let (m, n) = self.parse_counted_rep()?;
                (m, n)
            }
            _ => return Ok(Some(atom)),
        };
        let greedy = if self.peek() == Some('?') {
            self.bump();
            false
        } else {
            true
        };
        Ok(Some(Ast::Rep {
            sub: Box::new(atom),
            min,
            max,
            greedy,
        }))
    }

    fn parse_counted_rep(&mut self) -> Result<(usize, Option<usize>), &'static str> {
        let mut min: usize = 0;
        let mut has_min = false;
        while let Some(c) = self.peek() {
            if let Some(d) = c.to_digit(10) {
                min = min.saturating_mul(10).saturating_add(d as usize);
                has_min = true;
                self.bump();
            } else {
                break;
            }
        }
        if !has_min {
            return Err("invalid repetition count");
        }
        let max = if self.peek() == Some(',') {
            self.bump();
            if self.peek() == Some('}') {
                None
            } else {
                let mut m: usize = 0;
                let mut has_m = false;
                while let Some(c) = self.peek() {
                    if let Some(d) = c.to_digit(10) {
                        m = m.saturating_mul(10).saturating_add(d as usize);
                        has_m = true;
                        self.bump();
                    } else {
                        break;
                    }
                }
                if !has_m || m < min {
                    return Err("invalid repetition range");
                }
                Some(m)
            }
        } else {
            Some(min)
        };
        if self.bump() != Some('}') {
            return Err("unclosed repetition brace");
        }
        if max.unwrap_or(min) > 256 {
            return Err("repetition bound too large");
        }
        Ok((min, max))
    }

    fn parse_atom(&mut self) -> Result<Option<Ast>, &'static str> {
        let Some(c) = self.bump() else {
            return Ok(None);
        };
        match c {
            '.' => Ok(Some(Ast::Any(self.dot_all))),
            '^' => Ok(Some(Ast::Assert(AssertKind::StartLine))),
            '$' => Ok(Some(Ast::Assert(AssertKind::EndLine))),
            '*' | '+' | '?' => Err("dangling quantifier"),
            '(' => {
                if self.peek() == Some('?') {
                    self.bump();
                    // Check inline flags `(?i)`, `(?s)`, `(?is)` or non-capturing `(?:...)`
                    let mut saved_ci = self.ci;
                    let mut saved_dot = self.dot_all;
                    let mut flags_only = false;
                    while let Some(f) = self.peek() {
                        match f {
                            'i' => {
                                self.ci = true;
                                self.bump();
                                flags_only = true;
                            }
                            's' => {
                                self.dot_all = true;
                                self.bump();
                                flags_only = true;
                            }
                            ':' => {
                                self.bump();
                                break;
                            }
                            ')' if flags_only => {
                                self.bump();
                                return Ok(None);
                            }
                            _ => return Err("unsupported group modifier"),
                        }
                    }
                    let inner = self.parse_expr()?;
                    if self.bump() != Some(')') {
                        return Err("unclosed group");
                    }
                    std::mem::swap(&mut self.ci, &mut saved_ci);
                    std::mem::swap(&mut self.dot_all, &mut saved_dot);
                    Ok(Some(inner))
                } else {
                    let inner = self.parse_expr()?;
                    if self.bump() != Some(')') {
                        return Err("unclosed group");
                    }
                    Ok(Some(inner))
                }
            }
            ')' => Err("unmatched closing paren"),
            '[' => self.parse_class().map(Some),
            '\\' => self.parse_escape().map(Some),
            ch => Ok(Some(self.char_node(ch))),
        }
    }

    fn char_node(&self, ch: char) -> Ast {
        if ch.is_ascii() {
            Ast::Byte(ch as u8, self.ci)
        } else {
            Ast::Char(ch, self.ci)
        }
    }

    fn parse_escape(&mut self) -> Result<Ast, &'static str> {
        let Some(c) = self.bump() else {
            return Err("trailing backslash");
        };
        match c {
            'd' => Ok(Ast::Class { items: vec![ClassItem::Digit], negated: false, ci: false }),
            'D' => Ok(Ast::Class { items: vec![ClassItem::Digit], negated: true, ci: false }),
            'w' => Ok(Ast::Class { items: vec![ClassItem::Word], negated: false, ci: false }),
            'W' => Ok(Ast::Class { items: vec![ClassItem::Word], negated: true, ci: false }),
            's' => Ok(Ast::Class { items: vec![ClassItem::Space], negated: false, ci: false }),
            'S' => Ok(Ast::Class { items: vec![ClassItem::Space], negated: true, ci: false }),
            'b' => Ok(Ast::Assert(AssertKind::WordBound)),
            'z' | 'Z' => Ok(Ast::Assert(AssertKind::EndText)),
            'n' => Ok(Ast::Byte(b'\n', false)),
            'r' => Ok(Ast::Byte(b'\r', false)),
            't' => Ok(Ast::Byte(b'\t', false)),
            other => Ok(self.char_node(other)),
        }
    }

    fn parse_class(&mut self) -> Result<Ast, &'static str> {
        let negated = if self.peek() == Some('^') {
            self.bump();
            true
        } else {
            false
        };
        let mut items = Vec::new();
        let mut first = true;
        while let Some(c) = self.peek() {
            if c == ']' && !first {
                self.bump();
                return Ok(Ast::Class { items, negated, ci: self.ci });
            }
            first = false;
            self.bump();
            let ch = if c == '\\' {
                let Some(esc) = self.bump() else {
                    return Err("unclosed character class escape");
                };
                match esc {
                    'd' => {
                        items.push(ClassItem::Digit);
                        continue;
                    }
                    'w' => {
                        items.push(ClassItem::Word);
                        continue;
                    }
                    's' => {
                        items.push(ClassItem::Space);
                        continue;
                    }
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    other => other,
                }
            } else {
                c
            };

            // Check for range `a-z`
            if self.peek() == Some('-') {
                let saved = self.chars.clone();
                let saved_peek = self.peeked;
                self.bump(); // consume '-'
                if let Some(end_c) = self.peek() && end_c != ']' {
                    self.bump();
                    let end_ch = if end_c == '\\' {
                        match self.bump() {
                            Some('n') => '\n',
                            Some('r') => '\r',
                            Some('t') => '\t',
                            Some(o) => o,
                            None => return Err("unclosed class range escape"),
                        }
                    } else {
                        end_c
                    };
                    if ch > end_ch {
                        return Err("invalid character class range");
                    }
                    if ch.is_ascii() && end_ch.is_ascii() {
                        items.push(ClassItem::Range(ch as u8, end_ch as u8));
                    } else {
                        items.push(ClassItem::CharRange(ch, end_ch));
                    }
                    continue;
                }
                self.chars = saved;
                self.peeked = saved_peek;
            }

            if ch.is_ascii() {
                items.push(ClassItem::Byte(ch as u8));
            } else {
                items.push(ClassItem::Char(ch));
            }
        }
        let _ = self.src;
        Err("unclosed character class")
    }
}

struct Compiler {
    insts: Vec<Inst>,
}

impl Compiler {
    fn compile(pattern: &str, ci: bool) -> Result<Prog, &'static str> {
        let mut parser = Parser::new(pattern, ci);
        let ast = parser.parse_expr()?;
        if parser.peek().is_some() {
            return Err("unexpected token in regex");
        }
        let mut comp = Self { insts: Vec::new() };
        let start = comp.emit_ast(&ast)?;
        let start_entry = start.entry;
        let match_pc = comp.push(Inst::Match)?;
        comp.patch_tail(&start, match_pc);

        let anchored_start = matches!(ast, Ast::Assert(AssertKind::StartLine));
        let (literal_prefix, prefix_ci) = extract_literal_prefix(&ast);

        let linear = is_linear(&comp.insts, start_entry);
        Ok(Prog {
            insts: comp.insts,
            start: start_entry,
            literal_prefix,
            prefix_ci,
            anchored_start,
            linear,
        })
    }

    fn push(&mut self, inst: Inst) -> Result<usize, &'static str> {
        if self.insts.len() >= MAX_INSTS {
            return Err("regex pattern too complex");
        }
        let pc = self.insts.len();
        self.insts.push(inst);
        Ok(pc)
    }

    fn emit_ast(&mut self, ast: &Ast) -> Result<Frag, &'static str> {
        match ast {
            Ast::Empty => {
                let pc = self.push(Inst::Jump { target: usize::MAX })?;
                Ok(Frag { entry: pc, exits: vec![pc] })
            }
            Ast::Byte(b, ci) => {
                let pc = self.push(Inst::Byte { b: *b, ci: *ci, next: usize::MAX })?;
                Ok(Frag { entry: pc, exits: vec![pc] })
            }
            Ast::Char(c, ci) => {
                let pc = self.push(Inst::Char { c: *c, ci: *ci, next: usize::MAX })?;
                Ok(Frag { entry: pc, exits: vec![pc] })
            }
            Ast::Any(dot_all) => {
                let pc = self.push(Inst::Any { dot_all: *dot_all, next: usize::MAX })?;
                Ok(Frag { entry: pc, exits: vec![pc] })
            }
            Ast::Class { items, negated, ci } => {
                let pc = self.push(Inst::Class {
                    items: items.clone(),
                    negated: *negated,
                    ci: *ci,
                    next: usize::MAX,
                })?;
                Ok(Frag { entry: pc, exits: vec![pc] })
            }
            Ast::Assert(kind) => {
                let pc = self.push(Inst::Assert { kind: kind.clone(), next: usize::MAX })?;
                Ok(Frag { entry: pc, exits: vec![pc] })
            }
            Ast::Concat(nodes) => {
                let mut iter = nodes.iter();
                let Some(first) = iter.next() else {
                    return self.emit_ast(&Ast::Empty);
                };
                let mut acc = self.emit_ast(first)?;
                for node in iter {
                    let next_frag = self.emit_ast(node)?;
                    self.patch_tail(&acc, next_frag.entry);
                    acc = Frag { entry: acc.entry, exits: next_frag.exits };
                }
                Ok(acc)
            }
            Ast::Alt(left, right) => {
                let split_pc = self.push(Inst::Split { out1: usize::MAX, out2: usize::MAX })?;
                let f1 = self.emit_ast(left)?;
                let f2 = self.emit_ast(right)?;
                self.insts[split_pc] = Inst::Split { out1: f1.entry, out2: f2.entry };
                let mut exits = f1.exits;
                exits.extend(f2.exits);
                Ok(Frag { entry: split_pc, exits })
            }
            Ast::Rep { sub, min, max, greedy } => match max {
                Some(m) => self.emit_bounded_rep(sub, *min, *m, *greedy),
                None => self.emit_unbounded_rep(sub, *min, *greedy),
            },
        }
    }

    fn emit_bounded_rep(&mut self, sub: &Ast, min: usize, max: usize, greedy: bool) -> Result<Frag, &'static str> {
        let mut parts: Vec<Frag> = Vec::new();
        for _ in 0..min {
            parts.push(self.emit_ast(sub)?);
        }
        for _ in min..max {
            let split_pc = self.push(Inst::Split { out1: usize::MAX, out2: usize::MAX })?;
            let f = self.emit_ast(sub)?;
            let skip_pc = self.push(Inst::Jump { target: usize::MAX })?;
            self.insts[split_pc] = if greedy {
                Inst::Split { out1: f.entry, out2: skip_pc }
            } else {
                Inst::Split { out1: skip_pc, out2: f.entry }
            };
            let mut exits = f.exits;
            exits.push(skip_pc);
            parts.push(Frag { entry: split_pc, exits });
        }
        if parts.is_empty() {
            return self.emit_ast(&Ast::Empty);
        }
        let mut iter = parts.into_iter();
        let mut acc = iter.next().unwrap();
        for next_f in iter {
            let entry = acc.entry;
            self.patch_tail(&acc, next_f.entry);
            acc = Frag { entry, exits: next_f.exits };
        }
        Ok(acc)
    }

    fn emit_unbounded_rep(&mut self, sub: &Ast, min: usize, greedy: bool) -> Result<Frag, &'static str> {
        let mut prefix: Option<Frag> = None;
        for _ in 0..min {
            let f = self.emit_ast(sub)?;
            prefix = Some(match prefix {
                None => f,
                Some(acc) => {
                    let entry = acc.entry;
                    self.patch_tail(&acc, f.entry);
                    Frag { entry, exits: f.exits }
                }
            });
        }
        // Star loop for the trailing `*`
        let split_pc = self.push(Inst::Split { out1: usize::MAX, out2: usize::MAX })?;
        let body = self.emit_ast(sub)?;
        let out_pc = self.push(Inst::Jump { target: usize::MAX })?;
        self.insts[split_pc] = if greedy {
            Inst::Split { out1: body.entry, out2: out_pc }
        } else {
            Inst::Split { out1: out_pc, out2: body.entry }
        };
        self.patch_tail(&body, split_pc);
        let star_frag = Frag { entry: split_pc, exits: vec![out_pc] };

        match prefix {
            None => Ok(star_frag),
            Some(acc) => {
                let entry = acc.entry;
                self.patch_tail(&acc, star_frag.entry);
                Ok(Frag { entry, exits: star_frag.exits })
            }
        }
    }

    fn patch_tail(&mut self, frag: &Frag, target: usize) {
        for &pc in &frag.exits {
            match &mut self.insts[pc] {
                Inst::Byte { next, .. }
                | Inst::Char { next, .. }
                | Inst::Any { next, .. }
                | Inst::Class { next, .. }
                | Inst::Assert { next, .. } => *next = target,
                Inst::Jump { target: t } => *t = target,
                _ => {}
            }
        }
    }
}

struct Frag {
    entry: usize,
    exits: Vec<usize>,
}

fn extract_literal_prefix(ast: &Ast) -> (Vec<u8>, bool) {
    let mut prefix = Vec::new();
    let mut ci = false;

    fn collect(ast: &Ast, prefix: &mut Vec<u8>, ci: &mut bool) -> bool {
        match ast {
            Ast::Byte(b, b_ci) => {
                if prefix.is_empty() {
                    *ci = *b_ci;
                } else if *ci != *b_ci {
                    return false;
                }
                prefix.push(if *ci { b.to_ascii_lowercase() } else { *b });
                true
            }
            Ast::Concat(nodes) => {
                for n in nodes {
                    match n {
                        Ast::Assert(_) => continue,
                        other => {
                            if !collect(other, prefix, ci) {
                                return false;
                            }
                        }
                    }
                }
                true
            }
            Ast::Rep { sub, min, .. } if *min >= 1 => {
                for _ in 0..*min {
                    if !collect(sub, prefix, ci) {
                        return false;
                    }
                }
                true
            }
            _ => false,
        }
    }

    collect(ast, &mut prefix, &mut ci);
    (prefix, ci)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_fast_path_and_smart_case() {
        let re_cs = Regex::with_options("foo.bar", false, false).unwrap();
        let m: Vec<_> = re_cs.find_iter("xx foo.bar FOO.BAR fooXbar").map(|m| m.as_str()).collect();
        assert_eq!(m, ["foo.bar"]);

        let re_ci = Regex::with_options("foo.bar", true, false).unwrap();
        let m_ci: Vec<_> = re_ci.find_iter("xx foo.bar FOO.BAR fooXbar").map(|m| m.as_str()).collect();
        assert_eq!(m_ci, ["foo.bar", "FOO.BAR"]);
    }

    #[test]
    fn nfa_quantifiers_classes_and_alternation() {
        let re = Regex::new(r"ERR_\d{2,4}|WARN_[A-Z]+").unwrap();
        let m: Vec<_> = re.find_iter("ok ERR_1 bad ERR_4200 and WARN_DISK!").map(|m| m.as_str()).collect();
        assert_eq!(m, ["ERR_4200", "WARN_DISK"]);
    }

    #[test]
    fn anchors_and_word_boundaries() {
        let re = Regex::new(r"^foo\b.*bar$").unwrap();
        assert!(re.find_at("foo_1 bar", 0).is_none());
        assert_eq!(re.find_at("foo 123 bar", 0).map(|m| m.as_str()), Some("foo 123 bar"));
    }

    #[test]
    fn nfa_scan_allocates_once_not_per_match() {
        let re = Regex::new(r"ERR_\d{4}").unwrap();
        let text = "ERR_0001 ok ".repeat(500);
        let mut count = 0;
        let allocations = crate::test_alloc::allocations(|| count = re.find_iter(&text).count());
        assert_eq!(count, 500);
        // Scratch buffers are built once per scan, never once per match.
        assert!(allocations <= 8, "{allocations} allocations for 500 matches");
    }

    #[test]
    fn find_iter_matches_agree_with_find_at() {
        let re = Regex::new(r"ERR_\d{2,4}|WARN_[A-Z]+").unwrap();
        let text = "ok ERR_1 bad ERR_4200 and WARN_DISK! ERR_77 end";
        let via_iter: Vec<_> = re.find_iter(text).map(|m| m.range()).collect();
        let mut via_at = Vec::new();
        let mut pos = 0;
        while let Some(m) = re.find_at(text, pos) {
            via_at.push(m.range());
            pos = m.end();
        }
        assert_eq!(via_iter, via_at);
        assert_eq!(via_iter.len(), 3);
    }

    fn all_matches(prog: &Prog, text: &str) -> Vec<(usize, usize)> {
        let mut scratch = Scratch::new(prog.insts.len());
        let (mut pos, mut out) = (0, Vec::new());
        while let Some(m) = prog.find_at(text, pos, &mut scratch) {
            out.push((m.start(), m.end()));
            pos = if m.end() > m.start() { m.end() } else { m.end() + 1 };
            if pos > text.len() {
                break;
            }
        }
        out
    }

    #[test]
    fn linear_fast_path_agrees_with_the_full_nfa() {
        let texts = [
            "ERR_0001 ERR_12 ERR_1234 xERR_9999 ERR_ERR_0042",
            "héllo wörld 日本語 ERR_5678 naïve",
            "foo.bar fooXbar FOO.BAR foo\nbar",
            "",
            "ERR_",
        ];
        for (pattern, ci) in [(r"ERR_\d{4}", false), (r"foo.bar", true), (r"w.rld", false), (r"\w\w\s", false), (r"é.l", false)] {
            let Engine::Nfa(prog) = Regex::with_options(pattern, ci, true).unwrap().engine else { continue };
            assert!(prog.linear, "{pattern} should be linear");
            let mut full = prog.clone();
            full.linear = false;
            for text in texts {
                assert_eq!(all_matches(&prog, text), all_matches(&full, text), "{pattern:?} on {text:?}");
            }
        }
    }

    #[test]
    fn branching_programs_are_not_linear() {
        for pattern in [r"ERR_\d{2,4}", r"a|b", r"^foo", r"a*b", r"x\b"] {
            let Engine::Nfa(prog) = Regex::new(pattern).unwrap().engine else { panic!("{pattern}") };
            assert!(!prog.linear, "{pattern}");
        }
    }

    #[test]
    fn malformed_patterns_return_err() {
        for bad in ["(", "[abc", "a{5,2}", "*abc", "abc\\"] {
            assert!(Regex::new(bad).is_err(), "expected error for {bad:?}");
        }
    }
}
