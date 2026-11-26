//! Lookup, filter-information, non-blank and statistical functions of the DAX evaluator
//! ([`super`]): `LOOKUPVALUE`, `CONTAINS`, `CONTAINSROW`, `HASONEFILTER`, `ISCROSSFILTERED`,
//! `FILTERS`, `FIRSTNONBLANK`, `LASTNONBLANK`, `FIRSTNONBLANKVALUE`, `LASTNONBLANKVALUE`,
//! `RELATEDTABLE`, `PRODUCT`, `PRODUCTX`, `GEOMEAN`, `GEOMEANX`, `MEDIANX`, `PERCENTILE.INC`,
//! `PERCENTILE.EXC`, `PERCENTILEX.INC`, `PERCENTILEX.EXC`, `STDEVX.S`, `STDEVX.P`, `VARX.S`,
//! `VARX.P`, `MAXA`, `MINA`, `AVERAGEA`, `RANK.EQ`, `ERROR`.
//!
//! Checked against the engine of Power BI Desktop 2.159 (`tests/dax_coverage`):
//! - `LOOKUPVALUE` ignores the filter context (`CALCULATE(LOOKUPVALUE(…), Product[Category] =
//!   "Toys")` still finds a product of another category); several distinct results are an error
//!   unless an alternate result is given; the search columns are columns of the result's table;
//! - `FILTERS(column)` lists the values the filters on that column allow, every value (with the
//!   blank row) when it has none; `HASONEFILTER` is "`FILTERS` has one row" for a filtered column;
//!   `ISCROSSFILTERED` is true when a filter reaches the column's table, directly or through
//!   relationships;
//! - `FIRSTNONBLANK(column, expression)` evaluates the expression for each value of the column in
//!   the context (ascending), with that value as row context (a measure sees it as a filter).

use std::cmp::Ordering;
use std::collections::HashSet;

use bicraft_dax::ast::Expr;

use super::super::Fail;
use super::tables::sort_cmp;
use super::{
    ColId, Dir, Engine, FCtx, RowB, Scope, TCol, Tab, TabId, V, arg, dax_error, need, num, strictly_equal,
    unsupported,
};

/// Scalar functions this module evaluates.
pub(super) fn handles(up: &str) -> bool {
    matches!(
        up,
        "LOOKUPVALUE"
            | "ERROR"
            | "CONTAINS"
            | "CONTAINSROW"
            | "HASONEFILTER"
            | "ISCROSSFILTERED"
            | "FIRSTNONBLANKVALUE"
            | "LASTNONBLANKVALUE"
            | "PRODUCT"
            | "PRODUCTX"
            | "GEOMEAN"
            | "GEOMEANX"
            | "MEDIANX"
            | "PERCENTILE.INC"
            | "PERCENTILE.EXC"
            | "PERCENTILEX.INC"
            | "PERCENTILEX.EXC"
            | "STDEVX.S"
            | "STDEVX.P"
            | "VARX.S"
            | "VARX.P"
            | "MAXA"
            | "MINA"
            | "AVERAGEA"
            | "RANK.EQ"
    )
}

/// The `k`-th percentile of sorted `xs` (Excel's interpolation); `exclusive`: `.EXC`.
fn percentile(xs: &[f64], k: f64, exclusive: bool) -> Option<f64> {
    let n = xs.len() as f64;
    if xs.is_empty() || !(0.0..=1.0).contains(&k) {
        return None;
    }
    let rank = if exclusive {
        if k < 1.0 / (n + 1.0) || k > n / (n + 1.0) {
            return None;
        }
        k * (n + 1.0) - 1.0
    } else {
        k * (n - 1.0)
    };
    let lo = rank.floor();
    let frac = rank - lo;
    let i = lo as usize;
    let a = *xs.get(i)?;
    let b = xs.get(i.saturating_add(1)).copied().unwrap_or(a);
    Some(a + frac * (b - a))
}

