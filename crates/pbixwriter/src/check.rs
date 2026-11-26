//! The loader rules of `docs/formats/pbix-package.md` §7 (plus the OPC and encoding rules they
//! imply), as a validator usable on any package.

use std::collections::HashSet;

use bicraft_opc::Package;
use serde_json::Value;

use crate::{content_types, part};

/// Single parts Desktop stores as UTF-16LE text without BOM and refuses in another encoding
/// (packaging-verification.md PK-VER-ENC, PK-ENC-16, PK-ENC-LAYOUT).
const UTF16_PARTS: [&str; 6] = [
    "Version",
    "DataModelSchema",
    "DiagramLayout",
    "Settings",
    "Metadata",
    "Report/Layout",
];

/// Highest `Version` minor Desktop accepts (`1.36`; oracle cases v01, v02).
const MAX_MINOR: u32 = 36;
const MAX_CUSTOM_PROPERTIES: usize = 2 * 1024 * 1024;

/// Lists every loader-rule violation in `pkg` (empty when Desktop 2.158 should accept it as far as
/// the packaging layer goes). Messages name the part and the rule; they never quote part content.
pub fn check_package(pkg: &Package) -> Vec<String> {
    let mut out = Vec::new();
    let has = |name: &str| pkg.parts.iter().any(|p| p.name.eq_ignore_ascii_case(name));
    let get = |name: &str| pkg.parts.iter().find(|p| p.name.eq_ignore_ascii_case(name));

    // OPC: unique names (case-insensitive), well-formed names, a content type for every part.
    let mut seen = HashSet::new();
    for p in &pkg.parts {
        if !seen.insert(p.name.to_ascii_lowercase()) {
            out.push(format!(
                "`{}`: duplicate part name (OPC names ignore case)",
                p.name
            ));
        }
        if p.name != part::CONTENT_TYPES
            && let Err(m) = crate::names::check_relative_path(&p.name)
        {
            out.push(format!("`{}`: invalid part name ({m})", p.name));
        }
    }
    match get(part::CONTENT_TYPES) {
        None => out.push("`[Content_Types].xml` is missing".into()),
        Some(ct) => match content_types::parse(&ct.data) {
            Err(e) => out.push(format!("`[Content_Types].xml` is malformed: {e}")),
            Ok(types) => {
                out.extend(content_types::strict_issues(&ct.data));
                // System.IO.Packaging hides a part no content type covers: Desktop never sees it.
                for p in &pkg.parts {
                    if p.name != part::CONTENT_TYPES && types.lookup(&p.name).is_none() {
                        out.push(format!(
                            "`{}`: no content type in `[Content_Types].xml` (Desktop would not see the part)",
                            p.name
                        ));
                    }
                }
            }
        },
    }

    // Rule 2: no `Version.txt` (oracle case k01 refused; an unrelated `foo.txt` opens, k02;
    // other `<part>.txt` siblings were not tried).
    if has("Version.txt") {
        out.push("`Version.txt` exists: Desktop refuses a package with this part".into());
    }

    // Rules 3-4: `Version` exists and holds an accepted value.
    let _minor = match get(part::VERSION) {
        None => {
            out.push("`Version` is missing (the only required part)".into());
            None
        }
        Some(v) => match utf16_text(&v.data) {
            Err(e) => {
                out.push(format!("`Version`: {e}"));
                None
            }
            Ok(text) => {
                let minor = accepted_version(&text);
                if minor.is_none() {
                    out.push("`Version` is not one of the accepted values (`1.<minor>` up to `1.36`)".into());
                }
                minor
            }
        },
    };

    // Rule 5.
    if has(part::DATA_MODEL) && has(part::DATA_MODEL_SCHEMA) {
        out.push("`DataModel` and `DataModelSchema` must not both exist".into());
    }

    // Rule 6 (and rule 11: PBIR documents).
    let pbir: Vec<_> = pkg
        .parts
        .iter()
        .filter(|p| {
            p.name
                .get(..part::PBIR_PREFIX.len())
                .is_some_and(|s| s.eq_ignore_ascii_case(part::PBIR_PREFIX))
        })
        .collect();
    if !pbir.is_empty() {
        if has(part::LEGACY_LAYOUT) {
            out.push("`Report/Layout` must not coexist with a non-empty `Report/definition/`".into());
        }
        // No minimum `Version` for PBIR: Desktop 2.158 opens a PBIR package marked 1.28 (oracle
        // case v07, packaging-verification.md PK-VER-PBIR). The writers still emit 1.32, as Desktop.
        for p in &pbir {
            if let Err(e) = utf8_no_bom(&p.data) {
                out.push(format!("`{}`: {e}", p.name));
            } else if p.name.to_ascii_lowercase().ends_with(".json")
                && serde_json::from_slice::<Value>(&p.data).is_err()
            {
                out.push(format!("`{}`: invalid JSON", p.name));
            }
        }
        let rel = |p: &&bicraft_opc::Part| {
            p.name
                .get(part::PBIR_PREFIX.len()..)
                .unwrap_or_default()
                .to_string()
        };
        let docs: Vec<String> = pbir.iter().map(rel).collect();
        for required in ["version.json", "report.json"] {
            if !docs.iter().any(|d| d == required) {
                out.push(format!("`Report/definition/{required}` is missing"));
            }
        }
        let is_page =
            |d: &String| d.starts_with("pages/") && d.ends_with("/page.json") && d.matches('/').count() == 2;
        if !docs.iter().any(is_page) {
            out.push("`Report/definition/` has no page (`pages/<page>/page.json`)".into());
        }
    }

    // Rule 7.
    if let Some(p) = get("docProps/custom.xml")
        && p.data.len() > MAX_CUSTOM_PROPERTIES
    {
        out.push("`docProps/custom.xml` is larger than 2 MiB".into());
    }

    // Rules 8-9 (MS-QDEFF: the leading version is 0; Desktop refused both parts together, ORA-x02).
    if has("DataMashup") && has("UnappliedChanges") {
        out.push("`DataMashup` and `UnappliedChanges` must not both exist".into());
    }
    if let Some(p) = get("DataMashup")
        && p.data.get(..4) != Some(&[0, 0, 0, 0][..])
    {
        out.push("`DataMashup` must start with the 32-bit version 0".into());
    }

    // Encodings of the UTF-16LE parts.
    for name in UTF16_PARTS {
        if name == part::VERSION {
            continue; // checked above
        }
        if let Some(p) = get(name)
            && let Err(e) = utf16_text(&p.data)
        {
            out.push(format!("`{name}`: {e}"));
        }
    }

    // `DataModelSchema` and `DiagramLayout` must be UTF-16LE JSON: Desktop refuses either part in
    // UTF-8 (oracle cases e01, e03, packaging-verification.md PK-ENC-16). UTF-8 bytes of even
    // length decode as (wrong) UTF-16LE, so the JSON parse is what catches them.
    for name in [part::DATA_MODEL_SCHEMA, part::DIAGRAM_LAYOUT] {
        if let Some(p) = get(name)
            && let Ok(t) = utf16_text(&p.data)
            && serde_json::from_str::<Value>(&t).is_err()
        {
            out.push(format!("`{name}`: not JSON in UTF-16LE"));
        }
    }

    // Rule 10: JSON parts and their inner versions (PK-SETTINGS, PK-QSV, PK-METADATA).
    inner_version(&mut out, get(part::SETTINGS).map(|p| &p.data), "Settings", 4);
    if let Some(p) = get(part::SETTINGS) {
        mashup_version(&mut out, &p.data);
    }
    inner_version(&mut out, get(part::METADATA).map(|p| &p.data), "Metadata", 5);
    out
}

