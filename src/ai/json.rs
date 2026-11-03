//! Bounded JSON parser and serializer; no additional dependencies.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

/// Nesting limit so a hostile response cannot overflow the stack.
pub(crate) const MAX_JSON_DEPTH: usize = 64;

impl Json {
    pub fn parse(s: &str) -> Result<Json, String> {
        let mut p = JsonParser {
            b: s.as_bytes(),
            i: 0,
        };
        let value = p.value(0)?;
        p.ws();
        if p.i != p.b.len() {
            return Err(p.err("trailing data"));
        }
        Ok(value)
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(members) => members.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
}

struct JsonParser<'a> {
    b: &'a [u8],
    i: usize,
}

impl JsonParser<'_> {
    fn err(&self, what: &str) -> String {
        format!("invalid JSON at byte {}: {what}", self.i)
    }

    fn ws(&mut self) {
        while matches!(self.b.get(self.i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        self.ws();
        if self.b.get(self.i) == Some(&c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn literal(&mut self, word: &str, value: Json) -> Result<Json, String> {
        if self.b[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(value)
        } else {
            Err(self.err("unexpected token"))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, String> {
        if depth > MAX_JSON_DEPTH {
            return Err(self.err("nesting too deep"));
        }
        self.ws();
        match self.b.get(self.i) {
            Some(b'{') => {
                self.i += 1;
                let mut members = Vec::new();
                if self.eat(b'}') {
                    return Ok(Json::Obj(members));
                }
                loop {
                    self.ws();
                    let key = self.string()?;
                    if !self.eat(b':') {
                        return Err(self.err("expected ':'"));
                    }
                    members.push((key, self.value(depth + 1)?));
                    if self.eat(b',') {
                        continue;
                    }
                    if self.eat(b'}') {
                        return Ok(Json::Obj(members));
                    }
                    return Err(self.err("expected ',' or '}'"));
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                if self.eat(b']') {
                    return Ok(Json::Arr(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    if self.eat(b',') {
                        continue;
                    }
                    if self.eat(b']') {
                        return Ok(Json::Arr(items));
                    }
                    return Err(self.err("expected ',' or ']'"));
                }
            }
            Some(b'"') => self.string().map(Json::Str),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'n') => self.literal("null", Json::Null),
            Some(b'-' | b'0'..=b'9') => {
                let start = self.i;
                self.eat_number()?;
                Ok(Json::Num(
                    std::str::from_utf8(&self.b[start..self.i]).unwrap().into(),
                ))
            }
            _ => Err(self.err("unexpected token")),
        }
    }

    fn eat_number(&mut self) -> Result<(), String> {
        if self.b.get(self.i) == Some(&b'-') {
            self.i += 1;
        }
        match self.b.get(self.i) {
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => {
                self.i += 1;
                while self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
                    self.i += 1;
                }
            }
            _ => return Err(self.err("invalid number")),
        }
        for marker in *b".e" {
            let hit = self
                .b
                .get(self.i)
                .is_some_and(|&b| b == marker || (marker == b'e' && b == b'E'));
            if hit {
                self.i += 1;
                if marker == b'e' && matches!(self.b.get(self.i), Some(b'+' | b'-')) {
                    self.i += 1;
                }
                let start = self.i;
                while self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
                    self.i += 1;
                }
                if self.i == start {
                    return Err(self.err("invalid number"));
                }
            }
        }
        Ok(())
    }

    fn string(&mut self) -> Result<String, String> {
        if self.b.get(self.i) != Some(&b'"') {
            return Err(self.err("expected string"));
        }
        self.i += 1;
        let mut out = Vec::new();
        loop {
            let c = *self
                .b
                .get(self.i)
                .ok_or_else(|| self.err("unterminated string"))?;
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(out).map_err(|_| self.err("invalid UTF-8")),
                b'\\' => {
                    let e = *self
                        .b
                        .get(self.i)
                        .ok_or_else(|| self.err("unterminated escape"))?;
                    self.i += 1;
                    let ch = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => self.unicode_escape()?,
                        _ => return Err(self.err("invalid escape")),
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                c if c < 0x20 => return Err(self.err("control character in string")),
                c => out.push(c),
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self
            .b
            .get(self.i..self.i + 4)
            .filter(|h| h.iter().all(u8::is_ascii_hexdigit))
            .ok_or_else(|| self.err("invalid \\u escape"))?;
        let value = digits.iter().fold(0u32, |acc, &d| {
            acc * 16 + (d as char).to_digit(16).unwrap_or(0)
        });
        self.i += 4;
        Ok(value)
    }

    fn unicode_escape(&mut self) -> Result<char, String> {
        let hi = self.hex4()?;
        let code = if (0xD800..0xDC00).contains(&hi) {
            if self.b.get(self.i..self.i + 2) != Some(b"\\u") {
                return Err(self.err("unpaired surrogate"));
            }
            self.i += 2;
            let lo = self.hex4()?;
            if !(0xDC00..0xE000).contains(&lo) {
                return Err(self.err("unpaired surrogate"));
            }
            0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
        } else {
            hi
        };
        char::from_u32(code).ok_or_else(|| self.err("invalid code point"))
    }
}

impl From<&str> for Json {
    fn from(s: &str) -> Self {
        Self::Str(s.into())
    }
}
impl From<String> for Json {
    fn from(s: String) -> Self {
        Self::Str(s)
    }
}
impl From<bool> for Json {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}
impl From<usize> for Json {
    fn from(v: usize) -> Self {
        Self::Num(v.to_string())
    }
}
pub fn obj<const N: usize>(pairs: [(&str, Json); N]) -> Json {
    Json::Obj(pairs.into_iter().map(|(k, v)| (k.into(), v)).collect())
}
impl Json {
    pub fn array(&self) -> &[Json] {
        if let Self::Arr(a) = self { a } else { &[] }
    }
    pub fn boolean(&self) -> Option<bool> {
        if let Self::Bool(b) = self {
            Some(*b)
        } else {
            None
        }
    }
    pub fn usize(&self) -> Option<usize> {
        if let Self::Num(n) = self {
            n.parse().ok()
        } else {
            None
        }
    }
    pub fn text(&self, key: &str) -> &str {
        self.get(key).and_then(Self::as_str).unwrap_or("")
    }
    pub fn set(&mut self, key: &str, value: Json) {
        if let Self::Obj(pairs) = self {
            if let Some((_, v)) = pairs.iter_mut().find(|(k, _)| k == key) {
                *v = value;
            } else {
                pairs.push((key.into(), value));
            }
        }
    }
    pub(super) fn encoded_len(&self) -> usize {
        fn string_len(s: &str) -> usize {
            2 + s.bytes().map(|b| match b { 0..=31 => 6, b'"' | b'\\' => 2, _ => 1 }).sum::<usize>()
        }
        match self {
            Self::Null => 4,
            Self::Bool(true) => 4,
            Self::Bool(false) => 5,
            Self::Num(n) => n.len(),
            Self::Str(s) => string_len(s),
            Self::Arr(items) => 2 + items.len().saturating_sub(1) + items.iter().map(Self::encoded_len).sum::<usize>(),
            Self::Obj(items) => 2 + items.len().saturating_sub(1) + items.iter().map(|(k,v)| string_len(k) + 1 + v.encoded_len()).sum::<usize>(),
        }
    }

    pub fn encode(&self) -> String {
        match self {
            Self::Null => "null".into(),
            Self::Bool(b) => b.to_string(),
            Self::Num(n) => n.clone(),
            Self::Str(s) => {
                let mut out = String::from("\"");
                for c in s.chars() {
                    match c {
                        '"' => out.push_str("\\\""),
                        '\\' => out.push_str("\\\\"),
                        c if c < ' ' => {
                            use std::fmt::Write;
                            let _ = write!(out, "\\u{:04x}", c as u32);
                        }
                        c => out.push(c),
                    }
                }
                out.push('"');
                out
            }
            Self::Arr(a) => format!(
                "[{}]",
                a.iter().map(Self::encode).collect::<Vec<_>>().join(",")
            ),
            Self::Obj(o) => format!(
                "{{{}}}",
                o.iter()
                    .map(|(k, v)| format!("{}:{}", Self::from(k.clone()).encode(), v.encode()))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip() {
        let s = r#"{"s":"é😀\\\"\n\u0001","n":-2.5e+3,"b":true,"a":[null,2]}"#;
        let v = Json::parse(s).unwrap();
        assert_eq!(Json::parse(&v.encode()).unwrap(), v);
    }
    #[test]
    fn rejects_invalid() {
        for s in ["-", "01", "1.", "1e", "--1", "[1,]", r#""\uD800""#] {
            assert!(Json::parse(s).is_err(), "{s}");
        }
    }
}

#[cfg(test)]
#[test]
fn budget_length_is_exact_without_allocations() {
    let controls: String = (0..32).map(char::from).collect();
    let values = [Json::Null, Json::Bool(true), Json::Bool(false), Json::Num("-2.5e+30".into()),
        Json::Str(format!("{controls}é😀\\\"")), Json::Arr(vec![]), Json::Obj(vec![]),
        obj([("\\\"\n鍵", Json::Arr(vec![Json::Null, "❤️".into(), false.into()]))])];
    for value in values {
        let expected = value.encode().len();
        assert_eq!(crate::test_alloc::allocations(|| assert_eq!(value.encoded_len(), expected)), 0);
    }
}