impl Engine<'_, '_> {
    /// Values of column `c` on the rows of its table visible in `fc` (one per row).
    fn column_values(&mut self, c: ColId, fc: &FCtx) -> Result<Vec<V>, Fail> {
        let t = self.col_table(c)?;
        let rows = self.visible_rows(fc, t)?;
        let d = self.col(c)?;
        Ok(rows
            .iter()
            .map(|r| {
                let code = d.codes.get(*r as usize).unwrap_or(0);
                d.values.get(code as usize).map_or(V::Blank, V::of_cell)
            })
            .collect())
    }

    /// Non-blank numbers of a column (`col_fn`) or of an iterator's expression.
    fn numbers(
        &mut self,
        up: &str,
        args: &[Option<Expr>],
        s: &Scope,
        iterator: bool,
    ) -> Result<Vec<f64>, Fail> {
        let vals = if iterator {
            let t = self.table(need(args, 0, up)?, s)?;
            self.each(&t, need(args, 1, up)?, s)?
        } else {
            let c = self.col_arg(need(args, 0, up)?, s.rc, up)?;
            self.column_values(c, s.fc)?
        };
        let mut out = Vec::with_capacity(vals.len());
        for v in &vals {
            match v {
                V::Blank => {}
                V::Text(_) if up.ends_with('A') && !iterator => out.push(0.0),
                V::Bool(b) if up.ends_with('A') && !iterator => out.push(f64::from(u8::from(*b))),
                V::Text(_) => return dax_error(format!("{up}() of text values")),
                other => {
                    if let Some(x) = num(other)? {
                        out.push(x);
                    }
                }
            }
        }
        Ok(out)
    }

    /// The filter context with only the filters on column `c`.
    fn direct_filters(fc: &FCtx, c: ColId) -> FCtx {
        FCtx {
            filters: fc
                .filters
                .iter()
                .filter(|f| f.cols.contains(&c))
                .cloned()
                .collect(),
            ..FCtx::default()
        }
    }

    /// Whether a filter of `fc` reaches table `t` (on `t` itself or through relationships).
    fn crossfiltered(&self, fc: &FCtx, t: TabId) -> bool {
        let mut reached: HashSet<TabId> = fc.filters.iter().map(|f| f.table).collect();
        let mut frontier: Vec<TabId> = reached.iter().copied().collect();
        while let Some(x) = frontier.pop() {
            for r in 0..self.rels.len() {
                let (active, dir) = self.rel_state(fc, r);
                let Some(rel) = self.rels.get(r) else { continue };
                if !active || dir == Dir::None {
                    continue;
                }
                // filters flow from the one side into the many side, both ways when Both
                let next = if rel.to_t == x {
                    Some(rel.from_t)
                } else if rel.from_t == x && dir == Dir::Both {
                    Some(rel.to_t)
                } else {
                    None
                };
                if let Some(n) = next
                    && reached.insert(n)
                {
                    frontier.push(n);
                }
            }
        }
        reached.contains(&t)
    }

    /// `FIRSTNONBLANK` / `LASTNONBLANK`: the first (last) value of a column for which the
    /// expression is not blank, with that expression's value.
    fn non_blank(
        &mut self,
        up: &str,
        args: &[Option<Expr>],
        s: &Scope,
    ) -> Result<Option<(ColId, V, V)>, Fail> {
        let col = need(args, 0, up)?;
        let c = self.col_arg(col, s.rc, up)?;
        let body = need(args, 1, up)?;
        let fc = if s.rc.is_empty() {
            s.fc.child()
        } else {
            self.transitioned(s.fc, s.rc)?
        };
        let codes = self.values_codes(&fc, c, true)?;
        let mut vals = codes
            .iter()
            .map(|code| self.value_of_code(c, *code))
            .collect::<Result<Vec<V>, Fail>>()?;
        vals.sort_by(sort_cmp);
        if up.starts_with("LAST") {
            vals.reverse();
        }
        let cols = std::sync::Arc::new(vec![TCol::of(c)]);
        for v in vals {
            let mut rc: Vec<RowB> = s.rc.to_vec();
            rc.push(RowB::Tuple {
                cols: std::sync::Arc::clone(&cols),
                vals: vec![v.clone()],
            });
            let inner = Scope {
                fc: &fc,
                rc: &rc,
                ..*s
            };
            let r = self.scalar(body, &inner)?;
            if !r.is_blank() {
                return Ok(Some((c, v, r)));
            }
        }
        Ok(None)
    }

