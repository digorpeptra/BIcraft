//! Builds the catalog from the vendored report theme JSON schema (see `docs/formats/visual-options.md`).
//!
//! The theme schema describes, per visual type, every formatting object a theme may style and each
//! property's JSON type, enumerated values and (for text sizes) range. It does not say which
//! objects are `visualContainerObjects`, which take selectors, nor the data roles: those come from
//! the tables below (the public PBIR `visualConfiguration` schema for container objects, our own
//! reading for the rest) and from `roles_bb.rs` (data roles, black-box tests against Desktop).

use serde_json::{Map, Value};

use crate::{Object, PropType, Property, VisualType};

/// The vendored schema, unmodified.
pub(crate) const THEME_SCHEMA: &str = include_str!("../schemas/reportThemeSchema-2.159.json");

/// `visualStyles` entries that are not visual types.
const NOT_VISUALS: &[&str] = &["report", "page", "filter", "group"];

/// `visualContainerObjects` objects and their properties, from the public PBIR
/// `visualConfiguration` 2.7.0 schema (`VisualContainerFormattingObjects`), which rejects any
/// other property. A test checks this table against the vendored PBIR schema.
pub(crate) const CONTAINER_OBJECTS: &[(&str, &[&str])] = &[
    (
        "title",
        &[
            "show",
            "text",
            "heading",
            "titleWrap",
            "fontColor",
            "background",
            "alignment",
            "fontSize",
            "bold",
            "italic",
            "underline",
            "fontFamily",
        ],
    ),
    (
        "subTitle",
        &[
            "show",
            "text",
            "heading",
            "titleWrap",
            "fontColor",
            "alignment",
            "fontSize",
            "bold",
            "italic",
            "underline",
            "fontFamily",
        ],
    ),
    ("divider", &["ignorePadding", "show", "color", "width", "style"]),
    (
        "spacing",
        &[
            "customizeSpacing",
            "verticalSpacing",
            "spaceBelowTitle",
            "spaceBelowSubTitle",
            "spaceBelowTitleArea",
            "spaceAbovePlotArea",
            "spaceAboveDivider",
            "spaceAboveSubtitle",
        ],
    ),
    ("background", &["show", "color", "transparency"]),
    ("padding", &["top", "bottom", "left", "right"]),
    ("lockAspect", &["show"]),
    (
        "general",
        &[
            "x",
            "y",
            "width",
            "height",
            "altText",
            "allowBinnedLineSample",
            "allowOverlappingPointsSample",
            "keepLayerOrder",
        ],
    ),
    ("border", &["show", "color", "radius", "width"]),
    (
        "dropShadow",
        &[
            "show",
            "preset",
            "position",
            "color",
            "transparency",
            "shadowSpread",
            "shadowBlur",
            "angle",
            "shadowDistance",
        ],
    ),
    (
        "visualLink",
        &[
            "show",
            "type",
            "bookmark",
            "disabledTooltip",
            "drillthroughSection",
            "enabledTooltip",
            "qna",
            "suppressDefaultTooltip",
            "showDefaultTooltip",
            "navigationSection",
            "tooltip",
            "tooltipPlaceholderText",
            "webUrl",
            "dataFunction",
        ],
    ),
    (
        "visualTooltip",
        &[
            "show",
            "type",
            "section",
            "titleFontColor",
            "valueFontColor",
            "fontSize",
            "bold",
            "italic",
            "underline",
            "fontFamily",
            "background",
            "transparency",
            "actionFontColor",
            "themedTitleFontColor",
            "themedBackground",
            "themedValueFontColor",
            "showTooltipFieldsOnly",
            "showSentenceFormat",
            "sentenceTemplate",
            "showChartSpecificTooltips",
            "showValuesInBold",
        ],
    ),
    ("stylePreset", &["name"]),
    (
        "visualHeader",
        &[
            "show",
            "background",
            "border",
            "transparency",
            "foreground",
            "showVisualInformationButton",
            "showVisualWarningButton",
            "showVisualErrorButton",
            "showDrillRoleSelector",
            "showDrillUpButton",
            "showDrillToggleButton",
            "showDrillDownLevelButton",
            "showDrillDownExpandButton",
            "showPinButton",
            "showFilterRestatementButton",
            "showFocusModeButton",
            "showCopyVisualImageButton",
            "showSeeDataLayoutToggleButton",
            "showOptionsMenu",
            "showCommentButton",
            "showTooltipButton",
            "showPersonalizeVisualButton",
            "showSmartNarrativeButton",
            "showSetAlertButton",
            "showFollowVisualButton",
            "showCopilotSummaryButton",
        ],
    ),
    (
        "visualHeaderTooltip",
        &[
            "type",
            "section",
            "text",
            "titleFontColor",
            "fontSize",
            "fontFamily",
            "bold",
            "italic",
            "underline",
            "background",
            "transparency",
            "themedTitleFontColor",
            "themedBackground",
        ],
    ),
];

