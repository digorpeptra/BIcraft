//! Lexer for the Power Query M language (lexical grammar of the M specification).

use super::value::Num;
use crate::{Error, Limits};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Tok {
    /// Identifier or keyword, dotted parts included (`Table.FromRows`, `Int64.Type`).
    Ident(String),
    /// `#table`, `#date`, `#infinity`, …
    Hash(String),
    /// `#"quoted identifier"`.
    Quoted(String),
    Text(String),
    Number(Num),
    /// Punctuators and operators: `( ) [ ] { } , ; = < > + - * / & @ ! ? <= >= <> => .. ... ??`.
    Punct(&'static str),
    Eof,
}

#[derive(Debug, Clone)]
pub(crate) struct Token {
    pub tok: Tok,
    pub line: usize,
    pub column: usize,
    /// Byte range in the source (used for generalized identifiers).
    pub start: usize,
    pub end: usize,
}

pub(crate) fn syntax(line: usize, column: usize, message: impl Into<String>) -> Error {
    Error::Syntax {
        format: "M",
        line,
        column,
        message: message.into(),
    }
}

struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    line: usize,
    column: usize,
    max_cell: usize,
}

const PUNCTS: [&str; 26] = [
    "...", "=>", "<=", ">=", "<>", "..", "??", "(", ")", "[", "]", "{", "}", ",", ";", "=", "<", ">", "+",
    "-", "*", "/", "&", "@", "!", "?",
];

