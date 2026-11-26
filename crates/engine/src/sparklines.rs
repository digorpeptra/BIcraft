//! Two field options of a table or matrix, as Power BI Desktop's Format pane and field menu set
//! them (docs/formats/sparklines.md):
//!
//! - **Data format** (`visual.set_data_format`): a format string scoped to the visual, written as
//!   the projection's `format` (public `visualConfiguration` schema, `RoleProjection.format`:
//!   "format string scoped to the visual"). The visual's values are displayed with it; the
//!   model's own format string is not changed.
//! - **Sparklines** (`visual.add_sparkline`, `visual.edit_sparkline`): a projection whose field
//!   is `SparklineData {Measure, Groupings, ApplyCalculationGroupTo}` (public `semanticQuery`
//!   schema, `QuerySparklineDataExpression`; Desktop 2.159.1231.0 writes no
//!   `PointsPerSparkline`), with its line, colour and markers in the
//!   visual's `objects.sparklines` entry selected by the projection's `queryRef` (the
//!   `sparklines` properties of Microsoft's report theme schema). Microsoft Learn ("Create
//!   sparklines in a table or matrix"): tables and matrices, at most five per visual, up to 52
//!   points each.
//!
//! Each command checks everything first and edits the report once nothing can fail.

use bicraft_report::query::{self, Sparkline, sparkline, sparkline_field, visual_calc};
use bicraft_report::{FieldBinding, FieldRef, Visual};
use serde_json::{Map, Value, json};

use crate::Session;
use crate::params::{allow, opt_bool, opt_str, str_param};
use crate::refs::{ParsedField, field_text, parse_field, resolve_field, same_field};
use crate::report::{bindings_json, check_role, page_at_mut, page_index, parts, visual_index};
use crate::visual_props::{num_text, quote};
use crate::visual_series::{Check, EntryOp, apply_ops, color_value, literal, number_value};

/// The visual types that take sparklines (Microsoft Learn: tables and matrices).
pub(crate) const SPARKLINE_TYPES: &[&str] = &["tableEx", "pivotTable"];
/// Most sparklines per visual (Microsoft Learn: "up to five sparklines per visual").
pub(crate) const MAX_SPARKLINES: usize = 5;
/// The role a sparkline is bound to (the Values well of a table and of a matrix).
const SPARKLINE_ROLE: &str = "Values";
/// The format object of a sparkline's look.
pub(crate) const SPARKLINE_OBJECT: &str = "sparklines";
/// Longest visual-scoped format string (public schema: `maxLength` 255).
const MAX_FORMAT: usize = 255;
/// Most decimals a Data format writes.
const MAX_DECIMALS: u64 = 15;

/// `visual.set_data_format`'s format types.
pub(crate) const FORMAT_TYPES: &[&str] = &[
    "auto",
    "general",
    "whole",
    "decimal",
    "percentage",
    "currency",
    "scientific",
    "date",
    "custom",
];

/// Chart types of a sparkline (`sparklines.chartType`, report theme schema).
pub(crate) const CHART_TYPES: &[&str] = &["line", "column"];
/// Marker shapes (`sparklines.markerShape`, report theme schema).
pub(crate) const MARKER_SHAPES: &[&str] = &[
    "circle",
    "square",
    "diamond",
    "triangle",
    "x",
    "shortDash",
    "longDash",
    "plus",
];
/// The markers a sparkline shows, as bits of `sparklines.markers` ("Show these markers" in the
/// theme schema), as Power BI Desktop 2.159.1231.0's Sparklines card writes them (a double
/// literal such as `136D`; docs/formats/sparklines.md): every point, highest, lowest, first,
/// last, negative, isolated. Without `markers`, Desktop marks isolated points only.
pub(crate) const MARKERS: &[(&str, u64)] = &[
    ("all", 1),
    ("highest", 2),
    ("lowest", 4),
    ("first", 8),
    ("last", 16),
    ("negative", 32),
    ("isolated", 128),
];