/// Oldest Mashup package version Desktop accepts in `Settings` `QueriesSettings.Version`.
const OLDEST_MASHUP_VERSION: [u32; 4] = [1, 0, 3207, 0];

/// `Settings` `QueriesSettings.Version` must be a 2-4 component version of at least
/// `1.0.3207.0`; without it Desktop refuses the file as created by an unrecognized version. (It
/// must also not be newer than the opening Desktop's engine, which a static check cannot know.)
fn mashup_version(out: &mut Vec<String>, data: &[u8]) {
    let Some(v) = utf16_text(data)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
    else {
        return; // reported by `inner_version`
    };
    let text = v
        .get("QueriesSettings")
        .and_then(|q| q.get("Version"))
        .and_then(Value::as_str);
    let parsed = text.and_then(|t| {
        let parts: Option<Vec<u32>> = t.split('.').map(|c| c.parse().ok()).collect();
        parts.filter(|p| (2..=4).contains(&p.len()))
    });
    match parsed {
        None => out.push(
            "`Settings`: `QueriesSettings.Version` (the Power Query package version) is missing or not a version; Desktop rejects the file".into(),
        ),
        Some(mut p) => {
            p.resize(4, 0);
            if p.as_slice() < &OLDEST_MASHUP_VERSION[..] {
                out.push("`Settings`: `QueriesSettings.Version` is older than 1.0.3207.0".into());
            }
        }
    }
}

