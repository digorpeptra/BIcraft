//! Row iterators over a fact table as the items of a grouping (`docs/storage-engine.md`, "Grouped
//! row iterators"): `SUMMARIZECOLUMNS(D[g], "x", SUMX(T, T[a] * RELATED(P[b])))`, or a measure
//! whose expression is such an iterator, computed for every combination of the grouping at once
//! instead of once per cell.
//!
//! Each cell of the reference evaluates the iterator in its own filter context: the base filters
//! of the grouping plus one code of each grouping column. Its storage-engine path
//! ([`Engine::iterate_table_scan`]) already evaluates the body once per combination of the codes it
//! reads, so a cell costs one scan of the visible rows; a grouping of 1,000 products cost 1,000
//! scans. Here one grouped scan serves every cell. Its key is made of the grouping columns (their
//! lookup tables from the fact's columns, as the grouped aggregations use them) and of the columns
//! the body reads; parts that read the same fact column (`Produit[Produit]` and
//! `RELATED(Produit[PrixCatalogue])` both read `Ventes[ProduitKey]`) are fused into that column's
//! own codes when it has fewer codes than their product, so the key stays as small as the fact's
//! foreign keys and every part's code is a lookup from it, with no per-row hashing.
//!
//! - **Exact products** ([`Engine::grouped_products`]): `SUMX`, `AVERAGEX` and `COUNTX` of a
//!   product of columns of the table and `RELATED` columns holding whole numbers or Currency (at
//!   most one Currency factor, since Currency × Currency is a double), when every product and
//!   every partial sum stays below 2^52 units: one factor of the table is summed by the grouped
//!   sum kernel (the rows are counted when there is none), the others are read per group, and
//!   each cell adds its groups' sums times their factors, in exact integers. The reference
//!   multiplies and adds the same integers in doubles or Currency units, exactly, so the results
//!   are the same values (Currency stays Currency, a blank factor makes the product blank and the
//!   row is skipped, no row makes the cell blank).
//! - **Any other row expression** ([`Engine::grouped_weighted`]): a tally of the rows and the
//!   first row of each (cell, body combination); the body evaluated once per combination met; each
//!   cell aggregates its combinations, weighted by their rows, in the order of their first rows,
//!   exactly as the per-cell path does it ([`Engine::weighted`]).
//!
//! A `FILTER(T, p)` source whose predicate reads one column is judged once per code of it, as the
//! per-cell path does. Anything else, a relationship override (`USERELATIONSHIP`,
//! `CROSSFILTER`), a calculation group, an evaluation error, a key or a result too large, gives
//! `None` and the cells are evaluated one by one, which reports what the reference reports.

use std::collections::HashMap;
use std::sync::Arc;

use bicraft_dax::ast::Expr;
use bicraft_scan as scan;

use super::super::super::store::ColData;
use super::{
    Acc, Agg, ColId, Combos, Engine, Env, FCtx, Fail, Filter, Lut, PredSet, Scope, Tab, TabId, V, currency,
    internal, overflow, rel_key,
};

/// Most groups of the fused key of the tally ([`Engine::grouped_weighted`]).
const MAX_TALLY_GROUPS: usize = 1 << 22;

/// Most cells of a grouping computed at once (dense accumulators).
const MAX_CELLS: usize = 1 << 22;

/// Every product and partial sum of [`Engine::grouped_products`] stays below this (in units).
const EXACT: u128 = 1 << 52;

/// A grouping part: the column it stands for (the scan cache's key) and the lookup table from a
/// column of the scanned table to its codes.
type KPart = (ColId, Arc<Lut>);

/// The fused key of some parts: the key's parts, and for each input part the key part it reads
/// and the lookup from that part's codes to its own (`None`: the same codes).
struct Fused {
    keys: Vec<KPart>,
    of: Vec<(usize, Option<Arc<Lut>>)>,
}

impl Fused {
    /// The code of input part `i` in the key group whose part codes are `kc`.
    fn code(&self, i: usize, kc: &[u32]) -> u32 {
        let Some((k, l)) = self.of.get(i) else {
            return 0;
        };
        let c = kc.get(*k).copied().unwrap_or(0);
        match l.as_ref().and_then(|l| l.map.as_ref()) {
            Some(m) => m.get(c as usize).copied().unwrap_or(0),
            None => c,
        }
    }
}

