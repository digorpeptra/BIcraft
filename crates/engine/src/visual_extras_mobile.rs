//! The mobile (phone) layout of a page (docs/formats/pbir.md "Mobile layout"): `visual.mobile`
//! places, moves, resizes or removes visuals on it, `visual.mobile_format` sets their mobile
//! formatting.
//!
//! A visual's phone layout is its own document, `visuals/<v>/mobile.json`
//! (`visualContainerMobileState`), kept in `Report::resources`: `position` (the phone canvas's
//! coordinates) and, optionally, `objects` and `visualContainerObjects` with the mobile
//! formatting. A report read from a legacy layout holds the same document in
//! `Visual::extra["legacy:mobileState"]` instead (its `layouts` id 1 and `singleVisual`
//! `mobileObjects` / `mobileVcObjects`): it is edited there, and the legacy writer folds a new
//! `mobile.json` resource into that form.
//!
//! Microsoft Learn ("Optimize visual formatting for mobile layout") says a mobile format setting,
//! once changed, no longer follows the desktop layout's. BIcraft reads the mobile `objects` as
//! overrides of the visual's own: for each object, an entry with the same selector has its
//! properties replaced one by one, any other entry is added ([`merged_visual`]). This reading is
//! BIcraft's, not checked against a file Desktop saved (docs/gaps.md).

use std::collections::HashSet;

use super::*;

/// Most visuals one `visual.mobile` call places.
const MAX_ITEMS: usize = 2000;
/// The two formatting keys a mobile document may hold, as the visual's `visual` object names
/// them too.
const FORMAT_KEYS: [&str; 2] = ["objects", "visualContainerObjects"];
/// The `visual.format` keys `visual.mobile_format` also takes beside the props keys.
const PROPS_KEYS: [&str; 6] = ["title", "axes", "series", "colors", "conditional", "tooltip_page"];

fn mobile_path(page: &str, visual: &str) -> String {
    format!("definition/pages/{page}/visuals/{visual}/mobile.json")
}

fn mobile_index(r: &Report, path: &str) -> Option<usize> {
    r.resources
        .iter()
        .position(|x| x.path.replace('\\', "/").eq_ignore_ascii_case(path))
}

/// The phone layout a legacy layout's visual carries (see the module documentation).
fn legacy_mobile<'a>(r: &'a Report, page: &str, visual: &str) -> Option<&'a Value> {
    r.pages
        .iter()
        .find(|p| p.name == page)?
        .visual(visual)?
        .extra
        .get(bicraft_layout::MOBILE_STATE_KEY)
}

/// The mobile document of a visual (`mobile.json`, else the legacy layout's), when it has one
/// that parses.
pub(crate) fn mobile_doc(r: &Report, page: &str, visual: &str) -> Option<Value> {
    match mobile_index(r, &mobile_path(page, visual)) {
        Some(i) => serde_json::from_slice(&r.resources.get(i)?.bytes).ok(),
        None => legacy_mobile(r, page, visual).cloned(),
    }
}

/// The mobile position of a visual (`{x, y, width, height}`), when it has one.
pub(crate) fn mobile_position(r: &Report, page: &str, visual: &str) -> Option<Value> {
    position_of(&mobile_doc(r, page, visual)?)
}

fn position_of(doc: &Value) -> Option<Value> {
    let pos = doc.get("position")?.as_object()?;
    let mut out = Map::new();
    for k in ["x", "y", "width", "height"] {
        out.insert(k.into(), pos.get(k)?.clone());
    }
    Some(Value::Object(out))
}

