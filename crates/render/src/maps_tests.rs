//! Map visuals drawn offline (`maps.rs`): bubbles at latitude and longitude, the geocoding
//! placeholder, a shape map's custom areas with their colours, legend, tooltips and clicks, and
//! hostile `shape_map` values.

use std::collections::HashMap;

use serde_json::{Value, json};

use super::*;

fn snap(visuals: Value, lang: &str) -> Value {
    json!({
        "version": 1,
        "ui": {"strings_language": lang},
        "model": {"culture": "en-US", "tables": []},
        "report": {"theme": null, "pages": [{"name": "p1", "display_name": "Page 1", "width": 640, "height": 360, "visuals": visuals}]}
    })
}

fn render(s: &Value) -> PageImage {
    let fonts = FontBook::bundled_only();
    let o = RenderOptions {
        system_fonts: false,
        hits: true,
        text_dump: true,
        ..RenderOptions::default()
    };
    render_page(s, 0, &HashMap::<String, Vec<u8>>::new(), &fonts, &o).unwrap()
}

fn texts(img: &PageImage) -> Vec<String> {
    img.texts.iter().map(|t| t.text.clone()).collect()
}

fn ok(cols: Value, rows: Value) -> Value {
    json!({"status": "ok", "columns": cols, "rows": rows})
}

fn place_fields() -> Value {
    json!([{"role": "Category", "kind": "column", "table": "Places", "name": "Region"}])
}

fn bubbles(ty: &str) -> Value {
    json!({"name": "m", "type": ty, "x": 0, "y": 0, "width": 640, "height": 360, "title": "Sales",
        "fields": place_fields(),
        "data": ok(json!([{"name": "Region", "role": "Category", "kind": "column", "data_type": "string"},
                          {"name": "Lat", "role": "Y", "kind": "aggregation"},
                          {"name": "Lon", "role": "X", "kind": "aggregation"},
                          {"name": "Total", "role": "Size", "kind": "measure"}]),
                   json!([["Paris", 48.85, 2.35, 50], ["Lyon", 45.76, 4.83, 10], ["Lille", 50.63, 3.06, 5]]))})
}

#[test]
fn bubble_maps_draw_offline_and_select_their_locations() {
    for ty in ["map", "azureMap", "filledMap"] {
        let img = render(&snap(json!([bubbles(ty)]), ""));
        let v = &img.visuals[0];
        assert_eq!(v.status, "drawn", "{ty}");
        assert!(
            v.note.as_deref().is_some_and(|n| n.contains("offline")),
            "{ty}: {:?}",
            v.note
        );
        assert!(texts(&img).iter().any(|t| t == "Made with Natural Earth"), "{ty}");
        // a region: the 1:50m layers name the cities around (Paris's own bubble aside)
        assert!(
            texts(&img).iter().any(|t| t == "Paris" || t == "Brussels"),
            "{ty}: {:?}",
            texts(&img)
        );
        let circles: Vec<&hits::Hit> = img
            .hits
            .iter()
            .filter(|h| matches!(h.shape, hits::HitShape::Circle { .. }))
            .collect();
        assert_eq!(circles.len(), 3, "{ty}");
        // Paris is the biggest bubble, Lille the northernmost (smallest y)
        let paris = circles
            .iter()
            .find(|h| h.tip.iter().any(|(_, v)| v == "Paris"))
            .unwrap();
        let lille = circles
            .iter()
            .find(|h| h.tip.iter().any(|(_, v)| v == "Lille"))
            .unwrap();
        let (_, py, pw, _) = paris.shape.bounds();
        let (_, ly, lw, _) = lille.shape.bounds();
        assert!(pw > lw, "{ty}: size");
        assert!(ly < py, "{ty}: north is up");
        let a = paris.action_for(false).unwrap();
        assert_eq!(a["action"], "select");
        assert_eq!(a["point"], json!([{"c": 0, "v": "Paris"}]));
        assert_eq!(a["ctrl"], false);
    }
}

