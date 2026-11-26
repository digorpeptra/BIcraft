//! The key influencers visual (`keyDriversVisual`) in the snapshot (`docs/key-influencers.md`):
//! what the visual analyses (its target, explain-by and expand-by fields, the goal it was saved
//! with), and, only when the viewer or `report.render {key_influencers: "estimate"}` asks for it,
//! **BIcraft's own estimate**: a transparent analysis of the session's data through the preview
//! evaluator ([`super::compute`], so filters, slicers and relationships apply as in any visual).
//!
//! Power BI computes this visual with its own models, which BIcraft neither knows nor reproduces:
//! the estimate is never presented as Power BI's result (the renderer badges it).
//!
//! Bounded: at most [`MAX_FIELDS`] explain-by fields, [`FIELD_ROWS`] rows per query, three fields
//! in the segments' joint query, a time budget per visual ([`VISUAL_BUDGET`]) and the snapshot's
//! budget; an estimate is cached only when it is complete.

#[path = "key_influencers_stats.rs"]
pub(crate) mod stats;

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

use bicraft_model::SemanticModel;
use bicraft_report::{FieldBinding, FieldRef, Page, Report, Visual};
use serde_json::{Map, Value, json};

use super::{PreviewLimits, State, compute, data_type_name, filters, hash_of, visual_hash};
use crate::Session;
use stats::{Cell, FieldCells, Goal, Influencer, Key, What};

/// The visual type.
pub(crate) const VISUAL_TYPE: &str = "keyDriversVisual";
/// Explain-by fields analysed (the others are named in `notes`).
pub(crate) const MAX_FIELDS: usize = 12;
/// Rows one query may return (distinct values of a field, or joint cells).
pub(crate) const FIELD_ROWS: usize = 20_000;
/// Fields of the top segments' joint query.
const SEGMENT_FIELDS: usize = 3;
/// Influencers returned.
const MAX_INFLUENCERS: usize = 10;
/// Bars of a field's detail chart.
const MAX_BARS: usize = 12;
/// Classes of a categorical target.
const MAX_CLASSES: usize = 1000;
/// Time one visual's estimate may take.
pub(crate) const VISUAL_BUDGET: Duration = Duration::from_secs(20);

/// Whether a visual is a key influencers visual.
pub(crate) fn is_key_influencers(v: &Visual) -> bool {
    v.visual_type.as_deref() == Some(VISUAL_TYPE)
}

// ---------------------------------------------------------------------------------------------
// what the visual is set to analyse
// ---------------------------------------------------------------------------------------------

/// A PBIR literal's text: `'It''s'` → `It's`, `12D`/`3L` → `12`/`3`, `true` → `true`.
fn literal_text(v: &Value) -> Option<String> {
    let s = v
        .get("expr")
        .and_then(|e| e.get("Literal"))
        .and_then(|l| l.get("Value"))
        .and_then(Value::as_str)?;
    if let Some(inner) = s.strip_prefix('\'').and_then(|t| t.strip_suffix('\'')) {
        return Some(inner.replace("''", "'"));
    }
    let t = s.trim_end_matches(['D', 'L', 'M']);
    Some(if t.parse::<f64>().is_ok() { t } else { s }.to_string())
}