    /// Table functions of this module: `FILTERS`, `FIRSTNONBLANK`, `LASTNONBLANK`,
    /// `RELATEDTABLE`.
    pub(super) fn more_table(
        &mut self,
        up: &str,
        args: &[Option<Expr>],
        s: &Scope,
    ) -> Result<Option<Tab>, Fail> {
        Ok(Some(match up {
            "FILTERS" => {
                let c = self.col_arg(need(args, 0, up)?, s.rc, up)?;
                let sub = Self::direct_filters(s.fc, c);
                let codes = self.values_codes(&sub, c, true)?;
                let rows = codes
                    .iter()
                    .map(|code| self.value_of_code(c, *code).map(|v| vec![v]))
                    .collect::<Result<Vec<_>, _>>()?;
                Tab::tuples(vec![TCol::of(c)], rows)
            }
            "FIRSTNONBLANK" | "LASTNONBLANK" => match self.non_blank(up, args, s)? {
                Some((c, v, _)) => Tab::tuples(vec![TCol::of(c)], vec![vec![v]]),
                None => {
                    let c = self.col_arg(need(args, 0, up)?, s.rc, up)?;
                    Tab::empty_of(c)
                }
            },
            "RELATEDTABLE" => {
                let Some(Expr::Name { name, .. }) = arg(args, 0) else {
                    return unsupported("RELATEDTABLE() of something other than a table");
                };
                let Some(t) = self.table_id(name) else {
                    return unsupported(format!("RELATEDTABLE() of unknown table `{name}`"));
                };
                let fc = self.transitioned(s.fc, s.rc)?;
                let rows = self.visible_rows(&fc, t)?;
                Tab::rows_of(t, rows)
            }
            _ => return Ok(None),
        }))
    }