/// The part codes of group `g` of a key of `cards` codes per part, the first part slowest.
fn decode(cards: &[usize], mut g: usize, out: &mut [u32]) {
    for (o, card) in out.iter_mut().zip(cards).rev() {
        let card = (*card).max(1);
        *o = u32::try_from(g % card).unwrap_or(u32::MAX);
        g /= card;
    }
}

/// A factor of an exact product: its values per code, its lookup from a column of the scanned
/// table, and whether it is the summed column (a column of the table).
struct Factor {
    col: ColId,
    vals: Arc<Vec<Option<i64>>>,
    cur: bool,
    part: KPart,
    card: usize,
    own: bool,
}

/// How the cells of a grouping map to dense numbers: the grouping columns that reach the table,
/// their position among the grouping's columns, and their codes.
struct Cells {
    pos: Vec<usize>,
    cards: Vec<usize>,
    count: usize,
}

impl Cells {
    /// The cell of a combination of the grouping (`None`: a code none of the rows can have).
    fn of_combo(&self, combo: &[u32]) -> Option<usize> {
        let mut id = 0usize;
        for (p, card) in self.pos.iter().zip(&self.cards) {
            let c = *combo.get(*p)? as usize;
            if c >= (*card).max(1) {
                return None;
            }
            id = id.checked_mul((*card).max(1))?.checked_add(c)?;
        }
        Some(id)
    }

    /// The cell of the cell parts' codes `codes` (in the order of `pos`).
    fn of_codes(&self, codes: impl Iterator<Item = u32>) -> Option<usize> {
        let mut id = 0usize;
        for (c, card) in codes.zip(&self.cards) {
            let c = c as usize;
            if c >= (*card).max(1) {
                return None;
            }
            id = id.checked_mul((*card).max(1))?.checked_add(c)?;
        }
        Some(id)
    }
}

/// What both paths share: the iterated table, its rows, the cells, the base filters and the
/// rows the iterator keeps.
struct Setup {
    t: TabId,
    n: usize,
    cells: Cells,
    cell_parts: Vec<KPart>,
    base: Vec<Filter>,
    /// The base's rows with the `FILTER` predicate's codes (`None`: no `FILTER`, the base alone).
    filtered: Option<PredSet>,
}

impl Engine<'_, '_> {
    /// The values of the item `e` of a grouping by `groups` in `fc` for each of `combos`, when it
    /// is a row iterator over a model table (inline, or a measure whose expression is one) that
    /// one grouped scan computes for every cell; `None`: the cells are evaluated one by one.
    pub(in crate::preview::general) fn vector_iter(
        &mut self,
        e: &Expr,
        s: &Scope,
        fc: &FCtx,
        groups: &[ColId],
        combos: &Combos,
        depth: usize,
    ) -> Result<Option<Vec<V>>, Fail> {
        if off() || self.store.is_none() || !self.groups.is_empty() || depth > 16 || !rel_key(fc).is_empty() {
            return Ok(None);
        }
        match e {
            Expr::Column {
                table,
                name,
                variation: None,
                ..
            } => {
                if self.refs.contains_key(&(e as *const Expr as usize)) {
                    return Ok(None);
                }
                let Some(def) = self.measure_def(table.as_deref(), name) else {
                    return Ok(None);
                };
                let Some(body) = def.expr.as_ref() else {
                    return Ok(None);
                };
                // a measure's expression sees no variable of the query
                let env = Env::new();
                let inner = Scope { env: &env, ..*s };
                self.vector_iter(body, &inner, fc, groups, combos, depth.saturating_add(1))
            }
            Expr::Call { name, args, .. } if name.eq_ignore_ascii_case("CALCULATE") => {
                // `CALCULATE(x, T[c] > 4, …)` with filters on tables no grouping column belongs
                // to: a boolean filter replaces the filters on its column (and, on a date table's
                // key, on its table), never a cell's, so the context of each cell is the
                // grouping's context with the filters applied, plus the cell's codes
                let Some(Some(inner)) = args.first() else {
                    return Ok(None);
                };
                for a in args.iter().skip(1) {
                    let Some(a) = a else {
                        return Ok(None);
                    };
                    if !self.simple_filter_outside(a, groups)? {
                        return Ok(None);
                    }
                }
                let sc = Scope { fc, rc: &[], ..*s };
                let Ok(fc2) = self.calc_ctx(args, &sc) else {
                    return Ok(None);
                };
                self.vector_iter(inner, s, &fc2, groups, combos, depth.saturating_add(1))
            }
            Expr::Call { name, args, .. } => {
                let f = name.to_ascii_uppercase();
                if !matches!(
                    f.as_str(),
                    "SUMX" | "AVERAGEX" | "MINX" | "MAXX" | "COUNTX" | "COUNTAX"
                ) {
                    return Ok(None);
                }
                let [Some(src), Some(body)] = args.as_slice() else {
                    return Ok(None);
                };
                let Some(setup) = self.iter_setup(src, body, s, fc, groups)? else {
                    return Ok(None);
                };
                if matches!(f.as_str(), "SUMX" | "AVERAGEX" | "COUNTX" | "COUNTAX")
                    && let Some(v) = self.grouped_products(&f, body, s, fc, &setup, combos)?
                {
                    return Ok(Some(v));
                }
                self.grouped_weighted(&f, body, s, fc, &setup, combos)
            }
            _ => Ok(None),
        }
    }

