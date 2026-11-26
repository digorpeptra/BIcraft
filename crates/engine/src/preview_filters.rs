//! Report, page and visual filters and slicer selections for the data preview ([`super`]).
//!
//! [`collect`] gathers, for one visual, every filter Power BI Desktop applies to its query:
//! - the report's, the page's and the visual's own `filterConfig` filters that carry a condition;
//! - the selection of every other slicer of the same page (hidden ones too: a slicer synced to a
//!   page is a slicer of that page), unless the page's `visualInteractions` say that slicer does
//!   not filter this visual (`NoFilter`). A slicer never filters itself.
//!
//! Each filter is a semantic-query `{Version, From, Where}` (the public `semanticQuery` schema):
//! the `Where` conditions are AND-ed. Conditions are parsed into [`Cond`] trees over columns,
//! measures and aggregations: `In` (values or tuples), `Not`, `And`, `Or`, `Comparison`,
//! `Between`, `Contains`, `StartsWith`, `EndsWith`; relative dates (`Now`, `DateAdd`, `DateSpan`)
//! are resolved against today's date. A top N filter (`In` over a subquery with `Top` and
//! `OrderBy`) becomes a [`TopN`]. Anything else is not applied and named in `ignored`.
//!
//! The evaluator ([`super::general`]) turns column conditions into filter-context filters,
//! computes top N values, and applies measure and aggregation conditions to the visual's rows.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use bicraft_report::{Page, Report, Visual};
use serde_json::Value;

use crate::visual_props::{Lit, parse_lit};

/// Most filter conditions applied to one visual.
const MAX_ITEMS: usize = 200;
/// Most values of one `In` condition.
const MAX_VALUES: usize = 100_000;
/// Deepest condition nesting parsed.
const MAX_DEPTH: usize = 32;
/// Largest N of a top N filter.
const MAX_TOP: usize = 100_000;

/// A model object a condition reads.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Operand {
    Col {
        table: String,
        column: String,
    },
    Measure {
        table: String,
        name: String,
    },
    /// `function`: the PBIR aggregation name (`Sum`, `Avg`…).
    Agg {
        table: String,
        column: String,
        function: String,
    },
}

/// A literal value of a condition. Dates are days since 1899-12-30.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Val {
    Null,
    Num(f64),
    Text(String),
    Bool(bool),
    Date(f64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CmpOp {
    Eq,
    Gt,
    Ge,
    Lt,
    Le,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TextOp {
    Contains,
    StartsWith,
    EndsWith,
}

/// A filter condition.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Cond {
    /// The operands' values (one per operand) are one of `rows`.
    In {
        exprs: Vec<Operand>,
        rows: Vec<Vec<Val>>,
    },
    Cmp {
        op: CmpOp,
        left: Operand,
        right: Val,
    },
    Text {
        op: TextOp,
        left: Operand,
        right: String,
    },
    Not(Box<Cond>),
    And(Box<Cond>, Box<Cond>),
    Or(Box<Cond>, Box<Cond>),
}

impl Cond {
    /// Every operand the condition reads, in order, without duplicates.
    pub(super) fn operands(&self) -> Vec<&Operand> {
        let mut out: Vec<&Operand> = Vec::new();
        self.walk(&mut out);
        out
    }

    fn walk<'a>(&'a self, out: &mut Vec<&'a Operand>) {
        let mut push = |o: &'a Operand| {
            if !out.contains(&o) {
                out.push(o);
            }
        };
        match self {
            Cond::In { exprs, .. } => exprs.iter().for_each(push),
            Cond::Cmp { left, .. } | Cond::Text { left, .. } => push(left),
            Cond::Not(c) => c.walk(out),
            Cond::And(a, b) | Cond::Or(a, b) => {
                a.walk(out);
                b.walk(out);
            }
        }
    }
}

/// `Top N of column by measure`.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct TopN {
    pub table: String,
    pub column: String,
    pub n: usize,
    pub by: Operand,
    /// Top (descending) or bottom.
    pub desc: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Item {
    Cond(Cond),
    TopN(TopN),
}

/// The filters that apply to one visual.
#[derive(Debug, Clone, Default)]
pub(super) struct Applied {
    pub items: Vec<Item>,
    /// Why some filters are not applied (empty: all are).
    pub ignored: Vec<String>,
    /// Hash of everything the filters depend on (for the result cache).
    pub hash: u64,
}

/// A slicer visual (classic, list, tile, text…).
pub(crate) fn is_slicer(v: &Visual) -> bool {
    v.visual_type
        .as_deref()
        .is_some_and(|t| t.to_ascii_lowercase().contains("slicer"))
}

/// The selection a slicer stores: `objects.general[*].properties.filter.filter`.
pub(crate) fn slicer_filter(v: &Visual) -> Option<&Value> {
    v.visual_extra
        .get("objects")?
        .get("general")?
        .as_array()?
        .iter()
        .find_map(|e| e.get("properties")?.get("filter")?.get("filter"))
}

