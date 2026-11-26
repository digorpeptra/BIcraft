//! DAX expression tree.

use serde::Serialize;

use crate::Span;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Expr {
    Number {
        text: String,
        span: Span,
    },
    Str {
        value: String,
        span: Span,
    },
    Bool {
        value: bool,
        span: Span,
    },
    /// `NAME(arg, , arg)`; skipped optional arguments are `None`.
    Call {
        name: String,
        name_span: Span,
        args: Vec<Option<Expr>>,
        span: Span,
    },
    /// A bare or quoted name that is not a call: a table or a variable (or a keyword argument such
    /// as `DESC`, which only the binder can tell apart).
    Name {
        name: String,
        quoted: bool,
        span: Span,
    },
    /// `Table[Column]`, `'My Table'[Column]` or `[Column or Measure]`, optionally followed by a
    /// column variation: `'Sales'[OrderDate].[Year]` (auto date/time).
    Column {
        table: Option<String>,
        table_span: Option<Span>,
        name: String,
        /// `.[Name]` after the column: the column `Name` of the table the column's variation
        /// points to. `span` covers the whole reference, variation included.
        #[serde(skip_serializing_if = "Option::is_none")]
        variation: Option<Box<Variation>>,
        span: Span,
    },
    Unary {
        op: String,
        operand: Box<Expr>,
        span: Span,
    },
    Binary {
        op: String,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Span,
    },
    /// `VAR a = … VAR b = … RETURN body`
    Var {
        bindings: Vec<Binding>,
        body: Box<Expr>,
        span: Span,
    },
    /// `{ 1, 2 }` or `{ (1, "a"), (2, "b") }`
    TableCtor {
        rows: Vec<Expr>,
        span: Span,
    },
    /// `(a, b)` inside a table constructor or on the left of `IN`.
    Tuple {
        items: Vec<Expr>,
        span: Span,
    },
}

/// The `.[Name]` part of a column variation reference.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Variation {
    pub name: String,
    /// The `[Name]` token.
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Binding {
    pub name: String,
    pub name_span: Span,
    pub value: Expr,
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Number { span, .. }
            | Expr::Str { span, .. }
            | Expr::Bool { span, .. }
            | Expr::Call { span, .. }
            | Expr::Name { span, .. }
            | Expr::Column { span, .. }
            | Expr::Unary { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Var { span, .. }
            | Expr::TableCtor { span, .. }
            | Expr::Tuple { span, .. } => *span,
        }
    }
}

/// A DAX query: `[DEFINE definition…] EVALUATE table [ORDER BY …] [START AT …] …`.
///
/// XMLA parameters (`@name`) appear in expressions as [`Expr::Name`] whose name starts with `@`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Query {
    pub defines: Vec<Define>,
    pub statements: Vec<EvaluateStatement>,
    pub span: Span,
}

/// One entry of a query's `DEFINE` block.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Define {
    /// `MEASURE Table[Name] = scalar expression`
    Measure {
        table: String,
        table_span: Span,
        name: String,
        name_span: Span,
        expr: Expr,
        /// Source text of `expr`, including any parentheses around it.
        expr_span: Span,
        span: Span,
    },
    /// `COLUMN Table[Name] = scalar expression`
    Column {
        table: String,
        table_span: Span,
        name: String,
        name_span: Span,
        expr: Expr,
        /// Source text of `expr`, including any parentheses around it.
        expr_span: Span,
        span: Span,
    },
    /// `VAR name = expression`
    Var {
        name: String,
        name_span: Span,
        expr: Expr,
        /// Source text of `expr`, including any parentheses around it.
        expr_span: Span,
        span: Span,
    },
    /// `TABLE Name = table expression`
    Table {
        name: String,
        name_span: Span,
        expr: Expr,
        /// Source text of `expr`, including any parentheses around it.
        expr_span: Span,
        span: Span,
    },
    /// `FUNCTION Name = ( param [: hints] [= default], … ) => body` (user-defined function).
    Function {
        name: String,
        name_span: Span,
        params: Vec<FunctionParam>,
        body: Expr,
        /// Source text of `body`, including any parentheses around it.
        body_span: Span,
        span: Span,
    },
}

/// A user-defined function parameter, e.g. `amount : SCALAR NUMERIC VAL = 0`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FunctionParam {
    pub name: String,
    pub name_span: Span,
    /// Type hint words after `:` as written (type, subtype, passing mode).
    pub hints: Vec<TypeHint>,
    /// `= expression`: makes the parameter optional.
    pub default: Option<Expr>,
    /// Source text of `default`, including any parentheses around it.
    pub default_span: Option<Span>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TypeHint {
    pub word: String,
    pub span: Span,
}

/// `EVALUATE table [ORDER BY key, …] [START AT value, …]`
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EvaluateStatement {
    pub expr: Expr,
    /// Source text of `expr`, including any parentheses around it.
    pub expr_span: Span,
    pub order_by: Vec<OrderKey>,
    /// `START AT` values: constants or `@parameters`.
    pub start_at: Vec<Expr>,
    pub span: Span,
}

/// One `ORDER BY` key: `expression [ASC|DESC]`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OrderKey {
    pub expr: Expr,
    /// Source text of `expr`, including any parentheses around it.
    pub expr_span: Span,
    pub descending: bool,
    pub span: Span,
}

impl Define {
    pub fn span(&self) -> Span {
        match self {
            Define::Measure { span, .. }
            | Define::Column { span, .. }
            | Define::Var { span, .. }
            | Define::Table { span, .. }
            | Define::Function { span, .. } => *span,
        }
    }

    /// `measure`, `column`, `var`, `table` or `function`.
    pub fn kind(&self) -> &'static str {
        match self {
            Define::Measure { .. } => "measure",
            Define::Column { .. } => "column",
            Define::Var { .. } => "var",
            Define::Table { .. } => "table",
            Define::Function { .. } => "function",
        }
    }

    /// The defined name (without its table for measures and columns).
    pub fn name(&self) -> &str {
        match self {
            Define::Measure { name, .. }
            | Define::Column { name, .. }
            | Define::Var { name, .. }
            | Define::Table { name, .. }
            | Define::Function { name, .. } => name,
        }
    }

    pub fn name_span(&self) -> Span {
        match self {
            Define::Measure { name_span, .. }
            | Define::Column { name_span, .. }
            | Define::Var { name_span, .. }
            | Define::Table { name_span, .. }
            | Define::Function { name_span, .. } => *name_span,
        }
    }
}
