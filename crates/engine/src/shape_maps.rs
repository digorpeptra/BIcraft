//! Shape maps' custom maps (`docs/formats/maps.md`): `shape_map.set_map` stores a TopoJSON or
//! GeoJSON file the author adds (Format › Map settings › Add a map type, Microsoft Learn "Create
//! Shape Map visualizations in Power BI") as a `ShapeMap` item of the report's
//! `RegisteredResources` package (an item type of the public PBIR `report.json` schema) and points
//! the visual's `shape` object at it; the snapshot carries the map's decoded shapes
//! (`shape_map`), which the renderer draws offline.
//!
//! The map file is untrusted: at most [`MAX_MAP_BYTES`], parsed by `serde_json` (its recursion
//! limit), at most [`MAX_FEATURES`] shapes and [`MAX_POINTS`] decoded points, every arc index
//! checked. The snapshot keeps at most [`SNAPSHOT_POINTS`] points (rings thinned evenly).

use std::sync::{Arc, Mutex};

use bicraft_report::{Report, Visual};
use serde_json::{Map, Value, json};

use crate::Session;
use crate::params::{allow, opt_bool, opt_str, str_param};
use crate::report::{page_at_mut, page_index, parts, visual_index};

/// Largest map file accepted.
pub const MAX_MAP_BYTES: usize = 16 << 20;
/// Longest `data` parameter (base64 of [`MAX_MAP_BYTES`] plus a `data:` header).
const MAX_DATA_CHARS: usize = MAX_MAP_BYTES / 3 * 4 + 1024;
/// Most shapes (features with an area) read from one map.
pub const MAX_FEATURES: usize = 10_000;
/// Most points decoded from one map.
pub const MAX_POINTS: usize = 2_000_000;
/// Most points one snapshot carries for a map.
pub const SNAPSHOT_POINTS: usize = 40_000;
/// Most keys kept per shape, and their longest text.
const MAX_KEYS: usize = 16;
const MAX_KEY_CHARS: usize = 200;
/// Deepest nesting of geometry collections followed.
const MAX_DEPTH: usize = 8;
/// Longest file name kept.
const MAX_NAME: usize = 200;
/// The resource item type of a custom shape map (public PBIR `report.json` schema).
pub(crate) const SHAPE_MAP_ITEM: &str = "ShapeMap";

/// One shape of a map: its keys (the feature's `id` and its property values, as text) and its
/// rings (outer rings and holes, in the map's coordinates).
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub keys: Vec<String>,
    pub rings: Vec<Vec<(f64, f64)>>,
}

/// A decoded map.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GeoMap {
    pub shapes: Vec<Shape>,
    pub points: usize,
}

// ---------------------------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------------------------

fn num(v: &Value) -> Option<f64> {
    v.as_f64().filter(|x| x.is_finite())
}

fn pair(v: &Value) -> Option<(f64, f64)> {
    let a = v.as_array()?;
    Some((num(a.first()?)?, num(a.get(1)?)?))
}

