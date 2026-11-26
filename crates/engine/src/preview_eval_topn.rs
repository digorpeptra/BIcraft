//! `TOPN` selection helpers of the preview evaluator ([`super`]).
//!
//! DAX semantics kept (the reference is the generic selection of `top_n_table`):
//! - the order is DAX's sort order per key (blank first, then numbers and dates, text, booleans),
//!   each key ascending or descending;
//! - every row that ties with the `n`-th on all keys is kept, so the result can have more than
//!   `n` rows;
//! - the kept rows come in that order, ties by position.
//!
//! The first key is read once into a typed array when its values allow it (numbers and dates,
//! Currency, booleans, with blanks): its `n`-th value bounds the rows that can be kept (a row
//! after it on the first key is after the `n`-th row on every key), so only those few candidates
//! are compared on every key.

use std::cell::OnceCell;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::rc::Rc;
use std::sync::Arc;

use bicraft_dax::ast::Expr;

use super::super::Fail;
use super::super::store::ColData;
use super::tables::{pairs, sort_cmp};
use super::{Bind, ColId, Combos, Engine, Env, FCtx, MeasureDef, Scope, TCol, Tab, V, arg, num};

/// One order key of every row, read once.
pub(super) enum TopKey {
    /// (sort rank, exact value): Currency units, or booleans.
    Int(Vec<(u8, i64)>),
    /// (sort rank, value): numbers and dates (and Currency mixed with them, compared as
    /// numbers as the generic comparison does).
    Float(Vec<(u8, f64)>),
}

impl TopKey {
    /// The key of rows `0..len`, `None` when a value is text, NaN, or booleans mix with numbers.
    /// Long keys are read by shares on several threads.
    pub(super) fn read<'v>(len: usize, at: impl Fn(usize) -> &'v V + Sync) -> Option<TopKey> {
        let shares = par_shares(len);
        let chunk = len.div_ceil(shares).max(1);
        let rows = |i: usize| {
            let lo = i.saturating_mul(chunk).min(len);
            lo..lo.saturating_add(chunk).min(len)
        };
        // (Currency, numbers, booleans) of each share, `None` for text or NaN
        let kinds = on_shares(shares, &|s| {
            let (mut cur, mut numeric, mut boolean) = (false, false, false);
            for i in rows(s) {
                match at(i) {
                    V::Blank => {}
                    V::Cur(_) => cur = true,
                    V::Num(x) | V::Date(x) => {
                        if x.is_nan() {
                            return None;
                        }
                        numeric = true;
                    }
                    V::Bool(_) => boolean = true,
                    V::Text(_) => return None,
                }
            }
            Some((cur, numeric, boolean))
        });
        let (mut cur, mut numeric, mut boolean) = (false, false, false);
        for k in kinds {
            let (c, n, b) = k?;
            (cur, numeric, boolean) = (cur || c, numeric || n, boolean || b);
        }
        if boolean && (cur || numeric) {
            return None;
        }
        if numeric {
            let parts = on_shares(shares, &|s| {
                rows(s)
                    .map(|i| match at(i) {
                        V::Blank => Some((0, 0.0)),
                        x => Some((1, num(x).ok().flatten()?)),
                    })
                    .collect::<Option<Vec<_>>>()
            });
            let mut v = Vec::with_capacity(len);
            for p in parts {
                v.extend(p?);
            }
            return Some(TopKey::Float(v));
        }
        let parts = on_shares(shares, &|s| {
            rows(s)
                .map(|i| match at(i) {
                    V::Cur(n) => (1, *n),
                    V::Bool(b) => (3, i64::from(*b)),
                    _ => (0, 0),
                })
                .collect::<Vec<_>>()
        });
        let mut v = Vec::with_capacity(len);
        for p in parts {
            v.extend(p);
        }
        Some(TopKey::Int(v))
    }

    fn len(&self) -> usize {
        match self {
            TopKey::Int(v) => v.len(),
            TopKey::Float(v) => v.len(),
        }
    }

    /// Rows `a` and `b` in ascending order of this key.
    pub(super) fn cmp_rows(&self, a: usize, b: usize) -> Ordering {
        match self {
            TopKey::Int(v) => match (v.get(a), v.get(b)) {
                (Some(x), Some(y)) => x.cmp(y),
                _ => Ordering::Equal,
            },
            TopKey::Float(v) => match (v.get(a), v.get(b)) {
                (Some(x), Some(y)) => cmp_f(x, y),
                _ => Ordering::Equal,
            },
        }
    }

    /// The rows not after the `take`-th on this key (in its direction): a superset of `TOPN`'s
    /// rows, in position order. Every row when `take` is 0 or reaches the row count.
    pub(super) fn candidates(&self, take: usize, desc: bool) -> Vec<usize> {
        match self {
            TopKey::Int(v) => bound(v, take, desc, |x, y| x.cmp(y)),
            TopKey::Float(v) => bound(v, take, desc, cmp_f),
        }
        .unwrap_or_else(|| (0..self.len()).collect())
    }
}

fn cmp_f(x: &(u8, f64), y: &(u8, f64)) -> Ordering {
    x.0.cmp(&y.0)
        .then(x.1.partial_cmp(&y.1).unwrap_or(Ordering::Equal))
}

/// The fewest rows of a key a thread takes when it is read or bounded on several.
const PAR_ROWS: usize = 1 << 17;

/// The shares a key of `len` rows is read in: one per thread, of at least [`PAR_ROWS`] rows.
fn par_shares(len: usize) -> usize {
    bicraft_scan::configured_threads().min(len / PAR_ROWS).max(1)
}

