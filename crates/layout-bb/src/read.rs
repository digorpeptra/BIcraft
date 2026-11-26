//! Legacy layout → report. Each legacy object is walked key by key: a key with a PBIR form is
//! converted, every other one goes to the unmapped map of its report, page or visual.

use bicraft_report::{NamedDocument, Page, Position, Report, Visual};
use serde_json::{Map, Value, json};

use crate::codes::{self, Table};
use crate::path::{At, Object, Unmapped, norm_num, slot};
use crate::{BOOKMARKS_METADATA_KEY, Error, MOBILE_STATE_KEY, Summary, UNMAPPED_KEY, query};

pub(crate) struct Read {
    pub report: Report,
    /// Per page (report order): the section's index in `sections` and its `ordinal`.
    pub sections: Vec<(usize, Option<Value>)>,
}

/// Formatting objects and properties PBIR allows in `report.json` `objects` (public report
/// schema 3.3.0, `ReportFormattingObjects`); Desktop drops the others (LO-pages).
const REPORT_OBJECTS: &[(&str, &[&str])] = &[
    ("outspacePane", &["expanded", "visible"]),
    ("section", &["verticalAlignment"]),
];

/// Formatting objects and properties of a filter card (public filterConfiguration schema 1.3.0,
/// `FilterContainerFormattingObjects`); Desktop drops the others (LO-report).
const FILTER_OBJECTS: &[(&str, &[&str])] =
    &[("general", &["requireSingleSelect", "isInvertedSelectionMode"])];

/// `slowDataSourceSettings` properties of the public report schema 3.3.0
/// (`ExplorationSlowDataSourceSettings`): the query reduction options.
const SLOW_SOURCE_SETTINGS: &[&str] = &[
    "isCrossHighlightingDisabled",
    "isSlicerSelectionsButtonEnabled",
    "isFilterSelectionsButtonEnabled",
    "isFieldWellButtonEnabled",
    "isApplyAllButtonEnabled",
];

/// `settings` properties of the public report schema 3.3.0 (`ExplorationSettings`).
pub(crate) const SETTINGS: &[&str] = &[
    "isPersistentUserStateDisabled",
    "hideVisualContainerHeader",
    "useStylableVisualContainerHeader",
    "exportDataMode",
    "isReportAnnotationsDisabled",
    "defaultFilterActionIsDataFilter",
    "defaultDrillFilterOtherVisuals",
    "useCrossReportDrillthrough",
    "allowChangeFilterTypes",
    "allowInlineExploration",
    "useEnhancedTooltips",
    "useScaledTooltips",
    "filterPaneHiddenInEditMode",
    "disableFilterPaneSearch",
    "pagesPosition",
    "allowAutomatedInsightsNotification",
    "useDefaultAggregateDisplayName",
    "enableDeveloperMode",
    "pauseQueries",
    "queryLimitOption",
    "customMemoryLimit",
    "customTimeoutLimit",
    "fieldParameterReportSettings",
    "defaultDataExplorePerspective",
    "locale",
    "defaultDisplayUnitsToNone",
];

/// Settings with a numeric legacy code and their table.
pub(crate) const SETTING_CODES: &[(&str, Table)] = &[
    ("exportDataMode", codes::EXPORT_DATA_MODE),
    ("pagesPosition", codes::PAGES_POSITION),
    ("queryLimitOption", codes::QUERY_LIMIT_OPTION),
];

/// Settings Desktop leaves out of PBIR when they hold this value (LO-report).
const SETTING_DEFAULTS: &[(&str, bool)] = &[
    ("hideVisualContainerHeader", false),
    ("filterPaneHiddenInEditMode", false),
];

