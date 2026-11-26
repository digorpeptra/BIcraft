//! Map visuals drawn offline (docs/formats/maps.md, docs/render.md "Maps"): no tiles; an open
//! basemap of Natural Earth's vector data (public domain, [`crate::geo`]). Drawing only: nothing
//! here writes to the file.
//!
//! - Map, filled map and Azure Maps: a Web Mercator base map (sea, parallels and meridians,
//!   every country's land and borders in theme colours; for a region, the 1:50m countries, lakes,
//!   admin-1 boundaries and city names) fitted to the data, marked "Made with Natural Earth".
//!   Bubbles at Latitude and Longitude, at the label point of the country a location names
//!   (English or French name, ISO alpha-2 or alpha-3 code) or at the city it names (Natural
//!   Earth's populated places), sized by Size, coloured by Legend. A filled map whose locations
//!   are countries fills them (Legend colours). Other place names need geocoding by Bing or
//!   Azure Maps: a placeholder that says so.
//! - Shape map with a custom TopoJSON or GeoJSON map (the snapshot's `shape_map`): its areas,
//!   projected (Mercator, equirectangular, orthographic, Albers USA; maps in other coordinates
//!   are drawn as they are), coloured by Legend (Color saturation alone does not shade them, as
//!   in Desktop), keys matched without case to the shapes' id or property values. Built-in maps are Microsoft's
//!   and maps from a URL are not fetched: a placeholder.
//!
//! Every bubble and area is a hit: a click selects its location (cross-filtering the page),
//! the tooltip lists its fields.

use serde_json::Value;

use crate::canvas::{Anchor, Canvas, Rgb};
use crate::charts::{Bx, blank_visual, place_legend, tw, tx};
use crate::data::{Col, Ctx, Dataset, Fmt, Rc, arr, clip, fin, get, key_of, sk};
use crate::geo;
use crate::hits::{self, HitShape, Point};

/// Map visual kinds (`render_kind`).
pub(crate) const MAP_KINDS: &[&str] = &["mapBubble", "mapFilled", "mapAzure", "mapShape"];

/// Most bubbles and shapes drawn.
const MAX_MARKS: usize = 5000;
/// Most points of one ring in a hit region.
const MAX_HIT_POINTS: usize = 400;
/// Most rings of one shape in a hit region.
const MAX_HIT_RINGS: usize = 32;

/// The map outline icon of an empty map.
pub(crate) const ICON_MAP: &[&str] = &[
    "M1.5 3.5l4-1.5 5 1.5 4-1.5v10.5l-4 1.5-5-1.5-4 1.5z",
    "M5.5 2v10.5",
    "M10.5 3.5V14",
];

/// What the map drew: the visual's status and note.
pub(crate) struct MapOut {
    pub status: &'static str,
    pub note: Option<String>,
}

fn drawn(note: Option<String>) -> MapOut {
    MapOut {
        status: "drawn",
        note,
    }
}

fn fr(ctx: &Ctx<'_>) -> bool {
    ctx.ui_lang == "fr"
}

fn placeholder(cv: &mut Canvas, b: Bx, ctx: &Ctx<'_>, lines: &[String], note: String) -> MapOut {
    crate::bodies::placeholder_frame(cv, b, ctx);
    crate::bodies::placeholder_lines(cv, lines, b, ctx);
    MapOut {
        status: "placeholder",
        note: Some(note),
    }
}

fn col_at(cols: &[Col], role: &str) -> Option<usize> {
    cols.iter().position(|c| c.role.eq_ignore_ascii_case(role))
}

fn rows(ds: &Dataset) -> &[Vec<Value>] {
    if ds.fake { &[] } else { &ds.rows }
}

/// The tooltip rows of a data row: every column but the ones in `skip`, formatted.
fn tip(ds: &Dataset, r: &[Value], skip: &[usize], ctx: &Ctx<'_>) -> Vec<(String, String)> {
    ds.cols
        .iter()
        .enumerate()
        .filter(|(i, _)| !skip.contains(i))
        .take(12)
        .map(|(i, c)| {
            let x = r.get(i).unwrap_or(&Value::Null);
            let t = if x.is_null() {
                ctx.blank()
            } else {
                hits::cell(x, c, ctx)
            };
            (c.name.clone(), clip(&t, 200))
        })
        .collect()
}

/// The grouping values of a row (location and legend), as `view.interact` `select` takes them.
fn point(r: &[Value], groups: &[Option<usize>]) -> Point {
    groups
        .iter()
        .flatten()
        .map(|&c| (c, r.get(c).cloned().unwrap_or(Value::Null)))
        .collect()
}

