//! `visual.analyze`: analyze a data point of a visual (docs/analyze-point.md). Two analyses,
//! both BIcraft's own method, never presented as Power BI's result:
//!
//! - **change** (a point on a date or number axis): the point is compared with the previous
//!   point of its axis; each candidate field is scored by how much of the change one of its
//!   members carries (its member's change in the direction of the total change, over the sum of
//!   every member's absolute change);
//! - **distribution** (a point on any axis): each candidate field's share of the measure under
//!   the point is compared with its share under the visual's other points (the mean of their
//!   shares); the score is the total variation distance of the two.
//!
//! `visual.explain_anomaly` explains a point a line chart's anomaly detection flags with the same
//! change score, against the value the detection expected for each member (or against the previous
//! point).
//!
//! Every number comes from grouped queries through the preview evaluator
//! ([`crate::preview::grouped_rows`]), so the report's, the page's and the visual's filters,
//! the reader's slicer selections and the model's relationships apply as in the visual.
//! Bounded: at most [`MAX_CANDIDATES`] fields, [`MAX_MEMBERS`] members each, [`MAX_ROWS`] rows a
//! query, and a time budget ([`BUDGET`]); what is left out is listed in `skipped`.

use std::cmp::Ordering;
use std::time::{Duration, Instant};

use bicraft_model::{DataType, SemanticModel};
use bicraft_report::{FieldRef, Page, Report, Visual};
use serde_json::{Value, json};

use crate::Session;
use crate::params::allow;

/// Most candidate fields analysed.
const MAX_CANDIDATES: usize = 40;
/// Most members a candidate field may have.
const MAX_MEMBERS: usize = 100;
/// Most rows one grouped query may return.
const MAX_ROWS: usize = 20_000;
/// Candidates returned when `top` is not given.
const DEFAULT_TOP: u64 = 10;
/// Most candidates returned.
const MAX_TOP: u64 = 100;
/// Time the whole analysis may take.
const BUDGET: Duration = Duration::from_secs(60);

/// Which analysis runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Change,
    Distribution,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Change => "change",
            Kind::Distribution => "distribution",
        }
    }
}

fn sequential(ty: DataType) -> bool {
    matches!(
        ty,
        DataType::Int64 | DataType::Double | DataType::Decimal | DataType::DateTime
    )
}

/// Whether two cell values are the same member (text without case, as the engine groups).
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(x), Value::String(y)) => x.to_lowercase() == y.to_lowercase(),
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Null, Value::String(s)) | (Value::String(s), Value::Null) => s.is_empty(),
        _ => a == b,
    }
}

/// The order of two axis values (numbers, or ISO date texts).
fn order(a: &Value, b: &Value) -> Ordering {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x.total_cmp(&y),
        _ => match (a.as_str(), b.as_str()) {
            (Some(x), Some(y)) => x.cmp(y),
            // blanks first
            _ => a.is_null().cmp(&b.is_null()).reverse(),
        },
    }
}

fn num(v: Option<&Value>) -> f64 {
    v.and_then(Value::as_f64).filter(|x| x.is_finite()).unwrap_or(0.0)
}

fn column(t: &str, c: &str) -> FieldRef {
    FieldRef::Column {
        table: t.to_string(),
        column: c.to_string(),
    }
}

/// The visual's first value: a measure or an aggregated column outside the tooltips.
fn measure_of(v: &Visual) -> Option<FieldRef> {
    v.fields
        .iter()
        .filter(|b| !b.role.eq_ignore_ascii_case("Tooltips"))
        .map(|b| &b.field)
        .find(|f| matches!(f, FieldRef::Measure { .. } | FieldRef::Aggregation { .. }))
        .cloned()
}

/// The default candidates: every visible column of the model but the point's, the measure's own
/// column and the decimal columns (amounts rather than categories).
fn default_candidates(model: &SemanticModel, point: &[FieldRef], measure: &FieldRef) -> Vec<FieldRef> {
    let mut out = Vec::new();
    for t in &model.tables {
        for c in &t.columns {
            if c.is_hidden
                || c.is_row_number()
                || matches!(
                    c.data_type,
                    DataType::Binary | DataType::Double | DataType::Decimal
                )
            {
                continue;
            }
            let in_point = point.iter().any(|p| {
                matches!(p, FieldRef::Column { table, column }
                    if table.eq_ignore_ascii_case(&t.name) && column.eq_ignore_ascii_case(&c.name))
            });
            let measured = matches!(measure, FieldRef::Aggregation { table, column, .. }
                if table.eq_ignore_ascii_case(&t.name) && column.eq_ignore_ascii_case(&c.name));
            if !in_point && !measured {
                out.push(column(&t.name, &c.name));
            }
        }
    }
    out
}

