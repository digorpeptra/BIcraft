// Ported from DashCraft (MIT OR Apache-2.0, used here under MIT),
// crates/pbi-m/src/service/format.rs@0958fd82b04656ee1797101a2b4df8d114dfebd1
// Copyright (c) 2026 Jack Sweeney and the DashCraft contributors. Licensed under the MIT License.
// See crates/data/LICENSE-MIT-dashcraft and THIRD_PARTY_NOTICES.md at the repository root.
// Adapted: a syntax tree of token ranges over bicraft's own lexer and grammar (`lexer.rs`,
// `parser.rs`), comments recovered from the gaps between tokens, no section documents.

//! M formatter (`m.format`): lays out an M expression deterministically and idempotently.
//!
//! The syntax tree decides where lines break, but the output is the source's own tokens and
//! comments: only the white space between them changes. Comments stay where they were: after a
//! token on its line, or on their own lines before the next token. A blank line between `let`
//! steps or before a comment is kept.
//!
//! `let` puts each step on its own line and the expression after `in` on the next, indented, as
//! Power BI Desktop writes queries. A group (the arguments of a call, a record, a list,
//! parentheses, a chain of operators of one precedence, an `if`, a `try`, the value of a step or
//! field) stays on one line when it fits in [`FormatOptions::max_width`]. Otherwise it breaks:
//! one item per line with the closing bracket on its own line, operators at the start of the
//! continuation lines, `then` and `else` branches indented, and the value of a step on the next
//! line.
//!
//! The result is checked before it is returned: it must tokenize to the same tokens and comments
//! and parse to the same tree as the input. Otherwise [`format`] returns
//! [`FormatError::Unstable`] rather than a text with another meaning.

use std::fmt;

use super::lexer::{Tok, Token, tokenize};
use crate::{Error, Limits};

/// Largest indentation width [`format`] uses (wider values are capped).
pub const MAX_INDENT_WIDTH: usize = 16;

/// Layout options of [`format`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatOptions {
    /// Preferred maximum line length in characters (default 100). A line is longer only where a
    /// single token, comment or unbreakable run is.
    pub max_width: usize,
    /// Spaces per indentation level (default 4, at most [`MAX_INDENT_WIDTH`]), or the width a
    /// tab counts for with `use_tabs`.
    pub indent_width: usize,
    /// Indent with tabs instead of spaces.
    pub use_tabs: bool,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            max_width: 100,
            indent_width: 4,
            use_tabs: false,
        }
    }
}

/// Why [`format`] returned no text.
#[derive(Debug)]
pub enum FormatError {
    /// The text does not parse, or is over a [`Limits`] cap.
    Invalid(Error),
    /// The layout would have changed the text's tokens or syntax tree (a formatter bug); the
    /// text is better left as it is.
    Unstable,
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormatError::Invalid(e) => write!(f, "{e}"),
            FormatError::Unstable => f.write_str("the formatted text would not be equivalent to the input"),
        }
    }
}

impl std::error::Error for FormatError {}

/// Formats `src`, an M expression. A blank text formats to an empty one. The output uses `\n`
/// line breaks (line breaks inside text literals and comments stay as written) and has no
/// trailing line break. `limits` caps the source size and nesting as for evaluation; the work
/// runs on the evaluation thread's large stack.
pub fn format(src: &str, opts: &FormatOptions, limits: &Limits) -> Result<String, FormatError> {
    limits
        .check_bytes(src.len(), "M expression")
        .map_err(FormatError::Invalid)?;
    if src.trim().is_empty() {
        return Ok(String::new());
    }
    let (src, opts, limits) = (src.to_owned(), *opts, *limits);
    super::on_eval_thread(move || Ok(format_here(&src, &opts, &limits))).map_err(FormatError::Invalid)?
}

fn format_here(src: &str, opts: &FormatOptions, limits: &Limits) -> Result<String, FormatError> {
    let opts = FormatOptions {
        indent_width: opts.indent_width.min(MAX_INDENT_WIDTH),
        ..*opts
    };
    // bicraft's parser gives the syntax errors (with their line and column).
    super::parser::parse(src, limits).map_err(FormatError::Invalid)?;
    let toks = tokenize(src, limits).map_err(FormatError::Invalid)?;
    let (cst, names) = Cst::parse(&toks, limits.max_depth).ok_or(FormatError::Unstable)?;
    let mut emitter = Emitter::new(src, &toks);
    emitter.expr(&cst);
    let out = emitter.finish(&opts);
    // Equivalence: the same tokens and comments, the same tree, and the same generalized names
    // (the only syntax whose spaces count).
    let again = tokenize(&out, limits).map_err(|_| FormatError::Unstable)?;
    let (cst_again, names_again) = Cst::parse(&again, limits.max_depth).ok_or(FormatError::Unstable)?;
    let spelled = |text: &str, toks: &[Token], names: &[(usize, usize)]| -> Vec<String> {
        names
            .iter()
            .map(|&(a, b)| {
                let start = toks.get(a).map_or(0, |t| t.start);
                let end = b.checked_sub(1).and_then(|i| toks.get(i)).map_or(0, |t| t.end);
                text.get(start..end).unwrap_or("").to_owned()
            })
            .collect()
    };
    if format!("{cst:?}") != format!("{cst_again:?}")
        || spelled(src, &toks, &names) != spelled(&out, &again, &names_again)
        || !same_tokens(src, &toks, &out, &again)
    {
        return Err(FormatError::Unstable);
    }
    Ok(out)
}

// ---- syntax tree of token ranges -------------------------------------------

/// One past the index of the last token of a name or a run of words.
type End = usize;

/// An M expression as token ranges, mirroring `parser.rs`. Chains are flat vectors so that long
/// inputs never nest deeply.
#[derive(Debug)]
enum N {
    /// Tokens written without spaces (literal, identifier, `@name`, `...`).
    Leaf(End),
    /// The implicit `_` of `[field]`: no token.
    Implicit,
    Paren(Box<N>),
    Let(Vec<(End, N)>, Box<N>),
    If(Box<N>, Box<N>, Box<N>),
    Each(Box<N>),
    Error(Box<N>),
    Try(Box<N>, Handler),
    Func(Vec<P>, Option<End>, Box<N>),
    Postfix(Box<N>, Vec<Suffix>),
    List(Vec<(N, Option<N>)>),
    Record(Vec<(End, N)>),
    Unary(bool, Box<N>),
    /// Operands of one precedence; each operator is the token before its operand.
    Chain(Box<N>, Vec<N>),
    /// `as` / `is` types (each a run of words).
    Typed(Box<N>, Vec<End>),
    Type(Box<T>),
}

