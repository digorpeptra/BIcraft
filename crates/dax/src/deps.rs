//! Dependencies between model objects and circular-reference detection (M6.5).
//!
//! Every measure, calculated column, calculated table and row-level security filter is parsed and
//! its references are resolved like the binder does. The graph has one node per model table,
//! column, measure and row filter; an edge `a → b` means *a depends on b*. Cycles are the
//! strongly connected components of that graph (Tarjan, iterative, so hostile models cannot
//! exhaust the stack), plus self-loops.
//!
//! Not modelled: the implicit dependency of a calculated column that uses `CALCULATE` (context
//! transition) on every other column of its table, which the engine reports as a cycle only when
//! the table has no key column.

use std::collections::{HashMap, HashSet};
use std::fmt;

use bicraft_model::SemanticModel;
use serde::Serialize;

use crate::ast::Expr;
use crate::binder::{Resolved, Scope, collect_declared_columns, resolve};
use crate::index::{ModelIndex, Names};
use crate::{Diagnostic, Severity, Span, lexer, parser};

/// A model object that can depend on, or be depended on by, a DAX expression.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ObjectRef {
    Measure {
        table: String,
        name: String,
    },
    /// A data or calculated column.
    Column {
        table: String,
        name: String,
    },
    /// A table as a whole (a calculated table's definition, or any table referenced by name).
    Table {
        name: String,
    },
    /// The row-level security filter of `role` on `table`.
    RowFilter {
        role: String,
        table: String,
    },
}

fn quote(name: &str) -> String {
    format!("'{}'", name.replace('\'', "''"))
}

impl fmt::Display for ObjectRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ObjectRef::Measure { name, .. } => write!(f, "[{name}]"),
            ObjectRef::Column { table, name } => write!(f, "{}[{name}]", quote(table)),
            ObjectRef::Table { name } => write!(f, "{}", quote(name)),
            ObjectRef::RowFilter { role, table } => {
                write!(f, "row filter of role {role} on {}", quote(table))
            }
        }
    }
}

impl ObjectRef {
    /// Case-insensitive identity (DAX object names are case-insensitive).
    fn key(&self) -> (u8, String, String) {
        let up = |s: &str| s.to_ascii_uppercase();
        match self {
            ObjectRef::Measure { name, .. } => (0, String::new(), up(name)),
            ObjectRef::Column { table, name } => (1, up(table), up(name)),
            ObjectRef::Table { name } => (2, up(name), String::new()),
            ObjectRef::RowFilter { role, table } => (3, up(role), up(table)),
        }
    }
}

/// `edges[i]` lists the nodes `nodes[i]` depends on (sorted, without duplicates).
#[derive(Debug, Clone, Default, Serialize)]
pub struct DependencyGraph {
    pub nodes: Vec<ObjectRef>,
    pub edges: Vec<Vec<usize>>,
    #[serde(skip)]
    index: HashMap<(u8, String, String), usize>,
}

impl DependencyGraph {
    /// Index of `obj` in [`Self::nodes`] (case-insensitive).
    pub fn index_of(&self, obj: &ObjectRef) -> Option<usize> {
        self.index.get(&obj.key()).copied()
    }

    /// What `obj` depends on directly.
    pub fn dependencies_of(&self, obj: &ObjectRef) -> Vec<&ObjectRef> {
        self.index_of(obj)
            .and_then(|i| self.edges.get(i))
            .map(|e| e.iter().filter_map(|&j| self.nodes.get(j)).collect())
            .unwrap_or_default()
    }

    /// What depends on `obj` directly.
    pub fn dependents_of(&self, obj: &ObjectRef) -> Vec<&ObjectRef> {
        let Some(target) = self.index_of(obj) else {
            return Vec::new();
        };
        self.edges
            .iter()
            .enumerate()
            .filter(|(_, e)| e.binary_search(&target).is_ok())
            .filter_map(|(i, _)| self.nodes.get(i))
            .collect()
    }

    fn add_node(&mut self, obj: ObjectRef) -> usize {
        let key = obj.key();
        if let Some(&i) = self.index.get(&key) {
            return i;
        }
        let i = self.nodes.len();
        self.nodes.push(obj);
        self.edges.push(Vec::new());
        self.index.insert(key, i);
        i
    }
}

fn is_calculated_partition(source_type: &str, expression: Option<&str>) -> bool {
    source_type.eq_ignore_ascii_case("calculated") && expression.is_some()
}

