//! L2: DAX validation. `lex` → `parse` → `bind` (resolve tables, columns, measures, variables and
//! functions against a [`SemanticModel`]). Every stage reports [`Diagnostic`]s instead of failing,
//! so one call returns all the problems in an expression.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod ast;
mod binder;
pub mod deps;
pub mod format;
mod functions;
mod index;
pub mod info;
mod lexer;
mod parser;
mod query;
pub mod rename;
mod reserved;
mod typecheck;
pub mod types;
pub mod udf;
pub mod visual_calc;

pub use binder::format_string_ref;
pub use format::{FormatOptions, format as format_dax};
pub use functions::{
    Arity, all as all_functions, description as function_description, lookup as lookup_function, signature,
    signature_text,
};
pub use index::ModelIndex;
pub use lexer::{Token, TokenKind};
pub use parser::MAX_DEPTH;
pub use query::{parse_query, validate_query, validate_query_parsed};
pub use reserved::table_needs_quotes;

use bicraft_model::SemanticModel;

/// Whether `name` is an `INFO.*` function that only a DAX query may call: every one but the
/// four `INFO.VIEW.*` (Microsoft's DAX reference, "INFO functions": the `INFO.VIEW` functions run
/// in calculated tables, columns, measures and queries, the others only in queries; see
/// `docs/formats/dax-functions.md`, "INFO functions"). Case-insensitive.
pub fn is_query_only_info(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() > 5
        && b.get(..5).is_some_and(|p| p.eq_ignore_ascii_case(b"INFO."))
        && !b.get(5..10).is_some_and(|p| p.eq_ignore_ascii_case(b"VIEW."))
}
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

/// Byte range into the expression text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    pub severity: Severity,
    /// Stable machine-readable code, e.g. `unknown-column`.
    pub code: &'static str,
    pub message: String,
    pub span: Span,
    /// 1-based line and column (in characters) of `span.start`.
    pub line: usize,
    pub column: usize,
}

/// Where an expression lives; decides which references are legal.
#[derive(Debug, Clone, Default)]
pub struct Context<'a> {
    pub model: Option<&'a SemanticModel>,
    /// Table the expression belongs to (a calculated column's or measure's home table).
    pub table: Option<&'a str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Validation {
    pub valid: bool,
    pub diagnostics: Vec<Diagnostic>,
    /// The inferred type of the whole expression (`None` when it does not parse).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_type: Option<types::ValueType>,
}

/// What kind of model object an expression defines; decides the evaluation-context rules.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExprKind {
    /// A bare expression of unknown role: only the context-free rules apply.
    #[default]
    Expression,
    /// A measure: evaluated in a filter context only; must return a scalar.
    Measure,
    /// A calculated column: evaluated in a row context over its home table; must return a scalar.
    CalculatedColumn,
    /// A calculated table: no row context; must return a table.
    CalculatedTable,
    /// A row-level security filter: evaluated in a row context over its table.
    RowFilter,
}

/// Validates a DAX expression. Without a model only syntax, function arity and the context-free
/// type rules are checked. Same as [`validate_as`] with [`ExprKind::Expression`].
pub fn validate(text: &str, ctx: &Context<'_>) -> Validation {
    validate_as(text, ctx, ExprKind::Expression)
}

/// Validates a DAX expression as the definition of a `kind` object (`ctx.table` is its home
/// table): name resolution, argument types, and the evaluation-context rules for that kind.
pub fn validate_as(text: &str, ctx: &Context<'_>, kind: ExprKind) -> Validation {
    validate_with(text, ctx, kind, functions::signature)
}

/// [`validate`] against `base` and tables added after its own ([`index::Names::Overlay`]): the
/// same diagnostics as against a copy of `base` with `extra` pushed at its end.
pub(crate) fn validate_overlay(
    text: &str,
    base: &bicraft_model::SemanticModel,
    extra: &[bicraft_model::Table],
    table: Option<&str>,
) -> Validation {
    run(
        text,
        Some(index::Names::Overlay(base, extra)),
        table,
        ExprKind::Expression,
        functions::signature,
        None,
    )
}

pub(crate) fn validate_with(
    text: &str,
    ctx: &Context<'_>,
    kind: ExprKind,
    sig: typecheck::SignatureFn,
) -> Validation {
    let names = ctx.model.map(index::Names::Scan);
    run(text, names, ctx.table, kind, sig, None)
}

/// Validates many expressions against one model: every object of a model, or a batch of edits.
///
/// Gives the same diagnostics as [`validate_as`] with a [`Context`] on the same model, but builds
/// the name index once ([`ModelIndex`]) and keeps the inferred types of measures and calculated
/// columns from one expression to the next, so checking every object of a model costs about one
/// pass over its expressions instead of growing with the square of the number of measures.
///
/// The model is borrowed for the validator's lifetime, so it cannot change under the caches.
/// Result types can be more precise than [`validate_as`]'s for objects at the end of dependency
/// chains deeper than the per-expression nesting budget, which [`validate_as`] reports as
/// `Variant` or unknown.
#[derive(Debug)]
pub struct Validator<'m> {
    index: ModelIndex<'m>,
    types: typecheck::TypeCache,
}

impl<'m> Validator<'m> {
    pub fn new(model: &'m SemanticModel) -> Self {
        Validator {
            index: ModelIndex::new(model),
            types: typecheck::TypeCache::new(model),
        }
    }