/// Legend values in their order, at most 12, and each row's colour index.
fn legend(ds: &Dataset, li: Option<usize>, ctx: &Ctx<'_>) -> (Vec<(String, Rgb)>, Vec<usize>) {
    let Some(li) = li else {
        return (Vec::new(), vec![0; rows(ds).len()]);
    };
    let mut vals: Vec<Value> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for r in rows(ds) {
        let v = r.get(li).cloned().unwrap_or(Value::Null);
        if seen.insert(key_of(&v)) {
            vals.push(v);
        }
    }
    vals.sort_by(crate::data::cmp_raw);
    vals.truncate(12);
    let at: std::collections::HashMap<String, usize> =
        vals.iter().enumerate().map(|(i, v)| (key_of(v), i)).collect();
    let col = ds.cols.get(li);
    let items = vals
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let t = col.map(|c| hits::cell(v, c, ctx)).unwrap_or_default();
            (if t.is_empty() { ctx.blank() } else { t }, ctx.color(i))
        })
        .collect();
    let idx = rows(ds)
        .iter()
        .map(|r| r.get(li).and_then(|v| at.get(&key_of(v)).copied()).unwrap_or(0))
        .collect();
    (items, idx)
}

/// The "drawn offline" mark at the bottom right of a map.
fn offline_mark(cv: &mut Canvas, ctx: &Ctx<'_>, b: Bx, text: &str) {
    let px = (ctx.chart_px * 0.85).max(6.0);
    let w = tw(ctx, text, px);
    if w + 8.0 > b.w || b.h < px * 3.0 {
        return;
    }
    cv.rect(b.w - w - 8.0, b.h - px * 1.6, w + 8.0, px * 1.6, ctx.bg, 0.8);
    tx(
        cv,
        ctx,
        b.w - 4.0,
        b.h - px * 0.8,
        text,
        px,
        ctx.muted,
        Anchor::End,
    );
}

// ---------------------------------------------------------------------------------------------
// Map, filled map, Azure Maps
// ---------------------------------------------------------------------------------------------

/// Web Mercator, in radians (latitudes clamped to ±85°).
fn mercator(lon: f64, lat: f64) -> (f64, f64) {
    let phi = lat.clamp(-85.0, 85.0).to_radians();
    (
        lon.to_radians(),
        (std::f64::consts::FRAC_PI_4 + phi / 2.0).tan().ln(),
    )
}

/// A Web Mercator view of a box `w`×`h`: the projected centre (radians) and the scale.
#[derive(Clone, Copy)]
struct View {
    cx: f64,
    cy: f64,
    k: f64,
    w: f64,
    h: f64,
}

impl View {
    /// The view of projected bounds `(x0, y0, x1, y1)` grown by `grow` (1.3: 15 % around), at
    /// least `min_deg` across.
    fn fit(r: (f64, f64, f64, f64), grow: f64, min_deg: f64, w: f32, h: f32) -> Option<View> {
        let min_span = min_deg.to_radians();
        let sx = ((r.2 - r.0) * grow).max(min_span);
        let sy = ((r.3 - r.1) * grow).max(min_span);
        let k = (f64::from(w) / sx).min(f64::from(h) / sy);
        (k.is_finite() && k > 0.0).then_some(View {
            cx: (r.0 + r.2) / 2.0,
            cy: (r.1 + r.3) / 2.0,
            k,
            w: f64::from(w),
            h: f64::from(h),
        })
    }
    fn px(&self, x: f64) -> f32 {
        (self.w / 2.0 + (x - self.cx) * self.k) as f32
    }
    fn py(&self, y: f64) -> f32 {
        (self.h / 2.0 - (y - self.cy) * self.k) as f32
    }
    /// The projected bounds shown.
    fn shown(&self) -> (f64, f64, f64, f64) {
        let (hw, hh) = (self.w / 2.0 / self.k, self.h / 2.0 / self.k);
        (self.cx - hw, self.cy - hh, self.cx + hw, self.cy + hh)
    }
}

/// Projected bounds of longitude and latitude bounds.
fn merc_bounds((a0, b0, a1, b1): (f64, f64, f64, f64)) -> (f64, f64, f64, f64) {
    let (x0, y0) = mercator(a0, b0);
    let (x1, y1) = mercator(a1, b1);
    (x0, y0, x1, y1)
}

fn union(a: Option<(f64, f64, f64, f64)>, b: (f64, f64, f64, f64)) -> (f64, f64, f64, f64) {
    match a {
        None => b,
        Some(a) => (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)),
    }
}

/// The base map's colours, from the theme's text and background (light and dark): the sea a
/// tint of the first data colour, the land near the background, the borders between.
fn base_colors(ctx: &Ctx<'_>) -> (Rgb, Rgb, Rgb) {
    let sea = ctx.band.mix(ctx.color(0), 0.14);
    let land = ctx.fg.mix(ctx.bg, 0.96);
    let border = ctx.fg.mix(ctx.bg, 0.7);
    (sea, land, border)
}

/// The pixel rings of a country's polygons in `view` (polygons out of view left out).
fn country_rings(c: &geo::Country, view: &View) -> Vec<Vec<(f32, f32)>> {
    poly_rings(&c.polygons, &c.bounds, view)
}

/// The pixel rings of polygons in `view`, given each polygon's bounds (those out of view left
/// out).
fn poly_rings(
    polygons: &[Vec<Vec<(f64, f64)>>],
    bounds: &[(f64, f64, f64, f64)],
    view: &View,
) -> Vec<Vec<(f32, f32)>> {
    let shown = view.shown();
    let mut out = Vec::new();
    for (poly, bounds) in polygons.iter().zip(bounds) {
        let (x0, y0, x1, y1) = merc_bounds(*bounds);
        if x1 < shown.0 || x0 > shown.2 || y1 < shown.1 || y0 > shown.3 {
            continue;
        }
        for ring in poly {
            out.push(
                ring.iter()
                    .map(|&(lon, lat)| {
                        let (x, y) = mercator(lon, lat);
                        (view.px(x), view.py(y))
                    })
                    .collect(),
            );
        }
    }
    out
}