/// The visual's `objects.keyDrivers` properties.
fn options(v: &Visual) -> Map<String, Value> {
    v.visual_extra
        .get("objects")
        .and_then(|o| o.get("keyDrivers"))
        .and_then(Value::as_array)
        .and_then(|a| {
            a.iter()
                .find(|e| e.get("selector").is_none())
                .or_else(|| a.first())
        })
        .and_then(|e| e.get("properties"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

fn option(o: &Map<String, Value>, k: &str) -> Option<String> {
    o.get(k).and_then(literal_text)
}

/// What the visual analyses, from its fields and options.
#[derive(Debug, Clone)]
pub(crate) struct Spec {
    /// Index of the target in `Visual::fields`.
    pub target: Option<usize>,
    pub explain_by: Vec<usize>,
    pub details: Vec<usize>,
    pub categorical: bool,
    pub goal: Goal,
    /// The class a categorical target is analysed for, as saved (`targetValue`).
    pub class: Option<String>,
    pub top_segments: bool,
}

fn is_numeric_type(t: &str) -> bool {
    matches!(t, "int64" | "double" | "decimal")
}

impl Spec {
    pub(crate) fn of(v: &Visual, model: Option<&SemanticModel>) -> Spec {
        let role = |r: &str| -> Vec<usize> {
            v.fields
                .iter()
                .enumerate()
                .filter(|(_, b)| b.role.eq_ignore_ascii_case(r))
                .map(|(i, _)| i)
                .collect()
        };
        let o = options(v);
        let target = role("Target").first().copied();
        let numeric_option = option(&o, "selectedNumericAnalysis");
        let categorical = match target.and_then(|i| v.fields.get(i)).map(|b| &b.field) {
            Some(FieldRef::Column { table, column }) => {
                let numeric = model.is_some_and(|m| is_numeric_type(&data_type_name(m, table, column)));
                !numeric || numeric_option.as_deref() == Some("Categorical")
            }
            _ => false,
        };
        // 0 = increase, 1 = decrease (docs/key-influencers.md)
        let goal = if categorical {
            Goal::Class
        } else if option(&o, "numericTargetSelectedKind").as_deref() == Some("1") {
            Goal::Decrease
        } else {
            Goal::Increase
        };
        Spec {
            target,
            explain_by: role("ExplainBy"),
            details: role("Details"),
            categorical,
            goal,
            class: option(&o, "targetValue").filter(|_| categorical),
            top_segments: option(&o, "selectedAnalysis").as_deref() == Some("topProfiles"),
        }
    }
}

/// The snapshot's `key_influencers` object of a key influencers visual (without the estimate).
pub(crate) fn describe(v: &Visual, model: Option<&SemanticModel>, estimate_on: bool) -> Value {
    let spec = Spec::of(v, model);
    json!({
        "analysis": if spec.top_segments { "top_segments" } else { "key_influencers" },
        "target": spec.target,
        "explain_by": spec.explain_by,
        "details": spec.details,
        "target_kind": if spec.categorical { "categorical" } else { "numeric" },
        "goal": match spec.goal { Goal::Increase => "increase", Goal::Decrease => "decrease", Goal::Class => "class" },
        "class": spec.class,
        "mode": if estimate_on { "estimate" } else { "placeholder" },
    })
}

// ---------------------------------------------------------------------------------------------
// the estimate
// ---------------------------------------------------------------------------------------------

/// The estimates of the report's key influencers visuals, keyed by (page name, visual name).
/// Visuals not reached within the budget get none (they are computed by a later snapshot).
pub(crate) fn report_estimates(
    s: &Session,
    report: &Report,
    pages: usize,
    visuals_per_page: usize,
    limits: &PreviewLimits,
) -> HashMap<(String, String), Value> {
    let mut out = HashMap::new();
    let has_any = report
        .pages
        .iter()
        .take(pages)
        .any(|p| p.visuals.iter().take(visuals_per_page).any(is_key_influencers));
    if !has_any {
        return out;
    }
    let start = Instant::now();
    let mut st = s.preview.acquire(s);
    let (generation, model_hash) = (st.generation, st.model_hash(s));
    let now = limits.now;
    let clock = super::clock_key(s, limits);
    for page in report.pages.iter().take(pages) {
        for v in page
            .visuals
            .iter()
            .take(visuals_per_page)
            .filter(|v| is_key_influencers(v))
        {
            let applied = filters::collect(report, page, v, now);
            let key = hash_of(&[
                b"key_influencers",
                &generation.to_le_bytes(),
                &model_hash.to_le_bytes(),
                &visual_hash(v).to_le_bytes(),
                &applied.hash.to_le_bytes(),
                &clock.to_le_bytes(),
            ]);
            let e = match st.results.get(&key) {
                Some(d) => d.clone(),
                None if start.elapsed() < limits.budget => {
                    // this visual's share: the snapshot's budget left, at most VISUAL_BUDGET
                    let deadline = start.checked_add(limits.budget).unwrap_or_else(Instant::now).min(
                        Instant::now()
                            .checked_add(VISUAL_BUDGET)
                            .unwrap_or_else(Instant::now),
                    );
                    let base = hash_of(&[&key.to_le_bytes(), b"query"]);
                    let r = catch_unwind(AssertUnwindSafe(|| {
                        estimate(s, &mut st, report, page, v, limits, deadline, base)
                    }));
                    match r {
                        Ok(Ok(e)) => {
                            st.results.insert(key, e.clone());
                            e
                        }
                        // over the time budget: not cached, a later snapshot tries again
                        Ok(Err(e)) => e,
                        Err(_) => json!({"status": "error", "reason": "internal error while estimating"}),
                    }
                }
                None => continue,
            };
            out.insert((page.name.clone(), v.name.clone()), e);
        }
    }
    out
}

/// Why there is no estimate.
fn none(status: &str, reason: impl Into<String>) -> Value {
    json!({"status": status, "reason": reason.into()})
}

/// One synthetic grouped query through the preview evaluator: `groups` then `values`. `Err` is
/// the reason it gave no rows.
#[allow(clippy::too_many_arguments)]
fn query(
    s: &Session,
    st: &mut State,
    v: &Visual,
    applied: &filters::Applied,
    limits: &PreviewLimits,
    base: u64,
    groups: &[FieldRef],
    values: &[FieldRef],
) -> Result<(Vec<Vec<Value>>, bool), String> {
    let mut fields: Vec<FieldBinding> = groups
        .iter()
        .map(|f| FieldBinding {
            display_name: None,
            show_as: None,
            role: "Category".into(),
            field: f.clone(),
        })
        .collect();
    fields.extend(values.iter().map(|f| FieldBinding {
        display_name: None,
        show_as: None,
        role: "Y".into(),
        field: f.clone(),
    }));
    let sv = Visual {
        name: v.name.clone(),
        position: v.position,
        visual_type: Some("clusteredColumnChart".into()),
        fields,
        parent_group: None,
        is_hidden: false,
        filter_config: v.filter_config.clone(),
        visual_extra: Default::default(),
        extra: Default::default(),
    };
    let lim = PreviewLimits {
        rows_per_visual: FIELD_ROWS,
        ..*limits
    };
    // cached on its own, so an estimate cut by the time budget resumes where it stopped
    let key = hash_of(&[
        &base.to_le_bytes(),
        &serde_json::to_vec(&sv.fields).unwrap_or_default(),
    ]);
    let d = match st.results.get(&key) {
        Some(d) => d.clone(),
        None => {
            let d = compute(s, st, &sv, applied, &lim);
            if d.get("status").and_then(Value::as_str) == Some("ok") {
                st.results.insert(key, d.clone());
            }
            d
        }
    };
    if d.get("status").and_then(Value::as_str) != Some("ok") {
        let reason = d
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("the data could not be evaluated");
        return Err(reason.to_string());
    }
    let rows = d
        .get("rows")
        .and_then(Value::as_array)
        .map(|r| r.iter().filter_map(|x| x.as_array().cloned()).collect::<Vec<_>>())
        .unwrap_or_default();
    let truncated = d.get("truncated").and_then(Value::as_bool).unwrap_or(false);
    Ok((rows, truncated))
}

/// A cell value as a category key (blank: the empty string).
fn key_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn num(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64).filter(|x| x.is_finite())
}

/// How the target is read.
enum Read {
    /// A categorical column: rows `[x…, y, count]`.
    Class { column: FieldRef, count: FieldRef },
    /// A numeric column: rows `[x…, n, mean, variance]`.
    Moments(FieldRef, FieldRef, FieldRef),
    /// One observation per row of (details…, x…): rows `[d…, x…, value]`.
    PerRow { details: Vec<FieldRef>, value: FieldRef },
}

impl Read {
    fn groups(&self, xs: &[FieldRef]) -> Vec<FieldRef> {
        match self {
            Read::Class { column, .. } => {
                let mut g = xs.to_vec();
                g.push(column.clone());
                g
            }
            Read::Moments(..) => xs.to_vec(),
            Read::PerRow { details, .. } => details.iter().chain(xs).cloned().collect(),
        }
    }
    fn values(&self) -> Vec<FieldRef> {
        match self {
            Read::Class { count, .. } => vec![count.clone()],
            Read::Moments(n, m, var) => vec![n.clone(), m.clone(), var.clone()],
            Read::PerRow { value, .. } => vec![value.clone()],
        }
    }
    /// The rows as (keys of the x fields, cell) pairs, merged by keys (first-seen order).
    fn cells(&self, rows: &[Vec<Value>], xs: usize, class: &str) -> Vec<(Vec<Value>, Cell)> {
        let mut out: Vec<(Vec<Value>, Cell)> = Vec::new();
        let mut at: HashMap<String, usize> = HashMap::new();
        let skip = match self {
            Read::PerRow { details, .. } => details.len(),
            _ => 0,
        };
        for r in rows {
            let keys: Vec<Value> = r.iter().skip(skip).take(xs).cloned().collect();
            if keys.len() < xs {
                continue;
            }
            let rest = r.get(skip + xs..).unwrap_or_default();
            let cell = match self {
                Read::Class { .. } => {
                    let n = num(rest.get(1)).unwrap_or(0.0);
                    let k = if rest.first().map(key_text).as_deref() == Some(class) {
                        n
                    } else {
                        0.0
                    };
                    Cell::from_counts(n, k)
                }
                Read::Moments(..) => {
                    let n = num(rest.first()).unwrap_or(0.0);
                    match num(rest.get(1)) {
                        Some(m) => Cell::from_moments(n, m, num(rest.get(2)).unwrap_or(0.0)),
                        None => continue,
                    }
                }
                Read::PerRow { .. } => match num(rest.first()) {
                    Some(y) => Cell::one(y),
                    None => continue,
                },
            };
            if cell.n <= 0.0 {
                continue;
            }
            let k = keys.iter().map(key_text).collect::<Vec<_>>().join("\u{1}");
            match at.get(&k) {
                Some(i) => {
                    if let Some(e) = out.get_mut(*i) {
                        e.1 = e.1.merge(&cell);
                    }
                }
                None => {
                    at.insert(k, out.len());
                    out.push((keys, cell));
                }
            }
        }
        out
    }
}

/// The estimate of one visual: `Ok` complete (cached), `Err` cut by the time budget.
#[allow(clippy::too_many_arguments)]
fn estimate(
    s: &Session,
    st: &mut State,
    report: &Report,
    page: &Page,
    v: &Visual,
    limits: &PreviewLimits,
    deadline: Instant,
    base: u64,
) -> Result<Value, Value> {
    let Some(model) = s.model.as_ref() else {
        return Ok(none("unavailable", "the session has no model"));
    };
    let spec = Spec::of(v, Some(model));
    let field = |i: usize| v.fields.get(i).map(|b| b.field.clone());
    let Some(target) = spec.target.and_then(field) else {
        return Ok(none("unsupported", "the visual has no field to analyze (Target)"));
    };
    if spec.explain_by.is_empty() {
        return Ok(none("unsupported", "the visual has no Explain by field"));
    }
    let applied = filters::collect(report, page, v, limits.now);
    let details: Vec<FieldRef> = spec.details.iter().filter_map(|i| field(*i)).collect();
    let read = match (&target, spec.categorical) {
        (FieldRef::Column { table, column }, true) => Read::Class {
            column: target.clone(),
            count: FieldRef::Aggregation {
                table: table.clone(),
                column: column.clone(),
                function: "CountNonNull".into(),
            },
        },
        (FieldRef::Column { table, column } | FieldRef::Aggregation { table, column, .. }, false)
            if details.is_empty() =>
        {
            let agg = |f: &str| FieldRef::Aggregation {
                table: table.clone(),
                column: column.clone(),
                function: f.into(),
            };
            Read::Moments(agg("CountNonNull"), agg("Avg"), agg("Variance"))
        }
        (FieldRef::Measure { .. } | FieldRef::Aggregation { .. }, false) if !details.is_empty() => {
            Read::PerRow {
                details: details.clone(),
                value: target.clone(),
            }
        }
        (FieldRef::Column { table, column }, false) => Read::PerRow {
            details: details.clone(),
            value: FieldRef::Aggregation {
                table: table.clone(),
                column: column.clone(),
                function: "Avg".into(),
            },
        },
        (FieldRef::Measure { .. }, false) => {
            return Ok(none(
                "unsupported",
                "a measure target is estimated only with an Expand by (Details) field: its rows are the observations",
            ));
        }
        _ => return Ok(none("unsupported", "this kind of target field is not estimated")),
    };
    let mut notes: Vec<String> = Vec::new();
    if spec.categorical && !details.is_empty() {
        notes.push("Expand by is ignored for a categorical target".into());
    }

    // the class of a categorical target: as saved, else the first value in order
    let mut class = String::new();
    let mut classes: Vec<Value> = Vec::new();
    if let Read::Class { column, count } = &read {
        let (rows, _) = match query(
            s,
            st,
            v,
            &applied,
            limits,
            base,
            std::slice::from_ref(column),
            std::slice::from_ref(count),
        ) {
            Ok(r) => r,
            Err(e) => return Ok(none("unavailable", e)),
        };
        classes = rows
            .iter()
            .filter_map(|r| r.first().cloned())
            .filter(|x| !x.is_null())
            .take(MAX_CLASSES)
            .collect();
        class = match &spec.class {
            Some(c) if classes.iter().any(|x| key_text(x) == *c) => c.clone(),
            Some(c) => {
                notes.push(format!(
                    "the saved class `{c}` is not in the data: the first class is used"
                ));
                classes.first().map(key_text).unwrap_or_default()
            }
            None => classes.first().map(key_text).unwrap_or_default(),
        };
        if classes.is_empty() {
            return Ok(none("no_rows", "the target has no value"));
        }
    }

    // one query per explain-by field
    let mut analysed: Vec<(usize, bool, stats::FieldResult, Vec<Value>)> = Vec::new();
    let mut overall = Cell::default();
    for (n, i) in spec.explain_by.iter().enumerate() {
        if n >= MAX_FIELDS {
            notes.push(format!(
                "{} more Explain by fields are not analyzed (at most {MAX_FIELDS})",
                spec.explain_by.len() - MAX_FIELDS
            ));
            break;
        }
        if Instant::now() >= deadline {
            return Err(none(
                "pending",
                "the estimate did not finish within the time budget",
            ));
        }
        let Some(x) = field(*i) else { continue };
        let FieldRef::Column { table, column } = &x else {
            notes.push(format!("field {i}: only columns are analyzed in Explain by"));
            continue;
        };
        let numeric = is_numeric_type(&data_type_name(model, table, column));
        let rows = match query(
            s,
            st,
            v,
            &applied,
            limits,
            base,
            &read.groups(std::slice::from_ref(&x)),
            &read.values(),
        ) {
            Ok((_, true)) => {
                notes.push(format!("field {i}: more than {FIELD_ROWS} rows, not analyzed"));
                continue;
            }
            Ok((rows, false)) => rows,
            Err(e) => {
                notes.push(format!("field {i}: {e}"));
                continue;
            }
        };
        let cells = read.cells(&rows, 1, &class);
        let total = cells.iter().fold(Cell::default(), |a, (_, c)| a.merge(c));
        if total.n > overall.n {
            overall = total;
        }
        let mut raw: Vec<Value> = Vec::new();
        let fc = if numeric {
            FieldCells::Numbers(
                cells
                    .iter()
                    .filter_map(|(k, c)| num(k.first()).map(|x| (x, *c)))
                    .collect(),
            )
        } else {
            FieldCells::Categories(
                cells
                    .iter()
                    .map(|(k, c)| {
                        let kv = k.first().cloned().unwrap_or(Value::Null);
                        raw.push(kv.clone());
                        (key_text(&kv), *c)
                    })
                    .collect(),
            )
        };
        analysed.push((*i, numeric, stats::analyse_field(*i, &fc, spec.goal), raw));
    }
    if overall.n <= 0.0 {
        return Ok(none("no_rows", "no rows to analyze"));
    }
    let results: Vec<stats::FieldResult> = analysed.iter().map(|a| a.2.clone()).collect();
    let ranked = stats::rank(&results, spec.goal, MAX_INFLUENCERS);
    let tests: usize = results.iter().map(|r| r.tests).sum();

    // top segments: the strongest fields first, then the others in order
    let mut seg_fields: Vec<usize> = Vec::new();
    for f in ranked.iter().map(|r| r.field).chain(analysed.iter().map(|a| a.0)) {
        if !seg_fields.contains(&f) && seg_fields.len() < SEGMENT_FIELDS {
            seg_fields.push(f);
        }
    }
    let mut segments: Vec<stats::Segment> = Vec::new();
    let mut seg_used: Vec<usize> = Vec::new();
    while !seg_fields.is_empty() {
        if Instant::now() >= deadline {
            return Err(none(
                "pending",
                "the estimate did not finish within the time budget",
            ));
        }
        let xs: Vec<FieldRef> = seg_fields.iter().filter_map(|i| field(*i)).collect();
        match query(
            s,
            st,
            v,
            &applied,
            limits,
            base,
            &read.groups(&xs),
            &read.values(),
        ) {
            Ok((rows, false)) => {
                let numeric: Vec<bool> = seg_fields
                    .iter()
                    .map(|f| analysed.iter().any(|a| a.0 == *f && a.1))
                    .collect();
                let cells: Vec<(Vec<Key>, Cell)> = read
                    .cells(&rows, xs.len(), &class)
                    .into_iter()
                    .map(|(k, c)| {
                        let keys = k
                            .iter()
                            .zip(&numeric)
                            .map(|(x, n)| match (n, num(Some(x))) {
                                (true, Some(f)) => Key::Num(f),
                                _ => Key::Cat(key_text(x)),
                            })
                            .collect();
                        (keys, c)
                    })
                    .collect();
                let edges: Vec<Vec<f64>> = seg_fields
                    .iter()
                    .map(|f| {
                        analysed
                            .iter()
                            .find(|a| a.0 == *f)
                            .map(|a| a.2.edges.clone())
                            .unwrap_or_default()
                    })
                    .collect();
                segments = stats::segments(&cells, &edges, spec.goal);
                seg_used = seg_fields.clone();
                break;
            }
            Ok((_, true)) => {
                seg_fields.pop();
            }
            Err(e) => {
                notes.push(format!("top segments: {e}"));
                break;
            }
        }
    }

    // the JSON
    let raw_of = |f: usize, key: &str| -> Value {
        analysed
            .iter()
            .find(|a| a.0 == f)
            .and_then(|a| a.3.iter().find(|x| key_text(x) == key).cloned())
            .unwrap_or_else(|| json!(key))
    };
    let influencers: Vec<Value> = ranked.iter().map(|i| influencer_json(i, &raw_of)).collect();
    let fields: Vec<Value> = analysed
        .iter()
        .map(|(i, numeric, r, _)| {
            let bars: Vec<Value> = r
                .bars
                .iter()
                .take(MAX_BARS)
                .map(|b| {
                    let mut o = Map::new();
                    match &b.label {
                        stats::BarLabel::Category(k) => {
                            o.insert("value".into(), raw_of(*i, k));
                        }
                        stats::BarLabel::Range(a, z) => {
                            o.insert("from".into(), number(*a));
                            o.insert("to".into(), number(*z));
                        }
                    }
                    o.insert("mean".into(), number(b.mean));
                    o.insert("count".into(), number(b.count));
                    Value::Object(o)
                })
                .collect();
            json!({"field": i, "numeric": numeric, "bars": bars, "bars_truncated": r.bars.len() > MAX_BARS})
        })
        .collect();
    let segments_json: Vec<Value> = segments
        .iter()
        .map(|sg| {
            let conds: Vec<Value> = sg
                .conds
                .iter()
                .map(|c| {
                    let real = |k: usize| seg_used.get(k).copied().unwrap_or(0);
                    match c {
                        stats::Cond::Is(k, x) => json!({"field": real(*k), "op": "is", "value": raw_of(real(*k), x)}),
                        stats::Cond::IsNot(k, x) => {
                            json!({"field": real(*k), "op": "is_not", "value": raw_of(real(*k), x)})
                        }
                        stats::Cond::AtMost(k, t) => json!({"field": real(*k), "op": "at_most", "value": number(*t)}),
                        stats::Cond::Above(k, t) => json!({"field": real(*k), "op": "above", "value": number(*t)}),
                    }
                })
                .collect();
            json!({"conditions": conds, "mean": number(sg.mean), "count": number(sg.count), "share": number(sg.share)})
        })
        .collect();
    let mut o = Map::new();
    o.insert("status".into(), json!("ok"));
    o.insert("method".into(), json!("bicraft-1"));
    o.insert("rows".into(), number(overall.n));
    o.insert("overall".into(), number(overall.mean()));
    if spec.categorical {
        o.insert("class".into(), json!(class));
        o.insert("classes".into(), json!(classes.len()));
    }
    o.insert("tests".into(), json!(tests));
    o.insert("alpha".into(), number(stats::ALPHA / tests.max(1) as f64));
    o.insert("influencers".into(), Value::Array(influencers));
    o.insert("fields".into(), Value::Array(fields));
    o.insert("segments".into(), Value::Array(segments_json));
    o.insert("segment_fields".into(), json!(seg_used));
    if !notes.is_empty() {
        notes.truncate(20);
        o.insert("notes".into(), json!(notes));
    }
    Ok(Value::Object(o))
}

/// A finite number as JSON, rounded to 12 significant digits (stable across platforms' last
/// bits), else null.
fn number(x: f64) -> Value {
    if !x.is_finite() {
        return Value::Null;
    }
    let r: f64 = format!("{x:.11e}").parse().unwrap_or(x);
    if r == r.trunc() && r.abs() < 9e15 {
        json!(r as i64)
    } else {
        json!(r)
    }
}

fn influencer_json(i: &Influencer, raw_of: &dyn Fn(usize, &str) -> Value) -> Value {
    let mut o = Map::new();
    o.insert("field".into(), json!(i.field));
    match &i.what {
        What::Is(k) => {
            o.insert("kind".into(), json!("is"));
            o.insert("value".into(), raw_of(i.field, k));
        }
        What::Bin { from, to } => {
            o.insert("kind".into(), json!("range"));
            o.insert("from".into(), number(*from));
            o.insert("to".into(), number(*to));
        }
        What::Up { step } => {
            o.insert("kind".into(), json!("up"));
            o.insert("step".into(), number(*step));
        }
    }
    o.insert("value_mean".into(), number(i.value));
    o.insert("rest_mean".into(), number(i.rest));
    o.insert("effect".into(), number(i.effect));
    o.insert("count".into(), number(i.count));
    o.insert("p".into(), number(i.p));
    Value::Object(o)
}

#[cfg(test)]
#[path = "preview_key_influencers_tests.rs"]
mod tests;
