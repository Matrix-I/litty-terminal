//! Thompson NFA compiler. `Split` lists its preferred branch first, which the
//! matcher follows to give leftmost-first (Perl / `regex` crate) results.

use super::prefilter::{required_literal, Prefilter};
use super::simd::LiteralFinder;
use super::syntax::{fold, class_matches, Ast, AssertKind, ClassItem};

/// Maximum compiled NFA instructions to bound memory against pathological patterns.
const MAX_INSTS: usize = 2048;

/// Placeholder target patched once the successor is emitted.
const HOLE: usize = usize::MAX;

#[derive(Clone, Debug)]
pub(super) enum Inst {
    /// One ASCII byte; when `ci` it is a lowercase letter matching either case.
    Byte { b: u8, ci: bool, next: usize },
    /// One non-ASCII char; when `ci`, `c` is a [`fold`] key.
    Char { c: char, ci: bool, next: usize },
    /// A char in `Prog::classes[class]`.
    Set { class: usize, next: usize },
    Any { dot_all: bool, next: usize },
    Assert { kind: AssertKind, next: usize },
    /// Try `first`, then `second`.
    Split { first: usize, second: usize },
    Jump { target: usize },
    Match,
}

/// A character class with its ASCII members precomputed as a bitmap.
#[derive(Clone, Debug)]
pub(super) struct Class {
    ascii: [u64; 2],
    items: Vec<ClassItem>,
    negated: bool,
    ci: bool,
}

impl Class {
    pub(super) fn new(items: Vec<ClassItem>, negated: bool, ci: bool) -> Self {
        let mut ascii = [0u64; 2];
        for b in 0..128u8 {
            if class_matches(&items, negated, ci, b as char) {
                ascii[(b >> 6) as usize] |= 1 << (b & 63);
            }
        }
        Class { ascii, items, negated, ci }
    }

    #[inline(always)]
    pub(super) fn matches_ascii(&self, b: u8) -> bool {
        b < 128 && (self.ascii[(b >> 6) as usize] >> (b & 63)) & 1 != 0
    }

    #[inline]
    pub(super) fn matches(&self, c: char) -> bool {
        if c.is_ascii() { self.matches_ascii(c as u8) } else { class_matches(&self.items, self.negated, self.ci, c) }
    }

    /// `true` when no non-ASCII char can match (case folding never maps
    /// non-ASCII onto ASCII, so `ci` does not matter).
    pub(super) fn is_ascii_only(&self) -> bool {
        !self.negated && self.items.iter().all(|item| item.is_ascii_only())
    }
}

#[derive(Clone, Debug)]
pub(super) struct Prog {
    pub(super) insts: Vec<Inst>,
    pub(super) classes: Vec<Class>,
    pub(super) start: usize,
    /// Every path from `start` to `Match` is the same chain of consuming
    /// instructions (no `Split`, `Jump` or `Assert`), so one pass decides a match.
    pub(super) linear: bool,
    pub(super) prefilter: Prefilter,
    /// A literal every match contains; see [`required_literal`].
    pub(super) required: Option<LiteralFinder>,
}

impl Prog {
    pub(super) fn compile(ast: &Ast) -> Result<Prog, &'static str> {
        let mut comp = Compiler { insts: Vec::new(), classes: Vec::new() };
        let frag = comp.emit(ast)?;
        let match_pc = comp.push(Inst::Match)?;
        comp.patch(&frag, match_pc);
        let linear = is_linear(&comp.insts, frag.entry);
        let prefilter = Prefilter::build(ast, &comp.insts, &comp.classes, frag.entry);
        let required = required_literal(ast, &prefilter);
        Ok(Prog { insts: comp.insts, classes: comp.classes, start: frag.entry, linear, prefilter, required })
    }
}

/// `true` when following `next` links from `start` only meets consuming
/// instructions until `Match`.
fn is_linear(insts: &[Inst], start: usize) -> bool {
    let mut pc = start;
    for _ in 0..=insts.len() {
        pc = match &insts[pc] {
            Inst::Byte { next, .. } | Inst::Char { next, .. } | Inst::Set { next, .. } | Inst::Any { next, .. } => *next,
            Inst::Match => return true,
            Inst::Split { .. } | Inst::Jump { .. } | Inst::Assert { .. } => return false,
        };
    }
    false
}

struct Frag {
    entry: usize,
    exits: Vec<usize>,
}

struct Compiler {
    insts: Vec<Inst>,
    classes: Vec<Class>,
}