/// The snapshot's `mobile` entry of visual `v` of page `page` (`{x, y, z, width, height,
/// formatted}`), and the visual as the phone layout draws it when its mobile document holds
/// formatting ([`merged_visual`]). `None` when the visual is not on the mobile layout.
pub(crate) fn mobile_snapshot(r: &Report, page: &str, v: &Visual) -> Option<(Value, Option<Visual>)> {
    let doc = mobile_doc(r, page, &v.name)?;
    let pos = doc.get("position")?.as_object()?;
    let num = |k: &str| pos.get(k).and_then(Value::as_f64).filter(|x| x.is_finite());
    let (x, y, w, h) = (num("x")?, num("y")?, num("width")?, num("height")?);
    let formatted = FORMAT_KEYS.iter().any(|k| {
        doc.get(*k)
            .and_then(Value::as_object)
            .is_some_and(|o| !o.is_empty())
    });
    let entry = json!({"x": x, "y": y, "z": num("z").unwrap_or(0.0), "width": w, "height": h,
                       "formatted": formatted});
    Some((entry, formatted.then(|| merged_visual(v, &doc))))
}

/// `v` with the mobile formatting of `doc` over its own (see the module documentation).
pub(crate) fn merged_visual(v: &Visual, doc: &Value) -> Visual {
    let mut out = v.clone();
    for k in FORMAT_KEYS {
        let Some(over) = doc.get(k).and_then(Value::as_object) else {
            continue;
        };
        let mut base = out
            .visual_extra
            .get(k)
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (object, entries) in over {
            let mut list = base
                .get(object)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for e in entries.as_array().into_iter().flatten().take(MAX_ITEMS) {
                let sel = e.get("selector");
                match list.iter_mut().find(|b| b.get("selector") == sel) {
                    Some(b) => {
                        let props = e.get("properties").and_then(Value::as_object);
                        if let (Some(dst), Some(src)) = (b.as_object_mut(), props) {
                            let slot = dst
                                .entry("properties")
                                .or_insert_with(|| Value::Object(Map::new()));
                            match slot.as_object_mut() {
                                Some(m) => m.extend(src.clone()),
                                None => *slot = Value::Object(src.clone()),
                            }
                        }
                    }
                    None => list.push(e.clone()),
                }
            }
            base.insert(object.clone(), Value::Array(list));
        }
        out.visual_extra.insert(k.into(), Value::Object(base));
    }
    out
}

/// A JSON number, integral when it can be (as Desktop writes positions).
fn num_json(v: f64) -> Value {
    if v.fract() == 0.0 && v.abs() < 9.0e15 {
        json!(v as i64)
    } else {
        json!(v)
    }
}

fn to_bytes(v: &Value) -> Vec<u8> {
    serde_json::to_vec_pretty(v).unwrap_or_default()
}

/// `doc` in Desktop's key order: `$schema`, the formatting, `position`, then the rest.
fn ordered(mut doc: Map<String, Value>) -> Value {
    let mut out = Map::new();
    if let Some(sch) = doc.shift_remove("$schema") {
        out.insert("$schema".into(), sch);
    }
    for k in FORMAT_KEYS {
        if let Some(x) = doc.shift_remove(k) {
            out.insert(k.into(), x);
        }
    }
    if let Some(x) = doc.shift_remove("position") {
        out.insert("position".into(), x);
    }
    out.extend(doc);
    Value::Object(out)
}

/// Writes the mobile document of visual `vi` of page `pi` (`None` removes it): into the legacy
/// layout's slot when the visual has one, else `mobile.json`. True when something changed.
fn store(r: &mut Report, pi: usize, vi: usize, doc: Option<Value>) -> Result<bool, String> {
    let page = r.pages.get(pi).ok_or("internal: page index out of range")?;
    let visual = page
        .visuals
        .get(vi)
        .ok_or("internal: visual index out of range")?;
    let path = mobile_path(&page.name, &visual.name);
    let at = mobile_index(r, &path);
    let legacy = at.is_none() && visual.extra.contains_key(bicraft_layout::MOBILE_STATE_KEY);
    let v = r
        .pages
        .get_mut(pi)
        .and_then(|p| p.visuals.get_mut(vi))
        .ok_or("internal: visual not found")?;
    match doc {
        None => {
            let mut changed = v.extra.shift_remove(bicraft_layout::MOBILE_STATE_KEY).is_some();
            if let Some(i) = at {
                r.resources.remove(i);
                changed = true;
            }
            Ok(changed)
        }
        Some(d) if legacy => {
            let same = v.extra.get(bicraft_layout::MOBILE_STATE_KEY) == Some(&d);
            if !same {
                v.extra.insert(bicraft_layout::MOBILE_STATE_KEY.into(), d);
            }
            Ok(!same)
        }
        Some(d) => {
            let bytes = to_bytes(&d);
            match at.and_then(|i| r.resources.get_mut(i)) {
                Some(res) => {
                    let same = serde_json::from_slice::<Value>(&res.bytes).ok().as_ref() == Some(&d);
                    if !same {
                        res.bytes = bytes;
                    }
                    Ok(!same)
                }
                None => {
                    r.resources.push(Resource { path, bytes });
                    Ok(true)
                }
            }
        }
    }
}