    pub(super) fn more_function(&mut self, up: &str, args: &[Option<Expr>], s: &Scope) -> Result<V, Fail> {
        match up {
            "LOOKUPVALUE" => self.lookupvalue(args, s),
            // an evaluation error with the given text (caught by IFERROR and ISERROR)
            "ERROR" => {
                let text = super::text_of(&self.scalar(need(args, 0, up)?, s)?);
                dax_error(text)
            }
            "CONTAINS" => {
                let t = self.table(need(args, 0, up)?, s)?;
                let cols = self.tab_cols(&t);
                let mut want: Vec<(usize, V)> = Vec::new();
                let mut i = 1usize;
                while i < args.len() {
                    let c = self.col_arg(need(args, i, up)?, s.rc, up)?;
                    let Some(pos) = cols.iter().position(|x| x.lin == Some(c)) else {
                        return unsupported("CONTAINS() of a column that is not in the table");
                    };
                    let v = self.scalar(need(args, i.saturating_add(1), up)?, s)?;
                    want.push((pos, v));
                    i = i.saturating_add(2);
                }
                let rows = self.tab_values(&t)?;
                for r in rows.iter() {
                    let mut all = true;
                    for (pos, v) in &want {
                        let cell = r.get(*pos).cloned().unwrap_or(V::Blank);
                        if !strictly_equal(&cell, v).unwrap_or(false) {
                            all = false;
                            break;
                        }
                    }
                    if all {
                        return Ok(V::Bool(true));
                    }
                }
                Ok(V::Bool(false))
            }
            "CONTAINSROW" => {
                let t = self.table(need(args, 0, up)?, s)?;
                let mut want = Vec::new();
                for i in 1..args.len() {
                    want.push(self.scalar(need(args, i, up)?, s)?);
                }
                let rows = self.tab_values(&t)?;
                for r in rows.iter() {
                    if r.len() != want.len() {
                        return dax_error("CONTAINSROW(): the table and the values have different widths");
                    }
                    if r.iter()
                        .zip(&want)
                        .all(|(a, b)| strictly_equal(a, b).unwrap_or(false))
                    {
                        return Ok(V::Bool(true));
                    }
                }
                Ok(V::Bool(false))
            }
            "HASONEFILTER" => {
                let c = self.col_arg(need(args, 0, up)?, s.rc, up)?;
                if !s.fc.filters.iter().any(|f| f.cols.contains(&c)) {
                    return Ok(V::Bool(false));
                }
                let sub = Self::direct_filters(s.fc, c);
                Ok(V::Bool(self.values_codes(&sub, c, true)?.len() == 1))
            }
            "ISCROSSFILTERED" => {
                let t = match need(args, 0, up)? {
                    Expr::Name { name, .. } => match self.table_id(name) {
                        Some(t) => t,
                        None => return unsupported(format!("ISCROSSFILTERED() of unknown table `{name}`")),
                    },
                    other => {
                        let c = self.col_arg(other, s.rc, up)?;
                        self.col_table(c)?
                    }
                };
                Ok(V::Bool(self.crossfiltered(s.fc, t)))
            }
            "FIRSTNONBLANKVALUE" | "LASTNONBLANKVALUE" => {
                let f = if up.starts_with("FIRST") {
                    "FIRSTNONBLANK"
                } else {
                    "LASTNONBLANK"
                };
                Ok(self.non_blank(f, args, s)?.map_or(V::Blank, |(_, _, r)| r))
            }
            "MAXA" | "MINA" => {
                let xs = self.numbers(up, args, s, false)?;
                let best = xs.iter().copied().fold(None, |acc: Option<f64>, x| {
                    Some(match acc {
                        None => x,
                        Some(a) if up == "MAXA" => a.max(x),
                        Some(a) => a.min(x),
                    })
                });
                Ok(best.map_or(V::Blank, V::Num))
            }
            "RANK.EQ" => {
                let v = num(&self.scalar(need(args, 0, up)?, s)?)?;
                let c = self.col_arg(need(args, 1, up)?, s.rc, up)?;
                let asc = match arg(args, 2) {
                    Some(e) => match super::keyword(e).as_deref() {
                        Some("ASC") => true,
                        Some("DESC") => false,
                        _ => num(&self.scalar(e, s)?)?.unwrap_or(0.0) != 0.0,
                    },
                    None => false,
                };
                let Some(v) = v else {
                    return Ok(V::Blank);
                };
                let mut xs = Vec::new();
                for x in self.column_values(c, s.fc)? {
                    if let Some(x) = num(&x)? {
                        xs.push(x);
                    }
                }
                if !xs.contains(&v) {
                    return dax_error("RANK.EQ(): the value is not in the column");
                }
                let better = xs.iter().filter(|x| if asc { **x < v } else { **x > v }).count();
                Ok(V::Num(better.saturating_add(1) as f64))
            }
            _ => {
                let iterator = matches!(
                    up,
                    "PRODUCTX"
                        | "GEOMEANX"
                        | "MEDIANX"
                        | "PERCENTILEX.INC"
                        | "PERCENTILEX.EXC"
                        | "STDEVX.S"
                        | "STDEVX.P"
                        | "VARX.S"
                        | "VARX.P"
                );
                let mut xs = self.numbers(up, args, s, iterator)?;
                let n = xs.len() as f64;
                match up {
                    "PRODUCT" | "PRODUCTX" => Ok(if xs.is_empty() {
                        V::Blank
                    } else {
                        V::Num(xs.iter().product())
                    }),
                    "GEOMEAN" | "GEOMEANX" => {
                        if xs.is_empty() {
                            return Ok(V::Blank);
                        }
                        if xs.iter().any(|x| *x <= 0.0) {
                            return dax_error(format!("{up}() of a number that is not positive"));
                        }
                        Ok(V::Num((xs.iter().map(|x| x.ln()).sum::<f64>() / n).exp()))
                    }
                    "AVERAGEA" => Ok(if xs.is_empty() {
                        V::Blank
                    } else {
                        V::Num(xs.iter().sum::<f64>() / n)
                    }),
                    "MEDIANX" | "PERCENTILE.INC" | "PERCENTILE.EXC" | "PERCENTILEX.INC"
                    | "PERCENTILEX.EXC" => {
                        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
                        if xs.is_empty() {
                            return Ok(V::Blank);
                        }
                        let (k, exclusive) = if up == "MEDIANX" {
                            (0.5, false)
                        } else {
                            let ki = if iterator { 2 } else { 1 };
                            (
                                num(&self.scalar(need(args, ki, up)?, s)?)?.unwrap_or(0.0),
                                up.ends_with("EXC"),
                            )
                        };
                        match percentile(&xs, k, exclusive) {
                            Some(x) => Ok(V::Num(x)),
                            None => dax_error(format!("{up}(): the percentile is out of range")),
                        }
                    }
                    "STDEVX.S" | "STDEVX.P" | "VARX.S" | "VARX.P" => {
                        let population = up.ends_with('P');
                        if xs.len() < if population { 1 } else { 2 } {
                            return Ok(V::Blank);
                        }
                        let mean = xs.iter().sum::<f64>() / n;
                        let ss: f64 = xs.iter().map(|x| (x - mean) * (x - mean)).sum();
                        let var = ss / if population { n } else { n - 1.0 };
                        Ok(V::Num(if up.starts_with("STDEV") { var.sqrt() } else { var }))
                    }
                    _ => unsupported(format!("{up}() is not evaluated by the preview")),
                }
            }
        }
    }