    /// Whether `a` is a boolean filter `T[c] op constant` (or `constant op T[c]`) on a column of
    /// a table that holds none of `groups`.
    fn simple_filter_outside(&mut self, a: &Expr, groups: &[ColId]) -> Result<bool, Fail> {
        let Expr::Binary { op, lhs, rhs, .. } = a else {
            return Ok(false);
        };
        if !matches!(op.as_str(), "=" | "==" | "<>" | "<" | "<=" | ">" | ">=") {
            return Ok(false);
        }
        let constant = |e: &Expr| match e {
            Expr::Number { .. } | Expr::Str { .. } | Expr::Bool { .. } => true,
            Expr::Unary { op, operand, .. } => op == "-" && matches!(**operand, Expr::Number { .. }),
            _ => false,
        };
        let col = match (&**lhs, &**rhs) {
            (c @ Expr::Column { .. }, k) if constant(k) => c,
            (k, c @ Expr::Column { .. }) if constant(k) => c,
            _ => return Ok(false),
        };
        let Expr::Column {
            table: Some(tn),
            name,
            variation: None,
            ..
        } = col
        else {
            return Ok(false);
        };
        if self.measure_def(Some(tn), name).is_some() {
            return Ok(false);
        }
        let Some(c) = self.col_id(tn, name) else {
            return Ok(false);
        };
        let ct = self.col_table(c)?;
        for g in groups {
            if self.col_table(*g)? == ct {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// The table `src` iterates (`T` or `FILTER(T, p)`, `p` reading one column of `T`), when
    /// `body` reads only its columns and `RELATED` columns, with the cells of the grouping.
    fn iter_setup(
        &mut self,
        src: &Expr,
        body: &Expr,
        s: &Scope,
        fc: &FCtx,
        groups: &[ColId],
    ) -> Result<Option<Setup>, Fail> {
        let (name, pred) = match src {
            Expr::Name { name, .. } => (name, None),
            Expr::Call {
                name: fname, args, ..
            } if fname.eq_ignore_ascii_case("FILTER") => match args.as_slice() {
                [Some(Expr::Name { name, .. }), Some(p)] => (name, Some(p)),
                _ => return Ok(None),
            },
            _ => return Ok(None),
        };
        if s.env.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)) {
            return Ok(None);
        }
        let Some(t) = self.table_id(name) else {
            return Ok(None);
        };
        let mut deps = Vec::new();
        if !self.row_deps(body, t, s.env, &mut deps, 0)? {
            return Ok(None);
        }
        let n = self.rows(t)?;
        let reach = self.reach(fc, t)?;
        let mut cell_parts = Vec::new();
        let mut pos = Vec::new();
        let mut cards = Vec::new();
        for (gi, g) in groups.iter().enumerate() {
            // a filter that does not reach the table changes nothing in it
            if !reach.get(self.col_table(*g)?).copied().unwrap_or(true) {
                continue;
            }
            let Some(l) = self.group_lut(fc, t, *g)? else {
                return Ok(None);
            };
            cards.push(l.card);
            cell_parts.push((*g, l));
            pos.push(gi);
        }
        let count = cards.iter().try_fold(1usize, |a, c| a.checked_mul((*c).max(1)));
        let Some(count) = count.filter(|c| *c <= MAX_CELLS) else {
            return Ok(None);
        };
        let base: Vec<Filter> = fc
            .filters
            .iter()
            .filter(|f| reach.get(f.table).copied().unwrap_or(true))
            .cloned()
            .collect();
        let filtered = match pred {
            None => None,
            Some(pe) => {
                let mut pdeps = Vec::new();
                if !self.row_deps(pe, t, s.env, &mut pdeps, 0)? {
                    return Ok(None);
                }
                let [pc] = pdeps.as_slice() else {
                    return Ok(None);
                };
                let Some(keep) = self.keep_codes(pe, t, *pc, s, fc)? else {
                    return Ok(None);
                };
                let base_fc = FCtx {
                    filters: base.clone(),
                    rels: fc.rels.clone(),
                    applied: fc.applied,
                    memo: Default::default(),
                };
                let mut p = self.pred(&base_fc, t)?.unwrap_or_else(|| PredSet::all(n));
                p.codes.push((self.col(*pc)?, Arc::new(keep)));
                Some(p)
            }
        };
        Ok(Some(Setup {
            t,
            n,
            cells: Cells { pos, cards, count },
            cell_parts,
            base,
            filtered,
        }))
    }

    /// The codes of column `c` of `t` whose value passes the row predicate `pe`, judged on the
    /// first row of each code (the predicate reads only that column): what the per-cell path
    /// keeps. `None` on any error (the cells then report it, or not, as before).
    fn keep_codes(
        &mut self,
        pe: &Expr,
        t: TabId,
        c: ColId,
        s: &Scope,
        fc: &FCtx,
    ) -> Result<Option<Vec<bool>>, Fail> {
        let d = self.col(c)?;
        let Some(first) = d.first_rows() else {
            return Ok(None);
        };
        let mut reps: Vec<(u32, u32)> = first
            .iter()
            .enumerate()
            .filter(|(_, r)| **r != u32::MAX)
            .filter_map(|(g, r)| u32::try_from(g).ok().map(|g| (*r, g)))
            .collect();
        reps.sort_unstable();
        let rep_tab = Tab::rows_of(t, reps.iter().map(|(r, _)| *r).collect());
        let inner = Scope { fc, rc: &[], ..*s };
        let Ok(vals) = self.each_with(&rep_tab, &inner, |eng, sc| eng.scalar(pe, sc)) else {
            return Ok(None);
        };
        let mut keep = vec![false; d.values.len()];
        for ((_, code), v) in reps.iter().zip(&vals) {
            match (keep.get_mut(*code as usize), super::super::truth(v)) {
                (Some(k), Ok(b)) => *k = b,
                _ => return Ok(None),
            }
        }
        Ok(Some(keep))
    }

    /// Fuses parts that read the same column of the scanned table `t` into that column's own
    /// codes when it has fewer codes than their product (see the module's notes).
    fn fuse(&mut self, fc: &FCtx, t: TabId, parts: &[KPart]) -> Result<Option<Fused>, Fail> {
        let mut keys: Vec<KPart> = Vec::new();
        let mut of: Vec<(usize, Option<Arc<Lut>>)> = vec![(0, None); parts.len()];
        let mut done = vec![false; parts.len()];
        for i in 0..parts.len() {
            if done.get(i).copied().unwrap_or(true) {
                continue;
            }
            let Some((_, li)) = parts.get(i) else {
                return Ok(None);
            };
            let src = li.col;
            // the distinct parts (by lookup table) over the same column
            let mut members: Vec<usize> = Vec::new();
            let mut distinct: Vec<Arc<Lut>> = Vec::new();
            for (j, (_, lj)) in parts.iter().enumerate().skip(i) {
                if lj.col == src && !done.get(j).copied().unwrap_or(true) {
                    members.push(j);
                    if !distinct.iter().any(|l| Arc::ptr_eq(l, lj)) {
                        distinct.push(Arc::clone(lj));
                    }
                }
            }
            for j in &members {
                if let Some(d) = done.get_mut(*j) {
                    *d = true;
                }
            }
            let identity = distinct.iter().any(|l| l.map.is_none());
            let product = distinct
                .iter()
                .try_fold(1usize, |a, l| a.checked_mul(l.card.max(1)));
            let src_card = self.col(src)?.values.len();
            if distinct.len() > 1 && (identity || product.is_none_or(|p| src_card <= p)) {
                // the column's own codes: every part a lookup from them
                let Some(own) = self.group_lut(fc, t, src)? else {
                    return Ok(None);
                };
                if own.map.is_some() {
                    return Ok(None);
                }
                let k = keys.len();
                keys.push((src, own));
                for j in members {
                    let lj = parts.get(j).map(|(_, l)| Arc::clone(l));
                    if let Some(o) = of.get_mut(j) {
                        *o = (k, lj.filter(|l| l.map.is_some()));
                    }
                }
            } else {
                // each distinct part its own key part
                for l in distinct {
                    let k = keys.len();
                    let g = members
                        .iter()
                        .find_map(|j| {
                            parts
                                .get(*j)
                                .filter(|(_, lj)| Arc::ptr_eq(lj, &l))
                                .map(|(g, _)| *g)
                        })
                        .unwrap_or(src);
                    keys.push((g, Arc::clone(&l)));
                    for j in &members {
                        if parts.get(*j).is_some_and(|(_, lj)| Arc::ptr_eq(lj, &l))
                            && let Some(o) = of.get_mut(*j)
                        {
                            *o = (k, None);
                        }
                    }
                }
            }
        }
        Ok(Some(Fused { keys, of }))
    }

    /// The factors of `e` when it is a product of columns of `t` and `RELATED` columns (see
    /// [`Engine::grouped_products`]); `false`: another expression.
    fn product_factors(
        &mut self,
        e: &Expr,
        t: TabId,
        s: &Scope,
        fc: &FCtx,
        out: &mut Vec<Factor>,
        depth: usize,
    ) -> Result<bool, Fail> {
        if depth > 16 || out.len() > 8 {
            return Ok(false);
        }
        if let Expr::Binary { op, lhs, rhs, .. } = e {
            return Ok(op == "*"
                && self.product_factors(lhs, t, s, fc, out, depth.saturating_add(1))?
                && self.product_factors(rhs, t, s, fc, out, depth.saturating_add(1))?);
        }
        let (col, part, own) = if let Some(c) = self.own_column(e, t) {
            let Some(l) = self.group_lut(fc, t, c)? else {
                return Ok(false);
            };
            (c, (c, l), true)
        } else {
            let Expr::Call { name, args, .. } = e else {
                return Ok(false);
            };
            if !name.eq_ignore_ascii_case("RELATED") {
                return Ok(false);
            }
            let [
                Some(Expr::Column {
                    table: Some(tn),
                    name: cn,
                    variation: None,
                    ..
                }),
            ] = args.as_slice()
            else {
                return Ok(false);
            };
            let Some(b) = self.col_id(tn, cn) else {
                return Ok(false);
            };
            if self.col_table(b)? == t {
                return Ok(false);
            }
            // the foreign key `RELATED` follows, and the lookup from it to the column's codes
            let mut fk = Vec::new();
            if !self.row_deps(e, t, s.env, &mut fk, 0)? {
                return Ok(false);
            }
            let [fk] = fk.as_slice() else {
                return Ok(false);
            };
            match self.group_lut(fc, t, b)? {
                Some(l) if l.col == *fk && l.map.is_some() => (b, (b, l), false),
                _ => return Ok(false),
            }
        };
        let d = self.col(col)?;
        let (_, all_cur, all_int) = d.kinds();
        let (vals, cur) = if all_cur && !all_int {
            (self.cur_codes(&d), true)
        } else if all_int {
            (self.int_codes(col, &d)?, false)
        } else {
            (None, false)
        };
        let Some(vals) = vals else {
            return Ok(false);
        };
        out.push(Factor {
            col,
            vals,
            cur,
            card: d.values.len(),
            part,
            own,
        });
        Ok(true)
    }

    /// `SUMX` / `AVERAGEX` / `COUNTX` of an exact product for every cell (see the module's
    /// notes); `None`: another path.
    fn grouped_products(
        &mut self,
        f: &str,
        body: &Expr,
        s: &Scope,
        fc: &FCtx,
        st: &Setup,
        combos: &Combos,
    ) -> Result<Option<Vec<V>>, Fail> {
        let t = st.t;
        let mut factors = Vec::new();
        if !self.product_factors(body, t, s, fc, &mut factors, 0)? {
            return Ok(None);
        }
        let curs = factors.iter().filter(|x| x.cur).count();
        if curs > 1 || factors.is_empty() {
            return Ok(None);
        }
        // every product and partial sum exact
        let bound = factors.iter().fold(st.n as u128, |b, x| {
            let m = x
                .vals
                .iter()
                .flatten()
                .map(|v| v.unsigned_abs())
                .max()
                .unwrap_or(0);
            b.saturating_mul(u128::from(m.max(1)))
        });
        if bound >= EXACT {
            return Ok(None);
        }
        // the summed factor: the column of the table with the most codes (else the rows)
        let summed = factors
            .iter()
            .enumerate()
            .filter(|(_, x)| x.own)
            .max_by_key(|(_, x)| x.card)
            .map(|(i, _)| i);
        let (agg, c) = match summed.and_then(|i| factors.get(i)) {
            Some(x) if f == "COUNTX" || f == "COUNTAX" => (Agg::Count, x.col),
            Some(x) => (Agg::Sum, x.col),
            None => match self.value_cols(t).first() {
                Some(c) => (Agg::CountRows, *c),
                None => return Ok(None),
            },
        };
        let keyed: Vec<&Factor> = factors
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != summed)
            .map(|(_, x)| x)
            .collect();
        let mut parts: Vec<KPart> = st.cell_parts.clone();
        parts.extend(keyed.iter().map(|x| (x.part.0, Arc::clone(&x.part.1))));
        let Some(fused) = self.fuse(fc, t, &parts)? else {
            return Ok(None);
        };
        let d = self.col(c)?;
        let groups = fused
            .keys
            .iter()
            .try_fold(1usize, |a, (_, l)| a.checked_mul(l.card.max(1)));
        match groups {
            Some(g)
                if g <= scan::MAX_GROUPS
                    && g.saturating_mul(Self::group_bytes(agg, d.values.len())) <= super::MAX_ENTRY_BYTES => {
            }
            _ => return Ok(None),
        }
        let entry = match &st.filtered {
            Some(p) => self
                .compute_with(agg, c, &d, st.n, Some(p.clone()), &fused.keys)?
                .map(Arc::new),
            None => self.entry(agg, c, &d, t, st.n, fc, st.base.clone(), &fused.keys)?,
        };
        let Some(entry) = entry else {
            return Ok(None);
        };
        let (sums, counts): (Option<&Vec<i128>>, &[u64]) = match &entry.acc {
            Acc::Cur { sums, counts } | Acc::Int { sums, counts } => (Some(sums), counts.as_slice()),
            Acc::Rows(v) => (None, v.as_slice()),
            Acc::Blanks(b) => (None, b.nonblank.as_slice()),
            _ => return Ok(None),
        };
        if agg == Agg::Sum && sums.is_none() {
            return Ok(None);
        }
        self.tick(counts.len())?;
        let ncells = st.cells.count;
        let mut totals = vec![0i128; ncells];
        let mut rows = vec![0u64; ncells];
        let ncell_parts = st.cell_parts.len();
        let mut kc = vec![0u32; entry.cards.len()];
        for (g, cnt) in counts.iter().enumerate() {
            if *cnt == 0 {
                continue;
            }
            decode(&entry.cards, g, &mut kc);
            // the keyed factors' product (blank: no product)
            let mut prod = 1i128;
            let mut blank = false;
            for (i, x) in keyed.iter().enumerate() {
                let code = fused.code(ncell_parts.saturating_add(i), &kc);
                match x.vals.get(code as usize).copied().flatten() {
                    Some(v) => prod = prod.saturating_mul(i128::from(v)),
                    None => {
                        blank = true;
                        break;
                    }
                }
            }
            if blank {
                continue;
            }
            let Some(cell) = st.cells.of_codes((0..ncell_parts).map(|i| fused.code(i, &kc))) else {
                continue;
            };
            let base = match sums {
                Some(v) if agg == Agg::Sum => v.get(g).copied().unwrap_or(0),
                _ => i128::from(*cnt),
            };
            if let (Some(tot), Some(r)) = (totals.get_mut(cell), rows.get_mut(cell)) {
                *tot = tot.saturating_add(base.saturating_mul(prod));
                *r = r.saturating_add(*cnt);
            }
        }
        let cur = curs == 1;
        let mut out = Vec::with_capacity(combos.len());
        self.tick(combos.len())?;
        for combo in combos.iter() {
            let Some(cell) = st.cells.of_combo(combo) else {
                out.push(V::Blank);
                continue;
            };
            let (total, k) = (
                totals.get(cell).copied().unwrap_or(0),
                rows.get(cell).copied().unwrap_or(0),
            );
            out.push(if k == 0 {
                V::Blank
            } else {
                match f {
                    "COUNTX" | "COUNTAX" => V::Num(k as f64),
                    "AVERAGEX" if cur => match currency::div_round(total, i128::from(k)) {
                        Some(x) => V::Cur(x),
                        None => return overflow(),
                    },
                    "AVERAGEX" => V::Num(total as f64 / k as f64),
                    _ if cur => match i64::try_from(total) {
                        Ok(x) => V::Cur(x),
                        Err(_) => return overflow(),
                    },
                    _ => V::Num(total as f64),
                }
            });
        }
        hit(0);
        Ok(Some(out))
    }

