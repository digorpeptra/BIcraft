//! DAX formatting (`dax.format`, the DAX query view's Format query): our own layout rules, only
//! white space changes. Tokens keep their text (names, quotes, literals, case) and comments are
//! kept where they are; the result is checked to lex to the same tokens and comments, and to
//! format to itself (idempotent).
//!
//! The layout depends only on the tokens, the comments and whether each comment starts its own
//! line, never on the original line breaks or spacing:
//!
//! - a query: `DEFINE`, `EVALUATE`, `ORDER BY` and `START AT` start a line at the left margin
//!   (a blank line before `DEFINE` and `EVALUATE` when something comes before them); each
//!   `MEASURE`, `COLUMN`, `TABLE`, `VAR` and `FUNCTION` definition starts a line one level in;
//!   a definition's expression stays on its line when it fits, else it goes on the next line
//!   two levels in; `EVALUATE`'s table expression and each `ORDER BY` key go on a line of their
//!   own one level in;
//! - an expression: a call `F(…)` or a table constructor `{…}` stays on one line when it fits in
//!   the width and holds no line comment and no `VAR`; otherwise each argument goes on a line of
//!   its own one level in, and the closing bracket on a line of its own; in a `VAR … RETURN`
//!   block each `VAR` and the `RETURN` start a line.
//!
//! The **formula** style ([`Style::Formula`], the formula bar's Format DAX) is BIcraft's own
//! layout following the DAX formatting conventions SQLBI describes publicly ("Rules for DAX code
//! formatting", sqlbi.com): the same breaks as above, plus the names of DAX functions and the
//! keywords in upper case, a space before every opening parenthesis and inside the brackets of
//! a call or a table constructor that holds something (`SUM ( Sales[Amount] )`, `{ 1, 2 }`), and
//! the expression after `RETURN` on a line of its own one level in. Only white space and the
//! case of function names and keywords change; names of tables, columns, measures and
//! variables stay as written.

use crate::lexer::{self, Token, TokenKind};
use crate::{Severity, Span};

/// The layout's flavour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Style {
    /// The DAX query view's Format query: `F(x)`, every token as written.
    #[default]
    Query,
    /// The formula bar's Format DAX (see the module docs).
    Formula,
}

/// How [`format`] lays the text out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatOptions {
    /// Lines are kept within this many characters where the tokens allow it.
    pub max_width: usize,
    /// Spaces per indentation level.
    pub indent: usize,
    pub style: Style,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            max_width: 100,
            indent: 4,
            style: Style::Query,
        }
    }
}

/// Longest text formatted.
pub const MAX_FORMAT_BYTES: usize = 4 * 1024 * 1024;
/// Deepest bracket nesting laid out (the parser stops well before).
const MAX_NESTING: usize = 512;

/// Words the layout or the spacing treats as keywords (a `(` after one gets a space).
const KEYWORDS: &[&str] = &[
    "DEFINE", "EVALUATE", "MEASURE", "COLUMN", "TABLE", "VAR", "RETURN", "ORDER", "BY", "START", "AT", "ASC",
    "DESC", "IN", "FUNCTION",
];

/// The definitions of a `DEFINE` block.
const DEFINITIONS: &[&str] = &["MEASURE", "COLUMN", "TABLE", "VAR", "FUNCTION"];

/// Formats a DAX query (text starting with `DEFINE` or `EVALUATE`) or a DAX expression. Errors
/// when the text does not parse; nothing is formatted then.
pub fn format(text: &str, opts: &FormatOptions) -> Result<String, String> {
    if text.len() > MAX_FORMAT_BYTES {
        return Err(format!("the text is longer than {MAX_FORMAT_BYTES} bytes"));
    }
    let opts = FormatOptions {
        max_width: opts.max_width.clamp(20, 1000),
        indent: opts.indent.min(16),
        style: opts.style,
    };
    let once = format_once(text, &opts)?;
    // the formula style upper-cases function names and keywords: names compared without case
    let fold = opts.style == Style::Formula;
    if signature(text, fold)? != signature(&once, fold)? {
        return Err(
            "internal error: formatting would change the tokens or comments; nothing was changed".into(),
        );
    }
    // idempotent by construction (the layout reads only tokens and comments); checked anyway
    let twice = format_once(&once, &opts)?;
    if twice != once {
        return Err(
            "internal error: the formatted text does not format to itself; nothing was changed".into(),
        );
    }
    Ok(once)
}

/// A comment, by its bytes in the text.
#[derive(Debug, Clone, Copy)]
struct Comment {
    span: Span,
    /// `//` or `--` (runs to the end of the line).
    line: bool,
    /// A line break comes before it (or it starts the text).
    own_line: bool,
}

