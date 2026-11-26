//! Table functions of the DAX evaluator ([`super`]) that pick or complete rows:
//! `ADDMISSINGITEMS`, `SUBSTITUTEWITHINDEX`, `SAMPLE` and `TOPNSKIP`.
//!
//! Semantics from the public DAX reference (Microsoft Learn, `docs/formats/dax-functions.md`):
//! - `ADDMISSINGITEMS(show_all…, table, group_by… [, filter_table]…)`: the rows of `table`, then
//!   one row for each combination of the show-all columns (their existing combinations within a
//!   table, crossed between tables, under the filter tables: what `SUMMARIZECOLUMNS` of those
//!   columns returns) that no row of `table` has on the group-by columns; the new rows hold the
//!   combination in those columns and blank elsewhere. Show-all columns left out are the group-by
//!   columns. With `ROLLUPISSUBTOTAL(group, [is_subtotal]…)` (a group is a column or a
//!   `ROLLUPGROUP`), each group is a subtotal level whose rows have the table's `is_subtotal`
//!   column TRUE; the levels nest as `ROLLUPADDISSUBTOTAL` builds them, and the missing
//!   combinations of the show-all columns still grouped are added at every level, flagged as
//!   it (one row when none is left grouped). Its filter arguments are filter tables of one
//!   level: `grandTotalFilter` of the level where every group is rolled up, the
//!   `groupLevelFilter` after a group of the level where that group is the deepest one still
//!   grouped (a reading of the reference's "filter applied to the current level"). Several
//!   `ROLLUPISSUBTOTAL` are crossed, as `SUMMARIZECOLUMNS` crosses several
//!   `ROLLUPADDISSUBTOTAL`: every combination of their levels.
//! - `SUBSTITUTEWITHINDEX(table, name, index_table [, order_by [, order]]…)`: a left semijoin on
//!   the columns both tables have (same name, same kind of data type); the left rows without a
//!   match are dropped and the common columns are replaced, at the place of the first of them, by
//!   one integer column `name`: the 0-based position of the matching row of `index_table` sorted
//!   by the order keys (descending when the order is left out, as the reference says; ties keep
//!   the table's order).
//! - `SAMPLE(n, table, order_by [, order]…)`: `n` rows of `table` sorted by the keys: the first
//!   and the last and rows evenly spaced between them (row `round(i·(len−1)/(n−1))` for
//!   `i = 0…n−1`); all rows when `n` is at least the row count, the first row when `n` is 1, an
//!   empty table when `n` is 0 or less. The reference gives no default order: ascending here.
//!   Without an order key the reference's sample is random: refused.
//! - `TOPNSKIP(n, skip, table [, order_by [, order]]…)` (no Learn page; the engine's function
//!   list names its parameters): the rows after the first `skip` of `table` sorted by the keys
//!   (descending when the order is left out, as `TOPN`), at most `n` of them; unlike `TOPN`, rows
//!   that tie on the keys are not added, except rows identical to the last one kept. The `ties`
//!   argument (dax.guide's reading, unconfirmed): the rows ranked in the sort order, ties
//!   sharing a rank, `SKIP` counting the rows before, `DENSE` the distinct keys before; the
//!   rows ranked after `skip` and up to `skip + n` are kept.
//!
//! Counts are truncated to whole rows; blank, negative and NaN counts are 0. Sorting is DAX's
//! order (`sort_cmp`), stable, so rows the keys do not tell apart keep the table's order.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use bicraft_dax::ast::Expr;
use bicraft_model::DataType;

use super::super::Fail;
use super::tables::{row_key, sort_cmp};
use super::{
    ColId, Engine, Scope, TCol, Tab, V, arg, dax_error, keyword, need, num, subset, truth, unsupported,
};

/// Most rows `ADDMISSINGITEMS` builds.
const MAX_ROWS: usize = 2_000_000;

/// Table functions this module evaluates.
pub(super) fn handles_table(up: &str) -> bool {
    matches!(
        up,
        "ADDMISSINGITEMS" | "SUBSTITUTEWITHINDEX" | "SAMPLE" | "TOPNSKIP"
    )
}

