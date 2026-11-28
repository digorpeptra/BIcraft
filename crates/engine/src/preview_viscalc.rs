//! Visual calculations and hidden fields, applied to a visual's evaluated data
//! (`docs/formats/visual-calculations.md`; semantics from Microsoft Learn, "Visual calculations
//! overview" and the DAX reference pages of `RUNNINGSUM`, `MOVINGAVERAGE`, `PREVIOUS`, `NEXT`,
//! `FIRST`, `LAST`, `RANGE`, `COLLAPSE`, `COLLAPSEALL`, `EXPAND`, `EXPANDALL`, `ISATLEVEL`,
//! `LOOKUP`, `LOOKUPWITHTOTALS`, `OFFSET`, `INDEX`, `WINDOW`, `RANK`, `ROWNUMBER`).
//!
//! The visual is evaluated without its calculations ([`split`]); [`apply`] then builds the
//! visual matrix from the result: its levels are the combinations of the row-axis and column-axis
//! fields kept (the data rows are the deepest level, a table's total row the top one, a matrix's
//! subtotals the levels between; other levels a calculation navigates to are evaluated like
//! "Show value as" totals). Each calculation is evaluated at every cell of every level, in the
//! order of their references to each other, and its column is inserted where the projection
//! stands. Hidden fields stay in the matrix and are flagged `hidden` in the data's columns (the
//! renderer leaves them out).

use std::cmp::Ordering;
use std::collections::HashMap;

use bicraft_dax::ast::Expr;
use bicraft_dax::visual_calc;
use bicraft_report::query::{self, projection_meta, visual_calc as calc_of};
use bicraft_report::{FieldRef, Visual};
use serde_json::{Map, Value, json};

use super::{Ctx, Item, matrix, out_json, show};

/// Most levels of the visual matrix evaluated for navigation (`COLLAPSE`, `EXPAND`…).
const MAX_LEVELS: usize = 36;
/// Nesting limit of an evaluated expression.
const MAX_DEPTH: usize = 96;
/// Most cells one function call visits (a running sum over a long axis is quadratic).
const MAX_VISITS: usize = 20_000_000;

/// One visual calculation of a visual.
#[derive(Debug, Clone)]
pub(super) struct Calc {
    /// Its position among the visual's fields.
    at: usize,
    name: String,
    expression: String,
}

/// A visual with visual calculations or hidden fields, split for evaluation.
#[derive(Debug, Clone)]
pub(super) struct Split {
    /// The visual without its calculations (what the evaluator computes).
    pub(super) base: Visual,
    /// Per field of the visual: its position in `base`, `None` for a calculation.
    map: Vec<Option<usize>>,
    calcs: Vec<Calc>,
    /// Per field: the names a calculation refers to it by (its `nativeQueryRef`, its display
    /// name, its default name), the first one canonical.
    names: Vec<Vec<String>>,
    hidden: Vec<bool>,
    roles: Vec<String>,
}

/// The split of `v`, `None` when it has neither visual calculations nor hidden fields.
pub(super) fn split(v: &Visual) -> Option<Split> {
    let q = query::effective(v);
    let meta = projection_meta(q.as_ref());
    let aligned = meta.len() == v.fields.len();
    let hidden: Vec<bool> = (0..v.fields.len())
        .map(|i| aligned && meta.get(i).is_some_and(|m| m.1))
        .collect();
    let calcs: Vec<Calc> = v
        .fields
        .iter()
        .enumerate()
        .filter_map(|(at, b)| {
            calc_of(&b.field).map(|c| Calc {
                at,
                name: c.name,
                expression: c.expression,
            })
        })
        .collect();
    if calcs.is_empty() && !hidden.iter().any(|h| *h) {
        return None;
    }
    let keep: Vec<bool> = v.fields.iter().map(|b| calc_of(&b.field).is_none()).collect();
    let mut base = matrix::with_fields(v, &keep);
    if let Some(q) = &q
        && !v.fields.is_empty()
    {
        // the base keeps the query of the bindings it keeps
        let mut full = v.clone();
        full.visual_extra.insert("query".into(), q.clone());
        base = matrix::with_fields(&full, &keep);
    }
    let mut k = 0usize;
    let map = keep
        .iter()
        .map(|kept| {
            kept.then(|| {
                let i = k;
                k = k.saturating_add(1);
                i
            })
        })
        .collect();
    let names = crate::visual_calcs::field_names(v);
    Some(Split {
        base,
        map,
        calcs,
        names,
        hidden,
        roles: v.fields.iter().map(|b| b.role.clone()).collect(),
    })
}

// ---------------------------------------------------------------------------------------------
// values
// ---------------------------------------------------------------------------------------------

/// A value of the visual matrix.
#[derive(Debug, Clone, PartialEq)]
enum V {
    Blank,
    Num(f64),
    Text(String),
    Bool(bool),
}

impl V {
    fn of(j: &Value) -> V {
        match j {
            Value::Null => V::Blank,
            Value::Bool(b) => V::Bool(*b),
            Value::Number(n) => n.as_f64().map_or(V::Blank, V::Num),
            Value::String(s) => V::Text(s.clone()),
            other => V::Text(other.to_string()),
        }
    }

    fn json(&self) -> Value {
        match self {
            V::Blank => Value::Null,
            V::Num(x) if x.is_finite() => json!(x),
            V::Num(_) => Value::Null,
            V::Text(s) => json!(s),
            V::Bool(b) => json!(b),
        }
    }

    fn num(&self) -> Option<f64> {
        match self {
            V::Num(x) => Some(*x),
            V::Bool(b) => Some(f64::from(u8::from(*b))),
            V::Text(s) => s.trim().parse().ok(),
            V::Blank => None,
        }
    }

    fn truthy(&self) -> Result<bool, Fail> {
        match self {
            V::Blank => Ok(false),
            V::Bool(b) => Ok(*b),
            V::Num(x) => Ok(*x != 0.0),
            V::Text(s) => match s.to_ascii_lowercase().as_str() {
                "true" => Ok(true),
                "false" => Ok(false),
                _ => Err(Fail::Error(format!("cannot convert \"{s}\" to true/false"))),
            },
        }
    }

    fn text(&self) -> String {
        match self {
            V::Blank => String::new(),
            V::Num(x) => {
                if x.fract() == 0.0 && x.abs() < 1e15 {
                    format!("{x:.0}")
                } else {
                    x.to_string()
                }
            }
            V::Text(s) => s.clone(),
            V::Bool(b) => if *b { "True" } else { "False" }.to_string(),
        }
    }

    /// A key for equality (case-insensitive text, as DAX compares).
    fn key(&self) -> String {
        match self {
            V::Blank => "b".into(),
            V::Num(x) => format!("n{x}"),
            V::Text(s) => format!("s{}", s.to_lowercase()),
            V::Bool(b) => format!("l{b}"),
        }
    }
}

/// Why a calculation (or one cell of it) has no value.
#[derive(Debug, Clone, PartialEq)]
enum Fail {
    /// A construct BIcraft does not evaluate: the whole calculation is left blank.
    Unsupported(String),
    /// An error in this cell (it is blank).
    Error(String),
}

fn unsupported<T>(why: impl Into<String>) -> Result<T, Fail> {
    Err(Fail::Unsupported(why.into()))
}

