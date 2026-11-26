//! DAX queries: `[DEFINE …] EVALUATE table [ORDER BY …] [START AT …] …`, as described by the
//! public "DAX queries" and "DAX user-defined functions" pages of the DAX reference.
//!
//! Validation reuses the expression validator rather than the binder's internals:
//! 1. the query's `TABLE`, `COLUMN` and `MEASURE` definitions are added to a clone of the model
//!    (a query table's columns are inferred from simple table expressions such as
//!    `SELECTCOLUMNS`, `ADDCOLUMNS`, `SUMMARIZECOLUMNS`, `DATATABLE`, `ROW` or `{…}`; when they
//!    cannot be, the table is *opaque* and references to its columns are not checked);
//! 2. every expression is validated on its own with [`validate`], prefixed with `VAR` bindings
//!    for the query variables (and function parameters) it references, and the diagnostics are
//!    mapped back to query offsets;
//! 3. query-level rules are checked here: `EVALUATE` needs a table, duplicate definitions,
//!    measures and columns on unknown tables, `START AT` arity and constants, user-defined
//!    function names, type hints, arity and recursion.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};

use bicraft_model::{Column, Measure, SemanticModel, Table};

use crate::ast::{Define, Expr, FunctionParam, Query};
use crate::lexer::{self, Token, TokenKind};
use crate::types::{Returns, ValueType};
use crate::{Context, Diagnostic, Raw, Severity, Span, Validation, parser, validate};

/// Parses a DAX query without binding it; returns the tree (if syntactically valid) and the
/// lexer/parser diagnostics.
pub fn parse_query(text: &str) -> (Option<Query>, Vec<Diagnostic>) {
    let mut raw = Vec::new();
    let tokens = lexer::lex_query(text, &mut raw);
    let query = parser::parse_query(&tokens, &mut raw);
    (query, raw.into_iter().map(|r| r.locate(text)).collect())
}

/// Validates a DAX query. Without a model only syntax, structure, arity and the query-level rules
/// are checked. `ctx.table` is ignored: a query has no home table.
pub fn validate_query(text: &str, ctx: &Context<'_>) -> Validation {
    validate_query_parsed(text, ctx).0
}

/// [`validate_query`], also returning the parsed query (what [`parse_query`] returns), so a caller
/// that runs the query does not lex and parse it a second time.
pub fn validate_query_parsed(text: &str, ctx: &Context<'_>) -> (Validation, Option<Query>) {
    let mut raw = Vec::new();
    let tokens = lexer::lex_query(text, &mut raw);
    let query = parser::parse_query(&tokens, &mut raw);
    if let Some(q) = &query {
        let vtext = substitute_params(text, &tokens);
        let mut checker = Checker::new(q, &vtext, ctx.model);
        checker.param_spans = tokens
            .iter()
            .filter(|t| matches!(t.kind, TokenKind::Param(_)))
            .map(|t| t.span)
            .collect();
        checker.check(&mut raw);
    }
    raw.sort_by_key(|r| r.span.start);
    let diagnostics: Vec<Diagnostic> = raw.into_iter().map(|r| r.locate(text)).collect();
    let valid = query.is_some() && diagnostics.iter().all(|d| d.severity != Severity::Error);
    (
        Validation {
            valid,
            diagnostics,
            result_type: None,
        },
        query,
    )
}

/// Replaces every `@param` with a string literal of the same byte length (`@p` → `""`,
/// `@name` → `"   "`), so expression slices can go through the expression validator with
/// unchanged offsets. XMLA parameters are typed as text, so a string is the right stand-in.
fn substitute_params(text: &str, tokens: &[Token]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for t in tokens {
        if !matches!(t.kind, TokenKind::Param(_)) {
            continue;
        }
        let (Some(before), Some(len)) = (text.get(last..t.span.start), t.span.end.checked_sub(t.span.start))
        else {
            continue;
        };
        out.push_str(before);
        out.push('"');
        out.extend(std::iter::repeat_n(' ', len.saturating_sub(2)));
        out.push('"');
        last = t.span.end;
    }
    out.push_str(text.get(last..).unwrap_or_default());
    out
}

/// What an expression evaluates to, as far as the query checker can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    Table,
    Scalar,
    Unknown,
}

/// Recursion limit for following variables, query tables and function bodies.
const MAX_INDIRECTION: usize = 64;

/// User-defined functions accept between 0 and 256 parameters.
const MAX_FUNCTION_PARAMS: usize = 256;

/// Beyond this many inferred columns (e.g. CROSSJOINs of CROSSJOINs) a table is treated as opaque.
const MAX_INFERRED_COLUMNS: usize = 4096;

/// Name of the synthetic table holding an EVALUATE's result columns while its ORDER BY is
/// validated, and the one holding the columns of table-valued query variables. Not writable in DAX.
const RESULT_TABLE: &str = "\u{1}evaluate result";
const VARS_TABLE: &str = "\u{1}query variables";

const TYPE_WORDS: &[&str] = &[
    "ANYVAL",
    "SCALAR",
    "TABLE",
    "ANYREF",
    "CALENDARREF",
    "COLUMNREF",
    "MEASUREREF",
    "TABLEREF",
];
const SUBTYPE_WORDS: &[&str] = &[
    "VARIANT", "INT64", "DECIMAL", "DOUBLE", "STRING", "DATETIME", "BOOLEAN", "NUMERIC",
];
const MODE_WORDS: &[&str] = &["VAL", "EXPR"];

/// Words a user-defined function may not be named after.
const RESERVED: &[&str] = &[
    "DEFINE", "EVALUATE", "MEASURE", "COLUMN", "TABLE", "FUNCTION", "VAR", "RETURN", "ORDER", "BY", "START",
    "AT", "ASC", "DESC", "IN", "NOT", "TRUE", "FALSE",
];