impl Lexer<'_> {
    fn peek(&self) -> Option<char> {
        self.src.get(self.pos..).and_then(|s| s.chars().next())
    }

    fn peek2(&self) -> Option<char> {
        let mut it = self.src.get(self.pos..)?.chars();
        it.next();
        it.next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        if c == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(c)
    }

    fn string_body(&mut self, line: usize, column: usize) -> Result<String, Error> {
        let mut s = String::new();
        loop {
            let c = self
                .bump()
                .ok_or_else(|| syntax(line, column, "unterminated text literal"))?;
            match c {
                '"' if self.peek() == Some('"') => {
                    self.bump();
                    s.push('"');
                }
                '"' => break,
                '#' if self.peek() == Some('(') => {
                    self.bump();
                    let mut body = String::new();
                    loop {
                        match self.bump() {
                            Some(')') => break,
                            Some(ch) if body.len() < 64 => body.push(ch),
                            _ => return Err(syntax(self.line, self.column, "bad `#(...)` escape")),
                        }
                    }
                    for part in body.split(',') {
                        let part = part.trim();
                        match part {
                            "cr" => s.push('\r'),
                            "lf" => s.push('\n'),
                            "tab" => s.push('\t'),
                            "#" => s.push('#'),
                            hex if (hex.len() == 4 || hex.len() == 8)
                                && hex.bytes().all(|b| b.is_ascii_hexdigit()) =>
                            {
                                let ch = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32);
                                s.push(ch.ok_or_else(|| {
                                    syntax(self.line, self.column, format!("bad escape `{hex}`"))
                                })?);
                            }
                            other => {
                                return Err(syntax(
                                    self.line,
                                    self.column,
                                    format!("unknown escape `#({other})`"),
                                ));
                            }
                        }
                    }
                }
                c => s.push(c),
            }
            if s.len() > self.max_cell {
                return Err(Error::LimitExceeded(format!(
                    "a text literal is longer than {} bytes",
                    self.max_cell
                )));
            }
        }
        Ok(s)
    }

    fn ident_tail(&mut self, first: char) -> String {
        let mut s = String::from(first);
        loop {
            match self.peek() {
                Some(c) if c.is_alphanumeric() || c == '_' => {
                    s.push(c);
                    self.bump();
                }
                Some('.') if self.peek2().is_some_and(|c| c.is_alphabetic() || c == '_') => {
                    s.push('.');
                    self.bump();
                }
                _ => return s,
            }
        }
    }

    fn number(&mut self, first: char, line: usize, column: usize) -> Result<Tok, Error> {
        let mut s = String::from(first);
        if first == '0' && matches!(self.peek(), Some('x' | 'X')) {
            self.bump();
            let mut hex = String::new();
            while let Some(c) = self.peek().filter(char::is_ascii_hexdigit) {
                hex.push(c);
                self.bump();
            }
            return u64::from_str_radix(&hex, 16)
                .map(|n| Tok::Number(Num::int(i128::from(n))))
                .map_err(|_| syntax(line, column, "bad hex number"));
        }
        let mut real = first == '.';
        loop {
            match self.peek() {
                Some(c) if c.is_ascii_digit() => s.push(c),
                Some('.') if !real && self.peek2().is_some_and(|c| c.is_ascii_digit()) => {
                    real = true;
                    s.push('.');
                }
                Some('e' | 'E') => {
                    real = true;
                    s.push('e');
                    self.bump();
                    if let Some(sign @ ('+' | '-')) = self.peek() {
                        s.push(sign);
                        self.bump();
                    }
                    continue;
                }
                _ => break,
            }
            self.bump();
            if s.len() > 400 {
                return Err(syntax(line, column, "number literal too long"));
            }
        }
        Num::parse(&s)
            .map(Tok::Number)
            .ok_or_else(|| syntax(line, column, format!("bad number `{s}`")))
    }

    fn skip_trivia(&mut self) -> Result<(), Error> {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() || c == '\u{feff}' => {
                    self.bump();
                }
                Some('/') if self.peek2() == Some('/') => {
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.bump();
                    }
                }
                Some('/') if self.peek2() == Some('*') => {
                    let (line, column) = (self.line, self.column);
                    self.bump();
                    self.bump();
                    loop {
                        match self.bump() {
                            None => return Err(syntax(line, column, "unterminated comment")),
                            Some('*') if self.peek() == Some('/') => {
                                self.bump();
                                break;
                            }
                            _ => {}
                        }
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    fn next(&mut self) -> Result<Token, Error> {
        self.skip_trivia()?;
        let (line, column, start) = (self.line, self.column, self.pos);
        let Some(c) = self.peek() else {
            return Ok(Token {
                tok: Tok::Eof,
                line,
                column,
                start,
                end: start,
            });
        };
        let tok = if c == '"' {
            self.bump();
            Tok::Text(self.string_body(line, column)?)
        } else if c == '#' && self.peek2() == Some('"') {
            self.bump();
            self.bump();
            Tok::Quoted(self.string_body(line, column)?)
        } else if c == '#' && self.peek2().is_some_and(char::is_alphabetic) {
            self.bump();
            let first = self.bump().unwrap_or('#');
            Tok::Hash(format!("#{}", self.ident_tail(first)))
        } else if c.is_ascii_digit() {
            self.bump();
            self.number(c, line, column)?
        } else if c == '.' && self.peek2().is_some_and(|c| c.is_ascii_digit()) {
            self.bump();
            self.number('.', line, column)?
        } else if c.is_alphabetic() || c == '_' {
            self.bump();
            Tok::Ident(self.ident_tail(c))
        } else {
            let rest = self.src.get(self.pos..).unwrap_or("");
            let Some(p) = PUNCTS.iter().find(|p| rest.starts_with(**p)) else {
                return Err(syntax(line, column, format!("unexpected character `{c}`")));
            };
            for _ in 0..p.len() {
                self.bump();
            }
            Tok::Punct(p)
        };
        Ok(Token {
            tok,
            line,
            column,
            start,
            end: self.pos,
        })
    }
}

pub(crate) fn tokenize(src: &str, limits: &Limits) -> Result<Vec<Token>, Error> {
    let mut lx = Lexer {
        src,
        pos: 0,
        line: 1,
        column: 1,
        max_cell: limits.max_cell_bytes,
    };
    let mut out = Vec::new();
    loop {
        let t = lx.next()?;
        let end = t.tok == Tok::Eof;
        out.push(t);
        if end {
            return Ok(out);
        }
    }
}

pub(crate) fn describe(t: &Tok) -> String {
    match t {
        Tok::Ident(s) | Tok::Hash(s) => format!("`{s}`"),
        Tok::Quoted(s) => format!("`#\"{s}\"`"),
        Tok::Text(s) => format!("text \"{}\"", crate::value::truncate(s)),
        Tok::Number(r) => format!("`{}`", r.f()),
        Tok::Punct(c) => format!("`{c}`"),
        Tok::Eof => "end of input".into(),
    }
}
