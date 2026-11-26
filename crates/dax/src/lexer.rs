//! DAX tokens: identifiers, `'quoted table'` names, `[bracketed]` column/measure names, string and
//! number literals, operators, and the three comment forms (`//`, `--`, `/* */`).
//!
//! DAX queries ([`lex_query`]) add three tokens that expressions never contain: `@name` XMLA
//! parameters, `:` (type hints of user-defined function parameters) and `=>` (function bodies).

use crate::{Raw, Span};

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    /// Bare name: function, table, variable or keyword. May contain `.` (`PERCENTILE.INC`).
    Ident(String),
    /// `'Sales Table'` with `''` unescaped.
    QuotedTable(String),
    /// `[Amount]` with `]]` unescaped.
    Bracket(String),
    Str(String),
    Number(String),
    /// `@name`: an XMLA query parameter (DAX queries only); holds the name without `@`.
    Param(String),
    /// An operator. DAX queries also produce `:` and `=>`; `.` only appears in column variations
    /// (`T[Date].[Year]`).
    Op(&'static str),
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

/// Longest operators first so `<=` wins over `<`.
const OPS: [&str; 15] = [
    "&&", "||", "<=", ">=", "<>", "==", "+", "-", "*", "/", "^", "&", "=", "<", ">",
];

pub(crate) fn lex(text: &str, diags: &mut Vec<Raw>) -> Vec<Token> {
    lex_mode(text, diags, false)
}

/// Lexes a DAX query: like [`lex`], plus `@param`, `:` and `=>`.
pub(crate) fn lex_query(text: &str, diags: &mut Vec<Raw>) -> Vec<Token> {
    lex_mode(text, diags, true)
}

fn lex_mode(text: &str, diags: &mut Vec<Raw>, query: bool) -> Vec<Token> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        let start = i;
        let rest = text.get(i..).unwrap_or_default();
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if rest.starts_with("//") || rest.starts_with("--") {
            i += rest.find('\n').unwrap_or(rest.len());
            continue;
        }
        if rest.starts_with("/*") {
            match rest.get(2..).and_then(|r| r.find("*/")) {
                Some(end) => i += end + 4,
                None => {
                    diags.push(Raw::error(
                        "unterminated-comment",
                        Span {
                            start,
                            end: text.len(),
                        },
                        "unterminated /* comment",
                    ));
                    i = text.len();
                }
            }
            continue;
        }
        let single = |kind| Token {
            kind,
            span: Span {
                start,
                end: start + 1,
            },
        };
        match b {
            b'(' => out.push(single(TokenKind::LParen)),
            b')' => out.push(single(TokenKind::RParen)),
            b'{' => out.push(single(TokenKind::LBrace)),
            b'}' => out.push(single(TokenKind::RBrace)),
            b',' => out.push(single(TokenKind::Comma)),
            b'"' => {
                let (s, end) = delimited(text, i, '"');
                if end.is_none() {
                    diags.push(Raw::error(
                        "unterminated-string",
                        Span {
                            start,
                            end: text.len(),
                        },
                        "unterminated string literal",
                    ));
                }
                i = end.unwrap_or(text.len());
                out.push(Token {
                    kind: TokenKind::Str(s),
                    span: Span { start, end: i },
                });
                continue;
            }
            b'\'' => {
                let (s, end) = delimited(text, i, '\'');
                if end.is_none() {
                    diags.push(Raw::error(
                        "unterminated-table-name",
                        Span {
                            start,
                            end: text.len(),
                        },
                        "unterminated 'table name'",
                    ));
                }
                i = end.unwrap_or(text.len());
                out.push(Token {
                    kind: TokenKind::QuotedTable(s),
                    span: Span { start, end: i },
                });
                continue;
            }
            b'[' => {
                let (s, end) = bracketed(text, i);
                if end.is_none() {
                    diags.push(Raw::error(
                        "unterminated-bracket",
                        Span {
                            start,
                            end: text.len(),
                        },
                        "unterminated [name]",
                    ));
                }
                i = end.unwrap_or(text.len());
                out.push(Token {
                    kind: TokenKind::Bracket(s),
                    span: Span { start, end: i },
                });
                continue;
            }
            _ if b.is_ascii_digit() || (b == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)) => {
                i = number_end(bytes, i);
                let s = text.get(start..i).unwrap_or_default().to_string();
                out.push(Token {
                    kind: TokenKind::Number(s),
                    span: Span { start, end: i },
                });
                continue;
            }
            b'@' if query && rest.get(1..).is_some_and(|r| r.starts_with(is_param_char)) => {
                let after = rest.get(1..).unwrap_or_default();
                let len = after
                    .char_indices()
                    .find(|&(_, c)| !is_param_char(c))
                    .map_or(after.len(), |(n, _)| n);
                i += 1 + len;
                let name = after.get(..len).unwrap_or_default().to_string();
                out.push(Token {
                    kind: TokenKind::Param(name),
                    span: Span { start, end: i },
                });
                continue;
            }
            b'=' if query && rest.starts_with("=>") => {
                i += 2;
                out.push(Token {
                    kind: TokenKind::Op("=>"),
                    span: Span { start, end: i },
                });
                continue;
            }
            b':' if query => out.push(single(TokenKind::Op(":"))),
            // A lone `.` (not a number's) separates a column from its variation: `T[Date].[Year]`.
            b'.' => out.push(single(TokenKind::Op("."))),
            _ if is_ident_start(rest) => {
                let len = rest
                    .char_indices()
                    .find(|&(_, c)| !(c.is_alphanumeric() || c == '_' || c == '.'))
                    .map_or(rest.len(), |(n, _)| n);
                i += len;
                let s = text.get(start..i).unwrap_or_default().to_string();
                out.push(Token {
                    kind: TokenKind::Ident(s),
                    span: Span { start, end: i },
                });
                continue;
            }
            _ => match OPS.iter().find(|op| rest.starts_with(**op)) {
                Some(op) => {
                    i += op.len();
                    out.push(Token {
                        kind: TokenKind::Op(op),
                        span: Span { start, end: i },
                    });
                    continue;
                }
                None => {
                    let ch = rest.chars().next().unwrap_or('\u{FFFD}');
                    let end = start + ch.len_utf8();
                    diags.push(Raw::error(
                        "unexpected-character",
                        Span { start, end },
                        format!("unexpected character `{ch}`"),
                    ));
                    i = end;
                    continue;
                }
            },
        }
        i += 1;
    }
    out.push(Token {
        kind: TokenKind::Eof,
        span: Span {
            start: text.len(),
            end: text.len(),
        },
    });
    out
}