/// The candidates named by `fields`: columns of the model.
fn named_candidates(model: &SemanticModel, v: &Value) -> Result<Vec<FieldRef>, String> {
    let list = v
        .as_array()
        .ok_or("`fields` must be a list of columns like Product[Color]")?;
    if list.is_empty() || list.len() > MAX_CANDIDATES {
        return Err(format!("`fields` names 1 to {MAX_CANDIDATES} columns"));
    }
    let mut out = Vec::new();
    for x in list {
        let text = x
            .as_str()
            .ok_or("each entry of `fields` must be a column like Product[Color]")?;
        let r = crate::refs::resolve_field(Some(model), text)?;
        if !matches!(r.field, FieldRef::Column { .. }) {
            return Err(format!(
                "`{text}` is not a column: only columns can explain a point"
            ));
        }
        if !out.contains(&r.field) {
            out.push(r.field);
        }
    }
    Ok(out)
}

/// Where the queries run.
struct Scope<'a> {
    s: &'a Session,
    report: &'a Report,
    page: &'a Page,
    v: &'a Visual,
    measure: &'a FieldRef,
}

impl Scope<'_> {
    /// Rows of `groups` then the measure; `Err` says why there are none.
    fn rows(&self, groups: &[FieldRef]) -> Result<Vec<Vec<Value>>, String> {
        let d = crate::preview::grouped_rows(
            self.s,
            self.report,
            self.page,
            self.v,
            groups,
            std::slice::from_ref(self.measure),
            MAX_ROWS,
        );
        match d.get("status").and_then(Value::as_str) {
            Some("ok") => {}
            Some("no_rows") => return Ok(Vec::new()),
            _ => {
                return Err(d
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("the data could not be evaluated")
                    .to_string());
            }
        }
        if d.get("truncated").and_then(Value::as_bool) == Some(true) {
            return Err(format!("more than {MAX_ROWS} rows"));
        }
        Ok(d.get("rows")
            .and_then(Value::as_array)
            .map(|r| r.iter().filter_map(|x| x.as_array().cloned()).collect())
            .unwrap_or_default())
    }
}

/// `top`: how many candidates are returned.
fn top_of(p: &Value) -> Result<u64, String> {
    match p.get("top") {
        None | Some(Value::Null) => Ok(DEFAULT_TOP),
        Some(v) => v
            .as_u64()
            .filter(|n| (1..=MAX_TOP).contains(n))
            .ok_or_else(|| format!("`top` must be a whole number from 1 to {MAX_TOP}")),
    }
}

/// One scored candidate.
struct Scored {
    field: String,
    score: f64,
    row: Value,
}

