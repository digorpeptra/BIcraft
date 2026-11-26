//! The report's settings (`report.settings`): Desktop's File › Options and settings › Options ›
//! Report settings of the current file, written to `report.json` `settings` (public report schema
//! 3.3.0, `ExplorationSettings`) and, for the Filters pane's visibility to readers, to
//! `report.json` `objects.outspacePane` (`ReportFormattingObjects`). A legacy layout keeps both
//! in its `config` (`docs/formats/legacy-layout-bb.md`). What each setting means is in
//! `docs/formats/pbir.md` ("Report settings").

use serde_json::{Map, Value, json};

use crate::Session;
use crate::params::{allow, opt_bool, opt_str};
use crate::report::parts;
use bicraft_report::Report;

/// `report.json` `settings`, when it is an object.
fn settings(r: &Report) -> Option<&Map<String, Value>> {
    r.extra.get("settings").and_then(Value::as_object)
}

/// A boolean of `report.json` `settings`.
fn flag(r: &Report, key: &str) -> Option<bool> {
    settings(r).and_then(|s| s.get(key)).and_then(Value::as_bool)
}

/// A formatting property's literal (`{"expr": {"Literal": {"Value": "false"}}}`).
fn literal(v: &Value) -> Option<&str> {
    v.pointer("/expr/Literal/Value").and_then(Value::as_str)
}

/// Whether readers see the Filters pane: `objects.outspacePane` `visible` (the pane's eye in
/// Desktop); a report without it shows the pane.
pub(crate) fn filter_pane_visible(r: &Report) -> bool {
    let entries = r
        .extra
        .get("objects")
        .and_then(|o| o.get("outspacePane"))
        .and_then(Value::as_array);
    !entries.into_iter().flatten().any(|e| {
        e.get("selector").is_none()
            && e.pointer("/properties/visible")
                .and_then(literal)
                .is_some_and(|v| v.eq_ignore_ascii_case("false"))
    })
}

/// Writes the Filters pane's visibility to readers into `objects.outspacePane` of `extra`:
/// `false` as the literal Desktop writes, `true` by leaving `visible` out (the default).
fn set_filter_pane_visible(extra: &mut Map<String, Value>, visible: bool) -> Result<(), String> {
    let mut objects = match extra.get("objects") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(m)) => m.clone(),
        Some(_) => return Err("the report's `objects` is not an object".into()),
    };
    let mut entries = match objects.get("outspacePane") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a.clone(),
        Some(_) => return Err("the report's `objects.outspacePane` is not a list".into()),
    };
    // the entry without a selector holds the pane's own properties
    let at = entries.iter().position(|e| e.get("selector").is_none());
    let i = match at {
        Some(i) => i,
        None if visible => return Ok(()),
        None => {
            entries.push(json!({"properties": {}}));
            entries.len().saturating_sub(1)
        }
    };
    let Some(entry) = entries.get_mut(i).and_then(Value::as_object_mut) else {
        return Err("the report's `objects.outspacePane` holds a value that is not an object".into());
    };
    let props = entry
        .entry("properties")
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(props) = props.as_object_mut() else {
        return Err("the report's `objects.outspacePane` properties are not an object".into());
    };
    if visible {
        props.shift_remove("visible");
    } else {
        props.insert("visible".into(), json!({"expr": {"Literal": {"Value": "false"}}}));
    }
    let empty = props.is_empty() && entry.len() == 1;
    if empty {
        entries.remove(i);
    }
    if entries.is_empty() {
        objects.shift_remove("outspacePane");
    } else {
        objects.insert("outspacePane".into(), Value::Array(entries));
    }
    if objects.is_empty() {
        extra.shift_remove("objects");
    } else {
        extra.insert("objects".into(), Value::Object(objects));
    }
    Ok(())
}

/// The settings as the snapshot and `report.settings` show them (without the export mode, which
/// each names its own way).
pub(crate) fn settings_json(r: &Report) -> Map<String, Value> {
    let cross = crate::interact::cross_filter_default(r);
    let mut o = Map::new();
    o.insert(
        "default_interaction".into(),
        json!(if cross { "filter" } else { "highlight" }),
    );
    o.insert(
        "drill_filters_other_visuals".into(),
        json!(crate::interact::drill_filters_default(r)),
    );
    o.insert(
        "persistent_filters".into(),
        json!(flag(r, "isPersistentUserStateDisabled") != Some(true)),
    );
    o.insert(
        "personalize_visuals".into(),
        json!(flag(r, "allowInlineExploration").unwrap_or(false)),
    );
    o.insert(
        "cross_report_drillthrough".into(),
        json!(flag(r, "useCrossReportDrillthrough").unwrap_or(false)),
    );
    o.insert("filter_pane_visible".into(), json!(filter_pane_visible(r)));
    o.insert(
        "allow_change_filter_types".into(),
        json!(flag(r, "allowChangeFilterTypes").unwrap_or(false)),
    );
    // the Optimize ribbon (optimize.rs)
    o.insert("paused".into(), json!(crate::optimize::paused(r)));
    o.insert("query_reduction".into(), crate::optimize::query_reduction_json(r));
    o
}

