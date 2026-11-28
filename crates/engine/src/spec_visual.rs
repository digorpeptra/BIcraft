//! The BIcraft Spec visual (ROADMAP M11.32, docs/formats/spec-visual.md): one custom visual of
//! BIcraft's whose script BIcraft writes (`spec_visual/visual.ts` and `spec.ts`, compiled by
//! `visual.build`'s swc bundler) and which draws a declarative JSON spec stored in its
//! `spec.json` format property. `visual.set_spec` checks a spec with the renderer's parser
//! (`bicraft_render::spec`, the same layout as the TypeScript), embeds the package in the report
//! when it is missing or older, turns the visual into a Spec visual and stores the spec;
//! `visual.spec_template` hands out starting specs; `visual.scaffold {template: "spec"}` writes
//! the visual's project to edit. Its interactions come from BIcraft's visual runtime
//! (`crate::visual_runtime`, ROADMAP M12.2). The snapshot carries the parsed spec (`spec`), so
//! `report.render`, PDF export and the app draw it natively.

use std::sync::OnceLock;

use bicraft_render::spec;
use bicraft_report::{FieldBinding, FieldRef, Visual};
use bicraft_visuals::RoleKind;
use serde_json::{Map, Value, json};

use crate::Session;
use crate::builtin_visuals::{Built, BuiltinVisual};
use crate::params::{allow, opt_str, str_param};
use crate::report::{page_at_mut, page_index, parts, visual_index};
use crate::visual_project::{API_VERSION, Project};

/// The visual's guid: its visual type in reports (letters and digits, starting with a letter).
pub const SPEC_GUID: &str = "bicraftSpecVisual7F3A9C2E5B1D4E6F8A0C2B4D";
/// Its name in the Visualizations pane.
pub const SPEC_DISPLAY_NAME: &str = "BIcraft Spec";
/// Its version; raised with every change of its TypeScript.
pub const SPEC_PACKAGE_VERSION: &str = "1.5.1.0";
/// The data roles and their capabilities.json names.
pub const CATEGORY_ROLE: &str = "category";
pub const VALUES_ROLE: &str = "values";

/// The visual's sources.
pub(crate) const VISUAL_TS: &str = include_str!("spec_visual/visual.ts");
pub(crate) const SPEC_TS: &str = include_str!("spec_visual/spec.ts");
pub(crate) const CSS: &str = include_str!("spec_visual/visual.css");

/// The visual's `capabilities.json`: a category and up to 16 measures in one categorical
/// mapping (1,000 categories at most), the `spec.json` text property, and what BIcraft's visual
/// runtime needs (tooltips, landing page, keyboard focus, and `supportsHighlight`: another
/// visual's selection highlights this one, its bars drawing their highlighted part over the
/// dimmed full value and its other marks dimmed where nothing is highlighted, as the renderer
/// draws them; docs/formats/bicraft-visuals.md "Highlights").
pub(crate) fn capabilities() -> Value {
    let caps = json!({
        "dataRoles": [
            {"displayName": "Category", "name": CATEGORY_ROLE, "kind": "Grouping"},
            {"displayName": "Values", "name": VALUES_ROLE, "kind": "Measure"}
        ],
        "dataViewMappings": [{
            "conditions": [{CATEGORY_ROLE: {"max": 1}, VALUES_ROLE: {"max": spec::MAX_VALUES}}],
            "categorical": {
                "categories": {"for": {"in": CATEGORY_ROLE}, "dataReductionAlgorithm": {"top": {"count": spec::MAX_ROWS}}},
                "values": {"select": [{"bind": {"to": VALUES_ROLE}}]}
            }
        }],
        "privileges": []
    });
    // the spec, the text settings (data labels, axes, legend: docs/formats/spec-visual.md
    // "Text"), then the Style card of BIcraft's look
    let mut objects = Map::new();
    objects.insert(
        "spec".into(),
        json!({"displayName": "Spec", "properties": {"json": {"displayName": "Spec (JSON)", "type": {"text": true}}}}),
    );
    objects.extend(crate::visual_runtime::text_objects());
    objects.extend(crate::bvis::objects_of(crate::bvis::STYLE_PROPS.iter()));
    let mut caps = caps;
    if let Some(m) = caps.as_object_mut() {
        m.insert("objects".into(), Value::Object(objects));
    }
    crate::visual_runtime::with_capabilities(caps, crate::visual_runtime::Uses { highlight: true })
}

/// The visual's `capabilities.json` as its package carries it (tests read its format objects).
pub fn capabilities_json() -> Value {
    capabilities()
}

/// The visual's styles followed by the runtime's.
pub(crate) fn css() -> String {
    format!("{CSS}{}", crate::visual_runtime::CSS)
}