/// A row count: truncated; blank, negative and NaN are 0.
fn count(x: Option<f64>) -> usize {
    match x {
        Some(v) if v >= 1.0 => v as usize,
        _ => 0,
    }
}

/// Positions of `n` rows out of `len` (`1 < n < len`): the first, the last and rows evenly spaced
/// between them (rounded half up).
fn sample_positions(n: usize, len: usize) -> Vec<usize> {
    let (n128, last) = (n as u128, len.saturating_sub(1) as u128);
    let gaps = n128.saturating_sub(1).max(1);
    (0..n128)
        .map(|i| {
            let p = (2 * i * last + gaps) / (2 * gaps);
            usize::try_from(p).unwrap_or(usize::MAX)
        })
        .collect()
}

/// The kind of a data type, for matching columns (`None`: not checked).
fn type_kind(t: Option<DataType>) -> Option<u8> {
    match t? {
        DataType::String => Some(0),
        DataType::Int64 | DataType::Double | DataType::Decimal => Some(1),
        DataType::DateTime => Some(2),
        DataType::Boolean => Some(3),
        DataType::Unknown | DataType::Binary | DataType::Variant => None,
    }
}

fn is_rollup(e: &Expr) -> bool {
    matches!(e, Expr::Call { name, .. } if matches!(
        name.to_ascii_uppercase().as_str(),
        "ROLLUPISSUBTOTAL" | "ROLLUPGROUP" | "ROLLUPADDISSUBTOTAL" | "ROLLUP"
    ))
}

impl Engine<'_, '_> {
    pub(super) fn pick_table(&mut self, up: &str, args: &[Option<Expr>], s: &Scope) -> Result<Tab, Fail> {
        match up {
            "SAMPLE" => self.sample(args, s),
            "TOPNSKIP" => self.top_n_skip(args, s),
            "SUBSTITUTEWITHINDEX" => self.substitute_with_index(args, s),
            "ADDMISSINGITEMS" => self.add_missing_items(args, s),
            _ => unsupported(format!("{up}()")),
        }
    }

