//! Recursive-descent parser for the M expression grammar (M specification,
//! "Expressions"). Nesting is bounded by `Limits::max_depth`.

use std::rc::Rc;

use super::ast::{BinOp, Expr, Handler, ListItem, Param, TypeExpr, UnOp};
use super::lexer::{Tok, Token, describe, syntax, tokenize};
use super::value::{Kind, MType, Value};
use crate::{Error, Limits};

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

pub(crate) struct Parser<'a> {
    src: &'a str,
    toks: Vec<Token>,
    pos: usize,
    max_depth: usize,
    depth: usize,
    /// Where each identifier reference of the tree was written (`m.validate`), keyed by the
    /// `Rc` of its [`Expr::Ident`] name.
    idents: Vec<IdentAt>,
}

/// The source position of one identifier reference ([`Expr::Ident`]) of a parsed tree.
#[derive(Debug, Clone)]
pub(crate) struct IdentAt {
    pub name: Rc<str>,
    pub line: usize,
    pub column: usize,
    /// Byte range of the identifier token (`#"quoted"` included).
    pub start: usize,
    pub end: usize,
}

const EOF: Token = Token {
    tok: Tok::Eof,
    line: 0,
    column: 0,
    start: 0,
    end: 0,
};

/// Parses a whole M expression (a partition or shared-expression body).
pub(crate) fn parse(src: &str, limits: &Limits) -> Result<Expr, Error> {
    parse_with_idents(src, limits).map(|(e, _)| e)
}

/// [`parse`], also returning where each identifier reference was written.
pub(crate) fn parse_with_idents(src: &str, limits: &Limits) -> Result<(Expr, Vec<IdentAt>), Error> {
    let toks = tokenize(src, limits)?;
    let mut p = Parser {
        src,
        toks,
        pos: 0,
        max_depth: limits.max_depth,
        depth: 0,
        idents: Vec::new(),
    };
    let e = p.expr()?;
    if p.peek().tok != Tok::Eof {
        return Err(p.err_here(format!("unexpected {}", describe(&p.peek().tok))));
    }
    Ok((e, p.idents))
}

pub(crate) fn primitive(name: &str) -> Option<Kind> {
    Some(match name {
        "any" => Kind::Any,
        "anynonnull" => Kind::AnyNonNull,
        "binary" => Kind::Binary,
        "date" => Kind::Date,
        "datetime" => Kind::DateTime,
        "datetimezone" => Kind::DateTimeZone,
        "duration" => Kind::Duration,
        "function" => Kind::Function,
        "list" => Kind::List,
        "logical" => Kind::Logical,
        "none" => Kind::None,
        "null" => Kind::Null,
        "number" => Kind::Number,
        "record" => Kind::Record,
        "table" => Kind::Table,
        "text" => Kind::Text,
        "time" => Kind::Time,
        "type" => Kind::Type,
        _ => return None,
    })
}

/// Whether `t` can start a type after a `type` written inside a type.
fn starts_type(t: &Tok) -> bool {
    match t {
        Tok::Ident(s) => s == "nullable" || s == "type" || primitive(s).is_some(),
        Tok::Punct(p) => matches!(*p, "{" | "["),
        _ => false,
    }
}