/// Ascending order of axis keys: blank numbers between the negative numbers and zero, blank text
/// before every text (`DEFAULT`), or blanks first / last.
fn cmp_v(a: &V, b: &V, blanks: Blanks) -> Ordering {
    match (a, b) {
        (V::Blank, V::Blank) => Ordering::Equal,
        (V::Blank, other) | (other, V::Blank) => {
            let blank_first = match blanks {
                Blanks::First => true,
                Blanks::Last => false,
                Blanks::Default => !matches!(other, V::Num(x) if *x < 0.0),
            };
            let o = if blank_first {
                Ordering::Less
            } else {
                Ordering::Greater
            };
            if matches!(a, V::Blank) { o } else { o.reverse() }
        }
        (V::Num(x), V::Num(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (V::Text(x), V::Text(y)) => x.to_lowercase().cmp(&y.to_lowercase()).then_with(|| x.cmp(y)),
        (V::Bool(x), V::Bool(y)) => x.cmp(y),
        (x, y) => rank(x).cmp(&rank(y)),
    }
}

fn rank(v: &V) -> u8 {
    match v {
        V::Blank => 0,
        V::Num(_) => 1,
        V::Text(_) => 2,
        V::Bool(_) => 3,
    }
}

// ---------------------------------------------------------------------------------------------
// the visual matrix
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ax {
    Rows,
    Cols,
    /// The small multiples (`ROWPAGES`): kept by the visual's own cells, walked only when named.
    Pages,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Blanks {
    Default,
    First,
    Last,
}

/// A level: how many fields of the row axis, of the column axis and of the small multiples it
/// keeps. The visual's own cells keep every small multiple; `COLLAPSE` along `ROWPAGES` and
/// `LOOKUPWITHTOTALS` reach the levels across them.
type Lv = (usize, usize, usize);

#[derive(Debug, Clone, Default)]
struct Level {
    rows: Vec<Vec<V>>,
    index: HashMap<String, usize>,
}

/// The visual matrix: the fields' positions on each axis and the levels' cells (one row of
/// every field per cell).
#[derive(Debug, Clone, Default)]
struct Matrix {
    /// Row-axis grouping fields, top level first (positions among the visual's fields).
    rows: Vec<usize>,
    /// Column-axis grouping fields.
    cols: Vec<usize>,
    /// Small-multiple fields: the visual's cells keep them, a walk partitions by them unless it
    /// walks `ROWPAGES`.
    pages: Vec<usize>,
    /// Per field: the names it is referred to by.
    names: Vec<Vec<String>>,
    levels: HashMap<Lv, Level>,
    /// Per grouping field whose model column has a sort-by column: the sort-by value of each of
    /// its values (by [`V::key`]); the axes are ordered by it.
    sort_keys: HashMap<usize, HashMap<String, V>>,
}

impl Matrix {
    /// How many fields of axis `ax` level `lv` keeps.
    fn kept_on(&self, lv: Lv, ax: Ax) -> usize {
        level_of(lv, ax)
    }

    /// Field `f` of cell (`lv`, `row`) as the axis orders it: its sort-by value when it has one.
    fn sorted(&self, lv: Lv, row: usize, f: usize) -> V {
        let v = self.cell(lv, row, f);
        self.sort_keys
            .get(&f)
            .and_then(|m| m.get(&v.key()))
            .cloned()
            .unwrap_or(v)
    }

    fn kept(&self, lv: Lv) -> Vec<usize> {
        self.pages
            .iter()
            .take(lv.2)
            .chain(self.rows.iter().take(lv.0))
            .chain(self.cols.iter().take(lv.1))
            .copied()
            .collect()
    }

    fn key(&self, lv: Lv, row: &[V]) -> String {
        self.kept(lv)
            .iter()
            .map(|i| row.get(*i).map_or_else(|| "b".to_string(), V::key))
            .collect::<Vec<_>>()
            .join("\u{1}")
    }

    fn add(&mut self, lv: Lv, rows: Vec<Vec<V>>) {
        if self.levels.contains_key(&lv) {
            return;
        }
        let mut level = Level::default();
        for r in rows {
            let k = self.key(lv, &r);
            if !level.index.contains_key(&k) {
                level.index.insert(k, level.rows.len());
                level.rows.push(r);
            }
        }
        self.levels.insert(lv, level);
    }

    fn cell(&self, lv: Lv, row: usize, field: usize) -> V {
        self.levels
            .get(&lv)
            .and_then(|l| l.rows.get(row))
            .and_then(|r| r.get(field))
            .cloned()
            .unwrap_or(V::Blank)
    }

    fn row(&self, lv: Lv, row: usize) -> &[V] {
        self.levels
            .get(&lv)
            .and_then(|l| l.rows.get(row))
            .map_or(&[], Vec::as_slice)
    }

    fn depth(&self, ax: Ax) -> usize {
        match ax {
            Ax::Rows => self.rows.len(),
            Ax::Cols => self.cols.len(),
            Ax::Pages => self.pages.len(),
        }
    }

    fn axis_fields(&self, ax: Ax) -> &[usize] {
        match ax {
            Ax::Rows => &self.rows,
            Ax::Cols => &self.cols,
            Ax::Pages => &self.pages,
        }
    }

    /// The field named `name` (case-insensitive).
    fn field(&self, name: &str) -> Option<usize> {
        self.names
            .iter()
            .position(|n| n.iter().any(|x| x.eq_ignore_ascii_case(name)))
    }

    /// The axis and level (0-based) of grouping field `f` (a small multiple's on `ROWPAGES`).
    fn axis_of(&self, f: usize) -> Option<(Ax, usize)> {
        if let Some(k) = self.rows.iter().position(|x| *x == f) {
            return Some((Ax::Rows, k));
        }
        if let Some(k) = self.pages.iter().position(|x| *x == f) {
            return Some((Ax::Pages, k));
        }
        self.cols.iter().position(|x| *x == f).map(|k| (Ax::Cols, k))
    }

    /// The cell of level `to` that holds cell (`lv`, `row`) (`to` keeps fewer fields).
    fn ancestor(&self, lv: Lv, row: usize, to: Lv) -> Option<usize> {
        let k = self.key(to, self.row(lv, row));
        self.levels.get(&to)?.index.get(&k).copied()
    }

    /// The cells of level `to` under cell (`lv`, `row`) (`to` keeps more fields).
    fn descendants(&self, lv: Lv, row: usize, to: Lv) -> Vec<usize> {
        let k = self.key(lv, self.row(lv, row));
        let Some(level) = self.levels.get(&to) else {
            return Vec::new();
        };
        (0..level.rows.len())
            .filter(|r| level.rows.get(*r).is_some_and(|x| self.key(lv, x) == k))
            .collect()
    }
}

/// The depth of `lv` on axis `ax`.
fn level_of(lv: Lv, ax: Ax) -> usize {
    match ax {
        Ax::Rows => lv.0,
        Ax::Cols => lv.1,
        Ax::Pages => lv.2,
    }
}

/// `lv` at depth `d` on axis `ax`.
fn with_level(lv: Lv, ax: Ax, d: usize) -> Lv {
    match ax {
        Ax::Rows => (d, lv.1, lv.2),
        Ax::Cols => (lv.0, d, lv.2),
        Ax::Pages => (lv.0, lv.1, d),
    }
}

/// Where an expression is evaluated: one cell, or a set of cells of one level.
#[derive(Debug, Clone, PartialEq)]
enum Cx {
    Cell(Lv, usize),
    Set(Lv, Vec<usize>),
}

impl Cx {
    fn lv(&self) -> Lv {
        match self {
            Cx::Cell(lv, _) | Cx::Set(lv, _) => *lv,
        }
    }

    fn cells(&self) -> Vec<usize> {
        match self {
            Cx::Cell(_, r) => vec![*r],
            Cx::Set(_, rs) => rs.clone(),
        }
    }

    fn of(lv: Lv, mut rows: Vec<usize>) -> Cx {
        rows.dedup();
        match rows.as_slice() {
            [one] => Cx::Cell(lv, *one),
            _ => Cx::Set(lv, rows),
        }
    }
}

#[derive(Debug, Clone)]
enum Val {
    Scalar(V),
    Table(Cx),
}

/// How a window-like function walks an axis.
#[derive(Debug, Clone)]
struct Walk {
    axes: Vec<Ax>,
    blanks: Blanks,
    reset: Reset,
    /// `ORDERBY` keys and whether descending.
    order: Vec<(OrderKey, bool)>,
    /// `PARTITIONBY` fields.
    partition: Vec<usize>,
    /// `MATCHBY` fields: they must single out the current cell within its partition.
    matchby: Vec<usize>,
}

/// One key of an `ORDERBY`: a field of the visual, or an expression evaluated at each cell.
#[derive(Debug, Clone)]
enum OrderKey {
    Field(usize),
    Expr(Box<Expr>),
}

/// A partition walked in order: its cells, the current cell's position, and each cell's
/// ordering keys (the `ORDERBY` keys, else the walked axes' fields).
#[derive(Debug, Clone)]
struct Walked {
    cells: Vec<usize>,
    pos: usize,
    keys: Vec<Vec<V>>,
    desc: Vec<bool>,
    blanks: Blanks,
}

impl Walked {
    /// Whether cells `i` and `j` of the walk tie on their ordering keys.
    fn ties(&self, i: usize, j: usize) -> bool {
        match (self.keys.get(i), self.keys.get(j)) {
            (Some(a), Some(b)) => a
                .iter()
                .zip(b)
                .all(|(x, y)| cmp_v(x, y, self.blanks) == Ordering::Equal),
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Reset {
    None,
    /// Partition by the first `k` fields of the axis (and every field when the cell is at or
    /// above that level).
    Absolute(usize),
    /// Partition by the fields `k` or more levels above the cell's.
    Relative(usize),
    /// By a field of the axis.
    Field(usize),
}

struct Eval<'m> {
    m: &'m Matrix,
    visits: usize,
}

fn number(e: &Expr) -> Option<f64> {
    match e {
        Expr::Number { text, .. } => text.parse().ok(),
        Expr::Unary { op, operand, .. } if op == "-" => number(operand).map(|x| -x),
        Expr::Unary { op, operand, .. } if op == "+" => number(operand),
        _ => None,
    }
}

fn name_of(e: &Expr) -> Option<String> {
    match e {
        Expr::Name {
            name, quoted: false, ..
        } => Some(name.to_ascii_uppercase()),
        _ => None,
    }
}

fn axes_of(e: &Expr) -> Option<Vec<Ax>> {
    let n = name_of(e)?;
    if !visual_calc::is_axis(&n) {
        return None;
    }
    let mut out = Vec::new();
    for w in n.split_whitespace() {
        match w {
            "ROWS" => out.push(Ax::Rows),
            "COLUMNS" => out.push(Ax::Cols),
            "ROWPAGES" => out.push(Ax::Pages),
            _ => {}
        }
    }
    Some(out)
}

fn arg(args: &[Option<Expr>], i: usize) -> Option<&Expr> {
    args.get(i).and_then(Option::as_ref)
}

impl Eval<'_> {
    fn tick(&mut self, n: usize) -> Result<(), Fail> {
        self.visits = self.visits.saturating_add(n);
        if self.visits > MAX_VISITS {
            return unsupported(format!("more than {MAX_VISITS} cells visited"));
        }
        Ok(())
    }

    /// The default axis: ROWS, or COLUMNS when the visual has no row axis.
    fn default_axes(&self) -> Vec<Ax> {
        if self.m.rows.is_empty() && !self.m.cols.is_empty() {
            vec![Ax::Cols]
        } else {
            vec![Ax::Rows]
        }
    }

    /// The axis, blanks, `ORDERBY` and reset arguments of a window-like function (`args[from..]`),
    /// told apart by their shape; a number is the reset.
    fn walk_args(&self, args: &[Option<Expr>], from: usize) -> Result<Walk, Fail> {
        let mut w = Walk {
            axes: self.default_axes(),
            blanks: Blanks::Default,
            reset: Reset::None,
            order: Vec::new(),
            partition: Vec::new(),
            matchby: Vec::new(),
        };
        for a in args.iter().skip(from).flatten() {
            if let Some(ax) = axes_of(a) {
                w.axes = ax;
                continue;
            }
            if let Some(n) = number(a) {
                w.reset = if n > 0.0 {
                    Reset::Absolute(n as usize)
                } else if n < 0.0 {
                    Reset::Relative((-n) as usize)
                } else {
                    Reset::None
                };
                continue;
            }
            match a {
                Expr::Name { .. } => match name_of(a).as_deref() {
                    Some("NONE") => w.reset = Reset::None,
                    Some("HIGHESTPARENT") => w.reset = Reset::Absolute(1),
                    Some("LOWESTPARENT") => w.reset = Reset::Relative(1),
                    Some("DEFAULT" | "KEEP") => w.blanks = Blanks::Default,
                    Some("FIRST") => w.blanks = Blanks::First,
                    Some("LAST") => w.blanks = Blanks::Last,
                    _ => return unsupported("an argument of a visual calculation function"),
                },
                Expr::Column {
                    table: None, name, ..
                } => match self.m.field(name) {
                    Some(f) if self.m.axis_of(f).is_some() => w.reset = Reset::Field(f),
                    _ => return unsupported(format!("reset by [{name}], which is no axis field")),
                },
                Expr::Call { name, args, .. } if name.eq_ignore_ascii_case("ORDERBY") => {
                    let mut i = 0usize;
                    while let Some(e) = arg(args, i) {
                        let key = match e {
                            Expr::Column {
                                table: None, name, ..
                            } => OrderKey::Field(
                                self.m
                                    .field(name)
                                    .ok_or_else(|| Fail::Unsupported(format!("ORDERBY [{name}]")))?,
                            ),
                            other => OrderKey::Expr(Box::new(other.clone())),
                        };
                        let desc = arg(args, i.saturating_add(1))
                            .and_then(name_of)
                            .filter(|k| k == "ASC" || k == "DESC");
                        i = i.saturating_add(if desc.is_some() { 2 } else { 1 });
                        w.order.push((key, desc.as_deref() == Some("DESC")));
                    }
                }
                Expr::Call { name, args, .. } if name.eq_ignore_ascii_case("PARTITIONBY") => {
                    for e in args.iter().flatten() {
                        let Expr::Column {
                            table: None, name, ..
                        } = e
                        else {
                            return unsupported("PARTITIONBY of something else than a field of the visual");
                        };
                        let f = self
                            .m
                            .field(name)
                            .ok_or_else(|| Fail::Unsupported(format!("PARTITIONBY [{name}]")))?;
                        w.partition.push(f);
                    }
                }
                Expr::Call { name, args, .. } if name.eq_ignore_ascii_case("MATCHBY") => {
                    for e in args.iter().flatten() {
                        let Expr::Column {
                            table: None, name, ..
                        } = e
                        else {
                            return unsupported("MATCHBY of something else than a field of the visual");
                        };
                        let f = self
                            .m
                            .field(name)
                            .ok_or_else(|| Fail::Unsupported(format!("MATCHBY [{name}]")))?;
                        w.matchby.push(f);
                    }
                }
                _ => return unsupported("an argument of a visual calculation function"),
            }
        }
        // an axis that is not on the visual is ignored
        w.axes.retain(|a| self.m.depth(*a) > 0);
        Ok(w)
    }

    /// The cells of the partition of cell (`lv`, `row`) along `w`, in order, with the cell's
    /// position in it and every cell's ordering keys.
    fn walk(
        &mut self,
        lv: Lv,
        row: usize,
        w: &Walk,
        env: &[(String, Val)],
        depth: usize,
    ) -> Result<Walked, Fail> {
        let mut out = Walked {
            cells: vec![row],
            pos: 0,
            keys: vec![Vec::new()],
            desc: Vec::new(),
            blanks: w.blanks,
        };
        let Some(first) = w.axes.first().copied() else {
            return Ok(out);
        };
        let m = self.m;
        let Some(level) = m.levels.get(&lv) else {
            return Ok(out);
        };
        // the partition: the axes not walked (the small multiples unless named), the reset's
        // fields, the PARTITIONBY fields
        let d = m.kept_on(lv, first);
        let reset = match w.reset {
            Reset::None => 0,
            Reset::Absolute(k) => k.min(d),
            Reset::Relative(k) => d.saturating_sub(k),
            Reset::Field(f) => match m.axis_of(f) {
                Some((ax, k)) if ax == first => k.saturating_add(1).min(d),
                _ => 0,
            },
        };
        let mut part: Vec<usize> = Vec::new();
        for ax in [Ax::Pages, Ax::Rows, Ax::Cols] {
            if !w.axes.contains(&ax) {
                part.extend(m.axis_fields(ax).iter().take(m.kept_on(lv, ax)));
            }
        }
        part.extend(m.axis_fields(first).iter().take(reset));
        part.extend(w.partition.iter().copied());
        let pkey = |r: &[V]| -> String {
            part.iter()
                .map(|i| r.get(*i).map_or_else(|| "b".to_string(), V::key))
                .collect::<Vec<_>>()
                .join("\u{1}")
        };
        let me = level.rows.get(row).map(|r| pkey(r)).unwrap_or_default();
        self.tick(level.rows.len())?;
        let cells: Vec<usize> = (0..level.rows.len())
            .filter(|r| level.rows.get(*r).is_some_and(|x| pkey(x) == me))
            .collect();
        // MATCHBY: its fields kept at this level must single out the current cell (which the walk
        // then starts from, as without MATCHBY); fields the level does not keep (a subtotal's)
        // are left out
        let kept = m.kept(lv);
        let by: Vec<usize> = w.matchby.iter().copied().filter(|f| kept.contains(f)).collect();
        if !by.is_empty() {
            let mine: Vec<String> = by.iter().map(|f| m.cell(lv, row, *f).key()).collect();
            let same = cells
                .iter()
                .filter(|r| by.iter().zip(&mine).all(|(f, k)| m.cell(lv, **r, *f).key() == *k))
                .count();
            if same != 1 {
                return Err(Fail::Error(
                    "the MATCHBY fields do not single out the current cell of the visual matrix".into(),
                ));
            }
        }
        // the walked axes' fields, the last axis outermost, in their sort-by order
        let mut axis: Vec<usize> = Vec::new();
        for ax in w.axes.iter().rev() {
            axis.extend(m.axis_fields(*ax).iter().take(m.kept_on(lv, *ax)));
        }
        // the order: ORDERBY (ties by the axes), else the axes
        let mut keyed: Vec<(usize, Vec<V>, Vec<V>)> = Vec::with_capacity(cells.len());
        for r in cells {
            let tie: Vec<V> = axis.iter().map(|f| m.sorted(lv, r, *f)).collect();
            let primary = if w.order.is_empty() {
                tie.clone()
            } else {
                let mut ks = Vec::with_capacity(w.order.len());
                for (k, _) in &w.order {
                    ks.push(match k {
                        OrderKey::Field(f) => m.cell(lv, r, *f),
                        OrderKey::Expr(e) => {
                            self.tick(1)?;
                            match self.eval(e, &Cx::Cell(lv, r), env, depth) {
                                Ok(v) => v,
                                Err(Fail::Error(_)) => V::Blank,
                                Err(u) => return Err(u),
                            }
                        }
                    });
                }
                ks
            };
            keyed.push((r, primary, tie));
        }
        out.desc = if w.order.is_empty() {
            vec![false; axis.len()]
        } else {
            w.order.iter().map(|(_, d)| *d).collect()
        };
        let desc = &out.desc;
        keyed.sort_by(|a, b| {
            for (i, (x, y)) in a.1.iter().zip(&b.1).enumerate() {
                let o = cmp_v(x, y, w.blanks);
                let o = if desc.get(i).copied().unwrap_or(false) {
                    o.reverse()
                } else {
                    o
                };
                if o != Ordering::Equal {
                    return o;
                }
            }
            for (x, y) in a.2.iter().zip(&b.2) {
                let o = cmp_v(x, y, w.blanks);
                if o != Ordering::Equal {
                    return o;
                }
            }
            a.0.cmp(&b.0)
        });
        out.pos = keyed.iter().position(|k| k.0 == row).unwrap_or(0);
        out.cells = keyed.iter().map(|k| k.0).collect();
        out.keys = keyed.into_iter().map(|k| k.1).collect();
        Ok(out)
    }

    fn cell_of(cx: &Cx) -> Result<(Lv, usize), Fail> {
        match cx {
            Cx::Cell(lv, r) => Ok((*lv, *r)),
            Cx::Set(lv, rs) if rs.len() == 1 => Ok((*lv, rs.first().copied().unwrap_or(0))),
            Cx::Set(..) => Err(Fail::Error("a window function over several cells".into())),
        }
    }

    fn eval(&mut self, e: &Expr, cx: &Cx, env: &[(String, Val)], depth: usize) -> Result<V, Fail> {
        if depth > MAX_DEPTH {
            return unsupported("the expression nests too deeply");
        }
        let d = depth.saturating_add(1);
        match e {
            Expr::Number { text, .. } => text
                .parse::<f64>()
                .map(V::Num)
                .map_err(|_| Fail::Error(format!("bad number {text}"))),
            Expr::Str { value, .. } => Ok(V::Text(value.clone())),
            Expr::Bool { value, .. } => Ok(V::Bool(*value)),
            Expr::Name { name, .. } => match env.iter().rev().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
                Some((_, Val::Scalar(v))) => Ok(v.clone()),
                Some((_, Val::Table(_))) => unsupported(format!("table variable {name} used as a value")),
                None => unsupported(format!("`{name}` as a value")),
            },
            Expr::Column {
                table: None, name, ..
            } => {
                let f = self
                    .m
                    .field(name)
                    .ok_or_else(|| Fail::Unsupported(format!("no field [{name}]")))?;
                Ok(self.value(f, cx))
            }
            Expr::Column { .. } => unsupported("a model column in a visual calculation"),
            Expr::Unary { op, operand, .. } => {
                let v = self.eval(operand, cx, env, d)?;
                match op.to_ascii_uppercase().as_str() {
                    "-" => Ok(match v {
                        V::Blank => V::Blank,
                        other => V::Num(-other.num().ok_or_else(|| Fail::Error("not a number".into()))?),
                    }),
                    "+" => Ok(v),
                    "NOT" => Ok(V::Bool(!v.truthy()?)),
                    _ => unsupported(format!("operator {op}")),
                }
            }
            Expr::Binary { op, lhs, rhs, .. } => {
                let up = op.to_ascii_uppercase();
                if up == "&&" || up == "||" {
                    let a = self.eval(lhs, cx, env, d)?.truthy()?;
                    if (up == "&&" && !a) || (up == "||" && a) {
                        return Ok(V::Bool(a));
                    }
                    return Ok(V::Bool(self.eval(rhs, cx, env, d)?.truthy()?));
                }
                let a = self.eval(lhs, cx, env, d)?;
                let b = self.eval(rhs, cx, env, d)?;
                binary(&up, &a, &b)
            }
            Expr::Var { bindings, body, .. } => {
                let mut env: Vec<(String, Val)> = env.to_vec();
                for b in bindings {
                    let v = match self.table(&b.value, cx, &env, d) {
                        Ok(t) => Val::Table(t),
                        Err(_) => Val::Scalar(self.eval(&b.value, cx, &env, d)?),
                    };
                    env.push((b.name.clone(), v));
                }
                self.eval(body, cx, &env, d)
            }
            Expr::Call { name, args, .. } => self.call(&name.to_ascii_uppercase(), args, cx, env, d),
            Expr::TableCtor { .. } | Expr::Tuple { .. } => unsupported("a table constructor"),
        }
    }

    /// Field `f` in `cx`: its cell, or over several cells their one value (else blank).
    fn value(&self, f: usize, cx: &Cx) -> V {
        let lv = cx.lv();
        let mut it = cx.cells().into_iter().map(|r| self.m.cell(lv, r, f));
        let Some(first) = it.next() else {
            return V::Blank;
        };
        if it.all(|x| x.key() == first.key()) {
            first
        } else {
            V::Blank
        }
    }

    /// The cells an expression evaluated as a table stands for (`RANGE`, navigation-only
    /// `COLLAPSE` / `EXPAND`, a table variable).
    fn table(&mut self, e: &Expr, cx: &Cx, env: &[(String, Val)], depth: usize) -> Result<Cx, Fail> {
        if depth > MAX_DEPTH {
            return unsupported("the expression nests too deeply");
        }
        match e {
            Expr::Name { name, .. } => match env.iter().rev().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
                Some((_, Val::Table(t))) => Ok(t.clone()),
                _ => unsupported(format!("`{name}` as a table")),
            },
            Expr::Call { name, args, .. } => {
                let up = name.to_ascii_uppercase();
                match up.as_str() {
                    "RANGE" => {
                        let (lv, row) = Self::cell_of(cx)?;
                        let step = arg(args, 0)
                            .and_then(number)
                            .ok_or_else(|| Fail::Unsupported("RANGE's step must be a number".into()))?;
                        let (include, from) = match arg(args, 1) {
                            Some(Expr::Bool { value, .. }) => (*value, 2),
                            Some(Expr::Call { name, args: a, .. })
                                if a.is_empty()
                                    && (name.eq_ignore_ascii_case("TRUE")
                                        || name.eq_ignore_ascii_case("FALSE")) =>
                            {
                                (name.eq_ignore_ascii_case("TRUE"), 2)
                            }
                            _ => (true, 1),
                        };
                        let w = self.walk_args(args, from)?;
                        let Walked { cells, pos, .. } = self.walk(lv, row, &w, env, depth)?;
                        let s = step.abs().min(1e9) as usize;
                        let (lo, hi) = if step < 0.0 {
                            (pos.saturating_sub(s), pos)
                        } else {
                            (pos.saturating_add(1), pos.saturating_add(1).saturating_add(s))
                        };
                        let mut picked: Vec<usize> = cells
                            .iter()
                            .enumerate()
                            .filter(|(i, _)| (*i >= lo && *i < hi) || (include && *i == pos))
                            .map(|(_, r)| *r)
                            .collect();
                        picked.sort_unstable();
                        Ok(Cx::Set(lv, picked))
                    }
                    "COLLAPSE" | "COLLAPSEALL" | "EXPAND" | "EXPANDALL" => {
                        // navigation only: every argument is an axis, a number or a field
                        self.navigate(&up, args, 0, cx)
                    }
                    "OFFSET" | "INDEX" | "WINDOW" => self.window(&up, args, cx, env, depth),
                    _ => unsupported(format!("{up} as a table in a visual calculation")),
                }
            }
            _ => unsupported("a table expression"),
        }
    }

    /// The cells `COLLAPSE` / `COLLAPSEALL` / `EXPAND` / `EXPANDALL` navigate to from `cx`; their
    /// navigation arguments start at `args[from]`.
    fn navigate(&mut self, up: &str, args: &[Option<Expr>], from: usize, cx: &Cx) -> Result<Cx, Fail> {
        let lv = cx.lv();
        let mut target = lv;
        let rest: Vec<&Expr> = args.iter().skip(from).flatten().collect();
        let collapse = up.starts_with("COLLAPSE");
        let all = up.ends_with("ALL");
        if let Some(mut axes) = rest.first().and_then(|e| axes_of(e)) {
            // an axis that is not on the visual is ignored
            axes.retain(|a| self.m.depth(*a) > 0);
            let n = match rest.get(1) {
                Some(e) => number(e)
                    .filter(|x| *x >= 0.0)
                    .ok_or_else(|| Fail::Unsupported(format!("{up}'s level count must be a number")))?
                    as usize,
                None => 1,
            };
            if all {
                for ax in &axes {
                    let to = if collapse { 0 } else { self.m.depth(*ax) };
                    target = with_level(target, *ax, to);
                }
            } else if collapse {
                // up the hierarchy of the axes (the last axis is the deepest)
                let mut left = n;
                for ax in axes.iter().rev() {
                    let cur = level_of(target, *ax);
                    let take = left.min(cur);
                    target = with_level(target, *ax, cur.saturating_sub(take));
                    left = left.saturating_sub(take);
                }
            } else {
                let mut left = n;
                for ax in &axes {
                    let cur = level_of(target, *ax);
                    let add = left.min(self.m.depth(*ax).saturating_sub(cur));
                    target = with_level(target, *ax, cur.saturating_add(add));
                    left = left.saturating_sub(add);
                }
            }
        } else if !all && !rest.is_empty() {
            for e in &rest {
                let Expr::Column {
                    table: None, name, ..
                } = e
                else {
                    return unsupported(format!("an argument of {up}"));
                };
                let Some((ax, k)) = self.m.field(name).and_then(|f| self.m.axis_of(f)) else {
                    return unsupported(format!("{up} to [{name}], which is no axis field"));
                };
                let cur = level_of(target, ax);
                target = if collapse {
                    with_level(target, ax, cur.min(k))
                } else {
                    with_level(target, ax, cur.max(k.saturating_add(1)))
                };
            }
        } else {
            return unsupported(format!("{up} without an axis"));
        }
        if target == lv {
            return Ok(cx.clone());
        }
        if !self.m.levels.contains_key(&target) {
            return Err(Fail::Error(format!(
                "level {target:?} of the visual matrix is not available"
            )));
        }
        let mut rows = Vec::new();
        for r in cx.cells() {
            self.tick(1)?;
            if target.0 <= lv.0 && target.1 <= lv.1 && target.2 <= lv.2 {
                if let Some(a) = self.m.ancestor(lv, r, target) {
                    rows.push(a);
                }
            } else if target.0 >= lv.0 && target.1 >= lv.1 && target.2 >= lv.2 {
                let ds = self.m.descendants(lv, r, target);
                self.tick(ds.len())?;
                rows.extend(ds);
            } else {
                return unsupported(format!("{up} up one axis and down the other"));
            }
        }
        rows.sort_unstable();
        Ok(Cx::of(target, rows))
    }

    /// A position argument of `OFFSET`, `INDEX` or `WINDOW`, evaluated at `cx`: a whole number
    /// (truncated, capped), `None` when blank.
    fn position(&mut self, e: &Expr, cx: &Cx, env: &[(String, Val)], d: usize) -> Result<Option<i128>, Fail> {
        match self.eval(e, cx, env, d)? {
            V::Blank => Ok(None),
            v => {
                let x = v
                    .num()
                    .ok_or_else(|| Fail::Error(format!("the position \"{}\" is not a number", v.text())))?;
                Ok(Some(x.trunc().clamp(-1e12, 1e12) as i128))
            }
        }
    }

    /// The cells `OFFSET`, `INDEX` or `WINDOW` return from cell `cx` (none when outside the
    /// partition).
    fn window(
        &mut self,
        up: &str,
        args: &[Option<Expr>],
        cx: &Cx,
        env: &[(String, Val)],
        d: usize,
    ) -> Result<Cx, Fail> {
        let (lv, row) = Self::cell_of(cx)?;
        let need =
            |i: usize| arg(args, i).ok_or_else(|| Fail::Unsupported(format!("{up} without its position")));
        let is_kind = |e: Option<&Expr>| -> Option<bool> {
            match e.and_then(name_of).as_deref() {
                Some("ABS") => Some(true),
                Some("REL") => Some(false),
                _ => None,
            }
        };
        // (from, from is absolute, to, to is absolute, first walk argument)
        let (from, from_abs, to, to_abs, rest) = match up {
            "OFFSET" => {
                let n = self.position(need(0)?, cx, env, d)?;
                (n, false, n, false, 1)
            }
            "INDEX" => {
                let n = self.position(need(0)?, cx, env, d)?;
                // 0 and blank are no position
                let n = n.filter(|x| *x != 0);
                (n, true, n, true, 1)
            }
            _ => {
                // WINDOW(from[, from_type], to[, to_type], …)
                let f = self.position(need(0)?, cx, env, d)?;
                let (fa, ti) = match (args.get(1), is_kind(arg(args, 1))) {
                    (_, Some(abs)) => (abs, 2),
                    (Some(None), None) => (false, 2),
                    _ => (false, 1),
                };
                let t_expr =
                    arg(args, ti).ok_or_else(|| Fail::Unsupported("WINDOW without its end".to_string()))?;
                let t = self.position(t_expr, cx, env, d)?;
                let (ta, rest) = match is_kind(arg(args, ti.saturating_add(1))) {
                    Some(abs) => (abs, ti.saturating_add(2)),
                    None => (false, ti.saturating_add(1)),
                };
                // a blank bound counts as 0
                (Some(f.unwrap_or(0)), fa, Some(t.unwrap_or(0)), ta, rest)
            }
        };
        let w = self.walk_args(args, rest)?;
        let wk = self.walk(lv, row, &w, env, d)?;
        let len = wk.cells.len() as i128;
        let pos = wk.pos as i128;
        let at = |n: i128, abs: bool| -> i128 {
            if !abs {
                pos.saturating_add(n)
            } else if n > 0 {
                n.saturating_sub(1)
            } else if n == 0 {
                0
            } else {
                len.saturating_add(n)
            }
        };
        let (Some(f), Some(t)) = (from, to) else {
            return Ok(Cx::Set(lv, Vec::new()));
        };
        let (mut lo, mut hi) = (at(f, from_abs), at(t, to_abs));
        if up != "WINDOW" && (lo < 0 || lo >= len) {
            return Ok(Cx::Set(lv, Vec::new()));
        }
        if lo > hi || hi < 0 || lo >= len {
            return Ok(Cx::Set(lv, Vec::new()));
        }
        lo = lo.max(0);
        hi = hi.min(len.saturating_sub(1));
        let mut picked: Vec<usize> = wk
            .cells
            .iter()
            .enumerate()
            .filter(|(i, _)| (*i as i128) >= lo && (*i as i128) <= hi)
            .map(|(_, r)| *r)
            .collect();
        self.tick(picked.len())?;
        picked.sort_unstable();
        Ok(Cx::Set(lv, picked))
    }

    /// `LOOKUP` / `LOOKUPWITHTOTALS`: `expression` at the one cell of the visual matrix whose
    /// fields equal the given values; the other fields are the current cell's (`LOOKUP`) or the
    /// total (`LOOKUPWITHTOTALS`).
    fn lookup(
        &mut self,
        up: &str,
        args: &[Option<Expr>],
        cx: &Cx,
        env: &[(String, Val)],
        d: usize,
    ) -> Result<V, Fail> {
        let e = arg(args, 0).ok_or_else(|| Fail::Unsupported(format!("{up} without its expression")))?;
        let (lv, row) = Self::cell_of(cx)?;
        let totals = up == "LOOKUPWITHTOTALS";
        let mut pairs: Vec<(usize, V)> = Vec::new();
        let mut inferred = false;
        let mut i = 1usize;
        while i < args.len() {
            match (arg(args, i), arg(args, i.saturating_add(1))) {
                (
                    Some(Expr::Column {
                        table: None, name, ..
                    }),
                    Some(ve),
                ) => {
                    let f = self
                        .m
                        .field(name)
                        .ok_or_else(|| Fail::Unsupported(format!("no field [{name}]")))?;
                    if self.m.axis_of(f).is_none() {
                        return unsupported(format!(
                            "{up} filters [{name}], which is on no axis of the visual"
                        ));
                    }
                    let v = self.eval(ve, cx, env, d)?;
                    pairs.push((f, v));
                    i = i.saturating_add(2);
                }
                (Some(k), None) if !totals && i.saturating_add(1) == args.len() => {
                    match name_of(k).as_deref() {
                        Some("EXPLICIT") => break,
                        Some("INFERRED") => {
                            inferred = true;
                            break;
                        }
                        _ => return unsupported(format!("an argument of {up}")),
                    }
                }
                _ => return unsupported(format!("an argument of {up}")),
            }
        }
        // the level: the current one (or the total), deep enough for every field filtered
        let mut target: Lv = if totals { (0, 0, 0) } else { lv };
        for (f, _) in &pairs {
            if let Some((ax, k)) = self.m.axis_of(*f) {
                let need = k.saturating_add(1).max(level_of(target, ax));
                target = with_level(target, ax, need);
            }
        }
        let Some(level) = self.m.levels.get(&target) else {
            return Err(Fail::Error(format!(
                "level {target:?} of the visual matrix is not available"
            )));
        };
        let mut current: Vec<usize> = if totals { Vec::new() } else { self.m.kept(lv) };
        if inferred {
            // INFERRED: a field on the axes of the supplied fields that has one value among the
            // cells matching the supplied values is determined by them; it is not taken from the
            // current cell
            let axes: Vec<Ax> = pairs
                .iter()
                .filter_map(|(f, _)| self.m.axis_of(*f).map(|(ax, _)| ax))
                .collect();
            self.tick(level.rows.len())?;
            let matching: Vec<&Vec<V>> = level
                .rows
                .iter()
                .filter(|cells| {
                    pairs
                        .iter()
                        .all(|(f, v)| lookup_eq(cells.get(*f).unwrap_or(&V::Blank), v))
                })
                .collect();
            let m = self.m;
            current.retain(|f| {
                let on_axis = axes.iter().any(|ax| m.axis_fields(*ax).contains(f));
                let mut values = matching
                    .iter()
                    .map(|c| c.get(*f).map_or_else(|| "b".to_string(), V::key));
                let determined = values.next().is_some_and(|first| values.all(|x| x == first));
                !(on_axis && determined)
            });
        }
        let mut want: Vec<(usize, V)> = Vec::new();
        for f in self.m.kept(target) {
            if let Some((_, v)) = pairs.iter().rev().find(|(x, _)| *x == f) {
                want.push((f, v.clone()));
            } else if current.contains(&f) {
                want.push((f, self.m.cell(lv, row, f)));
            }
        }
        self.tick(level.rows.len())?;
        let mut found: Option<usize> = None;
        for (r, cells) in level.rows.iter().enumerate() {
            let hit = want
                .iter()
                .all(|(f, v)| lookup_eq(cells.get(*f).unwrap_or(&V::Blank), v));
            if hit {
                if found.is_some() {
                    return Err(Fail::Error(format!(
                        "several cells of the visual matrix match {up}'s filters"
                    )));
                }
                found = Some(r);
            }
        }
        match found {
            Some(r) => self.eval(e, &Cx::Cell(target, r), env, d),
            None => Err(Fail::Error(format!(
                "no cell of the visual matrix matches {up}'s filters"
            ))),
        }
    }

    fn call(
        &mut self,
        up: &str,
        args: &[Option<Expr>],
        cx: &Cx,
        env: &[(String, Val)],
        d: usize,
    ) -> Result<V, Fail> {
        let a0 = arg(args, 0);
        let need0 = || a0.ok_or_else(|| Fail::Unsupported(format!("{up} without its first argument")));
        match up {
            "RUNNINGSUM" | "FIRST" | "LAST" | "PREVIOUS" | "NEXT" | "MOVINGAVERAGE" => {
                let e = need0()?;
                let (lv, row) = Self::cell_of(cx)?;
                let (steps, from) = match up {
                    "PREVIOUS" | "NEXT" => match arg(args, 1).and_then(number) {
                        Some(n) => (n, 2),
                        None => (1.0, 1),
                    },
                    "MOVINGAVERAGE" => (
                        arg(args, 1).and_then(number).ok_or_else(|| {
                            Fail::Unsupported("MOVINGAVERAGE's window size must be a number".into())
                        })?,
                        2,
                    ),
                    _ => (0.0, 1),
                };
                let (include, from) = if up == "MOVINGAVERAGE" {
                    match arg(args, 2) {
                        Some(Expr::Bool { value, .. }) => (*value, 3),
                        _ => (true, from),
                    }
                } else {
                    (true, from)
                };
                let w = self.walk_args(args, from)?;
                let Walked { cells, pos, .. } = self.walk(lv, row, &w, env, d)?;
                let at = |i: usize| cells.get(i).copied();
                match up {
                    "RUNNINGSUM" => {
                        let mut sum: Option<f64> = None;
                        for i in 0..=pos {
                            let Some(r) = at(i) else { continue };
                            self.tick(1)?;
                            let v = self.eval(e, &Cx::Cell(lv, r), env, d)?;
                            if let Some(x) = v.num() {
                                sum = Some(sum.unwrap_or(0.0) + x);
                            }
                        }
                        Ok(sum.map_or(V::Blank, V::Num))
                    }
                    "MOVINGAVERAGE" => {
                        let size = steps.clamp(0.0, 1e9) as usize;
                        let (lo, hi) = if include {
                            (pos.saturating_add(1).saturating_sub(size), pos.saturating_add(1))
                        } else {
                            (pos.saturating_sub(size), pos)
                        };
                        let (mut sum, mut n) = (0.0, 0usize);
                        for i in lo..hi {
                            let Some(r) = at(i) else { continue };
                            self.tick(1)?;
                            if let Some(x) = self.eval(e, &Cx::Cell(lv, r), env, d)?.num() {
                                sum += x;
                                n = n.saturating_add(1);
                            }
                        }
                        Ok(if n == 0 { V::Blank } else { V::Num(sum / n as f64) })
                    }
                    _ => {
                        let target = match up {
                            "FIRST" => Some(0),
                            "LAST" => cells.len().checked_sub(1),
                            "PREVIOUS" => usize::try_from(i128::from(pos as u64) - steps as i128).ok(),
                            _ => usize::try_from(i128::from(pos as u64) + steps as i128).ok(),
                        };
                        match target.and_then(at) {
                            Some(r) => self.eval(e, &Cx::Cell(lv, r), env, d),
                            None => Ok(V::Blank),
                        }
                    }
                }
            }
            "COLLAPSE" | "COLLAPSEALL" | "EXPAND" | "EXPANDALL" => {
                let e = need0()?;
                let to = self.navigate(up, args, 1, cx)?;
                self.eval(e, &to, env, d)
            }
            "ISATLEVEL" => {
                let Some(Expr::Column {
                    table: None, name, ..
                }) = a0
                else {
                    return unsupported("ISATLEVEL of something else than a field");
                };
                let f = self
                    .m
                    .field(name)
                    .ok_or_else(|| Fail::Unsupported(format!("no field [{name}]")))?;
                let lv = cx.lv();
                Ok(V::Bool(match self.m.axis_of(f) {
                    Some((ax, k)) => k < level_of(lv, ax),
                    // not grouping: present at every level
                    None => true,
                }))
            }
            "CALCULATE" => {
                let e = need0()?;
                let mut to = cx.clone();
                for a in args.iter().skip(1).flatten() {
                    to = self.table(a, &to, env, d)?;
                }
                self.eval(e, &to, env, d)
            }
            "SUM" | "AVERAGE" | "MIN" | "MAX" | "COUNT" | "COUNTA" | "COUNTBLANK" | "DISTINCTCOUNT"
                if args.len() == 1 =>
            {
                let Some(Expr::Column {
                    table: None, name, ..
                }) = a0
                else {
                    return unsupported(format!("{up} of an expression (use {up}X)"));
                };
                let f = self
                    .m
                    .field(name)
                    .ok_or_else(|| Fail::Unsupported(format!("no field [{name}]")))?;
                let lv = cx.lv();
                let vals: Vec<V> = cx.cells().into_iter().map(|r| self.m.cell(lv, r, f)).collect();
                Ok(aggregate(up, &vals))
            }
            "SUMX" | "AVERAGEX" | "MINX" | "MAXX" | "COUNTX" | "COUNTAX" => {
                let t = self.table(need0()?, cx, env, d)?;
                let e =
                    arg(args, 1).ok_or_else(|| Fail::Unsupported(format!("{up} without an expression")))?;
                let lv = t.lv();
                let mut vals = Vec::new();
                for r in t.cells() {
                    self.tick(1)?;
                    vals.push(self.eval(e, &Cx::Cell(lv, r), env, d)?);
                }
                let base = up.trim_end_matches('X');
                Ok(aggregate(if base == "COUNTA" { "COUNTA" } else { base }, &vals))
            }
            "COUNTROWS" => {
                let t = self.table(need0()?, cx, env, d)?;
                let n = t.cells().len();
                Ok(if n == 0 { V::Blank } else { V::Num(n as f64) })
            }
            "IF" | "IF.EAGER" => {
                let c = self.eval(need0()?, cx, env, d)?.truthy()?;
                match arg(args, if c { 1 } else { 2 }) {
                    Some(e) => self.eval(e, cx, env, d),
                    None => Ok(V::Blank),
                }
            }
            "SWITCH" => {
                let v = self.eval(need0()?, cx, env, d)?;
                let mut i = 1usize;
                while let (Some(w), Some(r)) = (arg(args, i), arg(args, i.saturating_add(1))) {
                    let wv = self.eval(w, cx, env, d)?;
                    if binary("=", &v, &wv)? == V::Bool(true) {
                        return self.eval(r, cx, env, d);
                    }
                    i = i.saturating_add(2);
                }
                match arg(args, i) {
                    Some(e) => self.eval(e, cx, env, d),
                    None => Ok(V::Blank),
                }
            }
            "DIVIDE" => {
                let n = self.eval(need0()?, cx, env, d)?;
                let m = match arg(args, 1) {
                    Some(e) => self.eval(e, cx, env, d)?,
                    None => return unsupported("DIVIDE without a denominator"),
                };
                match (n.num(), m.num()) {
                    (_, Some(0.0)) => match arg(args, 2) {
                        Some(e) => self.eval(e, cx, env, d),
                        None => Ok(V::Blank),
                    },
                    (_, None) => match arg(args, 2) {
                        Some(e) => self.eval(e, cx, env, d),
                        None => Ok(V::Blank),
                    },
                    (None, _) => Ok(V::Blank),
                    (Some(a), Some(b)) => Ok(V::Num(a / b)),
                }
            }
            "BLANK" => Ok(V::Blank),
            "TRUE" => Ok(V::Bool(true)),
            "FALSE" => Ok(V::Bool(false)),
            "ISBLANK" => Ok(V::Bool(self.eval(need0()?, cx, env, d)? == V::Blank)),
            "NOT" => Ok(V::Bool(!self.eval(need0()?, cx, env, d)?.truthy()?)),
            "AND" | "OR" => {
                let a = self.eval(need0()?, cx, env, d)?.truthy()?;
                let b = match arg(args, 1) {
                    Some(e) => self.eval(e, cx, env, d)?.truthy()?,
                    None => return unsupported(format!("{up} with one argument")),
                };
                Ok(V::Bool(if up == "AND" { a && b } else { a || b }))
            }
            "COALESCE" => {
                for e in args.iter().flatten() {
                    let v = self.eval(e, cx, env, d)?;
                    if v != V::Blank {
                        return Ok(v);
                    }
                }
                Ok(V::Blank)
            }
            "MIN" | "MAX" => {
                let a = self.eval(need0()?, cx, env, d)?;
                let b = match arg(args, 1) {
                    Some(e) => self.eval(e, cx, env, d)?,
                    None => return unsupported(format!("{up} with one argument")),
                };
                let vals = [a, b];
                Ok(aggregate(up, &vals))
            }
            "ABS" | "SQRT" | "INT" | "SIGN" | "EXP" | "LN" => {
                let v = self.eval(need0()?, cx, env, d)?;
                let Some(x) = v.num() else { return Ok(V::Blank) };
                Ok(V::Num(match up {
                    "ABS" => x.abs(),
                    "SQRT" => x.sqrt(),
                    "INT" => x.floor(),
                    "SIGN" => x.signum(),
                    "EXP" => x.exp(),
                    _ => x.ln(),
                }))
            }
            "ROUND" | "POWER" | "MOD" => {
                let a = self.eval(need0()?, cx, env, d)?;
                let b = match arg(args, 1) {
                    Some(e) => self.eval(e, cx, env, d)?,
                    None => V::Num(0.0),
                };
                let (Some(x), Some(y)) = (a.num(), b.num().or(Some(0.0))) else {
                    return Ok(V::Blank);
                };
                Ok(V::Num(match up {
                    "ROUND" => {
                        let p = 10f64.powi(y.clamp(-15.0, 15.0) as i32);
                        (x * p).round() / p
                    }
                    "POWER" => x.powf(y),
                    _ => {
                        if y == 0.0 {
                            return Err(Fail::Error("MOD by zero".into()));
                        }
                        x - y * (x / y).floor()
                    }
                }))
            }
            "LOOKUP" | "LOOKUPWITHTOTALS" => self.lookup(up, args, cx, env, d),
            "RANK" | "ROWNUMBER" => {
                let (lv, row) = Self::cell_of(cx)?;
                let (dense, from) = match (up, a0.and_then(name_of).as_deref()) {
                    ("RANK", Some("DENSE")) => (true, 1),
                    ("RANK", Some("SKIP")) => (false, 1),
                    _ => (false, 0),
                };
                let w = self.walk_args(args, from)?;
                // a total of the walked axes has no rank
                if w.axes.iter().all(|a| self.m.kept_on(lv, *a) == 0) {
                    return Ok(V::Blank);
                }
                let wk = self.walk(lv, row, &w, env, d)?;
                if up == "ROWNUMBER" {
                    return Ok(V::Num(wk.pos as f64 + 1.0));
                }
                // the first cell tied with this one; DENSE counts the distinct keys before it
                let mut first = wk.pos;
                while first > 0 && wk.ties(first - 1, wk.pos) {
                    first -= 1;
                }
                let rank = if dense {
                    1 + (1..=first).filter(|i| !wk.ties(i - 1, *i)).count()
                } else {
                    first + 1
                };
                Ok(V::Num(rank as f64))
            }
            "OFFSET" | "INDEX" | "WINDOW" => unsupported(format!(
                "{up} returns cells: use it as the table of CALCULATE or of an X function"
            )),
            _ => unsupported(format!(
                "{up} in a visual calculation is not evaluated by BIcraft"
            )),
        }
    }
}

/// Whether a cell's value matches a `LOOKUP` filter value: blank only matches blank, text is
/// compared case-insensitively (a number matches its text, as `2025` and `"2025"`).
fn lookup_eq(cell: &V, want: &V) -> bool {
    match (cell, want) {
        (V::Blank, V::Blank) => true,
        (V::Blank, _) | (_, V::Blank) => false,
        _ => binary("=", cell, want).is_ok_and(|v| v == V::Bool(true)),
    }
}

fn aggregate(f: &str, vals: &[V]) -> V {
    let nums: Vec<f64> = vals.iter().filter_map(V::num).collect();
    match f {
        "SUM" => {
            if nums.is_empty() {
                V::Blank
            } else {
                V::Num(nums.iter().sum())
            }
        }
        "AVERAGE" => {
            if nums.is_empty() {
                V::Blank
            } else {
                V::Num(nums.iter().sum::<f64>() / nums.len() as f64)
            }
        }
        "MIN" | "MAX" => {
            let it = vals.iter().filter(|v| **v != V::Blank);
            let best = if f == "MIN" {
                it.min_by(|a, b| cmp_v(a, b, Blanks::Default))
            } else {
                it.max_by(|a, b| cmp_v(a, b, Blanks::Default))
            };
            best.cloned().unwrap_or(V::Blank)
        }
        "COUNT" => count(nums.len()),
        "COUNTA" => count(vals.iter().filter(|v| **v != V::Blank).count()),
        "COUNTBLANK" => count(vals.iter().filter(|v| **v == V::Blank).count()),
        _ => {
            // DISTINCTCOUNT
            let mut keys: Vec<String> = vals.iter().map(V::key).collect();
            keys.sort();
            keys.dedup();
            count(keys.len())
        }
    }
}

fn count(n: usize) -> V {
    if n == 0 { V::Blank } else { V::Num(n as f64) }
}

fn binary(op: &str, a: &V, b: &V) -> Result<V, Fail> {
    let num = |v: &V| -> Result<f64, Fail> {
        match v {
            V::Blank => Ok(0.0),
            other => other
                .num()
                .ok_or_else(|| Fail::Error(format!("cannot convert \"{}\" to a number", other.text()))),
        }
    };
    match op {
        "+" | "-" => {
            if *a == V::Blank && *b == V::Blank {
                return Ok(V::Blank);
            }
            let (x, y) = (num(a)?, num(b)?);
            Ok(V::Num(if op == "+" { x + y } else { x - y }))
        }
        "*" => {
            if *a == V::Blank || *b == V::Blank {
                return Ok(V::Blank);
            }
            Ok(V::Num(num(a)? * num(b)?))
        }
        "/" => {
            if *a == V::Blank {
                return Ok(V::Blank);
            }
            let (x, y) = (num(a)?, num(b)?);
            // DAX gives infinity on a zero divisor: no finite value to show
            Ok(if y == 0.0 { V::Blank } else { V::Num(x / y) })
        }
        "^" => {
            if *a == V::Blank {
                return Ok(V::Blank);
            }
            Ok(V::Num(num(a)?.powf(num(b)?)))
        }
        "&" => Ok(V::Text(format!("{}{}", a.text(), b.text()))),
        "=" | "==" | "<>" | "<" | ">" | "<=" | ">=" => {
            let o = match (a, b) {
                (V::Text(_), _) | (_, V::Text(_)) => a.text().to_lowercase().cmp(&b.text().to_lowercase()),
                _ => {
                    if op == "==" && (*a == V::Blank) != (*b == V::Blank) {
                        return Ok(V::Bool(false));
                    }
                    num(a)?.partial_cmp(&num(b)?).unwrap_or(Ordering::Equal)
                }
            };
            Ok(V::Bool(match op {
                "=" | "==" => o == Ordering::Equal,
                "<>" => o != Ordering::Equal,
                "<" => o == Ordering::Less,
                ">" => o == Ordering::Greater,
                "<=" => o != Ordering::Greater,
                _ => o != Ordering::Less,
            }))
        }
        _ => unsupported(format!("operator {op}")),
    }
}

/// Whether an expression navigates the hierarchy (needs levels the data may not hold).
fn navigates(e: &Expr) -> bool {
    match e {
        Expr::Call { name, args, .. } => {
            let up = name.to_ascii_uppercase();
            matches!(
                up.as_str(),
                "COLLAPSE" | "COLLAPSEALL" | "EXPAND" | "EXPANDALL" | "LOOKUP" | "LOOKUPWITHTOTALS"
            ) || args.iter().flatten().any(navigates)
        }
        Expr::Unary { operand, .. } => navigates(operand),
        Expr::Binary { lhs, rhs, .. } => navigates(lhs) || navigates(rhs),
        Expr::Var { bindings, body, .. } => bindings.iter().any(|b| navigates(&b.value)) || navigates(body),
        _ => false,
    }
}

// ---------------------------------------------------------------------------------------------
// applying the calculations to a visual's data
// ---------------------------------------------------------------------------------------------

/// Roles whose grouping fields are the column axis (a matrix's columns, a chart's legend).
const COLUMN_ROLES: [&str; 3] = ["Columns", "Legend", "Series"];
/// The small-multiples role.
const PAGE_ROLES: [&str; 1] = ["SmallMultiple"];

/// What [`evaluate`] returns per calculation: its values per level and cell, or why it has
/// none.
type Results = Vec<Result<HashMap<Lv, Vec<V>>, String>>;

/// The calculations of `sp` evaluated over matrix `m` (whose rows already hold a slot per
/// field), in reference order; each calculation's values are written into `m` for the next.
fn evaluate(m: &mut Matrix, sp: &Split) -> Results {
    let all_names: Vec<&str> = m.names.iter().flatten().map(String::as_str).collect();
    let mut out: Results = sp.calcs.iter().map(|_| Err(String::new())).collect();
    let mut parsed: Vec<Option<Expr>> = Vec::new();
    let mut refs: Vec<(String, Vec<String>)> = Vec::new();
    for (k, c) in sp.calcs.iter().enumerate() {
        let ch = visual_calc::check(&c.expression, &all_names, &c.name);
        if !ch.valid {
            let why = ch
                .diagnostics
                .iter()
                .find(|d| d.severity == bicraft_dax::Severity::Error)
                .map_or_else(
                    || "the expression does not parse".to_string(),
                    |d| d.message.clone(),
                );
            if let Some(slot) = out.get_mut(k) {
                *slot = Err(why);
            }
            parsed.push(None);
        } else {
            parsed.push(ch.expr);
        }
        // references by canonical names
        let canon: Vec<String> = ch
            .refs
            .iter()
            .filter_map(|r| m.field(r))
            .filter_map(|f| m.names.get(f).and_then(|n| n.first()).cloned())
            .collect();
        refs.push((c.name.clone(), canon));
    }
    let order = match visual_calc::order(&refs) {
        Ok(o) => o,
        Err(why) => {
            return sp.calcs.iter().map(|_| Err(why.clone())).collect();
        }
    };
    let mut levels: Vec<Lv> = m.levels.keys().copied().collect();
    levels.sort_unstable();
    for k in order {
        let (Some(c), Some(Some(expr))) = (sp.calcs.get(k), parsed.get(k)) else {
            continue;
        };
        // a calculation over a failed one fails too
        let failed_dep = refs.get(k).is_some_and(|(_, rs)| {
            rs.iter().any(|r| {
                sp.calcs
                    .iter()
                    .position(|x| x.name.eq_ignore_ascii_case(r))
                    .is_some_and(|j| out.get(j).is_some_and(Result::is_err))
            })
        });
        if failed_dep {
            if let Some(slot) = out.get_mut(k) {
                *slot = Err("it refers to a visual calculation that has no value".into());
            }
            continue;
        }
        let mut values: HashMap<Lv, Vec<V>> = HashMap::new();
        let mut why: Option<String> = None;
        {
            let mut ev = Eval { m, visits: 0 };
            'levels: for lv in &levels {
                let n = ev.m.levels.get(lv).map_or(0, |l| l.rows.len());
                let mut col = Vec::with_capacity(n);
                for r in 0..n {
                    match ev.eval(expr, &Cx::Cell(*lv, r), &[], 0) {
                        Ok(v) => col.push(v),
                        Err(Fail::Error(_)) => col.push(V::Blank),
                        Err(Fail::Unsupported(w)) => {
                            why = Some(w);
                            break 'levels;
                        }
                    }
                }
                values.insert(*lv, col);
            }
        }
        if let Some(w) = why {
            if let Some(slot) = out.get_mut(k) {
                *slot = Err(w);
            }
            continue;
        }
        for (lv, col) in &values {
            if let Some(level) = m.levels.get_mut(lv) {
                for (row, v) in level.rows.iter_mut().zip(col) {
                    if let Some(slot) = row.get_mut(c.at) {
                        *slot = v.clone();
                    }
                }
            }
        }
        if let Some(slot) = out.get_mut(k) {
            *slot = Ok(values);
        }
    }
    out
}

/// A base row (the visual without its calculations) as a row of every field.
fn full_row(sp: &Split, base: &[Value]) -> Vec<V> {
    sp.map
        .iter()
        .map(|b| b.and_then(|i| base.get(i)).map_or(V::Blank, V::of))
        .collect()
}

/// Inserts the calculations' cells into `row` (a base row of the data), from the values of
/// level `lv` of `m`.
fn insert_cells(row: &mut Value, sp: &Split, m: &Matrix, lv: Lv, results: &Results) {
    let Some(cells) = row.as_array_mut() else { return };
    let full = full_row(sp, cells);
    let key = m.key(lv, &full);
    let at = m.levels.get(&lv).and_then(|l| l.index.get(&key)).copied();
    let mut order: Vec<usize> = (0..sp.calcs.len()).collect();
    order.sort_by_key(|k| sp.calcs.get(*k).map_or(usize::MAX, |c| c.at));
    for k in order {
        let Some(c) = sp.calcs.get(k) else { continue };
        let v = match (results.get(k), at) {
            (Some(Ok(values)), Some(r)) => values
                .get(&lv)
                .and_then(|col| col.get(r))
                .map_or(Value::Null, V::json),
            _ => Value::Null,
        };
        let pos = c.at.min(cells.len());
        cells.insert(pos, v);
    }
}

/// Adds the visual calculations of `sp` to `d` (the data of `sp.base`, evaluated as `v`, whose
/// grouping fields are `is_group`) and flags the hidden fields' columns.
pub(super) fn apply(
    ctx: &mut Ctx<'_>,
    v: &Visual,
    sp: &Split,
    is_group: &[bool],
    filters: &[Item],
    d: &mut Value,
) {
    let base_width = sp.base.fields.len();
    let aligned = v.fields.len() == base_width && is_group.len() >= base_width;
    // the axes, by role
    let mut m = Matrix {
        names: sp.names.clone(),
        ..Matrix::default()
    };
    for (full, b) in sp.map.iter().enumerate() {
        let Some(bi) = *b else { continue };
        if !is_group.get(bi).copied().unwrap_or(false) {
            continue;
        }
        let role = sp.roles.get(full).map_or("", String::as_str);
        // a cartesian chart binds its small multiples to `Rows`
        let charted = crate::visual_extras::small_multiples_types(v.visual_type.as_deref())
            && role.eq_ignore_ascii_case(crate::visual_extras::SMALL_MULTIPLES_ROLE);
        if charted || PAGE_ROLES.iter().any(|r| r.eq_ignore_ascii_case(role)) {
            m.pages.push(full);
        } else if COLUMN_ROLES.iter().any(|r| r.eq_ignore_ascii_case(role)) {
            m.cols.push(full);
        } else {
            m.rows.push(full);
        }
    }
    // the axes follow the model's sort-by columns (a month name by its month number)
    if !sp.calcs.is_empty() {
        let axis: Vec<usize> = m.rows.iter().chain(&m.cols).chain(&m.pages).copied().collect();
        for full in axis {
            let Some(bi) = sp.map.get(full).copied().flatten() else {
                continue;
            };
            let Some(FieldRef::Column { table, column }) = sp.base.fields.get(bi).map(|b| &b.field) else {
                continue;
            };
            if let Some(pairs) = super::sort_by_cells(ctx, table, column) {
                let map: HashMap<String, V> = pairs
                    .iter()
                    .map(|(value, by)| (V::of(value).key(), V::of(by)))
                    .collect();
                m.sort_keys.insert(full, map);
            }
        }
    }
    let (n, c, p) = (m.rows.len(), m.cols.len(), m.pages.len());
    let rows_of = |x: Option<&Value>| -> Vec<Vec<V>> {
        x.and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_array)
            .map(|r| full_row(sp, r))
            .collect()
    };
    // the levels the data holds
    m.add((n, c, p), rows_of(d.get("rows")));
    if let Some(t) = d.get("totals").and_then(Value::as_array) {
        m.add((0, 0, p), vec![full_row(sp, t)]);
    }
    let subtotal_levels: Vec<(Lv, Vec<Vec<V>>)> = d
        .get("subtotals")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|l| {
            let r = usize::try_from(l.get("rows")?.as_u64()?).ok()?;
            let k = usize::try_from(l.get("columns")?.as_u64()?).ok()?;
            Some(((r.min(n), k.min(c), p), rows_of(l.get("data"))))
        })
        .collect();
    for (lv, rows) in subtotal_levels {
        m.add(lv, rows);
    }
    // the levels a calculation navigates to
    let mut notes: Vec<String> = Vec::new();
    let nav = sp.calcs.iter().any(|x| {
        let (e, _) = visual_calc::parse(&x.expression);
        e.as_ref().is_some_and(navigates)
    });
    if nav && aligned {
        // with the levels across the small multiples (COLLAPSE along ROWPAGES, LOOKUPWITHTOTALS)
        let all: Vec<Lv> = (0..=n)
            .flat_map(|i| (0..=c).flat_map(move |j| (0..=p).map(move |k| (i, j, k))))
            .collect();
        if all.len() > MAX_LEVELS {
            notes.push(format!(
                "more than {MAX_LEVELS} levels: the visual matrix is not navigated"
            ));
        } else {
            let base_of = |full: usize| sp.map.get(full).copied().flatten();
            for lv in all {
                if m.levels.contains_key(&lv) {
                    continue;
                }
                let kept: Vec<usize> = m.kept(lv).into_iter().filter_map(base_of).collect();
                match show::totals_at(ctx, v, is_group, filters, &kept) {
                    Ok(rows) => {
                        let rows: Vec<Vec<V>> = rows
                            .iter()
                            .map(|r| full_row(sp, &r.iter().map(out_json).collect::<Vec<_>>()))
                            .collect();
                        m.add(lv, rows);
                    }
                    Err(r) => {
                        notes.push(format!(
                            "level {lv:?} of the visual matrix could not be evaluated: {r}"
                        ));
                        break;
                    }
                }
            }
        }
    }
    let results: Results = if aligned {
        evaluate(&mut m, sp)
    } else {
        sp.calcs
            .iter()
            .map(|_| Err("the visual's fields could not be matched to its query".to_string()))
            .collect()
    };

    // the data: columns, rows, total row, subtotals; then the indices that follow columns
    let Some(o) = d.as_object_mut() else { return };
    let ok = o.get("status").and_then(Value::as_str) == Some("ok");
    if !ok {
        return;
    }
    if let Some(rows) = o.get_mut("rows").and_then(Value::as_array_mut) {
        for r in rows {
            insert_cells(r, sp, &m, (n, c, p), &results);
        }
    }
    if let Some(t) = o.get_mut("totals") {
        insert_cells(t, sp, &m, (0, 0, p), &results);
    }
    if let Some(levels) = o.get_mut("subtotals").and_then(Value::as_array_mut) {
        for l in levels {
            let r = l.get("rows").and_then(Value::as_u64).unwrap_or(0) as usize;
            let k = l.get("columns").and_then(Value::as_u64).unwrap_or(0) as usize;
            if let Some(data) = l.get_mut("data").and_then(Value::as_array_mut) {
                for row in data {
                    insert_cells(row, sp, &m, (r.min(n), k.min(c), p), &results);
                }
            }
        }
    }
    let new_index = |base: usize| -> usize { sp.map.iter().position(|b| *b == Some(base)).unwrap_or(base) };
    for key in ["cell_styles", "error_bars"] {
        let field = if key == "cell_styles" { "c" } else { "column" };
        if let Some(list) = o.get_mut(key).and_then(Value::as_array_mut) {
            for s in list {
                if let Some(x) = s.get(field).and_then(Value::as_u64) {
                    s[field] = json!(new_index(x as usize));
                }
            }
        }
    }
    let mut info: Vec<Value> = Vec::new();
    if let Some(cols) = o.get_mut("columns").and_then(Value::as_array_mut) {
        let mut order: Vec<usize> = (0..sp.calcs.len()).collect();
        order.sort_by_key(|k| sp.calcs.get(*k).map_or(usize::MAX, |x| x.at));
        for k in order {
            let Some(calc) = sp.calcs.get(k) else { continue };
            let numeric = match results.get(k) {
                Some(Ok(values)) => values
                    .values()
                    .flatten()
                    .all(|x| matches!(x, V::Num(_) | V::Blank)),
                _ => true,
            };
            let boolean = match results.get(k) {
                Some(Ok(values)) => {
                    values.values().flatten().any(|x| matches!(x, V::Bool(_)))
                        && values
                            .values()
                            .flatten()
                            .all(|x| matches!(x, V::Bool(_) | V::Blank))
                }
                _ => false,
            };
            let mut col = Map::new();
            col.insert("name".into(), json!(calc.name));
            col.insert(
                "role".into(),
                json!(sp.roles.get(calc.at).cloned().unwrap_or_default()),
            );
            col.insert("kind".into(), json!("visual_calculation"));
            col.insert(
                "data_type".into(),
                json!(if boolean {
                    "boolean"
                } else if numeric {
                    "double"
                } else {
                    "string"
                }),
            );
            col.insert("expression".into(), json!(calc.expression));
            let mut entry = json!({"name": calc.name});
            if let Some(Err(why)) = results.get(k) {
                col.insert("reason".into(), json!(why));
                entry["reason"] = json!(why);
            }
            info.push(entry);
            let pos = calc.at.min(cols.len());
            cols.insert(pos, Value::Object(col));
        }
        for (i, h) in sp.hidden.iter().enumerate() {
            if *h && let Some(col) = cols.get_mut(i).and_then(Value::as_object_mut) {
                col.insert("hidden".into(), json!(true));
            }
        }
    }
    if !info.is_empty() {
        o.insert("visual_calculations".into(), Value::Array(info));
    }
    if !notes.is_empty() {
        o.insert("visual_calculations_notes".into(), json!(notes));
    }
}

#[cfg(test)]
#[path = "preview_viscalc_tests.rs"]
mod tests;
