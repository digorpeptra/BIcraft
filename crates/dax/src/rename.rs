//! Renaming model objects inside DAX text (G15).
//!
//! When a table, column or measure is renamed, every expression that references it must follow,
//! as Power BI Desktop does. [`Renamer::rewrite`] lexes and parses the expression, resolves each
//! reference against the model *as it is before the rename* (same rules as the binder and the
//! type checker, including the row contexts iterators create), and replaces only the tokens that
//! denote the renamed object:
//!
//! - a table: `'Old'[x]`, `Old[x]`, `'Old'` and bare `Old` (not when a variable of that name is in
//!   scope: the engine resolves `Old` and even `'Old'` to the variable, checked with Power BI
//!   Desktop 2.158); the new name keeps the old reference's quoting, and is quoted when it must be
//!   (spaces, non-ASCII, a reserved word);
//! - a column: the `[name]` part of `T[Old]` / `[Old]` (and of `.[Old]` in a column variation);
//! - a measure: the `[name]` part of `[Old]` / `T[Old]`.
//!
//! [`Renamer::move_measure`] rewrites the `T` of `T[Measure]` when a measure moves to another
//! table. A user-defined function of the model (`(params) => body`, [`Site::udf`]) is handled
//! like any expression, its parameters being variables.
//!
//! Everything else keeps its bytes: whitespace, comments, strings, the casing of other names.
//! An expression that does not parse is left alone ([`Rewrite::Unparsable`]).
//!
//! [`Renamer::references`] lists the model objects an expression references with the same
//! resolution, for the "what breaks if I remove this" check of removals.

use std::collections::HashSet;

use bicraft_model::{SemanticModel, Table};

use crate::ast::{Define, Expr};
use crate::binder::{Resolved, Scope, collect_declared_columns, resolve};
use crate::deps::ObjectRef;
use crate::functions::{is_keyword_argument, signature};
use crate::index::{ModelIndex, Names};
use crate::lexer::{Token, TokenKind};
use crate::reserved::table_needs_quotes;
use crate::types::{ParamKind, Signature};
use crate::{Span, lexer, parser};

/// Deepest expression nesting walked (the parser already caps it at [`crate::MAX_DEPTH`]).
const MAX_WALK_DEPTH: usize = 1024;

/// What [`Renamer::rewrite`] did to one expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rewrite {
    /// No reference to the renamed object.
    Unchanged,
    /// The new text and how many references were rewritten. `captured` counts the references
    /// left as they are because a variable named like the new table name is in scope there: the
    /// engine resolves a table name (quoted or not) to a variable first, so the new name would
    /// denote the variable. When every reference was captured, `references` is 0 and `text` is
    /// the original.
    Rewritten {
        text: String,
        references: usize,
        captured: usize,
    },
    /// The expression does not parse, so its references cannot be told apart from strings,
    /// comments or other objects: it is left as it is.
    Unparsable,
}

/// Resolves and rewrites references against one model snapshot (the model before the rename).
#[derive(Debug)]
pub struct Renamer<'m> {
    index: ModelIndex<'m>,
}

/// Where an expression lives: its home table and whether it is evaluated in a row context over
/// it (calculated columns and row-level security filters).
#[derive(Debug, Clone, Copy, Default)]
pub struct Site<'a> {
    pub home: Option<&'a str>,
    pub row_context: bool,
    /// The text is a user-defined function of the model, `(params) => body` (TMSL/TMDL
    /// `functions`): parameters are variables of the body.
    pub udf: bool,
}

/// Prefix that turns a model function's text into a query the parser reads.
const UDF_PREFIX: &str = "DEFINE FUNCTION BicraftUdf = ";
/// Its end: a query needs an EVALUATE.
const UDF_SUFFIX: &str = "\nEVALUATE {0}";

