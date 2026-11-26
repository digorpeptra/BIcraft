//! Errors the engine only raises when an expression is evaluated, reported as warnings when they
//! can be told statically (G20). The definition is valid for the engine, so these never make an
//! expression invalid, with one exception: a comparison of incompatible types is a definition
//! error in calculated columns and calculated tables, whose types the engine fixes when it
//! defines them.
//!
//! Every warning code starts with `runtime-`:
//! - `runtime-incompatible-comparison` (error `incompatible-comparison` in calculated columns and
//!   tables): text compared with a number or a date, a boolean with a number… (`=`, `<`…, `IN`,
//!   `SWITCH`, `CONTAINS`).
//! - `runtime-conversion`: a text literal that cannot become the number, date or boolean it must
//!   be (`"a" + 1`, `YEAR("abc")`, `IF("x", …)`, `VALUE("twelve")`).
//! - `runtime-multiple-values`: a table of several rows used as a single value (`{1, 2} + 1`, a
//!   measure that is `VALUES(T[C])`).
//! - `runtime-invalid-argument`: a literal argument outside the function's domain (`MOD(x, 0)`,
//!   `SQRT(-1)`, `LEFT(t, -1)`…).
//! - `runtime-aggregation-type`: `SUM`/`AVERAGE`(`X`) over text or booleans.
//! - `runtime-dates-type`: a time-intelligence `dates` column that is not of type date, or dates
//!   from a table constructor.
//! - `runtime-non-boolean-filter`: a row-level security filter that does not return a boolean.

use super::*;

/// Value classes the engine compares: numbers (dates included), text, booleans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Number,
    Text,
    Boolean,
}

fn class(t: ScalarType) -> Option<Class> {
    match t {
        ScalarType::Integer | ScalarType::Decimal | ScalarType::Currency | ScalarType::DateTime => {
            Some(Class::Number)
        }
        ScalarType::String => Some(Class::Text),
        ScalarType::Boolean => Some(Class::Boolean),
        ScalarType::Variant => None,
    }
}

fn type_name(t: ScalarType) -> &'static str {
    match t {
        ScalarType::Integer => "Integer",
        ScalarType::Decimal => "Decimal",
        ScalarType::Currency => "Currency",
        ScalarType::DateTime => "Date",
        ScalarType::String => "Text",
        ScalarType::Boolean => "True/False",
        ScalarType::Variant => "Variant",
    }
}

fn data_type(t: DataType) -> Option<ScalarType> {
    match t {
        DataType::String => Some(ScalarType::String),
        DataType::Int64 => Some(ScalarType::Integer),
        DataType::Double => Some(ScalarType::Decimal),
        DataType::Decimal => Some(ScalarType::Currency),
        DataType::DateTime => Some(ScalarType::DateTime),
        DataType::Boolean => Some(ScalarType::Boolean),
        DataType::Binary | DataType::Variant | DataType::Unknown => None,
    }
}

/// A text the engine cannot convert to a number or a date: empty, or without any digit
/// (`"2024-01-01"` and `"12"` convert; `"x"` and `""` do not).
fn not_numeric(text: &str) -> bool {
    !text.bytes().any(|b| b.is_ascii_digit())
}

/// A text the engine cannot convert to a boolean (`""`, `"TRUE"`, `"1"` convert).
fn not_boolean(text: &str) -> bool {
    !text.is_empty()
        && !text.eq_ignore_ascii_case("TRUE")
        && !text.eq_ignore_ascii_case("FALSE")
        && not_numeric(text)
}

/// The value of a numeric literal (`3`, `-1.5`).
fn literal_number(e: &Expr) -> Option<f64> {
    match e {
        Expr::Number { text, .. } => text.parse().ok(),
        Expr::Unary { op, operand, .. } if op == "-" => literal_number(operand).map(|v| -v),
        Expr::Unary { op, operand, .. } if op == "+" => literal_number(operand),
        _ => None,
    }
}

const COMPARISONS: &[&str] = &["=", "<>", "<", ">", "<=", ">="];
const ARITHMETIC: &[&str] = &["+", "-", "*", "/", "^"];

impl Checker<'_, '_> {
    fn runtime(&mut self, code: &'static str, span: Span, message: String) {
        self.warning(code, span, format!("{message} (fails when evaluated)"));
    }