/// `visual.analyze`; see the module docs.
pub(crate) fn visual_analyze(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "point", "kind", "fields", "top"])?;
    let top = top_of(p)?;
    let asked = match p.get("kind").and_then(|k| (!k.is_null()).then_some(k)) {
        None => None,
        Some(k) => match k.as_str() {
            Some("auto") => None,
            Some("change") => Some(Kind::Change),
            Some("distribution") => Some(Kind::Distribution),
            _ => return Err("`kind` must be \"auto\", \"change\" or \"distribution\"".into()),
        },
    };
    let base = s.report.as_ref().ok_or("no report is open")?;
    let effective = s.interactions.effective(base);
    let report = effective.as_ref().map_or(base, |e| &e.data);
    let model = s.model.as_ref().ok_or("no model is open (file.open first)")?;
    let (page, v) = crate::interact::find_visual(report, p)?;
    let point = match p.get("point").filter(|x| !x.is_null()) {
        Some(pt) => crate::interact::resolve_point(v, pt, Some(model))?,
        None => {
            return Err(
                "`point` is required: the data point to analyze, as view.interact `select` takes it".into(),
            );
        }
    };
    let measure = measure_of(v).ok_or_else(|| {
        format!(
            "visual `{}` shows no measure or aggregated value: there is nothing to analyze",
            v.name
        )
    })?;
    // the axis: the point's first sequential column for a change, else its first column
    let seq = point.iter().position(|x| sequential(x.ty));
    let kind = match asked {
        Some(k) => k,
        None if seq.is_some() => Kind::Change,
        None => Kind::Distribution,
    };
    let axis = match kind {
        Kind::Change => seq.ok_or(
            "explaining a change needs a point on a date or number axis; this point's axis is \
             categorical (use kind \"distribution\")",
        )?,
        Kind::Distribution => 0,
    };
    let point_cols: Vec<FieldRef> = point.iter().map(|x| column(&x.table, &x.column)).collect();
    let candidates = match p.get("fields").filter(|x| !x.is_null()) {
        Some(f) => named_candidates(model, f)?,
        None => default_candidates(model, &point_cols, &measure),
    };
    let scope = Scope {
        s,
        report,
        page,
        v,
        measure: &measure,
    };
    // the point's other columns (a legend) stay as they are
    let fixed = |row: &[Value]| {
        point
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != axis)
            .all(|(i, x)| row.get(i).is_some_and(|c| same(c, &x.value)))
    };
    let Some(x0) = point.get(axis).map(|x| x.value.clone()) else {
        return Err("the point has no value".into());
    };
    let axis_rows: Vec<Vec<Value>> = scope
        .rows(&point_cols)?
        .into_iter()
        .filter(|r| fixed(r))
        .collect();
    let n = point_cols.len();
    let total_at = |x: &Value| -> Option<f64> {
        axis_rows
            .iter()
            .find(|r| r.get(axis).is_some_and(|c| same(c, x)))
            .map(|r| num(r.get(n)))
    };
    let current = total_at(&x0).ok_or("the visual shows no value at this point")?;
    let mut others: Vec<Value> = axis_rows
        .iter()
        .filter_map(|r| r.get(axis).cloned())
        .filter(|x| !same(x, &x0))
        .collect();
    let mut previous = None;
    match kind {
        Kind::Change => {
            others.retain(|x| !x.is_null() && order(x, &x0) == Ordering::Less);
            others.sort_by(order);
            let prev = others
                .pop()
                .ok_or("the point is the first on its axis: there is no previous point to compare with")?;
            let before = total_at(&prev).unwrap_or(0.0);
            if current == before {
                return Err(
                    "the point has the same value as the previous one: there is no change to explain".into(),
                );
            }
            previous = Some((prev, before));
        }
        Kind::Distribution => {
            if others.is_empty() {
                return Err("the visual has no other point to compare this one with".into());
            }
        }
    }
    let start = Instant::now();
    let mut scored: Vec<Scored> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    let mut considered = 0usize;
    for (i, f) in candidates.iter().enumerate() {
        let name = crate::refs::field_text(f);
        if i >= MAX_CANDIDATES {
            skipped.push(json!({"field": name, "reason": format!("past the first {MAX_CANDIDATES} fields")}));
            continue;
        }
        if start.elapsed() > BUDGET {
            skipped.push(json!({"field": name, "reason": "not reached within the time budget"}));
            continue;
        }
        considered += 1;
        let mut groups = point_cols.clone();
        groups.push(f.clone());
        let rows = match scope.rows(&groups) {
            Ok(r) => r,
            Err(e) => {
                skipped.push(json!({"field": name, "reason": e}));
                continue;
            }
        };
        let rows: Vec<&Vec<Value>> = rows.iter().filter(|r| fixed(r)).collect();
        let member = |r: &Vec<Value>| r.get(n).cloned().unwrap_or(Value::Null);
        let mut members: Vec<Value> = Vec::new();
        for r in &rows {
            let m = member(r);
            if !members.iter().any(|x| same(x, &m)) {
                members.push(m);
            }
        }
        if members.len() > MAX_MEMBERS {
            skipped.push(json!({"field": name, "reason": format!("more than {MAX_MEMBERS} values")}));
            continue;
        }
        if members.len() < 2 {
            skipped.push(json!({"field": name, "reason": "fewer than two values under the points"}));
            continue;
        }
        // the measure of each member at axis value x
        let cell = |x: &Value, m: &Value| -> f64 {
            rows.iter()
                .find(|r| r.get(axis).is_some_and(|c| same(c, x)) && same(&member(r), m))
                .map_or(0.0, |r| num(r.get(n + 1)))
        };
        let result = match (&kind, &previous) {
            (Kind::Change, Some((prev, before))) => score_change(
                &name,
                &members,
                |m| (cell(&x0, m), cell(prev, m)),
                (current, *before),
            ),
            _ => score_distribution(&name, &members, &x0, &others, &cell),
        };
        match result {
            Ok(sc) => scored.push(sc),
            Err(reason) => skipped.push(json!({"field": name, "reason": reason})),
        }
    }
    scored.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.field.cmp(&b.field)));
    let found = scored.len();
    let list: Vec<Value> = scored
        .into_iter()
        .take(usize::try_from(top).unwrap_or(usize::MAX))
        .map(|x| x.row)
        .collect();
    let mut out = json!({
        "page": page.name,
        "visual": v.name,
        "kind": kind.name(),
        "measure": crate::refs::field_text(&measure),
        "point": point.iter().map(|x| json!({"table": x.table, "column": x.column, "value": x.value})).collect::<Vec<_>>(),
        "candidates": list,
        "found": found,
        "considered": considered,
        "skipped": skipped,
        "estimate": "BIcraft's own ranking of the model's fields (docs/analyze-point.md), not Power BI's result",
    });
    if let (Some((prev, before)), Some(o)) = (previous, out.as_object_mut()) {
        let delta = current - before;
        o.insert(
            "change".into(),
            json!({
                "axis": point_cols.get(axis).map(crate::refs::field_text),
                "current": current,
                "previous_point": prev,
                "previous": before,
                "delta": delta,
                "direction": if delta > 0.0 { "increase" } else { "decrease" },
            }),
        );
    } else if let Some(o) = out.as_object_mut() {
        o.insert("value".into(), json!(current));
        o.insert("other_points".into(), json!(others.len()));
    }
    Ok(out)
}