#[derive(Debug)]
enum Handler {
    None,
    Otherwise(Box<N>),
    /// `catch` and its function.
    Catch(Box<N>),
}

#[derive(Debug)]
enum Suffix {
    Call(Vec<N>),
    Item(Box<N>, bool),
    Field(End, bool),
    Project(Vec<End>, bool),
}

/// `optional? name (as nullable? primitive)?`.
#[derive(Debug)]
struct P {
    optional: bool,
    name: End,
    ty: Option<End>,
}

#[derive(Debug)]
enum T {
    Words(End),
    Nullable(Box<T>),
    /// A nested `type` keyword.
    Type(Box<T>),
    Table(Box<T>),
    Record(Vec<F>, bool),
    List(Box<T>),
    Function(Vec<P>, Option<End>),
    Expr(Box<N>),
}

#[derive(Debug)]
struct F {
    optional: bool,
    name: End,
    ty: Option<T>,
}

const KEYWORDS: [&str; 20] = [
    "and",
    "as",
    "each",
    "else",
    "error",
    "false",
    "if",
    "in",
    "is",
    "let",
    "meta",
    "not",
    "null",
    "or",
    "otherwise",
    "section",
    "shared",
    "then",
    "true",
    "try",
];

/// The parser of [`N`]: the grammar of `parser.rs`, keeping token positions instead of values.
struct Cst<'a> {
    toks: &'a [Token],
    pos: usize,
    depth: usize,
    max_depth: usize,
    /// Token ranges of the generalized names of more than one token.
    names: Vec<(usize, usize)>,
}

impl<'a> Cst<'a> {
    fn parse(toks: &'a [Token], max_depth: usize) -> Option<(N, Vec<(usize, usize)>)> {
        let mut p = Cst {
            toks,
            pos: 0,
            depth: 0,
            max_depth,
            names: Vec::new(),
        };
        let e = p.expr()?;
        matches!(p.peek(), Tok::Eof).then_some((e, p.names))
    }

    fn peek(&self) -> &Tok {
        self.toks.get(self.pos).map_or(&Tok::Eof, |t| &t.tok)
    }

    fn peek_at(&self, k: usize) -> &Tok {
        self.toks
            .get(self.pos.saturating_add(k))
            .map_or(&Tok::Eof, |t| &t.tok)
    }

    fn bump(&mut self) {
        if self.pos < self.toks.len() {
            self.pos += 1;
        }
    }

    fn is_punct(&self, p: &str) -> bool {
        matches!(self.peek(), Tok::Punct(q) if *q == p)
    }

    fn eat(&mut self, p: &str) -> bool {
        let hit = self.is_punct(p);
        if hit {
            self.bump();
        }
        hit
    }

    fn expect(&mut self, p: &str) -> Option<()> {
        self.eat(p).then_some(())
    }

    fn is_kw(&self, kw: &str) -> bool {
        matches!(self.peek(), Tok::Ident(s) if s == kw)
    }

    fn eat_kw(&mut self, kw: &str) -> bool {
        let hit = self.is_kw(kw);
        if hit {
            self.bump();
        }
        hit
    }