    /// The scalar type of `e` when it is certain: literals, typed model columns, measures with an
    /// inferred scalar type, variables bound to such values, functions with a fixed result type,
    /// and aggregations of a typed column.
    pub(super) fn sure_type(&mut self, e: &Expr, depth: usize) -> Option<ScalarType> {
        if depth > 32 {
            return None;
        }
        let next = depth.saturating_add(1);
        match e {
            Expr::Number { text, .. } => Some(
                if !text.contains(['.', 'e', 'E']) && text.parse::<i64>().is_ok() {
                    ScalarType::Integer
                } else {
                    ScalarType::Decimal
                },
            ),
            Expr::Str { .. } => Some(ScalarType::String),
            Expr::Bool { .. } => Some(ScalarType::Boolean),
            Expr::Unary { op, operand, .. } => {
                if op.eq_ignore_ascii_case("NOT") {
                    Some(ScalarType::Boolean)
                } else {
                    self.sure_type(operand, next).map(|_| ScalarType::Decimal)
                }
            }
            Expr::Binary { op, lhs, rhs, .. } => {
                if COMPARISONS.contains(&op.as_str())
                    || ["==", "&&", "||"].contains(&op.as_str())
                    || op.eq_ignore_ascii_case("IN")
                {
                    Some(ScalarType::Boolean)
                } else if op == "&" {
                    Some(ScalarType::String)
                } else {
                    // Arithmetic gives a number (or fails) once both operands are typed.
                    self.sure_type(lhs, next)?;
                    self.sure_type(rhs, next)?;
                    Some(ScalarType::Decimal)
                }
            }
            Expr::Column { .. } => {
                if let Some((_, c)) = self.model_column(e) {
                    return data_type(c.data_type);
                }
                let (t, m) = self.measure_of(e)?;
                match self.measure_type(t, &m.name, &m.expression) {
                    ValueType::Scalar(ScalarType::Variant) => None,
                    ValueType::Scalar(s) => Some(s),
                    _ => None,
                }
            }
            Expr::Name { name, .. } => {
                let i = self
                    .vars
                    .iter()
                    .rposition(|(n, _)| n.eq_ignore_ascii_case(name))?;
                self.sure_vars.get(i).copied().flatten()
            }
            Expr::Call { name, args, .. } => {
                let sig = self.signature(name)?;
                let slots = bind_args(sig, args);
                let bound = |params: &[u8]| -> Vec<&Expr> {
                    slots
                        .iter()
                        .zip(args)
                        .filter(|(p, _)| p.is_some_and(|p| params.iter().any(|&q| usize::from(q) == p)))
                        .filter_map(|(_, a)| a.as_ref())
                        .collect()
                };
                match sig.returns {
                    Returns::Fixed(ValueType::Scalar(ScalarType::Variant)) => None,
                    Returns::Fixed(ValueType::Scalar(t)) => Some(t),
                    Returns::SameAsArg(i) => {
                        let p = sig.params.get(usize::from(i))?;
                        if p.kind != ParamKind::ColumnRef {
                            return None;
                        }
                        match bound(&[i]).as_slice() {
                            [only] => self.sure_type(only, next),
                            _ => None,
                        }
                    }
                    Returns::CommonOfArgs(idx) => {
                        let mut types = Vec::new();
                        for a in bound(idx) {
                            types.push(self.sure_type(a, next));
                        }
                        let first = types.first().copied().flatten()?;
                        let c = class(first)?;
                        types
                            .iter()
                            .all(|t| t.and_then(class) == Some(c))
                            .then_some(first)
                    }
                    Returns::Fixed(_) => None,
                }
            }
            _ => None,
        }
    }

    /// `a` compared with `b`: values of different classes cannot be compared.
    fn check_comparison(&mut self, a: &Expr, b: &Expr, span: Span, what: &str) {
        let (Some(ta), Some(tb)) = (self.sure_type(a, 0), self.sure_type(b, 0)) else {
            return;
        };
        let (Some(ca), Some(cb)) = (class(ta), class(tb)) else {
            return;
        };
        if ca == cb {
            return;
        }
        let message = format!(
            "{what} cannot compare a {} value with a {} value; convert one side with VALUE or FORMAT",
            type_name(ta),
            type_name(tb)
        );
        if matches!(self.kind, ExprKind::CalculatedColumn | ExprKind::CalculatedTable) {
            self.error("incompatible-comparison", span, message);
        } else {
            self.runtime("runtime-incompatible-comparison", span, message);
        }
    }

    /// Checks of a binary operator's operands.
    pub(super) fn check_operands(&mut self, op: &str, lhs: &Expr, rhs: &Expr, span: Span) {
        // Diagnostics only: nothing to do while inferring another object's type.
        if !self.report {
            return;
        }
        if COMPARISONS.contains(&op) {
            self.check_comparison(lhs, rhs, span, "a comparison");
        } else if ARITHMETIC.contains(&op) {
            for e in [lhs, rhs] {
                self.check_literal_conversion(e, ScalarType::Decimal, "an arithmetic operator");
                self.check_text_operand(e, "a number", "an arithmetic operator");
            }
        } else if op == "&&" || op == "||" {
            for e in [lhs, rhs] {
                self.check_literal_conversion(e, ScalarType::Boolean, "a logical operator");
                self.check_text_operand(e, "True/False", "a logical operator");
            }
        }
    }