pub(crate) fn read(text: &str) -> Result<Read, Error> {
    let text = text.trim_start_matches('\u{feff}');
    let root: Value = serde_json::from_str(text).map_err(|e| Error::Json(e.to_string()))?;
    let Value::Object(root) = root else {
        return Err(Error::NotObject);
    };
    let mut report = Report::default();
    let mut un = Unmapped::default();
    let top = At::root();

    let config = match slot(root.get("config")) {
        Ok(Some(Value::Object(m))) => m,
        Ok(None) => Map::new(),
        Ok(Some(other)) | Err(other) => {
            un.put(&top.key("config"), other);
            Map::new()
        }
    };

    // Pods, by bound section.
    let mut pods: Vec<(usize, Object)> = Vec::new();
    if let Some(p) = root.get("pods") {
        match p.as_array() {
            Some(list) => {
                for (i, pod) in list.iter().enumerate() {
                    match pod.as_object() {
                        Some(o) if o.get("boundSection").and_then(Value::as_str).is_some() => {
                            pods.push((i, o.clone()));
                        }
                        _ => un.put(&top.key("pods").index(i), pod.clone()),
                    }
                }
            }
            None => un.put(&top.key("pods"), p.clone()),
        }
    }

    // Sections.
    let mut pages: Vec<(Option<Value>, usize, Page)> = Vec::new();
    if let Some(s) = root.get("sections") {
        match s.as_array() {
            Some(list) => {
                for (i, section) in list.iter().enumerate() {
                    match section.as_object() {
                        Some(o) => {
                            let page = read_page(o, i, &mut pods);
                            pages.push((o.get("ordinal").cloned(), i, page));
                        }
                        None => un.put(&top.key("sections").index(i), section.clone()),
                    }
                }
            }
            None => un.put(&top.key("sections"), s.clone()),
        }
    }
    // Pages in `ordinal` order (a section without one counts as 0), ties in file order (LO-pages,
    // LO-groups).
    pages.sort_by(|a, b| {
        let o = |v: &Option<Value>| v.as_ref().and_then(Value::as_f64).unwrap_or(0.0);
        o(&a.0).total_cmp(&o(&b.0)).then(a.1.cmp(&b.1))
    });
    for (i, pod) in pods {
        un.put(&top.key("pods").index(i), Value::Object(pod));
    }

    let mut filter_config = Map::new();
    if let Some(f) = root.get("filters") {
        let list = read_filters(f, &top.key("filters"), &mut un);
        if !list.is_empty() {
            filter_config.insert("filters".into(), Value::Array(list));
        }
    }

    for (k, v) in &root {
        let at = top.key(k);
        match k.as_str() {
            "config" | "pods" | "sections" | "filters" => {}
            "resourcePackages" => {
                if let Some(p) = read_packages(v, &at, &mut un) {
                    report.extra.insert("resourcePackages".into(), p);
                }
            }
            "publicCustomVisuals" if v.as_array().is_some_and(|a| !a.is_empty()) => {
                report.extra.insert(k.clone(), v.clone());
            }
            _ => un.put(&at, v.clone()),
        }
    }

    // config
    let cat = top.key("config");
    let mut settings = Map::new();
    for (k, v) in &config {
        let at = cat.key(k);
        match k.as_str() {
            "themeCollection" => match read_themes(v, &at, &mut un) {
                Some(t) => report.theme_collection = Some(t),
                None => un.put(&at, v.clone()),
            },
            "activeSectionIndex" => {
                let page = v
                    .as_u64()
                    .and_then(|i| usize::try_from(i).ok())
                    .and_then(|i| pages.get(i));
                match page {
                    Some((_, _, p)) => report.active_page = Some(p.name.clone()),
                    None => un.put(&at, v.clone()),
                }
            }
            "objects" => {
                let objects = filter_objects(v, REPORT_OBJECTS, &at, &mut un);
                if let Some(o) = objects {
                    report.extra.insert("objects".into(), o);
                }
            }
            "settings" => read_settings(v, &at, &mut settings, &mut un),
            "slowDataSourceSettings" => read_slow_source(v, &at, &mut report, &mut un),
            "defaultDrillFilterOtherVisuals" if v.is_boolean() => {
                settings.insert(k.clone(), v.clone());
            }
            "filterSortOrder" => match codes::name(codes::FILTER_SORT_ORDER, v) {
                Some(n) => {
                    filter_config.insert("filterSortOrder".into(), json!(n));
                }
                None => un.put(&at, v.clone()),
            },
            "bookmarks" => read_bookmarks(v, &at, &mut report, &mut un),
            _ => un.put(&at, v.clone()),
        }
    }
    if !settings.is_empty() {
        let mut ordered = Map::new();
        for k in SETTINGS {
            if let Some(v) = settings.shift_remove(*k) {
                ordered.insert((*k).to_string(), v);
            }
        }
        report.extra.insert("settings".into(), Value::Object(ordered));
    }
    if !filter_config.is_empty() {
        report.filter_config = Some(Value::Object(filter_config));
    }
    // Always present, so a report read from a legacy layout is recognizable as one.
    report.extra.insert(UNMAPPED_KEY.into(), un.into_value());

    let mut sections = Vec::new();
    for (ordinal, index, page) in pages {
        sections.push((index, ordinal));
        report.pages.push(page);
    }
    Ok(Read { report, sections })
}

pub(crate) fn summary(report: &Report) -> Summary {
    let mut s = Summary {
        pages: report.pages.len(),
        visuals: report.pages.iter().map(|p| p.visuals.len()).sum(),
        bookmarks: report.bookmarks.len(),
        unmapped: Vec::new(),
    };
    let mut list = |map: &Object, at: &str| {
        if let Some(Value::Object(u)) = map.get(UNMAPPED_KEY) {
            s.unmapped.extend(u.keys().map(|k| format!("{at}: {k}")));
        }
    };
    list(&report.extra, "report");
    for p in &report.pages {
        list(&p.extra, &format!("page {}", p.name));
        for v in &p.visuals {
            list(&v.extra, &format!("visual {}/{}", p.name, v.name));
        }
    }
    s
}