/// The search a slicer keeps for its own items: `objects.general[*].properties.selfFilter.filter`.
pub(crate) fn slicer_self_filter(v: &Visual) -> Option<&Value> {
    v.visual_extra
        .get("objects")?
        .get("general")?
        .as_array()?
        .iter()
        .find_map(|e| e.get("properties")?.get("selfFilter")?.get("filter"))
}

fn val_json(v: &Val) -> Value {
    match v {
        Val::Null => Value::Null,
        Val::Num(x) => super::num_json(*x),
        Val::Text(t) => Value::String(t.clone()),
        Val::Bool(b) => Value::Bool(*b),
        Val::Date(x) => super::ole_to_iso(*x).map_or(Value::Null, Value::String),
    }
}

/// Most selected values listed for a slicer.
const MAX_SELECTED: usize = 1000;

/// A slicer's selection for the viewer: `{selected: [values]}` (`inverted: true` when they are
/// the excluded values), `{range: {from?, to?}}` or `{search, match}` (a text search), plus the
/// slicer's own search (`selfFilter`) as `{search, match}`; `None` when nothing is selected or the
/// selection has another shape.
pub(crate) fn selection_json(v: &Visual) -> Option<Value> {
    let mut out = slicer_filter(v).and_then(condition_json);
    if let Some(search) = slicer_self_filter(v)
        .and_then(condition_json)
        .filter(|s| s.get("search").is_some())
    {
        let o = out.get_or_insert_with(|| Value::Object(serde_json::Map::new()));
        if let (Some(o), Some(s)) = (o.as_object_mut(), search.as_object())
            && !o.contains_key("search")
        {
            o.extend(s.clone());
        }
    }
    out
}

/// One slicer filter as viewer JSON (see [`selection_json`]).
fn condition_json(f: &Value) -> Option<Value> {
    let mut p = Parser {
        now: today(),
        relative: false,
        fine: false,
    };
    let items = p.filter(f).ok()?;
    let [Item::Cond(c)] = items.as_slice() else {
        return None;
    };
    let list = |rows: &[Vec<Val>]| -> Value {
        Value::Array(
            rows.iter()
                .take(MAX_SELECTED)
                .filter_map(|r| r.first().map(val_json))
                .collect(),
        )
    };
    let mut o = serde_json::Map::new();
    match c {
        Cond::In { rows, .. } => {
            o.insert("selected".into(), list(rows));
        }
        Cond::Not(inner) => match inner.as_ref() {
            Cond::In { rows, .. } => {
                o.insert("selected".into(), list(rows));
                o.insert("inverted".into(), Value::Bool(true));
            }
            _ => return None,
        },
        Cond::Cmp {
            op: CmpOp::Eq, right, ..
        } => {
            o.insert("selected".into(), Value::Array(vec![val_json(right)]));
        }
        Cond::Text { op, right, .. } => {
            // a text slicer's search (or a search typed in a list slicer)
            let kind = match op {
                TextOp::Contains => "contains",
                TextOp::StartsWith => "starts_with",
                TextOp::EndsWith => "ends_with",
            };
            o.insert("search".into(), Value::String(right.chars().take(1000).collect()));
            o.insert("match".into(), Value::String(kind.into()));
        }
        Cond::Cmp { .. } | Cond::And(..) => {
            let mut range = serde_json::Map::new();
            // Desktop shows an exclusive date bound as the day inside the range (`< 2025-10-01`
            // displays 30/09/2025); an exclusive number bound is flagged.
            let mut add = |x: &Cond| match x {
                Cond::Cmp { op, right, .. }
                    if matches!(op, CmpOp::Ge | CmpOp::Gt | CmpOp::Le | CmpOp::Lt) =>
                {
                    let from = matches!(op, CmpOp::Ge | CmpOp::Gt);
                    let strict = matches!(op, CmpOp::Gt | CmpOp::Lt);
                    let value = match right {
                        Val::Date(d) if strict => {
                            let day = if from {
                                d.floor() + 1.0
                            } else if d.fract() > 0.0 {
                                d.floor()
                            } else {
                                d - 1.0
                            };
                            val_json(&Val::Date(day))
                        }
                        other => {
                            if strict {
                                let flag = if from { "from_exclusive" } else { "to_exclusive" };
                                range.insert(flag.into(), Value::Bool(true));
                            }
                            val_json(other)
                        }
                    };
                    range.insert((if from { "from" } else { "to" }).into(), value);
                    true
                }
                _ => false,
            };
            let ok = match c {
                Cond::And(a, b) => add(a) && add(b),
                other => add(other),
            };
            if !ok {
                return None;
            }
            o.insert("range".into(), Value::Object(range));
        }
        _ => return None,
    }
    Some(Value::Object(o))
}