/// The named date formats the card offers and the .NET standard format Desktop 2.159.1231.0
/// writes for each (`*14/03/2001 (Short Date)` is written `d`; a name such as `Short Date`
/// written as is draws as a custom code, "S12or Da e").
const DATE_CODES: &[(&str, &str)] = &[
    ("short date", "d"),
    ("long date", "D"),
    ("general date", "G"),
    ("long time", "T"),
    ("short time", "t"),
];

/// Summarizations of a column on a sparkline's Y axis (`QueryAggregateFunction` names).
pub(crate) const SUMMARIZATIONS: &[&str] = &[
    "Sum",
    "Average",
    "Min",
    "Max",
    "Count",
    "CountNonNull",
    "Median",
    "StandardDeviation",
    "Variance",
];

// ---------------------------------------------------------------------------------------------
// shared
// ---------------------------------------------------------------------------------------------

fn visual_mut<'r>(
    r: &'r mut bicraft_report::Report,
    page: &str,
    visual: &str,
) -> Result<(String, &'r mut Visual), String> {
    let pi = page_index(r, page)?;
    let page = page_at_mut(r, pi)?;
    let name = page.name.clone();
    let vi = visual_index(page, visual)?;
    let v = page
        .visuals
        .get_mut(vi)
        .ok_or("internal: visual index out of range")?;
    Ok((name, v))
}

/// `v` with its raw query regenerated from its fields.
fn sync(v: &mut Visual) {
    match query::effective(v) {
        Some(q) => {
            v.visual_extra.insert("query".into(), q);
        }
        None => {
            v.visual_extra.shift_remove("query");
        }
    }
}

/// The `queryRef` of binding `at` of `v` (whose query is in sync).
fn query_ref_at(v: &Visual, at: usize) -> Option<String> {
    let mut q = v.visual_extra.get("query")?.clone();
    query::projection_mut(&mut q, at)?
        .get("queryRef")?
        .as_str()
        .map(str::to_string)
}

// ---------------------------------------------------------------------------------------------
// Data format
// ---------------------------------------------------------------------------------------------

/// The number part of a format: `#,0.00` / `0.00` with `decimals` decimals.
fn number_part(decimals: u64, thousands: bool) -> String {
    let mut s = String::from(if thousands { "#,0" } else { "0" });
    if decimals > 0 {
        s.push('.');
        for _ in 0..decimals {
            s.push('0');
        }
    }
    s
}

/// A currency format for `symbol` around `number`. `$` and `€` take the three-section forms Power
/// BI Desktop 2.159.1231.0's card recognizes as Currency (`\$#,0.00;(\$#,0.00);\$#,0.00`, "$
/// English (United States)") and writes (`#,0.00 "€";-#,0.00 "€";#,0.00 "€"`, "€ Euro (123
/// €)"); another symbol goes before the number (`£#,0.00`, `"CHF "#,0.00`), which Desktop draws
/// and shows as Custom.
fn currency_format(symbol: &str, number: &str) -> String {
    let mut chars = symbol.chars();
    match (chars.next(), chars.next()) {
        (Some('$'), None) => format!("\\${number};(\\${number});\\${number}"),
        (Some('€'), None) => {
            let n = format!("{number} \"€\"");
            format!("{n};-{n};{n}")
        }
        (Some(c), None) if !c.is_ascii_alphanumeric() && c != '"' && c != '\\' => {
            format!("{c}{number}")
        }
        _ => format!("\"{} \"{number}", symbol.replace('"', "")),
    }
}

fn opt_u64(p: &Value, key: &str, max: u64) -> Result<Option<u64>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_u64() {
            Some(n) if n <= max => Ok(Some(n)),
            _ => Err(format!("`{key}` must be a whole number from 0 to {max}")),
        },
    }
}

