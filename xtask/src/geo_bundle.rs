//! `cargo xtask geo-bundle SRC.geojson`: the renderer's bundled country outlines
//! (`crates/render/geo/`, docs/formats/maps.md "Bundled country outlines"); `cargo xtask
//! geo-bundle --basemap DIR`: the open basemap's 1:50m layers (format below, docs/formats/maps.md
//! "The open basemap's detailed layers").
//!
//! The source is Natural Earth's 1:110m Admin 0 countries (public domain), the GeoJSON file of
//! the `nvkelso/natural-earth-vector` repository at [`COMMIT`]; its SHA-256 is checked. Each
//! country keeps its ISO 3166-1 alpha-2 and alpha-3 codes (the `_EH` columns, which fill the
//! codes Natural Earth leaves at `-99` for France and Norway), its names (`NAME`, `NAME_LONG`,
//! `ADMIN`, `FORMAL_EN`, `NAME_FR`), its label point and its polygons, quantized to 0.01° and
//! delta-encoded. The format is read by `crates/render/src/geo.rs`:
//!
//! ```text
//! "BCGEO\x01\0\0"  u16 country count
//! per country:     str iso2, str iso3, u8 name count, str names…, i16 label lon, i16 label lat
//!                  (1/100 °), u16 polygon count; per polygon u16 ring count; per ring u16 point
//!                  count, then zigzag LEB128 deltas of lon, lat (1/100 °) from the previous
//!                  point (the first from 0, 0); rings are not closed
//! str:             u8 byte length, UTF-8
//! ```

use serde_json::Value;

/// The pinned commit of `nvkelso/natural-earth-vector`.
pub const COMMIT: &str = "ca96624a56bd078437bca8184e78163e5039ad19";
const PATH: &str = "geojson/ne_110m_admin_0_countries.geojson";
const SHA256: &str = "6866c877d39cba9c357620878839b336d569f8c662d3cfab4cb1dbe2d39c977f";
const OUT_DIR: &str = "crates/render/geo";
const OUT: &str = "ne_110m_admin_0_countries.bin";
const NAME_FIELDS: &[&str] = &["NAME", "NAME_LONG", "ADMIN", "FORMAL_EN", "NAME_FR"];

pub fn cmd(args: &[String]) -> Result<(), String> {
    if let [flag, dir] = args
        && flag == "--basemap"
    {
        return basemap_cmd(dir);
    }
    let [src] = args else {
        return Err(format!(
            "usage: cargo xtask geo-bundle SRC.geojson | --basemap DIR\n  SRC: https://raw.githubusercontent.com/nvkelso/natural-earth-vector/{COMMIT}/{PATH}\n  DIR: a folder holding the files of {COMMIT}/geojson/ named in xtask/src/geo_bundle.rs (BASEMAP_SRC)"
        ));
    };
    let bytes = std::fs::read(src).map_err(|e| format!("cannot read {src}: {e}"))?;
    let sha = crate::font_subset::sha256_hex(&bytes);
    if sha != SHA256 {
        return Err(format!(
            "{src}: SHA-256 {sha}, expected {SHA256} ({PATH} at {COMMIT})"
        ));
    }
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| format!("{src}: {e}"))?;
    let (out, stats) = encode(&v)?;
    let dir = crate::root().join(OUT_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::write(dir.join(OUT), &out).map_err(|e| format!("{OUT}: {e}"))?;
    let source = format!(
        "{OUT}: the country outlines of the renderer's offline maps (crates/render/src/geo.rs),\n\
         made by `cargo xtask geo-bundle SRC` (xtask/src/geo_bundle.rs) from Natural Earth.\n\n\
         Upstream:  Natural Earth, 1:110m Cultural Vectors, Admin 0 - Countries (https://www.naturalearthdata.com/)\n\
         Mirror:    https://github.com/nvkelso/natural-earth-vector, commit {COMMIT}\n\
         Source:    {PATH} ({} bytes)\n\
         \x20          SHA-256 {SHA256}\n\
         Licence:   public domain (https://www.naturalearthdata.com/about/terms-of-use/)\n\
         Kept:      ISO_A2_EH / ISO_A2, ISO_A3_EH / ISO_A3 (-99 dropped), {}, LABEL_X / LABEL_Y,\n\
         \x20          the polygons quantized to 0.01 degree, consecutive repeated points dropped\n\
         Output:    {} countries, {} polygons, {} points, {} bytes\n\
         \x20          SHA-256 {}\n",
        bytes.len(),
        NAME_FIELDS.join(", "),
        stats.0,
        stats.1,
        stats.2,
        out.len(),
        crate::font_subset::sha256_hex(&out),
    );
    std::fs::write(dir.join("SOURCE.txt"), source).map_err(|e| format!("SOURCE.txt: {e}"))?;
    println!(
        "geo-bundle: {} countries, {} polygons, {} points: {OUT_DIR}/{OUT} ({} bytes)",
        stats.0,
        stats.1,
        stats.2,
        out.len()
    );
    Ok(())
}