/// What [`format`] compares before and after: each token's kind and text, each comment's text.
fn signature(text: &str, fold: bool) -> Result<(Vec<String>, Vec<String>), String> {
    let mut raw = Vec::new();
    let toks = lexer::lex_query(text, &mut raw);
    let coms = comments(text, &toks)?;
    let t = toks
        .iter()
        .map(|t| {
            let s = text.get(t.span.start..t.span.end).unwrap_or("");
            let s = if fold && matches!(t.kind, TokenKind::Ident(_)) {
                s.to_ascii_uppercase()
            } else {
                s.to_string()
            };
            format!("{:?}", std::mem::discriminant(&t.kind)) + &s
        })
        .collect();
    let c = coms
        .iter()
        .map(|c| text.get(c.span.start..c.span.end).unwrap_or("").to_string())
        .collect();
    Ok((t, c))
}

/// The comments between the tokens.
fn comments(text: &str, toks: &[Token]) -> Result<Vec<Comment>, String> {
    let mut out = Vec::new();
    let mut prev_end = 0usize;
    let mut first = true;
    for t in toks {
        let mut j = prev_end;
        let mut nl = first;
        while j < t.span.start {
            let rest = text.get(j..t.span.start).unwrap_or_default();
            let Some(c) = rest.chars().next() else { break };
            if c == '\n' {
                nl = true;
                j += 1;
            } else if c.is_ascii_whitespace() {
                j += 1;
            } else if rest.starts_with("//") || rest.starts_with("--") {
                let len = rest.find('\n').unwrap_or(rest.len());
                let body = rest.get(..len).unwrap_or_default().trim_end();
                out.push(Comment {
                    span: Span {
                        start: j,
                        end: j + body.len(),
                    },
                    line: true,
                    own_line: nl,
                });
                j += len;
                nl = false;
            } else if rest.starts_with("/*") {
                let len = rest
                    .get(2..)
                    .and_then(|r| r.find("*/"))
                    .map(|e| e + 4)
                    .ok_or("an unterminated /* comment")?;
                out.push(Comment {
                    span: Span {
                        start: j,
                        end: j + len,
                    },
                    line: false,
                    own_line: nl,
                });
                j += len;
                nl = false;
            } else {
                return Err(format!("unexpected character `{c}`"));
            }
        }
        prev_end = t.span.end;
        first = false;
    }
    Ok(out)
}

/// A token or a comment, in text order.
#[derive(Debug, Clone, Copy)]
enum Item {
    Tok(usize),
    Com(usize),
}

/// An expression as a tree of brackets.
#[derive(Debug)]
enum Node {
    Tok(usize),
    Com(usize),
    Group(Group),
}

#[derive(Debug)]
struct Group {
    open: usize,
    args: Vec<Vec<Node>>,
    /// The comma after each argument but the last.
    commas: Vec<usize>,
    close: Option<usize>,
}

fn is_open(k: &TokenKind) -> bool {
    matches!(k, TokenKind::LParen | TokenKind::LBrace)
}

fn is_close(k: &TokenKind) -> bool {
    matches!(k, TokenKind::RParen | TokenKind::RBrace)
}

/// The tree of `items` (brackets matched as the parser did).
fn tree(items: &[Item], toks: &[Token]) -> Result<Vec<Node>, String> {
    let mut at = 0usize;
    let mut out = Vec::new();
    while at < items.len() {
        let (node, next) = node(items, toks, at, 0)?;
        out.push(node);
        at = next;
    }
    Ok(out)
}

fn node(items: &[Item], toks: &[Token], at: usize, depth: usize) -> Result<(Node, usize), String> {
    let Some(item) = items.get(at).copied() else {
        return Err("internal error: an empty expression".into());
    };
    let i = match item {
        Item::Com(c) => return Ok((Node::Com(c), at + 1)),
        Item::Tok(i) => i,
    };
    if !toks.get(i).is_some_and(|t| is_open(&t.kind)) {
        return Ok((Node::Tok(i), at + 1));
    }
    if depth > MAX_NESTING {
        return Err(format!("the text nests deeper than {MAX_NESTING} brackets"));
    }
    let mut g = Group {
        open: i,
        args: Vec::new(),
        commas: Vec::new(),
        close: None,
    };
    let mut arg = Vec::new();
    let mut k = at + 1;
    while let Some(item) = items.get(k).copied() {
        if let Item::Tok(j) = item {
            let kind = toks.get(j).map(|t| &t.kind);
            if kind.is_some_and(is_close) {
                g.close = Some(j);
                k += 1;
                break;
            }
            if kind == Some(&TokenKind::Comma) {
                g.args.push(std::mem::take(&mut arg));
                g.commas.push(j);
                k += 1;
                continue;
            }
        }
        let (n, next) = node(items, toks, k, depth + 1)?;
        arg.push(n);
        k = next;
    }
    if !arg.is_empty() || !g.commas.is_empty() {
        g.args.push(arg);
    }
    Ok((Node::Group(g), k))
}

