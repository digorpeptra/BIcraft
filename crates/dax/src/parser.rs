//! Precedence-climbing DAX parser. Lowest to highest:
//! `||`, `&&`, `NOT`, comparisons and `IN`, `&`, `+ -`, `* /`, unary sign, `^`.
//! Parsing stops at the first syntax error (later errors would mostly be noise).

use crate::ast::{
    Binding, Define, EvaluateStatement, Expr, FunctionParam, OrderKey, Query, TypeHint, Variation,
};
use crate::functions::is_keyword_argument;
use crate::lexer::{Token, TokenKind};
use crate::reserved::{is_reserved, is_reserved_bare, is_reserved_name};
use crate::{Raw, Span};

/// Nesting limit. Input is hostile: deep nesting must produce a diagnostic, not a stack overflow.
pub const MAX_DEPTH: usize = 128;

pub(crate) fn parse(tokens: &[Token], diags: &mut Vec<Raw>) -> Option<Expr> {
    // Lexer errors already explain the problem; parsing broken tokens adds only noise.
    if diags.iter().any(|d| d.severity == crate::Severity::Error) {
        return None;
    }
    let mut p = Parser {
        tokens,
        pos: 0,
        depth: 0,
        query: false,
        diags,
    };
    if matches!(p.peek(), TokenKind::Eof) {
        let span = p.span();
        p.diags
            .push(Raw::error("empty-expression", span, "the expression is empty"));
        return None;
    }
    let expr = p.expr()?;
    if !matches!(p.peek(), TokenKind::Eof) {
        let span = p.span();
        let found = describe(p.peek());
        p.diags.push(Raw::error(
            "unexpected-token",
            span,
            format!("unexpected {found} after the end of the expression"),
        ));
        return None;
    }
    Some(expr)
}

struct Parser<'t, 'd> {
    tokens: &'t [Token],
    pos: usize,
    depth: usize,
    /// Parsing a DAX query: query keywords cannot start an expression and `@param` is a value.
    query: bool,
    diags: &'d mut Vec<Raw>,
}

static EOF: TokenKind = TokenKind::Eof;

/// Where an unquoted name appears, for the reserved-word check.
#[derive(Clone, Copy)]
enum Place {
    /// `Name[Column]`.
    Qualifier,
    /// The table of a query definition: `DEFINE TABLE Name = …`, `MEASURE Name[m] = …`.
    TableName,
    /// A bare table reference: `COUNTROWS(Name)`.
    BareTable,
    /// A variable name; the text says which kind.
    Name(&'static str),
}

impl Parser<'_, '_> {
    fn peek(&self) -> &TokenKind {
        self.tokens.get(self.pos).map_or(&EOF, |t| &t.kind)
    }

    fn peek_at(&self, n: usize) -> &TokenKind {
        self.tokens
            .get(self.pos.saturating_add(n))
            .map_or(&EOF, |t| &t.kind)
    }

    fn span(&self) -> Span {
        self.tokens
            .get(self.pos)
            .or(self.tokens.last())
            .map_or(Span::default(), |t| t.span)
    }

    fn bump(&mut self) -> Span {
        let s = self.span();
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        s
    }

    fn is_keyword(&self, kw: &str) -> bool {
        matches!(self.peek(), TokenKind::Ident(s) if s.eq_ignore_ascii_case(kw))
    }

    fn is_op(&self, op: &str) -> bool {
        matches!(self.peek(), TokenKind::Op(o) if *o == op)
    }

