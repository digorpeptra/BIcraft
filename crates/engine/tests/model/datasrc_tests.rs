//! The M evaluator wired into the engine (G21): `data_root`, culture and shared queries in
//! `table.import`, structural PBIX saves and the viewer; text stored as Analysis Services stores
//! it. Synthetic data in temporary folders only.

use std::path::Path;

use bicraft_model::NamedExpression;
use bicraft_opc::Package;
use bicraft_pbix::structural::DataModelEditor;
use bicraft_vertipaq::CellValue;
use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

/// A fresh folder per call, removed when the guard drops.
fn temp(name: &str) -> TempDir {
    TempDir::new(&format!("engine-datasrc-{name}"))
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// A CSV of ours with case and trailing-blank variants of the same names.
const PEOPLE: &str = "Name,Score\nann,1\nAnn ,2\nBOB\t,3\nbob,4\nàb,5\nÀb,6\n";

/// The query a user would write for `people.csv` under the data root.
const QUERY: &str = r#"let
    Source = Csv.Document(File.Contents("people.csv"), [Delimiter = ",", Encoding = 65001]),
    Promoted = Table.PromoteHeaders(Source, [PromoteAllScalars = true]),
    Typed = Table.TransformColumnTypes(Promoted, {{"Name", type text}, {"Score", Int64.Type}})
in
    Typed"#;

fn data_dir() -> TempDir {
    let d = temp("data");
    std::fs::write(d.join("people.csv"), PEOPLE).unwrap();
    d
}

fn model_session(culture: &str) -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "M", "culture": culture}));
    s
}

/// The single value of a card bound to `measure` (added to `table`).
fn card(s: &mut Session, table: &str, measure: &str, expression: &str) -> Value {
    run(
        s,
        "measure.add",
        json!({"table": table, "name": measure, "expression": expression}),
    );
    if s.report.is_none() {
        run(s, "report.new", json!({}));
        run(s, "page.add", json!({"display_name": "Main"}));
    }
    let r = run(
        s,
        "visual.add",
        json!({"page": "Main", "visual_type": "card", "x": 0, "y": 0, "width": 100, "height": 100,
               "fields": [{"role": "Values", "field": format!("[{measure}]")}]}),
    );
    let name = r["visual"].as_str().unwrap().to_string();
    let snap = s.snapshot();
    for p in snap["report"]["pages"].as_array().cloned().unwrap_or_default() {
        for v in p["visuals"].as_array().cloned().unwrap_or_default() {
            if v["name"] == name {
                return v["data"].clone();
            }
        }
    }
    panic!("visual {name} not in the snapshot")
}

fn single(d: &Value) -> Value {
    assert_eq!(d["status"], "ok", "{d}");
    d["rows"][0][0].clone()
}

