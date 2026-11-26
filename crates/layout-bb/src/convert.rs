//! Turning a report read from a legacy layout into pure PBIR, as Power BI Desktop does when it
//! upgrades a legacy report on save (black-box LO-* observations).

use bicraft_report::Report;
use serde_json::{Map, Value, json};

use crate::LEGACY_PREFIX;
use crate::codes::{self, DATA_TYPE};
use crate::path::MAX_DEPTH;

/// Built-in visual roles in which Desktop marks the projections `active` when it converts a
/// legacy report (the drill-down roles). Observed: Desktop added the marker (LO-visuals: slicer
/// `Values`, scatterChart `X`, treemap `Group`, pivotTable `Columns`; LO-filters, LO-mobile: slicer
/// `Values`) or the legacy input already had it on the first projection and Desktop wrote `false`
/// on the others (LO-visuals: stackedColumnChart `Category`, pivotTable `Rows`). The other rows
/// had `active: true` in every legacy input (LO-visuals, LO-pages, LO-groups) and are assumed to
/// behave the same.
const DRILL_ROLES: &[(&str, &str)] = &[
    ("areaChart", "Category"),
    ("barChart", "Category"),
    ("clusteredBarChart", "Category"),
    ("clusteredColumnChart", "Category"),
    ("donutChart", "Category"),
    ("funnel", "Category"),
    ("lineChart", "Category"),
    ("lineClusteredColumnComboChart", "Category"),
    ("pieChart", "Category"),
    ("pivotTable", "Columns"),
    ("pivotTable", "Rows"),
    ("ribbonChart", "Category"),
    ("scatterChart", "Category"),
    ("scatterChart", "X"),
    ("slicer", "Values"),
    ("stackedColumnChart", "Category"),
    ("treemap", "Group"),
    ("waterfallChart", "Category"),
];

/// Removes every legacy-only key (those starting with [`LEGACY_PREFIX`]) and applies Desktop's
/// conversion rules that have no legacy counterpart: drill-role `active` markers and the
/// `useEnhancedTooltips` setting (see the format note). Call it after taking the legacy keys the
/// caller converts itself (phone layouts, bookmark list, report measures).
pub fn strip_legacy(report: &mut Report) {
    let strip = |m: &mut Map<String, Value>| m.retain(|k, _| !k.starts_with(LEGACY_PREFIX));
    strip(&mut report.extra);
    strip(&mut report.pages_extra);
    strip(&mut report.version_extra);
    for page in &mut report.pages {
        strip(&mut page.extra);
        for v in &mut page.visuals {
            strip(&mut v.extra);
            strip(&mut v.visual_extra);
            mark_active(v);
            if let Some(e) = v.visual_extra.get_mut("expansionStates") {
                *e = clean_expansion(e, 0);
            }
        }
    }
    for b in &mut report.bookmarks {
        add_visual_containers(&mut b.document);
    }
    // Without `useStylableVisualContainerHeader: true`, Desktop writes no visual groups: it removes
    // the group containers and places each member at its position plus the positions of all its
    // groups (LO-visuals; LX-g1, g2, g3, g6, g7, g9). With the setting the groups stay as they are
    // (LO-groups, LX-g4, g5, g8).
    let stylable = report
        .extra
        .get("settings")
        .and_then(|s| s.get("useStylableVisualContainerHeader"))
        == Some(&Value::Bool(true));
    if !stylable {
        for page in &mut report.pages {
            flatten_groups(page);
        }
    }
    // A legacy `settings` object without `useEnhancedTooltips` comes out of Desktop with
    // `useEnhancedTooltips: false` (LO-groups, LX-x1, LX-x3); with the setting, Desktop keeps it
    // (LO-report, LX-x2).
    if let Some(Value::Object(s)) = report.extra.get_mut("settings")
        && !s.contains_key("useEnhancedTooltips")
    {
        s.insert("useEnhancedTooltips".into(), json!(false));
    }
}

fn flatten_groups(page: &mut bicraft_report::Page) {
    let groups: Vec<(String, f64, f64, Option<String>)> = page
        .visuals
        .iter()
        .filter(|v| v.extra.contains_key("visualGroup"))
        .map(|v| (v.name.clone(), v.position.x, v.position.y, v.parent_group.clone()))
        .collect();
    if groups.is_empty() {
        return;
    }
    for v in &mut page.visuals {
        let mut parent = v.parent_group.take();
        // Bounded: a cycle of groups is walked at most once per group.
        let mut steps = 0usize;
        while let Some(p) = parent {
            steps = steps.saturating_add(1);
            match groups.iter().find(|g| g.0 == p) {
                Some((_, x, y, up)) if steps <= groups.len() => {
                    v.position.x += x;
                    v.position.y += y;
                    parent = up.clone();
                }
                _ => break,
            }
        }
    }
    page.visuals.retain(|v| !v.extra.contains_key("visualGroup"));
}

