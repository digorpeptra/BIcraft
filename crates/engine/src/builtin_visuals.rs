//! BIcraft's own visuals, built in by default (ROADMAP M12.4, docs/community-visuals.md "Built in
//! by default"): the registry of the custom visuals whose TypeScript BIcraft ships and packages
//! itself (`bicraft_tsbuild`, `crate::visual_project`). The Visualizations pane lists them next to
//! the built-in types (the snapshot's `report.bicraft_visuals`); `visual.add` and `visual.set_type`
//! take their name or guid and embed the package on first use ([`ensure`]: missing or older
//! packages are replaced, a newer one is kept); `report.custom_visuals` reports them
//! ([`report_json`]); a save drops the package of one the edited report no longer uses
//! ([`drop_unused`]). A new visual adds one [`BuiltinVisual`] to [`VISUALS`].

use std::cmp::Ordering;
use std::sync::OnceLock;

use bicraft_report::{Report, Visual};
use bicraft_visuals::{DataRole, RoleKind, VisualType};
use serde_json::{Value, json};

use crate::Session;
use crate::visual_project::Project;

/// A built visual: its `.pbiviz` bytes and the body of its `resources/<guid>.pbiviz.json`.
pub(crate) type Built = (Vec<u8>, Vec<u8>);

/// One of BIcraft's visuals.
pub(crate) struct BuiltinVisual {
    /// Its name (`pbiviz.json` `visual.name`): what `visual.add {visual_type}` accepts besides the
    /// guid.
    pub name: &'static str,
    /// Its guid: the visual type its visuals carry and its package's name in the report.
    pub guid: &'static str,
    /// Its name in the Visualizations pane.
    pub display_name: &'static str,
    /// Its package version (`a.b.c.d`), raised with every change of its TypeScript.
    pub version: &'static str,
    /// Its data roles as its `capabilities.json` declares them (name, kind, most fields), given
    /// its guid.
    pub roles_of: fn(&str) -> Vec<(&'static str, RoleKind, usize)>,
    /// Its package, built once per process, given its guid.
    pub package: fn(&str) -> Result<&'static Built, String>,
    /// Its 20×20 PNG icon (its package's `assets/icon.png`, the gallery's cell), given its guid:
    /// [`icon_png`], its drawing in `bicraft_visuals::icons`.
    pub icon: fn(&str) -> Result<Vec<u8>, String>,
}

/// The side of a package icon: Power BI shows a visual's `assets/icon.png` at 20 × 20.
pub(crate) const ICON_SIDE: u32 = 20;

/// The 20×20 PNG icon of BIcraft's visual with guid `guid` (ignoring case): its drawing in
/// `bicraft_visuals::icons`, rasterized once per process. The Visualizations pane paints the same
/// drawing in the theme's ink; the package carries this PNG.
pub(crate) fn icon_png(guid: &str) -> Result<Vec<u8>, String> {
    static ICONS: [OnceLock<Result<Vec<u8>, String>>; 16] = [const { OnceLock::new() }; 16];
    let (i, b) = VISUALS
        .iter()
        .enumerate()
        .find(|(_, b)| b.guid.eq_ignore_ascii_case(guid))
        .ok_or("internal: not one of BIcraft's visuals")?;
    let draw = || {
        let marks = bicraft_visuals::icons::bicraft_visual_icon(b.name)
            .ok_or_else(|| format!("internal: {} has no icon drawing", b.name))?;
        crate::visual_project::marks_png(&marks, ICON_SIDE)
    };
    match ICONS.get(i) {
        Some(cell) => cell.get_or_init(draw).clone(),
        None => draw(),
    }
}

/// Every BIcraft visual, in the order the Visualizations pane lists them: the Spec visual, the
/// first pack's part A (`crate::bvis::DEFS`, through [`chart`]) and part B
/// (`crate::chart_pack::VISUALS`, through [`pack`]). A new visual adds one line.
pub(crate) const VISUALS: &[BuiltinVisual] = &[
    crate::spec_visual::SPEC,
    chart(crate::bvis::DEFS[0]),
    chart(crate::bvis::DEFS[1]),
    chart(crate::bvis::DEFS[2]),
    chart(crate::bvis::DEFS[3]),
    pack(&crate::chart_pack::VISUALS[0]),
    pack(&crate::chart_pack::VISUALS[1]),
    pack(&crate::chart_pack::VISUALS[2]),
    pack(&crate::chart_pack::VISUALS[3]),
    pack(&crate::chart_pack::VISUALS[4]),
    pack(&crate::chart_pack::VISUALS[5]),
    pack(&crate::chart_pack::VISUALS[6]),
    pack(&crate::chart_pack::VISUALS[7]),
    pack(&crate::chart_pack::VISUALS[8]),
];

/// The registry entry of a chart visual of the first pack's part A.
const fn chart(d: &'static crate::bvis::Def) -> BuiltinVisual {
    BuiltinVisual {
        name: d.name,
        guid: d.guid,
        display_name: d.display_name,
        version: d.version,
        roles_of: crate::bvis::registry_roles,
        package: crate::bvis::registry_package,
        icon: icon_png,
    }
}

/// The registry entry of a chart pack visual.
const fn pack(d: &'static crate::chart_pack::Def) -> BuiltinVisual {
    BuiltinVisual {
        name: d.name,
        guid: d.guid,
        display_name: d.display_name,
        version: d.version,
        roles_of: crate::chart_pack::registry_roles,
        package: crate::chart_pack::registry_package,
        icon: icon_png,
    }
}

/// The BIcraft visual named `name` or with guid `name` (ignoring case).
pub(crate) fn find(name: &str) -> Option<&'static BuiltinVisual> {
    VISUALS
        .iter()
        .find(|b| b.name.eq_ignore_ascii_case(name) || b.guid.eq_ignore_ascii_case(name))
}