/// Functions that return a table (fallback while the typed catalog has no signature for them).
const TABLE_FUNCTIONS: &[&str] = &[
    "ADDCOLUMNS",
    "ADDMISSINGITEMS",
    "ALL",
    "ALLCROSSFILTERED",
    "ALLEXCEPT",
    "ALLNOBLANKROW",
    "ALLSELECTED",
    "CALCULATETABLE",
    "CALENDAR",
    "CALENDARAUTO",
    "CROSSJOIN",
    "CURRENTGROUP",
    "DATATABLE",
    "DATEADD",
    "DATESBETWEEN",
    "DATESINPERIOD",
    "DATESMTD",
    "DATESQTD",
    "DATESYTD",
    "DETAILROWS",
    "DISTINCT",
    "ENDOFMONTH",
    "ENDOFQUARTER",
    "ENDOFYEAR",
    "EXCEPT",
    "FILTER",
    "FILTERS",
    "FIRSTDATE",
    "FIRSTNONBLANK",
    "GENERATE",
    "GENERATEALL",
    "GENERATESERIES",
    "GROUPBY",
    "INDEX",
    "INTERSECT",
    "LASTDATE",
    "LASTNONBLANK",
    "NATURALINNERJOIN",
    "NATURALLEFTOUTERJOIN",
    "NEXTDAY",
    "NEXTMONTH",
    "NEXTQUARTER",
    "NEXTYEAR",
    "OFFSET",
    "PARALLELPERIOD",
    "PREVIOUSDAY",
    "PREVIOUSMONTH",
    "PREVIOUSQUARTER",
    "PREVIOUSYEAR",
    "RELATEDTABLE",
    "ROW",
    "SAMEPERIODLASTYEAR",
    "SAMPLE",
    "SELECTCOLUMNS",
    "STARTOFMONTH",
    "STARTOFQUARTER",
    "STARTOFYEAR",
    "SUBSTITUTEWITHINDEX",
    "SUMMARIZE",
    "SUMMARIZECOLUMNS",
    "TOPN",
    "TOPNSKIP",
    "TREATAS",
    "UNION",
    "VALUES",
    "WINDOW",
];

/// Catalog functions whose result shape depends on their arguments or that are not values.
const AMBIGUOUS_FUNCTIONS: &[&str] = &[
    "IF",
    "IF.EAGER",
    "SWITCH",
    "IFERROR",
    "COALESCE",
    "ERROR",
    "KEEPFILTERS",
    "REMOVEFILTERS",
    "USERELATIONSHIP",
    "CROSSFILTER",
    "ORDERBY",
    "PARTITIONBY",
    "MATCHBY",
];

fn one_of(list: &[&str], word: &str) -> bool {
    list.iter().any(|w| w.eq_ignore_ascii_case(word))
}

fn up(s: &str) -> String {
    s.to_ascii_uppercase()
}

struct Function<'q> {
    params: &'q [FunctionParam],
    body: &'q Expr,
}

struct Checker<'q> {
    query: &'q Query,
    /// The query text with `@params` replaced (same offsets as the original).
    text: &'q str,
    model: Option<&'q SemanticModel>,
    /// Query variables by upper-case name: (name as written, expression). First definition wins.
    vars: HashMap<String, (&'q str, &'q Expr)>,
    /// Query tables by upper-case name.
    tables: HashMap<String, &'q Expr>,
    functions: HashMap<String, Function<'q>>,
    /// Upper-case names of query tables whose columns could not be inferred.
    opaque: HashSet<String>,
    /// Some table-valued query variable or query table has unknown columns, so an unqualified
    /// `[Name]` might be one of them.
    any_opaque: bool,
    /// Memoized shapes of query variables (`V:NAME`) and user-defined functions (`F:NAME`).
    /// Without it, variables referencing variables could take exponential time.
    shapes: RefCell<HashMap<String, Shape>>,
    /// Memoized columns of query variables (`V:NAME`) and query tables (`T:NAME`).
    columns: RefCell<HashMap<String, Option<Vec<String>>>>,
    /// Spans of the `@parameters`, replaced by text literals for validation: what the validator
    /// infers from those stand-ins (runtime type warnings) does not apply to the real values.
    param_spans: Vec<Span>,
}

/// How one expression is validated.
struct Sub<'a> {
    table: Option<&'a str>,
    /// Function parameters in scope (function bodies only).
    params: &'a [FunctionParam],
    /// ORDER BY of an EVALUATE whose result columns are unknown.
    result_opaque: bool,
    /// An `EVALUATE` statement (or its `ORDER BY`), compiled as a whole when the query runs:
    /// comparisons of incompatible types fail it before any row is read.
    statement: bool,
}

impl<'q> Checker<'q> {
    fn new(query: &'q Query, text: &'q str, model: Option<&'q SemanticModel>) -> Self {
        let mut vars = HashMap::new();
        let mut tables = HashMap::new();
        let mut functions = HashMap::new();
        for d in &query.defines {
            match d {
                Define::Var { name, expr, .. } => {
                    vars.entry(up(name)).or_insert((name.as_str(), expr));
                }
                Define::Table { name, expr, .. } => {
                    tables.entry(up(name)).or_insert(expr);
                }
                Define::Function {
                    name, params, body, ..
                } => {
                    functions.entry(up(name)).or_insert(Function { params, body });
                }
                Define::Measure { .. } | Define::Column { .. } => {}
            }
        }
        Self {
            query,
            text,
            model,
            vars,
            tables,
            functions,
            opaque: HashSet::new(),
            any_opaque: false,
            shapes: RefCell::new(HashMap::new()),
            columns: RefCell::new(HashMap::new()),
            param_spans: Vec::new(),
        }
    }

    /// `compute` memoized under `key`; a cycle (or a key being computed) reads as `Unknown`.
    fn memo_shape(&self, key: String, compute: impl FnOnce() -> Shape) -> Shape {
        let cached = self.shapes.borrow().get(&key).copied();
        if let Some(s) = cached {
            return s;
        }
        self.shapes.borrow_mut().insert(key.clone(), Shape::Unknown);
        let s = compute();
        self.shapes.borrow_mut().insert(key, s);
        s
    }

    fn memo_columns(
        &self,
        key: String,
        compute: impl FnOnce() -> Option<Vec<String>>,
    ) -> Option<Vec<String>> {
        let cached = self.columns.borrow().get(&key).cloned();
        if let Some(c) = cached {
            return c;
        }
        self.columns.borrow_mut().insert(key.clone(), None);
        let c = compute();
        self.columns.borrow_mut().insert(key, c.clone());
        c
    }