/// Whether the page's `visualInteractions` turn filtering from `source` to `target` off.
fn no_filter(page: &Page, source: &str, target: &str) -> bool {
    page.extra
        .get("visualInteractions")
        .and_then(Value::as_array)
        .is_some_and(|list| {
            list.iter().any(|i| {
                i.get("source").and_then(Value::as_str) == Some(source)
                    && i.get("target").and_then(Value::as_str) == Some(target)
                    && i.get("type").and_then(Value::as_str) == Some("NoFilter")
            })
        })
}

/// The conditions of a `filterConfig` (containers without a condition are skipped).
fn config_filters(config: Option<&Value>) -> Vec<&Value> {
    config
        .and_then(|c| c.get("filters"))
        .and_then(Value::as_array)
        .map(|fs| fs.iter().filter_map(|f| f.get("filter")).collect())
        .unwrap_or_default()
}

/// Today as days since 1899-12-30 (UTC).
pub(crate) fn today() -> f64 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    (secs / 86_400) as f64 + 25_569.0
}

/// Now as days since 1899-12-30 (UTC; the fraction is the time of day).
pub(crate) fn now_utc() -> f64 {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let ms = u64::try_from(ms).unwrap_or(u64::MAX);
    (ms / 1000) as f64 / 86_400.0 + 25_569.0
}

/// The process's local offset ([`crate::clock`]: UTC unless the app or the server set a time
/// zone or an offset) at an instant given in days since 1899-12-30, in days.
pub(crate) fn offset_days_at(day: f64) -> f64 {
    let ms = (day - 25_569.0) * 86_400_000.0;
    // `as` saturates: a value out of range is an instant no zone has an answer for
    let ms = if ms.is_finite() { ms as i64 } else { 0 };
    f64::from(crate::clock::local_clock().offset_at(ms)) / 1440.0
}

/// The session's "now" (`as_of`, else the clock in the process's local time zone): what
/// `TODAY()`/`NOW()` and relative date filters see.
pub(crate) fn clock(s: &crate::Session) -> f64 {
    s.as_of_now().unwrap_or_else(|| {
        let utc = now_utc();
        utc + offset_days_at(utc)
    })
}

/// Local time minus UTC, in days, at the session's "now" (`as_of` is local time, read in the
/// process's time zone): what separates `UTCNOW()` from `NOW()`.
pub(crate) fn utc_offset(s: &crate::Session) -> f64 {
    offset_days_at(s.as_of_now().unwrap_or_else(now_utc))
}

/// Parses an `as_of` parameter: `YYYY-MM-DD`, optionally followed by `THH:MM[:SS]` (or a
/// space), into days since 1899-12-30.
pub(crate) fn parse_as_of(text: &str) -> Result<f64, String> {
    let bad = || {
        format!("`as_of` must be a date such as \"2026-09-18\" or \"2026-09-18T08:30:00\", got \"{text}\"")
    };
    let t = text.trim();
    let (date, time) = match t.split_once(['T', ' ']) {
        Some((d, tm)) => (d, Some(tm)),
        None => (t, None),
    };
    let mut parts = date.split('-');
    let num = |p: Option<&str>| p.and_then(|x| x.parse::<i64>().ok());
    let (Some(y), Some(m), Some(d), None) = (
        num(parts.next()),
        num(parts.next()),
        num(parts.next()),
        parts.next(),
    ) else {
        return Err(bad());
    };
    let dim = match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1..=12 => 31,
        _ => return Err(bad()),
    };
    if !(1..=9999).contains(&y) || !(1..=dim).contains(&d) {
        return Err(bad());
    }
    let mut frac = 0.0;
    if let Some(tm) = time {
        let tm = tm.trim_end_matches('Z');
        let f: Vec<&str> = tm.split(':').collect();
        let get = |i: usize| f.get(i).map(|x| x.parse::<f64>().ok());
        let (h, mi, s) = match (f.len(), get(0), get(1), get(2)) {
            (2, Some(Some(h)), Some(Some(mi)), _) => (h, mi, 0.0),
            (3, Some(Some(h)), Some(Some(mi)), Some(Some(s))) => (h, mi, s),
            _ => return Err(bad()),
        };
        if !(0.0..24.0).contains(&h) || !(0.0..60.0).contains(&mi) || !(0.0..60.0).contains(&s) {
            return Err(bad());
        }
        frac = (h * 3600.0 + mi * 60.0 + s) / 86_400.0;
    }
    let days = bicraft_locale::datefmt::days_from_civil(y, m.unsigned_abs() as u32, d.unsigned_abs() as u32);
    Ok(days as f64 + 25_569.0 + frac)
}

