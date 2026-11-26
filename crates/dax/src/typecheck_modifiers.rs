//! Filter modifiers (G19) and the `SUMMARIZECOLUMNS` rules found with them, probed on the engine
//! (oracle categories `modifiers` and `sections-2`, see `docs/dax-oracle.md`).
//!
//! A *filter modifier* changes the filter context instead of producing a value: `USERELATIONSHIP`,
//! `CROSSFILTER`, `REMOVEFILTERS`, `ALLCROSSFILTERED`, `ALL()`/`ALLSELECTED()` without arguments,
//! `KEEPFILTERS` and `NONVISUAL`. Their type stays what the catalog says (the serialized result
//! type does not change); the type checker marks them internally ([`Shape::Modifier`]) and checks
//! where they stand. Each argument position gets a [`Slot`]; [`placement`] tells, per modifier,
//! slot and kind of object, whether the engine accepts it, rejects the definition, or only fails
//! when evaluating it. Measures are the lenient case: the engine defers some misplacements to
//! evaluation there, but rejects them at definition in calculated columns, calculated tables and
//! row filters.
//!
//! Codes:
//! - `misplaced-filter-modifier`: a modifier where the engine rejects the definition
//!   (`COUNTROWS(KEEPFILTERS(…))`, `SUMX(REMOVEFILTERS(T), …)`, `USERELATIONSHIP` outside a
//!   `CALCULATE` filter argument…).
//! - `runtime-misplaced-filter-modifier` (warning): a modifier the engine accepts in a measure's
//!   definition but cannot evaluate there (`COUNTROWS(REMOVEFILTERS(T))`, `NONVISUAL` in a
//!   `CALCULATE` filter). Statements of a query report it as the error above.
//! - `nonvisual-not-group-by`: a `NONVISUAL` filter of `SUMMARIZECOLUMNS` on columns that are not
//!   its group-by columns.
//! - `runtime-multiple-values` (warning): a `SUMMARIZECOLUMNS` expression that is a column's
//!   `VALUES`/`DISTINCT`/`ALL`, used as a value where a group can see several values.

use super::*;

/// The filter modifiers, by the rules they follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Modifier {
    /// `USERELATIONSHIP`, `CROSSFILTER`: only as a direct filter argument of `CALCULATE`,
    /// `CALCULATETABLE` or a function with a `CALCULATE`-like filter (`TOTALYTD`…).
    Relationship,
    /// `REMOVEFILTERS`.
    Remove,
    /// `ALLCROSSFILTERED`.
    CrossFiltered,
    /// `ALL()` and `ALLSELECTED()` without arguments (with arguments they are table functions).
    AllEmpty,
    /// `KEEPFILTERS`: also a table argument of an iterator.
    Keep,
    /// `NONVISUAL`: a filter argument of `SUMMARIZECOLUMNS` only.
    NonVisual,
}

/// The modifier a call is, if any.
pub(super) fn modifier(up: &str, args: &[Option<Expr>]) -> Option<Modifier> {
    Some(match up {
        "USERELATIONSHIP" | "CROSSFILTER" => Modifier::Relationship,
        "REMOVEFILTERS" => Modifier::Remove,
        "ALLCROSSFILTERED" => Modifier::CrossFiltered,
        "ALL" | "ALLSELECTED" if args.is_empty() => Modifier::AllEmpty,
        "KEEPFILTERS" => Modifier::Keep,
        "NONVISUAL" => Modifier::NonVisual,
        _ => return None,
    })
}

/// Where an expression stands, as far as the modifier rules are concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Slot {
    /// The expression of the object itself (after its variables).
    Root,
    /// The value of a variable.
    Var,
    /// A filter argument of `CALCULATE`, `CALCULATETABLE`, `TOTALYTD`…
    Filter,
    /// The argument of a `KEEPFILTERS` that is such a filter argument.
    FilterKeep,
    /// A filter argument of `SUMMARIZECOLUMNS` (or the argument of `KEEPFILTERS`/`NONVISUAL`
    /// standing there).
    ScFilter,
    /// The table argument of an iterator (a function with a row expression, `GENERATE`'s first
    /// table); `filter` for `FILTER`'s.
    Scan { filter: bool },
    /// The argument of `COUNTROWS` or `ISEMPTY`.
    Count,
    /// Another table argument; `checked` where the engine was seen to check it at definition
    /// (`CALCULATETABLE`'s table, `DISTINCT`, `GENERATE`'s second table).
    TableArg { checked: bool },
    /// An argument whose expectations are unknown (a function without a signature, an `Any`
    /// parameter): never reported.
    Unknown,
    /// Any other position: a scalar argument, an operand, a row expression…
    Value,
}

