//! Report → legacy layout. The edited report is converted back object by object (the inverse of
//! `read.rs`), each object's unmapped legacy properties are put back where they were, and the
//! document is serialized with the original text of every top-level key, section and visual
//! container whose content is unchanged.

use bicraft_report::{Page, Report, Visual};
use serde_json::value::RawValue;
use serde_json::{Map, Value, json};

use crate::codes::{self, Table};
use crate::path::{Object, apply_unmapped, encode_slot, float, reorder, slot, unmapped_under};
use crate::read::{self, SETTING_CODES};
use crate::{BOOKMARKS_METADATA_KEY, Error, LEGACY_PREFIX, MOBILE_STATE_KEY, UNMAPPED_KEY, query};

const ROOT_ORDER: [&str; 8] = [
    "id",
    "resourcePackages",
    "sections",
    "config",
    "layoutOptimization",
    "filters",
    "pods",
    "publicCustomVisuals",
];
const CONFIG_ORDER: [&str; 11] = [
    "version",
    "themeCollection",
    "activeSectionIndex",
    "slowDataSourceSettings",
    "linguisticSchemaSyncVersion",
    "objects",
    "settings",
    "defaultDrillFilterOtherVisuals",
    "filterSortOrder",
    "bookmarks",
    "modelExtensions",
];
const SECTION_ORDER: [&str; 9] = [
    "name",
    "displayName",
    "filters",
    "ordinal",
    "visualContainers",
    "config",
    "displayOption",
    "width",
    "height",
];
const CONTAINER_ORDER: [&str; 8] = ["x", "y", "z", "width", "height", "config", "filters", "tabOrder"];

pub(crate) fn write(original: &str, report: &Report) -> Result<String, Error> {
    let base = read::read(original)?;
    // Static resources and custom visuals are the package's business (the PBIX writer adds them to
    // the report it reads); only `definition/` documents concern the layout.
    let pbir_docs = report
        .resources
        .iter()
        .any(|r| r.path.replace('\\', "/").starts_with("definition/"));
    let unchanged = bicraft_report::Report {
        resources: base.report.resources.clone(),
        ..report.clone()
    } == base.report;
    if !pbir_docs && unchanged {
        return Ok(original.to_string());
    }
    let mut errors = Vec::new();
    let mobile = mobile_documents(report, &mut errors);
    if report.theme_collection != base.report.theme_collection {
        errors.push("report: theme (themeCollection) changes cannot be written into a legacy layout".into());
    }
    if report.extra.get("resourcePackages") != base.report.extra.get("resourcePackages") {
        errors.push(
            "report: resourcePackages changes (themes, images) cannot be written into a legacy layout".into(),
        );
    }
    let root = report_to_legacy(report, &base, &mobile, &mut errors);
    if !errors.is_empty() {
        return Err(Error::Unwritable(errors));
    }
    serialize(original, root)
}

/// Phone layouts handed over as PBIR `mobile.json` resources (`(page, visual, state)`); any other
/// `definition/` document has no legacy form.
fn mobile_documents(report: &Report, errors: &mut Vec<String>) -> Vec<(String, String, Value)> {
    let mut out = Vec::new();
    for r in &report.resources {
        let path = r.path.replace('\\', "/");
        let Some(rest) = path.strip_prefix("definition/") else {
            continue;
        };
        let parts: Vec<&str> = rest.split('/').collect();
        let doc = match parts.as_slice() {
            ["pages", page, "visuals", visual, "mobile.json"] => serde_json::from_slice::<Value>(&r.bytes)
                .ok()
                .and_then(|v| match v {
                    Value::Object(mut m) => {
                        m.shift_remove("$schema");
                        Some(((*page).to_string(), (*visual).to_string(), Value::Object(m)))
                    }
                    _ => None,
                }),
            _ => None,
        };
        match doc {
            Some(d) => out.push(d),
            None => errors.push(format!("{path}: a PBIR document with no legacy layout form")),
        }
    }
    out
}

fn code_or_error(table: Table, v: &Value, what: &str, errors: &mut Vec<String>) -> Option<Value> {
    match v.as_str().and_then(|s| codes::code(table, s)) {
        Some(c) => Some(json!(c)),
        None => {
            errors.push(format!("{what} = {v} has no legacy layout code"));
            None
        }
    }
}