    /// A reserved word used unquoted where the engine refuses it: a syntax error for the engine.
    /// Reported without stopping, so the rest of the expression is still checked.
    fn check_reserved(&mut self, name: &str, span: Span, place: Place) {
        let (reserved, role, hint) = match place {
            Place::Qualifier => (
                is_reserved(name),
                "qualify a column without quotes",
                format!(" (quote the table name: '{name}'[…])"),
            ),
            Place::TableName => (
                is_reserved(name),
                "name a table without quotes",
                format!(" (quote it: '{name}')"),
            ),
            Place::BareTable => (
                is_reserved_bare(name) && !is_keyword_argument(name),
                "be an unquoted table reference",
                format!(" (quote the table name: '{name}')"),
            ),
            Place::Name(role) => (is_reserved_name(name), role, String::new()),
        };
        if reserved {
            self.diags.push(Raw::error(
                "reserved-word",
                span,
                format!("`{name}` is a reserved word in DAX and cannot {role}{hint}"),
            ));
        }
    }

    fn fail<T>(&mut self, expected: &str) -> Option<T> {
        let span = self.span();
        let found = describe(self.peek());
        self.diags.push(Raw::error(
            "syntax",
            span,
            format!("expected {expected}, found {found}"),
        ));
        None
    }

    fn expect(&mut self, kind: &TokenKind, what: &str) -> Option<Span> {
        if self.peek() == kind {
            Some(self.bump())
        } else {
            self.fail(what)
        }
    }