/// The layout writer.
struct W<'a> {
    src: &'a str,
    toks: &'a [Token],
    coms: &'a [Comment],
    opts: FormatOptions,
    nl: &'static str,
    out: String,
    /// Only measures (a flat trial): nothing is written.
    measure: bool,
    /// A flat trial met something that cannot be flat, or went past the width.
    failed: bool,
    col: usize,
    /// Only indentation on the current line.
    bol: bool,
    level: usize,
    last: Option<usize>,
    last_unary: bool,
    after_comment: bool,
    /// Line comments that end the current line, written at the next line break.
    pending: Vec<usize>,
    /// The next token starts a line (after a comment on a line of its own).
    fresh: bool,
}

impl<'a> W<'a> {
    fn text(&self, i: usize) -> &'a str {
        self.toks
            .get(i)
            .and_then(|t| self.src.get(t.span.start..t.span.end))
            .unwrap_or("")
    }

    fn kind(&self, i: usize) -> Option<&'a TokenKind> {
        self.toks.get(i).map(|t| &t.kind)
    }

    fn word(&self, i: usize) -> Option<String> {
        match self.kind(i) {
            Some(TokenKind::Ident(s)) => Some(s.to_ascii_uppercase()),
            _ => None,
        }
    }

    fn is_kw(&self, i: usize, w: &str) -> bool {
        self.word(i).is_some_and(|x| x == w)
    }

    fn keyword(&self, i: usize) -> bool {
        self.word(i).is_some_and(|x| KEYWORDS.contains(&x.as_str()))
    }

    fn formula(&self) -> bool {
        self.opts.style == Style::Formula
    }

    /// Whether the formula style writes token `i` in upper case: a keyword (not a table name
    /// followed by a column), or a DAX function's name before its `(`.
    fn upper(&self, i: usize) -> bool {
        let next = self.kind(i + 1);
        match self.kind(i) {
            Some(TokenKind::Ident(s)) => {
                if next == Some(&TokenKind::LParen) {
                    crate::functions::lookup(s).is_some()
                } else {
                    self.keyword(i) && !matches!(next, Some(TokenKind::Bracket(_)))
                }
            }
            _ => false,
        }
    }

    /// A trial copy at the same place, writing nothing.
    fn trial(&self) -> W<'a> {
        W {
            src: self.src,
            toks: self.toks,
            coms: self.coms,
            opts: self.opts,
            nl: self.nl,
            out: String::new(),
            measure: true,
            failed: !self.pending.is_empty() || self.fresh,
            col: self.col,
            bol: self.bol,
            level: self.level,
            last: self.last,
            last_unary: self.last_unary,
            after_comment: self.after_comment,
            pending: Vec::new(),
            fresh: false,
        }
    }

    fn push(&mut self, s: &str) {
        if !self.measure {
            self.out.push_str(s);
        }
        if s.contains('\n') {
            if self.measure {
                self.failed = true;
            }
            self.col = s.rsplit('\n').next().map_or(0, |l| l.chars().count());
        } else {
            self.col = self.col.saturating_add(s.chars().count());
        }
        if self.measure && self.col > self.opts.max_width {
            self.failed = true;
        }
    }

    fn space_before(&self, next: usize) -> bool {
        let Some(last) = self.last else { return false };
        if self.bol {
            return false;
        }
        // `- -1` written `--1` would start a comment
        if self.kind(last) == Some(&TokenKind::Op("-")) && self.kind(next) == Some(&TokenKind::Op("-")) {
            return true;
        }
        if self.last_unary {
            return false;
        }
        let (Some(lk), Some(nk)) = (self.kind(last), self.kind(next)) else {
            return true;
        };
        if self.after_comment {
            return !matches!(nk, TokenKind::Comma | TokenKind::RParen | TokenKind::RBrace);
        }
        if self.formula() {
            // `F ( a, b )`, `{ 1, 2 }`, `F ()`
            match (lk, nk) {
                (TokenKind::LParen | TokenKind::LBrace, TokenKind::RParen | TokenKind::RBrace) => {
                    return false;
                }
                (_, TokenKind::RParen | TokenKind::RBrace) => return true,
                (TokenKind::LParen | TokenKind::LBrace, _) => return true,
                (TokenKind::Op("."), _) => return false,
                (_, TokenKind::LParen) => return true,
                _ => {}
            }
        }
        match nk {
            TokenKind::Comma | TokenKind::RParen | TokenKind::RBrace => return false,
            TokenKind::Op(".") | TokenKind::Op(":") => return false,
            _ => {}
        }
        match lk {
            TokenKind::LParen | TokenKind::LBrace | TokenKind::Op(".") => return false,
            _ => {}
        }
        match nk {
            TokenKind::LParen => !matches!(lk, TokenKind::Ident(_)) || self.keyword(last),
            TokenKind::Bracket(_) => match lk {
                TokenKind::QuotedTable(_) => false,
                TokenKind::Ident(_) => self.keyword(last),
                _ => true,
            },
            _ => true,
        }
    }

    /// Whether token `i` (a `+` or `-`) is a sign rather than an operator.
    fn unary(&self, i: usize) -> bool {
        if !matches!(self.kind(i), Some(TokenKind::Op("-" | "+"))) {
            return false;
        }
        match self.last.and_then(|l| self.kind(l)) {
            None => true,
            Some(TokenKind::Op(_) | TokenKind::LParen | TokenKind::LBrace | TokenKind::Comma) => true,
            Some(TokenKind::Ident(_)) => self.last.is_some_and(|l| self.keyword(l)),
            _ => false,
        }
    }

    fn token(&mut self, i: usize) {
        let comma = self.kind(i) == Some(&TokenKind::Comma);
        if (!self.pending.is_empty() && !comma) || self.fresh {
            if self.measure {
                self.failed = true;
            }
            let level = self.level;
            self.newline(level);
        }
        if self.space_before(i) {
            self.push(" ");
        }
        let unary = self.unary(i);
        let t = self.text(i);
        if self.formula() && self.upper(i) {
            let up = t.to_ascii_uppercase();
            self.push(&up);
        } else {
            self.push(t);
        }
        self.bol = false;
        self.last = Some(i);
        self.last_unary = unary;
        self.after_comment = false;
    }

    fn comment(&mut self, c: usize) {
        let Some(com) = self.coms.get(c).copied() else {
            return;
        };
        let text = self.src.get(com.span.start..com.span.end).unwrap_or("");
        if self.measure && (com.own_line || com.line) {
            self.failed = true;
            return;
        }
        if com.own_line {
            if !self.out.is_empty() || !self.pending.is_empty() {
                let level = self.level;
                self.newline(level);
            }
            self.push(text);
            self.bol = false;
            self.fresh = true;
            self.after_comment = true;
            return;
        }
        if com.line {
            self.pending.push(c);
            return;
        }
        if !self.bol && !self.out.is_empty() {
            self.push(" ");
        }
        self.push(text);
        self.bol = false;
        self.after_comment = true;
    }

    /// Ends the line (writing the comments that end it) and indents the next one.
    fn newline(&mut self, level: usize) {
        for c in std::mem::take(&mut self.pending) {
            if let Some(com) = self.coms.get(c)
                && let Some(text) = self.src.get(com.span.start..com.span.end)
            {
                self.push(" ");
                self.push(text);
            }
        }
        let kept = self.out.trim_end_matches(' ').len();
        self.out.truncate(kept);
        if !self.out.is_empty() && !self.out.ends_with('\n') {
            self.out.push_str(self.nl);
        }
        let pad = level.saturating_mul(self.opts.indent);
        if !self.out.is_empty() {
            self.out.extend(std::iter::repeat_n(' ', pad));
            self.col = pad;
        } else {
            self.col = 0;
        }
        self.bol = true;
        self.fresh = false;
        self.level = level;
    }

    /// A blank line, then the left margin.
    fn blank(&mut self) {
        self.newline(0);
        if !self.out.is_empty() && !self.out.ends_with(&format!("{}{}", self.nl, self.nl)) {
            self.out.push_str(self.nl);
        }
    }

    /// `nodes` on one line; false (in a trial) when they cannot be.
    fn flat(&mut self, nodes: &[Node]) -> bool {
        for n in nodes {
            if self.failed {
                return false;
            }
            match n {
                Node::Tok(i) => {
                    if self.is_kw(*i, "VAR") {
                        self.failed = true;
                        return false;
                    }
                    self.token(*i);
                }
                Node::Com(c) => self.comment(*c),
                Node::Group(g) => {
                    self.flat_group(g);
                }
            }
        }
        !self.failed
    }

    fn flat_group(&mut self, g: &Group) -> bool {
        self.token(g.open);
        for (k, arg) in g.args.iter().enumerate() {
            if !self.flat(arg) {
                return false;
            }
            if let Some(c) = g.commas.get(k) {
                self.token(*c);
            }
        }
        if let Some(c) = g.close {
            self.token(c);
        }
        !self.failed
    }

    /// `nodes` at `level`, broken where they do not fit.
    fn seq(&mut self, nodes: &[Node], level: usize) {
        let block = nodes
            .iter()
            .any(|n| matches!(n, Node::Tok(i) if self.is_kw(*i, "VAR")));
        let mut first = true;
        // the formula style: what follows RETURN, one level in on a line of its own
        let mut lvl = level;
        let mut after_return = false;
        for n in nodes {
            self.level = lvl;
            if after_return && !matches!(n, Node::Com(_)) {
                after_return = false;
                lvl = level + 1;
                self.newline(lvl);
            }
            match n {
                Node::Tok(i) => {
                    if block && !first && (self.is_kw(*i, "VAR") || self.is_kw(*i, "RETURN")) {
                        lvl = level;
                        self.newline(level);
                    }
                    self.token(*i);
                    after_return = block && self.formula() && self.is_kw(*i, "RETURN");
                    first = false;
                }
                Node::Com(c) => self.comment(*c),
                Node::Group(g) => {
                    self.group(g, lvl);
                    first = false;
                }
            }
        }
    }

    fn group(&mut self, g: &Group, level: usize) {
        let mut t = self.trial();
        if t.flat_group(g) {
            self.flat_group(g);
            return;
        }
        self.token(g.open);
        if g.args.is_empty() {
            if let Some(c) = g.close {
                self.token(c);
            }
            return;
        }
        for (k, arg) in g.args.iter().enumerate() {
            // comments right after the bracket or the comma stay on its line (one at the start of
            // the next line would start its line when the result is read again)
            let lead = arg
                .iter()
                .take_while(|n| matches!(n, Node::Com(c) if self.coms.get(*c).is_some_and(|x| !x.own_line)))
                .count();
            self.level = level + 1;
            for n in arg.iter().take(lead) {
                if let Node::Com(c) = n {
                    self.comment(*c);
                }
            }
            self.newline(level + 1);
            self.seq(arg.get(lead..).unwrap_or_default(), level + 1);
            if let Some(c) = g.commas.get(k) {
                self.token(*c);
            }
        }
        self.newline(level);
        if let Some(c) = g.close {
            self.token(c);
        }
    }
}