/// Objects whose entries may carry a selector (per series, per category, per measure, per
/// column, or one entry per analytics line), from our own reading. Objects with visual states
/// (a `$id` property in the theme schema) take selectors too.
const SELECTOR_OBJECTS: &[&str] = &[
    "dataPoint",
    "labels",
    "lineStyles",
    "seriesLabels",
    "markers",
    "columnFormatting",
    "columnWidth",
    "values",
    "columnHeaders",
    "rowHeaders",
    "subTotals",
    "columnTotal",
    "rowTotal",
    "total",
    "referenceLine",
    "xAxisReferenceLine",
    "y1AxisReferenceLine",
    "y2AxisReferenceLine",
    "trend",
    "forecast",
    "error",
    "anomalyDetection",
    "ratioLine",
    "fillPoint",
];

/// Number properties with no range in the schema that are percentages in Desktop's format pane.
fn is_percentage(name: &str) -> bool {
    name == "transparency" || name.ends_with("Transparency")
}

/// Gives a percentage its 0â€“100 range.
fn percentage_range(name: &str, ty: &mut PropType) {
    if let PropType::Number { min, max, .. } = ty
        && min.is_none()
        && max.is_none()
        && is_percentage(name)
    {
        *min = Some(0.0);
        *max = Some(100.0);
    }
}

/// Whether field and rule expressions are accepted on a property: every property with a literal
/// type. Power BI Desktop 2.158 keeps a measure on every such property when it saves a report
/// (reproducibility oracle, `docs/formats/visual-options.md`); its format pane offers an `fx`
/// button on fewer.
fn is_conditional(ty: &PropType) -> bool {
    !matches!(ty, PropType::Other { .. } | PropType::Field)
}

/// Most nested `$ref`s followed while typing one property.
const MAX_REF_DEPTH: usize = 4;

