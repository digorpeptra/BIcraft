//! Tests of the TopoJSON and GeoJSON reader of shape maps' custom maps (`shape_maps.rs`).

use super::*;

/// Two unit squares side by side sharing their middle edge (arc 0), quantized.
pub(crate) const TOPO: &str = r#"{"type":"Topology",
 "transform":{"scale":[0.5,0.5],"translate":[10,40]},
 "arcs":[[[2,0],[0,2]], [[2,2],[-2,0],[0,-2],[2,0]], [[2,0],[2,0],[0,2],[-2,0]]],
 "objects":{"regions":{"type":"GeometryCollection","geometries":[
   {"type":"Polygon","arcs":[[0,1]],"id":"west","properties":{"name":"West side","code":7}},
   {"type":"MultiPolygon","arcs":[[[2,-1]]],"properties":{"name":"East side"}},
   {"type":"Point","coordinates":[0,0]}
 ]}}}"#;

#[test]
fn topojson_decodes_quantized_arcs_and_reversed_arcs() {
    let m = parse_map(TOPO.as_bytes()).unwrap();
    assert_eq!(m.shapes.len(), 2, "the point has no area");
    let west = &m.shapes[0];
    assert_eq!(west.keys, ["west", "West side", "7"]);
    // arc 0: (11,40) → (11,41); arc 1 continues from (11,41) back to (11,40)
    assert_eq!(
        west.rings[0],
        vec![
            (11.0, 40.0),
            (11.0, 41.0),
            (10.0, 41.0),
            (10.0, 40.0),
            (11.0, 40.0)
        ]
    );
    let east = &m.shapes[1];
    assert_eq!(east.keys, ["East side"]);
    // arc 2, then arc 0 reversed (its first point dropped)
    assert_eq!(
        east.rings[0],
        vec![
            (11.0, 40.0),
            (12.0, 40.0),
            (12.0, 41.0),
            (11.0, 41.0),
            (11.0, 40.0)
        ]
    );
    assert_eq!(m.points, 10);
}

#[test]
fn geojson_features_polygons_and_multipolygons() {
    let g = r#"{"type":"FeatureCollection","features":[
      {"type":"Feature","id":12,"properties":{"name":"A","area":1.5,"flag":true},
       "geometry":{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,0]],[[0.2,0.1],[0.3,0.1],[0.3,0.2],[0.2,0.1]]]}},
      {"type":"Feature","properties":{"name":"B"},
       "geometry":{"type":"MultiPolygon","coordinates":[[[[2,0],[3,0],[3,1],[2,0]]],[[[4,0],[5,0],[5,1],[4,0]]]]}},
      {"type":"Feature","properties":{"name":"C"},"geometry":null}
    ]}"#;
    let m = parse_map(g.as_bytes()).unwrap();
    assert_eq!(m.shapes.len(), 2);
    assert_eq!(m.shapes[0].keys, ["12", "A", "1.5"]);
    assert_eq!(m.shapes[0].rings.len(), 2, "outer ring and hole");
    assert_eq!(m.shapes[1].rings.len(), 2, "two polygons, one shape");
}

#[test]
fn hostile_maps_are_errors() {
    for (bad, why) in [
        ("", "valid JSON"),
        ("[1,2]", "JSON object"),
        (r#"{"type":"Topology","arcs":[],"objects":{}}"#, "no shape"),
        (
            r#"{"type":"Topology","arcs":[[[0,0],[1,1]]],"objects":{"a":{"type":"Polygon","arcs":[[5]]}}}"#,
            "out of range",
        ),
        (
            r#"{"type":"Topology","arcs":[[[0,0],[1,1]]],"objects":{"a":{"type":"Polygon","arcs":[[-9223372036854775808]]}}}"#,
            "out of range",
        ),
        (
            r#"{"type":"Topology","arcs":[[[0,"x"]]],"objects":{}}"#,
            "two numbers",
        ),
        (
            r#"{"type":"FeatureCollection","features":[{"geometry":{"type":"Polygon","coordinates":[[[0,0],[1]]]}}]}"#,
            "two numbers",
        ),
        (r#"{"type":"LineString"}"#, "type"),
    ] {
        let e = parse_map(bad.as_bytes()).expect_err(bad);
        assert!(e.contains(why), "{bad}: {e}");
    }
    // nested collections stop at the depth cap
    let mut deep = String::from(r#"{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1]]]}"#);
    for _ in 0..12 {
        deep = format!(r#"{{"type":"GeometryCollection","geometries":[{deep}]}}"#);
    }
    assert!(parse_map(deep.as_bytes()).unwrap_err().contains("nested"));
    // a ring reusing one arc many times stays under the point budget
    let many = format!(
        r#"{{"type":"Topology","arcs":[[{}]],"objects":{{"a":{{"type":"Polygon","arcs":[[{}]]}}}}}}"#,
        vec!["[0,0]"; 1000].join(","),
        vec!["0"; 3000].join(",")
    );
    assert!(parse_map(many.as_bytes()).unwrap_err().contains("points"));
}

#[test]
fn the_snapshot_thins_large_maps() {
    let ring: Vec<(f64, f64)> = (0..100_000)
        .map(|i| {
            let a = f64::from(i) / 100_000.0 * std::f64::consts::TAU;
            (a.cos(), a.sin())
        })
        .collect();
    let m = GeoMap {
        points: ring.len(),
        shapes: vec![Shape {
            keys: vec!["k".into()],
            rings: vec![ring],
        }],
    };
    let (shapes, simplified) = shapes_json(&m);
    assert!(simplified);
    let n = shapes[0]["rings"][0].as_array().map_or(0, Vec::len) / 2;
    assert!(n <= SNAPSHOT_POINTS + 1 && n > SNAPSHOT_POINTS / 2, "{n}");
}