    /// Any row iterator for every cell: one tally of the rows by (cell, combination of the codes
    /// the body reads), the body once per combination, each cell's combinations aggregated in
    /// the order of their first rows (see the module's notes); `None`: the cells one by one.
    fn grouped_weighted(
        &mut self,
        f: &str,
        body: &Expr,
        s: &Scope,
        fc: &FCtx,
        st: &Setup,
        combos: &Combos,
    ) -> Result<Option<Vec<V>>, Fail> {
        let t = st.t;
        let mut deps = Vec::new();
        if !self.row_deps(body, t, s.env, &mut deps, 0)? {
            return Ok(None);
        }
        let body_parts = self.body_parts(fc, t, body, s.env, &deps)?;
        let mut parts: Vec<KPart> = st.cell_parts.clone();
        for (c, _, l) in &body_parts {
            let l = match l {
                Some(l) => Arc::clone(l),
                None => match self.group_lut(fc, t, *c)? {
                    Some(l) if l.map.is_none() => l,
                    _ => return Ok(None),
                },
            };
            parts.push((*c, l));
        }
        let Some(fused) = self.fuse(fc, t, &parts)? else {
            return Ok(None);
        };
        let mut datas: Vec<Arc<ColData>> = Vec::with_capacity(fused.keys.len());
        for (_, l) in &fused.keys {
            datas.push(self.col(l.col)?);
        }
        let cards: Vec<usize> = fused.keys.iter().map(|(_, l)| l.card).collect();
        let p = match &st.filtered {
            Some(p) => p.clone(),
            None => {
                let base_fc = FCtx {
                    filters: st.base.clone(),
                    rels: fc.rels.clone(),
                    applied: fc.applied,
                    memo: Default::default(),
                };
                self.pred(&base_fc, t)?.unwrap_or_else(|| PredSet::all(st.n))
            }
        };
        self.tick(st.n)?;
        let tally = {
            let key_parts: Vec<scan::KeyPart<'_>> = fused
                .keys
                .iter()
                .zip(&datas)
                .map(|((_, l), d)| scan::KeyPart {
                    codes: d.codes.col(),
                    lut: l.map.as_deref(),
                    card: l.card,
                })
                .collect();
            let Some(key) = scan::Key::new(key_parts) else {
                return Ok(None);
            };
            if key.groups() > MAX_TALLY_GROUPS {
                return Ok(None);
            }
            let sel = p.selection();
            let (rows, g) = (sel.rows, key.groups());
            let (_states, routed) =
                self.grouped_scan_charge("a tally of cells and combinations", rows, g, 12)?;
            let Some(tally) = scan::scan_groups_within(&sel, &key, None, 12, routed, scan::Tally::new) else {
                return internal();
            };
            tally
        };
        self.tick(tally.counts.len())?;
        // (cell, body combination) of each key group met: the first row and the rows of each
        let ncell_parts = st.cell_parts.len();
        let nbody = body_parts.len();
        let mut combo_of: HashMap<Vec<u32>, usize> = HashMap::new();
        let mut combo_first: Vec<u32> = Vec::new();
        let mut pairs: HashMap<(usize, usize), (u32, u64)> = HashMap::new();
        let mut kc = vec![0u32; cards.len()];
        for (g, (cnt, first)) in tally.counts.iter().zip(&tally.first).enumerate() {
            if *cnt == 0 || *first == u32::MAX {
                continue;
            }
            decode(&cards, g, &mut kc);
            let Some(cell) = st.cells.of_codes((0..ncell_parts).map(|i| fused.code(i, &kc))) else {
                continue;
            };
            let bc: Vec<u32> = (0..nbody)
                .map(|i| fused.code(ncell_parts.saturating_add(i), &kc))
                .collect();
            let next = combo_first.len();
            let ci = *combo_of.entry(bc).or_insert(next);
            if ci == next {
                combo_first.push(*first);
            } else if let Some(f0) = combo_first.get_mut(ci) {
                *f0 = (*f0).min(*first);
            }
            let e = pairs.entry((cell, ci)).or_insert((u32::MAX, 0));
            e.0 = e.0.min(*first);
            e.1 = e.1.saturating_add(*cnt);
        }
        drop(combo_of);
        // the body once per combination, on its first row (it reads only the combination's
        // values); an error: the cells one by one, which report it as before
        let mut order: Vec<(u32, usize)> = combo_first.iter().enumerate().map(|(i, r)| (*r, i)).collect();
        order.sort_unstable();
        let rep_tab = Tab::rows_of(t, order.iter().map(|(r, _)| *r).collect());
        let inner = Scope { fc, rc: &[], ..*s };
        let Ok(vals) = self.each_with(&rep_tab, &inner, |eng, sc| eng.scalar(body, sc)) else {
            return Ok(None);
        };
        let mut val_of: Vec<Option<V>> = vec![None; combo_first.len()];
        for ((_, i), v) in order.iter().zip(vals) {
            if let Some(slot) = val_of.get_mut(*i) {
                *slot = Some(v);
            }
        }
        // each cell's combinations in the order of their first rows
        let mut by_cell: Vec<Vec<(u32, u64, usize)>> = Vec::new();
        let mut cell_slot: HashMap<usize, usize> = HashMap::new();
        for ((cell, ci), (first, cnt)) in pairs {
            let next = by_cell.len();
            let slot = *cell_slot.entry(cell).or_insert(next);
            if slot == next {
                by_cell.push(Vec::new());
            }
            if let Some(v) = by_cell.get_mut(slot) {
                v.push((first, cnt, ci));
            }
        }
        let mut cell_vals: HashMap<usize, V> = HashMap::with_capacity(cell_slot.len());
        for (cell, slot) in cell_slot {
            let Some(list) = by_cell.get_mut(slot) else {
                continue;
            };
            list.sort_unstable();
            let mut vs: Vec<(V, u64)> = Vec::with_capacity(list.len());
            for (_, cnt, ci) in list.iter() {
                match val_of.get(*ci).cloned().flatten() {
                    Some(v) => vs.push((v, *cnt)),
                    None => return internal(),
                }
            }
            // an error (a text summed): the cells one by one, which report the reference's first
            let Ok(v) = self.weighted(f, vs) else {
                return Ok(None);
            };
            cell_vals.insert(cell, v);
        }
        self.tick(combos.len())?;
        let out = combos
            .iter()
            .map(|combo| {
                st.cells
                    .of_combo(combo)
                    .and_then(|c| cell_vals.get(&c).cloned())
                    .unwrap_or(V::Blank)
            })
            .collect();
        hit(1);
        Ok(Some(out))
    }
}

#[cfg(test)]
thread_local! {
    /// Results given by (exact products, weighted combinations), for the tests.
    static HITS: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

/// One more result of the exact products (`0`) or weighted (`1`) path.
#[cfg(test)]
fn hit(path: usize) {
    HITS.with(|h| {
        let (a, b) = h.get();
        h.set(if path == 0 { (a + 1, b) } else { (a, b + 1) });
    });
}

#[cfg(not(test))]
fn hit(_path: usize) {}

/// Results given so far on this thread by (exact products, weighted combinations).
#[cfg(test)]
pub(in crate::preview::general) fn iterx_hits() -> (usize, usize) {
    HITS.with(|h| h.get())
}

#[cfg(test)]
thread_local! {
    /// The path turned off (the cells one by one), for the tests.
    static OFF: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Turns the path off (`true`) or on, on this thread (tests).
#[cfg(test)]
pub(in crate::preview::general) fn set_iterx_off(off: bool) {
    OFF.with(|o| o.set(off));
}

#[cfg(test)]
fn off() -> bool {
    OFF.with(|o| o.get())
}

#[cfg(not(test))]
fn off() -> bool {
    false
}