fn rings_path(rings: &[Vec<(f32, f32)>]) -> Option<tiny_skia::Path> {
    let mut pb = tiny_skia::PathBuilder::new();
    for r in rings.iter().filter(|r| r.len() >= 3) {
        let mut it = r.iter();
        if let Some(&(x, y)) = it.next() {
            pb.move_to(x, y);
            for &(x, y) in it {
                pb.line_to(x, y);
            }
            pb.close();
        }
    }
    pb.finish()
}

/// Rings thinned for a hit region.
fn hit_rings(rings: &[Vec<(f32, f32)>]) -> Vec<Vec<(f32, f32)>> {
    rings
        .iter()
        .take(MAX_HIT_RINGS)
        .map(|r| {
            let step = r.len().div_ceil(MAX_HIT_POINTS).max(1);
            r.iter().step_by(step).copied().collect()
        })
        .collect()
}

/// The offline base map in `view`: the sea, parallels and meridians, then every country's land
/// and borders; `fills` gives a country (its [`geo::countries`] index) another fill and opacity.
fn base_map(
    cv: &mut Canvas,
    ctx: &Ctx<'_>,
    view: &View,
    fills: &std::collections::HashMap<usize, (Rgb, f32)>,
    avoid: &[(f32, f32, f32)],
) {
    let (sea, land, border) = base_colors(ctx);
    let (w, h) = (view.w as f32, view.h as f32);
    cv.rect(0.0, 0.0, w, h, sea, 1.0);
    // parallels and meridians, about four across
    let span_deg = (view.w / view.k).to_degrees();
    let step = [0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 45.0]
        .into_iter()
        .find(|s| span_deg / s <= 6.0)
        .unwrap_or(60.0);
    let grid = sea.mix(ctx.bg, 0.5);
    let shown = view.shown();
    let mut lon = (shown.0.to_degrees() / step).floor() * step;
    for _ in 0..40 {
        let x = view.px(lon.to_radians());
        if x > w {
            break;
        }
        if x >= 0.0 {
            cv.line(x, 0.0, x, h, grid, 1.0, None);
        }
        lon += step;
    }
    let lat_of = |y: f64| (2.0 * y.exp().atan() - std::f64::consts::FRAC_PI_2).to_degrees();
    let mut lat = ((lat_of(shown.1) / step).floor() * step).max(-85.0);
    for _ in 0..40 {
        if lat > 85.0 {
            break;
        }
        let y = view.py(mercator(0.0, lat).1);
        if y < 0.0 {
            break;
        }
        if y <= h {
            cv.line(0.0, y, w, y, grid, 1.0, None);
        }
        lat += step;
    }
    let bw = if view.k > 2000.0 { 0.8 } else { 0.6 };
    let m = geo::basemap();
    if span_deg >= DETAIL_DEG || m.countries.is_empty() {
        // the world: the 1:110m countries
        for (i, c) in geo::countries().iter().enumerate() {
            let rings = country_rings(c, view);
            let Some(path) = rings_path(&rings) else {
                continue;
            };
            cv.fill_path(&path, land, 1.0);
            if let Some(&(fill, alpha)) = fills.get(&i) {
                cv.fill_path(&path, fill, alpha);
            }
            cv.stroke(&path, border, 1.0, bw, None);
        }
    } else {
        // a region: the 1:50m countries, lakes and admin-1 boundaries
        for (i, a) in m.countries.iter().enumerate() {
            let rings = poly_rings(&a.polygons, &a.bounds, view);
            let Some(path) = rings_path(&rings) else {
                continue;
            };
            cv.fill_path(&path, land, 1.0);
            if let Some(&(fill, alpha)) = geo::area_country(i).and_then(|c| fills.get(&c)) {
                cv.fill_path(&path, fill, alpha);
            }
            cv.stroke(&path, border, 1.0, bw, None);
        }
        for l in &m.lakes {
            let rings = poly_rings(&l.polygons, &l.bounds, view);
            if let Some(path) = rings_path(&rings) {
                cv.fill_path(&path, sea, 1.0);
                cv.stroke(&path, border, 0.6, bw * 0.8, None);
            }
        }
        if span_deg < ADMIN1_DEG {
            let c = border.mix(land, 0.4);
            for (pts, b) in &m.lines {
                if let Some(path) = line_path(pts, *b, view) {
                    cv.stroke(&path, c, 1.0, 0.5, Some((3.0, 2.0)));
                }
            }
        }
    }
    if span_deg < CITY_DEG {
        cities(cv, ctx, view, land, avoid);
    }
}

/// Longitudes across under which the base map takes the 1:50m layers (else the 1:110m
/// countries of the whole world).
const DETAIL_DEG: f64 = 90.0;
/// Longitudes across under which the admin-1 boundaries are drawn.
const ADMIN1_DEG: f64 = 70.0;
/// Longitudes across under which cities are drawn.
const CITY_DEG: f64 = 70.0;