/// `report.settings {default_interaction?, drill_filters_other_visuals?, export_data?,
/// persistent_filters?, personalize_visuals?, cross_report_drillthrough?, filter_pane_visible?,
/// allow_change_filter_types?}`: the report's settings. "Change default visual interaction from
/// cross highlighting to cross filtering" is `defaultFilterActionIsDataFilter` (written only when
/// on); "Drilling filters other visuals" `defaultDrillFilterOtherVisuals`; the Export data setting
/// `exportDataMode`; persistent filters `isPersistentUserStateDisabled` (written only when they
/// are off); personalize visuals `allowInlineExploration`; cross-report drillthrough
/// `useCrossReportDrillthrough`; "Allow users to change filter types" `allowChangeFilterTypes`;
/// the Filters pane's visibility to readers `objects.outspacePane` `visible`. Without parameters
/// it changes nothing and returns the settings.
pub(crate) fn report_settings(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "default_interaction",
            "drill_filters_other_visuals",
            "export_data",
            "persistent_filters",
            "personalize_visuals",
            "cross_report_drillthrough",
            "filter_pane_visible",
            "allow_change_filter_types",
        ],
    )?;
    let default = match opt_str(p, "default_interaction")? {
        None => None,
        Some(t) => match t.to_ascii_lowercase().as_str() {
            "highlight" => Some(false),
            "filter" => Some(true),
            _ => return Err("`default_interaction` must be \"highlight\" or \"filter\"".into()),
        },
    };
    let drill = opt_bool(p, "drill_filters_other_visuals")?;
    // what readers may export from the visuals (`visual.export_data`, M11.21)
    let export = match opt_str(p, "export_data")? {
        None => None,
        Some(m) => Some(crate::export_data::Mode::parse(m).ok_or_else(|| {
            "`export_data` must be \"AllowSummarized\", \"AllowSummarizedAndUnderlying\" or \"None\""
                .to_string()
        })?),
    };
    let persistent = opt_bool(p, "persistent_filters")?;
    let flags = [
        ("allowInlineExploration", opt_bool(p, "personalize_visuals")?),
        (
            "useCrossReportDrillthrough",
            opt_bool(p, "cross_report_drillthrough")?,
        ),
        (
            "allowChangeFilterTypes",
            opt_bool(p, "allow_change_filter_types")?,
        ),
    ];
    let pane = opt_bool(p, "filter_pane_visible")?;
    let (_, r) = parts(s)?;
    let mut settings = match r.extra.get("settings") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(m)) => m.clone(),
        Some(_) => return Err("the report's `settings` is not an object".into()),
    };
    let before = settings.clone();
    if let Some(cross) = default {
        if cross {
            settings.insert("defaultFilterActionIsDataFilter".into(), json!(true));
        } else {
            settings.shift_remove("defaultFilterActionIsDataFilter");
        }
    }
    if let Some(d) = drill {
        settings.insert("defaultDrillFilterOtherVisuals".into(), json!(d));
    }
    if let Some(m) = export {
        settings.insert("exportDataMode".into(), json!(m.as_str()));
    }
    if let Some(on) = persistent {
        if on {
            settings.shift_remove("isPersistentUserStateDisabled");
        } else {
            settings.insert("isPersistentUserStateDisabled".into(), json!(true));
        }
    }
    for (key, value) in flags {
        if let Some(b) = value {
            settings.insert(key.into(), json!(b));
        }
    }
    let mut extra = r.extra.clone();
    if settings != before {
        if settings.is_empty() {
            extra.shift_remove("settings");
        } else {
            extra.insert("settings".into(), Value::Object(settings));
        }
    }
    if let Some(visible) = pane {
        set_filter_pane_visible(&mut extra, visible)?;
    }
    let changed = extra != r.extra;
    if changed {
        r.extra = extra;
        s.report_dirty = true;
    }
    let r = s.report.as_ref().ok_or("the session has no report")?;
    let mut out = settings_json(r);
    out.insert("changed".into(), json!(changed));
    out.insert(
        "export_data".into(),
        json!(crate::export_data::mode_of(r).as_str()),
    );
    Ok(Value::Object(out))
}