/// Keeps the formatting objects and properties `allowed` lists; the rest is unmapped.
fn filter_objects(v: &Value, allowed: &[(&str, &[&str])], at: &At, un: &mut Unmapped) -> Option<Value> {
    let Some(objects) = v.as_object() else {
        un.put(at, v.clone());
        return None;
    };
    let mut out = Map::new();
    for (name, entries) in objects {
        let oat = at.key(name);
        let Some(props) = allowed.iter().find(|(n, _)| n == name).map(|(_, p)| *p) else {
            un.put(&oat, entries.clone());
            continue;
        };
        let Some(list) = entries.as_array() else {
            un.put(&oat, entries.clone());
            continue;
        };
        let mut kept = Vec::new();
        for (i, e) in list.iter().enumerate() {
            let eat = oat.index(i);
            let Some(eo) = e.as_object() else {
                un.put(&eat, e.clone());
                continue;
            };
            let mut entry = Map::new();
            for (k, ev) in eo {
                match (k.as_str(), ev.as_object()) {
                    ("properties", Some(p)) => {
                        let mut pk = Map::new();
                        for (pn, pv) in p {
                            if props.contains(&pn.as_str()) {
                                pk.insert(pn.clone(), pv.clone());
                            } else {
                                un.put(&eat.key("properties").key(pn), pv.clone());
                            }
                        }
                        entry.insert(k.clone(), Value::Object(pk));
                    }
                    ("selector", _) => {
                        entry.insert(k.clone(), ev.clone());
                    }
                    _ => un.put(&eat.key(k), ev.clone()),
                }
            }
            kept.push(Value::Object(entry));
        }
        out.insert(name.clone(), Value::Array(kept));
    }
    if out.is_empty() {
        None
    } else {
        Some(Value::Object(out))
    }
}

fn read_settings(v: &Value, at: &At, out: &mut Object, un: &mut Unmapped) {
    let Some(m) = v.as_object() else {
        un.put(at, v.clone());
        return;
    };
    for (k, sv) in m {
        let sat = at.key(k);
        if !SETTINGS.contains(&k.as_str()) {
            // Not a PBIR setting (LO-report: `useNewFilterPaneExperience`).
            un.put(&sat, sv.clone());
        } else if SETTING_DEFAULTS
            .iter()
            .any(|(n, d)| n == k && sv.as_bool() == Some(*d))
        {
            un.put(&sat, sv.clone());
        } else if let Some((_, table)) = SETTING_CODES.iter().find(|(n, _)| n == k) {
            match codes::name(table, sv) {
                Some(n) => {
                    out.insert(k.clone(), json!(n));
                }
                None => un.put(&sat, sv.clone()),
            }
        } else {
            out.insert(k.clone(), sv.clone());
        }
    }
}

/// The query reduction options under the PBIR name (*inferred*: the legacy key is the PBIR one),
/// the schema's booleans only; anything else stays unmapped.
fn read_slow_source(v: &Value, at: &At, report: &mut Report, un: &mut Unmapped) {
    let Some(m) = v.as_object() else {
        un.put(at, v.clone());
        return;
    };
    let mut out = Map::new();
    for (k, sv) in m {
        if SLOW_SOURCE_SETTINGS.contains(&k.as_str()) && sv.is_boolean() {
            out.insert(k.clone(), sv.clone());
        } else {
            un.put(&at.key(k), sv.clone());
        }
    }
    if !out.is_empty() {
        report
            .extra
            .insert("slowDataSourceSettings".into(), Value::Object(out));
    }
}

