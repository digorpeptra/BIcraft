//! Writing an edited report back into the package it was read from ([`write_report`]).
//!
//! The report keeps the form the package holds: PBIR parts under `Report/definition/` or the
//! legacy UTF-16LE `Report/Layout`. Only the parts whose bytes change are replaced; new parts are
//! appended after the existing report parts; parts of deleted pages, visuals and resources are
//! removed. `[Content_Types].xml` is touched only when a new extension appears or a removed part
//! had an `Override`. See `docs/formats/pbix-package.md` §9.

use std::collections::{HashMap, HashSet};

use bicraft_opc::{Package, Part, decode_text, encode_text};
use bicraft_report::Report;
use serde::Serialize;

use crate::{
    CUSTOM_VISUALS_PREFIX, Error, LEGACY_LAYOUT, PBIR_PREFIX, REPORT_PREFIX, ReportFormat,
    STATIC_RESOURCES_PREFIX,
};

/// The part that binds the report to the machine and user that saved it.
pub const SECURITY_BINDINGS: &str = "SecurityBindings";
pub const CONTENT_TYPES: &str = "[Content_Types].xml";

/// What [`write_report`] changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReportWrite {
    /// The report form that was written (the one the package held).
    pub format: ReportFormat,
    /// Parts whose bytes changed.
    pub changed: Vec<String>,
    /// Parts added (new pages, visuals, resources).
    pub added: Vec<String>,
    /// Parts removed (deleted pages, visuals, resources).
    pub removed: Vec<String>,
    /// Whether `SecurityBindings` was dropped: it hashes the report parts, so it no longer
    /// matches once they change (see `docs/formats/pbix-package.md` §9).
    pub security_bindings_removed: bool,
    /// Whether `[Content_Types].xml` was rewritten.
    pub content_types_changed: bool,
}

impl ReportWrite {
    /// Whether any part changed.
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.added.is_empty() && self.removed.is_empty()
    }
}

/// The report form `pkg` holds: PBIR parts win over a legacy layout, as when reading.
pub fn report_format(pkg: &Package) -> ReportFormat {
    if pkg.parts.iter().any(|p| p.name.starts_with(PBIR_PREFIX)) {
        ReportFormat::Pbir
    } else if pkg.get(LEGACY_LAYOUT).is_some() {
        ReportFormat::Legacy
    } else {
        ReportFormat::None
    }
}