fn code(props: &Value, fields: &[&str]) -> String {
    fields
        .iter()
        .filter_map(|f| props.get(*f).and_then(Value::as_str))
        .find(|s| !s.is_empty() && *s != "-99")
        .unwrap_or("")
        .to_string()
}

fn put_str(out: &mut Vec<u8>, s: &str) -> Result<(), String> {
    let n = u8::try_from(s.len()).map_err(|_| format!("name too long: {s}"))?;
    out.push(n);
    out.extend_from_slice(s.as_bytes());
    Ok(())
}

fn put_u16(out: &mut Vec<u8>, n: usize) -> Result<(), String> {
    let n = u16::try_from(n).map_err(|_| format!("count {n} over 65535"))?;
    out.extend_from_slice(&n.to_le_bytes());
    Ok(())
}

fn put_var(out: &mut Vec<u8>, v: i32) {
    let mut z = ((v << 1) ^ (v >> 31)) as u32;
    loop {
        let b = (z & 0x7f) as u8;
        z >>= 7;
        if z == 0 {
            out.push(b);
            break;
        }
        out.push(b | 0x80);
    }
}

fn q(x: f64) -> i32 {
    (x * 100.0).round() as i32
}

/// The rings of a Polygon's coordinates, quantized, repeated points dropped, not closed.
fn rings(coords: &Value) -> Vec<Vec<(i32, i32)>> {
    let mut out = Vec::new();
    for ring in coords.as_array().into_iter().flatten() {
        let mut r: Vec<(i32, i32)> = Vec::new();
        for p in ring.as_array().into_iter().flatten() {
            let (Some(x), Some(y)) = (p.get(0).and_then(Value::as_f64), p.get(1).and_then(Value::as_f64))
            else {
                continue;
            };
            let pt = (q(x), q(y));
            if r.last() != Some(&pt) {
                r.push(pt);
            }
        }
        if r.len() > 1 && r.first() == r.last() {
            r.pop();
        }
        if r.len() >= 3 {
            out.push(r);
        }
    }
    out
}

/// Countries, polygons and points written.
type Stats = (usize, usize, usize);