/// The data roles as `visual.scaffold` and `visual.spec_template` list them.
pub(crate) fn roles_json() -> Vec<Value> {
    vec![
        json!({"name": CATEGORY_ROLE, "kind": "Grouping", "display_name": "Category"}),
        json!({"name": VALUES_ROLE, "kind": "Measure", "display_name": "Values"}),
    ]
}

/// The visual as a project in memory (what `visual.scaffold {template: "spec"}` writes).
fn project() -> Result<Project, String> {
    let pbiviz = json!({
        "visual": {
            "name": "bicraftSpec",
            "displayName": SPEC_DISPLAY_NAME,
            "guid": SPEC_GUID,
            "visualClassName": "Visual",
            "version": SPEC_PACKAGE_VERSION,
            "description": "Draws a declarative chart spec written by BIcraft (docs/formats/spec-visual.md)",
            "supportUrl": "",
            "gitHubUrl": "",
        },
        "apiVersion": API_VERSION,
        "author": {"name": "BIcraft", "email": ""},
    });
    let mut sources = std::collections::BTreeMap::new();
    sources.insert("src/visual.ts".to_string(), VISUAL_TS.to_string());
    sources.insert("src/spec.ts".to_string(), SPEC_TS.to_string());
    crate::visual_runtime::add_sources(&mut sources);
    let mut caps = capabilities();
    let string_resources = crate::visual_runtime::localize(&mut caps)?;
    Ok(Project {
        folder: std::path::PathBuf::new(),
        pbiviz,
        guid: SPEC_GUID.into(),
        display_name: SPEC_DISPLAY_NAME.into(),
        version: SPEC_PACKAGE_VERSION.into(),
        class_name: "Visual".into(),
        api_version: API_VERSION.into(),
        capabilities: caps,
        css: css(),
        icon: Some(crate::builtin_visuals::icon_png(SPEC_GUID)?),
        sources,
        entry: "src/visual.ts".into(),
        string_resources,
        warnings: Vec::new(),
    })
}

/// The `.pbiviz` package and its `.pbiviz.json` entry, built once per process.
fn built() -> Result<&'static Built, String> {
    static BUILT: OnceLock<Result<Built, String>> = OnceLock::new();
    crate::builtin_visuals::build_once(&BUILT, SPEC_DISPLAY_NAME, project)
}

/// The Spec visual's entry in BIcraft's visuals (`crate::builtin_visuals`, ROADMAP M12.4).
pub(crate) const SPEC: BuiltinVisual = BuiltinVisual {
    name: "bicraftSpec",
    guid: SPEC_GUID,
    display_name: SPEC_DISPLAY_NAME,
    version: SPEC_PACKAGE_VERSION,
    roles_of: |_| {
        vec![
            (CATEGORY_ROLE, RoleKind::Grouping, 1),
            (VALUES_ROLE, RoleKind::Measure, spec::MAX_VALUES),
        ]
    },
    package: |_| built(),
    icon: crate::builtin_visuals::icon_png,
};

/// The visual's `.pbiviz` bytes (the tests and `visual.spec_template {package}` read them).
pub fn package_bytes() -> Result<Vec<u8>, String> {
    built().map(|(p, _)| p.clone())
}

/// The bundled script of the visual (the golden tests run its layout).
pub fn layout_script() -> Result<String, String> {
    let mut files = std::collections::BTreeMap::new();
    files.insert("src/spec.ts".to_string(), SPEC_TS.to_string());
    crate::visual_runtime::add_sources(&mut files);
    bicraft_tsbuild::bundle(&files, "src/spec.ts").map(|o| o.js)
}

/// The text of a visual's `spec.json` property.
pub(crate) fn spec_text(v: &Visual) -> Option<String> {
    let lit = v
        .visual_extra
        .get("objects")?
        .get("spec")?
        .as_array()?
        .iter()
        .find(|e| e.get("selector").is_none_or(Value::is_null))?
        .get("properties")?
        .get("json")?
        .get("expr")?
        .get("Literal")?
        .get("Value")?
        .as_str()?;
    let inner = lit.strip_prefix('\'')?.strip_suffix('\'')?;
    Some(inner.replace("''", "'"))
}