/// Most anomalies listed in `anomalies`.
const MAX_LISTED: usize = 100;

/// What an anomaly is compared with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Compare {
    Expected,
    Previous,
}

/// `visual.explain_anomaly`: why a point the line chart's anomaly detection flags differs from
/// what was expected (docs/analyze-point.md "Explain an anomaly"). BIcraft's own method: with
/// `compare: "expected"` (the default), each candidate field's members are run through the same
/// detection as the line ([`bicraft_render::anomaly::detect`]), and the field is scored as a
/// change is, with the member's expected value in place of the previous point's; with `compare:
/// "previous"`, the point is analyzed as [`visual_analyze`]'s change from the previous point.
pub(crate) fn visual_explain_anomaly(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "point", "compare", "fields", "top"])?;
    let top = top_of(p)?;
    let compare = match p.get("compare").filter(|x| !x.is_null()) {
        None => Compare::Expected,
        Some(c) => match c.as_str() {
            Some("expected") => Compare::Expected,
            Some("previous") => Compare::Previous,
            _ => return Err("`compare` must be \"expected\" or \"previous\"".into()),
        },
    };
    let base = s.report.as_ref().ok_or("no report is open")?;
    let effective = s.interactions.effective(base);
    let report = effective.as_ref().map_or(base, |e| &e.data);
    let model = s.model.as_ref().ok_or("no model is open (file.open first)")?;
    let (page, v) = crate::interact::find_visual(report, p)?;
    let spec = crate::visual_extras::anomaly_detection(v).ok_or_else(|| {
        format!(
            "visual `{}` does not find anomalies: turn on Find anomalies first (visual.analytics kind \
             anomaly_detection)",
            v.name
        )
    })?;
    if v.fields.iter().any(|b| b.role.eq_ignore_ascii_case("Series")) {
        return Err("anomalies are not detected on a line chart with a legend".into());
    }
    let measure = v
        .fields
        .get(spec.series)
        .map(|b| b.field.clone())
        .filter(|f| matches!(f, FieldRef::Measure { .. } | FieldRef::Aggregation { .. }))
        .ok_or("the anomaly detection's series is not a measure or an aggregated value")?;
    let (cat, axis) = v
        .fields
        .iter()
        .enumerate()
        .find(|(_, b)| b.role.eq_ignore_ascii_case("Category"))
        .and_then(|(i, b)| matches!(b.field, FieldRef::Column { .. }).then(|| (i, b.field.clone())))
        .ok_or("the line chart's axis is not a column: there is no point to explain")?;
    let scope = Scope {
        s,
        report,
        page,
        v,
        measure: &measure,
    };
    // the line's points in axis order, and the detection the chart draws
    let mut line = scope.rows(std::slice::from_ref(&axis))?;
    line.sort_by(|a, b| {
        order(
            a.first().unwrap_or(&Value::Null),
            b.first().unwrap_or(&Value::Null),
        )
    });
    let xs: Vec<Value> = line
        .iter()
        .map(|r| r.first().cloned().unwrap_or(Value::Null))
        .collect();
    let vals: Vec<Option<f64>> = line
        .iter()
        .map(|r| r.get(1).and_then(Value::as_f64).filter(|x| x.is_finite()))
        .collect();
    let est = bicraft_render::anomaly::detect(&vals, spec.sensitivity);
    if est.is_empty() {
        return Err(format!(
            "the line has fewer than {} values: no anomaly is detected",
            bicraft_render::anomaly::MIN_POINTS
        ));
    }
    let flagged: Vec<usize> = (0..xs.len())
        .filter(|i| est.get(*i).copied().flatten().is_some_and(|e| e.anomaly))
        .collect();
    // the point asked for, else the anomaly furthest outside its range
    let idx = match p.get("point").filter(|x| !x.is_null()) {
        Some(pt) => {
            let point = crate::interact::resolve_point(v, pt, Some(model))?;
            let x0 = point
                .iter()
                .find(|x| same_field_column(&axis, &x.table, &x.column))
                .map(|x| x.value.clone())
                .ok_or("the point has no value on the line chart's axis")?;
            xs.iter()
                .position(|x| same(x, &x0))
                .ok_or("the visual shows no value at this point")?
        }
        None => {
            let outside = |i: &usize| -> f64 {
                match (est.get(*i).copied().flatten(), vals.get(*i).copied().flatten()) {
                    (Some(e), Some(y)) => {
                        let half = ((e.high - e.low) / 2.0).max(f64::MIN_POSITIVE);
                        ((y - e.expected).abs() / half).min(1e300)
                    }
                    _ => 0.0,
                }
            };
            flagged
                .iter()
                .copied()
                .max_by(|a, b| outside(a).total_cmp(&outside(b)))
                .ok_or("no point of the line is an anomaly at the detection's sensitivity")?
        }
    };
    let (Some(e), Some(current), Some(x0)) = (
        est.get(idx).copied().flatten(),
        vals.get(idx).copied().flatten(),
        xs.get(idx).cloned(),
    ) else {
        return Err("the visual shows no value at this point".into());
    };
    if !e.anomaly {
        return Err(format!(
            "the point at {x0} is not an anomaly: its value {current} is within the expected range {} to {}",
            round(e.low),
            round(e.high)
        ));
    }
    let deviation = current - e.expected;
    let anomaly = json!({
        "axis": crate::refs::field_text(&axis),
        "at": x0,
        "value": current,
        "expected": round(e.expected),
        "low": round(e.low),
        "high": round(e.high),
        "deviation": round(deviation),
        "direction": if deviation > 0.0 { "above" } else { "below" },
        "sensitivity": spec.sensitivity,
    });
    let listed: Vec<Value> = flagged
        .iter()
        .take(MAX_LISTED)
        .filter_map(|i| xs.get(*i).cloned())
        .collect();
    if compare == Compare::Previous {
        let mut q = json!({
            "page": page.name,
            "visual": v.name,
            "point": [{"c": cat, "v": x0}],
            "kind": "change",
            "top": top,
        });
        if let (Some(f), Some(o)) = (p.get("fields").filter(|x| !x.is_null()), q.as_object_mut()) {
            o.insert("fields".into(), f.clone());
        }
        let mut out = visual_analyze(s, &q)?;
        if let Some(o) = out.as_object_mut() {
            o.insert("compare".into(), json!("previous"));
            o.insert("anomaly".into(), anomaly);
            o.insert("anomalies".into(), Value::Array(listed));
            o.insert("estimate".into(), json!(ESTIMATE));
        }
        return Ok(out);
    }
    let axis_cols = vec![axis.clone()];
    let candidates = match p.get("fields").filter(|x| !x.is_null()) {
        Some(f) => named_candidates(model, f)?,
        None => default_candidates(model, &axis_cols, &measure),
    };
    let start = Instant::now();
    let mut scored: Vec<Scored> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    let mut considered = 0usize;
    for (i, f) in candidates.iter().enumerate() {
        let name = crate::refs::field_text(f);
        if i >= MAX_CANDIDATES {
            skipped.push(json!({"field": name, "reason": format!("past the first {MAX_CANDIDATES} fields")}));
            continue;
        }
        if start.elapsed() > BUDGET {
            skipped.push(json!({"field": name, "reason": "not reached within the time budget"}));
            continue;
        }
        considered += 1;
        let rows = match scope.rows(&[axis.clone(), f.clone()]) {
            Ok(r) => r,
            Err(e) => {
                skipped.push(json!({"field": name, "reason": e}));
                continue;
            }
        };
        let member = |r: &Vec<Value>| r.get(1).cloned().unwrap_or(Value::Null);
        let mut members: Vec<Value> = Vec::new();
        for r in &rows {
            let m = member(r);
            if !members.iter().any(|x| same(x, &m)) {
                members.push(m);
            }
        }
        if members.len() > MAX_MEMBERS {
            skipped.push(json!({"field": name, "reason": format!("more than {MAX_MEMBERS} values")}));
            continue;
        }
        if members.len() < 2 {
            skipped.push(json!({"field": name, "reason": "fewer than two values under the points"}));
            continue;
        }
        // each member's line over the chart's points (0 where it has no row), its value at the
        // anomaly and the value the same detection expects there
        let mut pairs: Vec<(f64, f64)> = Vec::with_capacity(members.len());
        for m in &members {
            let series: Vec<Option<f64>> = xs
                .iter()
                .map(|x| {
                    Some(
                        rows.iter()
                            .find(|r| r.first().is_some_and(|c| same(c, x)) && same(&member(r), m))
                            .map_or(0.0, |r| num(r.get(2))),
                    )
                })
                .collect();
            let at = series.get(idx).copied().flatten().unwrap_or(0.0);
            let expected = bicraft_render::anomaly::detect(&series, spec.sensitivity)
                .get(idx)
                .copied()
                .flatten()
                .map_or(at, |e| e.expected);
            pairs.push((at, expected));
        }
        let pair_of = |m: &Value| {
            members
                .iter()
                .position(|x| same(x, m))
                .and_then(|k| pairs.get(k).copied())
                .unwrap_or((0.0, 0.0))
        };
        match score_change(&name, &members, pair_of, (current, e.expected)) {
            Ok(mut sc) => {
                rename(&mut sc.row, "member_previous", "member_expected");
                rename(&mut sc.row, "member_change", "member_deviation");
                rename(&mut sc.row, "share_of_change", "share_of_deviation");
                scored.push(sc);
            }
            Err(reason) => skipped.push(json!({"field": name, "reason": reason})),
        }
    }
    scored.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.field.cmp(&b.field)));
    let found = scored.len();
    let list: Vec<Value> = scored
        .into_iter()
        .take(usize::try_from(top).unwrap_or(usize::MAX))
        .map(|x| x.row)
        .collect();
    Ok(json!({
        "page": page.name,
        "visual": v.name,
        "compare": "expected",
        "measure": crate::refs::field_text(&measure),
        "anomaly": anomaly,
        "anomalies": listed,
        "candidates": list,
        "found": found,
        "considered": considered,
        "skipped": skipped,
        "estimate": ESTIMATE,
    }))
}