/// The highest `z` of the page's mobile layout (`None` when nothing is on it).
fn top_z(r: &Report, pi: usize) -> Option<f64> {
    let page = r.pages.get(pi)?;
    page.visuals
        .iter()
        .take(MAX_ITEMS)
        .filter_map(|v| mobile_doc(r, &page.name, &v.name))
        .filter_map(|d| d.pointer("/position/z").and_then(Value::as_f64))
        .filter(|z| z.is_finite())
        .reduce(f64::max)
}

/// One item of `visual.mobile` on page `pi`: (changed, its answer).
fn mobile_one(r: &mut Report, pi: usize, row: &Value, snap: bool) -> Result<(bool, Value), String> {
    allow(row, &["visual", "x", "y", "width", "height", "layout"])?;
    let visual_key = str_param(row, "visual")?;
    let remove = match row.get("layout") {
        None => false,
        Some(Value::Null) => true,
        Some(_) => {
            return Err("`layout` can only be null (removes the visual from the mobile layout)".into());
        }
    };
    let coords = ["x", "y", "width", "height"]
        .iter()
        .map(|k| opt_num(row, k))
        .collect::<Result<Vec<_>, _>>()?;
    if remove && coords.iter().any(Option::is_some) {
        return Err("`layout: null` removes the mobile layout: give no position with it".into());
    }
    if remove && snap {
        return Err("`layout: null` removes the mobile layout: `snap` does not apply".into());
    }
    let page = r.pages.get(pi).ok_or("internal: page index out of range")?;
    let vi = visual_index(page, visual_key)?;
    let visual = page
        .visuals
        .get(vi)
        .ok_or("internal: visual index out of range")?;
    if visual.visual_type.is_none() {
        return Err(format!(
            "`{}` is a visual group: place its visuals on the mobile layout one by one",
            visual.name
        ));
    }
    let (page_name, visual_name) = (page.name.clone(), visual.name.clone());
    let path = mobile_path(&page_name, &visual_name);
    let changed = if remove {
        store(r, pi, vi, None)?
    } else {
        let current: Option<Value> = match mobile_index(r, &path) {
            Some(i) => r
                .resources
                .get(i)
                .map(|x| serde_json::from_slice(&x.bytes))
                .transpose()
                .map_err(|e| format!("{path} is not valid JSON ({e}); remove it with layout: null"))?,
            None => legacy_mobile(r, &page_name, &visual_name).cloned(),
        };
        let legacy = mobile_index(r, &path).is_none() && current.is_some();
        let mut doc = current
            .as_ref()
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let mut pos = doc
            .get("position")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (k, v) in ["x", "y", "width", "height"].iter().zip(&coords) {
            match v {
                Some(n) => {
                    let n = if *k == "x" || *k == "y" {
                        check_coordinate(k, *n)?
                    } else {
                        check_size(k, *n)?
                    };
                    pos.insert((*k).to_string(), num_json(n));
                }
                None if !pos.contains_key(*k) => {
                    return Err(format!(
                        "a new mobile layout needs x, y, width and height (missing `{k}`)"
                    ));
                }
                None => {}
            }
        }
        if snap {
            // Desktop's phone grid (bicraft_render::phone, docs/gaps.md G72)
            let get = |k: &str| pos.get(k).and_then(Value::as_f64).unwrap_or(0.0);
            let (x, y, w, h) =
                bicraft_render::phone::snap_rect(get("x"), get("y"), get("width"), get("height"));
            for (k, n) in [("x", x), ("y", y), ("width", w), ("height", h)] {
                pos.insert(k.to_string(), num_json(n));
            }
        }
        if !pos.contains_key("z") {
            // a visual placed on the phone canvas goes on its own layer, on top (Microsoft Learn,
            // "Create an initial layout")
            let z = top_z(r, pi).map_or(0.0, |z| (z + 1.0).floor());
            pos.insert("z".into(), num_json(z));
        }
        let mut sorted = Map::new();
        for k in ["x", "y", "z", "height", "width", "tabOrder", "angle"] {
            if let Some(x) = pos.shift_remove(k) {
                sorted.insert(k.into(), x);
            }
        }
        sorted.extend(pos);
        doc.insert("position".into(), Value::Object(sorted));
        if !doc.contains_key("$schema") && !legacy {
            let version = r
                .schemas
                .get("mobile")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| {
                    format!("{SCHEMA_PREFIX}visualContainerMobileState/{MOBILE_VERSION}/schema.json")
                });
            doc.insert("$schema".into(), json!(version));
        }
        let new_doc = ordered(doc);
        if current.as_ref() == Some(&new_doc) {
            false
        } else {
            store(r, pi, vi, Some(new_doc))?
        }
    };
    let out = json!({
        "page": page_name,
        "visual": visual_name,
        "changed": changed,
        "path": path,
        "mobile": mobile_position(r, &page_name, &visual_name),
    });
    Ok((changed, out))
}

