//! Matching: a direct walk for linear programs, the bounded backtracker
//! ([`super::backtrack`]) at prefilter candidates, and a single-pass
//! leftmost-first PikeVM when the backtracker's window overflows.
//!
//! The PikeVM keeps threads in priority order. A new start thread joins at
//! the lowest priority at each candidate position until some thread matches;
//! a thread reaching `Match` drops every lower-priority thread. One pass over
//! the text is `O(text * insts)` whatever the pattern, and the prefilter skips
//! the text between candidates whenever no thread is alive.

use super::backtrack::{Backtracker, Overflow};
use super::compile::{Inst, Prog};
use super::syntax::{fold, is_word_char, AssertKind};

/// Per-scan working memory, sized once for a program and reused across every
/// `find_at` of the scan.
pub(super) struct Scratch {
    current: Threads,
    next: Threads,
    /// Epsilon-closure work stack; each instruction is visited once per list
    /// and pushes at most two successors, so it never reallocates.
    stack: Vec<usize>,
    backtracker: Backtracker,
}

impl Scratch {
    pub(super) fn new(prog: &Prog) -> Self {
        let n = prog.insts.len();
        Scratch {
            current: Threads::new(n),
            next: Threads::new(n),
            stack: Vec::with_capacity(2 * n + 1),
            backtracker: Backtracker::new(),
        }
    }
}

/// Sparse set of instruction pcs in insertion (priority) order, each with the
/// start offset of its thread; clearing is `O(1)`.
struct Threads {
    dense: Vec<usize>,
    sparse: Vec<usize>,
    starts: Vec<usize>,
}

impl Threads {
    fn new(n: usize) -> Self {
        Threads { dense: Vec::with_capacity(n), sparse: vec![0; n], starts: vec![0; n] }
    }

    #[inline(always)]
    fn contains(&self, pc: usize) -> bool {
        let i = self.sparse[pc];
        i < self.dense.len() && self.dense[i] == pc
    }

    #[inline(always)]
    fn insert(&mut self, pc: usize, start: usize) {
        self.sparse[pc] = self.dense.len();
        self.dense.push(pc);
        self.starts[pc] = start;
    }
}

/// The char at byte `idx` of `text` and its UTF-8 length.
#[inline(always)]
pub(super) fn decode(text: &str, idx: usize) -> Option<(char, usize)> {
    let b = *text.as_bytes().get(idx)?;
    if b < 0x80 {
        return Some((b as char, 1));
    }
    text[idx..].chars().next().map(|c| (c, c.len_utf8()))
}

#[inline(always)]
pub(super) fn byte_eq(hay: u8, b: u8, ci: bool) -> bool {
    if ci { hay | 0x20 == b } else { hay == b }
}

pub(super) fn eval_assert(kind: AssertKind, text: &str, idx: usize) -> bool {
    let bytes = text.as_bytes();
    match kind {
        AssertKind::StartLine => idx == 0 || bytes[idx - 1] == b'\n',
        AssertKind::EndLine => idx == bytes.len() || bytes[idx] == b'\n',
        AssertKind::EndText => idx == bytes.len(),
        AssertKind::WordBound | AssertKind::NotWordBound => {
            let before = text[..idx].chars().next_back().is_some_and(is_word_char);
            let after = decode(text, idx).is_some_and(|(c, _)| is_word_char(c));
            (before != after) == (kind == AssertKind::WordBound)
        }
    }
}

impl Prog {
    /// The leftmost-first match starting at or after `pos`, as `(start, end)`.
    pub(super) fn find_at(&self, text: &str, pos: usize, scratch: &mut Scratch) -> Option<(usize, usize)> {
        if pos > text.len() {
            return None;
        }
        // Every match from `pos` holds the literal, so the scan stops inside the
        // next match and the scans of one `find_iter` never overlap.
        if let Some(required) = &self.required
            && required.find(&text.as_bytes()[pos..]).is_none()
        {
            return None;
        }
        if self.linear { self.find_linear(text, pos) } else { self.find_nfa(text, pos, scratch) }
    }

    /// Backtrack at each candidate in order; the first one that matches holds
    /// the leftmost-first match.
    fn find_nfa(&self, text: &str, mut pos: usize, scratch: &mut Scratch) -> Option<(usize, usize)> {
        let backtracker = &mut scratch.backtracker;
        backtracker.prepare(self, text.len());
        backtracker.reset(pos);
        loop {
            let start = self.prefilter.next(text, pos)?;
            backtracker.slide(start);
            match backtracker.run(self, text, start) {
                Ok(Some(end)) => return Some((start, end)),
                Ok(None) => pos = start + decode(text, start)?.1,
                Err(Overflow) => return self.find_pike(text, start, scratch),
            }
        }
    }