    fn enter(&mut self) -> Option<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            let span = self.span();
            self.diags.push(Raw::error(
                "too-deep",
                span,
                format!("expression nests deeper than {MAX_DEPTH} levels"),
            ));
            return None;
        }
        Some(())
    }

    fn expr(&mut self) -> Option<Expr> {
        self.enter()?;
        let e = self.or();
        self.depth -= 1;
        e
    }

    fn binary(lhs: Expr, op: &str, rhs: Expr) -> Expr {
        let span = Span {
            start: lhs.span().start,
            end: rhs.span().end,
        };
        Expr::Binary {
            op: op.to_string(),
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
            span,
        }
    }

    /// One left-associative precedence level. Each operator in a chain counts toward
    /// [`MAX_DEPTH`]: `1+1+…+1` builds a tree as deep as it is long.
    fn left_assoc(
        &mut self,
        next: fn(&mut Self) -> Option<Expr>,
        op_of: fn(&TokenKind) -> Option<&'static str>,
    ) -> Option<Expr> {
        let mut lhs = next(self)?;
        let mut chain = 0;
        while let Some(op) = op_of(self.peek()) {
            self.bump();
            self.enter()?;
            chain += 1;
            let rhs = next(self)?;
            lhs = Self::binary(lhs, op, rhs);
        }
        self.depth -= chain;
        Some(lhs)
    }

    fn or(&mut self) -> Option<Expr> {
        self.left_assoc(Self::and, |t| matches!(t, TokenKind::Op("||")).then_some("||"))
    }

    fn and(&mut self) -> Option<Expr> {
        self.left_assoc(Self::not, |t| matches!(t, TokenKind::Op("&&")).then_some("&&"))
    }

    fn not(&mut self) -> Option<Expr> {
        // `NOT x` is an operator; `NOT(x)` is the same thing written as a call.
        if self.is_keyword("NOT") && !matches!(self.peek_at(1), TokenKind::LParen) {
            let start = self.bump().start;
            self.enter()?;
            let operand = self.not();
            self.depth -= 1;
            let operand = operand?;
            let span = Span {
                start,
                end: operand.span().end,
            };
            return Some(Expr::Unary {
                op: "NOT".into(),
                operand: Box::new(operand),
                span,
            });
        }
        self.comparison()
    }

    fn comparison(&mut self) -> Option<Expr> {
        self.left_assoc(Self::concat, |t| match t {
            TokenKind::Op(o @ ("=" | "==" | "<>" | "<" | ">" | "<=" | ">=")) => Some(*o),
            TokenKind::Ident(s) if s.eq_ignore_ascii_case("IN") => Some("IN"),
            _ => None,
        })
    }

    fn concat(&mut self) -> Option<Expr> {
        self.left_assoc(Self::additive, |t| matches!(t, TokenKind::Op("&")).then_some("&"))
    }

    fn additive(&mut self) -> Option<Expr> {
        self.left_assoc(Self::multiplicative, |t| match t {
            TokenKind::Op(o @ ("+" | "-")) => Some(*o),
            _ => None,
        })
    }

    fn multiplicative(&mut self) -> Option<Expr> {
        self.left_assoc(Self::sign, |t| match t {
            TokenKind::Op(o @ ("*" | "/")) => Some(*o),
            _ => None,
        })
    }

    fn sign(&mut self) -> Option<Expr> {
        if let TokenKind::Op(o @ ("+" | "-")) = self.peek() {
            let op = *o;
            let start = self.bump().start;
            self.enter()?;
            let operand = self.sign();
            self.depth -= 1;
            let operand = operand?;
            let span = Span {
                start,
                end: operand.span().end,
            };
            return Some(Expr::Unary {
                op: op.into(),
                operand: Box::new(operand),
                span,
            });
        }
        self.power()
    }

    fn power(&mut self) -> Option<Expr> {
        self.left_assoc(Self::primary, |t| matches!(t, TokenKind::Op("^")).then_some("^"))
    }

    fn primary(&mut self) -> Option<Expr> {
        let tok = self.tokens.get(self.pos).cloned();
        let Some(Token { kind, span }) = tok else {
            return self.fail("an expression");
        };
        match kind {
            TokenKind::Number(text) => {
                self.bump();
                Some(Expr::Number { text, span })
            }
            TokenKind::Str(value) => {
                self.bump();
                Some(Expr::Str { value, span })
            }
            TokenKind::Bracket(name) => {
                self.bump();
                self.variation(Expr::Column {
                    table: None,
                    table_span: None,
                    name,
                    variation: None,
                    span,
                })
            }
            TokenKind::QuotedTable(name) => {
                self.bump();
                let e = self.after_table_name(name, true, span);
                self.variation(e)
            }
            TokenKind::LParen => self.paren(),
            TokenKind::LBrace => self.table_ctor(),
            TokenKind::Param(name) => {
                self.bump();
                Some(Expr::Name {
                    name: format!("@{name}"),
                    quoted: false,
                    span,
                })
            }
            TokenKind::Ident(name) => {
                if name.eq_ignore_ascii_case("VAR") {
                    return self.var_block();
                }
                if self.query
                    && is_query_keyword(&name)
                    && !matches!(self.peek_at(1), TokenKind::LParen | TokenKind::Bracket(_))
                {
                    return self.fail("an expression");
                }
                self.bump();
                if matches!(self.peek(), TokenKind::LParen) {
                    return self.call(name, span);
                }
                if name.eq_ignore_ascii_case("TRUE") || name.eq_ignore_ascii_case("FALSE") {
                    return Some(Expr::Bool {
                        value: name.eq_ignore_ascii_case("TRUE"),
                        span,
                    });
                }
                if name.eq_ignore_ascii_case("RETURN") {
                    self.pos -= 1;
                    return self.fail("an expression");
                }
                let e = self.after_table_name(name, false, span);
                self.variation(e)
            }
            _ => self.fail("an expression"),
        }
    }

    /// `.[Name]` after a column reference: a column variation (`'Sales'[OrderDate].[Year]`).
    /// Only one level: `T[C].[A].[B]` is a syntax error, as in the engine.
    fn variation(&mut self, e: Expr) -> Option<Expr> {
        if !self.is_op(".") {
            return Some(e);
        }
        let Expr::Column {
            table,
            table_span,
            name,
            variation: None,
            span,
        } = e
        else {
            return self.fail("an operator or the end of the expression");
        };
        self.bump(); // .
        let Some(Token {
            kind: TokenKind::Bracket(vname),
            span: vspan,
        }) = self.tokens.get(self.pos).cloned()
        else {
            return self.fail("[variation name] after `.`");
        };
        self.bump();
        Some(Expr::Column {
            table,
            table_span,
            name,
            variation: Some(Box::new(Variation {
                name: vname,
                span: vspan,
            })),
            span: Span {
                start: span.start,
                end: vspan.end,
            },
        })
    }

    fn after_table_name(&mut self, name: String, quoted: bool, span: Span) -> Expr {
        if let Some(Token {
            kind: TokenKind::Bracket(col),
            span: col_span,
        }) = self.tokens.get(self.pos).cloned()
        {
            if !quoted {
                self.check_reserved(&name, span, Place::Qualifier);
            }
            self.bump();
            return Expr::Column {
                table: Some(name),
                table_span: Some(span),
                name: col,
                variation: None,
                span: Span {
                    start: span.start,
                    end: col_span.end,
                },
            };
        }
        if !quoted {
            self.check_reserved(&name, span, Place::BareTable);
        }
        Expr::Name { name, quoted, span }
    }

    fn call(&mut self, name: String, name_span: Span) -> Option<Expr> {
        self.bump(); // (
        let mut args = Vec::new();
        if !matches!(self.peek(), TokenKind::RParen) {
            loop {
                if matches!(self.peek(), TokenKind::Comma | TokenKind::RParen) {
                    args.push(None);
                } else {
                    args.push(Some(self.expr()?));
                }
                if matches!(self.peek(), TokenKind::Comma) {
                    self.bump();
                    continue;
                }
                break;
            }
        }
        let end = self
            .expect(&TokenKind::RParen, &format!("`,` or `)` to close {name}("))?
            .end;
        Some(Expr::Call {
            name,
            name_span,
            args,
            span: Span {
                start: name_span.start,
                end,
            },
        })
    }

    fn paren(&mut self) -> Option<Expr> {
        let start = self.bump().start;
        let mut items = vec![self.expr()?];
        while matches!(self.peek(), TokenKind::Comma) {
            self.bump();
            items.push(self.expr()?);
        }
        let end = self.expect(&TokenKind::RParen, "`)`")?.end;
        if items.len() == 1 {
            items.pop()
        } else {
            Some(Expr::Tuple {
                items,
                span: Span { start, end },
            })
        }
    }

    fn table_ctor(&mut self) -> Option<Expr> {
        let start = self.bump().start;
        let mut rows = Vec::new();
        if !matches!(self.peek(), TokenKind::RBrace) {
            rows.push(self.expr()?);
            while matches!(self.peek(), TokenKind::Comma) {
                self.bump();
                rows.push(self.expr()?);
            }
        }
        let end = self.expect(&TokenKind::RBrace, "`,` or `}`")?.end;
        Some(Expr::TableCtor {
            rows,
            span: Span { start, end },
        })
    }

    fn var_block(&mut self) -> Option<Expr> {
        let start = self.span().start;
        let mut bindings = Vec::new();
        while self.is_keyword("VAR") {
            self.bump();
            let (name, name_span) = match self.tokens.get(self.pos).cloned() {
                Some(Token {
                    kind: TokenKind::Ident(n),
                    span,
                }) => {
                    self.bump();
                    (n, span)
                }
                _ => return self.fail("a variable name after VAR"),
            };
            self.check_reserved(&name, name_span, Place::Name("name a variable"));
            if !self.is_op("=") {
                return self.fail("`=` after the variable name");
            }
            self.bump();
            let value = self.expr()?;
            bindings.push(Binding {
                name,
                name_span,
                value,
            });
        }
        if !self.is_keyword("RETURN") {
            return self.fail("VAR or RETURN");
        }
        self.bump();
        let body = self.expr()?;
        let span = Span {
            start,
            end: body.span().end,
        };
        Some(Expr::Var {
            bindings,
            body: Box::new(body),
            span,
        })
    }
}