/// One resolved reference in an expression.
#[derive(Debug, Clone)]
struct Hit {
    /// The bytes to replace.
    span: Span,
    what: HitKind,
    /// The table qualifier of a `T[name]` column or measure reference (a [`HitKind::Bracket`]).
    qualifier: Option<Span>,
    /// The object the token denotes.
    object: ObjectRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HitKind {
    /// A table name (`Old` / `'Old'`), alone or qualifying a column.
    Table {
        quoted: bool,
        /// Qualifies a column (`Old[x]`): a reserved word must then be quoted.
        qualifier: bool,
        /// A variable named like the *new* table name is in scope here, so a bare new name
        /// would denote the variable.
        new_name_shadowed: bool,
    },
    /// The `[name]` token of a column or measure reference.
    Bracket,
}

impl<'m> Renamer<'m> {
    pub fn new(model: &'m SemanticModel) -> Self {
        Renamer {
            index: ModelIndex::new(model),
        }
    }

    /// Rewrites the references to `target` (a table, column or measure of the model, named as it
    /// is before the rename) in `text` so that they use `new_name`.
    pub fn rewrite(&self, text: &str, site: Site<'_>, target: &ObjectRef, new_name: &str) -> Rewrite {
        let avoid = matches!(target, ObjectRef::Table { .. }).then(|| new_name.to_ascii_uppercase());
        let Some(hits) = self.hits(text, site, avoid.as_deref()) else {
            return Rewrite::Unparsable;
        };
        let mut edits: Vec<(Span, String)> = Vec::new();
        let mut captured = 0usize;
        for h in &hits {
            let replacement = match (&h.what, target, &h.object) {
                (
                    HitKind::Table {
                        quoted,
                        qualifier,
                        new_name_shadowed,
                    },
                    ObjectRef::Table { name: old },
                    ObjectRef::Table { name },
                ) if name.eq_ignore_ascii_case(old) => {
                    // `x[Column]` resolves against tables only, so a variable cannot capture a
                    // qualifier.
                    if *new_name_shadowed && !*qualifier {
                        captured = captured.saturating_add(1);
                        continue;
                    }
                    table_reference(new_name, *quoted, *qualifier)
                }
                (
                    HitKind::Bracket,
                    ObjectRef::Column { table: tt, name: tn },
                    ObjectRef::Column { table, name },
                )
                | (
                    HitKind::Bracket,
                    ObjectRef::Measure { table: tt, name: tn },
                    ObjectRef::Measure { table, name },
                ) if table.eq_ignore_ascii_case(tt) && name.eq_ignore_ascii_case(tn) => bracket(new_name),
                _ => continue,
            };
            edits.push((h.span, replacement));
        }
        splice(text, edits, captured)
    }

    /// Rewrites the table qualifier of every `T[Measure]` reference to `target` (a measure of the
    /// model, in the table it is in before the move) so that it names `to_table`: the measure moves
    /// there. Unqualified `[Measure]` references stay as they are.
    pub fn move_measure(&self, text: &str, site: Site<'_>, target: &ObjectRef, to_table: &str) -> Rewrite {
        let ObjectRef::Measure { table: tt, name: tn } = target else {
            return Rewrite::Unchanged;
        };
        let Some(hits) = self.hits(text, site, None) else {
            return Rewrite::Unparsable;
        };
        let mut edits: Vec<(Span, String)> = Vec::new();
        for h in &hits {
            if let (HitKind::Bracket, ObjectRef::Measure { table, name }, Some(q)) =
                (&h.what, &h.object, h.qualifier)
                && table.eq_ignore_ascii_case(tt)
                && name.eq_ignore_ascii_case(tn)
            {
                let quoted = text.get(q.start..).is_some_and(|s| s.starts_with('\''));
                edits.push((q, table_reference(to_table, quoted, true)));
            }
        }
        splice(text, edits, 0)
    }

    /// The model objects `text` references (tables, columns, measures), resolved like
    /// [`Renamer::rewrite`] does; `None` when it does not parse.
    pub fn references(&self, text: &str, site: Site<'_>) -> Option<Vec<ObjectRef>> {
        let hits = self.hits(text, site, None)?;
        let mut seen = HashSet::new();
        let mut out: Vec<ObjectRef> = hits
            .into_iter()
            .map(|h| h.object)
            .filter(|o| seen.insert(key(o)))
            .collect();
        out.sort();
        Some(out)
    }