/// What the engine does with a modifier in a slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Ok,
    /// The definition is accepted, evaluation fails.
    Runtime,
    /// The definition is rejected.
    Error,
}

/// The engine's verdicts (Power BI Desktop 2.158, probed): `strict` for calculated columns,
/// calculated tables and row filters, which reject at definition what measures defer.
fn placement(m: Modifier, slot: Slot, strict: bool) -> Outcome {
    use Outcome::{Error, Ok, Runtime};
    if slot == Slot::Unknown {
        return Ok;
    }
    let allowed = match m {
        Modifier::Relationship => slot == Slot::Filter,
        Modifier::Remove | Modifier::CrossFiltered | Modifier::AllEmpty => {
            matches!(slot, Slot::Filter | Slot::FilterKeep | Slot::ScFilter)
        }
        Modifier::Keep => matches!(
            slot,
            Slot::Filter
                | Slot::FilterKeep
                | Slot::ScFilter
                | Slot::Scan { .. }
                | Slot::TableArg { checked: false }
        ),
        Modifier::NonVisual => slot == Slot::ScFilter,
    };
    if allowed {
        return Ok;
    }
    match m {
        Modifier::Relationship | Modifier::Keep => Error,
        _ if strict => Error,
        Modifier::Remove => match slot {
            Slot::Count | Slot::TableArg { checked: false } => Runtime,
            _ => Error,
        },
        Modifier::CrossFiltered => Runtime,
        Modifier::AllEmpty => match slot {
            Slot::Root | Slot::Var | Slot::Scan { filter: true } => Error,
            _ => Runtime,
        },
        Modifier::NonVisual => match slot {
            Slot::Root | Slot::Count => Error,
            _ => Runtime,
        },
    }
}

fn describe(m: Modifier, up: &str) -> String {
    match m {
        Modifier::Relationship => {
            format!("{up} can only be a filter argument of CALCULATE or CALCULATETABLE")
        }
        Modifier::Remove | Modifier::CrossFiltered | Modifier::AllEmpty => {
            let what = if m == Modifier::AllEmpty {
                format!("{up}() without arguments")
            } else {
                up.to_string()
            };
            format!("{what} removes filters and is no table: use it as a filter argument of CALCULATE")
        }
        Modifier::Keep => "KEEPFILTERS can only be a filter argument of CALCULATE or CALCULATETABLE, or \
                           the table argument of an iterator (SUMX, FILTER…)"
            .to_string(),
        Modifier::NonVisual => "NONVISUAL can only be a filter argument of SUMMARIZECOLUMNS".to_string(),
    }
}

/// Functions whose table argument the engine was seen to check at definition.
fn checked_table(up: &str, i: usize) -> bool {
    match up {
        "CALCULATETABLE" => i == 0,
        "DISTINCT" => true,
        "GENERATE" | "GENERATEALL" => i == 1,
        _ => false,
    }
}

impl Checker<'_, '_> {
    /// The slot recorded for the expression at `span` (an unrecorded position is a value).
    fn slot_of(&self, span: Span) -> Slot {
        self.slots
            .iter()
            .rev()
            .find(|(s, _)| *s == span)
            .map_or(Slot::Value, |(_, k)| *k)
    }

    /// Records the slot of `e`. Only calls (and the `VAR` blocks that may return one) can be
    /// modifiers, and an unrecorded position is a value: nothing else is recorded, so a call with
    /// thousands of scalar arguments costs nothing here.
    pub(super) fn mark_slot(&mut self, e: &Expr, slot: Slot) {
        if slot != Slot::Value && matches!(e, Expr::Call { .. } | Expr::Var { .. }) {
            self.slots.push((e.span(), slot));
        }
    }

    /// A `VAR … RETURN body` stands where its body stands.
    pub(super) fn inherit_slot(&mut self, var: Span, body: &Expr) {
        if matches!(body, Expr::Call { .. } | Expr::Var { .. }) {
            let slot = self.slot_of(var);
            self.mark_slot(body, slot);
        }
    }