/// Every filter that applies to visual `v` of `page` (see the module docs). `now`: days since
/// 1899-12-30, for relative dates.
pub(super) fn collect(report: &Report, page: &Page, v: &Visual, now: f64) -> Applied {
    let mut sources: Vec<(String, &Value)> = Vec::new();
    for f in config_filters(report.filter_config.as_ref()) {
        sources.push(("report filter".into(), f));
    }
    for f in config_filters(page.filter_config.as_ref()) {
        sources.push(("page filter".into(), f));
    }
    for f in config_filters(v.filter_config.as_ref()) {
        sources.push(("visual filter".into(), f));
    }
    if is_slicer(v)
        && let Some(f) = slicer_self_filter(v)
    {
        // the search typed in the slicer narrows its own items
        sources.push(("slicer search".into(), f));
    }
    for s in &page.visuals {
        if s.name == v.name || !is_slicer(s) || no_filter(page, &s.name, &v.name) {
            continue;
        }
        if let Some(f) = slicer_filter(s) {
            sources.push(("slicer selection".into(), f));
        }
    }
    let mut out = Applied::default();
    let mut relative = false;
    let mut fine = false;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for (what, f) in &sources {
        f.to_string().hash(&mut h);
        let mut p = Parser {
            now,
            relative: false,
            fine: false,
        };
        match p.filter(f) {
            Ok(items) => {
                for it in items {
                    if out.items.len() >= MAX_ITEMS {
                        out.ignored
                            .push(format!("more than {MAX_ITEMS} filter conditions"));
                        break;
                    }
                    out.items.push(it);
                }
            }
            Err(e) => out.ignored.push(format!("{what}: {e}")),
        }
        relative |= p.relative;
        fine |= p.fine;
    }
    if relative {
        // results that depend on today's date are recomputed the next day
        (now as i64).hash(&mut h);
        if fine {
            // relative time: recomputed every minute
            ((now * 1440.0) as i64).hash(&mut h);
        }
    }
    out.ignored.dedup();
    out.hash = h.finish();
    out
}

struct Parser {
    now: f64,
    relative: bool,
    /// A bound finer than a day (an instant, minutes, hours): the result changes within the day.
    fine: bool,
}

type Aliases = HashMap<String, String>;

fn aliases(query: &Value) -> Aliases {
    let mut out = Aliases::new();
    for f in query.get("From").and_then(Value::as_array).into_iter().flatten() {
        if let (Some(n), Some(e)) = (
            f.get("Name").and_then(Value::as_str),
            f.get("Entity").and_then(Value::as_str),
        ) {
            out.insert(n.to_string(), e.to_string());
        }
    }
    out
}

/// The only key of an object.
fn only(v: &Value) -> Option<(&str, &Value)> {
    let o = v.as_object()?;
    if o.len() != 1 {
        return None;
    }
    o.iter().next().map(|(k, x)| (k.as_str(), x))
}

impl Parser {
    /// The items of one `{Version, From, Where}` filter.
    fn filter(&mut self, f: &Value) -> Result<Vec<Item>, String> {
        let al = aliases(f);
        let mut out = Vec::new();
        let wheres = f
            .get("Where")
            .and_then(Value::as_array)
            .ok_or("a filter without `Where`")?;
        for w in wheres {
            let c = w.get("Condition").ok_or("a `Where` entry without `Condition`")?;
            if let Some(t) = self.top_n(c, f, &al)? {
                out.push(Item::TopN(t));
            } else {
                out.push(Item::Cond(self.cond(c, &al, 0)?));
            }
        }
        Ok(out)
    }