fn check_format_text(s: &str, what: &str) -> Result<(), String> {
    if s.chars().count() > MAX_FORMAT || s.chars().any(char::is_control) {
        return Err(format!(
            "`{what}` must be at most {MAX_FORMAT} characters, without control characters"
        ));
    }
    Ok(())
}

/// The format string `p` asks for (`None`: no visual format, the model's applies).
pub(crate) fn format_code(p: &Value) -> Result<Option<String>, String> {
    let custom = match p.get("format") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(s)) if s.trim().is_empty() => Some(None),
        Some(Value::String(s)) => {
            check_format_text(s, "format")?;
            Some(Some(s.clone()))
        }
        Some(_) => return Err("`format` must be a format string or null".into()),
    };
    let kind = opt_str(p, "type")?.map(str::to_ascii_lowercase);
    let decimals = opt_u64(p, "decimals", MAX_DECIMALS)?;
    let thousands = opt_bool(p, "thousands")?;
    let symbol = opt_str(p, "currency_symbol")?;
    let date = opt_str(p, "date_format")?;
    match (custom, kind.as_deref()) {
        (Some(_), Some(k)) if k != "custom" => {
            return Err("give `format` (a custom format string) or `type`, not both".into());
        }
        (Some(f), _) => return Ok(f),
        (None, None | Some("custom")) => {
            return Err("give `type` (auto, general, whole, decimal, percentage, currency, scientific, date) or `format` (custom)".into());
        }
        _ => {}
    }
    let kind = kind.unwrap_or_default();
    let numeric = matches!(
        kind.as_str(),
        "decimal" | "percentage" | "currency" | "scientific"
    );
    if decimals.is_some() && !numeric {
        return Err(format!("`decimals` does not apply to type {kind}"));
    }
    if thousands.is_some() && !matches!(kind.as_str(), "whole" | "decimal" | "percentage" | "currency") {
        return Err(format!("`thousands` does not apply to type {kind}"));
    }
    if symbol.is_some() && kind != "currency" {
        return Err("`currency_symbol` applies to type currency only".into());
    }
    if date.is_some() && kind != "date" {
        return Err("`date_format` applies to type date only".into());
    }
    let d = decimals.unwrap_or(2);
    // Desktop's card starts a percentage without the thousands separator
    let group = thousands.unwrap_or(kind != "percentage");
    Ok(Some(match kind.as_str() {
        "auto" => return Ok(None),
        // as Desktop 2.159.1231.0's card writes them: `G`, `0.00E+000`
        "general" => "G".to_string(),
        "whole" => number_part(0, group),
        "decimal" => number_part(d, group),
        "percentage" => format!("{}%", number_part(d, group)),
        "currency" => {
            let sym = symbol.unwrap_or("$");
            if sym.is_empty() || sym.chars().count() > 10 || sym.chars().any(char::is_control) {
                return Err("`currency_symbol` must be 1 to 10 characters".into());
            }
            currency_format(sym, &number_part(d, group))
        }
        "scientific" => format!("{}E+000", number_part(d, false)),
        "date" => {
            let f = date.unwrap_or("Short Date");
            check_format_text(f, "date_format")?;
            if f.trim().is_empty() {
                return Err("`date_format` is empty".into());
            }
            let named = f.trim().to_ascii_lowercase();
            DATE_CODES
                .iter()
                .find(|(n, _)| *n == named)
                .map_or_else(|| f.to_string(), |(_, code)| (*code).to_string())
        }
        other => {
            return Err(format!(
                "`type` must be one of {} (`{other}` is not)",
                FORMAT_TYPES.join(", ")
            ));
        }
    }))
}

