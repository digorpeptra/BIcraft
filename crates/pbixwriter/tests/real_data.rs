//! Structural comparison with Desktop-saved files from the local, never-committed test data
//! folder (`BICRAFT_TESTDATA`, see AGENTS.md). Only structure is compared and printed: part name
//! categories, content types, encodings, compression, `Version`. Part content is never printed.
//!
//! - `sample1/template.pbit`: the real model and report documents are fed back through
//!   `write_pbit_documents`, and the result must have the same structure (minus the parts this
//!   writer deliberately omits) and the same model.
//! - `sample1/report.pbix`: shared parts must have the same structure as ours.
//!
//! Without the variable or the files the tests print a note and pass.

use std::collections::BTreeMap;
use std::path::PathBuf;

use bicraft_opc::{Package, decode_text};
use bicraft_pbixwriter::{check_package, part, write_pbit_documents};

/// Parts Desktop writes that this writer omits on purpose.
const OMITTED: [&str; 1] = ["SecurityBindings"];

fn testdata(file: &str) -> Option<Package> {
    let Some(root) = bicraft_env::var_os("BICRAFT_TESTDATA").map(PathBuf::from) else {
        eprintln!("note: BICRAFT_TESTDATA not set; skipping");
        return None;
    };
    let path = root.join(file);
    if !path.is_file() {
        eprintln!("note: {file} not found in BICRAFT_TESTDATA; skipping");
        return None;
    }
    Some(Package::open(&path).unwrap())
}