/// The positions of `v` not after its `take`-th value in ascending (or descending) order.
fn bound<T: Copy + Send + Sync>(
    v: &[T],
    take: usize,
    desc: bool,
    cmp: impl Fn(&T, &T) -> Ordering + Sync,
) -> Option<Vec<usize>> {
    bound_with(v, take, desc, &cmp, par_shares(v.len()))
}

/// [`bound`] on `threads` threads: each keeps the `take` best values of its share in a buffer of
/// at most `2 * take` whose worst kept value rejects the others with one comparison (a top-k with
/// a threshold), the `take`-th of what they kept is the bound, then each share lists its
/// positions not after it, in position order.
pub(super) fn bound_with<T: Copy + Send + Sync>(
    v: &[T],
    take: usize,
    desc: bool,
    cmp: &(impl Fn(&T, &T) -> Ordering + Sync),
    threads: usize,
) -> Option<Vec<usize>> {
    let at = take.checked_sub(1)?;
    if at >= v.len().saturating_sub(1) {
        return None;
    }
    let ord = |x: &T, y: &T| if desc { cmp(y, x) } else { cmp(x, y) };
    let best = |part: &[T]| -> Vec<T> {
        let cap = take.saturating_mul(2).min(part.len()).max(1);
        let mut buf: Vec<T> = Vec::with_capacity(cap);
        let mut worst: Option<T> = None;
        for x in part {
            if worst.as_ref().is_some_and(|w| ord(x, w) != Ordering::Less) {
                continue;
            }
            buf.push(*x);
            if buf.len() >= cap && buf.len() > take {
                buf.select_nth_unstable_by(at, ord);
                buf.truncate(take);
                worst = buf.get(at).copied();
            }
        }
        if buf.len() > take {
            buf.select_nth_unstable_by(at, ord);
            buf.truncate(take);
        }
        buf
    };
    let chunk = v.len().div_ceil(threads.max(1)).max(1);
    let parts: Vec<&[T]> = v.chunks(chunk).collect();
    let part = |i: usize| parts.get(i).copied().unwrap_or(&[]);
    let mut kept: Vec<T> = on_shares(parts.len(), &|i| best(part(i)))
        .into_iter()
        .flatten()
        .collect();
    if kept.len() <= at {
        return None;
    }
    kept.select_nth_unstable_by(at, ord);
    let thr = *kept.get(at)?;
    drop(kept);
    let out = on_shares(parts.len(), &|i| {
        let base = i.saturating_mul(chunk);
        part(i)
            .iter()
            .enumerate()
            .filter(|(_, x)| ord(x, &thr) != Ordering::Greater)
            .map(|(j, _)| base.saturating_add(j))
            .collect::<Vec<_>>()
    });
    Some(out.into_iter().flatten().collect())
}

/// `f` of every share `0..shares`, one thread each when there are several; a worker that fails
/// has its share computed again on this thread.
fn on_shares<R: Send>(shares: usize, f: &(dyn Fn(usize) -> R + Sync)) -> Vec<R> {
    if shares < 2 {
        return (0..shares).map(f).collect();
    }
    let got: Vec<Option<R>> = std::thread::scope(|sc| {
        let hs: Vec<_> = (0..shares).map(|i| sc.spawn(move || f(i))).collect();
        hs.into_iter().map(|h| h.join().ok()).collect()
    });
    got.into_iter()
        .enumerate()
        .map(|(i, g)| g.unwrap_or_else(|| f(i)))
        .collect()
}

/// `TOPN`'s rows among `cands` (positions, in position order) for the order `cmp`: the `take`
/// first, with every row that ties with the last of them, sorted (ties by position).
pub(super) fn top_rows(
    mut cands: Vec<usize>,
    take: usize,
    cmp: &dyn Fn(usize, usize) -> Ordering,
) -> Vec<usize> {
    let Some(at) = take.checked_sub(1) else {
        return Vec::new();
    };
    if cands.is_empty() {
        return Vec::new();
    }
    let at = at.min(cands.len().saturating_sub(1));
    cands.select_nth_unstable_by(at, |a, b| cmp(*a, *b).then(a.cmp(b)));
    let Some(&last) = cands.get(at) else {
        return Vec::new();
    };
    cands.retain(|i| cmp(*i, last) != Ordering::Greater);
    cands.sort_unstable_by(|a, b| cmp(*a, *b).then(a.cmp(b)));
    cands
}

impl Engine<'_, '_> {
    /// `TOPN(n, SUMMARIZECOLUMNS(…), keys…)` without building every group's row, when the
    /// grouping has only group-by columns, filter tables and items that are one aggregation each
    /// (computed by grouped scans), and every key is one of its items (`[name]`) or group-by
    /// columns (`T[c]`). The grouping may come through a variable defined by the query and not
    /// evaluated yet (Desktop's `VAR __DS0Core = SUMMARIZECOLUMNS(…)`). `None`: another shape, or
    /// the grouping is already kept (the generic path reads it).
    pub(super) fn top_n_grouping(
        &mut self,
        n: f64,
        source: &Expr,
        keys_of: &[(&Expr, bool)],
        s: &Scope,
        exact: bool,
    ) -> Result<Option<(Tab, usize)>, Fail> {
        if self.store.is_none()
            || keys_of.is_empty()
            || n.partial_cmp(&1.0).is_none_or(|o| o == Ordering::Less)
        {
            return Ok(None);
        }
        match source {
            Expr::Call { name, args, .. } if name.eq_ignore_ascii_case("SUMMARIZECOLUMNS") => {
                self.grouping_top(n, args, keys_of, s, exact)
            }
            Expr::Name { name, .. } => {
                let lower = name.to_lowercase();
                let Some((_, Bind::L(l))) = s.env.iter().rev().find(|(x, _)| *x == lower) else {
                    return Ok(None);
                };
                let l = Rc::clone(l);
                let Expr::Call { name, args, .. } = &l.expr else {
                    return Ok(None);
                };
                if l.cell.get().is_some() || !name.eq_ignore_ascii_case("SUMMARIZECOLUMNS") {
                    return Ok(None);
                }
                // evaluated where the variable is defined: the query's top level
                let fc = FCtx::default();
                let scope = Scope {
                    fc: &fc,
                    rc: &[],
                    env: &l.env,
                    depth: 0,
                    mdepth: 0,
                };
                let saved = self.take_dynamic();
                let r = self.grouping_top(n, args, keys_of, &scope, exact);
                self.restore_dynamic(saved);
                r
            }
            _ => Ok(None),
        }
    }