/// Whether `v` is one of BIcraft's visuals that another visual's selection highlights rather
/// than filters (docs/formats/bicraft-visuals.md "Highlights"): its package declares
/// `supportsHighlight` (it has a measure role). With no field in its measure roles it gets no
/// highlights and stays as it is, as Power BI Desktop 2.159.1231.0 leaves it (2026-11-24).
pub(crate) fn highlights(v: &Visual) -> bool {
    let Some(b) = v.visual_type.as_deref().and_then(find) else {
        return false;
    };
    b.roles().into_iter().any(|(_, k, _)| k == RoleKind::Measure)
}

impl BuiltinVisual {
    /// Its data roles: name, kind, most fields.
    pub(crate) fn roles(&self) -> Vec<(&'static str, RoleKind, usize)> {
        (self.roles_of)(self.guid)
    }

    /// The names of its data roles.
    pub(crate) fn role_names(&self) -> Vec<&'static str> {
        self.roles().into_iter().map(|(n, _, _)| n).collect()
    }

    /// The role named `role` (ignoring case), spelled as the visual declares it.
    pub(crate) fn role(&self, role: &str) -> Option<&'static str> {
        self.role_names()
            .into_iter()
            .find(|n| n.eq_ignore_ascii_case(role))
    }

    /// The visual as a catalog type with known data roles (what `visual.set_type` moves fields
    /// into).
    pub(crate) fn visual_type(&self) -> VisualType {
        let roles = self.roles();
        VisualType {
            name: self.guid.into(),
            display_name: Some(self.display_name.into()),
            roles: roles.iter().map(|(n, _, _)| (*n).to_string()).collect(),
            objects: Vec::new(),
            roles_known: true,
            data_roles: roles
                .iter()
                .map(|(n, kind, max)| DataRole {
                    name: (*n).into(),
                    kind: *kind,
                    min: 0,
                    max: u32::try_from(*max).ok(),
                    drill: false,
                })
                .collect(),
            role_conditions: Vec::new(),
            required_roles: Vec::new(),
        }
    }

    /// Path of its `.pbiviz.json` in the report.
    fn metadata_path(&self) -> String {
        format!("CustomVisuals/{0}/resources/{0}.pbiviz.json", self.guid)
    }
}

/// Builds a visual's package from its project (the body of [`BuiltinVisual::package`], cached in
/// `cell`).
pub(crate) fn build_once(
    cell: &'static OnceLock<Result<Built, String>>,
    display_name: &str,
    project: fn() -> Result<Project, String>,
) -> Result<&'static Built, String> {
    cell.get_or_init(|| {
        let p = project()?;
        let out = crate::visual_project::build(&p)?;
        let files = crate::visual_project::package_files(&p, &out.js)?;
        let body = files
            .iter()
            .find(|(n, _)| n.ends_with(".pbiviz.json"))
            .map(|(_, b)| b.clone())
            .ok_or("internal: the package has no .pbiviz.json")?;
        Ok((crate::visual_project::package(&p, &out.js)?, body))
    })
    .as_ref()
    .map_err(|e| format!("cannot build the {display_name} visual: {e}"))
}

