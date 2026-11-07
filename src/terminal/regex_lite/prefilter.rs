//! Candidate start positions: the matcher only starts threads where a
//! prefilter says a match can begin, and skips straight between them.
//!
//! In order of preference:
//! 1. `Literal` / `Literals`: every match starts with one of a few literals
//!    (`ERR_\d+`, `ERR|WARN`, `(?i)error`).
//! 2. `Literal` at an `offset`: every match has a literal exactly `offset`
//!    bytes after its start, all of them ASCII-only (`\d{4}-\d{2}`).
//! 3. `FirstByte`: the set of bytes a match can start with (`\d+\.`).

use super::compile::{Class, Inst};
use super::simd::{rank, starts_with, ByteSet, LiteralFinder};
use super::syntax::{case_variants, Ast};

/// Prefix literal sets stay small: each candidate is verified against all of them.
const MAX_LITERALS: usize = 8;
const MAX_LITERAL_LEN: usize = 32;
/// UTF-8 lead bytes of non-ASCII chars.
const LEAD_BYTES: std::ops::RangeInclusive<u8> = 0xC2..=0xF4;

#[derive(Clone, Debug)]
pub(super) enum Prefilter {
    /// Any char boundary may start a match.
    None,
    /// Every match holds `finder`'s literal exactly `offset` bytes after its start.
    Literal { finder: LiteralFinder, offset: usize },
    /// Every match starts with one of `needles` (lowercased when `ci`), all of
    /// which hold a byte of `probe` at `offset`.
    Literals { needles: Vec<Vec<u8>>, ci: bool, probe: ByteSet, offset: usize },
    /// Every match starts with one of these bytes.
    FirstByte(ByteSet),
}

impl Prefilter {
    pub(super) fn build(ast: &Ast, insts: &[Inst], classes: &[Class], start: usize) -> Prefilter {
        let prefixes = prefix_literals(ast);
        let shortest = prefixes.as_ref().map_or(0, |(set, _)| set.iter().map(Vec::len).min().unwrap_or(0));
        if shortest < 2
            && let Some((needle, ci, offset)) = inner_literal(ast)
            && needle.len() > shortest
        {
            return Prefilter::Literal { finder: LiteralFinder::new(&needle, ci), offset };
        }
        if let Some((mut needles, ci)) = prefixes {
            if needles.len() == 1 {
                return Prefilter::Literal { finder: LiteralFinder::new(&needles[0], ci), offset: 0 };
            }
            if ci {
                needles.iter_mut().for_each(|n| n.make_ascii_lowercase());
            }
            let (probe, offset) = literals_probe(&needles, ci);
            return Prefilter::Literals { needles, ci, probe, offset };
        }
        match first_bytes(insts, classes, start) {
            Some(set) if set.len() < 128 => Prefilter::FirstByte(set),
            _ => Prefilter::None,
        }
    }

    /// The smallest char boundary `>= pos` where a match may start.
    #[inline]
    pub(super) fn next(&self, text: &str, pos: usize) -> Option<usize> {
        let bytes = text.as_bytes();
        match self {
            Prefilter::None => (pos <= bytes.len()).then_some(pos),
            Prefilter::Literal { finder, offset } => {
                let mut from = pos + offset;
                loop {
                    let at = from + finder.find(bytes.get(from..)?)?;
                    // Only ASCII bytes precede the literal, so a valid start is a boundary.
                    if text.is_char_boundary(at - offset) {
                        return Some(at - offset);
                    }
                    from = at + 1;
                }
            }
            Prefilter::Literals { needles, ci, probe, offset } => {
                let mut from = pos + offset;
                loop {
                    let start = probe.find(bytes, from)? - offset;
                    // A verified needle starts with an ASCII or lead byte, so on a boundary.
                    if needles.iter().any(|n| starts_with(&bytes[start..], n, *ci)) {
                        return Some(start);
                    }
                    from = start + offset + 1;
                }
            }
            Prefilter::FirstByte(set) => set.find(bytes, pos),
        }
    }
}

