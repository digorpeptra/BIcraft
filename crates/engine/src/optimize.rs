//! Desktop's Optimize ribbon (`docs/gaps.md` G60; Microsoft Learn, "Optimize ribbon in Power BI
//! Desktop", `power-bi/create-reports/desktop-optimize-ribbon`, and "DirectQuery optimization
//! scenarios with the Optimize ribbon", `power-bi/create-reports/desktop-optimize-ribbon-scenarios`):
//!
//! - **Pause visuals** (`report.pause_visuals`): while the report is paused, a visual whose data
//!   would need an evaluation keeps the data it last showed, marked `pending`, or shows the
//!   `pending` status when it has none; nothing is evaluated for it. Learn: "When saving and
//!   closing a Power BI Desktop file with the report paused, the report remains paused when
//!   reopened", so it is a report setting, `report.json` `settings.pauseQueries` (public report
//!   schema 3.3.0), not view state.
//! - **Refresh visuals** (`report.refresh_visuals`): the next view of the report evaluates its
//!   visuals once and the report stays paused (the Performance analyzer's `perf.refresh` asks the
//!   same).
//! - **A visual's Refresh** (`visual.refresh`): the next view evaluates that visual once, the
//!   others staying paused. While paused, a measure-bound formatting value (a title or a
//!   button's text bound to a measure) waits like the data, and report-page tooltips are off.
//! - **Optimization presets** (`report.query_reduction`): Query reduction "turns off
//!   cross-highlighting and cross-filtering and adds an Apply button to the filter pane", with
//!   Apply buttons on the slicers (Learn's scenario); Interactivity is Desktop's default (all
//!   off); Customize sets each option. Written to `report.json` `slowDataSourceSettings`
//!   (`ExplorationSlowDataSourceSettings`).

use serde_json::{Map, Value, json};

use crate::Session;
use crate::params::{allow, opt_bool, opt_str};
use crate::report::parts;
use bicraft_report::Report;

const CROSS: &str = "isCrossHighlightingDisabled";
const SLICERS: &str = "isSlicerSelectionsButtonEnabled";
const FILTERS: &str = "isFilterSelectionsButtonEnabled";
const APPLY_ALL: &str = "isApplyAllButtonEnabled";

