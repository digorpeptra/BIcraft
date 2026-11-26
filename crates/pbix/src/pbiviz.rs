//! `.pbiviz` custom visual packages (`docs/formats/pbiviz.md`).
//!
//! A `.pbiviz` file is a ZIP archive, as the public `powerbi-visuals-tools` packaging writes it
//! ("Package a Power BI visual", Microsoft Learn): a `package.json` manifest (`visual` with its
//! `guid`, `author`, `resources[{resourceId, sourceType, file}]`) and, under `resources/`, the
//! visual's `<guid>.pbiviz.json` (`visual`, `apiVersion`, `capabilities` and `content` with the
//! JavaScript, the CSS and the icon as a base64 `data:` URL).
//!
//! [`read_pbiviz`] reads one safely. The archive is hostile: its size, entry count, entry sizes
//! and expansion ratio are capped (through [`bicraft_opc::for_each_zip_entry`], which also refuses
//! names that leave the folder, links, special and encrypted entries); every kept name is checked
//! again here, so a stored path is always `package.json` or `resources/<plain names>`. Nothing in
//! the package is run: BIcraft never executes a visual's JavaScript.

use bicraft_opc::{ExtractError, ZipLimits, for_each_zip_entry};
use serde_json::Value;

/// Largest `.pbiviz` file read.
pub const MAX_PBIVIZ_BYTES: usize = 32 << 20;
/// Largest decompressed entry.
const MAX_ENTRY_BYTES: u64 = 32 << 20;
/// Largest sum of decompressed entries.
const MAX_TOTAL_BYTES: u64 = 64 << 20;
/// Most entries in the archive.
const MAX_ENTRIES: usize = 256;
/// Largest expansion of an entry past 1 MiB (JavaScript compresses well, a bomb far better).
const MAX_RATIO: u64 = 100;
/// Longest visual guid: it names the visual's folder.
pub const MAX_GUID: usize = 128;
/// Longest name component kept.
const MAX_COMPONENT: usize = 128;
/// Deepest path kept under `resources/`.
const MAX_DEPTH: usize = 8;

/// A custom visual read from a `.pbiviz` file.
#[derive(Debug, Clone, PartialEq)]
pub struct Pbiviz {
    /// `visual.guid`: the visual type its visuals carry and its folder's name.
    pub guid: String,
    pub display_name: Option<String>,
    pub version: Option<String>,
    pub api_version: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
    /// The `.pbiviz.json` carries an icon (`content.iconBase64`).
    pub has_icon: bool,
    /// The `.pbiviz.json` carries JavaScript (`content.js`).
    pub has_js: bool,
    /// The files to store under `CustomVisuals/<guid>/`: `package.json`, then the `resources/…`
    /// files, as `(relative path, bytes)`.
    pub files: Vec<(String, Vec<u8>)>,
    /// The `CustomVisualMetadata` item's path: the `.pbiviz.json` relative to `resources/`.
    pub metadata_item: String,
    /// Entries left out (outside `package.json` and `resources/`, or with unusual names).
    pub skipped: Vec<String>,
    pub warnings: Vec<String>,
}

fn parse_json(bytes: &[u8]) -> Option<Value> {
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes);
    serde_json::from_slice(body).ok()
}

fn text_at(v: &Value, path: &[&str]) -> Option<String> {
    let mut cur = v;
    for key in path {
        cur = cur.get(key)?;
    }
    cur.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(1000).collect())
}

