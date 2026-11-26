//! Definition-time rules of specific functions, found by probing the engine (see
//! `docs/dax-oracle.md`): window-function relations and clauses, `GROUPBY`/`CURRENTGROUP`, the
//! time-intelligence `dates` argument, natural joins, the column names of calculated tables and
//! `CALENDARAUTO`. Each rule only fires when the shapes involved are statically known.
//!
//! Codes:
//! - `clause-outside-window`: `ORDERBY`/`PARTITIONBY`/`MATCHBY` outside a window function.
//! - `window-relation-column`: a window function's relation is a column, not a table.
//! - `window-column-not-in-relation`: a `PARTITIONBY`/`MATCHBY` column the relation lacks.
//! - `groupby-expression`: a `GROUPBY` expression that is not an aggregation over
//!   `CURRENTGROUP()`, or that uses a measure or `CALCULATE`.
//! - `currentgroup-outside-groupby`: `CURRENTGROUP()` anywhere else.
//! - `dates-expected`: a time-intelligence `dates` argument that is neither a column nor a
//!   one-column table with the lineage of a model column.
//! - `interval-needs-calendar`: `WEEK` intervals with a date column instead of a calendar.
//! - `invalid-year-end-date`: a year-end date literal that is no date.
//! - `no-common-join-columns`: a natural join of tables that share no column lineage.
//! - `duplicate-column-name`: a calculated table whose columns would share a name.
//! - `calendarauto-no-date-column`: `CALENDARAUTO` in a model without a data column of dates.

use bicraft_model::Measure;

use super::*;

/// `ORDERBY`, `PARTITIONBY` or `MATCHBY` when `e` is a call to one of them.
pub(super) fn clause_name(e: &Expr) -> Option<&'static str> {
    let Expr::Call { name, .. } = e else {
        return None;
    };
    ["ORDERBY", "PARTITIONBY", "MATCHBY"]
        .into_iter()
        .find(|c| c.eq_ignore_ascii_case(name))
}

/// Whether a function takes window clauses (`ORDERBY`…) as arguments.
pub(super) fn takes_clauses(sig: &Signature) -> bool {
    sig.params.iter().any(|p| {
        matches!(
            p.kind,
            ParamKind::OrderBy | ParamKind::PartitionBy | ParamKind::MatchBy
        )
    })
}

/// Aggregations the engine refuses over `CURRENTGROUP()` in `GROUPBY` (probed); the others of the
/// `…X` family (`SUMX`, `AVERAGEX`, `MINX`, `MAXX`, `COUNTX`, `COUNTAX`, `PRODUCTX`, `STDEVX.P`)
/// are accepted.
const GROUPBY_REFUSED: &[&str] = &[
    "CONCATENATEX",
    "COUNTROWS",
    "MEDIANX",
    "PERCENTILEX.EXC",
    "PERCENTILEX.INC",
    "RANKX",
];

/// Table functions whose columns have no model lineage: as a `dates` argument the engine cannot
/// find their table.
const NO_LINEAGE: &[&str] = &["ROW", "GENERATESERIES", "CALENDAR", "CALENDARAUTO"];

/// A result column of a table expression: its upper-case name and, when it is a model column,
/// that column's upper-case table and name.
type ResultColumn = (String, Option<(String, String)>);

fn is_call(e: &Expr, f: &str) -> bool {
    matches!(e, Expr::Call { name, .. } if name.eq_ignore_ascii_case(f))
}

/// `F(CURRENTGROUP(), …)`: the `CURRENTGROUP()` call.
fn currentgroup_arg(e: &Expr) -> Option<&Expr> {
    let Expr::Call { args, .. } = e else {
        return None;
    };
    match args.first() {
        Some(Some(c @ Expr::Call { args: inner, .. })) if inner.is_empty() && is_call(c, "CURRENTGROUP") => {
            Some(c)
        }
        _ => None,
    }
}

/// The expression arguments of `GROUPBY` (those after a name string).
fn groupby_expressions(args: &[Option<Expr>]) -> Vec<&Expr> {
    let mut out = Vec::new();
    let mut i = 1;
    while let Some(a) = args.get(i) {
        if let Some(Expr::Str { .. }) = a
            && let Some(Some(e)) = args.get(i.saturating_add(1))
        {
            out.push(e);
            i = i.saturating_add(2);
        } else {
            i = i.saturating_add(1);
        }
    }
    out
}

/// Functions the engine accepts only in a visual calculation's expression (M11.40 signature
/// probes: refused in measures and queries). Visual calculations have their own checker
/// (`visual_calc`).
const VISUAL_ONLY: &[&str] = &[
    "COLLAPSE",
    "COLLAPSEALL",
    "EXPAND",
    "EXPANDALL",
    "FIRST",
    "ISATLEVEL",
    "LAST",
    "LOOKUP",
    "LOOKUPWITHTOTALS",
    "MOVINGAVERAGE",
    "NEXT",
    "PREVIOUS",
    "RANGE",
    "RUNNINGSUM",
];

