//! The bundled country outlines of the offline maps: Natural Earth's 1:110m Admin 0 countries
//! (public domain), quantized by `cargo xtask geo-bundle` into `geo/ne_110m_admin_0_countries.bin`
//! (format in xtask/src/geo_bundle.rs, provenance in geo/SOURCE.txt, docs/formats/maps.md).
//!
//! [`countries`] decodes the asset once; [`find`] matches a country by name (English, Natural
//! Earth's French name, the long and formal names) or ISO 3166-1 alpha-2 / alpha-3 code, without
//! case or accents. A damaged asset yields no countries, never a panic.
//!
//! [`basemap`] decodes the detailed layers of the open basemap, `geo/ne_50m_basemap.bin`
//! (Natural Earth 1:50m countries, lakes, admin-1 boundary lines and populated places, public
//! domain; `cargo xtask geo-bundle --basemap`, provenance in geo/SOURCE-50m.txt); [`find_city`]
//! places a city by name. A damaged asset yields empty layers.

use std::collections::HashMap;
use std::sync::OnceLock;

static ASSET: &[u8] = include_bytes!("../geo/ne_110m_admin_0_countries.bin");

/// One country of the bundled map.
#[derive(Debug, Clone, PartialEq)]
pub struct Country {
    /// ISO 3166-1 alpha-2 (`FR`), empty when Natural Earth has none.
    pub iso2: String,
    /// ISO 3166-1 alpha-3 (`FRA`), empty when Natural Earth has none.
    pub iso3: String,
    /// `NAME` (`France`).
    pub name: String,
    /// `NAME_LONG`, `ADMIN`, `FORMAL_EN` and `NAME_FR`, each possibly empty.
    pub name_long: String,
    pub admin: String,
    pub formal_en: String,
    pub name_fr: String,
    /// Natural Earth's label point (longitude, latitude), where a bubble of the country goes.
    pub label: (f64, f64),
    /// Polygons, each an outer ring then its holes; rings of (longitude, latitude), not closed.
    pub polygons: Vec<Vec<Vec<(f64, f64)>>>,
    /// The bounds (lon0, lat0, lon1, lat1) of each polygon.
    pub bounds: Vec<(f64, f64, f64, f64)>,
}

impl Country {
    /// The bounds of the polygon with the widest outer ring: the country's main land (mainland
    /// France without French Guiana, Russia without the islands beyond the antimeridian).
    pub fn main_bounds(&self) -> Option<(f64, f64, f64, f64)> {
        self.bounds.iter().copied().max_by(|a, b| {
            let area = |r: &(f64, f64, f64, f64)| (r.2 - r.0) * (r.3 - r.1);
            area(a).partial_cmp(&area(b)).unwrap_or(std::cmp::Ordering::Equal)
        })
    }

    /// The names and codes a location may hold.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        [
            &self.name,
            &self.name_long,
            &self.admin,
            &self.formal_en,
            &self.name_fr,
            &self.iso2,
            &self.iso3,
        ]
        .into_iter()
        .map(String::as_str)
        .filter(|s| !s.is_empty())
    }
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn bytes(&mut self, n: usize) -> Option<&[u8]> {
        let end = self.at.checked_add(n)?;
        let s = self.b.get(self.at..end)?;
        self.at = end;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        self.bytes(1)?.first().copied()
    }
    fn u16(&mut self) -> Option<usize> {
        let s = self.bytes(2)?;
        Some(usize::from(u16::from_le_bytes([*s.first()?, *s.get(1)?])))
    }
    fn i16(&mut self) -> Option<f64> {
        let s = self.bytes(2)?;
        Some(f64::from(i16::from_le_bytes([*s.first()?, *s.get(1)?])) / 100.0)
    }
    fn str(&mut self) -> Option<String> {
        let n = usize::from(self.u8()?);
        String::from_utf8(self.bytes(n)?.to_vec()).ok()
    }
    fn var(&mut self) -> Option<i32> {
        let mut z: u32 = 0;
        for shift in (0..35).step_by(7) {
            let b = self.u8()?;
            z |= u32::from(b & 0x7f).checked_shl(shift)?;
            if b & 0x80 == 0 {
                return Some(((z >> 1) as i32) ^ -((z & 1) as i32));
            }
        }
        None
    }
    fn u32(&mut self) -> Option<u32> {
        let s = self.bytes(4)?;
        Some(u32::from_le_bytes([
            *s.first()?,
            *s.get(1)?,
            *s.get(2)?,
            *s.get(3)?,
        ]))
    }
    /// A u16 count of points, then their deltas (1/100 °) from the previous one.
    fn points(&mut self) -> Option<Vec<(f64, f64)>> {
        let np = self.u16()?;
        let mut ring = Vec::with_capacity(np);
        let (mut x, mut y) = (0i32, 0i32);
        for _ in 0..np {
            x = x.checked_add(self.var()?)?;
            y = y.checked_add(self.var()?)?;
            ring.push((f64::from(x) / 100.0, f64::from(y) / 100.0));
        }
        Some(ring)
    }
    /// A u16 ring count, then the rings' points.
    fn polygon(&mut self) -> Option<Vec<Vec<(f64, f64)>>> {
        let n = self.u16()?;
        let mut rings = Vec::with_capacity(n.min(1000));
        for _ in 0..n {
            rings.push(self.points()?);
        }
        Some(rings)
    }
}