#[test]
fn place_names_alone_show_the_geocoding_placeholder() {
    let v = json!({"name": "m", "type": "map", "x": 0, "y": 0, "width": 640, "height": 360,
        "fields": place_fields(),
        "data": ok(json!([{"name": "Region", "role": "Category", "kind": "column", "data_type": "string"}]),
                   json!([["North depot"], ["Hillside"]]))});
    let img = render(&snap(json!([v.clone()]), ""));
    assert_eq!(img.visuals[0].status, "placeholder");
    assert!(img.visuals[0].note.as_deref().unwrap_or("").contains("geocoding"));
    assert!(
        texts(&img)
            .iter()
            .any(|t| t.contains("Offline, only countries and major cities are placed")),
        "{:?}",
        texts(&img)
    );
    let fr = render(&snap(json!([v]), "fr"));
    assert!(
        texts(&fr)
            .iter()
            .any(|t| t.contains("seuls les pays et les grandes villes")),
        "{:?}",
        texts(&fr)
    );
    // nothing bound: the map icon and Desktop's prompt
    let empty = json!({"name": "m", "type": "azureMap", "x": 0, "y": 0, "width": 300, "height": 200});
    assert_eq!(render(&snap(json!([empty]), "")).visuals[0].status, "blank");
}

/// Two squares in longitude and latitude, keyed `west` / `East`.
fn squares() -> Value {
    json!({"source": "custom", "name": "regions.json", "projection": "mercator", "shapes": [
        {"keys": ["west", "West side"], "rings": [[10, 40, 11, 40, 11, 41, 10, 41, 10, 40]]},
        {"keys": ["East"], "rings": [[11, 40, 12, 40, 12, 41, 11, 41, 11, 40]]},
        {"keys": ["Nowhere"], "rings": [[12, 40, 13, 40, 13, 41, 12, 41, 12, 40]]}]})
}

fn shape_visual(sm: Value, data: Value) -> Value {
    json!({"name": "s", "type": "shapeMap", "x": 0, "y": 0, "width": 640, "height": 360,
        "fields": place_fields(), "shape_map": sm, "data": data})
}

fn pixel(img: &PageImage, x: f32, y: f32) -> (u8, u8, u8) {
    let w = img.width as usize;
    let i = (y as usize * w + x as usize) * 4;
    let p = img.rgba.get(i..i + 3).unwrap_or(&[0, 0, 0]);
    (p[0], p[1], p[2])
}

#[test]
fn shape_maps_colour_matched_areas_alike_and_select_them() {
    let data = ok(
        json!([{"name": "Region", "role": "Category", "kind": "column", "data_type": "string"},
               {"name": "Total", "role": "Value", "kind": "measure"}]),
        json!([["WEST", 10], ["east", 100], ["Elsewhere", 3]]),
    );
    let fonts = FontBook::bundled_only();
    let o = RenderOptions {
        system_fonts: false,
        hits: true,
        raw: true,
        text_dump: true,
        ..RenderOptions::default()
    };
    let s = snap(json!([shape_visual(squares(), data)]), "");
    let img = render_page(&s, 0, &HashMap::<String, Vec<u8>>::new(), &fonts, &o).unwrap();
    assert_eq!(img.visuals[0].status, "drawn", "{:?}", img.visuals[0].note);
    let polys: Vec<&hits::Hit> = img
        .hits
        .iter()
        .filter(|h| matches!(h.shape, hits::HitShape::Polygon { .. }))
        .collect();
    assert_eq!(polys.len(), 3);
    // keys match without case; Color saturation alone does not shade the areas: Desktop
    // 2.159.1231.0 fills every matched area with the one location colour (docs/formats/maps.md)
    let centre = |h: &hits::Hit| {
        let (x, y, w, hh) = h.shape.bounds();
        (x + w / 2.0, y + hh / 2.0)
    };
    let (wx, wy) = centre(polys[0]);
    let (ex, ey) = centre(polys[1]);
    let (nx, ny) = centre(polys[2]);
    assert!(wx < ex && (wy - ey).abs() < 1.0, "side by side");
    assert_eq!(pixel(&img, ex, ey), pixel(&img, wx, wy), "one colour");
    assert_ne!(pixel(&img, ex, ey), pixel(&img, nx, ny), "matched and unmatched");
    let a = hits::hit_at(&img.hits, wx, wy)
        .and_then(|h| h.action_for(true))
        .unwrap();
    assert_eq!(a["point"], json!([{"c": 0, "v": "WEST"}]));
    assert_eq!(a["ctrl"], true);
    let tip = &hits::hit_at(&img.hits, ex, ey).unwrap().tip;
    assert_eq!(
        tip,
        &vec![
            ("Region".to_string(), "east".to_string()),
            ("Total".to_string(), "100".to_string())
        ]
    );
    // an area without data: its key as the tooltip, no click action
    let none = hits::hit_at(&img.hits, nx, ny).unwrap();
    assert_eq!(none.action, hits::HitAction::None);
    assert_eq!(none.tip, vec![("Region".to_string(), "Nowhere".to_string())]);
    // no gradient legend
    let t = texts(&img);
    assert!(!t.iter().any(|x| x == "3" || x == "100"), "{t:?}");
}