/// The DAX expression defining a calculated table, if `table` is one.
pub fn calculated_table_expression(table: &bicraft_model::Table) -> Option<&str> {
    table
        .partitions
        .iter()
        .find(|p| is_calculated_partition(&p.source_type, p.expression.as_deref()))
        .and_then(|p| p.expression.as_deref())
}

/// Builds the dependency graph of every DAX object in the model.
pub fn dependencies(model: &SemanticModel) -> DependencyGraph {
    let index = ModelIndex::new(model);
    let names = Names::Index(&index);
    let mut g = DependencyGraph::default();
    for t in &model.tables {
        g.add_node(ObjectRef::Table { name: t.name.clone() });
        for c in t.columns.iter().filter(|c| !c.is_row_number()) {
            g.add_node(ObjectRef::Column {
                table: t.name.clone(),
                name: c.name.clone(),
            });
        }
        for m in &t.measures {
            g.add_node(ObjectRef::Measure {
                table: t.name.clone(),
                name: m.name.clone(),
            });
        }
    }
    for r in &model.roles {
        for tp in r
            .table_permissions
            .iter()
            .filter(|tp| tp.filter_expression.is_some())
        {
            g.add_node(ObjectRef::RowFilter {
                role: r.name.clone(),
                table: tp.table.clone(),
            });
        }
    }

    let add_edges = |g: &mut DependencyGraph, from: ObjectRef, deps: Vec<ObjectRef>| {
        let Some(i) = g.index_of(&from) else { return };
        let mut targets: Vec<usize> = deps.iter().filter_map(|d| g.index_of(d)).collect();
        if let Some(e) = g.edges.get_mut(i) {
            e.append(&mut targets);
            e.sort_unstable();
            e.dedup();
        }
    };
    for t in &model.tables {
        let table_ref = ObjectRef::Table { name: t.name.clone() };
        let calculated = calculated_table_expression(t);
        if let Some(expr) = calculated {
            let deps = references(names, expr, None, false);
            add_edges(&mut g, table_ref.clone(), deps);
        }
        for c in t.columns.iter().filter(|c| !c.is_row_number()) {
            let col = ObjectRef::Column {
                table: t.name.clone(),
                name: c.name.clone(),
            };
            let mut deps = c
                .expression
                .as_deref()
                .map(|e| references(names, e, Some(&t.name), true))
                .unwrap_or_default();
            // The columns of a calculated table exist only through its definition.
            if calculated.is_some() {
                deps.push(table_ref.clone());
            }
            add_edges(&mut g, col, deps);
        }
        for m in &t.measures {
            let deps = references(names, &m.expression, Some(&t.name), false);
            let from = ObjectRef::Measure {
                table: t.name.clone(),
                name: m.name.clone(),
            };
            add_edges(&mut g, from, deps);
        }
    }
    for r in &model.roles {
        for tp in &r.table_permissions {
            let Some(expr) = tp.filter_expression.as_deref() else {
                continue;
            };
            let deps = references(names, expr, Some(&tp.table), true);
            let from = ObjectRef::RowFilter {
                role: r.name.clone(),
                table: tp.table.clone(),
            };
            add_edges(&mut g, from, deps);
        }
    }
    add_context_transition_edges(names, &mut g);
    g
}

/// The model objects a DAX expression references directly. `home` is its home table;
/// `row_context` is true for calculated columns and row filters (an unqualified `[Name]` then
/// prefers a column of the home table over a measure). Unparsable text has no dependencies.
pub fn expression_dependencies(
    model: &SemanticModel,
    text: &str,
    home: Option<&str>,
    row_context: bool,
) -> Vec<ObjectRef> {
    references(Names::Scan(model), text, home, row_context)
}