fn read_packages(v: &Value, at: &At, un: &mut Unmapped) -> Option<Value> {
    let Some(list) = v.as_array() else {
        un.put(at, v.clone());
        return None;
    };
    let mut out = Vec::new();
    for (i, entry) in list.iter().enumerate() {
        let eat = at.index(i);
        let pkg = entry.get("resourcePackage").and_then(Value::as_object);
        let ty = pkg
            .and_then(|p| p.get("type"))
            .and_then(|t| codes::name(codes::PACKAGE_TYPE, t));
        let (Some(pkg), Some(ty), Some(1)) = (pkg, ty, entry.as_object().map(Map::len)) else {
            un.put(&eat, entry.clone());
            continue;
        };
        let pat = eat.key("resourcePackage");
        let mut p = Map::new();
        for (k, pv) in pkg {
            match k.as_str() {
                "name" => {
                    p.insert("name".into(), pv.clone());
                }
                "type" => {
                    p.insert("type".into(), json!(ty));
                }
                "items" => {
                    let mut items = Vec::new();
                    for (j, item) in pv.as_array().into_iter().flatten().enumerate() {
                        let it = item.get("type").and_then(|t| codes::name(codes::ITEM_TYPE, t));
                        let ok = item.as_object().is_some_and(|o| {
                            o.keys().all(|k| matches!(k.as_str(), "name" | "path" | "type"))
                        });
                        match (it, ok) {
                            (Some(it), true) => {
                                let mut m = Map::new();
                                if let Some(n) = item.get("name") {
                                    m.insert("name".into(), n.clone());
                                }
                                if let Some(n) = item.get("path") {
                                    m.insert("path".into(), n.clone());
                                }
                                m.insert("type".into(), json!(it));
                                items.push(Value::Object(m));
                            }
                            _ => un.put(&pat.key("items").index(j), item.clone()),
                        }
                    }
                    if !pv.is_array() {
                        un.put(&pat.key("items"), pv.clone());
                    }
                    p.insert("items".into(), Value::Array(items));
                }
                _ => un.put(&pat.key(k), pv.clone()),
            }
        }
        out.push(Value::Object(p));
    }
    Some(Value::Array(out))
}

fn read_themes(v: &Value, at: &At, un: &mut Unmapped) -> Option<Value> {
    let m = v.as_object()?;
    let mut out = Map::new();
    for (k, t) in m {
        let tat = at.key(k);
        let to = t.as_object()?;
        let ty = codes::name(codes::THEME_TYPE, to.get("type")?)?;
        let version = to.get("version").and_then(Value::as_str);
        let triple = theme_versions(version);
        let mut theme = Map::new();
        for (tk, tv) in to {
            match tk.as_str() {
                "name" | "type" => {}
                // Desktop replaces the version by `reportVersionAtImport`: the legacy value has
                // no PBIR form (LO-report, LO-groups).
                _ => un.put(&tat.key(tk), tv.clone()),
            }
        }
        theme.insert("name".into(), to.get("name")?.clone());
        theme.insert(
            "reportVersionAtImport".into(),
            json!({"visual": triple[0], "report": triple[1], "page": triple[2]}),
        );
        theme.insert("type".into(), json!(ty));
        out.insert(k.clone(), Value::Object(theme));
    }
    Some(Value::Object(out))
}