/// Desktop writes an empty `visualContainers` in each bookmark exploration section that has
/// none (LO-bookmarks).
fn add_visual_containers(doc: &mut Value) {
    if let Some(Value::Object(sections)) = doc
        .get_mut("explorationState")
        .and_then(|e| e.get_mut("sections"))
    {
        for s in sections.values_mut() {
            if let Value::Object(so) = s
                && !so.contains_key("visualContainers")
            {
                so.insert("visualContainers".into(), json!({}));
            }
        }
    }
}

/// Desktop drops `identityValues: null` from expansion states (LO-visuals).
fn clean_expansion(v: &Value, depth: usize) -> Value {
    if depth > MAX_DEPTH {
        return v.clone();
    }
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .filter(|(k, x)| !(k.as_str() == "identityValues" && x.is_null()))
                .map(|(k, x)| (k.clone(), clean_expansion(x, depth + 1)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(|x| clean_expansion(x, depth + 1)).collect()),
        other => other.clone(),
    }
}

fn mark_active(v: &mut bicraft_report::Visual) {
    let Some(ty) = v.visual_type.clone() else {
        return;
    };
    let Some(Value::Object(state)) = v
        .visual_extra
        .get_mut("query")
        .and_then(|q| q.get_mut("queryState"))
    else {
        return;
    };
    for (role, rs) in state.iter_mut() {
        if !DRILL_ROLES.iter().any(|(t, r)| *t == ty && r == role) {
            continue;
        }
        let Some(Value::Array(list)) = rs.get_mut("projections") else {
            continue;
        };
        let any_active = list
            .iter()
            .any(|p| p.get("active").and_then(Value::as_bool) == Some(true));
        for (i, p) in list.iter_mut().enumerate() {
            if let Value::Object(m) = p
                && !m.contains_key("active")
            {
                m.insert("active".into(), json!(i == 0 && !any_active));
            }
        }
    }
}

/// The PBIR `reportExtensions.json` content (without `$schema`) of a legacy
/// `config.modelExtensions` value (an array, or its JSON text): the report-level measures.
/// `None` when the value has a shape we have not seen Desktop convert.
pub fn report_extensions(legacy: &Value) -> Option<Value> {
    let parsed;
    let list = match legacy {
        Value::Array(a) => a,
        Value::String(s) => {
            parsed = serde_json::from_str::<Value>(s).ok()?;
            parsed.as_array()?
        }
        _ => return None,
    };
    let [ext] = list.as_slice() else {
        return None;
    };
    let ext = ext.as_object()?;
    if ext.keys().any(|k| k != "name" && k != "entities") {
        return None;
    }
    let mut entities = Vec::new();
    for e in ext
        .get("entities")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let e = e.as_object()?;
        if e.keys()
            .any(|k| !matches!(k.as_str(), "name" | "extends" | "measures"))
        {
            return None;
        }
        let name = e.get("name").or_else(|| e.get("extends"))?.as_str()?;
        let mut measures = Vec::new();
        for m in e.get("measures").and_then(Value::as_array).into_iter().flatten() {
            measures.push(measure(m.as_object()?)?);
        }
        let mut out = Map::new();
        out.insert("name".into(), json!(name));
        if !measures.is_empty() {
            out.insert("measures".into(), Value::Array(measures));
        }
        entities.push(Value::Object(out));
    }
    let mut doc = Map::new();
    doc.insert(
        "name".into(),
        ext.get("name").cloned().unwrap_or_else(|| json!("extension")),
    );
    doc.insert("entities".into(), Value::Array(entities));
    Some(Value::Object(doc))
}