fn describe(kind: &TokenKind) -> String {
    match kind {
        TokenKind::Ident(s) => format!("`{s}`"),
        TokenKind::QuotedTable(s) => format!("'{s}'"),
        TokenKind::Bracket(s) => format!("[{s}]"),
        TokenKind::Str(_) => "a string".into(),
        TokenKind::Number(n) => format!("`{n}`"),
        TokenKind::Param(p) => format!("`@{p}`"),
        TokenKind::Op(o) => format!("`{o}`"),
        TokenKind::LParen => "`(`".into(),
        TokenKind::RParen => "`)`".into(),
        TokenKind::LBrace => "`{`".into(),
        TokenKind::RBrace => "`}`".into(),
        TokenKind::Comma => "`,`".into(),
        TokenKind::Eof => "the end of the expression".into(),
    }
}

/// Words that structure a DAX query. Inside a query they cannot begin an expression (unless
/// followed by `(` or `[`), so a missing expression is reported where it is missing.
fn is_query_keyword(word: &str) -> bool {
    [
        "DEFINE", "EVALUATE", "MEASURE", "COLUMN", "TABLE", "FUNCTION", "ORDER", "START",
    ]
    .iter()
    .any(|k| k.eq_ignore_ascii_case(word))
}

/// Parses a DAX query (`DEFINE … EVALUATE … ORDER BY … START AT …`). Like [`parse`], it stops at
/// the first syntax error. Expressions are bounded by [`MAX_DEPTH`]; definitions and statements
/// are parsed in loops, so their number does not grow the stack.
pub(crate) fn parse_query(tokens: &[Token], diags: &mut Vec<Raw>) -> Option<Query> {
    if diags.iter().any(|d| d.severity == crate::Severity::Error) {
        return None;
    }
    let mut p = Parser {
        tokens,
        pos: 0,
        depth: 0,
        query: true,
        diags,
    };
    p.query()
}