    /// [`Renamer::references`] without the tables named only as qualifiers (`Sales` in
    /// `Sales[Amount]`): the tables listed are used as tables (`COUNTROWS(Sales)`, `ALL(Sales)`).
    pub fn references_as_used(&self, text: &str, site: Site<'_>) -> Option<Vec<ObjectRef>> {
        let hits = self.hits(text, site, None)?;
        let mut seen = HashSet::new();
        let mut out: Vec<ObjectRef> = hits
            .into_iter()
            .filter(|h| !matches!(h.what, HitKind::Table { qualifier: true, .. }))
            .map(|h| h.object)
            .filter(|o| seen.insert(key(o)))
            .collect();
        out.sort();
        Some(out)
    }

    fn hits(&self, text: &str, site: Site<'_>, avoid: Option<&str>) -> Option<Vec<Hit>> {
        if site.udf {
            return self.udf_hits(text, site, avoid);
        }
        let mut scratch = Vec::new();
        let tokens = lexer::lex(text, &mut scratch);
        let expr = parser::parse(&tokens, &mut scratch)?;
        let declared = declared_columns(&[&expr]);
        let mut w = self.walker(text, &tokens, site, avoid, &declared);
        w.visit(&expr, 0);
        Some(w.hits)
    }

    /// [`Renamer::hits`] of a user-defined function `(params) => body`: parsed as the function
    /// of a query, its parameters in scope as variables of the defaults that follow them and of
    /// the body.
    fn udf_hits(&self, text: &str, site: Site<'_>, avoid: Option<&str>) -> Option<Vec<Hit>> {
        let query = format!("{UDF_PREFIX}{text}{UDF_SUFFIX}");
        let mut scratch = Vec::new();
        let tokens = lexer::lex_query(&query, &mut scratch);
        let parsed = parser::parse_query(&tokens, &mut scratch)?;
        let Some(Define::Function { params, body, .. }) = parsed.defines.first() else {
            return None;
        };
        let mut roots: Vec<&Expr> = params.iter().filter_map(|p| p.default.as_ref()).collect();
        roots.push(body);
        let declared = declared_columns(&roots);
        let mut w = self.walker(&query, &tokens, site, avoid, &declared);
        for p in params {
            if let Some(d) = &p.default {
                w.visit(d, 0);
            }
            w.vars.push(p.name.to_ascii_uppercase());
        }
        w.visit(body, 0);
        // Back to offsets in `text`; anything outside it (cannot happen) is dropped.
        let offset = UDF_PREFIX.len();
        let shift = |s: Span| -> Option<Span> {
            let start = s.start.checked_sub(offset)?;
            let end = s.end.checked_sub(offset)?;
            (end <= text.len()).then_some(Span { start, end })
        };
        Some(
            w.hits
                .into_iter()
                .filter_map(|h| {
                    let qualifier = match h.qualifier {
                        Some(q) => Some(shift(q)?),
                        None => None,
                    };
                    Some(Hit {
                        span: shift(h.span)?,
                        qualifier,
                        ..h
                    })
                })
                .collect(),
        )
    }

    fn walker<'w>(
        &'w self,
        text: &'w str,
        tokens: &'w [Token],
        site: Site<'_>,
        avoid: Option<&'w str>,
        declared: &'w HashSet<String>,
    ) -> Walker<'w, 'w> {
        let names = Names::Index(&self.index);
        let home = site.home.and_then(|h| names.table(h)).map(|t| t.name.as_str());
        let mut w = Walker {
            names,
            text,
            tokens,
            scope: Scope {
                home,
                declared: Some(declared),
                prefer_column: site.row_context,
            },
            vars: Vec::new(),
            rows: Vec::new(),
            avoid,
            hits: Vec::new(),
        };
        if site.row_context
            && let Some(h) = home
        {
            w.rows.push(h.to_string());
        }
        w
    }
}