/// A line of (longitude, latitude) with its bounds, in `view` (`None` out of view).
fn line_path(pts: &[(f64, f64)], b: (f64, f64, f64, f64), view: &View) -> Option<tiny_skia::Path> {
    let shown = view.shown();
    let (x0, y0, x1, y1) = merc_bounds(b);
    if x1 < shown.0 || x0 > shown.2 || y1 < shown.1 || y0 > shown.3 {
        return None;
    }
    let mut pb = tiny_skia::PathBuilder::new();
    for (i, &(lon, lat)) in pts.iter().enumerate() {
        let (x, y) = mercator(lon, lat);
        if i == 0 {
            pb.move_to(view.px(x), view.py(y));
        } else {
            pb.line_to(view.px(x), view.py(y));
        }
    }
    pb.finish()
}

/// The cities of the view: a dot and the name on a veil of the land colour (readable over
/// filled countries), the larger first (Natural Earth's `scalerank`, then population), names
/// never overlapping each other or the circles `avoid` (x, y, radius: the data's bubbles),
/// about one per 16,000 square pixels.
fn cities(cv: &mut Canvas, ctx: &Ctx<'_>, view: &View, land: Rgb, avoid: &[(f32, f32, f32)]) {
    let (w, h) = (view.w as f32, view.h as f32);
    let shown = view.shown();
    let mut cand: Vec<&geo::City> = geo::basemap()
        .cities
        .iter()
        .filter(|c| {
            let (x, y) = mercator(c.lon, c.lat);
            x >= shown.0 && x <= shown.2 && y >= shown.1 && y <= shown.3
        })
        .collect();
    cand.sort_by_key(|c| (c.rank, std::cmp::Reverse(c.pop)));
    let most = ((w * h) / 16_000.0).clamp(2.0, 40.0) as usize;
    let px = (ctx.chart_px * 0.85).max(6.0);
    let dot = ctx.fg.mix(ctx.bg, 0.45);
    let mut taken: Vec<(f32, f32, f32, f32)> = avoid
        .iter()
        .map(|&(x, y, r)| (x - r, y - r, x + r, y + r))
        .collect();
    let mut placed = 0usize;
    for c in cand {
        if placed >= most {
            break;
        }
        let (mx, my) = mercator(c.lon, c.lat);
        let (x, y) = (view.px(mx), view.py(my));
        let name = c.name.as_str();
        let nw = tw(ctx, name, px);
        // the name right of the dot, else left of it
        let right = (x - 3.0, y - px * 0.7, x + 4.0 + nw + 1.0, y + px * 0.7);
        let left = (x - 4.0 - nw - 1.0, y - px * 0.7, x + 3.0, y + px * 0.7);
        let fits = |r: &(f32, f32, f32, f32)| {
            r.0 >= 0.0
                && r.2 <= w
                && r.1 >= 0.0
                && r.3 <= h
                && !taken
                    .iter()
                    .any(|t| r.0 < t.2 && t.0 < r.2 && r.1 < t.3 && t.1 < r.3)
        };
        let Some((r, anchor, tx_x)) = [(right, Anchor::Start, x + 4.0), (left, Anchor::End, x - 4.0)]
            .into_iter()
            .find(|(r, _, _)| fits(r))
        else {
            continue;
        };
        taken.push(r);
        placed += 1;
        let (lx, lw) = match anchor {
            Anchor::End => (tx_x - nw - 1.0, nw + 2.0),
            _ => (tx_x - 1.0, nw + 2.0),
        };
        cv.rect(lx, y - px * 0.6, lw, px * 1.2, land, 0.7);
        let rad = if c.capital { 2.2 } else { 1.7 };
        cv.circle(x, y, rad, Some((dot, 1.0)), Some((ctx.bg, 0.8)));
        tx(cv, ctx, tx_x, y, name, px, ctx.muted, anchor);
    }
}

/// The note of locations the offline map could not place.
fn unplaced_note(n: usize) -> Option<String> {
    (n > 0).then(|| {
        format!(
            "{n} location(s) are not country names, ISO codes or major cities: placing them needs geocoding \
             by the online map service (Bing or Azure Maps)"
        )
    })
}

