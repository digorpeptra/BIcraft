//! BIcraft's chart pack (ROADMAP M12.3, docs/formats/chart-pack.md): seven custom visuals of
//! BIcraft's, the organization chart, the Venn diagram, the Marimekko, the calendar heatmap, the
//! Sankey diagram, Scatter+ and Line / Area+.
//! Each is TypeScript BIcraft writes (`chart_pack/*_visual.ts`, its layout `chart_pack/<kind>.ts`
//! and the shared `kit.ts` and `base.ts`), bundled with BIcraft's visual runtime
//! (`crate::visual_runtime`) by `bicraft_tsbuild` into a `.pbiviz` built once per process.
//! [`install`] embeds a visual's package in the report as `visual.import_custom` does; the
//! snapshot carries `chart_pack: {kind, options}` for a visual of the pack, so `report.render`,
//! PDF export and the app draw it with the renderer's twin layout (`bicraft_render::chart_pack`).

use std::collections::BTreeMap;
use std::sync::OnceLock;

use bicraft_report::Visual;
use serde_json::{Map, Value, json};

use crate::Session;
use crate::builtin_visuals::Built;
use crate::visual_project::{API_VERSION, Project};

mod calendar;
mod gauge;
mod linearea;
mod mekko;
mod org;
mod sankey;
mod scatter;
mod venn;
mod waterfall;

/// A data role of a visual.
#[derive(Debug, Clone, Copy)]
pub struct Role {
    pub name: &'static str,
    /// `Grouping` or `Measure`.
    pub kind: &'static str,
    pub display: &'static str,
    /// The visual draws nothing without a field here.
    pub required: bool,
}

/// What a format property holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropKind {
    Fill,
    Numeric,
    Bool,
    /// A text holding a JSON list of texts (the organization chart's toggled nodes).
    TextList,
    /// A text, at most 200 characters (a label, a series name, a date).
    Text,
    /// One of the listed values (`value`, display name): a drop-down in the format pane.
    Enum(&'static [(&'static str, &'static str)]),
}

/// The Style card every visual of the pack has (BIcraft's look, docs/formats/visual-look.md):
/// its options are the layouts' flat `style…` keys.
pub const STYLE: Object = Object {
    name: "style",
    display: "Style",
    props: &[
        Prop {
            name: "preset",
            display: "Preset",
            kind: PropKind::Enum(crate::bvis::PRESET_ITEMS),
            key: "stylePreset",
        },
        Prop {
            name: "palette",
            display: "Colors",
            kind: PropKind::Enum(crate::bvis::PALETTE_ITEMS),
            key: "stylePalette",
        },
        Prop {
            name: "rounded",
            display: "Rounded corners",
            kind: PropKind::Bool,
            key: "styleRounded",
        },
        Prop {
            name: "gradient",
            display: "Gradient",
            kind: PropKind::Bool,
            key: "styleGradient",
        },
        Prop {
            name: "shadow",
            display: "Shadow",
            kind: PropKind::Bool,
            key: "styleShadow",
        },
        Prop {
            name: "animation",
            display: "Animation",
            kind: PropKind::Bool,
            key: "styleAnimation",
        },
        Prop {
            name: "card",
            display: "Card background",
            kind: PropKind::Bool,
            key: "styleCard",
        },
        Prop {
            name: "pills",
            display: "Value chips",
            kind: PropKind::Bool,
            key: "stylePills",
        },
    ],
};

/// A format property and the layout option it feeds.
#[derive(Debug, Clone, Copy)]
pub struct Prop {
    pub name: &'static str,
    pub display: &'static str,
    pub kind: PropKind,
    /// Its key in the layout's `options`.
    pub key: &'static str,
}

/// A format object (a card of the format pane).
#[derive(Debug, Clone, Copy)]
pub struct Object {
    pub name: &'static str,
    pub display: &'static str,
    pub props: &'static [Prop],
}