/// Column names the expressions declare themselves (`ADDCOLUMNS`-like functions).
fn declared_columns(roots: &[&Expr]) -> HashSet<String> {
    let mut declared = HashSet::new();
    for e in roots {
        collect_declared_columns(e, &mut declared);
    }
    declared
}

/// Applies `edits` (spans of `text` and their replacements) to `text`.
fn splice(text: &str, mut edits: Vec<(Span, String)>, captured: usize) -> Rewrite {
    if edits.is_empty() {
        return if captured > 0 {
            Rewrite::Rewritten {
                text: text.to_string(),
                references: 0,
                captured,
            }
        } else {
            Rewrite::Unchanged
        };
    }
    edits.sort_by_key(|(s, _)| s.start);
    let mut out = String::with_capacity(text.len().saturating_add(16));
    let mut at = 0usize;
    let mut references = 0usize;
    for (span, replacement) in &edits {
        // Overlapping or out-of-range spans cannot happen with a parsed tree; skip them rather
        // than produce garbage.
        if span.start < at || span.end < span.start || text.get(span.start..span.end).is_none() {
            continue;
        }
        let Some(before) = text.get(at..span.start) else {
            continue;
        };
        out.push_str(before);
        out.push_str(replacement);
        at = span.end;
        references = references.saturating_add(1);
    }
    out.push_str(text.get(at..).unwrap_or_default());
    if out == text && captured == 0 {
        return Rewrite::Unchanged;
    }
    Rewrite::Rewritten {
        text: out,
        references,
        captured,
    }
}

fn key(o: &ObjectRef) -> (u8, String, String) {
    let up = |s: &str| s.to_ascii_uppercase();
    match o {
        ObjectRef::Measure { name, .. } => (0, String::new(), up(name)),
        ObjectRef::Column { table, name } => (1, up(table), up(name)),
        ObjectRef::Table { name } => (2, up(name), String::new()),
        ObjectRef::RowFilter { role, table } => (3, up(role), up(table)),
    }
}

/// `[name]` with `]` escaped.
fn bracket(name: &str) -> String {
    format!("[{}]", name.replace(']', "]]"))
}