/// `SUMMARIZE`, `GROUPBY` and `SUMMARIZECOLUMNS`: arguments in sections (group-by columns,
/// `SUMMARIZECOLUMNS` filter tables, then name/expression pairs), matched by shape.
pub(super) fn is_multi_section(fname: &str) -> bool {
    ["SUMMARIZE", "GROUPBY", "SUMMARIZECOLUMNS"]
        .iter()
        .any(|f| f.eq_ignore_ascii_case(fname))
}

/// Group-by modifiers that stand in a group-by column's place.
fn is_group_by_modifier(e: &Expr) -> bool {
    let Expr::Call { name, .. } = e else {
        return false;
    };
    let up = name.to_ascii_uppercase();
    up.starts_with("ROLLUP") || up == "NONVISUAL"
}

/// Whether an argument stands in a sort-order slot: skipped, `ASC`/`DESC`, or a constant.
fn looks_like_order(a: &Option<Expr>) -> bool {
    match a {
        None => true,
        Some(Expr::Name {
            name, quoted: false, ..
        }) => ["ASC", "DESC"].iter().any(|k| k.eq_ignore_ascii_case(name)),
        Some(Expr::Number { .. } | Expr::Str { .. } | Expr::Bool { .. }) => true,
        Some(Expr::Unary { operand, .. }) => matches!(**operand, Expr::Number { .. }),
        Some(_) => false,
    }
}

/// Repeating units that end with an optional sort order (`TOPN`'s order-by expression and
/// order, `ISAFTER`'s value pairs and order…), bound by shape as the engine binds them: after
/// the unit's expressions, an argument that looks like an order is the order, anything else
/// starts the next unit (M11.40 signature probes). `SELECTCOLUMNS`' optional column name is a
/// string literal; without one the argument is the expression. `None` for other signatures.
fn bind_order_units(sig: &Signature, args: &[Option<Expr>]) -> Option<Vec<Option<usize>>> {
    let r = usize::from(sig.repeat_from?);
    let rest = sig.params.get(r..)?;
    let unit = rest.iter().take_while(|p| p.repeating).count();
    if unit < 2 || rest.len() != unit {
        return None;
    }
    let last = rest.get(unit - 1)?;
    let ordered = matches!(last.kind, ParamKind::KeywordOrScalar(_)) && last.optional;
    let selectcolumns = sig.name == "SELECTCOLUMNS";
    if !ordered && !selectcolumns {
        return None;
    }
    let mut out: Vec<Option<usize>> = Vec::with_capacity(args.len());
    let mut pos = 0usize;
    for (i, a) in args.iter().enumerate() {
        if i < r {
            out.push(Some(i));
            continue;
        }
        if selectcolumns {
            // [name], expression
            if pos == 0 && matches!(a, Some(Expr::Str { .. })) {
                out.push(Some(r));
                pos = 1;
            } else {
                out.push(Some(r + 1));
                pos = 0;
            }
            continue;
        }
        if pos == unit - 1 {
            pos = 0;
            if looks_like_order(a) {
                out.push(Some(r + unit - 1));
                continue;
            }
        }
        out.push(Some(r + pos));
        pos += 1;
    }
    Some(out)
}

/// The param each argument binds to (`None`: unchecked). Multi-section functions are matched by
/// shape (see [`is_multi_section`]); every other signature by [`param_index`].
pub(super) fn bind_args(sig: &Signature, args: &[Option<Expr>]) -> Vec<Option<usize>> {
    let argc = args.len();
    if matches!(sig.name, "ROLLUPADDISSUBTOTAL" | "ROLLUPISSUBTOTAL")
        && matches!(args.first(), Some(Some(Expr::Column { .. })))
    {
        // No grand-total filter: the arguments start with the group-by columns (by shape).
        return vec![None; argc];
    }
    if crate::is_query_only_info(sig.name) && sig.params.len() == 2 {
        // Restriction name/value pairs.
        return (0..argc).map(|i| Some(i % 2)).collect();
    }
    if sig.name == "LINEST" && argc >= 3 && !matches!(args.last(), Some(Some(Expr::Column { .. }))) {
        // LINEST(y, x…, const): a last argument that is no column is `const` (by shape, as the
        // engine binds it; M11.40 signature probes).
        let mut out: Vec<Option<usize>> = (0..argc)
            .map(|i| param_index(sig, i, argc.saturating_sub(1)))
            .collect();
        if let Some(last) = out.last_mut() {
            *last = Some(2);
        }
        return out;
    }
    if let Some(out) = bind_order_units(sig, args) {
        return out;
    }
    if !is_multi_section(sig.name) {
        return (0..argc).map(|i| param_index(sig, i, argc)).collect();
    }
    let index = |name: &str| sig.params.iter().position(|p| p.name == name);
    let (group_by, filter, name, expression) = (
        index("group_by_column"),
        index("filter_table"),
        index("name"),
        index("expression"),
    );
    let first = usize::from(sig.repeat_from.unwrap_or(0));
    let mut out: Vec<Option<usize>> = (0..argc).map(|i| (i < first).then_some(i)).collect();
    // 0: group-by columns, 1: filter tables, 2: name/expression pairs.
    let mut phase = 0u8;
    let mut i = first;
    while let Some(a) = args.get(i) {
        let slot = out.get_mut(i);
        match a {
            Some(Expr::Str { .. }) => {
                phase = 2;
                if let Some(s) = slot {
                    *s = name;
                }
                if let Some(s) = out.get_mut(i.saturating_add(1)) {
                    *s = expression;
                }
                i = i.saturating_add(2);
                continue;
            }
            // Misplaced after the pairs: reported by the section rules.
            _ if phase == 2 => {}
            // SUMMARIZE takes ROLLUP/ROLLUPGROUP, not the SUMMARIZECOLUMNS subtotal modifiers.
            Some(e)
                if is_group_by_modifier(e)
                    && !(sig.name == "SUMMARIZE"
                        && matches!(e, Expr::Call { name, .. }
                            if name.eq_ignore_ascii_case("ROLLUPADDISSUBTOTAL")
                                || name.eq_ignore_ascii_case("ROLLUPISSUBTOTAL"))) => {}
            Some(Expr::Column { .. }) if phase == 0 => {
                if let Some(s) = slot {
                    *s = group_by;
                }
            }
            Some(_) => {
                // A filter table (SUMMARIZECOLUMNS), or a non-column where a group-by column is
                // expected (SUMMARIZE, GROUPBY).
                if filter.is_some() {
                    phase = 1;
                }
                if let Some(s) = slot {
                    *s = filter.or(group_by);
                }
            }
            None => {}
        }
        i = i.saturating_add(1);
    }
    out
}

