//! Syntax tree of the supported M subset.

use std::rc::Rc;

use super::value::{MType, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnOp {
    Plus,
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Concat,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Coalesce,
}

#[derive(Debug, Clone)]
pub(crate) struct Param {
    pub name: Rc<str>,
    pub optional: bool,
}

#[derive(Debug)]
pub(crate) enum ListItem {
    One(Expr),
    Range(Expr, Expr),
}

#[derive(Debug)]
pub(crate) enum Handler {
    None,
    Otherwise(Box<Expr>),
    Catch(Rc<Vec<Param>>, Rc<Expr>),
}

/// A type written after `type` (or a field type inside one).
#[derive(Debug)]
pub(crate) enum TypeExpr {
    Prim(MType),
    Nullable(Box<TypeExpr>),
    List(Box<TypeExpr>),
    /// Record type: fields (name, type, optional) and whether it is open (`...`).
    Record(Vec<(Rc<str>, TypeExpr)>),
    /// `table [..]` or `table <row type expression>`.
    Table(Box<TypeExpr>),
    /// A primary expression that must evaluate to a type value (`Int64.Type`, `_t`).
    Expr(Box<Expr>),
}

pub(crate) type Bindings = Rc<Vec<(Rc<str>, Rc<Expr>)>>;

#[derive(Debug)]
pub(crate) enum Expr {
    Const(Value),
    Ident(Rc<str>),
    Let(Bindings, Box<Expr>),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    Func(Rc<Vec<Param>>, Rc<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    /// `x[name]`, `x[name]?`.
    Field(Box<Expr>, Rc<str>, bool),
    /// `x[[a], [b]]`, optional.
    Project(Box<Expr>, Vec<Rc<str>>, bool),
    /// `x{i}`, `x{i}?`.
    Item(Box<Expr>, Box<Expr>, bool),
    List(Vec<ListItem>),
    Record(Bindings),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Is(Box<Expr>, Box<TypeExpr>),
    As(Box<Expr>, Box<TypeExpr>),
    Meta(Box<Expr>, Box<Expr>),
    Type(Box<TypeExpr>),
    Error(Box<Expr>),
    Try(Box<Expr>, Handler),
    NotImplemented,
}