/// A part name with every client-chosen segment replaced by `*`.
fn category(name: &str) -> String {
    let segs: Vec<&str> = name.split('/').collect();
    let ext = |s: &str| {
        s.rsplit_once('.')
            .map_or("*".to_string(), |(_, e)| format!("*.{e}"))
    };
    match segs.as_slice() {
        ["Report", "StaticResources", dirs @ .., file] => {
            let mut out = vec!["Report".to_string(), "StaticResources".to_string()];
            out.extend(dirs.iter().map(|d| d.to_string()));
            out.push(ext(file));
            out.join("/")
        }
        ["Report", "definition", "pages", _, "visuals", _, file] => {
            format!("Report/definition/pages/*/visuals/*/{file}")
        }
        ["Report", "definition", "pages", _, file] if *file != "pages.json" => {
            format!("Report/definition/pages/*/{file}")
        }
        ["Report", "definition", "bookmarks", file] => {
            format!("Report/definition/bookmarks/{}", ext(file))
        }
        _ => name.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Shape {
    content_type: String,
    stored: bool,
    /// `None` for binary resources.
    encoding: Option<String>,
}

fn shapes(pkg: &Package) -> (Vec<String>, BTreeMap<String, Shape>) {
    let ct = pkg.get(part::CONTENT_TYPES).unwrap();
    let ct_text = decode_text("ct", &ct.data).unwrap().0;
    let mut order = Vec::new();
    let mut map = BTreeMap::new();
    for p in &pkg.parts {
        let cat = category(&p.name);
        if !order.contains(&cat) {
            order.push(cat.clone());
        }
        let content_type = if p.name == part::CONTENT_TYPES {
            "-".to_string()
        } else {
            content_type_of(&ct_text, &p.name)
        };
        let text = !p.name.starts_with(part::STATIC_RESOURCES_PREFIX) && p.name != "SecurityBindings";
        let encoding = text.then(|| {
            decode_text(&p.name, &p.data)
                .map(|(_, e)| format!("{e:?}"))
                .unwrap_or_else(|_| "undecodable".into())
        });
        map.entry(cat).or_insert(Shape {
            content_type,
            stored: p.stored,
            encoding,
        });
    }
    (order, map)
}

/// Content type of a part from the XML text (Override first, then Default by extension).
fn content_type_of(xml: &str, name: &str) -> String {
    let attr = |tag: &str, key: &str| {
        let start = tag.find(&format!("{key}=\""))? + key.len() + 2;
        let len = tag.get(start..)?.find('"')?;
        tag.get(start..start + len).map(str::to_string)
    };
    let tags: Vec<&str> = xml.split('<').collect();
    for t in &tags {
        if t.starts_with("Override") && attr(t, "PartName").as_deref() == Some(&format!("/{name}")) {
            return attr(t, "ContentType").unwrap_or_default();
        }
    }
    let ext = name
        .rsplit('/')
        .next()
        .and_then(|f| f.rsplit_once('.'))
        .map(|(_, e)| e);
    for t in &tags {
        if t.starts_with("Default") && attr(t, "Extension").as_deref() == ext {
            return attr(t, "ContentType").unwrap_or_default();
        }
    }
    "(none)".into()
}

fn version(pkg: &Package) -> String {
    decode_text("Version", &pkg.get("Version").unwrap().data)
        .unwrap()
        .0
}

/// Report documents of a package, relative to the report root.
fn report_docs(pkg: &Package) -> Vec<(String, Vec<u8>)> {
    pkg.parts
        .iter()
        .filter_map(|p| {
            p.name
                .strip_prefix(part::REPORT_PREFIX)
                .filter(|rel| rel.starts_with("definition/") || rel.starts_with("StaticResources/"))
                .map(|rel| (rel.to_string(), p.data.clone()))
        })
        .collect()
}

/// Prints and returns structural differences between `ours` and `real`, by category.
fn compare(label: &str, ours: &Package, real: &Package, only_shared: bool) -> Vec<String> {
    let (our_order, our_shapes) = shapes(ours);
    let (real_order, real_shapes) = shapes(real);
    let mut diffs = Vec::new();
    for cat in &real_order {
        match (our_shapes.get(cat), real_shapes.get(cat)) {
            (None, Some(_)) if OMITTED.contains(&cat.as_str()) => {
                eprintln!("[{label}] omitted on purpose: {cat}");
            }
            (None, Some(_)) if !only_shared => diffs.push(format!("missing category: {cat}")),
            (Some(a), Some(b)) if a != b => diffs.push(format!("{cat}: ours {a:?} vs Desktop {b:?}")),
            _ => {}
        }
    }
    for cat in &our_order {
        if !real_shapes.contains_key(cat) && !only_shared {
            diffs.push(format!("extra category: {cat}"));
        }
    }
    let shared = |order: &[String], other: &BTreeMap<String, Shape>| -> Vec<String> {
        order.iter().filter(|c| other.contains_key(*c)).cloned().collect()
    };
    if shared(&our_order, &real_shapes) != shared(&real_order, &our_shapes) {
        diffs.push("category order differs".into());
    }
    if version(ours) != version(real) {
        diffs.push(format!(
            "Version: ours {} vs Desktop {}",
            version(ours),
            version(real)
        ));
    }
    for d in &diffs {
        eprintln!("[{label}] difference: {d}");
    }
    eprintln!(
        "[{label}] {} categories compared, {} differences",
        real_order.len(),
        diffs.len()
    );
    diffs
}

#[test]
fn real_pbit_passes_check_package() {
    let Some(real) = testdata("sample1/template.pbit") else {
        return;
    };
    let v = check_package(&real);
    assert!(v.is_empty(), "{} loader-rule violations", v.len());
}

#[test]
fn real_pbix_passes_check_package() {
    let Some(real) = testdata("sample1/report.pbix") else {
        return;
    };
    let v = check_package(&real);
    assert!(v.is_empty(), "{} loader-rule violations", v.len());
}

#[test]
fn rebuilt_pbit_matches_desktop_structure() {
    let Some(real) = testdata("sample1/template.pbit") else {
        return;
    };
    let schema = real.get(part::DATA_MODEL_SCHEMA).unwrap();
    let model = bicraft_tmsl::read(&decode_text("s", &schema.data).unwrap().0).unwrap();
    let ours = write_pbit_documents(&model, &report_docs(&real)).unwrap();
    let ours = Package::read_bytes(&ours.to_bytes().unwrap()).unwrap();

    let diffs = compare("pbit", &ours, &real, false);
    assert!(diffs.is_empty(), "{} structural differences", diffs.len());

    // `[Content_Types].xml`: identical apart from the omitted parts' overrides.
    let ct = |p: &Package| decode_text("ct", &p.get(part::CONTENT_TYPES).unwrap().data).unwrap();
    let (real_ct, real_enc) = ct(&real);
    let (our_ct, our_enc) = ct(&ours);
    assert_eq!(our_enc, real_enc);
    let mut expected = real_ct;
    for omitted in OMITTED {
        expected = expected.replace(
            &format!(r#"<Override PartName="/{omitted}" ContentType="" />"#),
            "",
        );
    }
    assert!(our_ct == expected, "[Content_Types].xml differs");

    // Same model, same report documents.
    let back = ours.get(part::DATA_MODEL_SCHEMA).unwrap();
    let back = bicraft_tmsl::read(&decode_text("s", &back.data).unwrap().0).unwrap();
    assert!(back == model, "model differs after the round trip");
    assert!(
        report_docs(&ours) == report_docs(&real),
        "report documents differ"
    );
}

#[test]
fn shared_parts_match_desktop_pbix_structure() {
    let Some(real) = testdata("sample1/report.pbix") else {
        return;
    };
    let Some(template) = testdata("sample1/template.pbit") else {
        return;
    };
    let schema = template.get(part::DATA_MODEL_SCHEMA).unwrap();
    let model = bicraft_tmsl::read(&decode_text("s", &schema.data).unwrap().0).unwrap();
    let ours = write_pbit_documents(&model, &report_docs(&real)).unwrap();
    let diffs = compare("pbix", &ours, &real, true);
    assert!(diffs.is_empty(), "{} structural differences", diffs.len());
}