impl<'m> Checker<'_, 'm> {
    /// The model column a column reference denotes (resolved as the type checker does).
    pub(super) fn model_column(&self, e: &Expr) -> Option<(&'m Table, &'m Column)> {
        let Expr::Column {
            table,
            name,
            variation: None,
            ..
        } = e
        else {
            return None;
        };
        let model = self.sh.names?;
        let table_is_var = table.as_deref().is_some_and(|t| self.qualifier_is_var(t));
        let scope = Scope {
            home: self.home,
            declared: Some(&self.declared),
            prefer_column: matches!(self.kind, ExprKind::CalculatedColumn | ExprKind::RowFilter),
        };
        let row_tables = self.row_tables();
        match resolve(model, scope, table.as_deref(), name, table_is_var, &row_tables) {
            Resolved::Column(t, c) => Some((t, c)),
            _ => None,
        }
    }

    /// Placement rules checked when a call is met: window clauses outside window functions,
    /// `CURRENTGROUP()` outside `GROUPBY`, `CALENDARAUTO` without dates to scan.
    pub(super) fn placement_rules(&mut self, up: &str, args: &[Option<Expr>], span: Span) {
        if !self.report {
            return;
        }
        if matches!(up, "ORDERBY" | "PARTITIONBY" | "MATCHBY") {
            self.error(
                "clause-outside-window",
                span,
                format!(
                    "{up} can only be used as an argument of a window function (OFFSET, INDEX, \
                     WINDOW, RANK, ROWNUMBER…)"
                ),
            );
        }
        if VISUAL_ONLY.contains(&up) {
            self.error(
                "visual-calc-only",
                span,
                format!("{up} can only be used in the expression of a visual calculation"),
            );
        }
        // Arguments of one function only (M11.40 signature probes).
        let home = match up {
            "IGNORE" | "NATURALJOINUSAGE" | "ROLLUPADDISSUBTOTAL" => Some("SUMMARIZECOLUMNS"),
            "ROLLUPISSUBTOTAL" => Some("ADDMISSINGITEMS"),
            _ => None,
        };
        if let Some(home) = home
            && !self.callers.iter().any(|c| c == home)
        {
            self.error(
                "misplaced-function",
                span,
                format!("{up} can only be used inside {home}"),
            );
        }
        if up == "CURRENTGROUP" && !self.currentgroup_ok.contains(&span) {
            self.error(
                "currentgroup-outside-groupby",
                span,
                "CURRENTGROUP() can only be the table argument of an aggregation (SUMX, COUNTX…) \
                 that is an expression argument of GROUPBY",
            );
        }
        if up == "CALENDARAUTO"
            && let Some(names) = self.sh.names
            && !has_base_date_column(names.model())
        {
            self.error(
                "calendarauto-no-date-column",
                span,
                "CALENDARAUTO scans the date columns of the model's data tables, and this model \
                 has none (columns of calculated tables and calculated columns do not count); use \
                 CALENDAR with explicit dates",
            );
        }
        if crate::is_query_only_info(up)
            && matches!(
                self.kind,
                ExprKind::Measure
                    | ExprKind::CalculatedColumn
                    | ExprKind::CalculatedTable
                    | ExprKind::RowFilter
            )
        {
            self.error("info-query-only", span, crate::info_query_only_message(up));
        }
        let _ = args;
    }

    /// Marks the nested calls a call legitimately contains, before its arguments are visited.
    pub(super) fn allow_nested(&mut self, up: &str, args: &[Option<Expr>]) {
        if up == "GROUPBY" {
            for e in groupby_expressions(args) {
                if let Some(cg) = currentgroup_arg(e) {
                    self.currentgroup_ok.push(cg.span());
                }
            }
        }
    }

    /// The relation argument of a window function, when given (not a clause, not skipped).
    fn relation_arg<'a>(&self, fname: &str, args: &'a [Option<Expr>]) -> Option<&'a Expr> {
        let keyword = |a: Option<&Option<Expr>>, set: &[&str]| match a {
            Some(Some(Expr::Name {
                name, quoted: false, ..
            })) => set.iter().any(|k| k.eq_ignore_ascii_case(name)),
            _ => false,
        };
        let index = match fname {
            "OFFSET" | "INDEX" => 1,
            "ROWNUMBER" => 0,
            "RANK" => {
                if matches!(args.first(), Some(None)) || keyword(args.first(), &["SKIP", "DENSE"]) {
                    1
                } else {
                    0
                }
            }
            "WINDOW" => {
                // from [, from_type], to [, to_type], relation
                let mut i: usize = 1;
                if keyword(args.get(i), &["ABS", "REL"]) {
                    i = i.saturating_add(1);
                }
                i = i.saturating_add(1); // to
                if keyword(args.get(i), &["ABS", "REL"]) {
                    i = i.saturating_add(1);
                }
                i
            }
            _ => return None,
        };
        match args.get(index) {
            Some(Some(e)) if clause_name(e).is_none() => Some(e),
            _ => None,
        }
    }

    /// Visits a window clause (`ORDERBY(…)` and friends) in the row context of the window
    /// function's relation: `ORDERBY` expressions are evaluated over the relation's rows, and
    /// `PARTITIONBY`/`MATCHBY` columns must be columns of the relation.
    pub(super) fn window_clause(
        &mut self,
        fname: &str,
        args: &[Option<Expr>],
        clause_expr: &Expr,
        clause: &str,
    ) -> Info {
        let Expr::Call { args: cargs, .. } = clause_expr else {
            return Info::UNKNOWN;
        };
        let row = self
            .relation_arg(fname, args)
            .map_or(Row::Unknown, |r| self.row_of(r));
        if clause == "ORDERBY" {
            self.rows.push(row);
            let mut previous_was_expression = false;
            for a in cargs.iter().flatten() {
                let order = match a {
                    Expr::Name {
                        name, quoted: false, ..
                    } => ["ASC", "DESC"].iter().any(|k| k.eq_ignore_ascii_case(name)),
                    Expr::Number { .. } | Expr::Bool { .. } => previous_was_expression,
                    // Text after an expression stands in the order's place, which the engine
                    // refuses (M11.40 signature probes).
                    Expr::Str { span, .. } if previous_was_expression => {
                        self.error(
                            "invalid-keyword",
                            *span,
                            "ORDERBY: an order is ASC, DESC, 0/FALSE or 1/TRUE, not text",
                        );
                        true
                    }
                    _ => false,
                };
                if order {
                    previous_was_expression = false;
                    continue;
                }
                let info = self.infer(a, true);
                self.expect_scalar(info, a.span(), "scalar-expected", "ORDERBY");
                previous_was_expression = true;
            }
            self.rows.pop();
            return Info::UNKNOWN;
        }
        for a in cargs.iter().flatten() {
            let info = self.unchecked(a);
            self.expect_column(clause, a, info, false);
            let Some((t, c)) = self.model_column(a) else {
                continue;
            };
            let (tu, cu) = (t.name.to_ascii_uppercase(), c.name.to_ascii_uppercase());
            let inside = match &row {
                Row::Table(rt) => *rt == tu,
                Row::Columns(rt, cols) => *rt == tu && cols.contains(&cu),
                Row::Unknown | Row::Filter { .. } => true,
            };
            if !inside {
                let kind = if clause == "PARTITIONBY" {
                    "PartitionBy"
                } else {
                    "MatchBy"
                };
                self.error(
                    "window-column-not-in-relation",
                    a.span(),
                    format!(
                        "{kind} column {}[{}] is not a column of {fname}'s relation",
                        quote_table(&t.name),
                        c.name
                    ),
                );
            }
        }
        Info::UNKNOWN
    }

    /// Rules tied to specific functions, checked once their arguments are typed.
    pub(super) fn function_rules(&mut self, up: &str, args: &[Option<Expr>], span: Span) {
        if !self.report {
            return;
        }
        match up {
            "OFFSET" | "INDEX" | "WINDOW" | "RANK" | "ROWNUMBER" => {
                if let Some(rel) = self.relation_arg(up, args)
                    && matches!(rel, Expr::Column { .. })
                    && (self.model_column(rel).is_some() || self.is_measure_ref(rel))
                {
                    self.error(
                        "window-relation-column",
                        rel.span(),
                        format!("the relation of {up} must be a table expression, not a column"),
                    );
                }
                self.check_window_clauses(up, args, span);
            }
            "DATEADD" | "PARALLELPERIOD" | "DATESINPERIOD" => {
                self.check_week_interval(up, args);
                self.check_calendar_arguments(up, args);
            }
            _ if crate::is_query_only_info(up) => self.check_info_restrictions(up, args),
            "SELECTCOLUMNS" => {
                // A column's name may be left out only for a plain column reference.
                let mut named = false;
                for a in args.iter().skip(1).flatten() {
                    if !named && matches!(a, Expr::Str { .. }) {
                        named = true;
                        continue;
                    }
                    if !named && !matches!(a, Expr::Column { .. }) {
                        self.error(
                            "new-column-name-expected",
                            a.span(),
                            "SELECTCOLUMNS expects the new column's name before an expression; \
                             only a column reference may go without one",
                        );
                    }
                    named = false;
                }
            }
            "DATATABLE" if args.len().is_multiple_of(2) => {
                // name, type pairs, then the data.
                self.error(
                    "datatable-pairs",
                    span,
                    "DATATABLE takes column name and type pairs followed by the data, an odd \
                     number of arguments",
                );
            }
            "PATHITEM" | "PATHITEMREVERSE" => {
                // `type` is the constant 0 (text) or 1 (integer) (M11.40 signature probes).
                if let Some(Some(a @ Expr::Str { .. })) = args.get(2) {
                    self.error(
                        "invalid-path-item-type",
                        a.span(),
                        format!("{up}: the type is 0 (text) or 1 (integer)"),
                    );
                }
            }
            "SUMMARIZE" | "GROUPBY" => self.check_sections(up, args),
            "SUMMARIZECOLUMNS" => {
                self.check_sections(up, args);
                self.check_summarizecolumns(args);
            }
            _ => {}
        }
        if let Some(sig) = self.signature(up)
            && let Some(pos) = sig.params.iter().position(|p| p.name == "year_end_date")
        {
            // The year-end date may also take the filter's place (matched by shape).
            for a in args.iter().skip(pos.saturating_sub(1).max(1)).flatten() {
                if let Expr::Str { value, span } = a
                    && !value.bytes().any(|b| b.is_ascii_digit())
                {
                    self.error(
                        "invalid-year-end-date",
                        *span,
                        format!("{up}: \"{value}\" is not a year-end date (e.g. \"6/30\")"),
                    );
                }
            }
        }
        self.runtime_function_rules(up, args, span);
    }

    fn is_measure_ref(&self, e: &Expr) -> bool {
        self.measure_of(e).is_some()
    }

    /// The measure a `[Measure]` / `Table[Measure]` reference denotes.
    pub(super) fn measure_of(&self, e: &Expr) -> Option<(&'m Table, &'m Measure)> {
        let Expr::Column {
            table,
            name,
            variation: None,
            ..
        } = e
        else {
            return None;
        };
        let model = self.sh.names?;
        let table_is_var = table.as_deref().is_some_and(|t| self.qualifier_is_var(t));
        let scope = Scope {
            home: self.home,
            declared: Some(&self.declared),
            prefer_column: matches!(self.kind, ExprKind::CalculatedColumn | ExprKind::RowFilter),
        };
        match resolve(model, scope, table.as_deref(), name, table_is_var, &[]) {
            Resolved::Measure(t, m) => Some((t, m)),
            _ => None,
        }
    }

    /// The row context `SUMMARIZE` evaluates its expressions in: its group-by columns, when they
    /// are columns of one table.
    pub(super) fn group_by_row(&self, args: &[Option<Expr>]) -> Row {
        let mut table: Option<String> = None;
        let mut cols = Vec::new();
        for a in args.iter().skip(1) {
            let col = match a {
                Some(Expr::Str { .. }) => break,
                Some(c @ Expr::Column { .. }) => c,
                _ => return Row::Unknown,
            };
            let Some((t, c)) = self.model_column(col) else {
                return Row::Unknown;
            };
            let t = t.name.to_ascii_uppercase();
            if table.as_ref().is_some_and(|x| *x != t) {
                return Row::Unknown;
            }
            table = Some(t);
            cols.push(c.name.to_ascii_uppercase());
        }
        table.map_or(Row::Unknown, |t| Row::Columns(t, cols))
    }

    /// Section order of the multi-section functions: group-by columns (then, for
    /// `SUMMARIZECOLUMNS`, filter tables), then name/expression pairs; each name has its
    /// expression, and no group-by column is given twice.
    fn check_sections(&mut self, up: &str, args: &[Option<Expr>]) {
        let first = usize::from(up != "SUMMARIZECOLUMNS");
        let mut seen: Vec<(String, String)> = Vec::new();
        let mut pairs = false;
        let mut i = first;
        while let Some(a) = args.get(i) {
            match a {
                Some(Expr::Str { value, span }) => {
                    pairs = true;
                    if matches!(args.get(i.saturating_add(1)), None | Some(None)) {
                        self.error(
                            "missing-expression",
                            *span,
                            format!("{up}: the new column \"{value}\" needs an expression after its name"),
                        );
                    }
                    i = i.saturating_add(2);
                    continue;
                }
                Some(e) if pairs => {
                    self.error(
                        "section-order",
                        e.span(),
                        format!("{up}: group-by columns and filters come before the name/expression pairs"),
                    );
                    return;
                }
                Some(c @ Expr::Column { .. }) => {
                    if let Some((t, col)) = self.model_column(c) {
                        let key = (t.name.to_ascii_uppercase(), col.name.to_ascii_uppercase());
                        if seen.contains(&key) {
                            self.error(
                                "duplicate-group-by-column",
                                c.span(),
                                format!(
                                    "{up}: {}[{}] is a group-by column already",
                                    quote_table(&t.name),
                                    col.name
                                ),
                            );
                        } else {
                            seen.push(key);
                        }
                    }
                }
                _ => {}
            }
            i = i.saturating_add(1);
        }
    }

    /// `WEEK` intervals need a calendar reference: with a date column or a table the engine
    /// accepts only `DAY`, `MONTH`, `QUARTER` and `YEAR`.
    fn check_week_interval(&mut self, up: &str, args: &[Option<Expr>]) {
        let index = if up == "DATESINPERIOD" { 3 } else { 2 };
        let Some(Some(Expr::Name {
            name,
            quoted: false,
            span,
        })) = args.get(index)
        else {
            return;
        };
        if !name.eq_ignore_ascii_case("WEEK") {
            return;
        }
        let calendar = match args.first() {
            Some(Some(Expr::Name { name, .. })) => {
                self.var(name).is_none() && self.sh.names.is_none_or(|m| m.table(name).is_none())
            }
            _ => false,
        };
        if !calendar {
            self.error(
                "interval-needs-calendar",
                *span,
                format!(
                    "{up}: the WEEK interval needs a calendar reference; with dates, use DAY, \
                     MONTH, QUARTER or YEAR"
                ),
            );
        }
    }

    /// Whether `e` may be a calendar reference: a bare name that is neither a variable nor a
    /// table (without a model, any bare name).
    pub(super) fn may_be_calendar(&self, e: Option<&Expr>) -> bool {
        match e {
            Some(Expr::Name { name, .. }) => {
                self.var(name).is_none() && self.sh.names.is_none_or(|m| m.table(name).is_none())
            }
            _ => false,
        }
    }

    /// `DATEADD`'s `extension` and `truncation` and `DATESINPERIOD`'s end behavior exist only
    /// with a calendar reference: with dates the engine takes 3 and 4 arguments (M11.40).
    fn check_calendar_arguments(&mut self, up: &str, args: &[Option<Expr>]) {
        let plain = match up {
            "DATEADD" => 3,
            "DATESINPERIOD" => 4,
            _ => return,
        };
        let Some(Some(extra)) = args.get(plain) else {
            return;
        };
        if !self.may_be_calendar(args.first().and_then(Option::as_ref)) {
            self.error(
                "argument-needs-calendar",
                extra.span(),
                format!("without a calendar reference, {up} takes only {plain} arguments"),
            );
        }
    }

    /// Window functions outside visual calculations: they need a relation, or an `ORDERBY` or
    /// `PARTITIONBY` clause to derive one from, and their `reset` argument cannot be combined
    /// with those clauses (M11.40 signature probes).
    fn check_window_clauses(&mut self, up: &str, args: &[Option<Expr>], span: Span) {
        let clauses = args
            .iter()
            .flatten()
            .filter_map(clause_name)
            .any(|c| matches!(c, "ORDERBY" | "PARTITIONBY"));
        if !clauses && self.relation_arg(up, args).is_none() {
            self.error(
                "window-needs-relation",
                span,
                format!("{up} needs a relation, or an ORDERBY or PARTITIONBY clause"),
            );
        }
        let reset = self.signature(up).and_then(|s| {
            let last = s.params.last()?;
            (last.name == "reset" && args.len() == s.params.len()).then(|| args.last())?
        });
        if clauses && let Some(Some(r)) = reset {
            self.error(
                "window-reset-with-clauses",
                r.span(),
                format!(
                    "{up}: the reset argument cannot be combined with ORDERBY or PARTITIONBY \
                     outside a visual calculation"
                ),
            );
        }
    }

    /// `INFO.*` restrictions come in pairs, each led by a constant string naming the restriction.
    fn check_info_restrictions(&mut self, up: &str, args: &[Option<Expr>]) {
        for a in args.iter().step_by(2).flatten() {
            if !matches!(a, Expr::Str { .. }) {
                self.error(
                    "info-restriction-name",
                    a.span(),
                    format!("{up} expects a constant string naming a restriction here"),
                );
            }
        }
    }

    /// A time-intelligence `dates` argument: a column reference (any expression that is one), a
    /// one-column table whose column has a model lineage, or a calendar reference.
    pub(super) fn dates_argument(&mut self, fname: &str, arg: &Expr) -> Info {
        if let Expr::Name { name, .. } = arg
            && self.var(name).is_none()
            && self.sh.names.is_some_and(|m| m.table(name).is_none())
        {
            // Not a table: a calendar reference (or an unknown name the binder reports).
            return Info::UNKNOWN;
        }
        let info = if matches!(arg, Expr::Column { .. }) {
            self.unchecked(arg)
        } else {
            self.infer(arg, false)
        };
        let span = arg.span();
        let lineage_free = match arg {
            Expr::Call { name, args, .. } => {
                NO_LINEAGE.iter().any(|f| f.eq_ignore_ascii_case(name))
                    || (name.eq_ignore_ascii_case("SELECTCOLUMNS")
                        && args
                            .iter()
                            .skip(1)
                            .flatten()
                            .any(|a| matches!(a, Expr::Str { .. })))
            }
            _ => false,
        };
        if info.shape == Shape::Measure
            || (matches!(info.ty, ValueType::Scalar(_))
                && !matches!(info.shape, Shape::Column | Shape::LocalColumn))
        {
            self.error(
                "dates-expected",
                span,
                format!("the dates argument of {fname} must be a date column or a one-column table of dates"),
            );
        } else if info.ty == ValueType::Table && info.width.is_some_and(|w| w > 1) {
            self.error(
                "dates-expected",
                span,
                format!(
                    "the dates argument of {fname} must be a column or a one-column table; this \
                     table has {} columns",
                    info.width.unwrap_or_default()
                ),
            );
        } else if lineage_free {
            self.error(
                "dates-expected",
                span,
                format!(
                    "the dates argument of {fname} must hold a column of the model; the columns \
                     this table function creates have no model lineage"
                ),
            );
        } else {
            self.check_dates_type(fname, arg);
        }
        info
    }

    /// `NATURALINNERJOIN`/`NATURALLEFTOUTERJOIN` join on the columns both sides share by lineage
    /// (a whole model table also carries the columns of the tables it expands to through
    /// many-to-one relationships).
    pub(super) fn check_natural_join(&mut self, up: &str, args: &[Option<Expr>], span: Span) {
        let (Some(Some(l)), Some(Some(r))) = (args.first(), args.get(1)) else {
            return;
        };
        let (Some(left), Some(right)) = (self.join_lineage(l), self.join_lineage(r)) else {
            return;
        };
        if !left.iter().any(|c| right.contains(c)) {
            self.error(
                "no-common-join-columns",
                span,
                format!(
                    "{up} needs at least one common join column: these tables share no column \
                     lineage and no relationship"
                ),
            );
        }
    }

    /// [`Checker::lineage`] plus, for a whole model table, the columns of every table it reaches
    /// through many-to-one relationships (its expanded table).
    fn join_lineage(&self, e: &Expr) -> Option<Vec<(String, String)>> {
        let mut cols = self.lineage(e)?;
        if let Some(t) = self.whole_table(e) {
            let model = self.sh.names?;
            for other in &model.model().tables {
                let up = other.name.to_ascii_uppercase();
                if up != t && reaches_one_side(model.model(), &t, &up) {
                    cols.extend(
                        other
                            .columns
                            .iter()
                            .filter(|c| !c.is_row_number())
                            .map(|c| (up.clone(), c.name.to_ascii_uppercase())),
                    );
                }
            }
        }
        Some(cols)
    }

    /// The model table (upper-case) a table expression returns whole, possibly filtered.
    fn whole_table(&self, e: &Expr) -> Option<String> {
        let model = self.sh.names?;
        let mut cur = e;
        loop {
            match cur {
                Expr::Name { name, .. } => {
                    if self.var(name).is_some() {
                        return None;
                    }
                    return model.table(name).map(|t| t.name.to_ascii_uppercase());
                }
                Expr::Call { name, args, .. } => {
                    let up = name.to_ascii_uppercase();
                    let next = match up.as_str() {
                        "FILTER" | "CALCULATETABLE" | "KEEPFILTERS" => args.first(),
                        "ALL" | "ALLNOBLANKROW" | "ALLSELECTED" | "VALUES" | "DISTINCT"
                            if args.len() == 1 =>
                        {
                            args.first()
                        }
                        "TOPN" | "SAMPLE" => args.get(1),
                        _ => None,
                    };
                    match next {
                        Some(Some(a)) => cur = a,
                        _ => return None,
                    }
                }
                _ => return None,
            }
        }
    }

    /// `GROUPBY` expressions must be aggregations (`SUMX`…) over `CURRENTGROUP()`, without
    /// measures or `CALCULATE`.
    pub(super) fn check_groupby(&mut self, args: &[Option<Expr>]) {
        for e in groupby_expressions(args) {
            let refused = match e {
                Expr::Call { name, .. } => GROUPBY_REFUSED.iter().any(|f| f.eq_ignore_ascii_case(name)),
                _ => true,
            };
            if currentgroup_arg(e).is_none() || refused {
                self.error(
                    "groupby-expression",
                    e.span(),
                    "each GROUPBY expression must be an aggregation over CURRENTGROUP(), e.g. \
                     SUMX(CURRENTGROUP(), Sales[Quantity])",
                );
                continue;
            }
            if let Some(bad) = self.first_context_transition(e) {
                self.error(
                    "groupby-expression",
                    bad,
                    "GROUPBY expressions cannot use CALCULATE or a measure reference",
                );
            }
        }
    }

    /// The first `CALCULATE`/`CALCULATETABLE` call or measure reference in `e`.
    fn first_context_transition(&self, e: &Expr) -> Option<Span> {
        let mut stack = vec![e];
        let mut budget = 4096usize;
        while let Some(x) = stack.pop() {
            budget = budget.checked_sub(1)?;
            match x {
                Expr::Call { name, args, span, .. } => {
                    if name.eq_ignore_ascii_case("CALCULATE") || name.eq_ignore_ascii_case("CALCULATETABLE") {
                        return Some(*span);
                    }
                    stack.extend(args.iter().flatten());
                }
                Expr::Column { span, .. } if self.is_measure_ref(x) => return Some(*span),
                Expr::Unary { operand, .. } => stack.push(operand),
                Expr::Binary { lhs, rhs, .. } => {
                    stack.push(lhs);
                    stack.push(rhs);
                }
                Expr::Var { bindings, body, .. } => {
                    stack.extend(bindings.iter().map(|b| &b.value));
                    stack.push(body);
                }
                Expr::Tuple { items, .. } => stack.extend(items.iter()),
                Expr::TableCtor { rows, .. } => stack.extend(rows.iter()),
                _ => {}
            }
        }
        None
    }

    /// The columns a table expression returns, with the model column each comes from when it is
    /// one (`None` for new columns), when statically known.
    fn result_columns(&self, e: &Expr, depth: usize) -> Option<Vec<ResultColumn>> {
        if depth > 32 {
            return None;
        }
        let model = self.sh.names?;
        let of = |cols: Vec<(String, String)>| {
            cols.into_iter()
                .map(|(t, c)| (c.clone(), Some((t, c))))
                .collect::<Vec<_>>()
        };
        let Expr::Call { name, args, .. } = e else {
            return self.lineage(e).map(of);
        };
        let up = name.to_ascii_uppercase();
        let tables = || -> Option<Vec<_>> {
            let mut out = Vec::new();
            for a in args {
                out.extend(self.result_columns(a.as_ref()?, depth.saturating_add(1))?);
            }
            Some(out)
        };
        match up.as_str() {
            "CROSSJOIN" | "GENERATE" | "GENERATEALL" => tables(),
            "NATURALINNERJOIN" | "NATURALLEFTOUTERJOIN" => {
                let mut out = tables()?;
                // Join columns (same lineage on both sides) appear once.
                let mut seen: Vec<(String, String)> = Vec::new();
                out.retain(|(_, l)| match l {
                    Some(l) if seen.contains(l) => false,
                    Some(l) => {
                        seen.push(l.clone());
                        true
                    }
                    None => true,
                });
                Some(out)
            }
            "ADDCOLUMNS" => {
                let mut out = self.result_columns(args.first()?.as_ref()?, depth.saturating_add(1))?;
                for a in args.iter().skip(1).step_by(2) {
                    match a {
                        Some(Expr::Str { value, .. }) => out.push((value.to_ascii_uppercase(), None)),
                        _ => return None,
                    }
                }
                Some(out)
            }
            "SUMMARIZE" | "SUMMARIZECOLUMNS" => {
                let start = usize::from(up == "SUMMARIZE");
                let mut out = Vec::new();
                let mut i = start;
                while let Some(a) = args.get(i) {
                    match a {
                        Some(c @ Expr::Column { .. }) => {
                            let (t, col) = self.model_column(c)?;
                            out.push((
                                col.name.to_ascii_uppercase(),
                                Some((t.name.to_ascii_uppercase(), col.name.to_ascii_uppercase())),
                            ));
                            i = i.saturating_add(1);
                        }
                        Some(Expr::Str { value, .. }) => {
                            out.push((value.to_ascii_uppercase(), None));
                            i = i.saturating_add(2);
                        }
                        // A filter table of SUMMARIZECOLUMNS adds no column.
                        Some(Expr::Name { .. }) if up == "SUMMARIZECOLUMNS" => i = i.saturating_add(1),
                        Some(Expr::Call { name, .. })
                            if up == "SUMMARIZECOLUMNS"
                                && !name.to_ascii_uppercase().starts_with("ROLLUP")
                                && !name.eq_ignore_ascii_case("NONVISUAL") =>
                        {
                            i = i.saturating_add(1);
                        }
                        _ => return None,
                    }
                }
                let _ = model;
                Some(out)
            }
            _ => self.lineage(e).map(of),
        }
    }

    /// A calculated table's columns must have distinct names.
    pub(super) fn check_table_column_names(&mut self, expr: &Expr) {
        if self.diags.iter().any(|d| d.code == "duplicate-column-name") {
            return;
        }
        let Some(cols) = self.result_columns(expr, 0) else {
            return;
        };
        for (i, (name, lineage)) in cols.iter().enumerate() {
            let clash = cols
                .iter()
                .take(i)
                .any(|(n, l)| n == name && (l.is_none() || lineage.is_none() || l != lineage));
            if clash {
                self.error(
                    "duplicate-column-name",
                    expr.span(),
                    format!(
                        "this calculated table would have two columns named [{name}] (from different \
                         tables); rename one, e.g. with SELECTCOLUMNS"
                    ),
                );
                return;
            }
        }
    }
}

/// Whether some data (non-calculated) table has a data column that may hold dates: what
/// `CALENDARAUTO` scans.
fn has_base_date_column(model: &SemanticModel) -> bool {
    model.tables.iter().any(|t| {
        let calculated = t
            .partitions
            .iter()
            .any(|p| p.source_type.eq_ignore_ascii_case("calculated"));
        !calculated
            && t.columns.iter().any(|c| {
                c.expression.is_none() && matches!(c.data_type, DataType::DateTime | DataType::Unknown)
            })
    })
}
