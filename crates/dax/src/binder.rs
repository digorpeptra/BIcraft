//! Name resolution: checks every reference in a parsed expression against the semantic model.
//! Function arity is checked with or without a model.

use std::collections::HashSet;

use bicraft_model::{Column, Measure, Table};

use crate::Raw;
use crate::ast::{Expr, Variation};
use crate::functions::{declares_columns, is_keyword_argument, lookup};
use crate::index::Names;
use crate::typecheck::quote_table;

/// Binds `expr` against `names` (no model: arity and variables only); `home` is its home table.
pub fn bind(expr: &Expr, names: Option<Names<'_>>, home: Option<&str>, diags: &mut Vec<Raw>) {
    let mut declared = HashSet::new();
    collect_declared_columns(expr, &mut declared);
    let mut b = Binder {
        names,
        home,
        scopes: Vec::new(),
        declared,
        diags,
    };
    b.visit(expr);
}

struct Binder<'c, 'm, 'd> {
    names: Option<Names<'m>>,
    home: Option<&'c str>,
    /// Variables in scope, innermost last (stored upper-case).
    scopes: Vec<String>,
    /// Column names introduced by ADDCOLUMNS & co. or by table constructors (upper-case).
    declared: HashSet<String>,
    diags: &'d mut Vec<Raw>,
}

