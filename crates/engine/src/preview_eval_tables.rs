//! Table functions of the preview evaluator ([`super`]): `ADDCOLUMNS`, `SELECTCOLUMNS`,
//! `SUMMARIZE` (with `ROLLUP` and `ISSUBTOTAL`), `SUMMARIZECOLUMNS`, `GENERATESERIES`,
//! `GENERATE`/`GENERATEALL`, `CROSSJOIN`, `TOPN`, `UNION`, `INTERSECT`, `EXCEPT`, `DISTINCT` and
//! `VALUES` of a table, `CALENDAR`, `CALENDARAUTO`, `ROW` and `DATATABLE`.
//!
//! DAX semantics kept:
//! - Columns keep the lineage of the model column they come from (`SELECTCOLUMNS` of a column,
//!   group-by columns, `UNION` when every table agrees), so a result used as a filter filters
//!   those columns; computed columns have no lineage and filter nothing.
//! - The expressions of `ADDCOLUMNS`, `SELECTCOLUMNS`, `GENERATE` and `TOPN` are evaluated in a row
//!   context over the table's rows; a measure or `CALCULATE` in them turns it into a filter
//!   context (context transition).
//! - `SUMMARIZE` evaluates its expressions with the rows of the group as a filter of the
//!   summarized table (its expanded table for a model table); `ROLLUP` adds the subtotal rows,
//!   with blank in the rolled-up columns.
//! - `SUMMARIZECOLUMNS` is evaluated like a visual's query: existing combinations of the
//!   group-by columns of each table under the filter tables, crossed between tables, without the
//!   rows whose expressions are all blank. One `ROLLUPADDISSUBTOTAL(column, "name")` adds the
//!   rows with that column rolled up (blank) and a boolean `name` column after it, TRUE on them.
//! - `VALUES(table)` and `ALL(table)` include the table's virtual blank row when some related row
//!   has no match; `DISTINCT(table)` does not.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bicraft_dax::ast::Expr;
use bicraft_model::DataType;

use super::super::Fail;
use super::super::store::Key;
use super::topn::{TopKey, top_rows};
use super::{
    ColId, Engine, Ext, RowB, Scope, TCol, Tab, V, arg, compare_vals, currency, dax_error, days_in_month,
    keyword, need, num, push_row, serial, subset, text_of, truth, unsupported, ymd,
};

/// Most rows a table function builds.
const MAX_TABLE_ROWS: usize = 2_000_000;
/// Deepest measure chain followed to guess a column's type.
const MAX_TYPE_DEPTH: usize = 8;

/// `(name, expression)` pairs from argument `from` on.
pub(super) fn pairs<'e>(
    args: &'e [Option<Expr>],
    from: usize,
    f: &str,
) -> Result<Vec<(String, &'e Expr)>, Fail> {
    let mut out = Vec::new();
    let mut i = from;
    while i < args.len() {
        let Some(Expr::Str { value, .. }) = arg(args, i) else {
            return unsupported(format!(
                "{f}() expects a column name (a string) as argument {}",
                i.saturating_add(1)
            ));
        };
        let e = need(args, i.saturating_add(1), f)?;
        out.push((value.clone(), e));
        i = i.saturating_add(2);
    }
    Ok(out)
}

/// A row as a key: blank is `None`, text compares without case.
pub(super) fn row_key(r: &[V]) -> Vec<Option<Key>> {
    r.iter().map(V::key).collect()
}

/// DAX sort order: blank first, then numbers (and dates), text (without case), booleans.
pub(super) fn sort_cmp(a: &V, b: &V) -> Ordering {
    let rank = |v: &V| match v {
        V::Blank => 0,
        V::Num(_) | V::Cur(_) | V::Date(_) => 1,
        V::Text(_) => 2,
        V::Bool(_) => 3,
    };
    match rank(a).cmp(&rank(b)) {
        Ordering::Equal => compare_vals(a, b).unwrap_or(Ordering::Equal),
        o => o,
    }
}

/// A date given as text (`YYYY-MM-DD`, optionally with ` hh:mm:ss` or `Thh:mm:ss`).
pub(super) fn parse_date_text(t: &str) -> Option<f64> {
    let t = t.trim();
    let (d, time) = match t.find(['T', ' ']) {
        Some(i) => (t.get(..i)?, t.get(i.saturating_add(1)..)),
        None => (t, None),
    };
    let mut p = d.split(['-', '/']);
    let y: i64 = p.next()?.parse().ok()?;
    let m: i64 = p.next()?.parse().ok()?;
    let dd: i64 = p.next()?.parse().ok()?;
    if p.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&dd) || !(1..=9999).contains(&y) {
        return None;
    }
    let mut x = serial(y, m, dd.min(days_in_month(y, m))) as f64;
    if let Some(time) = time.filter(|s| !s.is_empty()) {
        let mut q = time.split(':');
        let h: f64 = q.next()?.parse().ok()?;
        let mi: f64 = q.next().map_or(Some(0.0), |s| s.parse().ok())?;
        let se: f64 = q.next().map_or(Some(0.0), |s| s.parse().ok())?;
        x += (h * 3600.0 + mi * 60.0 + se) / 86_400.0;
    }
    Some(x)
}

/// The rows of `t` plus `new` columns with their values (one row of values per row).
fn extend(t: Tab, new: Vec<TCol>, vals: Vec<Vec<V>>) -> Tab {
    match t {
        Tab::Rows { table, rows, ext } => {
            let (mut cols, mut old) = match ext {
                Some(e) => (e.cols.as_ref().clone(), e.vals.clone()),
                None => (Vec::new(), vec![Vec::new(); rows.len()]),
            };
            cols.extend(new);
            for (o, v) in old.iter_mut().zip(vals) {
                o.extend(v);
            }
            Tab::Rows {
                table,
                rows,
                ext: Some(Arc::new(Ext {
                    cols: Arc::new(cols),
                    vals: old,
                })),
            }
        }
        Tab::Tuples { cols, rows } => {
            let mut c = cols.as_ref().clone();
            c.extend(new);
            let rows = rows
                .iter()
                .zip(vals)
                .map(|(r, v)| {
                    let mut r = r.clone();
                    r.extend(v);
                    r
                })
                .collect();
            Tab::tuples(c, rows)
        }
    }
}

/// A group-by column of `SUMMARIZE`: its lineage and where its value is read.
struct GroupCol {
    col: TCol,
    /// Position in the summarized table's columns; `None`: a column related to its rows.
    pos: Option<usize>,
}