/// Writes `report` into a copy of `pkg`, which must be the package the report was read from
/// (with edits applied), in the form the package holds:
///
/// - PBIR: the documents of `bicraft_pbir::write_documents`, UTF-8 without BOM, under
///   `Report/`; a part whose bytes are unchanged keeps them; parts under `Report/definition/`
///   the report no longer writes are removed.
/// - Legacy: `bicraft_layout::write_layout` patches the `Report/Layout` text, which is
///   re-encoded in its original encoding (UTF-16LE for Desktop files).
///
/// In both forms `Report/StaticResources/` and `Report/CustomVisuals/` parts follow
/// [`Report::resources`]: changed bytes
/// are written, new resources added, and a resource part the report no longer lists removed.
/// Every other part keeps its bytes, compression flag and position. When any report part
/// changes, `SecurityBindings` is removed (and its `Override` with it).
pub fn write_report(pkg: &Package, report: &Report) -> Result<(Package, ReportWrite), Error> {
    let format = report_format(pkg);
    let mut desired: Vec<(String, Vec<u8>)> = Vec::new();
    match format {
        ReportFormat::Pbir => {
            bicraft_pbir::check_names(report)?;
            for (path, bytes) in bicraft_pbir::write_documents(report) {
                // Desktop writes the PBIR documents of a PBIX without indentation.
                let bytes = if path.starts_with("definition/") {
                    compact_json(bytes)
                } else {
                    bytes
                };
                desired.push((format!("{REPORT_PREFIX}{path}"), bytes));
            }
        }
        ReportFormat::Legacy => {
            let part = pkg.get(LEGACY_LAYOUT).ok_or(Error::NoReport)?;
            let (text, encoding) = decode_text(&part.name, &part.data)?;
            let written = bicraft_layout::write_layout(&text, report)?;
            let bytes = if written == text {
                part.data.clone()
            } else {
                encode_text(&written, encoding)
            };
            desired.push((part.name.clone(), bytes));
            for r in &report.resources {
                let path = r.path.replace('\\', "/");
                if path.starts_with("StaticResources/") {
                    desired.push((format!("{REPORT_PREFIX}{path}"), r.bytes.clone()));
                }
            }
        }
        ReportFormat::None => return Err(Error::NoReport),
    }
    let listed: HashSet<String> = report
        .resources
        .iter()
        .flat_map(|r| {
            let name = format!("{REPORT_PREFIX}{}", r.path.replace('\\', "/"));
            [encode_part_name(&name).to_lowercase(), name.to_lowercase()]
        })
        .collect();

    // Desired parts by lower-case name, raw and percent-encoded.
    let mut index: HashMap<String, usize> = HashMap::new();
    for (i, (name, _)) in desired.iter().enumerate() {
        index.entry(name.to_lowercase()).or_insert(i);
        index.entry(encode_part_name(name).to_lowercase()).or_insert(i);
    }
    let mut used = vec![false; desired.len()];
    let mut out = Package::default();
    let mut write = ReportWrite {
        format,
        changed: Vec::new(),
        added: Vec::new(),
        removed: Vec::new(),
        security_bindings_removed: false,
        content_types_changed: false,
    };
    let mut insert_at = None;
    for part in &pkg.parts {
        let lower = part.name.to_lowercase();
        if let Some(&i) = index.get(&lower)
            && let Some((_, data)) = desired.get(i)
            && !used.get(i).copied().unwrap_or(true)
        {
            if let Some(u) = used.get_mut(i) {
                *u = true;
            }
            let data = if same_content(&part.name, &part.data, data) {
                part.data.clone()
            } else {
                write.changed.push(part.name.clone());
                data.clone()
            };
            out.parts.push(Part::new(part.name.clone(), data, part.stored));
            insert_at = Some(out.parts.len());
            continue;
        }
        let stale = match format {
            ReportFormat::Pbir => part.name.starts_with(PBIR_PREFIX),
            _ => false,
        } || ((part.name.starts_with(STATIC_RESOURCES_PREFIX)
            || part.name.starts_with(CUSTOM_VISUALS_PREFIX))
            && !listed.contains(&lower));
        if stale {
            write.removed.push(part.name.clone());
            continue;
        }
        if part.name.starts_with(REPORT_PREFIX) {
            insert_at = Some(out.parts.len() + 1);
        }
        out.parts.push(part.clone());
    }
    let mut new_parts = Vec::new();
    for ((name, data), used) in desired.into_iter().zip(used) {
        if !used {
            let name = encode_part_name(&name);
            write.added.push(name.clone());
            new_parts.push(Part::new(name, data, false));
        }
    }
    let at = insert_at.unwrap_or(out.parts.len()).min(out.parts.len());
    out.parts.splice(at..at, new_parts);

    if !write.is_empty() && out.get(SECURITY_BINDINGS).is_some() {
        let name = out
            .get(SECURITY_BINDINGS)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| SECURITY_BINDINGS.to_string());
        out.parts
            .retain(|p| !p.name.eq_ignore_ascii_case(SECURITY_BINDINGS));
        write.removed.push(name);
        write.security_bindings_removed = true;
    }
    if let Some(ct) = out.get(CONTENT_TYPES)
        && let Some(xml) = update_content_types(&ct.data, &write.added, &write.removed)
    {
        let stored = ct.stored;
        out.put(CONTENT_TYPES, xml, stored);
        write.content_types_changed = true;
    }
    Ok((out, write))
}

/// `bytes` without insignificant whitespace when it is JSON, else unchanged.
fn compact_json(bytes: Vec<u8>) -> Vec<u8> {
    match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(v) => serde_json::to_vec(&v).unwrap_or(bytes),
        Err(_) => bytes,
    }
}