pub(crate) fn visual_mobile(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "page", "visual", "visuals", "x", "y", "width", "height", "layout", "snap",
        ],
    )?;
    let snap = crate::params::opt_bool(p, "snap")?.unwrap_or(false);
    let page_key = str_param(p, "page")?;
    let list = match (p.get("visual"), p.get("visuals")) {
        (Some(_), None) => None,
        (None, Some(Value::Array(a))) => {
            if ["x", "y", "width", "height", "layout"]
                .iter()
                .any(|k| p.get(*k).is_some())
            {
                return Err("with `visuals`, give each item its own x, y, width, height or layout".into());
            }
            if a.is_empty() || a.len() > MAX_ITEMS {
                return Err(format!("`visuals` must list 1 to {MAX_ITEMS} items"));
            }
            Some(a.clone())
        }
        (None, Some(_)) => {
            return Err("`visuals` must be a list of {visual, x?, y?, width?, height?, layout?}".into());
        }
        (Some(_), Some(_)) => return Err("give `visual` or `visuals`, not both".into()),
        (None, None) => {
            return Err(
                "missing parameter `visual` (or `visuals`, a list of {visual, x?, y?, width?, height?, layout?})"
                    .into(),
            );
        }
    };
    let (_, r) = parts(s)?;
    let pi = page_index(r, page_key)?;
    let (changed, out) = match list {
        None => {
            let mut row = p.as_object().cloned().unwrap_or_default();
            row.shift_remove("page");
            row.shift_remove("snap");
            mobile_one(r, pi, &Value::Object(row), snap)?
        }
        Some(rows) => {
            // every item is checked before anything changes
            let mut work = r.clone();
            let mut seen = HashSet::new();
            let mut outs = Vec::new();
            let mut changed = false;
            for row in &rows {
                let (c, o) = mobile_one(&mut work, pi, row, snap)?;
                let name = o.get("visual").and_then(Value::as_str).unwrap_or("").to_string();
                if !seen.insert(name.clone()) {
                    return Err(format!("`visuals` lists `{name}` twice"));
                }
                changed |= c;
                outs.push(o);
            }
            if changed {
                *r = work;
            }
            let page = r.pages.get(pi).map(|p| p.name.clone());
            (
                changed,
                json!({"page": page, "changed": changed, "visuals": outs}),
            )
        }
    };
    if changed {
        s.report_dirty = true;
    }
    Ok(out)
}

