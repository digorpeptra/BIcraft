//! BIcraft's own chart visuals (ROADMAP M12.3, docs/formats/bicraft-visuals.md): custom visuals
//! whose TypeScript BIcraft carries (`bvis/`), built into `.pbiviz` packages by its own bundler
//! with the shared visual runtime (`crate::visual_runtime`), and drawn the same by BIcraft's
//! renderer (`bicraft_render::bvis`). Each visual is a [`Def`]: its guid, data roles, format
//! options and sources. `visual.set_bicraft_visual` embeds a visual's package in the report
//! when it is missing or older, turns a visual into it (its fields moved to the visual's roles)
//! and sets its options; the snapshot carries `bicraft_visual: {kind, options}` so
//! `report.render`, PDF export and the app draw it natively.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use bicraft_report::{FieldBinding, FieldRef, Visual};
use serde_json::{Map, Value, json};

use crate::Session;
use crate::builtin_visuals::Built;
use crate::params::{allow, str_param};
use crate::report::{page_at_mut, page_index, parts, visual_index};
use crate::visual_project::{API_VERSION, Project};

#[path = "bvis_box.rs"]
mod box_plot;
#[path = "bvis_bullet.rs"]
mod bullet;
#[path = "bvis_dumbbell.rs"]
mod dumbbell;
#[path = "bvis_histogram.rs"]
mod histogram;

/// The visuals.
pub const DEFS: &[&Def] = &[&histogram::DEF, &bullet::DEF, &box_plot::DEF, &dumbbell::DEF];

/// A data role.
#[derive(Debug)]
pub struct Role {
    pub name: &'static str,
    /// `Grouping` or `Measure`.
    pub kind: &'static str,
    pub display: &'static str,
    /// Most fields.
    pub max: usize,
}

/// The type of a format option.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PropKind {
    /// A number within the bounds (whole when `int`).
    Num {
        min: f64,
        max: f64,
        int: bool,
    },
    Bool,
    /// A `#RRGGBB` colour (a `fill` property).
    Color,
    /// One of the texts (an `enumeration` property: a drop-down list).
    Enum(&'static [&'static str]),
}

/// The Style card's options every visual has (BIcraft's look, docs/formats/visual-look.md),
/// after its own.
pub const STYLE_PROPS: &[Prop] = &[
    Prop {
        object: "style",
        property: "preset",
        kind: PropKind::Enum(&bicraft_render::look::PRESETS),
        display: "Preset",
    },
    Prop {
        object: "style",
        property: "palette",
        kind: PropKind::Enum(&bicraft_render::look::PALETTES),
        display: "Colors",
    },
    Prop {
        object: "style",
        property: "rounded",
        kind: PropKind::Bool,
        display: "Rounded corners",
    },
    Prop {
        object: "style",
        property: "gradient",
        kind: PropKind::Bool,
        display: "Gradient",
    },
    Prop {
        object: "style",
        property: "shadow",
        kind: PropKind::Bool,
        display: "Shadow",
    },
    Prop {
        object: "style",
        property: "animation",
        kind: PropKind::Bool,
        display: "Animation",
    },
    Prop {
        object: "style",
        property: "card",
        kind: PropKind::Bool,
        display: "Card background",
    },
    Prop {
        object: "style",
        property: "pills",
        kind: PropKind::Bool,
        display: "Value chips",
    },
];

/// The Style card's presets ([`bicraft_render::look::PRESETS`]) and palettes
/// ([`bicraft_render::look::PALETTES`]) with their display names, as drop-down items.
pub(crate) const PRESET_ITEMS: &[(&str, &str)] =
    &[("modern", "Modern"), ("minimal", "Minimal"), ("bold", "Bold")];
pub(crate) const PALETTE_ITEMS: &[(&str, &str)] = &[("theme", "Report theme"), ("bicraft", "BIcraft colors")];