/// The offset, shared by all needles, whose bytes are fewest and rarest, so
/// the SIMD byte scan stops at few false candidates (`(?i)tiếng|việt` probes
/// the UTF-8 lead byte at offset 2, absent from ASCII text).
fn literals_probe(needles: &[Vec<u8>], ci: bool) -> (ByteSet, usize) {
    let shortest = needles.iter().map(Vec::len).min().unwrap_or(0);
    let bytes_at = |k: usize| {
        let mut bits = [false; 256];
        for needle in needles {
            bits[needle[k] as usize] = true;
            if ci {
                bits[needle[k].to_ascii_uppercase() as usize] = true;
            }
        }
        bits
    };
    let score = |bits: &[bool; 256]| {
        let members = (0..=255u8).filter(|&b| bits[b as usize]);
        (members.clone().count() > 4, members.map(rank).max().unwrap_or(0))
    };
    let best = (0..shortest).min_by_key(|&k| (score(&bytes_at(k)), k)).unwrap_or(0);
    (ByteSet::new(bytes_at(best)), best)
}

/// A set of literals one of which starts every match; `exact` when the set is
/// the node's whole language, so what follows can be appended.
#[derive(Clone)]
struct Lits {
    strings: Vec<Vec<u8>>,
    exact: bool,
}

impl Lits {
    fn empty(exact: bool) -> Self {
        Lits { strings: vec![Vec::new()], exact }
    }

    /// Append `next` to every string; `false` once the set stops growing.
    fn cross(&mut self, next: Lits) -> bool {
        if !self.exact {
            return false;
        }
        if self.strings.len() * next.strings.len() > MAX_LITERALS {
            self.exact = false;
            return false;
        }
        let mut truncated = false;
        let mut out = Vec::with_capacity(self.strings.len() * next.strings.len());
        for a in &self.strings {
            for b in &next.strings {
                let mut s = [a.as_slice(), b.as_slice()].concat();
                truncated |= s.len() > MAX_LITERAL_LEN;
                s.truncate(MAX_LITERAL_LEN);
                out.push(s);
            }
        }
        out.sort();
        out.dedup();
        self.strings = out;
        self.exact = next.exact && !truncated;
        self.exact
    }
}

/// Prefix literals of every match and whether any must match ASCII-case-insensitively.
fn prefix_literals(ast: &Ast) -> Option<(Vec<Vec<u8>>, bool)> {
    let mut ci = false;
    let lits = lits(ast, &mut ci)?;
    let usable = !lits.strings.is_empty() && lits.strings.iter().all(|s| !s.is_empty());
    usable.then_some((lits.strings, ci))
}

fn lits(ast: &Ast, ci: &mut bool) -> Option<Lits> {
    match ast {
        Ast::Empty | Ast::Assert(_) => Some(Lits::empty(true)),
        Ast::Char { c, ci: true } if !c.is_ascii() => {
            let strings = case_variants(*c).into_iter().map(|v| v.to_string().into_bytes()).collect();
            Some(Lits { strings, exact: true })
        }
        Ast::Char { c, ci: char_ci } => {
            *ci |= *char_ci && c.is_ascii_alphabetic();
            Some(Lits { strings: vec![c.to_string().into_bytes()], exact: true })
        }
        Ast::Class { items, negated, ci: class_ci } => {
            let class = Class::new(items.clone(), *negated, *class_ci);
            if !class.is_ascii_only() {
                return None;
            }
            let strings: Vec<Vec<u8>> = (0..128u8).filter(|&b| class.matches_ascii(b)).map(|b| vec![b]).collect();
            (!strings.is_empty() && strings.len() <= MAX_LITERALS).then_some(Lits { strings, exact: true })
        }
        Ast::Any { .. } => None,
        Ast::Concat(nodes) => {
            let mut acc = Lits::empty(true);
            for node in nodes {
                let Some(next) = lits(node, ci) else {
                    acc.exact = false;
                    break;
                };
                if !acc.cross(next) {
                    break;
                }
            }
            Some(acc)
        }
        Ast::Alt(branches) => {
            let mut out = Lits { strings: Vec::new(), exact: true };
            for branch in branches {
                let lits = lits(branch, ci)?;
                out.exact &= lits.exact;
                out.strings.extend(lits.strings);
            }
            out.strings.sort();
            out.strings.dedup();
            (out.strings.len() <= MAX_LITERALS).then_some(out)
        }
        Ast::Rep { min: 0, .. } => Some(Lits::empty(false)),
        Ast::Rep { sub, min, max, .. } => {
            let one = lits(sub, ci)?;
            let mut acc = Lits::empty(true);
            for _ in 0..*min {
                if !acc.cross(one.clone()) {
                    break;
                }
            }
            acc.exact &= *max == Some(*min);
            Some(acc)
        }
    }
}