    fn grouping_top(
        &mut self,
        n: f64,
        args: &[Option<Expr>],
        keys_of: &[(&Expr, bool)],
        s: &Scope,
        exact: bool,
    ) -> Result<Option<(Tab, usize)>, Fail> {
        let f = "SUMMARIZECOLUMNS";
        if let Some(k) = self.table_cache_key(f, args, s)?
            && self.cached_table(&k)?.is_some()
        {
            return Ok(None);
        }
        // the shape: group-by columns, filter tables, then names and aggregations
        let mut groups: Vec<ColId> = Vec::new();
        let mut filters: Vec<Option<Expr>> = vec![None];
        let mut i = 0usize;
        while let Some(a) = arg(args, i) {
            match a {
                Expr::Str { .. } => break,
                Expr::Column { table, name, .. } if self.measure_def(table.as_deref(), name).is_none() => {
                    let Ok(c) = self.col_arg(a, &[], f) else {
                        return Ok(None);
                    };
                    if groups.contains(&c) {
                        return Ok(None);
                    }
                    groups.push(c);
                }
                other if !is_grouping_modifier(other) && self.is_table_expr(other, s.env) => {
                    filters.push(Some(other.clone()));
                }
                _ => return Ok(None),
            }
            i = i.saturating_add(1);
        }
        let Ok(items) = pairs(args, i, f) else {
            return Ok(None);
        };
        if items.is_empty() || groups.is_empty() {
            return Ok(None);
        }
        // every item one aggregation (`IGNORE` noted: it does not decide which rows stay)
        let mut direct = Vec::with_capacity(items.len());
        let mut ignored = Vec::with_capacity(items.len());
        for (_, e) in &items {
            let (e, ig) = match e {
                Expr::Call { name, args: ia, .. } if name.eq_ignore_ascii_case("IGNORE") => {
                    match arg(ia, 0) {
                        Some(x) => (x, true),
                        None => return Ok(None),
                    }
                }
                other => (*other, false),
            };
            let Some(d) = self.direct_agg(e, s.env, 0) else {
                return Ok(None);
            };
            direct.push(d);
            ignored.push(ig);
        }
        let mut cols: Vec<TCol> = groups.iter().map(|c| TCol::of(*c)).collect();
        for (nm, e) in &items {
            cols.push(TCol::named(nm.clone(), self.static_type(e, 0)));
        }
        if self.check_names(&cols, f).is_err() {
            return Ok(None);
        }
        // the keys: items by name, or group-by columns
        let mut keys: Vec<(KeyAt, bool)> = Vec::with_capacity(keys_of.len());
        for (e, desc) in keys_of {
            let Expr::Column {
                table,
                name,
                variation: None,
                ..
            } = e
            else {
                return Ok(None);
            };
            let at = match table {
                None => {
                    // `[name]` reads the first column of that name; group-by columns come first
                    let pos = cols
                        .iter()
                        .position(|c| self.tcol_name(c).eq_ignore_ascii_case(name));
                    match pos.and_then(|p| p.checked_sub(groups.len())) {
                        Some(ii) => KeyAt::Item(ii),
                        None => return Ok(None),
                    }
                }
                Some(tn) => {
                    if self.measure_def(Some(tn), name).is_some() {
                        return Ok(None);
                    }
                    let Some(c) = self.col_id(tn, name) else {
                        return Ok(None);
                    };
                    match groups.iter().position(|g| *g == c) {
                        Some(gi) => KeyAt::Group(gi),
                        None => return Ok(None),
                    }
                }
            };
            keys.push((at, *desc));
        }
        let scope = Scope { rc: &[], ..*s };
        let fc = if filters.len() > 1 {
            self.calc_ctx(&filters, &scope)?
        } else {
            s.fc.child()
        };
        // the groups SUMMARIZECOLUMNS keeps: some item not under IGNORE is not blank (all, when
        // every item is under IGNORE)
        let counted: Vec<usize> = (0..direct.len())
            .filter(|ii| !ignored.get(*ii).copied().unwrap_or(false))
            .collect();
        // the groups and one value per group and item, as SUMMARIZECOLUMNS computes them
        let saved = std::mem::replace(&mut self.group_cols, groups.clone());
        let saved_outer = self
            .shadow
            .then(|| std::mem::replace(&mut self.outer, fc.child()));
        let computed = (|| -> Result<Option<Computed>, Fail> {
            let combos = self.combos(&fc, &groups)?;
            self.prefetch_sums(&direct, &fc, &groups)?;
            let nc = combos.len();
            // the values, the kept positions and the first key of every group
            let per = std::mem::size_of::<V>()
                .saturating_mul(direct.len())
                .saturating_add(4 + 16);
            let pick = Pick {
                n,
                direct: &direct,
                counted: &counted,
                groups: &groups,
                keys: &keys,
                width: cols.len(),
                exact,
            };
            // an exact cut keeps only its rows: block by block under a cap, whatever the room
            if self.mem.on() && (exact || nc.saturating_mul(per) > self.mem.room()) {
                // over what the query's cap leaves: block by block, keeping only candidates
                return Ok(self.grouping_top_blocks(&pick, &combos, &fc)?.map(Computed::Done));
            }
            let nv = direct.len();
            let refused = self.mem.refused;
            let whole = (|| -> Result<Option<Charged>, Fail> {
                let charge = self.charge_bytes(
                    &move || {
                        format!("the {nv} value columns of a grouping of {nc} combinations under TOPN()")
                    },
                    nc.saturating_mul(per),
                )?;
                let mut vectors = Vec::with_capacity(direct.len());
                for (agg, c) in &direct {
                    match self.vector_agg(*agg, *c, &fc, &groups, &combos)? {
                        Some(v) => vectors.push(v),
                        None => return Ok(None),
                    }
                }
                Ok(Some((vectors, charge)))
            })();
            match whole {
                Ok(Some((vectors, charge))) => Ok(Some(Computed::Whole(combos, vectors, charge))),
                Ok(None) => Ok(None),
                // the values fit, not with the grouped scans that compute them: block by block
                Err(_) if self.mem.refused > refused => {
                    self.mem.refused = refused;
                    Ok(self.grouping_top_blocks(&pick, &combos, &fc)?.map(Computed::Done))
                }
                Err(e) => Err(e),
            }
        })();
        self.group_cols = saved;
        if let Some(o) = saved_outer {
            self.outer = o;
        }
        let (combos, vectors, _charge) = match computed? {
            None => return Ok(None),
            Some(Computed::Done((rows, len))) => {
                #[cfg(test)]
                GROUPING_TOPS.with(|c| c.set(c.get().saturating_add(1)));
                return Ok(Some((Tab::tuples(cols, rows), len)));
            }
            Some(Computed::Whole(c, v, ch)) => (c, v, ch),
        };
        let kept: Vec<u32> = (0..combos.len())
            .filter(|ci| {
                counted.is_empty()
                    || counted.iter().any(|ii| {
                        vectors
                            .get(*ii)
                            .and_then(|v| v.get(*ci))
                            .is_some_and(|v| !v.is_blank())
                    })
            })
            .filter_map(|ci| u32::try_from(ci).ok())
            .collect();
        let len = kept.len();
        let take = if n >= len as f64 { len } else { n as usize };
        let gdata = groups
            .iter()
            .map(|c| self.col(*c))
            .collect::<Result<Vec<_>, _>>()?;
        let group_value = |gi: usize, ci: usize| -> V {
            match (gdata.get(gi), combos.get(ci).and_then(|cb| cb.get(gi))) {
                (Some(d), Some(code)) => d.values.get(*code as usize).map_or(V::Blank, V::of_cell),
                _ => V::Blank,
            }
        };
        let item_value =
            |ii: usize, ci: usize| -> &V { vectors.get(ii).and_then(|v| v.get(ci)).unwrap_or(&V::Blank) };
        let kept_at = |p: usize| kept.get(p).map_or(usize::MAX, |ci| *ci as usize);
        // the first key once for every kept group: its `take`-th value bounds the candidates
        let first = keys.first().copied();
        let k0 = match first {
            Some((KeyAt::Item(ii), _)) => TopKey::read(len, |p| item_value(ii, kept_at(p))),
            Some((KeyAt::Group(gi), _)) => {
                let vals: Vec<V> = (0..len).map(|p| group_value(gi, kept_at(p))).collect();
                TopKey::read(len, |p| vals.get(p).unwrap_or(&V::Blank))
            }
            None => None,
        };
        let cands: Vec<usize> = match (&k0, first) {
            (Some(k), Some((_, desc))) => k.candidates(take, desc),
            _ => (0..len).collect(),
        };
        // every key of the candidates, then the exact selection among them
        let nk = cands.len();
        let _mini_charge = self.charge_bytes(
            &move || format!("the sort keys of {nk} TOPN() candidates"),
            nk.saturating_mul(super::mem::row_bytes(keys.len())),
        )?;
        let mut mini: Vec<Vec<V>> = Vec::with_capacity(cands.len());
        for p in &cands {
            let ci = kept_at(*p);
            mini.push(
                keys.iter()
                    .map(|(at, _)| match at {
                        KeyAt::Item(ii) => item_value(*ii, ci).clone(),
                        KeyAt::Group(gi) => group_value(*gi, ci),
                    })
                    .collect(),
            );
        }
        let blank = V::Blank;
        let cmp = |a: usize, b: usize| -> Ordering {
            for (k, (_, desc)) in keys.iter().enumerate() {
                let o = match (&k0, k) {
                    (Some(k0), 0) => k0.cmp_rows(
                        cands.get(a).copied().unwrap_or(usize::MAX),
                        cands.get(b).copied().unwrap_or(usize::MAX),
                    ),
                    _ => {
                        let x = mini.get(a).and_then(|r| r.get(k)).unwrap_or(&blank);
                        let y = mini.get(b).and_then(|r| r.get(k)).unwrap_or(&blank);
                        sort_cmp(x, y)
                    }
                };
                let o = if *desc { o.reverse() } else { o };
                if o != Ordering::Equal {
                    return o;
                }
            }
            Ordering::Equal
        };
        let picked = top_rows((0..cands.len()).collect(), take, &cmp);
        // only the kept rows are built
        let mut rows = Vec::with_capacity(picked.len());
        for j in picked {
            let ci = cands.get(j).map_or(usize::MAX, |p| kept_at(*p));
            let mut row = Vec::with_capacity(cols.len());
            for gi in 0..groups.len() {
                row.push(group_value(gi, ci));
            }
            for ii in 0..direct.len() {
                row.push(item_value(ii, ci).clone());
            }
            rows.push(row);
        }
        #[cfg(test)]
        GROUPING_TOPS.with(|c| c.set(c.get().saturating_add(1)));
        Ok(Some((Tab::tuples(cols, rows), len)))
    }

