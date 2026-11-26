//! Map visuals (docs/formats/maps.md): inserting the four map types with their wells, a shape
//! map's custom TopoJSON / GeoJSON map stored as a `ShapeMap` resource, its snapshot, its PBIP
//! and PBIX round trips, the data of a bubble map. Synthetic model only.

use bicraft_report::Report;
use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Two squares; the second's key is a property only.
const TOPO: &str = r#"{"type":"Topology",
 "transform":{"scale":[0.5,0.5],"translate":[10,40]},
 "arcs":[[[2,0],[0,2]], [[2,2],[-2,0],[0,-2],[2,0]], [[2,0],[2,0],[0,2],[-2,0]]],
 "objects":{"regions":{"type":"GeometryCollection","geometries":[
   {"type":"Polygon","arcs":[[0,1]],"id":"west","properties":{"name":"West side"}},
   {"type":"Polygon","arcs":[[2,-1]],"properties":{"name":"East"}}
 ]}}}"#;

const GEO: &str = r#"{"type":"FeatureCollection","features":[
  {"type":"Feature","properties":{"name":"Only"},
   "geometry":{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,0]]]}}]}"#;

/// Places(Region, Lat, Lon, Amount) and a report with one page.
fn session() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Geo"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "Places", "m": "let Source = #table(type table [Region = text, Lat = number, Lon = number, Amount = Int64.Type], {{\"west\", 48.85, 2.35, 5}, {\"East\", 45.76, 4.83, 7}, {\"North\", 50.63, 3.06, 2}}) in Source"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Places", "name": "Total", "expression": "SUM(Places[Amount])"}),
    );
    run(&mut s, "report.new", json!({}));
    s
}

fn page(s: &Session) -> String {
    s.report.as_ref().unwrap().pages[0].name.clone()
}

fn report(s: &Session) -> &Report {
    s.report.as_ref().unwrap()
}

fn add(s: &mut Session, ty: &str, fields: Value) -> String {
    let p = page(s);
    let out = run(
        s,
        "visual.add",
        json!({"page": p, "visual_type": ty, "x": 0, "y": 0, "width": 400, "height": 300, "fields": fields}),
    );
    out["visual"].as_str().unwrap().to_string()
}

fn snap_visual(s: &Session, name: &str) -> Value {
    let snap = s.snapshot();
    for p in snap["report"]["pages"].as_array().cloned().unwrap_or_default() {
        for v in p["visuals"].as_array().cloned().unwrap_or_default() {
            if v["name"] == name {
                return v;
            }
        }
    }
    panic!("visual {name} not in the snapshot")
}

fn items(s: &Session) -> Value {
    report(s).extra["resourcePackages"][0]["items"].clone()
}