fn report_to_legacy(
    report: &Report,
    base: &read::Read,
    mobile: &[(String, String, Value)],
    errors: &mut Vec<String>,
) -> Value {
    let mut root = Map::new();
    if let Some(p) = report.extra.get("resourcePackages") {
        root.insert("resourcePackages".into(), packages_to_legacy(p, errors));
    }

    // Sections: original order for the pages that were there, new pages after them. Ordinals are
    // kept when the pages keep their relative order and new pages come last; otherwise the pages
    // are renumbered in report order.
    let base_index = |name: &str| base.report.pages.iter().position(|p| p.name == name);
    let existing: Vec<usize> = report.pages.iter().filter_map(|p| base_index(&p.name)).collect();
    let first_new = report.pages.iter().position(|p| base_index(&p.name).is_none());
    let keep_order = existing.windows(2).all(|w| matches!(w, [a, b] if a < b))
        && first_new.is_none_or(|i| report.pages.iter().skip(i).all(|p| base_index(&p.name).is_none()));
    let mut next_ordinal = base
        .sections
        .iter()
        .filter_map(|(_, o)| o.as_ref().and_then(Value::as_i64))
        .max()
        .map_or(0, |m| m.saturating_add(1));
    let mut sections: Vec<(usize, Value)> = Vec::new();
    for (i, page) in report.pages.iter().enumerate() {
        let bi = base_index(&page.name);
        let ordinal = match (keep_order, bi.and_then(|b| base.sections.get(b))) {
            (true, Some((_, o))) => o.clone(),
            (true, None) => {
                let o = next_ordinal;
                next_ordinal = next_ordinal.saturating_add(1);
                Some(json!(o))
            }
            (false, _) => Some(json!(i)),
        };
        let section = page_to_legacy(page, ordinal, mobile, errors);
        let order = bi
            .and_then(|b| base.sections.get(b))
            .map_or(usize::MAX, |(s, _)| *s);
        sections.push((order, section));
    }
    sections.sort_by_key(|(o, _)| *o);
    root.insert(
        "sections".into(),
        Value::Array(sections.into_iter().map(|(_, s)| s).collect()),
    );

    // config
    let mut config = Map::new();
    if let Some(t) = &report.theme_collection {
        config.insert("themeCollection".into(), themes_to_legacy(t));
    }
    if let Some(active) = &report.active_page {
        // An active page that is gone (removed) leaves the index out: Desktop opens the first.
        if let Some(i) = report.pages.iter().position(|p| &p.name == active) {
            config.insert("activeSectionIndex".into(), json!(i));
        }
    }
    let mut filter_list = Vec::new();
    if let Some(fc) = &report.filter_config {
        filter_config_to_legacy(fc, "report", &mut filter_list, &mut config, errors);
    }
    for (k, v) in &report.extra {
        match k.as_str() {
            "resourcePackages" | UNMAPPED_KEY | BOOKMARKS_METADATA_KEY => {}
            "objects" => {
                config.insert("objects".into(), v.clone());
            }
            "settings" => settings_to_legacy(v, &mut config, errors),
            // the query reduction options; the legacy name is *inferred* (legacy-layout-bb.md)
            "slowDataSourceSettings" => {
                config.insert(k.clone(), v.clone());
            }
            "publicCustomVisuals" => {
                root.insert(k.clone(), v.clone());
            }
            k if k.starts_with(LEGACY_PREFIX) => {}
            _ => errors.push(format!("report: {k}")),
        }
    }
    if let Some(b) = bookmarks_to_legacy(report) {
        config.insert("bookmarks".into(), b);
    }
    for (what, map) in [("pages", &report.pages_extra), ("version", &report.version_extra)] {
        for k in map.keys().filter(|k| !k.starts_with(LEGACY_PREFIX)) {
            errors.push(format!("{what}: {k}"));
        }
    }
    root.insert("config".into(), Value::Object(config));
    root.insert("filters".into(), Value::Array(filter_list));

    // Pods: the page bindings, which must be the ones read.
    let mut pods = Vec::new();
    for page in &report.pages {
        let Some(binding) = page.extra.get("pageBinding") else {
            continue;
        };
        let base_binding = base
            .report
            .pages
            .iter()
            .find(|p| p.name == page.name)
            .and_then(|p| p.extra.get("pageBinding"));
        if base_binding != Some(binding) {
            errors.push(format!(
                "page {}: page binding (pageBinding: tooltip or drillthrough setup) cannot be written into a legacy layout",
                page.name
            ));
            continue;
        }
        let mut pod = pod_to_legacy(binding, &page.name, errors);
        apply_unmapped(
            &mut pod,
            Some(&unmapped_under(page.extra.get(UNMAPPED_KEY), "pod")),
            &[],
        );
        pods.push(pod);
    }
    root.insert("pods".into(), Value::Array(pods));

    let mut root = Value::Object(root);
    apply_unmapped(&mut root, report.extra.get(UNMAPPED_KEY), &[]);
    if let Value::Object(m) = &mut root {
        reorder(m, &ROOT_ORDER);
        if let Some(Value::Object(c)) = m.get_mut("config") {
            reorder(c, &CONFIG_ORDER);
        }
    }
    root
}