/// Adds `spec` (the parsed spec) or `spec_error` to a Spec visual's snapshot object.
pub(crate) fn snapshot_fields(o: &mut Map<String, Value>, v: &Visual) {
    if !v
        .visual_type
        .as_deref()
        .is_some_and(|t| t.eq_ignore_ascii_case(SPEC_GUID))
    {
        return;
    }
    match spec_text(v).as_deref().map(spec::parse_text) {
        Some(Ok((mut value, _))) => {
            // the Style card's settings over the spec's own `style`
            let own = crate::bvis::values_of(crate::bvis::STYLE_PROPS.iter(), v);
            if let Some(own) = own.get("style").and_then(Value::as_object)
                && let Some(spec) = value.as_object_mut()
            {
                let style = spec.entry("style").or_insert_with(|| json!({}));
                if let Some(style) = style.as_object_mut() {
                    for (k, x) in own {
                        // only what the spec's `style` takes (a stray value is not drawn)
                        let ok = crate::bvis::STYLE_PROPS
                            .iter()
                            .find(|p| p.property == k)
                            .is_some_and(|p| match p.kind {
                                crate::bvis::PropKind::Enum(list) => {
                                    x.as_str().is_some_and(|s| list.contains(&s))
                                }
                                crate::bvis::PropKind::Bool => x.is_boolean(),
                                _ => false,
                            });
                        if ok {
                            style.insert(k.clone(), x.clone());
                        }
                    }
                }
            }
            // the text cards' settings over the spec's own `text` (each checked as the layout
            // takes it, so the spec still parses)
            let own = crate::visual_runtime::text_of(v);
            if !own.is_empty()
                && let Some(spec) = value.as_object_mut()
            {
                let text = spec.entry("text").or_insert_with(|| json!({}));
                if let Some(text) = text.as_object_mut() {
                    for (part, settings) in own {
                        let into = text.entry(part).or_insert_with(|| json!({}));
                        if let (Some(into), Some(settings)) = (into.as_object_mut(), settings.as_object()) {
                            for (k, x) in settings {
                                into.insert(k.clone(), x.clone());
                            }
                        }
                    }
                }
            }
            o.insert("spec".into(), value);
        }
        Some(Err(e)) => {
            o.insert("spec_error".into(), json!(e));
        }
        None => {
            o.insert(
                "spec_error".into(),
                json!("the visual has no spec (visual.set_spec)"),
            );
        }
    }
}

/// The starting spec of a Spec visual that has none (one added from the gallery, by
/// `visual.add`, `visual.set_type` or `report.apply`): `visual.spec_template`'s `bar`, which
/// draws each bound measure as a series of its own, as a clustered column chart does
/// (docs/formats/spec-visual.md "Series"; until 1.5.0.0 two values or more took the `combo`,
/// whose line on the bars' scale hid a small second measure). Returns the kind written, `None`
/// when `v` is not a Spec visual or already has a spec.
pub(crate) fn default_spec(v: &mut Visual) -> Result<Option<&'static str>, String> {
    if !v
        .visual_type
        .as_deref()
        .is_some_and(|t| t.eq_ignore_ascii_case(SPEC_GUID))
        || spec_text(v).is_some()
    {
        return Ok(None);
    }
    let kind = "bar";
    let t = spec::template(kind).ok_or("internal: no spec template")?;
    let compact = serde_json::to_string(&t).map_err(|e| format!("cannot write the spec: {e}"))?;
    let list = crate::rich::object_list(v, "objects", "spec")?;
    let props = crate::rich::plain_props(list)?;
    props.insert("json".into(), crate::rich::text_lit(&compact));
    Ok(Some(kind))
}

fn is_value_field(f: &FieldRef) -> bool {
    matches!(f, FieldRef::Measure { .. } | FieldRef::Aggregation { .. })
}

fn field_label(f: &FieldRef) -> String {
    match f {
        FieldRef::Column { table, column } => format!("{table}[{column}]"),
        FieldRef::Measure { measure, .. } => format!("[{measure}]"),
        FieldRef::Aggregation {
            table,
            column,
            function,
        } => format!("{function}({table}[{column}])"),
        FieldRef::Other { .. } => "(field)".into(),
    }
}