fn is_param_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn is_ident_start(rest: &str) -> bool {
    rest.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_')
}

/// Reads a `q`-delimited literal where a doubled `q` is an escaped `q`. Returns the content and
/// the byte index just past the closing delimiter (`None` if unterminated).
fn delimited(text: &str, open: usize, q: char) -> (String, Option<usize>) {
    let mut s = String::new();
    let mut chars = text.get(open + 1..).unwrap_or_default().char_indices().peekable();
    while let Some((n, c)) = chars.next() {
        if c == q {
            if chars.peek().is_some_and(|&(_, d)| d == q) {
                chars.next();
                s.push(q);
            } else {
                return (s, Some(open + 1 + n + c.len_utf8()));
            }
        } else {
            s.push(c);
        }
    }
    (s, None)
}

/// `[name]`, with `]]` as an escaped `]`.
fn bracketed(text: &str, open: usize) -> (String, Option<usize>) {
    let mut s = String::new();
    let mut chars = text.get(open + 1..).unwrap_or_default().char_indices().peekable();
    while let Some((n, c)) = chars.next() {
        if c == ']' {
            if chars.peek().is_some_and(|&(_, d)| d == ']') {
                chars.next();
                s.push(']');
            } else {
                return (s, Some(open + 1 + n + 1));
            }
        } else {
            s.push(c);
        }
    }
    (s, None)
}

fn number_end(bytes: &[u8], mut i: usize) -> usize {
    while bytes.get(i).is_some_and(|b| b.is_ascii_digit() || *b == b'.') {
        i += 1;
    }
    if bytes.get(i).is_some_and(|b| *b == b'e' || *b == b'E') {
        let mut j = i + 1;
        if bytes.get(j).is_some_and(|b| *b == b'+' || *b == b'-') {
            j += 1;
        }
        if bytes.get(j).is_some_and(u8::is_ascii_digit) {
            i = j;
            while bytes.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
        }
    }
    i
}