    /// Query variables may reference variables defined after them, but not themselves.
    fn check_var_cycles(&self, diags: &mut Vec<Raw>) {
        let mut spans: HashMap<String, Span> = HashMap::new();
        for d in &self.query.defines {
            if let Define::Var { name, name_span, .. } = d {
                spans.entry(up(name)).or_insert(*name_span);
            }
        }
        let edges: HashMap<&str, Vec<String>> = self
            .vars
            .iter()
            .map(|(k, (_, e))| {
                // Inside its own definition a variable named like a model table is that table
                // (`DEFINE VAR Product = FILTER(Product, …)`, engine verdict `var-shadowing-025`).
                let own_table = self.model.is_some_and(|m| m.table(k).is_some());
                let deps = referenced_names(e)
                    .into_iter()
                    .filter(|n| self.vars.contains_key(n) && !(own_table && n == k))
                    .collect();
                (k.as_str(), deps)
            })
            .collect();
        // 1 = on the DFS stack, 2 = done.
        let mut state: HashMap<&str, u8> = HashMap::new();
        let mut reported = HashSet::new();
        let mut roots: Vec<&str> = edges.keys().copied().collect();
        roots.sort_unstable();
        for root in roots {
            if state.contains_key(root) {
                continue;
            }
            state.insert(root, 1);
            let mut stack: Vec<(&str, usize)> = vec![(root, 0)];
            while let Some((node, i)) = stack.last().copied() {
                let next = edges.get(node).and_then(|deps| deps.get(i));
                let Some(next) = next else {
                    state.insert(node, 2);
                    stack.pop();
                    continue;
                };
                if let Some(top) = stack.last_mut() {
                    top.1 = i.saturating_add(1);
                }
                match state.get(next.as_str()) {
                    Some(1) => {
                        if reported.insert(next.clone())
                            && let Some(span) = spans.get(next)
                        {
                            diags.push(Raw::error(
                                "circular-definition",
                                *span,
                                format!(
                                    "query variable `{}` depends on itself",
                                    self.vars.get(next).map_or("", |v| v.0)
                                ),
                            ));
                        }
                    }
                    Some(_) => {}
                    None => {
                        state.insert(next.as_str(), 1);
                        stack.push((next.as_str(), 0));
                    }
                }
            }
        }
    }

    fn check(mut self, diags: &mut Vec<Raw>) {
        self.check_definitions(diags);
        self.check_var_cycles(diags);
        let model = self.model;
        let mut aug = model.map(|m| self.augment(m, diags));
        if let Some(Aug::Full(m)) = &aug {
            self.check_used_cycles(m, diags);
        }
        let q = self.query;
        for d in &q.defines {
            self.check_define(d, aug.as_ref(), diags);
        }
        for st in &q.statements {
            self.validate_expr(
                &st.expr,
                st.expr_span,
                &Sub {
                    table: None,
                    params: &[],
                    result_opaque: false,
                    statement: true,
                },
                aug.as_ref(),
                diags,
            );
            if self.shape(&st.expr, &mut Vec::new(), 0) == Shape::Scalar {
                diags.push(Raw::error(
                    "evaluate-needs-table",
                    st.expr_span,
                    "EVALUATE needs a table expression; wrap a scalar in a table constructor { … } or ROW(\"Name\", …)",
                ));
            }
            if !st.order_by.is_empty() {
                let columns = self.infer_columns(&st.expr, 0);
                let result_opaque = columns.is_none();
                if let (Some(m), Some(cols)) = (aug.as_mut(), columns) {
                    m.push(synthetic_table(RESULT_TABLE, cols));
                }
                for key in &st.order_by {
                    self.validate_expr(
                        &key.expr,
                        key.expr_span,
                        &Sub {
                            table: None,
                            params: &[],
                            result_opaque,
                            statement: true,
                        },
                        aug.as_ref(),
                        diags,
                    );
                    if self.shape(&key.expr, &mut Vec::new(), 0) == Shape::Table {
                        diags.push(Raw::error(
                            "order-by-needs-scalar",
                            key.expr_span,
                            "an ORDER BY key must be a scalar expression or a column, not a table",
                        ));
                    }
                }
                if let Some(m) = aug.as_mut()
                    && !result_opaque
                {
                    m.pop();
                }
            }
            if let Some(extra) = st.start_at.get(st.order_by.len()..)
                && let (Some(first), Some(last)) = (extra.first(), extra.last())
            {
                diags.push(Raw::error(
                    "start-at-too-many-values",
                    Span {
                        start: first.span().start,
                        end: last.span().end,
                    },
                    format!(
                        "START AT has {} value(s) but ORDER BY has only {} key(s)",
                        st.start_at.len(),
                        st.order_by.len()
                    ),
                ));
            }
            for v in &st.start_at {
                if !is_constant(v, 0) {
                    diags.push(Raw::error(
                        "start-at-not-constant",
                        v.span(),
                        "a START AT value must be a constant or an @parameter, not an expression",
                    ));
                }
            }
        }
        self.check_function_calls(diags);
    }

    /// A circular dependency among query measures and columns fails the query when an `EVALUATE`
    /// statement reaches it; a cycle nothing evaluates is ignored, as by the engine. A query
    /// measure overrides a model measure of the same name for the query's own references only:
    /// model measures keep using the model's definitions, so only edges between query-defined
    /// objects can close such a cycle.
    fn check_used_cycles(&self, aug: &SemanticModel, diags: &mut Vec<Raw>) {
        use crate::deps::ObjectRef;
        let defined: Vec<ObjectRef> = self
            .query
            .defines
            .iter()
            .filter_map(|d| match d {
                Define::Measure { table, name, .. } => Some(ObjectRef::Measure {
                    table: table.clone(),
                    name: name.clone(),
                }),
                Define::Column { table, name, .. } => Some(ObjectRef::Column {
                    table: table.clone(),
                    name: name.clone(),
                }),
                _ => None,
            })
            .collect();
        if defined.is_empty() {
            return;
        }
        let g = crate::deps::dependencies(aug);
        let ours: HashSet<usize> = defined.iter().filter_map(|o| g.index_of(o)).collect();
        let next = |i: usize| -> Vec<usize> {
            g.edges
                .get(i)
                .map(|e| e.iter().copied().filter(|j| ours.contains(j)).collect())
                .unwrap_or_default()
        };
        // Objects on a cycle made of query-defined objects only (Tarjan, linear time).
        let restricted: Vec<Vec<usize>> = (0..g.nodes.len()).map(next).collect();
        let cyclic: HashSet<usize> = crate::deps::cyclic_components(&restricted)
            .into_iter()
            .flatten()
            .collect();
        if cyclic.is_empty() {
            return;
        }
        let on_cycle = |i: usize| cyclic.contains(&i);
        for st in &self.query.statements {
            // What the statement uses, through query variables and tables too.
            let mut texts: Vec<Span> = vec![st.expr_span];
            texts.extend(st.order_by.iter().map(|k| k.expr_span));
            let mut exprs: Vec<&Expr> = vec![&st.expr];
            exprs.extend(st.order_by.iter().map(|k| &k.expr));
            let mut seen: HashSet<String> = HashSet::new();
            let mut budget = 10_000usize;
            while let Some(e) = exprs.pop() {
                let Some(left) = budget.checked_sub(1) else { break };
                budget = left;
                for name in referenced_names(e) {
                    if !seen.insert(name.clone()) {
                        continue;
                    }
                    if let Some((_, v)) = self.vars.get(&name) {
                        exprs.push(v);
                    } else if let Some(t) = self.tables.get(&name) {
                        exprs.push(t);
                    }
                }
            }
            for d in &self.query.defines {
                if let Define::Var { name, expr_span, .. } | Define::Table { name, expr_span, .. } = d
                    && seen.contains(&up(name))
                {
                    texts.push(*expr_span);
                }
            }
            let mut stack: Vec<usize> = Vec::new();
            for span in texts {
                for obj in crate::deps::expression_dependencies(aug, self.slice(span), None, false) {
                    if let Some(i) = g.index_of(&obj).filter(|i| ours.contains(i)) {
                        stack.push(i);
                    }
                }
            }
            let mut visited: HashSet<usize> = HashSet::new();
            let mut found = None;
            while let Some(i) = stack.pop() {
                if !visited.insert(i) {
                    continue;
                }
                if on_cycle(i) {
                    found = g.nodes.get(i);
                    break;
                }
                stack.extend(next(i));
            }
            if let Some(obj) = found {
                diags.push(Raw::error(
                    "circular-dependency",
                    st.expr_span,
                    format!(
                        "circular dependency: this EVALUATE uses {obj}, which depends on itself                          through the query's definitions"
                    ),
                ));
            }
        }
    }