impl Binder<'_, '_, '_> {
    fn is_var(&self, name: &str) -> bool {
        let up = name.to_ascii_uppercase();
        self.scopes.contains(&up)
    }

    fn visit(&mut self, expr: &Expr) {
        match expr {
            Expr::Number { .. } | Expr::Str { .. } | Expr::Bool { .. } => {}
            Expr::Unary { operand, .. } => self.visit(operand),
            Expr::Binary { lhs, rhs, .. } => {
                self.visit(lhs);
                self.visit(rhs);
            }
            Expr::Tuple { items, .. } => items.iter().for_each(|e| self.visit(e)),
            Expr::TableCtor { rows, .. } => rows.iter().for_each(|e| self.visit(e)),
            Expr::Var { bindings, body, .. } => {
                let depth = self.scopes.len();
                for b in bindings {
                    // A variable is visible only after its own definition.
                    self.visit(&b.value);
                    let up = b.name.to_ascii_uppercase();
                    if self.scopes.get(depth..).is_some_and(|s| s.contains(&up)) {
                        self.diags.push(Raw::error(
                            "duplicate-variable",
                            b.name_span,
                            format!("variable `{}` is defined twice in the same VAR block", b.name),
                        ));
                    }
                    self.scopes.push(up);
                }
                self.visit(body);
                self.scopes.truncate(depth);
            }
            Expr::Call {
                name,
                name_span,
                args,
                span,
            } => {
                self.check_call(name, *name_span, args.len(), *span);
                if lookup(name).is_none()
                    && let Some(names) = self.names
                    && model_function(names.model(), name)
                {
                    let mut found = Vec::new();
                    let scopes = &self.scopes;
                    let is_var = |n: &str| scopes.contains(&n.to_ascii_uppercase());
                    crate::udf::check_call(names, name, args, *span, &is_var, &mut found);
                    self.diags.extend(found);
                }
                args.iter().flatten().for_each(|a| self.visit(a));
            }
            Expr::Name { name, quoted, span } => self.check_name(name, *quoted, *span),
            Expr::Column {
                table,
                table_span,
                name,
                variation,
                span,
            } => {
                let before = self.diags.len();
                self.check_column(table.as_deref(), *table_span, name, *span);
                if let Some(v) = variation
                    && self.diags.len() == before
                {
                    self.check_variation(table.as_deref(), name, v);
                }
            }
        }
    }

    /// `T[C].[V]`: `C` must be a column with variations, and `V` a column of the table its
    /// variation points to (the engine resolves the name against that table's columns).
    fn check_variation(&mut self, table: Option<&str>, name: &str, v: &Variation) {
        let Some(names) = self.names else { return };
        let target = match table {
            Some(t) => names.table_at(t).and_then(|tbl| {
                if names.column_of(tbl, name).is_none() && names.measure_of(tbl, name).is_some() {
                    return Some(Err(tbl.table.name.clone()));
                }
                names.column_of(tbl, name).map(|c| Ok((tbl.table, c)))
            }),
            None => {
                let column = self
                    .home
                    .and_then(|t| names.column(t, name))
                    .or_else(|| names.any_column(name));
                match (column, names.measure(name)) {
                    (Some(c), _) => Some(Ok(c)),
                    (None, Some((t, _))) => Some(Err(t.name.clone())),
                    (None, None) => None,
                }
            }
        };
        let (t, c) = match target {
            Some(Ok(found)) => found,
            Some(Err(measure_table)) => {
                self.diags.push(Raw::error(
                    "variation-on-measure",
                    v.span,
                    format!(
                        "[{name}] is a measure of {}: the variation notation .[{}] applies to columns only",
                        quote_table(&measure_table),
                        v.name
                    ),
                ));
                return;
            }
            // Local or unknown: the binder already reported an unknown name, or the column is
            // declared by the expression itself and cannot have variations.
            None => return,
        };
        let variations = c.variations();
        let model = names.model();
        let reference = format!("{}[{}]", quote_table(&t.name), c.name);
        if variations.is_empty() {
            self.diags.push(Raw::error(
                "column-without-variations",
                v.span,
                format!(
                    "{reference} has no variations, so .[{}] cannot be resolved (variations come \
                     from auto date/time: the column needs a `variations` entry pointing to its \
                     local date table)",
                    v.name
                ),
            ));
            return;
        }
        if model.variation_column(&t.name, &c.name, &v.name).is_some() {
            return;
        }
        let tables: Vec<&bicraft_model::Table> = variations
            .iter()
            .filter_map(|x| model.variation_table(&t.name, &c.name, x))
            .collect();
        let hint = match tables.first() {
            Some(vt) => {
                let mut cols: Vec<&str> = vt
                    .columns
                    .iter()
                    .filter(|x| !x.is_row_number())
                    .map(|x| x.name.as_str())
                    .take(12)
                    .collect();
                cols.sort_unstable_by_key(|x| x.to_ascii_lowercase());
                format!(
                    "; the variation table {} has the columns {}",
                    quote_table(&vt.name),
                    cols.join(", ")
                )
            }
            None => "; its variation does not lead to a table of the model".to_string(),
        };
        self.diags.push(Raw::error(
            "unknown-variation",
            v.span,
            format!("{reference} has no variation column [{}]{hint}", v.name),
        ));
    }

    fn check_call(&mut self, name: &str, name_span: crate::Span, argc: usize, span: crate::Span) {
        let Some(arity) = lookup(name) else {
            // The catalog lists every function of the engine it was checked against, so against a
            // model an unknown name is the engine's "failed to resolve name" error, unless the model
            // defines a user-defined function of that name. Without a model it stays a warning.
            match self.names {
                Some(names) if model_function(names.model(), name) => {}
                Some(_) => self.diags.push(Raw::error(
                    "unknown-function",
                    name_span,
                    format!(
                        "`{name}` is not a DAX function (not in the function catalog, nor a \
                         user-defined function of the model)"
                    ),
                )),
                None => self.diags.push(Raw::warning(
                    "unknown-function",
                    name_span,
                    format!("`{name}` is not in the function catalog (it may be valid but unchecked)"),
                )),
            }
            return;
        };
        let min = usize::from(arity.min);
        let too_few = argc < min;
        let too_many = arity.max.is_some_and(|m| argc > usize::from(m));
        if too_few || too_many {
            let expected = match arity.max {
                Some(m) if m == arity.min => format!("{m}"),
                Some(m) => format!("{min} to {m}"),
                None => format!("at least {min}"),
            };
            self.diags.push(Raw::error(
                "wrong-argument-count",
                span,
                format!(
                    "{} takes {expected} argument(s), got {argc}",
                    name.to_ascii_uppercase()
                ),
            ));
        }
    }

    fn check_name(&mut self, name: &str, quoted: bool, span: crate::Span) {
        // A variable wins over a table of the same name, even quoted (`'Product'` denotes
        // `VAR Product`), as in the engine.
        if self.is_var(name) {
            return;
        }
        let Some(names) = self.names else { return };
        if names.table(name).is_some()
            || (!quoted && is_keyword_argument(name))
            || model_calendar(names.model(), name)
        {
            return;
        }
        let what = if quoted { "table" } else { "table or variable" };
        self.diags.push(Raw::error(
            "unknown-table",
            span,
            format!("unknown {what} `{name}`"),
        ));
    }

    fn check_column(
        &mut self,
        table: Option<&str>,
        table_span: Option<crate::Span>,
        name: &str,
        span: crate::Span,
    ) {
        if let Some(t) = table
            && self.is_var(t)
            && self.names.is_none_or(|n| n.table(t).is_none())
        {
            // The engine resolves `x[Column]` against tables only, never variables (a table of
            // that name wins over a variable).
            self.diags.push(Raw::error(
                "unknown-table",
                table_span.unwrap_or(span),
                format!(
                    "`{t}` is a variable: a column cannot be qualified with a variable name \
                     (use [{name}] inside an iterator over it, or the column's original table)"
                ),
            ));
            return;
        }
        let Some(names) = self.names else { return };
        match table {
            Some(t) => {
                let Some(tbl) = names.table_at(t) else {
                    self.diags.push(Raw::error(
                        "unknown-table",
                        table_span.unwrap_or(span),
                        format!("unknown table `{t}`"),
                    ));
                    return;
                };
                if names.column_of(tbl, name).is_none()
                    && names.measure_of(tbl, name).is_none()
                    && !format_string_ref(name).is_some_and(|m| names.measure_of(tbl, m).is_some())
                    && !self.declared.contains(&name.to_ascii_uppercase())
                {
                    self.diags.push(Raw::error(
                        "unknown-column",
                        span,
                        format!("table `{}` has no column or measure `{name}`", tbl.table.name),
                    ));
                }
            }
            None => {
                let known = names.measure(name).is_some()
                    || format_string_ref(name).is_some_and(|m| names.measure(m).is_some())
                    || self.home.and_then(|t| names.column(t, name)).is_some()
                    || names.any_column(name).is_some()
                    || self.declared.contains(&name.to_ascii_uppercase());
                if !known {
                    self.diags.push(Raw::error(
                        "unknown-reference",
                        span,
                        format!("`[{name}]` is not a measure or a column of any table"),
                    ));
                }
            }
        }
    }
}