/// Whether the report registers `b`'s package with its metadata item named as its file (an item
/// named by the guid alone, as BIcraft wrote it before 2026-11-23, is lost by Desktop's save).
fn registered(r: &Report, b: &BuiltinVisual) -> bool {
    let item = format!("{}.pbiviz.json", b.guid);
    r.extra
        .get("resourcePackages")
        .and_then(Value::as_array)
        .is_some_and(|l| {
            l.iter().any(|p| {
                crate::custom_visuals::is_package_of(p, b.guid)
                    && p.get("items").and_then(Value::as_array).is_some_and(|i| {
                        i.iter()
                            .any(|x| x.get("name").and_then(Value::as_str) == Some(&item))
                    })
            })
        })
}

/// The version of `b` the report embeds (its `.pbiviz.json`), when it reads.
fn embedded_version(r: &Report, b: &BuiltinVisual) -> Option<String> {
    let want = b.metadata_path();
    let res = r
        .resources
        .iter()
        .find(|x| x.path.replace('\\', "/").eq_ignore_ascii_case(&want))?;
    if res.bytes.len() > 64 << 20 {
        return None;
    }
    let body = res
        .bytes
        .strip_prefix(&[0xEF, 0xBB, 0xBF][..])
        .unwrap_or(&res.bytes);
    let doc: Value = serde_json::from_slice(body).ok()?;
    doc.pointer("/visual/version")
        .and_then(Value::as_str)
        .map(|v| v.chars().take(40).collect())
}

/// `a.b.c.d` versions compared part by part; `None` when one does not read.
fn compare(a: &str, b: &str) -> Option<Ordering> {
    let parts = |v: &str| {
        v.trim()
            .split('.')
            .take(8)
            .map(|x| x.parse::<u64>().ok())
            .collect::<Option<Vec<u64>>>()
    };
    let (mut a, mut b) = (parts(a)?, parts(b)?);
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    Some(a.cmp(&b))
}

/// Embeds `b`'s package in the session's report unless it holds this build or a newer version;
/// returns whether it did. Refused (before any change) on a legacy report, as
/// `visual.import_custom` is.
pub(crate) fn ensure(s: &mut Session, b: &BuiltinVisual) -> Result<bool, String> {
    let (pkg, body) = (b.package)(b.guid)?;
    {
        let r = crate::report::report_ref(s)?;
        if registered(r, b) {
            let same = r
                .resources
                .iter()
                .any(|x| x.path.replace('\\', "/") == b.metadata_path() && x.bytes == *body);
            let newer = embedded_version(r, b)
                .and_then(|v| compare(&v, b.version))
                .is_some_and(|o| o == Ordering::Greater);
            if same || newer {
                return Ok(false);
            }
        }
    }
    crate::custom_visuals::visual_import_custom(s, &json!({"data": crate::render::base64(pkg)}))?;
    Ok(true)
}

/// Drops the package of every BIcraft visual the session's edited report carries but no visual
/// uses (before a save writes it); returns their guids. A report without unsaved edits, or a
/// legacy one, is left as it is.
pub(crate) fn drop_unused(s: &mut Session) -> Vec<String> {
    if !s.report_dirty || crate::convert::source_is_legacy(s) {
        return Vec::new();
    }
    let Some(r) = s.report.as_ref() else {
        return Vec::new();
    };
    let unused: Vec<&'static str> = VISUALS
        .iter()
        .filter(|b| {
            let carried = r
                .extra
                .get("resourcePackages")
                .and_then(Value::as_array)
                .is_some_and(|l| l.iter().any(|p| crate::custom_visuals::is_package_of(p, b.guid)))
                || r.resources
                    .iter()
                    .any(|x| crate::custom_visuals::in_folder(&x.path, b.guid));
            carried && crate::custom_visuals::used_by(r, b.guid).is_empty()
        })
        .map(|b| b.guid)
        .collect();
    unused
        .into_iter()
        .filter(|guid| crate::custom_visuals::visual_remove_custom(s, &json!({"name": guid})).is_ok())
        .map(str::to_string)
        .collect()
}

/// `report.custom_visuals`' `bicraft` list: every BIcraft visual, whether the report embeds it
/// and which of its visuals use it.
pub(crate) fn report_json(r: &Report) -> Value {
    Value::Array(
        VISUALS
            .iter()
            .map(|b| {
                let embedded = registered(r, b);
                let version = embedded.then(|| embedded_version(r, b)).flatten();
                json!({
                    "name": b.name,
                    "guid": b.guid,
                    "display_name": b.display_name,
                    "version": b.version,
                    "embedded": embedded,
                    "embedded_version": version,
                    "current": version.as_deref() == Some(b.version),
                    "roles": b.role_names(),
                    "used_by": crate::custom_visuals::used_by(r, b.guid),
                })
            })
            .collect(),
    )
}