impl Parser<'_, '_> {
    fn query(&mut self) -> Option<Query> {
        if matches!(self.peek(), TokenKind::Eof) {
            let span = self.span();
            self.diags.push(Raw::error(
                "empty-query",
                span,
                "the query is empty: it needs at least one EVALUATE statement",
            ));
            return None;
        }
        let start = self.span().start;
        let mut defines = Vec::new();
        if self.is_keyword("DEFINE") {
            self.bump();
            while self.is_definition_start() {
                defines.push(self.definition()?);
            }
            if defines.is_empty() {
                let span = self.span();
                self.diags.push(Raw::error(
                    "empty-define",
                    span,
                    "DEFINE needs at least one MEASURE, VAR, TABLE, COLUMN or FUNCTION definition",
                ));
                return None;
            }
        }
        let mut statements = Vec::new();
        while self.is_keyword("EVALUATE") {
            statements.push(self.evaluate()?);
        }
        if !matches!(self.peek(), TokenKind::Eof) {
            let span = self.span();
            if self.is_keyword("DEFINE") {
                let message = if statements.is_empty() {
                    "a query has only one DEFINE block"
                } else {
                    "DEFINE must come before the first EVALUATE"
                };
                self.diags.push(Raw::error("misplaced-define", span, message));
                return None;
            }
            if self.is_keyword("START") && !statements.is_empty() {
                self.diags.push(Raw::error(
                    "start-at-without-order-by",
                    span,
                    "START AT needs an ORDER BY clause before it",
                ));
                return None;
            }
            return self.fail(match (defines.is_empty(), statements.is_empty()) {
                (true, true) => "DEFINE or EVALUATE",
                (false, true) => "a definition (MEASURE, VAR, TABLE, COLUMN, FUNCTION) or EVALUATE",
                _ => "EVALUATE, ORDER BY, START AT or the end of the query",
            });
        }
        let Some(last) = statements.last() else {
            let span = self.span();
            self.diags.push(Raw::error(
                "missing-evaluate",
                span,
                "a query needs at least one EVALUATE statement",
            ));
            return None;
        };
        let end = last.span.end;
        Some(Query {
            defines,
            statements,
            span: Span { start, end },
        })
    }