/// `visual.set_data_format {page, visual, role, field? | index?, type? | format?, decimals?,
/// thousands?, currency_symbol?, date_format?}`: Format pane › General › Data format.
pub(crate) fn visual_set_data_format(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "page",
            "visual",
            "role",
            "field",
            "index",
            "type",
            "format",
            "decimals",
            "thousands",
            "currency_symbol",
            "date_format",
        ],
    )?;
    let key = str_param(p, "page")?;
    let visual = str_param(p, "visual")?;
    let role = str_param(p, "role")?;
    check_role(role)?;
    let code = format_code(p)?;
    let (_, r) = parts(s)?;
    let (page, v) = visual_mut(r, key, visual)?;
    let at = crate::visual_fields::source_position(v, p, role)?;
    let b = v
        .fields
        .get(at)
        .ok_or("internal: binding index out of range")?
        .clone();
    if sparkline(&b.field).is_some() {
        return Err("a sparkline has no data format: its look is in the Sparklines card".into());
    }
    if matches!(b.field, FieldRef::Other { .. }) && visual_calc(&b.field).is_none() {
        return Err(format!(
            "{} cannot take a data format here (a field parameter or an unmodelled field)",
            field_text(&b.field)
        ));
    }
    let mut nv = v.clone();
    sync(&mut nv);
    let mut q = nv
        .visual_extra
        .get("query")
        .cloned()
        .ok_or("internal: a visual with fields has a query")?;
    let proj = query::projection_mut(&mut q, at).ok_or("internal: the field is not in the visual's query")?;
    let old = proj.get("format").and_then(Value::as_str).map(str::to_string);
    if old.as_deref().filter(|s| !s.is_empty()) == code.as_deref() {
        return Err(match &code {
            Some(c) => format!(
                "{} already has data format `{c}` in this visual",
                field_text(&b.field)
            ),
            None => format!(
                "{} has no data format of its own in this visual",
                field_text(&b.field)
            ),
        });
    }
    match &code {
        Some(c) => {
            proj.insert("format".into(), json!(c));
        }
        None => {
            proj.shift_remove("format");
        }
    }
    nv.visual_extra.insert("query".into(), q);
    if query::derive(nv.visual_extra.get("query")) != nv.fields {
        return Err("internal: the visual's query no longer matches its fields".into());
    }
    *v = nv;
    let out = json!({
        "page": page,
        "visual": v.name,
        "role": b.role,
        "field": field_text(&b.field),
        "format": code,
        "fields": bindings_json(&v.fields),
    });
    s.report_dirty = true;
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Sparklines
// ---------------------------------------------------------------------------------------------

/// The sparkline's Y axis from `y` and `summarization`: a measure, or a column summarized
/// (`Sum` by default).
fn y_axis(
    model: Option<&bicraft_model::SemanticModel>,
    y: &str,
    summ: Option<&str>,
    warnings: &mut Vec<String>,
) -> Result<FieldRef, String> {
    let r = resolve_field(model, y)?;
    warnings.extend(r.warnings);
    let function = match summ {
        None => None,
        Some(f) => match parse_field(&format!("{f}(T[C])")) {
            Ok(ParsedField::Aggregation { function, .. }) => Some(function.to_string()),
            _ => {
                return Err(format!(
                    "`summarization` must be one of {} (`{f}` is not)",
                    SUMMARIZATIONS.join(", ")
                ));
            }
        },
    };
    match (r.field, function) {
        (FieldRef::Measure { .. }, Some(_)) => {
            Err("a measure is not summarized again: leave `summarization` out".into())
        }
        (m @ FieldRef::Measure { .. }, None) => Ok(m),
        (FieldRef::Column { table, column }, f) => Ok(FieldRef::Aggregation {
            table,
            column,
            function: f.unwrap_or_else(|| "Sum".into()),
        }),
        (
            FieldRef::Aggregation {
                table,
                column,
                function,
            },
            f,
        ) => Ok(FieldRef::Aggregation {
            table,
            column,
            function: f.unwrap_or(function),
        }),
        (other, _) => Err(format!("{} cannot be a sparkline's Y axis", field_text(&other))),
    }
}

