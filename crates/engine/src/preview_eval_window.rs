//! Window, grouping, join and balance functions of the DAX evaluator ([`super`]): `OFFSET`,
//! `INDEX`, `WINDOW`, `RANK`, `ROWNUMBER` (with `ORDERBY`, `PARTITIONBY`, `MATCHBY`), `GROUPBY`
//! with `CURRENTGROUP()`, `NATURALINNERJOIN`, `NATURALLEFTOUTERJOIN`, `OPENINGBALANCEMONTH`,
//! `OPENINGBALANCEQUARTER`, `OPENINGBALANCEYEAR`, `CLOSINGBALANCEMONTH`, `CLOSINGBALANCEQUARTER`
//! and `CLOSINGBALANCEYEAR` (`OPENINGBALANCEWEEK`, `CLOSINGBALANCEWEEK` and the balances over a
//! calendar go to `preview_eval_calendar.rs`).
//!
//! Semantics from the public DAX reference (Microsoft Learn):
//! - A window function sorts the rows of its relation (omitted: `ALLSELECTED` of the
//!   `ORDERBY`/`PARTITIONBY` columns, which must then be columns of one table) within each
//!   partition by the `ORDERBY` expressions (omitted: every relation column not in
//!   `PARTITIONBY`, ascending). Blanks: `DEFAULT` puts a blank number between the negative
//!   numbers and zero and a blank text before every text (reversed when descending), `FIRST` and
//!   `LAST` put blanks first or last in either direction; a blanks keyword inside `ORDERBY` wins
//!   over the function's.
//! - `OFFSET`, `INDEX`, `WINDOW` and `ROWNUMBER` need a total order: when the order keys do not
//!   tell the rows of a partition apart, the fewest relation columns that do are appended
//!   (ascending), else `MATCHBY` columns; rows that nothing tells apart are an error.
//! - The current row is found from the outer value of each of its key columns: the relation's
//!   columns (`OFFSET`, `WINDOW` with a relative bound), the `ORDERBY`/`PARTITIONBY`/`MATCHBY`
//!   columns (`RANK`, `ROWNUMBER`), the `PARTITIONBY` columns only (`INDEX`, `WINDOW` with two
//!   absolute bounds), or `MATCHBY` and `PARTITIONBY` when `MATCHBY` is given. The outer value is
//!   the innermost row context's value of that column, else the column's one value in the filter
//!   context; a column with several values there is free, and every row whose free columns hold
//!   one of those values is a current row: table functions return the union of their rows
//!   (without repeats), `RANK` and `ROWNUMBER` their value when all agree, else blank (a total
//!   row). A current row that is not in the relation gives an empty table or blank.
//! - `RANK` ties: `SKIP` (the default) counts the rows ordered strictly before, `DENSE` the
//!   distinct order keys before. `WINDOW` bounds: `REL` counts from the current row, `ABS` from
//!   the partition's start (1-based, 0 is 1) or its end (negative); the window is clipped to the
//!   partition, empty when it lies outside or starts after it ends. `INDEX(0)` or a blank position
//!   is empty.
//! - `GROUPBY(table, columns…, name, expression…)`: the existing combinations of the group-by
//!   columns (columns of the table or of tables it reaches through many-to-one relationships) in
//!   the table's order of first appearance; each expression sees the group's rows as
//!   `CURRENTGROUP()`.
//! - The natural joins match the columns both tables share by lineage (a model table also carries
//!   the columns of the tables it reaches through many-to-one relationships), blank matching
//!   blank; the result has the left table's columns, the join columns the left table only reaches
//!   through a relationship, then the right table's other columns. `NATURALLEFTOUTERJOIN` keeps
//!   the left rows without a match, with blanks on the right.
//! - `OPENINGBALANCE{MONTH,QUARTER,YEAR}(expression, dates [, filter] [, year_end])` evaluate the
//!   expression on the day before the period that holds the first of the dates (the end of the
//!   previous period), `CLOSINGBALANCE*` on the last date of the column in the period that holds
//!   the last of the dates; like `CALCULATE` (context transition, then the filter).
//!
//! The visual-calculation forms (an axis such as `ROWS`, a reset such as `HIGHESTPARENT` or a
//! number after the relation) are an error here: the reference makes them available in visual
//! calculations only, which `preview_viscalc.rs` evaluates.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bicraft_dax::ast::Expr;

use super::super::Fail;
use super::super::store::Key;
use super::tables::sort_cmp;
use super::{
    Bind, ColId, Engine, Env, RowB, Scope, TCol, Tab, V, arg, dax_error, fiscal_year, keyword, month_end,
    month_index, month_start, need, num, quarter_index, subset, unsupported,
};

/// The environment name `CURRENTGROUP()` reads (not a name DAX can spell).
const CURRENT_GROUP: &str = "\u{1}currentgroup";
/// Most rows a natural join builds.
const MAX_JOIN_ROWS: usize = 2_000_000;
/// Widest relation whose pairs of columns are tried as tie-breakers.
const MAX_PAIR_SEARCH: usize = 16;
/// Largest position or shift that can still land in a partition (larger ones are clipped here).
const MAX_SHIFT: f64 = 1.0e15;

/// Table functions this module evaluates.
pub(super) fn handles_table(up: &str) -> bool {
    matches!(
        up,
        "OFFSET"
            | "INDEX"
            | "WINDOW"
            | "GROUPBY"
            | "CURRENTGROUP"
            | "NATURALINNERJOIN"
            | "NATURALLEFTOUTERJOIN"
    )
}