    /// `In {Expressions: [col], Table: {SourceRef: {Source: <subquery>}}}`.
    fn top_n(&mut self, c: &Value, f: &Value, al: &Aliases) -> Result<Option<TopN>, String> {
        let Some(("In", inn)) = only(c) else {
            return Ok(None);
        };
        let Some(table) = inn.get("Table") else {
            return Ok(None);
        };
        let src = table
            .get("SourceRef")
            .and_then(|s| s.get("Source"))
            .and_then(Value::as_str)
            .ok_or("an `In` over an unsupported table")?;
        let sub = f
            .get("From")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|x| x.get("Name").and_then(Value::as_str) == Some(src))
            .and_then(|x| x.get("Expression"))
            .and_then(|e| e.get("Subquery"))
            .and_then(|s| s.get("Query"))
            .ok_or("an `In` over a table that is not a top N subquery")?;
        let exprs = inn
            .get("Expressions")
            .and_then(Value::as_array)
            .ok_or("`In` without expressions")?;
        let [e] = exprs.as_slice() else {
            return Err("a top N filter over several columns".into());
        };
        let Operand::Col { table, column } = operand(e, al)? else {
            return Err("a top N filter on something other than a column".into());
        };
        let n = sub
            .get("Top")
            .and_then(Value::as_u64)
            .ok_or("a subquery without `Top`")?;
        let n = usize::try_from(n).unwrap_or(MAX_TOP).min(MAX_TOP);
        let sub_al = aliases(sub);
        let order = sub
            .get("OrderBy")
            .and_then(Value::as_array)
            .and_then(|o| o.first())
            .ok_or("a top N subquery without `OrderBy`")?;
        let by = operand(
            order.get("Expression").ok_or("`OrderBy` without an expression")?,
            &sub_al,
        )?;
        if matches!(by, Operand::Col { .. }) {
            return Err("a top N filter ordered by a column".into());
        }
        let desc = order.get("Direction").and_then(Value::as_u64) != Some(1);
        Ok(Some(TopN {
            table,
            column,
            n,
            by,
            desc,
        }))
    }

    fn cond(&mut self, c: &Value, al: &Aliases, depth: usize) -> Result<Cond, String> {
        if depth > MAX_DEPTH {
            return Err("a condition nests too deeply".into());
        }
        let d = depth.saturating_add(1);
        let (kind, body) = only(c).ok_or("a malformed condition")?;
        let get = |k: &str| body.get(k).ok_or_else(|| format!("`{kind}` without `{k}`"));
        match kind {
            "And" | "Or" => {
                let a = Box::new(self.cond(get("Left")?, al, d)?);
                let b = Box::new(self.cond(get("Right")?, al, d)?);
                Ok(if kind == "And" {
                    Cond::And(a, b)
                } else {
                    Cond::Or(a, b)
                })
            }
            "Not" => Ok(Cond::Not(Box::new(self.cond(get("Expression")?, al, d)?))),
            "In" => {
                let exprs = get("Expressions")?
                    .as_array()
                    .ok_or("`In` expressions are not a list")?
                    .iter()
                    .map(|e| operand(e, al))
                    .collect::<Result<Vec<_>, _>>()?;
                let values = get("Values")?.as_array().ok_or("`In` without values")?;
                if values.len() > MAX_VALUES {
                    return Err(format!("an `In` with more than {MAX_VALUES} values"));
                }
                let mut rows = Vec::with_capacity(values.len());
                for r in values {
                    let r = r.as_array().ok_or("an `In` value row is not a list")?;
                    if r.len() != exprs.len() {
                        return Err("an `In` value row does not match its expressions".into());
                    }
                    rows.push(r.iter().map(|x| self.value(x)).collect::<Result<Vec<_>, _>>()?);
                }
                Ok(Cond::In { exprs, rows })
            }
            "Comparison" => {
                let op = match get("ComparisonKind")?.as_u64() {
                    Some(0) => CmpOp::Eq,
                    Some(1) => CmpOp::Gt,
                    Some(2) => CmpOp::Ge,
                    Some(3) => CmpOp::Lt,
                    Some(4) => CmpOp::Le,
                    _ => return Err("an unknown comparison kind".into()),
                };
                let left = operand(get("Left")?, al)?;
                self.compare(op, left, get("Right")?)
            }
            "Between" => {
                let left = operand(get("Expression")?, al)?;
                let lo = self.compare(CmpOp::Ge, left.clone(), get("LowerBound")?)?;
                let hi = self.compare(CmpOp::Le, left, get("UpperBound")?)?;
                Ok(Cond::And(Box::new(lo), Box::new(hi)))
            }
            "Contains" | "StartsWith" | "EndsWith" => {
                let op = match kind {
                    "Contains" => TextOp::Contains,
                    "StartsWith" => TextOp::StartsWith,
                    _ => TextOp::EndsWith,
                };
                let left = operand(get("Left")?, al)?;
                let right = match self.value(get("Right")?)? {
                    Val::Text(t) => t,
                    _ => return Err(format!("`{kind}` of something other than text")),
                };
                Ok(Cond::Text { op, left, right })
            }
            other => Err(format!("`{other}` conditions are not applied by the preview")),
        }
    }

    /// `left op right`, where `right` is a literal or a relative date.
    fn compare(&mut self, op: CmpOp, left: Operand, right: &Value) -> Result<Cond, String> {
        if let Some(("Literal", _)) = only(right) {
            return Ok(Cond::Cmp {
                op,
                left,
                right: self.value(right)?,
            });
        }
        // a relative date: [start, end) of a date span, or an instant
        let (start, end) = self.date(right, 0)?;
        self.relative = true;
        self.fine |= end - start < 1.0;
        let cmp = |op, x: f64| Cond::Cmp {
            op,
            left: left.clone(),
            right: Val::Date(x),
        };
        Ok(match op {
            CmpOp::Ge => cmp(CmpOp::Ge, start),
            CmpOp::Lt => cmp(CmpOp::Lt, start),
            CmpOp::Gt if end > start => cmp(CmpOp::Ge, end),
            CmpOp::Gt => cmp(CmpOp::Gt, start),
            CmpOp::Le if end > start => cmp(CmpOp::Lt, end),
            CmpOp::Le => cmp(CmpOp::Le, start),
            CmpOp::Eq if end > start => {
                Cond::And(Box::new(cmp(CmpOp::Ge, start)), Box::new(cmp(CmpOp::Lt, end)))
            }
            CmpOp::Eq => cmp(CmpOp::Eq, start),
        })
    }

    /// A date expression: `(start, end)` days (`end == start` for an instant).
    fn date(&self, e: &Value, depth: usize) -> Result<(f64, f64), String> {
        if depth > MAX_DEPTH {
            return Err("a date expression nests too deeply".into());
        }
        let (kind, body) = only(e).ok_or("an unsupported value expression")?;
        let unit = || {
            body.get("TimeUnit")
                .and_then(Value::as_u64)
                .ok_or("a date expression without a time unit")
        };
        match kind {
            "Now" => Ok((self.now, self.now)),
            "Literal" => match self.value(e)? {
                Val::Date(x) => Ok((x, x)),
                _ => Err("a date comparison with a value that is not a date".into()),
            },
            "DateAdd" => {
                let (x, _) = self.date(
                    body.get("Expression").ok_or("`DateAdd` without an expression")?,
                    depth + 1,
                )?;
                let amount = body
                    .get("Amount")
                    .and_then(Value::as_i64)
                    .ok_or("`DateAdd` without an amount")?;
                let x = add_units(x, amount.clamp(-100_000, 100_000), unit()?)?;
                Ok((x, x))
            }
            "DateSpan" => {
                let (x, _) = self.date(
                    body.get("Expression").ok_or("`DateSpan` without an expression")?,
                    depth + 1,
                )?;
                span(x, unit()?)
            }
            other => Err(format!("`{other}` values are not applied by the preview")),
        }
    }

    fn value(&self, v: &Value) -> Result<Val, String> {
        let text = v
            .get("Literal")
            .and_then(|l| l.get("Value"))
            .and_then(Value::as_str)
            .ok_or("a value that is not a literal")?;
        Ok(match parse_lit(text) {
            Lit::Null => Val::Null,
            Lit::Bool(b) => Val::Bool(b),
            Lit::Num(n, _) => Val::Num(n),
            Lit::Text(t) => Val::Text(t),
            Lit::Date(d) => Val::Date(parse_date(&d).ok_or_else(|| format!("an unreadable date `{d}`"))?),
            Lit::Other(o) => return Err(format!("an unsupported literal `{o}`")),
        })
    }
}