fn packages_to_legacy(p: &Value, errors: &mut Vec<String>) -> Value {
    let mut out = Vec::new();
    for pkg in p.as_array().into_iter().flatten() {
        let mut m = Map::new();
        if let Some(n) = pkg.get("name") {
            m.insert("name".into(), n.clone());
        }
        if let Some(t) = pkg
            .get("type")
            .and_then(|t| code_or_error(codes::PACKAGE_TYPE, t, "report: resource package type", errors))
        {
            m.insert("type".into(), t);
        }
        let mut items = Vec::new();
        for it in pkg.get("items").and_then(Value::as_array).into_iter().flatten() {
            let mut i = Map::new();
            if let Some(t) = it
                .get("type")
                .and_then(|t| code_or_error(codes::ITEM_TYPE, t, "report: resource item type", errors))
            {
                i.insert("type".into(), t);
            }
            for k in ["path", "name"] {
                if let Some(v) = it.get(k) {
                    i.insert(k.into(), v.clone());
                }
            }
            items.push(Value::Object(i));
        }
        m.insert("items".into(), Value::Array(items));
        out.push(json!({"resourcePackage": m}));
    }
    Value::Array(out)
}

fn themes_to_legacy(t: &Value) -> Value {
    let mut out = Map::new();
    for (k, theme) in t.as_object().into_iter().flatten() {
        let mut m = Map::new();
        if let Some(n) = theme.get("name") {
            m.insert("name".into(), n.clone());
        }
        // The legacy version comes back from the unmapped map; failing that, from the table.
        if let Some(r) = theme.get("reportVersionAtImport") {
            let triple = [
                r.get("visual").and_then(Value::as_str),
                r.get("report").and_then(Value::as_str),
                r.get("page").and_then(Value::as_str),
            ];
            if let Some((v, _)) = codes::THEME_VERSIONS
                .iter()
                .find(|(_, t)| t.iter().map(|s| Some(*s)).eq(triple.iter().copied()))
            {
                m.insert("version".into(), json!(v));
            }
        }
        if let Some(c) = theme
            .get("type")
            .and_then(Value::as_str)
            .and_then(|s| codes::code(codes::THEME_TYPE, s))
        {
            m.insert("type".into(), json!(c));
        }
        out.insert(k.clone(), Value::Object(m));
    }
    Value::Object(out)
}

fn settings_to_legacy(s: &Value, config: &mut Object, errors: &mut Vec<String>) {
    let mut out = Map::new();
    for (k, v) in s.as_object().into_iter().flatten() {
        if k == "defaultDrillFilterOtherVisuals" {
            config.insert(k.clone(), v.clone());
        } else if let Some((_, table)) = SETTING_CODES.iter().find(|(n, _)| n == k) {
            if let Some(c) = code_or_error(table, v, &format!("report: settings.{k}"), errors) {
                out.insert(k.clone(), c);
            }
        } else {
            out.insert(k.clone(), v.clone());
        }
    }
    if !out.is_empty() {
        config.insert("settings".into(), Value::Object(out));
    }
}

fn bookmarks_to_legacy(report: &Report) -> Option<Value> {
    if report.bookmarks.is_empty() && !report.extra.contains_key(BOOKMARKS_METADATA_KEY) {
        return None;
    }
    let doc = |name: &str| -> Option<Value> {
        let b = report.bookmarks.iter().find(|b| b.name == name)?;
        let mut d = b.document.clone();
        if let Value::Object(m) = &mut d {
            m.shift_remove("$schema");
            m.entry("name".to_string()).or_insert_with(|| json!(name));
        }
        Some(d)
    };
    let mut out = Vec::new();
    let mut done: Vec<String> = Vec::new();
    let items = report
        .extra
        .get(BOOKMARKS_METADATA_KEY)
        .and_then(|m| m.get("items"))
        .and_then(Value::as_array);
    for item in items.into_iter().flatten() {
        let Some(name) = item.get("name").and_then(Value::as_str) else {
            continue;
        };
        if let Some(children) = item.get("children").and_then(Value::as_array) {
            let mut group = Map::new();
            group.insert("name".into(), json!(name));
            if let Some(d) = item.get("displayName") {
                group.insert("displayName".into(), d.clone());
            }
            let mut list = Vec::new();
            for c in children.iter().filter_map(Value::as_str) {
                if let Some(d) = doc(c) {
                    list.push(d);
                    done.push(c.to_string());
                }
            }
            group.insert("children".into(), Value::Array(list));
            out.push(Value::Object(group));
        } else if let Some(d) = doc(name) {
            out.push(d);
            done.push(name.to_string());
        }
    }
    for b in &report.bookmarks {
        if !done.contains(&b.name)
            && let Some(d) = doc(&b.name)
        {
            out.push(d);
        }
    }
    Some(Value::Array(out))
}

