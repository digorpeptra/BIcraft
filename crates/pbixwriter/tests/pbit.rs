//! PBIT / thin PBIX round trips on synthetic inputs. `bicraft-pbix` is on the same layer (L4), so
//! these tests re-derive what its `inventory` / `read_model` report from the package directly.

use bicraft_model::SemanticModel;
use bicraft_opc::{Encoding, Package, decode_text};
use bicraft_pbixwriter::{
    Error, RemoteModel, check_package, part, write_pbit, write_pbit_documents, write_thin_pbix,
    write_thin_pbix_documents,
};
use bicraft_report::{Page, Report};

const MODEL: &str = r#"{
  "name": "Sales Model",
  "compatibilityLevel": 1567,
  "model": {
    "culture": "en-US",
    "tables": [
      {
        "name": "Sales",
        "columns": [
          {"name": "Amount", "dataType": "decimal", "sourceColumn": "Amount"},
          {"name": "ProductId", "dataType": "int64", "sourceColumn": "ProductId"}
        ],
        "measures": [{"name": "Total", "expression": "SUM(Sales[Amount])"}],
        "partitions": [{"name": "Sales", "mode": "import", "source": {"type": "m", "expression": ["let", "    Source = 1", "in", "    Source"]}}]
      },
      {
        "name": "Product",
        "columns": [{"name": "ProductId", "dataType": "int64", "sourceColumn": "ProductId"}]
      }
    ],
    "relationships": [
      {"name": "r1", "fromTable": "Sales", "fromColumn": "ProductId", "toTable": "Product", "toColumn": "ProductId"}
    ]
  }
}"#;

fn model() -> SemanticModel {
    bicraft_tmsl::read(MODEL).unwrap()
}

