//! Type inference and evaluation-context checks (M6.2).
//!
//! Every expression node gets a [`ValueType`]. Function calls are typed and their arguments
//! checked through the typed catalog ([`crate::signature`]); a function the catalog does not type
//! yet is `Unknown` and its arguments are visited without any expectation, so a missing signature
//! never produces a diagnostic.
//!
//! Severities follow the engine: what the engine rejects when the object is defined is an error;
//! what only fails at run time for some data (a one-column table used as a scalar) is a warning or
//! nothing at all.
//!
//! Codes:
//! - `table-expected`: a scalar where a table expression is required (`COUNTROWS(1)`).
//! - `scalar-expected`: a table where a scalar is required. Error when the table has more than one
//!   column (the engine cannot convert it); warning when its width is unknown; silent for
//!   one-column tables (`VALUES(T[C])` as a scalar is a common, valid idiom).
//! - `column-ref-expected`: a column reference is required (`SUM([Measure])`, `SUM(1)`).
//! - `invalid-keyword`: a bare keyword outside the set the parameter accepts.
//! - `new-column-name-expected`: a new column's name must be a string literal.
//! - `no-row-context`: a bare column used as a value outside any row context over its table, or
//!   `RELATED` outside any row context.
//! - `no-outer-row-context`: `EARLIER`/`EARLIEST` without an outer row context.
//! - `filter-uses-measure`: a boolean `CALCULATE` filter references a measure or `CALCULATE`.
//! - `filter-multiple-tables`: a boolean `CALCULATE` filter references columns of several tables.
//! - `measure-returns-table` / `column-returns-table`: same severity rule as `scalar-expected`.
//! - `calculated-table-returns-scalar`: a calculated table whose expression is a scalar.
//!
//! Function-specific engine rules live in `typecheck_rules.rs`, the run-time errors reported as
//! warnings (`runtime-*`, G20) in `typecheck_runtime.rs`, and the filter-modifier placement rules
//! (G19) with the `SUMMARIZECOLUMNS` `NONVISUAL`/value rules in `typecheck_modifiers.rs`.

use std::collections::{HashMap, HashSet};

use bicraft_model::{Column, DataType, SemanticModel, Table};

use crate::ast::Expr;
use crate::binder::{Resolved, Scope, collect_declared_columns, resolve};
use crate::functions::{declares_columns, is_keyword_argument};
use crate::index::Names;
use crate::types::{ParamKind, Returns, ScalarType, Signature, ValueType};
use crate::{ExprKind, Raw, Span, lexer, parser};

pub(crate) type SignatureFn = fn(&str) -> Option<&'static Signature>;

/// Budget for nested inference: expression depth plus the depth of every measure or calculated
/// column inferred on the way. Beyond it types are `Unknown`, so a hostile chain of measures
/// cannot exhaust the stack.
const MAX_NESTING: usize = 192;

/// How many measures / calculated columns deep type inference follows references.
const MAX_OBJECT_NESTING: usize = 16;

/// Infers the type of `expr` and reports type and context diagnostics into `diags`.
///
/// `cache` carries the types of other model objects from one call to the next; it must only be
/// reused against the same, unchanged model. Diagnostics never depend on it: a measure or column
/// reference is always a scalar, and only its scalar subtype comes from the cache.
pub(crate) fn check<'m>(
    expr: &Expr,
    names: Option<Names<'m>>,
    home: Option<&'m str>,
    kind: ExprKind,
    sig: SignatureFn,
    mut cache: Option<&mut TypeCache>,
    diags: &mut Vec<Raw>,
) -> ValueType {
    let mut shared = Shared {
        names,
        sig,
        memo: HashMap::new(),
        active: HashSet::new(),
        depth: 0,
        // Moved in for the call and back after it, so `Shared` needs no second lifetime.
        cache: cache.as_deref_mut().map(std::mem::take),
        taint: 0,
        never: 0,
        current: None,
        warm: None,
    };
    let ty = {
        let mut c = Checker::new(&mut shared, expr, home, kind, true);
        let ty = c.root(expr);
        diags.append(&mut c.diags);
        ty
    };
    if let (Some(c), Some(own)) = (cache, shared.cache.take()) {
        *c = own;
    }
    ty
}

/// Types of model objects, kept across the expressions checked against one model snapshot.
///
/// Only *exact* types are kept: those inferred without reaching a cycle or a nesting budget,
/// which are the same whichever expression asked first. Objects whose dependencies reach a cycle
/// are inferred again for each checked expression, as without a cache.
#[derive(Debug, Default)]
pub(crate) struct TypeCache {
    /// Keyed by [`object_key`]: `Some` exact type, or `None` for an object that gets none (a
    /// cycle among its dependencies, or an expression too deep to infer).
    types: HashMap<String, Option<ValueType>>,
    /// Most inference attempts one warm-up may make (see [`Checker::warm_up`]).
    budget: usize,
}

impl TypeCache {
    fn exact(&self, key: &str) -> Option<ValueType> {
        self.types.get(key).copied().flatten()
    }

    pub fn new(model: &SemanticModel) -> Self {
        let objects = model.tables.iter().fold(0usize, |n, t| {
            n.saturating_add(t.measures.len()).saturating_add(t.columns.len())
        });
        TypeCache {
            budget: objects.saturating_mul(4).saturating_add(64),
            ..TypeCache::default()
        }
    }
}

struct Shared<'m> {
    names: Option<Names<'m>>,
    sig: SignatureFn,
    /// Inferred types of measures and calculated columns for this expression, keyed by
    /// [`object_key`], with how exact the type is (see [`TypeCache`]).
    memo: HashMap<String, (ValueType, Exactness)>,
    /// Objects being inferred right now (cycle protection).
    active: HashSet<String>,
    depth: usize,
    /// Cross-expression cache; `None` infers every object from scratch.
    cache: Option<TypeCache>,
    /// Counts the places where inference gave up (cycle, budget, inexact memo entry): an object
    /// whose inference leaves it unchanged has an exact type.
    taint: u64,
    /// Counts the places where inference reached a cycle or an object that has no exact type:
    /// an object whose inference changes it never gets an exact type either.
    never: u64,
    /// The innermost object being inferred (with a cache only).
    current: Option<Pending<'m>>,
    /// Set while one [`Checker::warm_up`] attempt runs.
    warm: Option<Warm<'m>>,
}

impl Shared<'_> {
    fn tainted(&mut self) {
        self.taint = self.taint.wrapping_add(1);
    }

    /// A cycle, or an object without an exact type: see [`Shared::never`].
    fn never_exact(&mut self) {
        self.tainted();
        self.never = self.never.wrapping_add(1);
        if let Some(w) = self.warm.as_mut() {
            w.cyclic = true;
        }
    }
}

/// How far a memoized type can be trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Exactness {
    /// Inferred without reaching a cycle or a budget: the same from any expression.
    Exact,
    /// A nesting budget ran out: depends on where the inference started.
    Inexact,
    /// A cycle (or an object without an exact type) was reached: never exact.
    Never,
}

/// A model object whose type is wanted.
#[derive(Debug, Clone)]
struct Pending<'m> {
    key: String,
    text: &'m str,
    home: &'m str,
    kind: ExprKind,
}

/// What one warm-up attempt ran into.
struct Warm<'m> {
    root: String,
    /// Objects the attempt could not enter because a nesting budget ran out.
    frontier: Vec<Pending<'m>>,
    /// The attempt reached a cycle, or an object without an exact type.
    cyclic: bool,
    /// The root's own expression is too deep to infer from here.
    stuck: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    Other,
    /// A reference to a model column.
    Column,
    /// A reference to a measure.
    Measure,
    /// A column the model does not define (declared by the expression, of a table variable, or
    /// unresolved), or any `[Name]` checked without a model.
    LocalColumn,
    /// A reference to a model table.
    TableName,
    /// A filter modifier (`USERELATIONSHIP`, `REMOVEFILTERS`, `KEEPFILTERS`…, G19): its type is the
    /// catalog's, but it is only meaningful as a filter argument (see `typecheck_modifiers.rs`).
    Modifier,
}

#[derive(Debug, Clone, Copy)]
struct Info {
    ty: ValueType,
    /// Number of columns, for table expressions when it is known.
    width: Option<usize>,
    shape: Shape,
    /// A table known to have more than one row (a table constructor with several rows): used as
    /// a scalar it fails at run time.
    multi_row: bool,
}

impl Info {
    const UNKNOWN: Info = Info {
        ty: ValueType::Unknown,
        width: None,
        shape: Shape::Other,
        multi_row: false,
    };

    fn scalar(t: ScalarType) -> Self {
        Info {
            ty: ValueType::Scalar(t),
            width: None,
            shape: Shape::Other,
            multi_row: false,
        }
    }

    fn table(width: Option<usize>) -> Self {
        Info {
            ty: ValueType::Table,
            width: width.filter(|&w| w > 0),
            shape: Shape::Other,
            multi_row: false,
        }
    }
}

#[derive(Debug, Clone)]
enum Row {
    /// A row context over this model table (upper-case name).
    Table(String),
    /// A row context whose table is not known statically, or a position whose expectations are
    /// unknown: every column is accepted.
    Unknown,
    /// A row context over some columns of one model table (`VALUES(T[C])`, `ALL(T[A], T[B])`):
    /// upper-case table and column names.
    Columns(String, Vec<String>),
    /// A boolean `CALCULATE` filter; collects the tables its columns belong to. `other` records a
    /// reference that is not a model column (a declared column or a measure).
    Filter { tables: Vec<String>, other: bool },
}