    /// Records the slots of a call's arguments, before they are visited.
    pub(super) fn mark_arg_slots(&mut self, up: &str, args: &[Option<Expr>], span: Span) {
        let Some(sig) = self.signature(up) else {
            for a in args.iter().flatten() {
                self.mark_slot(a, Slot::Unknown);
            }
            return;
        };
        if up == "KEEPFILTERS" || up == "NONVISUAL" {
            let inner = match self.slot_of(span) {
                Slot::Filter | Slot::FilterKeep if up == "KEEPFILTERS" => Slot::FilterKeep,
                Slot::ScFilter => Slot::ScFilter,
                _ => Slot::Unknown,
            };
            for a in args.iter().flatten() {
                self.mark_slot(a, inner);
            }
            return;
        }
        if up == "SUMMARIZECOLUMNS" {
            for a in args.iter().flatten() {
                match a {
                    Expr::Str { .. } => break,
                    Expr::Column { .. } => {}
                    Expr::Call { name, .. } if name.to_ascii_uppercase().starts_with("ROLLUP") => {}
                    _ => self.mark_slot(a, Slot::ScFilter),
                }
            }
            return;
        }
        let scanning = sig.params.iter().any(|p| p.kind == ParamKind::RowExpression);
        let bound = bind_args(sig, args);
        for (i, a) in args.iter().enumerate() {
            let Some(a) = a else { continue };
            let kind = bound
                .get(i)
                .copied()
                .flatten()
                .and_then(|p| sig.params.get(p))
                .map(|p| p.kind);
            let slot = match kind {
                _ if i == 0 && matches!(up, "COUNTROWS" | "ISEMPTY") => Slot::Count,
                _ if i == 0 && matches!(up, "GENERATE" | "GENERATEALL") => Slot::Scan { filter: false },
                Some(ParamKind::Filter) => Slot::Filter,
                Some(ParamKind::Table | ParamKind::TableRef) if scanning => Slot::Scan {
                    filter: up == "FILTER",
                },
                Some(
                    ParamKind::Table
                    | ParamKind::TableRef
                    | ParamKind::TableOrColumn
                    | ParamKind::TableOrColumnRef
                    | ParamKind::Relation,
                ) => Slot::TableArg {
                    checked: checked_table(up, i),
                },
                Some(ParamKind::Any) | None => Slot::Unknown,
                Some(_) => Slot::Value,
            };
            self.mark_slot(a, slot);
        }
    }

    /// Checks where a filter modifier stands.
    pub(super) fn modifier_rules(&mut self, up: &str, args: &[Option<Expr>], span: Span) {
        if !self.report {
            return;
        }
        let Some(m) = modifier(up, args) else { return };
        let strict = matches!(
            self.kind,
            ExprKind::CalculatedColumn | ExprKind::CalculatedTable | ExprKind::RowFilter
        );
        match placement(m, self.slot_of(span), strict) {
            Outcome::Ok => {}
            Outcome::Error => self.error("misplaced-filter-modifier", span, describe(m, up)),
            Outcome::Runtime => self.warning(
                "runtime-misplaced-filter-modifier",
                span,
                format!("{} (fails when evaluated)", describe(m, up)),
            ),
        }
    }

    /// `SUMMARIZECOLUMNS` rules on its filter and expression sections: `NONVISUAL` filters only
    /// on group-by columns; a column's values used as a value where a group can see several.
    pub(super) fn check_summarizecolumns(&mut self, args: &[Option<Expr>]) {
        let Some(model) = self.sh.names else { return };
        // Group-by columns, as written (a column inside ROLLUPADDISSUBTOTAL does not count).
        // `None` when one does not resolve to a model column (a variation, an unknown name).
        let mut group: Option<Vec<(String, String)>> = Some(Vec::new());
        let mut filters: Vec<&Expr> = Vec::new();
        let mut pairs: Vec<&Expr> = Vec::new();
        let mut i = 0usize;
        while let Some(a) = args.get(i) {
            match a {
                Some(Expr::Str { .. }) => {
                    if let Some(Some(e)) = args.get(i.saturating_add(1)) {
                        pairs.push(e);
                    }
                    i = i.saturating_add(2);
                    continue;
                }
                Some(_) if !pairs.is_empty() => {}
                Some(c @ Expr::Column { .. }) => match self.model_column(c) {
                    Some((t, col)) => {
                        if let Some(g) = group.as_mut() {
                            g.push((t.name.to_ascii_uppercase(), col.name.to_ascii_uppercase()));
                        }
                    }
                    None => group = None,
                },
                Some(Expr::Call { name, .. }) if name.to_ascii_uppercase().starts_with("ROLLUP") => {}
                Some(f) => filters.push(f),
                None => {}
            }
            i = i.saturating_add(1);
        }
        let Some(group) = group else { return };
        for f in &filters {
            let Some(inner) = nonvisual_arg(f) else { continue };
            let Some(cols) = self.nonvisual_lineage(inner, 0) else {
                continue;
            };
            if let Some((t, c)) = cols.iter().find(|x| !group.contains(x)) {
                let shown = model.table(t).map_or_else(|| t.clone(), |x| quote_table(&x.name));
                self.error(
                    "nonvisual-not-group-by",
                    f.span(),
                    format!(
                        "NONVISUAL can only filter group-by columns of this SUMMARIZECOLUMNS; it \
                         filters {shown}[{c}], which is not one of them"
                    ),
                );
            }
        }
        // Filters that may narrow a column to one value; `None` when one cannot be told.
        let mut narrowing: Option<Vec<(String, String)>> = Some(Vec::new());
        for f in &filters {
            match self.nonvisual_lineage(f, 0) {
                // Filtering group-by columns again narrows nothing more.
                Some(cols) if cols.iter().all(|c| group.contains(c)) => {}
                Some(cols) => {
                    if let Some(n) = narrowing.as_mut() {
                        n.extend(cols);
                    }
                }
                None => narrowing = None,
            }
        }
        let Some(narrowing) = narrowing else { return };
        for e in pairs {
            self.check_group_values(e, &group, &narrowing);
        }
    }

