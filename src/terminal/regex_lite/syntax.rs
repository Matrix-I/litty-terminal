//! Pattern syntax: the parser producing an [`Ast`], and the Unicode character
//! predicates shared by the compiler and the matcher.
//!
//! Unicode comes from `std` tables only: `\w` is `char::is_alphanumeric`, `_`,
//! combining marks and joiners; `\s` is `char::is_whitespace`; `\d` is ASCII.
//! Case folding maps through single-char upper/lowercase and never folds a
//! non-ASCII char onto ASCII (so `K` KELVIN SIGN does not match `k`).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AssertKind {
    /// `^`: start of text or just after `\n`.
    StartLine,
    /// `$`: end of text or just before `\n`.
    EndLine,
    /// `\z`
    EndText,
    /// `\b`
    WordBound,
    /// `\B`
    NotWordBound,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ClassItem {
    Range(char, char),
    Digit,
    NotDigit,
    Word,
    NotWord,
    Space,
    NotSpace,
}

impl ClassItem {
    fn matches(self, c: char) -> bool {
        match self {
            ClassItem::Range(lo, hi) => (lo..=hi).contains(&c),
            ClassItem::Digit => c.is_ascii_digit(),
            ClassItem::NotDigit => !c.is_ascii_digit(),
            ClassItem::Word => is_word_char(c),
            ClassItem::NotWord => !is_word_char(c),
            ClassItem::Space => c.is_whitespace(),
            ClassItem::NotSpace => !c.is_whitespace(),
        }
    }

    /// `true` when the item can only match ASCII chars.
    pub(super) fn is_ascii_only(self) -> bool {
        match self {
            ClassItem::Range(_, hi) => hi.is_ascii(),
            ClassItem::Digit => true,
            _ => false,
        }
    }
}

/// Whether a bracket or escape class matches `c`.
pub(super) fn class_matches(items: &[ClassItem], negated: bool, ci: bool, c: char) -> bool {
    let hit = |x: char| items.iter().any(|item| item.matches(x));
    let found = hit(c) || (ci && [lower(c), upper(c)].into_iter().any(|v| v != c && hit(v)));
    found != negated
}

/// `\w`: alphanumerics, `_`, combining marks (decomposed accents such as
/// Vietnamese `e\u{302}\u{323}`) and the zero-width joiners.
pub(super) fn is_word_char(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_alphanumeric() || c == '_';
    }
    c.is_alphanumeric()
        || matches!(c as u32, 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F)
        || matches!(c, '\u{200C}' | '\u{200D}')
}

/// Case-fold key: two chars match case-insensitively when their keys are equal.
/// Going through uppercase first merges `ς`/`σ`/`Σ` and `ǅ`/`ǆ`/`Ǆ`.
#[inline]
pub(super) fn fold(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_lowercase();
    }
    lower(upper(c))
}

fn lower(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_lowercase();
    }
    single(c.to_lowercase()).filter(|l| !l.is_ascii()).unwrap_or(c)
}

fn upper(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_uppercase();
    }
    single(c.to_uppercase()).filter(|u| !u.is_ascii()).unwrap_or(c)
}

/// Non-ASCII chars whose [`fold`] key is neither their simple lowercase nor
/// uppercase partner's; generated from `std` and checked exhaustively by
/// `case_variants_cover_every_char_with_the_same_fold`.
const EXTRA_FOLD_MEMBERS: [char; 56] = [
    '\u{1E9E}', '\u{212B}', '\u{01C5}', '\u{01C8}', '\u{01CB}', '\u{01F2}', '\u{03D0}', '\u{03F5}', '\u{03D1}', '\u{03F4}',
    '\u{0345}', '\u{1FBE}', '\u{03F0}', '\u{00B5}', '\u{03D6}', '\u{03F1}', '\u{03C2}', '\u{03D5}', '\u{2126}', '\u{1C80}',
    '\u{1C81}', '\u{1C82}', '\u{1C83}', '\u{1C84}', '\u{1C85}', '\u{1C86}', '\u{1C87}', '\u{1E9B}', '\u{1F88}', '\u{1F89}',
    '\u{1F8A}', '\u{1F8B}', '\u{1F8C}', '\u{1F8D}', '\u{1F8E}', '\u{1F8F}', '\u{1F98}', '\u{1F99}', '\u{1F9A}', '\u{1F9B}',
    '\u{1F9C}', '\u{1F9D}', '\u{1F9E}', '\u{1F9F}', '\u{1FA8}', '\u{1FA9}', '\u{1FAA}', '\u{1FAB}', '\u{1FAC}', '\u{1FAD}',
    '\u{1FAE}', '\u{1FAF}', '\u{1FBC}', '\u{1FCC}', '\u{1FFC}', '\u{1C88}',
];