/// A column, measure or aggregation reference.
fn operand(e: &Value, al: &Aliases) -> Result<Operand, String> {
    let (kind, body) = only(e).ok_or("an unsupported expression")?;
    let entity = |x: &Value| -> Result<String, String> {
        let src = x
            .get("Expression")
            .and_then(|s| s.get("SourceRef"))
            .ok_or("a reference without a source")?;
        if let Some(e) = src.get("Entity").and_then(Value::as_str) {
            return Ok(e.to_string());
        }
        let alias = src
            .get("Source")
            .and_then(Value::as_str)
            .ok_or("a reference without a source")?;
        al.get(alias)
            .cloned()
            .ok_or_else(|| format!("unknown source `{alias}`"))
    };
    let prop = |x: &Value| -> Result<String, String> {
        x.get("Property")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| "a reference without a property".to_string())
    };
    match kind {
        "Column" => Ok(Operand::Col {
            table: entity(body)?,
            column: prop(body)?,
        }),
        "Measure" => Ok(Operand::Measure {
            table: entity(body)?,
            name: prop(body)?,
        }),
        "Aggregation" => {
            let col = body
                .get("Expression")
                .and_then(|x| x.get("Column"))
                .ok_or("an aggregation of something other than a column")?;
            let function = match body.get("Function").and_then(Value::as_u64) {
                Some(0) => "Sum",
                Some(1) => "Avg",
                Some(2) => "Count",
                Some(3) => "Min",
                Some(4) => "Max",
                Some(5) => "CountNonNull",
                Some(6) => "Median",
                Some(7) => "StandardDeviation",
                Some(8) => "Variance",
                _ => return Err("an unknown aggregation function".into()),
            };
            Ok(Operand::Agg {
                table: entity(col)?,
                column: prop(col)?,
                function: function.into(),
            })
        }
        "HierarchyLevel" => Err("filters on hierarchy levels are not applied by the preview".into()),
        other => Err(format!("`{other}` expressions are not applied by the preview")),
    }
}

// ---------------------------------------------------------------------------------------------
// dates
// ---------------------------------------------------------------------------------------------

