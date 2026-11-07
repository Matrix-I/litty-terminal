//! Bounded backtracker: the fast path for anchored attempts at prefilter
//! candidates.
//!
//! Depth-first search in `Split` priority order returns the leftmost-first
//! match of an anchored attempt directly. A visited bitset over `(offset, pc)`
//! of `Split` states makes each of them run at most once; it is shared by all
//! attempts of one `find_at`, which is sound because whether `(pc, idx)` can
//! reach `Match` does not depend on where the attempt started. Every loop
//! passes a `Split`, so only the straight runs between them can repeat, and
//! work stays linear in the text (times the pattern size). Past the bitset
//! window the caller falls back to the PikeVM.

use super::compile::{Inst, Prog};
use super::exec::{byte_eq, decode, eval_assert};

/// Visited bitset budget (256 KiB), as in the `regex` crate's backtracker.
const VISITED_BITS: usize = 256 * 1024 * 8;

/// The attempt needs text beyond the visited window.
pub(super) struct Overflow;

pub(super) struct Backtracker {
    jobs: Vec<(usize, usize)>,
    visited: Vec<u64>,
    /// Text offset of visited row 0.
    base: usize,
    /// Highest bit set since the last clear, plus one.
    dirty: usize,
    /// Text offsets the window covers, set by `prepare`.
    width: usize,
    /// `(instructions, text length)` the bitset was last sized for.
    sized_for: (usize, usize),
}

impl Backtracker {
    pub(super) fn new() -> Self {
        Backtracker { jobs: Vec::new(), visited: Vec::new(), base: 0, dirty: 0, width: 0, sized_for: (0, 0) }
    }

    /// Start a `find_at` from `pos`: forget states of earlier calls.
    pub(super) fn reset(&mut self, pos: usize) {
        self.visited[..self.dirty.div_ceil(64)].fill(0);
        self.dirty = 0;
        self.base = pos;
    }

    /// Re-base the window at `start` once it has drifted past half of it;
    /// clearing costs no more than the exploration that dirtied the bits.
    pub(super) fn slide(&mut self, start: usize) {
        if start - self.base >= self.width / 2 {
            self.reset(start);
        }
    }

    /// Size the bitset for `text`; a no-op on later calls of the same scan.
    pub(super) fn prepare(&mut self, prog: &Prog, text_len: usize) {
        let n = prog.insts.len().max(1);
        if self.sized_for == (n, text_len) {
            return;
        }
        self.sized_for = (n, text_len);
        self.width = (VISITED_BITS / n).min(text_len + 1);
        let words = (self.width * n).div_ceil(64);
        if self.visited.len() < words {
            self.visited.resize(words, 0);
        }
    }

    /// Mark Split state `(pc, idx)` visited; `Ok(false)` if it already was.
    #[inline(always)]
    fn visit(&mut self, n: usize, pc: usize, idx: usize) -> Result<bool, Overflow> {
        let row = idx - self.base;
        if row >= self.width {
            return Err(Overflow);
        }
        let bit = row * n + pc;
        let (word, mask) = (bit / 64, 1u64 << (bit % 64));
        if self.visited[word] & mask != 0 {
            return Ok(false);
        }
        self.visited[word] |= mask;
        self.dirty = self.dirty.max(bit + 1);
        Ok(true)
    }

    /// End of the leftmost-first match anchored at `start`, if any.
    pub(super) fn run(&mut self, prog: &Prog, text: &str, start: usize) -> Result<Option<usize>, Overflow> {
        let bytes = text.as_bytes();
        let n = prog.insts.len();
        self.jobs.clear();
        self.jobs.push((prog.start, start));
        while let Some((mut pc, mut idx)) = self.jobs.pop() {
            loop {
                let inst = &prog.insts[pc];
                match *inst {
                    Inst::Match => return Ok(Some(idx)),
                    Inst::Jump { target } => pc = target,
                    Inst::Split { first, second } => {
                        if let Inst::Set { class, next } = prog.insts[first]
                            && next == pc
                        {
                            self.run_class_loop(prog, text, pc, class, second, idx)?;
                            break;
                        }
                        if !self.visit(n, pc, idx)? {
                            break;
                        }
                        self.jobs.push((second, idx));
                        pc = first;
                    }
                    Inst::Assert { kind, next } => {
                        if !eval_assert(kind, text, idx) {
                            break;
                        }
                        pc = next;
                    }
                    Inst::Byte { b, ci, next } => {
                        if !bytes.get(idx).is_some_and(|&h| byte_eq(h, b, ci)) {
                            break;
                        }
                        (pc, idx) = (next, idx + 1);
                    }
                    Inst::Set { class, next } if bytes.get(idx).is_some_and(|&b| b < 0x80) => {
                        if !prog.classes[class].matches_ascii(bytes[idx]) {
                            break;
                        }
                        (pc, idx) = (next, idx + 1);
                    }
                    _ => {
                        let Some((c, len)) = decode(text, idx) else { break };
                        let Some(next) = prog.step(inst, c) else { break };
                        (pc, idx) = (next, idx + len);
                    }
                }
            }
        }
        Ok(None)
    }

    /// A greedy `class*` loop (`Split { first: Set -> split, second: exit }`)
    /// run in place: the same visits and exit jobs as stepping through it,
    /// without dispatching each instruction per char.
    fn run_class_loop(&mut self, prog: &Prog, text: &str, split: usize, class: usize, exit: usize, mut idx: usize) -> Result<(), Overflow> {
        let n = prog.insts.len();
        let class = &prog.classes[class];
        let bytes = text.as_bytes();
        while self.visit(n, split, idx)? {
            self.jobs.push((exit, idx));
            let len = match bytes.get(idx) {
                Some(&b) if b < 0x80 => usize::from(class.matches_ascii(b)),
                Some(_) => decode(text, idx).filter(|&(c, _)| class.matches(c)).map_or(0, |(_, len)| len),
                None => 0,
            };
            if len == 0 {
                break;
            }
            idx += len;
        }
        Ok(())
    }
}
