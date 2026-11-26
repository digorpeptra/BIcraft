//! General measure evaluator for the data preview ([`super`]): a DAX filter context, `CALCULATE`
//! and time intelligence. Used when the fast path (one grouped pass over a table, see `Compiler`)
//! cannot express a visual's measures.
//!
//! A visual is evaluated like `SUMMARIZECOLUMNS`: the candidate rows are the existing
//! combinations of the grouping columns of each table (plus the blank row of a table that has
//! unmatched related rows), crossed between tables; every value field is evaluated in the filter
//! context of each candidate, and candidates whose values are all blank are dropped.
//!
//! Filter context: a list of filters, each on one table — allowed values of one column, or
//! allowed rows of the table (a table filter, a context transition, a multi-column filter).
//! The rows of a table visible in a context are its own filters, plus the filters of the tables
//! filtering it through active relationships (into the many side always, into the one side when
//! the relationship filters both ways), recursively. Every table has a virtual blank row (the last
//! slot of a row mask) that rows with unmatched keys relate to.
//!
//! `CALCULATE` follows the DAX order: filter arguments are evaluated in the outer context, then
//! context transition, then `ALL`/`REMOVEFILTERS`/`ALLEXCEPT`/`USERELATIONSHIP`/`CROSSFILTER`,
//! then the filter arguments replace the filters on their columns (`KEEPFILTERS` intersects).
//! A filter on the date column of a date table (marked as such, or a date column on the one side
//! of a relationship) also removes the filters on the rest of that table, as DAX does.
//!
//! Work is bounded: measure and expression nesting, the number of candidates, row visits and time
//! per visual. Anything not evaluated is `unsupported` with a reason naming the function.

use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bicraft_dax::ast::Expr;
use bicraft_dax::types::{Returns, ValueType};
use bicraft_locale::{Locale, calendar};
use bicraft_model::{DataType, SemanticModel};
use bicraft_report::{FieldRef, Visual};
use bicraft_vertipaq::CellValue;
use serde_json::{Value, json};

use super::currency::{self, Kind};
use super::filters::{CmpOp, Cond, Item, Operand, TextOp, TopN, Val};
use super::store::{ColData, Key, Miss};
use super::{
    Agg, Ctx, Fail, Grid, MAX_HOPS, Out, Query, agg_label, agg_of_pbir, as_num, data_type_name, unsupported,
};

type ColId = usize;
type TabId = usize;

/// "No matching code" in relationship code maps.
const NONE: u32 = u32::MAX;
/// Deepest nesting of measure references.
const MAX_MEASURE_DEPTH: usize = 24;
/// Deepest expression nesting, measure bodies included.
const MAX_EXPR_DEPTH: usize = 160;
/// Most candidate rows (group combinations) evaluated for one visual.
const MAX_CANDIDATES: usize = 20_000;
/// Most row visits for one visual.
const MAX_WORK: u64 = 400_000_000;
/// Longest time spent on one visual.
const MAX_TIME: Duration = Duration::from_secs(5);
/// Relationship hops a filter propagates through.
const MAX_FILTER_HOPS: usize = 8;
/// Largest shift (in intervals) a time-intelligence function accepts.
const MAX_SHIFT: i64 = 100_000;

// ---------------------------------------------------------------------------------------------
// values
// ---------------------------------------------------------------------------------------------

thread_local! {
    /// Culture of the implicit conversions between text and numbers or dates (`"3/5/2024" + 0`,
    /// `DATE(2024, 3, 5) & ""`) of the evaluator running on this thread: its model's culture
    /// (set by [`Engine::new`]).
    static CONVERT_LOCALE: Cell<&'static Locale> = Cell::new(Locale::en_us());
}

fn convert_locale() -> &'static Locale {
    CONVERT_LOCALE.with(Cell::get)
}

thread_local! {
    /// Whether evaluators on this thread use the storage engine (`preview_eval_storage.rs`);
    /// off, every aggregation scans its table in its own filter context (the reference path).
    static STORAGE_ENGINE: Cell<bool> = const { Cell::new(true) };
}

/// Turns the storage engine on or off for the evaluators of this thread (tests and
/// micro-benchmarks compare both paths).
#[cfg(test)]
pub(crate) fn set_storage_engine(on: bool) {
    STORAGE_ENGINE.with(|c| c.set(on));
}

fn storage_engine() -> bool {
    STORAGE_ENGINE.with(Cell::get)
}

/// A scalar value.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum V {
    Blank,
    Num(f64),
    /// Currency (fixed decimal): 1/10 000 units.
    Cur(i64),
    /// Days since 1899-12-30.
    Date(f64),
    Text(String),
    Bool(bool),
}

impl V {
    fn of_cell(c: &CellValue) -> V {
        match c {
            CellValue::Null => V::Blank,
            CellValue::Int(n) => V::Num(*n as f64),
            CellValue::Real(x) => V::Num(*x),
            CellValue::Decimal(n) => V::Cur(*n),
            CellValue::DateTime(x) => V::Date(*x),
            CellValue::Text(s) => V::Text(s.clone()),
            CellValue::Bool(b) => V::Bool(*b),
        }
    }

    fn to_cell(&self) -> CellValue {
        match self {
            V::Blank => CellValue::Null,
            V::Num(x) => CellValue::Real(*x),
            V::Cur(n) => CellValue::Decimal(*n),
            V::Date(x) => CellValue::DateTime(*x),
            V::Text(s) => CellValue::Text(s.clone()),
            V::Bool(b) => CellValue::Bool(*b),
        }
    }

    fn key(&self) -> Option<Key> {
        Key::of(&self.to_cell())
    }

    fn is_blank(&self) -> bool {
        matches!(self, V::Blank)
    }

    fn out(self) -> Out {
        match self {
            V::Blank => Out::Blank,
            V::Num(x) => Out::Num(x),
            V::Cur(n) => Out::Num(currency::to_f64(n)),
            other => Out::Cell(other.to_cell()),
        }
    }
}

fn dax_error<T>(msg: impl Into<String>) -> Result<T, Fail> {
    Err(Fail::Miss(Miss::Error(msg.into())))
}

fn internal<T>() -> Result<T, Fail> {
    dax_error("internal error in the preview evaluator")
}

/// A number (blank: `None`); text that is not a number is an error, as in DAX.
fn num(v: &V) -> Result<Option<f64>, Fail> {
    match v {
        V::Blank => Ok(None),
        V::Num(x) | V::Date(x) => Ok(Some(*x)),
        V::Cur(n) => Ok(Some(currency::to_f64(*n))),
        V::Bool(b) => Ok(Some(if *b { 1.0 } else { 0.0 })),
        // A number, else a date or a time (`"2024-01-15" + 0` is 45306, `"14:30" + 0` 0.604…;
        // Desktop 2.159).
        V::Text(s) => match s.trim().parse::<f64>() {
            Ok(x) => Ok(Some(x)),
            Err(_) => match math::text_date(s, convert_locale()) {
                Some(x) => Ok(Some(x)),
                None => dax_error(format!("cannot convert value '{s}' of type Text to type Number")),
            },
        },
    }
}

/// A value as a condition: text converts when it is empty (FALSE) or names a truth value
/// (`"TRUE"`, `"false"`; Desktop 2.159).
fn truth(v: &V) -> Result<bool, Fail> {
    match v {
        V::Blank => Ok(false),
        V::Bool(b) => Ok(*b),
        V::Num(x) | V::Date(x) => Ok(*x != 0.0),
        V::Cur(n) => Ok(*n != 0),
        V::Text(t) if t.is_empty() || t.eq_ignore_ascii_case("false") => Ok(false),
        V::Text(t) if t.eq_ignore_ascii_case("true") => Ok(true),
        V::Text(_) => dax_error("cannot convert a text value to True/False"),
    }
}

/// A value as text, as DAX converts it (`&`, text functions): numbers with up to 15 significant
/// digits, dates in the culture's general date format (`3/5/2024`, `3/5/2024 2:30:00 PM`,
/// `2:30:00 PM` for a time alone), `TRUE`/`FALSE` (Desktop 2.159).
fn text_of(v: &V) -> String {
    match v {
        V::Blank => String::new(),
        V::Cur(n) => math::number_text(currency::to_f64(*n)),
        V::Num(x) => math::number_text(*x),
        V::Date(x) => {
            text::format_value(v, "General Date", convert_locale()).unwrap_or_else(|| math::number_text(*x))
        }
        V::Text(s) => s.clone(),
        V::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
    }
}

/// Text order without case: the order of `a.to_lowercase()` and `b.to_lowercase()`, compared
/// without building them. ASCII text compares byte by byte; other text compares the streams of
/// `char::to_lowercase`, which is what `str::to_lowercase` produces except for a capital sigma
/// (its lower case depends on the letters around it), so text with one keeps the allocating
/// reference. Comparing chars by scalar value orders as comparing their UTF-8 bytes.
fn cmp_text(a: &str, b: &str) -> Ordering {
    if a.is_ascii() && b.is_ascii() {
        // equal bytes have equal lower cases: skip the common prefix first
        let (x, y) = (a.as_bytes(), b.as_bytes());
        let p = x.iter().zip(y).take_while(|(c, d)| c == d).count();
        let (x, y) = (x.get(p..).unwrap_or_default(), y.get(p..).unwrap_or_default());
        return x
            .iter()
            .map(u8::to_ascii_lowercase)
            .cmp(y.iter().map(u8::to_ascii_lowercase));
    }
    if a.contains('\u{3a3}') || b.contains('\u{3a3}') {
        return a.to_lowercase().cmp(&b.to_lowercase());
    }
    a.chars()
        .flat_map(char::to_lowercase)
        .cmp(b.chars().flat_map(char::to_lowercase))
}

/// DAX `=` / `<` … comparison: blank is 0, "" or FALSE next to a value of that type.
fn compare_vals(a: &V, b: &V) -> Result<Ordering, Fail> {
    use V::*;
    Ok(match (a, b) {
        (Blank, Blank) => Ordering::Equal,
        (Text(x), Text(y)) => cmp_text(x, y),
        (Text(x), Blank) => cmp_text(x, ""),
        (Blank, Text(y)) => cmp_text("", y),
        (Bool(x), Bool(y)) => x.cmp(y),
        (Bool(x), Blank) => x.cmp(&false),
        (Blank, Bool(y)) => false.cmp(y),
        (Text(_), _) | (_, Text(_)) => return dax_error("cannot compare a text value with a number"),
        (Bool(_), _) | (_, Bool(_)) => return dax_error("cannot compare True/False with a number"),
        _ => {
            let x = num(a)?.unwrap_or(0.0);
            let y = num(b)?.unwrap_or(0.0);
            x.partial_cmp(&y).unwrap_or(Ordering::Equal)
        }
    })
}

/// DAX `==`: blank equals only blank.
fn strictly_equal(a: &V, b: &V) -> Result<bool, Fail> {
    if a.is_blank() || b.is_blank() {
        return Ok(a.is_blank() && b.is_blank());
    }
    Ok(compare_vals(a, b)? == Ordering::Equal)
}

/// The Currency kind of a value for [`currency::binary`] (`None`: text or a date, which Currency
/// rules do not cover). A blank counts as the whole number 0.
fn kind_of(v: &V) -> Option<(f64, Kind)> {
    match v {
        V::Blank => Some((0.0, Kind::Int)),
        V::Bool(b) => Some((f64::from(u8::from(*b)), Kind::Int)),
        V::Num(x) => Some((*x, Kind::of_value(*x))),
        V::Cur(n) => Some((currency::to_f64(*n), Kind::Currency)),
        V::Date(_) | V::Text(_) => None,
    }
}

fn overflow<T>() -> Result<T, Fail> {
    dax_error("the result of a Currency operation is too large or too small")
}

/// The Currency result of `a op b`, when the operation gives one (see [`currency::binary`]).
fn currency_arith(op: &str, a: &V, b: &V) -> Result<Option<V>, Fail> {
    if !matches!(a, V::Cur(_)) && !matches!(b, V::Cur(_)) {
        return Ok(None);
    }
    let c = match op {
        "+" => '+',
        "-" => '-',
        "*" => '*',
        "/" => '/',
        _ => return Ok(None),
    };
    // BLANK * x, x * BLANK and BLANK / x are blank: the double path handles them.
    if (c == '*' && (a.is_blank() || b.is_blank())) || (c == '/' && a.is_blank()) {
        return Ok(None);
    }
    let (Some((x, kx)), Some((y, ky))) = (kind_of(a), kind_of(b)) else {
        return Ok(None);
    };
    if c == '/' && kx == Kind::Currency && y == 0.0 {
        return dax_error("division by zero");
    }
    match currency::binary(c, x, kx, y, ky) {
        None => Ok(None),
        Some(Some(n)) => Ok(Some(V::Cur(n))),
        Some(None) => overflow(),
    }
}

fn arith(op: &str, a: &V, b: &V) -> Result<V, Fail> {
    if let Some(v) = currency_arith(op, a, b)? {
        return Ok(v);
    }
    // A date plus a number, or a date minus anything, is a date (the difference of two dates is
    // an interval counted from 1899-12-30); a number minus a date is a number (Desktop 2.159).
    let date = matches!(a, V::Date(_)) || (op == "+" && matches!(b, V::Date(_)));
    match op {
        // BLANK() & BLANK() is blank (Desktop 2.158).
        "&" if a.is_blank() && b.is_blank() => Ok(V::Blank),
        "&" => Ok(V::Text(format!("{}{}", text_of(a), text_of(b)))),
        "+" | "-" => {
            if a.is_blank() && b.is_blank() {
                return Ok(V::Blank);
            }
            let (x, y) = (num(a)?.unwrap_or(0.0), num(b)?.unwrap_or(0.0));
            let r = if op == "+" { x + y } else { x - y };
            Ok(if date { V::Date(r) } else { V::Num(r) })
        }
        "*" => match (num(a)?, num(b)?) {
            (Some(x), Some(y)) => Ok(V::Num(x * y)),
            _ => Ok(V::Blank),
        },
        "/" => match num(a)? {
            None => Ok(V::Blank),
            Some(x) => Ok(V::Num(x / num(b)?.unwrap_or(0.0))),
        },
        "^" => match num(a)? {
            None => Ok(V::Blank),
            Some(x) => Ok(V::Num(x.powf(num(b)?.unwrap_or(0.0)))),
        },
        other => unsupported(format!("operator `{other}`")),
    }
}

// ---------------------------------------------------------------------------------------------
// dates (days since 1899-12-30)
// ---------------------------------------------------------------------------------------------

/// (year, month 1-12, day 1-31) of a day number (`bicraft_locale::calendar`, through jiff).
fn ymd(x: i64) -> (i64, i64, i64) {
    calendar::civil_from_serial(x)
}

/// Day number of a civil date (month 1-12, day 1-31).
fn serial(y: i64, m: i64, d: i64) -> i64 {
    calendar::serial_from_civil(y, m, d)
}

fn days_in_month(y: i64, m: i64) -> i64 {
    calendar::days_in_month(y, m)
}

/// Months since year 0 (`y * 12 + m - 1`).
fn month_index(x: i64) -> i64 {
    calendar::month_index(x)
}

fn month_start(mi: i64) -> i64 {
    calendar::month_start(mi)
}

fn month_end(mi: i64) -> i64 {
    calendar::month_end(mi)
}

fn quarter_index(x: i64) -> i64 {
    month_index(x).div_euclid(3)
}

fn is_month_end(x: i64) -> bool {
    calendar::is_month_end(x)
}

/// `x` moved by `k` months, the day clipped to the target month.
fn shift_clip(x: i64, k: i64) -> i64 {
    calendar::add_months_clip(x, k)
}

/// `x` moved by `k` months; the last day of a month maps to the last day of the target month.
fn shift_eom(x: i64, k: i64) -> i64 {
    if is_month_end(x) {
        month_end(month_index(x) + k)
    } else {
        shift_clip(x, k)
    }
}

/// Last day of the fiscal year ending (month, day) in calendar year `y`.
fn year_end_in(y: i64, (m, d): (i64, i64)) -> i64 {
    serial(y, m, d.min(days_in_month(y, m)))
}

/// (first, last) day of the fiscal year containing `x`.
fn fiscal_year(x: i64, ye: (i64, i64)) -> (i64, i64) {
    let (y, _, _) = ymd(x);
    let e = year_end_in(y, ye);
    if x <= e {
        (year_end_in(y - 1, ye) + 1, e)
    } else {
        (e + 1, year_end_in(y + 1, ye))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Interval {
    Day,
    Month,
    Quarter,
    Year,
}

impl Interval {
    fn months(self) -> i64 {
        match self {
            Interval::Day => 0,
            Interval::Month => 1,
            Interval::Quarter => 3,
            Interval::Year => 12,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// tables, row and filter contexts
// ---------------------------------------------------------------------------------------------

/// A column of a table value.
#[derive(Debug, Clone, PartialEq)]
struct TCol {
    /// The model column it comes from (its lineage: what filters and context transition use).
    lin: Option<ColId>,
    /// Its name in the table; `None`: the lineage column's name.
    name: Option<String>,
    /// Its data type, when known (for calculated tables).
    ty: Option<DataType>,
    /// Named by its qualified reference (`T[C]`, an unnamed `SELECTCOLUMNS` column): a
    /// calculated table calls it `T_C` (observed on the engine of Desktop 2.158).
    qualified: bool,
    /// Without lineage, the model column a calculated table still names as this column's
    /// source (`UNION` keeps the first table's column; Desktop 2.158).
    origin: Option<ColId>,
}

impl TCol {
    fn of(c: ColId) -> TCol {
        TCol {
            lin: Some(c),
            name: None,
            ty: None,
            qualified: false,
            origin: None,
        }
    }

    fn named(name: impl Into<String>, ty: Option<DataType>) -> TCol {
        TCol {
            lin: None,
            name: Some(name.into()),
            ty,
            qualified: false,
            origin: None,
        }
    }
}

type Cols = Arc<Vec<TCol>>;

/// Combinations of grouping codes, `width` codes each, stored flat: 4 bytes per code instead of
/// a vector per combination (a grouping by customer at 10 M customers held 10 M vectors).
#[derive(Debug, Default)]
pub(super) struct Combos {
    width: usize,
    len: usize,
    codes: Vec<u32>,
    /// Their bytes, reserved against the query's memory cap (`preview_eval_mem.rs`).
    charge: mem::Charge,
}

impl Combos {
    fn new(width: usize) -> Combos {
        Combos {
            width,
            len: 0,
            codes: Vec::new(),
            charge: mem::Charge::default(),
        }
    }

    fn with_capacity(width: usize, n: usize) -> Combos {
        Combos {
            width,
            len: 0,
            codes: Vec::with_capacity(width.saturating_mul(n)),
            charge: mem::Charge::default(),
        }
    }

    /// Bytes `n` combinations of `width` codes take.
    fn bytes_for(width: usize, n: usize) -> usize {
        width.max(1).saturating_mul(n).saturating_mul(4)
    }

    /// Carries the reservation of its bytes.
    fn charged(mut self, c: mem::Charge) -> Combos {
        self.charge = c;
        self
    }

    /// Appends one combination (missing codes are 0, extra ones ignored).
    fn push(&mut self, c: &[u32]) {
        for i in 0..self.width {
            self.codes.push(c.get(i).copied().unwrap_or(0));
        }
        self.len = self.len.saturating_add(1);
    }

    /// Appends the combination of one code (a grouping of one column).
    fn push_one(&mut self, code: u32) {
        if self.width == 1 {
            self.codes.push(code);
            self.len = self.len.saturating_add(1);
        } else {
            self.push(&[code]);
        }
    }

    fn len(&self) -> usize {
        self.len
    }

    fn get(&self, i: usize) -> Option<&[u32]> {
        if i >= self.len {
            return None;
        }
        let at = i.checked_mul(self.width)?;
        self.codes.get(at..at.checked_add(self.width)?)
    }

    fn iter(&self) -> impl Iterator<Item = &[u32]> + '_ {
        (0..self.len).filter_map(|i| self.get(i))
    }

    /// Bytes it holds.
    fn bytes(&self) -> usize {
        self.codes.capacity().saturating_mul(4)
    }
}

/// Columns added to the rows of a model table (`ADDCOLUMNS`): one row of values per row.
#[derive(Debug)]
struct Ext {
    cols: Cols,
    vals: Vec<Vec<V>>,
}

/// A table value.
#[derive(Debug, Clone)]
enum Tab {
    /// Rows of a model table (row `n` of an `n`-row table is its virtual blank row), with the
    /// columns an iterator added to them.
    Rows {
        table: TabId,
        rows: Arc<Vec<u32>>,
        ext: Option<Arc<Ext>>,
    },
    /// Rows of values; each column may carry the lineage of a model column.
    Tuples { cols: Cols, rows: Arc<Vec<Vec<V>>> },
}

impl Tab {
    fn len(&self) -> usize {
        match self {
            Tab::Rows { rows, .. } => rows.len(),
            Tab::Tuples { rows, .. } => rows.len(),
        }
    }

    fn rows_of(table: TabId, rows: Vec<u32>) -> Tab {
        Tab::Rows {
            table,
            rows: Arc::new(rows),
            ext: None,
        }
    }

    fn tuples(cols: Vec<TCol>, rows: Vec<Vec<V>>) -> Tab {
        Tab::Tuples {
            cols: Arc::new(cols),
            rows: Arc::new(rows),
        }
    }

    fn empty_of(col: ColId) -> Tab {
        Tab::tuples(vec![TCol::of(col)], Vec::new())
    }
}

/// One row context.
#[derive(Debug, Clone)]
enum RowB {
    Row { table: TabId, row: u32 },
    Tuple { cols: Cols, vals: Vec<V> },
}

#[derive(Debug, Clone)]
enum Bind {
    S(V),
    T(Tab),
    /// A table evaluated on first use (`topn::Lazy`).
    L(Rc<topn::Lazy>),
}

type Env = Vec<(String, Bind)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dir {
    None,
    Single,
    Both,
}

/// One filter of a filter context.
#[derive(Debug, Clone)]
struct Filter {
    table: TabId,
    /// Columns the filter constrains (what `ALL`/`REMOVEFILTERS` and overriding filters act on).
    cols: Arc<Vec<ColId>>,
    kind: FKind,
}

#[derive(Debug, Clone)]
enum FKind {
    /// Allowed codes of the single column `cols[0]`.
    Codes(Arc<Vec<bool>>),
    /// The one allowed code of the single column `cols[0]` (a grouping cell, a context
    /// transition, `T[c] = value`): what the storage engine groups by.
    Code(u32),
    /// Allowed rows of the table (one more slot: the virtual blank row).
    Rows(Arc<Vec<bool>>),
}

#[derive(Debug, Clone, Copy)]
struct RelOverride {
    rel: usize,
    active: Option<bool>,
    dir: Option<Dir>,
}

/// A filter context. Immutable once built; visible-row masks are memoized.
#[derive(Debug, Default)]
struct FCtx {
    filters: Vec<Filter>,
    rels: Vec<RelOverride>,
    /// Calculation groups already applied in this evaluation (bit `i`: group `i` of
    /// `Engine::groups`; see `preview_eval_calc.rs`).
    applied: u64,
    memo: RefCell<HashMap<TabId, Option<Arc<Vec<bool>>>>>,
}

impl FCtx {
    fn child(&self) -> FCtx {
        FCtx {
            filters: self.filters.clone(),
            rels: self.rels.clone(),
            applied: self.applied,
            memo: RefCell::new(HashMap::new()),
        }
    }
}

#[derive(Clone, Copy)]
struct Scope<'s> {
    fc: &'s FCtx,
    rc: &'s [RowB],
    env: &'s Env,
    depth: usize,
    mdepth: usize,
}

impl<'s> Scope<'s> {
    fn deeper(&self) -> Result<Scope<'s>, Fail> {
        if self.depth >= MAX_EXPR_DEPTH {
            return unsupported("the expression nests too deeply");
        }
        Ok(Scope {
            depth: self.depth.saturating_add(1),
            ..*self
        })
    }
}

/// What one `CALCULATE` filter argument does.
enum ArgEff {
    RemoveAll,
    Remove(HashSet<ColId>),
    Rel(RelOverride, Vec<RelOverride>),
    Set {
        filters: Vec<Filter>,
        keep: bool,
        also_remove: Vec<ColId>,
    },
}

// ---------------------------------------------------------------------------------------------
// the engine
// ---------------------------------------------------------------------------------------------

#[derive(Clone)]
struct TabInfo {
    name: String,
    /// `name`, lower-cased (the key of the session's per-table caches).
    lower: String,
    cols: Vec<ColId>,
    rows: Option<usize>,
    blank_row: Option<bool>,
    /// Some column holds a distinct non-blank value per row.
    unique_col: Option<Option<ColId>>,
    date_table: bool,
}

#[derive(Clone)]
struct ColInfo {
    table: TabId,
    name: String,
    date: bool,
    /// The hidden row-number column of a table (not a value of its rows).
    row_number: bool,
    data: Option<Arc<ColData>>,
    index: Option<Arc<HashMap<Key, u32>>>,
    nums: Option<Arc<Vec<Option<f64>>>>,
    ranks: Option<Arc<Vec<u32>>>,
    /// Sorted (day, value) of the non-blank values.
    dates: Option<Arc<Vec<(i64, f64)>>>,
}

#[derive(Clone)]
struct Rel {
    from: ColId,
    to: ColId,
    from_t: TabId,
    to_t: TabId,
    active: bool,
    dir: Dir,
    from_one: bool,
    to_one: bool,
    /// from code → to code, to code → from code (with: some non-blank code maps to none).
    maps: [Option<(Arc<Vec<u32>>, bool)>; 2],
}

struct MeasureDef {
    name: String,
    expr: Option<Expr>,
    /// A measure of the session's model, as saved (not one a query defines or redefines).
    in_model: bool,
    /// Static format string.
    format: Option<String>,
    /// Dynamic format string expression.
    format_expr: Option<Expr>,
}

struct Engine<'c, 'a> {
    ctx: &'c mut Ctx<'a>,
    model: &'a SemanticModel,
    tabs: Vec<TabInfo>,
    cols: Vec<ColInfo>,
    tab_ix: Arc<HashMap<String, TabId>>,
    col_ix: Arc<HashMap<(String, String), ColId>>,
    rels: Vec<Rel>,
    measures: HashMap<String, Option<Rc<MeasureDef>>>,
    /// Column references already resolved, by expression address.
    refs: HashMap<usize, ColId>,
    group_cols: Vec<ColId>,
    /// The columns a `SUMMARIZE` subtotal row rolls up (`ISSUBTOTAL`).
    rolled: Vec<ColId>,
    /// The columns of the table whose rows an expression's type is guessed for (`[Name]`).
    type_scope: Vec<TCol>,
    /// The visual's outer filters (report, page, visual filters and slicers): what
    /// `ALLSELECTED` restores.
    outer: FCtx,
    /// Ambiguous `a/b` fiscal year ends read as day/month.
    day_first: bool,
    work: u64,
    start: Instant,
    /// Caps (the preview's by default; `dax.evaluate` sets its own).
    max_time: Duration,
    max_work: u64,
    max_candidates: usize,
    /// Data types of columns that are not in the model (query `COLUMN` definitions).
    col_types: HashMap<ColId, DataType>,
    /// Iterators and `SUMMARIZECOLUMNS` set the filters `ALLSELECTED` restores (their shadow
    /// filter context), as in a DAX query; the viewer keeps the visual's outer filters instead.
    shadow: bool,
    /// The model's calculation groups.
    groups: Vec<calc::CalcGroup>,
    /// The measures calculation items (or format strings) are evaluated for, innermost last.
    selected: Vec<Rc<MeasureDef>>,
    /// The storage engine's caches (`preview_eval_storage.rs`); `None`: it is off.
    store: Option<storage::Store>,
    /// The smallest fact that gets report-derived aggregations in this evaluation
    /// (`usize::MAX`: they are off), resolved once (`preview_eval_aggs.rs`).
    agg_floor: usize,
    /// The session's relationship indexes were synchronized with this model and data
    /// (`preview_eval_rels.rs`).
    rels_synced: bool,
    /// This evaluation's formula-engine memory, counted against its cap (`preview_eval_mem.rs`).
    mem: mem::QueryMem,
    /// How many measures of the model (not of a query's `DEFINE`) are being evaluated: the
    /// query-only `INFO.*` functions fail inside one (`preview_eval_info.rs`).
    in_model_measure: u32,
}

impl Drop for Engine<'_, '_> {
    /// Hands the storage engine's caches back to the session (for the next query).
    fn drop(&mut self) {
        self.ctx.st.activity.end();
        if self.mem.on() {
            let m = &mut self.ctx.st.memory;
            m.query_last_peak = self.mem.peak();
            m.query_peak = m.query_peak.max(self.mem.peak());
            m.query_refused = m.query_refused.saturating_add(self.mem.refused);
        }
        // the query's reservations go first: what it kept in the storage cache is then counted
        // with what the session keeps (lot 5)
        drop(std::mem::take(&mut self.mem));
        if let Some(gate) = super::store::UNPACK.with(|u| u.borrow_mut().take()) {
            if let Some(e) = gate.refused {
                self.ctx.st.memory.unpack_refused = Some(e);
            }
            drop(gate.held);
        }
        self.ctx.st.in_query = false;
        if let Some(mut s) = self.store.take()
            && !s.private
        {
            s.sync_res();
            self.ctx.st.storage = Some(s);
        }
        // the columns this evaluation held are released: the memory budget may evict them now
        if self.ctx.st.memory.budget().is_some() {
            for c in &mut self.cols {
                c.data = None;
            }
            self.ctx.st.enforce_budget();
        }
        self.ctx.st.sync_aggs_res();
        // what the session now keeps may not fit beside the other sessions': it frees the rest
        let pool = self.ctx.st.memory.pool();
        if let Some(limit) = pool.limit() {
            let used = pool.stats().used;
            if used > limit {
                self.ctx.st.evict_kept(used.saturating_sub(limit));
            }
        }
    }
}

fn and_into(acc: &mut Option<Vec<bool>>, m: Vec<bool>) {
    match acc {
        None => *acc = Some(m),
        Some(a) => {
            for (x, y) in a.iter_mut().zip(m) {
                *x &= y;
            }
        }
    }
}

fn visible(mask: Option<&Arc<Vec<bool>>>, row: usize) -> bool {
    mask.is_none_or(|m| m.get(row).copied().unwrap_or(false))
}

fn arg(args: &[Option<Expr>], i: usize) -> Option<&Expr> {
    args.get(i).and_then(Option::as_ref)
}

/// A short name of a table expression for messages: `FILTER()`, `'Sales'`, a variable's name.
fn expr_label(e: &Expr) -> String {
    match e {
        Expr::Call { name, .. } => format!("{}()", name.to_ascii_uppercase()),
        Expr::Name { name, .. } => format!("'{name}'"),
        Expr::TableCtor { .. } => "a table constructor".into(),
        Expr::Var { .. } => "a VAR expression".into(),
        _ => "an expression".into(),
    }
}

fn need<'e>(args: &'e [Option<Expr>], i: usize, f: &str) -> Result<&'e Expr, Fail> {
    match arg(args, i) {
        Some(e) => Ok(e),
        None => unsupported(format!("{f}() with a missing argument")),
    }
}