#[test]
fn shape_maps_colour_by_legend_and_dim_unselected_areas() {
    let data = ok(
        json!([{"name": "Region", "role": "Category", "kind": "column", "data_type": "string"},
               {"name": "Kind", "role": "Series", "kind": "column", "data_type": "string"},
               {"name": "Total", "role": "Value", "kind": "measure"}]),
        json!([["west", "A", 10], ["East", "B", 100]]),
    );
    let mut v = shape_visual(squares(), data);
    v["interaction"] = json!({"selected": {"points": [[{"c": 0, "v": "west"}]]}});
    let img = render(&snap(json!([v]), ""));
    assert_eq!(img.visuals[0].status, "drawn");
    let t = texts(&img);
    assert!(
        t.iter().any(|x| x == "Kind") && t.iter().any(|x| x == "A") && t.iter().any(|x| x == "B"),
        "{t:?}"
    );
}

#[test]
fn shape_maps_without_custom_maps_say_why() {
    for (sm, want, want_fr) in [
        (
            json!({"source": "builtin", "projection": "mercator"}),
            "Choose World countries (Natural Earth)",
            "Pays du monde (Natural Earth)",
        ),
        (
            json!({"source": "url"}),
            "not fetched offline",
            "pas récupérées hors ligne",
        ),
        (
            json!({"source": "custom", "error": "the map is not valid JSON"}),
            "cannot be read",
            "illisible",
        ),
    ] {
        let img = render(&snap(json!([shape_visual(sm.clone(), Value::Null)]), ""));
        assert_eq!(img.visuals[0].status, "placeholder");
        assert!(texts(&img).iter().any(|x| x.contains(want)), "{:?}", texts(&img));
        let fr = render(&snap(json!([shape_visual(sm, Value::Null)]), "fr"));
        assert!(texts(&fr).iter().any(|x| x.contains(want_fr)), "{:?}", texts(&fr));
    }
}

#[test]
fn hostile_map_values_never_crash() {
    let bad_maps = [
        json!({"source": "custom", "shapes": "x"}),
        json!({"source": "custom", "shapes": [{"keys": 5, "rings": [[1e308, -1e308, 1e308, 1e308, 0, 0]]}]}),
        json!({"source": "custom", "shapes": [{"rings": [[0, 0, 0, 0, 0, 0]]}]}),
        json!({"source": "custom", "projection": "orthographic", "shapes": [{"rings": [[179, 89, -179, -89, 0, 0, 90, 0]]}]}),
        json!({"source": "custom", "projection": "albersUsa", "shapes": [{"rings": [["a", null, 1]]}]}),
        json!({"source": "custom", "shapes": [{"keys": ["k"], "rings": [[5e6, 5e6, 5e6, 5e7, 6e6, 5e6]]}]}),
        json!(null),
    ];
    for sm in bad_maps {
        let img = render(&snap(
            json!([shape_visual(
                sm,
                ok(
                    json!([{"name": "R", "role": "Category"}]),
                    json!([[null], [1], ["k"]])
                )
            )]),
            "",
        ));
        assert_eq!(img.visuals.len(), 1);
    }
    for rows in [
        json!([["a", 1e308, 1e308, -1]]),
        json!([["a", 91, 181, null], ["b", "x", 0, 5]]),
        json!([["a", 0, 0, 0], ["a", 0, 0, 0]]),
    ] {
        let mut v = bubbles("map");
        v["data"]["rows"] = rows;
        assert_eq!(render(&snap(json!([v]), "")).visuals.len(), 1);
    }
}

fn circle_of(img: &PageImage, place: &str) -> Option<(f32, f32)> {
    img.hits.iter().find_map(|h| match h.shape {
        hits::HitShape::Circle { cx, cy, .. } if h.tip.iter().any(|(_, v)| v == place) => Some((cx, cy)),
        _ => None,
    })
}

fn merc(lon: f64, lat: f64) -> (f64, f64) {
    let p = lat.to_radians();
    (
        lon.to_radians(),
        (std::f64::consts::FRAC_PI_4 + p / 2.0).tan().ln(),
    )
}