    /// [`Engine::grouping_top`] when the values of every group would not fit in what the query's
    /// memory cap leaves: the groups go by blocks, each block's values read from the grouped
    /// scans (computed once, held for the whole loop), and only the candidates are kept: the
    /// groups not after the worst of the best `take` seen so far on every key (a bounded heap;
    /// with `exact`, the position breaks ties, so exactly `take` rows come out). That bound only
    /// gets stricter, so a group it rules out is never in the result; the candidates are pruned as
    /// they grow. The final selection is the whole path's (every key, ties by position), so the
    /// rows are the same. Returns the rows and the number of kept groups.
    ///
    /// When the grouped scans' states for every group do not fit either, the groups are scanned a
    /// range at a time ([`Engine::grouping_top_ranges`]) and offered to the same heap.
    fn grouping_top_blocks(
        &mut self,
        pick: &Pick<'_>,
        combos: &Combos,
        fc: &FCtx,
    ) -> Result<Option<Picked>, Fail> {
        let nc = combos.len();
        let ng = pick.groups.len();
        if pick.keys.is_empty() {
            return Ok(None);
        }
        let refused = self.mem.refused;
        let mut plans = Vec::with_capacity(pick.direct.len());
        let mut over: Option<Fail> = None;
        // tests: by ranges whatever the room
        #[cfg(test)]
        if TEST_RANGE_LEN.with(std::cell::Cell::get).is_some() {
            over = Some(Fail::Unsupported("ranges forced by a test".into()));
        }
        for (agg, c) in pick
            .direct
            .iter()
            .take(if over.is_some() { 0 } else { usize::MAX })
        {
            match self.vector_plan(*agg, *c, fc, pick.groups) {
                Ok(Some(p)) => plans.push(p),
                Ok(None) => return Ok(None),
                // the states of every group do not fit: by ranges of groups
                Err(e) if self.mem.on() && self.mem.refused > refused => {
                    over = Some(e);
                    break;
                }
                Err(e) => return Err(e),
            }
        }
        let gdata = pick
            .groups
            .iter()
            .map(|c| self.col(*c))
            .collect::<Result<Vec<_>, _>>()?;
        let take = if pick.n >= nc as f64 { nc } else { pick.n as usize };
        let mut sel = TopSel::new(self, pick, take)?;
        if let Some(e) = over {
            drop(plans);
            let counted = self.mem.refused;
            self.mem.refused = refused;
            if !self.grouping_top_ranges(pick, combos, fc, &gdata, &mut sel)? {
                self.mem.refused = counted;
                return Err(e);
            }
            return Ok(Some(sel.finish()));
        }
        let per_group = std::mem::size_of::<V>().saturating_mul(plans.len().max(1));
        let room = self.mem.room();
        let block = (room / 4 / per_group.max(1)).clamp(1024, 1 << 20);
        let mut start = 0usize;
        while start < nc {
            let count = block.min(nc.saturating_sub(start));
            let _block_charge = self.charge_bytes(
                &move || format!("a block of {count} groups of TOPN() over a grouping"),
                count.saturating_mul(per_group),
            )?;
            let mut vectors: Vec<Vec<V>> = Vec::with_capacity(plans.len());
            for plan in &plans {
                vectors.push(self.vector_fill(plan, count, |i, pos, out| {
                    let Some(combo) = combos.get(start.saturating_add(i)) else {
                        return false;
                    };
                    for (o, p) in out.iter_mut().zip(pos) {
                        match combo.get(*p) {
                            Some(x) => *o = *x,
                            None => return false,
                        }
                    }
                    true
                })?);
            }
            for i in 0..count {
                let keep = pick.counted.is_empty()
                    || pick.counted.iter().any(|ii| {
                        vectors
                            .get(*ii)
                            .and_then(|v| v.get(i))
                            .is_some_and(|v| !v.is_blank())
                    });
                if !keep {
                    continue;
                }
                let ci = start.saturating_add(i);
                let value_at = |col: usize| -> V {
                    if col < ng {
                        group_value(&gdata, combos, col, ci)
                    } else {
                        vectors
                            .get(col.saturating_sub(ng))
                            .and_then(|v| v.get(i))
                            .cloned()
                            .unwrap_or(V::Blank)
                    }
                };
                sel.offer(self, ci, &value_at)?;
            }
            start = start.saturating_add(count);
        }
        Ok(Some(sel.finish()))
    }