/// How the explanations are labelled wherever they are shown.
const ESTIMATE: &str = "BIcraft's own explanation of BIcraft's own anomaly estimate \
                        (docs/analyze-point.md), not Power BI's result";

/// Whether `f` is the column `table[column]`.
fn same_field_column(f: &FieldRef, t: &str, c: &str) -> bool {
    matches!(f, FieldRef::Column { table, column }
        if table.eq_ignore_ascii_case(t) && column.eq_ignore_ascii_case(c))
}

/// Renames the key `from` of the object `row` to `to`.
fn rename(row: &mut Value, from: &str, to: &str) {
    if let Some(o) = row.as_object_mut()
        && let Some(x) = o.remove(from)
    {
        o.insert(to.into(), x);
    }
}

/// The change score of a field: its member whose change goes furthest in the direction of the
/// total change, that change over the sum of every member's absolute change.
fn score_change(
    name: &str,
    members: &[Value],
    at: impl Fn(&Value) -> (f64, f64),
    (current, before): (f64, f64),
) -> Result<Scored, String> {
    let total = current - before;
    let pairs: Vec<(f64, f64)> = members.iter().map(&at).collect();
    // a field that does not filter the measure (no relationship to its table) shows the whole
    // measure under every value
    if pairs.iter().all(|p| *p == (current, before)) {
        return Err("every value shows the whole measure: the field does not split it".into());
    }
    let sign = total.signum();
    let spread: f64 = pairs.iter().map(|(c, b)| (c - b).abs()).sum();
    if spread <= 0.0 {
        return Err("no value of the field changed".into());
    }
    let mut best = 0usize;
    let mut best_d = f64::NEG_INFINITY;
    for (i, (c, b)) in pairs.iter().enumerate() {
        let d = (c - b) * sign;
        if d > best_d {
            best_d = d;
            best = i;
        }
    }
    let (c, b) = pairs.get(best).copied().unwrap_or((0.0, 0.0));
    let score = (best_d.max(0.0) / spread).clamp(0.0, 1.0);
    Ok(Scored {
        field: name.to_string(),
        score,
        row: json!({
            "field": name,
            "score": round(score),
            "member": members.get(best).cloned().unwrap_or(Value::Null),
            "member_current": c,
            "member_previous": b,
            "member_change": c - b,
            "share_of_change": if total == 0.0 { Value::Null } else { json!(round((c - b) / total)) },
            "members": members.len(),
        }),
    })
}