/// The page pixel of a longitude and latitude, from two bubbles drawn at known places.
fn locator(a: ((f64, f64), (f32, f32)), b: ((f64, f64), (f32, f32))) -> impl Fn(f64, f64) -> (f32, f32) {
    let (ma, mb) = (merc(a.0.0, a.0.1), merc(b.0.0, b.0.1));
    let k = f64::from(b.1.0 - a.1.0) / (mb.0 - ma.0);
    move |lon, lat| {
        let m = merc(lon, lat);
        (
            (f64::from(a.1.0) + (m.0 - ma.0) * k) as f32,
            (f64::from(a.1.1) - (m.1 - ma.1) * k) as f32,
        )
    }
}

fn raw_render(s: &Value) -> PageImage {
    let fonts = FontBook::bundled_only();
    let o = RenderOptions {
        system_fonts: false,
        hits: true,
        raw: true,
        text_dump: true,
        ..RenderOptions::default()
    };
    render_page(s, 0, &HashMap::<String, Vec<u8>>::new(), &fonts, &o).unwrap()
}

#[test]
fn bubble_maps_draw_country_outlines_under_the_bubbles() {
    let img = raw_render(&snap(json!([bubbles("map")]), ""));
    let at = locator(
        ((2.35, 48.85), circle_of(&img, "Paris").unwrap()),
        ((4.83, 45.76), circle_of(&img, "Lyon").unwrap()),
    );
    // inland France (Bourges) is land, the Bay of Biscay is sea
    let (lx, ly) = at(2.4, 47.1);
    let land = pixel(&img, lx, ly);
    let (sx, sy) = at(-3.5, 45.5);
    let sea = pixel(&img, sx, sy);
    assert_ne!(land, sea, "land {land:?}, sea {sea:?}");
    assert!(
        sea.2 > sea.0,
        "the sea is tinted with the first (blue) data colour: {sea:?}"
    );
    assert!(
        land.0 > 230 && land.1 > 230 && land.2 > 230,
        "light land: {land:?}"
    );
    // a region takes the 1:50m layers: Lake Geneva (absent at 1:110m) is drawn as water
    let (gx, gy) = at(6.55, 46.42);
    let around =
        [(-3.0, 0.0), (3.0, 0.0), (0.0, -2.0), (0.0, 0.0)].map(|(dx, dy)| pixel(&img, gx + dx, gy + dy));
    assert!(around.contains(&sea), "Lake Geneva: {around:?}, sea {sea:?}");
}

#[test]
fn city_names_are_placed_offline() {
    let v = json!({"name": "m", "type": "map", "x": 0, "y": 0, "width": 640, "height": 360,
        "fields": place_fields(),
        "data": ok(json!([{"name": "Region", "role": "Category", "kind": "column", "data_type": "string"},
                          {"name": "Total", "role": "Size", "kind": "measure"}]),
                   json!([["Paris", 50], ["Lyon", 10], ["Marseille, France", 20], ["Hillside", 5]]))});
    let img = render(&snap(json!([v]), ""));
    let m = &img.visuals[0];
    assert_eq!(m.status, "drawn");
    let note = m.note.as_deref().unwrap_or("");
    assert!(
        note.contains("countries and cities") && note.contains("1 location(s)"),
        "{note}"
    );
    let (p, l, s) = (
        circle_of(&img, "Paris").unwrap(),
        circle_of(&img, "Lyon").unwrap(),
        circle_of(&img, "Marseille, France").unwrap(),
    );
    assert!(p.1 < l.1 && l.1 < s.1, "north is up: {p:?} {l:?} {s:?}");
    assert!(circle_of(&img, "Hillside").is_none());
}

fn countries_visual(ty: &str, rows: Value) -> Value {
    json!({"name": "m", "type": ty, "x": 0, "y": 0, "width": 640, "height": 360,
        "fields": place_fields(),
        "data": ok(json!([{"name": "Country", "role": "Category", "kind": "column", "data_type": "string"},
                          {"name": "Total", "role": "Size", "kind": "measure"}]), rows)})
}