    /// [`Engine::grouping_top_blocks`] when the grouped scans' states for every group do not fit
    /// in what the query may use (`docs/beyond-ram.md`, "Groupings by ranges"): the group numbers
    /// are cut into ranges whose states (every item's, plus one scan's working memory) fit in half
    /// of it; per range, one grouped scan per item restricted to the range's groups, then every
    /// combination whose group falls in the range is offered to the heap, by its position. Each
    /// group's value is the one the whole scan gives (`Engine::scan_entry_in`), every group is in
    /// exactly one range, and the selection does not depend on the order of the offers, so the rows
    /// are the same. `false`: the items are not keyed alike (or a scan would take the reference
    /// path): the caller reports the refusal.
    fn grouping_top_ranges(
        &mut self,
        pick: &Pick<'_>,
        combos: &Combos,
        fc: &FCtx,
        gdata: &[Arc<ColData>],
        sel: &mut TopSel,
    ) -> Result<bool, Fail> {
        let ng = pick.groups.len();
        let mut plans = Vec::with_capacity(pick.direct.len());
        for (agg, c) in pick.direct {
            match self.range_plan(*agg, *c, fc, pick.groups)? {
                Some(p) => plans.push(p),
                None => return Ok(false),
            }
        }
        let Some(first) = plans.first() else {
            return Ok(false);
        };
        if plans.iter().any(|p| p.pos != first.pos || p.cards != first.cards) {
            return Ok(false);
        }
        let groups = first.groups();
        // per group of a range: every item's state, and one scan's (twice, while it is joined)
        let widest = plans.iter().map(|p| p.group_bytes).max().unwrap_or(0);
        let per = plans
            .iter()
            .fold(0usize, |a, p| a.saturating_add(p.group_bytes))
            .saturating_add(widest.saturating_mul(2));
        // (the candidates the heap may keep are left their room)
        let room = self.mem.room().saturating_sub(sel.cand_room());
        let len = range_len(room, per, groups);
        #[cfg(test)]
        let len = TEST_RANGE_LEN
            .with(std::cell::Cell::get)
            .map_or(len, |l| l.clamp(1, groups.max(1)));
        let nc = combos.len();
        let mut lo = 0usize;
        let mut len = len;
        while lo < groups {
            // the range's states; a range that does not fit is halved, down to one group
            let mut held = Vec::with_capacity(plans.len());
            let refused = self.mem.refused;
            for p in &plans {
                match self.range_entry(p, lo, len) {
                    Ok(Some(e)) => held.push(e),
                    Ok(None) => return Ok(false),
                    Err(_) if len > 1 && self.mem.refused > refused => break,
                    Err(e) => return Err(e),
                }
            }
            if held.len() < plans.len() {
                drop(held);
                self.mem.refused = refused;
                len = len.div_ceil(2);
                continue;
            }
            #[cfg(test)]
            GROUPING_RANGES.with(|c| c.set(c.get().saturating_add(1)));
            self.tick(nc)?;
            for ci in 0..nc {
                let Some(combo) = combos.get(ci) else {
                    break;
                };
                // a combination of codes no column holds: every value blank, in the first range
                let g = first.group(combo);
                let inside = match g {
                    Some(g) => g.checked_sub(lo).is_some_and(|x| x < len),
                    None => lo == 0,
                };
                if !inside {
                    continue;
                }
                let mut items = Vec::with_capacity(plans.len());
                for (p, e) in plans.iter().zip(&held) {
                    items.push(match g {
                        Some(g) => self.range_value(p, e, g)?,
                        None => V::Blank,
                    });
                }
                let keep = pick.counted.is_empty()
                    || pick
                        .counted
                        .iter()
                        .any(|ii| items.get(*ii).is_some_and(|v| !v.is_blank()));
                if !keep {
                    continue;
                }
                let value_at = |col: usize| -> V {
                    if col < ng {
                        group_value(gdata, combos, col, ci)
                    } else {
                        items.get(col.saturating_sub(ng)).cloned().unwrap_or(V::Blank)
                    }
                };
                sel.offer(self, ci, &value_at)?;
            }
            lo = lo.saturating_add(len);
        }
        Ok(true)
    }