fn key_text(v: &Value) -> Option<String> {
    let s = match v {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    (!s.is_empty()).then(|| s.chars().take(MAX_KEY_CHARS).collect())
}

/// The keys of a feature: its `id`, then its property values (text and numbers).
fn keys_of(id: Option<&Value>, props: Option<&Value>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |k: Option<String>| {
        if let Some(k) = k
            && out.len() < MAX_KEYS
            && !out.iter().any(|x| x.eq_ignore_ascii_case(&k))
        {
            out.push(k);
        }
    };
    push(id.and_then(key_text));
    if let Some(Value::Object(o)) = props {
        for v in o.values() {
            push(key_text(v));
        }
    }
    out
}

struct Budget {
    points: usize,
}

impl Budget {
    fn take(&mut self, n: usize) -> Result<(), String> {
        self.points = self.points.saturating_add(n);
        if self.points > MAX_POINTS {
            return Err(format!("the map has more than {MAX_POINTS} points"));
        }
        Ok(())
    }
}

fn push_shape(out: &mut Vec<Shape>, keys: Vec<String>, rings: Vec<Vec<(f64, f64)>>) -> Result<(), String> {
    let rings: Vec<Vec<(f64, f64)>> = rings.into_iter().filter(|r| r.len() >= 3).collect();
    if rings.is_empty() {
        return Ok(());
    }
    if out.len() >= MAX_FEATURES {
        return Err(format!("the map has more than {MAX_FEATURES} shapes"));
    }
    out.push(Shape { keys, rings });
    Ok(())
}

// --- TopoJSON ---

/// The decoded arcs of a topology (delta-decoded and transformed when quantized).
fn topo_arcs(root: &Map<String, Value>, budget: &mut Budget) -> Result<Vec<Vec<(f64, f64)>>, String> {
    let tf = root.get("transform");
    let (scale, translate) = match tf {
        Some(t) if t.is_object() => (
            t.get("scale")
                .and_then(pair)
                .ok_or("the topology's transform has no scale")?,
            t.get("translate")
                .and_then(pair)
                .ok_or("the topology's transform has no translate")?,
        ),
        _ => ((1.0, 1.0), (0.0, 0.0)),
    };
    let quantized = tf.is_some_and(Value::is_object);
    let arcs = root
        .get("arcs")
        .and_then(Value::as_array)
        .ok_or("the topology has no `arcs` list")?;
    let mut out = Vec::with_capacity(arcs.len().min(MAX_POINTS));
    for a in arcs {
        let pts = a.as_array().ok_or("an arc is not a list of positions")?;
        budget.take(pts.len())?;
        let mut arc = Vec::with_capacity(pts.len());
        let (mut x, mut y) = (0.0f64, 0.0f64);
        for p in pts {
            let (px, py) = pair(p).ok_or("an arc position is not two numbers")?;
            if quantized {
                x += px;
                y += py;
                arc.push((x * scale.0 + translate.0, y * scale.1 + translate.1));
            } else {
                arc.push((px, py));
            }
        }
        out.push(arc);
    }
    Ok(out)
}

/// One ring from its arc indices (a negative index `~i` is arc `i` reversed).
fn topo_ring(idx: &Value, arcs: &[Vec<(f64, f64)>], budget: &mut Budget) -> Result<Vec<(f64, f64)>, String> {
    let list = idx.as_array().ok_or("a ring is not a list of arc indices")?;
    let mut ring: Vec<(f64, f64)> = Vec::new();
    for i in list {
        let i = i.as_i64().ok_or("an arc index is not a whole number")?;
        let (k, rev) = if i >= 0 { (i, false) } else { (-1 - i, true) };
        let arc = usize::try_from(k)
            .ok()
            .and_then(|k| arcs.get(k))
            .ok_or_else(|| format!("arc index {i} is out of range ({} arcs)", arcs.len()))?;
        budget.take(arc.len())?;
        let pts: Box<dyn Iterator<Item = &(f64, f64)>> = if rev {
            Box::new(arc.iter().rev())
        } else {
            Box::new(arc.iter())
        };
        let skip = usize::from(!ring.is_empty());
        ring.extend(pts.skip(skip).copied());
    }
    Ok(ring)
}

fn topo_geometry(
    g: &Value,
    arcs: &[Vec<(f64, f64)>],
    out: &mut Vec<Shape>,
    budget: &mut Budget,
    depth: usize,
) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err("geometry collections are nested too deeply".into());
    }
    let keys = || keys_of(g.get("id"), g.get("properties"));
    match g.get("type").and_then(Value::as_str).unwrap_or("") {
        "GeometryCollection" => {
            for x in g
                .get("geometries")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                topo_geometry(x, arcs, out, budget, depth.saturating_add(1))?;
            }
        }
        "Polygon" => {
            let mut rings = Vec::new();
            for r in g.get("arcs").and_then(Value::as_array).into_iter().flatten() {
                rings.push(topo_ring(r, arcs, budget)?);
            }
            push_shape(out, keys(), rings)?;
        }
        "MultiPolygon" => {
            let mut rings = Vec::new();
            for poly in g.get("arcs").and_then(Value::as_array).into_iter().flatten() {
                for r in poly.as_array().into_iter().flatten() {
                    rings.push(topo_ring(r, arcs, budget)?);
                }
            }
            push_shape(out, keys(), rings)?;
        }
        // points and lines have no area: a shape map colours areas
        _ => {}
    }
    Ok(())
}

// --- GeoJSON ---

fn geo_rings(coords: &Value, budget: &mut Budget) -> Result<Vec<Vec<(f64, f64)>>, String> {
    let mut rings = Vec::new();
    for r in coords.as_array().into_iter().flatten() {
        let pts = r.as_array().ok_or("a ring is not a list of positions")?;
        budget.take(pts.len())?;
        let ring: Option<Vec<(f64, f64)>> = pts.iter().map(pair).collect();
        rings.push(ring.ok_or("a position is not two numbers")?);
    }
    Ok(rings)
}