/// The minor of an accepted `Version` string (`1.<n>`), or `None`.
fn accepted_version(text: &str) -> Option<u32> {
    let minor = text.strip_prefix("1.")?;
    // Exact membership: no sign, no leading zero, no extra component or whitespace.
    let canonical = minor.parse::<u32>().ok().filter(|m| m.to_string() == minor)?;
    (canonical <= MAX_MINOR).then_some(canonical)
}

fn utf16_text(data: &[u8]) -> Result<String, String> {
    if data.starts_with(&[0xFF, 0xFE]) {
        return Err("UTF-16LE text must not start with a BOM".into());
    }
    if !data.len().is_multiple_of(2) {
        return Err("odd byte count for UTF-16LE text".into());
    }
    let units: Vec<u16> = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    String::from_utf16(&units).map_err(|_| "invalid UTF-16LE text".to_string())
}

fn utf8_no_bom(data: &[u8]) -> Result<(), String> {
    if data.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Err("UTF-8 text must not start with a BOM".into());
    }
    std::str::from_utf8(data)
        .map(|_| ())
        .map_err(|_| "invalid UTF-8 text".to_string())
}

fn inner_version(out: &mut Vec<String>, data: Option<&Vec<u8>>, name: &str, max: i64) {
    let Some(data) = data else { return };
    let parsed = utf16_text(data).and_then(|t| {
        serde_json::from_str::<Value>(&t).map_err(|_| "invalid JSON (UTF-16LE expected)".into())
    });
    match parsed {
        Err(e) => out.push(format!("`{name}`: {e}")),
        Ok(v) => match v.get("Version").and_then(Value::as_i64) {
            Some(n) if (0..=max).contains(&n) => {}
            _ => out.push(format!("`{name}`: `Version` must be an integer of at most {max}")),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_versions_are_exact_strings() {
        assert_eq!(accepted_version("1.28"), Some(28));
        assert_eq!(accepted_version("1.36"), Some(36));
        for bad in ["1.37", "1.028", "1.28.0", " 1.28", "1.-1", "2.0", "1.", "", "0.1"] {
            assert_eq!(accepted_version(bad), None, "{bad}");
        }
    }

    #[test]
    fn utf16_checks() {
        assert!(utf16_text(&[0xFF, 0xFE, 0x31, 0]).is_err());
        assert!(utf16_text(&[0x31]).is_err());
        assert_eq!(utf16_text(&[0x31, 0]).unwrap(), "1");
    }
}