/// The measure a `[_<measure> FormatString]` reference names: the format string of that measure
/// (its dynamic format string evaluated in the current context, its static one, or blank), which
/// Power BI's visual queries read and the engine resolves for every measure (Desktop 2.159).
pub fn format_string_ref(name: &str) -> Option<&str> {
    const SUFFIX: &str = " formatstring";
    let rest = name.strip_prefix('_')?;
    let cut = rest.len().checked_sub(SUFFIX.len())?;
    let (base, suffix) = (rest.get(..cut)?, rest.get(cut..)?);
    (suffix.eq_ignore_ascii_case(SUFFIX) && !base.is_empty()).then_some(base)
}

/// Whether a table of the model defines a calendar `name` (calendar-based time intelligence;
/// TMSL `calendars`, kept in the table's unmodelled properties): a calendar reference.
pub(crate) fn model_calendar(model: &bicraft_model::SemanticModel, name: &str) -> bool {
    model.tables.iter().any(|t| {
        t.extra
            .get("calendars")
            .and_then(|c| c.as_array())
            .is_some_and(|cs| {
                cs.iter().any(|c| {
                    c.get("name")
                        .and_then(|n| n.as_str())
                        .is_some_and(|n| n.eq_ignore_ascii_case(name))
                })
            })
    })
}

/// Whether the model defines a user-defined DAX function `name` (TMSL/TMDL `functions`, kept in
/// the model's unmodelled properties).
pub(crate) fn model_function(model: &bicraft_model::SemanticModel, name: &str) -> bool {
    model
        .extra
        .get("functions")
        .and_then(|f| f.as_array())
        .is_some_and(|fs| {
            fs.iter().any(|f| {
                f.get("name")
                    .and_then(|n| n.as_str())
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
            })
        })
}