/// One legacy report measure as PBIR (LO-report): `dataType` spelled, `formatInformation`
/// reduced to its `formatString`, empty or default keys left out.
fn measure(m: &Map<String, Value>) -> Option<Value> {
    let mut out = Map::new();
    out.insert("name".into(), m.get("name").filter(|v| v.is_string())?.clone());
    let dt = codes::name(DATA_TYPE, m.get("dataType")?)?;
    out.insert("dataType".into(), json!(dt));
    out.insert(
        "expression".into(),
        m.get("expression").filter(|v| v.is_string())?.clone(),
    );
    for (k, v) in m {
        match (k.as_str(), v) {
            ("name" | "dataType" | "expression", _) => {}
            ("errorMessage" | "formulaOverride", Value::Null) => {}
            ("hidden", Value::Bool(false)) => {}
            ("hidden", Value::Bool(true)) => {
                out.insert("hidden".into(), json!(true));
            }
            ("formatInformation", Value::Object(f)) => {
                for (fk, fv) in f {
                    match (fk.as_str(), fv) {
                        ("formatString", Value::String(_)) => {
                            out.insert("formatString".into(), fv.clone());
                        }
                        ("format", Value::String(_)) => {}
                        _ => return None,
                    }
                }
            }
            ("description" | "displayFolder" | "dataCategory", Value::String(_)) => {
                out.insert(k.clone(), v.clone());
            }
            _ => return None,
        }
    }
    Some(Value::Object(out))
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn report_measures_convert_like_the_observed_one() {
        let legacy = json!([{"name": "extension", "entities": [{"name": "Sales", "extends": "Sales",
            "measures": [{"name": "M", "dataType": 3, "expression": "1", "errorMessage": null,
            "hidden": false, "formulaOverride": null,
            "formatInformation": {"formatString": "#,0", "format": "General"}}]}]}]);
        let doc = report_extensions(&legacy).unwrap();
        assert_eq!(
            doc,
            json!({"name": "extension", "entities": [{"name": "Sales", "measures": [
                {"name": "M", "dataType": "Double", "expression": "1", "formatString": "#,0"}]}]})
        );
        assert_eq!(report_extensions(&Value::String(legacy.to_string())), Some(doc));
        // Unknown shapes are not guessed.
        assert_eq!(report_extensions(&json!([])), None);
        assert_eq!(
            report_extensions(&json!([{"entities": [{"name": "T", "x": 1}]}])),
            None
        );
        assert_eq!(
            report_extensions(&json!([{"entities": [{"name": "T", "measures": [
                {"name": "M", "dataType": 99, "expression": "1"}]}]}])),
            None
        );
    }

    #[test]
    fn groups_are_flattened_without_the_stylable_header_setting() {
        let v = |name: &str, x: f64, parent: Option<&str>, group: bool| {
            let mut v = bicraft_report::Visual {
                name: name.into(),
                parent_group: parent.map(str::to_string),
                ..Default::default()
            };
            v.position.x = x;
            v.position.y = x;
            if group {
                v.extra.insert("visualGroup".into(), json!({"displayName": name}));
            }
            v
        };
        let page = bicraft_report::Page {
            visuals: vec![
                v("outer", 10.0, None, true),
                v("inner", 20.0, Some("outer"), true),
                v("a", 1.0, Some("inner"), false),
                v("b", 2.0, Some("outer"), false),
                v("c", 3.0, None, false),
            ],
            ..Default::default()
        };
        let mut r = Report {
            pages: vec![page],
            ..Default::default()
        };
        let mut kept = r.clone();
        kept.extra.insert(
            "settings".into(),
            json!({"useStylableVisualContainerHeader": true}),
        );
        strip_legacy(&mut kept);
        assert_eq!(kept.pages[0].visuals.len(), 5);
        strip_legacy(&mut r);
        let got: Vec<(&str, f64, Option<&str>)> = r.pages[0]
            .visuals
            .iter()
            .map(|v| (v.name.as_str(), v.position.x, v.parent_group.as_deref()))
            .collect();
        assert_eq!(got, [("a", 31.0, None), ("b", 12.0, None), ("c", 3.0, None)]);
    }

    #[test]
    fn drill_roles_get_their_active_markers() {
        let mut v = bicraft_report::Visual {
            visual_type: Some("pivotTable".into()),
            ..Default::default()
        };
        v.visual_extra.insert(
            "query".into(),
            json!({"queryState": {
                "Rows": {"projections": [{"queryRef": "a", "active": true}, {"queryRef": "b"}]},
                "Columns": {"projections": [{"queryRef": "c"}, {"queryRef": "d"}]},
                "Values": {"projections": [{"queryRef": "e"}]}}}),
        );
        mark_active(&mut v);
        let s = &v.visual_extra["query"]["queryState"];
        assert_eq!(s["Rows"]["projections"][1]["active"], false);
        assert_eq!(s["Columns"]["projections"][0]["active"], true);
        assert_eq!(s["Columns"]["projections"][1]["active"], false);
        assert!(s["Values"]["projections"][0].get("active").is_none());
    }
}