/// The Visualizations pane's cells of BIcraft's visuals, each with whether the report embeds it.
pub(crate) fn gallery(r: &Report) -> Vec<(&'static BuiltinVisual, bool)> {
    VISUALS.iter().map(|b| (b, registered(r, b))).collect()
}

/// The choices of `visual.export_bicraft_visual`'s form: `all`, then [`VISUALS`]' names in order
/// (a test keeps them in step).
pub(crate) const EXPORT_NAMES: &[&str] = &[
    "all",
    "bicraftSpec",
    "bicraftHistogram",
    "bicraftBullet",
    "bicraftBoxPlot",
    "bicraftDumbbell",
    "bicraftOrgChart",
    "bicraftVenn",
    "bicraftMarimekko",
    "bicraftCalendarHeatmap",
    "bicraftSankey",
    "bicraftScatterPlus",
    "bicraftLineAreaPlus",
    "bicraftWaterfallPlus",
    "bicraftGaugeKpi",
];

/// Most names `visual.export_bicraft_visual` takes in one list.
const MAX_EXPORT_NAMES: usize = 64;

/// The visuals `name` picks: `"all"`, one name or guid, or a list of them (duplicates once).
fn export_picks(name: Option<&Value>) -> Result<(Vec<&'static BuiltinVisual>, bool), String> {
    let known = || VISUALS.iter().map(|b| b.name).collect::<Vec<_>>().join(", ");
    let one = |n: &str| {
        find(n.trim()).ok_or_else(|| {
            format!(
                "`{}` is not one of BIcraft's visuals ({}, or all)",
                n.chars().take(80).collect::<String>(),
                known()
            )
        })
    };
    match name {
        Some(Value::String(n)) if n.trim().eq_ignore_ascii_case("all") => {
            Ok((VISUALS.iter().collect(), true))
        }
        Some(Value::String(n)) => Ok((vec![one(n)?], false)),
        Some(Value::Array(l)) if l.is_empty() => Err("`name` is an empty list".into()),
        Some(Value::Array(l)) if l.len() > MAX_EXPORT_NAMES => Err(format!(
            "`name` lists {} visuals; at most {MAX_EXPORT_NAMES}",
            l.len()
        )),
        Some(Value::Array(l)) => {
            let mut picks: Vec<&'static BuiltinVisual> = Vec::new();
            for v in l {
                let n = v.as_str().ok_or("`name` lists names (strings)")?;
                if n.trim().eq_ignore_ascii_case("all") {
                    return Ok((VISUALS.iter().collect(), true));
                }
                let b = one(n)?;
                if !picks.iter().any(|p| p.guid == b.guid) {
                    picks.push(b);
                }
            }
            Ok((picks, true))
        }
        Some(_) => Err("`name` is a visual's name, a list of names or \"all\"".into()),
        None => Err(format!(
            "visual.export_bicraft_visual needs `name` ({}, or all)",
            known()
        )),
    }
}