fn encode(v: &Value) -> Result<(Vec<u8>, Stats), String> {
    let feats = v.get("features").and_then(Value::as_array).ok_or("no features")?;
    let mut out = b"BCGEO\x01\0\0".to_vec();
    put_u16(&mut out, feats.len())?;
    let (mut polys, mut points) = (0usize, 0usize);
    for f in feats {
        let props = f.get("properties").unwrap_or(&Value::Null);
        put_str(&mut out, &code(props, &["ISO_A2_EH", "ISO_A2"]))?;
        put_str(&mut out, &code(props, &["ISO_A3_EH", "ISO_A3"]))?;
        let names: Vec<&str> = NAME_FIELDS
            .iter()
            .map(|n| props.get(*n).and_then(Value::as_str).unwrap_or(""))
            .collect();
        out.push(names.len() as u8);
        for n in names {
            put_str(&mut out, n)?;
        }
        let lx = props.get("LABEL_X").and_then(Value::as_f64).unwrap_or(0.0);
        let ly = props.get("LABEL_Y").and_then(Value::as_f64).unwrap_or(0.0);
        for c in [q(lx), q(ly)] {
            let c = i16::try_from(c).map_err(|_| "label out of range")?;
            out.extend_from_slice(&c.to_le_bytes());
        }
        let g = f.get("geometry").unwrap_or(&Value::Null);
        let coords = g.get("coordinates").unwrap_or(&Value::Null);
        let parts: Vec<Vec<Vec<(i32, i32)>>> = match g.get("type").and_then(Value::as_str) {
            Some("Polygon") => vec![rings(coords)],
            Some("MultiPolygon") => coords.as_array().into_iter().flatten().map(rings).collect(),
            _ => Vec::new(),
        };
        let parts: Vec<_> = parts.into_iter().filter(|p| !p.is_empty()).collect();
        put_u16(&mut out, parts.len())?;
        for p in &parts {
            polys += 1;
            put_u16(&mut out, p.len())?;
            for r in p {
                put_u16(&mut out, r.len())?;
                let mut prev = (0, 0);
                for &pt in r {
                    put_var(&mut out, pt.0 - prev.0);
                    put_var(&mut out, pt.1 - prev.1);
                    prev = pt;
                    points += 1;
                }
            }
        }
    }
    Ok((out, (feats.len(), polys, points)))
}

// ---------------------------------------------------------------------------------------------
// `cargo xtask geo-bundle --basemap DIR`: the detailed layers of the open basemap
// ---------------------------------------------------------------------------------------------
//
// Natural Earth's 1:50m countries, lakes, admin-1 boundary lines (the large countries Natural
// Earth splits at that scale) and populated places, from the GeoJSON files of the same commit,
// each SHA-256 checked, into `ne_50m_basemap.bin`, read by `crates/render/src/geo.rs`:
//
// ```text
// "BCMAP\x01\0\0"
// u16 country count; per country: str iso3, str admin, u16 polygon count, polygons
// u16 lake count;    per lake: a polygon
// u16 line count;    per line: points
// u16 city count;    per city: str name, str ascii name (empty when the same), str adm0_a3,
//                    str adm1 name, i16 lon, i16 lat (1/100 °), u8 scalerank, u8 flags
//                    (1: national capital), u32 pop_max (little endian)
// polygon:           u16 ring count, then each ring's points (rings not closed)
// points:            u16 count, then zigzag LEB128 deltas of lon, lat (1/100 °), the first
//                    from 0, 0
// ```

/// The detailed layers' sources: (file under `geojson/`, SHA-256).
const BASEMAP_SRC: &[(&str, &str)] = &[
    (
        "ne_50m_admin_0_countries.geojson",
        "3e458fc036ad0a66411f2c1e6cac49c5d7bfb81cb1123bc513b22511a2b7fdeb",
    ),
    (
        "ne_50m_lakes.geojson",
        "d350b75978b26fe839b797c2c529b2fb8f47fb3983c03f4964e36d5df9378a52",
    ),
    (
        "ne_50m_admin_1_states_provinces_lines.geojson",
        "72cca93c850d412628a5da4bc5ebfe21ba4d376eb34611bde6b623ee73f0fdcf",
    ),
    (
        "ne_50m_populated_places_simple.geojson",
        "8e70756b39fae9bcdc1e332bfc510c024c5edd3a13203ffd20092ee37b61d978",
    ),
];
const BASEMAP_OUT: &str = "ne_50m_basemap.bin";
const BASEMAP_SOURCE: &str = "SOURCE-50m.txt";