fn geo_geometry(
    g: &Value,
    keys: &[String],
    out: &mut Vec<Shape>,
    budget: &mut Budget,
    depth: usize,
) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err("geometry collections are nested too deeply".into());
    }
    let coords = g.get("coordinates").unwrap_or(&Value::Null);
    match g.get("type").and_then(Value::as_str).unwrap_or("") {
        "Polygon" => push_shape(out, keys.to_vec(), geo_rings(coords, budget)?)?,
        "MultiPolygon" => {
            let mut rings = Vec::new();
            for poly in coords.as_array().into_iter().flatten() {
                rings.extend(geo_rings(poly, budget)?);
            }
            push_shape(out, keys.to_vec(), rings)?;
        }
        "GeometryCollection" => {
            // one shape per feature: the parts join
            let mut parts = Vec::new();
            for x in g
                .get("geometries")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                geo_geometry(x, keys, &mut parts, budget, depth.saturating_add(1))?;
            }
            let rings: Vec<Vec<(f64, f64)>> = parts.into_iter().flat_map(|s| s.rings).collect();
            push_shape(out, keys.to_vec(), rings)?;
        }
        _ => {}
    }
    Ok(())
}

fn geo_feature(f: &Value, out: &mut Vec<Shape>, budget: &mut Budget) -> Result<(), String> {
    let keys = keys_of(f.get("id"), f.get("properties"));
    match f.get("geometry") {
        Some(g) if g.is_object() => geo_geometry(g, &keys, out, budget, 0),
        _ => Ok(()),
    }
}

/// Decodes a TopoJSON topology or a GeoJSON object (feature collection, feature or geometry).
pub fn parse_map(bytes: &[u8]) -> Result<GeoMap, String> {
    if bytes.len() > MAX_MAP_BYTES {
        return Err(format!(
            "the map is {} bytes; the limit is {MAX_MAP_BYTES} ({} MB)",
            bytes.len(),
            MAX_MAP_BYTES >> 20
        ));
    }
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let root: Value = serde_json::from_slice(bytes).map_err(|e| format!("the map is not valid JSON: {e}"))?;
    let o = root
        .as_object()
        .ok_or("the map is not a JSON object (TopoJSON or GeoJSON)")?;
    let mut budget = Budget { points: 0 };
    let mut shapes = Vec::new();
    match o.get("type").and_then(Value::as_str).unwrap_or("") {
        "Topology" => {
            let arcs = topo_arcs(o, &mut budget)?;
            let objects = o
                .get("objects")
                .and_then(Value::as_object)
                .ok_or("the topology has no `objects`")?;
            for g in objects.values() {
                topo_geometry(g, &arcs, &mut shapes, &mut budget, 0)?;
            }
        }
        "FeatureCollection" => {
            for f in o.get("features").and_then(Value::as_array).into_iter().flatten() {
                geo_feature(f, &mut shapes, &mut budget)?;
            }
        }
        "Feature" => geo_feature(&root, &mut shapes, &mut budget)?,
        "Polygon" | "MultiPolygon" | "GeometryCollection" => {
            geo_geometry(&root, &[], &mut shapes, &mut budget, 0)?;
        }
        other => {
            return Err(format!(
                "the map's type is `{}`: a TopoJSON `Topology` or a GeoJSON `FeatureCollection`, \
                 `Feature` or polygon geometry is expected",
                other.chars().take(40).collect::<String>()
            ));
        }
    }
    if shapes.is_empty() {
        return Err("the map has no shape with an area (polygons or multipolygons)".into());
    }
    let points = shapes
        .iter()
        .flat_map(|s| s.rings.iter())
        .map(Vec::len)
        .fold(0usize, usize::saturating_add);
    Ok(GeoMap { shapes, points })
}

// ---------------------------------------------------------------------------------------------
// The visual's map reference
// ---------------------------------------------------------------------------------------------

fn props<'a>(v: &'a Visual, object: &str) -> Option<&'a Map<String, Value>> {
    v.visual_extra
        .get("objects")?
        .get(object)?
        .as_array()?
        .iter()
        .find(|e| e.get("selector").is_none_or(Value::is_null))?
        .get("properties")?
        .as_object()
}