/// The `reportVersionAtImport` of a legacy theme version: the known entry, else the closest
/// lower known one (inference, see the format note), else the lowest.
pub(crate) fn theme_versions(version: Option<&str>) -> [&'static str; 3] {
    let parse = |s: &str| -> Vec<u64> { s.split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    let mut best = codes::THEME_VERSIONS
        .first()
        .map(|(_, t)| *t)
        .unwrap_or(["1.0.0", "1.0.0", "1.0.0"]);
    if let Some(v) = version {
        let want = parse(v);
        for (known, triple) in codes::THEME_VERSIONS {
            if parse(known) <= want {
                best = *triple;
            }
        }
    }
    best
}

fn read_bookmarks(v: &Value, at: &At, report: &mut Report, un: &mut Unmapped) {
    let Some(list) = v.as_array() else {
        un.put(at, v.clone());
        return;
    };
    let mut items = Vec::new();
    for (i, b) in list.iter().enumerate() {
        let bat = at.index(i);
        let Some(name) = b.get("name").and_then(Value::as_str) else {
            un.put(&bat, b.clone());
            continue;
        };
        if let Some(children) = b.get("children") {
            let Some(children) = children.as_array() else {
                un.put(&bat, b.clone());
                continue;
            };
            let mut names = Vec::new();
            for (j, c) in children.iter().enumerate() {
                match c.get("name").and_then(Value::as_str) {
                    Some(n) if !report.bookmarks.iter().any(|x| x.name == n) => {
                        names.push(json!(n));
                        report.bookmarks.push(NamedDocument {
                            name: n.to_string(),
                            document: c.clone(),
                        });
                    }
                    _ => un.put(&bat.key("children").index(j), c.clone()),
                }
            }
            let mut item = Map::new();
            item.insert("name".into(), json!(name));
            if let Some(d) = b.get("displayName") {
                item.insert("displayName".into(), d.clone());
            }
            item.insert("children".into(), Value::Array(names));
            if let Some(o) = b.as_object() {
                for (k, x) in o {
                    if !matches!(k.as_str(), "name" | "displayName" | "children") {
                        un.put(&bat.key(k), x.clone());
                    }
                }
            }
            items.push(Value::Object(item));
        } else if report.bookmarks.iter().any(|x| x.name == name) {
            un.put(&bat, b.clone());
        } else {
            items.push(json!({"name": name}));
            report.bookmarks.push(NamedDocument {
                name: name.to_string(),
                document: b.clone(),
            });
        }
    }
    if !items.is_empty() {
        report
            .extra
            .insert(BOOKMARKS_METADATA_KEY.into(), json!({"items": items}));
    }
}

/// A legacy filter list (a `filters` slot) as PBIR filters.
fn read_filters(v: &Value, at: &At, un: &mut Unmapped) -> Vec<Value> {
    let list = match slot(Some(v)) {
        Ok(Some(Value::Array(a))) => a,
        Ok(_) | Err(_) => {
            un.put(at, v.clone());
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for (i, f) in list.iter().enumerate() {
        if let Some(c) = read_filter(f, &at.index(i), un) {
            out.push(c);
        }
    }
    out
}

fn read_filter(f: &Value, at: &At, un: &mut Unmapped) -> Option<Value> {
    let Some(o) = f.as_object() else {
        un.put(at, f.clone());
        return None;
    };
    // Desktop does not carry transient filters into PBIR (LO-report, `Filter_rtrans`).
    if o.get("isTransient") == Some(&Value::Bool(true)) || o.get("name").and_then(Value::as_str).is_none() {
        un.put(at, f.clone());
        return None;
    }
    let mut out = Map::new();
    let mut filter = o.get("filter").cloned();
    for (k, v) in o {
        let kat = at.key(k);
        match k.as_str() {
            "name" | "displayName" | "ordinal" | "type" | "restatement" | "isHiddenInViewMode"
            | "isLockedInViewMode" | "filter" => {}
            "expression" => {}
            "howCreated" => {
                // 0 (`Auto`) has no PBIR spelling: Desktop writes no `howCreated` (LO-filters).
                if codes::name(codes::HOW_CREATED, v).is_none() {
                    un.put(&kat, v.clone());
                }
            }
            "objects" => {}
            "cachedDisplayNames" => {
                let field = o.get("expression");
                let done = match (field, filter.as_mut()) {
                    (Some(field), Some(flt)) => annotate(flt, field, v),
                    _ => false,
                };
                if !done {
                    un.put(&kat, v.clone());
                }
            }
            _ => un.put(&kat, v.clone()),
        }
    }
    for k in ["name", "displayName", "ordinal"] {
        if let Some(v) = o.get(k) {
            out.insert(k.into(), v.clone());
        }
    }
    if let Some(e) = o.get("expression") {
        out.insert("field".into(), e.clone());
    }
    if let Some(t) = o.get("type") {
        out.insert("type".into(), t.clone());
    }
    if let Some(f) = filter {
        out.insert("filter".into(), f);
    }
    if let Some(r) = o.get("restatement") {
        out.insert("restatement".into(), r.clone());
    }
    if let Some(n) = o
        .get("howCreated")
        .and_then(|h| codes::name(codes::HOW_CREATED, h))
    {
        out.insert("howCreated".into(), json!(n));
    }
    for k in ["isHiddenInViewMode", "isLockedInViewMode"] {
        if let Some(v) = o.get(k) {
            out.insert(k.into(), v.clone());
        }
    }
    if let Some(objs) = o.get("objects")
        && let Some(x) = filter_objects(objs, FILTER_OBJECTS, &at.key("objects"), un)
    {
        out.insert("objects".into(), x);
    }
    Some(Value::Object(out))
}

/// Turns `cachedDisplayNames` (display names of the filtered values) into the annotation Desktop
/// writes on the filter condition (LO-filters, LO-groups). Only the observed shape: one
/// condition, every cached id an equality of the filter's own field with a literal.
fn annotate(filter: &mut Value, field: &Value, cached: &Value) -> bool {
    let Some(entries) = cached.as_array().filter(|a| !a.is_empty()) else {
        return false;
    };
    let mut values = Vec::new();
    let mut names = Vec::new();
    for e in entries {
        let cmp = e
            .get("id")
            .and_then(|i| i.get("scopeId"))
            .and_then(|s| s.get("Comparison"));
        let ok = cmp.is_some_and(|c| {
            c.get("ComparisonKind").and_then(Value::as_i64) == Some(0)
                && c.get("Left") == Some(field)
                && c.get("Right").and_then(|r| r.get("Literal")).is_some()
                && c.as_object().map(Map::len) == Some(3)
        });
        let (true, Some(right), Some(name)) = (
            ok,
            cmp.and_then(|c| c.get("Right")),
            e.get("displayName").filter(|d| d.is_string()),
        ) else {
            return false;
        };
        if e.as_object().map(Map::len) != Some(2) {
            return false;
        }
        values.push(json!([{"0": [right]}]));
        names.push(json!({"0": name}));
    }
    let Some(Value::Array(wheres)) = filter.get_mut("Where") else {
        return false;
    };
    let [Value::Object(w)] = wheres.as_mut_slice() else {
        return false;
    };
    if w.contains_key("Annotations") {
        return false;
    }
    w.insert(
        "Annotations".into(),
        json!({"filterExpressionMetadata": {
            "expressions": [field],
            "decomposedIdentities": {"values": values, "columns": [{"value": field}]},
            "valueMap": names
        }}),
    );
    true
}

fn read_page(section: &Object, index: usize, pods: &mut Vec<(usize, Object)>) -> Page {
    let mut un = Unmapped::default();
    let top = At::root();
    let name = section
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("ReportSection{index}"));
    let mut page = Page {
        name: name.clone(),
        ..Default::default()
    };
    let mut filter_config = Map::new();
    let config = match slot(section.get("config")) {
        Ok(Some(Value::Object(m))) => m,
        Ok(None) => Map::new(),
        Ok(Some(other)) | Err(other) => {
            un.put(&top.key("config"), other);
            Map::new()
        }
    };
    for (k, v) in section {
        let at = top.key(k);
        match k.as_str() {
            "name" if v.is_string() => {}
            "config" | "visualContainers" | "ordinal" => {}
            "displayName" if v.is_string() => page.display_name = v.as_str().unwrap_or_default().to_string(),
            "width" if v.is_number() => page.width = v.as_f64().unwrap_or_default(),
            "height" if v.is_number() => page.height = v.as_f64().unwrap_or_default(),
            "displayOption" => match codes::name(codes::DISPLAY_OPTION, v) {
                Some(n) => page.display_option = Some(n.to_string()),
                None => un.put(&at, v.clone()),
            },
            "filters" => {
                let list = read_filters(v, &at, &mut un);
                if !list.is_empty() {
                    filter_config.insert("filters".into(), Value::Array(list));
                }
            }
            _ => un.put(&at, v.clone()),
        }
    }
    let cat = top.key("config");
    for (k, v) in &config {
        let at = cat.key(k);
        match k.as_str() {
            "objects" if v.is_object() => {
                page.extra.insert("objects".into(), v.clone());
            }
            "relationships" => {
                let Some(list) = v.as_array() else {
                    un.put(&at, v.clone());
                    continue;
                };
                let mut out = Vec::new();
                for (i, r) in list.iter().enumerate() {
                    let ty = r.get("type").and_then(|t| codes::name(codes::INTERACTION, t));
                    let ok = r.as_object().is_some_and(|o| {
                        o.len() == 3
                            && o.get("source").is_some_and(Value::is_string)
                            && o.get("target").is_some_and(Value::is_string)
                    });
                    match (ty, ok) {
                        (Some(t), true) => out.push(json!({
                            "source": r.get("source"),
                            "target": r.get("target"),
                            "type": t
                        })),
                        _ => un.put(&at.index(i), r.clone()),
                    }
                }
                if !out.is_empty() {
                    page.extra.insert("visualInteractions".into(), Value::Array(out));
                }
            }
            "visibility" => match codes::name(codes::VISIBILITY, v) {
                Some(n) => page.visibility = Some(n.to_string()),
                None => un.put(&at, v.clone()),
            },
            "type" => match codes::name(codes::PAGE_TYPE, v) {
                Some(n) => {
                    page.extra.insert("type".into(), json!(n));
                }
                None => un.put(&at, v.clone()),
            },
            "filterSortOrder" => match codes::name(codes::FILTER_SORT_ORDER, v) {
                Some(n) => {
                    filter_config.insert("filterSortOrder".into(), json!(n));
                }
                None => un.put(&at, v.clone()),
            },
            _ => un.put(&at, v.clone()),
        }
    }
    if !filter_config.is_empty() {
        page.filter_config = Some(Value::Object(filter_config));
    }
    // The pod bound to this section (the first one) becomes its page binding (LO-pages).
    if let Some(i) = pods
        .iter()
        .position(|(_, p)| p.get("boundSection").and_then(Value::as_str) == Some(name.as_str()))
    {
        let (_, pod) = pods.remove(i);
        let binding = read_pod(&pod, &top.key("pod"), &mut un);
        page.extra.insert("pageBinding".into(), binding);
    }

    if let Some(list) = section.get("visualContainers") {
        match list.as_array() {
            Some(list) => {
                for (i, c) in list.iter().enumerate() {
                    match c.as_object() {
                        Some(o) => page.visuals.push(read_visual(o, i)),
                        None => un.put(&top.key("visualContainers").index(i), c.clone()),
                    }
                }
            }
            None => un.put(&top.key("visualContainers"), list.clone()),
        }
    }
    if !un.is_empty() {
        page.extra.insert(UNMAPPED_KEY.into(), un.into_value());
    }
    page
}

fn read_pod(pod: &Object, at: &At, un: &mut Unmapped) -> Value {
    let mut b = Map::new();
    if let Some(n) = pod.get("name") {
        b.insert("name".into(), n.clone());
    }
    match pod.get("type").map(|t| (t, codes::name(codes::BINDING_TYPE, t))) {
        Some((_, Some(n))) => {
            b.insert("type".into(), json!(n));
        }
        Some((t, None)) => un.put(&at.key("type"), t.clone()),
        None => {}
    }
    match slot(pod.get("parameters")) {
        Ok(Some(p @ Value::Array(_))) => {
            b.insert("parameters".into(), p);
        }
        Ok(None) => {}
        Ok(Some(other)) | Err(other) => un.put(&at.key("parameters"), other),
    }
    for (k, v) in pod {
        let kat = at.key(k);
        match k.as_str() {
            "name" | "type" | "parameters" | "boundSection" => {}
            "referenceScope" => match codes::name(codes::REFERENCE_SCOPE, v) {
                Some(n) => {
                    b.insert("referenceScope".into(), json!(n));
                }
                None => un.put(&kat, v.clone()),
            },
            "config" => match slot(Some(v)) {
                Ok(Some(Value::Object(c))) => {
                    for (ck, cv) in &c {
                        match (ck.as_str(), codes::name(codes::ACCEPTS_FILTER_CONTEXT, cv)) {
                            ("acceptsFilterContext", Some(n)) => {
                                b.insert("acceptsFilterContext".into(), json!(n));
                            }
                            _ => un.put(&kat.key(ck), cv.clone()),
                        }
                    }
                }
                Ok(_) | Err(_) => un.put(&kat, v.clone()),
            },
            _ => un.put(&kat, v.clone()),
        }
    }
    Value::Object(b)
}

/// The position keys of a PBIR mobile state, in Desktop's order.
const POSITION_ORDER: [&str; 7] = ["x", "y", "z", "height", "width", "tabOrder", "angle"];

fn read_visual(container: &Object, index: usize) -> Visual {
    let mut un = Unmapped::default();
    let top = At::root();
    let config = match slot(container.get("config")) {
        Ok(Some(Value::Object(m))) => m,
        Ok(None) => Map::new(),
        Ok(Some(other)) | Err(other) => {
            un.put(&top.key("config"), other);
            Map::new()
        }
    };
    let name = config
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("visual{index}"));
    let mut v = Visual {
        name,
        ..Default::default()
    };
    let layout0 = config
        .get("layouts")
        .and_then(Value::as_array)
        .and_then(|l| l.iter().find(|x| x.get("id").and_then(Value::as_i64) == Some(0)))
        .and_then(|x| x.get("position"));
    // The desktop layout (id 0) wins over the container's own numbers when they differ (LX-x3).
    let num = |k: &str| -> Option<f64> {
        layout0
            .and_then(|p| p.get(k))
            .and_then(Value::as_f64)
            .or_else(|| container.get(k).and_then(Value::as_f64))
    };
    v.position = Position {
        x: num("x").unwrap_or_default(),
        y: num("y").unwrap_or_default(),
        z: num("z").unwrap_or_default(),
        width: num("width").unwrap_or_default(),
        height: num("height").unwrap_or_default(),
        tab_order: None,
    };
    if let Some(t) = num("tabOrder") {
        if t.fract() == 0.0 && t.abs() < 9.0e15 {
            v.position.tab_order = Some(t as i64);
        } else {
            v.extra.insert("position".into(), json!({"tabOrder": t}));
        }
    }
    let mut filter_config = Map::new();
    for (k, val) in container {
        let at = top.key(k);
        match k.as_str() {
            "x" | "y" | "z" | "width" | "height" | "tabOrder" if val.is_number() => {}
            "config" => {}
            "filters" => {
                let list = read_filters(val, &at, &mut un);
                if !list.is_empty() {
                    filter_config.insert("filters".into(), Value::Array(list));
                }
            }
            _ => un.put(&at, val.clone()),
        }
    }

    let cat = top.key("config");
    let mut mobile: Option<Object> = None;
    // `layouts` first: the phone formatting of `singleVisual` belongs to the phone layout.
    let ordered = config
        .iter()
        .filter(|(k, _)| k.as_str() == "layouts")
        .chain(config.iter().filter(|(k, _)| k.as_str() != "layouts"));
    for (k, val) in ordered {
        let at = cat.key(k);
        match k.as_str() {
            "name" if val.is_string() => {}
            "layouts" => {
                let Some(list) = val.as_array() else {
                    un.put(&at, val.clone());
                    continue;
                };
                for (i, l) in list.iter().enumerate() {
                    let lat = at.index(i);
                    match (l.get("id").and_then(Value::as_i64), l.get("position")) {
                        (Some(0), Some(_)) if l.as_object().map(Map::len) == Some(2) => {}
                        (Some(1), Some(Value::Object(p))) if mobile.is_none() => {
                            let mut pos = Map::new();
                            for key in POSITION_ORDER {
                                if let Some(x) = p.get(key) {
                                    pos.insert(key.into(), norm_num(x));
                                }
                            }
                            for (pk, pv) in p {
                                if !pos.contains_key(pk) {
                                    pos.insert(pk.clone(), norm_num(pv));
                                }
                            }
                            let mut state = Map::new();
                            state.insert("position".into(), Value::Object(pos));
                            if let Some(o) = l.as_object() {
                                for (lk, lv) in o {
                                    if lk != "id" && lk != "position" {
                                        un.put(&lat.key(lk), lv.clone());
                                    }
                                }
                            }
                            mobile = Some(state);
                        }
                        _ => un.put(&lat, l.clone()),
                    }
                }
            }
            "singleVisual" => match val.as_object() {
                Some(sv) => read_single_visual(sv, &at, &mut v, &mut mobile, &mut filter_config, &mut un),
                None => un.put(&at, val.clone()),
            },
            "singleVisualGroup" => match read_group(val, &at, &mut un) {
                Some(g) => {
                    v.extra.insert("visualGroup".into(), g);
                }
                None => un.put(&at, val.clone()),
            },
            "parentGroupName" if val.is_string() => {
                v.parent_group = val.as_str().map(str::to_string);
            }
            "howCreated" if val.is_string() => {
                v.extra.insert("howCreated".into(), val.clone());
            }
            _ => un.put(&at, val.clone()),
        }
    }
    if let Some(m) = mobile {
        v.extra.insert(MOBILE_STATE_KEY.into(), Value::Object(m));
    }
    if !filter_config.is_empty() {
        v.filter_config = Some(Value::Object(filter_config));
    }
    if !un.is_empty() {
        v.extra.insert(UNMAPPED_KEY.into(), un.into_value());
    }
    v
}

fn read_group(val: &Value, at: &At, un: &mut Unmapped) -> Option<Value> {
    let g = val.as_object()?;
    let mut out = Map::new();
    for (k, x) in g {
        match k.as_str() {
            "displayName" => {
                out.insert(k.clone(), x.clone());
            }
            "groupMode" => match codes::name(codes::GROUP_MODE, x) {
                Some(n) => {
                    out.insert(k.clone(), json!(n));
                }
                None => un.put(&at.key(k), x.clone()),
            },
            _ => un.put(&at.key(k), x.clone()),
        }
    }
    Some(Value::Object(out))
}

fn read_single_visual(
    sv: &Object,
    at: &At,
    v: &mut Visual,
    mobile: &mut Option<Object>,
    filter_config: &mut Object,
    un: &mut Unmapped,
) {
    if let Some(q) = query::from_legacy(sv, at, un) {
        v.fields = bicraft_report::query::derive(Some(&q));
        v.visual_extra.insert("query".into(), q);
    }
    for (k, val) in sv {
        let kat = at.key(k);
        match k.as_str() {
            "visualType" if val.is_string() => v.visual_type = val.as_str().map(str::to_string),
            k if query::KEYS.contains(&k) => {}
            "objects" => {
                v.visual_extra.insert("objects".into(), val.clone());
            }
            "vcObjects" => {
                v.visual_extra
                    .insert("visualContainerObjects".into(), val.clone());
            }
            "expansionStates" => {
                v.visual_extra.insert("expansionStates".into(), val.clone());
            }
            "syncGroup" | "drillFilterOtherVisuals" | "autoSelectVisualType" => {
                v.visual_extra.insert(k.to_string(), val.clone());
            }
            "display" if val == &json!({"mode": "hidden"}) => v.is_hidden = true,
            "mobileObjects" | "mobileVcObjects" if mobile.is_some() => {
                let key = if k == "mobileObjects" {
                    "objects"
                } else {
                    "visualContainerObjects"
                };
                if let Some(m) = mobile.as_mut() {
                    m.insert(key.into(), val.clone());
                }
            }
            "filterSortOrder" => match codes::name(codes::FILTER_SORT_ORDER, val) {
                Some(n) => {
                    filter_config.insert("filterSortOrder".into(), json!(n));
                }
                None => un.put(&kat, val.clone()),
            },
            _ => un.put(&kat, val.clone()),
        }
    }
}
