//! The packaging facts re-established from public and black-box evidence
//! (docs/formats/packaging-verification.md), encoded as structure checks on generated packages.
//! Each test names the fact ids of that note. None needs Power BI Desktop: the Desktop runs that
//! back them are `tools/packaging-oracle` (manual oracle steps).

use bicraft_model::SemanticModel;
use bicraft_opc::{Encoding, Package, decode_text};
use bicraft_pbixwriter::{
    Extras, ReportParts, check_package, write_pbit_documents, write_pbit_with, write_pbix,
};

const MODEL: &str = r#"{"name": "M", "compatibilityLevel": 1606, "model": {"culture": "en-US",
  "tables": [{"name": "T", "columns": [{"name": "V", "dataType": "int64", "sourceColumn": "V"}],
  "measures": [{"name": "Total", "expression": "SUM(T[V])"}]}]}}"#;

fn model() -> SemanticModel {
    bicraft_tmsl::read(MODEL).unwrap()
}

fn pbir_docs() -> Vec<(String, Vec<u8>)> {
    let s = "https://developer.microsoft.com/json-schemas/fabric/item/report/definition";
    let doc = |p: &str, v: String| (p.to_string(), v.into_bytes());
    vec![
        doc(
            "definition/version.json",
            format!(r#"{{"$schema":"{s}/versionMetadata/1.0.0/schema.json","version":"2.0.0"}}"#),
        ),
        doc(
            "definition/report.json",
            format!(r#"{{"$schema":"{s}/report/3.3.0/schema.json","themeCollection":{{}}}}"#),
        ),
        doc(
            "definition/pages/pages.json",
            format!(
                r#"{{"$schema":"{s}/pagesMetadata/1.1.0/schema.json","pageOrder":["p1"],"activePageName":"p1"}}"#
            ),
        ),
        doc(
            "definition/pages/p1/page.json",
            format!(
                r#"{{"$schema":"{s}/page/2.1.0/schema.json","name":"p1","displayName":"Page 1","displayOption":"FitToPage","height":720,"width":1280}}"#
            ),
        ),
    ]
}

fn pbit() -> Package {
    write_pbit_documents(&model(), &pbir_docs()).unwrap()
}

fn pbix() -> Package {
    write_pbix(
        vec![1, 2, 3, 4],
        ReportParts::Pbir(&pbir_docs()),
        &Extras::default(),
    )
    .unwrap()
}

fn text(pkg: &Package, name: &str) -> (String, Encoding) {
    let p = pkg.get(name).unwrap();
    decode_text(&p.name, &p.data).unwrap()
}

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// PK-ORDER, PK-COMP: `Version` first, `[Content_Types].xml` second (Desktop's own files);
/// `DataModel` stored, every other part deflated (Desktop's own files; Desktop also accepts a
/// stored part elsewhere, case z01).
#[test]
fn entry_order_and_compression_follow_desktop() {
    for pkg in [pbit(), pbix()] {
        let names: Vec<&str> = pkg.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names.first(), Some(&"Version"));
        assert_eq!(names.get(1), Some(&"[Content_Types].xml"));
        for p in &pkg.parts {
            assert_eq!(p.stored, p.name == "DataModel", "{}", p.name);
        }
    }
}

/// PK-CT: `[Content_Types].xml` as Desktop writes it: UTF-8 with BOM, XML declaration, one line,
/// a `Default` per extension with an empty type, an `Override` per extension-less part,
/// `application/json` for `Settings` and `Metadata`.
#[test]
fn content_types_have_desktop_shape() {
    let pkg = pbit();
    let ct = &pkg.get("[Content_Types].xml").unwrap().data;
    assert!(ct.starts_with(b"\xEF\xBB\xBF<?xml version=\"1.0\" encoding=\"utf-8\"?><Types "));
    let xml = std::str::from_utf8(ct.get(3..).unwrap()).unwrap();
    assert!(!xml.contains('\n'));
    assert!(xml.contains(r#"<Default Extension="json" ContentType="" />"#));
    for (part, ct) in [
        ("Version", ""),
        ("DataModelSchema", ""),
        ("DiagramLayout", ""),
        ("Settings", "application/json"),
        ("Metadata", "application/json"),
    ] {
        assert!(
            xml.contains(&format!(r#"<Override PartName="/{part}" ContentType="{ct}" />"#)),
            "{part}"
        );
    }
}

/// PK-ENC-16, PK-ENC-PBIR, PK-DMS: UTF-16LE without BOM for `Version`, `DataModelSchema`,
/// `DiagramLayout`, `Settings` and `Metadata`; UTF-8 without BOM, compact JSON for PBIR parts;
/// `DataModelSchema` with CRLF line ends and no trailing newline.
#[test]
fn text_encodings_follow_desktop() {
    let pkg = pbit();
    for name in [
        "Version",
        "DataModelSchema",
        "DiagramLayout",
        "Settings",
        "Metadata",
    ] {
        assert_eq!(text(&pkg, name).1, Encoding::Utf16Le, "{name}");
    }
    for p in pkg
        .parts
        .iter()
        .filter(|p| p.name.starts_with("Report/definition/"))
    {
        assert!(!p.data.starts_with(b"\xEF\xBB\xBF"), "{}", p.name);
        assert!(std::str::from_utf8(&p.data).is_ok(), "{}", p.name);
        assert!(!p.data.contains(&b'\n'), "{} is not compact", p.name);
    }
    let (schema, _) = text(&pkg, "DataModelSchema");
    assert!(schema.contains("\r\n") && !schema.replace("\r\n", "").contains('\n'));
    assert!(!schema.ends_with('\n'));
}

/// PK-VER: `/Version` text without newline; 1.32 with a PBIR report, 1.28 with a legacy one.
#[test]
fn version_values_follow_desktop() {
    assert_eq!(text(&pbit(), "Version").0, "1.32");
    let legacy = write_pbit_with(
        &model(),
        ReportParts::Legacy {
            layout: r#"{"config":"{}","layoutOptimization":0,"sections":[]}"#,
            resources: &[],
        },
        &Extras::default(),
    )
    .unwrap();
    assert_eq!(text(&legacy, "Version").0, "1.28");
}

/// PK-SETTINGS: inner versions 4 / 5 and a `QueriesSettings.Version` that Desktop accepts
/// (at least 1.0.3207.0, not newer than the opening Desktop).
#[test]
fn settings_and_metadata_versions() {
    let pkg = pbit();
    let settings: serde_json::Value = serde_json::from_str(&text(&pkg, "Settings").0).unwrap();
    assert_eq!(settings["Version"], 4);
    let q = settings["QueriesSettings"]["Version"].as_str().unwrap();
    let parts: Vec<u32> = q.split('.').map(|c| c.parse().unwrap()).collect();
    assert!(parts.as_slice() >= &[1, 0, 3207, 0][..] && parts.first() < Some(&99));
    let metadata: serde_json::Value = serde_json::from_str(&text(&pkg, "Metadata").0).unwrap();
    assert_eq!(metadata["Version"], 5);
}

/// Puts a part and gives it a content type the way Desktop's OPC layer needs (an `Override` for an
/// extension-less part, a `Default` for a new extension), so that only the fact under test differs.
fn put_typed(p: &mut Package, name: &str, data: Vec<u8>) {
    let fresh = p.get(name).is_none();
    p.put(name, data, false);
    if !fresh {
        return;
    }
    let entry = match name.rsplit('/').next().and_then(|n| n.rsplit_once('.')) {
        Some((_, ext)) => format!(r#"<Default Extension="{ext}" ContentType="" />"#),
        None => format!(r#"<Override PartName="/{name}" ContentType="" />"#),
    };
    let ct = p.get("[Content_Types].xml").unwrap().data.clone();
    let xml = String::from_utf8(ct)
        .unwrap()
        .replace("</Types>", &format!("{entry}</Types>"));
    p.put("[Content_Types].xml", xml.into_bytes(), false);
}

fn edit_json16(p: &mut Package, name: &str, f: impl Fn(&mut serde_json::Value)) {
    let mut v: serde_json::Value = serde_json::from_str(&text(p, name).0).unwrap();
    f(&mut v);
    p.put(name, utf16(&v.to_string()), false);
}

type Edit = Box<dyn Fn(&mut Package)>;

/// Each package Desktop refused in tools/packaging-oracle (case ids of packaging-verification.md)
/// gets the matching `check_package` message, and each one Desktop accepted gets none. A check
/// removed from `check_package` makes its refusal case fail here.
#[test]
fn check_package_agrees_with_desktop() {
    let refused: Vec<(&str, &str, Edit)> = vec![
        (
            "v02",
            "accepted values",
            Box::new(|p| p.put("Version", utf16("1.37"), false)),
        ),
        (
            "v03",
            "accepted values",
            Box::new(|p| p.put("Version", utf16("1.32.0"), false)),
        ),
        (
            "v04",
            "accepted values",
            Box::new(|p| p.put("Version", utf16(" 1.32"), false)),
        ),
        (
            "v05",
            "accepted values",
            Box::new(|p| p.put("Version", b"1.32".to_vec(), false)),
        ),
        (
            "v06",
            "`Version` is missing",
            Box::new(|p| p.parts.retain(|x| x.name != "Version")),
        ),
        (
            "v08",
            "accepted values",
            Box::new(|p| p.put("Version", utf16("1.32\r\n"), false)),
        ),
        (
            "k01",
            "`Version.txt` exists",
            Box::new(|p| put_typed(p, "Version.txt", utf16("1.32"))),
        ),
        (
            "d01",
            "must not both exist",
            Box::new(|p| put_typed(p, "DataModel", vec![0; 8])),
        ),
        (
            "l01",
            "must not coexist",
            Box::new(|p| put_typed(p, "Report/Layout", utf16("{}"))),
        ),
        (
            "s01",
            "`Settings`: `Version`",
            Box::new(|p| edit_json16(p, "Settings", |v| v["Version"] = 5.into())),
        ),
        (
            "s09",
            "`Settings`: `Version`",
            Box::new(|p| {
                edit_json16(p, "Settings", |v| {
                    v.as_object_mut().unwrap().remove("Version");
                })
            }),
        ),
        (
            "s02",
            "QueriesSettings.Version",
            Box::new(|p| {
                edit_json16(p, "Settings", |v| {
                    v["QueriesSettings"].as_object_mut().unwrap().remove("Version");
                })
            }),
        ),
        (
            "s03",
            "older than 1.0.3207.0",
            Box::new(|p| {
                edit_json16(p, "Settings", |v| {
                    v["QueriesSettings"]["Version"] = "1.0.3206.0".into();
                })
            }),
        ),
        (
            "s08",
            "`Settings`",
            Box::new(|p| {
                let t = text(p, "Settings").0;
                p.put("Settings", t.into_bytes(), false);
            }),
        ),
        (
            "e01",
            "`DataModelSchema`",
            Box::new(|p| {
                let t = text(p, "DataModelSchema").0;
                p.put("DataModelSchema", format!("{t} ").into_bytes(), false);
            }),
        ),
        (
            "e03",
            "`DiagramLayout`",
            Box::new(|p| {
                let t = text(p, "DiagramLayout").0;
                p.put("DiagramLayout", t.into_bytes(), false);
            }),
        ),
        (
            "m01",
            "`Metadata`: `Version`",
            Box::new(|p| edit_json16(p, "Metadata", |v| v["Version"] = 6.into())),
        ),
    ];
    for (id, expected, edit) in &refused {
        let mut p = pbit();
        edit(&mut p);
        let found = check_package(&p);
        assert!(found.iter().any(|m| m.contains(expected)), "{id}: {found:?}");
    }
    let accepted: Vec<(&str, Edit)> = vec![
        ("v01", Box::new(|p| p.put("Version", utf16("1.36"), false))),
        ("v07", Box::new(|p| p.put("Version", utf16("1.28"), false))),
        ("k02", Box::new(|p| put_typed(p, "foo.txt", b"x".to_vec()))),
        (
            "s04",
            Box::new(|p| {
                edit_json16(p, "Settings", |v| {
                    v["QueriesSettings"]["Version"] = "1.0.3207.0".into()
                })
            }),
        ),
        (
            "s06",
            Box::new(|p| {
                edit_json16(p, "Settings", |v| {
                    v["QueriesSettings"]["Version"] = "2.130".into()
                })
            }),
        ),
        ("s07", Box::new(|p| p.parts.retain(|x| x.name != "Settings"))),
        ("m02", Box::new(|p| p.parts.retain(|x| x.name != "Metadata"))),
        ("e02", Box::new(|p| p.parts.retain(|x| x.name != "DiagramLayout"))),
    ];
    for (id, edit) in &accepted {
        let mut p = pbit();
        edit(&mut p);
        let found = check_package(&p);
        // A removed part leaves its `Override` behind: harmless (System.IO.Packaging accepts it, §11).
        assert!(found.is_empty(), "{id}: {found:?}");
    }
}

/// Regression (packaging-verification.md PK-VER-PBIR, oracle case v07): Desktop opens a PBIR
/// package whose `/Version` is 1.28, so `check_package` no longer requires 1.32 for PBIR.
#[test]
fn pbir_with_version_1_28_passes_check_package() {
    let mut p = pbit();
    p.put("Version", utf16("1.28"), false);
    assert!(check_package(&p).is_empty(), "{:?}", check_package(&p));
}