fn lit_str(v: Option<&Value>) -> Option<String> {
    let s = v?.get("expr")?.get("Literal")?.get("Value")?.as_str()?;
    Some(
        s.strip_prefix('\'')
            .and_then(|x| x.strip_suffix('\''))
            .map_or_else(|| s.to_string(), |x| x.replace("''", "'")),
    )
}

fn lit_num(v: Option<&Value>) -> Option<f64> {
    let s = lit_str(v)?;
    let s = s.trim_end_matches(['D', 'L', 'M']);
    s.parse::<f64>().ok().filter(|x| x.is_finite())
}

fn lit_color(v: Option<&Value>) -> Option<String> {
    lit_str(v?.get("solid")?.get("color"))
}

/// The custom map item a shape map names (`shape.map.geoJson.content`), with its file name.
fn custom_item(v: &Visual) -> Option<(String, String)> {
    let gj = props(v, "shape")?.get("map")?.get("geoJson")?;
    let item = gj
        .get("content")?
        .get("expr")?
        .get("ResourcePackageItem")?
        .get("ItemName")?
        .as_str()?
        .to_string();
    let name = lit_str(gj.get("name")).unwrap_or_else(|| item.clone());
    Some((item, name))
}

fn norm(p: &str) -> String {
    p.replace('\\', "/").trim_start_matches('/').to_ascii_lowercase()
}

/// The bytes of a registered resource item.
fn item_bytes<'a>(r: &'a Report, item: &str) -> Option<&'a [u8]> {
    let path = r
        .extra
        .get("resourcePackages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find_map(|p| {
            let p = p.get("resourcePackage").unwrap_or(p);
            let pkg = p.get("name").and_then(Value::as_str)?;
            let it = p.get("items")?.as_array()?.iter().find(|i| {
                i.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|n| n.eq_ignore_ascii_case(item))
            })?;
            let file = it.get("path").and_then(Value::as_str).unwrap_or(item);
            Some(format!("StaticResources/{pkg}/{file}"))
        })
        .unwrap_or_else(|| format!("StaticResources/RegisteredResources/{item}"));
    let want = norm(&path);
    r.resources
        .iter()
        .find(|x| norm(&x.path) == want)
        .map(|x| x.bytes.as_slice())
}

/// Recently decoded maps, by (length, hash) of their bytes.
type Cached = (usize, u64, Arc<Result<GeoMap, String>>);
static CACHE: Mutex<Vec<Cached>> = Mutex::new(Vec::new());
const CACHE_SIZE: usize = 4;

fn fnv64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn decoded(bytes: &[u8]) -> Arc<Result<GeoMap, String>> {
    let key = (bytes.len(), fnv64(bytes));
    {
        let c = CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((_, _, m)) = c.iter().find(|(l, h, _)| (*l, *h) == key) {
            return Arc::clone(m);
        }
    }
    let m = Arc::new(parse_map(bytes));
    let mut c = CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if c.len() >= CACHE_SIZE {
        c.remove(0);
    }
    c.push((key.0, key.1, Arc::clone(&m)));
    m
}

fn round(x: f64) -> Value {
    json!((x * 1e5).round() / 1e5)
}

/// The shapes as the snapshot carries them, thinned evenly to [`SNAPSHOT_POINTS`].
fn shapes_json(m: &GeoMap) -> (Vec<Value>, bool) {
    let step = m.points.div_ceil(SNAPSHOT_POINTS).max(1);
    let mut out = Vec::with_capacity(m.shapes.len());
    for s in &m.shapes {
        let rings: Vec<Value> = s
            .rings
            .iter()
            .map(|r| {
                let mut flat = Vec::new();
                let n = r.len();
                for (i, (x, y)) in r.iter().enumerate() {
                    // keep every `step`th point, and enough of a small ring to stay a ring
                    if step == 1 || i % step == 0 || n <= 8 || i + 1 == n {
                        flat.push(round(*x));
                        flat.push(round(*y));
                    }
                }
                Value::Array(flat)
            })
            .collect();
        out.push(json!({"keys": s.keys, "rings": rings}));
    }
    (out, step > 1)
}