    /// A `SUMMARIZECOLUMNS` expression that is (or computes with operators on) `VALUES(T[C])`,
    /// `DISTINCT(T[C])` or `ALL(T[C])`: it fails for any group that sees several values of `C`.
    fn check_group_values(&mut self, e: &Expr, group: &[(String, String)], narrowing: &[(String, String)]) {
        let Some(model) = self.sh.names else { return };
        let mut found: Vec<&Expr> = Vec::new();
        collect_table_values(e, &mut Vec::new(), &mut found, 0);
        for call in found {
            let Expr::Call { name, args, span, .. } = call else {
                continue;
            };
            let Some(Some(col)) = args.first() else { continue };
            let Some((t, c)) = self.model_column(col) else {
                continue;
            };
            let key = (t.name.to_ascii_uppercase(), c.name.to_ascii_uppercase());
            let up = name.to_ascii_uppercase();
            // ALL ignores the group's filters: every value of the column is visible.
            if up == "ALL" || !determined(model.model(), t, &key, group, narrowing) {
                self.runtime_multiple(&up, t, c, *span);
            }
        }
    }

    fn runtime_multiple(&mut self, up: &str, t: &Table, c: &Column, span: Span) {
        self.warning(
            "runtime-multiple-values",
            span,
            format!(
                "{up}({}[{}]) is used as a single value, but the group-by columns do not determine \
                 {}[{}]: any group with several values fails when evaluated; use SELECTEDVALUE or \
                 an aggregation",
                quote_table(&t.name),
                c.name,
                quote_table(&t.name),
                c.name
            ),
        );
    }

    /// The model columns a filter table holds (upper-case table and column names), when known.
    /// Filter-removing calls (`ALL(…)`, `REMOVEFILTERS(…)`…) hold none that matter: `None`.
    fn nonvisual_lineage(&self, e: &Expr, depth: usize) -> Option<Vec<(String, String)>> {
        if depth > 16 {
            return None;
        }
        let Expr::Call { name, args, .. } = e else {
            return self.lineage(e);
        };
        let up = name.to_ascii_uppercase();
        match up.as_str() {
            "ALL" | "ALLSELECTED" | "ALLNOBLANKROW" | "ALLEXCEPT" | "ALLCROSSFILTERED" | "REMOVEFILTERS" => {
                None
            }
            "KEEPFILTERS" | "NONVISUAL" => {
                self.nonvisual_lineage(args.first()?.as_ref()?, depth.saturating_add(1))
            }
            "TREATAS" => {
                let mut out = Vec::new();
                for a in args.iter().skip(1) {
                    let (t, c) = self.model_column(a.as_ref()?)?;
                    out.push((t.name.to_ascii_uppercase(), c.name.to_ascii_uppercase()));
                }
                Some(out)
            }
            "CROSSJOIN" => {
                let mut out = Vec::new();
                for a in args {
                    out.extend(self.nonvisual_lineage(a.as_ref()?, depth.saturating_add(1))?);
                }
                Some(out)
            }
            _ => self.lineage(e),
        }
    }
}

/// The argument of `NONVISUAL(…)`, possibly inside `KEEPFILTERS(…)`.
fn nonvisual_arg(e: &Expr) -> Option<&Expr> {
    let mut cur = e;
    for _ in 0..8 {
        let Expr::Call { name, args, .. } = cur else {
            return None;
        };
        let Some(Some(inner)) = args.first() else {
            return None;
        };
        if name.eq_ignore_ascii_case("NONVISUAL") {
            return Some(inner);
        }
        if !name.eq_ignore_ascii_case("KEEPFILTERS") {
            return None;
        }
        cur = inner;
    }
    None
}