    /// The evaluator's state that depends on where an expression is evaluated, reset to the
    /// query's top level (where its variables are defined).
    pub(super) fn take_dynamic(&mut self) -> Dynamic {
        Dynamic {
            group_cols: std::mem::take(&mut self.group_cols),
            rolled: std::mem::take(&mut self.rolled),
            type_scope: std::mem::take(&mut self.type_scope),
            outer: std::mem::take(&mut self.outer),
            selected: std::mem::take(&mut self.selected),
        }
    }

    pub(super) fn restore_dynamic(&mut self, d: Dynamic) {
        self.group_cols = d.group_cols;
        self.rolled = d.rolled;
        self.type_scope = d.type_scope;
        self.outer = d.outer;
        self.selected = d.selected;
    }

    /// The table of a variable evaluated on first use ([`Bind::L`]).
    pub(super) fn force_lazy(&mut self, l: &Lazy) -> Result<Tab, Fail> {
        if let Some(t) = l.cell.get() {
            return Ok(t.clone());
        }
        let fc = FCtx::default();
        let scope = Scope {
            fc: &fc,
            rc: &[],
            env: &l.env,
            depth: 0,
            mdepth: 0,
        };
        let saved = self.take_dynamic();
        let r = self.table(&l.expr, &scope);
        self.restore_dynamic(saved);
        let t = r?;
        let _ = l.cell.set(t.clone());
        Ok(t)
    }
}

