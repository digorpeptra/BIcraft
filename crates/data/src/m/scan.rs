//! Token-level scans of M text that need no evaluation: the members of a section document
//! (`section Section1; shared Query1 = …;`, the form of a Power Query template's
//! `MashupDocument.pq`, a Dataflow's `mashup.pq` and a CDM `model.json`'s `pbi:mashup`
//! document), and the text literals of an expression with the call or record field they sit in
//! (`file.redact`). Both follow the lexical grammar of the M specification (Microsoft Learn,
//! "Power Query M language specification", *Lexical structure* and *Sections*).

use super::lexer::{Tok, tokenize};
use crate::{Error, Limits};

/// A member of a section document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionMember {
    pub name: String,
    /// The member's expression, as written (between `=` and the closing `;`).
    pub expression: String,
    pub shared: bool,
}

/// A section document's name and members. Text that is not a section document (no leading
/// `section` keyword) is an error. Nesting (brackets, braces, parentheses) is capped by
/// `limits.max_depth`.
pub fn section_members(src: &str, limits: &Limits) -> Result<(String, Vec<SectionMember>), Error> {
    if src.len() > limits.max_bytes {
        return Err(Error::LimitExceeded(format!(
            "the M document is {} bytes (limit {})",
            src.len(),
            limits.max_bytes
        )));
    }
    let toks = tokenize(src, limits)?;
    let bad = |line: usize, column: usize, m: &str| Error::Syntax {
        format: "M",
        line,
        column,
        message: m.to_string(),
    };
    let mut i = 0usize;
    // optional section attributes `[ … ]` before `section` are skipped with the members'
    // bracket counting below
    let at = |i: usize| toks.get(i);
    let Some(first) = at(i) else {
        return Err(bad(1, 1, "empty document"));
    };
    if !matches!(&first.tok, Tok::Ident(s) if s == "section") {
        return Err(bad(first.line, first.column, "expected `section`"));
    }
    i += 1;
    let name = match at(i).map(|t| &t.tok) {
        Some(Tok::Ident(s) | Tok::Quoted(s)) => {
            i += 1;
            s.clone()
        }
        _ => String::new(),
    };
    match at(i) {
        Some(t) if t.tok == Tok::Punct(";") => i += 1,
        Some(t) => return Err(bad(t.line, t.column, "expected `;` after the section name")),
        None => return Err(bad(1, 1, "unexpected end")),
    }
    let mut members = Vec::new();
    while let Some(t) = at(i) {
        if t.tok == Tok::Eof {
            break;
        }
        // member attributes `[ … ]` are skipped
        if t.tok == Tok::Punct("[") {
            i = skip_group(&toks, i, limits)?;
            continue;
        }
        let shared = matches!(&t.tok, Tok::Ident(s) if s == "shared");
        if shared {
            i += 1;
        }
        let Some(nt) = at(i) else { break };
        let name = match &nt.tok {
            Tok::Ident(s) | Tok::Quoted(s) => s.clone(),
            _ => return Err(bad(nt.line, nt.column, "expected a member name")),
        };
        i += 1;
        match at(i) {
            Some(t) if t.tok == Tok::Punct("=") => i += 1,
            Some(t) => return Err(bad(t.line, t.column, "expected `=` after the member name")),
            None => return Err(bad(nt.line, nt.column, "unexpected end")),
        }
        let start = at(i).map_or(src.len(), |t| t.start);
        let mut depth = 0usize;
        let end = loop {
            let Some(t) = at(i) else {
                return Err(bad(nt.line, nt.column, "member without a closing `;`"));
            };
            match &t.tok {
                Tok::Eof => return Err(bad(t.line, t.column, "member without a closing `;`")),
                Tok::Punct("(" | "[" | "{") => {
                    depth += 1;
                    if depth > limits.max_depth {
                        return Err(Error::LimitExceeded(format!(
                            "M nesting deeper than {}",
                            limits.max_depth
                        )));
                    }
                }
                Tok::Punct(")" | "]" | "}") => depth = depth.saturating_sub(1),
                Tok::Punct(";") if depth == 0 => break t.start,
                _ => {}
            }
            i += 1;
        };
        i += 1;
        let expression = src.get(start..end).unwrap_or_default().trim().to_string();
        members.push(SectionMember {
            name,
            expression,
            shared,
        });
    }
    Ok((name, members))
}