/// `visual.export_bicraft_visual {name, out}`: writes the `.pbiviz` of one of BIcraft's visuals
/// (or several, into a folder) for Desktop's Import a visual from a file and for organizational
/// visuals. The session is not read or changed.
pub(crate) fn visual_export_bicraft_visual(_s: &mut Session, p: &Value) -> Result<Value, String> {
    crate::params::allow(p, &["name", "out"])?;
    let (picks, folder_mode) = export_picks(p.get("name"))?;
    let out = crate::params::str_param(p, "out")?.trim();
    if out.is_empty() {
        return Err("`out` is empty: give a .pbiviz file or a folder".into());
    }
    let out = std::path::PathBuf::from(out);
    let names_a_file = out.extension().is_some_and(|e| e.eq_ignore_ascii_case("pbiviz"));
    if folder_mode && names_a_file {
        return Err(format!(
            "{} names a .pbiviz file; several visuals go into a folder",
            out.display()
        ));
    }
    let target = |b: &BuiltinVisual| {
        if names_a_file {
            out.clone()
        } else {
            out.join(format!("{}.{}.pbiviz", b.guid, b.version))
        }
    };
    if !names_a_file && std::fs::symlink_metadata(&out).is_ok_and(|m| !m.is_dir()) {
        return Err(format!("{} is a file, not a folder", out.display()));
    }
    // every package is built before anything is written
    let mut built = Vec::with_capacity(picks.len());
    for b in &picks {
        built.push((*b, &(b.package)(b.guid)?.0));
    }
    let mut files = Vec::with_capacity(built.len());
    for (b, bytes) in built {
        let path = target(b);
        crate::visual_project::write_out(&path, bytes)?;
        files.push(json!({
            "name": b.name,
            "guid": b.guid,
            "display_name": b.display_name,
            "version": b.version,
            "path": path.display().to_string(),
            "bytes": bytes.len(),
        }));
    }
    Ok(json!({
        "out": out.display().to_string(),
        "count": files.len(),
        "files": files,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_guids_are_unique_and_valid_visual_types() {
        for (i, b) in VISUALS.iter().enumerate() {
            for n in [b.name, b.guid] {
                assert!(n.chars().all(|c| c.is_ascii_alphanumeric()), "{n}");
                assert!(
                    !crate::report::KNOWN_VISUAL_TYPES
                        .iter()
                        .any(|t| t.eq_ignore_ascii_case(n))
                );
            }
            for o in VISUALS.iter().skip(i + 1) {
                assert!(find(o.name).is_some_and(|f| f.guid == o.guid), "{}", o.name);
                assert!(!o.guid.eq_ignore_ascii_case(b.guid));
            }
            assert!(
                compare(b.version, "0.0.0.0") == Some(Ordering::Greater),
                "{}",
                b.version
            );
            let read = bicraft_pbix::pbiviz::read_pbiviz(&(b.package)(b.guid).unwrap().0).unwrap();
            assert_eq!(read.guid, b.guid);
            assert_eq!(read.version.as_deref(), Some(b.version));
        }
        assert!(find("BICRAFTSPEC").is_some());
        assert!(find("card").is_none());
    }

    /// Every visual of both parts of the first pack is registered, with its roles.
    #[test]
    fn the_chart_pack_is_registered() {
        for d in crate::chart_pack::VISUALS {
            let b = find(d.name).unwrap_or_else(|| panic!("{} is not in VISUALS", d.name));
            assert_eq!(b.guid, d.guid);
            assert_eq!(b.version, d.version);
            let names: Vec<&str> = d.roles.iter().map(|r| r.name).collect();
            assert_eq!(b.role_names(), names, "{}", d.name);
        }
        for d in crate::bvis::DEFS {
            let b = find(d.name).unwrap_or_else(|| panic!("{} is not in VISUALS", d.name));
            assert_eq!((b.guid, b.version), (d.guid, d.version));
            let names: Vec<&str> = d.roles.iter().map(|r| r.name).collect();
            assert_eq!(b.role_names(), names, "{}", d.name);
            assert!((b.icon)(b.guid).is_ok());
        }
        assert_eq!(VISUALS.len(), 14);
    }

    /// Every visual has its own icon (ROADMAP M12.4): a 20×20 PNG drawn from its drawing in
    /// `bicraft_visuals::icons`, different from every other visual's and from the scaffold's, with
    /// both the ink and the accent on a transparent background; its package carries that PNG.
    #[test]
    fn every_visual_has_its_own_icon_in_its_package() {
        let mut seen: Vec<Vec<u8>> = vec![crate::visual_project::icon_png().unwrap()];
        for b in VISUALS {
            assert!(
                bicraft_visuals::icons::bicraft_visual_icon(b.name).is_some(),
                "{} has no drawing",
                b.name
            );
            let png = (b.icon)(b.guid).unwrap();
            assert_eq!(icon_png(&b.guid.to_ascii_lowercase()).unwrap(), png);
            let pm = tiny_skia::Pixmap::decode_png(&png).unwrap();
            assert_eq!((pm.width(), pm.height()), (ICON_SIDE, ICON_SIDE), "{}", b.name);
            let px = pm.pixels();
            let corner = px.first().unwrap();
            assert_eq!(corner.alpha(), 0, "{}: a transparent background", b.name);
            let painted = px.iter().filter(|p| p.alpha() > 0).count();
            assert!(painted >= 40, "{}: {painted} pixels painted", b.name);
            // the accent blue shows (blue well above red in some opaque-enough pixel)
            assert!(
                px.iter()
                    .any(|p| p.alpha() > 100 && u16::from(p.blue()) > u16::from(p.red()) + 60),
                "{}: no accent",
                b.name
            );
            assert!(!seen.contains(&png), "{}: its icon is another visual's", b.name);
            seen.push(png.clone());
            let body: Value = serde_json::from_slice(&(b.package)(b.guid).unwrap().1).unwrap();
            assert_eq!(
                body["content"]["iconBase64"],
                format!("data:image/png;base64,{}", crate::render::base64(&png)),
                "{}: the package carries the icon",
                b.name
            );
        }
        assert!(icon_png("card").is_err());
    }

    /// The contrast ratio of two sRGB colours (WCAG 2.1).
    fn contrast(a: [u8; 3], b: [u8; 3]) -> f32 {
        let lum = |c: [u8; 3]| {
            let lin = |v: u8| {
                let v = f32::from(v) / 255.0;
                if v <= 0.040_45 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
        };
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// The package icons' ink is dark and bold enough for Desktop's pane (G111): the ink keeps
    /// 4.5:1 on its light pane (#f5f5f5 in Power BI Desktop 2.160.1151.0) and 2:1 on its dark one
    /// (#141414, where its own icons' dark ink keeps less); each icon drawn with ink has an ink
    /// pixel that keeps 3:1 composited on white (WCAG 2.1 non-text contrast); a line one stroke
    /// wide covers about 1.5 px at 20 × 20.
    #[test]
    fn package_icons_have_dark_bold_ink() {
        use bicraft_visuals::icons::{Mark, Tone};
        let ink = crate::visual_project::ICON_INK;
        assert!(contrast(ink, [0xf5; 3]) >= 4.5, "ink on the light pane");
        assert!(contrast(ink, [0x14; 3]) >= 2.0, "ink on the dark pane");
        for b in VISUALS {
            let pm = tiny_skia::Pixmap::decode_png(&(b.icon)(b.guid).unwrap()).unwrap();
            let best = pm
                .pixels()
                .iter()
                .map(|p| p.demultiply())
                .filter(|p| p.alpha() > 0 && u16::from(p.blue()) < u16::from(p.red()) + 40)
                .map(|p| {
                    let a = f32::from(p.alpha()) / 255.0;
                    let on_white = |v: u8| (f32::from(v) * a + 255.0 * (1.0 - a)).round() as u8;
                    contrast(
                        [on_white(p.red()), on_white(p.green()), on_white(p.blue())],
                        [0xff; 3],
                    )
                })
                .fold(0.0f32, f32::max);
            let marks = bicraft_visuals::icons::bicraft_visual_icon(b.name).unwrap();
            let inked = marks.iter().any(|m| match m {
                Mark::Line { tone, .. } | Mark::Fill { tone, .. } => matches!(tone, Tone::Ink(_)),
                Mark::Rect { fill, stroke, .. } | Mark::Circle { fill, stroke, .. } => {
                    [fill, stroke].iter().any(|t| matches!(t, Some(Tone::Ink(_))))
                }
            });
            assert!(
                !inked || best >= 3.0,
                "{}: its darkest ink keeps {best:.2}:1 on white",
                b.name
            );
        }
        let rule = [Mark::Line {
            pts: vec![(2.0, 12.0), (22.0, 12.0)],
            width: 1.0,
            tone: Tone::Ink(1.0),
        }];
        let pm = tiny_skia::Pixmap::decode_png(&crate::visual_project::marks_png(&rule, ICON_SIDE).unwrap())
            .unwrap();
        let column: f32 = (0..ICON_SIDE)
            .filter_map(|y| pm.pixel(10, y))
            .map(|p| f32::from(p.alpha()) / 255.0)
            .sum();
        assert!(
            (1.4..=1.8).contains(&column),
            "a one-stroke line covers {column:.2} px"
        );
    }

    /// The export form's choices are `all` and every visual's name, in the registry's order.
    #[test]
    fn the_export_choices_follow_the_registry() {
        let names: Vec<&str> = VISUALS.iter().map(|b| b.name).collect();
        assert_eq!(EXPORT_NAMES.first(), Some(&"all"));
        assert_eq!(EXPORT_NAMES.get(1..), Some(names.as_slice()));
    }

    #[test]
    fn versions_compare_part_by_part() {
        assert_eq!(compare("1.10.0.0", "1.9.0.0"), Some(Ordering::Greater));
        assert_eq!(compare("1.1", "1.1.0.0"), Some(Ordering::Equal));
        assert_eq!(compare("1.0.0.0", "1.1.0.0"), Some(Ordering::Less));
        assert_eq!(compare("1.x", "1.0"), None);
    }
}
