//! Lexer and parser for the Dynamic LINQ predicates of BPA rules.
//!
//! ```text
//! expr    := coalesce ('?' expr ':' expr)?
//! coalesce:= or ('??' or)*
//! or      := and (('||' | 'or') and)*
//! and     := cmp (('&&' | 'and') cmp)*
//! cmp     := add (('=' | '==' | '!=' | '<>' | '<' | '<=' | '>' | '>=') add)*
//! add     := mul (('+' | '-' | '&') mul)*
//! mul     := unary (('*' | '/' | '%' | 'mod') unary)*
//! unary   := ('!' | 'not' | '-') unary | postfix
//! postfix := primary ('.' NAME ('(' args ')')? | '[' expr ']')*
//! primary := NUMBER | STRING | 'true' | 'false' | 'null' | 'it' | 'outerIt' | 'current'
//!          | 'iif' '(' expr ',' expr ',' expr ')' | NAME ('(' args ')')? | '(' expr ')'
//! args    := (arg (',' arg)*)?        arg := NAME '=>' expr | expr
//! ```
//!
//! String literals use `"` or `'`; a doubled quote stands for one quote and backslashes are
//! kept as written (so regular-expression escapes reach `RegEx.IsMatch` unchanged). Keywords
//! are case-insensitive. Parsing is bounded: [`MAX_DEPTH`] tree depth (operators, member
//! accesses and parentheses all count, so evaluation recursion is bounded too) and
//! [`MAX_TOKENS`] tokens; [`MAX_NEST`] bounds the parser's own recursion.