/// The days a relative-date slicer covers, as Desktop shows them under its inputs (today
/// included): `range` last / next / this, `duration` periods, `period` days, weeks, months,
/// years or their calendar variants ("calendar months" or "calendarmonths"). Day numbers, first
/// and last day included; `None` for minutes / hours or unknown settings.
pub(crate) fn relative_days(range: &str, duration: f64, period: &str, today: f64) -> Option<(f64, f64)> {
    let t = today.floor();
    if !t.is_finite() || t.abs() > 3_000_000.0 {
        return None;
    }
    let n = if duration.is_finite() {
        duration.round().clamp(1.0, 10_000.0) as i64
    } else {
        1
    };
    let p: String = period
        .chars()
        .filter(|c| !c.is_whitespace())
        .take(40)
        .collect::<String>()
        .to_ascii_lowercase();
    let (calendar, unit) = match p.strip_prefix("calendar") {
        Some(u) => (true, u.to_string()),
        None => (false, p),
    };
    // the calendar period holding day `x`, moved by `k` periods: (first day, first day after)
    let span = |x: f64, k: i64| -> Option<(f64, f64)> {
        let (y, m, _) = civil(x as i64);
        Some(match unit.as_str() {
            "days" => (x + k as f64, x + k as f64 + 1.0),
            "weeks" => {
                // weeks start on Sunday (day 1 = 1899-12-31 was a Sunday)
                let s = x - (x - 1.0).rem_euclid(7.0) + 7.0 * k as f64;
                (s, s + 7.0)
            }
            "months" => {
                let s = add_months(serial(y, m, 1) as f64, k);
                (s, add_months(s, 1))
            }
            "years" => {
                let s = add_months(serial(y, 1, 1) as f64, k.saturating_mul(12));
                (s, add_months(s, 12))
            }
            _ => return None,
        })
    };
    let back = |x: f64, k: i64| -> Option<f64> {
        Some(match unit.as_str() {
            "days" => x + k as f64,
            "weeks" => x + 7.0 * k as f64,
            "months" => add_months(x, k),
            "years" => add_months(x, k.saturating_mul(12)),
            _ => return None,
        })
    };
    match (range.to_ascii_lowercase().as_str(), calendar || unit == "days") {
        ("this", _) => span(t, 0).map(|(a, b)| (a, b - 1.0)),
        // calendar periods: the n whole periods before (after) the current one
        ("last", true) if unit != "days" => Some((span(t, -n)?.0, span(t, 0)?.0 - 1.0)),
        ("next", true) if unit != "days" => Some((span(t, 1)?.0, span(t, n + 1)?.0 - 1.0)),
        ("last", _) => Some((back(t, -n)? + 1.0, t)),
        ("next", _) => Some((t, back(t, n)? - 1.0)),
        _ => None,
    }
}