/// A map of bubbles at their latitude and longitude, or at the country a location names; a
/// filled map's countries filled.
pub(crate) fn bubble_map(cv: &mut Canvas, ds: &Dataset, b: Bx, f: &Fmt, ctx: &Ctx<'_>, kind: &str) -> MapOut {
    let cols = &ds.cols;
    let (yi, xi) = (col_at(cols, "Y"), col_at(cols, "X"));
    let ci = cols.iter().position(|c| c.rc == Rc::Cat);
    let li = col_at(cols, "Series");
    let si = col_at(cols, "Size");
    let latlon = yi.zip(xi);
    if latlon.is_none() && ci.is_none() {
        blank_visual(cv, ctx, b, ICON_MAP);
        return MapOut {
            status: "blank",
            note: None,
        };
    }
    // the country each row's location names
    let country: Vec<Option<usize>> = rows(ds)
        .iter()
        .map(|r| {
            ci.and_then(|c| r.get(c))
                .and_then(Value::as_str)
                .and_then(geo::find)
        })
        .collect();
    let any_country = country.iter().any(Option::is_some);
    // the city each other row's location names (without latitude and longitude)
    let city: Vec<Option<usize>> = rows(ds)
        .iter()
        .zip(&country)
        .map(|(r, c)| {
            if latlon.is_some() || c.is_some() {
                return None;
            }
            ci.and_then(|c| r.get(c))
                .and_then(Value::as_str)
                .and_then(geo::find_city)
        })
        .collect();
    let any_city = city.iter().any(Option::is_some);
    let filled = kind == "mapFilled" && any_country;
    if latlon.is_none() && !any_country && !any_city && !rows(ds).is_empty() {
        let lines = if fr(ctx) {
            vec![
                "Carte".to_string(),
                "Hors ligne, seuls les pays et les grandes villes sont placés ;".to_string(),
                "les autres lieux nécessitent le service de cartes en ligne".to_string(),
                "(Bing ou Azure Maps) : ajoutez Latitude et Longitude".to_string(),
            ]
        } else {
            vec![
                "Map".to_string(),
                "Offline, only countries and major cities are placed;".to_string(),
                "other places need geocoding by the online map service".to_string(),
                "(Bing or Azure Maps): add Latitude and Longitude".to_string(),
            ]
        };
        return placeholder(
            cv,
            b,
            ctx,
            &lines,
            "place names other than countries and major cities need geocoding by the online map service \
             (Bing or Azure Maps): not drawn offline; country names, ISO codes and Natural Earth's populated \
             places are placed offline, and Latitude and Longitude draw bubbles"
                .into(),
        );
    }
    let (items, leg_idx) = legend(ds, li, ctx);
    let (lt, lb) = if !items.is_empty() && f.legend_show != Some(false) {
        let title = li.and_then(|l| cols.get(l)).map_or("", |c| c.name.as_str());
        place_legend(cv, ctx, f, &items, title, b)
    } else {
        (0.0, 0.0)
    };
    let (top, h) = (lt, (b.h - lt - lb).max(1.0));
    // points: (row, projected x, y, size)
    let mut pts: Vec<(usize, f64, f64, Option<f64>)> = Vec::new();
    let mut extent: Option<(f64, f64, f64, f64)> = None;
    let mut cities_extent: Option<(f64, f64, f64, f64)> = None;
    let mut unplaced = 0usize;
    for (i, r) in rows(ds).iter().enumerate().take(MAX_MARKS) {
        let size = si.and_then(|s| r.get(s)).and_then(fin);
        let c = country
            .get(i)
            .copied()
            .flatten()
            .and_then(|c| geo::countries().get(c));
        if let Some((yi, xi)) = latlon.filter(|_| !filled) {
            let (Some(lat), Some(lon)) = (r.get(yi).and_then(fin), r.get(xi).and_then(fin)) else {
                continue;
            };
            if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
                continue;
            }
            let (x, y) = mercator(lon, lat);
            pts.push((i, x, y, size));
        } else if let Some(c) = c {
            if let Some(mb) = c.main_bounds() {
                extent = Some(union(extent, merc_bounds(mb)));
            }
            if !filled {
                let (x, y) = mercator(c.label.0, c.label.1);
                pts.push((i, x, y, size));
            }
        } else if let Some(t) = city
            .get(i)
            .copied()
            .flatten()
            .and_then(|t| geo::basemap().cities.get(t))
        {
            let (x, y) = mercator(t.lon, t.lat);
            cities_extent = Some(union(cities_extent, (x, y, x, y)));
            pts.push((i, x, y, size));
        } else {
            unplaced += 1;
        }
    }
    // the attribution Natural Earth suggests (its data is public domain)
    let off = if fr(ctx) {
        "Réalisé avec Natural Earth"
    } else {
        "Made with Natural Earth"
    };
    // the view: 15 % around the bubbles at latitude and longitude or at cities (at least 6°
    // across, so that a lone bubble shows its surroundings), 5 % around the countries
    let view = if latlon.is_some() && !filled && !pts.is_empty() {
        let mut r = None;
        for p in &pts {
            r = Some(union(r, (p.1, p.2, p.1, p.2)));
        }
        r.and_then(|r| View::fit(r, 1.3, 6.0, b.w, h))
    } else if let Some(e) = extent {
        let e = cities_extent.map_or(e, |c| union(Some(e), c));
        View::fit(e, 1.1, 2.0, b.w, h)
    } else if let Some(c) = cities_extent {
        View::fit(c, 1.3, 6.0, b.w, h)
    } else {
        None
    };
    let (view, placed) = match view {
        Some(v) => (Some(v), true),
        // nothing placed: the world
        None => (
            View::fit(merc_bounds((-180.0, -58.0, 180.0, 80.0)), 1.0, 2.0, b.w, h),
            false,
        ),
    };
    let Some(view) = view else {
        return drawn(None);
    };
    let Some(mut layer) = cv.layer(0.0, top, b.w, h) else {
        return drawn(None);
    };
    let groups = [ci, li];
    let mut fills = std::collections::HashMap::new();
    if filled {
        for (i, r) in rows(ds).iter().enumerate().take(MAX_MARKS) {
            if let Some(c) = country.get(i).copied().flatten()
                && !fills.contains_key(&c)
            {
                let col = ctx.color(leg_idx.get(i).copied().unwrap_or(0));
                fills.insert(c, (col, 0.9 * ctx.ix.alpha(&point(r, &groups))));
            }
        }
    }
    let zmax = pts.iter().map(|p| p.3.unwrap_or(0.0).abs()).fold(0.0, f64::max);
    let rmax = 20.0f32.min(h / 8.0).max(4.0);
    let radius = |z: Option<f64>| match z {
        Some(z) if zmax > 0.0 => 3.0 + ((z.abs() / zmax).sqrt() as f32) * (rmax - 3.0),
        _ => 5.0,
    };
    // city names keep clear of the bubbles
    let avoid: Vec<(f32, f32, f32)> = pts
        .iter()
        .map(|p| (view.px(p.1), view.py(p.2), radius(p.3)))
        .collect();
    base_map(&mut layer, ctx, &view, &fills, &avoid);
    if filled && ctx.ix.on() {
        let mut done = std::collections::HashSet::new();
        for (i, r) in rows(ds).iter().enumerate().take(MAX_MARKS) {
            let Some(c) = country.get(i).copied().flatten() else {
                continue;
            };
            let Some(cc) = geo::countries().get(c) else {
                continue;
            };
            if !done.insert(c) {
                continue;
            }
            let rings = country_rings(cc, &view);
            if rings.is_empty() {
                continue;
            }
            let pt = point(r, &groups);
            layer.hit(ctx.ix.hit(
                HitShape::Polygon {
                    rings: hit_rings(&rings),
                },
                ctx.ix.select(&pt),
                tip(ds, r, &[], ctx),
            ));
        }
    }
    // bubbles, biggest first so small ones stay clickable
    let mut order: Vec<usize> = (0..pts.len()).collect();
    order.sort_by(|a, b| {
        let z = |i: &usize| pts.get(*i).and_then(|p| p.3).unwrap_or(0.0).abs();
        z(b).partial_cmp(&z(a)).unwrap_or(std::cmp::Ordering::Equal)
    });
    for i in order {
        let Some(&(ri, x, y, z)) = pts.get(i) else {
            continue;
        };
        let Some(r) = rows(ds).get(ri) else { continue };
        let rad = radius(z);
        let c = ctx.color(leg_idx.get(ri).copied().unwrap_or(0));
        let pt = point(r, &groups);
        let alpha = ctx.ix.alpha(&pt);
        let (x, y) = (view.px(x), view.py(y));
        layer.circle(x, y, rad, Some((c, 0.85 * alpha)), Some((ctx.bg, 1.0)));
        if ctx.ix.on() {
            let t = tip(ds, r, &[], ctx);
            layer.hit(ctx.ix.hit(
                HitShape::Circle {
                    cx: x,
                    cy: y,
                    r: rad.max(4.0),
                },
                ctx.ix.select(&pt),
                t,
            ));
        }
    }
    cv.draw(&layer, 0.0, top, 1.0);
    offline_mark(cv, ctx, Bx { w: b.w, h: top + h }, off);
    if !placed {
        return drawn(Some(unplaced_note(unplaced).unwrap_or_else(|| {
            "no location with a valid latitude and longitude".into()
        })));
    }
    let mut note = if filled {
        "filled map drawn offline: the countries its locations name, filled on Natural Earth outlines, no map \
         tiles"
            .to_string()
    } else if kind == "mapFilled" {
        "filled map drawn offline: its locations are not countries, so they are drawn as bubbles on Natural \
         Earth outlines, no map tiles"
            .to_string()
    } else if latlon.is_some() {
        "drawn offline: bubbles at their latitude and longitude on the Natural Earth basemap, no map tiles"
            .to_string()
    } else {
        "drawn offline: bubbles at the countries and cities their locations name, on the Natural Earth \
         basemap, no map tiles"
            .to_string()
    };
    if let Some(u) = unplaced_note(unplaced) {
        note = format!("{note}; {u}");
    }
    drawn(Some(note))
}