/// A minimal hand-made PBIR document set (paths relative to the report root).
fn report_docs() -> Vec<(String, Vec<u8>)> {
    let s = "https://developer.microsoft.com/json-schemas/fabric/item/report/definition";
    let doc = |p: &str, v: String| (p.to_string(), v.into_bytes());
    vec![
        doc(
            "definition/pages/p1/visuals/v1/visual.json",
            format!(
                r#"{{"$schema":"{s}/visualContainer/2.0.0/schema.json","name":"v1","position":{{"x":0,"y":0,"z":0,"width":300,"height":200}},"visual":{{"visualType":"card"}}}}"#
            ),
        ),
        doc(
            "definition/pages/p1/page.json",
            format!(
                r#"{{"$schema":"{s}/page/2.1.0/schema.json","name":"p1","displayName":"Page 1","displayOption":"FitToPage","height":720,"width":1280}}"#
            ),
        ),
        doc(
            "definition/pages/pages.json",
            format!(
                r#"{{"$schema":"{s}/pagesMetadata/1.1.0/schema.json","pageOrder":["p1"],"activePageName":"p1"}}"#
            ),
        ),
        doc(
            "definition/report.json",
            format!(r#"{{"$schema":"{s}/report/3.3.0/schema.json","themeCollection":{{}}}}"#),
        ),
        doc(
            "definition/version.json",
            format!(r#"{{"$schema":"{s}/versionMetadata/1.0.0/schema.json","version":"2.0.0"}}"#),
        ),
        (
            "StaticResources/RegisteredResources/logo 1.png".to_string(),
            vec![0x89, b'P', b'N', b'G', 0, 0, 0xFF],
        ),
    ]
}

fn roundtrip(pkg: &Package) -> Package {
    Package::read_bytes(&pkg.to_bytes().unwrap()).unwrap()
}

fn names(pkg: &Package) -> Vec<&str> {
    pkg.parts.iter().map(|p| p.name.as_str()).collect()
}

fn encoding(pkg: &Package, name: &str) -> Encoding {
    let p = pkg.get(name).unwrap();
    decode_text(&p.name, &p.data).unwrap().1
}

#[test]
fn pbit_round_trip() {
    let input = model();
    let pkg = roundtrip(&write_pbit_documents(&input, &report_docs()).unwrap());

    assert_eq!(
        names(&pkg),
        [
            "Version",
            "[Content_Types].xml",
            "DataModelSchema",
            "DiagramLayout",
            "Settings",
            "Metadata",
            "Report/StaticResources/RegisteredResources/logo%201.png",
            "Report/definition/version.json",
            "Report/definition/report.json",
            "Report/definition/pages/pages.json",
            "Report/definition/pages/p1/visuals/v1/visual.json",
            "Report/definition/pages/p1/page.json",
        ]
    );
    // What `bicraft_pbix::inventory` reports: kind Pbit, report format Pbir.
    assert!(pkg.get(part::DATA_MODEL_SCHEMA).is_some());
    assert!(pkg.get(part::DATA_MODEL).is_none());
    assert!(pkg.get(part::LEGACY_LAYOUT).is_none());
    assert!(pkg.parts.iter().any(|p| p.name.starts_with(part::PBIR_PREFIX)));
    assert!(pkg.get(part::SECURITY_BINDINGS).is_none());
    assert!(pkg.get(part::CONNECTIONS).is_none());

    // What `bicraft_pbix::read_model` returns: the input model.
    let schema = pkg.get(part::DATA_MODEL_SCHEMA).unwrap();
    let (text, _) = decode_text(&schema.name, &schema.data).unwrap();
    assert_eq!(bicraft_tmsl::read(&text).unwrap(), input);

    let (version, _) = decode_text("Version", &pkg.get("Version").unwrap().data).unwrap();
    assert_eq!(version, "1.32");

    // Encodings.
    for name in [
        "Version",
        "DataModelSchema",
        "DiagramLayout",
        "Settings",
        "Metadata",
    ] {
        assert_eq!(encoding(&pkg, name), Encoding::Utf16Le, "{name}");
    }
    assert_eq!(encoding(&pkg, "[Content_Types].xml"), Encoding::Utf8Bom);
    for p in pkg.parts.iter().filter(|p| p.name.starts_with(part::PBIR_PREFIX)) {
        assert_eq!(encoding(&pkg, &p.name), Encoding::Utf8, "{}", p.name);
    }
    // Report documents are carried byte for byte.
    for (path, bytes) in report_docs() {
        let name = format!("Report/{}", path.replace(' ', "%20"));
        assert_eq!(pkg.get(&name).unwrap().data, bytes, "{name}");
    }

    // Deflate everywhere.
    assert!(pkg.parts.iter().all(|p| !p.stored));

    // Every part is in [Content_Types].xml, and the package passes the loader rules.
    let (ct, _) = decode_text("ct", &pkg.get("[Content_Types].xml").unwrap().data).unwrap();
    for name in ["Version", "DataModelSchema", "DiagramLayout"] {
        assert!(
            ct.contains(&format!(r#"<Override PartName="/{name}" ContentType="" />"#)),
            "{name}"
        );
    }
    for name in ["Settings", "Metadata"] {
        assert!(ct.contains(&format!(
            r#"<Override PartName="/{name}" ContentType="application/json" />"#
        )));
    }
    assert!(ct.contains(r#"<Default Extension="png" ContentType="" />"#));
    assert!(ct.contains(r#"<Default Extension="json" ContentType="" />"#));
    assert_eq!(check_package(&pkg), Vec::<String>::new());
}

#[test]
fn pbit_bytes_are_deterministic_with_fixed_timestamps() {
    let a = write_pbit_documents(&model(), &report_docs())
        .unwrap()
        .to_bytes()
        .unwrap();
    let b = write_pbit_documents(&model(), &report_docs())
        .unwrap()
        .to_bytes()
        .unwrap();
    assert_eq!(a, b);
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(a)).unwrap();
    for i in 0..zip.len() {
        let f = zip.by_index(i).unwrap();
        let t = f.last_modified().unwrap();
        assert_eq!(
            (t.year(), t.month(), t.day(), t.hour(), t.minute(), t.second()),
            (1980, 1, 1, 0, 0, 0),
            "{}",
            f.name().unwrap()
        );
        assert_eq!(f.compression(), zip::CompressionMethod::Deflated);
    }
}

#[test]
fn unnamed_model_gets_a_name() {
    let mut m = model();
    m.name.clear();
    let pkg = write_pbit_documents(&m, &report_docs()).unwrap();
    let schema = pkg.get(part::DATA_MODEL_SCHEMA).unwrap();
    let (text, _) = decode_text("s", &schema.data).unwrap();
    let back = bicraft_tmsl::read(&text).unwrap();
    assert_eq!(back.name.len(), 36);
    assert_eq!(back.tables, m.tables);
}

#[test]
fn a_report_without_pbir_documents_is_refused() {
    let only_resources = vec![("StaticResources/x.png".to_string(), vec![1, 2])];
    assert!(matches!(
        write_pbit_documents(&model(), &only_resources),
        Err(Error::Unsupported(_))
    ));
    // A page-less PBIR set breaks the loader rule.
    let no_page: Vec<_> = report_docs()
        .into_iter()
        .filter(|(p, _)| !p.starts_with("definition/pages/p1/"))
        .collect();
    assert!(matches!(
        write_pbit_documents(&model(), &no_page),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn pbit_from_report_model() {
    let report = Report {
        pages: vec![Page {
            name: "p1".into(),
            display_name: "Page 1".into(),
            width: 1280.0,
            height: 720.0,
            ..Default::default()
        }],
        ..Default::default()
    };
    let pkg = roundtrip(&write_pbit(&model(), &report).unwrap());
    assert!(pkg.get("Report/definition/pages/p1/page.json").is_some());
    assert_eq!(check_package(&pkg), Vec::<String>::new());
}

fn remote() -> RemoteModel {
    RemoteModel {
        connection: "powerbi://api.powerbi.com/v1.0/myorg/Finance".into(),
        model: "Sales Model".into(),
    }
}

#[test]
fn thin_pbix_round_trip() {
    let pkg = roundtrip(&write_thin_pbix_documents(&report_docs(), &remote()).unwrap());
    assert_eq!(
        names(&pkg).get(..5).unwrap(),
        [
            "Version",
            "[Content_Types].xml",
            "Connections",
            "Settings",
            "Metadata"
        ]
    );
    // Kind Thin: no model part at all.
    assert!(pkg.get(part::DATA_MODEL).is_none());
    assert!(pkg.get(part::DATA_MODEL_SCHEMA).is_none());
    assert!(pkg.get(part::DIAGRAM_LAYOUT).is_none());

    assert_eq!(encoding(&pkg, "Connections"), Encoding::Utf8);
    let conn: serde_json::Value = serde_json::from_slice(&pkg.get("Connections").unwrap().data).unwrap();
    assert_eq!(conn["Version"], 4);
    assert_eq!(
        conn["Connections"][0]["ConnectionType"],
        "pbiServiceXmlaStyleLive"
    );
    assert_eq!(
        conn["Connections"][0]["ConnectionString"],
        "Data Source=powerbi://api.powerbi.com/v1.0/myorg/Finance;Initial Catalog=Sales Model"
    );
    let (version, _) = decode_text("Version", &pkg.get("Version").unwrap().data).unwrap();
    assert_eq!(version, "1.32");
    let (ct, _) = decode_text("ct", &pkg.get("[Content_Types].xml").unwrap().data).unwrap();
    assert!(ct.contains(r#"<Override PartName="/Connections" ContentType="" />"#));
    assert_eq!(check_package(&pkg), Vec::<String>::new());
}

#[test]
fn thin_pbix_rejects_a_bad_remote() {
    let bad = RemoteModel {
        connection: "Data Source=x".into(),
        model: "m".into(),
    };
    assert!(write_thin_pbix_documents(&report_docs(), &bad).is_err());
}

#[test]
fn thin_pbix_from_report_model() {
    let report = Report {
        pages: vec![Page {
            name: "p1".into(),
            display_name: "Page 1".into(),
            width: 1280.0,
            height: 720.0,
            ..Default::default()
        }],
        ..Default::default()
    };
    let pkg = write_thin_pbix(&report, &remote()).unwrap();
    assert_eq!(check_package(&pkg), Vec::<String>::new());
}

// ---- check_package on broken packages ----

fn valid() -> Package {
    write_pbit_documents(&model(), &report_docs()).unwrap()
}

fn violations_after(edit: impl FnOnce(&mut Package)) -> Vec<String> {
    let mut pkg = valid();
    edit(&mut pkg);
    check_package(&pkg)
}

fn utf16(s: &str) -> Vec<u8> {
    bicraft_opc::encode_text(s, Encoding::Utf16Le)
}

type Edit = Box<dyn FnOnce(&mut Package)>;

#[test]
fn check_package_reports_each_loader_rule() {
    let cases: Vec<(&str, Edit)> = vec![
        (
            "Version` is missing",
            Box::new(|p| p.parts.retain(|x| x.name != "Version")),
        ),
        (
            "accepted values",
            Box::new(|p| p.put("Version", utf16("1.37"), false)),
        ),
        (
            "must not start with a BOM",
            Box::new(|p| p.put("Version", [&[0xFF, 0xFE][..], &utf16("1.32")].concat(), false)),
        ),
        ("both exist", Box::new(|p| p.put("DataModel", vec![0; 8], true))),
        (
            "must not coexist",
            Box::new(|p| p.put("Report/Layout", utf16("{}"), false)),
        ),
        (
            "`Version.txt` exists",
            Box::new(|p| p.put("Version.txt", vec![], false)),
        ),
        (
            "larger than 2 MiB",
            Box::new(|p| p.put("docProps/custom.xml", vec![b' '; 2 * 1024 * 1024 + 1], false)),
        ),
        (
            "`DataMashup` and `UnappliedChanges`",
            Box::new(|p| {
                p.put("DataMashup", vec![0; 20], false);
                p.put("UnappliedChanges", utf16(r#"{"version":"1.0"}"#), false);
            }),
        ),
        ("no content type", Box::new(|p| p.put("Extra", vec![], false))),
        (
            "`[Content_Types].xml` is missing",
            Box::new(|p| p.parts.retain(|x| x.name != "[Content_Types].xml")),
        ),
        (
            "`Settings`: `Version`",
            Box::new(|p| p.put("Settings", utf16(r#"{"Version":5}"#), false)),
        ),
        (
            "`QueriesSettings.Version` (the Power Query package version) is missing",
            Box::new(|p| p.put("Settings", utf16(r#"{"Version":4,"QueriesSettings":{}}"#), false)),
        ),
        (
            "older than 1.0.3207.0",
            Box::new(|p| {
                p.put(
                    "Settings",
                    utf16(r#"{"Version":4,"QueriesSettings":{"Version":"1.0.1"}}"#),
                    false,
                )
            }),
        ),
        (
            "`Metadata`: invalid JSON",
            Box::new(|p| p.put("Metadata", br#"{"Version":5} "#.to_vec(), false)),
        ),
        (
            "has no page",
            Box::new(|p| p.parts.retain(|x| !x.name.contains("/p1/"))),
        ),
        (
            "UTF-8 text must not start with a BOM",
            Box::new(|p| p.put("Report/definition/report.json", b"\xEF\xBB\xBF{}".to_vec(), false)),
        ),
        (
            "duplicate part name",
            Box::new(|p| {
                let mut dup = p.get("Settings").unwrap().clone();
                dup.name = "SETTINGS".into();
                p.parts.push(dup);
            }),
        ),
    ];
    for (expected, edit) in cases {
        let v = violations_after(edit);
        assert!(
            v.iter().any(|m| m.contains(expected)),
            "expected `{expected}` in {v:?}"
        );
    }
}

/// Rules no black-box run settled are not enforced: the `Connections` version range, the encoding
/// of `DiagramState` and `Report/MobileState`, and `<part>.txt` siblings other than `Version.txt`
/// (packaging-verification.md PK-CONN, PK-ENC-MINOR, PK-TXT).
#[test]
fn check_package_leaves_unverified_rules_alone() {
    let v = violations_after(|p| {
        p.put("Connections", br#"{"Version":7}"#.to_vec(), false);
        p.put("DiagramState", b"{}".to_vec(), false);
        p.put("Report/MobileState", b"{}".to_vec(), false);
        p.put("DataModelSchema.txt", vec![], false);
    });
    for name in [
        "Connections",
        "DiagramState",
        "MobileState",
        "DataModelSchema.txt",
    ] {
        assert!(
            !v.iter()
                .any(|m| m.contains(name) && !m.contains("no content type")),
            "{name}: {v:?}"
        );
    }
}

#[test]
fn check_package_never_panics_on_garbage() {
    let mut pkg = Package::default();
    for name in [
        "Version",
        "[Content_Types].xml",
        "Settings",
        "Metadata",
        "Connections",
        "UnappliedChanges",
        "DataMashup",
        "Report/definition/x.json",
        "DataModelSchema",
    ] {
        pkg.put(name, vec![0xFF, 0xD8, 0x00], false);
    }
    assert!(!check_package(&pkg).is_empty());
    assert!(!check_package(&Package::default()).is_empty());
}