/// Deepest expression tree accepted.
pub(crate) const MAX_DEPTH: u32 = 256;
/// Deepest nesting of parentheses, arguments and branches (each level costs the parser a dozen
/// stack frames, so it is bounded more tightly than the tree).
pub(crate) const MAX_NEST: u32 = 40;
/// Most tokens in one expression.
pub(crate) const MAX_TOKENS: usize = 20_000;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Tok {
    Name(String),
    Num(f64),
    Str(String),
    Op(&'static str),
}

const OPS: &[&str] = &[
    "==", "!=", "<>", "<=", ">=", "&&", "||", "=>", "??", "=", "<", ">", "!", "&", "+", "-", "*", "/", "%",
    "?", ":", "(", ")", "[", "]", ",", ".",
];

pub(crate) fn lex(text: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while let Some(&c) = chars.get(i) {
        if out.len() > MAX_TOKENS {
            return Err(format!("the expression has more than {MAX_TOKENS} tokens"));
        }
        if c.is_whitespace() {
            i += 1;
        } else if c.is_alphabetic() || c == '_' || c == '@' {
            let start = i;
            while chars
                .get(i)
                .is_some_and(|c| c.is_alphanumeric() || *c == '_' || *c == '@')
            {
                i += 1;
            }
            out.push(Tok::Name(
                chars.get(start..i).unwrap_or_default().iter().collect(),
            ));
        } else if c.is_ascii_digit() {
            let start = i;
            while chars.get(i).is_some_and(char::is_ascii_digit) {
                i += 1;
            }
            if chars.get(i) == Some(&'.') && chars.get(i + 1).is_some_and(char::is_ascii_digit) {
                i += 1;
                while chars.get(i).is_some_and(char::is_ascii_digit) {
                    i += 1;
                }
            }
            let s: String = chars.get(start..i).unwrap_or_default().iter().collect();
            // Type suffixes (`1L`, `2.5m`, `1d`, `1f`, `1u`) do not change the value here.
            if chars.get(i).is_some_and(|c| "LlMmDdFfUu".contains(*c))
                && !chars.get(i + 1).is_some_and(|c| c.is_alphanumeric())
            {
                i += 1;
            }
            let n = s.parse::<f64>().map_err(|_| format!("bad number `{s}`"))?;
            out.push(Tok::Num(n));
        } else if c == '"' || c == '\'' {
            let quote = c;
            i += 1;
            let mut s = String::new();
            loop {
                match chars.get(i) {
                    None => return Err("unterminated string literal".into()),
                    Some(&q) if q == quote => {
                        if chars.get(i + 1) == Some(&quote) {
                            s.push(quote);
                            i += 2;
                        } else {
                            i += 1;
                            break;
                        }
                    }
                    Some(&x) => {
                        s.push(x);
                        i += 1;
                    }
                }
            }
            out.push(Tok::Str(s));
        } else {
            let rest: String = chars
                .get(i..(i + 2).min(chars.len()))
                .unwrap_or_default()
                .iter()
                .collect();
            let Some(op) = OPS.iter().find(|op| rest.starts_with(**op)) else {
                return Err(format!("unexpected character `{c}`"));
            };
            i += op.chars().count();
            out.push(Tok::Op(op));
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BinOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Concat,
    Mul,
    Div,
    Mod,
    Coalesce,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnOp {
    Not,
    Neg,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Ex {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    It,
    OuterIt,
    Current,
    /// A bare name: a lambda parameter, a property of `it`, or a type name before `.`.
    Name(String),
    Member(Box<Ex>, String),
    /// `target.name(args)`; `target` is `None` for a call on `it` (or a conversion like
    /// `char(9)`).
    Call(Option<Box<Ex>>, String, Vec<Ex>),
    Index(Box<Ex>, Box<Ex>),
    Unary(UnOp, Box<Ex>),
    Binary(BinOp, Box<Ex>, Box<Ex>),
    Cond(Box<Ex>, Box<Ex>, Box<Ex>),
    /// `x => body`.
    Lambda(String, Box<Ex>),
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
    depth: u32,
    nest: u32,
}

/// Parses a rule expression.
pub(crate) fn parse(text: &str) -> Result<Ex, String> {
    let toks = lex(text)?;
    if toks.is_empty() {
        return Err("the expression is empty".into());
    }
    let mut p = Parser {
        toks,
        pos: 0,
        depth: 0,
        nest: 0,
    };
    let e = p.expr()?;
    match p.peek() {
        None => Ok(e),
        Some(t) => Err(format!("unexpected {} after the expression", describe(t))),
    }
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Name(n) => format!("`{n}`"),
        Tok::Num(n) => format!("number {n}"),
        Tok::Str(_) => "string".into(),
        Tok::Op(o) => format!("`{o}`"),
    }
}

/// Runs `f` and puts the depth back where it was, so each nested level is counted once.
macro_rules! scoped {
    ($self:ident, $body:expr) => {{
        let base = $self.depth;
        let r = $body;
        $self.depth = base;
        r
    }};
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn peek_at(&self, k: usize) -> Option<&Tok> {
        self.toks.get(self.pos.saturating_add(k))
    }

    fn is_op(&self, op: &str) -> bool {
        matches!(self.peek(), Some(Tok::Op(o)) if *o == op)
    }

    fn eat_op(&mut self, op: &str) -> bool {
        if self.is_op(op) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn eat_kw(&mut self, kw: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Name(n)) if n.eq_ignore_ascii_case(kw)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, op: &str) -> Result<(), String> {
        if self.eat_op(op) {
            Ok(())
        } else {
            Err(match self.peek() {
                Some(t) => format!("expected `{op}`, found {}", describe(t)),
                None => format!("expected `{op}` at the end of the expression"),
            })
        }
    }

    fn enter(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            Err(format!("the expression is nested deeper than {MAX_DEPTH} levels"))
        } else {
            Ok(())
        }
    }

    fn expr(&mut self) -> Result<Ex, String> {
        if self.nest >= MAX_NEST {
            return Err(format!("the expression nests deeper than {MAX_NEST} levels"));
        }
        self.nest += 1;
        let r = scoped!(self, self.enter().and_then(|()| self.conditional()));
        self.nest -= 1;
        r
    }

    fn conditional(&mut self) -> Result<Ex, String> {
        let c = self.coalesce()?;
        if self.eat_op("?") {
            self.enter()?;
            let a = self.expr()?;
            self.expect(":")?;
            let b = self.expr()?;
            return Ok(Ex::Cond(Box::new(c), Box::new(a), Box::new(b)));
        }
        Ok(c)
    }

    fn coalesce(&mut self) -> Result<Ex, String> {
        scoped!(self, {
            let mut l = self.or()?;
            while self.eat_op("??") {
                self.enter()?;
                let r = self.or()?;
                l = Ex::Binary(BinOp::Coalesce, Box::new(l), Box::new(r));
            }
            Ok(l)
        })
    }

    fn or(&mut self) -> Result<Ex, String> {
        scoped!(self, {
            let mut l = self.and()?;
            while self.eat_op("||") || self.eat_kw("or") {
                self.enter()?;
                let r = self.and()?;
                l = Ex::Binary(BinOp::Or, Box::new(l), Box::new(r));
            }
            Ok(l)
        })
    }

    fn and(&mut self) -> Result<Ex, String> {
        scoped!(self, {
            let mut l = self.cmp()?;
            while self.eat_op("&&") || self.eat_kw("and") {
                self.enter()?;
                let r = self.cmp()?;
                l = Ex::Binary(BinOp::And, Box::new(l), Box::new(r));
            }
            Ok(l)
        })
    }

    fn cmp(&mut self) -> Result<Ex, String> {
        scoped!(self, {
            let mut l = self.add()?;
            loop {
                let op = match self.peek() {
                    Some(Tok::Op("=" | "==")) => BinOp::Eq,
                    Some(Tok::Op("!=" | "<>")) => BinOp::Ne,
                    Some(Tok::Op("<")) => BinOp::Lt,
                    Some(Tok::Op("<=")) => BinOp::Le,
                    Some(Tok::Op(">")) => BinOp::Gt,
                    Some(Tok::Op(">=")) => BinOp::Ge,
                    _ => break Ok(l),
                };
                self.pos += 1;
                self.enter()?;
                let r = self.add()?;
                l = Ex::Binary(op, Box::new(l), Box::new(r));
            }
        })
    }

    fn add(&mut self) -> Result<Ex, String> {
        scoped!(self, {
            let mut l = self.mul()?;
            loop {
                let op = match self.peek() {
                    Some(Tok::Op("+")) => BinOp::Add,
                    Some(Tok::Op("-")) => BinOp::Sub,
                    Some(Tok::Op("&")) => BinOp::Concat,
                    _ => break Ok(l),
                };
                self.pos += 1;
                self.enter()?;
                let r = self.mul()?;
                l = Ex::Binary(op, Box::new(l), Box::new(r));
            }
        })
    }

    fn mul(&mut self) -> Result<Ex, String> {
        scoped!(self, {
            let mut l = self.unary()?;
            loop {
                let op = match self.peek() {
                    Some(Tok::Op("*")) => BinOp::Mul,
                    Some(Tok::Op("/")) => BinOp::Div,
                    Some(Tok::Op("%")) => BinOp::Mod,
                    Some(Tok::Name(n)) if n.eq_ignore_ascii_case("mod") => BinOp::Mod,
                    _ => break Ok(l),
                };
                self.pos += 1;
                self.enter()?;
                let r = self.unary()?;
                l = Ex::Binary(op, Box::new(l), Box::new(r));
            }
        })
    }

    fn unary(&mut self) -> Result<Ex, String> {
        let op = if self.eat_op("!") || self.eat_kw("not") {
            UnOp::Not
        } else if self.eat_op("-") {
            UnOp::Neg
        } else {
            return self.postfix();
        };
        let e = scoped!(self, self.enter().and_then(|()| self.unary()))?;
        Ok(Ex::Unary(op, Box::new(e)))
    }

    fn postfix(&mut self) -> Result<Ex, String> {
        scoped!(self, {
            let mut e = self.primary()?;
            loop {
                if self.eat_op(".") {
                    self.enter()?;
                    let Some(Tok::Name(name)) = self.peek().cloned() else {
                        break Err("expected a member name after `.`".to_string());
                    };
                    self.pos += 1;
                    if self.is_op("(") {
                        let args = self.args()?;
                        e = Ex::Call(Some(Box::new(e)), name, args);
                    } else {
                        e = Ex::Member(Box::new(e), name);
                    }
                } else if self.eat_op("[") {
                    self.enter()?;
                    let i = self.expr()?;
                    self.expect("]")?;
                    e = Ex::Index(Box::new(e), Box::new(i));
                } else {
                    break Ok(e);
                }
            }
        })
    }

    fn args(&mut self) -> Result<Vec<Ex>, String> {
        self.expect("(")?;
        let mut args = Vec::new();
        if self.eat_op(")") {
            return Ok(args);
        }
        loop {
            let lambda = match (self.peek(), self.peek_at(1)) {
                (Some(Tok::Name(n)), Some(Tok::Op("=>"))) => Some(n.clone()),
                _ => None,
            };
            if let Some(param) = lambda {
                self.pos += 2;
                let body = self.expr()?;
                args.push(Ex::Lambda(param, Box::new(body)));
            } else {
                args.push(self.expr()?);
            }
            if self.eat_op(")") {
                return Ok(args);
            }
            self.expect(",")?;
        }
    }

    fn primary(&mut self) -> Result<Ex, String> {
        let Some(t) = self.peek().cloned() else {
            return Err("the expression ends too early".into());
        };
        self.pos += 1;
        match t {
            Tok::Num(n) => Ok(Ex::Num(n)),
            Tok::Str(s) => Ok(Ex::Str(s)),
            Tok::Op("(") => {
                let e = self.expr()?;
                self.expect(")")?;
                Ok(e)
            }
            Tok::Op(o) => Err(format!("unexpected `{o}`")),
            Tok::Name(n) => {
                let lower = n.to_ascii_lowercase();
                match lower.as_str() {
                    "true" => return Ok(Ex::Bool(true)),
                    "false" => return Ok(Ex::Bool(false)),
                    "null" => return Ok(Ex::Null),
                    "it" => return Ok(Ex::It),
                    "outerit" => return Ok(Ex::OuterIt),
                    "current" => return Ok(Ex::Current),
                    "new" => return Err("`new` (object creation) is not supported".into()),
                    _ => {}
                }
                if self.is_op("(") {
                    let args = self.args()?;
                    if lower == "iif" {
                        let mut it = args.into_iter();
                        return match (it.next(), it.next(), it.next(), it.next()) {
                            (Some(c), Some(a), Some(b), None) => {
                                Ok(Ex::Cond(Box::new(c), Box::new(a), Box::new(b)))
                            }
                            _ => Err("iif takes three arguments".into()),
                        };
                    }
                    return Ok(Ex::Call(None, n, args));
                }
                Ok(Ex::Name(n))
            }
        }
    }
}