/// A table reference to `name`: quoted when the old reference was, or when a bare name would not
/// lex as one identifier or is reserved.
fn table_reference(name: &str, quoted: bool, qualifier: bool) -> String {
    let bare_ok = !table_needs_quotes(name) && !(!qualifier && is_keyword_argument(name));
    if !quoted && bare_ok {
        name.to_string()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

struct Walker<'w, 'm> {
    names: Names<'m>,
    text: &'w str,
    tokens: &'w [Token],
    scope: Scope<'w>,
    /// Variables in scope, innermost last (upper-case).
    vars: Vec<String>,
    /// Tables of the enclosing row contexts, outermost first.
    rows: Vec<String>,
    /// Upper-case new table name: record whether a variable of that name is in scope.
    avoid: Option<&'w str>,
    hits: Vec<Hit>,
}

impl<'m> Walker<'_, 'm> {
    fn is_var(&self, name: &str) -> bool {
        self.vars.iter().any(|v| v.eq_ignore_ascii_case(name))
    }

    fn shadowed(&self) -> bool {
        self.avoid.is_some_and(|a| self.is_var(a))
    }

    fn quoted_at(&self, span: Span) -> bool {
        self.text.get(span.start..).is_some_and(|s| s.starts_with('\''))
    }

    /// The `[name]` token of a column reference: the first token at `from` or after it.
    fn bracket_at(&self, from: usize) -> Option<Span> {
        let i = self.tokens.partition_point(|t| t.span.start < from);
        match self.tokens.get(i) {
            Some(Token {
                kind: TokenKind::Bracket(_),
                span,
            }) => Some(*span),
            _ => None,
        }
    }

    fn table_hit(&mut self, t: &'m Table, span: Span, qualifier: bool) {
        let what = HitKind::Table {
            quoted: self.quoted_at(span),
            qualifier,
            new_name_shadowed: self.shadowed(),
        };
        self.hits.push(Hit {
            span,
            what,
            qualifier: None,
            object: ObjectRef::Table { name: t.name.clone() },
        });
    }

    fn visit(&mut self, e: &Expr, depth: usize) {
        if depth > MAX_WALK_DEPTH {
            return;
        }
        let d = depth.saturating_add(1);
        match e {
            Expr::Number { .. } | Expr::Str { .. } | Expr::Bool { .. } => {}
            Expr::Unary { operand, .. } => self.visit(operand, d),
            Expr::Binary { lhs, rhs, .. } => {
                self.visit(lhs, d);
                self.visit(rhs, d);
            }
            Expr::Tuple { items, .. } => items.iter().for_each(|x| self.visit(x, d)),
            Expr::TableCtor { rows, .. } => rows.iter().for_each(|x| self.visit(x, d)),
            Expr::Var { bindings, body, .. } => {
                let mark = self.vars.len();
                for b in bindings {
                    // A variable is visible only after its own definition.
                    self.visit(&b.value, d);
                    self.vars.push(b.name.to_ascii_uppercase());
                }
                self.visit(body, d);
                self.vars.truncate(mark);
            }
            Expr::Name { name, span, .. } => {
                // A variable wins over a table of the same name, even quoted.
                if self.is_var(name) {
                    return;
                }
                if let Some(t) = self.names.table(name) {
                    self.table_hit(t, *span, false);
                }
            }
            Expr::Column {
                table,
                table_span,
                name,
                variation,
                span,
            } => self.column(table.as_deref(), *table_span, name, variation.as_deref(), *span),
            Expr::Call { name, args, .. } => self.call(name, args, d),
        }
    }

    fn column(
        &mut self,
        table: Option<&str>,
        table_span: Option<Span>,
        name: &str,
        variation: Option<&crate::ast::Variation>,
        span: Span,
    ) {
        // The engine resolves `x[Column]` against tables, never variables.
        let qualifier = table.and_then(|t| self.names.table(t));
        if let (Some(t), Some(ts)) = (qualifier, table_span) {
            self.table_hit(t, ts, true);
        }
        let name_span = match table_span {
            Some(ts) => self.bracket_at(ts.end),
            None => self.bracket_at(span.start),
        };
        let resolved = match table {
            Some(_) => resolve(self.names, self.scope, table, name, false, &[]),
            None => self.unqualified(name),
        };
        let Some(ns) = name_span else { return };
        let qualifier_span = table_span.filter(|_| qualifier.is_some());
        match resolved {
            Resolved::Column(t, c) => {
                self.hits.push(Hit {
                    span: ns,
                    what: HitKind::Bracket,
                    qualifier: qualifier_span,
                    object: ObjectRef::Column {
                        table: t.name.clone(),
                        name: c.name.clone(),
                    },
                });
                if let Some(v) = variation
                    && let Some((vt, vc)) = self.names.model().variation_column(&t.name, &c.name, &v.name)
                {
                    self.hits.push(Hit {
                        span: v.span,
                        what: HitKind::Bracket,
                        qualifier: None,
                        object: ObjectRef::Column {
                            table: vt.name.clone(),
                            name: vc.name.clone(),
                        },
                    });
                }
            }
            Resolved::Measure(t, m) => self.hits.push(Hit {
                span: ns,
                what: HitKind::Bracket,
                qualifier: qualifier_span,
                object: ObjectRef::Measure {
                    table: t.name.clone(),
                    name: m.name.clone(),
                },
            }),
            Resolved::Local | Resolved::Unresolved => {}
        }
    }

    /// An unqualified `[Name]`: a column of the innermost row context that has it (iterators,
    /// then the home table of a calculated column or row filter), else a measure, else a column
    /// the expression declares itself, else a column of the home table, else the first column of
    /// that name in the model.
    fn unqualified(&self, name: &str) -> Resolved<'m> {
        for rt in self.rows.iter().rev() {
            if let Some((t, c)) = self.names.column(rt, name) {
                return Resolved::Column(t, c);
            }
        }
        if let Some((t, m)) = self.names.measure(name) {
            return Resolved::Measure(t, m);
        }
        if self
            .scope
            .declared
            .is_some_and(|d| d.contains(&name.to_ascii_uppercase()))
        {
            return Resolved::Local;
        }
        if let Some((t, c)) = self.scope.home.and_then(|h| self.names.column(h, name)) {
            return Resolved::Column(t, c);
        }
        self.names
            .any_column(name)
            .map_or(Resolved::Unresolved, |(t, c)| Resolved::Column(t, c))
    }

    fn call(&mut self, name: &str, args: &[Option<Expr>], d: usize) {
        let up = name.to_ascii_uppercase();
        let Some(sig) = signature(name) else {
            args.iter().flatten().for_each(|a| self.visit(a, d));
            return;
        };
        let argc = args.len();
        let kinds: Vec<Option<ParamKind>> = (0..argc)
            .map(|i| {
                param_index(sig, i, argc)
                    .and_then(|p| sig.params.get(p))
                    .map(|p| p.kind)
            })
            .collect();
        let generate = matches!(up.as_str(), "GENERATE" | "GENERATEALL");
        for (i, arg) in args.iter().enumerate() {
            let Some(arg) = arg else { continue };
            let row = if generate && i == 1 {
                Some(args.first().and_then(Option::as_ref).and_then(|a| self.row_of(a)))
            } else if kinds.get(i).copied().flatten() == Some(ParamKind::RowExpression) {
                Some(self.iterated_row(i, args, &kinds))
            } else {
                None
            };
            match row {
                Some(Some(t)) => {
                    self.rows.push(t);
                    self.visit(arg, d);
                    self.rows.pop();
                }
                _ => self.visit(arg, d),
            }
        }
    }

    /// The table an iterator's row expression iterates: that of the nearest preceding table
    /// argument.
    fn iterated_row(&self, i: usize, args: &[Option<Expr>], kinds: &[Option<ParamKind>]) -> Option<String> {
        (0..i)
            .rev()
            .find_map(|j| match (kinds.get(j).copied().flatten(), args.get(j)) {
                (Some(ParamKind::Table | ParamKind::TableRef), Some(Some(a))) => Some(self.row_of(a)),
                _ => None,
            })?
    }

    /// The model table a table expression iterates, when it is statically known.
    fn row_of(&self, e: &Expr) -> Option<String> {
        let mut cur = e;
        for _ in 0..MAX_WALK_DEPTH {
            match cur {
                Expr::Name { name, .. } => {
                    if self.is_var(name) {
                        return None;
                    }
                    return self.names.table(name).map(|t| t.name.clone());
                }
                Expr::Column { table: Some(t), .. } => {
                    return self.names.table(t).map(|t| t.name.clone());
                }
                Expr::Call { name, args, .. } => {
                    let index = match name.to_ascii_uppercase().as_str() {
                        "FILTER" | "CALCULATETABLE" | "ADDCOLUMNS" | "KEEPFILTERS" | "RELATEDTABLE"
                        | "ALL" | "ALLNOBLANKROW" | "ALLSELECTED" | "ALLEXCEPT" | "DISTINCT" | "VALUES"
                        | "ALLCROSSFILTERED" => 0,
                        "TOPN" | "SAMPLE" => 1,
                        _ => return None,
                    };
                    cur = args.get(index)?.as_ref()?;
                }
                _ => return None,
            }
        }
        None
    }
}

/// Maps argument `i` (of `argc`) to its parameter: `params[..repeat_from]` is a fixed prefix,
/// then a repeating unit, then an optional tail. Units of several optional params (matched by
/// shape in the engine) and counts that fit no layout map to `None`.
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
    let t = (required_tail..=tail.len())
        .find(|&t| t <= region && (region.saturating_sub(t)) % group.len() == 0)?;
    let j = i.checked_sub(r)?;
    let in_group = region.checked_sub(t)?;
    if j < in_group {
        r.checked_add(j % group.len())
    } else {
        let k = j.checked_sub(in_group)?;
        (k < tail.len()).then(|| r.saturating_add(group.len()).saturating_add(k))
    }
}