    /// Duplicates, misplaced measures and columns, function names, parameters and type hints.
    fn check_definitions(&self, diags: &mut Vec<Raw>) {
        let mut seen: HashSet<(&'static str, String)> = HashSet::new();
        for d in &self.query.defines {
            let key = match d {
                Define::Column { table, name, .. } => format!("{}[{}]", up(table), up(name)),
                _ => up(d.name()),
            };
            if !seen.insert((d.kind(), key)) {
                diags.push(Raw::error(
                    "duplicate-definition",
                    d.name_span(),
                    format!(
                        "{} `{}` is defined more than once in this query",
                        d.kind().to_ascii_uppercase(),
                        d.name()
                    ),
                ));
            }
            match d {
                Define::Measure {
                    table, table_span, ..
                }
                | Define::Column {
                    table, table_span, ..
                } => {
                    if let Some(m) = self.model
                        && m.table(table).is_none()
                        && !self.tables.contains_key(&up(table))
                    {
                        diags.push(Raw::error(
                            "unknown-table",
                            *table_span,
                            format!(
                                "unknown table `{table}`: a query {} must belong to a model table or a table defined with TABLE",
                                d.kind().to_ascii_uppercase()
                            ),
                        ));
                    }
                }
                Define::Function {
                    name,
                    name_span,
                    params,
                    body,
                    ..
                } => self.check_function(name, *name_span, params, body, diags),
                Define::Var { .. } | Define::Table { .. } => {}
            }
        }
    }

    fn check_function(
        &self,
        name: &str,
        name_span: Span,
        params: &[FunctionParam],
        body: &Expr,
        diags: &mut Vec<Raw>,
    ) {
        if crate::lookup_function(name).is_some()
            || crate::signature(name).is_some()
            || one_of(RESERVED, name)
            || crate::reserved::is_reserved_name(name)
        {
            diags.push(Raw::error(
                "function-name-reserved",
                name_span,
                format!("`{name}` is a built-in DAX function or a reserved word"),
            ));
        } else if name.ends_with('.') || name.contains("..") {
            diags.push(Raw::error(
                "invalid-function-name",
                name_span,
                format!("`{name}`: a function name cannot end with a period or contain consecutive periods"),
            ));
        }
        if params.len() > MAX_FUNCTION_PARAMS {
            diags.push(Raw::error(
                "too-many-parameters",
                name_span,
                format!(
                    "`{name}` has {} parameters; a function takes at most {MAX_FUNCTION_PARAMS}",
                    params.len()
                ),
            ));
        }
        let mut seen = HashSet::new();
        for p in params {
            if p.name.contains('.') || one_of(RESERVED, &p.name) || crate::reserved::is_reserved_name(&p.name)
            {
                diags.push(Raw::error(
                    "invalid-parameter-name",
                    p.name_span,
                    format!("`{}` is not a valid parameter name (letters, digits and underscores; no reserved words)", p.name),
                ));
            }
            if !seen.insert(up(&p.name)) {
                diags.push(Raw::error(
                    "duplicate-definition",
                    p.name_span,
                    format!("parameter `{}` is declared more than once", p.name),
                ));
            }
            check_hints(p, diags);
        }
        let up_name = up(name);
        let mut stack = vec![body];
        stack.extend(params.iter().filter_map(|p| p.default.as_ref()));
        while let Some(e) = stack.pop() {
            if let Expr::Call {
                name: callee,
                name_span,
                ..
            } = e
                && up(callee) == up_name
            {
                diags.push(Raw::error(
                    "recursive-function",
                    *name_span,
                    format!("`{name}` calls itself; user-defined functions cannot be recursive"),
                ));
            }
            push_children(e, &mut stack);
        }
    }

    /// Adds the query's tables, columns and measures to a clone of the model; without query
    /// tables, columns or measures (Desktop's visual queries: variables only), the variables'
    /// table is added beside the model instead ([`Aug::Overlay`]): no copy of the model.
    fn augment(&mut self, model: &'q SemanticModel, diags: &mut Vec<Raw>) -> Aug<'q> {
        let needs_copy = self.query.defines.iter().any(|d| {
            matches!(
                d,
                Define::Table { .. } | Define::Column { .. } | Define::Measure { .. }
            )
        });
        if !needs_copy && !force_copy() {
            let extra = self.vars_table().into_iter().collect();
            return Aug::Overlay(model, extra);
        }
        let mut aug = model.clone();
        let mut done = HashSet::new();
        let q = self.query;
        for d in &q.defines {
            let Define::Table {
                name,
                name_span,
                expr,
                ..
            } = d
            else {
                continue;
            };
            if !done.insert(up(name)) {
                continue;
            }
            let columns = self.infer_columns(expr, 0);
            if columns.is_none() {
                self.opaque.insert(up(name));
                self.any_opaque = true;
                diags.push(Raw::warning(
                    "opaque-table",
                    *name_span,
                    format!("the columns of query table `{name}` could not be inferred; references to them are not checked"),
                ));
            }
            let columns = columns.unwrap_or_default();
            match aug.table_mut(name) {
                Some(existing) => {
                    diags.push(Raw::warning(
                        "shadows-model-table",
                        *name_span,
                        format!("query table `{name}` hides the model table of the same name"),
                    ));
                    existing.columns = columns.into_iter().map(new_column).collect();
                }
                None => aug.tables.push(synthetic_table(name, columns)),
            }
        }
        if let Some(t) = self.vars_table() {
            aug.tables.push(t);
        }
        for d in &q.defines {
            if let Define::Column {
                table,
                name,
                expr_span,
                ..
            } = d
            {
                let expression = self.slice(*expr_span).to_string();
                if let Some(t) = aug.table_mut(table)
                    && t.column(name).is_none()
                {
                    t.columns.push(Column {
                        expression: Some(expression),
                        ..new_column(name.clone())
                    });
                }
            }
        }
        // A query measure overrides the model measure of the same name.
        let overridden: HashSet<String> = q
            .defines
            .iter()
            .filter_map(|d| match d {
                Define::Measure { table, name, .. } if aug.table(table).is_some() => Some(up(name)),
                _ => None,
            })
            .collect();
        if !overridden.is_empty() {
            for t in &mut aug.tables {
                t.measures.retain(|m| !overridden.contains(&up(&m.name)));
            }
        }
        let mut added = HashSet::new();
        for d in &q.defines {
            if let Define::Measure {
                table,
                name,
                expr_span,
                ..
            } = d
            {
                if !added.insert(up(name)) {
                    continue;
                }
                let expression = self.slice(*expr_span).to_string();
                if let Some(t) = aug.table_mut(table) {
                    t.measures.push(Measure {
                        name: name.clone(),
                        expression,
                        ..Default::default()
                    });
                }
            }
        }
        Aug::Full(Box::new(aug))
    }

    /// A table of the columns of table-valued query variables, so `[Name]` of e.g. a
    /// SELECTCOLUMNS variable resolves; unknown columns make unqualified references uncheckable.
    fn vars_table(&mut self) -> Option<Table> {
        let mut var_columns = Vec::new();
        let mut var_names: Vec<&(&str, &Expr)> = self.vars.values().collect();
        var_names.sort_by_key(|(n, _)| *n);
        for (_, expr) in var_names {
            if self.shape(expr, &mut Vec::new(), 0) != Shape::Table {
                continue;
            }
            match self.infer_columns(expr, 0) {
                Some(cols) => var_columns.extend(cols),
                None => self.any_opaque = true,
            }
        }
        (!var_columns.is_empty()).then(|| synthetic_table(VARS_TABLE, var_columns))
    }

    fn check_define(&self, d: &Define, aug: Option<&Aug<'_>>, diags: &mut Vec<Raw>) {
        let plain = |table| Sub {
            table,
            params: &[],
            result_opaque: false,
            statement: false,
        };
        match d {
            Define::Measure {
                table,
                expr,
                expr_span,
                ..
            }
            | Define::Column {
                table,
                expr,
                expr_span,
                ..
            } => {
                self.validate_expr(expr, *expr_span, &plain(Some(table)), aug, diags);
                if self.shape(expr, &mut Vec::new(), 0) == Shape::Table {
                    diags.push(Raw::error(
                        "expected-scalar",
                        *expr_span,
                        format!(
                            "a query {} must be a scalar expression, not a table",
                            d.kind().to_ascii_uppercase()
                        ),
                    ));
                }
            }
            Define::Var { expr, expr_span, .. } => {
                self.validate_expr(expr, *expr_span, &plain(None), aug, diags)
            }
            Define::Table { expr, expr_span, .. } => {
                self.validate_expr(expr, *expr_span, &plain(None), aug, diags);
                if self.shape(expr, &mut Vec::new(), 0) == Shape::Scalar {
                    diags.push(Raw::error(
                        "expected-table",
                        *expr_span,
                        "a query TABLE needs a table expression; wrap a scalar in { … } or ROW(…)",
                    ));
                }
            }
            Define::Function {
                params,
                body,
                body_span,
                ..
            } => {
                for p in params {
                    if let (Some(default), Some(span)) = (&p.default, p.default_span) {
                        self.validate_expr(default, span, &plain(None), aug, diags);
                    }
                }
                self.validate_expr(
                    body,
                    *body_span,
                    &Sub {
                        table: None,
                        params,
                        result_opaque: false,
                        statement: false,
                    },
                    aug,
                    diags,
                );
            }
        }
    }

    fn slice(&self, span: Span) -> &'q str {
        self.text.get(span.start..span.end).unwrap_or_default()
    }

    /// Validates one expression of the query with the expression validator. Query variables and
    /// function parameters it references are bound by a `VAR … RETURN` prefix whose diagnostics are
    /// dropped; the others are shifted back to query offsets.
    fn validate_expr(
        &self,
        expr: &Expr,
        span: Span,
        sub: &Sub<'_>,
        model: Option<&Aug<'_>>,
        diags: &mut Vec<Raw>,
    ) {
        let body = self.slice(span);
        if body.is_empty() {
            return;
        }
        let referenced = referenced_names(expr);
        let mut prefix = String::new();
        let mut params_bound = HashSet::new();
        // Parameter references whose type the placeholder cannot represent (references, lazy or
        // untyped parameters): errors on them are reported as warnings.
        let mut loose_params = HashSet::new();
        for p in sub.params {
            let key = up(&p.name);
            if !referenced.contains(&key) || !params_bound.insert(key.clone()) {
                continue;
            }
            let (placeholder, strict) = param_placeholder(p);
            if !strict {
                loose_params.insert(key);
            }
            prefix.push_str(&format!("VAR {} = {placeholder} ", p.name));
        }
        for key in &referenced {
            if params_bound.contains(key) {
                continue;
            }
            if let Some((name, vexpr)) = self.vars.get(key) {
                let placeholder = if self.shape(vexpr, &mut Vec::new(), 0) == Shape::Table {
                    "{BLANK()}"
                } else {
                    "BLANK()"
                };
                prefix.push_str(&format!("VAR {name} = {placeholder} "));
            }
        }
        if !prefix.is_empty() {
            prefix.push_str("RETURN ");
        }
        let plen = prefix.len();
        prefix.push_str(body);
        let v = match model {
            Some(Aug::Overlay(base, extra)) => crate::validate_overlay(&prefix, base, extra, sub.table),
            Some(Aug::Full(m)) => validate(
                &prefix,
                &Context {
                    model: Some(&**m),
                    table: sub.table,
                },
            ),
            None => validate(
                &prefix,
                &Context {
                    model: None,
                    table: sub.table,
                },
            ),
        };
        let loose_spans = if loose_params.is_empty() {
            Vec::new()
        } else {
            name_spans(expr, &loose_params)
        };
        // Values replaced by stand-ins (`@parameters`, function parameters): type warnings about
        // the stand-ins say nothing about the real values.
        let mut stand_ins: Vec<Span> = self
            .param_spans
            .iter()
            .copied()
            .filter(|p| p.start >= span.start && p.end <= span.end)
            .collect();
        if !params_bound.is_empty() {
            stand_ins.extend(name_spans(expr, &params_bound));
        }
        for d in v.diagnostics {
            let Some(start) = d.span.start.checked_sub(plen) else {
                continue;
            };
            let mapped = Span {
                start: start.saturating_add(span.start),
                end: d.span.end.saturating_sub(plen).saturating_add(span.start),
            };
            let mut severity = d.severity;
            let mut code = d.code;
            let mut message = d.message;
            if (code.starts_with("runtime-") || code == "incompatible-comparison")
                && stand_ins
                    .iter()
                    .any(|s| s.start < mapped.end.max(mapped.start.saturating_add(1)) && mapped.start < s.end)
            {
                continue;
            }
            if sub.statement && code == "runtime-incompatible-comparison" {
                severity = Severity::Error;
                code = "incompatible-comparison";
            }
            // A misplaced filter modifier fails whatever the data: the statement fails.
            if sub.statement && code == "runtime-misplaced-filter-modifier" {
                severity = Severity::Error;
                code = "misplaced-filter-modifier";
            }
            match code {
                "unknown-function" if self.functions.contains_key(&up(self.slice(mapped))) => continue,
                "unknown-column" => {
                    if column_table_at(expr, mapped).is_some_and(|t| self.opaque.contains(&up(t))) {
                        continue;
                    }
                }
                "unknown-reference" if sub.result_opaque || self.any_opaque => {
                    severity = Severity::Warning;
                    code = "unchecked-reference";
                    message = format!(
                        "{message} (it may be a column of a table whose columns could not be inferred)"
                    );
                }
                _ => {}
            }
            if severity == Severity::Error
                && loose_spans
                    .iter()
                    .any(|s| s.start < mapped.end.max(mapped.start + 1) && mapped.start < s.end)
            {
                severity = Severity::Warning;
                message = format!("{message} (not checked for reference or untyped function parameters)");
            }
            diags.push(Raw {
                severity,
                code,
                message,
                span: mapped,
            });
        }
    }

    /// Arity of calls to the query's user-defined functions.
    fn check_function_calls(&self, diags: &mut Vec<Raw>) {
        if self.functions.is_empty() {
            return;
        }
        let mut stack: Vec<&Expr> = Vec::new();
        for d in &self.query.defines {
            match d {
                Define::Measure { expr, .. }
                | Define::Column { expr, .. }
                | Define::Var { expr, .. }
                | Define::Table { expr, .. } => stack.push(expr),
                Define::Function { params, body, .. } => {
                    stack.push(body);
                    stack.extend(params.iter().filter_map(|p| p.default.as_ref()));
                }
            }
        }
        for st in &self.query.statements {
            stack.push(&st.expr);
            stack.extend(st.order_by.iter().map(|k| &k.expr));
        }
        while let Some(e) = stack.pop() {
            if let Expr::Call { name, args, span, .. } = e
                && crate::lookup_function(name).is_none()
                && let Some(f) = self.functions.get(&up(name))
            {
                let max = f.params.len();
                let min = f
                    .params
                    .iter()
                    .rposition(|p| p.default.is_none())
                    .map_or(0, |i| i.saturating_add(1));
                let argc = args.len();
                if argc < min || argc > max {
                    let expected = if min == max {
                        format!("{max}")
                    } else {
                        format!("{min} to {max}")
                    };
                    diags.push(Raw::error(
                        "wrong-argument-count",
                        *span,
                        format!("{name} takes {expected} argument(s), got {argc}"),
                    ));
                }
            }
            push_children(e, &mut stack);
        }
    }

    /// Whether an expression is a table or a scalar. `scope` holds the local `VAR`s in scope.
    fn shape(&self, expr: &Expr, scope: &mut Vec<(String, Shape)>, depth: usize) -> Shape {
        if depth > MAX_INDIRECTION {
            return Shape::Unknown;
        }
        let next = depth.saturating_add(1);
        match expr {
            Expr::Number { .. }
            | Expr::Str { .. }
            | Expr::Bool { .. }
            | Expr::Column { .. }
            | Expr::Unary { .. }
            | Expr::Binary { .. } => Shape::Scalar,
            Expr::Tuple { .. } => Shape::Unknown,
            Expr::TableCtor { .. } => Shape::Table,
            Expr::Name { name, .. } => {
                if name.starts_with('@') {
                    return Shape::Scalar;
                }
                // A variable wins over a table of the same name, even quoted.
                let key = up(name);
                if let Some((_, s)) = scope.iter().rev().find(|(n, _)| *n == key) {
                    return *s;
                }
                if let Some((_, v)) = self.vars.get(&key) {
                    return self.memo_shape(format!("V:{key}"), || self.shape(v, &mut Vec::new(), next));
                }
                if self.tables.contains_key(&key) || self.model.is_some_and(|m| m.table(name).is_some()) {
                    Shape::Table
                } else {
                    Shape::Unknown
                }
            }
            Expr::Var { bindings, body, .. } => {
                let mark = scope.len();
                for b in bindings {
                    let s = self.shape(&b.value, scope, next);
                    scope.push((up(&b.name), s));
                }
                let s = self.shape(body, scope, next);
                scope.truncate(mark);
                s
            }
            Expr::Call { name, .. } => {
                if crate::lookup_function(name).is_none()
                    && let Some(f) = self.functions.get(&up(name))
                {
                    let mut params = f.params.iter().map(|p| (up(&p.name), param_shape(p))).collect();
                    return self.memo_shape(format!("F:{}", up(name)), || {
                        self.shape(f.body, &mut params, next)
                    });
                }
                builtin_shape(name)
            }
        }
    }

    /// Column names of a table expression, when they can be told statically.
    fn infer_columns(&self, expr: &Expr, depth: usize) -> Option<Vec<String>> {
        if depth > MAX_INDIRECTION {
            return None;
        }
        let next = depth.saturating_add(1);
        match expr {
            Expr::TableCtor { rows, .. } => {
                let width = rows
                    .iter()
                    .map(|r| match r {
                        Expr::Tuple { items, .. } => items.len(),
                        _ => 1,
                    })
                    .max()
                    .unwrap_or(1);
                if width <= 1 {
                    Some(vec!["Value".into()])
                } else {
                    Some((1..=width).map(|i| format!("Value{i}")).collect())
                }
            }
            Expr::Name { name, .. } => {
                let key = up(name);
                if let Some((_, v)) = self.vars.get(&key) {
                    return self.memo_columns(format!("V:{key}"), || self.infer_columns(v, next));
                }
                if let Some(t) = self.tables.get(&key) {
                    return self.memo_columns(format!("T:{key}"), || self.infer_columns(t, next));
                }
                let t = self.model?.table(name)?;
                Some(
                    t.columns
                        .iter()
                        .filter(|c| !c.is_row_number())
                        .map(|c| c.name.clone())
                        .collect(),
                )
            }
            Expr::Var { bindings, body, .. } => {
                // Local variables are not followed: only a body that names none of them is inferred.
                let shadowed: HashSet<String> = bindings.iter().map(|b| up(&b.name)).collect();
                if referenced_names(body).iter().any(|n| shadowed.contains(n)) {
                    return None;
                }
                self.infer_columns(body, next)
            }
            Expr::Call { name, args, .. } => self.infer_call_columns(name, args, next),
            _ => None,
        }
    }

    fn infer_call_columns(&self, name: &str, args: &[Option<Expr>], depth: usize) -> Option<Vec<String>> {
        let arg = |i: usize| args.get(i).and_then(Option::as_ref);
        let upper = up(name);
        match upper.as_str() {
            "SELECTCOLUMNS" => named_columns(args.get(1..)?, false),
            "ADDCOLUMNS" => {
                let mut base = self.infer_columns(arg(0)?, depth)?;
                base.extend(named_columns(args.get(1..)?, true)?);
                if base.len() > MAX_INFERRED_COLUMNS {
                    return None;
                }
                Some(base)
            }
            "SUMMARIZECOLUMNS" => named_columns(args, false),
            "SUMMARIZE" | "GROUPBY" => named_columns(args.get(1..)?, false),
            "ROW" | "DATATABLE" => {
                let mut out = Vec::new();
                let pairs = if upper == "DATATABLE" {
                    args.get(..args.len().saturating_sub(1))?
                } else {
                    args
                };
                for a in pairs.iter().step_by(2) {
                    match a {
                        Some(Expr::Str { value, .. }) => out.push(value.clone()),
                        _ => return None,
                    }
                }
                Some(out)
            }
            "GENERATESERIES" => Some(vec!["Value".into()]),
            "CALENDAR" | "CALENDARAUTO" => Some(vec!["Date".into()]),
            "FILTER" | "CALCULATETABLE" | "KEEPFILTERS" | "UNION" | "INTERSECT" | "EXCEPT"
            | "RELATEDTABLE" => self.infer_columns(arg(0)?, depth),
            "TOPN" | "SAMPLE" => self.infer_columns(arg(1)?, depth),
            "TOPNSKIP" => self.infer_columns(arg(2)?, depth),
            "DISTINCT" | "VALUES" | "ALL" | "ALLNOBLANKROW" | "ALLSELECTED" => {
                let all_columns: Option<Vec<String>> = args
                    .iter()
                    .map(|a| match a {
                        Some(Expr::Column { name, .. }) => Some(name.clone()),
                        _ => None,
                    })
                    .collect();
                match all_columns {
                    Some(cols) if !cols.is_empty() => Some(cols),
                    _ if args.len() == 1 => self.infer_columns(arg(0)?, depth),
                    _ => None,
                }
            }
            "CROSSJOIN" | "NATURALINNERJOIN" | "NATURALLEFTOUTERJOIN" | "GENERATE" | "GENERATEALL" => {
                let mut out = Vec::new();
                for a in args {
                    out.extend(self.infer_columns(a.as_ref()?, depth)?);
                    if out.len() > MAX_INFERRED_COLUMNS {
                        return None;
                    }
                }
                Some(out)
            }
            _ => None,
        }
    }
}

/// Columns of `SELECTCOLUMNS`/`SUMMARIZE`-style argument lists: `"Name", expression` pairs and
/// bare column references (which keep their name). Filter tables (calls, table names) are
/// skipped; with `pairs_only`, only `"Name", expression` pairs are accepted.
fn named_columns(args: &[Option<Expr>], pairs_only: bool) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(a) = args.get(i) {
        match a {
            Some(Expr::Str { value, .. }) => {
                out.push(value.clone());
                i = i.saturating_add(2);
                continue;
            }
            Some(Expr::Column { name, .. }) if !pairs_only => out.push(name.clone()),
            Some(Expr::Call { name, .. })
                if !pairs_only
                    && !name.eq_ignore_ascii_case("ROLLUPADDISSUBTOTAL")
                    && !name.eq_ignore_ascii_case("ROLLUPGROUP")
                    && !name.eq_ignore_ascii_case("ROLLUP")
                    && !name.eq_ignore_ascii_case("NONVISUAL")
                    && !name.eq_ignore_ascii_case("IGNORE") => {}
            Some(Expr::Name { .. }) if !pairs_only => {}
            _ => return None,
        }
        i = i.saturating_add(1);
    }
    Some(out)
}