fn is_table_function(name: &str) -> bool {
    bicraft_dax::signature(name).is_some_and(|s| matches!(s.returns, Returns::Fixed(ValueType::Table)))
}

fn keyword(e: &Expr) -> Option<String> {
    match e {
        Expr::Name { name, .. } => Some(name.to_ascii_uppercase()),
        _ => None,
    }
}

/// Pushes the row contexts of row `i` of `t` (a model row, plus the columns added to it);
/// returns how many.
fn push_row(t: &Tab, i: usize, rc: &mut Vec<RowB>) -> usize {
    match t {
        Tab::Rows { table, rows, ext } => {
            let Some(r) = rows.get(i) else {
                return 0;
            };
            rc.push(RowB::Row {
                table: *table,
                row: *r,
            });
            match ext {
                Some(e) => {
                    rc.push(RowB::Tuple {
                        cols: Arc::clone(&e.cols),
                        vals: e.vals.get(i).cloned().unwrap_or_default(),
                    });
                    2
                }
                None => 1,
            }
        }
        Tab::Tuples { cols, rows } => match rows.get(i) {
            Some(r) => {
                rc.push(RowB::Tuple {
                    cols: Arc::clone(cols),
                    vals: r.clone(),
                });
                1
            }
            None => 0,
        },
    }
}

/// The rows `picked` (positions) of `t`, in that order.
fn subset(t: &Tab, picked: &[usize]) -> Tab {
    match t {
        Tab::Rows { table, rows, ext } => Tab::Rows {
            table: *table,
            rows: Arc::new(picked.iter().filter_map(|i| rows.get(*i).copied()).collect()),
            ext: ext.as_ref().map(|e| {
                Arc::new(Ext {
                    cols: Arc::clone(&e.cols),
                    vals: picked
                        .iter()
                        .map(|i| e.vals.get(*i).cloned().unwrap_or_default())
                        .collect(),
                })
            }),
        },
        Tab::Tuples { cols, rows } => Tab::Tuples {
            cols: Arc::clone(cols),
            rows: Arc::new(picked.iter().filter_map(|i| rows.get(*i).cloned()).collect()),
        },
    }
}

impl<'c, 'a> Engine<'c, 'a> {
    fn new(ctx: &'c mut Ctx<'a>) -> Self {
        let model: &'a SemanticModel = ctx.model;
        let reg = Registry::of_ctx(ctx);
        CONVERT_LOCALE.with(|c| c.set(reg.locale));
        Engine {
            ctx,
            model,
            tabs: reg.tabs.clone(),
            cols: reg.cols.clone(),
            tab_ix: Arc::clone(&reg.tab_ix),
            col_ix: Arc::clone(&reg.col_ix),
            rels: reg.rels.clone(),
            measures: HashMap::new(),
            refs: HashMap::new(),
            group_cols: Vec::new(),
            rolled: Vec::new(),
            type_scope: Vec::new(),
            outer: FCtx::default(),
            day_first: reg.day_first,
            work: 0,
            start: Instant::now(),
            max_time: MAX_TIME,
            max_work: MAX_WORK,
            max_candidates: MAX_CANDIDATES,
            col_types: HashMap::new(),
            shadow: false,
            // calculation items hold parsed expressions, not shared across threads: per query
            // (a pass over the tables without calculation groups)
            groups: calc::load(model, |t, c| {
                reg.col_ix.get(&(t.to_lowercase(), c.to_lowercase())).copied()
            }),
            selected: Vec::new(),
            store: None,
            agg_floor: usize::MAX,
            rels_synced: false,
            mem: mem::QueryMem::default(),
            in_model_measure: 0,
        }
        .with_store()
    }
}

/// What an evaluation reads of the model's shape (tables, columns, relationships, culture), without data: built once per model and kept by the session for its own model
/// (by model hash: any edit builds it again), instead of once per query, which costs
/// O(model columns) (`docs/storage-engine.md`, "Per-query costs").
pub(super) struct Registry {
    /// The hash of the model it was built from (0: not the session's model).
    model: u64,
    tabs: Vec<TabInfo>,
    cols: Vec<ColInfo>,
    tab_ix: Arc<HashMap<String, TabId>>,
    col_ix: Arc<HashMap<(String, String), ColId>>,
    rels: Vec<Rel>,
    day_first: bool,
    locale: &'static Locale,
}

impl Registry {
    /// The registry of the evaluation's model: the session's kept one when the model is the
    /// session's own and unchanged, else built (and kept, for the session's model).
    fn of_ctx(ctx: &mut Ctx<'_>) -> Arc<Registry> {
        let own = ctx.s.model.as_ref().is_some_and(|m| std::ptr::eq(m, ctx.model));
        if !own {
            return Arc::new(Registry::of(ctx.model, 0));
        }
        let hash = ctx.st.model_hash(ctx.s);
        if let Some(r) = &ctx.st.registry
            && r.model == hash
        {
            return Arc::clone(r);
        }
        let r = Arc::new(Registry::of(ctx.model, hash));
        ctx.st.registry = Some(Arc::clone(&r));
        r
    }

    fn of(model: &SemanticModel, hash: u64) -> Registry {
        let mut tabs = Vec::new();
        let mut cols = Vec::new();
        let mut tab_ix = HashMap::new();
        let mut col_ix = HashMap::new();
        for t in &model.tables {
            let ti = tabs.len();
            let lower = t.name.to_lowercase();
            tab_ix.entry(lower.clone()).or_insert(ti);
            let mut ids = Vec::new();
            for c in &t.columns {
                let ci = cols.len();
                cols.push(ColInfo {
                    table: ti,
                    name: c.name.clone(),
                    date: c.data_type == DataType::DateTime,
                    row_number: c.extra.get("type").and_then(Value::as_str) == Some("rowNumber"),
                    data: None,
                    index: None,
                    nums: None,
                    ranks: None,
                    dates: None,
                });
                col_ix.entry((lower.clone(), c.name.to_lowercase())).or_insert(ci);
                ids.push(ci);
            }
            let date_table = t
                .extra
                .get("dataCategory")
                .and_then(Value::as_str)
                .is_some_and(|c| c.eq_ignore_ascii_case("time"));
            tabs.push(TabInfo {
                name: t.name.clone(),
                lower: lower.clone(),
                cols: ids,
                rows: None,
                blank_row: None,
                unique_col: None,
                date_table,
            });
        }
        let mut rels = Vec::new();
        for r in &model.relationships {
            let from = col_ix.get(&(r.from_table.to_lowercase(), r.from_column.to_lowercase()));
            let to = col_ix.get(&(r.to_table.to_lowercase(), r.to_column.to_lowercase()));
            let (Some(&from), Some(&to)) = (from, to) else {
                continue;
            };
            let (Some(from_t), Some(to_t)) = (
                cols.get(from).map(|c: &ColInfo| c.table),
                cols.get(to).map(|c: &ColInfo| c.table),
            ) else {
                continue;
            };
            if from_t == to_t {
                continue;
            }
            let is = |c: Option<&str>, v: &str| c.is_some_and(|c| c.eq_ignore_ascii_case(v));
            rels.push(Rel {
                from,
                to,
                from_t,
                to_t,
                active: r.is_active,
                dir: if is(r.cross_filtering_behavior.as_deref(), "bothDirections") {
                    Dir::Both
                } else {
                    Dir::Single
                },
                from_one: is(r.from_cardinality.as_deref(), "one"),
                to_one: !is(r.to_cardinality.as_deref(), "many"),
                maps: [None, None],
            });
        }
        let day_first = !model
            .culture
            .as_deref()
            .is_some_and(|c| c.eq_ignore_ascii_case("en-US"));
        let locale = model
            .culture
            .as_deref()
            .and_then(Locale::find)
            .unwrap_or(Locale::en_us());
        Registry {
            model: hash,
            tabs,
            cols,
            tab_ix: Arc::new(tab_ix),
            col_ix: Arc::new(col_ix),
            rels,
            day_first,
            locale,
        }
    }
}

impl<'c, 'a> Engine<'c, 'a> {
    /// The storage engine's caches: those the session kept for this model and data.
    fn with_store(mut self) -> Self {
        self.ctx.st.activity.begin();
        let m = &self.ctx.st.memory;
        self.mem = mem::QueryMem::new(m.query_cap(), m.pool(), m.background);
        // under a memory limit, the codes this evaluation unpacks are reserved too (lot 5)
        self.ctx.st.memory.unpack_refused = None;
        self.ctx.st.in_query = true;
        self.ctx.st.memory.begin_evaluation();
        let m = &self.ctx.st.memory;
        if m.pool().limit().is_some() {
            let gate = super::store::UnpackGate {
                pool: m.pool(),
                class: m.kept_class(),
                held: Vec::new(),
                refused: None,
            };
            super::store::UNPACK.with(|u| *u.borrow_mut() = Some(gate));
        }
        // a backup still to move to its file (no background work ran since it was opened)
        if self.ctx.st.memory.spill_pending {
            self.ctx.st.spill_now();
        }
        self.agg_floor = storage::aggs::agg_floor(&self.ctx.st.aggs_config);
        if storage_engine() {
            let key = {
                let s = self.ctx.s;
                let st = &mut *self.ctx.st;
                let mut h = std::collections::hash_map::DefaultHasher::new();
                std::hash::Hash::hash(&(st.model_hash(s), st.generation), &mut h);
                std::hash::Hasher::finish(&h)
            };
            let kept = self.ctx.st.storage.take();
            let (pool, class) = (self.ctx.st.memory.pool(), self.ctx.st.memory.kept_class());
            let mut store = storage::Store::reuse(kept, key, pool, class);
            // what the measures of this evaluation say (a query may redefine some): part of the
            // keys of results kept by expression text
            store.measures = self.measure_facts().fingerprint;
            // under a memory budget its cached aggregations take at most a quarter of it
            store.cap = self.ctx.st.memory.budget().map_or(0, |b| (b / 4).max(1));
            self.store = Some(store);
        }
        self
    }

    fn tick(&mut self, n: usize) -> Result<(), Fail> {
        self.work = self.work.saturating_add(u64::try_from(n).unwrap_or(u64::MAX));
        if self.work > self.max_work || self.start.elapsed() > self.max_time {
            return unsupported("the measures need more work than the preview allows");
        }
        if self.ctx.s.is_cancelled() {
            return unsupported(crate::CANCELLED);
        }
        Ok(())
    }

    // --- registry ---------------------------------------------------------------------------

    fn table_id(&self, name: &str) -> Option<TabId> {
        self.tab_ix.get(&name.to_lowercase()).copied()
    }

    fn col_id(&self, table: &str, col: &str) -> Option<ColId> {
        self.col_ix
            .get(&(table.to_lowercase(), col.to_lowercase()))
            .copied()
    }

    fn col_table(&self, c: ColId) -> Result<TabId, Fail> {
        match self.cols.get(c) {
            Some(i) => Ok(i.table),
            None => internal(),
        }
    }

    fn col_name(&self, c: ColId) -> String {
        let Some(i) = self.cols.get(c) else {
            return String::new();
        };
        let t = self.tabs.get(i.table).map_or("", |t| t.name.as_str());
        format!("'{t}'[{}]", i.name)
    }

    fn table_cols(&self, t: TabId) -> Vec<ColId> {
        self.tabs.get(t).map(|t| t.cols.clone()).unwrap_or_default()
    }

    /// The columns whose values a row of `t` holds (all but the row number).
    fn value_cols(&self, t: TabId) -> Vec<ColId> {
        self.table_cols(t)
            .into_iter()
            .filter(|c| self.cols.get(*c).is_some_and(|i| !i.row_number))
            .collect()
    }

    /// The columns of a table value, in the order of [`Engine::tab_values`].
    fn tab_cols(&self, t: &Tab) -> Vec<TCol> {
        match t {
            Tab::Rows { table, ext, .. } => {
                let mut out: Vec<TCol> = self.value_cols(*table).into_iter().map(TCol::of).collect();
                if let Some(e) = ext {
                    out.extend(e.cols.iter().cloned());
                }
                out
            }
            Tab::Tuples { cols, .. } => cols.as_ref().clone(),
        }
    }

    /// The name of a column of a table value.
    fn tcol_name(&self, c: &TCol) -> String {
        match (&c.name, c.lin.and_then(|l| self.cols.get(l))) {
            (Some(n), _) => n.clone(),
            (None, Some(i)) => i.name.clone(),
            (None, None) => "Value".to_string(),
        }
    }

    /// The value of the column `[name]` an iterator added (no lineage), from the innermost row
    /// context that has a column of that name; `None` when that is a model column (or none).
    fn ext_value(&self, name: &str, rc: &[RowB]) -> Option<V> {
        for b in rc.iter().rev() {
            match b {
                RowB::Row { table, .. } => {
                    let own = self.table_cols(*table).iter().any(|c| {
                        self.cols
                            .get(*c)
                            .is_some_and(|i| i.name.eq_ignore_ascii_case(name))
                    });
                    if own {
                        return None;
                    }
                }
                RowB::Tuple { cols, vals } => {
                    if let Some(i) = cols
                        .iter()
                        .position(|c| self.tcol_name(c).eq_ignore_ascii_case(name))
                    {
                        let c = cols.get(i)?;
                        return if c.lin.is_none() {
                            Some(vals.get(i).cloned().unwrap_or(V::Blank))
                        } else {
                            None
                        };
                    }
                }
            }
        }
        None
    }

    fn rows(&mut self, t: TabId) -> Result<usize, Fail> {
        let Some(info) = self.tabs.get(t) else {
            return internal();
        };
        if let Some(n) = info.rows {
            return Ok(n);
        }
        if let Some(n) = self.group_rows(t) {
            return Ok(n);
        }
        let name = info.name.clone();
        let n = self.ctx.rows(&name)?;
        if let Some(info) = self.tabs.get_mut(t) {
            info.rows = Some(n);
        }
        Ok(n)
    }

    fn col(&mut self, c: ColId) -> Result<Arc<ColData>, Fail> {
        let Some(info) = self.cols.get(c) else {
            return internal();
        };
        if let Some(d) = &info.data {
            return Ok(Arc::clone(d));
        }
        let (t, name) = (info.table, info.name.clone());
        let tname = self.tabs.get(t).map(|t| t.name.clone()).unwrap_or_default();
        // a calculation group's rows are its items (never the stored ones)
        let d = match self.group_col_data(c) {
            Some(d) => Arc::new(d?),
            None => self.ctx.column(&tname, &name)?,
        };
        if d.rows() != self.rows(t)? {
            return dax_error(format!("columns of '{tname}' have different lengths"));
        }
        if let Some(info) = self.cols.get_mut(c) {
            info.data = Some(Arc::clone(&d));
        }
        Ok(d)
    }

    fn index(&mut self, c: ColId) -> Result<Arc<HashMap<Key, u32>>, Fail> {
        if let Some(i) = self.cols.get(c).and_then(|i| i.index.clone()) {
            return Ok(i);
        }
        let d = self.col(c)?;
        let dv = d.derived.get();
        if dv.index.get().is_none() {
            // part of the column once built (lot 5: reserved first, counted with it after the query)
            self.keep_charge(
                &|| "a column's value index".into(),
                d.values.len().saturating_mul(48),
            )?;
        }
        let m = Arc::clone(dv.index.get_or_init(|| {
            let mut m = HashMap::with_capacity(d.values.len());
            for (code, v) in d.values.iter().enumerate().skip(1) {
                if let (Some(k), Ok(code)) = (Key::of(v), u32::try_from(code)) {
                    m.entry(k).or_insert(code);
                }
            }
            Arc::new(m)
        }));
        if let Some(info) = self.cols.get_mut(c) {
            info.index = Some(Arc::clone(&m));
        }
        Ok(m)
    }

    fn nums(&mut self, c: ColId) -> Result<Arc<Vec<Option<f64>>>, Fail> {
        if let Some(n) = self.cols.get(c).and_then(|i| i.nums.clone()) {
            return Ok(n);
        }
        let d = self.col(c)?;
        let dv = d.derived.get();
        if dv.nums.get().is_none() {
            self.keep_charge(&|| "a column's numbers".into(), d.values.len().saturating_mul(16))?;
        }
        let n = Arc::clone(
            dv.nums
                .get_or_init(|| Arc::new(d.values.iter().map(as_num).collect::<Vec<_>>())),
        );
        if let Some(info) = self.cols.get_mut(c) {
            info.nums = Some(Arc::clone(&n));
        }
        Ok(n)
    }