/// Whether `new` holds what `old` holds: the same bytes, or, for a JSON part, the same JSON
/// value (Desktop's formatting and bicraft's may differ; the original bytes are then kept).
fn same_content(name: &str, old: &[u8], new: &[u8]) -> bool {
    if old == new {
        return true;
    }
    if !name.to_ascii_lowercase().ends_with(".json") {
        return false;
    }
    match (
        serde_json::from_slice::<serde_json::Value>(old),
        serde_json::from_slice::<serde_json::Value>(new),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Bytes kept as-is in a part URI path: RFC 3986 unreserved, sub-delims, `:`, `@` and `/`.
fn is_path_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@/".contains(&b)
}

/// The zip entry name of a part path: System.IO.Packaging stores a part under its escaped URI
/// (percent-encoded UTF-8), e.g. `Report/StaticResources/My%20Logo.png`.
pub(crate) fn encode_part_name(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for &b in path.as_bytes() {
        if is_path_byte(b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `[Content_Types].xml` with a `Default` (empty content type, as Desktop writes for report
/// parts) for each new extension and an `Override` for each new extension-less part, and the
/// `Override`s of removed parts dropped. `None` when nothing needs to change or the document is
/// not understood (it is then kept as is).
fn update_content_types(data: &[u8], added: &[String], removed: &[String]) -> Option<Vec<u8>> {
    let (bom, body) = match data.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        Some(rest) => (true, rest),
        None => (false, data),
    };
    let xml = std::str::from_utf8(body).ok()?;
    let mut tags = Vec::new();
    let mut pos = 0usize;
    // (start, end) byte ranges of `<Default …/>` and `<Override …/>` elements.
    while let Some(off) = xml.get(pos..).and_then(|s| s.find('<')) {
        let start = pos.checked_add(off)?;
        let end = start.checked_add(xml.get(start..)?.find('>')?)?.checked_add(1)?;
        let tag = xml.get(start..end)?;
        if tag.starts_with("<Default ") || tag.starts_with("<Override ") {
            tags.push((start, end));
        }
        pos = end;
    }
    let attr = |tag: &str, name: &str| -> Option<String> {
        let key = format!("{name}=\"");
        let from = tag.find(&key)?.checked_add(key.len())?;
        let len = tag.get(from..)?.find('"')?;
        Some(tag.get(from..from.checked_add(len)?)?.to_string())
    };
    let mut drop: Vec<(usize, usize)> = Vec::new();
    let mut extensions: Vec<String> = Vec::new();
    let mut overrides: Vec<String> = Vec::new();
    for &(s, e) in &tags {
        let tag = xml.get(s..e)?;
        if tag.starts_with("<Default ") {
            extensions.extend(attr(tag, "Extension").map(|x| x.to_lowercase()));
        } else if let Some(name) = attr(tag, "PartName") {
            let gone = removed
                .iter()
                .any(|r| format!("/{r}").eq_ignore_ascii_case(&name));
            if gone {
                drop.push((s, e));
            } else {
                overrides.push(name.to_lowercase());
            }
        }
    }
    let mut new_defaults = String::new();
    let mut new_overrides = String::new();
    for name in added {
        let last = name.rsplit('/').next().unwrap_or(name);
        match last
            .rsplit_once('.')
            .map(|(_, ext)| ext)
            .filter(|e| !e.is_empty())
        {
            Some(ext) => {
                let ext = ext.to_lowercase();
                if !extensions.contains(&ext) {
                    new_defaults.push_str(&format!(
                        r#"<Default Extension="{}" ContentType="" />"#,
                        escape(&ext)
                    ));
                    extensions.push(ext);
                }
            }
            None => {
                let part = format!("/{name}");
                if !overrides.contains(&part.to_lowercase()) {
                    new_overrides.push_str(&format!(
                        r#"<Override PartName="{}" ContentType="" />"#,
                        escape(&part)
                    ));
                    overrides.push(part.to_lowercase());
                }
            }
        }
    }
    if drop.is_empty() && new_defaults.is_empty() && new_overrides.is_empty() {
        return None;
    }
    // Defaults go after the last Default (or before the first element); overrides at the end.
    let default_at = tags
        .iter()
        .filter(|(s, e)| xml.get(*s..*e).is_some_and(|t| t.starts_with("<Default ")))
        .map(|(_, e)| *e)
        .next_back()
        .or_else(|| tags.first().map(|(s, _)| *s))
        .or_else(|| xml.find("</Types>"))?;
    let override_at = xml.rfind("</Types>")?;
    let mut out = String::with_capacity(xml.len() + new_defaults.len() + new_overrides.len());
    let mut cursor = 0usize;
    let mut cuts: Vec<(usize, usize, &str)> = drop.iter().map(|&(s, e)| (s, e, "")).collect();
    cuts.push((default_at, default_at, &new_defaults));
    cuts.push((override_at, override_at, &new_overrides));
    cuts.sort_by_key(|&(s, e, _)| (s, e));
    for (s, e, insert) in cuts {
        out.push_str(xml.get(cursor..s)?);
        out.push_str(insert);
        cursor = e;
    }
    out.push_str(xml.get(cursor..)?);
    let mut bytes = Vec::with_capacity(out.len() + 3);
    if bom {
        bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    }
    bytes.extend_from_slice(out.as_bytes());
    Some(bytes)
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    const CT: &str = "\u{feff}<?xml version=\"1.0\" encoding=\"utf-8\"?><Types xmlns=\"x\"><Default Extension=\"json\" ContentType=\"\" /><Override PartName=\"/Version\" ContentType=\"\" /><Override PartName=\"/SecurityBindings\" ContentType=\"\" /></Types>";

    const METADATA: &[u8] = include_bytes!("../../metadata/tests/fixtures/synthetic_metadata.sqlitedb");

    fn names(pkg: &Package) -> Vec<&str> {
        pkg.parts.iter().map(|p| p.name.as_str()).collect()
    }

    #[test]
    fn an_unedited_report_changes_nothing() {
        for pkg in [
            crate::synthetic::pbir_pbix(METADATA).unwrap(),
            crate::synthetic::pbix(METADATA).unwrap(),
        ] {
            let report = crate::read_report(&pkg).unwrap();
            let (out, change) = write_report(&pkg, &report).unwrap();
            assert!(change.is_empty(), "{change:?}");
            assert!(!change.security_bindings_removed && !change.content_types_changed);
            assert_eq!(out, pkg);
        }
    }

    #[test]
    fn pbir_edits_touch_only_their_parts() {
        let pkg = crate::synthetic::pbir_pbix(METADATA).unwrap();
        let mut report = crate::read_report(&pkg).unwrap();
        report.pages[0].visuals[0].position.x = 99.0;
        let mut page = report.pages[0].clone();
        page.name = "p2".into();
        page.display_name = "Second".into();
        report.pages.push(page);
        report.active_page = Some("p2".into());
        report.resources.retain(|r| !r.path.ends_with("Base.json"));
        report.resources.push(bicraft_report::Resource {
            path: "StaticResources/RegisteredResources/my logo.png".into(),
            bytes: vec![1, 2, 3],
        });
        let (out, change) = write_report(&pkg, &report).unwrap();
        assert_eq!(
            change.changed,
            [
                "Report/definition/pages/pages.json",
                "Report/definition/pages/p1/visuals/v1/visual.json"
            ]
        );
        assert_eq!(
            change.added,
            [
                "Report/definition/pages/p2/page.json",
                "Report/definition/pages/p2/visuals/v1/visual.json",
                "Report/StaticResources/RegisteredResources/my%20logo.png"
            ]
        );
        assert_eq!(
            change.removed,
            [
                "Report/StaticResources/SharedResources/BaseThemes/Base.json",
                "SecurityBindings"
            ]
        );
        assert!(change.security_bindings_removed && change.content_types_changed);
        // new parts follow the report parts; the model part is untouched and stays stored
        let n = names(&out);
        let at = |name: &str| n.iter().position(|x| *x == name).unwrap();
        assert!(
            at("Report/definition/pages/p2/page.json")
                > at("Report/definition/pages/p1/visuals/v1/visual.json")
        );
        assert_eq!(n.last(), Some(&crate::DATA_MODEL));
        assert_eq!(out.get(crate::DATA_MODEL), pkg.get(crate::DATA_MODEL));
        for p in &pkg.parts {
            let same = out.get(&p.name) == Some(p);
            let expected = !change.changed.contains(&p.name)
                && !change.removed.contains(&p.name)
                && p.name != CONTENT_TYPES;
            assert_eq!(same, expected, "{}", p.name);
        }
        let ct = String::from_utf8(out.get(CONTENT_TYPES).unwrap().data.clone()).unwrap();
        assert!(
            !ct.contains("SecurityBindings") && ct.contains(r#"<Default Extension="png" ContentType="" />"#)
        );
        // what was written reads back
        let back = crate::read_report(&out).unwrap();
        assert_eq!(back.pages.len(), 2);
        assert_eq!(back.active_page.as_deref(), Some("p2"));
        assert_eq!(back.pages[0].visuals[0].position.x, 99.0);
        assert!(back.resources.iter().any(|r| r.bytes == [1, 2, 3]));
        // removing a page removes its parts
        let mut fewer = back.clone();
        fewer.pages.retain(|p| p.name != "p1");
        let (out2, change) = write_report(&out, &fewer).unwrap();
        assert_eq!(
            change.removed,
            [
                "Report/definition/pages/p1/page.json",
                "Report/definition/pages/p1/visuals/v1/visual.json"
            ]
        );
        assert!(!names(&out2).iter().any(|n| n.contains("/p1/")));
    }

    /// G26: embedded custom visuals and parts bicraft does not know (Copilot, DAX queries, TMDL
    /// scripts, a report-side diagram layout) survive a report save byte for byte and in place,
    /// in both report forms; the custom visual files are listed as report resources.
    #[test]
    fn custom_visuals_and_unknown_parts_survive_report_saves() {
        let extra: [(&str, &[u8]); 6] = [
            (
                "Report/CustomVisuals/hello1234/package.json",
                br#"{"visual":{"guid":"hello1234"}}"#,
            ),
            (
                "Report/CustomVisuals/hello1234/resources/hello1234.pbiviz.json",
                b"{\"content\":{}}",
            ),
            ("Copilot/version.json", b"\xff\xfe{\x00}\x00"),
            ("DAXQueries/.pbi/daxQueries.json", b"{}"),
            ("TMDLScripts/Script 1.tmdl", b"\xff\xfe"),
            ("Report/semanticModelDiagramLayout.json", b"{}"),
        ];
        for mut pkg in [
            crate::synthetic::pbir_pbix(METADATA).unwrap(),
            crate::synthetic::pbix(METADATA).unwrap(),
        ] {
            for (name, data) in extra {
                pkg.parts.insert(2, Part::new(name, data.to_vec(), false));
            }
            let mut report = crate::read_report(&pkg).unwrap();
            let listed: Vec<&str> = report
                .resources
                .iter()
                .map(|r| r.path.as_str())
                .filter(|p| p.starts_with("CustomVisuals/"))
                .collect();
            assert_eq!(listed.len(), 2, "{listed:?}");
            let (same, _) = write_report(&pkg, &report).unwrap();
            assert_eq!(same, pkg);
            report.pages[0].display_name = "Renamed".into();
            let (out, change) = write_report(&pkg, &report).unwrap();
            assert!(!change.is_empty());
            for (name, data) in extra {
                let at = |p: &Package| p.parts.iter().position(|x| x.name == name);
                assert_eq!(out.get(name).map(|p| p.data.as_slice()), Some(data), "{name}");
                assert_eq!(at(&out), at(&pkg), "{name} keeps its position");
            }
            let back = crate::read_report(&out).unwrap();
            assert_eq!(back.pages[0].display_name, "Renamed");
        }
    }

    /// G111 (seen in Power BI Desktop 2.159.1231.0, 2026-11-24): a custom visual whose files the
    /// report no longer lists (a BIcraft visual dropped on save) leaves no `Report/CustomVisuals/`
    /// part behind, in both report forms; a listed one, also under a percent-encoded name, stays.
    #[test]
    fn a_dropped_custom_visual_leaves_no_parts() {
        let kept = "Report/CustomVisuals/kept%20one/package.json";
        let gone: [&str; 2] = [
            "Report/CustomVisuals/gone1234/package.json",
            "Report/CustomVisuals/gone1234/resources/gone1234.pbiviz.json",
        ];
        for mut pkg in [
            crate::synthetic::pbir_pbix(METADATA).unwrap(),
            crate::synthetic::pbix(METADATA).unwrap(),
        ] {
            for name in gone.iter().chain([&kept]) {
                pkg.parts.insert(2, Part::new(*name, b"{}".to_vec(), false));
            }
            let mut report = crate::read_report(&pkg).unwrap();
            report.resources.retain(|r| !r.path.contains("gone1234"));
            let (out, change) = write_report(&pkg, &report).unwrap();
            for name in gone {
                assert!(out.get(name).is_none(), "{name}");
                assert!(change.removed.iter().any(|r| r == name), "{change:?}");
            }
            assert_eq!(out.get(kept).map(|p| p.data.as_slice()), Some(&b"{}"[..]));
            let back = crate::read_report(&out).unwrap();
            assert!(!back.resources.iter().any(|r| r.path.contains("gone1234")));
        }
    }

    #[test]
    fn legacy_layouts_stay_legacy() {
        let pkg = crate::synthetic::pbix(METADATA).unwrap();
        let mut report = crate::read_report(&pkg).unwrap();
        report.pages[0].display_name = "Renamed".into();
        let (out, change) = write_report(&pkg, &report).unwrap();
        assert_eq!(change.format, ReportFormat::Legacy);
        assert_eq!(change.changed, [LEGACY_LAYOUT]);
        assert!(change.added.is_empty() && change.removed.is_empty());
        assert!(!out.parts.iter().any(|p| p.name.starts_with(PBIR_PREFIX)));
        let (text, enc) = decode_text(LEGACY_LAYOUT, &out.get(LEGACY_LAYOUT).unwrap().data).unwrap();
        assert_eq!(enc, bicraft_opc::Encoding::Utf16Le);
        assert!(text.contains("Renamed"));
        assert_eq!(crate::read_report(&out).unwrap(), report);
        assert_eq!(out.get(crate::DATA_MODEL), pkg.get(crate::DATA_MODEL));
    }

    /// A phone layout given as a PBIR `mobile.json` resource is written as the legacy phone
    /// layout (`config.layouts` id 1, `mobileObjects`, `mobileVcObjects`), patched and removed
    /// later; other PBIR documents are refused, never dropped.
    #[test]
    fn legacy_layouts_hold_phone_layouts_and_refuse_other_pbir_documents() {
        use bicraft_layout::MOBILE_STATE_KEY;
        use bicraft_report::{FieldBinding, FieldRef, Resource, Visual};
        use serde_json::json;

        let pkg = crate::synthetic::pbix(METADATA).unwrap();
        let mut report = crate::read_report(&pkg).unwrap();
        let page = report.pages[0].name.clone();
        let mut card = Visual {
            name: "card1".into(),
            visual_type: Some("card".into()),
            fields: vec![FieldBinding {
                display_name: None,
                show_as: None,
                role: "Values".into(),
                field: FieldRef::Measure {
                    table: "Sales".into(),
                    measure: "Total".into(),
                },
            }],
            ..Visual::default()
        };
        card.position.width = 200.0;
        card.position.height = 100.0;
        report.pages[0].visuals.push(card);
        report.pages[0].extra.insert(
            "visualInteractions".into(),
            json!([{"source": "card1", "target": "x", "type": "NoFilter"}]),
        );
        let state = json!({
            "position": {"x": 1, "y": 2, "z": 0, "height": 50, "width": 100},
            "objects": {"labels": [{"properties": {"fontSize": {"expr": {"Literal": {"Value": "20D"}}}}}]},
            "visualContainerObjects": {"title": [{"properties": {"show": {"expr": {"Literal": {"Value": "false"}}}}}]}
        });
        let mut doc = state.clone();
        doc["$schema"] = json!("https://example.invalid/visualContainerMobileState/2.7.0/schema.json");
        report.resources.push(Resource {
            path: format!("definition/pages/{page}/visuals/card1/mobile.json"),
            bytes: serde_json::to_vec(&doc).unwrap(),
        });
        let (out, change) = write_report(&pkg, &report).unwrap();
        assert_eq!(change.changed, [LEGACY_LAYOUT]);
        assert!(change.added.is_empty(), "{change:?}");
        let text = decode_text(LEGACY_LAYOUT, &out.get(LEGACY_LAYOUT).unwrap().data)
            .unwrap()
            .0;
        assert!(text.contains(r#"\"id\":1,\"position\":{\"x\":1,"#), "{text}");
        assert!(text.contains(r#"\"mobileObjects\":{\"labels\""#), "{text}");
        assert!(text.contains(r#"\"mobileVcObjects\":{\"title\""#), "{text}");
        assert!(
            text.contains(r#"\"relationships\":[{\"source\":\"card1\",\"target\":\"x\",\"type\":3}]"#),
            "{text}"
        );
        let back = crate::read_report(&out).unwrap();
        let card = &back.pages[0].visuals[0];
        assert_eq!(card.extra[MOBILE_STATE_KEY], state);
        assert_eq!(
            back.pages[0].extra["visualInteractions"],
            report.pages[0].extra["visualInteractions"]
        );

        // Moving it patches the phone layout; the objects stay.
        let mut moved = back.clone();
        moved.pages[0].visuals[0].extra[MOBILE_STATE_KEY]["position"]["x"] = json!(30);
        let (out2, _) = write_report(&out, &moved).unwrap();
        let again = crate::read_report(&out2).unwrap();
        assert_eq!(
            again.pages[0].visuals[0].extra[MOBILE_STATE_KEY]["position"]["x"],
            30
        );
        assert_eq!(
            again.pages[0].visuals[0].extra[MOBILE_STATE_KEY]["objects"],
            state["objects"]
        );
        // Removing it removes the phone layout and its objects.
        let mut gone = again.clone();
        gone.pages[0].visuals[0].extra.shift_remove(MOBILE_STATE_KEY);
        let (out3, _) = write_report(&out2, &gone).unwrap();
        let text = decode_text(LEGACY_LAYOUT, &out3.get(LEGACY_LAYOUT).unwrap().data)
            .unwrap()
            .0;
        assert!(
            !text.contains("mobileObjects") && !text.contains(r#"\"id\":1"#),
            "{text}"
        );
        assert_eq!(crate::read_report(&out3).unwrap(), gone);

        // A PBIR document with no legacy form is refused, named.
        let mut other = back.clone();
        other.resources.push(Resource {
            path: "definition/reportExtensions.json".into(),
            bytes: b"{}".to_vec(),
        });
        let e = write_report(&out, &other).unwrap_err().to_string();
        assert!(e.contains("definition/reportExtensions.json"), "{e}");
    }

    #[test]
    fn a_package_without_a_report_is_refused() {
        let pkg = Package::default();
        assert!(matches!(
            write_report(&pkg, &Report::default()),
            Err(Error::NoReport)
        ));
    }

    #[test]
    fn content_types_follow_added_and_removed_parts() {
        let data = CT.as_bytes();
        assert_eq!(
            update_content_types(data, &["Report/definition/pages/a/page.json".into()], &[]),
            None
        );
        let out = update_content_types(
            data,
            &[
                "Report/StaticResources/RegisteredResources/x.png".into(),
                "Odd".into(),
            ],
            &["SecurityBindings".into()],
        )
        .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with('\u{feff}'));
        assert!(!text.contains("SecurityBindings"));
        assert!(text.contains(
            r#"<Default Extension="json" ContentType="" /><Default Extension="png" ContentType="" /><Override PartName="/Version""#
        ));
        assert!(text.ends_with(r#"<Override PartName="/Odd" ContentType="" /></Types>"#));
    }

    #[test]
    fn hostile_content_types_are_left_alone() {
        assert_eq!(
            update_content_types(b"<Types><Default Extension=", &["a.png".into()], &[]),
            None
        );
        assert_eq!(update_content_types(&[0xFF, 0xFE], &["a.png".into()], &[]), None);
    }

    #[test]
    fn part_names_are_percent_encoded() {
        assert_eq!(encode_part_name("Report/a b/é.png"), "Report/a%20b/%C3%A9.png");
        assert_eq!(
            encode_part_name("Report/definition/pages/x/page.json"),
            "Report/definition/pages/x/page.json"
        );
    }
}