/// `filterConfig` → legacy filter list (into `filters`) and `filterSortOrder` (into `target`).
fn filter_config_to_legacy(
    fc: &Value,
    at: &str,
    filters: &mut Vec<Value>,
    target: &mut Object,
    errors: &mut Vec<String>,
) {
    for (k, v) in fc.as_object().into_iter().flatten() {
        match k.as_str() {
            "filters" => {
                for f in v.as_array().into_iter().flatten() {
                    filters.push(filter_to_legacy(f, at, errors));
                }
            }
            "filterSortOrder" => {
                if let Some(c) = code_or_error(
                    codes::FILTER_SORT_ORDER,
                    v,
                    &format!("{at}: filterSortOrder"),
                    errors,
                ) {
                    target.insert("filterSortOrder".into(), c);
                }
            }
            other => errors.push(format!("{at}: filterConfig.{other}")),
        }
    }
}

fn filter_to_legacy(f: &Value, at: &str, errors: &mut Vec<String>) -> Value {
    let mut out = Map::new();
    let Some(o) = f.as_object() else {
        return f.clone();
    };
    if let Some(n) = o.get("name") {
        out.insert("name".into(), n.clone());
    }
    if let Some(e) = o.get("field") {
        out.insert("expression".into(), e.clone());
    }
    let mut cached = None;
    if let Some(flt) = o.get("filter") {
        let mut flt = flt.clone();
        cached = o.get("field").and_then(|field| unannotate(&mut flt, field));
        out.insert("filter".into(), flt);
    }
    if let Some(t) = o.get("type") {
        out.insert("type".into(), t.clone());
    }
    for (k, v) in o {
        match k.as_str() {
            "name" | "field" | "filter" | "type" => {}
            "howCreated" => {
                if let Some(c) =
                    code_or_error(codes::HOW_CREATED, v, &format!("{at}: filter howCreated"), errors)
                {
                    out.insert("howCreated".into(), c);
                }
            }
            "displayName" | "ordinal" | "objects" | "isHiddenInViewMode" | "isLockedInViewMode"
            | "restatement" => {
                out.insert(k.clone(), v.clone());
            }
            other => errors.push(format!("{at}: filter {other}")),
        }
    }
    if let Some(c) = cached {
        out.insert("cachedDisplayNames".into(), c);
    }
    Value::Object(out)
}

/// The inverse of `read::annotate`: the condition annotation back into `cachedDisplayNames`.
fn unannotate(filter: &mut Value, field: &Value) -> Option<Value> {
    let Some(Value::Array(wheres)) = filter.get_mut("Where") else {
        return None;
    };
    let [Value::Object(w)] = wheres.as_mut_slice() else {
        return None;
    };
    let meta = w.get("Annotations")?.get("filterExpressionMetadata")?;
    if w.get("Annotations")?.as_object()?.len() != 1
        || meta.get("expressions") != Some(&json!([field]))
        || meta.get("decomposedIdentities").and_then(|d| d.get("columns")) != Some(&json!([{"value": field}]))
    {
        return None;
    }
    let values = meta.get("decomposedIdentities")?.get("values")?.as_array()?;
    let names = meta.get("valueMap")?.as_array()?;
    if values.len() != names.len() {
        return None;
    }
    let mut out = Vec::new();
    for (v, n) in values.iter().zip(names) {
        let lit = v.get(0)?.get("0")?.get(0)?;
        let name = n.get("0")?;
        out.push(
            json!({"id": {"scopeId": {"Comparison": {"ComparisonKind": 0, "Left": field, "Right": lit}}},
                        "displayName": name}),
        );
    }
    w.shift_remove("Annotations");
    Some(Value::Array(out))
}