    /// The validated model.
    pub fn model(&self) -> &'m SemanticModel {
        self.index.model()
    }

    /// The name index, for callers that resolve names themselves.
    pub fn index(&self) -> &ModelIndex<'m> {
        &self.index
    }

    /// [`validate`] against this validator's model, `table` being the expression's home table.
    pub fn validate(&mut self, text: &str, table: Option<&str>) -> Validation {
        self.validate_as(text, table, ExprKind::Expression)
    }

    /// [`validate_as`] against this validator's model, `table` being the expression's home table.
    pub fn validate_as(&mut self, text: &str, table: Option<&str>, kind: ExprKind) -> Validation {
        let names = Some(index::Names::Index(&self.index));
        run(
            text,
            names,
            table,
            kind,
            functions::signature,
            Some(&mut self.types),
        )
    }
}

fn run<'m>(
    text: &str,
    names: Option<index::Names<'m>>,
    table: Option<&'m str>,
    kind: ExprKind,
    sig: typecheck::SignatureFn,
    cache: Option<&mut typecheck::TypeCache>,
) -> Validation {
    let mut raw = Vec::new();
    let tokens = lexer::lex(text, &mut raw);
    let expr = parser::parse(&tokens, &mut raw);
    let mut result_type = None;
    if let Some(expr) = &expr {
        binder::bind(expr, names, table, &mut raw);
        result_type = Some(typecheck::check(expr, names, table, kind, sig, cache, &mut raw));
    }
    let diagnostics: Vec<Diagnostic> = raw.into_iter().map(|r| r.locate(text)).collect();
    let valid = expr.is_some() && diagnostics.iter().all(|d| d.severity != Severity::Error);
    Validation {
        valid,
        diagnostics,
        result_type,
    }
}

/// Parses an expression without binding it; returns the tree (if syntactically valid) and the
/// lexer/parser diagnostics.
pub fn parse_expression(text: &str) -> (Option<ast::Expr>, Vec<Diagnostic>) {
    let mut raw = Vec::new();
    let tokens = lexer::lex(text, &mut raw);
    let expr = parser::parse(&tokens, &mut raw);
    (expr, raw.into_iter().map(|r| r.locate(text)).collect())
}

/// The tokens of an expression, comments dropped, ending with [`TokenKind::Eof`]. Lexer errors
/// (an unterminated literal, an unexpected character) are not reported here: the token stream
/// is what the lexer recovered.
pub fn tokenize(text: &str) -> Vec<Token> {
    lexer::lex(text, &mut Vec::new())
}

/// A diagnostic before line/column resolution.
#[derive(Debug, Clone)]
pub(crate) struct Raw {
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
    pub span: Span,
}

impl Raw {
    pub fn error(code: &'static str, span: Span, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            code,
            message: message.into(),
            span,
        }
    }

    pub fn warning(code: &'static str, span: Span, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            code,
            message: message.into(),
            span,
        }
    }

    fn locate(self, text: &str) -> Diagnostic {
        let before = text.get(..self.span.start.min(text.len())).unwrap_or_default();
        let line = before.matches('\n').count().saturating_add(1);
        let line_start = before.rfind('\n').map_or(0, |i| i.saturating_add(1));
        let column = before
            .get(line_start..)
            .map_or(0, |s| s.chars().count())
            .saturating_add(1);
        Diagnostic {
            severity: self.severity,
            code: self.code,
            message: self.message,
            span: self.span,
            line,
            column,
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_query;

#[cfg(test)]
mod tests_types;

#[cfg(test)]
mod tests_deps;

#[cfg(test)]
mod tests_validator;

#[cfg(test)]
mod tests_variations;

#[cfg(test)]
mod tests_rules;

#[cfg(test)]
mod tests_rename;

#[cfg(test)]
mod tests_udf;

/// What `info-query-only` says about a call of `up` (upper case).
pub(crate) fn info_query_only_message(up: &str) -> String {
    format!(
        "{up} can only be used in a DAX query, not in a measure, calculated column, calculated \
         table or row filter; INFO.VIEW.TABLES, INFO.VIEW.COLUMNS, INFO.VIEW.MEASURES and \
         INFO.VIEW.RELATIONSHIPS can"
    )
}

/// The `info-query-only` errors of a model object's DAX (a measure, a calculated column or
/// table, a row filter, a calculation item): one per call of an `INFO.*` function other than
/// `INFO.VIEW.*`, the rule [`validate_as`] applies for those kinds. Empty when the text does not
/// parse (the parser's errors say why).
pub fn query_only_info_calls(text: &str) -> Vec<Diagnostic> {
    let (Some(expr), _) = parse_expression(text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut stack = vec![&expr];
    while let Some(e) = stack.pop() {
        match e {
            ast::Expr::Call { name, args, span, .. } => {
                if is_query_only_info(name) {
                    let up = name.to_ascii_uppercase();
                    out.push(Raw::error("info-query-only", *span, info_query_only_message(&up)).locate(text));
                }
                stack.extend(args.iter().flatten());
            }
            ast::Expr::Unary { operand, .. } => stack.push(operand),
            ast::Expr::Binary { lhs, rhs, .. } => {
                stack.push(lhs);
                stack.push(rhs);
            }
            ast::Expr::Var { bindings, body, .. } => {
                stack.extend(bindings.iter().map(|b| &b.value));
                stack.push(body);
            }
            ast::Expr::TableCtor { rows, .. } => stack.extend(rows.iter()),
            ast::Expr::Tuple { items, .. } => stack.extend(items.iter()),
            ast::Expr::Number { .. }
            | ast::Expr::Str { .. }
            | ast::Expr::Bool { .. }
            | ast::Expr::Name { .. }
            | ast::Expr::Column { .. } => {}
        }
    }
    out.sort_by_key(|d| d.span.start);
    out
}