fn basemap_cmd(dir: &str) -> Result<(), String> {
    let mut docs = Vec::new();
    let mut src_lines = String::new();
    for (name, sha) in BASEMAP_SRC {
        let p = std::path::Path::new(dir).join(name);
        let bytes = std::fs::read(&p).map_err(|e| format!("cannot read {}: {e}", p.display()))?;
        let got = crate::font_subset::sha256_hex(&bytes);
        if got != *sha {
            return Err(format!(
                "{}: SHA-256 {got}, expected {sha} (geojson/{name} at {COMMIT})",
                p.display()
            ));
        }
        src_lines.push_str(&format!(
            "           geojson/{name} ({} bytes)\n             SHA-256 {sha}\n",
            bytes.len()
        ));
        docs.push(serde_json::from_slice::<Value>(&bytes).map_err(|e| format!("{name}: {e}"))?);
    }
    let [countries, lakes, lines, places] = docs.as_slice() else {
        return Err("four sources expected".into());
    };
    let mut out = b"BCMAP\x01\0\0".to_vec();
    let mut points = 0usize;
    // countries
    let feats = features(countries)?;
    put_u16(&mut out, feats.len())?;
    for f in feats {
        let props = f.get("properties").unwrap_or(&Value::Null);
        put_str(&mut out, &code(props, &["ISO_A3_EH", "ISO_A3", "ADM0_A3"]))?;
        put_str(&mut out, &code(props, &["ADMIN", "NAME"]))?;
        let parts = polygons(f);
        put_u16(&mut out, parts.len())?;
        for p in &parts {
            points += put_polygon(&mut out, p)?;
        }
    }
    let n_countries = feats.len();
    // lakes: every polygon of every feature
    let lake_polys: Vec<_> = features(lakes)?.iter().flat_map(polygons).collect();
    put_u16(&mut out, lake_polys.len())?;
    for p in &lake_polys {
        points += put_polygon(&mut out, p)?;
    }
    // admin-1 boundary lines
    let mut segs: Vec<Vec<(i32, i32)>> = Vec::new();
    for f in features(lines)? {
        let g = f.get("geometry").unwrap_or(&Value::Null);
        let coords = g.get("coordinates").unwrap_or(&Value::Null);
        match g.get("type").and_then(Value::as_str) {
            Some("LineString") => segs.push(line(coords)),
            Some("MultiLineString") => segs.extend(coords.as_array().into_iter().flatten().map(line)),
            _ => {}
        }
    }
    segs.retain(|s| s.len() >= 2);
    put_u16(&mut out, segs.len())?;
    for s in &segs {
        points += put_points(&mut out, s)?;
    }
    // populated places
    let cities = features(places)?;
    put_u16(&mut out, cities.len())?;
    for f in cities {
        let p = f.get("properties").unwrap_or(&Value::Null);
        let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let (name, ascii) = (s("name"), s("nameascii"));
        put_str(&mut out, &name)?;
        put_str(&mut out, if ascii == name { "" } else { &ascii })?;
        put_str(&mut out, &s("adm0_a3"))?;
        put_str(&mut out, &s("adm1name"))?;
        let num = |k: &str| p.get(k).and_then(Value::as_f64).unwrap_or(0.0);
        for c in [q(num("longitude")), q(num("latitude"))] {
            let c = i16::try_from(c).map_err(|_| "place out of range")?;
            out.extend_from_slice(&c.to_le_bytes());
        }
        out.push(num("scalerank").clamp(0.0, 255.0) as u8);
        out.push(u8::from(num("adm0cap") >= 1.0));
        out.extend_from_slice(&(num("pop_max").clamp(0.0, f64::from(u32::MAX)) as u32).to_le_bytes());
    }
    let root_dir = crate::root().join(OUT_DIR);
    std::fs::create_dir_all(&root_dir).map_err(|e| format!("{}: {e}", root_dir.display()))?;
    std::fs::write(root_dir.join(BASEMAP_OUT), &out).map_err(|e| format!("{BASEMAP_OUT}: {e}"))?;
    let stats = format!(
        "{n_countries} countries, {} lake polygons, {} admin-1 lines, {} places, {points} points",
        lake_polys.len(),
        segs.len(),
        cities.len()
    );
    let source = format!(
        "{BASEMAP_OUT}: the detailed layers of the renderer's open basemap (crates/render/src/geo.rs),\n\
         made by `cargo xtask geo-bundle --basemap DIR` (xtask/src/geo_bundle.rs) from Natural Earth.\n\n\
         Upstream:  Natural Earth, 1:50m Cultural and Physical Vectors (https://www.naturalearthdata.com/):\n\
         \x20          Admin 0 - Countries, Lakes, Admin 1 - States, Provinces (boundary lines),\n\
         \x20          Populated Places (simple)\n\
         Mirror:    https://github.com/nvkelso/natural-earth-vector, commit {COMMIT}\n\
         Sources:\n{src_lines}\
         Licence:   public domain (https://www.naturalearthdata.com/about/terms-of-use/)\n\
         Kept:      countries ISO_A3_EH / ISO_A3 / ADM0_A3 and ADMIN with their polygons; lake polygons;\n\
         \x20          admin-1 boundary lines; places name, nameascii, adm0_a3, adm1name, longitude,\n\
         \x20          latitude, scalerank, adm0cap, pop_max; coordinates quantized to 0.01 degree,\n\
         \x20          consecutive repeated points dropped\n\
         Output:    {stats}, {} bytes\n\
         \x20          SHA-256 {}\n",
        out.len(),
        crate::font_subset::sha256_hex(&out),
    );
    std::fs::write(root_dir.join(BASEMAP_SOURCE), source).map_err(|e| format!("{BASEMAP_SOURCE}: {e}"))?;
    println!(
        "geo-bundle --basemap: {stats}: {OUT_DIR}/{BASEMAP_OUT} ({} bytes)",
        out.len()
    );
    Ok(())
}