impl Compiler {
    fn push(&mut self, inst: Inst) -> Result<usize, &'static str> {
        if self.insts.len() >= MAX_INSTS {
            return Err("regex pattern too complex");
        }
        self.insts.push(inst);
        Ok(self.insts.len() - 1)
    }

    fn single(&mut self, inst: Inst) -> Result<Frag, &'static str> {
        let pc = self.push(inst)?;
        Ok(Frag { entry: pc, exits: vec![pc] })
    }

    fn set(&mut self, items: Vec<ClassItem>, negated: bool, ci: bool) -> Result<Frag, &'static str> {
        self.classes.push(Class::new(items, negated, ci));
        self.single(Inst::Set { class: self.classes.len() - 1, next: HOLE })
    }

    fn emit(&mut self, ast: &Ast) -> Result<Frag, &'static str> {
        match ast {
            Ast::Empty => self.single(Inst::Jump { target: HOLE }),
            Ast::Char { c, ci } if c.is_ascii() => {
                let ci = *ci && c.is_ascii_alphabetic();
                self.single(Inst::Byte { b: if ci { c.to_ascii_lowercase() } else { *c } as u8, ci, next: HOLE })
            }
            Ast::Char { c, ci } => self.single(Inst::Char { c: if *ci { fold(*c) } else { *c }, ci: *ci, next: HOLE }),
            Ast::Any { dot_all } => self.single(Inst::Any { dot_all: *dot_all, next: HOLE }),
            Ast::Class { items, negated, ci } => self.set(items.clone(), *negated, *ci),
            Ast::Assert(kind) => self.single(Inst::Assert { kind: *kind, next: HOLE }),
            Ast::Concat(nodes) => {
                let mut frags = Vec::with_capacity(nodes.len());
                for node in nodes {
                    frags.push(self.emit(node)?);
                }
                self.chain(frags)
            }
            Ast::Alt(branches) => self.emit_alt(branches),
            Ast::Rep { sub, min, max: Some(max), greedy } => self.emit_bounded_rep(sub, *min, *max, *greedy),
            Ast::Rep { sub, min, max: None, greedy } => self.emit_unbounded_rep(sub, *min, *greedy),
        }
    }

    /// Concatenate fragments; an empty list is an empty match.
    fn chain(&mut self, frags: Vec<Frag>) -> Result<Frag, &'static str> {
        let mut iter = frags.into_iter();
        let Some(mut acc) = iter.next() else {
            return self.single(Inst::Jump { target: HOLE });
        };
        for next in iter {
            self.patch(&acc, next.entry);
            acc = Frag { entry: acc.entry, exits: next.exits };
        }
        Ok(acc)
    }

    /// `Split(b1, Split(b2, ... bn))`, so earlier branches win.
    fn emit_alt(&mut self, branches: &[Ast]) -> Result<Frag, &'static str> {
        let mut entry = HOLE;
        let mut exits = Vec::new();
        let mut prev_split: Option<usize> = None;
        for (i, branch) in branches.iter().enumerate() {
            let split = if i + 1 < branches.len() { Some(self.push(Inst::Split { first: HOLE, second: HOLE })?) } else { None };
            let frag = self.emit(branch)?;
            let head = split.unwrap_or(frag.entry);
            match prev_split {
                Some(prev) => {
                    if let Inst::Split { second, .. } = &mut self.insts[prev] {
                        *second = head;
                    }
                }
                None => entry = head,
            }
            if let Some(s) = split {
                self.insts[s] = Inst::Split { first: frag.entry, second: HOLE };
            }
            prev_split = split;
            exits.extend(frag.exits);
        }
        Ok(Frag { entry, exits })
    }

    fn emit_split(&mut self, body: usize, skip: usize, greedy: bool) -> Inst {
        if greedy { Inst::Split { first: body, second: skip } } else { Inst::Split { first: skip, second: body } }
    }

    fn emit_bounded_rep(&mut self, sub: &Ast, min: usize, max: usize, greedy: bool) -> Result<Frag, &'static str> {
        let mut parts = Vec::new();
        for _ in 0..min {
            parts.push(self.emit(sub)?);
        }
        for _ in min..max {
            let split = self.push(Inst::Split { first: HOLE, second: HOLE })?;
            let body = self.emit(sub)?;
            let skip = self.push(Inst::Jump { target: HOLE })?;
            self.insts[split] = self.emit_split(body.entry, skip, greedy);
            let mut exits = body.exits;
            exits.push(skip);
            parts.push(Frag { entry: split, exits });
        }
        if parts.is_empty() {
            return self.single(Inst::Jump { target: HOLE });
        }
        self.chain(parts)
    }

    fn emit_unbounded_rep(&mut self, sub: &Ast, min: usize, greedy: bool) -> Result<Frag, &'static str> {
        let mut parts = Vec::new();
        for _ in 0..min {
            parts.push(self.emit(sub)?);
        }
        let split = self.push(Inst::Split { first: HOLE, second: HOLE })?;
        let body = self.emit(sub)?;
        let out = self.push(Inst::Jump { target: HOLE })?;
        self.insts[split] = self.emit_split(body.entry, out, greedy);
        self.patch(&body, split);
        parts.push(Frag { entry: split, exits: vec![out] });
        self.chain(parts)
    }

    fn patch(&mut self, frag: &Frag, target: usize) {
        for &pc in &frag.exits {
            match &mut self.insts[pc] {
                Inst::Byte { next, .. }
                | Inst::Char { next, .. }
                | Inst::Set { next, .. }
                | Inst::Any { next, .. }
                | Inst::Assert { next, .. } => *next = target,
                Inst::Jump { target: t } => *t = target,
                Inst::Split { .. } | Inst::Match => {}
            }
        }
    }
}