/// The display name of a value of a Style drop-down list.
fn enum_display(v: &str) -> &'static str {
    PRESET_ITEMS
        .iter()
        .chain(PALETTE_ITEMS)
        .find(|(value, _)| *value == v)
        .map_or("", |(_, name)| *name)
}

/// The visual's format options: its own, then the Style card's.
pub fn props(d: &Def) -> impl Iterator<Item = &'static Prop> {
    d.props.iter().chain(STYLE_PROPS.iter())
}

/// A format option: `object.property`.
#[derive(Debug)]
pub struct Prop {
    pub object: &'static str,
    pub property: &'static str,
    pub kind: PropKind,
    pub display: &'static str,
}

/// One of BIcraft's visuals.
#[derive(Debug)]
pub struct Def {
    /// The kind `visual.set_bicraft_visual` and the renderer name it by.
    pub kind: &'static str,
    /// Its visual type in reports.
    pub guid: &'static str,
    /// `pbiviz.json`'s `visual.name`.
    pub name: &'static str,
    pub display_name: &'static str,
    /// Raised with every change of its TypeScript or of what it bundles.
    pub version: &'static str,
    pub description: &'static str,
    pub roles: &'static [Role],
    /// The roles without a field in which nothing is drawn.
    pub required: &'static [&'static str],
    /// The role that takes the only column of a converted visual (else the first grouping role).
    pub one_column: Option<&'static str>,
    pub props: &'static [Prop],
    /// The `dataViewMappings`.
    pub mappings: fn() -> Value,
    /// The layout module (`src/<kind>.ts`) and the visual's entry (`src/visual.ts`).
    pub layout_ts: &'static str,
    pub visual_ts: &'static str,
}

/// The TypeScript every visual bundles besides its own.
const COMMON_TS: &str = include_str!("bvis/common.ts");
const HOST_TS: &str = include_str!("bvis/host.ts");
const CSS: &str = include_str!("bvis/visual.css");

/// The visual of `kind`.
pub fn def(kind: &str) -> Option<&'static Def> {
    DEFS.iter().copied().find(|d| d.kind == kind)
}

/// The visual whose guid is `guid` (ignoring case).
pub fn def_of_type(guid: &str) -> Option<&'static Def> {
    DEFS.iter().copied().find(|d| d.guid.eq_ignore_ascii_case(guid))
}

fn kinds() -> String {
    DEFS.iter().map(|d| d.kind).collect::<Vec<_>>().join(", ")
}

/// The visual's `capabilities.json`.
pub(crate) fn capabilities(d: &Def) -> Value {
    let roles: Vec<Value> = d
        .roles
        .iter()
        .map(|r| json!({"displayName": r.display, "name": r.name, "kind": r.kind}))
        .collect();
    let caps = json!({
        "dataRoles": roles,
        "dataViewMappings": (d.mappings)(),
        "objects": objects_of(props(d)),
        "privileges": []
    });
    // `supportsHighlight` where highlights can reach the visual: they come on value columns
    // (docs/formats/bicraft-visuals.md "Highlights"), so not for the Venn diagram, which has none
    let highlight = d.roles.iter().any(|r| r.kind == "Measure");
    crate::visual_runtime::with_capabilities(caps, crate::visual_runtime::Uses { highlight })
}

/// The `objects` of `capabilities.json` declaring the properties `list`.
pub(crate) fn objects_of<'a>(list: impl Iterator<Item = &'a Prop>) -> Map<String, Value> {
    let mut objects = Map::new();
    for p in list {
        let o = objects
            .entry(p.object.to_string())
            .or_insert_with(|| json!({"properties": {}}));
        let t = match p.kind {
            PropKind::Num { int: true, .. } => json!({"integer": true}),
            PropKind::Num { .. } => json!({"numeric": true}),
            PropKind::Bool => json!({"bool": true}),
            PropKind::Color => json!({"fill": {"solid": {"color": true}}}),
            PropKind::Enum(list) => json!({"enumeration": list
                .iter()
                .map(|v| json!({"value": v, "displayName": enum_display(v)}))
                .collect::<Vec<_>>()}),
        };
        if let Some(props) = o.get_mut("properties").and_then(Value::as_object_mut) {
            props.insert(p.property.into(), json!({"displayName": p.display, "type": t}));
        }
    }
    for (name, o) in objects.iter_mut() {
        if let Some(m) = o.as_object_mut() {
            m.insert("displayName".into(), json!(object_display(name)));
        }
    }
    objects
}