/// `visual.set_spec {page, visual, spec}`.
pub(crate) fn visual_set_spec(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "spec"])?;
    let key = str_param(p, "page")?;
    let visual = str_param(p, "visual")?;
    let text = match p.get("spec") {
        Some(Value::String(t)) => t.clone(),
        Some(v @ Value::Object(_)) => {
            serde_json::to_string(v).map_err(|e| format!("cannot read the spec: {e}"))?
        }
        Some(_) => return Err("`spec` must be a JSON object or its text".into()),
        None => return Err("missing parameter `spec` (visual.spec_template gives a starting one)".into()),
    };
    let (value, parsed) = spec::parse_text(&text)?;
    let compact = serde_json::to_string(&value).map_err(|e| format!("cannot write the spec: {e}"))?;
    // everything that can be refused is checked before the report changes
    {
        let (_, r) = parts(s)?;
        let pi = page_index(r, key)?;
        let page = r.pages.get(pi).ok_or("internal: page index out of range")?;
        let vi = visual_index(page, visual)?;
        let v = page
            .visuals
            .get(vi)
            .ok_or("internal: visual index out of range")?;
        if v.visual_type.is_none() {
            return Err(format!("`{}` is a visual group; a group draws no spec", v.name));
        }
    }
    let install = crate::builtin_visuals::ensure(s, &SPEC)?;
    let (_, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let page = page_at_mut(r, pi)?;
    let vi = visual_index(page, visual)?;
    let v = page
        .visuals
        .get_mut(vi)
        .ok_or("internal: visual index out of range")?;
    let from = v.visual_type.clone().unwrap_or_default();
    let mut dropped: Vec<Value> = Vec::new();
    if !from.eq_ignore_ascii_case(SPEC_GUID) {
        // a column becomes the category (the first one), a measure or aggregation a value
        let mut fields: Vec<FieldBinding> = Vec::new();
        for b in &v.fields {
            let (role, room) = if is_value_field(&b.field) {
                let n = fields.iter().filter(|f| f.role == VALUES_ROLE).count();
                (VALUES_ROLE, n < spec::MAX_VALUES)
            } else {
                (CATEGORY_ROLE, !fields.iter().any(|f| f.role == CATEGORY_ROLE))
            };
            let same = fields.iter().any(|f| f.role == role && f.field == b.field);
            if room && !same {
                fields.push(FieldBinding {
                    role: role.into(),
                    field: b.field.clone(),
                    display_name: b.display_name.clone(),
                    show_as: None,
                });
            } else {
                dropped.push(json!({"role": b.role, "field": field_label(&b.field)}));
            }
        }
        v.visual_type = Some(SPEC_GUID.into());
        v.fields = fields;
    }
    let list = crate::rich::object_list(v, "objects", "spec")?;
    let props = crate::rich::plain_props(list)?;
    props.insert("json".into(), crate::rich::text_lit(&compact));
    let values = v
        .fields
        .iter()
        .filter(|f| f.role.eq_ignore_ascii_case(VALUES_ROLE))
        .count();
    let has_category = v
        .fields
        .iter()
        .any(|f| f.role.eq_ignore_ascii_case(CATEGORY_ROLE));
    let mut warnings: Vec<String> = Vec::new();
    let used = parsed.values_used();
    if used > values {
        warnings.push(format!(
            "the spec reads value.{} but the visual binds {values} value field(s): bind more with visual.bind {{role: \"{VALUES_ROLE}\"}}",
            used.saturating_sub(1)
        ));
    }
    if !has_category {
        warnings.push(format!(
            "no field in the `{CATEGORY_ROLE}` role: the visual draws one row (visual.bind {{role: \"{CATEGORY_ROLE}\"}})"
        ));
    }
    let fields: Vec<Value> = v
        .fields
        .iter()
        .map(|b| json!({"role": b.role, "field": field_label(&b.field)}))
        .collect();
    let name = v.name.clone();
    s.report_dirty = true;
    Ok(json!({
        "visual": name,
        "visual_type": SPEC_GUID,
        "converted_from": if from.eq_ignore_ascii_case(SPEC_GUID) { Value::Null } else { json!(from) },
        "installed": install,
        "marks": parsed.marks.len(),
        "values_used": used,
        "fields": fields,
        "dropped": dropped,
        "spec_bytes": compact.len(),
        "warnings": warnings,
    }))
}

/// `visual.spec_template {kind?}`.
pub(crate) fn visual_spec_template(_s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["kind"])?;
    let kind = opt_str(p, "kind")?
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .unwrap_or("bar");
    let t = spec::template(kind).ok_or_else(|| {
        format!(
            "kind `{}`: one of {}",
            kind.chars().take(40).collect::<String>(),
            spec::TEMPLATES.join(", ")
        )
    })?;
    Ok(json!({
        "kind": kind,
        "spec": t,
        "kinds": spec::TEMPLATES,
        "visual_type": SPEC_GUID,
        "roles": roles_json(),
        "fields": ["category", "value", "value.1", "value.N (N < 16)"],
        "marks": spec::MARK_TYPES,
        "max_spec_bytes": spec::MAX_SPEC_BYTES,
        "doc": "docs/formats/spec-visual.md",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The visual's TypeScript builds into its package, which BIcraft's own reader accepts.
    #[test]
    fn the_spec_visual_builds_and_reads_back() {
        let bytes = package_bytes().unwrap();
        let read = bicraft_pbix::pbiviz::read_pbiviz(&bytes).unwrap();
        assert_eq!(read.guid, SPEC_GUID);
        let js = layout_script().unwrap();
        assert!(js.contains("layout"), "{}", js.len());
    }
}