/// One visual of the pack.
#[derive(Debug, Clone, Copy)]
pub struct Def {
    /// The layout's name (`org`, `venn`, `mekko`, `calendar`).
    pub kind: &'static str,
    /// The visual type in reports (letters and digits, starting with a letter).
    pub guid: &'static str,
    /// `pbiviz.json`'s `visual.name`.
    pub name: &'static str,
    pub display_name: &'static str,
    /// Raised with every change of its TypeScript or of the runtime it bundles.
    pub version: &'static str,
    pub description: &'static str,
    pub roles: &'static [Role],
    pub objects: &'static [Object],
    pub(crate) visual_ts: &'static str,
    /// The layout module: its file name under `src/` and its text.
    pub(crate) layout_ts: (&'static str, &'static str),
    /// The categorical mapping (`dataViewMappings[0].categorical`).
    pub(crate) categorical: fn() -> Value,
}

/// The visuals of the pack.
pub const VISUALS: &[Def] = &[
    org::DEF,
    venn::DEF,
    mekko::DEF,
    calendar::DEF,
    sankey::DEF,
    scatter::DEF,
    linearea::DEF,
    waterfall::DEF,
    gauge::DEF,
];

pub(crate) const KIT_TS: &str = include_str!("chart_pack/kit.ts");
pub(crate) const BASE_TS: &str = include_str!("chart_pack/base.ts");
pub(crate) const LAYOUTS_TS: &str = include_str!("chart_pack/layouts.ts");
pub(crate) const CSS: &str = include_str!("chart_pack/pack.css");

/// A visual of the pack by its kind, guid, name or display name (any case).
pub fn find(name: &str) -> Option<&'static Def> {
    let n = name.trim();
    VISUALS.iter().find(|d| {
        [d.kind, d.guid, d.name, d.display_name]
            .iter()
            .any(|x| x.eq_ignore_ascii_case(n))
    })
}

/// The most fields role `role` of `d` takes: one, or the renderer's limit for a role of several
/// fields (`bicraft_render::chart_pack::multi_role`: the Waterfall+'s values).
fn role_max(d: &Def, role: &str) -> usize {
    match bicraft_render::chart_pack::multi_role(d.kind) {
        Some((m, max)) if m == role => max,
        _ => 1,
    }
}

/// The visual's `capabilities.json`: its roles (one field each but a role of several fields,
/// [`role_max`]) in one categorical
/// mapping, its format objects, and what BIcraft's visual runtime needs.
pub fn capabilities(d: &Def) -> Value {
    let roles: Vec<Value> = d
        .roles
        .iter()
        .map(|r| json!({"displayName": r.display, "name": r.name, "kind": r.kind}))
        .collect();
    let mut cond = Map::new();
    for r in d.roles {
        cond.insert(r.name.into(), json!({"max": role_max(d, r.name)}));
    }
    let mut objects = Map::new();
    for o in d.objects {
        let mut props = Map::new();
        for p in o.props {
            let ty = match p.kind {
                PropKind::Fill => json!({"fill": {"solid": {"color": true}}}),
                PropKind::Numeric => json!({"numeric": true}),
                PropKind::Bool => json!({"bool": true}),
                PropKind::TextList | PropKind::Text => json!({"text": true}),
                PropKind::Enum(items) => json!({"enumeration": items
                    .iter()
                    .map(|(value, display)| json!({"value": value, "displayName": display}))
                    .collect::<Vec<_>>()}),
            };
            props.insert(p.name.into(), json!({"displayName": p.display, "type": ty}));
        }
        objects.insert(
            o.name.into(),
            json!({"displayName": o.display, "properties": props}),
        );
    }
    let caps = json!({
        "dataRoles": roles,
        "dataViewMappings": [{"conditions": [cond], "categorical": (d.categorical)()}],
        "objects": objects,
        "privileges": []
    });
    // `supportsHighlight` where highlights can reach the visual: they come on value columns
    // (docs/formats/bicraft-visuals.md "Highlights"), so not for the Venn diagram, which has none
    let highlight = d.roles.iter().any(|r| r.kind == "Measure");
    crate::visual_runtime::with_capabilities(caps, crate::visual_runtime::Uses { highlight })
}