#[test]
fn import_m_reads_files_only_under_an_explicit_data_root() {
    let dir = data_dir();
    let mut s = model_session("en-US");
    // No data root: refused, saying what to pass.
    let err = s
        .execute("table.import", &json!({"name": "People", "m": QUERY}))
        .unwrap_err();
    assert!(err.contains("data_root"), "{err}");
    assert!(s.model.as_ref().unwrap().table("People").is_none());
    // A data root that is not a folder.
    let err = s
        .execute(
            "table.import",
            &json!({"name": "People", "m": QUERY, "data_root": dir.join("people.csv")}),
        )
        .unwrap_err();
    assert!(err.contains("data_root"), "{err}");
    let out = run(
        &mut s,
        "table.import",
        json!({"name": "People", "m": QUERY, "data_root": dir.path()}),
    );
    assert_eq!(out["rows"], 6);
    assert_eq!(out["columns"][1]["data_type"], "int64");
    assert!(s.data_root.is_some());
    let t = s.model.as_ref().unwrap().table("People").unwrap();
    assert_eq!(t.partitions[0].expression.as_deref(), Some(QUERY));
    // A file outside the root stays unreadable.
    let err = s
        .execute(
            "table.import",
            &json!({"name": "Other", "m": "Csv.Document(File.Contents(\"../x.csv\"))"}),
        )
        .unwrap_err();
    assert!(err.contains("outside"), "{err}");
    // file.open resets the root.
    let bim = dir.join("m.bim");
    run(&mut s, "file.save_as", json!({"path": bim}));
    run(&mut s, "file.open", json!({"path": bim}));
    assert_eq!(s.data_root, None);
    run(&mut s, "file.open", json!({"path": bim, "data_root": dir.path()}));
    assert!(s.data_root.is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_m_uses_the_model_culture_and_shared_queries() {
    let mut s = model_session("fr-FR");
    let m = s.model.as_mut().unwrap();
    m.expressions.push(NamedExpression {
        name: "Factor".into(),
        kind: "m".into(),
        expression: "3 meta [IsParameterQuery = true, Type = \"Number\", IsParameterQueryRequired = true]"
            .into(),
        ..Default::default()
    });
    m.expressions.push(NamedExpression {
        name: "Unrelated".into(),
        kind: "m".into(),
        // Not referenced, so never parsed.
        expression: "let in (".into(),
        ..Default::default()
    });
    // fr-FR: "1,5" is a number.
    let q = "let T = #table(type table [A = text], {{\"1,5\"}, {\"2,5\"}}), \
             N = Table.TransformColumnTypes(T, {{\"A\", type number}}), \
             X = Table.AddColumn(N, \"B\", each [A] * Factor, type number) in X";
    let out = run(&mut s, "table.import", json!({"name": "T", "m": q}));
    assert_eq!(out["rows"], 2);
    let d = card(&mut s, "T", "SumB", "SUM(T[B])");
    assert_eq!(single(&d).as_f64(), Some(12.0), "{d}");
}

#[test]
fn viewer_previews_file_queries_and_stores_text_like_analysis_services() {
    let dir = data_dir();
    for (culture, names) in [("en-US", 4.0), ("fr-FR", 3.0)] {
        let mut s = model_session(culture);
        run(
            &mut s,
            "table.import",
            json!({"name": "People", "m": QUERY, "data_root": dir.path()}),
        );
        // ann/Ann /BOB\t/bob are two values; àb/Àb are one in fr-FR only.
        let d = card(&mut s, "People", "Names", "DISTINCTCOUNT(People[Name])");
        assert_eq!(single(&d).as_f64(), Some(names), "{culture}: {d}");
        let d = card(
            &mut s,
            "People",
            "Anns",
            "CALCULATE(COUNTROWS(People), People[Name] = \"ann\")",
        );
        assert_eq!(
            single(&d).as_f64(),
            Some(2.0),
            "{culture}: trailing blanks are dropped: {d}"
        );
        // Without the data root, the preview says why.
        s.data_root = None;
        s.revision += 1;
        let d = card(&mut s, "People", "Rows", "COUNTROWS(People)");
        assert_ne!(d["status"], "ok");
        assert!(d.to_string().contains("data_root"), "{d}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

fn stored(path: &Path, table: &str, column: &str) -> Vec<CellValue> {
    let pkg = Package::open(path).unwrap();
    let part = &pkg.get(bicraft_pbix::DATA_MODEL).unwrap().data;
    DataModelEditor::open(part, &Default::default())
        .unwrap()
        .column_values(table, column)
        .unwrap()
}

fn texts(v: &[&str]) -> Vec<CellValue> {
    v.iter().map(|s| CellValue::Text((*s).into())).collect()
}

#[test]
fn structural_save_runs_file_queries_under_the_data_root() {
    let dir = data_dir();
    let path = dir.join("Shop.pbix");
    let pkg = bicraft_pbix::synthetic::pbix(&bicraft_pbix::synthetic::storage_database().unwrap()).unwrap();
    std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    s.model.as_mut().unwrap().culture = Some("en-US".into());
    // A table whose query reads a file, added without table.import.
    run(
        &mut s,
        "table.add",
        json!({"name": "People", "partition": {"source_type": "m", "expression": QUERY}}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "People", "name": "Name", "data_type": "string"}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "People", "name": "Score", "data_type": "int64"}),
    );
    let err = s.execute("file.save", &json!({"backup": false})).unwrap_err();
    assert!(err.contains("data_root"), "{err}");
    assert_eq!(s.data_root, None);
    // An Enter-data table (Desktop's compressed form) needs no root and saves with the same rules.
    run(
        &mut s,
        "table.add",
        json!({"name": "Typed", "partition": {"source_type": "m", "expression": enter_data()}}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "Typed", "name": "K", "data_type": "string"}),
    );
    run(
        &mut s,
        "file.save",
        json!({"backup": false, "data_root": dir.path()}),
    );
    assert!(s.data_root.is_some());
    // Stored as Analysis Services stores it: trailing blanks dropped, ASCII case variants merged
    // (en-US), first spelling kept.
    assert_eq!(
        stored(&path, "People", "Name"),
        texts(&["ann", "ann", "BOB", "BOB", "àb", "Àb"])
    );
    assert_eq!(
        stored(&path, "People", "Score"),
        (1..=6).map(CellValue::Int).collect::<Vec<_>>()
    );
    assert_eq!(stored(&path, "Typed", "K"), texts(&["x", "x", "y"]));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Desktop's "Enter data" query for the rows `x `, `X`, `y` (raw DEFLATE of the JSON rows).
fn enter_data() -> String {
    let json = r#"[["x "],["X"],["y"]]"#;
    let deflated = deflate_stored(json.as_bytes());
    format!(
        "let Source = Table.FromRows(Json.Document(Binary.Decompress(Binary.FromText(\"{}\", \
         BinaryEncoding.Base64), Compression.Deflate)), let _t = ((type nullable text) meta \
         [Serialized.Text = true]) in type table [K = _t]) in Source",
        base64(&deflated)
    )
}

/// RFC 1951 with one stored (uncompressed) block.
fn deflate_stored(data: &[u8]) -> Vec<u8> {
    let len = u16::try_from(data.len()).unwrap();
    let mut out = vec![1u8];
    out.extend(len.to_le_bytes());
    out.extend((!len).to_le_bytes());
    out.extend(data);
    out
}

fn base64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A model parameter read by a table's query through a record initializer: the value of the
/// parameter (6), as text when it is text (see `shared_expressions_in_record_initializers` in
/// bicraft-data for the evaluator's cases).
#[test]
fn import_m_reads_a_parameter_in_a_record_initializer() {
    let number = "6 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]";
    for (body, expected) in [("6", "1,6"), (number, "1,6"), ("\"6\"", "1,6")] {
        let mut s = model_session("en-US");
        run(
            &mut s,
            "expression.add",
            json!({"name": "Threshold", "expression": body}),
        );
        run(
            &mut s,
            "expression.add",
            json!({"name": "Limit", "expression": "Threshold + 3"}),
        );
        let q = "Table.FromRecords({[A = 1], [A = Threshold]})";
        let out = run(&mut s, "table.import", json!({"name": "T", "m": q}));
        assert_eq!(out["rows"], 2, "{body}");
        let d = card(&mut s, "T", "Vals", "CONCATENATEX(T, T[A], \",\", T[A])");
        assert_eq!(single(&d), json!(expected), "{body}: {d}");
    }
}