    /// A text value (not a literal: those are checked exactly) used where a number or a boolean
    /// is needed is converted value by value, and fails on the first value that does not convert.
    fn check_text_operand(&mut self, e: &Expr, to: &str, what: &str) {
        if matches!(e, Expr::Str { .. }) || self.sure_type(e, 0) != Some(ScalarType::String) {
            return;
        }
        self.warning(
            "runtime-conversion",
            e.span(),
            format!(
                "{what}: text values are converted to {to} when evaluated, and any value that does                  not convert makes the expression fail"
            ),
        );
    }

    /// `x IN { … }`: every value of the table constructor is compared with `x`.
    pub(super) fn check_in_types(&mut self, lhs: &Expr, rhs: &Expr, span: Span) {
        // Diagnostics only: nothing to do while inferring another object's type.
        if !self.report {
            return;
        }
        let Expr::TableCtor { rows, .. } = rhs else {
            return;
        };
        let left: Vec<&Expr> = match lhs {
            Expr::Tuple { items, .. } => items.iter().collect(),
            other => vec![other],
        };
        for row in rows {
            let right: Vec<&Expr> = match row {
                Expr::Tuple { items, .. } => items.iter().collect(),
                other => vec![other],
            };
            for (a, b) in left.iter().zip(&right) {
                let before = self.diags.len();
                self.check_comparison(a, b, span, "IN");
                if self.diags.len() != before {
                    return;
                }
            }
        }
    }

    /// A text literal where a number, date or boolean is expected must convert.
    pub(super) fn check_literal_conversion(&mut self, arg: &Expr, target: ScalarType, what: &str) {
        // Diagnostics only: nothing to do while inferring another object's type.
        if !self.report {
            return;
        }
        let Expr::Str { value, span } = arg else {
            return;
        };
        let (bad, to) = match target {
            ScalarType::Integer | ScalarType::Decimal | ScalarType::Currency => {
                (not_numeric(value), "a number")
            }
            ScalarType::DateTime => (not_numeric(value), "a date"),
            ScalarType::Boolean => (not_boolean(value), "a boolean"),
            ScalarType::String | ScalarType::Variant => (false, ""),
        };
        if bad {
            self.runtime(
                "runtime-conversion",
                *span,
                format!("{what}: the text \"{value}\" cannot be converted to {to}"),
            );
        }
    }

    /// A time-intelligence `dates` column must be of type date.
    pub(super) fn check_dates_type(&mut self, fname: &str, arg: &Expr) {
        let column = match arg {
            Expr::Column { .. } => Some(arg),
            Expr::Call { name, args, .. }
                if (name.eq_ignore_ascii_case("VALUES") || name.eq_ignore_ascii_case("DISTINCT"))
                    && args.len() == 1 =>
            {
                args.first().and_then(Option::as_ref)
            }
            Expr::TableCtor { span, .. } => {
                self.runtime(
                    "runtime-dates-type",
                    *span,
                    format!("{fname} cannot work with the columns of a table constructor; use a date column"),
                );
                return;
            }
            _ => None,
        };
        let Some((t, c)) = column.and_then(|c| self.model_column(c)) else {
            return;
        };
        if let Some(ty) = data_type(c.data_type)
            && ty != ScalarType::DateTime
        {
            self.runtime(
                "runtime-dates-type",
                arg.span(),
                format!(
                    "{fname} needs a column of dates; {}[{}] is of type {}",
                    quote_table(&t.name),
                    c.name,
                    type_name(ty)
                ),
            );
        }
    }

    /// A measure that is a whole column's values (`VALUES(T[C])`) fails whenever more than one
    /// value is visible, e.g. at the grand total.
    pub(super) fn check_measure_result(&mut self, expr: &Expr) {
        let mut cur = expr;
        let mut budget = 64usize;
        while let Expr::Var { body, .. } = cur {
            let Some(b) = budget.checked_sub(1) else { return };
            budget = b;
            cur = body;
        }
        let Expr::Call { name, args, span, .. } = cur else {
            return;
        };
        let up = name.to_ascii_uppercase();
        if matches!(up.as_str(), "VALUES" | "DISTINCT" | "ALL")
            && args.len() == 1
            && matches!(args.first(), Some(Some(Expr::Column { .. })))
        {
            self.runtime(
                "runtime-multiple-values",
                *span,
                format!(
                    "the measure returns {up}(…), a table of values: it fails whenever more than \
                     one value is visible (e.g. at the grand total); use SELECTEDVALUE"
                ),
            );
        }
    }