pub(crate) fn visual_mobile_format(s: &mut Session, p: &Value) -> Result<Value, String> {
    let mut body = p.as_object().cloned().ok_or("parameters must be a JSON object")?;
    let page_key = str_param(p, "page")?.to_string();
    let visual_key = str_param(p, "visual")?.to_string();
    body.shift_remove("page");
    body.shift_remove("visual");
    let clear = match body.shift_remove("clear") {
        None => false,
        Some(Value::Bool(b)) => b,
        Some(_) => return Err("`clear` must be true or false".into()),
    };
    if clear && !body.is_empty() {
        return Err("`clear: true` removes every mobile format: give no other setting with it".into());
    }
    if !clear && body.is_empty() {
        return Err(
            "nothing to format: pass visual.format's settings (background, border, title, objects…), \
                    or clear: true"
                .into(),
        );
    }
    let mut props = Map::new();
    for k in PROPS_KEYS {
        if let Some(x) = body.shift_remove(k) {
            props.insert(k.into(), x);
        }
    }
    if !body.is_empty() {
        props.insert("format".into(), Value::Object(body));
    }
    let (model, r) = parts(s)?;
    let pi = page_index(r, &page_key)?;
    let page = r.pages.get(pi).ok_or("internal: page index out of range")?;
    let vi = visual_index(page, &visual_key)?;
    let v = page
        .visuals
        .get(vi)
        .ok_or("internal: visual index out of range")?
        .clone();
    let v = &v;
    let (page_name, visual_name) = (page.name.clone(), v.name.clone());
    let doc = mobile_doc(r, &page_name, &visual_name).ok_or_else(|| {
        format!("`{visual_name}` is not on the mobile layout: place it with visual.mobile first")
    })?;
    let mut map = doc.as_object().cloned().unwrap_or_default();
    // the mobile formatting alone, in the visual's place, takes the edit
    let mut scratch = v.clone();
    for k in FORMAT_KEYS {
        scratch.visual_extra.shift_remove(k);
        if let Some(x) = map.get(k).filter(|x| x.is_object()) {
            scratch.visual_extra.insert(k.into(), x.clone());
        }
    }
    let mut changes: Vec<String> = Vec::new();
    if clear {
        for k in FORMAT_KEYS {
            if map.shift_remove(k).is_some() {
                changes.push(format!("mobile {k} cleared"));
            }
        }
    } else {
        changes = crate::visual_props::apply_props(&mut scratch, &Value::Object(props), model)?;
        let strip = |x: &Visual| {
            let mut y = x.clone();
            for k in FORMAT_KEYS {
                y.visual_extra.shift_remove(k);
            }
            y
        };
        if strip(&scratch) != strip(v) {
            return Err(
                "the mobile layout formats objects only: fields, sort and filters are the visual's \
                        (use visual.format, visual.bind or visual.filter)"
                    .into(),
            );
        }
        for k in FORMAT_KEYS {
            match scratch.visual_extra.get(k) {
                Some(Value::Object(o)) if !o.is_empty() => {
                    map.insert(k.into(), Value::Object(o.clone()));
                }
                _ => {
                    map.shift_remove(k);
                }
            }
        }
    }
    let new_doc = ordered(map);
    let changed = new_doc != doc && store(r, pi, vi, Some(new_doc.clone()))?;
    let merged = merged_visual(v, &new_doc);
    let objects: Vec<String> = FORMAT_KEYS
        .iter()
        .filter_map(|k| new_doc.get(*k).and_then(Value::as_object))
        .flat_map(|o| o.keys().cloned())
        .collect();
    let out = json!({
        "page": page_name,
        "visual": visual_name,
        "changed": changed,
        "changes": if changed { changes } else { Vec::new() },
        "props": crate::visual_props::extract_props(&merged),
        "mobile_objects": objects,
    });
    if changed {
        s.report_dirty = true;
    }
    Ok(out)
}