fn references(names: Names<'_>, text: &str, home: Option<&str>, row_context: bool) -> Vec<ObjectRef> {
    let mut scratch = Vec::new();
    let tokens = lexer::lex(text, &mut scratch);
    let Some(expr) = parser::parse(&tokens, &mut scratch) else {
        return Vec::new();
    };
    let mut declared = HashSet::new();
    collect_declared_columns(&expr, &mut declared);
    let vars = variable_names(&expr);
    let scope = Scope {
        home,
        declared: Some(&declared),
        prefer_column: row_context,
    };
    let mut out: Vec<ObjectRef> = Vec::new();
    let mut seen = HashSet::new();
    let mut push = |o: ObjectRef| {
        if seen.insert(o.key()) {
            out.push(o);
        }
    };
    let mut stack = vec![&expr];
    while let Some(e) = stack.pop() {
        match e {
            Expr::Name { name, .. } => {
                // A variable wins over a table of the same name, even quoted.
                let is_var = vars.contains(&name.to_ascii_uppercase());
                if !is_var && let Some(t) = names.table(name) {
                    push(ObjectRef::Table { name: t.name.clone() });
                }
            }
            Expr::Column {
                table,
                name,
                variation,
                ..
            } => {
                let table_is_var = table
                    .as_deref()
                    .is_some_and(|t| vars.contains(&t.to_ascii_uppercase()));
                match resolve(names, scope, table.as_deref(), name, table_is_var, &[]) {
                    Resolved::Column(t, c) => {
                        push(ObjectRef::Column {
                            table: t.name.clone(),
                            name: c.name.clone(),
                        });
                        // `T[Date].[Year]` also reads the variation table's column.
                        if let Some((vt, vc)) = variation
                            .as_ref()
                            .and_then(|v| names.model().variation_column(&t.name, &c.name, &v.name))
                        {
                            push(ObjectRef::Column {
                                table: vt.name.clone(),
                                name: vc.name.clone(),
                            });
                        }
                    }
                    Resolved::Measure(t, m) => push(ObjectRef::Measure {
                        table: t.name.clone(),
                        name: m.name.clone(),
                    }),
                    Resolved::Local | Resolved::Unresolved => {}
                }
            }
            Expr::Call { args, .. } => stack.extend(args.iter().flatten()),
            Expr::Unary { operand, .. } => stack.push(operand),
            Expr::Binary { lhs, rhs, .. } => {
                stack.push(lhs);
                stack.push(rhs);
            }
            Expr::Tuple { items, .. } => stack.extend(items.iter()),
            Expr::TableCtor { rows, .. } => stack.extend(rows.iter()),
            Expr::Var { bindings, body, .. } => {
                stack.extend(bindings.iter().map(|b| &b.value));
                stack.push(body);
            }
            Expr::Number { .. } | Expr::Str { .. } | Expr::Bool { .. } => {}
        }
    }
    // Deterministic order.
    out.sort();
    out
}

/// Every `VAR` name in the expression (upper-case), whatever its scope.
fn variable_names(expr: &Expr) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut stack = vec![expr];
    while let Some(e) = stack.pop() {
        match e {
            Expr::Var { bindings, body, .. } => {
                for b in bindings {
                    out.insert(b.name.to_ascii_uppercase());
                    stack.push(&b.value);
                }
                stack.push(body);
            }
            Expr::Call { args, .. } => stack.extend(args.iter().flatten()),
            Expr::Unary { operand, .. } => stack.push(operand),
            Expr::Binary { lhs, rhs, .. } => {
                stack.push(lhs);
                stack.push(rhs);
            }
            Expr::Tuple { items, .. } => stack.extend(items.iter()),
            Expr::TableCtor { rows, .. } => stack.extend(rows.iter()),
            _ => {}
        }
    }
    out
}