    /// The `(order_by, descending)` keys from argument `from` on; an order argument is `ASC`,
    /// `DESC`, a number or a boolean after its expression.
    fn order_args<'e>(
        &mut self,
        args: &'e [Option<Expr>],
        from: usize,
        default_desc: bool,
        s: &Scope,
    ) -> Result<Vec<(&'e Expr, bool)>, Fail> {
        let mut keys = Vec::new();
        let mut i = from;
        while let Some(e) = arg(args, i) {
            i = i.saturating_add(1);
            let mut desc = default_desc;
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
            keys.push((e, desc));
        }
        Ok(keys)
    }

    /// The positions of the rows of `t` sorted by `keys` (stable; the table's order without keys).
    fn sorted_rows(&mut self, t: &Tab, keys: &[(&Expr, bool)], s: &Scope) -> Result<Vec<usize>, Fail> {
        Ok(self.sorted_rows_keyed(t, keys, s)?.0)
    }

    /// [`Engine::sorted_rows`], with the key values of each row (by row of `t`).
    fn sorted_rows_keyed(
        &mut self,
        t: &Tab,
        keys: &[(&Expr, bool)],
        s: &Scope,
    ) -> Result<(Vec<usize>, Vec<Vec<V>>), Fail> {
        let mut order: Vec<usize> = (0..t.len()).collect();
        if keys.is_empty() {
            return Ok((order, vec![Vec::new(); t.len()]));
        }
        let vals = self.each_with(t, s, |eng, inner| {
            keys.iter()
                .map(|(e, _)| eng.scalar(e, inner))
                .collect::<Result<Vec<V>, Fail>>()
        })?;
        let at = |row: usize, k: usize| vals.get(row).and_then(|r| r.get(k)).unwrap_or(&V::Blank);
        order.sort_by(|a, b| {
            for (k, (_, desc)) in keys.iter().enumerate() {
                let o = sort_cmp(at(*a, k), at(*b, k));
                let o = if *desc { o.reverse() } else { o };
                if o != Ordering::Equal {
                    return o;
                }
            }
            Ordering::Equal
        });
        Ok((order, vals))
    }

    fn sample(&mut self, args: &[Option<Expr>], s: &Scope) -> Result<Tab, Fail> {
        let f = "SAMPLE";
        let n = count(num(&self.scalar(need(args, 0, f)?, s)?)?);
        let source = need(args, 1, f)?;
        let keys = self.order_args(args, 2, false, s)?;
        if keys.is_empty() {
            return unsupported("SAMPLE() without an order key (a random sample)");
        }
        let t = self.table(source, s)?;
        if n == 0 {
            return Ok(subset(&t, &[]));
        }
        let order = self.sorted_rows(&t, &keys, s)?;
        let len = order.len();
        let picked: Vec<usize> = if n >= len {
            order
        } else if n == 1 {
            order.into_iter().take(1).collect()
        } else {
            sample_positions(n, len)
                .into_iter()
                .filter_map(|p| order.get(p).copied())
                .collect()
        };
        Ok(subset(&t, &picked))
    }

    fn top_n_skip(&mut self, args: &[Option<Expr>], s: &Scope) -> Result<Tab, Fail> {
        let f = "TOPNSKIP";
        let n = count(num(&self.scalar(need(args, 0, f)?, s)?)?);
        let skip = count(num(&self.scalar(need(args, 1, f)?, s)?)?);
        let source = need(args, 2, f)?;
        // a last argument `SKIP` or `DENSE` (a keyword, not a table or variable) is the ties mode
        let mut args = args;
        let mut ties: Option<bool> = None;
        if args.len() > 3
            && let Some(Some(a @ Expr::Name { name, .. })) = args.last()
            && self.table_id(name).is_none()
            && !s.env.iter().any(|(n, _)| n.eq_ignore_ascii_case(name))
        {
            match keyword(a).as_deref() {
                Some("SKIP") => ties = Some(false),
                Some("DENSE") => ties = Some(true),
                _ => {}
            }
            if ties.is_some() {
                args = args.get(..args.len().saturating_sub(1)).unwrap_or_default();
            }
        }
        let keys = self.order_args(args, 3, true, s)?;
        let t = self.table(source, s)?;
        if n == 0 {
            return Ok(subset(&t, &[]));
        }
        let (order, kvals) = self.sorted_rows_keyed(&t, &keys, s)?;
        let len = order.len();
        if let Some(dense) = ties {
            // the rank of each row in the sort order: `SKIP` counts the rows before it, `DENSE`
            // the distinct keys before it; the rows ranked after `skip` and up to `skip + n` stay
            // keys tie as they sort (text without regard to case)
            let key_of = |p: usize| order.get(p).and_then(|r| kvals.get(*r));
            let same = |a: Option<&Vec<V>>, b: Option<&Vec<V>>| match (a, b) {
                (Some(a), Some(b)) => {
                    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| sort_cmp(x, y) == Ordering::Equal)
                }
                _ => false,
            };
            let (lo, hi) = (skip as u128, (skip as u128).saturating_add(n as u128));
            let mut picked: Vec<usize> = Vec::new();
            let mut rank: u128 = 0;
            for p in 0..len {
                let new_key = p == 0 || !same(key_of(p), key_of(p.saturating_sub(1)));
                if new_key {
                    rank = if dense {
                        rank.saturating_add(1)
                    } else {
                        (p as u128).saturating_add(1)
                    };
                }
                if rank > hi {
                    break;
                }
                if rank > lo
                    && let Some(r) = order.get(p)
                {
                    picked.push(*r);
                }
            }
            return Ok(subset(&t, &picked));
        }
        let start = skip.min(len);
        let mut end = start.saturating_add(n).min(len);
        // rows identical to the last one kept tie with it and come along
        if end > start && end < len {
            let vals = self.tab_values(&t)?;
            let key_at = |p: usize| order.get(p).and_then(|r| vals.get(*r)).map(|r| row_key(r));
            let last = key_at(end.saturating_sub(1));
            while end < len && key_at(end) == last {
                end = end.saturating_add(1);
            }
        }
        let picked = order.get(start..end).unwrap_or_default();
        Ok(subset(&t, picked))
    }

    /// The data type of a column of a table value, when known.
    fn tcol_type(&self, c: &TCol) -> Option<DataType> {
        c.ty.or_else(|| c.lin.and_then(|l| self.col_type(l)))
    }

    fn substitute_with_index(&mut self, args: &[Option<Expr>], s: &Scope) -> Result<Tab, Fail> {
        let f = "SUBSTITUTEWITHINDEX";
        let left = self.table(need(args, 0, f)?, s)?;
        let Some(Expr::Str { value: name, .. }) = arg(args, 1) else {
            return unsupported(
                "SUBSTITUTEWITHINDEX() expects the index column name (a string) as argument 2",
            );
        };
        let right = self.table(need(args, 2, f)?, s)?;
        let keys = self.order_args(args, 3, true, s)?;
        let lcols = self.tab_cols(&left);
        let rcols = self.tab_cols(&right);
        // (left position, right position) of each common column
        let mut common: Vec<(usize, usize)> = Vec::new();
        for (ri, rc) in rcols.iter().enumerate() {
            let rn = self.tcol_name(rc);
            let Some(li) = lcols
                .iter()
                .position(|lc| self.tcol_name(lc).eq_ignore_ascii_case(&rn))
            else {
                continue;
            };
            let (lk, rk) = (
                type_kind(lcols.get(li).and_then(|c| self.tcol_type(c))),
                type_kind(self.tcol_type(rc)),
            );
            if matches!((lk, rk), (Some(a), Some(b)) if a != b) || common.iter().any(|(l, _)| *l == li) {
                continue;
            }
            common.push((li, ri));
        }
        let Some(first) = common.iter().map(|(l, _)| *l).min() else {
            return dax_error("SUBSTITUTEWITHINDEX(): the tables have no column in common");
        };
        self.tick(left.len().saturating_add(right.len()))?;
        // the index of each combination of the right table (its first row in the sort order)
        let order = self.sorted_rows(&right, &keys, s)?;
        let rvals = self.tab_values(&right)?;
        let mut index: HashMap<_, usize> = HashMap::new();
        for (pos, r) in order.iter().enumerate() {
            let Some(row) = rvals.get(*r) else { continue };
            let key: Vec<_> = common
                .iter()
                .map(|(_, ri)| row.get(*ri).and_then(V::key))
                .collect();
            index.entry(key).or_insert(pos);
        }
        let is_common = |i: usize| common.iter().any(|(l, _)| *l == i);
        let mut cols: Vec<TCol> = Vec::with_capacity(lcols.len());
        for (i, c) in lcols.iter().enumerate() {
            if i == first {
                cols.push(TCol::named(name.clone(), Some(DataType::Int64)));
            }
            if !is_common(i) {
                cols.push(c.clone());
            }
        }
        self.check_names(&cols, f)?;
        let lvals = self.tab_values(&left)?;
        let mut rows: Vec<Vec<V>> = Vec::new();
        for row in lvals.iter() {
            let key: Vec<_> = common
                .iter()
                .map(|(li, _)| row.get(*li).and_then(V::key))
                .collect();
            let Some(ix) = index.get(&key) else { continue };
            let mut out = Vec::with_capacity(cols.len());
            for (i, v) in row.iter().enumerate() {
                if i == first {
                    out.push(V::Num(*ix as f64));
                }
                if !is_common(i) {
                    out.push(v.clone());
                }
            }
            rows.push(out);
        }
        Ok(Tab::tuples(cols, rows))
    }

    fn add_missing_items(&mut self, args: &[Option<Expr>], s: &Scope) -> Result<Tab, Fail> {
        let f = "ADDMISSINGITEMS";
        // the show-all columns, up to the table
        let mut show: Vec<(ColId, Expr)> = Vec::new();
        let mut i = 0usize;
        let table_expr = loop {
            let Some(a) = args.get(i) else {
                return unsupported("ADDMISSINGITEMS() without a table");
            };
            i = i.saturating_add(1);
            let Some(a) = a else { continue };
            if is_rollup(a) {
                return unsupported("ADDMISSINGITEMS() with a rollup function before its table");
            }
            match a {
                Expr::Column { table, name, .. } if self.measure_def(table.as_deref(), name).is_none() => {
                    let c = self.col_arg(a, &[], f)?;
                    show.push((c, a.clone()));
                }
                other => break other,
            }
        };
        // the group-by columns, the subtotal levels and the filter tables
        let mut groups: Vec<(ColId, Expr)> = Vec::new();
        // the levels of every ROLLUPISSUBTOTAL, one after the other, and for each of them
        // (its first level, its level count, its grand total filter)
        let mut levels: Vec<Level> = Vec::new();
        let mut rollups: Vec<(usize, usize, Option<Expr>)> = Vec::new();
        let mut filters: Vec<Option<Expr>> = Vec::new();
        for a in args.iter().skip(i).flatten() {
            match a {
                Expr::Column { table, name, .. } if self.measure_def(table.as_deref(), name).is_none() => {
                    let c = self.col_arg(a, &[], f)?;
                    groups.push((c, a.clone()));
                }
                Expr::Call {
                    name, args: inner, ..
                } if name.eq_ignore_ascii_case("ROLLUPGROUP") => {
                    groups.extend(self.rollup_group_cols(inner)?);
                }
                Expr::Call {
                    name, args: inner, ..
                } if name.eq_ignore_ascii_case("ROLLUPISSUBTOTAL") => {
                    let (more, grand) = self.subtotal_levels(inner, s)?;
                    rollups.push((levels.len(), more.len(), grand));
                    levels.extend(more);
                }
                other if is_rollup(other) => {
                    return unsupported(format!(
                        "{}() in ADDMISSINGITEMS()",
                        match other {
                            Expr::Call { name, .. } => name.to_ascii_uppercase(),
                            _ => String::new(),
                        }
                    ));
                }
                other if self.is_table_expr(other, s.env) => filters.push(Some(other.clone())),
                _ => {
                    return unsupported("ADDMISSINGITEMS() argument that is not a column or a table");
                }
            }
        }
        let t = self.table(table_expr, s)?;
        // every group-by column, the subtotal levels' after the plain ones
        let mut all: Vec<(ColId, Expr)> = groups.clone();
        for l in &levels {
            all.extend(l.cols.iter().cloned());
        }
        if all.is_empty() {
            return Ok(t);
        }
        if show.is_empty() {
            show = all.clone();
        }
        let tcols = self.tab_cols(&t);
        let mut at_col: HashMap<ColId, usize> = HashMap::new();
        for (c, _) in &all {
            match tcols.iter().position(|x| x.lin == Some(*c)) {
                Some(p) => {
                    at_col.insert(*c, p);
                }
                None => {
                    return dax_error(format!(
                        "ADDMISSINGITEMS(): the table has no column {}",
                        self.col_name(*c)
                    ));
                }
            }
        }
        let mut at_flag: Vec<usize> = Vec::with_capacity(levels.len());
        for l in &levels {
            match tcols
                .iter()
                .position(|x| x.lin.is_none() && self.tcol_name(x).eq_ignore_ascii_case(&l.flag))
            {
                Some(p) => at_flag.push(p),
                None => {
                    return dax_error(format!("ADDMISSINGITEMS(): the table has no column [{}]", l.flag));
                }
            }
        }
        // no show-all column is a group-by column: nothing to add
        if !show.iter().any(|(c, _)| at_col.contains_key(c)) {
            return Ok(t);
        }
        let tvals = self.tab_values(&t)?;
        self.tick(tvals.len())?;
        let width = tcols.len();
        let mut rows: Vec<Vec<V>> = tvals.as_ref().clone();
        // The levels of each ROLLUPISSUBTOTAL: the detail rows (no subtotal level rolled up),
        // then the rows with its last level rolled up, …, then the rows with every level rolled
        // up (the grand total of the rolled-up columns); its levels from `rolled` on are rolled
        // up, their flags TRUE. Several ROLLUPISSUBTOTAL: every combination of their states.
        let mut states: Vec<Vec<usize>> = vec![Vec::new()];
        for (_, n, _) in &rollups {
            let mut next = Vec::new();
            for st in &states {
                for rolled in (0..=*n).rev() {
                    let mut x = st.clone();
                    x.push(rolled);
                    next.push(x);
                }
            }
            if next.len() > MAX_ROWS {
                return unsupported(format!("{f}() with too many subtotal levels"));
            }
            states = next;
        }
        for state in states {
            // levels still grouped, flags, and the filter tables of this state's levels
            let mut grouped = vec![false; levels.len()];
            let mut level_filters: Vec<Option<Expr>> = Vec::new();
            for ((start, n, grand), rolled) in rollups.iter().zip(&state) {
                for k in 0..*rolled {
                    if let Some(g) = grouped.get_mut(start.saturating_add(k)) {
                        *g = true;
                    }
                }
                let filter = match rolled.checked_sub(1) {
                    None => grand.clone(),
                    Some(k) if k < *n => levels.get(start.saturating_add(k)).and_then(|l| l.filter.clone()),
                    Some(_) => None,
                };
                if filter.is_some() {
                    level_filters.push(filter);
                }
            }
            let detail = grouped.iter().all(|g| *g);
            let active: HashSet<ColId> = groups
                .iter()
                .map(|(c, _)| *c)
                .chain(
                    levels
                        .iter()
                        .zip(&grouped)
                        .filter(|(_, g)| **g)
                        .flat_map(|(l, _)| l.cols.iter().map(|(c, _)| *c)),
                )
                .collect();
            let flags: Vec<bool> = grouped.iter().map(|g| !g).collect();
            // (position in the show-all columns, position in the table) of the active show-alls
            let pairs: Vec<(usize, usize)> = show
                .iter()
                .enumerate()
                .filter(|(_, (c, _))| active.contains(c))
                .filter_map(|(si, (c, _))| Some((si, *at_col.get(c)?)))
                .collect();
            if pairs.is_empty() && detail {
                // detail rows without a show-all column among them
                continue;
            }
            let of_level = |r: &[V]| -> Result<bool, Fail> {
                for (fi, want) in at_flag.iter().zip(&flags) {
                    let v = r.get(*fi).unwrap_or(&V::Blank);
                    let on = !matches!(v, V::Blank) && truth(v)?;
                    if on != *want {
                        return Ok(false);
                    }
                }
                Ok(true)
            };
            let mut seen: HashSet<Vec<Option<super::super::store::Key>>> = HashSet::new();
            for r in tvals.iter() {
                if of_level(r)? {
                    seen.insert(pairs.iter().map(|(_, ti)| r.get(*ti).and_then(V::key)).collect());
                }
            }
            // the combinations of the active show-all columns (one empty one without any)
            let combos: Vec<Vec<V>> = if pairs.is_empty() {
                vec![Vec::new()]
            } else {
                let mut domain_args: Vec<Option<Expr>> = pairs
                    .iter()
                    .filter_map(|(si, _)| show.get(*si).map(|(_, e)| Some(e.clone())))
                    .collect();
                domain_args.extend(filters.iter().cloned());
                domain_args.extend(level_filters.iter().cloned());
                let domain = self.summarize_columns(&domain_args, s)?;
                let d = self.tab_values(&domain)?;
                self.tick(d.len())?;
                d.as_ref().clone()
            };
            for combo in combos {
                let key: Vec<_> = (0..pairs.len()).map(|k| combo.get(k).and_then(V::key)).collect();
                if !seen.insert(key) {
                    continue;
                }
                let mut row = vec![V::Blank; width];
                for (k, (_, ti)) in pairs.iter().enumerate() {
                    if let (Some(slot), Some(v)) = (row.get_mut(*ti), combo.get(k)) {
                        *slot = v.clone();
                    }
                }
                for (fi, on) in at_flag.iter().zip(&flags) {
                    if let Some(slot) = row.get_mut(*fi) {
                        *slot = V::Bool(*on);
                    }
                }
                rows.push(row);
                if rows.len() > MAX_ROWS {
                    return unsupported(format!("{f}() builds more than {MAX_ROWS} rows"));
                }
            }
        }
        Ok(Tab::tuples(tcols, rows))
    }

    /// The columns of a `ROLLUPGROUP(column…)`.
    fn rollup_group_cols(&mut self, args: &[Option<Expr>]) -> Result<Vec<(ColId, Expr)>, Fail> {
        let mut out = Vec::new();
        for a in args.iter().flatten() {
            match a {
                Expr::Column { .. } => {
                    let c = self.col_arg(a, &[], "ROLLUPGROUP")?;
                    out.push((c, a.clone()));
                }
                _ => return dax_error("ROLLUPGROUP() takes columns only"),
            }
        }
        if out.is_empty() {
            return dax_error("ROLLUPGROUP() needs a column");
        }
        Ok(out)
    }

    /// The levels of a `ROLLUPISSUBTOTAL([grand_total_filter,] group, is_subtotal
    /// [, [level_filter,] group, is_subtotal]…)`: each group a column or a `ROLLUPGROUP`, each
    /// flag the name of a column of the table, each filter a table expression (the grand total
    /// filter before the first group, a level filter after its group's flag); returns the
    /// levels and the grand total filter.
    fn subtotal_levels(
        &mut self,
        args: &[Option<Expr>],
        s: &Scope,
    ) -> Result<(Vec<Level>, Option<Expr>), Fail> {
        let mut levels: Vec<Level> = Vec::new();
        let mut grand: Option<Expr> = None;
        let mut i = 0usize;
        while i < args.len() {
            let Some(a) = arg(args, i) else {
                i = i.saturating_add(1);
                continue;
            };
            i = i.saturating_add(1);
            let cols = match a {
                Expr::Column { table: Some(_), .. } => {
                    vec![(self.col_arg(a, &[], "ROLLUPISSUBTOTAL")?, a.clone())]
                }
                Expr::Call {
                    name, args: inner, ..
                } if name.eq_ignore_ascii_case("ROLLUPGROUP") => self.rollup_group_cols(inner)?,
                other if self.is_table_expr(other, s.env) => {
                    let slot = match levels.last_mut() {
                        Some(l) => &mut l.filter,
                        None => &mut grand,
                    };
                    if slot.is_some() {
                        return dax_error("ROLLUPISSUBTOTAL(): two filters for one level");
                    }
                    *slot = Some(other.clone());
                    continue;
                }
                _ => return dax_error("ROLLUPISSUBTOTAL(): expected a group-by column or a filter table"),
            };
            let flag = match arg(args, i) {
                Some(Expr::Column {
                    table: None, name, ..
                }) => name.clone(),
                _ => {
                    return dax_error(
                        "ROLLUPISSUBTOTAL(): each group-by column is followed by its subtotal column ([Name])",
                    );
                }
            };
            i = i.saturating_add(1);
            levels.push(Level {
                cols,
                flag,
                filter: None,
            });
        }
        if levels.is_empty() {
            return dax_error("ROLLUPISSUBTOTAL() needs a group-by column and its subtotal column");
        }
        Ok((levels, grand))
    }
}

/// One subtotal level of `ROLLUPISSUBTOTAL`: its group-by columns, the name of the table's
/// column that is TRUE on the rows where they are rolled up, and its level filter (a table
/// expression applied to the combinations added where this group is the deepest grouped).
struct Level {
    cols: Vec<(ColId, Expr)>,
    flag: String,
    filter: Option<Expr>,
}
