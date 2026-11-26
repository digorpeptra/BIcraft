use super::*;
use crate::tempdir::TempDir;
use crate::{ReportFormat, read_model};

const BIM: &str = r#"{
  "name": "Sales",
  "compatibilityLevel": 1567,
  "model": {
    "culture": "en-US",
    "tables": [
      {
        "name": "Order Lines",
        "columns": [
          { "name": "Qty", "dataType": "int64", "sourceColumn": "Qty" },
          { "name": "Product", "dataType": "string", "sourceColumn": "Product" }
        ],
        "measures": [ { "name": "Units", "expression": "SUM ( 'Order Lines'[Qty] )" } ]
      },
      {
        "name": "Product",
        "columns": [ { "name": "Product", "dataType": "string", "sourceColumn": "Product" } ]
      }
    ],
    "relationships": [
      {
        "name": "r1",
        "fromTable": "Order Lines",
        "fromColumn": "Product",
        "toTable": "Product",
        "toColumn": "Product"
      }
    ]
  }
}"#;

fn sample_model() -> SemanticModel {
    bicraft_tmsl::read(BIM).unwrap()
}

fn fixed_ids() -> ProjectIds {
    ProjectIds {
        semantic_model_logical_id: "11111111-2222-4333-8444-555555555555".into(),
        report_logical_id: "66666666-7777-4888-9999-aaaaaaaaaaaa".into(),
        page_name: "0123456789abcdef0123".into(),
    }
}

/// A guarded temp folder and a path inside it that does not exist yet (`create_project` makes it).
fn temp_dir(tag: &str) -> (TempDir, PathBuf) {
    let tmp = TempDir::new(&format!("pbip-new-{tag}"));
    let dir = tmp.join("out");
    (tmp, dir)
}