/// Strongly connected components of `edges` that contain a cycle (more than one node, or one node
/// with a self-loop). Iterative Tarjan. Each component is ordered by a depth-first walk inside it
/// from its first node, so a simple cycle comes out in dependency order.
pub(crate) fn cyclic_components(edges: &[Vec<usize>]) -> Vec<Vec<usize>> {
    struct Tarjan<'e> {
        edges: &'e [Vec<usize>],
        index: Vec<Option<usize>>,
        low: Vec<usize>,
        on_stack: Vec<bool>,
        stack: Vec<usize>,
        /// Explicit call stack: (node, next successor position).
        calls: Vec<(usize, usize)>,
        next: usize,
    }
    impl Tarjan<'_> {
        fn enter(&mut self, v: usize) {
            if let Some(slot) = self.index.get_mut(v) {
                *slot = Some(self.next);
            }
            if let Some(slot) = self.low.get_mut(v) {
                *slot = self.next;
            }
            self.next = self.next.saturating_add(1);
            self.stack.push(v);
            if let Some(slot) = self.on_stack.get_mut(v) {
                *slot = true;
            }
            self.calls.push((v, 0));
        }
        fn lower(&mut self, v: usize, to: usize) {
            if let Some(l) = self.low.get_mut(v) {
                *l = (*l).min(to);
            }
        }
    }

    let n = edges.len();
    let mut t = Tarjan {
        edges,
        index: vec![None; n],
        low: vec![0; n],
        on_stack: vec![false; n],
        stack: Vec::new(),
        calls: Vec::new(),
        next: 0,
    };
    let mut sccs = Vec::new();
    for root in 0..n {
        if t.index.get(root).copied().flatten().is_some() {
            continue;
        }
        t.enter(root);
        while let Some(&(v, pos)) = t.calls.last() {
            let succ = t.edges.get(v).and_then(|e| e.get(pos)).copied();
            if let Some(w) = succ {
                if let Some(top) = t.calls.last_mut() {
                    top.1 = pos.saturating_add(1);
                }
                match t.index.get(w).copied().flatten() {
                    None => t.enter(w),
                    Some(iw) if t.on_stack.get(w).copied().unwrap_or(false) => t.lower(v, iw),
                    Some(_) => {}
                }
                continue;
            }
            t.calls.pop();
            let lv = t.low.get(v).copied().unwrap_or(0);
            if let Some(&(u, _)) = t.calls.last() {
                t.lower(u, lv);
            }
            if Some(lv) == t.index.get(v).copied().flatten() {
                let mut members = Vec::new();
                while let Some(w) = t.stack.pop() {
                    if let Some(slot) = t.on_stack.get_mut(w) {
                        *slot = false;
                    }
                    members.push(w);
                    if w == v {
                        break;
                    }
                }
                let self_loop = edges.get(v).is_some_and(|e| e.contains(&v));
                if members.len() > 1 || self_loop {
                    sccs.push(order_component(&members, edges));
                }
            }
        }
    }
    sccs.sort_by_key(|c| c.first().copied());
    sccs
}

/// Orders a component by an iterative depth-first walk inside it from its smallest node.
fn order_component(members: &[usize], edges: &[Vec<usize>]) -> Vec<usize> {
    let inside: HashSet<usize> = members.iter().copied().collect();
    let Some(&start) = members.iter().min() else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut order = Vec::with_capacity(members.len());
    let mut stack = vec![start];
    while let Some(v) = stack.pop() {
        if !seen.insert(v) {
            continue;
        }
        order.push(v);
        if let Some(succ) = edges.get(v) {
            // Reversed so the first successor is visited first.
            for &w in succ.iter().rev() {
                if inside.contains(&w) && !seen.contains(&w) {
                    stack.push(w);
                }
            }
        }
    }
    order
}

/// Every circular dependency in the model: one entry per strongly connected component of the
/// dependency graph. For a simple cycle the entry is in dependency order (each object depends on
/// the next, the last on the first); a self-reference is a one-element entry.
pub fn find_cycles(model: &SemanticModel) -> Vec<Vec<ObjectRef>> {
    let g = dependencies(model);
    cycles_of(&g)
}

/// [`find_cycles`] on an already built graph.
pub fn cycles_of(g: &DependencyGraph) -> Vec<Vec<ObjectRef>> {
    cyclic_components(&g.edges)
        .into_iter()
        .map(|c| c.iter().filter_map(|&i| g.nodes.get(i).cloned()).collect())
        .collect()
}

/// One `circular-dependency` error per object involved in a cycle, worded from that object's
/// point of view. The span is empty: the cycle is a property of the model, not of one token.
pub fn cycle_diagnostics(model: &SemanticModel) -> Vec<(ObjectRef, Diagnostic)> {
    cycle_diagnostics_of(&dependencies(model))
}

/// [`cycle_diagnostics`] on an already built graph.
pub fn cycle_diagnostics_of(g: &DependencyGraph) -> Vec<(ObjectRef, Diagnostic)> {
    let mut out = Vec::new();
    for comp in cyclic_components(&g.edges) {
        let inside: HashSet<usize> = comp.iter().copied().collect();
        let simple = comp.iter().all(|&v| {
            g.edges
                .get(v)
                .is_some_and(|e| e.iter().filter(|w| inside.contains(w)).count() == 1)
        });
        for (pos, &v) in comp.iter().enumerate() {
            let Some(obj) = g.nodes.get(v) else { continue };
            let message = if simple {
                let path: Vec<String> = comp
                    .iter()
                    .cycle()
                    .skip(pos)
                    .take(comp.len().saturating_add(1))
                    .filter_map(|&i| g.nodes.get(i).map(ToString::to_string))
                    .collect();
                format!("circular dependency: {}", path.join(" -> "))
            } else {
                let names: Vec<String> = comp
                    .iter()
                    .filter_map(|&i| g.nodes.get(i).map(ToString::to_string))
                    .collect();
                format!(
                    "circular dependency: {obj} is part of a dependency cycle between {}",
                    names.join(", ")
                )
            };
            out.push((
                obj.clone(),
                Diagnostic {
                    severity: Severity::Error,
                    code: "circular-dependency",
                    message,
                    span: Span::default(),
                    line: 1,
                    column: 1,
                },
            ));
        }
    }
    out
}