    fn ranks(&mut self, c: ColId) -> Result<Arc<Vec<u32>>, Fail> {
        if let Some(r) = self.cols.get(c).and_then(|i| i.ranks.clone()) {
            return Ok(r);
        }
        let d = self.col(c)?;
        if d.derived.get().ranks.get().is_none() {
            // the ranks, the codes by rank and the sort's order
            self.keep_charge(
                &|| "a column's sort ranks".into(),
                d.values.len().saturating_mul(16),
            )?;
        }
        let rank = Arc::new(self.rank_table(&d).0.clone());
        if let Some(info) = self.cols.get_mut(c) {
            info.ranks = Some(Arc::clone(&rank));
        }
        Ok(rank)
    }

    /// (rank per code, code per rank) of a column, computed once per decoded column.
    fn rank_table(&self, d: &ColData) -> Arc<(Vec<u32>, Vec<u32>)> {
        let dv = d.derived.get();
        Arc::clone(dv.ranks.get_or_init(|| {
            let mut order: Vec<usize> = (0..d.values.len()).collect();
            // each value's sort key once (`compare`'s: blank, numbers, text without case,
            // booleans), not two copies of the values and their lower-cased text per comparison
            let keys: Vec<(u8, f64, Option<String>)> = d
                .values
                .iter()
                .map(|v| match v {
                    CellValue::Null => (0, 0.0, None),
                    CellValue::Text(s) => (2, 0.0, Some(s.to_lowercase())),
                    CellValue::Bool(b) => (3, f64::from(u8::from(*b)), None),
                    other => (1, as_num(other).unwrap_or(0.0), None),
                })
                .collect();
            let key = |i: usize| keys.get(i);
            let cmp = |a: &usize, b: &usize| match (key(*a), key(*b)) {
                (Some(x), Some(y)) => {
                    x.0.cmp(&y.0)
                        .then_with(|| x.1.partial_cmp(&y.1).unwrap_or(Ordering::Equal))
                        .then_with(|| x.2.as_deref().unwrap_or("").cmp(y.2.as_deref().unwrap_or("")))
                }
                _ => Ordering::Equal,
            };
            if keys.iter().any(|k| k.1.is_nan()) {
                order.sort_by(cmp);
            } else {
                // a total order once ties go by position: the stable sort's result, faster
                order.sort_unstable_by(|a, b| cmp(a, b).then_with(|| a.cmp(b)));
            }
            let mut rank = vec![0u32; d.values.len()];
            for (r, code) in order.iter().enumerate() {
                if let Some(slot) = rank.get_mut(*code) {
                    *slot = u32::try_from(r).unwrap_or(u32::MAX);
                }
            }
            let by_rank = order.iter().map(|c| u32::try_from(*c).unwrap_or(0)).collect();
            Arc::new((rank, by_rank))
        }))
    }