/// Scalar functions this module evaluates.
pub(super) fn handles_scalar(up: &str) -> bool {
    matches!(
        up,
        "RANK"
            | "ROWNUMBER"
            | "OPENINGBALANCEMONTH"
            | "OPENINGBALANCEQUARTER"
            | "OPENINGBALANCEYEAR"
            | "CLOSINGBALANCEMONTH"
            | "CLOSINGBALANCEQUARTER"
            | "CLOSINGBALANCEYEAR"
            | "OPENINGBALANCEWEEK"
            | "CLOSINGBALANCEWEEK"
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Blanks {
    Default,
    First,
    Last,
}

fn blanks_of(k: &str) -> Option<Blanks> {
    match k {
        "DEFAULT" | "KEEP" => Some(Blanks::Default),
        "FIRST" => Some(Blanks::First),
        "LAST" => Some(Blanks::Last),
        _ => None,
    }
}

/// The axis and reset keywords of the visual-calculation forms (`ROWS`, `COLUMNS`, `ROWPAGES`,
/// `NONE`, `HIGHESTPARENT`, `LOWESTPARENT`).
fn is_visual_keyword(k: &str) -> bool {
    matches!(
        k,
        "ROWS" | "COLUMNS" | "ROWPAGES" | "NONE" | "HIGHESTPARENT" | "LOWESTPARENT"
    )
}

/// Order of two values of an `ORDERBY` key.
fn order_cmp(a: &V, b: &V, desc: bool, blanks: Blanks) -> Ordering {
    match (a.is_blank(), b.is_blank()) {
        (true, true) => Ordering::Equal,
        (false, false) => {
            let o = sort_cmp(a, b);
            if desc { o.reverse() } else { o }
        }
        (blank_a, _) => {
            let other = if blank_a { b } else { a };
            let blank_first = match blanks {
                Blanks::First => true,
                Blanks::Last => false,
                Blanks::Default => {
                    // ascending: a blank number sits between the negative numbers and zero, a
                    // blank text (or boolean) before every value
                    let before = match other {
                        V::Num(x) | V::Date(x) => *x >= 0.0,
                        V::Cur(n) => *n >= 0,
                        _ => true,
                    };
                    before != desc
                }
            };
            match (blank_a, blank_first) {
                (true, true) | (false, false) => Ordering::Less,
                _ => Ordering::Greater,
            }
        }
    }
}

fn key_of(v: &V) -> Option<Key> {
    v.key()
}

/// One `ORDERBY` expression.
struct OrderKey<'e> {
    expr: &'e Expr,
    desc: bool,
    blanks: Option<Blanks>,
}

/// The arguments of a window function after its leading scalars, told apart by shape.
struct Clauses<'e> {
    relation: Option<&'e Expr>,
    order: Vec<OrderKey<'e>>,
    blanks: Blanks,
    partition: Vec<&'e Expr>,
    matchby: Vec<&'e Expr>,
    dense: bool,
}

/// Which columns identify the current row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeysOf {
    /// The relation's columns (`OFFSET`, a relative `WINDOW`).
    Relation,
    /// The `ORDERBY`/`PARTITIONBY` columns (`RANK`, `ROWNUMBER`).
    Clauses,
    /// The `PARTITIONBY` columns (`INDEX`, an absolute `WINDOW`).
    Partition,
}

/// A relation sorted into partitions.
struct Sorted {
    tab: Tab,
    /// Partitions, each its rows (positions in `tab`) in order.
    parts: Vec<Vec<usize>>,
    /// Per row: its partition and its position there.
    at: Vec<(usize, usize)>,
    /// The rank keys (`ORDERBY` values only, without tie-breakers), column by column.
    order: Vec<Vec<V>>,
    dirs: Vec<(bool, Blanks)>,
    /// The rows that are current rows.
    current: Vec<usize>,
}

/// A row's values as keys (blank: `None`).
type RowKey = Vec<Option<Key>>;
/// The outer value of a key column (as a key), or the values it may take when it is free.
type Outer = Result<Option<Key>, HashSet<Option<Key>>>;
/// A join side: its columns, its rows, and the columns its rows reach (with their values).
type Side = (Vec<TCol>, Arc<Vec<Vec<V>>>, Vec<(ColId, Vec<V>)>);

/// Values of a clause column for each row, with the column's lineage when it is one.
struct ClauseCol {
    vals: Vec<V>,
    lin: Option<ColId>,
}

impl Engine<'_, '_> {
    /// A bare keyword argument (`DESC`, `DENSE`…): a name that is no variable and no table.
    fn window_keyword(&self, e: &Expr, s: &Scope) -> Option<String> {
        match e {
            Expr::Name { name, quoted, .. }
                if !*quoted
                    && !s.env.iter().any(|(n, _)| n.eq_ignore_ascii_case(name))
                    && self.table_id(name).is_none() =>
            {
                keyword(e)
            }
            _ => None,
        }
    }

