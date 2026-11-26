//! The visual formatting catalog (M2.5, `bicraft-visuals`) in the engine: checking formatting
//! entries against it, and the read-only `visual.options` command.
//!
//! FROZEN API (other modules call it; keep the signature): [`check_object_entry`].
//!
//! Checks apply when the visual type is in the catalog (Desktop's built-in visuals). Custom
//! visuals and unknown types are not checked: the entry is accepted with a warning.

use bicraft_visuals::{self as visuals, Object, PropType, Property};
use serde_json::{Map, Value, json};

use crate::Session;
use crate::params::{allow, opt_str};

/// The warning for formatting a visual type the catalog does not know (`None` when it knows it).
pub(crate) fn unknown_type_warning(visual_type: Option<&str>) -> Option<String> {
    match visual_type {
        Some(t) if visuals::visual(t).is_some() => None,
        Some(t) => Some(format!(
            "`{t}` is not a built-in visual type (a custom visual?): its formatting is not checked"
        )),
        None => Some("the visual has no type: its formatting is not checked".into()),
    }
}

/// Checks one formatting entry, `object.property = value`, of a visual of type `visual_type`,
/// looking in the visual's `objects` first and then its `visualContainerObjects`.
///
/// - `Ok(None)`: known and valid (or `value` is `null`, a removal, which is always allowed);
/// - `Ok(Some(warning))`: not checked, because the visual type is unknown (custom visual);
/// - `Err`: unknown object or property (with a "did you mean" hint) or a bad value (what is
///   expected: type, allowed values, range).
#[allow(dead_code)] // the entry point for other formatting modules (selectors, conditional formatting)
pub(crate) fn check_object_entry(
    visual_type: Option<&str>,
    object: &str,
    property: &str,
    value: &Value,
) -> Result<Option<String>, String> {
    check_entry_at(visual_type, None, object, property, value)
}

/// [`check_object_entry`] with the target known: `Some(true)` for `visualContainerObjects`,
/// `Some(false)` for `objects`, `None` for either.
pub(crate) fn check_entry_at(
    visual_type: Option<&str>,
    container: Option<bool>,
    object: &str,
    property: &str,
    value: &Value,
) -> Result<Option<String>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let Some(t) = visual_type.filter(|t| visuals::visual(t).is_some()) else {
        return Ok(unknown_type_warning(visual_type));
    };
    let prop = find_property(t, container, object, property)?;
    visuals::check_value(prop, value)?;
    Ok(None)
}

fn where_of(container: Option<bool>) -> &'static str {
    match container {
        Some(true) => "container object",
        Some(false) => "visual object",
        None => "object",
    }
}

/// The catalog property `object.property` of visual type `t`, or an error with a hint.
fn find_property(
    t: &str,
    container: Option<bool>,
    object: &str,
    property: &str,
) -> Result<&'static Property, String> {
    let vt = visuals::visual(t).ok_or_else(|| format!("unknown visual type `{t}`"))?;
    let objects: Vec<&'static Object> = vt
        .objects
        .iter()
        .filter(|o| o.name == object && container.is_none_or(|c| o.container == c))
        .collect();
    if objects.is_empty() {
        let candidates = vt
            .objects
            .iter()
            .filter(|o| container.is_none_or(|c| o.container == c))
            .map(|o| o.name.as_str());
        let hint = match visuals::suggest(object, candidates) {
            Some(s) => format!("; did you mean `{s}`?"),
            None => {
                // Maybe the object exists on the other side (`target` given wrong).
                match container.and_then(|c| visuals::object_in(t, object, !c)) {
                    Some(o) if o.container => " (it is a container object: target \"container\")".into(),
                    Some(_) => " (it is a visual object: target \"visual\")".into(),
                    None => String::new(),
                }
            }
        };
        return Err(format!(
            "`{t}` has no {} `{object}`{hint} (visual.options {{\"visual_type\": \"{t}\"}} lists them)",
            where_of(container)
        ));
    }
    if let Some(p) = objects
        .iter()
        .find_map(|o| o.properties.iter().find(|p| p.name == property))
    {
        return Ok(p);
    }
    let names = objects
        .iter()
        .flat_map(|o| o.properties.iter().map(|p| p.name.as_str()));
    let hint = visuals::suggest(property, names)
        .map(|s| format!("; did you mean `{s}`?"))
        .unwrap_or_default();
    Err(format!(
        "`{t}` {} `{object}` has no property `{property}`{hint} (visual.options {{\"visual_type\": \"{t}\", \"object\": \"{object}\"}} lists them)",
        where_of(container)
    ))
}

// ---------------------------------------------------------------------------------------------
// visual.options
// ---------------------------------------------------------------------------------------------