fn read_json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Every file under `dir`, as paths relative to it with `/` separators, sorted.
fn files_under(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                let rel = path
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push(rel);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

fn assert_schema(value: &Value, key: &str) {
    let expected = format!("https://developer.microsoft.com/json-schemas/fabric/{key}");
    assert_eq!(value["$schema"], expected.as_str(), "{value}");
}

/// The files of a fresh TMSL project, the required fields of each public schema they declare
/// (checked by hand: tests stay offline and vendor no Microsoft schema), and `locate`.
#[test]
fn scaffolds_a_tmsl_project() {
    let (_tmp_dir, dir) = temp_dir("tmsl");
    let project =
        create_project_with_ids(&dir, "Sales", &sample_model(), ModelFormat::Tmsl, &fixed_ids()).unwrap();
    assert_eq!(
        files_under(&dir),
        [
            ".gitignore",
            "Sales.Report/.platform",
            "Sales.Report/definition.pbir",
            "Sales.Report/definition/pages/0123456789abcdef0123/page.json",
            "Sales.Report/definition/pages/pages.json",
            "Sales.Report/definition/report.json",
            "Sales.Report/definition/version.json",
            "Sales.SemanticModel/.platform",
            "Sales.SemanticModel/definition.pbism",
            "Sales.SemanticModel/model.bim",
            "Sales.pbip",
        ]
    );
    assert_eq!(project.root, dir);
    assert_eq!(project.model_format, ModelFormat::Tmsl);
    assert_eq!(project.report_format, ReportFormat::Pbir);
    assert_eq!(
        project.semantic_model.as_deref(),
        Some(dir.join("Sales.SemanticModel").as_path())
    );
    assert_eq!(
        project.report.as_deref(),
        Some(dir.join("Sales.Report").as_path())
    );

    let pbip = read_json(&dir.join("Sales.pbip"));
    assert_schema(&pbip, "pbip/pbipProperties/1.0.0/schema.json");
    assert_eq!(pbip["version"], "1.0");
    assert_eq!(
        pbip["artifacts"],
        json!([{ "report": { "path": "Sales.Report" } }])
    );
    assert_eq!(pbip["settings"], json!({ "enableAutoRecovery": true }));

    let pbism = read_json(&dir.join("Sales.SemanticModel/definition.pbism"));
    assert_schema(
        &pbism,
        "item/semanticModel/definitionProperties/1.0.0/schema.json",
    );
    assert_eq!(pbism["version"], "4.2");

    let pbir = read_json(&dir.join("Sales.Report/definition.pbir"));
    assert_schema(&pbir, "item/report/definitionProperties/2.0.0/schema.json");
    assert_eq!(pbir["version"], "4.0");
    assert_eq!(
        pbir["datasetReference"],
        json!({ "byPath": { "path": "../Sales.SemanticModel" } })
    );

    for (item, kind, id) in [
        (
            "Sales.SemanticModel",
            "SemanticModel",
            "11111111-2222-4333-8444-555555555555",
        ),
        ("Sales.Report", "Report", "66666666-7777-4888-9999-aaaaaaaaaaaa"),
    ] {
        let platform = read_json(&dir.join(item).join(".platform"));
        assert_schema(&platform, "gitIntegration/platformProperties/2.0.0/schema.json");
        assert_eq!(
            platform["metadata"],
            json!({ "type": kind, "displayName": "Sales" })
        );
        assert_eq!(platform["config"], json!({ "version": "2.0", "logicalId": id }));
    }

    let def = dir.join("Sales.Report/definition");
    let version = read_json(&def.join("version.json"));
    assert_schema(
        &version,
        "item/report/definition/versionMetadata/1.0.0/schema.json",
    );
    assert_eq!(version["version"], "2.0.0");
    let report = read_json(&def.join("report.json"));
    assert_schema(&report, "item/report/definition/report/3.3.0/schema.json");
    assert!(report["themeCollection"].is_object());
    let pages = read_json(&def.join("pages/pages.json"));
    assert_schema(&pages, "item/report/definition/pagesMetadata/1.1.0/schema.json");
    assert_eq!(pages["pageOrder"], json!(["0123456789abcdef0123"]));
    assert_eq!(pages["activePageName"], "0123456789abcdef0123");
    let page = read_json(&def.join("pages/0123456789abcdef0123/page.json"));
    assert_schema(&page, "item/report/definition/page/2.1.0/schema.json");
    for required in ["name", "displayName", "displayOption"] {
        assert!(page[required].is_string(), "page.json lacks {required}");
    }
    assert_eq!(page["name"], "0123456789abcdef0123");

    assert_eq!(
        std::fs::read_to_string(dir.join(".gitignore")).unwrap(),
        "**/.pbi/localSettings.json\n**/.pbi/cache.abf\n**/.pbi/bicraft-data.bin\n"
    );
    // `locate` from every entry point finds the same items.
    for start in [dir.join("Sales.pbip"), dir.clone(), dir.join("Sales.Report")] {
        let found = locate(&start).unwrap();
        assert_eq!(found.semantic_model, project.semantic_model);
        assert_eq!(found.model_format, ModelFormat::Tmsl);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scaffolds_a_tmdl_project() {
    let (_tmp_dir, dir) = temp_dir("tmdl");
    let project =
        create_project_with_ids(&dir, "Shop", &sample_model(), ModelFormat::Tmdl, &fixed_ids()).unwrap();
    let model_dir = dir.join("Shop.SemanticModel");
    assert!(model_dir.join("definition").is_dir());
    assert!(!model_dir.join("model.bim").exists());
    assert!(model_dir.join("definition.pbism").is_file());
    assert!(model_dir.join(".platform").is_file());
    assert!(dir.join("Shop.Report/definition.pbir").is_file());
    assert_eq!(project.model_format, ModelFormat::Tmdl);
    assert_eq!(project.report_format, ReportFormat::Pbir);
    let _ = std::fs::remove_dir_all(&dir);
}

/// All metadata JSON is UTF-8 without BOM, LF only, and byte-identical between runs with the
/// same ids, on Windows and Linux alike.
#[test]
fn metadata_is_utf8_lf_without_bom_and_deterministic() {
    let (_tmp_a, a) = temp_dir("det-a");
    let (_tmp_b, b) = temp_dir("det-b");
    for dir in [&a, &b] {
        create_project_with_ids(
            dir,
            "Ventes été",
            &sample_model(),
            ModelFormat::Tmdl,
            &fixed_ids(),
        )
        .unwrap();
    }
    let files = files_under(&a);
    assert_eq!(files, files_under(&b));
    let metadata: Vec<&String> = files
        .iter()
        .filter(|f| !f.contains("SemanticModel/definition/"))
        .collect();
    assert_eq!(metadata.len(), 10, "{metadata:?}");
    for file in metadata {
        let bytes = std::fs::read(a.join(file)).unwrap();
        assert_eq!(
            bytes,
            std::fs::read(b.join(file)).unwrap(),
            "{file} differs between runs"
        );
        assert!(!bytes.starts_with(&[0xEF, 0xBB, 0xBF]), "{file} has a BOM");
        assert!(!bytes.contains(&b'\r'), "{file} has a CR");
        let text = String::from_utf8(bytes).unwrap();
        if file != ".gitignore" {
            serde_json::from_str::<Value>(&text).unwrap();
        }
    }
    let platform = read_json(&a.join("Ventes été.SemanticModel/.platform"));
    assert_eq!(platform["metadata"]["displayName"], "Ventes été");
    for dir in [a, b] {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Golden text of the files Desktop is most sensitive to.
#[test]
fn pbip_and_pbir_text_is_stable() {
    let (_tmp_dir, dir) = temp_dir("golden");
    create_project_with_ids(&dir, "Sales", &sample_model(), ModelFormat::Tmsl, &fixed_ids()).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("Sales.pbip")).unwrap(),
        r#"{
  "$schema": "https://developer.microsoft.com/json-schemas/fabric/pbip/pbipProperties/1.0.0/schema.json",
  "version": "1.0",
  "artifacts": [
    {
      "report": {
        "path": "Sales.Report"
      }
    }
  ],
  "settings": {
    "enableAutoRecovery": true
  }
}"#
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("Sales.Report/definition.pbir")).unwrap(),
        r#"{
  "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definitionProperties/2.0.0/schema.json",
  "version": "4.0",
  "datasetReference": {
    "byPath": {
      "path": "../Sales.SemanticModel"
    }
  }
}"#
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rejects_bad_names() {
    for bad in [
        "",
        "   ",
        "a/b",
        "a\\b",
        "..",
        "a..b",
        ".",
        "x:y",
        "x*",
        "x?",
        "x\"y",
        "x<y",
        "x>y",
        "x|y",
        "tab\there",
        "trailing.",
        "trailing ",
        " leading",
        "CON",
        "con.txt",
        "Nul",
        "COM1",
        "lpt9",
    ] {
        let err = validate_project_name(bad).unwrap_err();
        assert!(matches!(err, Error::InvalidName { .. }), "{bad:?}: {err}");
    }
    let long = "x".repeat(200);
    assert!(validate_project_name(&long).is_err());
    for good in [
        "Sales",
        "Sales 2026",
        "Ventes été",
        "a.b",
        "COM10",
        "Console",
        "x-y_z (1)",
    ] {
        validate_project_name(good).unwrap_or_else(|e| panic!("{good:?}: {e}"));
    }
    let (_tmp_dir, dir) = temp_dir("badname");
    let err = create_project(&dir, "../escape", &sample_model(), ModelFormat::Tmsl).unwrap_err();
    assert!(matches!(err, Error::InvalidName { .. }), "{err}");
    assert!(!dir.exists(), "nothing is created for an invalid name");
}

#[test]
fn rejects_bad_ids_and_the_missing_format() {
    let (_tmp_dir, dir) = temp_dir("badids");
    let mut ids = fixed_ids();
    ids.page_name = "../x".into();
    let err = create_project_with_ids(&dir, "S", &sample_model(), ModelFormat::Tmsl, &ids).unwrap_err();
    assert!(matches!(err, Error::InvalidId { .. }), "{err}");
    let mut ids = fixed_ids();
    ids.report_logical_id = "not-a-guid".into();
    let err = create_project_with_ids(&dir, "S", &sample_model(), ModelFormat::Tmsl, &ids).unwrap_err();
    assert!(matches!(err, Error::InvalidId { .. }), "{err}");
    let err = create_project(&dir, "S", &sample_model(), ModelFormat::Missing).unwrap_err();
    assert!(matches!(err, Error::NoFormat), "{err}");
    assert!(!dir.exists());
}

#[test]
fn refuses_to_overwrite_an_existing_project() {
    let (_tmp_dir, dir) = temp_dir("overwrite");
    create_project(&dir, "Sales", &sample_model(), ModelFormat::Tmsl).unwrap();
    let pbip_before = std::fs::read(dir.join("Sales.pbip")).unwrap();
    let err = create_project(&dir, "Sales", &sample_model(), ModelFormat::Tmdl).unwrap_err();
    assert!(matches!(err, Error::AlreadyExists(_)), "{err}");
    // Case differences count too, so the project stays valid on case-insensitive Windows.
    let err = create_project(&dir, "SALES", &sample_model(), ModelFormat::Tmdl).unwrap_err();
    assert!(matches!(err, Error::AlreadyExists(_)), "{err}");
    assert_eq!(std::fs::read(dir.join("Sales.pbip")).unwrap(), pbip_before);
    assert!(dir.join("Sales.SemanticModel/model.bim").is_file());
    assert!(!dir.join("Sales.SemanticModel/definition").exists());

    // A lone leftover item folder blocks creation as well, and is left alone.
    let (_tmp_other, other) = temp_dir("overwrite-item");
    std::fs::create_dir_all(other.join("shop.report")).unwrap();
    let err = create_project(&other, "Shop", &sample_model(), ModelFormat::Tmsl).unwrap_err();
    assert!(matches!(err, Error::AlreadyExists(_)), "{err}");
    assert!(other.join("shop.report").is_dir());
    assert!(!other.join("Shop.pbip").exists());
    for d in [dir, other] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn appends_missing_gitignore_lines_once() {
    let (_tmp_dir, dir) = temp_dir("gitignore");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(".gitignore"), "target/\r\n**/.pbi/cache.abf").unwrap();
    create_project(&dir, "A", &sample_model(), ModelFormat::Tmsl).unwrap();
    create_project(&dir, "B", &sample_model(), ModelFormat::Tmsl).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join(".gitignore")).unwrap(),
        "target/\r\n**/.pbi/cache.abf\n**/.pbi/localSettings.json\n**/.pbi/bicraft-data.bin\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn locate_picks_the_items_named_like_the_pbip_file() {
    let (_tmp_dir, dir) = temp_dir("two");
    create_project(&dir, "Alpha", &sample_model(), ModelFormat::Tmsl).unwrap();
    let beta = create_project(&dir, "Beta", &sample_model(), ModelFormat::Tmdl).unwrap();
    assert_eq!(beta.semantic_model, Some(dir.join("Beta.SemanticModel")));
    assert_eq!(beta.report, Some(dir.join("Beta.Report")));
    assert_eq!(beta.model_format, ModelFormat::Tmdl);
    let alpha = locate(&dir.join("Alpha.SemanticModel")).unwrap();
    assert_eq!(alpha.report, Some(dir.join("Alpha.Report")));
    assert_eq!(alpha.model_format, ModelFormat::Tmsl);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `save_model` rewrites `model.bim` in place and leaves every report byte as it was. The model
/// name survives even the M1.2 stub writer, so this runs today.
#[test]
fn save_model_rewrites_the_bim_and_never_touches_the_report() {
    let (_tmp_dir, dir) = temp_dir("save");
    let project = create_project(&dir, "Sales", &sample_model(), ModelFormat::Tmsl).unwrap();
    let report = dir.join("Sales.Report");
    let snapshot = |root: &Path| -> Vec<(String, Vec<u8>)> {
        files_under(root)
            .into_iter()
            .map(|f| (f.clone(), std::fs::read(root.join(&f)).unwrap()))
            .collect()
    };
    let before = snapshot(&report);
    let pbism_before = std::fs::read(dir.join("Sales.SemanticModel/definition.pbism")).unwrap();

    let mut model = sample_model();
    model.name = "Renamed".into();
    save_model(&project, &model).unwrap();
    assert_eq!(read_model(&project).unwrap().name, "Renamed");
    assert_eq!(snapshot(&report), before);
    assert_eq!(
        std::fs::read(dir.join("Sales.SemanticModel/definition.pbism")).unwrap(),
        pbism_before
    );
    assert_eq!(
        files_under(&dir.join("Sales.SemanticModel")),
        [".platform", "definition.pbism", "model.bim"],
        "no temporary file is left behind"
    );

    let mut missing = project.clone();
    missing.model_format = ModelFormat::Missing;
    assert!(matches!(save_model(&missing, &model), Err(Error::NoModel(_))));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn random_ids_are_fresh_v4_guids() {
    let a = ProjectIds::random();
    let b = ProjectIds::random();
    a.validate().unwrap();
    assert_ne!(a, b);
    assert_ne!(a.semantic_model_logical_id, a.report_logical_id);
    for id in [&a.semantic_model_logical_id, &b.report_logical_id] {
        assert!(is_guid(id), "{id}");
        assert_eq!(id.as_bytes()[14], b'4', "{id} is not version 4");
        assert!(
            matches!(id.as_bytes()[19], b'8' | b'9' | b'a' | b'b'),
            "{id} has the wrong variant"
        );
    }
    assert_eq!(a.page_name.len(), 20);
}

#[test]
fn model_format_names() {
    assert_eq!(ModelFormat::parse("TMDL"), Some(ModelFormat::Tmdl));
    assert_eq!(ModelFormat::parse("model.bim"), Some(ModelFormat::Tmsl));
    assert_eq!(ModelFormat::parse(" tmsl "), Some(ModelFormat::Tmsl));
    assert_eq!(ModelFormat::parse("missing"), None);
    assert_eq!(ModelFormat::Tmdl.as_str(), "tmdl");
}

// ---- Cross-format round trips: these need the real M1.2 TMSL / TMDL writers. ----

fn assert_sample_model(model: &SemanticModel) {
    assert_eq!(model.compatibility_level, Some(1567));
    assert!(model.table("Order Lines").is_some());
    assert!(model.table("Product").is_some());
    assert_eq!(
        model.measure("Units").map(|(_, m)| m.expression.as_str()),
        Some("SUM ( 'Order Lines'[Qty] )")
    );
    assert_eq!(model.relationships.len(), 1);
}

fn round_trip(format: ModelFormat, tag: &str) {
    let (_tmp_dir, dir) = temp_dir(tag);
    let project = create_project(&dir, "Sales", &sample_model(), format).unwrap();
    let found = locate(&dir.join("Sales.pbip")).unwrap();
    assert_eq!(found.model_format, format);
    let model = read_model(&found).unwrap();
    assert_sample_model(&model);
    assert_eq!(model, read_model(&project).unwrap());

    // Every model file is UTF-8 without BOM. JSON files are LF only; `.tmdl` files are CRLF only,
    // as Desktop's TMDL serializer writes them (see docs/formats/tmdl.md).
    let model_dir = dir.join("Sales.SemanticModel");
    for file in files_under(&model_dir) {
        let bytes = std::fs::read(model_dir.join(&file)).unwrap();
        assert!(!bytes.starts_with(&[0xEF, 0xBB, 0xBF]), "{file} has a BOM");
        let text = String::from_utf8(bytes).unwrap();
        if file.ends_with(".tmdl") {
            assert!(
                !text.replace("\r\n", "").contains(['\r', '\n']),
                "{file} mixes line endings"
            );
        } else {
            assert!(!text.contains('\r'), "{file} has a CR");
        }
    }

    // Edit, save, read back.
    let mut edited = model.clone();
    edited.tables.retain(|t| t.name != "Product");
    edited.relationships.clear();
    save_model(&found, &edited).unwrap();
    let again = read_model(&locate(&dir).unwrap()).unwrap();
    assert!(again.table("Product").is_none());
    assert!(again.relationships.is_empty());
    assert!(again.table("Order Lines").is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tmsl_project_round_trips_the_model() {
    round_trip(ModelFormat::Tmsl, "rt-tmsl");
}

#[test]
fn tmdl_project_round_trips_the_model() {
    round_trip(ModelFormat::Tmdl, "rt-tmdl");
}

/// The blank report on disk reads back as [`blank_report`], so the engine can offer the same
/// report in memory (`report.new`) as `project.new` writes.
#[test]
fn the_blank_report_reads_back_as_blank_report() {
    let (_tmp_dir, dir) = temp_dir("blank-report");
    let project =
        create_project_with_ids(&dir, "Sales", &sample_model(), ModelFormat::Tmsl, &fixed_ids()).unwrap();
    let mut read = crate::read_report(&project).unwrap();
    read.schemas.clear();
    read.version_extra.clear();
    assert_eq!(read, blank_report("0123456789abcdef0123"));

    // Saving it back keeps a PBIR report that still reads the same.
    crate::save_report(&project, &read).unwrap();
    let mut again = crate::read_report(&locate(&dir).unwrap()).unwrap();
    again.schemas.clear();
    again.version_extra.clear();
    assert_eq!(again, read);
    let _ = std::fs::remove_dir_all(&dir);
}