/// The index after the group opened at `i` (`[`, `(` or `{`).
fn skip_group(toks: &[super::lexer::Token], mut i: usize, limits: &Limits) -> Result<usize, Error> {
    let mut depth = 0usize;
    while let Some(t) = toks.get(i) {
        match &t.tok {
            Tok::Punct("(" | "[" | "{") => {
                depth += 1;
                if depth > limits.max_depth {
                    return Err(Error::LimitExceeded(format!(
                        "M nesting deeper than {}",
                        limits.max_depth
                    )));
                }
            }
            Tok::Punct(")" | "]" | "}") => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Ok(i + 1);
                }
            }
            Tok::Eof => break,
            _ => {}
        }
        i += 1;
    }
    Ok(i)
}

/// A text literal of an M expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextLiteral {
    /// The literal's value (escapes resolved).
    pub value: String,
    /// When the literal is a whole argument of a call: the function's name and the argument's
    /// position (`Sql.Database("srv", "db")`: `("Sql.Database", 0)` and `(…, 1)`).
    pub call: Option<(String, usize)>,
    /// When the literal is the value of a record field (`[Password = "…"]`): the field name.
    pub field: Option<String>,
}

/// An identifier passed as a whole argument of a call (`Sql.Database(ServerName, …)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentArgument {
    pub name: String,
    pub call: String,
    pub position: usize,
}

/// The text literals of `src` and the identifiers passed as whole call arguments.
pub fn text_literals(src: &str, limits: &Limits) -> Result<(Vec<TextLiteral>, Vec<IdentArgument>), Error> {
    if src.len() > limits.max_bytes {
        return Err(Error::LimitExceeded(format!(
            "the M text is {} bytes (limit {})",
            src.len(),
            limits.max_bytes
        )));
    }
    let toks = tokenize(src, limits)?;
    // the open groups: the call's name for a call's parentheses, and its current argument
    struct Frame {
        call: Option<String>,
        arg: usize,
        record: bool,
    }
    let mut stack: Vec<Frame> = Vec::new();
    let mut lits = Vec::new();
    let mut idents = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        let prev = i.checked_sub(1).and_then(|j| toks.get(j)).map(|t| &t.tok);
        let next = toks.get(i + 1).map(|t| &t.tok);
        match &t.tok {
            Tok::Punct(p @ ("(" | "[" | "{")) => {
                if stack.len() >= limits.max_depth {
                    return Err(Error::LimitExceeded(format!(
                        "M nesting deeper than {}",
                        limits.max_depth
                    )));
                }
                let call = match (p, prev) {
                    (&"(", Some(Tok::Ident(name) | Tok::Quoted(name))) if !is_keyword(name) => {
                        Some(name.clone())
                    }
                    _ => None,
                };
                stack.push(Frame {
                    call,
                    arg: 0,
                    record: *p == "[",
                });
            }
            Tok::Punct(")" | "]" | "}") => {
                stack.pop();
            }
            Tok::Punct(",") => {
                if let Some(f) = stack.last_mut() {
                    f.arg = f.arg.saturating_add(1);
                }
            }
            Tok::Text(value) | Tok::Ident(value) | Tok::Quoted(value) => {
                let whole = matches!(prev, Some(Tok::Punct("(" | ",")))
                    && matches!(next, Some(Tok::Punct(")" | ",")));
                let in_call = stack
                    .last()
                    .and_then(|f| f.call.as_ref().map(|c| (c.clone(), f.arg)))
                    .filter(|_| whole);
                if let Tok::Text(_) = &t.tok {
                    let field = match (prev, i.checked_sub(2).and_then(|j| toks.get(j)).map(|t| &t.tok)) {
                        (Some(Tok::Punct("=")), Some(Tok::Ident(n) | Tok::Quoted(n)))
                            if stack.last().is_some_and(|f| f.record) =>
                        {
                            Some(n.clone())
                        }
                        _ => None,
                    };
                    lits.push(TextLiteral {
                        value: value.clone(),
                        call: in_call,
                        field,
                    });
                } else if let Some((call, position)) = in_call
                    && !is_keyword(value)
                {
                    idents.push(IdentArgument {
                        name: value.clone(),
                        call,
                        position,
                    });
                }
            }
            _ => {}
        }
    }
    Ok((lits, idents))
}

fn is_keyword(s: &str) -> bool {
    matches!(
        s,
        "and"
            | "as"
            | "each"
            | "else"
            | "error"
            | "false"
            | "if"
            | "in"
            | "is"
            | "let"
            | "meta"
            | "not"
            | "null"
            | "or"
            | "otherwise"
            | "section"
            | "shared"
            | "then"
            | "true"
            | "try"
            | "type"
    )
}