fn object_display(name: &str) -> &'static str {
    match name {
        "bins" => "Bins",
        "bands" => "Bands",
        "box" => "Boxes",
        "style" => "Style",
        "colors" => "Colors",
        "labels" => "Data labels",
        _ => "Options",
    }
}

/// The visual's sources (its project's `src/`).
fn sources(d: &Def) -> BTreeMap<String, String> {
    let mut s = BTreeMap::new();
    s.insert("src/visual.ts".to_string(), d.visual_ts.to_string());
    s.insert(format!("src/{}.ts", d.kind), d.layout_ts.to_string());
    s.insert("src/common.ts".to_string(), COMMON_TS.to_string());
    s.insert("src/host.ts".to_string(), HOST_TS.to_string());
    s.insert("src/spec.ts".to_string(), crate::spec_visual::SPEC_TS.to_string());
    crate::visual_runtime::add_sources(&mut s);
    s
}

/// The visual as a project in memory.
fn project(d: &Def) -> Result<Project, String> {
    let pbiviz = json!({
        "visual": {
            "name": d.name,
            "displayName": d.display_name,
            "guid": d.guid,
            "visualClassName": "Visual",
            "version": d.version,
            "description": d.description,
            "supportUrl": "",
            "gitHubUrl": "",
        },
        "apiVersion": API_VERSION,
        "author": {"name": "BIcraft", "email": ""},
    });
    let mut caps = capabilities(d);
    let string_resources = crate::visual_runtime::localize(&mut caps)?;
    Ok(Project {
        folder: std::path::PathBuf::new(),
        pbiviz,
        guid: d.guid.into(),
        display_name: d.display_name.into(),
        version: d.version.into(),
        class_name: "Visual".into(),
        api_version: API_VERSION.into(),
        capabilities: caps,
        css: format!("{CSS}{}", crate::visual_runtime::CSS),
        icon: Some(crate::builtin_visuals::icon_png(d.guid)?),
        sources: sources(d),
        entry: "src/visual.ts".into(),
        string_resources,
        warnings: Vec::new(),
    })
}

/// A visual's package, built once per process.
fn built(d: &Def) -> Result<&'static Built, String> {
    static BUILT: [OnceLock<Result<Built, String>>; 4] = [const { OnceLock::new() }; 4];
    let i = DEFS
        .iter()
        .position(|x| x.guid == d.guid)
        .ok_or("internal: not one of BIcraft's chart visuals")?;
    BUILT
        .get(i)
        .ok_or("internal: chart visual index out of range")?
        .get_or_init(|| {
            let p = project(d)?;
            let out = crate::visual_project::build(&p)?;
            let files = crate::visual_project::package_files(&p, &out.js)?;
            let body = files
                .iter()
                .find(|(n, _)| n.ends_with(".pbiviz.json"))
                .map(|(_, b)| b.clone())
                .ok_or("internal: the package has no .pbiviz.json")?;
            Ok((crate::visual_project::package(&p, &out.js)?, body))
        })
        .as_ref()
        .map_err(|e| format!("cannot build the {}: {e}", d.display_name))
}

/// The `.pbiviz` bytes of the visual of `kind`.
pub fn package_bytes(kind: &str) -> Result<Vec<u8>, String> {
    let d = def(kind).ok_or_else(|| format!("unknown BIcraft visual `{kind}` (one of {})", kinds()))?;
    built(d).map(|(p, _)| p.clone())
}