fn pod_to_legacy(binding: &Value, page: &str, errors: &mut Vec<String>) -> Value {
    let mut pod = Map::new();
    let mut config = Map::new();
    let at = format!("page {page}: pageBinding");
    for (k, v) in binding.as_object().into_iter().flatten() {
        match k.as_str() {
            "name" => {
                pod.insert("name".into(), v.clone());
            }
            "type" => {
                if let Some(c) = code_or_error(codes::BINDING_TYPE, v, &format!("{at}.type"), errors) {
                    pod.insert("type".into(), c);
                }
            }
            "parameters" => {
                pod.insert("parameters".into(), v.clone());
            }
            "referenceScope" => {
                if let Some(c) =
                    code_or_error(codes::REFERENCE_SCOPE, v, &format!("{at}.referenceScope"), errors)
                {
                    pod.insert("referenceScope".into(), c);
                }
            }
            "acceptsFilterContext" => {
                if let Some(c) = code_or_error(
                    codes::ACCEPTS_FILTER_CONTEXT,
                    v,
                    &format!("{at}.acceptsFilterContext"),
                    errors,
                ) {
                    config.insert("acceptsFilterContext".into(), c);
                }
            }
            other => errors.push(format!("{at}.{other}")),
        }
    }
    pod.insert("boundSection".into(), json!(page));
    pod.insert("config".into(), Value::Object(config));
    pod.entry("parameters".to_string()).or_insert_with(|| json!([]));
    reorder(
        &mut pod,
        &[
            "name",
            "boundSection",
            "config",
            "referenceScope",
            "parameters",
            "type",
        ],
    );
    Value::Object(pod)
}

fn page_to_legacy(
    page: &Page,
    ordinal: Option<Value>,
    mobile: &[(String, String, Value)],
    errors: &mut Vec<String>,
) -> Value {
    let at = format!("page {}", page.name);
    let mut s = Map::new();
    s.insert("name".into(), json!(page.name));
    s.insert("displayName".into(), json!(page.display_name));
    let mut config = Map::new();
    let mut filters = Vec::new();
    if let Some(fc) = &page.filter_config {
        filter_config_to_legacy(fc, &at, &mut filters, &mut config, errors);
    }
    s.insert("filters".into(), Value::Array(filters));
    if let Some(o) = ordinal {
        s.insert("ordinal".into(), o);
    }
    let containers: Vec<Value> = page
        .visuals
        .iter()
        .map(|v| {
            let state = mobile
                .iter()
                .find(|(p, n, _)| *p == page.name && *n == v.name)
                .map(|(_, _, s)| s);
            visual_to_legacy(v, &page.name, state, errors)
        })
        .collect();
    s.insert("visualContainers".into(), Value::Array(containers));
    for (k, v) in &page.extra {
        match k.as_str() {
            "objects" => {
                config.insert("objects".into(), v.clone());
            }
            "visualInteractions" => {
                let mut list = Vec::new();
                for r in v.as_array().into_iter().flatten() {
                    let ty = r.get("type").and_then(|t| {
                        code_or_error(codes::INTERACTION, t, &format!("{at}: interaction type"), errors)
                    });
                    list.push(json!({"source": r.get("source"), "target": r.get("target"), "type": ty}));
                }
                config.insert("relationships".into(), Value::Array(list));
            }
            "type" => {
                if let Some(c) = code_or_error(codes::PAGE_TYPE, v, &format!("{at}: type"), errors) {
                    config.insert("type".into(), c);
                }
            }
            "pageBinding" | UNMAPPED_KEY => {}
            k if k.starts_with(LEGACY_PREFIX) => {}
            other => errors.push(format!("{at}: {other}")),
        }
    }
    if let Some(v) = &page.visibility
        && let Some(c) = code_or_error(codes::VISIBILITY, &json!(v), &format!("{at}: visibility"), errors)
    {
        config.insert("visibility".into(), c);
    }
    reorder(
        &mut config,
        &[
            "objects",
            "relationships",
            "visibility",
            "type",
            "filterSortOrder",
        ],
    );
    s.insert("config".into(), Value::Object(config));
    if let Some(d) = &page.display_option
        && let Some(c) = code_or_error(
            codes::DISPLAY_OPTION,
            &json!(d),
            &format!("{at}: displayOption"),
            errors,
        )
    {
        s.insert("displayOption".into(), c);
    }
    s.insert("width".into(), float(page.width));
    s.insert("height".into(), float(page.height));
    let mut s = Value::Object(s);
    apply_unmapped(&mut s, page.extra.get(UNMAPPED_KEY), &["pod"]);
    if let Value::Object(m) = &mut s {
        reorder(m, &SECTION_ORDER);
    }
    s
}