    fn is_definition_start(&self) -> bool {
        ["MEASURE", "COLUMN", "VAR", "TABLE", "FUNCTION"]
            .iter()
            .any(|k| self.is_keyword(k))
    }

    /// An expression and its source text: unlike [`Expr::span`], the span covers parentheses
    /// around the expression (`(a + b)` spans `a + b` only).
    fn sourced(&mut self) -> Option<(Expr, Span)> {
        let start = self.span().start;
        let expr = self.expr()?;
        let end = self
            .pos
            .checked_sub(1)
            .and_then(|i| self.tokens.get(i))
            .map_or(expr.span().end, |t| t.span.end);
        Some((expr, Span { start, end }))
    }

    fn expect_op(&mut self, op: &str, what: &str) -> Option<Span> {
        if self.is_op(op) {
            Some(self.bump())
        } else {
            self.fail(what)
        }
    }

    fn ident(&mut self, what: &str) -> Option<(String, Span)> {
        match self.tokens.get(self.pos).cloned() {
            Some(Token {
                kind: TokenKind::Ident(n),
                span,
            }) => {
                self.bump();
                Some((n, span))
            }
            _ => self.fail(what),
        }
    }

    fn table_name(&mut self) -> Option<(String, Span)> {
        match self.tokens.get(self.pos).cloned() {
            Some(Token {
                kind: TokenKind::Ident(n),
                span,
            }) => {
                self.bump();
                self.check_reserved(&n, span, Place::TableName);
                Some((n, span))
            }
            Some(Token {
                kind: TokenKind::QuotedTable(n),
                span,
            }) => {
                self.bump();
                Some((n, span))
            }
            _ => self.fail("a table name"),
        }
    }

    /// `Table[Name]` of a MEASURE or COLUMN definition.
    fn qualified_name(&mut self, what: &str) -> Option<(String, Span, String, Span)> {
        let (table, table_span) = self.table_name()?;
        match self.tokens.get(self.pos).cloned() {
            Some(Token {
                kind: TokenKind::Bracket(n),
                span,
            }) => {
                self.bump();
                Some((table, table_span, n, span))
            }
            _ => self.fail(&format!("[{what} name] after the table name")),
        }
    }

    fn definition(&mut self) -> Option<Define> {
        let start = self.span().start;
        let keyword = match self.peek() {
            TokenKind::Ident(k) => k.to_ascii_uppercase(),
            _ => return self.fail("a definition"),
        };
        self.bump();
        match keyword.as_str() {
            "MEASURE" | "COLUMN" => {
                let what = if keyword == "MEASURE" { "measure" } else { "column" };
                let (table, table_span, name, name_span) = self.qualified_name(what)?;
                self.expect_op("=", &format!("`=` after the {what} name"))?;
                let (expr, expr_span) = self.sourced()?;
                let span = Span {
                    start,
                    end: expr_span.end,
                };
                Some(if keyword == "MEASURE" {
                    Define::Measure {
                        table,
                        table_span,
                        name,
                        name_span,
                        expr,
                        expr_span,
                        span,
                    }
                } else {
                    Define::Column {
                        table,
                        table_span,
                        name,
                        name_span,
                        expr,
                        expr_span,
                        span,
                    }
                })
            }
            "VAR" => {
                let (name, name_span) = self.ident("a variable name after VAR")?;
                self.check_reserved(&name, name_span, Place::Name("name a variable"));
                self.expect_op("=", "`=` after the variable name")?;
                let (expr, expr_span) = self.sourced()?;
                let span = Span {
                    start,
                    end: expr_span.end,
                };
                Some(Define::Var {
                    name,
                    name_span,
                    expr,
                    expr_span,
                    span,
                })
            }
            "TABLE" => {
                let (name, name_span) = self.table_name()?;
                self.expect_op("=", "`=` after the table name")?;
                let (expr, expr_span) = self.sourced()?;
                let span = Span {
                    start,
                    end: expr_span.end,
                };
                Some(Define::Table {
                    name,
                    name_span,
                    expr,
                    expr_span,
                    span,
                })
            }
            _ => self.function(start),
        }
    }