/// The data roles of the visual with guid `guid`, for BIcraft's visual registry
/// (`crate::builtin_visuals`).
pub(crate) fn registry_roles(guid: &str) -> Vec<(&'static str, bicraft_visuals::RoleKind, usize)> {
    let Some(d) = def_of_type(guid) else {
        return Vec::new();
    };
    d.roles
        .iter()
        .map(|r| {
            let kind = if r.kind.eq_ignore_ascii_case("Measure") {
                bicraft_visuals::RoleKind::Measure
            } else {
                bicraft_visuals::RoleKind::Grouping
            };
            (r.name, kind, r.max)
        })
        .collect()
}

/// The package of the visual with guid `guid`, for BIcraft's visual registry.
pub(crate) fn registry_package(guid: &str) -> Result<&'static Built, String> {
    built(def_of_type(guid).ok_or("internal: not one of BIcraft's chart visuals")?)
}

/// The layout of `kind` bundled into one script whose value exports `layout` and `dataFromJson`
/// (the golden tests run it under Node).
pub fn layout_script(kind: &str) -> Result<String, String> {
    let d = def(kind).ok_or_else(|| format!("unknown BIcraft visual `{kind}`"))?;
    let mut files = BTreeMap::new();
    files.insert(format!("src/{kind}.ts"), d.layout_ts.to_string());
    files.insert("src/common.ts".to_string(), COMMON_TS.to_string());
    files.insert("src/spec.ts".to_string(), crate::spec_visual::SPEC_TS.to_string());
    files.insert(
        "src/golden.ts".to_string(),
        format!("export {{ layout }} from \"./{kind}\";\nexport {{ dataFromJson }} from \"./common\";\n"),
    );
    crate::visual_runtime::add_sources(&mut files);
    bicraft_tsbuild::bundle(&files, "src/golden.ts").map(|o| o.js)
}

/// A format option's PBIR value as plain JSON (numbers, booleans, `#RRGGBB` for a fill).
fn plain_value(v: &Value) -> Option<Value> {
    let lit = |e: &Value| -> Option<Value> {
        let s = e.get("expr")?.get("Literal")?.get("Value")?.as_str()?;
        Some(match crate::visual_props::parse_lit(s) {
            crate::visual_props::Lit::Bool(b) => json!(b),
            crate::visual_props::Lit::Num(n, _) => json!(n),
            crate::visual_props::Lit::Text(t) => json!(t),
            _ => return None,
        })
    };
    match v.get("solid").and_then(|s| s.get("color")) {
        Some(c) => lit(c),
        None => lit(v),
    }
}

/// The visual's options: `{object: {property: value}}` from its objects' entries without a
/// selector, for the properties the visual declares.
pub(crate) fn options_of(d: &Def, v: &Visual) -> Value {
    values_of(props(d), v)
}

/// [`options_of`] for the properties `list`.
pub(crate) fn values_of<'a>(list: impl Iterator<Item = &'a Prop>, v: &Visual) -> Value {
    let mut out = Map::new();
    let objects = v.visual_extra.get("objects");
    for p in list {
        let Some(val) = objects
            .and_then(|o| o.get(p.object))
            .and_then(Value::as_array)
            .and_then(|l| l.iter().find(|e| e.get("selector").is_none_or(Value::is_null)))
            .and_then(|e| e.get("properties"))
            .and_then(|e| e.get(p.property))
            .and_then(plain_value)
        else {
            continue;
        };
        if let Some(o) = out
            .entry(p.object.to_string())
            .or_insert_with(|| json!({}))
            .as_object_mut()
        {
            o.insert(p.property.into(), val);
        }
    }
    Value::Object(out)
}