pub(crate) fn build() -> Vec<VisualType> {
    let Ok(root) = serde_json::from_str::<Value>(THEME_SCHEMA) else {
        return Vec::new();
    };
    let empty = Map::new();
    let defs = root
        .get("definitions")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let styles = root
        .get("properties")
        .and_then(|p| p.get("visualStyles"))
        .and_then(|v| v.get("properties"))
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let common = defs
        .get("commonCards")
        .and_then(|c| c.get("properties"))
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let containers: Vec<Object> = CONTAINER_OBJECTS
        .iter()
        .map(|(name, props)| container_object(name, props, common, defs))
        .collect();
    let names: Vec<String> = styles
        .keys()
        .filter(|n| !NOT_VISUALS.contains(&n.as_str()))
        .filter(|n| defs.contains_key(&format!("visual-{n}")))
        .cloned()
        .collect();
    let mut out = Vec::new();
    for name in &names {
        let mut objects = match defs.get(&format!("visual-{name}")) {
            Some(def) => visual_objects(def, defs),
            None => Vec::new(),
        };
        objects.extend(containers.iter().cloned());
        let roles = crate::roles_bb::role_names(name);
        let known = crate::roles_bb::roles_of(name);
        let roles_known = known.is_some();
        let (data_roles, role_conditions, required_roles) = known
            .map(|k| (k.data_roles, k.conditions, k.required))
            .unwrap_or_default();
        out.push(VisualType {
            name: name.clone(),
            display_name: None,
            roles,
            objects,
            roles_known,
            data_roles,
            role_conditions,
            required_roles,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// The `visual.objects` objects of a `visual-*` definition (`commonCards` excluded: it holds the
/// container objects).
fn visual_objects(def: &Value, defs: &Map<String, Value>) -> Vec<Object> {
    let parts: Vec<&Value> = match def.get("allOf").and_then(Value::as_array) {
        Some(all) => all.iter().collect(),
        None => vec![def],
    };
    let mut objects: Vec<Object> = Vec::new();
    for part in parts {
        if part.get("$ref").is_some() {
            continue;
        }
        let Some(props) = part.get("properties").and_then(Value::as_object) else {
            continue;
        };
        for (obj, schema) in props {
            if obj == "*" {
                continue;
            }
            let items = schema
                .get("items")
                .and_then(|i| i.get("properties"))
                .and_then(Value::as_object);
            let has_states = items.is_some_and(|i| i.contains_key("$id"));
            let properties: Vec<Property> = items
                .into_iter()
                .flatten()
                .filter(|(p, _)| !p.starts_with('$'))
                .map(|(p, s)| property(p, s, defs))
                .collect();
            match objects.iter_mut().find(|o| o.name == *obj) {
                Some(existing) => {
                    for p in properties {
                        if !existing.properties.iter().any(|q| q.name == p.name) {
                            existing.properties.push(p);
                        }
                    }
                    existing.selectors |= has_states;
                }
                None => objects.push(Object {
                    name: obj.clone(),
                    display_name: title(schema),
                    container: false,
                    selectors: has_states || SELECTOR_OBJECTS.contains(&obj.as_str()),
                    properties,
                }),
            }
        }
    }
    objects
}

fn container_object(
    name: &str,
    props: &[&str],
    common: &Map<String, Value>,
    defs: &Map<String, Value>,
) -> Object {
    let item_props = |obj: &str| {
        common
            .get(obj)
            .and_then(|o| o.get("items"))
            .and_then(|i| i.get("properties"))
            .and_then(Value::as_object)
    };
    let own = item_props(name);
    let any = item_props("*");
    let properties = props
        .iter()
        .map(|p| {
            let schema = own
                .and_then(|o| o.get(*p))
                .or_else(|| any.and_then(|o| o.get(*p)));
            match schema {
                Some(s) => property(p, s, defs),
                None => Property {
                    name: (*p).to_string(),
                    display_name: None,
                    ty: PropType::Other {
                        description: "not typed by the theme schema".into(),
                    },
                    default: None,
                    description: None,
                    conditional: false,
                },
            }
        })
        .collect();
    Object {
        name: name.to_string(),
        display_name: common.get(name).and_then(title),
        container: true,
        selectors: false,
        properties,
    }
}

fn title(schema: &Value) -> Option<String> {
    schema.get("title").and_then(Value::as_str).map(str::to_string)
}

fn property(name: &str, schema: &Value, defs: &Map<String, Value>) -> Property {
    let mut ty = prop_type(name, schema, defs, 0);
    percentage_range(name, &mut ty);
    // The theme schema caps text sizes at 60, but Desktop renders larger ones written in a report
    // (an 80 pt card callout, Desktop 2.158): only the minimum is enforced.
    if let PropType::Number { max, .. } = &mut ty
        && schema
            .get("$ref")
            .and_then(Value::as_str)
            .is_some_and(|r| r.ends_with("/fontSize"))
    {
        *max = None;
    }
    let conditional = is_conditional(&ty);
    Property {
        name: name.to_string(),
        display_name: title(schema),
        ty,
        default: None,
        description: schema
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string),
        conditional,
    }
}

fn prop_type(name: &str, schema: &Value, defs: &Map<String, Value>, depth: usize) -> PropType {
    if let Some(r) = schema.get("$ref").and_then(Value::as_str) {
        let target = r.rsplit('/').next().unwrap_or(r);
        match target {
            "fill" => return PropType::Color,
            "image" => return PropType::Image,
            _ => {}
        }
        return match defs.get(target) {
            Some(def) if depth < MAX_REF_DEPTH => match prop_type(name, def, defs, depth + 1) {
                PropType::Other { .. } => PropType::Other {
                    description: target.to_string(),
                },
                t => t,
            },
            _ => PropType::Other {
                description: target.to_string(),
            },
        };
    }
    let consts: Option<Vec<String>> = schema.get("oneOf").and_then(Value::as_array).and_then(|alts| {
        alts.iter()
            .map(|a| match a.get("const")? {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .collect()
    });
    let ty = schema.get("type");
    match (ty.and_then(Value::as_str), consts) {
        (Some("string" | "integer" | "number"), Some(values)) if !values.is_empty() => {
            PropType::Enum { values }
        }
        (Some("boolean"), _) => PropType::Bool,
        (Some(t @ ("number" | "integer")), _) => PropType::Number {
            min: schema.get("minimum").and_then(Value::as_f64),
            max: schema.get("maximum").and_then(Value::as_f64),
            integer: t == "integer",
        },
        (Some("string"), _) if name.to_ascii_lowercase().ends_with("fontfamily") => PropType::Font,
        (Some("string"), _) => PropType::Text,
        _ => match ty.and_then(Value::as_array) {
            Some(types) => PropType::Other {
                description: format!(
                    "a {} literal",
                    types
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" or ")
                ),
            },
            None => PropType::Other {
                description: "a structured value".into(),
            },
        },
    }
}