/// Every char that matches `c` case-insensitively, `c` included.
pub(super) fn case_variants(c: char) -> Vec<char> {
    let key = fold(c);
    let mut out: Vec<char> = [key, lower(key), upper(key), c].into_iter().chain(EXTRA_FOLD_MEMBERS).filter(|&v| fold(v) == key).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The only item of `it`, if it has exactly one (`ß` uppercases to `SS`).
fn single(mut it: impl Iterator<Item = char>) -> Option<char> {
    let first = it.next()?;
    it.next().is_none().then_some(first)
}

#[derive(Clone, Debug)]
pub(super) enum Ast {
    Empty,
    Char { c: char, ci: bool },
    Any { dot_all: bool },
    Class { items: Vec<ClassItem>, negated: bool, ci: bool },
    Assert(AssertKind),
    Concat(Vec<Ast>),
    /// Branches in priority order.
    Alt(Vec<Ast>),
    Rep { sub: Box<Ast>, min: usize, max: Option<usize>, greedy: bool },
}

/// Largest `{m,n}` bound, keeping compiled programs small.
const MAX_REPEAT: usize = 256;

pub(super) fn parse(pattern: &str, ci: bool) -> Result<Ast, &'static str> {
    let mut parser = Parser { chars: pattern.chars().peekable(), ci, dot_all: false };
    let ast = parser.parse_expr()?;
    match parser.peek() {
        None => Ok(ast),
        Some(')') => Err("unmatched closing paren"),
        Some(_) => Err("unexpected token in regex"),
    }
}

/// The AST of `text` matched literally.
pub(super) fn literal(text: &str, ci: bool) -> Ast {
    Ast::Concat(text.chars().map(|c| Ast::Char { c, ci }).collect())
}

struct Parser<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    ci: bool,
    dot_all: bool,
}

enum ClassEscape {
    Item(ClassItem),
    Char(char),
}