/// The visual's sources: its class (`src/visual.ts`), its layout, the pack's kit and base, the
/// Spec visual's number and date code (`src/spec.ts`) and the runtime.
pub fn sources(d: &Def) -> BTreeMap<String, String> {
    let mut s = BTreeMap::new();
    s.insert("src/visual.ts".to_string(), d.visual_ts.to_string());
    s.insert(format!("src/{}", d.layout_ts.0), d.layout_ts.1.to_string());
    s.insert("src/kit.ts".to_string(), KIT_TS.to_string());
    s.insert("src/base.ts".to_string(), BASE_TS.to_string());
    s.insert("src/spec.ts".to_string(), crate::spec_visual::SPEC_TS.to_string());
    crate::visual_runtime::add_sources(&mut s);
    s
}

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

fn built(d: &Def) -> Result<&'static Built, String> {
    static BUILT: [OnceLock<Result<Built, String>>; VISUALS.len()] =
        [const { OnceLock::new() }; VISUALS.len()];
    let i = VISUALS
        .iter()
        .position(|x| x.guid == d.guid)
        .ok_or("internal: not a visual of the chart pack")?;
    BUILT
        .get(i)
        .ok_or("internal: chart pack index out of range")?
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
        .map_err(|e| format!("cannot build {}: {e}", d.display_name))
}

/// A visual's `.pbiviz` bytes.
pub fn package_bytes(d: &Def) -> Result<Vec<u8>, String> {
    built(d).map(|(p, _)| p.clone())
}

/// Every layout bundled into one script whose value is the module's exports (`layout(kind,
/// input, w, h)`); the golden tests run it under Node.
pub fn layout_script() -> Result<String, String> {
    let mut files = BTreeMap::new();
    for d in VISUALS {
        files.insert(format!("src/{}", d.layout_ts.0), d.layout_ts.1.to_string());
    }
    files.insert("src/kit.ts".to_string(), KIT_TS.to_string());
    files.insert("src/layouts.ts".to_string(), LAYOUTS_TS.to_string());
    files.insert("src/spec.ts".to_string(), crate::spec_visual::SPEC_TS.to_string());
    crate::visual_runtime::add_sources(&mut files);
    bicraft_tsbuild::bundle(&files, "src/layouts.ts").map(|o| o.js)
}

/// The data roles of the pack visual with guid `guid`, for BIcraft's visual registry
/// (`crate::builtin_visuals`): each takes at most one field.
pub(crate) fn registry_roles(guid: &str) -> Vec<(&'static str, bicraft_visuals::RoleKind, usize)> {
    let Some(d) = VISUALS.iter().find(|d| d.guid == guid) else {
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
            (r.name, kind, role_max(d, r.name))
        })
        .collect()
}

/// The package of the pack visual with guid `guid`, for BIcraft's visual registry.
pub(crate) fn registry_package(guid: &str) -> Result<&'static Built, String> {
    let d = VISUALS
        .iter()
        .find(|d| d.guid == guid)
        .ok_or("internal: not a visual of the chart pack")?;
    built(d)
}

/// Embeds the visual `name` (kind, guid or name) in the report when it is missing or an older
/// build (a newer one is kept, as for every BIcraft visual: `crate::builtin_visuals::ensure`);
/// returns whether it was embedded now.
pub fn install(s: &mut Session, name: &str) -> Result<bool, String> {
    let d = find(name).ok_or_else(|| {
        format!(
            "`{}` is not a visual of BIcraft's chart pack ({})",
            name.chars().take(60).collect::<String>(),
            VISUALS.iter().map(|d| d.kind).collect::<Vec<_>>().join(", ")
        )
    })?;
    crate::report::parts(s)?;
    let b =
        crate::builtin_visuals::find(d.guid).ok_or("internal: the chart pack visual is not registered")?;
    crate::builtin_visuals::ensure(s, b)
}

/// The value of a format property: the entry without a selector of `objects.<object>`.
fn prop_value<'a>(v: &'a Visual, object: &str, prop: &str) -> Option<&'a Value> {
    v.visual_extra
        .get("objects")?
        .get(object)?
        .as_array()?
        .iter()
        .find(|e| e.get("selector").is_none_or(Value::is_null))?
        .get("properties")?
        .get(prop)
}