/// `YYYY-MM-DD` of a day number.
pub(crate) fn iso_day(x: f64) -> String {
    if !x.is_finite() || x.abs() > 100_000_000.0 {
        return String::new();
    }
    let (y, m, d) = civil(x.floor() as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Day number (days since 1899-12-30) of a civil date.
pub(super) fn serial(y: i64, m: i64, d: i64) -> i64 {
    bicraft_locale::calendar::serial_from_civil(y, m, d)
}

/// (year, month, day) of a day number.
fn civil(x: i64) -> (i64, i64, i64) {
    bicraft_locale::calendar::civil_from_serial(x)
}

/// `YYYY-MM-DD[Thh:mm[:ss[.fff]]]` as a day number.
fn parse_date(s: &str) -> Option<f64> {
    let num = |r: std::ops::Range<usize>| s.get(r).and_then(|x| x.parse::<i64>().ok());
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || s.get(4..5) != Some("-") {
        return None;
    }
    let mut x = serial(y, m, d) as f64;
    if s.len() >= 16 {
        let (hh, mm) = (num(11..13)?, num(14..16)?);
        let ss = if s.len() >= 19 { num(17..19)? } else { 0 };
        x += (hh * 3600 + mm * 60 + ss) as f64 / 86_400.0;
    }
    Some(x)
}

fn add_months(x: f64, k: i64) -> f64 {
    let day = x.floor();
    let frac = x - day;
    bicraft_locale::calendar::add_months_clip(day as i64, k) as f64 + frac
}

/// `x` moved by `amount` time units (`TimeUnit`: 0 day, 1 week, 2 month, 3 year, 4 decade,
/// 5 second, 6 minute, 7 hour).
fn add_units(x: f64, amount: i64, unit: u64) -> Result<f64, String> {
    let a = amount as f64;
    Ok(match unit {
        0 => x + a,
        1 => x + a * 7.0,
        2 => add_months(x, amount),
        3 => add_months(x, amount.saturating_mul(12)),
        4 => add_months(x, amount.saturating_mul(120)),
        5 => x + a / 86_400.0,
        6 => x + a / 1440.0,
        7 => x + a / 24.0,
        _ => return Err("an unknown time unit".into()),
    })
}

/// The span of `unit` containing `x`: `[start, end)`. Weeks start on Sunday.
fn span(x: f64, unit: u64) -> Result<(f64, f64), String> {
    let day = x.floor();
    let (y, m, _) = civil(day as i64);
    Ok(match unit {
        0 => (day, day + 1.0),
        1 => {
            // 1899-12-31 (day 1) was a Sunday
            let start = day - (day - 1.0).rem_euclid(7.0);
            (start, start + 7.0)
        }
        2 => (serial(y, m, 1) as f64, serial(y, m + 1, 1) as f64),
        3 => (serial(y, 1, 1) as f64, serial(y + 1, 1, 1) as f64),
        4 => {
            let d0 = y - y.rem_euclid(10);
            (serial(d0, 1, 1) as f64, serial(d0 + 10, 1, 1) as f64)
        }
        5 => {
            let s = (x * 86_400.0).floor() / 86_400.0;
            (s, s + 1.0 / 86_400.0)
        }
        6 => {
            let s = (x * 1440.0).floor() / 1440.0;
            (s, s + 1.0 / 1440.0)
        }
        7 => {
            let s = (x * 24.0).floor() / 24.0;
            (s, s + 1.0 / 24.0)
        }
        _ => return Err("an unknown time unit".into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn col(alias: &str, p: &str) -> Value {
        json!({"Column": {"Expression": {"SourceRef": {"Source": alias}}, "Property": p}})
    }

    #[test]
    fn dates() {
        assert_eq!(serial(1899, 12, 30), 0);
        assert_eq!(serial(2024, 1, 31), 45322);
        assert_eq!(civil(45322), (2024, 1, 31));
        assert_eq!(add_months(45322.0, 1), serial(2024, 2, 29) as f64);
        assert_eq!(parse_date("2024-01-31T12:00:00"), Some(45322.5));
        assert_eq!(
            span(45322.3, 2),
            Ok((serial(2024, 1, 1) as f64, serial(2024, 2, 1) as f64))
        );
        // 2024-01-31 was a Wednesday: its week starts on Sunday the 28th
        assert_eq!(span(45322.0, 1).map(|s| s.0), Ok(serial(2024, 1, 28) as f64));
    }

    #[test]
    fn conditions_parse() {
        let f = json!({"Version": 2, "From": [{"Name": "s", "Entity": "Sales", "Type": 0}],
        "Where": [
            {"Condition": {"Not": {"Expression": {"In": {"Expressions": [col("s", "Product")],
                "Values": [[{"Literal": {"Value": "'Bike'"}}], [{"Literal": {"Value": "null"}}]]}}}}},
            {"Condition": {"Between": {"Expression": col("s", "Day"),
                "LowerBound": {"DateSpan": {"Expression": {"DateAdd": {"Expression": {"Now": {}}, "Amount": -1, "TimeUnit": 2}}, "TimeUnit": 2}},
                "UpperBound": {"Literal": {"Value": "datetime'2024-01-31T00:00:00'"}}}}}
        ]});
        let mut p = Parser {
            now: serial(2024, 3, 15) as f64,
            relative: false,
            fine: false,
        };
        let items = p.filter(&f).unwrap_or_default();
        assert_eq!(items.len(), 2);
        assert!(p.relative);
        let day = Operand::Col {
            table: "Sales".into(),
            column: "Day".into(),
        };
        assert_eq!(
            items.get(1),
            Some(&Item::Cond(Cond::And(
                Box::new(Cond::Cmp {
                    op: CmpOp::Ge,
                    left: day.clone(),
                    right: Val::Date(serial(2024, 2, 1) as f64)
                }),
                Box::new(Cond::Cmp {
                    op: CmpOp::Le,
                    left: day,
                    right: Val::Date(serial(2024, 1, 31) as f64)
                })
            )))
        );
        // hierarchy levels are reported, not applied
        let h = json!({"Version": 2, "From": [], "Where": [{"Condition": {"In": {"Expressions": [{"HierarchyLevel": {}}],
            "Values": [[{"Literal": {"Value": "1L"}}]]}}}]});
        assert!(p.filter(&h).is_err());
    }

    #[test]
    fn top_n_parses() {
        let f = json!({"Version": 2, "From": [
                {"Name": "subquery", "Expression": {"Subquery": {"Query": {"Version": 2,
                    "From": [{"Name": "p", "Entity": "Product", "Type": 0}],
                    "Select": [{"Column": {"Expression": {"SourceRef": {"Source": "p"}}, "Property": "Product"}, "Name": "field"}],
                    "OrderBy": [{"Direction": 2, "Expression": {"Measure": {"Expression": {"SourceRef": {"Source": "p"}}, "Property": "Total"}}}],
                    "Top": 2}}}, "Type": 2},
                {"Name": "p", "Entity": "Product", "Type": 0}],
            "Where": [{"Condition": {"In": {"Expressions": [col("p", "Product")], "Table": {"SourceRef": {"Source": "subquery"}}}}}]});
        let mut p = Parser {
            now: 0.0,
            relative: false,
            fine: false,
        };
        let items = p.filter(&f).unwrap_or_default();
        assert_eq!(
            items,
            vec![Item::TopN(TopN {
                table: "Product".into(),
                column: "Product".into(),
                n: 2,
                by: Operand::Measure {
                    table: "Product".into(),
                    name: "Total".into()
                },
                desc: true,
            })]
        );
    }
}