// ---------------------------------------------------------------------------------------------
// Shape map
// ---------------------------------------------------------------------------------------------

/// A projection of longitude and latitude in degrees (`None`: the far side of a globe).
fn project(kind: &str, lon: f64, lat: f64, center: (f64, f64)) -> Option<(f64, f64)> {
    let (l, p) = (lon.to_radians(), lat.to_radians());
    match kind {
        "equirectangular" => Some((l, p)),
        "orthographic" => {
            let (l0, p0) = (center.0.to_radians(), center.1.to_radians());
            let cosc = p0.sin() * p.sin() + p0.cos() * p.cos() * (l - l0).cos();
            (cosc >= 0.0).then(|| {
                (
                    p.cos() * (l - l0).sin(),
                    p0.cos() * p.sin() - p0.sin() * p.cos() * (l - l0).cos(),
                )
            })
        }
        "albersusa" => {
            // Albers equal-area conic on the lower 48 states' parallels (no Alaska and Hawaii insets)
            let (p1, p2, p0, l0) = (
                29.5f64.to_radians(),
                45.5f64.to_radians(),
                37.5f64.to_radians(),
                (-96.0f64).to_radians(),
            );
            let n = (p1.sin() + p2.sin()) / 2.0;
            let c = p1.cos().powi(2) + 2.0 * n * p1.sin();
            let rho = |q: f64| (c - 2.0 * n * q.sin()).max(0.0).sqrt() / n;
            let th = n * (l - l0);
            Some((rho(p) * th.sin(), rho(p0) - rho(p) * th.cos()))
        }
        _ => Some(mercator(lon, lat)),
    }
}