/// Adds `bicraft_visual: {kind, options}` to the snapshot object of one of these visuals.
pub(crate) fn snapshot_fields(o: &mut Map<String, Value>, v: &Visual) {
    if let Some(d) = v.visual_type.as_deref().and_then(def_of_type) {
        o.insert(
            "bicraft_visual".into(),
            json!({"kind": d.kind, "options": options_of(d, v)}),
        );
    }
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

/// The checked PBIR value of option `object.property` (null: remove it).
fn option_value(d: &Def, object: &str, property: &str, v: &Value) -> Result<Value, String> {
    let at = format!("options.{object}.{property}");
    let p = props(d)
        .find(|p| p.object == object && p.property == property)
        .ok_or_else(|| {
            let names: Vec<String> = props(d).map(|p| format!("{}.{}", p.object, p.property)).collect();
            format!(
                "{at}: the {} has no such option (one of {})",
                d.display_name,
                names.join(", ")
            )
        })?;
    if v.is_null() {
        return Ok(Value::Null);
    }
    match p.kind {
        PropKind::Num { min, max, int } => {
            let n = v
                .as_f64()
                .filter(|n| n.is_finite())
                .ok_or_else(|| format!("{at}: a number"))?;
            if n < min || n > max || (int && n.fract() != 0.0) {
                let what = if int { "a whole number" } else { "a number" };
                return Err(format!("{at}: {what} from {min} to {max}"));
            }
            Ok(crate::rich::num_lit(n))
        }
        PropKind::Bool => v
            .as_bool()
            .map(crate::rich::bool_lit)
            .ok_or_else(|| format!("{at}: true or false")),
        PropKind::Color => {
            let c = v
                .as_str()
                .filter(|c| bicraft_render::bvis::is_color(c))
                .ok_or_else(|| format!("{at}: a colour #RRGGBB"))?;
            Ok(crate::rich::color_lit(&c.to_ascii_uppercase()))
        }
        PropKind::Enum(list) => {
            let s = v
                .as_str()
                .filter(|s| list.contains(s))
                .ok_or_else(|| format!("{at}: one of {}", list.join(", ")))?;
            Ok(crate::rich::text_lit(s))
        }
    }
}

/// The checked options: `(object, property, PBIR value or null to remove)`.
fn checked_options(d: &Def, p: &Value) -> Result<Vec<(String, String, Value)>, String> {
    let mut out = Vec::new();
    let Some(o) = p.get("options") else {
        return Ok(out);
    };
    let o = o
        .as_object()
        .ok_or("`options` must be an object {object: {property: value}}")?;
    for (object, props) in o.iter().take(32) {
        let props = props
            .as_object()
            .ok_or_else(|| format!("options.{object}: an object {{property: value}}"))?;
        for (property, v) in props.iter().take(32) {
            out.push((
                object.clone(),
                property.clone(),
                option_value(d, object, property, v)?,
            ));
        }
    }
    Ok(out)
}

/// The fields of a visual turned into `d`: columns to the grouping roles, measures and
/// aggregations to the measure roles, in the roles' order while they have room.
fn convert_fields(d: &Def, fields: &[FieldBinding]) -> (Vec<FieldBinding>, Vec<Value>) {
    let columns = fields.iter().filter(|b| !is_value_field(&b.field)).count();
    let mut out: Vec<FieldBinding> = Vec::new();
    let mut dropped = Vec::new();
    for b in fields {
        let measure = is_value_field(&b.field);
        let room = |r: &Role, out: &[FieldBinding]| out.iter().filter(|f| f.role == r.name).count() < r.max;
        let role = match d.one_column.filter(|_| !measure && columns == 1) {
            Some(only) => d.roles.iter().find(|r| r.name == only),
            None => d
                .roles
                .iter()
                .filter(|r| (r.kind == "Measure") == measure)
                .find(|r| room(r, &out)),
        };
        match role {
            Some(r) if !out.iter().any(|f| f.role == r.name && f.field == b.field) => {
                out.push(FieldBinding {
                    role: r.name.into(),
                    field: b.field.clone(),
                    display_name: b.display_name.clone(),
                    show_as: None,
                })
            }
            _ => dropped.push(json!({"role": b.role, "field": field_label(&b.field)})),
        }
    }
    (out, dropped)
}

/// `visual.set_bicraft_visual {page, visual, kind, options?}`.
pub(crate) fn visual_set_bicraft_visual(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "kind", "options"])?;
    let key = str_param(p, "page")?;
    let visual = str_param(p, "visual")?;
    let kind = str_param(p, "kind")?;
    let d = def(kind.trim()).ok_or_else(|| {
        format!(
            "kind `{}`: one of {}",
            kind.chars().take(40).collect::<String>(),
            kinds()
        )
    })?;
    let opts = checked_options(d, p)?;
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
            return Err(format!("`{}` is a visual group; a group draws no chart", v.name));
        }
    }
    // embedded when missing or older (BIcraft's visual registry, ROADMAP M12.4)
    let b = crate::builtin_visuals::find(d.guid).ok_or("internal: the visual is not registered")?;
    let install = crate::builtin_visuals::ensure(s, b)?;
    let (_, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let page = page_at_mut(r, pi)?;
    let vi = visual_index(page, visual)?;
    let v = page
        .visuals
        .get_mut(vi)
        .ok_or("internal: visual index out of range")?;
    let from = v.visual_type.clone().unwrap_or_default();
    let mut dropped = Vec::new();
    if !from.eq_ignore_ascii_case(d.guid) {
        let (fields, lost) = convert_fields(d, &v.fields);
        v.visual_type = Some(d.guid.into());
        v.fields = fields;
        dropped = lost;
    }
    for (object, property, lit) in &opts {
        let list = crate::rich::object_list(v, "objects", object)?;
        let props = crate::rich::plain_props(list)?;
        if lit.is_null() {
            props.remove(property);
        } else {
            props.insert(property.clone(), lit.clone());
        }
    }
    let mut warnings = Vec::new();
    for r in d.required {
        if !v.fields.iter().any(|f| f.role == *r) {
            warnings.push(format!(
                "no field in the `{r}` role: the visual draws nothing until one is bound (visual.bind {{role: \"{r}\"}})"
            ));
        }
    }
    let fields: Vec<Value> = v
        .fields
        .iter()
        .map(|b| json!({"role": b.role, "field": field_label(&b.field)}))
        .collect();
    let options = options_of(d, v);
    let name = v.name.clone();
    s.report_dirty = true;
    Ok(json!({
        "visual": name,
        "kind": d.kind,
        "visual_type": d.guid,
        "display_name": d.display_name,
        "converted_from": if from.eq_ignore_ascii_case(d.guid) { Value::Null } else { json!(from) },
        "installed": install,
        "fields": fields,
        "dropped": dropped,
        "options": options,
        "warnings": warnings,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every visual builds into a package BIcraft's own reader accepts, under its guid, and its
    /// layout bundles alone.
    #[test]
    fn every_visual_builds_and_reads_back() {
        for d in DEFS {
            let bytes = package_bytes(d.kind).unwrap();
            let read = bicraft_pbix::pbiviz::read_pbiviz(&bytes).unwrap();
            assert_eq!(read.guid, d.guid);
            assert!(layout_script(d.kind).unwrap().contains("layout"), "{}", d.kind);
            assert_eq!(def_of_type(&d.guid.to_lowercase()).map(|x| x.kind), Some(d.kind));
        }
        let mut kinds: Vec<&str> = DEFS.iter().map(|d| d.kind).collect();
        kinds.sort_unstable();
        let mut want = bicraft_render::bvis::KINDS.to_vec();
        want.sort_unstable();
        assert_eq!(kinds, want, "the renderer draws every visual");
    }
}