fn new_column(name: String) -> Column {
    Column {
        name,
        ..Default::default()
    }
}

#[cfg(test)]
thread_local! {
    /// Tests validate with a copy of the model too, to compare.
    pub(crate) static FORCE_COPY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn force_copy() -> bool {
    #[cfg(test)]
    {
        FORCE_COPY.with(std::cell::Cell::get)
    }
    #[cfg(not(test))]
    {
        false
    }
}

/// The model a query's expressions are validated against: a copy of the model holding the
/// query's definitions, or the model itself with tables added after its own.
enum Aug<'q> {
    Full(Box<SemanticModel>),
    Overlay(&'q SemanticModel, Vec<Table>),
}

impl Aug<'_> {
    /// Adds a table after the others.
    fn push(&mut self, t: Table) {
        match self {
            Aug::Full(m) => m.tables.push(t),
            Aug::Overlay(_, extra) => extra.push(t),
        }
    }

    /// Removes the last table added.
    fn pop(&mut self) {
        match self {
            Aug::Full(m) => {
                m.tables.pop();
            }
            Aug::Overlay(_, extra) => {
                extra.pop();
            }
        }
    }
}

fn synthetic_table(name: &str, columns: Vec<String>) -> Table {
    Table {
        name: name.to_string(),
        columns: columns.into_iter().map(new_column).collect(),
        ..Default::default()
    }
}