/// Which part of a query an item belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    /// Keywords, names and punctuation around the expressions.
    Skeleton,
    /// The expression of definition `n`, the table expression of an `EVALUATE`, an `ORDER BY` key.
    Define(usize),
    Evaluate(usize),
    Order(usize),
    /// The whole text is one expression.
    Expression,
}

fn format_once(text: &str, opts: &FormatOptions) -> Result<String, String> {
    let mut raw = Vec::new();
    let toks = lexer::lex_query(text, &mut raw);
    let coms = comments(text, &toks)?;
    let real = toks.len().saturating_sub(1);
    if real == 0 {
        // only comments (or nothing): left as they are
        return Ok(text.to_string());
    }
    let first_word = match toks.first().map(|t| &t.kind) {
        Some(TokenKind::Ident(s)) => s.to_ascii_uppercase(),
        _ => String::new(),
    };
    let query = first_word == "DEFINE" || first_word == "EVALUATE";
    // what each token belongs to
    let mut part = vec![Part::Expression; real];
    if query {
        let (q, diags) = crate::parse_query(text);
        let q = match q {
            Some(q) if diags.iter().all(|d| d.severity != Severity::Error) => q,
            _ => return Err(first_error(&diags, "the query does not parse")),
        };
        let mut spans = Vec::new();
        for (n, d) in q.defines.iter().enumerate() {
            let s = match d {
                crate::ast::Define::Measure { expr_span, .. }
                | crate::ast::Define::Column { expr_span, .. }
                | crate::ast::Define::Var { expr_span, .. }
                | crate::ast::Define::Table { expr_span, .. } => *expr_span,
                crate::ast::Define::Function { body_span, .. } => *body_span,
            };
            spans.push((s, Part::Define(n)));
        }
        let mut keys = 0usize;
        for (n, st) in q.statements.iter().enumerate() {
            spans.push((st.expr_span, Part::Evaluate(n)));
            for k in &st.order_by {
                spans.push((k.expr_span, Part::Order(keys)));
                keys += 1;
            }
        }
        for (i, t) in toks.iter().take(real).enumerate() {
            let p = spans
                .iter()
                .find(|(s, _)| t.span.start >= s.start && t.span.end <= s.end)
                .map_or(Part::Skeleton, |(_, p)| *p);
            if let Some(slot) = part.get_mut(i) {
                *slot = p;
            }
        }
    } else {
        let (e, diags) = crate::parse_expression(text);
        if e.is_none() || diags.iter().any(|d| d.severity == Severity::Error) {
            return Err(first_error(&diags, "the expression does not parse"));
        }
    }
    // tokens and comments in order, each comment with the part of the tokens around it
    let mut items: Vec<(Item, Part)> = Vec::with_capacity(real + coms.len());
    let mut c = 0usize;
    for i in 0..=real {
        let start = toks.get(i).map_or(text.len(), |t| t.span.start);
        while let Some(com) = coms.get(c)
            && com.span.start < start
        {
            let before = i.checked_sub(1).and_then(|p| part.get(p)).copied();
            let after = part.get(i).copied();
            let p = match (before, after) {
                (Some(a), Some(b)) if a == b => a,
                (None, Some(Part::Expression)) | (Some(Part::Expression), None) => Part::Expression,
                _ => Part::Skeleton,
            };
            items.push((Item::Com(c), p));
            c += 1;
        }
        if i < real {
            items.push((Item::Tok(i), part.get(i).copied().unwrap_or(Part::Skeleton)));
        }
    }
    let mut w = W {
        src: text,
        toks: &toks,
        coms: &coms,
        opts: *opts,
        nl: if text.contains("\r\n") { "\r\n" } else { "\n" },
        out: String::with_capacity(text.len() + text.len() / 4),
        measure: false,
        failed: false,
        col: 0,
        bol: true,
        level: 0,
        last: None,
        last_unary: false,
        after_comment: false,
        pending: Vec::new(),
        fresh: false,
    };
    if query {
        layout_query(&mut w, &items)?;
    } else {
        let only: Vec<Item> = items.iter().map(|(it, _)| *it).collect();
        let nodes = tree(&only, &toks)?;
        w.seq(&nodes, 0);
    }
    let level = w.level;
    w.newline(level);
    let kept = w.out.trim_end().len();
    w.out.truncate(kept);
    Ok(w.out)
}