/// Column names an expression declares itself: string arguments of ADDCOLUMNS-like functions, the
/// `Value`/`Value1..n` columns of table constructors, and the fixed names of GENERATESERIES and
/// CALENDAR.
pub(crate) fn collect_declared_columns(expr: &Expr, out: &mut HashSet<String>) {
    let mut stack = vec![expr];
    while let Some(e) = stack.pop() {
        match e {
            Expr::Call { name, args, .. } => {
                if declares_columns(name) {
                    for a in args.iter().flatten() {
                        if let Expr::Str { value, .. } = a {
                            out.insert(value.to_ascii_uppercase());
                        }
                    }
                }
                if name.eq_ignore_ascii_case("GENERATESERIES") {
                    out.insert("VALUE".into());
                }
                if let Some(cols) = crate::info::info_columns(name) {
                    out.extend(cols.iter().map(|(c, _)| c.to_ascii_uppercase()));
                }
                if name.eq_ignore_ascii_case("CALENDAR") || name.eq_ignore_ascii_case("CALENDARAUTO") {
                    out.insert("DATE".into());
                }
                stack.extend(args.iter().flatten());
            }
            Expr::TableCtor { rows, .. } => {
                let width = rows
                    .iter()
                    .map(|r| {
                        if let Expr::Tuple { items, .. } = r {
                            items.len()
                        } else {
                            1
                        }
                    })
                    .max()
                    .unwrap_or(1);
                // One column is `Value`; several are `Value1`…`ValueN`.
                if width <= 1 {
                    out.insert("VALUE".into());
                } else {
                    for i in 1..=width {
                        out.insert(format!("VALUE{i}"));
                    }
                }
                stack.extend(rows.iter());
            }
            Expr::Unary { operand, .. } => stack.push(operand),
            Expr::Binary { lhs, rhs, .. } => {
                stack.push(lhs);
                stack.push(rhs);
            }
            Expr::Tuple { items, .. } => stack.extend(items.iter()),
            Expr::Var { bindings, body, .. } => {
                stack.extend(bindings.iter().map(|b| &b.value));
                stack.push(body);
            }
            Expr::Number { .. }
            | Expr::Str { .. }
            | Expr::Bool { .. }
            | Expr::Name { .. }
            | Expr::Column { .. } => {}
        }
    }
}

/// What a `Table[Name]` or `[Name]` reference denotes.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Resolved<'m> {
    Column(&'m Table, &'m Column),
    Measure(&'m Table, &'m Measure),
    /// A column the expression declares itself (`ADDCOLUMNS` names, table constructors…) or a
    /// column of a table variable: real, but not a model object.
    Local,
    /// Not found; the binder reports it.
    Unresolved,
}

/// How to look up a `[Name]` / `Table[Name]` reference.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Scope<'a> {
    /// The expression's home table.
    pub home: Option<&'a str>,
    /// Columns the expression declares itself (upper-case).
    pub declared: Option<&'a HashSet<String>>,
    /// Row-context objects (calculated columns, row filters): an unqualified `[Name]` that is a
    /// column of the home table wins over a measure of the same name.
    pub prefer_column: bool,
}

/// Resolves a column or measure reference against the model. `table_is_var` says the qualifier
/// is a table variable; `row_tables` are the tables of the enclosing row contexts, innermost
/// first, used for unqualified names that are neither measures nor home-table columns.
pub(crate) fn resolve<'m>(
    names: Names<'m>,
    scope: Scope<'_>,
    table: Option<&str>,
    name: &str,
    table_is_var: bool,
    row_tables: &[&str],
) -> Resolved<'m> {
    let is_declared = || {
        scope
            .declared
            .is_some_and(|d| d.contains(&name.to_ascii_uppercase()))
    };
    if let Some(t) = table {
        if table_is_var && names.table_at(t).is_none() {
            return Resolved::Local;
        }
        let Some(tbl) = names.table_at(t) else {
            return Resolved::Unresolved;
        };
        if let Some(c) = names.column_of(tbl, name) {
            return Resolved::Column(tbl.table, c);
        }
        if let Some(m) = names.measure_of(tbl, name) {
            return Resolved::Measure(tbl.table, m);
        }
        return if is_declared() {
            Resolved::Local
        } else {
            Resolved::Unresolved
        };
    }
    let home_column = scope.home.and_then(|h| names.column(h, name));
    let measure = names.measure(name);
    match (scope.prefer_column, home_column, measure) {
        (true, Some((t, c)), _) | (false, Some((t, c)), None) => return Resolved::Column(t, c),
        (_, _, Some((t, m))) => return Resolved::Measure(t, m),
        _ => {}
    }
    for rt in row_tables {
        if let Some((t, c)) = names.column(rt, name) {
            return Resolved::Column(t, c);
        }
    }
    if is_declared() {
        return Resolved::Local;
    }
    names
        .any_column(name)
        .map_or(Resolved::Unresolved, |(t, c)| Resolved::Column(t, c))
}