fn type_json(ty: &PropType) -> Map<String, Value> {
    let mut o = Map::new();
    match ty {
        PropType::Bool => {
            o.insert("type".into(), json!("bool"));
        }
        PropType::Number { min, max, integer } => {
            o.insert("type".into(), json!(if *integer { "integer" } else { "number" }));
            if let Some(m) = min {
                o.insert("min".into(), json!(m));
            }
            if let Some(m) = max {
                o.insert("max".into(), json!(m));
            }
        }
        PropType::Text => {
            o.insert("type".into(), json!("text"));
        }
        PropType::Color => {
            o.insert("type".into(), json!("color"));
        }
        PropType::Enum { values } => {
            o.insert("type".into(), json!("enum"));
            o.insert("values".into(), json!(values));
        }
        PropType::Font => {
            o.insert("type".into(), json!("font"));
        }
        PropType::Image => {
            o.insert("type".into(), json!("image"));
        }
        PropType::Field => {
            o.insert("type".into(), json!("field"));
        }
        PropType::Other { description } => {
            o.insert("type".into(), json!("other"));
            o.insert("accepts".into(), json!(description));
        }
    }
    o
}

fn property_json(p: &Property, detailed: bool) -> Value {
    let mut o = Map::new();
    o.insert("name".into(), json!(p.name));
    o.extend(type_json(&p.ty));
    if p.conditional {
        o.insert("conditional".into(), json!(true));
    }
    if let Some(d) = &p.default {
        o.insert("default".into(), d.clone());
    }
    if detailed {
        if let Some(d) = &p.display_name {
            o.insert("display_name".into(), json!(d));
        }
        if let Some(d) = &p.description {
            o.insert("description".into(), json!(d));
        }
    }
    Value::Object(o)
}

fn object_json(o: &Object, detailed: bool) -> Value {
    let mut out = Map::new();
    out.insert("name".into(), json!(o.name));
    out.insert(
        "target".into(),
        json!(if o.container { "container" } else { "visual" }),
    );
    out.insert("selectors".into(), json!(o.selectors));
    if let Some(d) = &o.display_name {
        out.insert("display_name".into(), json!(d));
    }
    out.insert(
        "properties".into(),
        Value::Array(o.properties.iter().map(|p| property_json(p, detailed)).collect()),
    );
    Value::Object(out)
}