impl Row {
    /// The model table this row context iterates, when known.
    fn table(&self) -> Option<&str> {
        match self {
            Row::Table(t) | Row::Columns(t, _) => Some(t),
            Row::Unknown | Row::Filter { .. } => None,
        }
    }
}

struct Checker<'s, 'm> {
    sh: &'s mut Shared<'m>,
    home: Option<&'m str>,
    kind: ExprKind,
    report: bool,
    diags: Vec<Raw>,
    /// Variables in scope, innermost last (upper-case name, value info).
    vars: Vec<(String, Info)>,
    rows: Vec<Row>,
    declared: HashSet<String>,
    /// Scalar types known for certain of the variables in scope (parallel to `vars`).
    sure_vars: Vec<Option<ScalarType>>,
    /// `CURRENTGROUP()` calls placed where `GROUPBY` takes them.
    currentgroup_ok: Vec<Span>,
    /// Where the expressions being visited stand, for the filter-modifier rules.
    slots: Vec<(Span, modifiers::Slot)>,
    /// The calls being visited, outermost first (upper-case names).
    callers: Vec<String>,
}

fn object_key(prefix: char, table: &str, name: &str) -> String {
    format!(
        "{prefix}:{}\u{0}{}",
        table.to_ascii_uppercase(),
        name.to_ascii_uppercase()
    )
}

#[path = "typecheck_modifiers.rs"]
mod modifiers;
#[path = "typecheck_rules.rs"]
mod rules;
#[path = "typecheck_runtime.rs"]
mod runtime;

use rules::{bind_args, clause_name, takes_clauses};

pub(crate) fn quote_table(name: &str) -> String {
    format!("'{}'", name.replace('\'', "''"))
}

impl<'s, 'm> Checker<'s, 'm> {
    fn new(sh: &'s mut Shared<'m>, expr: &Expr, home: Option<&'m str>, kind: ExprKind, report: bool) -> Self {
        let mut declared = HashSet::new();
        collect_declared_columns(expr, &mut declared);
        Checker {
            sh,
            home,
            kind,
            report,
            diags: Vec::new(),
            vars: Vec::new(),
            rows: Vec::new(),
            declared,
            sure_vars: Vec::new(),
            currentgroup_ok: Vec::new(),
            slots: Vec::new(),
            callers: Vec::new(),
        }
    }

    fn error(&mut self, code: &'static str, span: Span, message: impl Into<String>) {
        if self.report {
            self.diags.push(Raw::error(code, span, message));
        }
    }

    fn warning(&mut self, code: &'static str, span: Span, message: impl Into<String>) {
        if self.report {
            self.diags.push(Raw::warning(code, span, message));
        }
    }

    /// Context rules depend on the kind of object; a bare expression is not checked for them.
    fn checks_context(&self) -> bool {
        self.kind != ExprKind::Expression
    }

    fn root(&mut self, expr: &Expr) -> ValueType {
        if matches!(self.kind, ExprKind::CalculatedColumn | ExprKind::RowFilter) {
            let base = match self.home.and_then(|h| self.sh.names?.table(h)) {
                Some(t) => Row::Table(t.name.to_ascii_uppercase()),
                None => Row::Unknown,
            };
            self.rows.push(base);
        }
        // A calculated table that is a bare column is a scalar used without a row context.
        let scalar = match self.kind {
            ExprKind::Measure | ExprKind::CalculatedColumn | ExprKind::RowFilter => true,
            ExprKind::CalculatedTable => matches!(expr, Expr::Column { .. }),
            ExprKind::Expression => false,
        };
        self.mark_slot(expr, modifiers::Slot::Root);
        let info = self.infer(expr, scalar);
        let span = expr.span();
        // A misplaced filter modifier is reported as such, not as a table result.
        let modifier = info.shape == Shape::Modifier;
        match self.kind {
            ExprKind::Measure => {
                if !modifier {
                    self.expect_scalar(info, span, "measure-returns-table", "a measure");
                }
                self.check_measure_result(expr);
            }
            ExprKind::CalculatedColumn if !modifier => {
                self.expect_scalar(info, span, "column-returns-table", "a calculated column");
            }
            ExprKind::CalculatedColumn => {}
            ExprKind::CalculatedTable => {
                if matches!(info.ty, ValueType::Scalar(_)) && info.shape != Shape::Column {
                    self.error(
                        "calculated-table-returns-scalar",
                        span,
                        "a calculated table must be defined by a table expression, not a scalar",
                    );
                }
                self.check_table_column_names(expr);
            }
            ExprKind::RowFilter => self.check_row_filter_result(expr, info),
            ExprKind::Expression => {}
        }
        info.ty
    }

    fn infer(&mut self, e: &Expr, scalar: bool) -> Info {
        if self.sh.depth >= MAX_NESTING {
            self.sh.tainted();
            if let Some(w) = self.sh.warm.as_mut() {
                match &self.sh.current {
                    Some(cur) if cur.key != w.root => w.frontier.push(cur.clone()),
                    _ => w.stuck = true,
                }
            }
            return Info::UNKNOWN;
        }
        self.sh.depth = self.sh.depth.saturating_add(1);
        let info = self.infer_inner(e, scalar);
        self.sh.depth = self.sh.depth.saturating_sub(1);
        info
    }

    fn infer_inner(&mut self, e: &Expr, scalar: bool) -> Info {
        match e {
            Expr::Number { text, .. } => {
                let integer = !text.contains(['.', 'e', 'E']) && text.parse::<i64>().is_ok();
                Info::scalar(if integer {
                    ScalarType::Integer
                } else {
                    ScalarType::Decimal
                })
            }
            Expr::Str { .. } => Info::scalar(ScalarType::String),
            Expr::Bool { .. } => Info::scalar(ScalarType::Boolean),
            Expr::Name { name, .. } => self.name(name),
            Expr::Column {
                table,
                name,
                variation,
                span,
                ..
            } => match variation {
                Some(v) => self.variation(table.as_deref(), name, &v.name, *span, scalar),
                None => self.column(table.as_deref(), name, *span, scalar),
            },
            Expr::Unary { op, operand, .. } => {
                let inner = self.infer(operand, true);
                self.expect_scalar(inner, operand.span(), "scalar-expected", "an operator");
                let target = if op.eq_ignore_ascii_case("NOT") {
                    ScalarType::Boolean
                } else {
                    ScalarType::Decimal
                };
                self.check_literal_conversion(operand, target, "an operator");
                if op.eq_ignore_ascii_case("NOT") {
                    Info::scalar(ScalarType::Boolean)
                } else {
                    Info::scalar(negate(inner.ty))
                }
            }
            Expr::Binary { op, lhs, rhs, span } => {
                if op.eq_ignore_ascii_case("IN") {
                    self.infer(lhs, true);
                    let r = self.infer(rhs, false);
                    self.check_in(lhs, rhs, r);
                    self.check_in_types(lhs, rhs, *span);
                    return Info::scalar(ScalarType::Boolean);
                }
                let l = self.infer(lhs, true);
                let r = self.infer(rhs, true);
                self.expect_scalar(l, lhs.span(), "scalar-expected", "an operator");
                self.expect_scalar(r, rhs.span(), "scalar-expected", "an operator");
                self.check_operands(op, lhs, rhs, *span);
                Info::scalar(binary_type(op, l.ty, r.ty))
            }
            Expr::Tuple { items, .. } => {
                for i in items {
                    self.infer(i, true);
                }
                Info::UNKNOWN
            }
            Expr::TableCtor { rows, span } => {
                let mut widths = Vec::with_capacity(rows.len());
                for r in rows {
                    if let Expr::Tuple { items, .. } = r {
                        widths.push(items.len());
                        for i in items {
                            self.infer(i, true);
                        }
                    } else {
                        widths.push(1);
                        self.infer(r, true);
                    }
                }
                let width = widths.iter().copied().max().unwrap_or(1);
                if widths.iter().any(|&w| w != width) {
                    self.error(
                        "ragged-table-constructor",
                        *span,
                        "every row of a table constructor must have the same number of columns",
                    );
                }
                Info {
                    multi_row: rows.len() > 1,
                    ..Info::table(Some(width))
                }
            }
            Expr::Var { bindings, body, span } => {
                let depth = self.vars.len();
                let slots = self.slots.len();
                for b in bindings {
                    self.mark_slot(&b.value, modifiers::Slot::Var);
                    let mut info = self.infer(&b.value, true);
                    // A variable holds a value, never a column or measure reference.
                    info.shape = Shape::Other;
                    let sure = if self.report {
                        self.sure_type(&b.value, 0)
                    } else {
                        None
                    };
                    self.vars.push((b.name.to_ascii_uppercase(), info));
                    self.sure_vars.push(sure);
                }
                self.inherit_slot(*span, body);
                let info = self.infer(body, scalar);
                self.vars.truncate(depth);
                self.sure_vars.truncate(depth);
                self.slots.truncate(slots);
                info
            }
            Expr::Call { name, args, span, .. } => self.call(name, args, *span),
        }
    }