fn first_error(diags: &[crate::Diagnostic], what: &str) -> String {
    match diags.iter().find(|d| d.severity == Severity::Error) {
        Some(d) => format!(
            "{what} (line {}, column {}: {}); nothing was changed",
            d.line, d.column, d.message
        ),
        None => format!("{what}; nothing was changed"),
    }
}

fn layout_query(w: &mut W<'_>, items: &[(Item, Part)]) -> Result<(), String> {
    let toks = w.toks;
    let mut at = 0usize;
    let mut in_define = false;
    let mut depth = 0usize;
    while let Some((item, part)) = items.get(at).copied() {
        if part != Part::Skeleton {
            let end = items
                .iter()
                .skip(at)
                .position(|(_, p)| *p != part)
                .map_or(items.len(), |n| at + n);
            let run: Vec<Item> = items
                .get(at..end)
                .unwrap_or_default()
                .iter()
                .map(|(it, _)| *it)
                .collect();
            let nodes = tree(&run, toks)?;
            match part {
                Part::Define(_) => {
                    let starts_var = matches!(nodes.first(), Some(Node::Tok(i)) if w.is_kw(*i, "VAR"));
                    w.level = 2;
                    let mut t = w.trial();
                    if !starts_var && t.flat(&nodes) {
                        w.flat(&nodes);
                    } else {
                        w.newline(2);
                        w.seq(&nodes, 2);
                    }
                }
                _ => {
                    w.newline(1);
                    w.seq(&nodes, 1);
                }
            }
            at = end;
            continue;
        }
        match item {
            Item::Com(c) => w.comment(c),
            Item::Tok(i) => {
                let word = w.word(i).unwrap_or_default();
                match toks.get(i).map(|t| &t.kind) {
                    Some(k) if is_open(k) => depth += 1,
                    Some(k) if is_close(k) => depth = depth.saturating_sub(1),
                    _ => {}
                }
                match word.as_str() {
                    "DEFINE" | "EVALUATE" if depth == 0 => {
                        in_define = word == "DEFINE";
                        if w.last.is_some() {
                            w.blank();
                        } else {
                            w.newline(0);
                        }
                    }
                    "ORDER" | "START" if depth == 0 => w.newline(0),
                    d if depth == 0 && in_define && DEFINITIONS.contains(&d) => w.newline(1),
                    _ => {}
                }
                w.token(i);
            }
        }
        at += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(s: &str) -> String {
        format(s, &FormatOptions::default()).unwrap()
    }

    #[test]
    fn short_calls_stay_on_one_line() {
        assert_eq!(fmt("SUM ( 'Sales'[Amount] )"), "SUM('Sales'[Amount])");
        assert_eq!(
            fmt("CALCULATE([Total],Sales[Year]=2024)"),
            "CALCULATE([Total], Sales[Year] = 2024)"
        );
        assert_eq!(fmt("-1 + - [x] * 2"), "-1 + -[x] * 2");
        // found by the fuzz smoke test: `--` would start a comment
        assert_eq!(fmt("- -1"), "- -1");
        assert_eq!(fmt("1 - - - 2"), "1 - - -2");
        assert_eq!(fmt("x IN {1,2}"), "x IN {1, 2}");
        assert_eq!(fmt("T[Date].[Year]"), "T[Date].[Year]");
        assert_eq!(fmt("NOT ( ISBLANK ( [m] ) )"), "NOT(ISBLANK([m]))");
    }

    fn formula(s: &str, width: usize) -> String {
        format(
            s,
            &FormatOptions {
                max_width: width,
                style: Style::Formula,
                ..FormatOptions::default()
            },
        )
        .unwrap()
    }

    /// The formula bar's Format DAX: upper-case functions and keywords, spaces around the
    /// brackets, RETURN's expression one level in (BIcraft's style after SQLBI's public rules).
    #[test]
    fn the_formula_style_follows_the_public_conventions() {
        assert_eq!(formula("sum('Sales'[Amount])", 100), "SUM ( 'Sales'[Amount] )");
        assert_eq!(
            formula("calculate([Total],Sales[Year]=2024, all(Dates))", 100),
            "CALCULATE ( [Total], Sales[Year] = 2024, ALL ( Dates ) )"
        );
        assert_eq!(formula("today()", 100), "TODAY ()");
        assert_eq!(formula("x in {1,2}", 100), "x IN { 1, 2 }");
        assert_eq!(formula("-(1+2)", 100), "-( 1 + 2 )");
        // names stay as written; a UDF keeps its case
        assert_eq!(
            formula("myFunc(Sales[Qty]) + var_x", 100),
            "myFunc ( Sales[Qty] ) + var_x"
        );
        assert_eq!(
            formula(
                "var a = sumx(Sales, Sales[Q] * Sales[P]) var b = if(a > 0, a, 0) return a + b",
                100
            ),
            "VAR a = SUMX ( Sales, Sales[Q] * Sales[P] )\nVAR b = IF ( a > 0, a, 0 )\nRETURN\n    a + b"
        );
        assert_eq!(
            formula(
                "CALCULATE(SUMX(FILTER(Sales, Sales[Quantity] > 10), Sales[Quantity] * Sales[Price]), ALL(Dates))",
                60
            ),
            "CALCULATE (\n    SUMX (\n        FILTER ( Sales, Sales[Quantity] > 10 ),\n        Sales[Quantity] * Sales[Price]\n    ),\n    ALL ( Dates )\n)"
        );
        assert_eq!(
            formula("divide(VAR a = 1 RETURN a, 2)", 100),
            "DIVIDE (\n    VAR a = 1\n    RETURN\n        a,\n    2\n)"
        );
        // comments kept; the result formats to itself
        let c = "var a = 1 // one\nreturn /* the end */ a";
        let once = formula(c, 100);
        assert_eq!(once, "VAR a = 1 // one\nRETURN /* the end */\n    a");
        assert_eq!(formula(&once, 100), once);
        // the query style is unchanged
        assert_eq!(fmt("sum('Sales'[Amount])"), "sum('Sales'[Amount])");
    }

    #[test]
    fn a_query_is_laid_out_by_its_keywords() {
        let q = "define measure 'Sales'[Total] = SUM(Sales[Amount]) var x = 1 evaluate SUMMARIZECOLUMNS(Sales[Year], \"Total\", [Total]) order by Sales[Year] desc, [Total] start at 2020";
        assert_eq!(
            fmt(q),
            "define\n    measure 'Sales'[Total] = SUM(Sales[Amount])\n    var x = 1\n\nevaluate\n    SUMMARIZECOLUMNS(Sales[Year], \"Total\", [Total])\norder by\n    Sales[Year] desc,\n    [Total]\nstart at 2020"
        );
    }

    #[test]
    fn long_calls_and_var_blocks_break() {
        let e = "CALCULATE(SUMX(FILTER(Sales, Sales[Quantity] > 10 && Sales[Price] > 100), Sales[Quantity] * Sales[Price]), ALL(Dates))";
        assert_eq!(
            fmt(e),
            "CALCULATE(\n    SUMX(FILTER(Sales, Sales[Quantity] > 10 && Sales[Price] > 100), Sales[Quantity] * Sales[Price]),\n    ALL(Dates)\n)"
        );
        let v = "VAR a = 1 VAR b = IF(a > 0, a, 0) RETURN a + b";
        assert_eq!(fmt(v), "VAR a = 1\nVAR b = IF(a > 0, a, 0)\nRETURN a + b");
        let q = "EVALUATE ROW(\"x\", VAR a = 1 RETURN a)";
        assert_eq!(
            fmt(q),
            "EVALUATE\n    ROW(\n        \"x\",\n        VAR a = 1\n        RETURN a\n    )"
        );
        let m = "DEFINE MEASURE T[m] = VAR a = 1 RETURN a EVALUATE {[m]}";
        assert_eq!(
            fmt(m),
            "DEFINE\n    MEASURE T[m] =\n        VAR a = 1\n        RETURN a\n\nEVALUATE\n    {[m]}"
        );
    }

    #[test]
    fn comments_are_kept_in_place() {
        let q = "// Welcome\n// second line\nEVALUATE // the table\n  FILTER(Sales, /* big */ Sales[Q] > 1 -- more\n  , TRUE())";
        let out = fmt(q);
        assert_eq!(
            out,
            "// Welcome\n// second line\nEVALUATE // the table\n    FILTER(\n        Sales, /* big */\n        Sales[Q] > 1, -- more\n        TRUE()\n    )"
        );
        assert_eq!(fmt(&out), out);
        assert_eq!(fmt("// only a comment\n"), "// only a comment\n");
    }

    #[test]
    fn crlf_text_stays_crlf_and_texts_keep_their_bytes() {
        let q = "EVALUATE\r\n{ \"a  b\" , 'My  Table'[c d] }";
        assert_eq!(fmt(q), "EVALUATE\r\n    {\"a  b\", 'My  Table'[c d]}");
    }

    #[test]
    fn formatting_is_idempotent_over_a_corpus() {
        let corpus = [
            "EVALUATE 'Sales'",
            "DEFINE VAR x = 1 TABLE t = {1,2,3} COLUMN Sales[c] = Sales[a] * 2 MEASURE Sales[m] = SUM(Sales[c]) EVALUATE t EVALUATE ROW(\"m\", [m])",
            "DEFINE FUNCTION f = (a : SCALAR, b) => a + b EVALUATE {f(1, 2)}",
            "EVALUATE TOPN(10, SUMMARIZECOLUMNS('Date'[Year], 'Product'[Category], \"Sales\", [Sales Amount], \"Margin\", [Margin %]), [Sales], DESC) ORDER BY [Sales] DESC START AT 5",
            "IF(ISBLANK([a]), BLANK(), DIVIDE([a], [b], 0) * 100 /* pct */)",
            "SWITCH(TRUE(), [x] < 0, \"neg\", [x] = 0, \"zero\", -- zero\n \"pos\")",
            "VAR t = FILTER(ALL(Sales), Sales[Q] > 0) VAR n = COUNTROWS(t) RETURN IF(n > 0, SUMX(t, Sales[Q] * RELATED(Product[Price])) / n)",
            "{(1, \"a\"), (2, \"b\")}",
            "EVALUATE\n// c1\n\n/* c2 */\nVALUES(T[c]) // end",
        ];
        for q in corpus {
            let once = fmt(q);
            assert_eq!(fmt(&once), once, "{q}");
            for w in [20, 40, 60] {
                for style in [Style::Query, Style::Formula] {
                    let o = FormatOptions {
                        max_width: w,
                        indent: 2,
                        style,
                    };
                    let a = format(q, &o).unwrap();
                    assert_eq!(format(&a, &o).unwrap(), a, "{q} at width {w}, {style:?}");
                }
            }
        }
    }

    #[test]
    fn text_that_does_not_parse_is_refused() {
        assert!(format("EVALUATE SUM(", &FormatOptions::default()).is_err());
        assert!(format("1 +", &FormatOptions::default()).is_err());
        assert!(format("\"open", &FormatOptions::default()).is_err());
        assert_eq!(fmt("   "), "   ");
    }

    #[test]
    fn deep_nesting_is_bounded() {
        let deep = format!("{}1{}", "(".repeat(200), ")".repeat(200));
        assert!(format(&deep, &FormatOptions::default()).is_err());
        let ok = format!("{}1{}", "(".repeat(100), ")".repeat(100));
        assert!(format(&ok, &FormatOptions::default()).is_ok());
    }
}