    /// Sorted (day, value) of every non-blank value of a date column.
    fn all_dates(&mut self, c: ColId) -> Result<Arc<Vec<(i64, f64)>>, Fail> {
        if let Some(d) = self.cols.get(c).and_then(|i| i.dates.clone()) {
            return Ok(d);
        }
        let d = self.col(c)?;
        let mut out: Vec<(i64, f64)> = d
            .values
            .iter()
            .skip(1)
            .filter_map(as_num)
            .filter(|x| x.is_finite() && x.abs() < 3_000_000.0)
            .map(|x| (x.floor() as i64, x))
            .collect();
        out.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal))
        });
        out.dedup_by_key(|p| p.0);
        let out = Arc::new(out);
        if let Some(info) = self.cols.get_mut(c) {
            info.dates = Some(Arc::clone(&out));
        }
        Ok(out)
    }

    /// Code of a value in a column (`Some(0)`: blank; `None`: not in the column).
    fn code_of(&mut self, c: ColId, v: &V) -> Result<Option<u32>, Fail> {
        let Some(k) = v.key() else {
            return Ok(Some(0));
        };
        Ok(self.index(c)?.get(&k).copied())
    }

    /// Code map of a relationship: `from_to` maps from-column codes to to-column codes.
    fn rel_map(&mut self, r: usize, from_to: bool) -> Result<Arc<Vec<u32>>, Fail> {
        Ok(self.rel_entry(r, from_to)?.0)
    }

    /// [`Engine::rel_map`], with: some non-blank code maps to none. Kept by the session for
    /// every later query on the same model and data (`preview_eval_rels.rs`).
    fn rel_entry(&mut self, r: usize, from_to: bool) -> Result<(Arc<Vec<u32>>, bool), Fail> {
        let slot = usize::from(!from_to);
        let Some(rel) = self.rels.get(r) else {
            return internal();
        };
        if let Some(m) = rel.maps.get(slot).and_then(Clone::clone) {
            return Ok(m);
        }
        let (src, dst) = if from_to {
            (rel.from, rel.to)
        } else {
            (rel.to, rel.from)
        };
        let src = self.col(src)?;
        let dst = self.col(dst)?;
        let m = self.rel_map_of(&src, &dst);
        let m = (m.map, m.unmatched);
        if let Some(s) = self.rels.get_mut(r).and_then(|rel| rel.maps.get_mut(slot)) {
            *s = Some(m.clone());
        }
        Ok(m)
    }

    /// Whether `t` has a virtual blank row: some active relationship where it is the one side
    /// has keys on the other side that it does not hold.
    fn has_blank_row(&mut self, t: TabId) -> Result<bool, Fail> {
        if let Some(b) = self.tabs.get(t).and_then(|i| i.blank_row) {
            return Ok(b);
        }
        let mut found = false;
        for r in 0..self.rels.len() {
            let Some(rel) = self.rels.get(r) else {
                continue;
            };
            if !rel.active {
                continue;
            }
            let other_from = if rel.to_t == t && rel.to_one {
                true
            } else if rel.from_t == t && rel.from_one {
                false
            } else {
                continue;
            };
            let other_col = if other_from { rel.from } else { rel.to };
            let (_, unmatched_value) = self.rel_entry(r, other_from)?;
            let data = self.col(other_col)?;
            let blank_key = self.blank_code_kept(&data)?;
            if unmatched_value || blank_key {
                found = true;
                break;
            }
        }
        if let Some(info) = self.tabs.get_mut(t) {
            info.blank_row = Some(found);
        }
        Ok(found)
    }

    fn unique_col(&mut self, t: TabId) -> Result<Option<ColId>, Fail> {
        if let Some(u) = self.tabs.get(t).and_then(|i| i.unique_col) {
            return Ok(u);
        }
        let n = self.rows(t)?;
        let mut found = None;
        for c in self.table_cols(t) {
            let Ok(d) = self.col(c) else {
                continue;
            };
            // (whether some row is blank is kept by the session: a scan of the rows otherwise)
            if d.values.len() == n.saturating_add(1)
                && !self.blank_code_kept(&d).unwrap_or_else(|_| d.codes.contains(&0))
            {
                found = Some(c);
                break;
            }
        }
        if let Some(info) = self.tabs.get_mut(t) {
            info.unique_col = Some(found);
        }
        Ok(found)
    }

    /// `t` and the tables it reaches through active many-to-one relationships (its expanded
    /// table).
    fn expanded(&self, t: TabId) -> Vec<(TabId, Option<(usize, TabId)>)> {
        let mut out: Vec<(TabId, Option<(usize, TabId)>)> = vec![(t, None)];
        let mut frontier = vec![t];
        for _ in 0..MAX_HOPS {
            let mut next = Vec::new();
            for x in &frontier {
                for (ri, r) in self.rels.iter().enumerate() {
                    if r.active && r.from_t == *x && r.to_one && !out.iter().any(|(y, _)| *y == r.to_t) {
                        out.push((r.to_t, Some((ri, *x))));
                        next.push(r.to_t);
                    }
                }
            }
            frontier = next;
        }
        out
    }

    fn expanded_cols(&self, t: TabId) -> HashSet<ColId> {
        self.expanded(t)
            .iter()
            .flat_map(|(x, _)| self.table_cols(*x))
            .collect()
    }

    // --- masks ------------------------------------------------------------------------------

    fn rel_state(&self, fc: &FCtx, r: usize) -> (bool, Dir) {
        let Some(rel) = self.rels.get(r) else {
            return (false, Dir::None);
        };
        let mut st = (rel.active, rel.dir);
        for o in fc.rels.iter().filter(|o| o.rel == r) {
            if let Some(a) = o.active {
                st.0 = a;
            }
            if let Some(d) = o.dir {
                st.1 = d;
            }
        }
        st
    }

    /// Visible rows of `t` (one more slot: the virtual blank row); `None`: all rows.
    fn mask(&mut self, fc: &FCtx, t: TabId) -> Result<Option<Arc<Vec<bool>>>, Fail> {
        if fc.filters.is_empty() {
            return Ok(None);
        }
        if let Some(m) = fc.memo.borrow().get(&t) {
            return Ok(m.clone());
        }
        let m = if self.store.is_some() {
            match self.pred(fc, t)? {
                Some(p) => Some(Arc::new(self.materialize(&p)?)),
                None => None,
            }
        } else {
            let mut visited = vec![false; self.tabs.len()];
            self.mask_from(fc, t, None, &mut visited, 0)?.map(Arc::new)
        };
        if let Some(m) = &m {
            let n = m.len();
            self.hold_mask(m, &move || format!("a row mask of {n} rows"))?;
        }
        fc.memo.borrow_mut().insert(t, m.clone());
        Ok(m)
    }

    fn mask_from(
        &mut self,
        fc: &FCtx,
        t: TabId,
        via: Option<usize>,
        visited: &mut [bool],
        depth: usize,
    ) -> Result<Option<Vec<bool>>, Fail> {
        let n = self.rows(t)?;
        let mut acc: Option<Vec<bool>> = None;
        for f in fc.filters.iter().filter(|f| f.table == t) {
            let m = self.filter_rows(f, n)?;
            and_into(&mut acc, m);
        }
        if depth >= MAX_FILTER_HOPS {
            return Ok(acc);
        }
        if let Some(v) = visited.get_mut(t) {
            *v = true;
        }
        for r in 0..self.rels.len() {
            if via == Some(r) {
                continue;
            }
            let (active, dir) = self.rel_state(fc, r);
            if !active || dir == Dir::None {
                continue;
            }
            let Some(rel) = self.rels.get(r) else {
                continue;
            };
            let (other, t_is_from) = if rel.from_t == t {
                (rel.to_t, true)
            } else if rel.to_t == t {
                (rel.from_t, false)
            } else {
                continue;
            };
            // filters flow into the from (many) side, and into the to side when both ways
            if !t_is_from && dir != Dir::Both {
                continue;
            }
            if visited.get(other).copied().unwrap_or(true) {
                continue;
            }
            let Some(sub) = self.mask_from(fc, other, Some(r), visited, depth.saturating_add(1))? else {
                continue;
            };
            let m = self.propagate(r, !t_is_from, &sub, n)?;
            and_into(&mut acc, m);
        }
        if let Some(v) = visited.get_mut(t) {
            *v = false;
        }
        Ok(acc)
    }

    /// Rows of the table at one end of relationship `r` allowed by the visible rows `sub` of the
    /// table at the other end (`other_is_from`: `sub` is the from side's).
    fn propagate(
        &mut self,
        r: usize,
        other_is_from: bool,
        sub: &[bool],
        n_t: usize,
    ) -> Result<Vec<bool>, Fail> {
        let (tc, t_ok, t_blank) = self.propagate_codes(r, other_is_from, sub)?;
        let tcd = self.col(tc)?;
        self.tick(tcd.codes.len())?;
        let mut m: Vec<bool> = tcd
            .codes
            .iter()
            .map(|c| t_ok.get(*c as usize).copied().unwrap_or(false))
            .collect();
        m.resize(n_t, false);
        m.push(t_blank);
        Ok(m)
    }

    /// [`Engine::propagate`] as allowed codes: the column of `t` at its end of `r`, which of its
    /// codes are allowed, and whether `t`'s virtual blank row is.
    fn propagate_codes(
        &mut self,
        r: usize,
        other_is_from: bool,
        sub: &[bool],
    ) -> Result<(ColId, Vec<bool>, bool), Fail> {
        let Some(rel) = self.rels.get(r) else {
            return internal();
        };
        let (oc, tc) = if other_is_from {
            (rel.from, rel.to)
        } else {
            (rel.to, rel.from)
        };
        let (other_one, t_one) = if other_is_from {
            (rel.from_one, rel.to_one)
        } else {
            (rel.to_one, rel.from_one)
        };
        let ocd = self.col(oc)?;
        self.tick(ocd.codes.len())?;
        let mut o_ok = vec![false; ocd.values.len()];
        for (row, code) in ocd.codes.iter().enumerate() {
            if sub.get(row).copied().unwrap_or(false)
                && let Some(s) = o_ok.get_mut(*code as usize)
            {
                *s = true;
            }
        }
        let o_blank = sub.get(ocd.codes.len()).copied().unwrap_or(false);
        // t codes → o codes
        let t2o = self.rel_map(r, !other_is_from)?;
        let t_ok: Vec<bool> = t2o
            .iter()
            .map(|oc| {
                if *oc == NONE {
                    other_one && o_blank
                } else {
                    o_ok.get(*oc as usize).copied().unwrap_or(false)
                }
            })
            .collect();
        let t_blank = if t_one {
            let o2t = self.rel_map(r, other_is_from)?;
            o_ok.iter().zip(o2t.iter()).any(|(ok, m)| *ok && *m == NONE)
        } else {
            t_ok.first().copied().unwrap_or(false)
        };
        Ok((tc, t_ok, t_blank))
    }

    fn filter_rows(&mut self, f: &Filter, n: usize) -> Result<Vec<bool>, Fail> {
        match &f.kind {
            FKind::Codes(allowed) => {
                let Some(c) = f.cols.first().copied() else {
                    return internal();
                };
                let d = self.col(c)?;
                self.tick(n)?;
                let mut m: Vec<bool> = d
                    .codes
                    .iter()
                    .map(|code| allowed.get(*code as usize).copied().unwrap_or(false))
                    .collect();
                m.resize(n, false);
                m.push(allowed.first().copied().unwrap_or(false));
                Ok(m)
            }
            FKind::Code(x) => {
                let Some(c) = f.cols.first().copied() else {
                    return internal();
                };
                let d = self.col(c)?;
                self.tick(n)?;
                let mut m: Vec<bool> = d.codes.iter().map(|code| code == x).collect();
                m.resize(n, false);
                m.push(*x == 0);
                Ok(m)
            }
            FKind::Rows(m) => {
                let mut m = m.as_ref().clone();
                m.resize(n.saturating_add(1), false);
                Ok(m)
            }
        }
    }

    /// Distinct codes of a column among the rows visible in `fc` (with the virtual blank row
    /// when `blank_row` and it is visible).
    fn values_codes(&mut self, fc: &FCtx, c: ColId, blank_row: bool) -> Result<Vec<u32>, Fail> {
        if self.store.is_some()
            && let Some(codes) = self.values_codes_storage(fc, c, blank_row)?
        {
            return Ok(codes);
        }
        let t = self.col_table(c)?;
        let d = self.col(c)?;
        let mask = self.mask(fc, t)?;
        self.tick(d.codes.len())?;
        let mut seen = vec![false; d.values.len()];
        for (row, code) in d.codes.iter().enumerate() {
            if visible(mask.as_ref(), row)
                && let Some(s) = seen.get_mut(*code as usize)
            {
                *s = true;
            }
        }
        if blank_row
            && visible(mask.as_ref(), d.codes.len())
            && self.has_blank_row(t)?
            && let Some(s) = seen.first_mut()
        {
            *s = true;
        }
        Ok(seen
            .iter()
            .enumerate()
            .filter(|(_, s)| **s)
            .filter_map(|(i, _)| u32::try_from(i).ok())
            .collect())
    }

    fn value_of_code(&mut self, c: ColId, code: u32) -> Result<V, Fail> {
        Ok(self
            .col(c)?
            .values
            .get(code as usize)
            .map_or(V::Blank, V::of_cell))
    }

    fn visible_rows(&mut self, fc: &FCtx, t: TabId) -> Result<Vec<u32>, Fail> {
        if self.store.is_some() {
            return self.visible_rows_storage(fc, t);
        }
        let n = self.rows(t)?;
        let mask = self.mask(fc, t)?;
        self.tick(n)?;
        Ok((0..n)
            .filter(|r| visible(mask.as_ref(), *r))
            .filter_map(|r| u32::try_from(r).ok())
            .collect())
    }

    // --- filters ----------------------------------------------------------------------------

    fn codes_filter(&mut self, c: ColId, codes: &[u32]) -> Result<Filter, Fail> {
        if let Some(first) = codes.first().copied()
            && codes.iter().all(|x| *x == first)
        {
            return Ok(Filter {
                table: self.col_table(c)?,
                cols: Arc::new(vec![c]),
                kind: FKind::Code(first),
            });
        }
        let d = self.col(c)?;
        let mut allowed = vec![false; d.values.len()];
        for code in codes {
            if let Some(s) = allowed.get_mut(*code as usize) {
                *s = true;
            }
        }
        Ok(Filter {
            table: self.col_table(c)?,
            cols: Arc::new(vec![c]),
            kind: FKind::Codes(Arc::new(allowed)),
        })
    }

    /// The filters a table value places when used as a filter.
    fn tab_filters(&mut self, tab: &Tab) -> Result<Vec<Filter>, Fail> {
        match tab {
            Tab::Rows { table, rows, .. } => {
                let n = self.rows(*table)?;
                let mut m = vec![false; n.saturating_add(1)];
                for r in rows.iter() {
                    if let Some(s) = m.get_mut(*r as usize) {
                        *s = true;
                    }
                }
                self.rows_filters(*table, m)
            }
            Tab::Tuples { cols, rows } => {
                // Columns without lineage (computed ones) filter nothing.
                let at: Vec<usize> = (0..cols.len())
                    .filter(|i| cols.get(*i).is_some_and(|c| c.lin.is_some()))
                    .collect();
                let lineage: Vec<ColId> = cols.iter().filter_map(|c| c.lin).collect();
                let Some(first) = lineage.first().copied() else {
                    return unsupported("a filter table without column lineage");
                };
                let projected: Vec<Vec<V>>;
                let rows: &Vec<Vec<V>> = if at.len() == cols.len() {
                    rows
                } else {
                    projected = rows
                        .iter()
                        .map(|r| {
                            at.iter()
                                .map(|i| r.get(*i).cloned().unwrap_or(V::Blank))
                                .collect()
                        })
                        .collect();
                    &projected
                };
                let t = self.col_table(first)?;
                for c in &lineage {
                    if self.col_table(*c)? != t {
                        return unsupported("a filter table whose columns come from several tables");
                    }
                }
                if lineage.len() == 1
                    && cols.len() == 1
                    && let Some(codes) = self.tuple_codes(tab, first)
                {
                    // built from codes: the codes the lookups below would give
                    return Ok(vec![self.codes_filter(first, &codes)?]);
                }
                if lineage.len() == 1 {
                    let mut codes = Vec::with_capacity(rows.len());
                    for r in rows.iter() {
                        if let Some(code) = self.code_of(first, r.first().unwrap_or(&V::Blank))? {
                            codes.push(code);
                        }
                    }
                    return Ok(vec![self.codes_filter(first, &codes)?]);
                }
                let mut set: HashSet<Vec<u32>> = HashSet::new();
                for r in rows.iter() {
                    let mut key = Vec::with_capacity(lineage.len());
                    for (c, v) in lineage.iter().zip(r) {
                        match self.code_of(*c, v)? {
                            Some(code) => key.push(code),
                            None => break,
                        }
                    }
                    if key.len() == lineage.len() {
                        set.insert(key);
                    }
                }
                let datas = lineage
                    .iter()
                    .map(|c| self.col(*c))
                    .collect::<Result<Vec<_>, _>>()?;
                let n = self.rows(t)?;
                self.tick(n.saturating_mul(lineage.len()))?;
                let mut m: Vec<bool> = (0..n)
                    .map(|row| {
                        let key: Vec<u32> = datas.iter().map(|d| d.codes.get(row).unwrap_or(0)).collect();
                        set.contains(&key)
                    })
                    .collect();
                m.push(set.contains(&vec![0; lineage.len()]));
                Ok(vec![Filter {
                    table: t,
                    cols: Arc::new(lineage),
                    kind: FKind::Rows(Arc::new(m)),
                }])
            }
        }
    }

    /// A row filter on `t` covering all its columns, plus the filters it places on the tables of
    /// its expanded table.
    fn rows_filters(&mut self, t: TabId, m: Vec<bool>) -> Result<Vec<Filter>, Fail> {
        let mut masks: HashMap<TabId, Vec<bool>> = HashMap::new();
        masks.insert(t, m.clone());
        let mut out = vec![Filter {
            table: t,
            cols: Arc::new(self.table_cols(t)),
            kind: FKind::Rows(Arc::new(m)),
        }];
        for (d, how) in self.expanded(t) {
            let Some((r, from)) = how else {
                continue;
            };
            let Some(sub) = masks.get(&from).cloned() else {
                continue;
            };
            let n = self.rows(d)?;
            let dm = self.propagate(r, true, &sub, n)?;
            masks.insert(d, dm.clone());
            out.push(Filter {
                table: d,
                cols: Arc::new(self.table_cols(d)),
                kind: FKind::Rows(Arc::new(dm)),
            });
        }
        Ok(out)
    }

    /// Removes the filters on `cols` (a multi-column filter keeps its other columns).
    fn remove_cols(&mut self, filters: &mut Vec<Filter>, cols: &HashSet<ColId>) -> Result<(), Fail> {
        let mut out = Vec::with_capacity(filters.len());
        for f in filters.drain(..) {
            let hit = f.cols.iter().filter(|c| cols.contains(c)).count();
            if hit == 0 {
                out.push(f);
            } else if hit < f.cols.len()
                && let Some(p) = self.project(&f, cols)?
            {
                out.push(p);
            }
        }
        *filters = out;
        Ok(())
    }

    fn project(&mut self, f: &Filter, removed: &HashSet<ColId>) -> Result<Option<Filter>, Fail> {
        let FKind::Rows(m) = &f.kind else {
            return Ok(None);
        };
        let mut keep = Vec::new();
        let mut datas = Vec::new();
        for c in f.cols.iter().filter(|c| !removed.contains(c)) {
            if let Ok(d) = self.col(*c) {
                keep.push(*c);
                datas.push(d);
            }
        }
        if keep.is_empty() {
            return Ok(None);
        }
        let n = self.rows(f.table)?;
        self.tick(n.saturating_mul(keep.len()).saturating_mul(2))?;
        let key = |row: usize| -> Vec<u32> { datas.iter().map(|d| d.codes.get(row).unwrap_or(0)).collect() };
        let set: HashSet<Vec<u32>> = (0..n)
            .filter(|r| m.get(*r).copied().unwrap_or(false))
            .map(&key)
            .collect();
        let mut nm: Vec<bool> = (0..n).map(|r| set.contains(&key(r))).collect();
        nm.push(m.get(n).copied().unwrap_or(false));
        Ok(Some(Filter {
            table: f.table,
            cols: Arc::new(keep),
            kind: FKind::Rows(Arc::new(nm)),
        }))
    }

    /// Filters equivalent to the row contexts (context transition). An inner row context
    /// overrides the filters an outer one puts on the same columns, as CALCULATE filters do
    /// (`RANKX(ALL(T[c]), [M])` in the row context of `VALUES(T[c])`).
    fn transition(&mut self, rc: &[RowB]) -> Result<Vec<Filter>, Fail> {
        let mut out: Vec<Filter> = Vec::new();
        for b in rc {
            let mut one = self.transition_one(b)?;
            let cols: HashSet<ColId> = one.iter().flat_map(|f| f.cols.iter().copied()).collect();
            if !out.is_empty() && !cols.is_empty() {
                self.remove_cols(&mut out, &cols)?;
            }
            out.append(&mut one);
        }
        Ok(out)
    }

    /// The filters of one row context.
    fn transition_one(&mut self, b: &RowB) -> Result<Vec<Filter>, Fail> {
        let mut out = Vec::new();
        {
            match b {
                RowB::Row { table, row } => {
                    let n = self.rows(*table)?;
                    let row = *row as usize;
                    let mut m = vec![false; n.saturating_add(1)];
                    if row >= n {
                        // the virtual blank row
                        if let Some(s) = m.get_mut(n) {
                            *s = true;
                        }
                    } else if let Some(u) = self.unique_col(*table)? {
                        let d = self.col(u)?;
                        let code = d.codes.get(row);
                        self.tick(n)?;
                        for (r, c) in d.codes.iter().enumerate() {
                            if Some(*c) == code
                                && let Some(s) = m.get_mut(r)
                            {
                                *s = true;
                            }
                        }
                    } else {
                        let mut datas = Vec::new();
                        for c in self.table_cols(*table) {
                            if let Ok(d) = self.col(c) {
                                datas.push(d);
                            }
                        }
                        self.tick(n.saturating_mul(datas.len().max(1)))?;
                        let key = |r: usize| -> Vec<u32> {
                            datas.iter().map(|d| d.codes.get(r).unwrap_or(0)).collect()
                        };
                        let target = key(row);
                        for (r, s) in m.iter_mut().enumerate().take(n) {
                            *s = key(r) == target;
                        }
                    }
                    out.extend(self.rows_filters(*table, m)?);
                }
                RowB::Tuple { cols, vals } => {
                    for (c, v) in cols.iter().zip(vals) {
                        let Some(c) = &c.lin else {
                            continue;
                        };
                        let code = self.code_of(*c, v)?;
                        let codes: Vec<u32> = code.into_iter().collect();
                        out.push(self.codes_filter(*c, &codes)?);
                    }
                }
            }
        }
        Ok(out)
    }

    /// `fc` with the row contexts turned into filters (context transition).
    fn transitioned(&mut self, fc: &FCtx, rc: &[RowB]) -> Result<FCtx, Fail> {
        let filters = self.transition(rc)?;
        let mut out = fc.child();
        let cols: HashSet<ColId> = filters.iter().flat_map(|f| f.cols.iter().copied()).collect();
        self.reset_groups(&mut out, &mut cols.iter().copied());
        self.remove_cols(&mut out.filters, &cols)?;
        out.filters.extend(filters);
        Ok(out)
    }

    /// Whether a filter on column `c` also removes the filters on the rest of its table.
    fn is_date_key(&self, c: ColId) -> bool {
        let Some(info) = self.cols.get(c) else {
            return false;
        };
        if !info.date {
            return false;
        }
        if self.tabs.get(info.table).is_some_and(|t| t.date_table) {
            return true;
        }
        self.rels
            .iter()
            .any(|r| (r.to == c && r.to_one) || (r.from == c && r.from_one))
    }

    // --- measures and column references ------------------------------------------------------

    fn measure_def(&mut self, table: Option<&str>, name: &str) -> Option<Rc<MeasureDef>> {
        let lower = name.to_lowercase();
        let key = format!("{}\u{0}{lower}", table.map(str::to_lowercase).unwrap_or_default());
        if let Some(m) = self.measures.get(&key) {
            return m.clone();
        }
        let in_table = |t: &bicraft_model::Table| {
            t.measures
                .iter()
                .find(|m| m.name.to_lowercase() == lower)
                .map(|m| {
                    (
                        t.name.clone(),
                        m.name.clone(),
                        m.expression.clone(),
                        (m.format_string.clone(), calc::measure_format_expr(m)),
                    )
                })
        };
        let found = table
            .and_then(|t| self.table_id(t))
            .and_then(|ti| self.model.tables.get(ti))
            .and_then(in_table)
            .or_else(|| self.model.tables.iter().find_map(in_table));
        let def = found
            .filter(|(home, _, _, _)| table.is_none_or(|t| t.to_lowercase() == home.to_lowercase()))
            .map(|(home, name, text, (format, format_expr))| {
                let (expr, diags) =
                    bicraft_dax::parse_expression(&crate::udf::expand_calls(self.model, &text));
                let errors = diags.iter().any(|d| d.severity == bicraft_dax::Severity::Error);
                // the session's own model has it, unchanged: not a query's `DEFINE MEASURE`
                let in_model = self
                    .ctx
                    .s
                    .model
                    .as_ref()
                    .and_then(|m| m.table(&home))
                    .is_some_and(|t| {
                        t.measures
                            .iter()
                            .any(|m| m.name.eq_ignore_ascii_case(&name) && m.expression == text)
                    });
                Rc::new(MeasureDef {
                    name,
                    in_model,
                    expr: expr.filter(|_| !errors),
                    format,
                    format_expr,
                })
            });
        self.measures.insert(key, def.clone());
        def
    }

    fn eval_measure(&mut self, def: &Rc<MeasureDef>, s: &Scope) -> Result<V, Fail> {
        if s.mdepth >= MAX_MEASURE_DEPTH {
            return unsupported("measure references nest too deeply");
        }
        let transitioned;
        let fc = if s.rc.is_empty() {
            s.fc
        } else {
            transitioned = self.transitioned(s.fc, s.rc)?;
            &transitioned
        };
        if let Some(v) = self.measure_with_items(def, fc, s)? {
            return Ok(v);
        }
        self.measure_body(def, fc, s)
    }

    /// A measure's own expression in `fc` (row contexts already transitioned).
    fn measure_body(&mut self, def: &MeasureDef, fc: &FCtx, s: &Scope) -> Result<V, Fail> {
        if s.mdepth >= MAX_MEASURE_DEPTH {
            return unsupported("measure references nest too deeply");
        }
        let Some(expr) = &def.expr else {
            return unsupported(format!("measure [{}] does not parse", def.name));
        };
        let env = Env::new();
        let inner = Scope {
            fc,
            rc: &[],
            env: &env,
            depth: s.depth.saturating_add(1),
            mdepth: s.mdepth.saturating_add(1),
        };
        if def.in_model {
            self.in_model_measure = self.in_model_measure.saturating_add(1);
        }
        let v = self.scalar(expr, &inner);
        if def.in_model {
            self.in_model_measure = self.in_model_measure.saturating_sub(1);
        }
        v.map_err(|e| match e {
            Fail::Unsupported(r) if !r.starts_with("measure [") => {
                Fail::Unsupported(format!("measure [{}]: {r}", def.name))
            }
            other => other,
        })
    }

    /// The column of a variation reference `T[c].[v]` (auto date/time): column `v` of the table
    /// `T[c]`'s variation points to. `[c].[v]` takes the first column named `c`.
    fn variation_col(&self, table: Option<&str>, name: &str, v: &str) -> Result<ColId, Fail> {
        let base = match table {
            Some(t) => self.model.table(t).map(|t| t.name.as_str()),
            None => self
                .model
                .tables
                .iter()
                .find(|t| t.column(name).is_some())
                .map(|t| t.name.as_str()),
        };
        let found = base
            .and_then(|t| self.model.variation_column(t, name, v))
            .and_then(|(vt, vc)| self.col_id(&vt.name, &vc.name));
        match found {
            Some(c) => Ok(c),
            None => unsupported(format!(
                "the variation [{name}].[{v}] does not resolve to a column of the model"
            )),
        }
    }

    /// The column a reference names: `T[c]`, or `[c]` resolved against the row contexts; for
    /// a variation `T[c].[v]`, the variation table's column.
    fn resolve_col(&mut self, e: &Expr, table: Option<&str>, name: &str, rc: &[RowB]) -> Result<ColId, Fail> {
        let addr = e as *const Expr as usize;
        if let Some(c) = self.refs.get(&addr) {
            return Ok(*c);
        }
        if let Expr::Column {
            variation: Some(v), ..
        } = e
        {
            let c = self.variation_col(table, name, &v.name)?;
            self.refs.insert(addr, c);
            return Ok(c);
        }
        match table {
            Some(t) => match self.col_id(t, name) {
                Some(c) => {
                    self.refs.insert(addr, c);
                    Ok(c)
                }
                None => unsupported(format!("unknown column '{t}'[{name}]")),
            },
            None => {
                let lower = name.to_lowercase();
                for b in rc.iter().rev() {
                    match b {
                        RowB::Row { table, .. } => {
                            if let Some(c) = self
                                .table_cols(*table)
                                .into_iter()
                                .find(|c| self.cols.get(*c).is_some_and(|i| i.name.to_lowercase() == lower))
                            {
                                return Ok(c);
                            }
                        }
                        RowB::Tuple { cols, .. } => {
                            if let Some(c) = cols
                                .iter()
                                .find(|c| c.lin.is_some() && self.tcol_name(c).to_lowercase() == lower)
                                .and_then(|c| c.lin)
                            {
                                return Ok(c);
                            }
                        }
                    }
                }
                unsupported(format!(
                    "[{name}] is neither a measure nor a column in row context"
                ))
            }
        }
    }

    /// A column argument (`T[c]`) of a function.
    fn col_arg(&mut self, e: &Expr, rc: &[RowB], f: &str) -> Result<ColId, Fail> {
        match e {
            Expr::Column { table, name, .. } => self.resolve_col(e, table.as_deref(), name, rc),
            _ => unsupported(format!("{f}() of something other than a column")),
        }
    }

    /// The value of a column in the row contexts.
    fn row_value(&mut self, c: ColId, rc: &[RowB]) -> Result<V, Fail> {
        let t = self.col_table(c)?;
        for b in rc.iter().rev() {
            match b {
                RowB::Row { table, row } if *table == t => {
                    let d = self.col(c)?;
                    let code = d.codes.get(*row as usize).unwrap_or(0);
                    return Ok(d.values.get(code as usize).map_or(V::Blank, V::of_cell));
                }
                RowB::Tuple { cols, vals } => {
                    if let Some(i) = cols.iter().position(|x| x.lin == Some(c)) {
                        return Ok(vals.get(i).cloned().unwrap_or(V::Blank));
                    }
                }
                RowB::Row { .. } => {}
            }
        }
        dax_error(format!(
            "a single value for column {} cannot be determined (no row context)",
            self.col_name(c)
        ))
    }

    // --- scalar expressions -------------------------------------------------------------------

    fn scalar(&mut self, e: &Expr, s: &Scope) -> Result<V, Fail> {
        let s = s.deeper()?;
        match e {
            Expr::Number { text, .. } => match text.parse::<f64>() {
                Ok(x) => Ok(V::Num(x)),
                Err(_) => unsupported(format!("number `{text}`")),
            },
            Expr::Str { value, .. } => Ok(V::Text(value.clone())),
            Expr::Bool { value, .. } => Ok(V::Bool(*value)),
            Expr::Name { name, .. } => {
                let lower = name.to_lowercase();
                match s.env.iter().rev().find(|(n, _)| *n == lower) {
                    Some((_, Bind::S(v))) => Ok(v.clone()),
                    Some((_, Bind::T(t))) => {
                        let t = t.clone();
                        self.tab_scalar(&t)
                    }
                    Some((_, Bind::L(l))) => {
                        let l = Rc::clone(l);
                        let t = self.force_lazy(&l)?;
                        self.tab_scalar(&t)
                    }
                    None => unsupported(format!("`{name}` is not a value the preview evaluates")),
                }
            }
            Expr::Column {
                table,
                name,
                variation: Some(_),
                ..
            } => {
                // In a row context of the variation's own table, its value; in a row context of
                // the base table, the related value (the engine follows the variation's
                // relationship, like RELATED).
                let c = self.resolve_col(e, table.as_deref(), name, s.rc)?;
                let t = self.col_table(c)?;
                let own_row = s.rc.iter().any(|b| match b {
                    RowB::Row { table, .. } => *table == t,
                    RowB::Tuple { cols, .. } => cols.iter().any(|x| x.lin == Some(c)),
                });
                if own_row {
                    self.row_value(c, s.rc)
                } else {
                    self.related_value(c, s.rc)
                }
            }
            Expr::Column { table, name, .. } => {
                if table.is_none()
                    && let Some(v) = self.ext_value(name, s.rc)
                {
                    return Ok(v);
                }
                if let Some(c) = self.refs.get(&(e as *const Expr as usize)).copied() {
                    return self.row_value(c, s.rc);
                }
                if let Some(def) = self.measure_def(table.as_deref(), name) {
                    return self.eval_measure(&def, &s);
                }
                // `[_<measure> FormatString]`: the measure's format string
                if let Some(def) = bicraft_dax::format_string_ref(name)
                    .and_then(|m| self.measure_def(table.as_deref(), m))
                    .filter(|_| self.col_id(table.as_deref().unwrap_or_default(), name).is_none())
                {
                    let transitioned;
                    let fc = if s.rc.is_empty() {
                        s.fc
                    } else {
                        transitioned = self.transitioned(s.fc, s.rc)?;
                        &transitioned
                    };
                    return self.format_of(&def, fc, &s);
                }
                let c = self.resolve_col(e, table.as_deref(), name, s.rc)?;
                self.row_value(c, s.rc)
            }
            Expr::Unary { op, operand, .. } => {
                let v = self.scalar(operand, &s)?;
                match op.to_ascii_uppercase().as_str() {
                    "-" => Ok(match num(&v)? {
                        None => V::Blank,
                        Some(x) => match v {
                            V::Cur(n) => match n.checked_neg() {
                                Some(m) => V::Cur(m),
                                None => return overflow(),
                            },
                            _ => V::Num(-x),
                        },
                    }),
                    "+" => Ok(v),
                    "NOT" => Ok(V::Bool(!truth(&v)?)),
                    other => unsupported(format!("operator `{other}`")),
                }
            }
            Expr::Binary { op, lhs, rhs, .. } => self.binary(op, lhs, rhs, &s),
            Expr::Var { bindings, body, .. } => {
                let mut env = s.env.clone();
                for b in bindings {
                    let val = {
                        let inner = Scope { env: &env, ..s };
                        if self.is_table_expr(&b.value, &env) {
                            Bind::T(self.table(&b.value, &inner)?)
                        } else {
                            Bind::S(self.scalar(&b.value, &inner)?)
                        }
                    };
                    env.push((b.name.to_lowercase(), val));
                }
                let inner = Scope { env: &env, ..s };
                self.scalar(body, &inner)
            }
            Expr::Call { name, args, .. } => self.call(e, name, args, &s),
            Expr::TableCtor { .. } => {
                let t = self.table(e, &s)?;
                self.tab_scalar(&t)
            }
            Expr::Tuple { .. } => unsupported("a row value where a scalar is expected"),
        }
    }

    /// A one-cell table as a scalar.
    fn tab_scalar(&mut self, t: &Tab) -> Result<V, Fail> {
        match t {
            Tab::Tuples { cols, rows } if cols.len() == 1 => match rows.len() {
                0 => Ok(V::Blank),
                1 => Ok(rows.first().and_then(|r| r.first()).cloned().unwrap_or(V::Blank)),
                _ => dax_error("a table of multiple values was supplied where a single value was expected"),
            },
            Tab::Rows { rows, .. } if rows.is_empty() => Ok(V::Blank),
            Tab::Tuples { rows, .. } if rows.is_empty() => Ok(V::Blank),
            _ => dax_error("a table of multiple values was supplied where a single value was expected"),
        }
    }

    fn binary(&mut self, op: &str, lhs: &Expr, rhs: &Expr, s: &Scope) -> Result<V, Fail> {
        let upper = op.to_ascii_uppercase();
        match upper.as_str() {
            "&&" => {
                let a = truth(&self.scalar(lhs, s)?)?;
                Ok(V::Bool(a && truth(&self.scalar(rhs, s)?)?))
            }
            "||" => {
                let a = truth(&self.scalar(lhs, s)?)?;
                Ok(V::Bool(a || truth(&self.scalar(rhs, s)?)?))
            }
            "IN" => {
                let left: Vec<V> = match lhs {
                    Expr::Tuple { items, .. } => items
                        .iter()
                        .map(|i| self.scalar(i, s))
                        .collect::<Result<_, _>>()?,
                    other => vec![self.scalar(other, s)?],
                };
                let t = self.table(rhs, s)?;
                let rows = self.tab_values(&t)?;
                for r in rows.iter() {
                    if r.len() != left.len() {
                        return dax_error("IN compares rows of different widths");
                    }
                    let mut all = true;
                    for (a, b) in left.iter().zip(r) {
                        if !strictly_equal(a, b)? {
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
            "=" | "==" | "<>" | "<" | "<=" | ">" | ">=" => {
                let a = self.scalar(lhs, s)?;
                let b = self.scalar(rhs, s)?;
                if upper == "==" {
                    return Ok(V::Bool(strictly_equal(&a, &b)?));
                }
                let o = compare_vals(&a, &b)?;
                Ok(V::Bool(match upper.as_str() {
                    "=" => o == Ordering::Equal,
                    "<>" => o != Ordering::Equal,
                    "<" => o == Ordering::Less,
                    "<=" => o != Ordering::Greater,
                    ">" => o == Ordering::Greater,
                    _ => o != Ordering::Less,
                }))
            }
            _ => {
                let a = self.scalar(lhs, s)?;
                let b = self.scalar(rhs, s)?;
                arith(&upper, &a, &b)
            }
        }
    }

    /// Rows of a table as values (for `IN`, `TREATAS`), in the order of [`Engine::tab_cols`].
    fn tab_values(&mut self, t: &Tab) -> Result<Arc<Vec<Vec<V>>>, Fail> {
        match t {
            Tab::Tuples { rows, .. } => Ok(Arc::clone(rows)),
            Tab::Rows { table, rows, ext } => {
                let cols = self.value_cols(*table);
                let datas = cols.iter().map(|c| self.col(*c)).collect::<Result<Vec<_>, _>>()?;
                self.tick(rows.len().saturating_mul(cols.len().max(1)))?;
                let width = cols
                    .len()
                    .saturating_add(ext.as_ref().map_or(0, |e| e.cols.len()));
                let n = rows.len();
                let name = match self.mem.on() {
                    true => self.tabs.get(*table).map(|t| t.name.clone()).unwrap_or_default(),
                    false => String::new(),
                };
                let what = move || format!("the {n} rows of '{name}' as values");
                let charge = self.charge_bytes(&what, n.saturating_mul(mem::row_bytes(width)))?;
                let values: Arc<Vec<Vec<V>>> = Arc::new(
                    rows.iter()
                        .enumerate()
                        .map(|(i, r)| {
                            let mut vals: Vec<V> = datas
                                .iter()
                                .map(|d| {
                                    let code = d.codes.get(*r as usize).unwrap_or(0);
                                    d.values.get(code as usize).map_or(V::Blank, V::of_cell)
                                })
                                .collect();
                            if let Some(e) = ext {
                                vals.extend(e.vals.get(i).cloned().unwrap_or_default());
                            }
                            vals
                        })
                        .collect(),
                );
                drop(charge);
                self.hold_rows(&values, width, &what)?;
                Ok(values)
            }
        }
    }

    fn is_table_expr(&self, e: &Expr, env: &Env) -> bool {
        match e {
            Expr::Name { name, .. } => {
                let lower = name.to_lowercase();
                match env.iter().rev().find(|(n, _)| *n == lower) {
                    Some((_, b)) => matches!(b, Bind::T(_) | Bind::L(_)),
                    None => self.table_id(name).is_some(),
                }
            }
            Expr::Call { name, .. } => is_table_function(name),
            Expr::TableCtor { .. } => true,
            Expr::Var { body, .. } => self.is_table_expr(body, env),
            _ => false,
        }
    }

    fn call(&mut self, e: &Expr, name: &str, args: &[Option<Expr>], s: &Scope) -> Result<V, Fail> {
        let up = name.to_ascii_uppercase();
        let n = args.len();
        if let Some(agg) = agg_function(&up, n) {
            let c = self.col_arg(need(args, 0, &up)?, s.rc, &up)?;
            return self.aggregate(agg, c, s.fc);
        }
        match up.as_str() {
            "COUNTROWS" => {
                if self.store.is_some()
                    && let Some(Expr::Name { name, .. }) = arg(args, 0)
                    && !s.env.iter().any(|(n, _)| n.eq_ignore_ascii_case(name))
                    && let Some(t) = self.table_id(name)
                    && let Some(c) = self.value_cols(t).first().copied()
                    && self.col(c).is_ok()
                {
                    return self.aggregate(Agg::CountRows, c, s.fc);
                }
                if let Some(v) = self.count_grouping(need(args, 0, &up)?, s)? {
                    return Ok(v);
                }
                let t = self.table(need(args, 0, &up)?, s)?;
                Ok(match t.len() {
                    0 => V::Blank,
                    k => V::Num(k as f64),
                })
            }
            "SUMX" | "AVERAGEX" | "MINX" | "MAXX" | "COUNTX" | "COUNTAX" => {
                if let Some(v) = self.iterate_table(&up, need(args, 0, &up)?, need(args, 1, &up)?, s)? {
                    return Ok(v);
                }
                let t = self.table(need(args, 0, &up)?, s)?;
                let body = need(args, 1, &up)?;
                if let Some(v) = self.iterate_by_codes(&up, &t, body, s)? {
                    return Ok(v);
                }
                let vals = self.each(&t, body, s)?;
                iterate_agg(&up, vals)
            }
            "RANKX" => self.rankx(args, s),
            "CONCATENATEX" => self.concatenatex(args, s),
            "CALCULATE" => {
                let fc = self.calc_ctx(args, s)?;
                // The row contexts became filters (context transition): the expression no longer sees
                // them, so a measure in it does not transition again (Desktop 2.158).
                let inner = Scope {
                    fc: &fc,
                    rc: &[],
                    ..*s
                };
                self.scalar(need(args, 0, &up)?, &inner)
            }
            "TOTALYTD" | "TOTALQTD" | "TOTALMTD" | "TOTALWTD" => self.total_to_date(e, &up, args, s),
            "DIVIDE" => {
                let va = self.scalar(need(args, 0, &up)?, s)?;
                let vb = self.scalar(need(args, 1, &up)?, s)?;
                match (num(&va)?, num(&vb)?) {
                    (_, None) | (_, Some(0.0)) => match arg(args, 2) {
                        Some(alt) => self.scalar(alt, s),
                        None => Ok(V::Blank),
                    },
                    (None, Some(_)) => Ok(V::Blank),
                    (Some(a), Some(b)) => match currency_arith("/", &va, &vb)? {
                        Some(v) => Ok(v),
                        None => Ok(V::Num(a / b)),
                    },
                }
            }
            "CURRENCY" => match self.scalar(need(args, 0, &up)?, s)? {
                V::Blank => Ok(V::Blank),
                V::Cur(n) => Ok(V::Cur(n)),
                v => match num(&v)?.map(currency::from_f64) {
                    Some(Some(n)) => Ok(V::Cur(n)),
                    _ => overflow(),
                },
            },
            // outside SUMMARIZECOLUMNS (which reads it), IGNORE is its argument
            "IGNORE" => self.scalar(need(args, 0, &up)?, s),
            "IF" | "IF.EAGER" => {
                let c = truth(&self.scalar(need(args, 0, &up)?, s)?)?;
                if c {
                    self.scalar(need(args, 1, &up)?, s)
                } else {
                    match arg(args, 2) {
                        Some(x) => self.scalar(x, s),
                        None => Ok(V::Blank),
                    }
                }
            }
            "SWITCH" => {
                let v = self.scalar(need(args, 0, &up)?, s)?;
                let mut i = 1usize;
                while i.saturating_add(1) < n {
                    let w = self.scalar(need(args, i, &up)?, s)?;
                    let hit = if v.is_blank() || w.is_blank() {
                        v.is_blank() && w.is_blank()
                    } else {
                        compare_vals(&v, &w)? == Ordering::Equal
                    };
                    if hit {
                        return self.scalar(need(args, i.saturating_add(1), &up)?, s);
                    }
                    i = i.saturating_add(2);
                }
                match arg(args, i).filter(|_| i < n) {
                    Some(x) => self.scalar(x, s),
                    None => Ok(V::Blank),
                }
            }
            "SELECTEDVALUE" => {
                let c = self.col_arg(need(args, 0, &up)?, s.rc, &up)?;
                let codes = self.values_codes(s.fc, c, true)?;
                match codes.as_slice() {
                    [one] => self.value_of_code(c, *one),
                    _ => match arg(args, 1) {
                        Some(x) => self.scalar(x, s),
                        None => Ok(V::Blank),
                    },
                }
            }
            "HASONEVALUE" => {
                let c = self.col_arg(need(args, 0, &up)?, s.rc, &up)?;
                Ok(V::Bool(self.values_codes(s.fc, c, true)?.len() == 1))
            }
            "ISFILTERED" => {
                let target = need(args, 0, &up)?;
                let cols: HashSet<ColId> = match target {
                    Expr::Name { name, .. } => match self.table_id(name) {
                        Some(t) => self.table_cols(t).into_iter().collect(),
                        None => return unsupported("ISFILTERED() of something other than a table or column"),
                    },
                    other => HashSet::from([self.col_arg(other, s.rc, &up)?]),
                };
                Ok(V::Bool(
                    s.fc.filters
                        .iter()
                        .any(|f| f.cols.iter().any(|c| cols.contains(c))),
                ))
            }
            "ISINSCOPE" => {
                let c = self.col_arg(need(args, 0, &up)?, s.rc, &up)?;
                Ok(V::Bool(self.group_cols.contains(&c)))
            }
            "ISSUBTOTAL" => {
                let c = self.col_arg(need(args, 0, &up)?, s.rc, &up)?;
                Ok(V::Bool(self.rolled.contains(&c)))
            }
            "ISBLANK" => Ok(V::Bool(self.scalar(need(args, 0, &up)?, s)?.is_blank())),
            "ISEMPTY" => Ok(V::Bool(self.table(need(args, 0, &up)?, s)?.len() == 0)),
            "COALESCE" => {
                for a in args.iter().flatten() {
                    let v = self.scalar(a, s)?;
                    if !v.is_blank() {
                        return Ok(v);
                    }
                }
                Ok(V::Blank)
            }
            "BLANK" => Ok(V::Blank),
            "TRUE" => Ok(V::Bool(true)),
            "FALSE" => Ok(V::Bool(false)),
            "NOT" => Ok(V::Bool(!truth(&self.scalar(need(args, 0, &up)?, s)?)?)),
            "AND" | "OR" => {
                let a = truth(&self.scalar(need(args, 0, &up)?, s)?)?;
                let b = truth(&self.scalar(need(args, 1, &up)?, s)?)?;
                Ok(V::Bool(if up == "AND" { a && b } else { a || b }))
            }
            "MIN" | "MAX" => {
                let a = self.scalar(need(args, 0, &up)?, s)?;
                let b = self.scalar(need(args, 1, &up)?, s)?;
                if a.is_blank() && b.is_blank() {
                    return Ok(V::Blank);
                }
                if let (V::Cur(x), V::Cur(y)) = (&a, &b) {
                    return Ok(V::Cur(if up == "MIN" { *x.min(y) } else { *x.max(y) }));
                }
                let (x, y) = (num(&a)?.unwrap_or(0.0), num(&b)?.unwrap_or(0.0));
                Ok(V::Num(if up == "MIN" { x.min(y) } else { x.max(y) }))
            }
            "ABS" | "INT" | "SIGN" => match self.scalar(need(args, 0, &up)?, s)? {
                // ABS and INT of Currency stay Currency.
                V::Cur(n) if up != "SIGN" => {
                    let r = if up == "ABS" {
                        n.checked_abs()
                    } else {
                        n.checked_sub(n.rem_euclid(10_000))
                    };
                    r.map_or_else(overflow, |m| Ok(V::Cur(m)))
                }
                v => match num(&v)? {
                    None => Ok(V::Blank),
                    Some(x) => Ok(V::Num(match up.as_str() {
                        "ABS" => x.abs(),
                        "INT" => x.floor(),
                        _ => x.signum() * f64::from(u8::from(x != 0.0)),
                    })),
                },
            },
            "ROUND" => {
                let v = self.scalar(need(args, 0, &up)?, s)?;
                let x = num(&v)?;
                let d = match arg(args, 1) {
                    Some(d) => num(&self.scalar(d, s)?)?.unwrap_or(0.0),
                    None => 0.0,
                };
                let Some(x) = x else {
                    return Ok(V::Num(0.0));
                };
                let d = d.clamp(-15.0, 15.0).trunc() as i32;
                if let V::Cur(n) = v {
                    // Currency stays Currency: round the scaled integer.
                    if d >= 4 {
                        return Ok(V::Cur(n));
                    }
                    // d in -15..=3: one unit of the result is 10^(4 - d) units of Currency
                    let unit = 10i128.pow(4u32.saturating_add_signed(-d));
                    return match currency::div_round(i128::from(n), unit)
                        .and_then(|q| i128::from(q).checked_mul(unit))
                        .and_then(|r| i64::try_from(r).ok())
                    {
                        Some(r) => Ok(V::Cur(r)),
                        None => overflow(),
                    };
                }
                let p = 10f64.powi(d);
                Ok(V::Num((x * p).round() / p))
            }
            "DATE" => {
                // Arguments are rounded half away from zero; years 0–49 are 2000–2049, 50–99 are
                // 1950–1999 and negative years count back from 1900; months and days out of range
                // carry into the next or previous ones, but a day of 0 is an error, and so is a
                // result after 9999 (Desktop 2.159).
                let mut part = [0i64; 3];
                for (i, slot) in part.iter_mut().enumerate() {
                    let x = num(&self.scalar(need(args, i, &up)?, s)?)?.unwrap_or(0.0);
                    if !x.is_finite() || x.abs() > 1.0e6 {
                        return dax_error("DATE() argument out of range");
                    }
                    *slot = x.round() as i64;
                }
                let [y, m, d] = part;
                let y = match y {
                    0..=49 => y + 2000,
                    50..=99 => y + 1900,
                    _ if y < 0 => y + 1900,
                    _ => y,
                };
                if d == 0 || !(1..=9999).contains(&y) {
                    return dax_error("an argument of DATE() is out of range");
                }
                let mi = y * 12 + m - 1;
                let x = month_start(mi) + d - 1;
                if ymd(x).0 > 9999 || x < -657_434 {
                    return dax_error("an argument of DATE() is out of range");
                }
                Ok(V::Date(x as f64))
            }
            "YEAR" | "MONTH" | "DAY" | "QUARTER" => {
                let Some(x) = self.day_arg(need(args, 0, &up)?, s)? else {
                    return Ok(V::Blank);
                };
                let (y, m, d) = ymd(x);
                Ok(V::Num(match up.as_str() {
                    "YEAR" => y,
                    "MONTH" => m,
                    "DAY" => d,
                    _ => (m - 1) / 3 + 1,
                } as f64))
            }
            "EOMONTH" | "EDATE" => {
                let Some(x) = self.day_arg(need(args, 0, &up)?, s)? else {
                    return Ok(V::Blank);
                };
                let k = self.int_arg(need(args, 1, &up)?, s)?;
                Ok(V::Date(if up == "EOMONTH" {
                    month_end(month_index(x) + k)
                } else {
                    shift_clip(x, k)
                } as f64))
            }
            // The session's clock: `as_of`, else the system clock. Desktop's TODAY()/NOW() are
            // the machine's local time, UTCTODAY()/UTCNOW() UTC (Desktop 2.159); the offset
            // between them is the caller's `utc_offset_minutes` (0 by default).
            "TODAY" => Ok(V::Date(self.ctx.limits.now.floor())),
            "NOW" => Ok(V::Date(self.ctx.limits.now)),
            "UTCTODAY" => Ok(V::Date(
                (self.ctx.limits.now - self.ctx.limits.utc_offset).floor(),
            )),
            "UTCNOW" => Ok(V::Date(self.ctx.limits.now - self.ctx.limits.utc_offset)),
            // View as's "Other user" (empty without one); outside View as there is no user
            "USERPRINCIPALNAME" | "USERNAME" => match &self.ctx.st.user {
                Some(u) => Ok(V::Text(u.clone())),
                None => unsupported(format!(
                    "{up}() has no signed-in user here: use View as with Other user"
                )),
            },
            "DATEDIFF" => self.datediff(args, s),
            "SELECTEDMEASURE"
            | "SELECTEDMEASURENAME"
            | "ISSELECTEDMEASURE"
            | "SELECTEDMEASUREFORMATSTRING" => self.selected_measure(&up, args, s),
            "RELATED" => self.related(need(args, 0, &up)?, s),
            "NAMEOF" => self.nameof(args, s),
            _ if window::handles_scalar(&up) => self.window_scalar(&up, args, s),
            _ if text::handles(&up) => self.text_function(&up, args, s),
            _ if math::handles(&up) => self.math_function(&up, args, s),
            _ if more::handles(&up) => self.more_function(&up, args, s),
            _ if is_table_function(&up) => {
                let t = self.table(e, s)?;
                self.tab_scalar(&t)
            }
            _ => unsupported(format!("{up}() is not evaluated by the preview")),
        }
    }

    fn day_arg(&mut self, e: &Expr, s: &Scope) -> Result<Option<i64>, Fail> {
        match num(&self.scalar(e, s)?)? {
            None => Ok(None),
            Some(x) if x.is_finite() && x.abs() < 3_000_000.0 => Ok(Some(x.floor() as i64)),
            Some(_) => dax_error("date out of range"),
        }
    }

    /// `DATEDIFF(start, end, interval)`: interval boundaries crossed from `start` to `end`
    /// (negative when `end` is earlier); blank when either date is blank.
    fn datediff(&mut self, args: &[Option<Expr>], s: &Scope) -> Result<V, Fail> {
        let date = |eng: &mut Self, i: usize| -> Result<Option<f64>, Fail> {
            match num(&eng.scalar(need(args, i, "DATEDIFF")?, s)?)? {
                None => Ok(None),
                Some(x) if x.is_finite() && x.abs() < 3_000_000.0 => Ok(Some(x)),
                Some(_) => dax_error("date out of range"),
            }
        };
        let (Some(a), Some(b)) = (date(self, 0)?, date(self, 1)?) else {
            return Ok(V::Blank);
        };
        let Some(unit) = need(args, 2, "DATEDIFF").map(keyword)? else {
            return unsupported("DATEDIFF() with an interval that is not a keyword");
        };
        let (da, db) = (a.floor() as i64, b.floor() as i64);
        // weeks start on Sunday; day 1 (1899-12-31) is a Sunday
        let week = |d: i64| (d - 1).div_euclid(7);
        let n = match unit.as_str() {
            "SECOND" => (b * 86_400.0).round() as i64 - (a * 86_400.0).round() as i64,
            "MINUTE" => (b * 1440.0 + 1e-9).floor() as i64 - (a * 1440.0 + 1e-9).floor() as i64,
            "HOUR" => (b * 24.0 + 1e-9).floor() as i64 - (a * 24.0 + 1e-9).floor() as i64,
            "DAY" => db - da,
            "WEEK" => week(db) - week(da),
            "MONTH" => month_index(db) - month_index(da),
            "QUARTER" => quarter_index(db) - quarter_index(da),
            "YEAR" => ymd(db).0 - ymd(da).0,
            other => return unsupported(format!("DATEDIFF() interval {other}")),
        };
        Ok(V::Num(n as f64))
    }

    fn int_arg(&mut self, e: &Expr, s: &Scope) -> Result<i64, Fail> {
        let x = num(&self.scalar(e, s)?)?.unwrap_or(0.0);
        if !x.is_finite() || x.abs() > MAX_SHIFT as f64 {
            return unsupported("an interval count out of range");
        }
        Ok(x.trunc() as i64)
    }

    /// Evaluates `body` for each row of `t` (row context added).
    fn each(&mut self, t: &Tab, body: &Expr, s: &Scope) -> Result<Vec<V>, Fail> {
        if let Some(v) = self.each_direct(t, body, s)? {
            return Ok(v);
        }
        if let Some(v) = self.each_by_codes(t, body, s)? {
            return Ok(v);
        }
        self.each_with(t, s, |eng, inner| eng.scalar(body, inner))
    }

    /// Calls `f` for each row of `t`, with that row added to the row contexts.
    fn each_with<R>(
        &mut self,
        t: &Tab,
        s: &Scope,
        mut f: impl FnMut(&mut Self, &Scope) -> Result<R, Fail>,
    ) -> Result<Vec<R>, Fail> {
        let mut out = Vec::with_capacity(t.len());
        let mut rc: Vec<RowB> = s.rc.to_vec();
        self.tick(t.len())?;
        // The shadow filter context: the context of the iteration and the iterated rows.
        let saved = if self.shadow {
            let mut sc = s.fc.child();
            // rows without model lineage (GENERATESERIES, CALENDAR…) filter nothing
            let lineage = match t {
                Tab::Rows { .. } => true,
                Tab::Tuples { cols, .. } => cols.iter().any(|c| c.lin.is_some()),
            };
            if lineage {
                sc.filters.extend(self.tab_filters(t)?);
            }
            Some(std::mem::replace(&mut self.outer, sc))
        } else {
            None
        };
        let mut failed = None;
        for i in 0..t.len() {
            let pushed = push_row(t, i, &mut rc);
            let inner = Scope { rc: &rc, ..*s };
            let v = f(self, &inner);
            rc.truncate(rc.len().saturating_sub(pushed));
            match v {
                Ok(v) => out.push(v),
                Err(e) => {
                    failed = Some(e);
                    break;
                }
            }
        }
        if let Some(o) = saved {
            self.outer = o;
        }
        match failed {
            Some(e) => Err(e),
            None => Ok(out),
        }
    }

    /// `CONCATENATEX(table, expr [, delimiter [, orderBy [, ASC|DESC]]…])`: the text of `expr`
    /// for each row, joined by `delimiter`, rows sorted by the order keys (stable; table order
    /// otherwise); blank for an empty table.
    fn concatenatex(&mut self, args: &[Option<Expr>], s: &Scope) -> Result<V, Fail> {
        let t = self.table(need(args, 0, "CONCATENATEX")?, s)?;
        let body = need(args, 1, "CONCATENATEX")?;
        let delim = match arg(args, 2) {
            Some(d) => text_of(&self.scalar(d, s)?),
            None => String::new(),
        };
        // (key expression, descending) pairs
        let mut keys: Vec<(&Expr, bool)> = Vec::new();
        let mut i = 3usize;
        while let Some(k) = arg(args, i) {
            // an order keyword after the key, else the next argument is another key
            let desc = match arg(args, i.saturating_add(1)).and_then(keyword).as_deref() {
                Some("DESC") => Some(true),
                Some("ASC") => Some(false),
                _ => None,
            };
            i = i.saturating_add(if desc.is_some() { 2 } else { 1 });
            keys.push((k, desc.unwrap_or(false)));
        }
        let rows = self.each_with(&t, s, |eng, inner| {
            let v = eng.scalar(body, inner)?;
            let mut ks = Vec::with_capacity(keys.len());
            for (k, _) in &keys {
                ks.push(eng.scalar(k, inner)?);
            }
            Ok((v, ks))
        })?;
        if rows.is_empty() {
            return Ok(V::Blank);
        }
        let mut order: Vec<usize> = (0..rows.len()).collect();
        if !keys.is_empty() {
            let mut failed = None;
            order.sort_by(|a, b| {
                let (Some((_, ka)), Some((_, kb))) = (rows.get(*a), rows.get(*b)) else {
                    return Ordering::Equal;
                };
                for (j, (_, desc)) in keys.iter().enumerate() {
                    let (Some(x), Some(y)) = (ka.get(j), kb.get(j)) else {
                        continue;
                    };
                    let o = match compare_vals(x, y) {
                        Ok(o) => o,
                        Err(e) => {
                            failed.get_or_insert(e);
                            Ordering::Equal
                        }
                    };
                    let o = if *desc { o.reverse() } else { o };
                    if o != Ordering::Equal {
                        return o;
                    }
                }
                Ordering::Equal
            });
            if let Some(e) = failed {
                return Err(e);
            }
        }
        let mut out = String::new();
        for (n, i) in order.iter().enumerate() {
            if n > 0 {
                out.push_str(&delim);
            }
            if let Some((v, _)) = rows.get(*i) {
                out.push_str(&text_of(v));
            }
            if out.len() > text::MAX_TEXT {
                return dax_error(format!("a text value longer than {} bytes", text::MAX_TEXT));
            }
        }
        Ok(V::Text(out))
    }

    fn rankx(&mut self, args: &[Option<Expr>], s: &Scope) -> Result<V, Fail> {
        // the values ranked among, sorted, kept by the storage engine for every row of a grouping
        let ranked = self.rank_values(args, s)?;
        let vals = match &ranked {
            Some(_) => Vec::new(),
            None => {
                let t = self.table(need(args, 0, "RANKX")?, s)?;
                self.each(&t, need(args, 1, "RANKX")?, s)?
            }
        };
        let body = need(args, 1, "RANKX")?;
        let v = match arg(args, 2) {
            Some(x) => self.scalar(x, s)?,
            None => self.scalar(body, s)?,
        };
        let asc = match arg(args, 3) {
            None => false,
            Some(e) => match keyword(e).as_deref() {
                Some("ASC") => true,
                Some("DESC") => false,
                _ => truth(&self.scalar(e, s)?)?,
            },
        };
        let dense = match arg(args, 4).and_then(keyword).as_deref() {
            None | Some("SKIP") => false,
            Some("DENSE") => true,
            Some(other) => return unsupported(format!("RANKX ties `{other}`")),
        };
        let x = num(&v)?.unwrap_or(0.0);
        if let Some(r) = ranked
            && !x.is_nan()
        {
            let among = if dense { &r.1 } else { &r.0 };
            let better = if asc {
                among.partition_point(|y| *y < x)
            } else {
                among.len() - among.partition_point(|y| *y <= x)
            };
            return Ok(V::Num(better as f64 + 1.0));
        }
        let vals = if vals.is_empty() {
            let t = self.table(need(args, 0, "RANKX")?, s)?;
            self.each(&t, body, s)?
        } else {
            vals
        };
        let mut better: Vec<f64> = Vec::new();
        for w in &vals {
            let y = num(w)?.unwrap_or(0.0);
            if (asc && y < x) || (!asc && y > x) {
                better.push(y);
            }
        }
        if dense {
            better.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
            better.dedup();
        }
        Ok(V::Num(better.len() as f64 + 1.0))
    }

    fn total_to_date(&mut self, e: &Expr, up: &str, args: &[Option<Expr>], s: &Scope) -> Result<V, Fail> {
        // a calendar reference: calendar-based time intelligence (`preview_eval_calendar.rs`)
        if let Some(cal) = arg(args, 1).and_then(|a| self.calendar_of(a)) {
            return self.calendar_total(up, &cal, args, s);
        }
        self.refuse_nested_calendar(up, arg(args, 1))?;
        if up == "TOTALWTD" {
            return dax_error(
                "TOTALWTD() needs a calendar: week functions only work with calendar-based time intelligence",
            );
        }
        let body = need(args, 0, up)?;
        let dates = need(args, 1, up)?;
        let span = e.span();
        let mut filter = None;
        let mut year_end = None;
        for a in args.iter().skip(2).flatten() {
            if matches!(a, Expr::Str { .. }) && up == "TOTALYTD" {
                year_end = Some(a.clone());
            } else {
                filter = Some(a.clone());
            }
        }
        let fname = match up {
            "TOTALYTD" => "DATESYTD",
            "TOTALQTD" => "DATESQTD",
            _ => "DATESMTD",
        };
        let mut dargs = vec![Some(dates.clone())];
        if let Some(y) = year_end {
            dargs.push(Some(y));
        }
        let call = Expr::Call {
            name: fname.to_string(),
            name_span: span,
            args: dargs,
            span,
        };
        let mut cargs = vec![Some(body.clone()), Some(call)];
        if let Some(f) = filter {
            cargs.push(Some(f));
        }
        let fc = self.calc_ctx(&cargs, s)?;
        let inner = Scope { fc: &fc, ..*s };
        self.scalar(body, &inner)
    }

    fn related(&mut self, e: &Expr, s: &Scope) -> Result<V, Fail> {
        let c = self.col_arg(e, s.rc, "RELATED")?;
        self.related_value(c, s.rc)
    }

    /// The value of column `c` on the row related (many-to-one) to the innermost row context
    /// that reaches `c`'s table.
    fn related_value(&mut self, c: ColId, rc: &[RowB]) -> Result<V, Fail> {
        let target = self.col_table(c)?;
        for b in rc.iter().rev() {
            let RowB::Row { table, row } = b else {
                continue;
            };
            let path = self.expanded(*table);
            let Some(pos) = path.iter().position(|(t, _)| *t == target) else {
                continue;
            };
            // walk back from the target to the row's table
            let mut hops = Vec::new();
            let mut at = pos;
            while let Some((_, Some((r, from)))) = path.get(at) {
                hops.push(*r);
                match path.iter().position(|(t, _)| t == from) {
                    Some(p) => at = p,
                    None => break,
                }
            }
            hops.reverse();
            let mut cur = Some(*row as usize);
            for r in hops {
                let Some(rel) = self.rels.get(r) else {
                    return internal();
                };
                let (fc, tc) = (rel.from, rel.to);
                let fd = self.col(fc)?;
                let map = self.rel_map(r, true)?;
                let td = self.col(tc)?;
                // the first row holding the key's code, from the column's table of first rows
                // (built once per column: one pass instead of a search of the rows per call)
                match td.rows_of_codes() {
                    Some(rows_of) => {
                        cur = cur.and_then(|rw| {
                            let code = fd.codes.get(rw)?;
                            let tcode = map.get(code as usize).copied().filter(|m| *m != NONE)?;
                            let r = rows_of.get(tcode as usize).copied().filter(|r| *r != u32::MAX)?;
                            Some(r as usize)
                        });
                        self.tick(1)?;
                    }
                    None => {
                        cur = cur.and_then(|rw| {
                            let code = fd.codes.get(rw)?;
                            let tcode = map.get(code as usize).copied().filter(|m| *m != NONE)?;
                            td.codes.iter().position(|x| *x == tcode)
                        });
                        self.tick(td.codes.len())?;
                    }
                }
            }
            return match cur {
                Some(rw) => {
                    let d = self.col(c)?;
                    let code = d.codes.get(rw).unwrap_or(0);
                    Ok(d.values.get(code as usize).map_or(V::Blank, V::of_cell))
                }
                None => Ok(V::Blank),
            };
        }
        dax_error(format!(
            "RELATED({}) needs a row context of a related table",
            self.col_name(c)
        ))
    }

    /// An aggregation of a column over the rows visible in `fc`.
    fn aggregate(&mut self, agg: Agg, c: ColId, fc: &FCtx) -> Result<V, Fail> {
        if self.store.is_some()
            && let Some(v) = self.aggregate_storage(agg, c, fc)?
        {
            return Ok(v);
        }
        let t = self.col_table(c)?;
        let d = self.col(c)?;
        let mask = self.mask(fc, t)?;
        self.tick(d.codes.len())?;
        let mut counts = vec![0u64; d.values.len()];
        for (row, code) in d.codes.iter().enumerate() {
            if visible(mask.as_ref(), row)
                && let Some(n) = counts.get_mut(*code as usize)
            {
                *n = n.saturating_add(1);
            }
        }
        self.finish(agg, c, &d, &counts)
    }

    /// An aggregation of column `c` (data `d`) from the visible rows per code (`counts[code]`).
    fn finish(&mut self, agg: Agg, c: ColId, d: &ColData, counts: &[u64]) -> Result<V, Fail> {
        let blank_or = |n: u64| if n == 0 { V::Blank } else { V::Num(n as f64) };
        Ok(match agg {
            Agg::CountRows => blank_or(counts.iter().sum()),
            Agg::Count => blank_or(counts.iter().skip(1).sum()),
            Agg::CountBlank => blank_or(counts.first().copied().unwrap_or(0)),
            Agg::Distinct => blank_or(counts.iter().filter(|n| **n > 0).count() as u64),
            Agg::DistinctNoBlank => blank_or(counts.iter().skip(1).filter(|n| **n > 0).count() as u64),
            Agg::Min | Agg::Max => {
                let ranks = self.ranks(c)?;
                let mut best: Option<(u32, usize)> = None;
                for (code, n) in counts.iter().enumerate().skip(1) {
                    if *n == 0 {
                        continue;
                    }
                    let r = ranks.get(code).copied().unwrap_or(0);
                    let better = best.is_none_or(|(br, _)| if agg == Agg::Min { r < br } else { r > br });
                    if better {
                        best = Some((r, code));
                    }
                }
                best.and_then(|(_, code)| d.values.get(code))
                    .map_or(V::Blank, V::of_cell)
            }
            Agg::Sum | Agg::Avg | Agg::StdDev | Agg::StdDevP | Agg::Median if d.kinds().0 => {
                currency_aggregate(agg, &d.values, counts)?
            }
            Agg::Sum | Agg::Avg | Agg::StdDev | Agg::Var | Agg::StdDevP | Agg::VarP | Agg::Median => {
                let nums = self.nums(c)?;
                let mut items: Vec<(f64, u64)> = Vec::new();
                for (code, n) in counts.iter().enumerate() {
                    if *n > 0
                        && let Some(x) = nums.get(code).copied().flatten()
                    {
                        items.push((x, *n));
                    }
                }
                let count: u64 = items.iter().map(|(_, n)| *n).sum();
                if count == 0 {
                    return Ok(V::Blank);
                }
                let total: f64 = items.iter().map(|(x, n)| x * *n as f64).sum();
                let nf = count as f64;
                match agg {
                    Agg::Sum => V::Num(total),
                    Agg::Avg => V::Num(total / nf),
                    Agg::Median => {
                        items.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));
                        let nth = |k: u64| {
                            let mut seen = 0u64;
                            for (x, n) in &items {
                                seen = seen.saturating_add(*n);
                                if seen > k {
                                    return *x;
                                }
                            }
                            items.last().map_or(0.0, |p| p.0)
                        };
                        if count % 2 == 1 {
                            V::Num(nth(count / 2))
                        } else {
                            V::Num((nth(count / 2 - 1) + nth(count / 2)) / 2.0)
                        }
                    }
                    _ => {
                        let population = matches!(agg, Agg::StdDevP | Agg::VarP);
                        if count < if population { 1 } else { 2 } {
                            return Ok(V::Blank);
                        }
                        let mean = total / nf;
                        let ss: f64 = items
                            .iter()
                            .map(|(x, n)| (x - mean) * (x - mean) * *n as f64)
                            .sum();
                        let var = ss / if population { nf } else { nf - 1.0 };
                        V::Num(if matches!(agg, Agg::StdDev | Agg::StdDevP) {
                            var.sqrt()
                        } else {
                            var
                        })
                    }
                }
            }
        })
    }

    // --- CALCULATE --------------------------------------------------------------------------

    /// The filter context of `CALCULATE(args[0], args[1..])` evaluated in `s`.
    fn calc_ctx(&mut self, args: &[Option<Expr>], s: &Scope) -> Result<FCtx, Fail> {
        let filters: Vec<&Expr> = args.iter().skip(1).flatten().collect();
        self.calc_ctx_with(&filters, Vec::new(), s)
    }

    /// The context of `CALCULATE(…, filters…, tabs…)`: the filter arguments `filters`, then
    /// tables already evaluated (`tabs`, each a filter argument, as the balance functions'
    /// dates).
    fn calc_ctx_with(&mut self, filters: &[&Expr], tabs: Vec<Tab>, s: &Scope) -> Result<FCtx, Fail> {
        self.calc_ctx_removing(filters, tabs, None, s)
    }

    /// [`Engine::calc_ctx_with`] for calendar-based time intelligence when `removed` is given: the
    /// filters on those columns are removed first (as a `REMOVEFILTERS` of them before the other
    /// arguments), and the tables' filters do not remove the rest of a date table's filters
    /// (its untagged columns keep theirs).
    fn calc_ctx_removing(
        &mut self,
        filters: &[&Expr],
        tabs: Vec<Tab>,
        removed: Option<HashSet<ColId>>,
        s: &Scope,
    ) -> Result<FCtx, Fail> {
        let mut effects = Vec::new();
        let calendar = removed.is_some();
        if let Some(r) = removed {
            effects.push(ArgEff::Remove(r));
        }
        for a in filters {
            self.filter_arg(a, s, false, &mut effects)?;
        }
        for tab in tabs {
            let filters = self.tab_filters(&tab)?;
            effects.push(if calendar {
                ArgEff::Set {
                    filters,
                    keep: false,
                    also_remove: Vec::new(),
                }
            } else {
                self.set_effect(filters, false)
            });
        }
        let mut fc = if s.rc.is_empty() {
            s.fc.child()
        } else {
            self.transitioned(s.fc, s.rc)?
        };
        for eff in &effects {
            match eff {
                ArgEff::RemoveAll => {
                    fc.filters.clear();
                    fc.applied = 0;
                }
                ArgEff::Remove(cols) => {
                    self.reset_groups(&mut fc, &mut cols.iter().copied());
                    self.remove_cols(&mut fc.filters, cols)?;
                }
                ArgEff::Rel(o, others) => {
                    fc.rels.push(*o);
                    fc.rels.extend(others.iter().copied());
                }
                ArgEff::Set { .. } => {}
            }
        }
        let mut removed: HashSet<ColId> = HashSet::new();
        for eff in &effects {
            if let ArgEff::Set {
                filters,
                keep: false,
                also_remove,
            } = eff
            {
                removed.extend(filters.iter().flat_map(|f| f.cols.iter().copied()));
                removed.extend(also_remove.iter().copied());
            }
        }
        if !removed.is_empty() {
            self.remove_cols(&mut fc.filters, &removed)?;
        }
        for eff in effects {
            if let ArgEff::Set { filters, .. } = eff {
                let cols: Vec<ColId> = filters.iter().flat_map(|f| f.cols.iter().copied()).collect();
                self.reset_groups(&mut fc, &mut cols.into_iter());
                fc.filters.extend(filters);
            }
        }
        Ok(fc)
    }

    fn filter_arg(&mut self, e: &Expr, s: &Scope, keep: bool, out: &mut Vec<ArgEff>) -> Result<(), Fail> {
        if let Expr::Call { name, args, .. } = e {
            let up = name.to_ascii_uppercase();
            match up.as_str() {
                "KEEPFILTERS" => return self.filter_arg(need(args, 0, &up)?, s, true, out),
                "ALL" | "REMOVEFILTERS" | "ALLSELECTED" | "ALLNOBLANKROW" | "ALLCROSSFILTERED" => {
                    if args.iter().all(Option::is_none) {
                        out.push(ArgEff::RemoveAll);
                        if up == "ALLSELECTED" {
                            // back to the visual's outer filters
                            let filters = self.outer_on(None)?;
                            out.push(ArgEff::Set {
                                filters,
                                keep: true,
                                also_remove: Vec::new(),
                            });
                        }
                        return Ok(());
                    }
                    let mut cols = HashSet::new();
                    for a in args.iter().flatten() {
                        match a {
                            Expr::Name { name, .. } => match self.table_id(name) {
                                Some(t) => cols.extend(self.expanded_cols(t)),
                                None => return unsupported(format!("{up}() of `{name}`")),
                            },
                            other => {
                                cols.insert(self.col_arg(other, s.rc, &up)?);
                            }
                        }
                    }
                    let restore = if up == "ALLSELECTED" {
                        self.outer_on(Some(&cols))?
                    } else {
                        Vec::new()
                    };
                    out.push(ArgEff::Remove(cols));
                    if !restore.is_empty() {
                        out.push(ArgEff::Set {
                            filters: restore,
                            keep: true,
                            also_remove: Vec::new(),
                        });
                    }
                    return Ok(());
                }
                "ALLEXCEPT" => {
                    let Some(Expr::Name { name: t, .. }) = arg(args, 0) else {
                        return unsupported("ALLEXCEPT() of something other than a table");
                    };
                    let Some(t) = self.table_id(t) else {
                        return unsupported(format!("ALLEXCEPT() of unknown table `{t}`"));
                    };
                    let mut cols = self.expanded_cols(t);
                    for a in args.iter().skip(1).flatten() {
                        match a {
                            Expr::Name { name, .. } => {
                                if let Some(x) = self.table_id(name) {
                                    for c in self.table_cols(x) {
                                        cols.remove(&c);
                                    }
                                }
                            }
                            other => {
                                let c = self.col_arg(other, s.rc, &up)?;
                                cols.remove(&c);
                            }
                        }
                    }
                    out.push(ArgEff::Remove(cols));
                    return Ok(());
                }
                "USERELATIONSHIP" | "CROSSFILTER" => {
                    let a = self.col_arg(need(args, 0, &up)?, s.rc, &up)?;
                    let b = self.col_arg(need(args, 1, &up)?, s.rc, &up)?;
                    let Some(r) = self
                        .rels
                        .iter()
                        .position(|r| (r.from == a && r.to == b) || (r.from == b && r.to == a))
                    else {
                        return unsupported(format!("{up}() without a relationship between its columns"));
                    };
                    if up == "USERELATIONSHIP" {
                        let (ft, tt) = self.rels.get(r).map_or((0, 0), |x| (x.from_t, x.to_t));
                        let others = self
                            .rels
                            .iter()
                            .enumerate()
                            .filter(|(i, x)| {
                                *i != r
                                    && ((x.from_t == ft && x.to_t == tt) || (x.from_t == tt && x.to_t == ft))
                            })
                            .map(|(i, _)| RelOverride {
                                rel: i,
                                active: Some(false),
                                dir: None,
                            })
                            .collect();
                        out.push(ArgEff::Rel(
                            RelOverride {
                                rel: r,
                                active: Some(true),
                                dir: None,
                            },
                            others,
                        ));
                    } else {
                        let dir = match arg(args, 2).and_then(keyword).as_deref() {
                            Some("NONE") => Dir::None,
                            Some("BOTH") => Dir::Both,
                            Some("ONEWAY") => Dir::Single,
                            other => {
                                return unsupported(format!(
                                    "CROSSFILTER direction `{}`",
                                    other.unwrap_or("?")
                                ));
                            }
                        };
                        out.push(ArgEff::Rel(
                            RelOverride {
                                rel: r,
                                active: None,
                                dir: Some(dir),
                            },
                            Vec::new(),
                        ));
                    }
                    return Ok(());
                }
                _ => {}
            }
        }
        // a calendar-based time intelligence function (`preview_eval_calendar.rs`)
        if self.calendar_filter(e, s, keep, out)? {
            return Ok(());
        }
        // a filter over one column's values: straight from its codes when it compiles (also a
        // query variable not evaluated yet, Desktop's `VAR __DS0FilterTable = FILTER(…)`, in the
        // context it is defined in)
        let lazy_codes = match e {
            Expr::Name { name, .. } => match s.env.iter().rev().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
                Some((_, Bind::L(l))) if l.cell.get().is_none() => {
                    let l = Rc::clone(l);
                    let fc = FCtx::default();
                    let scope = Scope {
                        fc: &fc,
                        rc: &[],
                        env: &l.env,
                        depth: 0,
                        mdepth: 0,
                    };
                    let saved = self.take_dynamic();
                    let r = self.filter_codes_arg(&l.expr, &scope);
                    self.restore_dynamic(saved);
                    r?
                }
                _ => None,
            },
            _ => None,
        };
        let codes = match lazy_codes {
            Some(f) => Some(f),
            None => self.filter_codes_arg(e, s)?,
        };
        let filters = match codes {
            Some(f) => vec![f],
            None => {
                let tab = if self.is_table_expr(e, s.env) {
                    self.table(e, s)?
                } else {
                    self.predicate_table(e, s)?
                };
                self.tab_filters(&tab)?
            }
        };
        out.push(self.set_effect(filters, keep));
        Ok(())
    }

    /// The effect of a filter argument that sets `filters`; a filter on the date key of a date
    /// table also removes the filters on the rest of that table.
    fn set_effect(&self, filters: Vec<Filter>, keep: bool) -> ArgEff {
        let mut also_remove = Vec::new();
        if let [f] = filters.as_slice()
            && let (FKind::Codes(_) | FKind::Code(_), Some(c)) = (&f.kind, f.cols.first())
            && self.is_date_key(*c)
        {
            also_remove = self.table_cols(f.table);
        }
        ArgEff::Set {
            filters,
            keep,
            also_remove,
        }
    }

    /// A boolean filter argument as the table `FILTER(ALL(columns), predicate)`.
    fn predicate_table(&mut self, e: &Expr, s: &Scope) -> Result<Tab, Fail> {
        let mut found: Vec<ColId> = Vec::new();
        self.predicate_cols(e, &mut found, 0)?;
        let Some(first) = found.first().copied() else {
            return unsupported("a CALCULATE filter that references no column");
        };
        let t = self.col_table(first)?;
        for c in &found {
            if self.col_table(*c)? != t {
                return unsupported("a CALCULATE filter over columns of several tables");
            }
        }
        let all = self.all_tuples(&found)?;
        self.filter_tab(&all, e, s)
    }

    fn predicate_cols(&mut self, e: &Expr, out: &mut Vec<ColId>, depth: usize) -> Result<(), Fail> {
        if depth > MAX_EXPR_DEPTH {
            return unsupported("the expression nests too deeply");
        }
        let d = depth.saturating_add(1);
        match e {
            Expr::Column {
                table,
                name,
                variation: Some(v),
                ..
            } => {
                let c = self.variation_col(table.as_deref(), name, &v.name)?;
                if !out.contains(&c) {
                    out.push(c);
                }
            }
            Expr::Column { table, name, .. } => {
                if self.measure_def(table.as_deref(), name).is_none()
                    && let Some(t) = table
                    && let Some(c) = self.col_id(t, name)
                    && !out.contains(&c)
                {
                    out.push(c);
                }
            }
            Expr::Unary { operand, .. } => self.predicate_cols(operand, out, d)?,
            Expr::Binary { lhs, rhs, .. } => {
                self.predicate_cols(lhs, out, d)?;
                self.predicate_cols(rhs, out, d)?;
            }
            Expr::Call { name, args, .. } => {
                // aggregations inside the predicate are evaluated in the outer filter context
                let up = name.to_ascii_uppercase();
                let row_level = matches!(
                    up.as_str(),
                    "IF" | "SWITCH"
                        | "NOT"
                        | "AND"
                        | "OR"
                        | "ISBLANK"
                        | "YEAR"
                        | "MONTH"
                        | "DAY"
                        | "QUARTER"
                        | "ABS"
                        | "INT"
                        | "ROUND"
                        | "COALESCE"
                        | "DATE"
                        | "EOMONTH"
                        | "EDATE"
                        | "CONTAINSSTRING"
                        | "LEFT"
                        | "RIGHT"
                        | "UPPER"
                        | "LOWER"
                        | "TRIM"
                        | "LEN"
                        | "SEARCH"
                        | "FIND"
                );
                if row_level {
                    for a in args.iter().flatten() {
                        self.predicate_cols(a, out, d)?;
                    }
                }
            }
            Expr::Tuple { items, .. } => {
                for i in items {
                    self.predicate_cols(i, out, d)?;
                }
            }
            Expr::Var { bindings, body, .. } => {
                for b in bindings {
                    self.predicate_cols(&b.value, out, d)?;
                }
                self.predicate_cols(body, out, d)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// `ALL(cols)` (columns of one table): their existing combinations, with the blank row.
    fn all_tuples(&mut self, cols: &[ColId]) -> Result<Tab, Fail> {
        let Some(first) = cols.first().copied() else {
            return unsupported("ALL() without columns");
        };
        if let Some(t) = self.all_tuples_storage(cols)? {
            return Ok(t);
        }
        let t = self.col_table(first)?;
        let blank = self.has_blank_row(t)?;
        let datas = cols.iter().map(|c| self.col(*c)).collect::<Result<Vec<_>, _>>()?;
        let n = self.rows(t)?;
        self.tick(n.saturating_mul(cols.len()))?;
        let mut seen: HashSet<Vec<u32>> = HashSet::new();
        let mut keys: Vec<Vec<u32>> = Vec::new();
        for row in 0..n {
            let key: Vec<u32> = datas.iter().map(|d| d.codes.get(row).unwrap_or(0)).collect();
            if seen.insert(key.clone()) {
                keys.push(key);
            }
        }
        let zero = vec![0u32; cols.len()];
        if blank && !seen.contains(&zero) {
            keys.push(zero);
        }
        let rows: Vec<Vec<V>> = keys
            .iter()
            .map(|k| {
                datas
                    .iter()
                    .zip(k)
                    .map(|(d, code)| d.values.get(*code as usize).map_or(V::Blank, V::of_cell))
                    .collect()
            })
            .collect();
        Ok(Tab::tuples(cols.iter().map(|c| TCol::of(*c)).collect(), rows))
    }

    /// The rows of `t` where `pred` is true.
    fn filter_tab(&mut self, t: &Tab, pred: &Expr, s: &Scope) -> Result<Tab, Fail> {
        if let Some(t) = self.filter_by_codes(t, pred, s)? {
            return Ok(t);
        }
        if let Some(t) = self.filter_values(t, pred, s)? {
            return Ok(t);
        }
        let keep = self.each(t, pred, s)?;
        let mut picked = Vec::new();
        for (i, k) in keep.iter().enumerate() {
            if truth(k)? {
                picked.push(i);
            }
        }
        Ok(subset(t, &picked))
    }

    // --- table expressions ------------------------------------------------------------------

    /// The table `e` evaluates to, counted against the query's memory cap while it lives
    /// (`preview_eval_mem.rs`).
    fn table(&mut self, e: &Expr, s: &Scope) -> Result<Tab, Fail> {
        let t = self.table_of(e, s)?;
        if self.mem.on() {
            let n = t.len();
            self.hold_tab(&t, &|| format!("the table of {} ({n} rows)", expr_label(e)))?;
        }
        Ok(t)
    }

    fn table_of(&mut self, e: &Expr, s: &Scope) -> Result<Tab, Fail> {
        let s = s.deeper()?;
        match e {
            Expr::Name { name, .. } => {
                let lower = name.to_lowercase();
                if let Some((_, b)) = s.env.iter().rev().find(|(n, _)| *n == lower) {
                    return match b {
                        Bind::T(t) => Ok(t.clone()),
                        Bind::L(l) => {
                            let l = Rc::clone(l);
                            self.force_lazy(&l)
                        }
                        Bind::S(_) => unsupported(format!("`{name}` is a scalar where a table is expected")),
                    };
                }
                match self.table_id(name) {
                    Some(t) => {
                        let rows = self.visible_rows(s.fc, t)?;
                        Ok(Tab::rows_of(t, rows))
                    }
                    None => unsupported(format!("unknown table `{name}`")),
                }
            }
            Expr::TableCtor { rows, .. } => {
                let mut out = Vec::with_capacity(rows.len());
                let mut width = None;
                for r in rows {
                    let vals: Vec<V> = match r {
                        Expr::Tuple { items, .. } => items
                            .iter()
                            .map(|i| self.scalar(i, &s))
                            .collect::<Result<_, _>>()?,
                        other => vec![self.scalar(other, &s)?],
                    };
                    if *width.get_or_insert(vals.len()) != vals.len() {
                        return dax_error("table constructor rows of different widths");
                    }
                    out.push(vals);
                }
                // Columns [Value], or [Value1], [Value2]…
                let width = width.unwrap_or(1);
                let cols = (1..=width)
                    .map(|i| {
                        let name = if width == 1 {
                            "Value".to_string()
                        } else {
                            format!("Value{i}")
                        };
                        // One type for the column: numbers widen, any other mix is text
                        // (`{1, "a"}` is a text column, `{1, 2.5}` a double one; Desktop 2.158).
                        let mut ty: Option<DataType> = None;
                        for r in rows {
                            let item = match r {
                                Expr::Tuple { items, .. } => items.get(i.saturating_sub(1)),
                                other => Some(other),
                            };
                            if let Some(t) = item.and_then(|e| self.static_type(e, 0)) {
                                ty = Some(match (ty, t) {
                                    (None, t) => t,
                                    (Some(a), b) if a == b => a,
                                    (Some(DataType::Int64), DataType::Decimal)
                                    | (Some(DataType::Decimal), DataType::Int64) => DataType::Decimal,
                                    (
                                        Some(DataType::Int64 | DataType::Double | DataType::Decimal),
                                        DataType::Int64 | DataType::Double | DataType::Decimal,
                                    ) => DataType::Double,
                                    _ => DataType::String,
                                });
                            }
                        }
                        TCol::named(name, ty)
                    })
                    .collect::<Vec<TCol>>();
                for (i, c) in cols.iter().enumerate() {
                    if c.ty == Some(DataType::String) {
                        for r in &mut out {
                            if let Some(v) = r.get_mut(i)
                                && !matches!(v, V::Blank | V::Text(_))
                            {
                                *v = V::Text(text_of(v));
                            }
                        }
                    }
                }
                Ok(Tab::tuples(cols, out))
            }
            Expr::Var { bindings, body, .. } => {
                let mut env = s.env.clone();
                for b in bindings {
                    let val = {
                        let inner = Scope { env: &env, ..s };
                        if self.is_table_expr(&b.value, &env) {
                            Bind::T(self.table(&b.value, &inner)?)
                        } else {
                            Bind::S(self.scalar(&b.value, &inner)?)
                        }
                    };
                    env.push((b.name.to_lowercase(), val));
                }
                let inner = Scope { env: &env, ..s };
                self.table(body, &inner)
            }
            Expr::Call { name, args, .. } => self.table_call(name, args, &s),
            _ => unsupported("an expression that is not a table where a table is expected"),
        }
    }

    fn table_call(&mut self, name: &str, args: &[Option<Expr>], s: &Scope) -> Result<Tab, Fail> {
        let up = name.to_ascii_uppercase();
        match up.as_str() {
            "FILTER" => {
                let t = self.table(need(args, 0, &up)?, s)?;
                self.filter_tab(&t, need(args, 1, &up)?, s)
            }
            "VALUES" | "DISTINCT" => match need(args, 0, &up)? {
                e @ Expr::Column { table, name, .. }
                    if table.is_some() || self.ext_value(name, s.rc).is_none() =>
                {
                    let c = self.col_arg(e, s.rc, &up)?;
                    let codes = self.values_codes(s.fc, c, up == "VALUES")?;
                    // the column once, not once per value
                    let d = self.col(c)?;
                    let rows: Vec<Vec<V>> = codes
                        .iter()
                        .map(|code| vec![d.values.get(*code as usize).map_or(V::Blank, V::of_cell)])
                        .collect();
                    let t = Tab::tuples(vec![TCol::of(c)], rows);
                    self.note_tuple_codes(&t, c, codes)?;
                    Ok(t)
                }
                other => self.values_table(&up, other, s),
            },
            "ALL" | "ALLNOBLANKROW" | "ALLSELECTED" | "REMOVEFILTERS" | "ALLCROSSFILTERED" => {
                let first = need(args, 0, &up)?;
                if up == "ALLSELECTED" && !self.outer.filters.is_empty() {
                    // the rows or values the visual's outer filters leave
                    let outer = self.outer.child();
                    if let Expr::Name { name, .. } = first {
                        let Some(t) = self.table_id(name) else {
                            return unsupported(format!("{up}() of `{name}`"));
                        };
                        let rows = self.visible_rows(&outer, t)?;
                        return Ok(Tab::rows_of(t, rows));
                    }
                    if args.len() == 1 {
                        let c = self.col_arg(first, s.rc, &up)?;
                        let codes = self.values_codes(&outer, c, true)?;
                        let rows = codes
                            .iter()
                            .map(|code| self.value_of_code(c, *code).map(|v| vec![v]))
                            .collect::<Result<Vec<_>, _>>()?;
                        return Ok(Tab::tuples(vec![TCol::of(c)], rows));
                    }
                }
                if let Expr::Name { name, .. } = first {
                    let Some(t) = self.table_id(name) else {
                        return unsupported(format!("{up}() of `{name}`"));
                    };
                    let n = self.rows(t)?;
                    let mut rows: Vec<u32> = (0..n).filter_map(|r| u32::try_from(r).ok()).collect();
                    // ALL(table) includes the virtual blank row when it exists.
                    if up != "ALLNOBLANKROW"
                        && self.has_blank_row(t)?
                        && let Ok(b) = u32::try_from(n)
                    {
                        rows.push(b);
                    }
                    return Ok(Tab::rows_of(t, rows));
                }
                let mut cols = Vec::new();
                for a in args.iter().flatten() {
                    cols.push(self.col_arg(a, s.rc, &up)?);
                }
                let t = self.all_tuples(&cols)?;
                if up != "ALLNOBLANKROW" {
                    return Ok(t);
                }
                let Tab::Tuples { cols, rows } = t else {
                    return Ok(t);
                };
                Ok(Tab::Tuples {
                    cols,
                    rows: Arc::new(
                        rows.iter()
                            .filter(|r| !r.iter().all(V::is_blank))
                            .cloned()
                            .collect(),
                    ),
                })
            }
            "CALCULATETABLE" => {
                let fc = self.calc_ctx(args, s)?;
                let inner = Scope {
                    fc: &fc,
                    rc: &[],
                    ..*s
                };
                self.table(need(args, 0, &up)?, &inner)
            }
            "KEEPFILTERS" => self.table(need(args, 0, &up)?, s),
            // Desktop's line charts sample a large axis down to `size` points; a table that is
            // not larger is returned as it is (Desktop 2.159). Sampling itself is not evaluated.
            "SAMPLEAXISWITHLOCALMINMAX" => {
                let size = num(&self.scalar(need(args, 0, &up)?, s)?)?.unwrap_or(0.0);
                let t = self.table(need(args, 1, &up)?, s)?;
                if (t.len() as f64) <= size {
                    Ok(t)
                } else {
                    unsupported(format!(
                        "SAMPLEAXISWITHLOCALMINMAX() sampling {} rows down to {size}",
                        t.len()
                    ))
                }
            }
            "TREATAS" => {
                let t = self.table(need(args, 0, &up)?, s)?;
                let rows = self.tab_values(&t)?;
                let mut cols = Vec::new();
                for a in args.iter().skip(1).flatten() {
                    cols.push(TCol::of(self.col_arg(a, s.rc, &up)?));
                }
                if rows.first().is_some_and(|r| r.len() != cols.len()) {
                    return dax_error("TREATAS() column count does not match the table");
                }
                Ok(Tab::Tuples {
                    cols: Arc::new(cols),
                    rows,
                })
            }
            _ if window::handles_table(&up) => self.window_table(&up, args, s),
            _ if pick::handles_table(&up) => self.pick_table(&up, args, s),
            _ => match self.table_function(&up, args, s)? {
                Some(t) => Ok(t),
                None => match self.more_table(&up, args, s)? {
                    Some(t) => Ok(t),
                    None => match self.info_table(&up, args, s)? {
                        Some(t) => Ok(t),
                        None => self.time_intelligence(&up, args, s),
                    },
                },
            },
        }
    }

    // --- time intelligence ------------------------------------------------------------------

    /// The date column and the sorted days of a `dates` argument.
    fn dates_arg(&mut self, e: &Expr, s: &Scope, f: &str) -> Result<(ColId, Vec<i64>), Fail> {
        let (c, vals): (ColId, Vec<V>) = match e {
            Expr::Column { table, name, .. } if self.measure_def(table.as_deref(), name).is_none() => {
                let c = self.resolve_col(e, table.as_deref(), name, s.rc)?;
                // a column argument is CALCULATETABLE(DISTINCT(column)): context transition
                let codes = if s.rc.is_empty() {
                    self.values_codes(s.fc, c, false)?
                } else {
                    let fc = self.transitioned(s.fc, s.rc)?;
                    self.values_codes(&fc, c, false)?
                };
                let vals = codes
                    .iter()
                    .map(|code| self.value_of_code(c, *code))
                    .collect::<Result<_, _>>()?;
                (c, vals)
            }
            other => match self.table(other, s)? {
                Tab::Tuples { cols, rows } if cols.len() == 1 => match cols.first().and_then(|c| c.lin) {
                    Some(c) => (c, rows.iter().filter_map(|r| r.first().cloned()).collect()),
                    None => return unsupported(format!("{f}() of a table without a date column")),
                },
                _ => return unsupported(format!("{f}() of a table that is not one date column")),
            },
        };
        let mut days: Vec<i64> = vals
            .iter()
            .filter_map(|v| num(v).ok().flatten())
            .filter(|x| x.is_finite() && x.abs() < 3_000_000.0)
            .map(|x| x.floor() as i64)
            .collect();
        days.sort_unstable();
        days.dedup();
        Ok((c, days))
    }

    /// The dates of column `c` within the union of `ranges`.
    fn dates_in(&mut self, c: ColId, ranges: &[(i64, i64)]) -> Result<Tab, Fail> {
        let all = self.all_dates(c)?;
        let mut picked: Vec<(i64, f64)> = Vec::new();
        for (lo, hi) in ranges {
            if lo > hi {
                continue;
            }
            let start = all.partition_point(|(d, _)| d < lo);
            for p in all.iter().skip(start) {
                if p.0 > *hi {
                    break;
                }
                picked.push(*p);
            }
        }
        picked.sort_by_key(|a| a.0);
        picked.dedup_by_key(|p| p.0);
        self.tick(picked.len())?;
        Ok(Tab::tuples(
            vec![TCol::of(c)],
            picked.into_iter().map(|(_, x)| vec![V::Date(x)]).collect(),
        ))
    }

    fn interval(&self, e: Option<&Expr>, f: &str) -> Result<Interval, Fail> {
        match e.and_then(keyword).as_deref() {
            Some("DAY") => Ok(Interval::Day),
            Some("MONTH") => Ok(Interval::Month),
            Some("QUARTER") => Ok(Interval::Quarter),
            Some("YEAR") => Ok(Interval::Year),
            _ => unsupported(format!("{f}() interval")),
        }
    }

    /// The fiscal year end (month, day) of an optional `year_end_date` argument.
    fn year_end(&mut self, e: Option<&Expr>, s: &Scope) -> Result<(i64, i64), Fail> {
        let Some(e) = e else {
            return Ok((12, 31));
        };
        let text = match self.scalar(e, s)? {
            V::Text(t) => t,
            V::Date(x) => {
                let (_, m, d) = ymd(x.floor() as i64);
                return Ok((m, d));
            }
            V::Blank => return Ok((12, 31)),
            _ => return unsupported("a year-end date that is not text"),
        };
        let parts: Vec<i64> = text
            .split(['/', '-', '.', ' '])
            .filter(|p| !p.is_empty())
            .take(2)
            .map(|p| p.trim().parse::<i64>())
            .collect::<Result<_, _>>()
            .map_err(|_| Fail::Unsupported(format!("year-end date `{text}`")))?;
        let [a, b] = parts.as_slice() else {
            return unsupported(format!("year-end date `{text}`"));
        };
        let (m, d) = if *a > 12 {
            (*b, *a)
        } else if *b > 12 || !self.day_first {
            (*a, *b)
        } else {
            (*b, *a)
        };
        if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
            return unsupported(format!("year-end date `{text}`"));
        }
        Ok((m, d))
    }

    fn time_intelligence(&mut self, up: &str, args: &[Option<Expr>], s: &Scope) -> Result<Tab, Fail> {
        let f = up;
        // a calendar reference: calendar-based time intelligence (`preview_eval_calendar.rs`)
        if calendar_ti::takes_calendar(up)
            && let Some(cal) = arg(args, 0).and_then(|a| self.calendar_of(a))
        {
            return Ok(self.calendar_table(up, &cal, args, s)?.0);
        }
        if calendar_ti::takes_calendar(up) {
            self.refuse_nested_calendar(up, arg(args, 0))?;
        }
        if matches!(
            up,
            "DATESWTD" | "STARTOFWEEK" | "ENDOFWEEK" | "PREVIOUSWEEK" | "NEXTWEEK"
        ) {
            return dax_error(format!(
                "{up}() needs a calendar: week functions only work with calendar-based time intelligence"
            ));
        }
        match up {
            "DATESYTD" | "DATESQTD" | "DATESMTD" => {
                let (c, d) = self.dates_arg(need(args, 0, f)?, s, f)?;
                let Some(&last) = d.last() else {
                    return Ok(Tab::empty_of(c));
                };
                let start = match up {
                    "DATESYTD" => {
                        let ye = self.year_end(arg(args, 1), s)?;
                        fiscal_year(last, ye).0
                    }
                    "DATESQTD" => month_start(quarter_index(last) * 3),
                    _ => month_start(month_index(last)),
                };
                self.dates_in(c, &[(start, last)])
            }
            "SAMEPERIODLASTYEAR" | "DATEADD" => {
                let (c, d) = self.dates_arg(need(args, 0, f)?, s, f)?;
                let (k, iv) = if up == "DATEADD" {
                    let k = self.int_arg(need(args, 1, f)?, s)?;
                    (k, self.interval(arg(args, 2), f)?)
                } else {
                    (-1, Interval::Year)
                };
                let ranges = shift_blocks(&d, k, iv);
                self.dates_in(c, &ranges)
            }
            "PARALLELPERIOD" => {
                let (c, d) = self.dates_arg(need(args, 0, f)?, s, f)?;
                let k = self.int_arg(need(args, 1, f)?, s)?;
                let iv = self.interval(arg(args, 2), f)?;
                let (Some(&lo), Some(&hi)) = (d.first(), d.last()) else {
                    return Ok(Tab::empty_of(c));
                };
                let range = match iv {
                    Interval::Month => (month_start(month_index(lo) + k), month_end(month_index(hi) + k)),
                    Interval::Quarter => (
                        month_start((quarter_index(lo) + k) * 3),
                        month_end((quarter_index(hi) + k) * 3 + 2),
                    ),
                    Interval::Year => {
                        let (ylo, _, _) = ymd(lo);
                        let (yhi, _, _) = ymd(hi);
                        (serial(ylo + k, 1, 1), serial(yhi + k, 12, 31))
                    }
                    Interval::Day => return unsupported("PARALLELPERIOD() by day"),
                };
                self.dates_in(c, &[range])
            }
            "PREVIOUSDAY" | "PREVIOUSMONTH" | "PREVIOUSQUARTER" | "PREVIOUSYEAR" | "NEXTDAY"
            | "NEXTMONTH" | "NEXTQUARTER" | "NEXTYEAR" => {
                let (c, d) = self.dates_arg(need(args, 0, f)?, s, f)?;
                let (Some(&lo), Some(&hi)) = (d.first(), d.last()) else {
                    return Ok(Tab::empty_of(c));
                };
                let range = match up {
                    "PREVIOUSDAY" => (lo - 1, lo - 1),
                    "NEXTDAY" => (hi + 1, hi + 1),
                    "PREVIOUSMONTH" => (month_start(month_index(lo) - 1), month_end(month_index(lo) - 1)),
                    "NEXTMONTH" => (month_start(month_index(hi) + 1), month_end(month_index(hi) + 1)),
                    "PREVIOUSQUARTER" => {
                        let q = quarter_index(lo) - 1;
                        (month_start(q * 3), month_end(q * 3 + 2))
                    }
                    "NEXTQUARTER" => {
                        let q = quarter_index(hi) + 1;
                        (month_start(q * 3), month_end(q * 3 + 2))
                    }
                    "PREVIOUSYEAR" => {
                        let ye = self.year_end(arg(args, 1), s)?;
                        let end = fiscal_year(lo, ye).0 - 1;
                        (fiscal_year(end, ye).0, end)
                    }
                    _ => {
                        let ye = self.year_end(arg(args, 1), s)?;
                        let start = fiscal_year(hi, ye).1 + 1;
                        (start, fiscal_year(start, ye).1)
                    }
                };
                self.dates_in(c, &[range])
            }
            "DATESINPERIOD" => {
                let c = self.col_arg(need(args, 0, f)?, s.rc, f)?;
                let Some(start) = self.day_arg(need(args, 1, f)?, s)? else {
                    return Ok(Tab::empty_of(c));
                };
                let k = self.int_arg(need(args, 2, f)?, s)?;
                let iv = self.interval(arg(args, 3), f)?;
                let range = match (iv, k.cmp(&0)) {
                    (_, Ordering::Equal) => return Ok(Tab::empty_of(c)),
                    (Interval::Day, Ordering::Less) => (start + k + 1, start),
                    (Interval::Day, Ordering::Greater) => (start, start + k - 1),
                    (iv, Ordering::Less) => (shift_eom(start, k * iv.months()) + 1, start),
                    (iv, Ordering::Greater) => (start, shift_eom(start, k * iv.months()) - 1),
                };
                self.dates_in(c, &[range])
            }
            "DATESBETWEEN" => {
                let c = self.col_arg(need(args, 0, f)?, s.rc, f)?;
                let all = self.all_dates(c)?;
                let lo = match self.day_arg(need(args, 1, f)?, s)? {
                    Some(x) => x,
                    None => all.first().map_or(0, |p| p.0),
                };
                let hi = match self.day_arg(need(args, 2, f)?, s)? {
                    Some(x) => x,
                    None => all.last().map_or(0, |p| p.0),
                };
                self.dates_in(c, &[(lo, hi)])
            }
            "FIRSTDATE" | "LASTDATE" => {
                let (c, d) = self.dates_arg(need(args, 0, f)?, s, f)?;
                let pick = if up == "FIRSTDATE" { d.first() } else { d.last() };
                match pick {
                    Some(x) => self.dates_in(c, &[(*x, *x)]),
                    None => Ok(Tab::empty_of(c)),
                }
            }
            "STARTOFMONTH" | "STARTOFQUARTER" | "STARTOFYEAR" | "ENDOFMONTH" | "ENDOFQUARTER"
            | "ENDOFYEAR" => {
                let (c, d) = self.dates_arg(need(args, 0, f)?, s, f)?;
                let start = up.starts_with("START");
                let pivot = if start { d.first() } else { d.last() };
                let Some(&x) = pivot else {
                    return Ok(Tab::empty_of(c));
                };
                let (lo, hi) = match up {
                    _ if up.ends_with("MONTH") => (month_start(month_index(x)), month_end(month_index(x))),
                    _ if up.ends_with("QUARTER") => {
                        let q = quarter_index(x);
                        (month_start(q * 3), month_end(q * 3 + 2))
                    }
                    _ => {
                        let ye = self.year_end(arg(args, 1), s)?;
                        fiscal_year(x, ye)
                    }
                };
                let Tab::Tuples { rows, .. } = self.dates_in(c, &[(lo, hi)])? else {
                    return internal();
                };
                let one = if start { rows.first() } else { rows.last() };
                Ok(Tab::tuples(vec![TCol::of(c)], one.cloned().into_iter().collect()))
            }
            _ => unsupported(format!("{up}() is not evaluated by the preview")),
        }
    }
}

/// `DATEADD` ranges: each run of consecutive days moves by `k` intervals; a run of several days
/// ending on a month's last day ends on the target month's last day.
fn shift_blocks(days: &[i64], k: i64, iv: Interval) -> Vec<(i64, i64)> {
    let mut blocks: Vec<(i64, i64)> = Vec::new();
    for d in days {
        match blocks.last_mut() {
            Some((_, e)) if *e + 1 == *d => *e = *d,
            _ => blocks.push((*d, *d)),
        }
    }
    let months = k * iv.months();
    blocks
        .into_iter()
        .map(|(s, e)| {
            if iv == Interval::Day {
                (s + k, e + k)
            } else if s == e {
                let x = shift_clip(s, months);
                (x, x)
            } else {
                let end = if is_month_end(e) {
                    month_end(month_index(e) + months)
                } else {
                    shift_clip(e, months)
                };
                (shift_clip(s, months), end)
            }
        })
        .collect()
}

/// The aggregation a function call with `n` arguments computes over a column (`SUM(T[c])`).
fn agg_function(up: &str, n: usize) -> Option<Agg> {
    Some(match up {
        "SUM" => Agg::Sum,
        "AVERAGE" => Agg::Avg,
        "MIN" if n == 1 => Agg::Min,
        "MAX" if n == 1 => Agg::Max,
        "MEDIAN" => Agg::Median,
        "STDEV.S" => Agg::StdDev,
        "VAR.S" => Agg::Var,
        "STDEV.P" => Agg::StdDevP,
        "VAR.P" => Agg::VarP,
        "COUNT" | "COUNTA" => Agg::Count,
        "COUNTBLANK" => Agg::CountBlank,
        "DISTINCTCOUNT" => Agg::Distinct,
        "DISTINCTCOUNTNOBLANK" => Agg::DistinctNoBlank,
        _ => return None,
    })
}

fn iterate_agg(f: &str, vals: Vec<V>) -> Result<V, Fail> {
    let present: Vec<V> = vals.into_iter().filter(|v| !v.is_blank()).collect();
    match f {
        "COUNTX" | "COUNTAX" => Ok(if present.is_empty() {
            V::Blank
        } else {
            V::Num(present.len() as f64)
        }),
        "MINX" | "MAXX" => {
            let mut best: Option<V> = None;
            for v in present {
                let better = match &best {
                    None => true,
                    Some(b) => {
                        let o = compare_vals(&v, b)?;
                        if f == "MINX" {
                            o == Ordering::Less
                        } else {
                            o == Ordering::Greater
                        }
                    }
                };
                if better {
                    best = Some(v);
                }
            }
            Ok(best.unwrap_or(V::Blank))
        }
        _ if !present.is_empty() && present.iter().all(|v| matches!(v, V::Cur(_))) => {
            let total: i128 = present
                .iter()
                .map(|v| if let V::Cur(n) = v { i128::from(*n) } else { 0 })
                .sum();
            let count = i128::try_from(present.len()).unwrap_or(i128::MAX);
            let r = if f == "AVERAGEX" {
                currency::div_round(total, count)
            } else {
                i64::try_from(total).ok()
            };
            r.map_or_else(overflow, |n| Ok(V::Cur(n)))
        }
        _ => {
            let mut sum = 0.0;
            let mut n = 0u64;
            for v in &present {
                if let Some(x) = num(v)? {
                    sum += x;
                    n = n.saturating_add(1);
                }
            }
            Ok(match (n, f) {
                (0, _) => V::Blank,
                (_, "AVERAGEX") => V::Num(sum / n as f64),
                _ => V::Num(sum),
            })
        }
    }
}

/// `SUM`, `AVERAGE`, `MEDIAN`, `STDEV.S` of a Currency column (`counts[code]` visible rows per
/// value): Currency results, exact sums, rounded half away from zero to 4 decimals as the engine
/// does. `VAR.S` is a double and takes the double path.
fn currency_aggregate(agg: Agg, values: &[CellValue], counts: &[u64]) -> Result<V, Fail> {
    let mut items: Vec<(i64, u64)> = Vec::new();
    for (code, n) in counts.iter().enumerate() {
        if *n == 0 {
            continue;
        }
        match values.get(code) {
            Some(CellValue::Decimal(x)) => items.push((*x, *n)),
            Some(CellValue::Null) | None => {}
            Some(other) => match as_num(other).and_then(currency::from_f64) {
                Some(x) => items.push((x, *n)),
                None => return overflow(),
            },
        }
    }
    let count: u64 = items.iter().map(|(_, n)| *n).sum();
    if count == 0 {
        return Ok(V::Blank);
    }
    let total: i128 = items
        .iter()
        .map(|(x, n)| i128::from(*x).saturating_mul(i128::from(*n)))
        .fold(0i128, i128::saturating_add);
    let count_i = i128::from(count);
    let r = match agg {
        Agg::Sum => i64::try_from(total).ok(),
        Agg::Avg => currency::div_round(total, count_i),
        Agg::Median => {
            items.sort_by_key(|(x, _)| *x);
            let nth = |k: u64| {
                let mut seen = 0u64;
                for (x, n) in &items {
                    seen = seen.saturating_add(*n);
                    if seen > k {
                        return *x;
                    }
                }
                items.last().map_or(0, |p| p.0)
            };
            if count % 2 == 1 {
                Some(nth(count / 2))
            } else {
                currency::div_round(i128::from(nth(count / 2 - 1)) + i128::from(nth(count / 2)), 2)
            }
        }
        _ => {
            let population = agg == Agg::StdDevP;
            if count < if population { 1 } else { 2 } {
                return Ok(V::Blank);
            }
            let nf = count as f64;
            let mean = currency::to_f64(i64::try_from(total).unwrap_or(0)) / nf;
            let ss: f64 = items
                .iter()
                .map(|(x, n)| {
                    let d = currency::to_f64(*x) - mean;
                    d * d * *n as f64
                })
                .sum();
            currency::from_f64((ss / if population { nf } else { nf - 1.0 }).sqrt())
        }
    };
    r.map_or_else(overflow, |n| Ok(V::Cur(n)))
}

// ---------------------------------------------------------------------------------------------
// report filters and slicers
// ---------------------------------------------------------------------------------------------

fn val_v(v: &Val) -> V {
    match v {
        Val::Null => V::Blank,
        Val::Num(x) => V::Num(*x),
        Val::Text(t) => V::Text(t.clone()),
        Val::Bool(b) => V::Bool(*b),
        Val::Date(x) => V::Date(*x),
    }
}

/// A filter condition compiled against value positions.
pub(super) enum CCond {
    In {
        at: Vec<usize>,
        set: HashSet<Vec<Option<Key>>>,
    },
    Cmp {
        at: usize,
        op: CmpOp,
        right: V,
    },
    Text {
        at: usize,
        op: TextOp,
        right: String,
    },
    Not(Box<CCond>),
    And(Box<CCond>, Box<CCond>),
    Or(Box<CCond>, Box<CCond>),
}

impl CCond {
    /// `pos`: the position of an operand's value.
    pub(super) fn compile(c: &Cond, pos: &dyn Fn(&Operand) -> Option<usize>) -> Option<CCond> {
        Some(match c {
            Cond::In { exprs, rows } => {
                let at = exprs.iter().map(pos).collect::<Option<Vec<_>>>()?;
                let set = rows
                    .iter()
                    .map(|r| r.iter().map(|x| val_v(x).key()).collect())
                    .collect();
                CCond::In { at, set }
            }
            Cond::Cmp { op, left, right } => CCond::Cmp {
                at: pos(left)?,
                op: *op,
                right: val_v(right),
            },
            Cond::Text { op, left, right } => CCond::Text {
                at: pos(left)?,
                op: *op,
                right: right.to_lowercase(),
            },
            Cond::Not(x) => CCond::Not(Box::new(Self::compile(x, pos)?)),
            Cond::And(a, b) => CCond::And(Box::new(Self::compile(a, pos)?), Box::new(Self::compile(b, pos)?)),
            Cond::Or(a, b) => CCond::Or(Box::new(Self::compile(a, pos)?), Box::new(Self::compile(b, pos)?)),
        })
    }

    /// Whether the values satisfy the condition (Power BI filter semantics: DAX comparisons,
    /// `= blank` is "is blank", case-insensitive text).
    pub(super) fn holds(&self, vals: &[V]) -> bool {
        let at = |i: &usize| vals.get(*i).unwrap_or(&V::Blank);
        match self {
            CCond::In { at: idx, set } => {
                let key: Vec<Option<Key>> = idx.iter().map(|i| at(i).key()).collect();
                set.contains(&key)
            }
            CCond::Cmp { at: i, op, right } => {
                let v = at(i);
                if right.is_blank() && *op == CmpOp::Eq {
                    return v.is_blank();
                }
                let Ok(o) = compare_vals(v, right) else {
                    return false;
                };
                match op {
                    CmpOp::Eq => o == Ordering::Equal,
                    CmpOp::Gt => o == Ordering::Greater,
                    CmpOp::Ge => o != Ordering::Less,
                    CmpOp::Lt => o == Ordering::Less,
                    CmpOp::Le => o != Ordering::Greater,
                }
            }
            CCond::Text { at: i, op, right } => {
                let v = at(i);
                if v.is_blank() {
                    return false;
                }
                let t = text_of(v).to_lowercase();
                match op {
                    TextOp::Contains => t.contains(right.as_str()),
                    TextOp::StartsWith => t.starts_with(right.as_str()),
                    TextOp::EndsWith => t.ends_with(right.as_str()),
                }
            }
            CCond::Not(x) => !x.holds(vals),
            CCond::And(a, b) => a.holds(vals) && b.holds(vals),
            CCond::Or(a, b) => a.holds(vals) || b.holds(vals),
        }
    }
}

/// A row value as a condition operand.
pub(super) fn out_v(o: &Out) -> V {
    match o {
        Out::Blank => V::Blank,
        Out::Num(x) => V::Num(*x),
        Out::Cell(c) => V::of_cell(c),
    }
}

impl Engine<'_, '_> {
    /// The candidate rows of a visual or `SUMMARIZECOLUMNS` (codes of `groups`): the existing
    /// combinations of each table's group columns among the rows visible in `fc` (plus the
    /// table's blank row when visible), crossed between tables.
    fn combos(&mut self, fc: &FCtx, groups: &[ColId]) -> Result<Combos, Fail> {
        let mut by_table: Vec<(TabId, Vec<usize>)> = Vec::new();
        for (gi, c) in groups.iter().enumerate() {
            let t = self.col_table(*c)?;
            match by_table.iter_mut().find(|(x, _)| *x == t) {
                Some((_, v)) => v.push(gi),
                None => by_table.push((t, vec![gi])),
            }
        }
        let mut candidates = Combos::new(groups.len());
        candidates.push(&[]);
        for (t, gis) in &by_table {
            let cols: Vec<ColId> = gis.iter().filter_map(|g| groups.get(*g).copied()).collect();
            let tuples = match self.store.is_some() {
                true => self.combos_storage(fc, *t, &cols)?,
                false => None,
            };
            let tuples = match tuples {
                Some(t) => t,
                None => self.combos_table(fc, *t, &cols)?,
            };
            let total = candidates.len().saturating_mul(tuples.len());
            if total > self.max_candidates {
                return unsupported(format!(
                    "more than {} combinations of grouping values to evaluate",
                    self.max_candidates
                ));
            }
            // one table holding every grouping column, in order: its tuples are the candidates
            if by_table.len() == 1 && gis.iter().enumerate().all(|(i, g)| i == *g) {
                return Ok(tuples);
            }
            let charge = self.charge_bytes(
                &|| format!("the {total} combinations of {} grouping columns", groups.len()),
                Combos::bytes_for(groups.len(), total),
            )?;
            let mut next = Combos::with_capacity(groups.len(), total).charged(charge);
            let mut c: Vec<u32> = Vec::with_capacity(groups.len());
            for cand in candidates.iter() {
                for tup in tuples.iter() {
                    c.clear();
                    c.extend_from_slice(cand);
                    for (gi, code) in gis.iter().zip(tup) {
                        if let Some(slot) = c.get_mut(*gi) {
                            *slot = *code;
                        }
                    }
                    next.push(&c);
                }
            }
            candidates = next;
        }
        Ok(candidates)
    }

    /// The existing combinations of `cols` (columns of `t`) among its rows visible in `fc`, in
    /// the order of their first row, then the blank row's when it is visible.
    fn combos_table(&mut self, fc: &FCtx, t: TabId, cols: &[ColId]) -> Result<Combos, Fail> {
        let t = &t;
        {
            let datas = cols.iter().map(|c| self.col(*c)).collect::<Result<Vec<_>, _>>()?;
            let n = self.rows(*t)?;
            let mask = self.mask(fc, *t)?;
            self.tick(n)?;
            let mut seen: HashSet<Vec<u32>> = HashSet::new();
            let mut tuples = Combos::new(cols.len());
            // reserved by blocks before they are built: the combination and its entry in `seen`
            let per = Combos::bytes_for(cols.len(), 1).saturating_add(64);
            let mut charges: Vec<mem::Charge> = Vec::new();
            for row in 0..n {
                if !visible(mask.as_ref(), row) {
                    continue;
                }
                let k: Vec<u32> = datas.iter().map(|d| d.codes.get(row).unwrap_or(0)).collect();
                if !seen.contains(&k) {
                    if tuples.len().is_multiple_of(65_536) && self.mem.on() {
                        let at = tuples.len();
                        charges.push(self.charge_bytes(
                            &|| format!("the combinations of {} grouping columns (over {at})", cols.len()),
                            per.saturating_mul(65_536),
                        )?);
                    }
                    tuples.push(&k);
                    seen.insert(k);
                }
            }
            let zero = vec![0u32; cols.len()];
            if visible(mask.as_ref(), n) && self.has_blank_row(*t)? && !seen.contains(&zero) {
                tuples.push(&zero);
            }
            drop(seen);
            drop(charges);
            let bytes = tuples.bytes();
            let c = self.charge_bytes(
                &|| format!("the combinations of {} grouping columns", cols.len()),
                bytes,
            )?;
            Ok(tuples.charged(c))
        }
    }

    /// The outer filters on `cols` (all of them for `None`), projected on those columns.
    fn outer_on(&mut self, cols: Option<&HashSet<ColId>>) -> Result<Vec<Filter>, Fail> {
        let outer: Vec<Filter> = self.outer.filters.clone();
        let Some(set) = cols else {
            return Ok(outer);
        };
        let mut out = Vec::new();
        for f in &outer {
            let hit = f.cols.iter().filter(|c| set.contains(c)).count();
            if hit == f.cols.len() {
                out.push(f.clone());
            } else if hit > 0 {
                let others: HashSet<ColId> = f.cols.iter().filter(|c| !set.contains(c)).copied().collect();
                if let Some(p) = self.project(f, &others)? {
                    out.push(p);
                }
            }
        }
        Ok(out)
    }

    /// The filter-context filter of a condition over columns of one table (`Err`: why it is
    /// not applied).
    fn cond_filter(&mut self, c: &Cond) -> Result<Result<Filter, String>, Fail> {
        let ops = c.operands();
        let mut cols: Vec<ColId> = Vec::new();
        for o in &ops {
            let Operand::Col { table, column } = o else {
                return Ok(Err("a condition mixing columns and measures".into()));
            };
            let Some(id) = self.col_id(table, column) else {
                return Ok(Err(format!("a filter on unknown column '{table}'[{column}]")));
            };
            cols.push(id);
        }
        let Some(first) = cols.first().copied() else {
            return Ok(Err("a condition without a column".into()));
        };
        let t = self.col_table(first)?;
        for x in &cols {
            if self.col_table(*x)? != t {
                return Ok(Err("a condition over columns of several tables".into()));
            }
        }
        let Some(cc) = CCond::compile(c, &|o| ops.iter().position(|x| *x == o)) else {
            return internal();
        };
        if cols.len() == 1 {
            let d = self.col(first)?;
            self.tick(d.values.len())?;
            let allowed: Vec<bool> = d
                .values
                .iter()
                .map(|x| cc.holds(std::slice::from_ref(&V::of_cell(x))))
                .collect();
            return Ok(Ok(Filter {
                table: t,
                cols: Arc::new(cols),
                kind: FKind::Codes(Arc::new(allowed)),
            }));
        }
        let datas = cols.iter().map(|x| self.col(*x)).collect::<Result<Vec<_>, _>>()?;
        let n = self.rows(t)?;
        self.tick(n.saturating_mul(cols.len()))?;
        let mut m: Vec<bool> = (0..n)
            .map(|row| {
                let vals: Vec<V> = datas
                    .iter()
                    .map(|d| {
                        d.codes
                            .get(row)
                            .and_then(|code| d.values.get(code as usize))
                            .map_or(V::Blank, V::of_cell)
                    })
                    .collect();
                cc.holds(&vals)
            })
            .collect();
        m.push(cc.holds(&vec![V::Blank; cols.len()]));
        Ok(Ok(Filter {
            table: t,
            cols: Arc::new(cols),
            kind: FKind::Rows(Arc::new(m)),
        }))
    }

    /// The value of a measure or aggregation operand in `fc`.
    fn operand_value(&mut self, o: &Operand, fc: &FCtx) -> Result<Result<V, String>, Fail> {
        match o {
            Operand::Measure { table, name } => {
                let Some(def) = self
                    .measure_def(Some(table), name)
                    .or_else(|| self.measure_def(None, name))
                else {
                    return Ok(Err(format!("a filter on unknown measure [{name}]")));
                };
                let env = Env::new();
                let scope = Scope {
                    fc,
                    rc: &[],
                    env: &env,
                    depth: 0,
                    mdepth: 0,
                };
                self.eval_measure(&def, &scope).map(Ok)
            }
            Operand::Agg {
                table,
                column,
                function,
            } => {
                let (Some(agg), Some(c)) = (agg_of_pbir(function), self.col_id(table, column)) else {
                    return Ok(Err(format!("a filter on {function}('{table}'[{column}])")));
                };
                self.aggregate(agg, c, fc).map(Ok)
            }
            Operand::Col { .. } => Ok(Err("a top N filter ordered by a column".into())),
        }
    }

    /// The codes filter of a top N filter, evaluated in `fc` (ties at the N-th value are kept,
    /// as `TOPN` does; blank values are never kept).
    fn top_n(&mut self, t: &TopN, fc: &FCtx) -> Result<Result<Filter, String>, Fail> {
        let Some(c) = self.col_id(&t.table, &t.column) else {
            return Ok(Err(format!(
                "a top N filter on unknown column '{}'[{}]",
                t.table, t.column
            )));
        };
        let codes = self.values_codes(fc, c, true)?;
        if codes.len() > MAX_CANDIDATES {
            return Ok(Err(format!(
                "a top N filter over more than {MAX_CANDIDATES} values"
            )));
        }
        let mut scored: Vec<(u32, V)> = Vec::with_capacity(codes.len());
        for code in codes {
            let mut sub = fc.child();
            sub.filters.push(self.codes_filter(c, &[code])?);
            match self.operand_value(&t.by, &sub)? {
                Ok(v) if !v.is_blank() => scored.push((code, v)),
                Ok(_) => {}
                Err(e) => return Ok(Err(e)),
            }
        }
        let cmp = |a: &V, b: &V| compare_vals(a, b).unwrap_or(Ordering::Equal);
        scored.sort_by(|a, b| if t.desc { cmp(&b.1, &a.1) } else { cmp(&a.1, &b.1) });
        let keep: Vec<u32> = if t.n == 0 {
            Vec::new()
        } else {
            match scored.get(t.n.saturating_sub(1)) {
                None => scored.iter().map(|(code, _)| *code).collect(),
                Some((_, nth)) => {
                    let nth = nth.clone();
                    scored
                        .iter()
                        .filter(|(_, v)| {
                            let o = cmp(v, &nth);
                            if t.desc {
                                o != Ordering::Less
                            } else {
                                o != Ordering::Greater
                            }
                        })
                        .map(|(code, _)| *code)
                        .collect()
                }
            }
        };
        Ok(Ok(self.codes_filter(c, &keep)?))
    }

    /// The filter context of a visual's outer filters (column conditions, then top N filters
    /// evaluated in them). Filters that cannot be applied are named in `ignored`.
    fn outer_ctx(&mut self, items: &[Item], ignored: &mut Vec<String>) -> Result<FCtx, Fail> {
        let mut fc = FCtx::default();
        for it in items {
            if let Item::Cond(c) = it {
                match self.cond_filter(c)? {
                    Ok(f) => fc.filters.push(f),
                    Err(e) => ignored.push(e),
                }
            }
        }
        let mut tops = Vec::new();
        for it in items {
            if let Item::TopN(t) = it {
                match self.top_n(t, &fc)? {
                    Ok(f) => tops.push(f),
                    Err(e) => ignored.push(e),
                }
            }
        }
        // scoring the top N memoized visible rows without them: the context changes here
        if !tops.is_empty() {
            fc.memo.get_mut().clear();
        }
        fc.filters.extend(tops);
        self.outer = fc.child();
        Ok(fc)
    }
}

/// Visible rows of tables (by lower-cased name) under a visual's outer filters; `None`: nothing
/// filters the table.
pub(super) type Masks = HashMap<String, Option<Arc<Vec<bool>>>>;

/// The visible rows of `tables` under the outer filters `items`, for the fast path, and the
/// filters not applied.
pub(super) fn masks(
    ctx: &mut Ctx<'_>,
    items: &[Item],
    tables: &[String],
) -> Result<(Masks, Vec<String>), Fail> {
    let mut eng = Engine::new(ctx);
    let mut ignored = Vec::new();
    let fc = eng.outer_ctx(items, &mut ignored)?;
    let mut out = Masks::new();
    for t in tables {
        let Some(ti) = eng.table_id(t) else {
            continue;
        };
        out.insert(t.to_lowercase(), eng.mask(&fc, ti)?);
    }
    Ok((out, ignored))
}

// ---------------------------------------------------------------------------------------------
// visuals
// ---------------------------------------------------------------------------------------------

/// A value field of a visual.
enum Field {
    Group(usize),
    Measure(Rc<MeasureDef>),
    Agg(Agg, ColId),
}

/// Evaluates a visual like `SUMMARIZECOLUMNS(group columns, outer filters, values…)`, plus the
/// query's hidden fields.
pub(super) fn evaluate(ctx: &mut Ctx<'_>, v: &Visual, q: &Query<'_>) -> Result<Grid, Fail> {
    let model = ctx.model;
    let mut eng = Engine::new(ctx);
    let mut fields: Vec<Field> = Vec::new();
    let mut columns_json = Vec::new();
    let mut group_names: Vec<Option<(String, String)>> = Vec::new();
    let shown = v.fields.len();
    let all = v
        .fields
        .iter()
        .map(|b| (b.role.as_str(), &b.field))
        .chain(q.extra.iter().map(|f| ("", f)));
    for (i, (role, field)) in all.enumerate() {
        let display = v.fields.get(i).and_then(|b| b.display_name.clone());
        let mut group_name = None;
        let (field, kind, name, data_type) = match field {
            FieldRef::Column { table, column } => {
                let Some(c) = eng.col_id(table, column) else {
                    return unsupported(format!("unknown column '{table}'[{column}]"));
                };
                let gi = match eng.group_cols.iter().position(|x| *x == c) {
                    Some(gi) => gi,
                    None => {
                        eng.group_cols.push(c);
                        eng.group_cols.len().saturating_sub(1)
                    }
                };
                let (t, cn) = eng
                    .cols
                    .get(c)
                    .map(|i| {
                        (
                            eng.tabs.get(i.table).map(|t| t.name.clone()).unwrap_or_default(),
                            i.name.clone(),
                        )
                    })
                    .unwrap_or_default();
                let dt = data_type_name(model, &t, &cn);
                group_name = Some((t, cn.clone()));
                (Field::Group(gi), "column", cn, dt)
            }
            FieldRef::Measure { table, measure } => {
                let Some(def) = eng
                    .measure_def(Some(table), measure)
                    .or_else(|| eng.measure_def(None, measure))
                else {
                    return unsupported(format!("unknown measure [{measure}]"));
                };
                (
                    Field::Measure(def),
                    "measure",
                    measure.clone(),
                    "double".to_string(),
                )
            }
            FieldRef::Aggregation {
                table,
                column,
                function,
            } => {
                let Some(agg) = agg_of_pbir(function) else {
                    return unsupported(format!("aggregation `{function}`"));
                };
                let Some(c) = eng.col_id(table, column) else {
                    return unsupported(format!("unknown column '{table}'[{column}]"));
                };
                let cn = eng.cols.get(c).map(|i| i.name.clone()).unwrap_or_default();
                let dt = match agg {
                    Agg::Min | Agg::Max => data_type_name(model, table, &cn),
                    Agg::Count | Agg::Distinct => "int64".to_string(),
                    _ => "double".to_string(),
                };
                let label = format!("{} of {cn}", agg_label(function));
                (Field::Agg(agg, c), "aggregation", label, dt)
            }
            FieldRef::Other { .. } => {
                return unsupported(format!(
                    "field {} of role {} (hierarchy or expression)",
                    i.saturating_add(1),
                    role
                ));
            }
        };
        if i < shown {
            columns_json.push(json!({
                "name": display.unwrap_or(name),
                "role": role,
                "kind": kind,
                "data_type": data_type,
            }));
        }
        group_names.push(group_name);
        fields.push(field);
    }

    let mut ignored = Vec::new();
    let base = eng.outer_ctx(q.filters, &mut ignored)?;

    // candidate rows: existing combinations per table among the rows the outer filters leave,
    // crossed between tables
    let groups = eng.group_cols.clone();
    let candidates = eng.combos(&base, &groups)?;

    let has_values = fields.iter().take(shown).any(|f| !matches!(f, Field::Group(_)));
    let env = Env::new();
    let mut table: Vec<Vec<Out>> = Vec::new();
    for cand in candidates.iter() {
        let mut fc = base.child();
        for (c, code) in groups.iter().zip(cand) {
            fc.filters.push(eng.codes_filter(*c, &[*code])?);
        }
        let scope = Scope {
            fc: &fc,
            rc: &[],
            env: &env,
            depth: 0,
            mdepth: 0,
        };
        let mut row = Vec::with_capacity(fields.len());
        let mut any_value = false;
        for (i, f) in fields.iter().enumerate() {
            let cell = match f {
                Field::Group(gi) => {
                    let c = groups.get(*gi).copied().unwrap_or(0);
                    let code = cand.get(*gi).copied().unwrap_or(0);
                    eng.value_of_code(c, code)?.out()
                }
                Field::Measure(def) => eng.eval_measure(def, &scope)?.out(),
                Field::Agg(agg, c) => eng.aggregate(*agg, *c, &fc)?.out(),
            };
            if i < shown && !matches!(f, Field::Group(_)) {
                any_value |= cell != Out::Blank;
            }
            row.push(cell);
        }
        if has_values && !any_value && !groups.is_empty() {
            continue;
        }
        table.push(row);
    }
    let is_group: Vec<bool> = fields.iter().map(|f| matches!(f, Field::Group(_))).collect();
    Ok(Grid {
        columns: columns_json,
        is_group,
        groups: group_names,
        rows: table,
        ignored,
    })
}

/// The grand-total row of a table visual: each visible value field evaluated in the visual's
/// outer filter context (`filters`) without its grouping columns, as Desktop's total row is
/// (`ALLSELECTED` sees the outer filters, `ISINSCOPE` is false); blank under grouping columns.
pub(super) fn totals(ctx: &mut Ctx<'_>, v: &Visual, filters: &[Item]) -> Result<Vec<Out>, Fail> {
    let fields: Vec<FieldRef> = v.fields.iter().map(|b| b.field.clone()).collect();
    values_in(ctx, &fields, filters)
}

/// Each of `fields` evaluated in the visual's outer filter context (`filters`) without
/// grouping (a column is blank): the grand totals, and the values a whole-visual conditional
/// format (a title, a background) reads.
pub(super) fn values_in(ctx: &mut Ctx<'_>, fields: &[FieldRef], filters: &[Item]) -> Result<Vec<Out>, Fail> {
    let mut eng = Engine::new(ctx);
    let mut ignored = Vec::new();
    let base = eng.outer_ctx(filters, &mut ignored)?;
    let env = Env::new();
    let scope = Scope {
        fc: &base,
        rc: &[],
        env: &env,
        depth: 0,
        mdepth: 0,
    };
    let mut out = Vec::with_capacity(fields.len());
    for field in fields {
        let cell = match field {
            FieldRef::Column { .. } => Out::Blank,
            FieldRef::Measure { table, measure } => {
                let Some(def) = eng
                    .measure_def(Some(table), measure)
                    .or_else(|| eng.measure_def(None, measure))
                else {
                    return unsupported(format!("unknown measure [{measure}]"));
                };
                eng.eval_measure(&def, &scope)?.out()
            }
            FieldRef::Aggregation {
                table,
                column,
                function,
            } => {
                let (Some(agg), Some(c)) = (agg_of_pbir(function), eng.col_id(table, column)) else {
                    return unsupported(format!("aggregation `{function}` of '{table}'[{column}]"));
                };
                eng.aggregate(agg, c, &base)?.out()
            }
            FieldRef::Other { .. } => return unsupported("a hierarchy or expression field"),
        };
        out.push(cell);
    }
    Ok(out)
}

#[path = "preview_eval_struct.rs"]
pub(crate) mod structural;

#[path = "preview_eval_rls.rs"]
pub(super) mod rls;

#[path = "preview_eval_rows.rs"]
pub(super) mod rows_calc;

#[path = "preview_eval_tables.rs"]
mod tables;

#[path = "preview_eval_info.rs"]
mod info;

#[path = "preview_eval_mem.rs"]
mod mem;
#[path = "preview_eval_topn.rs"]
mod topn;

#[path = "dax_query.rs"]
pub(crate) mod query;

#[path = "preview_eval_nameof.rs"]
mod nameof;

#[path = "preview_eval_text.rs"]
mod text;

#[path = "preview_eval_math.rs"]
mod math;

#[path = "preview_eval_calc.rs"]
mod calc;

#[path = "preview_eval_more.rs"]
mod more;

#[path = "preview_eval_window.rs"]
mod window;

#[path = "preview_eval_pick.rs"]
mod pick;

#[path = "preview_eval_calendar.rs"]
mod calendar_ti;

#[path = "preview_eval_storage.rs"]
pub(super) mod storage;

#[path = "preview_eval_pred.rs"]
mod pred;

#[path = "preview_eval_rels.rs"]
pub(super) mod rels;

#[cfg(test)]
#[path = "preview_eval_rels_tests.rs"]
mod rels_tests;

#[cfg(test)]
#[path = "preview_eval_storage_tests.rs"]
mod storage_tests;

#[cfg(test)]
#[path = "preview_eval_storage_fuzz.rs"]
mod storage_fuzz;

#[cfg(test)]
#[path = "preview_eval_bigdim_tests.rs"]
mod bigdim_tests;

#[cfg(test)]
#[path = "preview_eval_aggs_tests.rs"]
mod aggs_tests;

#[cfg(test)]
#[path = "preview_eval_mem_tests.rs"]
mod mem_tests;
#[cfg(test)]
#[path = "preview_eval_topn_tests.rs"]
mod topn_tests;

#[cfg(test)]
#[path = "preview_eval_iterx_tests.rs"]
mod iterx_tests;

#[cfg(test)]
#[path = "preview_eval_tests.rs"]
mod tests;
