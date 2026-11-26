//! A bounded JSON reader producing M values (`Json.Document`): objects become
//! records, arrays lists, numbers numbers.

use std::rc::Rc;

use super::value::{Record, Value};

pub(crate) struct JsonError(pub String);

struct P<'a> {
    s: &'a [u8],
    pos: usize,
    depth: usize,
    max_depth: usize,
    items: usize,
    max_items: usize,
}

/// Parses JSON text. `max_depth` bounds nesting and `max_items` the total number of values.
pub(crate) fn parse(text: &str, max_depth: usize, max_items: usize) -> Result<Value, JsonError> {
    let mut p = P {
        s: text.as_bytes(),
        pos: 0,
        depth: 0,
        max_depth,
        items: 0,
        max_items,
    };
    p.ws();
    let v = p.value()?;
    p.ws();
    if p.pos != p.s.len() {
        return Err(p.err("unexpected text after the JSON value"));
    }
    Ok(v)
}

impl P<'_> {
    fn err(&self, m: &str) -> JsonError {
        JsonError(format!("{m} (at byte {})", self.pos))
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }

    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.pos += 1;
        }
    }

    fn lit(&mut self, word: &str, v: Value) -> Result<Value, JsonError> {
        if self.s.get(self.pos..self.pos + word.len()) == Some(word.as_bytes()) {
            self.pos += word.len();
            Ok(v)
        } else {
            Err(self.err("invalid literal"))
        }
    }

    fn value(&mut self) -> Result<Value, JsonError> {
        self.items += 1;
        if self.items > self.max_items {
            return Err(JsonError(format!("more than {} JSON values", self.max_items)));
        }
        match self.peek() {
            Some(b'{') => self.nested(true),
            Some(b'[') => self.nested(false),
            Some(b'"') => Ok(Value::text(self.string()?)),
            Some(b't') => self.lit("true", Value::Logical(true)),
            Some(b'f') => self.lit("false", Value::Logical(false)),
            Some(b'n') => self.lit("null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(self.err("expected a JSON value")),
        }
    }

    fn nested(&mut self, object: bool) -> Result<Value, JsonError> {
        self.depth += 1;
        if self.depth > self.max_depth {
            return Err(JsonError(format!("JSON nested deeper than {}", self.max_depth)));
        }
        self.pos += 1;
        self.ws();
        let close = if object { b'}' } else { b']' };
        let mut fields: Vec<(Rc<str>, Value)> = Vec::new();
        let mut items: Vec<Value> = Vec::new();
        if self.peek() == Some(close) {
            self.pos += 1;
        } else {
            loop {
                self.ws();
                if object {
                    if self.peek() != Some(b'"') {
                        return Err(self.err("expected a field name"));
                    }
                    let k = self.string()?;
                    self.ws();
                    if self.peek() != Some(b':') {
                        return Err(self.err("expected `:`"));
                    }
                    self.pos += 1;
                    self.ws();
                    let v = self.value()?;
                    // A repeated key keeps the last value, in the first key's position.
                    match fields.iter_mut().find(|(n, _)| **n == *k) {
                        Some(slot) => slot.1 = v,
                        None => fields.push((k.into(), v)),
                    }
                } else {
                    items.push(self.value()?);
                }
                self.ws();
                match self.peek() {
                    Some(b',') => self.pos += 1,
                    Some(c) if c == close => {
                        self.pos += 1;
                        break;
                    }
                    _ => return Err(self.err("expected `,` or a closing bracket")),
                }
            }
        }
        self.depth -= 1;
        Ok(if object {
            Value::Record(Rc::new(Record::new(fields)))
        } else {
            Value::list(items)
        })
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        let h = self
            .s
            .get(self.pos..self.pos + 4)
            .ok_or_else(|| self.err("bad \\u escape"))?;
        let h = std::str::from_utf8(h).map_err(|_| self.err("bad \\u escape"))?;
        let n = u32::from_str_radix(h, 16).map_err(|_| self.err("bad \\u escape"))?;
        self.pos += 4;
        Ok(n)
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.pos += 1;
        let mut out = String::new();
        loop {
            let start = self.pos;
            while let Some(c) = self.peek() {
                if c == b'"' || c == b'\\' || c < 0x20 {
                    break;
                }
                self.pos += 1;
            }
            let chunk = self.s.get(start..self.pos).unwrap_or(&[]);
            out.push_str(std::str::from_utf8(chunk).map_err(|_| self.err("invalid UTF-8"))?);
            match self.peek() {
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let e = self.peek().ok_or_else(|| self.err("unterminated string"))?;
                    self.pos += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let mut c = self.hex4()?;
                            if (0xD800..0xDC00).contains(&c)
                                && self.s.get(self.pos..self.pos + 2) == Some(b"\\u")
                            {
                                self.pos += 2;
                                let lo = self.hex4()?;
                                if (0xDC00..0xE000).contains(&lo) {
                                    c = 0x10000 + ((c - 0xD800) << 10) + (lo - 0xDC00);
                                }
                            }
                            out.push(char::from_u32(c).unwrap_or('\u{fffd}'));
                        }
                        _ => return Err(self.err("bad escape")),
                    }
                }
                _ => return Err(self.err("unterminated string")),
            }
        }
    }

    fn number(&mut self) -> Result<Value, JsonError> {
        let start = self.pos;
        while matches!(self.peek(), Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')) {
            self.pos += 1;
        }
        let t = std::str::from_utf8(self.s.get(start..self.pos).unwrap_or(&[])).unwrap_or("");
        crate::m::value::Num::parse(t)
            .map(Value::Number)
            .ok_or_else(|| self.err("bad number"))
    }
}