/// The bounds (lon0, lat0, lon1, lat1) of some points.
fn bounds_of(pts: &[(f64, f64)]) -> (f64, f64, f64, f64) {
    let (mut a0, mut b0, mut a1, mut b1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for &(x, y) in pts {
        a0 = a0.min(x);
        a1 = a1.max(x);
        b0 = b0.min(y);
        b1 = b1.max(y);
    }
    (a0, b0, a1, b1)
}

fn decode(b: &[u8]) -> Option<Vec<Country>> {
    let mut r = Reader { b, at: 0 };
    if r.bytes(8)? != b"BCGEO\x01\0\0" {
        return None;
    }
    let n = r.u16()?;
    let mut out = Vec::with_capacity(n.min(1000));
    for _ in 0..n {
        let iso2 = r.str()?;
        let iso3 = r.str()?;
        let mut names: Vec<String> = Vec::new();
        for _ in 0..r.u8()? {
            names.push(r.str()?);
        }
        let mut name = names.into_iter();
        let mut next = || name.next().unwrap_or_default();
        let (nm, long, admin, formal, fr) = (next(), next(), next(), next(), next());
        let label = (r.i16()?, r.i16()?);
        let mut polygons = Vec::new();
        let mut bounds = Vec::new();
        for _ in 0..r.u16()? {
            let rings = r.polygon()?;
            bounds.push(bounds_of(rings.first().map_or(&[], Vec::as_slice)));
            polygons.push(rings);
        }
        out.push(Country {
            iso2,
            iso3,
            name: nm,
            name_long: long,
            admin,
            formal_en: formal,
            name_fr: fr,
            label,
            polygons,
            bounds,
        });
    }
    Some(out)
}

/// The bundled countries (empty if the asset cannot be read).
pub fn countries() -> &'static [Country] {
    static C: OnceLock<Vec<Country>> = OnceLock::new();
    C.get_or_init(|| decode(ASSET).unwrap_or_default())
}

/// A location's text as it is matched: trimmed, lower case, Latin accents dropped, runs of
/// spaces as one.
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.trim().chars().flat_map(char::to_lowercase) {
        let f = match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => "a",
            'ç' => "c",
            'è' | 'é' | 'ê' | 'ë' => "e",
            'ì' | 'í' | 'î' | 'ï' => "i",
            'ñ' => "n",
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => "o",
            'ù' | 'ú' | 'û' | 'ü' => "u",
            'ý' | 'ÿ' => "y",
            'œ' => "oe",
            'æ' => "ae",
            '’' => "'",
            c if c.is_whitespace() => {
                if !out.ends_with(' ') {
                    out.push(' ');
                }
                continue;
            }
            c => {
                out.push(c);
                continue;
            }
        };
        out.push_str(f);
    }
    out
}

/// The index of the country named or coded `key` in [`countries`].
pub fn find(key: &str) -> Option<usize> {
    static IDX: OnceLock<HashMap<String, usize>> = OnceLock::new();
    let idx = IDX.get_or_init(|| {
        let mut m = HashMap::new();
        for (i, c) in countries().iter().enumerate() {
            for k in c.keys() {
                m.entry(fold(k)).or_insert(i);
            }
        }
        m
    });
    let k = fold(key);
    if k.is_empty() {
        return None;
    }
    idx.get(&k).copied()
}

// ---------------------------------------------------------------------------------------------
// The detailed layers of the open basemap (1:50m)
// ---------------------------------------------------------------------------------------------

static BASEMAP: &[u8] = include_bytes!("../geo/ne_50m_basemap.bin");

/// An area of the detailed layers: a 1:50m country (its ISO alpha-3 code and name) or a lake.
#[derive(Debug, Clone, PartialEq)]
pub struct Area {
    pub iso3: String,
    pub admin: String,
    /// Polygons, each an outer ring then its holes (longitude, latitude), not closed.
    pub polygons: Vec<Vec<Vec<(f64, f64)>>>,
    /// The bounds (lon0, lat0, lon1, lat1) of each polygon.
    pub bounds: Vec<(f64, f64, f64, f64)>,
}