/// The `shape_map` snapshot field of a shape map visual (`None` for other visuals):
/// `{source: custom|url|builtin, name?, projection, auto_zoom?, default_color?, border_color?,
/// border_width?, show_blank?, shapes?: [{keys, rings: [[x, y, x, y, …]]}], simplified?,
/// error?}`.
pub(crate) fn snapshot(r: &Report, v: &Visual) -> Option<Value> {
    if !v
        .visual_type
        .as_deref()
        .is_some_and(|t| t.eq_ignore_ascii_case("shapeMap"))
    {
        return None;
    }
    let mut o = Map::new();
    let shape = props(v, "shape");
    let projection =
        lit_str(shape.and_then(|p| p.get("projectionEnum"))).unwrap_or_else(|| "mercator".into());
    o.insert("projection".into(), json!(projection));
    // Zoom › Auto zoom (theme schema: "zoom in on shapes with available data")
    if let Some(z) = lit_str(props(v, "zoom").and_then(|p| p.get("autoZoom"))) {
        o.insert("auto_zoom".into(), json!(z == "true"));
    }
    if let Some(dc) = props(v, "defaultColors") {
        if let Some(c) = lit_color(dc.get("defaultColor")) {
            o.insert("default_color".into(), json!(c));
        }
        if let Some(c) = lit_color(dc.get("borderColor")) {
            o.insert("border_color".into(), json!(c));
        }
        if let Some(w) = lit_num(dc.get("borderThickness")) {
            o.insert("border_width".into(), json!(w.clamp(0.0, 20.0)));
        }
        if let Some(b) = lit_str(dc.get("defaultShow")) {
            o.insert("show_blank".into(), json!(b == "true"));
        }
    }
    match custom_item(v) {
        Some((item, name)) => {
            o.insert("source".into(), json!("custom"));
            o.insert(
                "name".into(),
                json!(name.chars().take(MAX_NAME).collect::<String>()),
            );
            match item_bytes(r, &item) {
                None => {
                    o.insert(
                        "error".into(),
                        json!(format!("the map file `{}` is not in the report", clip(&item, 80))),
                    );
                }
                Some(bytes) => match &*decoded(bytes) {
                    Ok(m) => {
                        let (shapes, simplified) = shapes_json(m);
                        o.insert("shapes".into(), Value::Array(shapes));
                        if simplified {
                            o.insert("simplified".into(), json!(true));
                        }
                    }
                    Err(e) => {
                        o.insert("error".into(), json!(clip(e, 300)));
                    }
                },
            }
        }
        None => {
            let url = lit_str(shape.and_then(|p| p.get("mapUrl"))).filter(|u| !u.is_empty());
            let by_url = lit_str(shape.and_then(|p| p.get("datasourceType"))).as_deref() == Some("url");
            if by_url || url.is_some() {
                o.insert("source".into(), json!("url"));
            } else {
                o.insert("source".into(), json!("builtin"));
            }
        }
    }
    Some(Value::Object(o))
}