    fn clauses<'e>(
        &self,
        args: &'e [Option<Expr>],
        from: usize,
        f: &str,
        s: &Scope,
        ranked: bool,
    ) -> Result<Clauses<'e>, Fail> {
        let mut c = Clauses {
            relation: None,
            order: Vec::new(),
            blanks: Blanks::Default,
            partition: Vec::new(),
            matchby: Vec::new(),
            dense: false,
        };
        for i in from..args.len() {
            let Some(a) = arg(args, i) else {
                continue;
            };
            if let Expr::Call {
                name, args: inner, ..
            } = a
            {
                match name.to_ascii_uppercase().as_str() {
                    "ORDERBY" => {
                        self.order_keys(inner, s, &mut c.order)?;
                        continue;
                    }
                    "PARTITIONBY" => {
                        c.partition.extend(inner.iter().flatten());
                        continue;
                    }
                    "MATCHBY" => {
                        c.matchby.extend(inner.iter().flatten());
                        continue;
                    }
                    _ => {}
                }
            }
            if let Some(k) = self.window_keyword(a, s) {
                match k.as_str() {
                    "DENSE" if ranked => c.dense = true,
                    "SKIP" if ranked => c.dense = false,
                    other => match blanks_of(other) {
                        Some(b) => c.blanks = b,
                        None if is_visual_keyword(other) => {
                            return dax_error(format!(
                                "{f}(): `{other}` is an axis or a reset of visual calculations and \
                                 can only be used in a visual calculation"
                            ));
                        }
                        None => return unsupported(format!("{f}() with `{other}`")),
                    },
                }
                continue;
            }
            if c.relation.is_some() {
                return dax_error(format!(
                    "{f}(): a reset argument can only be used in a visual calculation"
                ));
            }
            c.relation = Some(a);
        }
        Ok(c)
    }

    /// The expressions of an `ORDERBY(expression [, ASC|DESC [, blanks]] …)`.
    fn order_keys<'e>(
        &self,
        args: &'e [Option<Expr>],
        s: &Scope,
        out: &mut Vec<OrderKey<'e>>,
    ) -> Result<(), Fail> {
        let mut i = 0usize;
        while i < args.len() {
            let Some(e) = arg(args, i) else {
                i = i.saturating_add(1);
                continue;
            };
            i = i.saturating_add(1);
            let mut key = OrderKey {
                expr: e,
                desc: false,
                blanks: None,
            };
            if let Some(o) = arg(args, i) {
                let desc = match o {
                    Expr::Bool { value, .. } => Some(!*value),
                    Expr::Number { text, .. } => match text.trim() {
                        "0" => Some(true),
                        "1" => Some(false),
                        _ => None,
                    },
                    other => match self.window_keyword(other, s).as_deref() {
                        Some("ASC") => Some(false),
                        Some("DESC") => Some(true),
                        _ => None,
                    },
                };
                if let Some(d) = desc {
                    key.desc = d;
                    i = i.saturating_add(1);
                    if let Some(b) = arg(args, i)
                        .and_then(|b| self.window_keyword(b, s))
                        .and_then(|k| blanks_of(&k))
                    {
                        key.blanks = Some(b);
                        i = i.saturating_add(1);
                    }
                }
            }
            out.push(key);
        }
        Ok(())
    }

    /// `ALLSELECTED(columns)`, as the evaluator computes it (the relation a window function
    /// defaults to).
    fn allselected_cols(&mut self, cols: &[ColId]) -> Result<Tab, Fail> {
        if let [c] = cols
            && !self.outer.filters.is_empty()
        {
            let outer = self.outer.child();
            let codes = self.values_codes(&outer, *c, true)?;
            let rows = codes
                .iter()
                .map(|code| self.value_of_code(*c, *code).map(|v| vec![v]))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(Tab::tuples(vec![TCol::of(*c)], rows));
        }
        self.all_tuples(cols)
    }

    /// The model column `T[c]` names (not a measure).
    fn model_col(&mut self, e: &Expr) -> Option<ColId> {
        match e {
            Expr::Column {
                table: Some(t),
                name,
                variation: None,
                ..
            } => self.col_id(t, name),
            _ => None,
        }
    }

    /// The values of `e` for each row of `tab` (columns `cols`, values `vals`): a column of the
    /// relation (by lineage, or by name for `[c]`), a column of a table the rows reach through
    /// many-to-one relationships, or any expression in the row context of each row.
    fn clause_col(
        &mut self,
        e: &Expr,
        tab: &Tab,
        cols: &[TCol],
        vals: &[Vec<V>],
        s: &Scope,
    ) -> Result<ClauseCol, Fail> {
        let from_pos = |pos: usize| -> Vec<V> {
            vals.iter()
                .map(|r| r.get(pos).cloned().unwrap_or(V::Blank))
                .collect()
        };
        if let Expr::Column {
            table,
            name,
            variation: None,
            ..
        } = e
        {
            match table {
                None => {
                    if let Some(pos) = cols
                        .iter()
                        .position(|c| self.tcol_name(c).eq_ignore_ascii_case(name))
                    {
                        let lin = cols.get(pos).and_then(|c| c.lin);
                        return Ok(ClauseCol {
                            vals: from_pos(pos),
                            lin,
                        });
                    }
                }
                Some(t) => {
                    if let Some(c) = self.col_id(t, name) {
                        if let Some(pos) = cols.iter().position(|x| x.lin == Some(c)) {
                            return Ok(ClauseCol {
                                vals: from_pos(pos),
                                lin: Some(c),
                            });
                        }
                        let vals = self.each_with(tab, s, |eng, inner| {
                            let own = inner.rc.last().into_iter().find_map(|b| outer_in(eng, b, c));
                            match own {
                                Some(v) => Ok(v),
                                None => eng.related_value(c, inner.rc),
                            }
                        })?;
                        return Ok(ClauseCol { vals, lin: Some(c) });
                    }
                }
            }
        }
        let vals = self.each_with(tab, s, |eng, inner| eng.scalar(e, inner))?;
        Ok(ClauseCol { vals, lin: None })
    }

    /// The outer value of column `c`: the innermost row context's, else its one value in the
    /// filter context; `Err(values)`: free, with the values the filter context leaves.
    fn outer_value(&mut self, c: ColId, s: &Scope) -> Result<Result<V, HashSet<Option<Key>>>, Fail> {
        for b in s.rc.iter().rev() {
            if let Some(v) = outer_in(self, b, c) {
                return Ok(Ok(v));
            }
        }
        let codes = self.values_codes(s.fc, c, true)?;
        let mut vals = Vec::with_capacity(codes.len());
        for code in &codes {
            vals.push(self.value_of_code(c, *code)?);
        }
        match vals.as_slice() {
            [one] => Ok(Ok(one.clone())),
            _ => Ok(Err(vals.iter().map(key_of).collect())),
        }
    }

    /// Evaluates the relation and clauses of a window function, sorts its partitions and finds
    /// the current rows. `total`: tie-break the order (every function but `RANK`).
    fn sorted(
        &mut self,
        f: &str,
        c: &Clauses,
        keys_of: KeysOf,
        total: bool,
        s: &Scope,
    ) -> Result<Sorted, Fail> {
        // the relation
        let tab = match c.relation {
            Some(e) => self.table(e, s)?,
            None => {
                if c.order.is_empty() {
                    return dax_error(format!("{f}() needs a relation or an ORDERBY clause"));
                }
                let mut cols = Vec::new();
                for e in c.order.iter().map(|k| k.expr).chain(c.partition.iter().copied()) {
                    let Some(col) = self.model_col(e) else {
                        return dax_error(format!(
                            "{f}() without a relation needs ORDERBY and PARTITIONBY columns of one table"
                        ));
                    };
                    if !cols.contains(&col) {
                        cols.push(col);
                    }
                }
                let t = match cols.first() {
                    Some(first) => self.col_table(*first)?,
                    None => return internal_error(),
                };
                for col in &cols {
                    if self.col_table(*col)? != t {
                        return dax_error(format!(
                            "{f}() without a relation needs ORDERBY and PARTITIONBY columns of one table"
                        ));
                    }
                }
                self.allselected_cols(&cols)?
            }
        };
        let cols = self.tab_cols(&tab);
        let vals = self.tab_values(&tab)?;
        let n = vals.len();
        self.tick(n)?;
        // clause values
        let mut part = Vec::with_capacity(c.partition.len());
        for e in &c.partition {
            part.push(self.clause_col(e, &tab, &cols, &vals, s)?);
        }
        let mut order = Vec::new();
        let mut dirs = Vec::new();
        if c.order.is_empty() {
            let in_part: Vec<Option<ColId>> = part.iter().map(|p| p.lin).collect();
            for (pos, col) in cols.iter().enumerate() {
                if col.lin.is_some() && in_part.contains(&col.lin) {
                    continue;
                }
                order.push(ClauseCol {
                    vals: vals
                        .iter()
                        .map(|r| r.get(pos).cloned().unwrap_or(V::Blank))
                        .collect(),
                    lin: col.lin,
                });
                dirs.push((false, c.blanks));
            }
        } else {
            for k in &c.order {
                order.push(self.clause_col(k.expr, &tab, &cols, &vals, s)?);
                dirs.push((k.desc, k.blanks.unwrap_or(c.blanks)));
            }
        }
        let mut matchby = Vec::with_capacity(c.matchby.len());
        for e in &c.matchby {
            let m = self.clause_col(e, &tab, &cols, &vals, s)?;
            if m.lin.is_none() && !matches!(e, Expr::Column { .. }) {
                return dax_error(format!("{f}() MATCHBY takes columns"));
            }
            matchby.push(m);
        }
        // partitions, in order of first appearance
        let mut part_ix: HashMap<Vec<Option<Key>>, usize> = HashMap::new();
        let mut parts: Vec<Vec<usize>> = Vec::new();
        let mut part_of = Vec::with_capacity(n);
        for r in 0..n {
            let k: Vec<Option<Key>> = part.iter().map(|p| p.vals.get(r).and_then(key_of)).collect();
            let next = parts.len();
            let ix = *part_ix.entry(k).or_insert(next);
            if ix == next {
                parts.push(Vec::new());
            }
            if let Some(p) = parts.get_mut(ix) {
                p.push(r);
            }
            part_of.push(ix);
        }
        // tie-breakers: MATCHBY, else the fewest relation columns that tell the rows apart
        let mut tie: Vec<Vec<V>> = Vec::new();
        if total {
            let order_keys = |r: usize| -> Vec<Option<Key>> {
                order.iter().map(|o| o.vals.get(r).and_then(key_of)).collect()
            };
            let unique = |extra: &[&Vec<V>]| -> bool {
                let mut seen: HashSet<(usize, RowKey, RowKey)> = HashSet::new();
                (0..n).all(|r| {
                    let x: Vec<Option<Key>> = extra.iter().map(|v| v.get(r).and_then(key_of)).collect();
                    seen.insert((part_of.get(r).copied().unwrap_or(0), order_keys(r), x))
                })
            };
            self.tick(n.saturating_mul(order.len().saturating_add(1)))?;
            if !matchby.is_empty() {
                let m: Vec<&Vec<V>> = matchby.iter().map(|m| &m.vals).collect();
                if !unique(&m) {
                    return dax_error(format!(
                        "{f}() MATCHBY columns do not identify the rows of the relation"
                    ));
                }
                if !unique(&[]) {
                    tie = matchby.iter().map(|m| m.vals.clone()).collect();
                }
            } else if !unique(&[]) {
                let columns: Vec<Vec<V>> = (0..cols.len())
                    .map(|pos| {
                        vals.iter()
                            .map(|r| r.get(pos).cloned().unwrap_or(V::Blank))
                            .collect()
                    })
                    .collect();
                let mut found: Option<Vec<usize>> = None;
                for (i, col) in columns.iter().enumerate() {
                    self.tick(n)?;
                    if unique(&[col]) {
                        found = Some(vec![i]);
                        break;
                    }
                }
                if found.is_none() && columns.len() <= MAX_PAIR_SEARCH {
                    'pairs: for (i, a) in columns.iter().enumerate() {
                        for (j, b) in columns.iter().enumerate().skip(i.saturating_add(1)) {
                            self.tick(n)?;
                            if unique(&[a, b]) {
                                found = Some(vec![i, j]);
                                break 'pairs;
                            }
                        }
                    }
                }
                if found.is_none() {
                    let all: Vec<&Vec<V>> = columns.iter().collect();
                    if unique(&all) {
                        found = Some((0..columns.len()).collect());
                    }
                }
                let Some(found) = found else {
                    return dax_error(format!(
                        "{f}() cannot tell the rows of its relation apart: add MATCHBY or ORDERBY columns"
                    ));
                };
                tie = found.iter().filter_map(|i| columns.get(*i).cloned()).collect();
            }
        }
        // sort each partition
        let cmp = |a: &usize, b: &usize| -> Ordering {
            for (o, (desc, bl)) in order.iter().zip(&dirs) {
                let (Some(x), Some(y)) = (o.vals.get(*a), o.vals.get(*b)) else {
                    continue;
                };
                let r = order_cmp(x, y, *desc, *bl);
                if r != Ordering::Equal {
                    return r;
                }
            }
            for t in &tie {
                let (Some(x), Some(y)) = (t.get(*a), t.get(*b)) else {
                    continue;
                };
                let r = order_cmp(x, y, false, Blanks::Default);
                if r != Ordering::Equal {
                    return r;
                }
            }
            Ordering::Equal
        };
        self.tick(n.saturating_mul(dirs.len().saturating_add(tie.len()).max(1)))?;
        for p in &mut parts {
            p.sort_by(cmp);
        }
        let mut at = vec![(0usize, 0usize); n];
        for (pi, p) in parts.iter().enumerate() {
            for (pos, r) in p.iter().enumerate() {
                if let Some(slot) = at.get_mut(*r) {
                    *slot = (pi, pos);
                }
            }
        }
        // the key columns of the current row, with their values on each row
        let mut keys: Vec<(ColId, Vec<V>)> = Vec::new();
        let push_key = |keys: &mut Vec<(ColId, Vec<V>)>, c: ColId, v: &Vec<V>| {
            if !keys.iter().any(|(k, _)| *k == c) {
                keys.push((c, v.clone()));
            }
        };
        let relation_keys = |keys: &mut Vec<(ColId, Vec<V>)>| {
            for (pos, col) in cols.iter().enumerate() {
                if let Some(c) = col.lin
                    && !keys.iter().any(|(k, _)| *k == c)
                {
                    keys.push((
                        c,
                        vals.iter()
                            .map(|r| r.get(pos).cloned().unwrap_or(V::Blank))
                            .collect(),
                    ));
                }
            }
        };
        if !matchby.is_empty() && keys_of != KeysOf::Partition {
            for m in matchby.iter().chain(&part) {
                if let Some(c) = m.lin {
                    push_key(&mut keys, c, &m.vals);
                }
            }
        } else {
            match keys_of {
                KeysOf::Relation => {
                    relation_keys(&mut keys);
                    if keys.is_empty() {
                        return dax_error(format!(
                            "{f}(): every column of the relation was added by a table function"
                        ));
                    }
                }
                KeysOf::Clauses => {
                    for p in &part {
                        if let Some(c) = p.lin {
                            push_key(&mut keys, c, &p.vals);
                        }
                    }
                    if order.iter().all(|o| o.lin.is_some()) {
                        for o in &order {
                            if let Some(c) = o.lin {
                                push_key(&mut keys, c, &o.vals);
                            }
                        }
                        // the tie-breakers ROWNUMBER appended identify the row too
                        for (pos, col) in cols.iter().enumerate() {
                            if let Some(c) = col.lin {
                                let v: Vec<V> = vals
                                    .iter()
                                    .map(|r| r.get(pos).cloned().unwrap_or(V::Blank))
                                    .collect();
                                if tie.contains(&v) {
                                    push_key(&mut keys, c, &v);
                                }
                            }
                        }
                    } else {
                        relation_keys(&mut keys);
                    }
                }
                KeysOf::Partition => {
                    for p in &part {
                        if let Some(c) = p.lin {
                            push_key(&mut keys, c, &p.vals);
                        }
                    }
                }
            }
        }
        let mut bound: Vec<(Vec<V>, Outer)> = Vec::with_capacity(keys.len());
        for (c, v) in keys {
            let o = match self.outer_value(c, s)? {
                Ok(x) => Ok(key_of(&x)),
                Err(set) => Err(set),
            };
            bound.push((v, o));
        }
        self.tick(n.saturating_mul(bound.len().max(1)))?;
        let current: Vec<usize> = (0..n)
            .filter(|r| {
                bound.iter().all(|(v, o)| {
                    let k = v.get(*r).and_then(key_of);
                    match o {
                        Ok(x) => k == *x,
                        Err(set) => set.contains(&k),
                    }
                })
            })
            .collect();
        Ok(Sorted {
            tab,
            parts,
            at,
            order: order.into_iter().map(|o| o.vals).collect(),
            dirs,
            current,
        })
    }

    pub(super) fn window_table(&mut self, up: &str, args: &[Option<Expr>], s: &Scope) -> Result<Tab, Fail> {
        match up {
            "OFFSET" => {
                let delta = shift_arg(&self.scalar(need(args, 0, up)?, s)?)?;
                let c = self.clauses(args, 1, up, s, false)?;
                let w = self.sorted(up, &c, KeysOf::Relation, true, s)?;
                let mut picked = Vec::new();
                if let Some(d) = delta {
                    for r in &w.current {
                        let Some((p, pos)) = w.at.get(*r) else { continue };
                        if let Some(t) = shifted(*pos, d).and_then(|t| w.parts.get(*p)?.get(t)) {
                            picked.push(*t);
                        }
                    }
                }
                Ok(union_rows(&w.tab, picked))
            }
            "INDEX" => {
                let position = shift_arg(&self.scalar(need(args, 0, up)?, s)?)?;
                let c = self.clauses(args, 1, up, s, false)?;
                let w = self.sorted(up, &c, KeysOf::Partition, true, s)?;
                let mut picked = Vec::new();
                if let Some(k) = position.filter(|k| *k != 0) {
                    for p in current_parts(&w) {
                        let Some(rows) = w.parts.get(p) else { continue };
                        if let Some(t) = absolute(k, rows.len()).and_then(|t| rows.get(t)) {
                            picked.push(*t);
                        }
                    }
                }
                Ok(union_rows(&w.tab, picked))
            }
            "WINDOW" => self.window(args, s),
            "CURRENTGROUP" => match s.env.iter().rev().find(|(n, _)| n == CURRENT_GROUP) {
                Some((_, Bind::T(t))) => Ok(t.clone()),
                _ => dax_error("CURRENTGROUP() can only be used in an expression of GROUPBY"),
            },
            "GROUPBY" => self.groupby(args, s),
            _ => self.natural_join(up, args, s),
        }
    }

    /// `WINDOW(from [, ABS|REL], to [, ABS|REL], …)`.
    fn window(&mut self, args: &[Option<Expr>], s: &Scope) -> Result<Tab, Fail> {
        let f = "WINDOW";
        let mut i = 0usize;
        let bound = |eng: &mut Self, i: &mut usize| -> Result<(Option<i128>, bool), Fail> {
            let v = eng.scalar(need(args, *i, f)?, s)?;
            *i = i.saturating_add(1);
            let mut abs = false;
            if let Some(k) = arg(args, *i).and_then(|e| eng.window_keyword(e, s)) {
                match k.as_str() {
                    "ABS" => {
                        abs = true;
                        *i = i.saturating_add(1);
                    }
                    "REL" => *i = i.saturating_add(1),
                    _ => {}
                }
            } else if arg(args, *i).is_none() && *i < args.len() {
                // an empty type: REL
                *i = i.saturating_add(1);
            }
            Ok((shift_arg(&v)?, abs))
        };
        let (from, from_abs) = bound(self, &mut i)?;
        let (to, to_abs) = bound(self, &mut i)?;
        let c = self.clauses(args, i, f, s, false)?;
        let keys_of = if from_abs && to_abs {
            KeysOf::Partition
        } else {
            KeysOf::Relation
        };
        let w = self.sorted(f, &c, keys_of, true, s)?;
        let (from, to) = (from.unwrap_or(0), to.unwrap_or(0));
        // (partition, current position) pairs: one per partition when both bounds are absolute
        let starts: Vec<(usize, usize)> = if from_abs && to_abs {
            current_parts(&w).into_iter().map(|p| (p, 0)).collect()
        } else {
            w.current.iter().filter_map(|r| w.at.get(*r).copied()).collect()
        };
        let mut picked = Vec::new();
        for (p, pos) in starts {
            let Some(rows) = w.parts.get(p) else { continue };
            let len = i128::try_from(rows.len()).unwrap_or(i128::MAX);
            let place = |k: i128, abs: bool| -> i128 {
                if abs {
                    match k.cmp(&0) {
                        Ordering::Greater => k.saturating_sub(1),
                        Ordering::Equal => 0,
                        Ordering::Less => len.saturating_add(k),
                    }
                } else {
                    i128::try_from(pos).unwrap_or(i128::MAX).saturating_add(k)
                }
            };
            let lo = place(from, from_abs).max(0);
            let hi = place(to, to_abs).min(len.saturating_sub(1));
            if lo > hi {
                continue;
            }
            let (Ok(lo), Ok(hi)) = (usize::try_from(lo), usize::try_from(hi)) else {
                continue;
            };
            picked.extend(rows.get(lo..=hi).unwrap_or_default().iter().copied());
        }
        Ok(union_rows(&w.tab, picked))
    }

    pub(super) fn window_scalar(&mut self, up: &str, args: &[Option<Expr>], s: &Scope) -> Result<V, Fail> {
        match up {
            "RANK" | "ROWNUMBER" => {
                let rank = up == "RANK";
                let c = self.clauses(args, 0, up, s, rank)?;
                let w = self.sorted(up, &c, KeysOf::Clauses, !rank, s)?;
                let mut found: Option<usize> = None;
                for r in &w.current {
                    let Some((p, pos)) = w.at.get(*r).copied() else {
                        continue;
                    };
                    let v = if rank {
                        let Some(rows) = w.parts.get(p) else { continue };
                        rank_at(&w, rows, pos, c.dense)
                    } else {
                        pos.saturating_add(1)
                    };
                    match found {
                        None => found = Some(v),
                        // several current rows that disagree: a total row
                        Some(x) if x != v => return Ok(V::Blank),
                        Some(_) => {}
                    }
                }
                Ok(found.map_or(V::Blank, |x| V::Num(x as f64)))
            }
            _ => self.balance(up, args, s),
        }
    }

    /// The balance functions: the expression on the last day before the period (opening) or the
    /// period's last date (closing).
    fn balance(&mut self, up: &str, args: &[Option<Expr>], s: &Scope) -> Result<V, Fail> {
        // a calendar reference: calendar-based time intelligence (`preview_eval_calendar.rs`)
        if let Some(cal) = arg(args, 1).and_then(|e| self.calendar_of(e)) {
            return self.calendar_balance(up, &cal, args, s);
        }
        self.refuse_nested_calendar(up, arg(args, 1))?;
        if up.ends_with("WEEK") {
            return dax_error(format!(
                "{up}() needs a calendar: week functions only work with calendar-based time intelligence"
            ));
        }
        let body = need(args, 0, up)?;
        let (c, days) = self.dates_arg(need(args, 1, up)?, s, up)?;
        let year = up.ends_with("YEAR");
        // a string literal is the fiscal year end; the other optional argument is a filter
        let (filter, year_end) = match (arg(args, 2), arg(args, 3)) {
            (Some(e @ Expr::Str { .. }), None) if year => (None, Some(e)),
            (f, ye) => (f, ye.filter(|_| year)),
        };
        if arg(args, 3).is_some() && !year {
            return dax_error(format!("{up}() takes at most 3 arguments"));
        }
        let ye = self.year_end(year_end, s)?;
        let period = |x: i64| -> (i64, i64) {
            if up.ends_with("MONTH") {
                let m = month_index(x);
                (month_start(m), month_end(m))
            } else if up.ends_with("QUARTER") {
                let q = quarter_index(x);
                (
                    month_start(q.saturating_mul(3)),
                    month_end(q.saturating_mul(3).saturating_add(2)),
                )
            } else {
                fiscal_year(x, ye)
            }
        };
        let dates = if up.starts_with("OPENING") {
            match days.first() {
                Some(x) => {
                    let day = period(*x).0.saturating_sub(1);
                    self.dates_in(c, &[(day, day)])?
                }
                None => Tab::empty_of(c),
            }
        } else {
            match days.last() {
                Some(x) => {
                    let (lo, hi) = period(*x);
                    match self.dates_in(c, &[(lo, hi)])? {
                        Tab::Tuples { cols, rows } => Tab::Tuples {
                            cols,
                            rows: Arc::new(rows.last().cloned().into_iter().collect()),
                        },
                        other => other,
                    }
                }
                None => Tab::empty_of(c),
            }
        };
        let filters: Vec<&Expr> = filter.into_iter().collect();
        let fc = self.calc_ctx_with(&filters, vec![dates], s)?;
        let inner = Scope {
            fc: &fc,
            rc: &[],
            ..*s
        };
        self.scalar(body, &inner)
    }

    /// `GROUPBY(table, columns…, name, expression…)`.
    fn groupby(&mut self, args: &[Option<Expr>], s: &Scope) -> Result<Tab, Fail> {
        let f = "GROUPBY";
        let tab = self.table(need(args, 0, f)?, s)?;
        let cols = self.tab_cols(&tab);
        let vals = self.tab_values(&tab)?;
        let n = vals.len();
        let mut i = 1usize;
        let mut group_cols: Vec<TCol> = Vec::new();
        let mut group_vals: Vec<Vec<V>> = Vec::new();
        while let Some(e) = arg(args, i) {
            if matches!(e, Expr::Str { .. }) {
                break;
            }
            if !matches!(e, Expr::Column { .. }) {
                return dax_error("GROUPBY() group-by arguments must be columns");
            }
            let cc = self.clause_col(e, &tab, &cols, &vals, s)?;
            let tcol = match (cc.lin, e) {
                (Some(c), _) => cols
                    .iter()
                    .find(|x| x.lin == Some(c))
                    .cloned()
                    .unwrap_or_else(|| TCol::of(c)),
                (None, Expr::Column { name, .. }) => cols
                    .iter()
                    .find(|x| self.tcol_name(x).eq_ignore_ascii_case(name))
                    .cloned()
                    .unwrap_or_else(|| TCol::named(name.clone(), None)),
                (None, _) => return internal_error(),
            };
            group_cols.push(tcol);
            group_vals.push(cc.vals);
            i = i.saturating_add(1);
        }
        let named = super::tables::pairs(args, i, f)?;
        // groups in order of first appearance
        let mut ix: HashMap<Vec<Option<Key>>, usize> = HashMap::new();
        let mut groups: Vec<Vec<usize>> = Vec::new();
        self.tick(n.saturating_mul(group_vals.len().max(1)))?;
        for r in 0..n {
            let k: Vec<Option<Key>> = group_vals.iter().map(|g| g.get(r).and_then(key_of)).collect();
            let next = groups.len();
            let g = *ix.entry(k).or_insert(next);
            if g == next {
                groups.push(Vec::new());
            }
            if let Some(rows) = groups.get_mut(g) {
                rows.push(r);
            }
        }
        if group_vals.is_empty() && groups.is_empty() {
            // no group-by column: one group of every row, even none
            groups.push(Vec::new());
        }
        let mut out_cols = group_cols;
        for (name, e) in &named {
            let ty = self.static_type(e, 0);
            out_cols.push(TCol::named(name.clone(), ty));
        }
        self.check_names(&out_cols, f)?;
        let mut rows = Vec::with_capacity(groups.len());
        for g in &groups {
            let mut row: Vec<V> = group_vals
                .iter()
                .map(|v| g.first().and_then(|r| v.get(*r)).cloned().unwrap_or(V::Blank))
                .collect();
            let mut env: Env = s.env.clone();
            env.push((CURRENT_GROUP.to_string(), Bind::T(subset(&tab, g))));
            let inner = Scope { env: &env, ..*s };
            for (_, e) in &named {
                row.push(self.scalar(e, &inner)?);
            }
            rows.push(row);
        }
        Ok(Tab::tuples(out_cols, rows))
    }

    /// The columns of a table value with their values on each row, plus (for a model table) the
    /// columns of the tables its rows reach through many-to-one relationships.
    fn join_side(&mut self, tab: &Tab, s: &Scope) -> Result<Side, Fail> {
        let cols = self.tab_cols(tab);
        let vals = self.tab_values(tab)?;
        let mut reach = Vec::new();
        if let Tab::Rows { table, .. } = tab {
            let own: HashSet<ColId> = self.table_cols(*table).into_iter().collect();
            let mut far: Vec<ColId> = self
                .expanded_cols(*table)
                .into_iter()
                .filter(|c| !own.contains(c) && !self.cols.get(*c).is_some_and(|i| i.row_number))
                .collect();
            far.sort_unstable();
            for c in far {
                let v = self.each_with(tab, s, |eng, inner| eng.related_value(c, inner.rc))?;
                reach.push((c, v));
            }
        }
        Ok((cols, vals, reach))
    }

    fn natural_join(&mut self, up: &str, args: &[Option<Expr>], s: &Scope) -> Result<Tab, Fail> {
        let left = self.table(need(args, 0, up)?, s)?;
        let right = self.table(need(args, 1, up)?, s)?;
        let (lcols, lvals, lreach) = self.join_side(&left, s)?;
        let (rcols, rvals, rreach) = self.join_side(&right, s)?;
        // the lineage of each side's columns, own then reached, with a getter per row
        let lin_of = |cols: &[TCol], reach: &[(ColId, Vec<V>)]| -> Vec<ColId> {
            cols.iter()
                .filter_map(|c| c.lin)
                .chain(reach.iter().map(|(c, _)| *c))
                .collect()
        };
        let llin = lin_of(&lcols, &lreach);
        let rlin = lin_of(&rcols, &rreach);
        let common: Vec<ColId> = llin.iter().copied().filter(|c| rlin.contains(c)).collect();
        if common.is_empty() {
            return dax_error(format!("{up}() needs at least one common join column"));
        }
        let value = |cols: &[TCol], vals: &[Vec<V>], reach: &[(ColId, Vec<V>)], c: ColId, r: usize| -> V {
            match cols.iter().position(|x| x.lin == Some(c)) {
                Some(p) => vals
                    .get(r)
                    .and_then(|row| row.get(p))
                    .cloned()
                    .unwrap_or(V::Blank),
                None => reach
                    .iter()
                    .find(|(x, _)| *x == c)
                    .and_then(|(_, v)| v.get(r))
                    .cloned()
                    .unwrap_or(V::Blank),
            }
        };
        let key = |cols: &[TCol], vals: &[Vec<V>], reach: &[(ColId, Vec<V>)], r: usize| -> Vec<Option<Key>> {
            common
                .iter()
                .map(|c| key_of(&value(cols, vals, reach, *c, r)))
                .collect()
        };
        let mut index: HashMap<Vec<Option<Key>>, Vec<usize>> = HashMap::new();
        for r in 0..rvals.len() {
            index.entry(key(&rcols, &rvals, &rreach, r)).or_default().push(r);
        }
        self.tick(lvals.len().saturating_add(rvals.len()))?;
        // output columns: the left ones, the join columns the left side only reaches, the right
        // side's other own columns
        let extra: Vec<ColId> = common
            .iter()
            .copied()
            .filter(|c| !lcols.iter().any(|x| x.lin == Some(*c)))
            .collect();
        let rkeep: Vec<usize> = (0..rcols.len())
            .filter(|p| {
                rcols
                    .get(*p)
                    .is_none_or(|c| c.lin.is_none_or(|l| !common.contains(&l)))
            })
            .collect();
        let mut out_cols: Vec<TCol> = lcols.clone();
        out_cols.extend(extra.iter().map(|c| TCol::of(*c)));
        out_cols.extend(rkeep.iter().filter_map(|p| rcols.get(*p).cloned()));
        let outer = up == "NATURALLEFTOUTERJOIN";
        let mut rows: Vec<Vec<V>> = Vec::new();
        for (l, lrow) in lvals.iter().enumerate() {
            let k = key(&lcols, &lvals, &lreach, l);
            let matches = index.get(&k).map(Vec::as_slice).unwrap_or_default();
            if matches.is_empty() && !outer {
                continue;
            }
            let mut base = lrow.clone();
            base.extend(extra.iter().map(|c| value(&lcols, &lvals, &lreach, *c, l)));
            if matches.is_empty() {
                base.extend(rkeep.iter().map(|_| V::Blank));
                rows.push(base);
                continue;
            }
            for r in matches {
                let mut row = base.clone();
                row.extend(
                    rkeep
                        .iter()
                        .map(|p| rvals.get(*r).and_then(|x| x.get(*p)).cloned().unwrap_or(V::Blank)),
                );
                rows.push(row);
                if rows.len() > MAX_JOIN_ROWS {
                    return unsupported(format!("{up}() of more than {MAX_JOIN_ROWS} rows"));
                }
            }
            self.tick(matches.len().max(1))?;
        }
        Ok(Tab::tuples(out_cols, rows))
    }
}