impl Parser<'_> {
    fn peek(&self) -> &Token {
        self.toks.get(self.pos).unwrap_or(&EOF)
    }

    fn peek_at(&self, k: usize) -> &Tok {
        self.toks
            .get(self.pos.saturating_add(k))
            .map_or(&Tok::Eof, |t| &t.tok)
    }

    fn bump(&mut self) -> Token {
        let t = self.peek().clone();
        if self.pos < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    /// An identifier reference read from token `t`, with its position recorded.
    fn ident(&mut self, name: &str, t: &Token) -> Expr {
        let name: Rc<str> = name.into();
        self.idents.push(IdentAt {
            name: name.clone(),
            line: t.line,
            column: t.column,
            start: t.start,
            end: t.end,
        });
        Expr::Ident(name)
    }

    fn err_here(&self, message: impl Into<String>) -> Error {
        let t = self.peek();
        syntax(t.line, t.column, message)
    }

    fn is_punct(&self, p: &str) -> bool {
        matches!(&self.peek().tok, Tok::Punct(q) if *q == p)
    }

    fn eat(&mut self, p: &str) -> bool {
        let hit = self.is_punct(p);
        if hit {
            self.bump();
        }
        hit
    }

    fn expect(&mut self, p: &str) -> Result<(), Error> {
        if self.eat(p) {
            Ok(())
        } else {
            Err(self.err_here(format!("expected `{p}`, found {}", describe(&self.peek().tok))))
        }
    }

    fn is_kw(&self, kw: &str) -> bool {
        matches!(&self.peek().tok, Tok::Ident(s) if s == kw)
    }

    fn eat_kw(&mut self, kw: &str) -> bool {
        let hit = self.is_kw(kw);
        if hit {
            self.bump();
        }
        hit
    }

    fn expect_kw(&mut self, kw: &str) -> Result<(), Error> {
        if self.eat_kw(kw) {
            Ok(())
        } else {
            Err(self.err_here(format!("expected `{kw}`, found {}", describe(&self.peek().tok))))
        }
    }

    fn enter(&mut self) -> Result<(), Error> {
        self.depth += 1;
        if self.depth > self.max_depth {
            return Err(Error::LimitExceeded(format!(
                "M nesting deeper than {}",
                self.max_depth
            )));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// A variable name: regular or `#"quoted"` identifier.
    fn name(&mut self) -> Result<Rc<str>, Error> {
        match self.peek().tok.clone() {
            Tok::Ident(s) if !KEYWORDS.contains(&s.as_str()) => {
                self.bump();
                Ok(s.into())
            }
            Tok::Quoted(s) => {
                self.bump();
                Ok(s.into())
            }
            other => Err(self.err_here(format!("expected a name, found {}", describe(&other)))),
        }
    }

    /// A generalized identifier (field name): `#"Quoted"`, or the source text of
    /// the tokens up to `=`, `]` or `,` (`Unit Price`, `Sales.Amount`, `2020`).
    fn field_name(&mut self) -> Result<Rc<str>, Error> {
        if let Tok::Quoted(s) = &self.peek().tok {
            let s: Rc<str> = s.as_str().into();
            self.bump();
            return Ok(s);
        }
        let first = self.peek().clone();
        let mut last_end = first.start;
        let mut n = 0usize;
        while let Tok::Ident(_) | Tok::Number(_) | Tok::Hash(_) = &self.peek().tok {
            last_end = self.peek().end;
            self.bump();
            n += 1;
        }
        if n == 0 {
            return Err(self.err_here(format!(
                "expected a field name, found {}",
                describe(&self.peek().tok)
            )));
        }
        Ok(self.src.get(first.start..last_end).unwrap_or("").into())
    }

    pub(crate) fn expr(&mut self) -> Result<Expr, Error> {
        self.enter()?;
        let r = self.expr_inner();
        self.leave();
        r
    }

    fn expr_inner(&mut self) -> Result<Expr, Error> {
        match self.peek().tok.clone() {
            Tok::Ident(s) if s == "let" => {
                self.bump();
                let mut bindings = Vec::new();
                loop {
                    let name = self.name()?;
                    self.expect("=")?;
                    let e = self.expr()?;
                    bindings.push((name, Rc::new(e)));
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect_kw("in")?;
                let body = self.expr()?;
                Ok(Expr::Let(Rc::new(bindings), Box::new(body)))
            }
            Tok::Ident(s) if s == "if" => {
                self.bump();
                let c = self.expr()?;
                self.expect_kw("then")?;
                let t = self.expr()?;
                self.expect_kw("else")?;
                let f = self.expr()?;
                Ok(Expr::If(Box::new(c), Box::new(t), Box::new(f)))
            }
            Tok::Ident(s) if s == "each" => {
                self.bump();
                let body = self.expr()?;
                Ok(Expr::Func(
                    Rc::new(vec![Param {
                        name: "_".into(),
                        optional: false,
                    }]),
                    Rc::new(body),
                ))
            }
            Tok::Ident(s) if s == "error" => {
                self.bump();
                Ok(Expr::Error(Box::new(self.expr()?)))
            }
            Tok::Ident(s) if s == "try" => {
                self.bump();
                let protected = self.expr()?;
                let handler = if self.eat_kw("otherwise") {
                    Handler::Otherwise(Box::new(self.expr()?))
                } else if self.eat_kw("catch") {
                    let Some(Expr::Func(params, body)) = self.try_function()? else {
                        return Err(self.err_here("expected a function after `catch`"));
                    };
                    if params.len() > 1 {
                        return Err(self.err_here("a `catch` function takes at most one parameter"));
                    }
                    Handler::Catch(params, body)
                } else {
                    Handler::None
                };
                Ok(Expr::Try(Box::new(protected), handler))
            }
            Tok::Punct("(") => match self.try_function()? {
                Some(f) => Ok(f),
                None => self.coalesce(),
            },
            _ => self.coalesce(),
        }
    }

    /// Parses `(params) [as type] => body` if the tokens form a function
    /// expression; otherwise restores the position and returns `None`.
    fn try_function(&mut self) -> Result<Option<Expr>, Error> {
        let save = self.pos;
        let Some(params) = self.params() else {
            self.pos = save;
            return Ok(None);
        };
        if self.eat_kw("as") {
            // Return type: parse and ignore.
            let _ = self.nullable_primitive();
        }
        if !self.eat("=>") {
            self.pos = save;
            return Ok(None);
        }
        let body = self.expr()?;
        Ok(Some(Expr::Func(Rc::new(params), Rc::new(body))))
    }

    fn params(&mut self) -> Option<Vec<Param>> {
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
            let name = self.name().ok()?;
            if self.eat_kw("as") {
                self.nullable_primitive().ok()?;
            }
            params.push(Param { name, optional });
            if self.eat(")") {
                return Some(params);
            }
            if !self.eat(",") {
                return None;
            }
        }
    }

    /// `[nullable] primitive-type` (after `is`, `as` and in parameter lists).
    fn nullable_primitive(&mut self) -> Result<MType, Error> {
        let nullable = self.eat_kw("nullable");
        match self.peek().tok.clone() {
            Tok::Ident(s) => match primitive(&s) {
                Some(k) => {
                    self.bump();
                    let t = MType::prim(k);
                    Ok(if nullable { t.nullable() } else { t })
                }
                None => Err(self.err_here(format!("expected a primitive type, found `{s}`"))),
            },
            other => Err(self.err_here(format!("expected a primitive type, found {}", describe(&other)))),
        }
    }

    fn binary_left(
        &mut self,
        next: fn(&mut Self) -> Result<Expr, Error>,
        ops: &[(&str, BinOp)],
        keyword: bool,
    ) -> Result<Expr, Error> {
        let mut lhs = next(self)?;
        loop {
            let op = ops
                .iter()
                .find(|(s, _)| if keyword { self.is_kw(s) } else { self.is_punct(s) });
            let Some((_, op)) = op else { return Ok(lhs) };
            let op = *op;
            self.bump();
            self.enter()?;
            let rhs = next(self);
            self.leave();
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs?));
        }
    }

    fn coalesce(&mut self) -> Result<Expr, Error> {
        let lhs = self.logical_or()?;
        if self.eat("??") {
            self.enter()?;
            let rhs = self.coalesce();
            self.leave();
            return Ok(Expr::Binary(BinOp::Coalesce, Box::new(lhs), Box::new(rhs?)));
        }
        Ok(lhs)
    }

    fn logical_or(&mut self) -> Result<Expr, Error> {
        self.binary_left(Self::logical_and, &[("or", BinOp::Or)], true)
    }

    fn logical_and(&mut self) -> Result<Expr, Error> {
        self.binary_left(Self::is_expr, &[("and", BinOp::And)], true)
    }

    fn is_expr(&mut self) -> Result<Expr, Error> {
        let mut e = self.as_expr()?;
        while self.eat_kw("is") {
            let t = self.nullable_primitive()?;
            e = Expr::Is(Box::new(e), Box::new(TypeExpr::Prim(t)));
        }
        Ok(e)
    }

    fn as_expr(&mut self) -> Result<Expr, Error> {
        let mut e = self.equality()?;
        while self.eat_kw("as") {
            let t = self.nullable_primitive()?;
            e = Expr::As(Box::new(e), Box::new(TypeExpr::Prim(t)));
        }
        Ok(e)
    }

    fn equality(&mut self) -> Result<Expr, Error> {
        self.binary_left(Self::relational, &[("=", BinOp::Eq), ("<>", BinOp::Ne)], false)
    }

    fn relational(&mut self) -> Result<Expr, Error> {
        self.binary_left(
            Self::additive,
            &[
                ("<=", BinOp::Le),
                (">=", BinOp::Ge),
                ("<", BinOp::Lt),
                (">", BinOp::Gt),
            ],
            false,
        )
    }

    fn additive(&mut self) -> Result<Expr, Error> {
        self.binary_left(
            Self::multiplicative,
            &[("+", BinOp::Add), ("-", BinOp::Sub), ("&", BinOp::Concat)],
            false,
        )
    }

    fn multiplicative(&mut self) -> Result<Expr, Error> {
        self.binary_left(Self::metadata, &[("*", BinOp::Mul), ("/", BinOp::Div)], false)
    }

    fn metadata(&mut self) -> Result<Expr, Error> {
        let mut e = self.unary()?;
        while self.eat_kw("meta") {
            let m = self.unary()?;
            e = Expr::Meta(Box::new(e), Box::new(m));
        }
        Ok(e)
    }

    fn unary(&mut self) -> Result<Expr, Error> {
        self.enter()?;
        let r = self.unary_inner();
        self.leave();
        r
    }

    fn unary_inner(&mut self) -> Result<Expr, Error> {
        if self.eat("+") {
            return Ok(Expr::Unary(UnOp::Plus, Box::new(self.unary()?)));
        }
        if self.eat("-") {
            return Ok(Expr::Unary(UnOp::Neg, Box::new(self.unary()?)));
        }
        if self.eat_kw("not") {
            return Ok(Expr::Unary(UnOp::Not, Box::new(self.unary()?)));
        }
        if self.eat_kw("type") {
            return Ok(Expr::Type(Box::new(self.primary_type()?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr, Error> {
        let mut e = self.base()?;
        loop {
            if self.is_punct("(") {
                self.bump();
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
                e = Expr::Call(Box::new(e), args);
            } else if self.is_punct("{") {
                self.bump();
                let i = self.expr()?;
                self.expect("}")?;
                let opt = self.eat("?");
                e = Expr::Item(Box::new(e), Box::new(i), opt);
            } else if self.is_punct("[") {
                self.bump();
                e = self.field_suffix(e)?;
            } else {
                return Ok(e);
            }
        }
    }

    /// After `[`: `name]`, `name]?`, `[a], [b]]` (projection).
    fn field_suffix(&mut self, target: Expr) -> Result<Expr, Error> {
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
            return Ok(Expr::Project(Box::new(target), names, opt));
        }
        let name = self.field_name()?;
        self.expect("]")?;
        let opt = self.eat("?");
        Ok(Expr::Field(Box::new(target), name, opt))
    }

    fn base(&mut self) -> Result<Expr, Error> {
        let t = self.peek().clone();
        match t.tok.clone() {
            Tok::Number(n) => {
                self.bump();
                Ok(Expr::Const(Value::Number(n)))
            }
            Tok::Text(s) => {
                self.bump();
                Ok(Expr::Const(Value::text(s)))
            }
            Tok::Hash(h) => {
                self.bump();
                Ok(match h.as_str() {
                    "#infinity" => Expr::Const(Value::num(f64::INFINITY)),
                    "#nan" => Expr::Const(Value::num(f64::NAN)),
                    _ => self.ident(&h, &t),
                })
            }
            Tok::Quoted(s) => {
                self.bump();
                Ok(self.ident(&s, &t))
            }
            Tok::Ident(s) => match s.as_str() {
                "true" => {
                    self.bump();
                    Ok(Expr::Const(Value::Logical(true)))
                }
                "false" => {
                    self.bump();
                    Ok(Expr::Const(Value::Logical(false)))
                }
                "null" => {
                    self.bump();
                    Ok(Expr::Const(Value::Null))
                }
                kw if KEYWORDS.contains(&kw) => Err(self.err_here(format!("unexpected keyword `{kw}`"))),
                _ => {
                    self.bump();
                    Ok(self.ident(&s, &t))
                }
            },
            Tok::Punct("@") => {
                self.bump();
                let at = self.peek().clone();
                let name = self.name()?;
                Ok(self.ident(&name, &at))
            }
            Tok::Punct("...") => {
                self.bump();
                Ok(Expr::NotImplemented)
            }
            Tok::Punct("(") => {
                self.bump();
                let e = self.expr()?;
                self.expect(")")?;
                Ok(e)
            }
            Tok::Punct("{") => {
                self.bump();
                let mut items = Vec::new();
                if !self.eat("}") {
                    loop {
                        let a = self.expr()?;
                        if self.eat("..") {
                            let b = self.expr()?;
                            items.push(ListItem::Range(a, b));
                        } else {
                            items.push(ListItem::One(a));
                        }
                        if self.eat("}") {
                            break;
                        }
                        self.expect(",")?;
                    }
                }
                Ok(Expr::List(items))
            }
            Tok::Punct("[") => {
                self.bump();
                if self.eat("]") {
                    return Ok(Expr::Record(Rc::new(Vec::new())));
                }
                if self.is_punct("[") {
                    return self.field_suffix(Expr::Ident("_".into()));
                }
                let save = self.pos;
                let name = self.field_name()?;
                if !self.is_punct("=") {
                    self.pos = save;
                    return self.field_suffix(Expr::Ident("_".into()));
                }
                let mut fields = Vec::new();
                let mut name = name;
                loop {
                    self.expect("=")?;
                    let e = self.expr()?;
                    fields.push((name, Rc::new(e)));
                    if self.eat("]") {
                        break;
                    }
                    self.expect(",")?;
                    name = self.field_name()?;
                }
                Ok(Expr::Record(Rc::new(fields)))
            }
            other => Err(self.err_here(format!("unexpected {}", describe(&other)))),
        }
    }

    fn primary_type(&mut self) -> Result<TypeExpr, Error> {
        self.enter()?;
        let r = self.primary_type_inner();
        self.leave();
        r
    }

    fn primary_type_inner(&mut self) -> Result<TypeExpr, Error> {
        match self.peek().tok.clone() {
            Tok::Ident(s) if s == "nullable" => {
                self.bump();
                Ok(TypeExpr::Nullable(Box::new(self.primary_type()?)))
            }
            // `type X` inside a type reads as X; `type` alone is the primitive type of types
            Tok::Ident(s) if s == "type" && starts_type(self.peek_at(1)) => {
                self.bump();
                self.primary_type()
            }
            Tok::Ident(s) if s == "table" => {
                self.bump();
                match &self.peek().tok {
                    Tok::Punct("[") => {
                        let row = self.primary_type()?;
                        Ok(TypeExpr::Table(Box::new(row)))
                    }
                    Tok::Ident(n) if !KEYWORDS.contains(&n.as_str()) && primitive(n).is_none() => {
                        let e = self.primary()?;
                        Ok(TypeExpr::Table(Box::new(TypeExpr::Expr(Box::new(e)))))
                    }
                    Tok::Quoted(_) | Tok::Punct("(") => {
                        let e = self.primary()?;
                        Ok(TypeExpr::Table(Box::new(TypeExpr::Expr(Box::new(e)))))
                    }
                    _ => Ok(TypeExpr::Prim(MType::prim(Kind::Table))),
                }
            }
            Tok::Ident(s) if s == "function" && matches!(self.peek_at(1), Tok::Punct("(")) => {
                self.bump();
                // Function types: parameters and return type are parsed and ignored.
                if self.params().is_none() {
                    return Err(self.err_here("bad function type"));
                }
                if self.eat_kw("as") {
                    self.nullable_primitive()?;
                }
                Ok(TypeExpr::Prim(MType::prim(Kind::Function)))
            }
            Tok::Ident(s) if primitive(&s).is_some() => {
                self.bump();
                Ok(TypeExpr::Prim(MType::prim(primitive(&s).unwrap_or(Kind::Any))))
            }
            Tok::Punct("{") => {
                self.bump();
                let item = self.primary_type()?;
                self.expect("}")?;
                Ok(TypeExpr::List(Box::new(item)))
            }
            Tok::Punct("[") => {
                self.bump();
                let mut fields = Vec::new();
                if self.eat("]") {
                    return Ok(TypeExpr::Record(fields));
                }
                loop {
                    if self.eat("...") {
                        self.expect("]")?;
                        return Ok(TypeExpr::Record(fields));
                    }
                    if matches!(self.peek_at(1), Tok::Ident(_) | Tok::Quoted(_)) {
                        self.eat_kw("optional");
                    }
                    let name = self.field_name()?;
                    let ty = if self.eat("=") {
                        self.primary_type()?
                    } else {
                        TypeExpr::Prim(MType::any())
                    };
                    fields.push((name, ty));
                    if self.eat("]") {
                        return Ok(TypeExpr::Record(fields));
                    }
                    self.expect(",")?;
                }
            }
            _ => Ok(TypeExpr::Expr(Box::new(self.primary()?))),
        }
    }
}