fn builtin_shape(name: &str) -> Shape {
    if let Some(sig) = crate::signature(name) {
        match sig.returns {
            Returns::Fixed(ValueType::Table) => return Shape::Table,
            Returns::Fixed(ValueType::Scalar(_)) => return Shape::Scalar,
            _ => {}
        }
    }
    if one_of(TABLE_FUNCTIONS, name) {
        Shape::Table
    } else if one_of(AMBIGUOUS_FUNCTIONS, name) || crate::lookup_function(name).is_none() {
        Shape::Unknown
    } else {
        Shape::Scalar
    }
}

fn hint_words(p: &FunctionParam) -> impl Iterator<Item = String> + '_ {
    p.hints.iter().map(|h| up(&h.word))
}

fn param_shape(p: &FunctionParam) -> Shape {
    let words: Vec<String> = hint_words(p).collect();
    if words.iter().any(|w| w == "TABLE" || w == "TABLEREF") {
        Shape::Table
    } else if words.iter().any(|w| w == "SCALAR" || one_of(SUBTYPE_WORDS, w)) {
        Shape::Scalar
    } else {
        Shape::Unknown
    }
}

/// The stand-in value bound to a function parameter while its body is validated, and whether
/// that stand-in has the parameter's real shape (scalar or table passed by value).
fn param_placeholder(p: &FunctionParam) -> (&'static str, bool) {
    let words: Vec<String> = hint_words(p).collect();
    let by_value = !words.iter().any(|w| w == "EXPR" || w.ends_with("REF"));
    match param_shape(p) {
        Shape::Table => ("{BLANK()}", by_value),
        Shape::Scalar => ("BLANK()", by_value),
        Shape::Unknown => ("BLANK()", false),
    }
}