fn x_axis(
    model: Option<&bicraft_model::SemanticModel>,
    x: &str,
    warnings: &mut Vec<String>,
) -> Result<FieldRef, String> {
    let r = resolve_field(model, x)?;
    warnings.extend(r.warnings);
    match r.field {
        c @ FieldRef::Column { .. } => Ok(c),
        other => Err(format!(
            "a sparkline's X axis is a column (typically a date): {} is not one",
            field_text(&other)
        )),
    }
}

fn apply_to(p: &Value) -> Result<Option<Option<String>>, String> {
    match p.get("apply_calculation_group_to") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) => match s.to_ascii_lowercase().as_str() {
            "sparkline" | "whole" | "entire" => Ok(Some(Some("Sparkline".into()))),
            "point" | "individual" => Ok(Some(Some("Point".into()))),
            _ => Err("`apply_calculation_group_to` must be sparkline (the whole sparkline) or point (individual values)".into()),
        },
        Some(_) => Err("`apply_calculation_group_to` must be a string or null".into()),
    }
}

/// The `objects.sparklines` edits of `p`'s look parameters, for the sparkline `query_ref`.
fn style_ops(p: &Value, query_ref: &str) -> Result<Vec<EntryOp>, String> {
    let sel = vec![json!({"metadata": query_ref})];
    let mut ops = Vec::new();
    let mut put = |prop: &str, value: Option<Value>, shown: String| {
        ops.push(match value {
            Some(value) => EntryOp::Set {
                object: SPARKLINE_OBJECT.into(),
                selectors: sel.clone(),
                prop: prop.into(),
                value,
                label: format!("sparkline {query_ref}: {prop} = {shown}"),
                check: Check::None,
            },
            None => EntryOp::Remove {
                object: SPARKLINE_OBJECT.into(),
                selectors: sel.clone(),
                prop: Some(prop.into()),
                label: format!("sparkline {query_ref}: {prop} removed"),
            },
        });
    };
    if let Some(v) = p.get("chart_type") {
        match v {
            Value::Null => put("chartType", None, String::new()),
            Value::String(t) if CHART_TYPES.contains(&t.to_ascii_lowercase().as_str()) => {
                let t = t.to_ascii_lowercase();
                put("chartType", Some(literal(quote(&t))), t);
            }
            _ => return Err(format!("`chart_type` must be one of {}", CHART_TYPES.join(", "))),
        }
    }
    for (key, prop) in [("color", "dataColor"), ("marker_color", "markerColor")] {
        match p.get(key) {
            None => {}
            Some(Value::Null) => put(prop, None, String::new()),
            Some(Value::String(c)) if crate::visual_props::is_color(c) => {
                put(prop, Some(color_value(c)), c.clone());
            }
            Some(_) => return Err(format!("`{key}` must be a colour like #118DFF, or null")),
        }
    }
    for (key, prop, lo, hi) in [
        ("width", "strokeWidth", 0.5, 10.0),
        ("marker_size", "markerSize", 1.0, 50.0),
    ] {
        match p.get(key) {
            None => {}
            Some(Value::Null) => put(prop, None, String::new()),
            Some(v) => match v.as_f64() {
                Some(n) if n.is_finite() && (lo..=hi).contains(&n) => {
                    put(prop, Some(number_value(n, false)), num_text(n));
                }
                _ => return Err(format!("`{key}` must be a number from {lo} to {hi}, or null")),
            },
        }
    }
    match p.get("marker_shape") {
        None => {}
        Some(Value::Null) => put("markerShape", None, String::new()),
        Some(Value::String(m)) if MARKER_SHAPES.iter().any(|x| x.eq_ignore_ascii_case(m)) => {
            let m = MARKER_SHAPES
                .iter()
                .find(|x| x.eq_ignore_ascii_case(m))
                .copied()
                .unwrap_or("circle");
            put("markerShape", Some(literal(quote(m))), m.into());
        }
        Some(_) => {
            return Err(format!(
                "`marker_shape` must be one of {}",
                MARKER_SHAPES.join(", ")
            ));
        }
    }
    match p.get("markers") {
        None => {}
        Some(Value::Null) => put("markers", None, String::new()),
        Some(Value::Array(a)) => {
            let mut bits = 0u64;
            for m in a.iter().take(MARKERS.len().saturating_add(1)) {
                let name = m.as_str().unwrap_or_default().to_ascii_lowercase();
                let bit = MARKERS
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, b)| *b)
                    .ok_or_else(|| {
                        format!(
                            "`markers` lists {} (`{name}` is not one)",
                            MARKERS.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
                        )
                    })?;
                bits |= bit;
            }
            if a.len() > MARKERS.len() {
                return Err("`markers` lists each kind at most once".into());
            }
            put("markers", Some(literal(format!("{bits}D"))), bits.to_string());
        }
        Some(_) => return Err("`markers` must be a list like [\"highest\", \"last\"], or null".into()),
    }
    Ok(ops)
}