/// The value of column `c` in one row context, when it has that column.
fn outer_in(eng: &mut Engine<'_, '_>, b: &RowB, c: ColId) -> Option<V> {
    match b {
        RowB::Row { table, row } => {
            if eng.col_table(c).ok()? != *table {
                return None;
            }
            let d = eng.col(c).ok()?;
            let code = d.codes.get(*row as usize).unwrap_or(0);
            Some(d.values.get(code as usize).map_or(V::Blank, V::of_cell))
        }
        RowB::Tuple { cols, vals } => cols
            .iter()
            .position(|x| x.lin == Some(c))
            .map(|i| vals.get(i).cloned().unwrap_or(V::Blank)),
    }
}

fn internal_error<T>() -> Result<T, Fail> {
    dax_error("internal error in the window functions")
}

/// A shift or position argument, truncated (and clipped far outside any partition); `None`:
/// blank or not a number.
fn shift_arg(v: &V) -> Result<Option<i128>, Fail> {
    Ok(match num(v)? {
        Some(x) if !x.is_nan() => Some(x.trunc().clamp(-MAX_SHIFT, MAX_SHIFT) as i128),
        _ => None,
    })
}

/// `pos + d` when it is a position.
fn shifted(pos: usize, d: i128) -> Option<usize> {
    usize::try_from(i128::try_from(pos).ok()?.checked_add(d)?).ok()
}