/// Whether a calculated column's expression triggers a context transition (`CALCULATE`,
/// `CALCULATETABLE`, or a measure reference, which is an implicit `CALCULATE`).
fn uses_context_transition(names: Names<'_>, text: &str, home: &str) -> bool {
    let mut scratch = Vec::new();
    let tokens = lexer::lex(text, &mut scratch);
    let Some(expr) = parser::parse(&tokens, &mut scratch) else {
        return false;
    };
    let mut stack = vec![&expr];
    while let Some(e) = stack.pop() {
        match e {
            Expr::Call { name, args, .. } => {
                if name.eq_ignore_ascii_case("CALCULATE") || name.eq_ignore_ascii_case("CALCULATETABLE") {
                    return true;
                }
                stack.extend(args.iter().flatten());
            }
            Expr::Unary { operand, .. } => stack.push(operand),
            Expr::Binary { lhs, rhs, .. } => {
                stack.push(lhs);
                stack.push(rhs);
            }
            Expr::Tuple { items, .. } => stack.extend(items.iter()),
            Expr::TableCtor { rows, .. } => stack.extend(rows.iter()),
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
    references(names, text, Some(home), true)
        .iter()
        .any(|r| matches!(r, ObjectRef::Measure { .. }))
}

/// A calculated column with a context transition filters every column of its table, so it
/// depends on the table's other calculated columns, except those that already depend on it. Two
/// such columns therefore depend on each other: the engine's classic circular dependency. Tables
/// with a key column (or on the one side of a relationship) are skipped: there the engine can
/// filter the key alone.
fn add_context_transition_edges(names: Names<'_>, g: &mut DependencyGraph) {
    let model = names.model();
    let keyed: HashSet<String> = model
        .relationships
        .iter()
        .map(|r| r.to_table.to_ascii_uppercase())
        .collect();
    let mut reverse: Vec<Vec<usize>> = vec![Vec::new(); g.nodes.len()];
    for (from, targets) in g.edges.iter().enumerate() {
        for &to in targets {
            if let Some(r) = reverse.get_mut(to) {
                r.push(from);
            }
        }
    }
    let mut extra: Vec<(usize, usize)> = Vec::new();
    for t in &model.tables {
        let has_key = t
            .columns
            .iter()
            .any(|c| c.extra.get("isKey").and_then(|v| v.as_bool()) == Some(true));
        if has_key || keyed.contains(&t.name.to_ascii_uppercase()) {
            continue;
        }
        let calculated: Vec<(usize, &str)> = t
            .columns
            .iter()
            .filter(|c| !c.is_row_number())
            .filter_map(|c| {
                let expr = c.expression.as_deref()?;
                let id = g.index_of(&ObjectRef::Column {
                    table: t.name.clone(),
                    name: c.name.clone(),
                })?;
                Some((id, expr))
            })
            .collect();
        if calculated.len() < 2 {
            continue;
        }
        for &(a, expr) in &calculated {
            if !uses_context_transition(names, expr, &t.name) {
                continue;
            }
            // Everything that already depends on `a`, directly or not.
            let mut upstream: HashSet<usize> = HashSet::new();
            let mut stack = vec![a];
            while let Some(v) = stack.pop() {
                for &w in reverse.get(v).map_or(&[][..], Vec::as_slice) {
                    if upstream.insert(w) {
                        stack.push(w);
                    }
                }
            }
            for &(b, _) in &calculated {
                if b != a && !upstream.contains(&b) {
                    extra.push((a, b));
                }
            }
        }
    }
    for (a, b) in extra {
        if let Some(e) = g.edges.get_mut(a)
            && let Err(pos) = e.binary_search(&b)
        {
            e.insert(pos, b);
        }
    }
}