const STYLE_KEYS: &[&str] = &[
    "chart_type",
    "color",
    "width",
    "markers",
    "marker_color",
    "marker_shape",
    "marker_size",
];

fn check_type(v: &Visual) -> Result<(), String> {
    if !v
        .visual_type
        .as_deref()
        .is_some_and(|t| SPARKLINE_TYPES.contains(&t))
    {
        return Err(format!(
            "sparklines go in a table or a matrix: visual `{}` is a {}",
            v.name,
            v.visual_type.as_deref().unwrap_or("group")
        ));
    }
    Ok(())
}

/// The positions of `v`'s sparklines among its fields.
fn sparkline_positions(v: &Visual) -> Vec<usize> {
    v.fields
        .iter()
        .enumerate()
        .filter(|(_, b)| sparkline(&b.field).is_some())
        .map(|(i, _)| i)
        .collect()
}

fn result(
    page: &str,
    v: &Visual,
    at: usize,
    query_ref: Option<String>,
    changes: Vec<String>,
    warnings: Vec<String>,
) -> Value {
    let s = v.fields.get(at).and_then(|b| sparkline(&b.field));
    json!({
        "page": page,
        "visual": v.name,
        "index": sparkline_positions(v).iter().position(|i| *i == at),
        "field": v.fields.get(at).map(|b| field_text(&b.field)),
        "y": s.as_ref().map(|s| field_text(&s.measure)),
        "x": s.as_ref().and_then(|s| s.groupings.first()).map(field_text),
        "query_ref": query_ref,
        "changes": changes,
        "fields": bindings_json(&v.fields),
        "warnings": warnings,
    })
}