#[cfg(test)]
thread_local! {
    /// How many `TOPN`s the grouping path answered on this thread (tests check it runs).
    pub(super) static GROUPING_TOPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// How many of them went block by block under a memory cap.
    pub(super) static GROUPING_BLOCKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// How many ranges of groups were scanned by [`Engine::grouping_top_ranges`].
    pub(super) static GROUPING_RANGES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Tests: the groups of one range (instead of what the room allows).
    pub(super) static TEST_RANGE_LEN: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Groups of one range of [`Engine::grouping_top_ranges`] to start with: as many as fit in half
/// of `room` at `per` bytes per group, at least one, at most every group (a range whose scan is
/// then refused is halved).
fn range_len(room: usize, per: usize, groups: usize) -> usize {
    ((room / 2) / per.max(1)).clamp(1, groups.max(1))
}

/// The value of grouping column `col` of combination `ci`.
fn group_value(gdata: &[Arc<ColData>], combos: &Combos, col: usize, ci: usize) -> V {
    match (gdata.get(col), combos.get(ci).and_then(|cb| cb.get(col))) {
        (Some(d), Some(code)) => d.values.get(*code as usize).map_or(V::Blank, V::of_cell),
        _ => V::Blank,
    }
}

/// The bounded selection of [`Engine::grouping_top_blocks`]: the best `take` keys so far (a
/// max-heap whose top is the worst kept) and the candidates' rows.
struct TopSel {
    order: Rc<KeyOrder>,
    take: usize,
    width: usize,
    heap: BinaryHeap<HeapKey>,
    /// (position, row) of the candidates.
    cands: Vec<(usize, Vec<V>)>,
    cand_charges: Vec<super::mem::Charge>,
    cand_bytes: usize,
    /// Groups kept (offered).
    len: usize,
    _heap_charge: super::mem::Charge,
}

impl TopSel {
    fn new(eng: &mut Engine<'_, '_>, pick: &Pick<'_>, take: usize) -> Result<TopSel, Fail> {
        let ng = pick.groups.len();
        let order = Rc::new(KeyOrder {
            cols: pick.keys.iter().map(|(at, d)| (key_col(*at, ng), *d)).collect(),
            exact: pick.exact,
        });
        let nk = pick.keys.len();
        let heap_bytes = take.saturating_mul(super::mem::row_bytes(nk).saturating_add(16));
        let heap_charge = eng.charge_bytes(
            &move || format!("the {take} best keys of TOPN() over a grouping"),
            heap_bytes,
        )?;
        Ok(TopSel {
            order,
            take,
            width: pick.width,
            heap: BinaryHeap::new(),
            cands: Vec::new(),
            cand_charges: Vec::new(),
            cand_bytes: super::mem::row_bytes(pick.width).saturating_add(8),
            len: 0,
            _heap_charge: heap_charge,
        })
    }

    /// Most bytes the candidates take before they are pruned (about twice `take` rows, plus 1,040).
    fn cand_room(&self) -> usize {
        self.take
            .saturating_mul(2)
            .saturating_add(1024 + 16)
            .saturating_mul(self.cand_bytes)
    }

    /// A kept group at position `pos` (positions only order the groups), its row's columns read
    /// by `value_at`: a candidate unless it comes after the worst of the best `take` so far.
    fn offer(
        &mut self,
        eng: &mut Engine<'_, '_>,
        pos: usize,
        value_at: &dyn Fn(usize) -> V,
    ) -> Result<(), Fail> {
        self.len = self.len.saturating_add(1);
        let take = self.take;
        if take == 0 {
            return Ok(());
        }
        let key = HeapKey {
            keys: self.order.cols.iter().map(|(col, _)| value_at(*col)).collect(),
            pos,
            order: Rc::clone(&self.order),
        };
        let full = self.heap.len() >= take;
        if full
            && self
                .heap
                .peek()
                .is_some_and(|top| key.cmp(top) == Ordering::Greater)
        {
            return Ok(());
        }
        // a candidate: its whole row
        if self.cands.len().is_multiple_of(16) && eng.mem.on() {
            let at = self.cands.len();
            self.cand_charges.push(eng.charge_bytes(
                &move || format!("the candidates of TOPN() over a grouping (over {at})"),
                self.cand_bytes.saturating_mul(16),
            )?);
        }
        self.cands.push((pos, (0..self.width).map(value_at).collect()));
        if !full {
            self.heap.push(key);
        } else if self.heap.peek().is_some_and(|top| key.cmp(top) == Ordering::Less) {
            self.heap.pop();
            self.heap.push(key);
        }
        // the candidates the bound now rules out go
        if self.cands.len() > take.saturating_mul(2).saturating_add(1024)
            && self.heap.len() >= take
            && let Some(top) = self.heap.peek()
        {
            let order = &self.order;
            self.cands
                .retain(|(p, r)| order.row_cmp(r, *p, &top.keys, top.pos) != Ordering::Greater);
            self.cand_charges.truncate(self.cands.len().div_ceil(16));
        }
        Ok(())
    }

    /// The exact selection among the candidates: every key, ties by position. Returns the rows
    /// and the number of kept groups.
    fn finish(mut self) -> Picked {
        if self.heap.len() >= self.take
            && let Some(top) = self.heap.peek()
        {
            let order = &self.order;
            self.cands
                .retain(|(p, r)| order.row_cmp(r, *p, &top.keys, top.pos) != Ordering::Greater);
        }
        self.heap.clear();
        // in position order (offers by ranges come out of it)
        self.cands.sort_by_key(|(p, _)| *p);
        let take = self.take.min(self.len);
        let order = &self.order;
        let cands = &self.cands;
        let cmp = |a: usize, b: usize| -> Ordering {
            match (cands.get(a), cands.get(b)) {
                (Some(x), Some(y)) => order.rows_cmp(&x.1, x.0, &y.1, y.0),
                _ => Ordering::Equal,
            }
        };
        let picked = top_rows((0..cands.len()).collect(), take, &cmp);
        #[cfg(test)]
        GROUPING_BLOCKS.with(|c| c.set(c.get().saturating_add(1)));
        let mut slots: Vec<Option<Vec<V>>> = std::mem::take(&mut self.cands)
            .into_iter()
            .map(|(_, r)| Some(r))
            .collect();
        let mut rows = Vec::with_capacity(picked.len());
        for j in picked {
            if let Some(r) = slots.get_mut(j).and_then(Option::take) {
                rows.push(r);
            }
        }
        (rows, self.len)
    }
}

/// Rows a block selection picked, and the number of groups kept.
type Picked = (Vec<Vec<V>>, usize);

/// Every group's values, and their reservation.
type Charged = (Vec<Vec<V>>, super::mem::Charge);

/// What [`Engine::grouping_top`] computed: every group's values (with their reservation), or the
/// rows the block path already selected and the number of kept groups.
enum Computed {
    Whole(Combos, Vec<Vec<V>>, super::mem::Charge),
    Done(Picked),
}

/// The selection [`Engine::grouping_top_blocks`] makes.
struct Pick<'p> {
    n: f64,
    direct: &'p [(super::Agg, ColId)],
    /// Items that decide whether a group is kept (not under `IGNORE`).
    counted: &'p [usize],
    groups: &'p [ColId],
    keys: &'p [(KeyAt, bool)],
    /// Columns of a row: the group-by columns, then the items.
    width: usize,
    /// Ties broken by position: exactly `n` rows (an `ORDER BY` cut by `max_rows`), not
    /// `TOPN`'s rows tying with the last.
    exact: bool,
}