/// The position of the absolute (1-based, negative from the end) position `k` in `len` rows.
fn absolute(k: i128, len: usize) -> Option<usize> {
    let len = i128::try_from(len).ok()?;
    let p = if k > 0 {
        k.checked_sub(1)?
    } else {
        len.checked_add(k)?
    };
    if p < 0 || p >= len {
        return None;
    }
    usize::try_from(p).ok()
}

/// The partitions of the current rows, each once.
fn current_parts(w: &Sorted) -> Vec<usize> {
    let mut seen = HashSet::new();
    w.current
        .iter()
        .filter_map(|r| w.at.get(*r).map(|(p, _)| *p))
        .filter(|p| seen.insert(*p))
        .collect()
}

/// The rows `picked` of `tab`, each once, in the order first picked.
fn union_rows(tab: &Tab, picked: Vec<usize>) -> Tab {
    let mut seen = HashSet::new();
    let picked: Vec<usize> = picked.into_iter().filter(|r| seen.insert(*r)).collect();
    subset(tab, &picked)
}

/// `RANK` of the row at `pos` of the sorted partition `rows`: one more than the rows (`SKIP`) or
/// the distinct order keys (`DENSE`) ordered strictly before it.
fn rank_at(w: &Sorted, rows: &[usize], pos: usize, dense: bool) -> usize {
    let same = |a: usize, b: usize| -> bool {
        w.order
            .iter()
            .zip(&w.dirs)
            .all(|(o, (desc, bl))| match (o.get(a), o.get(b)) {
                (Some(x), Some(y)) => order_cmp(x, y, *desc, *bl) == Ordering::Equal,
                _ => true,
            })
    };
    let mut rank = 1usize;
    let mut dense_rank = 1usize;
    for i in 1..=pos {
        let (Some(prev), Some(cur)) = (rows.get(i.saturating_sub(1)), rows.get(i)) else {
            break;
        };
        if !same(*prev, *cur) {
            rank = i.saturating_add(1);
            dense_rank = dense_rank.saturating_add(1);
        }
    }
    if dense { dense_rank } else { rank }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blanks_order_between_negatives_and_zero() {
        let mut v = vec![V::Num(1.0), V::Blank, V::Num(-2.0), V::Num(0.0)];
        v.sort_by(|a, b| order_cmp(a, b, false, Blanks::Default));
        assert_eq!(v, vec![V::Num(-2.0), V::Blank, V::Num(0.0), V::Num(1.0)]);
        v.sort_by(|a, b| order_cmp(a, b, true, Blanks::Default));
        assert_eq!(v, vec![V::Num(1.0), V::Num(0.0), V::Blank, V::Num(-2.0)]);
        v.sort_by(|a, b| order_cmp(a, b, true, Blanks::First));
        assert_eq!(v.first(), Some(&V::Blank));
        v.sort_by(|a, b| order_cmp(a, b, false, Blanks::Last));
        assert_eq!(v.last(), Some(&V::Blank));
        let mut t = [V::Text("b".into()), V::Blank, V::Text(String::new())];
        t.sort_by(|a, b| order_cmp(a, b, false, Blanks::Default));
        assert_eq!(t.first(), Some(&V::Blank));
    }

    #[test]
    fn positions_never_overflow() {
        assert_eq!(absolute(1, 3), Some(0));
        assert_eq!(absolute(-1, 3), Some(2));
        assert_eq!(absolute(4, 3), None);
        assert_eq!(absolute(-4, 3), None);
        assert_eq!(absolute(i128::MIN, 3), None);
        assert_eq!(shifted(0, -1), None);
        assert_eq!(shifted(usize::MAX, i128::MAX), None);
        assert_eq!(shift_arg(&V::Num(f64::NAN)).ok(), Some(None));
        assert_eq!(
            shift_arg(&V::Num(f64::NEG_INFINITY)).ok(),
            Some(Some(-1_000_000_000_000_000))
        );
        assert_eq!(shift_arg(&V::Num(1e300)).ok(), Some(Some(1_000_000_000_000_000)));
    }
}