/// `visual.add_sparkline {page, visual, y, summarization?, x, apply_calculation_group_to?,
/// chart_type?, color?, width?, markers?, marker_color?, marker_shape?, marker_size?}`.
pub(crate) fn visual_add_sparkline(s: &mut Session, p: &Value) -> Result<Value, String> {
    let mut keys = vec![
        "page",
        "visual",
        "y",
        "summarization",
        "x",
        "apply_calculation_group_to",
    ];
    keys.extend_from_slice(STYLE_KEYS);
    allow(p, &keys)?;
    let page = str_param(p, "page")?;
    let visual = str_param(p, "visual")?;
    let y = str_param(p, "y")?;
    let x = str_param(p, "x")?;
    let summ = opt_str(p, "summarization")?;
    // Desktop 2.159.1231.0's Add dialog starts on "Individual values" and writes `Point`; null
    // leaves it out (the schema's default, the whole sparkline)
    let calc_to = apply_to(p)?.unwrap_or_else(|| Some("Point".into()));
    let (model, r) = parts(s)?;
    let mut warnings = Vec::new();
    let measure = y_axis(model, y, summ, &mut warnings)?;
    let x = x_axis(model, x, &mut warnings)?;
    let (page, v) = visual_mut(r, page, visual)?;
    check_type(v)?;
    let existing = sparkline_positions(v);
    if existing.len() >= MAX_SPARKLINES {
        return Err(format!(
            "visual `{}` already has {MAX_SPARKLINES} sparklines, the most Power BI shows",
            v.name
        ));
    }
    let spark = Sparkline {
        measure,
        groupings: vec![x],
        apply_calc_group_to: calc_to,
    };
    let field = sparkline_field(&spark);
    if existing.iter().any(|i| {
        v.fields
            .get(*i)
            .and_then(|b| sparkline(&b.field))
            .is_some_and(|o| same_field(&o.measure, &spark.measure) && o.groupings == spark.groupings)
    }) {
        return Err(format!(
            "visual `{}` already has a sparkline of {} by {}",
            v.name,
            field_text(&spark.measure),
            spark.groupings.first().map(field_text).unwrap_or_default()
        ));
    }
    let at = v
        .fields
        .iter()
        .rposition(|b| b.role.eq_ignore_ascii_case(SPARKLINE_ROLE))
        .map_or(v.fields.len(), |i| i.saturating_add(1));
    let mut nv = v.clone();
    nv.fields.insert(
        at,
        FieldBinding {
            role: SPARKLINE_ROLE.into(),
            field,
            display_name: None,
            show_as: None,
        },
    );
    sync(&mut nv);
    let qr = query_ref_at(&nv, at).ok_or("internal: the sparkline is not in the visual's query")?;
    let mut changes = vec![format!("sparkline {qr} added")];
    apply_ops(&mut nv, &style_ops(p, &qr)?, false, &mut changes)?;
    *v = nv;
    let out = result(&page, v, at, Some(qr), changes, warnings);
    s.report_dirty = true;
    Ok(out)
}

/// `visual.edit_sparkline {page, visual, index?, y?, summarization?, x?,
/// apply_calculation_group_to?, chart_type?, color?, width?, markers?, marker_color?,
/// marker_shape?, marker_size?}`: the sparkline keeps its `queryRef`, so its look stays with it.
pub(crate) fn visual_edit_sparkline(s: &mut Session, p: &Value) -> Result<Value, String> {
    let mut keys = vec![
        "page",
        "visual",
        "index",
        "y",
        "summarization",
        "x",
        "apply_calculation_group_to",
    ];
    keys.extend_from_slice(STYLE_KEYS);
    allow(p, &keys)?;
    let page = str_param(p, "page")?;
    let visual = str_param(p, "visual")?;
    let index = crate::report::opt_index(p, "index")?;
    let y = opt_str(p, "y")?;
    let summ = opt_str(p, "summarization")?;
    let x = opt_str(p, "x")?;
    let calc_to = apply_to(p)?;
    if y.is_none()
        && summ.is_none()
        && x.is_none()
        && calc_to.is_none()
        && !STYLE_KEYS.iter().any(|k| p.get(*k).is_some())
    {
        return Err("nothing to change: give y, summarization, x or a look parameter".into());
    }
    let (model, r) = parts(s)?;
    let mut warnings = Vec::new();
    let (page, v) = visual_mut(r, page, visual)?;
    let positions = sparkline_positions(v);
    let at = match (index, positions.len()) {
        (_, 0) => return Err(format!("visual `{}` has no sparkline", v.name)),
        (None, 1) => positions.first().copied().unwrap_or(0),
        (None, n) => {
            return Err(format!(
                "visual `{}` has {n} sparklines: give `index` (0 to {})",
                v.name,
                n.saturating_sub(1)
            ));
        }
        (Some(i), n) => positions
            .get(i)
            .copied()
            .ok_or_else(|| format!("index {i} is past the visual's sparklines (it has {n})"))?,
    };
    let old = v
        .fields
        .get(at)
        .and_then(|b| sparkline(&b.field))
        .ok_or("internal: not a sparkline")?;
    let mut spark = old.clone();
    if y.is_some() || summ.is_some() {
        let base = match (y, &old.measure) {
            (Some(t), _) => t.to_string(),
            (None, FieldRef::Aggregation { table, column, .. }) => crate::edit::dax_column(table, column),
            (None, other) => field_text(other),
        };
        spark.measure = y_axis(model, &base, summ, &mut warnings)?;
    }
    if let Some(t) = x {
        spark.groupings = vec![x_axis(model, t, &mut warnings)?];
    }
    if let Some(c) = calc_to {
        spark.apply_calc_group_to = c;
    }
    let mut nv = v.clone();
    sync(&mut nv);
    let qr = query_ref_at(&nv, at).ok_or("internal: the sparkline is not in the visual's query")?;
    let mut changes = Vec::new();
    if spark != old {
        let field = sparkline_field(&spark);
        let mut q = nv
            .visual_extra
            .get("query")
            .cloned()
            .ok_or("internal: a visual with fields has a query")?;
        let proj = query::projection_mut(&mut q, at)
            .ok_or("internal: the sparkline is not in the visual's query")?;
        proj.insert("field".into(), query::field_json(&field));
        nv.visual_extra.insert("query".into(), q);
        if let Some(b) = nv.fields.get_mut(at) {
            b.field = field;
        }
        changes.push(format!(
            "sparkline {qr}: {} by {}",
            field_text(&spark.measure),
            spark.groupings.first().map(field_text).unwrap_or_default()
        ));
    }
    apply_ops(&mut nv, &style_ops(p, &qr)?, false, &mut changes)?;
    if changes.is_empty() {
        return Err("the sparkline is already like that".into());
    }
    if query::derive(nv.visual_extra.get("query")) != nv.fields {
        return Err("internal: the visual's query no longer matches its fields".into());
    }
    *v = nv;
    let out = result(&page, v, at, Some(qr), changes, warnings);
    s.report_dirty = true;
    Ok(out)
}