fn visual_to_legacy(v: &Visual, page: &str, mobile_doc: Option<&Value>, errors: &mut Vec<String>) -> Value {
    let at = format!("visual {page}/{}", v.name);
    let p = &v.position;
    let mut c = Map::new();
    let mut pos = Map::new();
    for (k, x) in [
        ("x", p.x),
        ("y", p.y),
        ("z", p.z),
        ("width", p.width),
        ("height", p.height),
    ] {
        c.insert(k.into(), float(x));
        pos.insert(k.into(), float(x));
    }
    let tab = match (
        p.tab_order,
        v.extra.get("position").and_then(|x| x.get("tabOrder")),
    ) {
        (Some(t), _) => Some(float(t as f64)),
        (None, Some(t)) => Some(t.clone()),
        (None, None) => None,
    };
    if let Some(t) = &tab {
        c.insert("tabOrder".into(), t.clone());
        pos.insert("tabOrder".into(), t.clone());
    }
    let mut config = Map::new();
    config.insert("name".into(), json!(v.name));
    let mut layouts = vec![json!({"id": 0, "position": pos})];

    let mut sv = Map::new();
    let mut filters = Vec::new();
    if let Some(fc) = &v.filter_config {
        filter_config_to_legacy(fc, &at, &mut filters, &mut sv, errors);
    }
    let mobile_state = mobile_doc.or_else(|| v.extra.get(MOBILE_STATE_KEY));
    if let Some(m) = mobile_state {
        for (k, x) in m.as_object().into_iter().flatten() {
            match k.as_str() {
                "position" => layouts.push(json!({"id": 1, "position": x})),
                "objects" => {
                    sv.insert("mobileObjects".into(), x.clone());
                }
                "visualContainerObjects" => {
                    sv.insert("mobileVcObjects".into(), x.clone());
                }
                other => errors.push(format!("{at}: phone layout {other}")),
            }
        }
    }
    config.insert("layouts".into(), Value::Array(layouts));

    let group = v.extra.get("visualGroup");
    if let Some(g) = group {
        let mut out = Map::new();
        for (k, x) in g.as_object().into_iter().flatten() {
            match k.as_str() {
                "displayName" => {
                    out.insert(k.clone(), x.clone());
                }
                "groupMode" => {
                    if let Some(c) = code_or_error(codes::GROUP_MODE, x, &format!("{at}: groupMode"), errors)
                    {
                        out.insert(k.clone(), c);
                    }
                }
                other => errors.push(format!("{at}: visualGroup.{other}")),
            }
        }
        config.insert("singleVisualGroup".into(), Value::Object(out));
        if v.visual_type.is_some() || !v.visual_extra.is_empty() || !v.fields.is_empty() || v.is_hidden {
            errors.push(format!("{at}: a visual group with visual properties"));
        }
        for k in sv.keys() {
            errors.push(format!("{at}: {k} on a visual group"));
        }
    } else {
        let mut single = Map::new();
        if let Some(t) = &v.visual_type {
            single.insert("visualType".into(), json!(t));
        }
        match bicraft_report::query::effective(v) {
            Some(q) => query::to_legacy(&q, &mut single, &at, errors),
            None => {
                single.insert("projections".into(), json!({}));
            }
        }
        for (k, x) in &v.visual_extra {
            match k.as_str() {
                "query" => {}
                "objects"
                | "expansionStates"
                | "syncGroup"
                | "drillFilterOtherVisuals"
                | "autoSelectVisualType" => {
                    single.insert(k.clone(), x.clone());
                }
                "visualContainerObjects" => {
                    single.insert("vcObjects".into(), x.clone());
                }
                k if k.starts_with(LEGACY_PREFIX) => {}
                other => errors.push(format!("{at}: visual.{other}")),
            }
        }
        if v.is_hidden {
            single.insert("display".into(), json!({"mode": "hidden"}));
        }
        single.extend(sv);
        config.insert("singleVisual".into(), Value::Object(single));
    }
    if let Some(g) = &v.parent_group {
        config.insert("parentGroupName".into(), json!(g));
    }
    for (k, x) in &v.extra {
        match k.as_str() {
            "visualGroup" | "position" | MOBILE_STATE_KEY | UNMAPPED_KEY => {}
            "howCreated" => {
                config.insert(k.clone(), x.clone());
            }
            k if k.starts_with(LEGACY_PREFIX) => {}
            other => errors.push(format!("{at}: {other}")),
        }
    }
    if let Some(Value::Object(pos)) = v.extra.get("position") {
        for k in pos.keys().filter(|k| k.as_str() != "tabOrder") {
            errors.push(format!("{at}: position.{k}"));
        }
    }
    c.insert("config".into(), Value::Object(config));
    c.insert("filters".into(), Value::Array(filters));
    let mut c = Value::Object(c);
    apply_unmapped(&mut c, v.extra.get(UNMAPPED_KEY), &[]);
    if let Some(Value::Object(sv)) = c.get_mut("config").and_then(|c| c.get_mut("singleVisual")) {
        query::unresolve_kept_selects(sv);
    }
    if let Value::Object(m) = &mut c {
        reorder(m, &CONTAINER_ORDER);
    }
    c
}

// ------------------------------------------------------------------------------------------------
// Serialization

fn decode_slots(v: &Value, keys: &[&str]) -> Value {
    let mut v = v.clone();
    if let Value::Object(m) = &mut v {
        for k in keys {
            if let Ok(Some(p)) = slot(m.get(*k))
                && let Some(x) = m.get_mut(*k)
            {
                *x = p;
            }
        }
    }
    v
}