#[test]
fn country_names_and_iso_codes_are_placed_offline() {
    let img = raw_render(&snap(
        json!([countries_visual(
            "map",
            json!([["France", 50], ["DEU", 30], ["it", 20], ["Atlantis", 5]])
        )]),
        "",
    ));
    let v = &img.visuals[0];
    assert_eq!(v.status, "drawn", "{:?}", v.note);
    let note = v.note.clone().unwrap_or_default();
    assert!(
        note.contains("countries") && note.contains("1 location(s)"),
        "{note}"
    );
    let (fx, fy) = circle_of(&img, "France").unwrap();
    let (gx, gy) = circle_of(&img, "DEU").unwrap();
    let (ix, iy) = circle_of(&img, "it").unwrap();
    assert!(circle_of(&img, "Atlantis").is_none());
    assert!(gx > fx && gy < fy, "Germany is north-east of France");
    assert!(iy > gy && ix > fx, "Italy is south-east of Germany");
    // French names too
    let fr = raw_render(&snap(
        json!([countries_visual(
            "azureMap",
            json!([["Allemagne", 1], ["Espagne", 2]])
        )]),
        "",
    ));
    assert!(circle_of(&fr, "Allemagne").is_some() && circle_of(&fr, "Espagne").is_some());
}

#[test]
fn filled_maps_fill_the_countries_their_locations_name() {
    let img = raw_render(&snap(
        json!([countries_visual(
            "filledMap",
            json!([["France", 50], ["Germany", 30]])
        )]),
        "",
    ));
    let v = &img.visuals[0];
    assert_eq!(v.status, "drawn");
    assert!(v.note.as_deref().unwrap_or("").contains("filled"), "{:?}", v.note);
    let polys: Vec<&hits::Hit> = img
        .hits
        .iter()
        .filter(|h| matches!(h.shape, hits::HitShape::Polygon { .. }))
        .collect();
    assert_eq!(polys.len(), 2, "one area per country");
    assert!(
        img.hits
            .iter()
            .all(|h| !matches!(h.shape, hits::HitShape::Circle { .. }))
    );
    let france = polys
        .iter()
        .find(|h| h.tip.iter().any(|(_, v)| v == "France"))
        .unwrap();
    assert_eq!(
        france.action_for(false).unwrap()["point"],
        json!([{"c": 0, "v": "France"}])
    );
    // France (Bourges) takes the first data colour; Spain (Madrid), without data, the land's
    // the pixel of a place, from the bounds of France's European polygons and of its area
    let fr = &crate::geo::countries()[crate::geo::find("FR").unwrap()];
    let (mut a0, mut b0, mut a1, mut b1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for b in fr.bounds.iter().filter(|b| b.0 > -10.0) {
        (a0, b0, a1, b1) = (a0.min(b.0), b0.min(b.1), a1.max(b.2), b1.max(b.3));
    }
    let (m0, m1) = (merc(a0, b0), merc(a1, b1));
    let (x, y, w, h) = france.shape.bounds();
    let at = |lon: f64, lat: f64| {
        let m = merc(lon, lat);
        (
            x + ((m.0 - m0.0) / (m1.0 - m0.0)) as f32 * w,
            y + ((m1.1 - m.1) / (m1.1 - m0.1)) as f32 * h,
        )
    };
    let (bx, by) = at(2.4, 47.1);
    let p = pixel(&img, bx, by);
    assert!(p.2 > p.0 + 40, "France filled with the data colour: {p:?}");
    let (sx, sy) = at(8.2, 46.8);
    let swiss = pixel(&img, sx, sy);
    assert!(
        swiss.0 > 200 && swiss.1 > 200,
        "Switzerland keeps the land colour: {swiss:?}"
    );
}

#[test]
fn shape_maps_auto_zoom_on_the_areas_with_data() {
    let data = ok(
        json!([{"name": "Region", "role": "Category", "kind": "column", "data_type": "string"}]),
        json!([["East"]]),
    );
    let width = |sm: Value| {
        let mut v = shape_visual(sm, data.clone());
        v["width"] = json!(200); // the width limits the scale
        let img = render(&snap(json!([v]), ""));
        img.hits
            .iter()
            .find(|h| h.tip.iter().any(|(_, v)| v == "East"))
            .map(|h| h.shape.bounds().3) // the height: the island is level with the square
            .unwrap()
    };
    let whole = width(squares());
    let mut zoomed = squares();
    zoomed["auto_zoom"] = json!(true);
    // a far, small island of East (French Guiana to mainland France) does not widen the view
    zoomed["shapes"][1]["rings"]
        .as_array_mut()
        .unwrap()
        .push(json!([60, 40, 60.2, 40, 60.2, 40.2, 60, 40]));
    let z = width(zoomed);
    assert!(z > whole * 1.5, "auto zoom: {z} against {whole}");
}