/// The distribution score of a field: the total variation distance between its shares under
/// the point and the mean of its shares under each other point.
fn score_distribution(
    name: &str,
    members: &[Value],
    x0: &Value,
    others: &[Value],
    cell: &dyn Fn(&Value, &Value) -> f64,
) -> Result<Scored, String> {
    let shares = |x: &Value| -> Result<Option<Vec<f64>>, String> {
        let xs: Vec<f64> = members.iter().map(|m| cell(x, m)).collect();
        if xs.iter().any(|v| *v < 0.0) {
            return Err("negative values: shares are not defined".to_string());
        }
        let sum: f64 = xs.iter().sum();
        Ok((sum > 0.0).then(|| xs.iter().map(|v| v / sum).collect()))
    };
    let here = shares(x0)?.ok_or("the point's value is zero under this field")?;
    let mut mean = vec![0.0; members.len()];
    let mut count = 0usize;
    for x in others {
        if let Some(sh) = shares(x)? {
            for (m, v) in mean.iter_mut().zip(sh) {
                *m += v;
            }
            count += 1;
        }
    }
    if count == 0 {
        return Err("the other points have no value under this field".into());
    }
    for m in &mut mean {
        *m /= count as f64;
    }
    let score = (0.5 * here.iter().zip(&mean).map(|(a, b)| (a - b).abs()).sum::<f64>()).clamp(0.0, 1.0);
    let mut best = 0usize;
    let mut best_d = f64::NEG_INFINITY;
    for (i, (a, b)) in here.iter().zip(&mean).enumerate() {
        if a - b > best_d {
            best_d = a - b;
            best = i;
        }
    }
    Ok(Scored {
        field: name.to_string(),
        score,
        row: json!({
            "field": name,
            "score": round(score),
            "member": members.get(best).cloned().unwrap_or(Value::Null),
            "share_at_point": round(here.get(best).copied().unwrap_or(0.0)),
            "share_elsewhere": round(mean.get(best).copied().unwrap_or(0.0)),
            "members": members.len(),
        }),
    })
}

/// A score or a share to 4 decimals.
fn round(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}