/// Type hints: `[type] [subtype] [mode]`, each at most once, in that order.
fn check_hints(p: &FunctionParam, diags: &mut Vec<Raw>) {
    let mut last_rank = 0usize;
    let mut type_word: Option<String> = None;
    for h in &p.hints {
        let w = up(&h.word);
        let rank = if one_of(TYPE_WORDS, &w) {
            1
        } else if one_of(SUBTYPE_WORDS, &w) {
            2
        } else if one_of(MODE_WORDS, &w) {
            3
        } else {
            diags.push(Raw::error(
                "unknown-type-hint",
                h.span,
                format!(
                    "unknown type hint `{}` (types: AnyVal, Scalar, Table, AnyRef, CalendarRef, ColumnRef, MeasureRef, TableRef; subtypes: Variant, Int64, Decimal, Double, String, DateTime, Boolean, Numeric; modes: val, expr)",
                    h.word
                ),
            ));
            return;
        };
        let problem = if rank <= last_rank {
            Some("type hints are written once each, in the order type, subtype, mode".to_string())
        } else if rank == 2 && type_word.as_deref().is_some_and(|t| t != "SCALAR") {
            Some(format!(
                "a subtype applies only to Scalar parameters, not {}",
                type_word.as_deref().unwrap_or_default()
            ))
        } else if w == "VAL" && type_word.as_deref().is_some_and(|t| t.ends_with("REF")) {
            Some("reference parameters are always passed as expr".to_string())
        } else {
            None
        };
        if let Some(message) = problem {
            diags.push(Raw::error("invalid-type-hint", h.span, message));
            return;
        }
        if rank == 1 {
            type_word = Some(w);
        }
        last_rank = rank;
    }
}