/// The `VALUES`/`DISTINCT`/`ALL` calls of one column that `e` uses as its value: `e` itself, the
/// operands of its arithmetic, concatenation and comparison operators, and the variables of its
/// own `VAR` blocks it returns. Calls inside function arguments are not followed: a function may
/// guard them (`IF(HASONEVALUE(…), VALUES(…))`) or aggregate them.
fn collect_table_values<'e>(
    e: &'e Expr,
    vars: &mut Vec<(String, &'e Expr)>,
    out: &mut Vec<&'e Expr>,
    depth: usize,
) {
    if depth > 32 {
        return;
    }
    let next = depth.saturating_add(1);
    match e {
        Expr::Call { name, args, .. } => {
            let up = name.to_ascii_uppercase();
            if matches!(up.as_str(), "VALUES" | "DISTINCT" | "ALL")
                && args.len() == 1
                && matches!(args.first(), Some(Some(Expr::Column { variation: None, .. })))
            {
                out.push(e);
            }
        }
        Expr::Binary { op, lhs, rhs, .. }
            if ["+", "-", "*", "/", "^", "&", "=", "<>", "<", ">", "<=", ">="].contains(&op.as_str()) =>
        {
            collect_table_values(lhs, vars, out, next);
            collect_table_values(rhs, vars, out, next);
        }
        Expr::Unary { op, operand, .. } if op == "-" || op == "+" => {
            collect_table_values(operand, vars, out, next);
        }
        Expr::Var { bindings, body, .. } => {
            let depth_before = vars.len();
            for b in bindings {
                vars.push((b.name.to_ascii_uppercase(), &b.value));
            }
            collect_table_values(body, vars, out, next);
            vars.truncate(depth_before);
        }
        Expr::Name { name, .. } => {
            let up = name.to_ascii_uppercase();
            if let Some(pos) = vars.iter().rposition(|(n, _)| *n == up)
                && let Some(&(_, value)) = vars.get(pos)
            {
                // The variable's value sees only the variables defined before it.
                let mut outer: Vec<(String, &Expr)> = vars.iter().take(pos).cloned().collect();
                collect_table_values(value, &mut outer, out, next);
            }
        }
        _ => {}
    }
}

/// Whether the group-by columns (and the filters) of a `SUMMARIZECOLUMNS` give `col` of `t` one
/// value per group: `col` is a group-by column, a group-by column is a key of `t`, or a filter
/// may narrow `t` (it filters `t` or a table `t` reaches through many-to-one relationships).
/// Bidirectional and one-to-one relationships of `t` let other tables narrow it: then nothing is
/// claimed.
fn determined(
    model: &SemanticModel,
    t: &Table,
    col: &(String, String),
    group: &[(String, String)],
    narrowing: &[(String, String)],
) -> bool {
    if group.contains(col) {
        return true;
    }
    let tu = &col.0;
    let two_way = model.relationships.iter().any(|r| {
        let touches = r.from_table.eq_ignore_ascii_case(tu) || r.to_table.eq_ignore_ascii_case(tu);
        let both = r
            .cross_filtering_behavior
            .as_deref()
            .is_some_and(|b| b.eq_ignore_ascii_case("bothDirections"));
        let one_to_one = r
            .from_cardinality
            .as_deref()
            .is_some_and(|c| c.eq_ignore_ascii_case("one"));
        touches && (both || one_to_one)
    });
    if two_way {
        return true;
    }
    if group.iter().any(|(gt, gc)| gt == tu && is_key(model, t, gc)) {
        return true;
    }
    narrowing
        .iter()
        .any(|(nt, _)| nt == tu || reaches_one_side(model, tu, nt))
}

/// A column whose values are unique in its table: marked as the table's key, the row number, or
/// the one side of a relationship.
fn is_key(model: &SemanticModel, t: &Table, column: &str) -> bool {
    let Some(c) = t.columns.iter().find(|c| c.name.eq_ignore_ascii_case(column)) else {
        return false;
    };
    if c.is_row_number() || c.extra.get("isKey").and_then(|v| v.as_bool()) == Some(true) {
        return true;
    }
    model.relationships.iter().any(|r| {
        let same =
            |table: &str, col: &str| table.eq_ignore_ascii_case(&t.name) && col.eq_ignore_ascii_case(&c.name);
        let many = |card: &Option<String>| card.as_deref().is_some_and(|x| x.eq_ignore_ascii_case("many"));
        (same(&r.to_table, &r.to_column) && !many(&r.to_cardinality))
            || (same(&r.from_table, &r.from_column)
                && r.from_cardinality
                    .as_deref()
                    .is_some_and(|x| x.eq_ignore_ascii_case("one")))
    })
}