fn clip(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

// ---------------------------------------------------------------------------------------------
// shape_map.set_map
// ---------------------------------------------------------------------------------------------

/// The map file of `p`: `path` or `data` (the JSON text, base64 or a `data:` URL), with its name.
fn map_param(p: &Value) -> Result<Option<(Vec<u8>, String)>, String> {
    let path = opt_str(p, "path")?.filter(|s| !s.trim().is_empty());
    let data = opt_str(p, "data")?.filter(|s| !s.trim().is_empty());
    match (path, data) {
        (Some(_), Some(_)) => Err("give `path` or `data`, not both".into()),
        (None, None) => Ok(None),
        (Some(path), None) => {
            let fp = std::path::Path::new(path);
            let len = std::fs::metadata(fp)
                .map_err(|e| format!("cannot read {}: {e}", fp.display()))?
                .len();
            if len > MAX_MAP_BYTES as u64 {
                return Err(format!(
                    "{} is {len} bytes; the limit is {MAX_MAP_BYTES} ({} MB)",
                    fp.display(),
                    MAX_MAP_BYTES >> 20
                ));
            }
            let bytes = std::fs::read(fp).map_err(|e| format!("cannot read {}: {e}", fp.display()))?;
            let file = fp
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default();
            Ok(Some((bytes, file)))
        }
        (None, Some(d)) => {
            if d.len() > MAX_DATA_CHARS {
                return Err(format!(
                    "`data` is longer than {MAX_DATA_CHARS} characters (maps up to {} MB)",
                    MAX_MAP_BYTES >> 20
                ));
            }
            let t = d.trim_start();
            if t.starts_with('{') {
                return Ok(Some((d.as_bytes().to_vec(), String::new())));
            }
            let body = if t.get(..5).is_some_and(|h| h.eq_ignore_ascii_case("data:")) {
                let (head, body) = t.split_once(',').ok_or("a `data:` URL needs a comma")?;
                if !head.to_ascii_lowercase().ends_with(";base64") {
                    return Err("`data` must be the JSON text, base64 or a `data:…;base64,` URL".into());
                }
                body
            } else {
                t
            };
            let bytes = crate::images::base64(body).ok_or("`data` is neither JSON text nor valid base64")?;
            Ok(Some((bytes, String::new())))
        }
    }
}

/// The id of BIcraft's built-in shape map of the world's countries.
pub const WORLD_COUNTRIES: &str = "world_countries";
/// The file name it is stored under (the name Format › Map settings shows).
pub const WORLD_COUNTRIES_FILE: &str = "World countries (Natural Earth).topojson";

/// BIcraft's built-in map `world_countries`: Natural Earth's 1:110m countries (public domain,
/// bundled with the renderer, `bicraft_render::geo`) without Antarctica, as an ordinary TopoJSON
/// topology (Microsoft Learn: custom shape maps are TopoJSON): one object `countries`, each
/// country a Polygon or MultiPolygon geometry whose `id` is its ISO 3166-1 alpha-3 code (else its
/// name) and whose properties are `name`, `iso_a2`, `iso_a3`, `name_long`, `admin`, `formal_en`
/// and `name_fr` (Natural Earth's columns, the ones it has); coordinates quantized to 0.01°
/// (`transform`), one delta-encoded arc per ring.
pub fn world_topojson() -> Vec<u8> {
    let mut arcs: Vec<Value> = Vec::new();
    let mut geometries: Vec<Value> = Vec::new();
    for c in bicraft_render::geo::countries() {
        if c.iso3 == "ATA" {
            continue;
        }
        let mut polys: Vec<Value> = Vec::new();
        for poly in &c.polygons {
            let mut rings: Vec<Value> = Vec::new();
            for ring in poly {
                let q = |x: f64| (x * 100.0).round() as i64;
                let mut pts: Vec<(i64, i64)> = ring.iter().map(|&(x, y)| (q(x), q(y))).collect();
                if let Some(&first) = pts.first() {
                    pts.push(first); // TopoJSON rings are closed
                }
                let mut prev = (0i64, 0i64);
                let arc: Vec<Value> = pts
                    .iter()
                    .map(|&(x, y)| {
                        let d = json!([x - prev.0, y - prev.1]);
                        prev = (x, y);
                        d
                    })
                    .collect();
                rings.push(json!([arcs.len()]));
                arcs.push(Value::Array(arc));
            }
            polys.push(Value::Array(rings));
        }
        let mut props = Map::new();
        for (k, v) in [
            ("name", &c.name),
            ("iso_a2", &c.iso2),
            ("iso_a3", &c.iso3),
            ("name_long", &c.name_long),
            ("admin", &c.admin),
            ("formal_en", &c.formal_en),
            ("name_fr", &c.name_fr),
        ] {
            if !v.is_empty() {
                props.insert(k.into(), json!(v));
            }
        }
        let id = if c.iso3.is_empty() { &c.name } else { &c.iso3 };
        let g = match polys.len() {
            0 => continue,
            1 => json!({"type": "Polygon", "id": id, "properties": props, "arcs": polys.swap_remove(0)}),
            _ => json!({"type": "MultiPolygon", "id": id, "properties": props, "arcs": polys}),
        };
        geometries.push(g);
    }
    let topo = json!({
        "type": "Topology",
        "transform": {"scale": [0.01, 0.01], "translate": [0, 0]},
        "objects": {"countries": {"type": "GeometryCollection", "geometries": geometries}},
        "arcs": arcs,
    });
    topo.to_string().into_bytes()
}

/// `shape_map.set_map {page, visual, path? | data? | builtin?, name?, remove?}`.
pub(crate) fn set_map(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &["page", "visual", "path", "data", "builtin", "name", "remove"],
    )?;
    let key = str_param(p, "page")?;
    let visual = str_param(p, "visual")?;
    let remove = opt_bool(p, "remove")?.unwrap_or(false);
    let name = opt_str(p, "name")?;
    if name.is_some_and(|n| n.chars().count() > MAX_NAME) {
        return Err(format!("`name` is longer than {MAX_NAME} characters"));
    }
    let builtin = opt_str(p, "builtin")?.filter(|b| !b.trim().is_empty());
    let file = match (builtin, map_param(p)?) {
        (Some(_), Some(_)) => return Err("give `builtin`, `path` or `data`, not several".into()),
        (Some(b), None) if b.trim().eq_ignore_ascii_case(WORLD_COUNTRIES) => {
            Some((world_topojson(), WORLD_COUNTRIES_FILE.to_string()))
        }
        (Some(b), None) => {
            return Err(format!(
                "unknown built-in map `{}`: BIcraft's built-in map is `{WORLD_COUNTRIES}` (Natural Earth's \
                 countries); Desktop's own built-in maps are Microsoft's map data",
                clip(b, 80)
            ));
        }
        (None, f) => f,
    };
    let map = match (&file, remove) {
        (Some(_), true) => {
            return Err("give a map (`path`, `data` or `builtin`) or `remove`, not both".into());
        }
        (None, false) => {
            return Err(
                "shape_map.set_map needs a TopoJSON or GeoJSON map (`path` or `data`), `builtin: world_countries`, or `remove: true`"
                    .into(),
            );
        }
        (Some((bytes, _)), false) => Some(parse_map(bytes)?),
        (None, true) => None,
    };
    let (_, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let page = r.pages.get(pi).ok_or("internal: page index out of range")?;
    let vi = visual_index(page, visual)?;
    let v = page
        .visuals
        .get(vi)
        .ok_or("internal: visual index out of range")?;
    if !v
        .visual_type
        .as_deref()
        .is_some_and(|t| t.eq_ignore_ascii_case("shapeMap"))
    {
        return Err(format!(
            "visual `{visual}` is a {}, not a shape map",
            v.visual_type.as_deref().unwrap_or("group")
        ));
    }
    let old = custom_item(v).map(|(i, _)| i);
    let before = r.clone();
    let item = match file {
        Some((bytes, from)) => {
            let shown = name
                .map(str::to_string)
                .filter(|n| !n.trim().is_empty())
                .unwrap_or(if from.is_empty() {
                    "map.json".to_string()
                } else {
                    from
                });
            let ext = if shown.to_ascii_lowercase().ends_with(".topojson") {
                "topojson"
            } else {
                "json"
            };
            let f = crate::rich::resource_file_of(&shown, &bytes, ext, "map");
            let item = crate::rich::register_item(r, f, bytes, SHAPE_MAP_ITEM)?;
            Some((item, shown))
        }
        None => None,
    };
    let page = page_at_mut(r, pi)?;
    let v = page
        .visuals
        .get_mut(vi)
        .ok_or("internal: visual index out of range")?;
    let list = crate::rich::object_list(v, "objects", "shape")?;
    let props = crate::rich::plain_props(list)?;
    match &item {
        Some((item, shown)) => {
            props.insert("datasourceType".into(), crate::rich::text_lit("file_upload"));
            props.insert(
                "map".into(),
                json!({"geoJson": {
                    "type": crate::rich::text_lit("custom"),
                    "name": crate::rich::text_lit(shown),
                    "content": crate::rich::resource_url(item),
                }}),
            );
        }
        None => {
            props.remove("map");
            props.remove("datasourceType");
        }
    }
    if let Some(old) = old
        && item.as_ref().is_none_or(|(n, _)| !n.eq_ignore_ascii_case(&old))
    {
        crate::rich::drop_unreferenced_items(r, &[old], SHAPE_MAP_ITEM);
    }
    let changed = *r != before;
    if changed {
        s.report_dirty = true;
    }
    let keys: Vec<Value> = map
        .as_ref()
        .map(|m| {
            m.shapes
                .iter()
                .take(50)
                .map(|sh| json!(sh.keys.first().cloned().unwrap_or_default()))
                .collect()
        })
        .unwrap_or_default();
    Ok(json!({
        "visual": visual,
        "resource": item.as_ref().map(|(i, _)| i.clone()),
        "shapes": map.as_ref().map_or(0, |m| m.shapes.len()),
        "points": map.as_ref().map_or(0, |m| m.points),
        "keys": keys,
        "changed": changed,
    }))
}

#[cfg(test)]
#[path = "shape_maps_tests.rs"]
mod tests;