pub(crate) fn visual_options(_s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["visual_type", "object"])?;
    let visual_type = opt_str(p, "visual_type")?;
    let object = opt_str(p, "object")?;
    let Some(t) = visual_type else {
        if object.is_some() {
            return Err("`object` needs a `visual_type`".into());
        }
        let types: Vec<Value> = visuals::visual_types()
            .into_iter()
            .filter_map(visuals::visual)
            .map(|v| {
                json!({
                    "name": v.name,
                    "roles": v.roles,
                    "objects": v.objects.iter().filter(|o| !o.container).count(),
                })
            })
            .collect();
        let containers: Vec<&str> = visuals::container_objects().iter().map(|(o, _)| *o).collect();
        return Ok(json!({
            "count": types.len(),
            "visual_types": types,
            "container_objects": containers,
            "source": format!(
                "report theme schema {}; data roles observed with Power BI Desktop {}",
                visuals::THEME_SCHEMA_VERSION,
                visuals::ROLES_DESKTOP_VERSION
            ),
        }));
    };
    let Some(v) = visuals::visual(t) else {
        let hint = visuals::suggest(t, visuals::visual_types())
            .map(|s| format!("; did you mean `{s}`?"))
            .unwrap_or_default();
        return Err(format!(
            "`{t}` is not in the catalog of built-in visual types{hint} (custom visuals have no catalog; visual.options without visual_type lists the types)"
        ));
    };
    match object {
        None => Ok(json!({
            "visual_type": v.name,
            "roles": v.roles,
            "data_roles": v.data_roles,
            "role_conditions": v.role_conditions,
            "required_roles": v.required_roles,
            "roles_checked": v.roles_known,
            "objects": v.objects.iter().map(|o| object_json(o, false)).collect::<Vec<_>>(),
        })),
        Some(name) => {
            let found: Vec<Value> = v
                .objects
                .iter()
                .filter(|o| o.name == name)
                .map(|o| object_json(o, true))
                .collect();
            if found.is_empty() {
                let hint = visuals::suggest(name, v.objects.iter().map(|o| o.name.as_str()))
                    .map(|s| format!("; did you mean `{s}`?"))
                    .unwrap_or_default();
                return Err(format!("`{t}` has no object `{name}`{hint}"));
            }
            Ok(json!({"visual_type": v.name, "objects": found}))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(s: &str) -> Value {
        json!({"expr": {"Literal": {"Value": s}}})
    }

    #[test]
    fn entries_are_checked_against_the_catalog() {
        let t = Some("clusteredColumnChart");
        assert_eq!(check_object_entry(t, "legend", "show", &lit("true")), Ok(None));
        assert_eq!(check_object_entry(t, "title", "fontSize", &lit("14D")), Ok(None));
        assert_eq!(check_object_entry(t, "general", "altText", &lit("'a'")), Ok(None));
        assert_eq!(check_object_entry(t, "legnd", "bogus", &Value::Null), Ok(None));
        let e = check_object_entry(t, "legnd", "show", &lit("true")).unwrap_err();
        assert!(e.contains("did you mean `legend`"), "{e}");
        let e = check_object_entry(t, "legend", "positon", &lit("'Top'")).unwrap_err();
        assert!(e.contains("did you mean `position`"), "{e}");
        let e = check_object_entry(t, "legend", "position", &lit("'Middle'")).unwrap_err();
        assert!(e.contains("one of"), "{e}");
        let e = check_entry_at(t, Some(false), "title", "text", &lit("'x'")).unwrap_err();
        assert!(e.contains("container object"), "{e}");
        let e = check_entry_at(t, Some(true), "legend", "show", &lit("true")).unwrap_err();
        assert!(e.contains("visual object"), "{e}");
        // Custom visuals: accepted with a warning.
        let w = check_object_entry(Some("myVisual123"), "anything", "x", &lit("1D")).unwrap();
        assert!(w.unwrap().contains("not checked"));
        assert!(check_object_entry(None, "a", "b", &lit("1D")).unwrap().is_some());
    }

    /// Real reports, local only (skipped without `BICRAFT_TESTDATA`): every formatting value
    /// Desktop wrote in a built-in visual passes the catalog. Only object and property names are
    /// printed, never values (client material).
    #[test]
    fn real_visuals_pass_the_catalog() {
        let Some(root) = bicraft_env::var_os("BICRAFT_TESTDATA").map(std::path::PathBuf::from) else {
            eprintln!("BICRAFT_TESTDATA unset: skipping");
            return;
        };
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&root)
            .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect())
            .unwrap_or_default();
        files.push(root.join("sample1").join("report.pbix"));
        files.retain(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("pbix") || e.eq_ignore_ascii_case("pbit"))
        });
        files.sort();
        let (mut values, mut unchecked) = (0, 0);
        let mut failures = std::collections::BTreeMap::<String, usize>::new();
        for path in files {
            let mut s = Session::new();
            if s.execute("file.open", &json!({"path": path})).is_err() || s.report.is_none() {
                continue;
            }
            for p in &s.report.as_ref().unwrap().pages {
                for v in &p.visuals {
                    let t = v.visual_type.as_deref();
                    for (key, container) in [("objects", false), ("visualContainerObjects", true)] {
                        for (o, entries) in v
                            .visual_extra
                            .get(key)
                            .and_then(Value::as_object)
                            .into_iter()
                            .flatten()
                        {
                            for e in entries.as_array().into_iter().flatten() {
                                for (prop, value) in e
                                    .get("properties")
                                    .and_then(Value::as_object)
                                    .into_iter()
                                    .flatten()
                                {
                                    match check_entry_at(t, Some(container), o, prop, value) {
                                        Ok(None) => values += 1,
                                        Ok(Some(_)) => unchecked += 1,
                                        Err(_) => {
                                            *failures
                                                .entry(format!("{}.{key}.{o}.{prop}", t.unwrap_or("?")))
                                                .or_default() += 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        eprintln!("real formatting values: {values} checked, {unchecked} on unknown types");
        assert!(
            failures.is_empty(),
            "not in the catalog or refused: {failures:#?}"
        );
    }

    #[test]
    fn options_list_types_objects_and_one_object() {
        let s = Session::new();
        let all = visual_options(&s, &json!({})).unwrap();
        assert!(all["count"].as_u64().unwrap() >= 45);
        assert!(
            all["visual_types"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["name"] == "tableEx")
        );
        let chart = visual_options(&s, &json!({"visual_type": "clusteredColumnChart"})).unwrap();
        let objects = chart["objects"].as_array().unwrap();
        let legend = objects.iter().find(|o| o["name"] == "legend").unwrap();
        assert_eq!(legend["target"], "visual");
        let pos = legend["properties"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "position")
            .unwrap();
        assert_eq!(pos["type"], "enum");
        assert!(pos.get("description").is_none());
        let one = visual_options(
            &s,
            &json!({"visual_type": "clusteredColumnChart", "object": "labels"}),
        )
        .unwrap();
        let labels = &one["objects"][0];
        let size = labels["properties"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "fontSize")
            .unwrap();
        assert_eq!((size["min"].as_f64(), size["max"].as_f64()), (Some(8.0), None));
        assert_eq!(labels["selectors"], true);
        // `general` is both a visual and a container object.
        let general = visual_options(
            &s,
            &json!({"visual_type": "clusteredColumnChart", "object": "general"}),
        )
        .unwrap();
        assert_eq!(general["objects"].as_array().unwrap().len(), 2);
        for bad in [
            json!({"visual_type": "clusteredColumChart"}),
            json!({"visual_type": "clusteredColumnChart", "object": "legnd"}),
            json!({"object": "legend"}),
            json!({"visual_type": 3}),
            json!({"type": "x"}),
        ] {
            let e = visual_options(&s, &bad).unwrap_err();
            assert!(!e.is_empty());
        }
        let e = visual_options(&s, &json!({"visual_type": "clusteredColumChart"})).unwrap_err();
        assert!(e.contains("did you mean `clusteredColumnChart`"), "{e}");
    }
}