impl Engine<'_, '_> {
    /// The table functions of this module; `None` for the others.
    pub(super) fn table_function(
        &mut self,
        up: &str,
        args: &[Option<Expr>],
        s: &Scope,
    ) -> Result<Option<Tab>, Fail> {
        let f = up;
        Ok(Some(match up {
            "ADDCOLUMNS" => {
                let t = self.table(need(args, 0, f)?, s)?;
                let items = pairs(args, 1, f)?;
                if items.is_empty() {
                    return unsupported("ADDCOLUMNS() without a column to add");
                }
                let scope = self.tab_cols(&t);
                let mut new = Vec::with_capacity(items.len());
                for (n, e) in &items {
                    new.push(TCol::named(n.clone(), self.type_in(e, &scope)));
                }
                let mut all = scope;
                all.extend(new.iter().cloned());
                self.check_names(&all, f)?;
                let vals = match self.each_direct_items(&t, &items, s)? {
                    Some(v) => v,
                    None => self.each_with(&t, s, |eng, inner| {
                        items
                            .iter()
                            .map(|(_, e)| eng.scalar(e, inner))
                            .collect::<Result<Vec<V>, Fail>>()
                    })?,
                };
                extend(t, new, vals)
            }
            "SELECTCOLUMNS" => {
                let t = self.table(need(args, 0, f)?, s)?;
                let tcols = self.tab_cols(&t);
                let mut cols = Vec::new();
                let mut exprs: Vec<&Expr> = Vec::new();
                let mut i = 1usize;
                while i < args.len() {
                    let (name, e, qualified) = match arg(args, i) {
                        Some(Expr::Str { value, .. }) => {
                            let e = need(args, i.saturating_add(1), f)?;
                            i = i.saturating_add(2);
                            (value.clone(), e, false)
                        }
                        Some(e @ Expr::Column { name, .. }) => {
                            i = i.saturating_add(1);
                            (name.clone(), e, true)
                        }
                        _ => {
                            return unsupported(
                                "SELECTCOLUMNS() expects a name and an expression, or a column",
                            );
                        }
                    };
                    let lin = self.lineage_in(e, &tcols);
                    let ty = match lin {
                        Some(c) => self.col_type(c),
                        None => self.type_in(e, &tcols),
                    };
                    cols.push(TCol {
                        lin,
                        name: Some(name),
                        ty,
                        qualified: qualified && lin.is_some(),
                        origin: None,
                    });
                    exprs.push(e);
                }
                self.check_names(&cols, f)?;
                let rows = self.each_with(&t, s, |eng, inner| {
                    exprs
                        .iter()
                        .map(|e| eng.scalar(e, inner))
                        .collect::<Result<Vec<V>, Fail>>()
                })?;
                Tab::tuples(cols, rows)
            }
            "SUMMARIZE" => self.summarize(args, s)?,
            "SUMMARIZECOLUMNS" => self.summarize_columns(args, s)?,
            "GENERATESERIES" => self.generate_series(args, s)?,
            "GENERATE" | "GENERATEALL" => {
                let t1 = self.table(need(args, 0, f)?, s)?;
                let e2 = need(args, 1, f)?;
                let cols1 = self.tab_cols(&t1);
                let vals1 = self.tab_values(&t1)?;
                let mut cols2: Option<Vec<TCol>> = None;
                let mut rows = Vec::new();
                let mut rc: Vec<RowB> = s.rc.to_vec();
                for i in 0..t1.len() {
                    let pushed = push_row(&t1, i, &mut rc);
                    let inner = Scope { rc: &rc, ..*s };
                    let t2 = self.table(e2, &inner);
                    rc.truncate(rc.len().saturating_sub(pushed));
                    let t2 = t2?;
                    let c2 = self.tab_cols(&t2);
                    let width = match &cols2 {
                        Some(c) if c.len() != c2.len() => {
                            return dax_error("GENERATE(): the second table changes width");
                        }
                        Some(c) => c.len(),
                        None => {
                            let w = c2.len();
                            cols2 = Some(c2);
                            w
                        }
                    };
                    let v2 = self.tab_values(&t2)?;
                    let left = vals1.get(i).cloned().unwrap_or_default();
                    if v2.is_empty() && up == "GENERATEALL" {
                        let mut r = left.clone();
                        r.extend(std::iter::repeat_n(V::Blank, width));
                        rows.push(r);
                    }
                    for r2 in v2.iter() {
                        let mut r = left.clone();
                        r.extend(r2.iter().cloned());
                        rows.push(r);
                    }
                    self.cap_rows(rows.len(), f)?;
                }
                let cols2 = match cols2 {
                    Some(c) => c,
                    // no row on the left: the right table's columns, if it evaluates alone
                    None => match self.table(e2, s) {
                        Ok(t2) => self.tab_cols(&t2),
                        Err(_) => Vec::new(),
                    },
                };
                let mut cols = cols1;
                cols.extend(cols2);
                self.check_names(&cols, f)?;
                Tab::tuples(cols, rows)
            }
            "CROSSJOIN" => {
                let mut cols: Vec<TCol> = Vec::new();
                let mut rows: Vec<Vec<V>> = vec![Vec::new()];
                for a in args.iter().flatten() {
                    let t = self.table(a, s)?;
                    cols.extend(self.tab_cols(&t));
                    let vals = self.tab_values(&t)?;
                    let total = rows.len().saturating_mul(vals.len());
                    self.cap_rows(total, f)?;
                    self.tick(total)?;
                    let mut next = Vec::with_capacity(total);
                    for r in &rows {
                        for v in vals.iter() {
                            let mut x = r.clone();
                            x.extend(v.iter().cloned());
                            next.push(x);
                        }
                    }
                    rows = next;
                }
                if cols.is_empty() {
                    return unsupported("CROSSJOIN() without a table");
                }
                self.check_names(&cols, f)?;
                Tab::tuples(cols, rows)
            }
            "TOPN" => self.top_n_table(args, s)?,
            "UNION" => {
                let mut tabs = Vec::new();
                for a in args.iter().flatten() {
                    tabs.push(self.table(a, s)?);
                }
                let Some(first) = tabs.first() else {
                    return unsupported("UNION() without a table");
                };
                let mut cols = self.tab_cols(first);
                let mut rows: Vec<Vec<V>> = Vec::new();
                for t in &tabs {
                    let c = self.tab_cols(t);
                    if c.len() != cols.len() {
                        return dax_error("UNION(): the tables have different numbers of columns");
                    }
                    for (mine, theirs) in cols.iter_mut().zip(&c) {
                        if mine.lin.is_some() && mine.lin != theirs.lin {
                            // lineage is kept only when every table agrees; a calculated table
                            // still names the first table's column as the source
                            // (an explicitly named column has none)
                            mine.origin = if mine.name.is_none() { mine.lin } else { None };
                            mine.name = Some(self.tcol_name(mine));
                            mine.lin = None;
                        }
                    }
                    rows.extend(self.tab_values(t)?.iter().cloned());
                    self.cap_rows(rows.len(), f)?;
                }
                Tab::tuples(cols, rows)
            }
            "INTERSECT" | "EXCEPT" => {
                let t1 = self.table(need(args, 0, f)?, s)?;
                let t2 = self.table(need(args, 1, f)?, s)?;
                if self.tab_cols(&t1).len() != self.tab_cols(&t2).len() {
                    return dax_error(format!("{f}(): the tables have different numbers of columns"));
                }
                let right: HashSet<Vec<Option<Key>>> =
                    self.tab_values(&t2)?.iter().map(|r| row_key(r)).collect();
                let left = self.tab_values(&t1)?;
                let keep = up == "INTERSECT";
                let picked: Vec<usize> = left
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| right.contains(&row_key(r)) == keep)
                    .map(|(i, _)| i)
                    .collect();
                subset(&t1, &picked)
            }
            "CALENDAR" => {
                let lo = self.day_arg(need(args, 0, f)?, s)?;
                let hi = self.day_arg(need(args, 1, f)?, s)?;
                let (Some(lo), Some(hi)) = (lo, hi) else {
                    return dax_error("CALENDAR(): a blank start or end date");
                };
                self.calendar(lo, hi)?
            }
            "CALENDARAUTO" => {
                let end_month = match arg(args, 0) {
                    None => 12,
                    Some(e) => match num(&self.scalar(e, s)?)? {
                        Some(x) if (1.0..=12.0).contains(&x.trunc()) => x.trunc() as i64,
                        _ => return dax_error("CALENDARAUTO(): the fiscal year end month must be 1 to 12"),
                    },
                };
                // No date in the data tables: an empty calendar (Desktop 2.158).
                let Some((lo, hi)) = self.model_date_range()? else {
                    return Ok(Some(Tab::tuples(
                        vec![TCol::named("Date", Some(DataType::DateTime))],
                        Vec::new(),
                    )));
                };
                let fiscal = |x: i64, last: bool| -> i64 {
                    let (y, m, _) = ymd(x);
                    if end_month == 12 {
                        return if last { serial(y, 12, 31) } else { serial(y, 1, 1) };
                    }
                    // the fiscal year ending in `end_month` of year `fy`
                    let fy = if m > end_month { y + 1 } else { y };
                    if last {
                        serial(fy, end_month, days_in_month(fy, end_month))
                    } else {
                        serial(fy - 1, end_month + 1, 1)
                    }
                };
                self.calendar(fiscal(lo, false), fiscal(hi, true))?
            }
            "ROW" => {
                let items = pairs(args, 0, f)?;
                if items.is_empty() {
                    return unsupported("ROW() without a column");
                }
                let mut cols = Vec::with_capacity(items.len());
                let mut row = Vec::with_capacity(items.len());
                for (n, e) in &items {
                    cols.push(TCol::named(n.clone(), self.static_type(e, 0)));
                    row.push(self.scalar(e, s)?);
                }
                self.check_names(&cols, f)?;
                Tab::tuples(cols, vec![row])
            }
            "DATATABLE" => self.datatable(args, s)?,
            _ => return Ok(None),
        }))
    }

    /// `VALUES(table)`, `DISTINCT(table)` and `DISTINCT(table expression)`.
    pub(super) fn values_table(&mut self, up: &str, e: &Expr, s: &Scope) -> Result<Tab, Fail> {
        if let Expr::Name { name, .. } = e
            && up == "VALUES"
            && !s.env.iter().any(|(n, _)| *n == name.to_lowercase())
            && let Some(t) = self.table_id(name)
        {
            let mut rows = self.visible_rows(s.fc, t)?;
            let n = self.rows(t)?;
            let mask = self.mask(s.fc, t)?;
            if super::visible(mask.as_ref(), n)
                && self.has_blank_row(t)?
                && let Ok(b) = u32::try_from(n)
            {
                rows.push(b);
            }
            return Ok(Tab::rows_of(t, rows));
        }
        if let Expr::Column { .. } = e {
            return unsupported(format!("{up}() of a column added by an iterator"));
        }
        let t = self.table(e, s)?;
        if let Tab::Rows { table, ext: None, .. } = &t
            && self.unique_col(*table)?.is_some()
        {
            return Ok(t);
        }
        let vals = self.tab_values(&t)?;
        let mut seen = HashSet::new();
        let picked: Vec<usize> = vals
            .iter()
            .enumerate()
            .filter(|(_, r)| seen.insert(row_key(r)))
            .map(|(i, _)| i)
            .collect();
        Ok(subset(&t, &picked))
    }

    fn cap_rows(&self, n: usize, f: &str) -> Result<(), Fail> {
        if n > MAX_TABLE_ROWS {
            return unsupported(format!("{f}() builds more than {MAX_TABLE_ROWS} rows"));
        }
        Ok(())
    }

    /// Two columns with one name are an error (two lineage columns of different tables may share
    /// a name).
    pub(super) fn check_names(&self, cols: &[TCol], f: &str) -> Result<(), Fail> {
        let mut seen: HashMap<String, Option<usize>> = HashMap::new();
        for c in cols {
            let name = self.tcol_name(c).to_lowercase();
            let table = match (&c.name, c.lin) {
                (None, Some(l)) => self.cols.get(l).map(|i| i.table),
                _ => None,
            };
            if let Some(prev) = seen.insert(name, table)
                && (prev.is_none() || table.is_none() || prev == table)
            {
                return dax_error(format!(
                    "{f}(): the result has two columns named `{}`",
                    self.tcol_name(c)
                ));
            }
        }
        Ok(())
    }

    pub(super) fn col_type(&self, c: ColId) -> Option<DataType> {
        if let Some(t) = self.col_types.get(&c) {
            return Some(*t);
        }
        let info = self.cols.get(c)?;
        let t = self.tabs.get(info.table)?;
        let mt = self.model.tables.iter().find(|x| x.name == t.name)?;
        mt.columns
            .iter()
            .find(|x| x.name == info.name)
            .map(|x| x.data_type)
    }

    /// The lineage of `e` when it is a column of the table whose columns are `tcols`.
    fn lineage_in(&mut self, e: &Expr, tcols: &[TCol]) -> Option<ColId> {
        let Expr::Column {
            table,
            name,
            variation: None,
            ..
        } = e
        else {
            return None;
        };
        match table {
            Some(t) => {
                let c = self.col_id(t, name)?;
                tcols.iter().any(|x| x.lin == Some(c)).then_some(c)
            }
            None => tcols
                .iter()
                .find(|x| self.tcol_name(x).eq_ignore_ascii_case(name))
                .and_then(|x| x.lin),
        }
    }

    /// A group-by column of `SUMMARIZE` over a table with columns `tcols` (`base`: the model
    /// table of its rows, whose related columns may be grouped by too).
    fn group_col(&mut self, e: &Expr, tcols: &[TCol], base: Option<usize>) -> Result<GroupCol, Fail> {
        let Expr::Column {
            table,
            name,
            variation: None,
            ..
        } = e
        else {
            return unsupported("SUMMARIZE() groups by something other than a column");
        };
        let pos = match table {
            Some(t) => {
                let Some(c) = self.col_id(t, name) else {
                    return unsupported(format!("unknown column '{t}'[{name}]"));
                };
                match tcols.iter().position(|x| x.lin == Some(c)) {
                    Some(p) => p,
                    None => {
                        let reach = base.is_some_and(|b| {
                            let ct = self.cols.get(c).map(|i| i.table);
                            self.expanded(b).iter().any(|(x, _)| Some(*x) == ct)
                        });
                        if !reach {
                            return unsupported(format!(
                                "SUMMARIZE() by '{t}'[{name}], which is not a column of the table"
                            ));
                        }
                        return Ok(GroupCol {
                            col: TCol::of(c),
                            pos: None,
                        });
                    }
                }
            }
            None => match tcols
                .iter()
                .position(|x| self.tcol_name(x).eq_ignore_ascii_case(name))
            {
                Some(p) => p,
                None => {
                    return unsupported(format!(
                        "SUMMARIZE() by [{name}], which is not a column of the table"
                    ));
                }
            },
        };
        let col = tcols
            .get(pos)
            .cloned()
            .unwrap_or_else(|| TCol::named(name.clone(), None));
        Ok(GroupCol { col, pos: Some(pos) })
    }

    fn summarize(&mut self, args: &[Option<Expr>], s: &Scope) -> Result<Tab, Fail> {
        let f = "SUMMARIZE";
        let t = self.table(need(args, 0, f)?, s)?;
        let tcols = self.tab_cols(&t);
        let base = match &t {
            Tab::Rows { table, .. } => Some(*table),
            Tab::Tuples { .. } => None,
        };
        let mut groups: Vec<GroupCol> = Vec::new();
        let mut rolled_from: Option<usize> = None;
        let mut i = 1usize;
        while let Some(a) = arg(args, i) {
            match a {
                Expr::Str { .. } => break,
                Expr::Call {
                    name, args: inner, ..
                } if name.eq_ignore_ascii_case("ROLLUP") => {
                    if rolled_from.is_some() {
                        return unsupported("SUMMARIZE() with two ROLLUPs");
                    }
                    rolled_from = Some(groups.len());
                    for r in inner.iter().flatten() {
                        if matches!(r, Expr::Call { .. }) {
                            return unsupported("ROLLUPGROUP() inside ROLLUP()");
                        }
                        groups.push(self.group_col(r, &tcols, base)?);
                    }
                }
                Expr::Call { name, .. } => {
                    return unsupported(format!("{}() in SUMMARIZE()", name.to_ascii_uppercase()));
                }
                other => {
                    if rolled_from.is_some() {
                        return unsupported("SUMMARIZE() with a group-by column after ROLLUP()");
                    }
                    groups.push(self.group_col(other, &tcols, base)?);
                }
            }
            i = i.saturating_add(1);
        }
        let items = pairs(args, i, f)?;
        let mut cols: Vec<TCol> = groups.iter().map(|g| g.col.clone()).collect();
        for (n, e) in &items {
            cols.push(TCol::named(n.clone(), self.type_in(e, &tcols)));
        }
        self.check_names(&cols, f)?;

        // the group-by values of every row
        let vals = self.tab_values(&t)?;
        let mut keys: Vec<Vec<V>> = Vec::with_capacity(t.len());
        self.tick(t.len().saturating_mul(groups.len().max(1)))?;
        for (r, row) in vals.iter().enumerate() {
            let mut k = Vec::with_capacity(groups.len());
            for g in &groups {
                k.push(match (g.pos, &t) {
                    (Some(p), _) => row.get(p).cloned().unwrap_or(V::Blank),
                    (None, Tab::Rows { table, rows, .. }) => {
                        let Some(c) = g.col.lin else {
                            return super::internal();
                        };
                        let rc = [RowB::Row {
                            table: *table,
                            row: rows.get(r).copied().unwrap_or(0),
                        }];
                        self.related_value(c, &rc)?
                    }
                    (None, Tab::Tuples { .. }) => return super::internal(),
                });
            }
            keys.push(k);
        }

        // groups at each ROLLUP level: every column, then without the last rolled-up ones
        let levels = rolled_from.map_or(0, |from| groups.len().saturating_sub(from));
        let mut out_rows: Vec<Vec<V>> = Vec::new();
        let group_cols: Arc<Vec<TCol>> = Arc::new(groups.iter().map(|g| g.col.clone()).collect());
        for level in 0..=levels {
            let active = groups.len().saturating_sub(level);
            let mut index: HashMap<Vec<Option<Key>>, usize> = HashMap::new();
            let mut found: Vec<(Vec<V>, Vec<usize>)> = Vec::new();
            for (r, k) in keys.iter().enumerate() {
                let mut kv: Vec<V> = k.iter().take(active).cloned().collect();
                kv.resize(groups.len(), V::Blank);
                let id = row_key(&kv);
                match index.get(&id) {
                    Some(at) => {
                        if let Some((_, m)) = found.get_mut(*at) {
                            m.push(r);
                        }
                    }
                    None => {
                        index.insert(id, found.len());
                        found.push((kv, vec![r]));
                    }
                }
            }
            self.cap_rows(out_rows.len().saturating_add(found.len()), f)?;
            let rolled: Vec<ColId> = groups.iter().skip(active).filter_map(|g| g.col.lin).collect();
            for (kv, members) in found {
                let mut row = kv.clone();
                if !items.is_empty() {
                    let sub = subset(&t, &members);
                    let filters = self.tab_filters(&sub)?;
                    let mut fc = if s.rc.is_empty() {
                        s.fc.child()
                    } else {
                        self.transitioned(s.fc, s.rc)?
                    };
                    let on: HashSet<ColId> = filters.iter().flat_map(|x| x.cols.iter().copied()).collect();
                    self.remove_cols(&mut fc.filters, &on)?;
                    fc.filters.extend(filters);
                    let mut rc: Vec<RowB> = s.rc.to_vec();
                    rc.push(RowB::Tuple {
                        cols: Arc::clone(&group_cols),
                        vals: kv,
                    });
                    let inner = Scope {
                        fc: &fc,
                        rc: &rc,
                        ..*s
                    };
                    let saved = std::mem::replace(&mut self.rolled, rolled.clone());
                    let mut ext = Vec::with_capacity(items.len());
                    let mut failed = None;
                    for (_, e) in &items {
                        match self.scalar(e, &inner) {
                            Ok(v) => ext.push(v),
                            Err(x) => {
                                failed = Some(x);
                                break;
                            }
                        }
                    }
                    self.rolled = saved;
                    if let Some(x) = failed {
                        return Err(x);
                    }
                    row.extend(ext);
                }
                out_rows.push(row);
            }
        }
        Ok(Tab::tuples(cols, out_rows))
    }

    pub(super) fn summarize_columns(&mut self, args: &[Option<Expr>], s: &Scope) -> Result<Tab, Fail> {
        // a grouping whose items are all aggregations (computed by grouped scans) depends only
        // on its text and its filter context: kept between queries
        let key = self.table_cache_key("SUMMARIZECOLUMNS", args, s)?;
        if let Some(k) = &key
            && let Some(t) = self.cached_table(k)?
        {
            return Ok(t);
        }
        let (t, cacheable) = self.summarize_columns_eval(args, s)?;
        if let (Some(k), true) = (key, cacheable) {
            self.keep_table(k, &t)?;
        }
        Ok(t)
    }

    fn summarize_columns_eval(&mut self, args: &[Option<Expr>], s: &Scope) -> Result<(Tab, bool), Fail> {
        let f = "SUMMARIZECOLUMNS";
        let mut groups: Vec<ColId> = Vec::new();
        // `ROLLUPADDISSUBTOTAL(column, "name")`: (position in `groups`, name of the flag column)
        let mut rollup: Option<(usize, String)> = None;
        let mut filters: Vec<Option<Expr>> = vec![None];
        let mut i = 0usize;
        while let Some(a) = arg(args, i) {
            match a {
                Expr::Str { .. } => break,
                Expr::Column { table, name, .. } if self.measure_def(table.as_deref(), name).is_none() => {
                    let c = self.col_arg(a, &[], f)?;
                    if groups.contains(&c) {
                        return dax_error("SUMMARIZECOLUMNS(): a group-by column is given twice");
                    }
                    groups.push(c);
                }
                Expr::Call { name, args: ra, .. }
                    if name.eq_ignore_ascii_case("ROLLUPADDISSUBTOTAL") && rollup.is_none() =>
                {
                    let (Some(col @ Expr::Column { .. }), Some(Expr::Str { value, .. }), 2) =
                        (arg(ra, 0), arg(ra, 1), ra.len())
                    else {
                        return unsupported(
                            "ROLLUPADDISSUBTOTAL() in SUMMARIZECOLUMNS() other than (column, \"name\")",
                        );
                    };
                    let c = self.col_arg(col, &[], f)?;
                    if groups.contains(&c) {
                        return dax_error("SUMMARIZECOLUMNS(): a group-by column is given twice");
                    }
                    rollup = Some((groups.len(), value.clone()));
                    groups.push(c);
                }
                Expr::Call { name, .. }
                    if matches!(
                        name.to_ascii_uppercase().as_str(),
                        "ROLLUPADDISSUBTOTAL" | "ROLLUPGROUP" | "NONVISUAL" | "ROLLUP"
                    ) =>
                {
                    return unsupported(format!("{}() in SUMMARIZECOLUMNS()", name.to_ascii_uppercase()));
                }
                other if self.is_table_expr(other, s.env) => filters.push(Some(other.clone())),
                _ => {
                    return unsupported(
                        "SUMMARIZECOLUMNS() argument that is not a column, a table or a name",
                    );
                }
            }
            i = i.saturating_add(1);
        }
        let items = pairs(args, i, f)?;
        let scope = Scope { rc: &[], ..*s };
        let fc = if filters.len() > 1 {
            self.calc_ctx(&filters, &scope)?
        } else {
            s.fc.child()
        };
        let mut cols: Vec<TCol> = Vec::new();
        for c in &groups {
            cols.push(TCol::of(*c));
        }
        // the subtotal flag comes after every group-by column, as the engine returns it
        if let Some((_, name)) = &rollup {
            cols.push(TCol::named(name.clone(), Some(DataType::Boolean)));
        }
        for (n, e) in &items {
            cols.push(TCol::named(n.clone(), self.static_type(e, 0)));
        }
        self.check_names(&cols, f)?;
        let saved = std::mem::replace(&mut self.group_cols, groups.clone());
        // In a query, `ALLSELECTED` restores the context with the filter arguments.
        let saved_outer = self
            .shadow
            .then(|| std::mem::replace(&mut self.outer, fc.child()));
        // the detail level, then (with `ROLLUPADDISSUBTOTAL`) the level without the rolled-up
        // column: its rows have that column blank and the flag column TRUE
        let mut levels: Vec<Option<usize>> = vec![None];
        if let Some((at, _)) = &rollup {
            levels.push(Some(*at));
        }
        // items that are one aggregation each: evaluated for every combination by one grouped scan
        let mut direct = Vec::with_capacity(items.len());
        let mut unwrapped: Vec<&Expr> = Vec::with_capacity(items.len());
        for (_, e) in &items {
            let e = match e {
                Expr::Call { name, args: ia, .. } if name.eq_ignore_ascii_case("IGNORE") => {
                    match arg(ia, 0) {
                        Some(x) => x,
                        None => *e,
                    }
                }
                other => *other,
            };
            unwrapped.push(e);
            direct.push(self.direct_agg(e, s.env, 0));
        }
        // reservations of the value vectors and of the rows, by blocks before they are built
        let mut charges: Vec<super::mem::Charge> = Vec::new();
        let width = cols.len();
        let result = (|| -> Result<Vec<Vec<V>>, Fail> {
            let mut rows = Vec::new();
            for rolled in levels {
                let active: Vec<ColId> = groups
                    .iter()
                    .enumerate()
                    .filter(|(gi, _)| Some(*gi) != rolled)
                    .map(|(_, c)| *c)
                    .collect();
                self.group_cols = active.clone();
                let combos = self.combos(&fc, &active)?;
                // the sums of several columns in one pass
                let sums: Vec<_> = direct.iter().flatten().copied().collect();
                self.prefetch_sums(&sums, &fc, &active)?;
                let nv = direct.iter().flatten().count();
                let nc = combos.len();
                charges.push(self.charge_bytes(
                    &move || format!("SUMMARIZECOLUMNS()'s {nv} value columns over {nc} combinations"),
                    nc.saturating_mul(nv).saturating_mul(std::mem::size_of::<V>()),
                )?);
                let mut vectors: Vec<Option<Vec<V>>> = Vec::with_capacity(items.len());
                for (d, e) in direct.iter().zip(&unwrapped) {
                    vectors.push(match d {
                        Some((agg, c)) => self.vector_agg(*agg, *c, &fc, &active, &combos)?,
                        // a row iterator over a fact: every cell from one grouped scan
                        None => self.vector_iter(e, s, &fc, &active, &combos, 0)?,
                    });
                }
                // every item computed by grouped scans: no filter context per combination
                let all_vectors = vectors.iter().all(Option::is_some);
                for (ci, combo) in combos.iter().enumerate() {
                    if ci % 4096 == 0 && self.mem.on() {
                        let block = nc.saturating_sub(ci).min(4096);
                        charges.push(self.charge_bytes(
                            &move || format!("SUMMARIZECOLUMNS()'s rows (over {ci} of {nc})"),
                            block.saturating_mul(super::mem::row_bytes(width)),
                        )?);
                    }
                    let mut sub = if all_vectors {
                        super::FCtx::default()
                    } else {
                        fc.child()
                    };
                    let mut row = Vec::with_capacity(cols.len());
                    let mut codes = combo.iter();
                    for (gi, c) in groups.iter().enumerate() {
                        if Some(gi) == rolled {
                            row.push(V::Blank);
                        } else {
                            let code = codes.next().copied().unwrap_or(0);
                            if !all_vectors {
                                sub.filters.push(self.codes_filter(*c, &[code])?);
                            }
                            row.push(self.value_of_code(*c, code)?);
                        }
                    }
                    if rollup.is_some() {
                        row.push(V::Bool(rolled.is_some()));
                    }
                    let inner = Scope {
                        fc: &sub,
                        rc: &[],
                        ..*s
                    };
                    // A row is kept when an expression that is not wrapped in IGNORE is not
                    // blank; with only IGNORE expressions (or none), every combination is kept.
                    let mut any = false;
                    let mut counted = 0usize;
                    for (ii, (_, e)) in items.iter().enumerate() {
                        let (e, ignored) = match e {
                            Expr::Call { name, args: ia, .. } if name.eq_ignore_ascii_case("IGNORE") => {
                                (need(ia, 0, "IGNORE")?, true)
                            }
                            other => (*other, false),
                        };
                        let v = match vectors.get(ii).and_then(Option::as_ref).and_then(|v| v.get(ci)) {
                            Some(v) => v.clone(),
                            None => self.scalar(e, &inner)?,
                        };
                        if !ignored {
                            counted = counted.saturating_add(1);
                            any |= !v.is_blank();
                        }
                        row.push(v);
                    }
                    if counted == 0 || any {
                        rows.push(row);
                    }
                }
            }
            Ok(rows)
        })();
        self.group_cols = saved;
        if let Some(o) = saved_outer {
            self.outer = o;
        }
        let cacheable = direct.iter().all(Option::is_some) || self.pure_args(args);
        Ok((Tab::tuples(cols, result?), cacheable))
    }

    fn generate_series(&mut self, args: &[Option<Expr>], s: &Scope) -> Result<Tab, Fail> {
        let f = "GENERATESERIES";
        let a = self.scalar(need(args, 0, f)?, s)?;
        let b = self.scalar(need(args, 1, f)?, s)?;
        let st = match arg(args, 2) {
            Some(e) => self.scalar(e, s)?,
            None => V::Num(1.0),
        };
        if a.is_blank() || b.is_blank() || st.is_blank() {
            return dax_error("GENERATESERIES(): a blank argument");
        }
        let (Some(x0), Some(x1), Some(dx)) = (num(&a)?, num(&b)?, num(&st)?) else {
            return dax_error("GENERATESERIES(): a blank argument");
        };
        if dx.partial_cmp(&0.0) != Some(Ordering::Greater)
            || !x0.is_finite()
            || !x1.is_finite()
            || !dx.is_finite()
        {
            return dax_error("GENERATESERIES(): the increment must be a positive number");
        }
        let date = matches!(a, V::Date(_)) || matches!(b, V::Date(_));
        let cur = !date && [&a, &b, &st].iter().any(|v| matches!(v, V::Cur(_)));
        let mut vals = Vec::new();
        if x1 >= x0 {
            let span = ((x1 - x0) / dx).floor();
            if span >= MAX_TABLE_ROWS as f64 {
                return unsupported(format!("{f}() builds more than {MAX_TABLE_ROWS} rows"));
            }
            let n = span as usize;
            self.tick(n)?;
            // The engine adds the increment step by step (0.1 three times is 0.30000000000000004).
            let mut x = x0;
            for _ in 0..=n.saturating_add(1) {
                if x > x1 {
                    break;
                }
                vals.push(vec![if date {
                    V::Date(x)
                } else if cur {
                    match currency::from_f64(x) {
                        Some(c) => V::Cur(c),
                        None => return super::overflow(),
                    }
                } else {
                    V::Num(x)
                }]);
                x += dx;
            }
        }
        let ty = if date {
            Some(DataType::DateTime)
        } else if cur {
            Some(DataType::Decimal)
        } else {
            let mut int = true;
            for i in 0..3 {
                if let Some(e) = arg(args, i) {
                    int &= self.static_type(e, 0) == Some(DataType::Int64);
                }
            }
            Some(if int { DataType::Int64 } else { DataType::Double })
        };
        Ok(Tab::tuples(vec![TCol::named("Value", ty)], vals))
    }

    fn top_n_table(&mut self, args: &[Option<Expr>], s: &Scope) -> Result<Tab, Fail> {
        let f = "TOPN";
        let n = num(&self.scalar(need(args, 0, f)?, s)?)?.unwrap_or(0.0);
        let source = need(args, 1, f)?;
        // (order-by expression, descending)
        let mut keys_of: Vec<(&Expr, bool)> = Vec::new();
        let mut i = 2usize;
        while let Some(e) = arg(args, i) {
            i = i.saturating_add(1);
            let mut desc = true;
            if let Some(o) = arg(args, i) {
                let order = match o {
                    Expr::Name { .. } => match keyword(o).as_deref() {
                        Some("ASC") => Some(false),
                        Some("DESC") => Some(true),
                        _ => None,
                    },
                    Expr::Number { .. } | Expr::Bool { .. } => Some(!truth(&self.scalar(o, s)?)?),
                    _ => None,
                };
                if let Some(d) = order {
                    desc = d;
                    i = i.saturating_add(1);
                }
            }
            keys_of.push((e, desc));
        }
        // over a grouping of aggregations: only the kept groups' rows are built
        if n >= 1.0
            && let Some((t, _)) = self.top_n_grouping(n, source, &keys_of, s, false)?
        {
            return Ok(t);
        }
        let t = self.table(source, s)?;
        let len = t.len();
        if n.partial_cmp(&1.0).is_none_or(|o| o == Ordering::Less) {
            return Ok(subset(&t, &[]));
        }
        let take = if n >= len as f64 { len } else { n as usize };
        if keys_of.is_empty() {
            let picked: Vec<usize> = (0..take).collect();
            return Ok(subset(&t, &picked));
        }
        // the keys: columns of the table when they are (read in place), else evaluated per row
        let direct = self.tuple_keys(&t, &keys_of);
        let keys = match &direct {
            Some(_) => Vec::new(),
            None => self.each_with(&t, s, |eng, inner| {
                keys_of
                    .iter()
                    .map(|(e, _)| eng.scalar(e, inner))
                    .collect::<Result<Vec<V>, Fail>>()
            })?,
        };
        let tuple_rows = match &t {
            Tab::Tuples { rows, .. } => Some(Arc::clone(rows)),
            Tab::Rows { .. } => None,
        };
        let key_of = |row: usize, k: usize| -> &V {
            match (&direct, &tuple_rows) {
                (Some(at), Some(rows)) => at
                    .get(k)
                    .and_then(|i| rows.get(row).and_then(|r| r.get(*i)))
                    .unwrap_or(&V::Blank),
                _ => keys.get(row).and_then(|r| r.get(k)).unwrap_or(&V::Blank),
            }
        };
        let cmp = |a: usize, b: usize| -> Ordering {
            for (k, (_, desc)) in keys_of.iter().enumerate() {
                let (x, y) = (key_of(a, k), key_of(b, k));
                let o = sort_cmp(x, y);
                let o = if *desc { o.reverse() } else { o };
                if o != Ordering::Equal {
                    return o;
                }
            }
            Ordering::Equal
        };
        // the rows up to the `take`-th in order (a partial selection), with the rows that tie
        // with it, in order (ties by position: the stable order of a full sort); a first key
        // read once into a typed array narrows the rows compared on every key
        let picked = match take
            .checked_sub(1)
            .and_then(|_| TopKey::read(len, |i| key_of(i, 0)))
        {
            Some(k0) => {
                let first_desc = keys_of.first().is_some_and(|(_, d)| *d);
                let cands = k0.candidates(take, first_desc);
                let full = |a: usize, b: usize| -> Ordering {
                    let o = k0.cmp_rows(a, b);
                    let o = if first_desc { o.reverse() } else { o };
                    o.then_with(|| {
                        for (k, (_, desc)) in keys_of.iter().enumerate().skip(1) {
                            let o = sort_cmp(key_of(a, k), key_of(b, k));
                            let o = if *desc { o.reverse() } else { o };
                            if o != Ordering::Equal {
                                return o;
                            }
                        }
                        Ordering::Equal
                    })
                };
                top_rows(cands, take, &full)
            }
            None => top_rows((0..len).collect(), take, &cmp),
        };
        Ok(subset(&t, &picked))
    }

    /// The positions of the `TOPN` order keys in a table of values when each is one of its
    /// columns (`[Name]` of a computed column, `T[c]` of a column with that lineage), read in
    /// place instead of evaluated row by row; `None` otherwise.
    fn tuple_keys(&mut self, t: &Tab, keys_of: &[(&Expr, bool)]) -> Option<Vec<usize>> {
        let Tab::Tuples { cols, .. } = t else {
            return None;
        };
        let mut at = Vec::with_capacity(keys_of.len());
        for (e, _) in keys_of {
            let Expr::Column {
                table,
                name,
                variation: None,
                ..
            } = e
            else {
                return None;
            };
            let i = match table {
                None => {
                    // as `ext_value`: the first column of that name, when it has no lineage
                    let i = cols
                        .iter()
                        .position(|c| self.tcol_name(c).eq_ignore_ascii_case(name))?;
                    if cols.get(i).is_some_and(|c| c.lin.is_some()) {
                        return None;
                    }
                    i
                }
                Some(tn) => {
                    if self.measure_def(Some(tn), name).is_some() {
                        return None;
                    }
                    let c = self.col_id(tn, name)?;
                    cols.iter().position(|x| x.lin == Some(c))?
                }
            };
            at.push(i);
        }
        Some(at)
    }

    /// One `Date` column of the days `lo..=hi`.
    fn calendar(&mut self, lo: i64, hi: i64) -> Result<Tab, Fail> {
        if lo > hi {
            return dax_error("CALENDAR(): the start date is after the end date");
        }
        let n = usize::try_from(hi.saturating_sub(lo)).unwrap_or(usize::MAX);
        if n >= MAX_TABLE_ROWS {
            return unsupported(format!("a calendar of more than {MAX_TABLE_ROWS} days"));
        }
        self.tick(n)?;
        let rows = (lo..=hi).map(|d| vec![V::Date(d as f64)]).collect();
        Ok(Tab::tuples(
            vec![TCol::named("Date", Some(DataType::DateTime))],
            rows,
        ))
    }

    /// First and last day over the date columns of the model's data tables (not calculated
    /// tables or calculated columns), as `CALENDARAUTO` reads them.
    fn model_date_range(&mut self) -> Result<Option<(i64, i64)>, Fail> {
        let model = self.model;
        let mut found: Vec<ColId> = Vec::new();
        for t in &model.tables {
            if t.partitions
                .iter()
                .any(|p| p.source_type.eq_ignore_ascii_case("calculated"))
            {
                continue;
            }
            for c in &t.columns {
                if c.expression.is_none()
                    && c.data_type == DataType::DateTime
                    && let Some(id) = self.col_id(&t.name, &c.name)
                {
                    found.push(id);
                }
            }
        }
        let mut range: Option<(i64, i64)> = None;
        for c in found {
            let d = self.all_dates(c)?;
            if let (Some(first), Some(last)) = (d.first(), d.last()) {
                range = Some(match range {
                    None => (first.0, last.0),
                    Some((a, b)) => (a.min(first.0), b.max(last.0)),
                });
            }
        }
        Ok(range)
    }

    fn datatable(&mut self, args: &[Option<Expr>], s: &Scope) -> Result<Tab, Fail> {
        let f = "DATATABLE";
        let Some(Expr::TableCtor { rows, .. }) = args.last().and_then(Option::as_ref) else {
            return unsupported("DATATABLE() without its rows");
        };
        let mut cols = Vec::new();
        let mut i = 0usize;
        while i.saturating_add(1) < args.len() {
            let Some(Expr::Str { value, .. }) = arg(args, i) else {
                return unsupported("DATATABLE() column name");
            };
            let ty = match arg(args, i.saturating_add(1)).and_then(keyword).as_deref() {
                Some("INTEGER") => DataType::Int64,
                Some("DOUBLE") => DataType::Double,
                Some("STRING") => DataType::String,
                Some("BOOLEAN") => DataType::Boolean,
                Some("CURRENCY") => DataType::Decimal,
                Some("DATETIME") => DataType::DateTime,
                _ => return unsupported("DATATABLE() column type"),
            };
            cols.push(TCol::named(value.clone(), Some(ty)));
            i = i.saturating_add(2);
        }
        self.check_names(&cols, f)?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let items: Vec<&Expr> = match r {
                Expr::Tuple { items, .. } => items.iter().collect(),
                Expr::TableCtor { rows: inner, .. } => inner.iter().collect(),
                other => vec![other],
            };
            if items.len() != cols.len() {
                return dax_error("DATATABLE(): a row does not have a value for each column");
            }
            let mut row = Vec::with_capacity(cols.len());
            for (c, e) in cols.iter().zip(items) {
                let v = self.scalar(e, s)?;
                row.push(if v.is_blank() {
                    V::Blank
                } else {
                    match c.ty {
                        Some(DataType::Int64) => V::Num(num(&v)?.unwrap_or(0.0).trunc()),
                        Some(DataType::Double) => V::Num(num(&v)?.unwrap_or(0.0)),
                        Some(DataType::String) => V::Text(text_of(&v)),
                        Some(DataType::Boolean) => V::Bool(truth(&v)?),
                        Some(DataType::Decimal) => match num(&v)?.and_then(currency::from_f64) {
                            Some(n) => V::Cur(n),
                            None => return super::overflow(),
                        },
                        _ => match &v {
                            V::Text(t) => match parse_date_text(t) {
                                Some(x) => V::Date(x),
                                None => return dax_error(format!("DATATABLE(): `{t}` is not a date")),
                            },
                            other => V::Date(num(other)?.unwrap_or(0.0)),
                        },
                    }
                });
            }
            out.push(row);
        }
        Ok(Tab::tuples(cols, out))
    }

    /// [`Engine::static_type`] with `[Name]` read as a column of a table with columns `scope`.
    fn type_in(&mut self, e: &Expr, scope: &[TCol]) -> Option<DataType> {
        let saved = std::mem::replace(&mut self.type_scope, scope.to_vec());
        let t = self.static_type(e, 0);
        self.type_scope = saved;
        t
    }

    /// The data type of a scalar expression when it can be told without evaluating it.
    pub(super) fn static_type(&mut self, e: &Expr, depth: usize) -> Option<DataType> {
        use DataType as D;
        if depth > MAX_TYPE_DEPTH {
            return None;
        }
        let d = depth.saturating_add(1);
        match e {
            Expr::Number { text, .. } => Some(if text.contains(['.', 'e', 'E']) {
                D::Double
            } else {
                D::Int64
            }),
            Expr::Str { .. } => Some(D::String),
            Expr::Bool { .. } => Some(D::Boolean),
            Expr::Column {
                table,
                name,
                variation: None,
                ..
            } => {
                if table.is_none()
                    && let Some(c) = self
                        .type_scope
                        .iter()
                        .find(|c| self.tcol_name(c).eq_ignore_ascii_case(name))
                        .cloned()
                {
                    return c.ty.or_else(|| c.lin.and_then(|l| self.col_type(l)));
                }
                if let Some(m) = self.measure_def(table.as_deref(), name) {
                    let body = m.expr.clone()?;
                    let saved = std::mem::take(&mut self.type_scope);
                    let t = self.static_type(&body, d.saturating_add(1));
                    self.type_scope = saved;
                    return t;
                }
                if bicraft_dax::format_string_ref(name)
                    .is_some_and(|m| self.measure_def(table.as_deref(), m).is_some())
                {
                    return Some(D::String);
                }
                let t = table.as_deref()?;
                let c = self.col_id(t, name)?;
                self.col_type(c)
            }
            Expr::Unary { op, operand, .. } => match op.to_ascii_uppercase().as_str() {
                "NOT" => Some(D::Boolean),
                // the opposite of a date is a number (Desktop 2.159)
                "-" => match self.static_type(operand, d) {
                    Some(D::DateTime) => Some(D::Double),
                    t => t,
                },
                _ => self.static_type(operand, d),
            },
            Expr::Binary { op, lhs, rhs, .. } => match op.to_ascii_uppercase().as_str() {
                "&" => Some(D::String),
                "=" | "==" | "<>" | "<" | "<=" | ">" | ">=" | "&&" | "||" | "IN" => Some(D::Boolean),
                "^" => Some(D::Double),
                _ => {
                    let (a, b) = (self.static_type(lhs, d)?, self.static_type(rhs, d)?);
                    // Numbers: the engine's rules, as the evaluator applies them
                    // (`currency::binary_kind`: Currency × Currency is a double, Currency ×
                    // double is Currency, …).
                    let kind = |t: DataType| match t {
                        D::Int64 | D::Boolean => Some(currency::Kind::Int),
                        D::Decimal => Some(currency::Kind::Currency),
                        D::Double => Some(currency::Kind::Real),
                        _ => None,
                    };
                    let c = op.chars().next().unwrap_or('+');
                    Some(match (a, b) {
                        // a date plus a number, or a date minus anything, is a date; a number
                        // minus a date is a number (Desktop 2.159)
                        (D::DateTime, _) | (_, D::DateTime) if c == '+' => D::DateTime,
                        (D::DateTime, _) if c == '-' => D::DateTime,
                        _ => match (kind(a), kind(b)) {
                            (Some(ka), Some(kb)) => match currency::binary_kind(c, ka, kb) {
                                currency::Kind::Int => D::Int64,
                                currency::Kind::Currency => D::Decimal,
                                currency::Kind::Real => D::Double,
                            },
                            _ => D::Double,
                        },
                    })
                }
            },
            Expr::Var { body, .. } => self.static_type(body, d),
            Expr::Call { name, args, .. } => {
                let up = name.to_ascii_uppercase();
                let first = |eng: &mut Self| arg(args, 0).and_then(|a| eng.static_type(a, d));
                match up.as_str() {
                    "COUNTROWS"
                    | "COUNT"
                    | "COUNTA"
                    | "COUNTX"
                    | "COUNTAX"
                    | "COUNTBLANK"
                    | "DISTINCTCOUNT"
                    | "DISTINCTCOUNTNOBLANK"
                    | "RANKX"
                    | "DATEDIFF"
                    | "YEAR"
                    | "MONTH"
                    | "DAY"
                    | "QUARTER"
                    | "WEEKDAY"
                    | "WEEKNUM"
                    | "HOUR"
                    | "MINUTE"
                    | "SECOND"
                    | "LEN"
                    | "INT"
                    | "ROWNUMBER"
                    | "RANK"
                    | "FIND"
                    | "SEARCH"
                    | "UNICODE"
                    | "SIGN"
                    | "GCD"
                    | "LCM"
                    | "QUOTIENT"
                    | "BITAND"
                    | "BITOR"
                    | "BITXOR"
                    | "BITLSHIFT"
                    | "BITRSHIFT"
                    | "PATHLENGTH"
                    | "NETWORKDAYS" => Some(D::Int64),
                    "PI" | "SIN" | "COS" | "TAN" | "ASIN" | "ACOS" | "ATAN" | "SINH" | "COSH" | "TANH"
                    | "ASINH" | "ACOSH" | "ATANH" | "COT" | "COTH" | "DEGREES" | "RADIANS" | "LOG10"
                    | "SQRTPI" | "FACT" | "COMBIN" | "COMBINA" | "PERMUT" | "YEARFRAC" => Some(D::Double),
                    // Rounded to a multiple: the type of the significance (a whole number gives an
                    // integer; Desktop 2.159).
                    "CEILING" | "FLOOR" | "MROUND" | "ISO.CEILING" => {
                        let sig = arg(args, 1).map(|a| self.static_type(a, d));
                        if first(self) == Some(D::Decimal) || sig == Some(Some(D::Decimal)) {
                            Some(D::Decimal)
                        } else {
                            match sig {
                                None | Some(Some(D::Int64 | D::Boolean)) => Some(D::Int64),
                                _ => Some(D::Double),
                            }
                        }
                    }
                    "EVEN" | "ODD" => match first(self) {
                        Some(D::Double) => Some(D::Double),
                        _ => Some(D::Int64),
                    },
                    "TIME" | "TIMEVALUE" => Some(D::DateTime),
                    "ISEVEN" | "ISODD" | "ISNUMBER" | "ISTEXT" | "ISNONTEXT" | "ISLOGICAL" | "ISERROR"
                    | "PATHCONTAINS" => Some(D::Boolean),
                    "CONVERT" => match arg(args, 1).and_then(super::keyword).as_deref() {
                        Some("INTEGER") => Some(D::Int64),
                        Some("DOUBLE") => Some(D::Double),
                        Some("CURRENCY") => Some(D::Decimal),
                        Some("STRING") => Some(D::String),
                        Some("BOOLEAN") => Some(D::Boolean),
                        Some("DATETIME") => Some(D::DateTime),
                        _ => None,
                    },
                    "PATHITEM" | "PATHITEMREVERSE" => match arg(args, 2) {
                        None => Some(D::String),
                        Some(e) => match (super::keyword(e).as_deref(), e) {
                            (Some("INTEGER"), _) => Some(D::Int64),
                            (Some("TEXT"), _) => Some(D::String),
                            (_, Expr::Number { text, .. }) => {
                                Some(if text == "1" { D::Int64 } else { D::String })
                            }
                            _ => None,
                        },
                    },
                    "IFERROR" => {
                        let a = first(self);
                        let b = arg(args, 1).and_then(|x| self.static_type(x, d));
                        widen(a, b)
                    }
                    "VALUE" => Some(D::Double),
                    "MOD" => match (first(self), arg(args, 1).and_then(|a| self.static_type(a, d))) {
                        (Some(D::Int64), Some(D::Int64)) => Some(D::Int64),
                        _ => Some(D::Double),
                    },
                    "AVERAGE" | "AVERAGEX" | "MEDIAN" | "MEDIANX" | "DIVIDE" | "STDEV.S" | "STDEV.P"
                    | "VAR.S" | "VAR.P" | "SQRT" | "POWER" | "LN" | "LOG" | "EXP" | "PERCENTILE.INC" => {
                        match (up.as_str(), first(self)) {
                            ("AVERAGE" | "MEDIAN", Some(D::Decimal)) => Some(D::Decimal),
                            _ => Some(D::Double),
                        }
                    }
                    // two-argument MIN and MAX: the wider type of both (Desktop 2.159)
                    "MIN" | "MAX" if args.len() == 2 => {
                        let a = first(self);
                        let b = arg(args, 1).and_then(|x| self.static_type(x, d));
                        widen(a, b)
                    }
                    // rounding keeps the type: ROUND(5, 2) and TRUNC(5) are integers (Desktop 2.159)
                    "SUM" | "MIN" | "MAX" | "FIRSTNONBLANK" | "LASTNONBLANK" | "ABS" | "CALCULATE"
                    | "COALESCE" | "SELECTEDVALUE" | "ROUND" | "ROUNDUP" | "ROUNDDOWN" => first(self),
                    "TRUNC" => match first(self) {
                        Some(D::Int64) => Some(D::Int64),
                        _ => Some(D::Double),
                    },
                    // over a table whose columns are not known here: a number, as most are
                    "SUMX" | "MINX" | "MAXX" => match arg(args, 1).and_then(|a| self.static_type(a, d)) {
                        Some(t) => Some(t),
                        None if up == "SUMX" => Some(D::Double),
                        None => None,
                    },
                    "DATE" | "EOMONTH" | "EDATE" | "TODAY" | "NOW" | "UTCTODAY" | "UTCNOW" | "DATEVALUE"
                    | "MINDATE" => Some(D::DateTime),
                    "FORMAT" | "CONCATENATE" | "CONCATENATEX" | "LEFT" | "RIGHT" | "MID" | "UPPER"
                    | "LOWER" | "TRIM" | "SUBSTITUTE" | "REPLACE" | "REPT" | "FIXED" | "UNICHAR"
                    | "COMBINEVALUES" | "NAMEOF" => Some(D::String),
                    "CURRENCY" => Some(D::Decimal),
                    "TRUE"
                    | "FALSE"
                    | "NOT"
                    | "AND"
                    | "OR"
                    | "ISBLANK"
                    | "ISEMPTY"
                    | "HASONEVALUE"
                    | "ISFILTERED"
                    | "ISINSCOPE"
                    | "ISSUBTOTAL"
                    | "CONTAINS"
                    | "CONTAINSSTRING"
                    | "CONTAINSSTRINGEXACT"
                    | "EXACT" => Some(D::Boolean),
                    "IF" | "IF.EAGER" => {
                        let a = arg(args, 1).and_then(|x| self.static_type(x, d));
                        let b = match arg(args, 2) {
                            Some(x) => self.static_type(x, d),
                            None => a,
                        };
                        if a == b { a } else { None }
                    }
                    "RELATED" | "IGNORE" | "MAXA" | "MINA" => first(self),
                    "LOOKUPVALUE" => first(self),
                    "FIRSTNONBLANKVALUE" | "LASTNONBLANKVALUE" => {
                        arg(args, 1).and_then(|a| self.static_type(a, d))
                    }
                    "RANK.EQ" => Some(D::Int64),
                    "PRODUCT" | "PRODUCTX" | "GEOMEAN" | "GEOMEANX" | "AVERAGEA" | "PERCENTILE.EXC"
                    | "PERCENTILEX.INC" | "PERCENTILEX.EXC" | "STDEVX.S" | "STDEVX.P" | "VARX.S"
                    | "VARX.P" => Some(D::Double),
                    "CONTAINSROW" | "HASONEFILTER" | "ISCROSSFILTERED" => Some(D::Boolean),
                    _ => None,
                }
            }
            _ => None,
        }
    }
}