/// Whether the report's visual queries are paused (`settings.pauseQueries`).
pub(crate) fn paused(r: &Report) -> bool {
    r.extra
        .get("settings")
        .and_then(|s| s.get("pauseQueries"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn slow(r: &Report, key: &str) -> bool {
    r.extra
        .get("slowDataSourceSettings")
        .and_then(|s| s.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Whether the report turns off the default cross-highlighting and cross-filtering
/// (`slowDataSourceSettings.isCrossHighlightingDisabled`): a selection then reaches only the
/// visuals an Edit interactions rule names (Learn: the preset "only turns off default
/// cross-highlighting and cross-filtering").
pub(crate) fn interactions_off(r: &Report) -> bool {
    slow(r, CROSS)
}

/// Whether each slicer has its own Apply button (query reduction's "Add an Apply button to each
/// slicer", `slowDataSourceSettings.isSlicerSelectionsButtonEnabled`): a slicer's changes then
/// wait until it is applied (`view.interact` `slicer_apply`).
pub(crate) fn slicer_apply_buttons(r: &Report) -> bool {
    slow(r, SLICERS)
}

/// The query reduction options as `report.query_reduction` and the snapshot show them.
pub(crate) fn query_reduction_json(r: &Report) -> Value {
    let cross = slow(r, CROSS);
    let slicers = if slow(r, SLICERS) {
        "apply_button"
    } else {
        "instantly"
    };
    let filters = if slow(r, APPLY_ALL) {
        "single_button"
    } else if slow(r, FILTERS) {
        "apply_buttons"
    } else {
        "instantly"
    };
    let preset = match (cross, slicers, filters) {
        (true, "apply_button", "single_button") => "query_reduction",
        (false, "instantly", "instantly") => "interactivity",
        _ => "customized",
    };
    json!({
        "preset": preset,
        "disable_cross_highlighting": cross,
        "slicers": slicers,
        "filters": filters,
    })
}

/// `report.pause_visuals {paused?: bool (default true)}`.
pub(crate) fn pause_visuals(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["paused"])?;
    let on = opt_bool(p, "paused")?.unwrap_or(true);
    let (_, r) = parts(s)?;
    let mut settings = match r.extra.get("settings") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(m)) => m.clone(),
        Some(_) => return Err("the report's `settings` is not an object".into()),
    };
    let before = settings.clone();
    if on {
        settings.insert("pauseQueries".into(), json!(true));
    } else {
        settings.shift_remove("pauseQueries");
    }
    let changed = settings != before;
    if changed {
        if settings.is_empty() {
            r.extra.shift_remove("settings");
        } else {
            r.extra.insert("settings".into(), Value::Object(settings));
        }
        s.report_dirty = true;
    }
    Ok(json!({"paused": on, "changed": changed}))
}

/// `report.refresh_visuals {}`: the next view of the report evaluates its visuals once, paused or
/// not. Changes neither the report nor the model.
pub(crate) fn refresh_visuals(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &[])?;
    let r = s.report.as_ref().ok_or("the session has no report")?;
    let paused = paused(r);
    s.preview.request_refresh();
    Ok(json!({"paused": paused, "refresh": "next view"}))
}

/// `visual.refresh {page, visual}`: the next view evaluates that visual once, the report's
/// other visuals staying paused (the Refresh of a visual's pending-changes banner). Changes
/// neither the report nor the model.
pub(crate) fn refresh_visual(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual"])?;
    let r = s.report.as_ref().ok_or("the session has no report")?;
    let (page, v) = crate::interact::find_visual(r, p)?;
    let out = json!({"paused": paused(r), "page": page.name, "visual": v.name, "refresh": "next view"});
    s.preview.request_refresh_one(&page.name, &v.name);
    Ok(out)
}

/// `report.query_reduction {preset?, disable_cross_highlighting?, slicers?, filters?}`.
pub(crate) fn query_reduction(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["preset", "disable_cross_highlighting", "slicers", "filters"])?;
    let preset = opt_str(p, "preset")?;
    let (mut cross, mut slicers, mut filters) = match preset {
        None => (None, None, None),
        Some(x) => match x.to_ascii_lowercase().as_str() {
            "query_reduction" => (Some(true), Some(true), Some("single_button")),
            "interactivity" => (Some(false), Some(false), Some("instantly")),
            _ => return Err("`preset` must be \"query_reduction\" or \"interactivity\"".into()),
        },
    };
    if let Some(c) = opt_bool(p, "disable_cross_highlighting")? {
        cross = Some(c);
    }
    if let Some(x) = opt_str(p, "slicers")? {
        slicers = Some(match x {
            "instantly" => false,
            "apply_button" => true,
            _ => return Err("`slicers` must be \"instantly\" or \"apply_button\"".into()),
        });
    }
    if let Some(x) = opt_str(p, "filters")? {
        filters = Some(match x {
            "instantly" => "instantly",
            "apply_buttons" => "apply_buttons",
            "single_button" => "single_button",
            _ => {
                return Err("`filters` must be \"instantly\", \"apply_buttons\" or \"single_button\"".into());
            }
        });
    }
    let (_, r) = parts(s)?;
    let mut slow = match r.extra.get("slowDataSourceSettings") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(m)) => m.clone(),
        Some(_) => return Err("the report's `slowDataSourceSettings` is not an object".into()),
    };
    let before = slow.clone();
    // an option turned off leaves the file (Desktop's default)
    let mut set = |key: &str, on: bool| {
        if on {
            slow.insert(key.into(), json!(true));
        } else {
            slow.shift_remove(key);
        }
    };
    if let Some(c) = cross {
        set(CROSS, c);
    }
    if let Some(b) = slicers {
        set(SLICERS, b);
    }
    if let Some(f) = filters {
        set(FILTERS, f == "apply_buttons");
        set(APPLY_ALL, f == "single_button");
    }
    let changed = slow != before;
    if changed {
        if slow.is_empty() {
            r.extra.shift_remove("slowDataSourceSettings");
        } else {
            r.extra
                .insert("slowDataSourceSettings".into(), Value::Object(slow));
        }
        s.report_dirty = true;
    }
    let r = s.report.as_ref().ok_or("the session has no report")?;
    let mut out = query_reduction_json(r);
    if let Some(o) = out.as_object_mut() {
        o.insert("changed".into(), json!(changed));
    }
    Ok(out)
}