    fn enter(&mut self) -> Option<()> {
        self.depth += 1;
        (self.depth <= self.max_depth).then_some(())
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn name(&mut self) -> Option<End> {
        match self.peek() {
            Tok::Ident(s) if !KEYWORDS.contains(&s.as_str()) => {}
            Tok::Quoted(_) => {}
            _ => return None,
        }
        self.bump();
        Some(self.pos)
    }

    fn field_name(&mut self) -> Option<End> {
        if let Tok::Quoted(_) = self.peek() {
            self.bump();
            return Some(self.pos);
        }
        let start = self.pos;
        while let Tok::Ident(_) | Tok::Number(_) | Tok::Hash(_) = self.peek() {
            self.bump();
        }
        if self.pos > start + 1 {
            self.names.push((start, self.pos));
        }
        (self.pos > start).then_some(self.pos)
    }

    fn expr(&mut self) -> Option<N> {
        self.enter()?;
        let r = self.expr_inner();
        self.leave();
        r
    }

    fn expr_inner(&mut self) -> Option<N> {
        if self.eat_kw("let") {
            let mut bindings = Vec::new();
            loop {
                let name = self.name()?;
                self.expect("=")?;
                bindings.push((name, self.expr()?));
                if !self.eat(",") {
                    break;
                }
            }
            if !self.eat_kw("in") {
                return None;
            }
            return Some(N::Let(bindings, Box::new(self.expr()?)));
        }
        if self.eat_kw("if") {
            let c = self.expr()?;
            if !self.eat_kw("then") {
                return None;
            }
            let t = self.expr()?;
            if !self.eat_kw("else") {
                return None;
            }
            let f = self.expr()?;
            return Some(N::If(Box::new(c), Box::new(t), Box::new(f)));
        }
        if self.eat_kw("each") {
            return Some(N::Each(Box::new(self.expr()?)));
        }
        if self.eat_kw("error") {
            return Some(N::Error(Box::new(self.expr()?)));
        }
        if self.eat_kw("try") {
            let protected = self.expr()?;
            let handler = if self.eat_kw("otherwise") {
                Handler::Otherwise(Box::new(self.expr()?))
            } else if self.eat_kw("catch") {
                Handler::Catch(Box::new(self.try_function()?))
            } else {
                Handler::None
            };
            return Some(N::Try(Box::new(protected), handler));
        }
        if self.is_punct("(") {
            let save = self.pos;
            if let Some(f) = self.try_function() {
                return Some(f);
            }
            self.pos = save;
        }
        self.coalesce()
    }

    /// `(params) [as type] => body`, or `None` (the caller restores the position).
    fn try_function(&mut self) -> Option<N> {
        let save = self.pos;
        let Some(params) = self.params() else {
            self.pos = save;
            return None;
        };
        let ret = if self.eat_kw("as") {
            // As in `parser.rs`: the return type is read and a bad one ignored.
            let _ = self.nullable_primitive();
            Some(self.pos)
        } else {
            None
        };
        if !self.eat("=>") {
            self.pos = save;
            return None;
        }
        let body = self.expr()?;
        Some(N::Func(params, ret, Box::new(body)))
    }

    fn params(&mut self) -> Option<Vec<P>> {
        if !self.eat("(") {
            return None;
        }
        let mut params = Vec::new();
        if self.eat(")") {
            return Some(params);
        }
        loop {
            let optional =
                matches!(self.peek_at(1), Tok::Ident(_) | Tok::Quoted(_)) && self.eat_kw("optional");
            let name = self.name()?;
            let ty = if self.eat_kw("as") {
                self.nullable_primitive()?;
                Some(self.pos)
            } else {
                None
            };
            params.push(P { optional, name, ty });
            if self.eat(")") {
                return Some(params);
            }
            if !self.eat(",") {
                return None;
            }
        }
    }

    fn nullable_primitive(&mut self) -> Option<End> {
        self.eat_kw("nullable");
        match self.peek() {
            Tok::Ident(s) if super::parser::primitive(s).is_some() => {
                self.bump();
                Some(self.pos)
            }
            _ => None,
        }
    }

    fn chain(&mut self, next: fn(&mut Self) -> Option<N>, ops: &[&str], keyword: bool) -> Option<N> {
        let first = next(self)?;
        let mut rest = Vec::new();
        while ops
            .iter()
            .any(|s| if keyword { self.is_kw(s) } else { self.is_punct(s) })
        {
            self.bump();
            self.enter()?;
            let rhs = next(self);
            self.leave();
            rest.push(rhs?);
        }
        Some(if rest.is_empty() {
            first
        } else {
            N::Chain(Box::new(first), rest)
        })
    }

    fn coalesce(&mut self) -> Option<N> {
        let lhs = self.logical_or()?;
        if self.eat("??") {
            self.enter()?;
            let rhs = self.coalesce();
            self.leave();
            return Some(N::Chain(Box::new(lhs), vec![rhs?]));
        }
        Some(lhs)
    }

    fn logical_or(&mut self) -> Option<N> {
        self.chain(Self::logical_and, &["or"], true)
    }

    fn logical_and(&mut self) -> Option<N> {
        self.chain(Self::is_expr, &["and"], true)
    }

    fn typed(&mut self, next: fn(&mut Self) -> Option<N>, kw: &str) -> Option<N> {
        let e = next(self)?;
        let mut types = Vec::new();
        while self.eat_kw(kw) {
            types.push(self.nullable_primitive()?);
        }
        Some(if types.is_empty() {
            e
        } else {
            N::Typed(Box::new(e), types)
        })
    }

    fn is_expr(&mut self) -> Option<N> {
        self.typed(Self::as_expr, "is")
    }

    fn as_expr(&mut self) -> Option<N> {
        self.typed(Self::equality, "as")
    }

    fn equality(&mut self) -> Option<N> {
        self.chain(Self::relational, &["=", "<>"], false)
    }

    fn relational(&mut self) -> Option<N> {
        self.chain(Self::additive, &["<=", ">=", "<", ">"], false)
    }

    fn additive(&mut self) -> Option<N> {
        self.chain(Self::multiplicative, &["+", "-", "&"], false)
    }

    fn multiplicative(&mut self) -> Option<N> {
        self.chain(Self::metadata, &["*", "/"], false)
    }

    fn metadata(&mut self) -> Option<N> {
        let first = self.unary()?;
        let mut rest = Vec::new();
        while self.eat_kw("meta") {
            rest.push(self.unary()?);
        }
        Some(if rest.is_empty() {
            first
        } else {
            N::Chain(Box::new(first), rest)
        })
    }

    fn unary(&mut self) -> Option<N> {
        self.enter()?;
        let r = self.unary_inner();
        self.leave();
        r
    }

    fn unary_inner(&mut self) -> Option<N> {
        if self.eat("+") || self.eat("-") {
            return Some(N::Unary(false, Box::new(self.unary()?)));
        }
        if self.eat_kw("not") {
            return Some(N::Unary(true, Box::new(self.unary()?)));
        }
        if self.eat_kw("type") {
            return Some(N::Type(Box::new(self.primary_type()?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Option<N> {
        let base = self.base()?;
        let (target, mut suffixes) = match base {
            N::Postfix(t, s) => (t, s),
            other => (Box::new(other), Vec::new()),
        };
        loop {
            if self.eat("(") {
                let mut args = Vec::new();
                if !self.eat(")") {
                    loop {
                        args.push(self.expr()?);
                        if self.eat(")") {
                            break;
                        }
                        self.expect(",")?;
                    }
                }
                suffixes.push(Suffix::Call(args));
            } else if self.eat("{") {
                let i = self.expr()?;
                self.expect("}")?;
                let opt = self.eat("?");
                suffixes.push(Suffix::Item(Box::new(i), opt));
            } else if self.eat("[") {
                suffixes.push(self.field_suffix()?);
            } else {
                break;
            }
        }
        Some(if suffixes.is_empty() {
            *target
        } else {
            N::Postfix(target, suffixes)
        })
    }

    /// After `[`: `name]`, `name]?`, `[a], [b]]`.
    fn field_suffix(&mut self) -> Option<Suffix> {
        if self.is_punct("[") {
            let mut names = Vec::new();
            loop {
                self.expect("[")?;
                names.push(self.field_name()?);
                self.expect("]")?;
                if self.eat("]") {
                    break;
                }
                self.expect(",")?;
            }
            let opt = self.eat("?");
            return Some(Suffix::Project(names, opt));
        }
        let name = self.field_name()?;
        self.expect("]")?;
        let opt = self.eat("?");
        Some(Suffix::Field(name, opt))
    }

    fn base(&mut self) -> Option<N> {
        match self.peek().clone() {
            Tok::Number(_) | Tok::Text(_) | Tok::Hash(_) | Tok::Quoted(_) => {
                self.bump();
                Some(N::Leaf(self.pos))
            }
            Tok::Ident(s) => {
                if KEYWORDS.contains(&s.as_str()) && !matches!(s.as_str(), "true" | "false" | "null") {
                    return None;
                }
                self.bump();
                Some(N::Leaf(self.pos))
            }
            Tok::Punct("@") => {
                self.bump();
                self.name()?;
                Some(N::Leaf(self.pos))
            }
            Tok::Punct("...") => {
                self.bump();
                Some(N::Leaf(self.pos))
            }
            Tok::Punct("(") => {
                self.bump();
                let e = self.expr()?;
                self.expect(")")?;
                Some(N::Paren(Box::new(e)))
            }
            Tok::Punct("{") => {
                self.bump();
                let mut items = Vec::new();
                if !self.eat("}") {
                    loop {
                        let a = self.expr()?;
                        let b = if self.eat("..") { Some(self.expr()?) } else { None };
                        items.push((a, b));
                        if self.eat("}") {
                            break;
                        }
                        self.expect(",")?;
                    }
                }
                Some(N::List(items))
            }
            Tok::Punct("[") => {
                self.bump();
                if self.eat("]") {
                    return Some(N::Record(Vec::new()));
                }
                if self.is_punct("[") {
                    let s = self.field_suffix()?;
                    return Some(N::Postfix(Box::new(N::Implicit), vec![s]));
                }
                let save = self.pos;
                let name = self.field_name()?;
                if !self.is_punct("=") {
                    self.pos = save;
                    let s = self.field_suffix()?;
                    return Some(N::Postfix(Box::new(N::Implicit), vec![s]));
                }
                let mut fields = Vec::new();
                let mut name = name;
                loop {
                    self.expect("=")?;
                    fields.push((name, self.expr()?));
                    if self.eat("]") {
                        break;
                    }
                    self.expect(",")?;
                    name = self.field_name()?;
                }
                Some(N::Record(fields))
            }
            _ => None,
        }
    }

    fn primary_type(&mut self) -> Option<T> {
        self.enter()?;
        let r = self.primary_type_inner();
        self.leave();
        r
    }

    fn primary_type_inner(&mut self) -> Option<T> {
        let word = match self.peek() {
            Tok::Ident(s) => Some(s.clone()),
            _ => None,
        };
        match word.as_deref() {
            Some("nullable") => {
                self.bump();
                return Some(T::Nullable(Box::new(self.primary_type()?)));
            }
            Some("type") => {
                self.bump();
                return Some(T::Type(Box::new(self.primary_type()?)));
            }
            Some("table") => {
                self.bump();
                return Some(match self.peek() {
                    Tok::Punct("[") => T::Table(Box::new(self.primary_type()?)),
                    Tok::Ident(n)
                        if !KEYWORDS.contains(&n.as_str()) && super::parser::primitive(n).is_none() =>
                    {
                        T::Table(Box::new(T::Expr(Box::new(self.primary()?))))
                    }
                    Tok::Quoted(_) | Tok::Punct("(") => {
                        T::Table(Box::new(T::Expr(Box::new(self.primary()?))))
                    }
                    _ => T::Words(self.pos),
                });
            }
            Some("function") if matches!(self.peek_at(1), Tok::Punct("(")) => {
                self.bump();
                let params = self.params()?;
                let ret = if self.eat_kw("as") {
                    self.nullable_primitive()?;
                    Some(self.pos)
                } else {
                    None
                };
                return Some(T::Function(params, ret));
            }
            Some(s) if super::parser::primitive(s).is_some() => {
                self.bump();
                return Some(T::Words(self.pos));
            }
            _ => {}
        }
        if self.eat("{") {
            let item = self.primary_type()?;
            self.expect("}")?;
            return Some(T::List(Box::new(item)));
        }
        if self.eat("[") {
            let mut fields = Vec::new();
            if self.eat("]") {
                return Some(T::Record(fields, false));
            }
            loop {
                if self.eat("...") {
                    self.expect("]")?;
                    return Some(T::Record(fields, true));
                }
                let optional =
                    matches!(self.peek_at(1), Tok::Ident(_) | Tok::Quoted(_)) && self.eat_kw("optional");
                let name = self.field_name()?;
                let ty = if self.eat("=") {
                    Some(self.primary_type()?)
                } else {
                    None
                };
                fields.push(F { optional, name, ty });
                if self.eat("]") {
                    return Some(T::Record(fields, false));
                }
                self.expect(",")?;
            }
        }
        Some(T::Expr(Box::new(self.primary()?)))
    }
}

// ---- comments and tokens ---------------------------------------------------

/// Line breaks in `s`, as the M lexical grammar counts them (`CR LF` is one).
fn line_breaks(s: &str) -> usize {
    let mut n = 0usize;
    let mut prev_cr = false;
    for c in s.chars() {
        match c {
            '\n' if prev_cr => {}
            '\r' | '\n' | '\u{0085}' | '\u{2028}' | '\u{2029}' => n = n.saturating_add(1),
            _ => {}
        }
        prev_cr = c == '\r';
    }
    n
}

/// The comments in `src[from..to]`, a gap between tokens (white space and comments only, as the
/// lexer skips them): `(start, end, is_line_comment)`.
fn gap_comments(src: &str, from: usize, to: usize, out: &mut Vec<(usize, usize, bool)>) {
    let Some(gap) = src.get(from..to) else {
        return;
    };
    let mut i = 0usize;
    while let Some(rest) = gap.get(i..) {
        if rest.is_empty() {
            break;
        }
        if rest.starts_with("//") {
            let len = rest.find('\n').unwrap_or(rest.len());
            out.push((from + i, from + i + len, true));
            i += len;
        } else if let Some(body) = rest.strip_prefix("/*") {
            let len = body.find("*/").map_or(rest.len(), |k| k + 4);
            out.push((from + i, from + i + len, false));
            i += len;
        } else {
            i += rest.chars().next().map_or(1, char::len_utf8);
        }
    }
}

/// A token (`Some(index)`) or a comment (`None`, line comment or not), in source order:
/// `(start, end, token, line)`.
fn items(src: &str, toks: &[Token]) -> Vec<(usize, usize, Option<usize>, bool)> {
    let mut out = Vec::new();
    let mut comments = Vec::new();
    let mut prev_end = 0usize;
    for (i, t) in toks.iter().enumerate() {
        gap_comments(src, prev_end, t.start, &mut comments);
        out.extend(comments.drain(..).map(|(s, e, line)| (s, e, None, line)));
        if t.tok != Tok::Eof {
            out.push((t.start, t.end, Some(i), false));
        }
        prev_end = t.end;
    }
    out
}

#[derive(Debug, Clone, Copy)]
struct Note<'a> {
    text: &'a str,
    line: bool,
    /// A blank line separates it from what came before.
    blank_before: bool,
    /// A line break follows it in the source.
    newline_after: bool,
}

/// A token with the comments attached to it.
#[derive(Debug)]
struct Sig<'a> {
    tok: &'a Tok,
    start: usize,
    end: usize,
    text: &'a str,
    /// Comments on their own lines before the token.
    leading: Vec<Note<'a>>,
    /// Comments after the token on its line.
    trailing: Vec<Note<'a>>,
    /// A blank line separates it from what came before (its last leading comment, or the
    /// previous token).
    blank_before: bool,
}

/// Splits the source into tokens with their comments, and the comments after the last token.
fn attach<'a>(src: &'a str, toks: &'a [Token]) -> (Vec<Sig<'a>>, Vec<Note<'a>>) {
    let all = items(src, toks);
    let mut sigs: Vec<Sig<'a>> = Vec::new();
    let mut pending: Vec<Note<'a>> = Vec::new();
    let mut newline_since_sig = false;
    let mut prev_end = 0usize;
    for (i, &(start, end, tok, line)) in all.iter().enumerate() {
        let breaks = line_breaks(src.get(prev_end..start).unwrap_or(""));
        newline_since_sig |= breaks > 0;
        let text = src.get(start..end).unwrap_or("");
        match tok.and_then(|k| toks.get(k)) {
            None => {
                let next = all.get(i + 1).map_or(src.len(), |&(s, ..)| s);
                let newline_after = line || line_breaks(src.get(end..next).unwrap_or("")) > 0;
                let note = Note {
                    text: if line { text.trim_end() } else { text },
                    line,
                    blank_before: breaks >= 2,
                    newline_after,
                };
                match sigs.last_mut() {
                    Some(last) if !newline_since_sig => last.trailing.push(note),
                    _ => pending.push(note),
                }
            }
            Some(t) => {
                sigs.push(Sig {
                    tok: &t.tok,
                    start,
                    end,
                    text,
                    leading: std::mem::take(&mut pending),
                    trailing: Vec::new(),
                    blank_before: breaks >= 2,
                });
                newline_since_sig = false;
            }
        }
        prev_end = end;
    }
    (sigs, pending)
}

// ---- layout commands -------------------------------------------------------

/// A layout command. Groups render flat (`Line` as a space, `SoftLine` as nothing) when they
/// fit on the rest of the line, else broken (both as line breaks).
#[derive(Debug)]
enum Cmd<'a> {
    Text(&'a str),
    Space,
    Line,
    SoftLine,
    HardLine,
    /// A line break with an empty line before the next text.
    Blank,
    /// A comment on its own line.
    Leading(Note<'a>),
    /// A comment after the text on the line.
    Trailing(Note<'a>),
    GroupStart,
    GroupEnd,
    Indent,
    Dedent,
}

struct Emitter<'a> {
    src: &'a str,
    sigs: Vec<Sig<'a>>,
    tail: Vec<Note<'a>>,
    pos: usize,
    cmds: Vec<Cmd<'a>>,
    /// Trailing comments of the last token, placed after any group or indentation that ends
    /// right after it.
    trailing: Vec<Note<'a>>,
}

impl<'a> Emitter<'a> {
    fn new(src: &'a str, toks: &'a [Token]) -> Self {
        let (sigs, tail) = attach(src, toks);
        Self {
            src,
            sigs,
            tail,
            pos: 0,
            cmds: Vec::new(),
            trailing: Vec::new(),
        }
    }

    fn finish(mut self, opts: &FormatOptions) -> String {
        // Tokens the layout did not place (none for a parsed text) still go out, so nothing is
        // lost; the equivalence check then decides.
        while self.pos < self.sigs.len() {
            self.push(Cmd::Space);
            self.tok();
        }
        self.flush_trailing();
        let tail = std::mem::take(&mut self.tail);
        self.cmds.extend(tail.into_iter().map(Cmd::Leading));
        render(&self.cmds, opts)
    }

    fn flush_trailing(&mut self) {
        if !self.trailing.is_empty() {
            let trailing = std::mem::take(&mut self.trailing);
            self.cmds.extend(trailing.into_iter().map(Cmd::Trailing));
        }
    }

    fn push(&mut self, cmd: Cmd<'a>) {
        if !matches!(cmd, Cmd::GroupEnd | Cmd::Dedent) {
            self.flush_trailing();
        }
        self.cmds.push(cmd);
    }

    fn peek(&self) -> Option<&Sig<'a>> {
        self.sigs.get(self.pos)
    }

    fn peek_is(&self, p: &str) -> bool {
        self.peek()
            .is_some_and(|s| matches!(s.tok, Tok::Punct(q) if *q == p))
    }

    /// Places the next token's own-line comments now, ahead of any group that starts with the
    /// token.
    fn leading(&mut self) {
        let Some(sig) = self.sigs.get_mut(self.pos) else {
            return;
        };
        if sig.leading.is_empty() {
            return;
        }
        let comments = std::mem::take(&mut sig.leading);
        let blank_after = std::mem::take(&mut sig.blank_before);
        self.flush_trailing();
        let mut at = self.cmds.len();
        while at > 0 && matches!(self.cmds.get(at - 1), Some(Cmd::GroupStart)) {
            at -= 1;
        }
        let mut insert: Vec<Cmd<'a>> = comments.into_iter().map(Cmd::Leading).collect();
        if blank_after {
            insert.push(Cmd::Blank);
        }
        self.cmds.splice(at..at, insert);
    }

    /// Emits the next token.
    fn tok(&mut self) {
        self.leading();
        let Some(sig) = self.sigs.get_mut(self.pos) else {
            return;
        };
        self.pos += 1;
        let text = sig.text;
        let trailing = std::mem::take(&mut sig.trailing);
        self.push(Cmd::Text(text));
        self.trailing = trailing;
    }

    /// Emits the tokens before index `end`, without spaces.
    fn leaf(&mut self, end: End) {
        while self.pos < end.min(self.sigs.len()) {
            self.tok();
        }
    }

    /// Emits the tokens before index `end`, separated by spaces (`nullable text`).
    fn words(&mut self, end: End) {
        let mut first = true;
        while self.pos < end.min(self.sigs.len()) {
            if !first {
                self.push(Cmd::Space);
            }
            self.tok();
            first = false;
        }
    }

    /// A name. A generalized identifier (`[Order Date]`) is several tokens whose spaces are part
    /// of the name, so it goes out as written.
    fn name(&mut self, end: End) {
        let end = end.min(self.sigs.len());
        if self.pos >= end {
            return;
        }
        let last = end - 1;
        if last == self.pos {
            self.tok();
            return;
        }
        self.leading();
        let start = self.peek().map_or(0, |s| s.start);
        let Some(last_sig) = self.sigs.get_mut(last) else {
            return;
        };
        let trailing = std::mem::take(&mut last_sig.trailing);
        let text = self.src.get(start..last_sig.end).unwrap_or("");
        self.pos = end;
        self.push(Cmd::Text(text));
        self.trailing = trailing;
    }

    /// A line break, with an empty line when the source had one before the next token.
    fn statement_break(&mut self) {
        let blank = self
            .peek()
            .is_some_and(|s| s.blank_before && s.leading.is_empty());
        self.push(if blank { Cmd::Blank } else { Cmd::HardLine });
    }

    /// `,` and a line break or space.
    fn separator(&mut self) {
        if self.peek_is(",") {
            self.tok();
        }
        self.push(Cmd::Line);
    }

    /// `n` items of a list whose opener was just emitted, then its closer.
    fn items_then_close(&mut self, n: usize, mut each: impl FnMut(&mut Self, usize)) {
        if n > 0 {
            self.push(Cmd::Indent);
            self.push(Cmd::SoftLine);
            for i in 0..n {
                if i > 0 {
                    self.separator();
                }
                each(self, i);
            }
            self.leading();
            self.push(Cmd::Dedent);
            self.push(Cmd::SoftLine);
        }
        self.tok();
    }

    /// `( … )`, `[ … ]`, `{ … }`: the next token is the opener.
    fn list(&mut self, n: usize, each: impl FnMut(&mut Self, usize)) {
        self.push(Cmd::GroupStart);
        self.tok();
        self.items_then_close(n, each);
        self.push(Cmd::GroupEnd);
    }

    /// The value of a step or field after its `=` (or a body after `each`, `try`, `error`):
    /// bracketed and simple values follow on the line and break inside; `let`, `if`, `try` and
    /// operator chains move to the next line, indented, when they do not fit.
    fn value(&mut self, e: &N) {
        match e {
            N::Let(..) | N::If(..) | N::Try(..) | N::Chain(..) | N::Typed(..) | N::Error(_) => self.assign(e),
            _ => {
                self.push(Cmd::Space);
                self.expr(e);
            }
        }
    }

    /// `e` on this line when it fits, else on the next line, indented.
    fn assign(&mut self, e: &N) {
        self.push(Cmd::GroupStart);
        self.push(Cmd::Indent);
        self.push(Cmd::Line);
        self.expr(e);
        self.push(Cmd::Dedent);
        self.push(Cmd::GroupEnd);
    }

    fn expr(&mut self, e: &N) {
        match e {
            N::Leaf(end) => self.leaf(*end),
            N::Implicit => {}
            N::Paren(inner) => self.list(1, |s, _| s.expr(inner)),
            N::Let(bindings, body) => self.let_expr(bindings, body),
            N::If(..) => self.if_expr(e),
            N::Each(body) | N::Error(body) => {
                self.tok();
                self.value(body);
            }
            N::Func(params, ret, body) => {
                self.signature(params, *ret);
                // The body, not the parameters, moves to the next line.
                self.assign(body);
            }
            N::Postfix(target, suffixes) => {
                self.expr(target);
                for s in suffixes {
                    self.suffix(s);
                }
            }
            N::Record(fields) => self.list(fields.len(), |s, i| {
                if let Some((name, value)) = fields.get(i) {
                    s.name(*name);
                    s.push(Cmd::Space);
                    s.tok(); // =
                    s.value(value);
                }
            }),
            N::List(items) => self.list(items.len(), |s, i| {
                if let Some((start, end)) = items.get(i) {
                    s.expr(start);
                    if let Some(end) = end {
                        s.tok(); // ..
                        // `1.. .5` must not become `1...5`.
                        if s.peek().is_some_and(|t| t.text.starts_with('.')) {
                            s.push(Cmd::Space);
                        }
                        s.expr(end);
                    }
                }
            }),
            N::Unary(not, operand) => {
                self.tok();
                if *not {
                    self.push(Cmd::Space);
                }
                self.expr(operand);
            }
            N::Chain(..) => self.binary(e, 0),
            N::Typed(inner, types) => {
                self.expr(inner);
                for end in types {
                    self.push(Cmd::Space);
                    self.tok(); // as / is
                    self.push(Cmd::Space);
                    self.words(*end);
                }
            }
            N::Type(ty) => {
                self.tok(); // type
                self.push(Cmd::Space);
                self.ty(ty);
            }
            N::Try(protected, handler) => {
                self.push(Cmd::GroupStart);
                self.tok(); // try
                self.value(protected);
                match handler {
                    Handler::None => {}
                    Handler::Otherwise(default) => {
                        self.push(Cmd::Indent);
                        self.push(Cmd::Line);
                        self.tok(); // otherwise
                        self.value(default);
                        self.push(Cmd::Dedent);
                    }
                    Handler::Catch(function) => {
                        self.push(Cmd::Indent);
                        self.push(Cmd::Line);
                        self.tok(); // catch
                        self.push(Cmd::Space);
                        // The handler's body follows on its line, as a step's value does.
                        match &**function {
                            N::Func(params, ret, body) if params.len() <= 1 => {
                                self.tok(); // (
                                if let Some(p) = params.first() {
                                    self.param(p);
                                }
                                self.tok(); // )
                                if let Some(end) = ret {
                                    self.push(Cmd::Space);
                                    self.tok(); // as
                                    self.push(Cmd::Space);
                                    self.words(*end);
                                }
                                self.push(Cmd::Space);
                                self.tok(); // =>
                                self.value(body);
                            }
                            other => self.expr(other),
                        }
                        self.push(Cmd::Dedent);
                    }
                }
                self.push(Cmd::GroupEnd);
            }
        }
    }

    fn suffix(&mut self, s: &Suffix) {
        match s {
            Suffix::Call(args) => self.list(args.len(), |s, i| {
                if let Some(a) = args.get(i) {
                    s.expr(a);
                }
            }),
            Suffix::Item(selector, optional) => {
                self.tok(); // {
                self.expr(selector);
                self.tok(); // }
                if *optional {
                    self.tok();
                }
            }
            Suffix::Field(name, optional) => {
                self.tok(); // [
                self.name(*name);
                self.tok(); // ]
                if *optional {
                    self.tok();
                }
            }
            Suffix::Project(names, optional) => {
                self.list(names.len(), |s, i| {
                    if let Some(name) = names.get(i) {
                        s.tok(); // [
                        s.name(*name);
                        s.tok(); // ]
                    }
                });
                if *optional {
                    self.tok();
                }
            }
        }
    }

    /// `(params) as type =>` of a function expression.
    fn signature(&mut self, params: &[P], ret: Option<End>) {
        self.list(params.len(), |s, i| {
            if let Some(p) = params.get(i) {
                s.param(p);
            }
        });
        if let Some(end) = ret {
            self.push(Cmd::Space);
            self.tok(); // as
            self.push(Cmd::Space);
            self.words(end);
        }
        self.push(Cmd::Space);
        self.tok(); // =>
    }

    /// `optional? name (as nullable? type)?` of a function expression or type.
    fn param(&mut self, p: &P) {
        if p.optional {
            self.tok();
            self.push(Cmd::Space);
        }
        self.name(p.name);
        if let Some(end) = p.ty {
            self.push(Cmd::Space);
            self.tok(); // as
            self.push(Cmd::Space);
            self.words(end);
        }
    }

    /// `let` with one step per line, then `in` and the result on the next line.
    fn let_expr(&mut self, bindings: &[(End, N)], body: &N) {
        self.tok(); // let
        self.push(Cmd::Indent);
        self.push(Cmd::HardLine);
        for (i, (name, value)) in bindings.iter().enumerate() {
            if i > 0 {
                if self.peek_is(",") {
                    self.tok();
                }
                self.statement_break();
            }
            self.name(*name);
            self.push(Cmd::Space);
            self.tok(); // =
            self.value(value);
        }
        self.leading();
        self.push(Cmd::Dedent);
        self.push(Cmd::HardLine);
        self.tok(); // in
        self.push(Cmd::Indent);
        self.push(Cmd::HardLine);
        self.expr(body);
        self.push(Cmd::Dedent);
    }

    /// `if … then … else …`, flat or with indented branches; `else if` chains stay at one level.
    fn if_expr(&mut self, e: &N) {
        self.push(Cmd::GroupStart);
        let mut e = e;
        while let N::If(condition, then_branch, else_branch) = e {
            self.tok(); // if
            self.push(Cmd::Space);
            self.expr(condition);
            self.push(Cmd::Space);
            self.tok(); // then
            self.push(Cmd::Indent);
            self.push(Cmd::Line);
            self.expr(then_branch);
            self.push(Cmd::Dedent);
            self.push(Cmd::Line);
            self.tok(); // else
            if matches!(**else_branch, N::If(..)) {
                self.push(Cmd::Space);
                e = else_branch;
                continue;
            }
            self.push(Cmd::Indent);
            self.push(Cmd::Line);
            self.expr(else_branch);
            self.push(Cmd::Dedent);
            break;
        }
        self.push(Cmd::GroupEnd);
    }

    /// A chain of operators of one precedence (`a & b & c`): flat, or one operator and operand
    /// per continuation line. `extra` indents the continuation lines further: a tighter chain
    /// that is the first operand of another breaks one level deeper than it, so their operators
    /// never line up.
    fn binary(&mut self, e: &N, extra: usize) {
        let N::Chain(first, operands) = e else {
            self.expr(e);
            return;
        };
        self.push(Cmd::GroupStart);
        match &**first {
            N::Chain(..) => self.binary(first, extra.saturating_add(1)),
            other => self.expr(other),
        }
        for _ in 0..=extra {
            self.push(Cmd::Indent);
        }
        for operand in operands {
            self.push(Cmd::Line);
            self.tok();
            self.push(Cmd::Space);
            self.expr(operand);
        }
        for _ in 0..=extra {
            self.push(Cmd::Dedent);
        }
        self.push(Cmd::GroupEnd);
    }

    fn ty(&mut self, t: &T) {
        match t {
            T::Words(end) => self.words(*end),
            T::Nullable(inner) | T::Type(inner) | T::Table(inner) => {
                self.tok(); // nullable / type / table
                self.push(Cmd::Space);
                self.ty(inner);
            }
            T::Record(fields, open) => {
                let n = fields.len() + usize::from(*open);
                self.list(n, |s, i| match fields.get(i) {
                    Some(f) => s.field_spec(f),
                    None => s.tok(), // ...
                });
            }
            T::List(item) => {
                self.tok(); // {
                self.ty(item);
                self.tok(); // }
            }
            T::Function(params, ret) => {
                self.tok(); // function
                self.push(Cmd::Space);
                self.list(params.len(), |s, i| {
                    if let Some(p) = params.get(i) {
                        s.param(p);
                    }
                });
                if let Some(end) = ret {
                    self.push(Cmd::Space);
                    self.tok(); // as
                    self.push(Cmd::Space);
                    self.words(*end);
                }
            }
            T::Expr(e) => self.expr(e),
        }
    }

    fn field_spec(&mut self, f: &F) {
        if f.optional {
            self.tok();
            self.push(Cmd::Space);
        }
        self.name(f.name);
        if let Some(ty) = &f.ty {
            self.push(Cmd::Space);
            self.tok(); // =
            self.push(Cmd::Space);
            self.ty(ty);
        }
    }
}

// ---- rendering -------------------------------------------------------------

/// Width of anything that cannot be on one line.
const INF: usize = usize::MAX / 4;

fn has_line_break(s: &str) -> bool {
    s.contains(['\r', '\n', '\u{0085}', '\u{2028}', '\u{2029}'])
}

fn text_width(s: &str) -> usize {
    if has_line_break(s) { INF } else { s.chars().count() }
}

/// Width of the last line of `s` (the column after writing it at column 0 of a new line).
fn last_line_width(s: &str) -> usize {
    s.rsplit(['\r', '\n', '\u{0085}', '\u{2028}', '\u{2029}'])
        .next()
        .map_or(0, |l| l.chars().count())
}

fn first_line_width(s: &str) -> usize {
    s.split(['\r', '\n', '\u{0085}', '\u{2028}', '\u{2029}'])
        .next()
        .map_or(0, |l| l.chars().count())
}

/// Width of a command on a flat line.
fn flat_width(cmd: &Cmd<'_>) -> usize {
    match cmd {
        Cmd::Text(t) => text_width(t),
        Cmd::Space | Cmd::Line => 1,
        Cmd::Trailing(c) if !c.line => text_width(c.text).saturating_add(1),
        Cmd::HardLine | Cmd::Blank | Cmd::Leading(_) | Cmd::Trailing(_) => INF,
        Cmd::SoftLine | Cmd::GroupStart | Cmd::GroupEnd | Cmd::Indent | Cmd::Dedent => 0,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Break {
    None,
    Space,
    Newline,
    Blank,
}

struct Writer {
    out: String,
    col: usize,
    line_has_text: bool,
    level: usize,
    pending: Break,
    unit: String,
    indent_width: usize,
}

impl Writer {
    fn raise(&mut self, b: Break) {
        self.pending = self.pending.max(b);
    }

    /// The column the next text starts at.
    fn next_col(&self) -> usize {
        match self.pending {
            Break::None => self.col,
            Break::Space => self.col.saturating_add(usize::from(self.line_has_text)),
            Break::Newline | Break::Blank => self.level.saturating_mul(self.indent_width),
        }
    }

    fn write(&mut self, text: &str) {
        match self.pending {
            Break::None => {}
            Break::Space => {
                if self.line_has_text {
                    self.out.push(' ');
                    self.col = self.col.saturating_add(1);
                }
            }
            Break::Newline | Break::Blank => {
                if !self.out.is_empty() {
                    self.out.push('\n');
                    if self.pending == Break::Blank {
                        self.out.push('\n');
                    }
                    for _ in 0..self.level {
                        self.out.push_str(&self.unit);
                    }
                    self.col = self.level.saturating_mul(self.indent_width);
                }
            }
        }
        self.pending = Break::None;
        self.out.push_str(text);
        if has_line_break(text) {
            self.col = last_line_width(text);
        } else {
            self.col = self.col.saturating_add(text.chars().count());
        }
        self.line_has_text = true;
    }
}

/// Lays the commands out (Wadler-style: a group is flat when it and the text after it up to the
/// next possible line break fit in the width).
fn render(cmds: &[Cmd<'_>], opts: &FormatOptions) -> String {
    let n = cmds.len();
    // Flat width and end of every group (indexed by its GroupStart).
    let mut width = vec![0usize; n];
    let mut end = vec![0usize; n];
    let mut open: Vec<(usize, usize)> = Vec::new();
    for (i, cmd) in cmds.iter().enumerate() {
        match cmd {
            Cmd::GroupStart => open.push((i, 0)),
            Cmd::GroupEnd => {
                if let Some((start, w)) = open.pop() {
                    if let Some(slot) = width.get_mut(start) {
                        *slot = w;
                    }
                    if let Some(slot) = end.get_mut(start) {
                        *slot = i;
                    }
                    if let Some(parent) = open.last_mut() {
                        parent.1 = parent.1.saturating_add(w).min(INF);
                    }
                }
            }
            _ => {
                if let Some(parent) = open.last_mut() {
                    parent.1 = parent.1.saturating_add(flat_width(cmd)).min(INF);
                }
            }
        }
    }
    // Width of the text after each command up to the next possible break.
    let mut rest = vec![0usize; n];
    let mut acc = 0usize;
    for (i, cmd) in cmds.iter().enumerate().rev() {
        if let Some(slot) = rest.get_mut(i) {
            *slot = acc;
        }
        acc = match cmd {
            Cmd::Text(t) if has_line_break(t) => first_line_width(t),
            Cmd::Text(t) => acc.saturating_add(text_width(t)),
            Cmd::Space => acc.saturating_add(1),
            Cmd::Trailing(c) if !c.line => {
                if has_line_break(c.text) {
                    first_line_width(c.text).saturating_add(1)
                } else {
                    acc.saturating_add(text_width(c.text)).saturating_add(1)
                }
            }
            Cmd::Line | Cmd::SoftLine | Cmd::HardLine | Cmd::Blank | Cmd::Leading(_) | Cmd::Trailing(_) => 0,
            Cmd::GroupStart | Cmd::GroupEnd | Cmd::Indent | Cmd::Dedent => acc,
        }
        .min(INF);
    }

    let mut w = Writer {
        out: String::new(),
        col: 0,
        line_has_text: false,
        level: 0,
        pending: Break::None,
        unit: if opts.use_tabs {
            "\t".to_owned()
        } else {
            " ".repeat(opts.indent_width)
        },
        indent_width: opts.indent_width,
    };
    let mut flat: Vec<bool> = Vec::new();
    for (i, cmd) in cmds.iter().enumerate() {
        let in_flat = flat.last().copied().unwrap_or(false);
        match cmd {
            Cmd::GroupStart => {
                let fits = in_flat || {
                    let group = width.get(i).copied().unwrap_or(INF);
                    let after = end.get(i).and_then(|&e| rest.get(e)).copied().unwrap_or(0);
                    group < INF && w.next_col().saturating_add(group).saturating_add(after) <= opts.max_width
                };
                flat.push(fits);
            }
            Cmd::GroupEnd => {
                flat.pop();
            }
            Cmd::Indent => w.level = w.level.saturating_add(1),
            Cmd::Dedent => w.level = w.level.saturating_sub(1),
            Cmd::Space => w.raise(Break::Space),
            Cmd::Line => w.raise(if in_flat { Break::Space } else { Break::Newline }),
            Cmd::SoftLine => {
                if !in_flat {
                    w.raise(Break::Newline);
                }
            }
            Cmd::HardLine => w.raise(Break::Newline),
            Cmd::Blank => w.raise(Break::Blank),
            Cmd::Text(t) => w.write(t),
            Cmd::Trailing(c) => {
                w.raise(Break::Space);
                w.write(c.text);
                if c.line {
                    w.raise(Break::Newline);
                }
            }
            Cmd::Leading(c) => {
                if c.blank_before {
                    w.raise(Break::Blank);
                } else if w.line_has_text {
                    w.raise(Break::Newline);
                }
                w.write(c.text);
                w.raise(if c.newline_after {
                    Break::Newline
                } else {
                    Break::Space
                });
            }
        }
    }
    w.out
}

// ---- equivalence check -----------------------------------------------------

/// The output has the input's tokens and comments, in order and spelled the same (comments
/// without trailing blanks).
fn same_tokens(src: &str, toks: &[Token], out: &str, again: &[Token]) -> bool {
    let spelled = |text: &str, all: &[Token]| -> Vec<(Option<Tok>, String)> {
        items(text, all)
            .into_iter()
            .map(|(start, end, tok, _)| {
                let s = text.get(start..end).unwrap_or("");
                match tok.and_then(|k| all.get(k)) {
                    Some(t) => (Some(t.tok.clone()), s.to_owned()),
                    None => (None, s.trim_end().to_owned()),
                }
            })
            .collect()
    };
    spelled(src, toks) == spelled(out, again)
}