/// `START AT` values: literals, signed numbers, `@parameters` and calls such as `DATE(2024, 1, 1)`
/// or `BLANK()` whose arguments are constants.
fn is_constant(e: &Expr, depth: usize) -> bool {
    if depth > MAX_INDIRECTION {
        return false;
    }
    match e {
        Expr::Number { .. } | Expr::Str { .. } | Expr::Bool { .. } => true,
        Expr::Unary { op, operand, .. } => {
            (op == "-" || op == "+") && matches!(**operand, Expr::Number { .. })
        }
        Expr::Name { name, .. } => name.starts_with('@'),
        Expr::Call { name, args, .. } => {
            one_of(&["DATE", "TIME", "BLANK", "TRUE", "FALSE"], name)
                && args
                    .iter()
                    .flatten()
                    .all(|a| is_constant(a, depth.saturating_add(1)))
        }
        _ => false,
    }
}

pub(crate) fn push_children<'e>(e: &'e Expr, stack: &mut Vec<&'e Expr>) {
    match e {
        Expr::Call { args, .. } => stack.extend(args.iter().flatten()),
        Expr::Unary { operand, .. } => stack.push(operand),
        Expr::Binary { lhs, rhs, .. } => {
            stack.push(lhs);
            stack.push(rhs);
        }
        Expr::Var { bindings, body, .. } => {
            stack.extend(bindings.iter().map(|b| &b.value));
            stack.push(body);
        }
        Expr::TableCtor { rows, .. } => stack.extend(rows.iter()),
        Expr::Tuple { items, .. } => stack.extend(items.iter()),
        Expr::Number { .. }
        | Expr::Str { .. }
        | Expr::Bool { .. }
        | Expr::Name { .. }
        | Expr::Column { .. } => {}
    }
}

/// Upper-case names an expression may resolve as variables: bare names and the table part of
/// `table[column]` references.
fn referenced_names(expr: &Expr) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut stack = vec![expr];
    while let Some(e) = stack.pop() {
        match e {
            Expr::Name { name, .. } if !name.starts_with('@') => {
                out.insert(up(name));
            }
            Expr::Column { table: Some(t), .. } => {
                out.insert(up(t));
            }
            _ => {}
        }
        push_children(e, &mut stack);
    }
    out
}

/// Spans of the bare names in `names` (upper-case) inside an expression.
fn name_spans(expr: &Expr, names: &HashSet<String>) -> Vec<Span> {
    let mut out = Vec::new();
    let mut stack = vec![expr];
    while let Some(e) = stack.pop() {
        match e {
            Expr::Name { name, span, .. } if names.contains(&up(name)) => out.push(*span),
            Expr::Column {
                table: Some(t),
                table_span: Some(span),
                ..
            } if names.contains(&up(t)) => out.push(*span),
            _ => {}
        }
        push_children(e, &mut stack);
    }
    out
}

/// The table of the `table[column]` reference at exactly `span`.
fn column_table_at(expr: &Expr, span: Span) -> Option<&str> {
    let mut stack = vec![expr];
    while let Some(e) = stack.pop() {
        if let Expr::Column {
            table: Some(t),
            span: s,
            ..
        } = e
            && *s == span
        {
            return Some(t);
        }
        push_children(e, &mut stack);
    }
    None
}