    /// `FUNCTION name = ( params ) => body`, after the FUNCTION keyword.
    fn function(&mut self, start: usize) -> Option<Define> {
        let (name, name_span) = self.ident("a function name after FUNCTION")?;
        self.expect_op("=", "`=` after the function name")?;
        self.expect(&TokenKind::LParen, "`(` to start the parameter list")?;
        let mut params = Vec::new();
        if !matches!(self.peek(), TokenKind::RParen) {
            loop {
                let (pname, pspan) = self.ident("a parameter name")?;
                let mut end = pspan.end;
                let mut hints = Vec::new();
                if self.is_op(":") {
                    self.bump();
                    while let Some(Token {
                        kind: TokenKind::Ident(word),
                        span,
                    }) = self.tokens.get(self.pos).cloned()
                    {
                        self.bump();
                        end = span.end;
                        hints.push(TypeHint { word, span });
                    }
                    if hints.is_empty() {
                        return self.fail("a type hint after `:`");
                    }
                }
                let (default, default_span) = if self.is_op("=") {
                    self.bump();
                    let (e, e_span) = self.sourced()?;
                    end = e_span.end;
                    (Some(e), Some(e_span))
                } else {
                    (None, None)
                };
                params.push(FunctionParam {
                    name: pname,
                    name_span: pspan,
                    hints,
                    default,
                    default_span,
                    span: Span {
                        start: pspan.start,
                        end,
                    },
                });
                if matches!(self.peek(), TokenKind::Comma) {
                    self.bump();
                    continue;
                }
                break;
            }
        }
        self.expect(&TokenKind::RParen, "`,` or `)` in the parameter list")?;
        self.expect_op("=>", "`=>` before the function body")?;
        let (body, body_span) = self.sourced()?;
        let span = Span {
            start,
            end: body_span.end,
        };
        Some(Define::Function {
            name,
            name_span,
            params,
            body,
            body_span,
            span,
        })
    }

    fn evaluate(&mut self) -> Option<EvaluateStatement> {
        let start = self.bump().start; // EVALUATE
        let (expr, expr_span) = self.sourced()?;
        let mut end = expr_span.end;
        let mut order_by = Vec::new();
        if self.is_keyword("ORDER") {
            self.bump();
            if !self.is_keyword("BY") {
                return self.fail("BY after ORDER");
            }
            self.bump();
            loop {
                let (key, key_span) = self.sourced()?;
                let key_start = key_span.start;
                let mut key_end = key_span.end;
                let mut descending = false;
                if self.is_keyword("ASC") || self.is_keyword("DESC") {
                    descending = self.is_keyword("DESC");
                    key_end = self.bump().end;
                }
                order_by.push(OrderKey {
                    expr: key,
                    expr_span: key_span,
                    descending,
                    span: Span {
                        start: key_start,
                        end: key_end,
                    },
                });
                end = key_end;
                if matches!(self.peek(), TokenKind::Comma) {
                    self.bump();
                    continue;
                }
                break;
            }
        }
        let mut start_at = Vec::new();
        if self.is_keyword("START") {
            if order_by.is_empty() {
                let span = self.span();
                self.diags.push(Raw::error(
                    "start-at-without-order-by",
                    span,
                    "START AT needs an ORDER BY clause before it",
                ));
                return None;
            }
            self.bump();
            if !self.is_keyword("AT") {
                return self.fail("AT after START");
            }
            self.bump();
            loop {
                let (value, value_span) = self.sourced()?;
                end = value_span.end;
                start_at.push(value);
                if matches!(self.peek(), TokenKind::Comma) {
                    self.bump();
                    continue;
                }
                break;
            }
        }
        Some(EvaluateStatement {
            expr,
            expr_span,
            order_by,
            start_at,
            span: Span { start, end },
        })
    }
}