/// A populated place of Natural Earth's 1:50m list.
#[derive(Debug, Clone, PartialEq)]
pub struct City {
    pub name: String,
    /// The ASCII spelling when it differs from `name`, else empty.
    pub ascii: String,
    /// The country's code (`adm0_a3`, e.g. `FRA`).
    pub adm0: String,
    /// The state, province or region (`adm1name`).
    pub adm1: String,
    pub lon: f64,
    pub lat: f64,
    /// Natural Earth's `scalerank` (0: shown at the smallest scales).
    pub rank: u8,
    /// A national capital.
    pub capital: bool,
    pub pop: u32,
}

/// The 1:50m layers: countries, lakes, admin-1 boundary lines (the large countries Natural Earth
/// splits at that scale) and populated places.
#[derive(Debug, Default)]
pub struct Basemap {
    pub countries: Vec<Area>,
    pub lakes: Vec<Area>,
    /// Lines of (longitude, latitude) with their bounds.
    pub lines: Vec<(Vec<(f64, f64)>, (f64, f64, f64, f64))>,
    pub cities: Vec<City>,
}

fn area(r: &mut Reader<'_>, iso3: String, admin: String, polys: usize) -> Option<Area> {
    let mut polygons = Vec::with_capacity(polys.min(1000));
    let mut bounds = Vec::with_capacity(polys.min(1000));
    for _ in 0..polys {
        let rings = r.polygon()?;
        bounds.push(bounds_of(rings.first().map_or(&[], Vec::as_slice)));
        polygons.push(rings);
    }
    Some(Area {
        iso3,
        admin,
        polygons,
        bounds,
    })
}

fn decode_basemap(b: &[u8]) -> Option<Basemap> {
    let mut r = Reader { b, at: 0 };
    if r.bytes(8)? != b"BCMAP\x01\0\0" {
        return None;
    }
    let mut m = Basemap::default();
    for _ in 0..r.u16()? {
        let (iso3, admin) = (r.str()?, r.str()?);
        let n = r.u16()?;
        m.countries.push(area(&mut r, iso3, admin, n)?);
    }
    for _ in 0..r.u16()? {
        m.lakes.push(area(&mut r, String::new(), String::new(), 1)?);
    }
    for _ in 0..r.u16()? {
        let pts = r.points()?;
        let b = bounds_of(&pts);
        m.lines.push((pts, b));
    }
    for _ in 0..r.u16()? {
        let (name, ascii, adm0, adm1) = (r.str()?, r.str()?, r.str()?, r.str()?);
        let (lon, lat) = (r.i16()?, r.i16()?);
        let rank = r.u8()?;
        let capital = r.u8()? & 1 == 1;
        let pop = r.u32()?;
        m.cities.push(City {
            name,
            ascii,
            adm0,
            adm1,
            lon,
            lat,
            rank,
            capital,
            pop,
        });
    }
    Some(m)
}

/// The bundled 1:50m layers (empty if the asset cannot be read).
pub fn basemap() -> &'static Basemap {
    static M: OnceLock<Basemap> = OnceLock::new();
    M.get_or_init(|| decode_basemap(BASEMAP).unwrap_or_default())
}

/// The [`countries`] index of each 1:50m country of [`basemap`] (by ISO alpha-3 code, else by
/// name), so a filled country is filled at both scales.
pub fn area_country(i: usize) -> Option<usize> {
    static IDX: OnceLock<Vec<Option<usize>>> = OnceLock::new();
    IDX.get_or_init(|| {
        basemap()
            .countries
            .iter()
            .map(|a| find(&a.iso3).or_else(|| find(&a.admin)))
            .collect()
    })
    .get(i)
    .copied()
    .flatten()
}