    /// The next instruction when consuming `inst` accepts `c`.
    #[inline(always)]
    pub(super) fn step(&self, inst: &Inst, c: char) -> Option<usize> {
        let (ok, next) = match *inst {
            Inst::Byte { b, ci, next } => (c.is_ascii() && byte_eq(c as u8, b, ci), next),
            Inst::Char { c: want, ci, next } => (if ci { fold(c) == want } else { c == want }, next),
            Inst::Set { class, next } => (self.classes[class].matches(c), next),
            Inst::Any { dot_all, next } => (dot_all || c != '\n', next),
            Inst::Assert { .. } | Inst::Split { .. } | Inst::Jump { .. } | Inst::Match => return None,
        };
        ok.then_some(next)
    }

    fn find_linear(&self, text: &str, mut pos: usize) -> Option<(usize, usize)> {
        loop {
            let start = self.prefilter.next(text, pos)?;
            if let Some(end) = self.match_linear(text, start) {
                return Some((start, end));
            }
            pos = start + decode(text, start)?.1;
        }
    }

    /// End of the match anchored at `pos`, for a `linear` program.
    #[inline]
    fn match_linear(&self, text: &str, pos: usize) -> Option<usize> {
        let bytes = text.as_bytes();
        let (mut pc, mut idx) = (self.start, pos);
        loop {
            let inst = &self.insts[pc];
            match *inst {
                Inst::Match => return Some(idx),
                Inst::Byte { b, ci, next } => {
                    if !byte_eq(*bytes.get(idx)?, b, ci) {
                        return None;
                    }
                    (pc, idx) = (next, idx + 1);
                }
                Inst::Set { class, next } if bytes.get(idx).is_some_and(|&b| b < 0x80) => {
                    if !self.classes[class].matches_ascii(bytes[idx]) {
                        return None;
                    }
                    (pc, idx) = (next, idx + 1);
                }
                _ => {
                    let (c, len) = decode(text, idx)?;
                    (pc, idx) = (self.step(inst, c)?, idx + len);
                }
            }
        }
    }

    pub(super) fn find_pike(&self, text: &str, pos: usize, scratch: &mut Scratch) -> Option<(usize, usize)> {
        let Scratch { current, next, stack, .. } = scratch;
        current.dense.clear();
        let mut matched = None;
        let mut pos = pos;
        // Smallest candidate start not yet passed; `usize::MAX` once there is none.
        let mut candidate = self.prefilter.next(text, pos).unwrap_or(usize::MAX);
        loop {
            if matched.is_none() {
                if candidate < pos {
                    candidate = self.prefilter.next(text, pos).unwrap_or(usize::MAX);
                }
                if current.dense.is_empty() {
                    if candidate == usize::MAX {
                        break;
                    }
                    pos = candidate;
                }
                if candidate == pos {
                    self.add_thread(current, stack, self.start, text, pos, pos);
                }
            }
            if current.dense.is_empty() {
                break;
            }
            let decoded = decode(text, pos);
            next.dense.clear();
            for i in 0..current.dense.len() {
                let pc = current.dense[i];
                let inst = &self.insts[pc];
                if let Inst::Match = inst {
                    matched = Some((current.starts[pc], pos));
                    break;
                }
                if let Some((c, len)) = decoded
                    && let Some(target) = self.step(inst, c)
                {
                    self.add_thread(next, stack, target, text, pos + len, current.starts[pc]);
                }
            }
            std::mem::swap(current, next);
            match decoded {
                Some((_, len)) => pos += len,
                None => break,
            }
        }
        matched
    }

    /// Add `pc` and its epsilon closure at `idx` to `list`, after every
    /// thread already there (lower priority).
    fn add_thread(&self, list: &mut Threads, stack: &mut Vec<usize>, pc: usize, text: &str, idx: usize, start: usize) {
        stack.push(pc);
        while let Some(pc) = stack.pop() {
            if list.contains(pc) {
                continue;
            }
            list.insert(pc, start);
            match self.insts[pc] {
                Inst::Jump { target } => stack.push(target),
                Inst::Split { first, second } => stack.extend([second, first]),
                Inst::Assert { kind, next } if eval_assert(kind, text, idx) => stack.push(next),
                _ => {}
            }
        }
    }
}
