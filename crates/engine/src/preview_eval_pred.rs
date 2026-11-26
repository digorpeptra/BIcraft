//! Row predicates over a one-column table of values, evaluated without the formula engine's
//! per-row machinery (`docs/storage-engine.md`, "Filters over a big dimension").
//!
//! A slicer or a `CALCULATE` filter on a column with millions of distinct values (a customer
//! key, a date-time) is a `FILTER` over the column's values: `CALCULATE([CA], Client[ClientKey]
//! <= n)` is `FILTER(ALL(Client[ClientKey]), Client[ClientKey] <= n)`, Desktop's between slicer
//! `FILTER(KEEPFILTERS(VALUES(Client[ClientKey])), AND(… >= a, … <= b))`, and "customers over a
//! threshold" `FILTER(VALUES(Client[ClientKey]), [CA] > x)`. The reference evaluates the
//! predicate once per value through a row context (and a context transition per value for a
//! measure). When the predicate is made only of the pieces below, it is compiled here into a
//! small tree evaluated per value with the evaluator's own comparison and truth functions
//! (`compare_vals`, `strictly_equal`, `truth`): the same answer by construction.
//!
//! - **Compiled**: the column itself; a measure that is one aggregation, computed for every value
//!   at once by one grouped scan ([`Engine::each_direct`], what the reference's context
//!   transition gives each row); constants (an expression that reads no column and is pure,
//!   [`Engine::row_deps`], evaluated once); `=`, `==`, `<>`, `<`, `<=`, `>`, `>=`; `&&`, `||`
//!   (short-circuit, as the evaluator); `AND`, `OR`, `NOT`; `x IN { constants }`.
//! - **Not compiled** (the reference runs): another column, any other measure, an aggregation,
//!   anything else; and any evaluation error, so that the reference reports it (or not, when it
//!   stops earlier) exactly as before.

use std::borrow::Cow;
use std::cmp::Ordering;

use bicraft_dax::ast::Expr;
use bicraft_vertipaq::CellValue;

use super::{ColId, Engine, Fail, Scope, Tab, TabId, V, compare_vals, strictly_equal, subset, truth};

/// The blank value a measure gives a row it has no value for.
static BLANK: V = V::Blank;

/// Most nested operators compiled.
const MAX_DEPTH: usize = 64;

/// Fewest values for which the compiled path is taken (below, the reference is as fast).
const MIN_ROWS: usize = 64;

/// A compiled predicate (or value) over a row: its value `x` of the column, its position `i`.
#[derive(Debug, Clone)]
pub(super) enum P {
    Col,
    Const(V),
    /// A measure's value for each row (by position).
    Each(Vec<V>),
    Cmp(Cmp, Box<P>, Box<P>),
    /// `&&` / `||`: the right side only when the left side does not decide.
    AndThen(Box<P>, Box<P>),
    OrElse(Box<P>, Box<P>),
    /// `AND()` / `OR()`: both sides evaluated.
    And(Box<P>, Box<P>),
    Or(Box<P>, Box<P>),
    Not(Box<P>),
    In(Box<P>, Vec<V>),
}

/// Two compiled operands.
type Pair = (Box<P>, Box<P>);