/// The index in [`basemap`]'s cities of the place `key` names: a city's name or ASCII name,
/// optionally followed by `, ` and its country (name or ISO code) or its state or province
/// (`Paris, France`, `Portland, Oregon`). Among cities of one name the most populous wins.
pub fn find_city(key: &str) -> Option<usize> {
    static IDX: OnceLock<HashMap<String, Vec<usize>>> = OnceLock::new();
    let idx = IDX.get_or_init(|| {
        let cities = &basemap().cities;
        let mut m: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, c) in cities.iter().enumerate() {
            for k in [&c.name, &c.ascii] {
                let k = fold(k);
                if !k.is_empty() {
                    let v = m.entry(k).or_default();
                    if !v.contains(&i) {
                        v.push(i);
                    }
                }
            }
        }
        for v in m.values_mut() {
            v.sort_by_key(|&i| std::cmp::Reverse(cities.get(i).map_or(0, |c| c.pop)));
        }
        m
    });
    let k = fold(key);
    if k.is_empty() {
        return None;
    }
    if let Some(v) = idx.get(&k) {
        return v.first().copied();
    }
    // "city, country" or "city, state"
    let (city, qual) = k.split_once(',')?;
    let (city, qual) = (city.trim(), qual.trim());
    let iso3 = find(qual)
        .and_then(|c| countries().get(c))
        .map(|c| c.iso3.as_str());
    idx.get(city)?.iter().copied().find(|&i| {
        basemap()
            .cities
            .get(i)
            .is_some_and(|c| fold(&c.adm1) == qual || iso3.is_some_and(|s| !s.is_empty() && s == c.adm0))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_asset_decodes_with_its_countries_and_codes() {
        let c = countries();
        assert_eq!(c.len(), 177);
        let fr = &c[find("France").unwrap()];
        assert_eq!((fr.iso2.as_str(), fr.iso3.as_str()), ("FR", "FRA"));
        assert_eq!(find("fr"), find("FRA"));
        assert_eq!(
            find("  république   française ").or(find("French Republic")),
            find("France")
        );
        assert_eq!(find("Allemagne"), find("Germany"));
        assert_eq!(find("etats-unis"), find("United States of America"));
        assert_eq!(find("United States"), find("USA"));
        assert_eq!(find("Paris"), None);
        assert_eq!(find(""), None);
        // mainland France, not French Guiana
        let (a0, b0, a1, b1) = fr.main_bounds().unwrap();
        assert!(
            a0 > -6.0 && a1 < 10.0 && b0 > 41.0 && b1 < 52.0,
            "{a0} {b0} {a1} {b1}"
        );
        assert!(fr.label.0 > 0.0 && fr.label.0 < 5.0 && fr.label.1 > 45.0 && fr.label.1 < 48.0);
    }

    #[test]
    fn the_basemap_decodes_its_layers_and_places_cities() {
        let m = basemap();
        assert_eq!(
            (m.countries.len(), m.lakes.len(), m.lines.len(), m.cities.len()),
            (242, 411, 591, 1251)
        );
        // every 110m country with a code has its 1:50m outline
        let fr = find("France").unwrap();
        assert!((0..m.countries.len()).any(|i| area_country(i) == Some(fr)));
        let city = |k: &str| find_city(k).map(|i| &m.cities[i]);
        let paris = city("Paris").unwrap();
        assert_eq!(paris.adm0, "FRA");
        assert!((paris.lon - 2.35).abs() < 0.1 && (paris.lat - 48.86).abs() < 0.1);
        assert_eq!(city("  sao   paulo").unwrap().adm0, "BRA");
        assert_eq!(city("São Paulo"), city("sao paulo"));
        // the most populous of one name, unless the country or state says otherwise
        assert_eq!(city("Portland").unwrap().adm0, "USA");
        assert_eq!(city("Portland, Victoria").unwrap().adm0, "AUS");
        assert_eq!(city("Portland, Australia").unwrap().adm0, "AUS");
        assert_eq!(city("Córdoba, Espagne").unwrap().adm0, "ESP");
        assert_eq!(city("Córdoba").unwrap().adm0, "ARG");
        assert_eq!(city("Paris, Japan"), None);
        assert_eq!(city("North"), None);
        assert_eq!(city(""), None);
    }

    #[test]
    fn a_damaged_basemap_yields_nothing() {
        assert!(decode_basemap(b"").is_none());
        assert!(decode_basemap(b"BCMAP\x01\0\0\xff\xff").is_none());
        for n in [9, 100, 10_000, BASEMAP.len() - 1] {
            assert!(decode_basemap(&BASEMAP[..n]).is_none(), "{n}");
        }
        let mut bad = BASEMAP.to_vec();
        for b in bad.iter_mut().skip(10).step_by(11) {
            *b = 0xff;
        }
        let _ = decode_basemap(&bad);
    }

    #[test]
    fn a_damaged_asset_yields_nothing() {
        assert_eq!(decode(b""), None);
        assert_eq!(decode(b"BCGEO\x01\0\0\x05\0"), None);
        for n in [9, 100, 1000, ASSET.len() - 1] {
            assert_eq!(decode(&ASSET[..n]), None, "{n}");
        }
        let mut bad = ASSET.to_vec();
        for b in bad.iter_mut().skip(10).step_by(7) {
            *b = 0xff;
        }
        let _ = decode(&bad);
    }
}