/// The type of a value that is one of two types: a number type wide enough for both (an integer
/// and a Currency make a Currency, a double wins), else the type they share.
fn widen(a: Option<DataType>, b: Option<DataType>) -> Option<DataType> {
    use DataType as D;
    let num = |t: DataType| match t {
        D::Int64 | D::Boolean => Some(0),
        D::Decimal => Some(1),
        D::Double => Some(2),
        _ => None,
    };
    match (a, b) {
        (Some(x), Some(y)) if x == y => Some(x),
        (Some(x), Some(y)) => match (num(x), num(y)) {
            (Some(i), Some(j)) => Some(match i.max(j) {
                0 => D::Int64,
                1 => D::Decimal,
                _ => D::Double,
            }),
            _ => None,
        },
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::V;
    use super::{parse_date_text, sort_cmp};
    use std::cmp::Ordering;

    #[test]
    fn dates_and_sort_order() {
        assert_eq!(parse_date_text("2024-01-01"), Some(45_292.0));
        assert_eq!(parse_date_text("2024-01-01 12:00:00"), Some(45_292.5));
        assert_eq!(parse_date_text("x"), None);
        assert_eq!(sort_cmp(&V::Blank, &V::Num(-5.0)), Ordering::Less);
        assert_eq!(sort_cmp(&V::Num(2.0), &V::Text("a".into())), Ordering::Less);
        assert_eq!(
            sort_cmp(&V::Text("B".into()), &V::Text("a".into())),
            Ordering::Greater
        );
    }
}