/// The column of a row a key reads.
fn key_col(at: KeyAt, groups: usize) -> usize {
    match at {
        KeyAt::Group(gi) => gi,
        KeyAt::Item(ii) => groups.saturating_add(ii),
    }
}

/// The order of the first `TOPN` key (ascending): DAX's sort order, Currency values compared
/// exactly (as [`TopKey`] compares them).
fn first_key_cmp(a: &V, b: &V) -> Ordering {
    match (a, b) {
        (V::Cur(x), V::Cur(y)) => x.cmp(y),
        _ => sort_cmp(a, b),
    }
}

static BLANK: V = V::Blank;

/// The order of the rows of [`Engine::grouping_top_blocks`]: each key in its direction (the
/// first as [`first_key_cmp`], the others as [`sort_cmp`]), then the position when `exact`.
struct KeyOrder {
    /// (column of the row, descending) per key.
    cols: Vec<(usize, bool)>,
    exact: bool,
}

impl KeyOrder {
    fn at(&self, k: usize, x: &V, y: &V) -> Ordering {
        let o = if k == 0 {
            first_key_cmp(x, y)
        } else {
            sort_cmp(x, y)
        };
        if self.cols.get(k).is_some_and(|c| c.1) {
            o.reverse()
        } else {
            o
        }
    }

    fn tail(&self, pa: usize, pb: usize) -> Ordering {
        if self.exact { pa.cmp(&pb) } else { Ordering::Equal }
    }

    /// A row (every column) against key values.
    fn row_cmp(&self, row: &[V], pos: usize, keys: &[V], kpos: usize) -> Ordering {
        for (k, (col, _)) in self.cols.iter().enumerate() {
            let o = self.at(k, row.get(*col).unwrap_or(&BLANK), keys.get(k).unwrap_or(&BLANK));
            if o != Ordering::Equal {
                return o;
            }
        }
        self.tail(pos, kpos)
    }

    /// Two rows (every column).
    fn rows_cmp(&self, a: &[V], pa: usize, b: &[V], pb: usize) -> Ordering {
        for (k, (col, _)) in self.cols.iter().enumerate() {
            let o = self.at(k, a.get(*col).unwrap_or(&BLANK), b.get(*col).unwrap_or(&BLANK));
            if o != Ordering::Equal {
                return o;
            }
        }
        self.tail(pa, pb)
    }

    /// Two rows' key values.
    fn keys_cmp(&self, a: &[V], pa: usize, b: &[V], pb: usize) -> Ordering {
        for k in 0..self.cols.len() {
            let o = self.at(k, a.get(k).unwrap_or(&BLANK), b.get(k).unwrap_or(&BLANK));
            if o != Ordering::Equal {
                return o;
            }
        }
        self.tail(pa, pb)
    }
}

/// A row's keys in the bounded heap of [`Engine::grouping_top_blocks`] (the heap's top is the
/// worst row kept).
struct HeapKey {
    keys: Vec<V>,
    pos: usize,
    order: Rc<KeyOrder>,
}

impl PartialEq for HeapKey {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for HeapKey {}

impl PartialOrd for HeapKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HeapKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.order.keys_cmp(&self.keys, self.pos, &other.keys, other.pos)
    }
}

/// Where a `TOPN` key of a grouping is read.
#[derive(Clone, Copy)]
enum KeyAt {
    Item(usize),
    Group(usize),
}

/// `ROLLUPADDISSUBTOTAL` and the other grouping modifiers (not filter tables).
fn is_grouping_modifier(e: &Expr) -> bool {
    matches!(e, Expr::Call { name, .. } if matches!(
        name.to_ascii_uppercase().as_str(),
        "ROLLUPADDISSUBTOTAL" | "ROLLUPGROUP" | "NONVISUAL" | "ROLLUP"
    ))
}

/// State saved by [`Engine::take_dynamic`].
pub(super) struct Dynamic {
    group_cols: Vec<ColId>,
    rolled: Vec<ColId>,
    type_scope: Vec<TCol>,
    outer: FCtx,
    selected: Vec<Rc<MeasureDef>>,
}

/// A query variable holding a table, evaluated on first use: `VAR __DS0Core =
/// SUMMARIZECOLUMNS(…)` read only by `TOPN` never builds its rows.
#[derive(Debug)]
pub(super) struct Lazy {
    pub(super) expr: Expr,
    /// The variables defined before it.
    pub(super) env: Env,
    pub(super) cell: OnceCell<Tab>,
}