#[test]
fn map_types_take_their_wells_and_bubble_maps_get_data() {
    let mut s = session();
    let bubbles = add(
        &mut s,
        "map",
        json!([{"role": "Category", "field": "Places[Region]"},
               {"role": "Y", "field": "Average(Places[Lat])"},
               {"role": "X", "field": "Average(Places[Lon])"},
               {"role": "Size", "field": "[Total]"}]),
    );
    let d = &snap_visual(&s, &bubbles)["data"];
    assert_eq!(d["status"], "ok", "{d}");
    let roles: Vec<&str> = d["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["role"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(roles, ["Category", "Y", "X", "Size"]);
    assert_eq!(d["rows"].as_array().map(Vec::len), Some(3));
    for (ty, fields) in [
        (
            "filledMap",
            json!([{"role": "Category", "field": "Places[Region]"}, {"role": "Tooltips", "field": "[Total]"}]),
        ),
        (
            "azureMap",
            json!([{"role": "Category", "field": "Places[Region]"}, {"role": "Size", "field": "[Total]"}]),
        ),
        (
            "shapeMap",
            json!([{"role": "Category", "field": "Places[Region]"}, {"role": "Value", "field": "[Total]"}]),
        ),
    ] {
        let v = add(&mut s, ty, fields);
        assert_eq!(snap_visual(&s, &v)["data"]["status"], "ok", "{ty}");
    }
    // a shape map without a custom map names the built-in map it cannot draw
    let sm = add(&mut s, "shapeMap", json!([]));
    assert_eq!(
        snap_visual(&s, &sm)["shape_map"],
        json!({"projection": "mercator", "source": "builtin"})
    );
    let v = run(&mut s, "report.validate", json!({}));
    assert_eq!(v["error_count"], 0, "{v:#}");
}

#[test]
fn a_custom_shape_map_is_a_registered_resource_and_round_trips() {
    let mut s = session();
    let p = page(&s);
    let sm = add(
        &mut s,
        "shapeMap",
        json!([{"role": "Category", "field": "Places[Region]"}, {"role": "Value", "field": "[Total]"}]),
    );
    let out = run(
        &mut s,
        "shape_map.set_map",
        json!({"page": p, "visual": sm, "data": TOPO, "name": "regions.topojson"}),
    );
    assert_eq!(out["shapes"], 2);
    assert_eq!(out["keys"], json!(["west", "East"]));
    let item = out["resource"].as_str().unwrap().to_string();
    assert!(
        item.starts_with("regions") && item.ends_with(".topojson"),
        "{item}"
    );
    assert_eq!(
        items(&s),
        json!([{"name": item, "path": item, "type": "ShapeMap"}])
    );
    let path = format!("StaticResources/RegisteredResources/{item}");
    assert!(report(&s).resources.iter().any(|r| r.path == path));
    let v = report(&s).pages[0].visuals.iter().find(|v| v.name == sm).unwrap();
    assert_eq!(
        v.visual_extra["objects"]["shape"],
        json!([{"properties": {
            "datasourceType": {"expr": {"Literal": {"Value": "'file_upload'"}}},
            "map": {"geoJson": {
                "type": {"expr": {"Literal": {"Value": "'custom'"}}},
                "name": {"expr": {"Literal": {"Value": "'regions.topojson'"}}},
                "content": {"expr": {"ResourcePackageItem": {"PackageName": "RegisteredResources", "PackageType": 1, "ItemName": item}}}
            }}
        }}])
    );
    let shot = snap_visual(&s, &sm)["shape_map"].clone();
    assert_eq!(shot["source"], "custom");
    assert_eq!(shot["name"], "regions.topojson");
    assert_eq!(shot["shapes"].as_array().map(Vec::len), Some(2), "{shot}");
    assert_eq!(shot["shapes"][0]["keys"], json!(["west", "West side"]));
    assert_eq!(
        shot["shapes"][0]["rings"][0],
        json!([11.0, 40.0, 11.0, 41.0, 10.0, 41.0, 10.0, 40.0, 11.0, 40.0])
    );
    // the format pane's projection reaches the snapshot
    run(
        &mut s,
        "visual.format",
        json!({"page": p, "visual": sm, "objects": [
            {"object": "shape", "property": "projectionEnum", "value": "orthographic"}]}),
    );
    assert_eq!(snap_visual(&s, &sm)["shape_map"]["projection"], "orthographic");
    // PBIP: the file on disk, the item and the reference come back
    let dir = TempDir::new("engine-shape-map");
    run(&mut s, "file.save_as", json!({"path": dir.join("Geo.pbip")}));
    let mut back = Session::new();
    run(&mut back, "file.open", json!({"path": dir.join("Geo.pbip")}));
    assert_eq!(items(&back), items(&s));
    assert_eq!(
        snap_visual(&back, &sm)["shape_map"],
        snap_visual(&s, &sm)["shape_map"]
    );
    let v = run(&mut back, "report.validate", json!({}));
    assert_eq!(v["error_count"], 0, "{v:#}");
    // a replacement drops the old file; remove drops the reference and the file
    let p2 = page(&back);
    let out = run(
        &mut back,
        "shape_map.set_map",
        json!({"page": p2, "visual": sm, "data": GEO}),
    );
    let item2 = out["resource"].as_str().unwrap().to_string();
    assert!(item2.starts_with("map") && item2.ends_with(".json"), "{item2}");
    assert_eq!(
        items(&back),
        json!([{"name": item2, "path": item2, "type": "ShapeMap"}])
    );
    assert!(!report(&back).resources.iter().any(|r| r.path == path));
    assert_eq!(snap_visual(&back, &sm)["shape_map"]["name"], "map.json");
    run(
        &mut back,
        "shape_map.set_map",
        json!({"page": p2, "visual": sm, "remove": true}),
    );
    assert_eq!(items(&back), json!([]));
    assert_eq!(snap_visual(&back, &sm)["shape_map"]["source"], "builtin");
}

#[test]
fn a_custom_shape_map_round_trips_through_a_pbir_pbix() {
    let mut s = session();
    let p = page(&s);
    let sm = add(&mut s, "shapeMap", json!([]));
    run(
        &mut s,
        "shape_map.set_map",
        json!({"page": p, "visual": sm, "data": GEO}),
    );
    let dir = TempDir::new("engine-shape-map-pbix");
    // a new report is PBIR: the PBIX keeps the package part, the item and the reference
    run(&mut s, "file.save_as", json!({"path": dir.join("Geo.pbix")}));
    let mut back = Session::new();
    run(&mut back, "file.open", json!({"path": dir.join("Geo.pbix")}));
    assert_eq!(items(&back), items(&s));
    assert_eq!(
        snap_visual(&back, &sm)["shape_map"],
        snap_visual(&s, &sm)["shape_map"]
    );
    assert_eq!(snap_visual(&back, &sm)["shape_map"]["source"], "custom");
    let v = run(&mut back, "report.validate", json!({}));
    assert_eq!(v["error_count"], 0, "{v:#}");
}

/// BIcraft's built-in world map is written as an ordinary custom TopoJSON map (a ShapeMap
/// resource named by `shape.map`), so Desktop opens it as any custom map, and round trips.
#[test]
fn the_built_in_world_map_is_written_as_an_ordinary_custom_topojson_map() {
    let mut s = session();
    let p = page(&s);
    let sm = add(
        &mut s,
        "shapeMap",
        json!([{"role": "Category", "field": "Places[Region]"}]),
    );
    let out = run(
        &mut s,
        "shape_map.set_map",
        json!({"page": p, "visual": sm, "builtin": "world_countries"}),
    );
    let n = out["shapes"].as_u64().unwrap();
    assert!((170..=177).contains(&n), "{out}");
    let item = out["resource"].as_str().unwrap().to_string();
    assert!(item.ends_with(".topojson"), "{item}");
    assert_eq!(
        items(&s),
        json!([{"name": item, "path": item, "type": "ShapeMap"}])
    );
    let v = report(&s).pages[0].visuals.iter().find(|v| v.name == sm).unwrap();
    let shape = &v.visual_extra["objects"]["shape"][0]["properties"];
    assert_eq!(
        shape["datasourceType"]["expr"]["Literal"]["Value"],
        "'file_upload'"
    );
    assert_eq!(
        shape["map"]["geoJson"]["name"]["expr"]["Literal"]["Value"],
        "'World countries (Natural Earth).topojson'"
    );
    assert_eq!(
        shape["map"]["geoJson"]["content"]["expr"]["ResourcePackageItem"]["ItemName"],
        item.as_str()
    );
    // the stored file is a TopoJSON topology with ISO codes and names as keys
    let path = format!("StaticResources/RegisteredResources/{item}");
    let file = report(&s).resources.iter().find(|r| r.path == path).unwrap();
    assert!(file.bytes.len() < 400_000, "{} bytes", file.bytes.len());
    let topo: Value = serde_json::from_slice(&file.bytes).unwrap();
    assert_eq!(topo["type"], "Topology");
    let geoms = topo["objects"]["countries"]["geometries"].as_array().unwrap();
    let france = geoms.iter().find(|g| g["id"] == "FRA").unwrap();
    assert_eq!(france["properties"]["name"], "France");
    assert_eq!(france["properties"]["iso_a2"], "FR");
    assert_eq!(france["properties"]["name_fr"], "France");
    assert!(!geoms.iter().any(|g| g["id"] == "ATA"), "no Antarctica");
    let shot = snap_visual(&s, &sm)["shape_map"].clone();
    assert_eq!(shot["source"], "custom");
    let keys: Vec<Value> = shot["shapes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["keys"].clone())
        .collect();
    assert!(
        keys.iter()
            .any(|k| k[0] == "FRA" && k.as_array().unwrap().contains(&json!("FR"))),
        "{:?}",
        keys.first()
    );
    // Zoom › Auto zoom reaches the snapshot
    run(
        &mut s,
        "visual.format",
        json!({"page": p, "visual": sm, "objects": [
            {"object": "zoom", "property": "autoZoom", "value": true}]}),
    );
    assert_eq!(snap_visual(&s, &sm)["shape_map"]["auto_zoom"], true);
    // PBIP and PBIX round trips
    let dir = TempDir::new("engine-shape-map-world");
    for f in ["World.pbip", "World.pbix"] {
        run(&mut s, "file.save_as", json!({"path": dir.join(f)}));
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": dir.join(f)}));
        assert_eq!(items(&back), items(&s), "{f}");
        assert_eq!(
            snap_visual(&back, &sm)["shape_map"],
            snap_visual(&s, &sm)["shape_map"],
            "{f}"
        );
        let v = run(&mut back, "report.validate", json!({}));
        assert_eq!(v["error_count"], 0, "{f}: {v:#}");
    }
    // unknown built-in maps and a built-in map with a file are errors
    for params in [
        json!({"page": p, "visual": sm, "builtin": "usa_states"}),
        json!({"page": p, "visual": sm, "builtin": "world_countries", "data": GEO}),
        json!({"page": p, "visual": sm, "builtin": "world_countries", "remove": true}),
        json!({"page": p, "visual": sm, "builtin": 3}),
    ] {
        assert!(s.execute("shape_map.set_map", &params).is_err(), "{params}");
    }
}

#[test]
fn bad_shape_map_params_are_errors() {
    let mut s = session();
    let p = page(&s);
    let sm = add(&mut s, "shapeMap", json!([]));
    let card = add(&mut s, "card", json!([]));
    let before = s.report.clone();
    for params in [
        json!({"page": p, "visual": sm}),
        json!({"page": p, "visual": sm, "data": "{\"type\":\"Topology\"}"}),
        json!({"page": p, "visual": sm, "data": "not base64 !"}),
        json!({"page": p, "visual": sm, "data": GEO, "remove": true}),
        json!({"page": p, "visual": sm, "data": GEO, "path": "x.json"}),
        json!({"page": p, "visual": card, "data": GEO}),
        json!({"page": p, "visual": "nope", "data": GEO}),
        json!({"page": p, "visual": sm, "data": 5}),
        json!({"page": p, "visual": sm, "data": GEO, "extra": 1}),
    ] {
        assert!(s.execute("shape_map.set_map", &params).is_err(), "{params}");
    }
    assert_eq!(s.report, before);
}