impl Parser<'_> {
    fn peek(&mut self) -> Option<char> {
        self.chars.peek().copied()
    }

    fn bump(&mut self) -> Option<char> {
        self.chars.next()
    }

    fn parse_expr(&mut self) -> Result<Ast, &'static str> {
        let mut branches = vec![self.parse_concat()?];
        while self.peek() == Some('|') {
            self.bump();
            branches.push(self.parse_concat()?);
        }
        Ok(if branches.len() == 1 { branches.pop().unwrap_or(Ast::Empty) } else { Ast::Alt(branches) })
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
            1 => seq.pop().unwrap_or(Ast::Empty),
            _ => Ast::Concat(seq),
        })
    }

    fn parse_rep(&mut self) -> Result<Option<Ast>, &'static str> {
        let Some(atom) = self.parse_atom()? else {
            return Ok(None);
        };
        let (min, max) = match self.peek() {
            Some('*') => (0, None),
            Some('+') => (1, None),
            Some('?') => (0, Some(1)),
            Some('{') => {
                self.bump();
                let bounds = self.parse_counted_rep()?;
                return Ok(Some(self.finish_rep(atom, bounds)));
            }
            _ => return Ok(Some(atom)),
        };
        self.bump();
        Ok(Some(self.finish_rep(atom, (min, max))))
    }

    fn finish_rep(&mut self, atom: Ast, (min, max): (usize, Option<usize>)) -> Ast {
        let greedy = if self.peek() == Some('?') {
            self.bump();
            false
        } else {
            true
        };
        Ast::Rep { sub: Box::new(atom), min, max, greedy }
    }

    fn parse_number(&mut self) -> Option<usize> {
        let mut value: Option<usize> = None;
        while let Some(d) = self.peek().and_then(|c| c.to_digit(10)) {
            self.bump();
            value = Some(value.unwrap_or(0).saturating_mul(10).saturating_add(d as usize));
        }
        value
    }

    fn parse_counted_rep(&mut self) -> Result<(usize, Option<usize>), &'static str> {
        let min = self.parse_number().ok_or("invalid repetition count")?;
        let max = if self.peek() == Some(',') {
            self.bump();
            if self.peek() == Some('}') {
                None
            } else {
                let max = self.parse_number().ok_or("invalid repetition range")?;
                if max < min {
                    return Err("invalid repetition range");
                }
                Some(max)
            }
        } else {
            Some(min)
        };
        if self.bump() != Some('}') {
            return Err("unclosed repetition brace");
        }
        if max.unwrap_or(min) > MAX_REPEAT {
            return Err("repetition bound too large");
        }
        Ok((min, max))
    }

    fn parse_atom(&mut self) -> Result<Option<Ast>, &'static str> {
        let Some(c) = self.bump() else {
            return Ok(None);
        };
        match c {
            '.' => Ok(Some(Ast::Any { dot_all: self.dot_all })),
            '^' => Ok(Some(Ast::Assert(AssertKind::StartLine))),
            '$' => Ok(Some(Ast::Assert(AssertKind::EndLine))),
            '*' | '+' | '?' => Err("dangling quantifier"),
            '(' => self.parse_group(),
            ')' => Err("unmatched closing paren"),
            '[' => self.parse_class().map(Some),
            '\\' => self.parse_escape().map(Some),
            ch => Ok(Some(Ast::Char { c: ch, ci: self.ci })),
        }
    }

    /// `(...)`, `(?:...)`, `(?flags:...)` or a bare `(?flags)`, whose flags
    /// last until the enclosing group closes.
    fn parse_group(&mut self) -> Result<Option<Ast>, &'static str> {
        let saved = (self.ci, self.dot_all);
        if self.peek() == Some('?') {
            self.bump();
            loop {
                match self.bump() {
                    Some('i') => self.ci = true,
                    Some('s') => self.dot_all = true,
                    Some(':') => break,
                    Some(')') => return Ok(None),
                    _ => return Err("unsupported group modifier"),
                }
            }
        }
        let inner = self.parse_expr()?;
        if self.bump() != Some(')') {
            return Err("unclosed group");
        }
        (self.ci, self.dot_all) = saved;
        Ok(Some(inner))
    }

    fn parse_escape(&mut self) -> Result<Ast, &'static str> {
        let c = self.bump().ok_or("trailing backslash")?;
        let assert = match c {
            'b' => AssertKind::WordBound,
            'B' => AssertKind::NotWordBound,
            'z' | 'Z' => AssertKind::EndText,
            _ => {
                return Ok(match escape_class(c) {
                    ClassEscape::Item(item) => Ast::Class { items: vec![item], negated: false, ci: false },
                    ClassEscape::Char(ch) => Ast::Char { c: ch, ci: self.ci },
                });
            }
        };
        Ok(Ast::Assert(assert))
    }

    fn parse_class(&mut self) -> Result<Ast, &'static str> {
        let negated = self.peek() == Some('^');
        if negated {
            self.bump();
        }
        let mut items = Vec::new();
        let mut first = true;
        loop {
            let c = self.bump().ok_or("unclosed character class")?;
            if c == ']' && !first {
                return Ok(Ast::Class { items, negated, ci: self.ci });
            }
            first = false;
            let lo = match self.class_char(c)? {
                ClassEscape::Item(item) => {
                    items.push(item);
                    continue;
                }
                ClassEscape::Char(ch) => ch,
            };
            let mut ahead = self.chars.clone();
            if ahead.next() == Some('-') && ahead.peek().is_some_and(|&n| n != ']') {
                self.bump();
                let end = self.bump().ok_or("unclosed character class")?;
                let ClassEscape::Char(hi) = self.class_char(end)? else {
                    return Err("invalid character class range");
                };
                if lo > hi {
                    return Err("invalid character class range");
                }
                items.push(ClassItem::Range(lo, hi));
            } else {
                items.push(ClassItem::Range(lo, lo));
            }
        }
    }

    /// A class member starting with `c`, reading its escape if `c` is `\`.
    fn class_char(&mut self, c: char) -> Result<ClassEscape, &'static str> {
        if c != '\\' {
            return Ok(ClassEscape::Char(c));
        }
        let esc = self.bump().ok_or("unclosed character class escape")?;
        Ok(escape_class(esc))
    }
}

fn escape_class(c: char) -> ClassEscape {
    match c {
        'd' => ClassEscape::Item(ClassItem::Digit),
        'D' => ClassEscape::Item(ClassItem::NotDigit),
        'w' => ClassEscape::Item(ClassItem::Word),
        'W' => ClassEscape::Item(ClassItem::NotWord),
        's' => ClassEscape::Item(ClassItem::Space),
        'S' => ClassEscape::Item(ClassItem::NotSpace),
        'n' => ClassEscape::Char('\n'),
        'r' => ClassEscape::Char('\r'),
        't' => ClassEscape::Char('\t'),
        other => ClassEscape::Char(other),
    }
}