fn literal(v: &Value) -> Option<&str> {
    v.get("expr")?.get("Literal")?.get("Value")?.as_str()
}

fn unquote(s: &str) -> Option<String> {
    Some(s.strip_prefix('\'')?.strip_suffix('\'')?.replace("''", "'"))
}

/// A property as the layout's option (`None`: not set or not readable, the layout's default).
fn option(v: &Value, kind: PropKind) -> Option<Value> {
    match kind {
        PropKind::Fill => {
            let lit = literal(v.get("solid")?.get("color")?)?;
            Some(json!(unquote(lit)?))
        }
        PropKind::Numeric => {
            let lit = literal(v)?;
            let n: f64 = lit
                .trim_end_matches(['D', 'L', 'M', 'd', 'l', 'm'])
                .parse()
                .ok()?;
            n.is_finite().then(|| json!(n))
        }
        PropKind::Bool => match literal(v)? {
            "true" => Some(json!(true)),
            "false" => Some(json!(false)),
            _ => None,
        },
        PropKind::TextList => {
            let text = unquote(literal(v)?)?;
            if text.len() > 65_536 {
                return None;
            }
            let list: Vec<Value> = serde_json::from_str::<Value>(&text)
                .ok()?
                .as_array()?
                .iter()
                .filter(|x| x.is_string())
                .take(500)
                .cloned()
                .collect();
            Some(Value::Array(list))
        }
        PropKind::Text => {
            let text = unquote(literal(v)?)?;
            Some(json!(text.chars().take(200).collect::<String>()))
        }
        PropKind::Enum(items) => {
            let text = unquote(literal(v)?)?;
            items.iter().any(|(value, _)| *value == text).then(|| json!(text))
        }
    }
}

/// The layout's options of a visual of the pack, from its format properties.
pub fn options(d: &Def, v: &Visual) -> Map<String, Value> {
    let mut out = Map::new();
    for o in d.objects {
        for p in o.props {
            if let Some(x) = prop_value(v, o.name, p.name).and_then(|x| option(x, p.kind)) {
                out.insert(p.key.into(), x);
            }
        }
    }
    out
}

/// Adds `chart_pack: {kind, visual, options}` to the snapshot object of a visual of the pack.
pub(crate) fn snapshot_fields(o: &mut Map<String, Value>, v: &Visual) {
    let Some(d) = v
        .visual_type
        .as_deref()
        .and_then(|t| VISUALS.iter().find(|d| d.guid.eq_ignore_ascii_case(t)))
    else {
        return;
    };
    let mut cp = json!({"kind": d.kind, "visual": d.display_name, "options": options(d, v)});
    if d.kind == "linearea" {
        // today (UTC midnight, ms since 1970) for a forecast from `today`
        cp["today"] = json!((crate::preview::filters::today() - 25_569.0) * 86_400_000.0);
    }
    o.insert("chart_pack".into(), cp);
}

/// Longest node id a toggle takes (500 of them stay within the 64 KiB `tree.toggled` read).
const MAX_NODE_ID: usize = 100;

/// The organization chart's toggled node ids of `v` after a click on node `id`'s collapse
/// button, as its TypeScript computes them (`org_visual.ts` `flip`): `current` (the reader's
/// list) or the stored `tree.toggled`, without `id` when it was there, else with `id` added
/// last (the newest [`bicraft_render::chart_pack::org::MAX_TOGGLED`] kept).
pub(crate) fn toggled_after(v: &Visual, current: Option<&[String]>, id: &str) -> Result<Vec<String>, String> {
    if !v
        .visual_type
        .as_deref()
        .is_some_and(|t| t.eq_ignore_ascii_case(org::DEF.guid))
    {
        return Err(format!("visual `{}` is not a BIcraft Org Chart", v.name));
    }
    if id.is_empty() || id.chars().count() > MAX_NODE_ID {
        return Err(format!("`id` must be a node id of 1 to {MAX_NODE_ID} characters"));
    }
    let mut list: Vec<String> = match current {
        Some(c) => c.to_vec(),
        None => stored_toggled(v),
    };
    if let Some(i) = list.iter().position(|x| x == id) {
        list.remove(i);
    } else {
        list.push(id.to_string());
        let max = bicraft_render::chart_pack::org::MAX_TOGGLED;
        if list.len() > max {
            list.drain(..list.len() - max);
        }
    }
    Ok(list)
}