fn features(v: &Value) -> Result<&Vec<Value>, String> {
    v.get("features")
        .and_then(Value::as_array)
        .ok_or_else(|| "no features".to_string())
}

/// A feature's polygons (Polygon or MultiPolygon), quantized, empty ones left out.
fn polygons(f: &Value) -> Vec<Vec<Vec<(i32, i32)>>> {
    let g = f.get("geometry").unwrap_or(&Value::Null);
    let coords = g.get("coordinates").unwrap_or(&Value::Null);
    let parts: Vec<Vec<Vec<(i32, i32)>>> = match g.get("type").and_then(Value::as_str) {
        Some("Polygon") => vec![rings(coords)],
        Some("MultiPolygon") => coords.as_array().into_iter().flatten().map(rings).collect(),
        _ => Vec::new(),
    };
    parts.into_iter().filter(|p| !p.is_empty()).collect()
}

/// A LineString's points, quantized, repeated points dropped.
fn line(coords: &Value) -> Vec<(i32, i32)> {
    let mut r: Vec<(i32, i32)> = Vec::new();
    for p in coords.as_array().into_iter().flatten() {
        if let (Some(x), Some(y)) = (p.get(0).and_then(Value::as_f64), p.get(1).and_then(Value::as_f64)) {
            let pt = (q(x), q(y));
            if r.last() != Some(&pt) {
                r.push(pt);
            }
        }
    }
    r
}

fn put_points(out: &mut Vec<u8>, pts: &[(i32, i32)]) -> Result<usize, String> {
    put_u16(out, pts.len())?;
    let mut prev = (0, 0);
    for &pt in pts {
        put_var(out, pt.0 - prev.0);
        put_var(out, pt.1 - prev.1);
        prev = pt;
    }
    Ok(pts.len())
}

fn put_polygon(out: &mut Vec<u8>, rings: &[Vec<(i32, i32)>]) -> Result<usize, String> {
    put_u16(out, rings.len())?;
    let mut n = 0;
    for r in rings {
        n += put_points(out, r)?;
    }
    Ok(n)
}