fn decode_container(v: &Value) -> Value {
    decode_slots(v, &["config", "filters", "query", "dataTransforms"])
}

fn decode_section(v: &Value) -> Value {
    let mut s = decode_slots(v, &["config", "filters"]);
    if let Some(Value::Array(list)) = s.get_mut("visualContainers") {
        for c in list.iter_mut() {
            *c = decode_container(c);
        }
    }
    s
}

fn decode_top(key: &str, v: &Value) -> Value {
    match key {
        "config" | "filters" => slot(Some(v)).ok().flatten().unwrap_or_else(|| v.clone()),
        "pods" => match v {
            Value::Array(a) => Value::Array(
                a.iter()
                    .map(|p| decode_slots(p, &["config", "parameters"]))
                    .collect(),
            ),
            other => other.clone(),
        },
        "sections" => match v {
            Value::Array(a) => Value::Array(a.iter().map(decode_section).collect()),
            other => other.clone(),
        },
        _ => v.clone(),
    }
}

fn encode_container(v: &Value) -> Value {
    let mut v = v.clone();
    if let Value::Object(m) = &mut v {
        for k in ["config", "filters", "query", "dataTransforms"] {
            encode_slot(m, k);
        }
    }
    v
}

fn encode_section_shallow(v: &Value) -> Value {
    let mut v = v.clone();
    if let Value::Object(m) = &mut v {
        encode_slot(m, "config");
        encode_slot(m, "filters");
    }
    v
}

fn encode_top(key: &str, v: &Value) -> Value {
    match (key, v) {
        ("config" | "filters", x) if !x.is_string() => Value::String(x.to_string()),
        ("pods", Value::Array(a)) => Value::Array(
            a.iter()
                .map(|p| {
                    let mut p = p.clone();
                    if let Value::Object(m) = &mut p {
                        encode_slot(m, "config");
                        encode_slot(m, "parameters");
                    }
                    p
                })
                .collect(),
        ),
        _ => v.clone(),
    }
}

fn text(v: &Value) -> Result<String, Error> {
    serde_json::to_string(v).map_err(|e| Error::Json(e.to_string()))
}

fn key_text(k: &str) -> Result<String, Error> {
    text(&Value::String(k.to_string()))
}

/// The raw text of the original section named `name`, if any, and its containers by name.
struct RawSection<'a> {
    raw: &'a RawValue,
    keys: Vec<String>,
    containers: Vec<(Option<String>, Vec<String>, &'a RawValue)>,
}

fn is_empty(v: &Value) -> bool {
    match v {
        Value::Array(a) => a.is_empty(),
        Value::Object(m) => m.is_empty(),
        Value::String(s) => s.is_empty() || s == "[]" || s == "{}",
        _ => false,
    }
}

/// Drops the empty keys the original object did not have (a regenerated object adds `filters:
/// []`, `pods: []`… that the original left out).
fn prune(v: &Value, keys: &[String]) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .filter(|(k, x)| keys.contains(k) || !is_empty(x))
                .map(|(k, x)| (k.clone(), x.clone()))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn serialize(original: &str, root: Value) -> Result<String, Error> {
    let bom = original.starts_with('\u{feff}');
    let body = original.trim_start_matches('\u{feff}');
    let top: indexmap_like::Ordered<'_> =
        serde_json::from_str(body).map_err(|e: serde_json::Error| Error::Json(e.to_string()))?;
    let Value::Object(root) = root else {
        return Err(Error::NotObject);
    };

    // Original sections, by name.
    let mut raw_sections: Vec<(Option<String>, RawSection<'_>)> = Vec::new();
    if let Some(sections) = top.get("sections")
        && let Ok(list) = serde_json::from_str::<Vec<&RawValue>>(sections.get())
    {
        for s in list {
            let parsed: Value = serde_json::from_str(s.get()).unwrap_or(Value::Null);
            let name = parsed.get("name").and_then(Value::as_str).map(str::to_string);
            let mut containers = Vec::new();
            if let Ok(fields) = serde_json::from_str::<indexmap_like::Ordered<'_>>(s.get())
                && let Some(vc) = fields.get("visualContainers")
                && let Ok(list) = serde_json::from_str::<Vec<&RawValue>>(vc.get())
            {
                for c in list {
                    let cv: Value = serde_json::from_str(c.get()).unwrap_or(Value::Null);
                    let cname = decode_container(&cv)
                        .get("config")
                        .and_then(|x| x.get("name"))
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    let keys = cv
                        .as_object()
                        .map(|m| m.keys().cloned().collect())
                        .unwrap_or_default();
                    containers.push((cname, keys, c));
                }
            }
            let keys = parsed
                .as_object()
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default();
            raw_sections.push((
                name,
                RawSection {
                    raw: s,
                    keys,
                    containers,
                },
            ));
        }
    }

    let mut out = String::with_capacity(body.len().saturating_add(1024));
    if bom {
        out.push('\u{feff}');
    }
    out.push('{');
    let root: Map<String, Value> = root
        .into_iter()
        .filter(|(k, v)| top.get(k).is_some() || !is_empty(v))
        .collect();
    for (i, (k, v)) in root.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&key_text(k)?);
        out.push(':');
        if k == "sections"
            && let Value::Array(sections) = v
        {
            out.push('[');
            for (j, s) in sections.iter().enumerate() {
                if j > 0 {
                    out.push(',');
                }
                let name = s.get("name").and_then(Value::as_str);
                let orig = raw_sections
                    .iter()
                    .find(|(n, _)| n.as_deref() == name && name.is_some())
                    .map(|(_, r)| r);
                let s = match orig {
                    Some(o) => prune(s, &o.keys),
                    None => s.clone(),
                };
                out.push_str(&section_text(&s, orig)?);
            }
            out.push(']');
            continue;
        }
        let same = top.get(k).filter(|r| {
            serde_json::from_str::<Value>(r.get()).is_ok_and(|o| same(&decode_top(k, &o), &decode_top(k, v)))
        });
        match same {
            Some(r) => out.push_str(r.get()),
            None => out.push_str(&text(&encode_top(k, v))?),
        }
    }
    out.push('}');
    Ok(out)
}