#[derive(Debug, Clone, Copy)]
pub(super) enum Cmp {
    Eq,
    StrictEq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl P {
    /// An operand's value for row `i`, whose column value is `x`, borrowed when it is the
    /// column, a constant or a measure's value (a comparison's result is `TRUE` / `FALSE`, as
    /// the evaluator's operators return it).
    fn operand<'a>(&'a self, i: usize, x: &'a V) -> Result<Cow<'a, V>, Fail> {
        Ok(match self {
            P::Col => Cow::Borrowed(x),
            P::Const(v) => Cow::Borrowed(v),
            P::Each(v) => Cow::Borrowed(v.get(i).unwrap_or(&BLANK)),
            other => Cow::Owned(V::Bool(other.holds(i, x)?)),
        })
    }

    /// Whether the predicate holds for row `i` (its value's truth), as the evaluator computes it:
    /// comparisons with `compare_vals` / `strictly_equal`, `&&` and `||` short-circuit, `AND`
    /// and `OR` evaluate both sides.
    pub(super) fn holds(&self, i: usize, x: &V) -> Result<bool, Fail> {
        match self {
            P::Col | P::Const(_) | P::Each(_) => truth(self.operand(i, x)?.as_ref()),
            P::Cmp(op, a, b) => {
                let (a, b) = (a.operand(i, x)?, b.operand(i, x)?);
                cmp(*op, &a, &b)
            }
            P::AndThen(a, b) => Ok(a.holds(i, x)? && b.holds(i, x)?),
            P::OrElse(a, b) => Ok(a.holds(i, x)? || b.holds(i, x)?),
            P::And(a, b) => {
                let (a, b) = (a.holds(i, x)?, b.holds(i, x)?);
                Ok(a && b)
            }
            P::Or(a, b) => {
                let (a, b) = (a.holds(i, x)?, b.holds(i, x)?);
                Ok(a || b)
            }
            P::Not(a) => Ok(!a.holds(i, x)?),
            P::In(a, set) => {
                let a = a.operand(i, x)?;
                for v in set {
                    if strictly_equal(&a, v)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
        }
    }

    /// Whether the row is read at all (a constant predicate is left to the reference).
    fn reads_row(&self) -> bool {
        match self {
            P::Col | P::Each(_) => true,
            P::Const(_) => false,
            P::Cmp(_, a, b) | P::AndThen(a, b) | P::OrElse(a, b) | P::And(a, b) | P::Or(a, b) => {
                a.reads_row() || b.reads_row()
            }
            P::Not(a) | P::In(a, _) => a.reads_row(),
        }
    }
}

/// `a op b` as the evaluator's binary operators compute it.
fn cmp(op: Cmp, a: &V, b: &V) -> Result<bool, Fail> {
    if let Cmp::StrictEq = op {
        return strictly_equal(a, b);
    }
    let o = compare_vals(a, b)?;
    Ok(match op {
        Cmp::Eq | Cmp::StrictEq => o == Ordering::Equal,
        Cmp::Ne => o != Ordering::Equal,
        Cmp::Lt => o == Ordering::Less,
        Cmp::Le => o != Ordering::Greater,
        Cmp::Gt => o == Ordering::Greater,
        Cmp::Ge => o != Ordering::Less,
    })
}

/// Values per task of [`picks`].
const PICK_CHUNK: usize = 1 << 15;

/// The positions `0..n` where `holds` is true, in order, on the worker pool for many values;
/// `None` when `holds` fails somewhere (`None` for a position).
fn picks(n: usize, holds: impl Fn(usize) -> Option<bool> + Sync) -> Option<Vec<usize>> {
    let chunk = |k: usize| -> Option<Vec<usize>> {
        let lo = k.saturating_mul(PICK_CHUNK);
        let hi = lo.saturating_add(PICK_CHUNK).min(n);
        let mut out = Vec::new();
        for i in lo..hi {
            if holds(i)? {
                out.push(i);
            }
        }
        Some(out)
    };
    let tasks = n.div_ceil(PICK_CHUNK);
    if tasks <= 1 {
        return chunk(0);
    }
    let parts = bicraft_scan::pool::map(tasks, chunk)?;
    let mut out = Vec::new();
    for p in parts {
        out.extend(p?);
    }
    Some(out)
}

/// What a predicate is compiled against.
struct On<'t> {
    /// The table filtered (`None`: the column's codes, without a table of values).
    tab: Option<&'t Tab>,
    /// Its one column's lineage, and that column's table.
    c: ColId,
    t: TabId,
}

impl Engine<'_, '_> {
    /// `FILTER` over a one-column table of values (`VALUES(c)`, `ALL(c)`, the values a boolean
    /// `CALCULATE` filter ranges over) whose predicate compiles: the rows where it holds, as the
    /// reference picks them. `None`: the reference path.
    pub(super) fn filter_values(&mut self, tab: &Tab, pred: &Expr, s: &Scope) -> Result<Option<Tab>, Fail> {
        let Tab::Tuples { cols, rows } = tab else {
            return Ok(None);
        };
        let [col] = cols.as_slice() else {
            return Ok(None);
        };
        let (Some(c), None) = (col.lin, &col.name) else {
            return Ok(None);
        };
        if rows.len() < MIN_ROWS || self.store.is_none() {
            return Ok(None);
        }
        let on = On {
            tab: Some(tab),
            c,
            t: self.col_table(c)?,
        };
        let Some(p) = self.compile(pred, &on, s, 0)?.filter(P::reads_row) else {
            return Ok(None);
        };
        self.tick(rows.len())?;
        // an error is left to the reference (its own message, or none when it stops earlier)
        let picked = picks(rows.len(), |i| p.holds(i, rows.get(i)?.first()?).ok());
        let Some(picked) = picked else {
            return Ok(None);
        };
        let out = subset(tab, &picked);
        if let Some(codes) = self.tuple_codes(tab, c) {
            let kept: Vec<u32> = picked.iter().filter_map(|i| codes.get(*i).copied()).collect();
            self.note_tuple_codes(&out, c, kept)?;
        }
        Ok(Some(out))
    }

    fn compile(&mut self, e: &Expr, on: &On<'_>, s: &Scope, depth: usize) -> Result<Option<P>, Fail> {
        if depth > MAX_DEPTH {
            return Ok(None);
        }
        let d = depth.saturating_add(1);
        let two = |eng: &mut Self, a: &Expr, b: &Expr| -> Result<Option<Pair>, Fail> {
            let Some(a) = eng.compile(a, on, s, d)? else {
                return Ok(None);
            };
            let Some(b) = eng.compile(b, on, s, d)? else {
                return Ok(None);
            };
            Ok(Some((Box::new(a), Box::new(b))))
        };
        match e {
            Expr::Column {
                table,
                name,
                variation: None,
                ..
            } => {
                if self.measure_def(table.as_deref(), name).is_some() {
                    // the measure for every row at once, as each row's context transition
                    // (a failure is left to the reference, which may not evaluate it at all)
                    let Some(tab) = on.tab else {
                        return Ok(None);
                    };
                    return Ok(self.each_direct(tab, e, s).ok().flatten().map(P::Each));
                }
                let Some(table) = table else {
                    return Ok(None);
                };
                return Ok((self.col_id(table, name) == Some(on.c)).then_some(P::Col));
            }
            Expr::Binary { op, lhs, rhs, .. } => {
                let op = op.to_ascii_uppercase();
                let cmp = match op.as_str() {
                    "=" => Some(Cmp::Eq),
                    "==" => Some(Cmp::StrictEq),
                    "<>" => Some(Cmp::Ne),
                    "<" => Some(Cmp::Lt),
                    "<=" => Some(Cmp::Le),
                    ">" => Some(Cmp::Gt),
                    ">=" => Some(Cmp::Ge),
                    _ => None,
                };
                if let Some(k) = cmp {
                    return Ok(two(self, lhs, rhs)?.map(|(a, b)| P::Cmp(k, a, b)));
                }
                match op.as_str() {
                    "&&" => return Ok(two(self, lhs, rhs)?.map(|(a, b)| P::AndThen(a, b))),
                    "||" => return Ok(two(self, lhs, rhs)?.map(|(a, b)| P::OrElse(a, b))),
                    "IN" => {
                        let Expr::TableCtor { rows, .. } = rhs.as_ref() else {
                            return Ok(None);
                        };
                        if matches!(lhs.as_ref(), Expr::Tuple { .. }) {
                            return Ok(None);
                        }
                        let Some(a) = self.compile(lhs, on, s, d)? else {
                            return Ok(None);
                        };
                        let mut set = Vec::with_capacity(rows.len());
                        for r in rows {
                            if matches!(r, Expr::Tuple { .. }) {
                                return Ok(None);
                            }
                            match self.constant(r, on.t, s)? {
                                Some(v) => set.push(v),
                                None => return Ok(None),
                            }
                        }
                        return Ok(Some(P::In(Box::new(a), set)));
                    }
                    _ => {}
                }
            }
            Expr::Call { name, args, .. } => {
                let up = name.to_ascii_uppercase();
                let arg = |i: usize| args.get(i).and_then(Option::as_ref);
                match (up.as_str(), args.len()) {
                    ("NOT", 1) => {
                        let Some(a) = arg(0) else { return Ok(None) };
                        return Ok(self.compile(a, on, s, d)?.map(|a| P::Not(Box::new(a))));
                    }
                    ("AND" | "OR", 2) => {
                        let (Some(a), Some(b)) = (arg(0), arg(1)) else {
                            return Ok(None);
                        };
                        return Ok(two(self, a, b)?
                            .map(|(a, b)| if up == "AND" { P::And(a, b) } else { P::Or(a, b) }));
                    }
                    _ => {}
                }
            }
            Expr::Unary { op, operand, .. } if op.eq_ignore_ascii_case("NOT") => {
                return Ok(self.compile(operand, on, s, d)?.map(|a| P::Not(Box::new(a))));
            }
            _ => {}
        }
        Ok(self.constant(e, on.t, s)?.map(P::Const))
    }

    /// The value of an expression that reads no column and is pure, evaluated once; `None`:
    /// not one, or its evaluation fails (left to the reference).
    fn constant(&mut self, e: &Expr, t: TabId, s: &Scope) -> Result<Option<V>, Fail> {
        let mut deps = Vec::new();
        if !self.row_deps(e, t, s.env, &mut deps, 0)? || !deps.is_empty() {
            return Ok(None);
        }
        Ok(self.scalar(e, s).ok())
    }
}

impl Engine<'_, '_> {
    /// A `CALCULATE` filter argument over one column's values, straight from its codes: a
    /// boolean predicate on one column (`FILTER(ALL(c), pred)`), or `FILTER(src, pred)` with
    /// `src` one of `VALUES(c)`, `DISTINCT(c)`, `ALL(c)`, `KEEPFILTERS(VALUES(c))`, whose
    /// predicate compiles without a measure. The values are not materialized as a table, and no
    /// value is looked up again by its key: each code of the domain is kept when the predicate
    /// holds for its value, which is the filter the reference builds from its table of values
    /// whenever two codes never share a key (checked on the column's value index). `None`: the
    /// reference path.
    pub(super) fn filter_codes_arg(&mut self, e: &Expr, s: &Scope) -> Result<Option<super::Filter>, Fail> {
        if self.store.is_none() {
            return Ok(None);
        }
        // the column, its domain (which codes the reference's table of values holds) and the
        // predicate
        let (c, domain_fc, blank_row, pred): (ColId, Option<&super::FCtx>, bool, &Expr) =
            if self.is_table_expr(e, s.env) {
                let Expr::Call { name, args, .. } = e else {
                    return Ok(None);
                };
                if !name.eq_ignore_ascii_case("FILTER") {
                    return Ok(None);
                }
                let (Some(Some(src)), Some(Some(pred)), 2) = (args.first(), args.get(1), args.len()) else {
                    return Ok(None);
                };
                let src = match src {
                    Expr::Call { name, args, .. }
                        if name.eq_ignore_ascii_case("KEEPFILTERS") && args.len() == 1 =>
                    {
                        match args.first() {
                            Some(Some(inner @ Expr::Call { name, .. }))
                                if name.eq_ignore_ascii_case("VALUES") =>
                            {
                                inner
                            }
                            _ => return Ok(None),
                        }
                    }
                    other => other,
                };
                let Expr::Call { name: f, args: a, .. } = src else {
                    return Ok(None);
                };
                let Some(Some(col @ Expr::Column { table: Some(_), .. })) = a.first() else {
                    return Ok(None);
                };
                if a.len() != 1 {
                    return Ok(None);
                }
                let Some(c) = self.col_ref(col) else {
                    return Ok(None);
                };
                match f.to_ascii_uppercase().as_str() {
                    "VALUES" => (c, Some(s.fc), true, pred),
                    "DISTINCT" => (c, Some(s.fc), false, pred),
                    "ALL" => (c, None, true, pred),
                    _ => return Ok(None),
                }
            } else {
                let mut found = Vec::new();
                self.predicate_cols(e, &mut found, 0)?;
                let [c] = found.as_slice() else {
                    return Ok(None);
                };
                (*c, None, true, e)
            };
        let d = self.col(c)?;
        if d.values.len() < MIN_ROWS {
            return Ok(None);
        }
        let on = On {
            tab: None,
            c,
            t: self.col_table(c)?,
        };
        let Some(p) = self.compile(pred, &on, s, 0)?.filter(P::reads_row) else {
            return Ok(None);
        };
        // every value has its own key: the reference's lookup by key gives back its code
        let index = self.index(c)?;
        let keyed = d
            .values
            .iter()
            .skip(1)
            .filter(|v| !matches!(v, CellValue::Null))
            .count();
        if index.len() != keyed {
            return Ok(None);
        }
        let none = super::FCtx::default();
        let domain = self.values_codes(domain_fc.unwrap_or(&none), c, blank_row)?;
        self.tick(domain.len())?;
        let kept = picks(domain.len(), |i| {
            let code = domain.get(i).copied().unwrap_or(0);
            let x = d.values.get(code as usize).map_or(V::Blank, V::of_cell);
            p.holds(0, &x).ok()
        });
        let Some(kept) = kept else {
            return Ok(None);
        };
        let codes: Vec<u32> = kept.iter().filter_map(|i| domain.get(*i).copied()).collect();
        self.codes_filter(c, &codes).map(Some)
    }

    /// The model column a qualified column reference names (not a measure).
    fn col_ref(&mut self, e: &Expr) -> Option<ColId> {
        let Expr::Column {
            table: Some(table),
            name,
            variation: None,
            ..
        } = e
        else {
            return None;
        };
        if self.measure_def(Some(table), name).is_some() {
            return None;
        }
        self.col_id(table, name)
    }
}

impl Engine<'_, '_> {
    /// Whether every value of `c` has its own key: a lookup of a value by its key (`code_of`)
    /// gives back its code.
    pub(super) fn own_keys(&mut self, c: ColId) -> Result<bool, Fail> {
        let d = self.col(c)?;
        let index = self.index(c)?;
        let keyed = d
            .values
            .iter()
            .skip(1)
            .filter(|v| !matches!(v, CellValue::Null))
            .count();
        Ok(index.len() == keyed)
    }

    /// Records the codes of a one-column table of values of `c` built from them (for this
    /// evaluation), when every value of `c` has its own key: later lookups of its values by key
    /// read them instead.
    pub(super) fn note_tuple_codes(&mut self, t: &Tab, c: ColId, codes: Vec<u32>) -> Result<(), Fail> {
        let Tab::Tuples { rows, .. } = t else {
            return Ok(());
        };
        if self.store.is_none() || codes.len() != rows.len() || rows.len() < MIN_ROWS || !self.own_keys(c)? {
            return Ok(());
        }
        if let Some(s) = self.store.as_mut() {
            s.note_tuple_codes(rows, c, codes);
        }
        Ok(())
    }

    /// The codes recorded for a one-column table of values of `c` ([`Engine::note_tuple_codes`]).
    pub(super) fn tuple_codes(&self, t: &Tab, c: ColId) -> Option<std::sync::Arc<Vec<u32>>> {
        let Tab::Tuples { rows, .. } = t else {
            return None;
        };
        self.store.as_ref()?.tuple_codes_of(rows, c)
    }
}