/// The rings of a snapshot shape (`[x, y, x, y, …]` lists).
fn rings_of(shape: &Value) -> Vec<Vec<(f64, f64)>> {
    arr(get(shape, "rings"))
        .iter()
        .take(10_000)
        .map(|r| {
            arr(r)
                .chunks(2)
                .filter_map(|c| Some((fin(c.first()?)?, fin(c.get(1)?)?)))
                .collect()
        })
        .collect()
}

/// A shape map's areas, coloured by its data.
pub(crate) fn shape_map(cv: &mut Canvas, v: &Value, ds: &Dataset, b: Bx, f: &Fmt, ctx: &Ctx<'_>) -> MapOut {
    let sm = get(v, "shape_map");
    let title = if fr(ctx) { "Carte de formes" } else { "Shape map" }.to_string();
    let shapes = arr(get(sm, "shapes"));
    if shapes.is_empty() {
        let err = sk(sm, "error");
        let (line, note) = match sk(sm, "source") {
            "custom" => (
                if fr(ctx) {
                    format!("La carte personnalisée est illisible : {}", clip(err, 80))
                } else {
                    format!("The custom map cannot be read: {}", clip(err, 80))
                },
                format!("the custom map cannot be read: {}", clip(err, 300)),
            ),
            "url" => (
                if fr(ctx) {
                    "Les cartes chargées depuis une URL ne sont pas récupérées hors ligne".to_string()
                } else {
                    "Maps from a URL are not fetched offline".to_string()
                },
                "the map comes from a URL: not fetched".to_string(),
            ),
            _ => (
                if fr(ctx) {
                    "Choisissez Pays du monde (Natural Earth) ou ajoutez une carte personnalisée".to_string()
                } else {
                    "Choose World countries (Natural Earth) or add a custom map".to_string()
                },
                "Desktop's built-in shape maps are Microsoft's map data, not part of BIcraft: choose World \
                 countries (Natural Earth) or add a custom TopoJSON or GeoJSON map (shape_map.set_map)"
                    .to_string(),
            ),
        };
        return placeholder(cv, b, ctx, &[title, line], note);
    }
    let cols = &ds.cols;
    let ci = cols.iter().position(|c| c.rc == Rc::Cat);
    let li = col_at(cols, "Series");
    // the rows by location key
    let mut by_key: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    if let Some(ci) = ci {
        for (i, r) in rows(ds).iter().enumerate() {
            if let Some(x) = r.get(ci) {
                let k = match x {
                    Value::String(s) => s.trim().to_lowercase(),
                    Value::Null => continue,
                    other => other.to_string(),
                };
                by_key.entry(k).or_insert(i);
            }
        }
    }
    let (items, leg_idx) = legend(ds, li, ctx);
    // Color saturation alone does not shade the areas: Desktop 2.159.1231.0 fills every matched
    // area with the one location colour (docs/formats/maps.md, "Checked in Desktop")
    let (lt, lb) = if !items.is_empty() && f.legend_show != Some(false) {
        let t = li.and_then(|l| cols.get(l)).map_or("", |c| c.name.as_str());
        place_legend(cv, ctx, f, &items, t, b)
    } else {
        (0.0, 0.0)
    };
    let (top, h) = (lt, (b.h - lt - lb).max(1.0));
    // projection: degrees when every coordinate is a longitude and a latitude
    let all: Vec<Vec<Vec<(f64, f64)>>> = shapes.iter().take(MAX_MARKS).map(rings_of).collect();
    let geo = all
        .iter()
        .flatten()
        .flatten()
        .all(|(x, y)| (-180.0..=180.0).contains(x) && (-90.0..=90.0).contains(y));
    let (mut a0, mut b0, mut a1, mut b1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for &(x, y) in all.iter().flatten().flatten() {
        a0 = a0.min(x);
        a1 = a1.max(x);
        b0 = b0.min(y);
        b1 = b1.max(y);
    }
    let center = ((a0 + a1) / 2.0, (b0 + b1) / 2.0);
    let kind = sk(sm, "projection").to_ascii_lowercase();
    let proj = |x: f64, y: f64| {
        if geo {
            project(&kind, x, y, center)
        } else {
            Some((x, y))
        }
    };
    let projected: Vec<Vec<Vec<(f64, f64)>>> = all
        .iter()
        .map(|rs| {
            rs.iter()
                .map(|r| r.iter().filter_map(|&(x, y)| proj(x, y)).collect())
                .collect()
        })
        .collect();
    // each shape's row: the first of its keys that a location matches
    let shape_rows: Vec<Option<usize>> = shapes
        .iter()
        .take(MAX_MARKS)
        .map(|shape| {
            arr(get(shape, "keys"))
                .iter()
                .filter_map(Value::as_str)
                .find_map(|k| by_key.get(&k.trim().to_lowercase()).copied())
        })
        .collect();
    // Zoom › Auto zoom ("zoom in on shapes with available data", theme schema): the bounds of
    // the matched shapes, else the whole map
    let auto_zoom = get(sm, "auto_zoom") == &Value::Bool(true) && shape_rows.iter().any(Option::is_some);
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    let ring_bounds = |r: &Vec<(f64, f64)>| {
        r.iter().fold(
            (f64::MAX, f64::MAX, f64::MIN, f64::MIN),
            |(a, b, c, d), &(x, y)| (a.min(x), b.min(y), c.max(x), d.max(y)),
        )
    };
    for (rs, row) in projected.iter().zip(&shape_rows) {
        if auto_zoom {
            // a matched shape's widest ring: its main land (mainland France, not French Guiana)
            if row.is_none() {
                continue;
            }
            let widest = rs
                .iter()
                .filter(|r| !r.is_empty())
                .map(ring_bounds)
                .max_by(|a, b| {
                    let area = |r: &(f64, f64, f64, f64)| (r.2 - r.0) * (r.3 - r.1);
                    area(a).partial_cmp(&area(b)).unwrap_or(std::cmp::Ordering::Equal)
                });
            if let Some(r) = widest {
                (x0, y0, x1, y1) = (x0.min(r.0), y0.min(r.1), x1.max(r.2), y1.max(r.3));
            }
            continue;
        }
        for &(x, y) in rs.iter().flatten() {
            x0 = x0.min(x);
            x1 = x1.max(x);
            y0 = y0.min(y);
            y1 = y1.max(y);
        }
    }
    let margin = if auto_zoom {
        f64::from(b.w.min(h)) * 0.06
    } else {
        4.0f64
    };
    let k = ((f64::from(b.w) - 2.0 * margin) / (x1 - x0)).min((f64::from(h) - 2.0 * margin) / (y1 - y0));
    if !k.is_finite() || k <= 0.0 {
        return drawn(None);
    }
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    // the areas are drawn on a layer of the map's box, so that zoomed shapes stay inside it
    let Some(mut layer) = cv.layer(0.0, top, b.w, h) else {
        return drawn(None);
    };
    let to_px = |(x, y): (f64, f64)| {
        (
            (f64::from(b.w) / 2.0 + (x - cx) * k) as f32,
            (f64::from(h) / 2.0 - (y - cy) * k) as f32,
        )
    };
    let blank_fill = Rgb::from_value(get(sm, "default_color")).unwrap_or(ctx.track);
    let show_blank = get(sm, "show_blank") != &Value::Bool(false);
    let border = Rgb::from_value(get(sm, "border_color")).unwrap_or(ctx.bg);
    let border_w = fin(get(sm, "border_width")).map_or(1.0, |w| w as f32);
    let groups = [ci, li];
    let mut matched = 0usize;
    for ((shape, rings), row) in shapes.iter().zip(&projected).zip(&shape_rows) {
        let px_rings: Vec<Vec<(f32, f32)>> = rings
            .iter()
            .filter(|r| r.len() >= 3)
            .map(|r| r.iter().map(|&p| to_px(p)).collect())
            .collect();
        if px_rings.is_empty() {
            continue;
        }
        let (fill, alpha, pt) = match row.and_then(|i| rows(ds).get(i).map(|r| (i, r))) {
            Some((i, r)) => {
                matched += 1;
                let c = ctx.color(leg_idx.get(i).copied().unwrap_or(0));
                let pt = point(r, &groups);
                (c, ctx.ix.alpha(&pt), Some((pt, r)))
            }
            None => (blank_fill, 1.0, None),
        };
        if pt.is_none() && !show_blank {
            continue;
        }
        let Some(path) = rings_path(&px_rings) else {
            continue;
        };
        layer.fill_path(&path, fill, alpha);
        if border_w > 0.0 {
            layer.stroke(&path, border, 1.0, border_w, None);
        }
        if ctx.ix.on() {
            let (action, t) = match &pt {
                Some((pt, r)) => (ctx.ix.select(pt), tip(ds, r, &[], ctx)),
                None => {
                    let key = arr(get(shape, "keys"))
                        .first()
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let name = ci
                        .and_then(|c| cols.get(c))
                        .map_or_else(|| title.clone(), |c| c.name.clone());
                    (hits::HitAction::None, vec![(name, clip(&key, 200))])
                }
            };
            layer.hit(ctx.ix.hit(
                HitShape::Polygon {
                    rings: hit_rings(&px_rings),
                },
                action,
                t,
            ));
        }
    }
    cv.draw(&layer, 0.0, top, 1.0);
    let note = if ci.is_some() && !rows(ds).is_empty() && matched == 0 {
        Some("no location matches the custom map's keys (its shapes' id or property values)".into())
    } else if get(sm, "simplified") == &Value::Bool(true) {
        Some("the custom map's outlines are thinned to 40,000 points".into())
    } else {
        None
    };
    drawn(note)
}