fn section_text(s: &Value, orig: Option<&RawSection<'_>>) -> Result<String, Error> {
    if let Some(o) = orig
        && serde_json::from_str::<Value>(o.raw.get())
            .is_ok_and(|x| same(&decode_section(&x), &decode_section(s)))
    {
        return Ok(o.raw.get().to_string());
    }
    let Value::Object(m) = encode_section_shallow(s) else {
        return text(s);
    };
    let mut out = String::from("{");
    for (i, (k, v)) in m.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&key_text(k)?);
        out.push(':');
        if k == "visualContainers"
            && let Value::Array(list) = v
        {
            out.push('[');
            for (j, c) in list.iter().enumerate() {
                if j > 0 {
                    out.push(',');
                }
                let name = c
                    .get("config")
                    .and_then(|x| x.get("name"))
                    .and_then(Value::as_str);
                let found = orig.and_then(|o| {
                    o.containers
                        .iter()
                        .find(|(n, _, _)| n.as_deref() == name && name.is_some())
                });
                let pruned;
                let c = match found {
                    Some((_, keys, _)) => {
                        pruned = prune(c, keys);
                        &pruned
                    }
                    None => c,
                };
                let raw = found.map(|(_, _, r)| *r);
                match raw.filter(|r| {
                    serde_json::from_str::<Value>(r.get())
                        .is_ok_and(|x| same(&decode_container(&x), &decode_container(c)))
                }) {
                    Some(r) => out.push_str(r.get()),
                    None => out.push_str(&text(&encode_container(c))?),
                }
            }
            out.push(']');
        } else {
            out.push_str(&text(v)?);
        }
    }
    out.push('}');
    Ok(out)
}

/// A JSON object's members as raw text, in document order.
mod indexmap_like {
    use serde::de::{Deserialize, Deserializer, MapAccess, Visitor};
    use serde_json::value::RawValue;

    pub struct Ordered<'a>(pub Vec<(String, &'a RawValue)>);

    impl<'a> Ordered<'a> {
        pub fn get(&self, key: &str) -> Option<&'a RawValue> {
            self.0.iter().find(|(k, _)| k == key).map(|(_, v)| *v)
        }
    }

    impl<'de: 'a, 'a> Deserialize<'de> for Ordered<'a> {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct V<'a>(std::marker::PhantomData<&'a ()>);
            impl<'de: 'a, 'a> Visitor<'de> for V<'a> {
                type Value = Ordered<'a>;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("a JSON object")
                }
                fn visit_map<M: MapAccess<'de>>(self, mut m: M) -> Result<Self::Value, M::Error> {
                    let mut out = Vec::new();
                    while let Some((k, v)) = m.next_entry::<String, &'de RawValue>()? {
                        out.push((k, v));
                    }
                    Ok(Ordered(out))
                }
            }
            d.deserialize_map(V(std::marker::PhantomData))
        }
    }
}

/// JSON equality with numbers compared by value (`0.0` equals `0`): the original text of an
/// object is kept when only the spelling of its numbers would change.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w)))
        }
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(v, w)| same(v, w)),
        _ => a == b,
    }
}