/// The look of the sparkline `query_ref` of `v`, from `objects.sparklines` (its own entry, over
/// an entry without selector), as the preview's data column carries it:
/// `{chart_type, color, width, markers, marker_color, marker_shape, marker_size}`.
pub(crate) fn style_of(v: &Visual, query_ref: &str) -> Map<String, Value> {
    let mut out = Map::new();
    let entries = v
        .visual_extra
        .get("objects")
        .and_then(|o| o.get(SPARKLINE_OBJECT))
        .and_then(Value::as_array);
    let Some(entries) = entries else {
        return out;
    };
    let general = entries.iter().filter(|e| e.get("selector").is_none());
    let own = entries.iter().filter(|e| {
        e.get("selector")
            .and_then(|s| s.get("metadata"))
            .and_then(Value::as_str)
            == Some(query_ref)
    });
    for e in general.chain(own).take(20) {
        let Some(props) = e.get("properties").and_then(Value::as_object) else {
            continue;
        };
        for (prop, key) in [("chartType", "chart_type"), ("markerShape", "marker_shape")] {
            if let Some(crate::visual_props::Lit::Text(t)) =
                props.get(prop).and_then(crate::visual_props::decode_literal)
            {
                out.insert(key.into(), json!(t));
            }
        }
        for (prop, key) in [("dataColor", "color"), ("markerColor", "marker_color")] {
            if let Some(c) = props.get(prop).and_then(crate::visual_props::color_text) {
                out.insert(key.into(), json!(c));
            }
        }
        for (prop, key) in [
            ("strokeWidth", "width"),
            ("markerSize", "marker_size"),
            ("markers", "markers"),
        ] {
            if let Some(crate::visual_props::Lit::Num(n, _)) =
                props.get(prop).and_then(crate::visual_props::decode_literal)
            {
                out.insert(key.into(), json!(n));
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "sparklines_tests.rs"]
mod tests;