    /// A row-level security filter must return a boolean.
    pub(super) fn check_row_filter_result(&mut self, expr: &Expr, info: Info) {
        let ty = match info.ty {
            ValueType::Scalar(t) => Some(t),
            _ => None,
        }
        .or_else(|| self.sure_type(expr, 0));
        if let Some(t) = ty
            && class(t).is_some_and(|c| c != Class::Boolean)
        {
            self.runtime(
                "runtime-non-boolean-filter",
                expr.span(),
                format!(
                    "a row-level security filter must return True/False, not a {} value",
                    type_name(t)
                ),
            );
        }
    }

    /// Function-specific runtime checks on literal arguments and argument types.
    pub(super) fn runtime_function_rules(&mut self, up: &str, args: &[Option<Expr>], span: Span) {
        // Diagnostics only: nothing to do while inferring another object's type.
        if !self.report {
            return;
        }
        let arg = |i: usize| args.get(i).and_then(Option::as_ref);
        let num = |i: usize| arg(i).and_then(literal_number);
        let domain = match up {
            "MOD" | "QUOTIENT" => num(1).filter(|&v| v == 0.0).map(|_| (1, "division by zero")),
            "SQRT" => num(0)
                .filter(|&v| v < 0.0)
                .map(|_| (0, "the square root of a negative number")),
            "LN" | "LOG10" | "LOG" => num(0)
                .filter(|&v| v <= 0.0)
                .map(|_| (0, "the logarithm of a number that is not positive")),
            "LEFT" | "RIGHT" => num(1)
                .filter(|&v| v < 0.0)
                .map(|_| (1, "a negative number of characters")),
            "REPT" => num(1)
                .filter(|&v| v < 0.0)
                .map(|_| (1, "a negative number of repetitions")),
            "MID" => {
                if num(1).is_some_and(|v| v < 1.0) {
                    Some((1, "a start position before the first character"))
                } else {
                    num(2)
                        .filter(|&v| v < 0.0)
                        .map(|_| (2, "a negative number of characters"))
                }
            }
            _ => None,
        };
        if let Some((i, what)) = domain
            && let Some(a) = arg(i)
        {
            self.runtime("runtime-invalid-argument", a.span(), format!("{up}: {what}"));
        }
        match up {
            "SUM" | "AVERAGE" | "SUMX" | "AVERAGEX" => {
                let i = usize::from(up.ends_with('X'));
                if let Some(a) = arg(i)
                    && let Some(t) = self.sure_type(a, 0)
                    && matches!(t, ScalarType::String | ScalarType::Boolean)
                {
                    self.runtime(
                        "runtime-aggregation-type",
                        a.span(),
                        format!("{up} cannot work with values of type {}", type_name(t)),
                    );
                }
            }
            "GENERATESERIES" => {
                for a in args.iter().take(2).flatten() {
                    if self.sure_type(a, 0) == Some(ScalarType::String) {
                        self.runtime(
                            "runtime-conversion",
                            a.span(),
                            "GENERATESERIES needs numbers or dates, not text".to_string(),
                        );
                        break;
                    }
                }
            }
            "VALUE" | "DATEVALUE" | "TIMEVALUE" => {
                let target = if up == "VALUE" {
                    ScalarType::Decimal
                } else {
                    ScalarType::DateTime
                };
                if let Some(a) = arg(0) {
                    self.check_literal_conversion(a, target, up);
                }
            }
            "CONVERT" => {
                let target = match arg(1) {
                    Some(Expr::Name { name, .. }) => match name.to_ascii_uppercase().as_str() {
                        "INTEGER" | "INT64" | "DOUBLE" | "DECIMAL" | "CURRENCY" => Some(ScalarType::Decimal),
                        "DATETIME" => Some(ScalarType::DateTime),
                        "BOOLEAN" | "LOGICAL" => Some(ScalarType::Boolean),
                        _ => None,
                    },
                    _ => None,
                };
                if let (Some(t), Some(a)) = (target, arg(0)) {
                    self.check_literal_conversion(a, t, up);
                }
            }
            "SWITCH" => {
                if let (Some(sig), Some(subject)) = (self.signature(up), arg(0)) {
                    let argc = args.len();
                    for i in 1..argc {
                        if bind_args(sig, args).get(i).copied().flatten() == Some(1)
                            && let Some(v) = arg(i)
                        {
                            self.check_comparison(subject, v, v.span(), "SWITCH");
                        }
                    }
                }
            }
            "CONTAINS" => {
                let mut i = 1;
                while let (Some(c), Some(v)) = (arg(i), arg(i.saturating_add(1))) {
                    self.check_comparison(c, v, span, "CONTAINS");
                    i = i.saturating_add(2);
                }
            }
            _ => {}
        }
    }
}