    fn lookupvalue(&mut self, args: &[Option<Expr>], s: &Scope) -> Result<V, Fail> {
        let f = "LOOKUPVALUE";
        let rc = self.col_arg(need(args, 0, f)?, s.rc, f)?;
        let t = self.col_table(rc)?;
        let pairs = args.len().saturating_sub(1) / 2;
        let mut search: Vec<(ColId, V)> = Vec::with_capacity(pairs);
        for k in 0..pairs {
            let c = self.col_arg(need(args, 1 + 2 * k, f)?, s.rc, f)?;
            if self.col_table(c)? != t {
                return unsupported("LOOKUPVALUE() with a search column of another table");
            }
            let v = self.scalar(need(args, 2 + 2 * k, f)?, s)?;
            search.push((c, v));
        }
        let alternate = args
            .len()
            .is_multiple_of(2)
            .then(|| arg(args, args.len().saturating_sub(1)))
            .flatten();
        let n = self.rows(t)?;
        self.tick(n.saturating_mul(search.len().max(1)))?;
        let datas = search
            .iter()
            .map(|(c, _)| self.col(*c))
            .collect::<Result<Vec<_>, _>>()?;
        let result = self.col(rc)?;
        let mut found: Option<u32> = None;
        let mut several = false;
        for row in 0..n {
            let mut all = true;
            for (d, (_, v)) in datas.iter().zip(&search) {
                let code = d.codes.get(row).unwrap_or(0);
                let cell = d.values.get(code as usize).map_or(V::Blank, V::of_cell);
                if !strictly_equal(&cell, v)? {
                    all = false;
                    break;
                }
            }
            if !all {
                continue;
            }
            let code = result.codes.get(row).unwrap_or(0);
            match found {
                None => found = Some(code),
                Some(f0) if f0 != code => several = true,
                Some(_) => {}
            }
        }
        match (found, several, alternate) {
            (Some(code), false, _) => self.value_of_code(rc, code),
            (None, _, Some(a)) | (Some(_), true, Some(a)) => self.scalar(a, s),
            (None, _, None) => Ok(V::Blank),
            (Some(_), true, None) => {
                dax_error("a table of multiple values was supplied where a single value was expected")
            }
        }
    }
}
