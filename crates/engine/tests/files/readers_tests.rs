//! M11.36: `file.open` of `.vpax`, `.pbids`, `.pq`/`.m`, `.pqt`, a Dataflow Gen2 definition and a
//! CDM `model.json` (synthetic fixtures under `tests/fixtures/import-sources/`), their round trip
//! through PBIX and PBIP, `objects.import` from them, and hostile inputs.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/import-sources")
        .join(name)
}

fn zip(path: &Path, parts: &[(&str, &[u8])]) {
    let mut pkg = bicraft_opc::Package::default();
    for (n, d) in parts {
        pkg.put(n, d.to_vec(), false);
    }
    std::fs::write(path, pkg.to_bytes().unwrap()).unwrap();
}

fn partition(s: &Session, table: &str) -> String {
    let m = s.model.as_ref().unwrap();
    m.table(table).unwrap().partitions[0].expression.clone().unwrap()
}

/// The open model's import tables get their columns and rows from their own queries, as the
/// readers' notes say.
fn load(s: &mut Session, tables: &[&str]) {
    for t in tables {
        let m = partition(s, t);
        run(s, "table.import", json!({"name": t, "m": m, "replace": true}));
    }
}

#[test]
fn pbids_opens_as_a_model_with_the_sources_query() {
    let mut s = Session::new();
    let out = run(&mut s, "file.open", json!({"path": fixture("sales.pbids")}));
    assert_eq!(out["format"], "pbids");
    assert_eq!(out["model"]["tables"], 1);
    assert!(s.source.is_none() && s.dirty, "a new, unsaved model");
    let m = s.model.as_ref().unwrap();
    assert_eq!(m.tables[0].name, "sales");
    assert_eq!(m.tables[0].partitions[0].mode.as_deref(), Some("directQuery"));
    assert_eq!(
        partition(&s, "sales"),
        "Sql.Database(\"sql01.example\", \"Sales\")"
    );
    // saved as a project and read back
    let dir = TempDir::new("readers-pbids");
    let out = run(&mut s, "file.save_as", json!({"path": dir.join("Sales.pbip")}));
    assert_eq!(out["format"], "pbip-tmdl", "{out}");
    let mut back = Session::new();
    run(&mut back, "file.open", json!({"path": dir.join("Sales.pbip")}));
    assert_eq!(
        partition(&back, "sales"),
        "Sql.Database(\"sql01.example\", \"Sales\")"
    );
}