    fn var(&self, name: &str) -> Option<Info> {
        self.vars
            .iter()
            .rev()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, i)| *i)
    }

    /// Whether the qualifier `t` of `t[c]` denotes a variable: a qualifier names a model table
    /// whenever one exists, even with a variable of that name in scope (engine verdicts
    /// `var-shadowing-007`, `-015`).
    pub(super) fn qualifier_is_var(&self, t: &str) -> bool {
        self.var(t).is_some() && self.sh.names.is_none_or(|m| m.table(t).is_none())
    }

    fn name(&mut self, name: &str) -> Info {
        // A variable wins over a table of the same name, even quoted, as in the engine.
        if let Some(v) = self.var(name) {
            return v;
        }
        match self.sh.names.and_then(|m| m.table(name)) {
            Some(t) => Info {
                ty: ValueType::Table,
                width: table_width(t),
                shape: Shape::TableName,
                multi_row: false,
            },
            None => Info::UNKNOWN,
        }
    }

    fn row_tables(&self) -> Vec<&str> {
        self.rows.iter().rev().filter_map(Row::table).collect()
    }

    fn column(&mut self, table: Option<&str>, name: &str, span: Span, scalar: bool) -> Info {
        let local = Info {
            ty: ValueType::Unknown,
            width: None,
            shape: Shape::LocalColumn,
            multi_row: false,
        };
        let Some(model) = self.sh.names else {
            return local;
        };
        let table_is_var = table.is_some_and(|t| self.qualifier_is_var(t));
        let scope = Scope {
            home: self.home,
            declared: Some(&self.declared),
            prefer_column: matches!(self.kind, ExprKind::CalculatedColumn | ExprKind::RowFilter),
        };
        let row_tables = self.row_tables();
        let resolved = resolve(model, scope, table, name, table_is_var, &row_tables);
        match resolved {
            Resolved::Column(t, c) => {
                if scalar {
                    self.check_row_context(t, c, span);
                }
                Info {
                    ty: ValueType::Scalar(self.column_type(t, c)),
                    width: None,
                    shape: Shape::Column,
                    multi_row: false,
                }
            }
            Resolved::Measure(t, m) => {
                if let Some(Row::Filter { other, .. }) = self.rows.last_mut() {
                    *other = true;
                    self.error(
                        "filter-uses-measure",
                        span,
                        format!(
                            "a boolean filter expression cannot reference the measure [{}]; \
                             compute it in a variable first or use FILTER",
                            m.name
                        ),
                    );
                }
                let ty = match self.measure_type(t, &m.name, &m.expression) {
                    ValueType::Scalar(s) => s,
                    _ => ScalarType::Variant,
                };
                Info {
                    ty: ValueType::Scalar(ty),
                    width: None,
                    shape: Shape::Measure,
                    multi_row: false,
                }
            }
            Resolved::Local | Resolved::Unresolved => {
                if let Some(Row::Filter { other, .. }) = self.rows.last_mut() {
                    *other = true;
                }
                local
            }
        }
    }

    /// `T[C].[V]`: the column `V` of the table `C`'s variation points to (the binder reports
    /// what does not resolve). A row context over `T` also gives it a value: the engine follows
    /// the variation's relationship, like `RELATED`.
    fn variation(&mut self, table: Option<&str>, name: &str, vname: &str, span: Span, scalar: bool) -> Info {
        let unknown = Info {
            ty: ValueType::Unknown,
            width: None,
            shape: Shape::LocalColumn,
            multi_row: false,
        };
        let Some(model) = self.sh.names else {
            return unknown;
        };
        let table_is_var = table.is_some_and(|t| self.qualifier_is_var(t));
        let scope = Scope {
            home: self.home,
            declared: Some(&self.declared),
            prefer_column: true,
        };
        let row_tables = self.row_tables();
        let Resolved::Column(t, c) = resolve(model, scope, table, name, table_is_var, &row_tables) else {
            return unknown;
        };
        let Some((vt, vc)) = model.model().variation_column(&t.name, &c.name, vname) else {
            return unknown;
        };
        if scalar {
            let reference = format!("{}[{}].[{}]", quote_table(&t.name), c.name, vc.name);
            self.check_row_context_of(vt, vc, Some((t, c)), &reference, span);
        }
        Info {
            ty: ValueType::Scalar(self.column_type(vt, vc)),
            width: None,
            shape: Shape::Column,
            multi_row: false,
        }
    }

    fn check_row_context(&mut self, t: &Table, c: &Column, span: Span) {
        let reference = format!("{}[{}]", quote_table(&t.name), c.name);
        self.check_row_context_of(t, c, None, &reference, span);
    }

    /// The column `t[c]` needs a row context over `t` (or, for a variation, over `base`'s table).
    fn check_row_context_of(
        &mut self,
        t: &Table,
        c: &Column,
        base: Option<(&Table, &Column)>,
        reference: &str,
        span: Span,
    ) {
        let upper = t.name.to_ascii_uppercase();
        if let Some(Row::Filter { tables, .. }) = self.rows.last_mut() {
            if !tables.contains(&upper) {
                tables.push(upper.clone());
            }
            return;
        }
        if !self.checks_context() {
            return;
        }
        if self.rows.is_empty() {
            self.error(
                "no-row-context",
                span,
                format!(
                    "a single value for column {reference} cannot be determined: there is no row \
                     context here (aggregate it, e.g. MAX({reference}), or use it inside an \
                     iterator such as SUMX or FILTER)"
                ),
            );
            return;
        }
        let column = c.name.to_ascii_uppercase();
        let base = base.map(|(bt, bc)| (bt.name.to_ascii_uppercase(), bc.name.to_ascii_uppercase()));
        let covers = |rt: &String, cols: Option<&Vec<String>>| {
            (*rt == upper && cols.is_none_or(|cs| cs.contains(&column)))
                || base
                    .as_ref()
                    .is_some_and(|(bt, bc)| rt == bt && cols.is_none_or(|cs| cs.contains(bc)))
        };
        let covered = self.rows.iter().any(|r| match r {
            Row::Table(rt) => covers(rt, None),
            Row::Columns(rt, cols) => covers(rt, Some(cols)),
            Row::Unknown | Row::Filter { .. } => true,
        });
        if !covered {
            self.error(
                "no-row-context",
                span,
                format!(
                    "a single value for column {reference} cannot be determined: no row context \
                     here iterates {} (use RELATED to follow a relationship, or aggregate it)",
                    quote_table(&t.name)
                ),
            );
        }
    }

    fn column_type(&mut self, t: &'m Table, c: &'m Column) -> ScalarType {
        match c.data_type {
            DataType::String => ScalarType::String,
            DataType::Int64 => ScalarType::Integer,
            DataType::Double => ScalarType::Decimal,
            DataType::Decimal => ScalarType::Currency,
            DataType::DateTime => ScalarType::DateTime,
            DataType::Boolean => ScalarType::Boolean,
            DataType::Binary | DataType::Variant => ScalarType::Variant,
            DataType::Unknown => match c.expression.as_deref() {
                Some(expr) => {
                    let key = object_key('C', &t.name, &c.name);
                    match self.infer_object(key, expr, &t.name, ExprKind::CalculatedColumn) {
                        ValueType::Scalar(s) => s,
                        _ => ScalarType::Variant,
                    }
                }
                None => ScalarType::Variant,
            },
        }
    }

    fn measure_type(&mut self, t: &'m Table, name: &str, expression: &'m str) -> ValueType {
        let key = object_key('M', &t.name, name);
        self.infer_object(key, expression, &t.name, ExprKind::Measure)
    }

    /// Infers the type of another model object's expression, memoized and cycle-safe.
    fn infer_object(&mut self, key: String, text: &'m str, home: &'m str, kind: ExprKind) -> ValueType {
        if let Some(&(t, exact)) = self.sh.memo.get(&key) {
            match exact {
                Exactness::Exact => {}
                Exactness::Inexact => self.sh.tainted(),
                Exactness::Never => self.sh.never_exact(),
            }
            return t;
        }
        if let Some(cache) = &self.sh.cache {
            match cache.types.get(&key) {
                Some(Some(t)) => return *t,
                Some(None) => {
                    if self.sh.warm.is_some() {
                        self.sh.never_exact();
                        return ValueType::Unknown;
                    }
                    // Otherwise inferred below, for this expression only.
                }
                None if self.sh.warm.is_none() && self.sh.depth < MAX_NESTING => {
                    self.warm_up(Pending {
                        key: key.clone(),
                        text,
                        home,
                        kind,
                    });
                    if let Some(t) = self.sh.cache.as_ref().and_then(|c| c.exact(&key)) {
                        return t;
                    }
                }
                None => {}
            }
        }
        if self.sh.active.contains(&key) {
            self.sh.never_exact();
            return ValueType::Unknown;
        }
        if self.sh.depth >= MAX_NESTING || self.sh.active.len() >= MAX_OBJECT_NESTING {
            self.sh.tainted();
            if let Some(w) = self.sh.warm.as_mut() {
                w.frontier.push(Pending {
                    key,
                    text,
                    home,
                    kind,
                });
            }
            return ValueType::Unknown;
        }
        let mut scratch = Vec::new();
        let tokens = lexer::lex(text, &mut scratch);
        let Some(expr) = parser::parse(&tokens, &mut scratch) else {
            if let Some(cache) = self.sh.cache.as_mut() {
                cache.types.insert(key.clone(), Some(ValueType::Unknown));
            }
            self.sh.memo.insert(key, (ValueType::Unknown, Exactness::Exact));
            return ValueType::Unknown;
        };
        self.sh.active.insert(key.clone());
        let (before, never_before) = (self.sh.taint, self.sh.never);
        // Only a warm-up attempt needs to know which object an expression belongs to.
        let outer = if self.sh.warm.is_some() {
            self.sh.current.replace(Pending {
                key: key.clone(),
                text,
                home,
                kind,
            })
        } else {
            None
        };
        let ty = {
            let mut sub = Checker::new(&mut *self.sh, &expr, Some(home), kind, false);
            sub.root(&expr)
        };
        self.sh.current = outer;
        self.sh.active.remove(&key);
        let exact = if self.sh.never != never_before {
            Exactness::Never
        } else if self.sh.taint != before {
            Exactness::Inexact
        } else {
            Exactness::Exact
        };
        if let Some(cache) = self.sh.cache.as_mut() {
            match exact {
                Exactness::Exact => {
                    cache.types.insert(key.clone(), Some(ty));
                }
                // Its dependencies reach a cycle: no point warming it up again.
                Exactness::Never => {
                    cache.types.insert(key.clone(), None);
                }
                Exactness::Inexact => {}
            }
        }
        self.sh.memo.insert(key, (ty, exact));
        ty
    }

    /// Gives `root` an exact type in the cache when it has one, typing its dependencies first.
    ///
    /// Inference nests one level per referenced object and stops at [`MAX_OBJECT_NESTING`], so a
    /// long chain of measures cannot be typed in one go. Each attempt starts with nothing being
    /// inferred, at the current depth. The objects an attempt could not enter (its frontier) are
    /// typed first, then the attempt is retried, until the root is exact or reaches a cycle (or
    /// an object without an exact type), in which case it is marked as given up. Every object an
    /// attempt completes without reaching a limit is cached, so the total work stays
    /// proportional to the number of objects.
    fn warm_up(&mut self, root: Pending<'m>) {
        let memo = std::mem::take(&mut self.sh.memo);
        let active = std::mem::take(&mut self.sh.active);
        let current = self.sh.current.take();
        let (taint, never) = (self.sh.taint, self.sh.never);
        let mut attempts = self.sh.cache.as_ref().map_or(0, |c| c.budget);
        let mut on_stack: HashSet<String> = HashSet::from([root.key.clone()]);
        let mut stack = vec![root];
        while let Some(top) = stack.last().cloned() {
            let Some(done) = self.sh.cache.as_ref().map(|c| c.types.contains_key(&top.key)) else {
                break;
            };
            if done {
                stack.pop();
                on_stack.remove(&top.key);
                continue;
            }
            let Some(left) = attempts.checked_sub(1) else {
                // Out of budget (not expected): the rest is inferred per expression.
                break;
            };
            attempts = left;
            self.sh.memo.clear();
            self.sh.active.clear();
            self.sh.warm = Some(Warm {
                root: top.key.clone(),
                frontier: Vec::new(),
                cyclic: false,
                stuck: false,
            });
            self.infer_object(top.key.clone(), top.text, top.home, top.kind);
            let Some(w) = self.sh.warm.take() else { break };
            let Some(cache) = self.sh.cache.as_mut() else {
                break;
            };
            if cache.types.contains_key(&top.key) {
                continue;
            }
            // A frontier object already on the stack depends on `top`, and `top` on it: a cycle.
            let mut give_up = w.cyclic
                || w.stuck
                || w.frontier.iter().any(|f| {
                    cache.types.get(&f.key) == Some(&None) || (f.key != top.key && on_stack.contains(&f.key))
                });
            if !give_up {
                let mut progress = false;
                for f in w.frontier {
                    if cache.exact(&f.key).is_some() {
                        // Typed later in the same attempt: the next attempt gets further.
                        progress = true;
                    } else if f.key != top.key && on_stack.insert(f.key.clone()) {
                        stack.push(f);
                        progress = true;
                    }
                }
                give_up = !progress;
            }
            if give_up {
                cache.types.insert(top.key.clone(), None);
            }
        }
        self.sh.warm = None;
        self.sh.memo = memo;
        self.sh.active = active;
        self.sh.current = current;
        // What the attempts ran into says nothing about the expression being checked.
        self.sh.taint = taint;
        self.sh.never = never;
    }

    fn signature(&self, name: &str) -> Option<&'static Signature> {
        (self.sh.sig)(name)
    }

    fn call(&mut self, name: &str, args: &[Option<Expr>], span: Span) -> Info {
        let up = name.to_ascii_uppercase();
        self.call_context_rules(&up, args, span);
        self.placement_rules(&up, args, span);
        self.modifier_rules(&up, args, span);
        let groups = self.currentgroup_ok.len();
        let slots = self.slots.len();
        self.allow_nested(&up, args);
        self.mark_arg_slots(&up, args, span);
        self.callers.push(up.clone());
        let mut info = self.call_inner(&up, name, args, span);
        self.callers.pop();
        self.currentgroup_ok.truncate(groups);
        self.slots.truncate(slots);
        if modifiers::modifier(&up, args).is_some() {
            info.shape = Shape::Modifier;
        }
        info
    }

    fn call_inner(&mut self, up: &str, name: &str, args: &[Option<Expr>], span: Span) -> Info {
        let up = up.to_string();
        let Some(sig) = self.signature(name) else {
            self.rows.push(Row::Unknown);
            for a in args.iter().flatten() {
                self.infer(a, false);
            }
            self.rows.pop();
            return Info::UNKNOWN;
        };
        let argc = args.len();
        let bound: Vec<Option<usize>> = bind_args(sig, args);
        let kinds: Vec<Option<ParamKind>> = bound
            .iter()
            .map(|p| p.and_then(|p| sig.params.get(p)).map(|p| p.kind))
            .collect();
        let mut infos: Vec<Option<Info>> = Vec::with_capacity(argc);
        for (i, arg) in args.iter().enumerate() {
            let Some(arg) = arg else {
                // An optional parameter may be skipped (`RANKX(t, e, , DESC)`); a repeating one
                // (`CALCULATE(e, , )`) may not.
                let param = bound.get(i).copied().flatten().and_then(|p| sig.params.get(p));
                // SAMPLEAXISWITHLOCALMINMAX's optional sections follow its repeating measures:
                // Desktop's line charts leave the dynamic series empty (`…, 350, , ALPHABETICAL, …`).
                if param.is_some_and(|p| p.repeating) && up != "SAMPLEAXISWITHLOCALMINMAX" {
                    self.error(
                        "empty-argument",
                        span,
                        format!("argument {} of {up} cannot be empty", i.saturating_add(1)),
                    );
                }
                infos.push(None);
                continue;
            };
            let kind = kinds.get(i).copied().flatten();
            let info = self.argument(&up, i, args, &kinds, kind, arg);
            infos.push(Some(info));
        }
        // Return-type indices are parameter indices: every argument bound to them counts. A
        // `BLANK()` branch takes the type of the others (`IF(c, BLANK(), [Currency])` is a
        // currency), as the engine types it.
        let of_params = |params: &[u8]| {
            let typed = |skip_blank: bool| {
                bound
                    .iter()
                    .zip(&infos)
                    .zip(args)
                    .filter(|((p, _), _)| p.is_some_and(|p| params.iter().any(|&q| usize::from(q) == p)))
                    .filter(move |(_, a)| !(skip_blank && a.as_ref().is_some_and(is_blank_call)))
                    .filter_map(|((_, info), _)| info.map(|i| i.ty))
            };
            if typed(true).next().is_some() {
                common_type(typed(true))
            } else {
                common_type(typed(false))
            }
        };
        let ty = match sig.returns {
            Returns::Fixed(t) => t,
            Returns::SameAsArg(i) => of_params(&[i]),
            Returns::CommonOfArgs(idx) => of_params(idx),
        };
        let ty = refine_return(&up, ty, args, &infos);
        self.call_shape_rules(&up, args, &kinds, &infos, span);
        match ty {
            ValueType::Table => Info::table(call_width(&up, args, &infos)),
            ty => Info {
                ty,
                width: None,
                shape: Shape::Other,
                multi_row: false,
            },
        }
    }

    /// Rules tied to specific functions rather than to their parameters.
    fn call_context_rules(&mut self, up: &str, args: &[Option<Expr>], span: Span) {
        match up {
            "CALCULATE" | "CALCULATETABLE" if matches!(self.rows.last(), Some(Row::Filter { .. })) => {
                self.error(
                    "filter-uses-measure",
                    span,
                    format!("{up} cannot be used inside a boolean filter expression"),
                );
            }
            "RELATED" if self.checks_context() && self.rows.is_empty() => {
                self.error(
                    "no-row-context",
                    span,
                    "RELATED needs a row context: use it inside an iterator (SUMX, FILTER…) or in \
                     a calculated column",
                );
            }
            "RELATED" if self.checks_context() => self.check_related(args, span),
            "USERELATIONSHIP" | "CROSSFILTER" => self.check_userelationship(up, args, span),
            "EARLIER" | "EARLIEST" if self.checks_context() => {
                if self.rows.iter().any(|r| matches!(r, Row::Unknown)) {
                    return;
                }
                let levels = if up == "EARLIER" {
                    match args.get(1) {
                        Some(Some(Expr::Number { text, .. })) => text.parse::<usize>().unwrap_or(1),
                        _ => 1,
                    }
                } else {
                    1
                };
                if self.rows.len() <= levels {
                    self.error(
                        "no-outer-row-context",
                        span,
                        format!(
                            "{up} refers to an outer row context that does not exist here \
                             ({} row context(s) are active)",
                            self.rows.len()
                        ),
                    );
                }
            }
            _ => {}
        }
    }

    /// `RELATED(T[C])` follows many-to-one relationships from a row context's table to `T`.
    fn check_related(&mut self, args: &[Option<Expr>], span: Span) {
        let Some(model) = self.sh.names else { return };
        let Some(Some(Expr::Column {
            table: Some(target),
            name,
            ..
        })) = args.first()
        else {
            return;
        };
        if self.var(target).is_some() {
            return;
        }
        let Some(target) = model.table(target).filter(|t| t.column(name).is_some()) else {
            return;
        };
        let mut sources = Vec::new();
        for r in &self.rows {
            match r.table() {
                Some(t) => sources.push(t.to_string()),
                // A row context of unknown table (or a filter) could be anything.
                None => return,
            }
        }
        let goal = target.name.to_ascii_uppercase();
        if sources.iter().any(|s| reaches_one_side(model.model(), s, &goal)) {
            return;
        }
        let from = sources
            .iter()
            .filter_map(|s| model.table(s))
            .map(|t| quote_table(&t.name))
            .collect::<Vec<_>>()
            .join(", ");
        self.error(
            "no-relationship",
            span,
            format!(
                "RELATED cannot reach {} from the row context over {from}: it follows many-to-one \
                 relationships only, to another table",
                quote_table(&target.name)
            ),
        );
    }

    /// `USERELATIONSHIP(a, b)` names an existing relationship, in either order.
    fn check_userelationship(&mut self, up: &str, args: &[Option<Expr>], span: Span) {
        let Some(model) = self.sh.names else { return };
        fn column<'a>(model: Names<'a>, a: Option<&Option<Expr>>) -> Option<(&'a str, &'a str)> {
            let Some(Some(Expr::Column {
                table: Some(t), name, ..
            })) = a
            else {
                return None;
            };
            let (table, column) = model.column(t, name)?;
            Some((table.name.as_str(), column.name.as_str()))
        }
        let (Some(a), Some(b)) = (column(model, args.first()), column(model, args.get(1))) else {
            return;
        };
        let same = |t: &str, c: &str, (xt, xc): (&str, &str)| {
            t.eq_ignore_ascii_case(xt) && c.eq_ignore_ascii_case(xc)
        };
        let exists = model.model().relationships.iter().any(|r| {
            (same(&r.from_table, &r.from_column, a) && same(&r.to_table, &r.to_column, b))
                || (same(&r.from_table, &r.from_column, b) && same(&r.to_table, &r.to_column, a))
        });
        if !exists {
            self.error(
                "no-relationship",
                span,
                format!(
                    "{up}: the model has no relationship between {}[{}] and {}[{}]",
                    quote_table(a.0),
                    a.1,
                    quote_table(b.0),
                    b.1
                ),
            );
        }
    }

    /// `x IN t`: `t` is a table with as many columns as `x` has values.
    fn check_in(&mut self, lhs: &Expr, rhs: &Expr, r: Info) {
        let left = match lhs {
            Expr::Tuple { items, .. } => items.len(),
            _ => 1,
        };
        if matches!(r.ty, ValueType::Scalar(_)) && !matches!(r.shape, Shape::Column | Shape::LocalColumn) {
            self.error(
                "table-expected",
                rhs.span(),
                "the right side of IN must be a table expression",
            );
            return;
        }
        let right = match rhs {
            Expr::TableCtor { rows, .. } if rows.is_empty() => Some(0),
            _ if r.ty == ValueType::Table => r.width,
            _ => None,
        };
        if let Some(right) = right
            && right != left
        {
            self.error(
                "in-arity-mismatch",
                rhs.span(),
                format!("IN compares {left} value(s) with a table of {right} column(s)"),
            );
        }
    }

    /// Rules about the shape of specific functions' arguments, checked once they are typed.
    fn call_shape_rules(
        &mut self,
        up: &str,
        args: &[Option<Expr>],
        kinds: &[Option<ParamKind>],
        infos: &[Option<Info>],
        span: Span,
    ) {
        // One value (CONTAINSROW) or one column (TREATAS) per column of the table (M11.40
        // signature probes).
        if matches!(up, "CONTAINSROW" | "TREATAS")
            && let Some(Some(table)) = infos.first()
            && table.ty == ValueType::Table
            && let Some(width) = table.width
        {
            let given = args.len().saturating_sub(1);
            if given != width {
                self.error(
                    "table-shape",
                    span,
                    format!("{up} needs one argument per column of its table ({width}), got {given}"),
                );
            }
        }
        // `MIN(x)` / `MAX(x)` with one argument aggregate a column.
        if matches!(up, "MIN" | "MAX")
            && args.len() == 1
            && let (Some(Some(arg)), Some(Some(info))) = (args.first(), infos.first())
        {
            self.expect_column(up, arg, *info, false);
        }
        // New column names must be unique within the call (and new to ADDCOLUMNS' table).
        let mut names: Vec<String> = Vec::new();
        if up == "ADDCOLUMNS"
            && let Some(Some(Expr::Name { name, .. })) = args.first()
            && self.var(name).is_none()
            && let Some(t) = self.sh.names.and_then(|m| m.table(name))
        {
            names.extend(
                t.columns
                    .iter()
                    .filter(|c| !c.is_row_number())
                    .map(|c| c.name.to_ascii_uppercase()),
            );
        }
        for (arg, kind) in args.iter().zip(kinds) {
            if let (Some(Expr::Str { value, span }), Some(ParamKind::NewColumnName)) = (arg, kind) {
                let upper = value.to_ascii_uppercase();
                if names.contains(&upper) {
                    self.error(
                        "duplicate-column-name",
                        *span,
                        format!("{up}: a column named \"{value}\" already exists in the result"),
                    );
                } else {
                    names.push(upper);
                }
            }
        }
        match up {
            "UNION" | "INTERSECT" | "EXCEPT" => {
                let widths: Vec<usize> = infos.iter().flatten().filter_map(|i| i.width).collect();
                if widths.windows(2).any(|w| w.first() != w.get(1)) {
                    self.error(
                        "table-width-mismatch",
                        span,
                        format!("{up} needs tables with the same number of columns"),
                    );
                }
            }
            "SELECTCOLUMNS" => {
                // (name, expression) pairs, or bare column references named after the column.
                let mut seen: Vec<String> = Vec::new();
                let mut i = 1;
                while let Some(arg) = args.get(i) {
                    let (name, at, step) = match arg {
                        Some(Expr::Str { value, span }) if i.saturating_add(1) < args.len() => {
                            (value.clone(), *span, 2)
                        }
                        Some(Expr::Column { name, span, .. }) => (name.clone(), *span, 1),
                        _ => break,
                    };
                    let upper = name.to_ascii_uppercase();
                    if seen.contains(&upper) {
                        self.error(
                            "duplicate-column-name",
                            at,
                            format!("{up}: a column named \"{name}\" already exists in the result"),
                        );
                    } else {
                        seen.push(upper);
                    }
                    i = i.saturating_add(step);
                }
            }
            "CROSSJOIN" | "GENERATE" | "GENERATEALL" => {
                let mut seen: Vec<(String, String)> = Vec::new();
                for arg in args.iter().flatten() {
                    let Some(cols) = self.lineage(arg) else { continue };
                    if let Some((t, c)) = cols.iter().find(|x| seen.contains(x)) {
                        let shown = self
                            .sh
                            .names
                            .and_then(|m| m.table(t))
                            .map_or_else(|| t.clone(), |x| quote_table(&x.name));
                        self.error(
                            "duplicate-column-name",
                            arg.span(),
                            format!("{up}: column {shown}[{c}] would appear twice in the result"),
                        );
                        break;
                    }
                    seen.extend(cols);
                }
            }
            "SUMMARIZE" => self.check_summarize(args),
            "NATURALINNERJOIN" | "NATURALLEFTOUTERJOIN" => self.check_natural_join(up, args, span),
            "GROUPBY" => self.check_groupby(args),
            "DATATABLE" => {
                if let Some(Some(data)) = args.last()
                    && let Some(bad) = first_non_constant(data)
                {
                    self.error(
                        "datatable-constant-expected",
                        bad,
                        "DATATABLE rows may only contain constant values",
                    );
                }
            }
            _ => {}
        }
        self.function_rules(up, args, span);
    }

    /// The model columns a table expression returns, when statically known (upper-case table and
    /// column names).
    fn lineage(&self, e: &Expr) -> Option<Vec<(String, String)>> {
        let model = self.sh.names?;
        let all = |t: &Table| {
            t.columns
                .iter()
                .filter(|c| !c.is_row_number())
                .map(|c| (t.name.to_ascii_uppercase(), c.name.to_ascii_uppercase()))
                .collect::<Vec<_>>()
        };
        let mut cur = e;
        loop {
            match cur {
                Expr::Name { name, .. } => {
                    if self.var(name).is_some() {
                        return None;
                    }
                    return model.table(name).map(all);
                }
                Expr::Call { name, args, .. } => {
                    let up = name.to_ascii_uppercase();
                    match up.as_str() {
                        "VALUES" | "DISTINCT" | "ALL" | "ALLNOBLANKROW" | "ALLSELECTED" => {
                            if let Some(Row::Columns(t, cols)) = self.columns_row(args) {
                                return Some(cols.into_iter().map(|c| (t.clone(), c)).collect());
                            }
                            match args.first() {
                                Some(Some(a @ Expr::Name { .. })) if args.len() == 1 => cur = a,
                                _ => return None,
                            }
                        }
                        "FILTER" | "CALCULATETABLE" | "KEEPFILTERS" => match args.first() {
                            Some(Some(a)) => cur = a,
                            _ => return None,
                        },
                        "TOPN" | "SAMPLE" => match args.get(1) {
                            Some(Some(a)) => cur = a,
                            _ => return None,
                        },
                        _ => return None,
                    }
                }
                _ => return None,
            }
        }
    }

    /// `SUMMARIZE(T, groupBy…)`: group-by columns come from `T` or tables it reaches through
    /// many-to-one relationships.
    fn check_summarize(&mut self, args: &[Option<Expr>]) {
        let Some(model) = self.sh.names else { return };
        let Some(Some(first)) = args.first() else { return };
        let Some(base) = self.row_of(first).table().map(str::to_string) else {
            return;
        };
        for arg in args.iter().skip(1).flatten() {
            let Expr::Column {
                table: Some(t),
                name,
                span,
                ..
            } = arg
            else {
                continue;
            };
            if self.qualifier_is_var(t) {
                continue;
            }
            let Some(tbl) = model.table(t).filter(|x| x.column(name).is_some()) else {
                continue;
            };
            let target = tbl.name.to_ascii_uppercase();
            if target != base && !reaches_one_side(model.model(), &base, &target) {
                self.error(
                    "no-relationship",
                    *span,
                    format!(
                        "SUMMARIZE can only group by columns of its table or of tables it reaches \
                         through many-to-one relationships; {}[{name}] is not one of them",
                        quote_table(&tbl.name)
                    ),
                );
            }
        }
    }

    fn argument(
        &mut self,
        fname: &str,
        i: usize,
        args: &[Option<Expr>],
        kinds: &[Option<ParamKind>],
        kind: Option<ParamKind>,
        arg: &Expr,
    ) -> Info {
        let span = arg.span();
        if let Some(clause) = clause_name(arg)
            && self.signature(fname).is_some_and(takes_clauses)
        {
            return self.window_clause(fname, args, arg, clause);
        }
        // GENERATE evaluates its second table once per row of the first.
        let generate_inner = i == 1 && matches!(fname, "GENERATE" | "GENERATEALL");
        match kind {
            // SUMMARIZE evaluates its expressions in a row context over the group-by columns;
            // GROUPBY's are checked by their own rule (aggregations over CURRENTGROUP()).
            Some(ParamKind::Scalar(_)) if matches!(fname, "SUMMARIZE" | "GROUPBY") => {
                let info = if fname == "SUMMARIZE" {
                    let row = self.group_by_row(args);
                    self.rows.push(row);
                    let info = self.infer(arg, true);
                    self.rows.pop();
                    info
                } else {
                    self.unchecked(arg)
                };
                self.expect_scalar(info, span, "scalar-expected", fname);
                info
            }
            Some(ParamKind::Scalar(t)) => {
                let info = self.infer(arg, true);
                self.expect_scalar(info, span, "scalar-expected", fname);
                if let Some(t) = t {
                    self.check_literal_conversion(arg, t, fname);
                }
                info
            }
            Some(ParamKind::RowExpression) if !generate_inner => {
                let row = self.iterated_row(i, args, kinds);
                self.rows.push(row);
                let info = self.infer(arg, true);
                self.rows.pop();
                self.expect_scalar(info, span, "scalar-expected", fname);
                info
            }
            Some(ParamKind::Table | ParamKind::TableRef | ParamKind::RowExpression) => {
                let pushed = if generate_inner {
                    let row = args
                        .first()
                        .and_then(Option::as_ref)
                        .map_or(Row::Unknown, |a| self.row_of(a));
                    self.rows.push(row);
                    true
                } else {
                    false
                };
                // A column reference is no table expression: the engine reads it as a value,
                // which needs a row context.
                let before = self.diags.len();
                let info = self.infer(arg, matches!(arg, Expr::Column { .. }));
                if pushed {
                    self.rows.pop();
                }
                let column_value = info.shape == Shape::Column && self.diags.len() == before;
                if column_value
                    || (matches!(info.ty, ValueType::Scalar(_))
                        && !matches!(info.shape, Shape::Column | Shape::LocalColumn))
                {
                    self.error(
                        "table-expected",
                        span,
                        format!("{fname} expects a table expression here, got a scalar value"),
                    );
                } else if kind == Some(ParamKind::TableRef)
                    && matches!(arg, Expr::Call { .. } | Expr::TableCtor { .. })
                {
                    // `RELATEDTABLE`, `ALLEXCEPT`, `ALLCROSSFILTERED` (M11.40 signature probes).
                    self.error(
                        "table-ref-expected",
                        span,
                        format!("{fname} expects a table reference here, not a table expression"),
                    );
                }
                info
            }
            Some(ParamKind::ColumnRef) => {
                let info = self.unchecked(arg);
                // These aggregations also take a measure reference (M11.40 signature probes).
                let measure_ok = matches!(
                    fname,
                    "AVERAGEA" | "MAXA" | "MINA" | "MEDIAN" | "PERCENTILE.INC" | "PERCENTILE.EXC"
                ) && info.shape == Shape::Measure;
                if !measure_ok {
                    self.expect_column(fname, arg, info, false);
                }
                info
            }
            Some(ParamKind::TableOrColumnRef) => {
                let info = self.unchecked(arg);
                self.expect_column(fname, arg, info, true);
                info
            }
            Some(ParamKind::Filter) => {
                // A text or number constant filters no column (M11.40 signature probes). A
                // year-end date may stand in the filter's place (`TOTALYTD(e, d, "6/30")`).
                let year_end = self
                    .signature(fname)
                    .is_some_and(|s| s.params.iter().any(|p| p.name == "year_end_date"));
                let constant = match arg {
                    Expr::Number { .. } => true,
                    Expr::Str { .. } => !year_end,
                    _ => false,
                };
                if constant {
                    self.error(
                        "filter-without-column",
                        span,
                        format!(
                            "{fname}: a filter argument must be a table or a boolean expression \
                             over a column, not a constant"
                        ),
                    );
                    return self.infer(arg, false);
                }
                if !self.is_boolean_filter(fname, arg) {
                    return self.infer(arg, false);
                }
                self.rows.push(Row::Filter {
                    tables: Vec::new(),
                    other: false,
                });
                let info = self.infer(arg, true);
                let Some(Row::Filter { tables, other }) = self.rows.pop() else {
                    return info;
                };
                if tables.is_empty() && !other {
                    self.error(
                        "filter-without-column",
                        span,
                        format!(
                            "{fname}: a boolean filter expression must reference a column (it \
                             filters that column's table)"
                        ),
                    );
                }
                if tables.len() > 1 {
                    let names = tables
                        .iter()
                        .map(|t| {
                            self.sh
                                .names
                                .and_then(|m| m.table(t))
                                .map_or_else(|| t.clone(), |x| quote_table(&x.name))
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.error(
                        "filter-multiple-tables",
                        span,
                        format!(
                            "a boolean filter expression can only reference columns of one table; \
                             this one references {names} (use FILTER, or one filter per table)"
                        ),
                    );
                }
                info
            }
            Some(ParamKind::NewColumnName) => {
                let info = self.unchecked(arg);
                // A column reference names the new column after itself (recent engines).
                let named = matches!(arg, Expr::Str { .. })
                    || (matches!(arg, Expr::Column { .. })
                        && matches!(info.shape, Shape::Column | Shape::LocalColumn));
                if !named {
                    self.error(
                        "new-column-name-expected",
                        span,
                        format!("{fname} expects the new column's name here, as a string literal"),
                    );
                }
                info
            }
            Some(ParamKind::Keyword(allowed)) => {
                let info = self.unchecked(arg);
                self.expect_keyword(fname, arg, allowed, false);
                info
            }
            Some(ParamKind::KeywordOrScalar(allowed)) => {
                let info = self.unchecked(arg);
                self.expect_keyword(fname, arg, allowed, true);
                info
            }
            Some(ParamKind::Dates) => self.dates_argument(fname, arg),
            Some(ParamKind::MeasureRef) => {
                let info = self.unchecked(arg);
                // The engine wants a measure reference (M11.40 signature probes).
                if !matches!(arg, Expr::Column { .. }) {
                    self.error(
                        "measure-ref-expected",
                        span,
                        format!("{fname} expects a measure reference ([Measure]) here"),
                    );
                }
                info
            }
            Some(ParamKind::TableOrColumn) => {
                let info = self.unchecked(arg);
                if is_literal(arg) {
                    self.error(
                        "table-expected",
                        span,
                        format!("{fname} expects a column reference or a table expression here"),
                    );
                }
                info
            }
            Some(
                kind @ (ParamKind::Relation
                | ParamKind::OrderBy
                | ParamKind::PartitionBy
                | ParamKind::MatchBy),
            ) => {
                let info = self.unchecked(arg);
                // A constant is never a relation or a window clause: the engine leaves it unused
                // or refuses it (M11.40 signature probes).
                if is_literal(arg) {
                    let what = match kind {
                        ParamKind::Relation => "a table expression (the relation)",
                        ParamKind::OrderBy => "an ORDERBY(…) clause",
                        ParamKind::PartitionBy => "a PARTITIONBY(…) clause",
                        _ => "a MATCHBY(…) clause",
                    };
                    self.error(
                        "window-argument-expected",
                        span,
                        format!("{fname} expects {what} here, not a constant"),
                    );
                }
                info
            }
            Some(ParamKind::Calendar) => {
                let info = self.unchecked(arg);
                // Week-based time intelligence takes only a calendar (M11.40 signature probes).
                if !self.may_be_calendar(Some(arg)) {
                    self.error(
                        "calendar-expected",
                        span,
                        format!("{fname} expects a calendar reference here"),
                    );
                }
                info
            }
            Some(ParamKind::Any) | None => self.unchecked(arg),
        }
    }

    /// Visits an argument whose expectations are unknown or not about values: no column in it is
    /// required to have a row context.
    fn unchecked(&mut self, arg: &Expr) -> Info {
        self.rows.push(Row::Unknown);
        let info = self.infer(arg, false);
        self.rows.pop();
        info
    }

    /// The row context an iterator's row expression runs in: that of the nearest preceding table
    /// argument.
    fn iterated_row(&self, i: usize, args: &[Option<Expr>], kinds: &[Option<ParamKind>]) -> Row {
        (0..i)
            .rev()
            .find_map(|j| match (kinds.get(j).copied().flatten(), args.get(j)) {
                (Some(ParamKind::Table | ParamKind::TableRef), Some(Some(a))) => Some(self.row_of(a)),
                _ => None,
            })
            .unwrap_or(Row::Unknown)
    }

    /// The table a table expression iterates, when it is statically known (a superset is fine: it
    /// only decides which columns may be used as values).
    fn row_of(&self, e: &Expr) -> Row {
        let mut cur = e;
        loop {
            match cur {
                Expr::Name { name, .. } => {
                    if self.var(name).is_some() {
                        return Row::Unknown;
                    }
                    return match self.sh.names.and_then(|m| m.table(name)) {
                        Some(t) => Row::Table(t.name.to_ascii_uppercase()),
                        None => Row::Unknown,
                    };
                }
                Expr::Column { table: Some(t), .. } => {
                    if self.qualifier_is_var(t) {
                        return Row::Unknown;
                    }
                    return match self.sh.names.and_then(|m| m.table(t)) {
                        Some(t) => Row::Table(t.name.to_ascii_uppercase()),
                        None => Row::Unknown,
                    };
                }
                Expr::Call { name, args, .. } => {
                    let up = name.to_ascii_uppercase();
                    if matches!(
                        up.as_str(),
                        "VALUES" | "DISTINCT" | "ALL" | "ALLNOBLANKROW" | "ALLSELECTED" | "ALLCROSSFILTERED"
                    ) && let Some(row) = self.columns_row(args)
                    {
                        return row;
                    }
                    let index = match up.as_str() {
                        "FILTER" | "CALCULATETABLE" | "ADDCOLUMNS" | "KEEPFILTERS" | "RELATEDTABLE"
                        | "ALL" | "ALLNOBLANKROW" | "ALLSELECTED" | "ALLEXCEPT" | "DISTINCT" | "VALUES"
                        | "ALLCROSSFILTERED" => 0,
                        "TOPN" | "SAMPLE" => 1,
                        _ => return Row::Unknown,
                    };
                    match args.get(index) {
                        Some(Some(a)) => cur = a,
                        _ => return Row::Unknown,
                    }
                }
                _ => return Row::Unknown,
            }
        }
    }

    /// `VALUES(T[A])`, `ALL(T[A], T[B])`: a row context over just those columns of `T`.
    fn columns_row(&self, args: &[Option<Expr>]) -> Option<Row> {
        let model = self.sh.names?;
        let mut table: Option<&Table> = None;
        let mut cols = Vec::new();
        for a in args {
            let Some(Expr::Column {
                table: Some(t),
                name,
                variation,
                ..
            }) = a
            else {
                return None;
            };
            if self.qualifier_is_var(t) {
                return None;
            }
            let (tbl, col) = match variation {
                // `VALUES(T[Date].[Year])` iterates the variation table's column.
                Some(v) => {
                    let base_table = model.table(t)?;
                    let base = base_table.column(name)?;
                    model
                        .model()
                        .variation_column(&base_table.name, &base.name, &v.name)?
                }
                None => {
                    let tbl = model.table(t)?;
                    (tbl, tbl.column(name)?)
                }
            };
            if table.is_some_and(|x| !std::ptr::eq(x, tbl)) {
                return None;
            }
            table = Some(tbl);
            cols.push(col.name.to_ascii_uppercase());
        }
        table.map(|t| Row::Columns(t.name.to_ascii_uppercase(), cols))
    }

    fn is_boolean_filter(&self, fname: &str, arg: &Expr) -> bool {
        match arg {
            // A constant is a (rejected) boolean filter in CALCULATE; elsewhere it may be another
            // optional argument (`TOTALYTD(e, dates, "6/30")`).
            Expr::Number { .. } | Expr::Str { .. } => matches!(fname, "CALCULATE" | "CALCULATETABLE"),
            // So is a scalar variable, even one named like a table (`VAR Customer = 2 RETURN
            // CALCULATE(…, 'Customer')`, engine verdict `var-shadowing-011`).
            Expr::Name { name, .. } => {
                matches!(fname, "CALCULATE" | "CALCULATETABLE")
                    && self
                        .var(name)
                        .is_some_and(|v| matches!(v.ty, ValueType::Scalar(_)))
            }
            Expr::Binary { .. }
            | Expr::Unary { .. }
            | Expr::Column { .. }
            | Expr::Bool { .. }
            | Expr::Var { .. } => true,
            Expr::Call { name, .. } => self.signature(name).is_some_and(|s| {
                matches!(
                    s.returns,
                    Returns::Fixed(ValueType::Scalar(_)) | Returns::CommonOfArgs(_)
                )
            }),
            _ => false,
        }
    }

    fn expect_scalar(&mut self, info: Info, span: Span, code: &'static str, what: &str) {
        if info.ty != ValueType::Table {
            return;
        }
        match info.width {
            Some(w) if w > 1 => self.error(
                code,
                span,
                format!(
                    "{what} needs a single value here, but this table expression has {w} columns; \
                     multiple columns cannot be converted to a scalar value"
                ),
            ),
            Some(_) => {
                if info.multi_row {
                    self.warning(
                        "runtime-multiple-values",
                        span,
                        format!(
                            "{what} needs a single value here, but this table has several rows:                              it fails at run time"
                        ),
                    );
                }
            }
            None => self.warning(
                code,
                span,
                format!(
                    "{what} needs a single value here, but this is a table expression; it fails at \
                     run time unless the table has one column and at most one row"
                ),
            ),
        }
    }

    fn expect_column(&mut self, fname: &str, arg: &Expr, info: Info, table_ok: bool) {
        let got = match arg {
            Expr::Column { .. } if info.shape == Shape::Measure => Some("a measure"),
            Expr::Column { .. } => None,
            Expr::Name { .. } if info.shape == Shape::TableName => (!table_ok).then_some("a table"),
            Expr::Name { .. } => match info.ty {
                ValueType::Scalar(_) => Some("a variable holding a value"),
                _ => None,
            },
            Expr::Call { .. } | Expr::Var { .. } => (!table_ok).then_some("an expression"),
            Expr::TableCtor { .. } => Some("a table constructor"),
            Expr::Number { .. }
            | Expr::Str { .. }
            | Expr::Bool { .. }
            | Expr::Unary { .. }
            | Expr::Binary { .. }
            | Expr::Tuple { .. } => Some("an expression"),
        };
        if let Some(got) = got {
            let expected = if table_ok {
                "a table or column reference"
            } else {
                "a column reference (Table[Column])"
            };
            self.error(
                "column-ref-expected",
                arg.span(),
                format!("{fname} expects {expected} here, got {got}"),
            );
        }
    }

    /// A keyword argument. With `scalar_ok` (sort orders) a number or a boolean stands for one;
    /// otherwise the engine refuses any constant (M11.40 signature probes).
    fn expect_keyword(&mut self, fname: &str, arg: &Expr, allowed: &[&str], scalar_ok: bool) {
        if !scalar_ok && is_literal(arg) && !matches!(arg, Expr::Str { .. }) {
            self.error(
                "invalid-keyword",
                arg.span(),
                format!(
                    "{fname} expects a bare keyword here, not a constant; expected one of {}",
                    allowed.join(", ")
                ),
            );
            return;
        }
        if let Expr::Str { value, span } = arg {
            self.error(
                "invalid-keyword",
                *span,
                format!(
                    "{fname} expects a bare keyword here, not the string \"{value}\"; expected one \
                     of {}",
                    allowed.join(", ")
                ),
            );
            return;
        }
        let Expr::Name {
            name,
            quoted: false,
            span,
        } = arg
        else {
            return;
        };
        if allowed.iter().any(|k| k.eq_ignore_ascii_case(name)) || self.var(name).is_some() {
            return;
        }
        // Names that are neither keywords nor tables are already reported by the binder.
        let known = is_keyword_argument(name) || self.sh.names.is_some_and(|m| m.table(name).is_some());
        if known {
            self.error(
                "invalid-keyword",
                *span,
                format!(
                    "{fname} does not accept `{name}` here; expected one of {}",
                    allowed.join(", ")
                ),
            );
        }
    }
}

/// A constant: a number (signed or not), a string or a boolean literal.
fn is_literal(e: &Expr) -> bool {
    match e {
        Expr::Number { .. } | Expr::Str { .. } | Expr::Bool { .. } => true,
        Expr::Unary { operand, .. } => matches!(**operand, Expr::Number { .. }),
        _ => false,
    }
}

/// Maps argument `i` (of `argc`) to the index of its parameter, following the catalog's
/// conventions: `params[..repeat_from]` is a fixed prefix, then a repeating unit (the run of
/// `repeating` params), then an optional non-repeating tail. A unit of several optional params is
/// matched by argument shape in the engine (`SUMMARIZECOLUMNS`), so its arguments are left
/// unmapped (`None`, unchecked), as are argument counts that fit no layout.
fn param_index(sig: &Signature, i: usize, argc: usize) -> Option<usize> {
    let params = sig.params;
    let Some(r) = sig.repeat_from.map(usize::from) else {
        return (i < params.len()).then_some(i);
    };
    if i < r {
        return (i < params.len()).then_some(i);
    }
    let rest = params.get(r..)?;
    let unit = rest.iter().take_while(|p| p.repeating).count();
    if unit == 0 {
        return (i < params.len()).then_some(i);
    }
    let (group, tail) = rest.split_at(unit);
    if group.len() > 1 && group.iter().any(|p| p.optional) {
        return None;
    }
    let region = argc.checked_sub(r)?;
    let required_tail = tail.iter().filter(|p| !p.optional).count();
    let t = (required_tail..=tail.len()).find(|&t| t <= region && (region - t) % group.len() == 0)?;
    let j = i.checked_sub(r)?;
    let in_group = region - t;
    if j < in_group {
        Some(r + j % group.len())
    } else {
        let k = j - in_group;
        (k < tail.len()).then_some(r + group.len() + k)
    }
}

/// Visible columns of a model table; `None` when the model lists none.
fn table_width(t: &Table) -> Option<usize> {
    let n = t.columns.iter().filter(|c| !c.is_row_number()).count();
    (n > 0).then_some(n)
}

/// Number of columns of a table-valued call, when it can be told from its arguments.
fn call_width(up: &str, args: &[Option<Expr>], infos: &[Option<Info>]) -> Option<usize> {
    let is_table = |i: &Info| i.ty == ValueType::Table;
    let is_column = |i: &Info| matches!(i.shape, Shape::Column | Shape::LocalColumn);
    let table_widths: Vec<Option<usize>> = infos
        .iter()
        .flatten()
        .filter(|i| is_table(i))
        .map(|i| i.width)
        .collect();
    let columns = infos.iter().flatten().filter(|i| is_column(i)).count();
    let names = args
        .iter()
        .flatten()
        .filter(|a| matches!(a, Expr::Str { .. }))
        .count();
    let first_table = || table_widths.first().copied().flatten();
    match up {
        "FILTER" | "CALCULATETABLE" | "KEEPFILTERS" | "RELATEDTABLE" | "TOPN" | "SAMPLE" | "UNION"
        | "INTERSECT" | "EXCEPT" | "DISTINCT" | "VALUES" | "ALL" | "ALLNOBLANKROW" | "ALLSELECTED"
        | "ALLCROSSFILTERED" | "TOPNSKIP" => {
            if table_widths.is_empty() {
                (columns > 0).then_some(columns)
            } else {
                first_table()
            }
        }
        "ADDCOLUMNS" => first_table().map(|w| w.saturating_add(names)),
        "CROSSJOIN" | "GENERATE" | "GENERATEALL" => table_widths
            .iter()
            .try_fold(0usize, |acc, w| w.map(|w| acc.saturating_add(w))),
        "ROW" | "DATATABLE" | "SELECTCOLUMNS" => (names > 0).then_some(names),
        "SUMMARIZE" | "GROUPBY" => {
            // The first argument is the table being grouped; it is not a column of the result.
            let skip_first = infos.first().copied().flatten().is_some_and(|i| is_column(&i));
            let n = columns
                .saturating_sub(usize::from(skip_first))
                .saturating_add(names);
            (n > 0).then_some(n)
        }
        "SUMMARIZECOLUMNS" => {
            let n = columns.saturating_add(names);
            (n > 0).then_some(n)
        }
        "GENERATESERIES" | "CALENDAR" | "CALENDARAUTO" => Some(1),
        _ if declares_columns(up) => None,
        // Time intelligence and similar: no table argument, the result has the columns given.
        _ if table_widths.is_empty() && columns > 0 => Some(columns),
        _ => None,
    }
}

fn numeric_rank(t: ScalarType) -> Option<u8> {
    match t {
        ScalarType::Boolean | ScalarType::Integer => Some(0),
        ScalarType::Currency => Some(1),
        ScalarType::Decimal | ScalarType::String | ScalarType::DateTime => Some(2),
        ScalarType::Variant => None,
    }
}

fn from_rank(r: u8) -> ScalarType {
    match r {
        0 => ScalarType::Integer,
        1 => ScalarType::Currency,
        _ => ScalarType::Decimal,
    }
}

fn negate(t: ValueType) -> ScalarType {
    match t {
        ValueType::Scalar(ScalarType::DateTime) => ScalarType::DateTime,
        ValueType::Scalar(s) => numeric_rank(s).map_or(ScalarType::Variant, from_rank),
        _ => ScalarType::Variant,
    }
}

fn binary_type(op: &str, l: ValueType, r: ValueType) -> ScalarType {
    match op {
        "&" => return ScalarType::String,
        "=" | "==" | "<>" | "<" | ">" | "<=" | ">=" | "&&" | "||" => return ScalarType::Boolean,
        _ => {}
    }
    let (ValueType::Scalar(a), ValueType::Scalar(b)) = (l, r) else {
        return ScalarType::Variant;
    };
    let (Some(ra), Some(rb)) = (numeric_rank(a), numeric_rank(b)) else {
        return ScalarType::Variant;
    };
    let date = a == ScalarType::DateTime || b == ScalarType::DateTime;
    match op {
        "+" | "-" if date => ScalarType::DateTime,
        "^" => ScalarType::Decimal,
        "/" if a == ScalarType::Currency && rb == 0 => ScalarType::Currency,
        "/" => ScalarType::Decimal,
        _ => from_rank(ra.max(rb)),
    }
}

/// `BLANK()`.
fn is_blank_call(e: &Expr) -> bool {
    matches!(e, Expr::Call { name, args, .. } if args.is_empty() && name.eq_ignore_ascii_case("BLANK"))
}

/// Result types the engine derives from the arguments, beyond the catalog's static return types
/// (checked against the engine's measure data types, see `docs/formats/bpa.md`): currency stays
/// currency through `AVERAGE`/`AVERAGEX`/`ROUND`/`INT`, `DIVIDE` types like `/`, and `CONVERT`
/// returns the type it names.
fn refine_return(up: &str, ty: ValueType, args: &[Option<Expr>], infos: &[Option<Info>]) -> ValueType {
    let arg_ty = |i: usize| infos.get(i).copied().flatten().map(|x| x.ty);
    let currency = |i: usize| arg_ty(i) == Some(ValueType::Scalar(ScalarType::Currency));
    match up {
        "AVERAGE" | "ROUND" | "INT" if currency(0) => ValueType::Scalar(ScalarType::Currency),
        "AVERAGEX" if currency(1) => ValueType::Scalar(ScalarType::Currency),
        "DIVIDE" => match (arg_ty(0), arg_ty(1)) {
            (Some(a @ ValueType::Scalar(_)), Some(b @ ValueType::Scalar(_))) => {
                match binary_type("/", a, b) {
                    ScalarType::Variant => ty,
                    s => ValueType::Scalar(s),
                }
            }
            _ => ty,
        },
        "CONVERT" => match args.get(1) {
            Some(Some(Expr::Name { name, .. })) => match name.to_ascii_uppercase().as_str() {
                "INTEGER" => ValueType::Scalar(ScalarType::Integer),
                "DOUBLE" => ValueType::Scalar(ScalarType::Decimal),
                "CURRENCY" => ValueType::Scalar(ScalarType::Currency),
                "STRING" => ValueType::Scalar(ScalarType::String),
                "BOOLEAN" => ValueType::Scalar(ScalarType::Boolean),
                "DATETIME" => ValueType::Scalar(ScalarType::DateTime),
                _ => ty,
            },
            _ => ty,
        },
        _ => ty,
    }
}

/// The common type of several branches (`IF`, `SWITCH`, `COALESCE`…).
fn common_type(types: impl Iterator<Item = ValueType>) -> ValueType {
    let mut acc: Option<ValueType> = None;
    for t in types {
        acc = Some(match acc {
            None => t,
            Some(a) if a == t => a,
            Some(ValueType::Scalar(x)) => match t {
                ValueType::Scalar(y) => match (numeric_rank(x), numeric_rank(y)) {
                    (Some(rx), Some(ry))
                        if x != ScalarType::String
                            && y != ScalarType::String
                            && x != ScalarType::DateTime
                            && y != ScalarType::DateTime =>
                    {
                        ValueType::Scalar(from_rank(rx.max(ry)))
                    }
                    _ => ValueType::Scalar(ScalarType::Variant),
                },
                _ => ValueType::Unknown,
            },
            Some(_) => ValueType::Unknown,
        });
    }
    acc.unwrap_or(ValueType::Unknown)
}

/// Whether `from` reaches `goal` (upper-case names) through relationships towards their one side
/// (both ways for one-to-one). The table itself does not count: RELATED needs a relationship.
fn reaches_one_side(model: &SemanticModel, from: &str, goal: &str) -> bool {
    let is = |c: &Option<String>, v: &str| c.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(v));
    let mut seen: HashSet<String> = HashSet::new();
    let mut stack = vec![from.to_ascii_uppercase()];
    while let Some(t) = stack.pop() {
        for r in &model.relationships {
            let (rf, rt) = (r.from_table.to_ascii_uppercase(), r.to_table.to_ascii_uppercase());
            // `to` is the one side unless declared many; `from` is one only when declared so.
            let next = if rf == t && !is(&r.to_cardinality, "many") {
                Some(rt)
            } else if rt == t && is(&r.from_cardinality, "one") {
                Some(rf)
            } else {
                None
            };
            if let Some(n) = next {
                if n == goal {
                    return true;
                }
                if seen.insert(n.clone()) {
                    stack.push(n);
                }
            }
        }
    }
    false
}

/// The span of the first value in DATATABLE data that is not a constant.
fn first_non_constant(data: &Expr) -> Option<Span> {
    let mut stack = vec![data];
    while let Some(e) = stack.pop() {
        match e {
            Expr::TableCtor { rows, .. } => stack.extend(rows.iter()),
            Expr::Tuple { items, .. } => stack.extend(items.iter()),
            Expr::Number { .. } | Expr::Str { .. } | Expr::Bool { .. } => {}
            Expr::Unary { op, operand, .. }
                if (op == "-" || op == "+") && matches!(**operand, Expr::Number { .. }) => {}
            Expr::Call { name, args, .. }
                if args.is_empty()
                    && ["BLANK", "TRUE", "FALSE"]
                        .iter()
                        .any(|n| n.eq_ignore_ascii_case(name)) => {}
            other => return Some(other.span()),
        }
    }
    None
}