/// A literal every match contains, when the prefilter does not already look
/// for one: text without it holds no match (`[^z]*z`).
pub(super) fn required_literal(ast: &Ast, prefilter: &Prefilter) -> Option<LiteralFinder> {
    if matches!(prefilter, Prefilter::Literal { .. }) {
        return None;
    }
    let nodes = match ast {
        Ast::Concat(nodes) => nodes.as_slice(),
        other => std::slice::from_ref(other),
    };
    let (run, ci) = (0..nodes.len()).map(|i| literal_run(&nodes[i..])).max_by_key(|(run, _)| run.len())?;
    (!run.is_empty()).then(|| LiteralFinder::new(&run, ci))
}

/// The longest literal run of the top-level sequence that sits at a fixed
/// byte offset, with everything before it matching ASCII bytes only.
fn inner_literal(ast: &Ast) -> Option<(Vec<u8>, bool, usize)> {
    let nodes = match ast {
        Ast::Concat(nodes) => nodes.as_slice(),
        other => std::slice::from_ref(other),
    };
    let mut best: Option<(Vec<u8>, bool, usize)> = None;
    let mut offset = 0;
    for (i, node) in nodes.iter().enumerate() {
        let (run, ci) = literal_run(&nodes[i..]);
        if !run.is_empty() && best.as_ref().is_none_or(|(b, _, _)| run.len() > b.len()) {
            best = Some((run, ci, offset));
        }
        offset += ascii_width(node)?;
    }
    best
}

/// Bytes of the leading `Char` nodes and whether any folds ASCII case.
fn literal_run(nodes: &[Ast]) -> (Vec<u8>, bool) {
    let mut run = Vec::new();
    let mut ci = false;
    for node in nodes {
        match node {
            Ast::Char { c, ci: char_ci } if c.is_ascii() || !char_ci => {
                ci |= *char_ci && c.is_ascii_alphabetic();
                let mut buf = [0; 4];
                run.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
            _ => break,
        }
    }
    (run, ci)
}

/// Byte width of a node that always consumes the same number of ASCII bytes.
fn ascii_width(ast: &Ast) -> Option<usize> {
    match ast {
        Ast::Assert(_) | Ast::Empty => Some(0),
        Ast::Char { c, .. } if c.is_ascii() => Some(1),
        Ast::Class { items, negated, ci } => Class::new(items.clone(), *negated, *ci).is_ascii_only().then_some(1),
        Ast::Concat(nodes) => nodes.iter().map(ascii_width).sum(),
        Ast::Rep { sub, min, max: Some(max), .. } if min == max => Some(ascii_width(sub)? * min),
        _ => None,
    }
}

/// Bytes the first consumed char of any match can start with, or `None`
/// when the program can match the empty string.
fn add_non_ascii(bits: &mut [bool; 256]) {
    LEAD_BYTES.for_each(|b| bits[b as usize] = true);
}

fn first_bytes(insts: &[Inst], classes: &[Class], start: usize) -> Option<ByteSet> {
    let mut bits = [false; 256];
    let mut seen = vec![false; insts.len()];
    let mut stack = vec![start];
        while let Some(pc) = stack.pop() {
        if std::mem::replace(&mut seen[pc], true) {
            continue;
        }
        match &insts[pc] {
            Inst::Match => return None,
            Inst::Jump { target } => stack.push(*target),
            Inst::Split { first, second } => stack.extend([*first, *second]),
            Inst::Assert { next, .. } => stack.push(*next),
            Inst::Byte { b, ci, .. } => {
                bits[*b as usize] = true;
                if *ci {
                    bits[b.to_ascii_uppercase() as usize] = true;
                }
            }
            // Folded variants of a non-ASCII char can start with other lead bytes.
            Inst::Char { ci: true, .. } => add_non_ascii(&mut bits),
            Inst::Char { c, ci: false, .. } => {
                let mut buf = [0; 4];
                bits[c.encode_utf8(&mut buf).as_bytes()[0] as usize] = true;
            }
            Inst::Set { class, .. } => {
                let class = &classes[*class];
                (0..128u8).filter(|&b| class.matches_ascii(b)).for_each(|b| bits[b as usize] = true);
                if !class.is_ascii_only() {
                    add_non_ascii(&mut bits);
                }
            }
            Inst::Any { dot_all, .. } => {
                (0..128u8).filter(|&b| *dot_all || b != b'\n').for_each(|b| bits[b as usize] = true);
                add_non_ascii(&mut bits);
            }
        }
    }
    Some(ByteSet::new(bits))
}