#[test]
fn pbids_protocols_and_bad_files() {
    let dir = TempDir::new("readers-pbids-kinds");
    let cases = [
        (
            r#"{"protocol":"file","address":{"path":"C:\\data\\a.csv"}}"#,
            "Csv.Document(File.Contents(\"C:\\data\\a.csv\")",
        ),
        (
            r#"{"protocol":"folder","address":{"path":"C:\\data"}}"#,
            "Folder.Files(\"C:\\data\")",
        ),
        (
            r#"{"protocol":"odata","address":{"url":"https://x.example/odata"}}"#,
            "OData.Feed(\"https://x.example/odata\")",
        ),
        (
            r#"{"protocol":"analysis-services","address":{"server":"as.example","database":"Cube"}}"#,
            "AnalysisServices.Database(\"as.example\", \"Cube\")",
        ),
        (
            r#"{"protocol":"tds","address":{"server":"s"},"query":"select \"x\""}"#,
            "Sql.Databases(\"s\")",
        ),
    ];
    for (i, (details, expect)) in cases.iter().enumerate() {
        let p = dir.join(format!("c{i}.pbids"));
        std::fs::write(
            &p,
            format!(r#"{{"version":"0.1","connections":[{{"details":{details}}}]}}"#),
        )
        .unwrap();
        let mut s = Session::new();
        run(&mut s, "file.open", json!({"path": p}));
        let e = partition(&s, &format!("c{i}"));
        assert!(e.contains(expect), "{e}");
    }
    for bad in [
        "",
        "{}",
        r#"{"connections":[]}"#,
        r#"{"connections":[{"details":{"protocol":"tds","address":{}}}]}"#,
        r#"{"connections":[{"details":{"protocol":"gopher","address":{}}}]}"#,
        &format!("{}1{}", "[".repeat(100_000), "]".repeat(100_000)),
    ] {
        let p = dir.join("bad.pbids");
        std::fs::write(&p, bad).unwrap();
        let mut s = Session::new();
        assert!(
            s.execute("file.open", &json!({"path": p})).is_err(),
            "{}",
            &bad[..bad.len().min(60)]
        );
    }
}

#[test]
fn m_files_give_tables_and_queries_and_save_to_pbix() {
    let dir = TempDir::new("readers-m");
    let mut s = Session::new();
    let out = run(&mut s, "file.open", json!({"path": fixture("queries.pq")}));
    assert_eq!(out["format"], "m");
    let m = s.model.as_ref().unwrap();
    let tables: Vec<&str> = m.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(tables, ["Customers", "Orders"]);
    let exprs: Vec<&str> = m.expressions.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(exprs, ["Double", "Threshold"]);
    // a PBIX with the queries' data, read back
    let pbix = dir.join("Queries.pbix");
    load(&mut s, &["Customers", "Orders"]);
    run(
        &mut s,
        "measure.add",
        json!({"table": "Customers", "name": "N", "expression": "COUNTROWS(Customers)"}),
    );
    run(&mut s, "file.save_as", json!({"path": pbix}));
    let mut back = Session::new();
    run(&mut back, "file.open", json!({"path": pbix}));
    let rows = run(&mut back, "table.rows", json!({"table": "Customers"}));
    assert_eq!(rows["total_rows"], 2, "{rows}");
    assert_eq!(back.model.as_ref().unwrap().expressions.len(), 2);
    // one expression
    let mut one = Session::new();
    run(&mut one, "file.open", json!({"path": fixture("single.m")}));
    assert_eq!(one.model.as_ref().unwrap().tables[0].name, "single");
}

#[test]
fn dataflow_definitions_open_from_either_part() {
    for f in ["dataflow/mashup.pq", "dataflow/queryMetadata.json"] {
        let mut s = Session::new();
        let out = run(&mut s, "file.open", json!({"path": fixture(f)}));
        assert_eq!(out["format"], "dataflow", "{f}");
        let m = s.model.as_ref().unwrap();
        assert_eq!(m.name, "Catalog flow");
        assert_eq!(m.culture.as_deref(), Some("fr-FR"));
        assert_eq!(m.tables.len(), 1);
        assert_eq!(m.tables[0].name, "Products");
        assert_eq!(m.expressions[0].name, "Helper");
    }
}

#[test]
fn cdm_model_json_gives_entities_columns_and_relationships() {
    let mut s = Session::new();
    let out = run(&mut s, "file.open", json!({"path": fixture("model.json")}));
    assert_eq!(out["format"], "cdm");
    let m = s.model.as_ref().unwrap();
    assert_eq!(m.name, "Retail flow");
    let stores = m.table("Stores").unwrap();
    assert_eq!(stores.columns.len(), 2);
    assert!(partition(&s, "Stores").contains("#table"), "the mashup's query");
    let sales = m.table("Sales").unwrap();
    assert_eq!(sales.columns.len(), 3);
    assert_eq!(sales.columns[1].data_type, bicraft_model::DataType::Decimal);
    assert!(partition(&s, "Sales").contains("https://storage.example/flow/Sales.csv"));
    assert_eq!(m.relationships.len(), 1);
    assert_eq!(m.relationships[0].from_table, "Sales");
    assert_eq!(m.expressions[0].name, "Rate");
    // validates
    let v = run(&mut s, "model.validate", json!({}));
    assert!(v.to_string().len() > 2);
}

#[test]
fn pqt_template_and_hostile_zips() {
    let dir = TempDir::new("readers-pqt");
    let secret = dir.join("secret.txt");
    std::fs::write(&secret, "TOP-SECRET-CONTENT").unwrap();
    // XXE and entity expansion in the content types: never parsed, never expanded
    let xxe = format!(
        "<?xml version=\"1.0\"?><!DOCTYPE t [<!ENTITY x SYSTEM \"file:///{}\"><!ENTITY a \"aaaaaaaaaa\"><!ENTITY b \"&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;\"><!ENTITY c \"&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;\">]><Types>&x;&c;</Types>",
        secret.display()
    );
    let pq = std::fs::read(fixture("queries.pq")).unwrap();
    let p = dir.join("Template.pqt");
    zip(
        &p,
        &[
            ("[Content_Types].xml", xxe.as_bytes()),
            ("MashupDocument.pq", &pq),
            ("MashupMetadata.json", br#"{"Document":{"queriesMetadata":{"Customers":{"queryName":"Customers","loadEnabled":true}},"culture":"de-DE"}}"#),
        ],
    );
    let mut s = Session::new();
    let out = run(&mut s, "file.open", json!({"path": p}));
    assert_eq!(out["format"], "pqt");
    assert!(!out.to_string().contains("TOP-SECRET"));
    let m = s.model.as_ref().unwrap();
    assert_eq!(m.tables.len(), 1, "only Customers loads");
    assert_eq!(m.expressions.len(), 3);
    // a ZIP declaring a huge part (a ZIP bomb's header): refused, quickly
    let mut pkg = bicraft_opc::Package::default();
    pkg.put("MashupDocument.pq", vec![b' '; 4096], false);
    let mut bytes = pkg.to_bytes().unwrap();
    for sig in [[0x50u8, 0x4b, 0x03, 0x04], [0x50, 0x4b, 0x01, 0x02]] {
        let off = if sig[2] == 3 { 22 } else { 24 };
        let mut i = 0;
        while let Some(at) = bytes[i..].windows(4).position(|w| w == sig) {
            let at = i + at;
            bytes[at + off..at + off + 4].copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
            i = at + 4;
        }
    }
    let bomb = dir.join("bomb.pqt");
    std::fs::write(&bomb, &bytes).unwrap();
    let t = Instant::now();
    assert!(
        Session::new()
            .execute("file.open", &json!({"path": bomb}))
            .is_err()
    );
    assert!(t.elapsed() < Duration::from_secs(5));
    // a template without an M document, garbage
    let empty = dir.join("empty.pqt");
    zip(&empty, &[("x.json", b"{}")]);
    assert!(
        Session::new()
            .execute("file.open", &json!({"path": empty}))
            .is_err()
    );
    let junk = dir.join("junk.pqt");
    std::fs::write(&junk, b"PK\x03\x04garbage").unwrap();
    assert!(
        Session::new()
            .execute("file.open", &json!({"path": junk}))
            .is_err()
    );
    // deep M nesting
    let deep = dir.join("deep.pq");
    std::fs::write(&deep, format!("section S; shared A = {};", "{".repeat(200_000))).unwrap();
    assert!(
        Session::new()
            .execute("file.open", &json!({"path": deep}))
            .is_err()
    );
}

#[test]
fn vpax_reads_back_with_and_without_model_bim() {
    let dir = TempDir::new("readers-vpax");
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": fixture("queries.pq")}));
    load(&mut s, &["Customers", "Orders"]);
    run(
        &mut s,
        "measure.add",
        json!({"table": "Customers", "name": "N", "expression": "COUNTROWS(Customers)"}),
    );
    run(
        &mut s,
        "relationship.add",
        json!({"from": "Orders[Customer]", "to": "Customers[Id]"}),
    );
    let pbix = dir.join("Shop.pbix");
    run(&mut s, "file.save_as", json!({"path": pbix}));
    for bim in [true, false] {
        let vpax = dir.join(format!("shop-{bim}.vpax"));
        run(
            &mut s,
            "model.export_vpax",
            json!({"path": vpax, "model_bim": bim}),
        );
        let mut v = Session::new();
        let out = run(&mut v, "file.open", json!({"path": vpax}));
        assert_eq!(out["format"], "vpax");
        assert_eq!(
            out["statistics"]["model_from"],
            if bim { "Model.bim" } else { "DaxModel.json" }
        );
        let st = out["statistics"]["tables"].as_array().unwrap();
        let customers = st.iter().find(|t| t["name"] == "Customers").unwrap();
        assert_eq!(customers["rows"], 2, "{customers}");
        assert!(customers["total_size"].as_u64().unwrap() > 0);
        let m = v.model.as_ref().unwrap();
        let c = m.table("Customers").unwrap();
        assert!(c.column("Name").is_some());
        assert_eq!(c.measures[0].expression, "COUNTROWS(Customers)");
        assert_eq!(m.relationships.len(), 1, "bim {bim}");
        assert_eq!(m.relationships[0].from_table, "Orders");
        // what opens validates and saves
        run(
            &mut v,
            "file.save_as",
            json!({"path": dir.join(format!("back{bim}.pbip"))}),
        );
    }
    // hostile VPAX: no DaxModel.json, deep JSON
    let bad = dir.join("bad.vpax");
    zip(&bad, &[("x", b"1")]);
    assert!(
        Session::new()
            .execute("file.open", &json!({"path": bad}))
            .is_err()
    );
    let deep = format!("{}{}", "[".repeat(50_000), "]".repeat(50_000));
    zip(&bad, &[("DaxModel.json", deep.as_bytes())]);
    assert!(
        Session::new()
            .execute("file.open", &json!({"path": bad}))
            .is_err()
    );
}

#[test]
fn objects_import_takes_the_new_files_as_sources() {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Target"}));
    let out = run(
        &mut s,
        "objects.import",
        json!({"source": fixture("queries.pq"), "tables": ["Customers"], "expressions": ["Threshold"]}),
    );
    assert_eq!(out["applied"], true, "{out}");
    let m = s.model.as_ref().unwrap();
    assert!(m.table("Customers").is_some());
    assert!(m.expressions.iter().any(|e| e.name == "Threshold"));
    let out = run(
        &mut s,
        "objects.import",
        json!({"source": fixture("model.json"), "tables": ["Sales", "Stores"]}),
    );
    assert_eq!(out["applied"], true, "{out}");
    assert_eq!(s.model.as_ref().unwrap().relationships.len(), 1);
}