/// The organization chart's stored `tree.toggled` ids.
pub(crate) fn stored_toggled(v: &Visual) -> Vec<String> {
    prop_value(v, "tree", "toggled")
        .and_then(|x| option(x, PropKind::TextList))
        .and_then(|x| x.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|x| x.as_str().map(str::to_string))
        .collect()
}

/// The `tree.toggled` text literal of `list` (a JSON list, as the TypeScript persists it).
fn toggled_text(list: &[String]) -> String {
    Value::Array(list.iter().map(|x| json!(x)).collect()).to_string()
}

/// Writes `list` as `v`'s `tree.toggled` (the entry without a selector), for the reader's view
/// state; the editing view saves it with `visual.toggle_node`.
pub(crate) fn put_toggled(v: &mut Visual, list: &[String]) {
    let lit = json!({"expr": {"Literal": {"Value": crate::visual_props::quote(&toggled_text(list))}}});
    let objects = v.visual_extra.entry("objects").or_insert_with(|| json!({}));
    let Some(objects) = objects.as_object_mut() else {
        return;
    };
    let tree = objects.entry("tree").or_insert_with(|| json!([]));
    let Some(entries) = tree.as_array_mut() else {
        return;
    };
    let at = entries
        .iter()
        .position(|e| e.get("selector").is_none_or(Value::is_null));
    let entry = match at {
        Some(i) => entries.get_mut(i),
        None => {
            entries.push(json!({"properties": {}}));
            entries.last_mut()
        }
    };
    if let Some(props) = entry
        .and_then(|e| e.as_object_mut())
        .map(|e| e.entry("properties").or_insert_with(|| json!({})))
        .and_then(Value::as_object_mut)
    {
        props.insert("toggled".into(), lit);
    }
}

/// `visual.toggle_node {page, visual, id}`: a click on an organization chart node's collapse
/// button in the editing view, saved as the TypeScript persists it in Power BI Desktop
/// (`tree.toggled`, through `visual.format`), undoable.
pub(crate) fn visual_toggle_node(s: &mut Session, p: &Value) -> Result<Value, String> {
    crate::params::allow(p, &["page", "visual", "id"])?;
    let id = p
        .get("id")
        .and_then(Value::as_str)
        .ok_or("`id` (the node's id, a text) is required")?;
    let report = s.report.as_ref().ok_or("no report is open")?;
    let (page, v) = crate::interact::find_visual(report, p)?;
    let list = toggled_after(v, None, id)?;
    let (page, visual) = (page.name.clone(), v.name.clone());
    crate::visual_props::visual_format(
        s,
        &json!({"page": page, "visual": visual, "objects": [
            {"object": "tree", "property": "toggled", "value": toggled_text(&list)}
        ]}),
    )?;
    Ok(json!({"page": page, "visual": visual, "id": id, "toggled": list}))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each visual's TypeScript builds into its package, which BIcraft's own reader accepts, and
    /// the layouts bundle on their own.
    #[test]
    fn the_pack_builds_and_reads_back() {
        for d in VISUALS {
            let bytes = package_bytes(d).unwrap();
            let read = bicraft_pbix::pbiviz::read_pbiviz(&bytes).unwrap();
            assert_eq!(read.guid, d.guid);
            assert_eq!(
                bicraft_render::chart_pack::roles(d.kind).len(),
                d.roles.len(),
                "{}",
                d.kind
            );
            for (r, want) in d.roles.iter().zip(bicraft_render::chart_pack::roles(d.kind)) {
                assert_eq!(r.name, *want, "{}", d.kind);
            }
        }
        assert!(layout_script().unwrap().contains("layoutVenn"));
        assert_eq!(find("VENN").map(|d| d.kind), Some("venn"));
        assert!(find("nope").is_none());
    }
}