/// Whether `guid` can name the visual's folder and type: ASCII letters, digits and `_`.
pub fn valid_guid(guid: &str) -> bool {
    !guid.is_empty() && guid.len() <= MAX_GUID && guid.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// A plain file or folder name: ASCII letters, digits, `.`, `-`, `_`, no leading dot.
fn plain_component(c: &str) -> bool {
    !c.is_empty()
        && c.len() <= MAX_COMPONENT
        && !c.starts_with('.')
        && !c.ends_with('.')
        && c.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

/// `package.json` or `resources/<plain names>` (prefix case kept as `resources/`); `None`: left out.
fn kept_name(name: &str) -> Option<String> {
    if name.eq_ignore_ascii_case("package.json") {
        return Some("package.json".into());
    }
    let rest = name
        .get(..10)
        .filter(|h| h.eq_ignore_ascii_case("resources/"))
        .and_then(|_| name.get(10..))?;
    let parts: Vec<&str> = rest.split('/').collect();
    (parts.len() <= MAX_DEPTH && parts.iter().all(|p| plain_component(p)))
        .then(|| format!("resources/{rest}"))
}

/// Reads a `.pbiviz` archive (module docs). Errors say why the file is refused.
pub fn read_pbiviz(bytes: &[u8]) -> Result<Pbiviz, String> {
    if bytes.len() > MAX_PBIVIZ_BYTES {
        return Err(format!(
            "the file is {} bytes; a .pbiviz is read up to {} MB",
            bytes.len(),
            MAX_PBIVIZ_BYTES >> 20
        ));
    }
    let limits = ZipLimits {
        max_entries: MAX_ENTRIES,
        max_entry_bytes: MAX_ENTRY_BYTES,
        max_total_bytes: MAX_TOTAL_BYTES,
        max_ratio: MAX_RATIO,
    };
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut skipped = Vec::new();
    for_each_zip_entry(std::io::Cursor::new(bytes), &limits, |entry, stream| {
        let mut data = Vec::new();
        stream.read_to_end(&mut data).map_err(|e| e.to_string())?;
        if entry.is_dir {
            return Ok(());
        }
        match kept_name(&entry.name) {
            Some(n) => files.push((n, data)),
            None => skipped.push(entry.name.chars().take(200).collect()),
        }
        Ok(())
    })
    .map_err(|e| match e {
        ExtractError::Invalid(m) => format!("not a .pbiviz package (a ZIP archive): {m}"),
        other => format!("the .pbiviz package is refused: {other}"),
    })?;
    let manifest_bytes = files
        .iter()
        .find(|(n, _)| n == "package.json")
        .map(|(_, b)| b.as_slice())
        .ok_or("the .pbiviz package has no package.json")?;
    let manifest = parse_json(manifest_bytes)
        .filter(Value::is_object)
        .ok_or("the package's package.json is not a JSON object")?;
    let guid =
        text_at(&manifest, &["visual", "guid"]).ok_or("the package's package.json names no visual.guid")?;
    if !valid_guid(&guid) {
        return Err(format!(
            "the visual's guid `{}` is not made of letters, digits and _ (at most {MAX_GUID})",
            guid.chars().take(60).collect::<String>()
        ));
    }
    // the visual's file: the manifest's resource that is a .pbiviz.json, else the only one
    let is_pbiviz_json = |n: &str| n.to_ascii_lowercase().ends_with(".pbiviz.json");
    let named: Option<String> = manifest
        .get("resources")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(1000)
        .filter_map(|r| r.get("file").and_then(Value::as_str))
        .map(|f| f.replace('\\', "/"))
        .find(|f| is_pbiviz_json(f));
    let candidates: Vec<&String> = files
        .iter()
        .map(|(n, _)| n)
        .filter(|n| n.starts_with("resources/") && is_pbiviz_json(n))
        .collect();
    let metadata_path = match named {
        Some(f) => candidates
            .iter()
            .find(|n| n.eq_ignore_ascii_case(&f))
            .map(|n| n.to_string())
            .ok_or_else(|| {
                format!(
                    "package.json names `{}`, which the package does not hold",
                    f.chars().take(120).collect::<String>()
                )
            })?,
        None => match candidates.as_slice() {
            [one] => one.to_string(),
            [] => return Err("the package holds no resources/*.pbiviz.json".into()),
            _ => {
                return Err(
                    "the package holds several .pbiviz.json files and package.json names none".into(),
                );
            }
        },
    };
    let visual = files
        .iter()
        .find(|(n, _)| *n == metadata_path)
        .and_then(|(_, b)| parse_json(b))
        .filter(Value::is_object)
        .ok_or_else(|| format!("`{metadata_path}` is not a JSON object"))?;
    if let Some(g) = text_at(&visual, &["visual", "guid"])
        && !g.eq_ignore_ascii_case(&guid)
    {
        return Err(format!(
            "`{metadata_path}` is visual `{}`, package.json visual `{guid}`",
            g.chars().take(60).collect::<String>()
        ));
    }
    let field =
        |key: &str| text_at(&visual, &["visual", key]).or_else(|| text_at(&manifest, &["visual", key]));
    let has_content = |key: &str| {
        visual
            .pointer(&format!("/content/{key}"))
            .and_then(Value::as_str)
            .is_some_and(|s| !s.trim().is_empty())
    };
    let mut warnings = Vec::new();
    let has_js = has_content("js");
    if !has_js {
        warnings.push("the visual carries no JavaScript (content.js): Power BI cannot draw it".to_string());
    }
    let api_version = text_at(&visual, &["apiVersion"]);
    if api_version.is_none() {
        warnings.push("the visual names no apiVersion".into());
    }
    if !skipped.is_empty() {
        warnings.push(format!(
            "{} file(s) outside package.json and resources/ (or with unusual names) are left out",
            skipped.len()
        ));
    }
    let metadata_item = metadata_path
        .get("resources/".len()..)
        .unwrap_or_default()
        .to_string();
    // package.json first, then resources/ by name
    files.sort_by(|a, b| (a.0 != "package.json", &a.0).cmp(&(b.0 != "package.json", &b.0)));
    Ok(Pbiviz {
        display_name: field("displayName"),
        version: field("version").or_else(|| text_at(&manifest, &["version"])),
        api_version,
        author: text_at(&visual, &["author", "name"]).or_else(|| text_at(&manifest, &["author", "name"])),
        description: field("description"),
        has_icon: has_content("iconBase64"),
        has_js,
        guid,
        files,
        metadata_item,
        skipped,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bicraft_opc::Package;
    use serde_json::json;

    const GUID: &str = "bicraftTiny0123456789ABCDEF";

    /// Our own tiny package, built from the public .pbiviz layout.
    fn package(extra: &[(&str, &[u8])]) -> Vec<u8> {
        let manifest = json!({
            "visual": {"name": "bicraftTiny", "displayName": "Tiny", "guid": GUID, "version": "1.0.0.0"},
            "author": {"name": "BIcraft tests"},
            "resources": [{"resourceId": "rId0", "sourceType": 5, "file": format!("resources/{GUID}.pbiviz.json")}]
        });
        let visual = json!({
            "visual": {"name": "bicraftTiny", "displayName": "Tiny", "guid": GUID, "version": "1.0.0.0"},
            "apiVersion": "5.3.0",
            "capabilities": {"dataRoles": [{"name": "value", "kind": "Measure"}]},
            "content": {"js": "var x=1;", "css": ".a{}", "iconBase64": "data:image/png;base64,AA=="}
        });
        let mut p = Package::default();
        p.put("package.json", manifest.to_string().into_bytes(), false);
        p.put(
            &format!("resources/{GUID}.pbiviz.json"),
            visual.to_string().into_bytes(),
            false,
        );
        for (n, b) in extra {
            p.put(n, b.to_vec(), false);
        }
        p.to_bytes().unwrap()
    }

    #[test]
    fn reads_our_package() {
        let v = read_pbiviz(&package(&[("readme.txt", b"hi")])).unwrap();
        assert_eq!(v.guid, GUID);
        assert_eq!(v.display_name.as_deref(), Some("Tiny"));
        assert_eq!(v.api_version.as_deref(), Some("5.3.0"));
        assert_eq!(v.author.as_deref(), Some("BIcraft tests"));
        assert!(v.has_icon && v.has_js);
        assert_eq!(v.metadata_item, format!("{GUID}.pbiviz.json"));
        assert_eq!(v.files[0].0, "package.json");
        assert_eq!(v.files[1].0, format!("resources/{GUID}.pbiviz.json"));
        assert_eq!(v.skipped, vec!["readme.txt".to_string()]);
    }

    #[test]
    fn refuses_hostile_and_broken_packages() {
        assert!(read_pbiviz(b"not a zip").unwrap_err().contains("not a .pbiviz"));
        // no manifest
        let mut p = Package::default();
        p.put("resources/x.pbiviz.json", b"{}".to_vec(), false);
        assert!(
            read_pbiviz(&p.to_bytes().unwrap())
                .unwrap_err()
                .contains("package.json")
        );
        // a guid that would leave its folder
        let mut p = Package::default();
        p.put(
            "package.json",
            br#"{"visual": {"guid": "../evil"}}"#.to_vec(),
            false,
        );
        assert!(read_pbiviz(&p.to_bytes().unwrap()).unwrap_err().contains("guid"));
        // the .pbiviz.json of another visual
        let mut p = Package::default();
        p.put("package.json", br#"{"visual": {"guid": "abc"}}"#.to_vec(), false);
        p.put(
            "resources/abc.pbiviz.json",
            br#"{"visual": {"guid": "xyz"}}"#.to_vec(),
            false,
        );
        assert!(read_pbiviz(&p.to_bytes().unwrap()).unwrap_err().contains("xyz"));
        // a zip bomb: a megabyte-plus entry that expands far past the ratio
        let bomb = vec![b'a'; 4 << 20];
        let err = read_pbiviz(&package(&[("resources/bomb.js", &bomb)])).unwrap_err();
        assert!(err.contains("ZIP bomb"), "{err}");
        // too large a file
        assert!(
            read_pbiviz(&vec![0u8; MAX_PBIVIZ_BYTES + 1])
                .unwrap_err()
                .contains("MB")
        );
    }

    #[test]
    fn unusual_names_are_left_out() {
        let v = read_pbiviz(&package(&[
            ("resources/a b.js", b"x"),
            ("resources/.hidden", b"x"),
        ]))
        .unwrap();
        assert_eq!(v.files.len(), 2);
        assert_eq!(v.skipped.len(), 2);
        assert!(kept_name("resources/../x").is_none());
        assert!(kept_name("RESOURCES/x.json").is_some());
        assert!(valid_guid("PBI_CV_9894B302_1DFF") && !valid_guid("a-b") && !valid_guid(""));
    }
}
