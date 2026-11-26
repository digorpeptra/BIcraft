//! Property order of every object kind, pinned by the TOM oracle.
//!
//! `fixtures/tom-props` is the TMDL that the TOM serializer of Power BI Desktop 2.158.1304.0
//! (the public TOM library, version 17.0.87.18) writes for our synthetic "every property set"
//! model (built and serialized through TOM by the maintainers' development oracles;
//! docs/formats/tmdl.md "Property order"). The writer must give the same bytes whatever order
//! the properties arrive in: commands append keys to `extra` in the order they run.

use std::path::Path;

use bicraft_model::{Column, DataType, Properties, SemanticModel, Table};
use serde_json::{Value, json};

/// `(relative path, CRLF text)` for every file of `tom-props`, sorted by path.
fn fixture() -> Vec<(String, String)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                let text = std::fs::read_to_string(&path).unwrap();
                out.push((relative, text.replace("\r\n", "\n").replace('\n', "\r\n")));
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tom-props");
    let mut out = Vec::new();
    walk(&root, &root, &mut out);
    out.sort();
    out
}

fn assert_same(expected: &[(String, String)], actual: &[(String, String)]) {
    let paths = |d: &[(String, String)]| d.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>();
    assert_eq!(paths(expected), paths(actual), "file lists differ");
    for ((path, want), (_, got)) in expected.iter().zip(actual) {
        if want != got {
            let w: Vec<&str> = want.split("\r\n").collect();
            let g: Vec<&str> = got.split("\r\n").collect();
            let line = w
                .iter()
                .zip(&g)
                .position(|(a, b)| a != b)
                .unwrap_or(w.len().min(g.len()));
            panic!(
                "{path}: first difference at line {}\n  want: {:?}\n  got:  {:?}\n--- got ---\n{got}",
                line + 1,
                w.get(line),
                g.get(line)
            );
        }
    }
}

/// Reverses the key order of `map` and of every object nested in it.
fn reverse_map(map: &mut Properties) {
    let mut entries: Vec<(String, Value)> = std::mem::take(map).into_iter().collect();
    entries.reverse();
    for (k, mut v) in entries {
        reverse_value(&mut v);
        map.insert(k, v);
    }
}

fn reverse_value(v: &mut Value) {
    match v {
        Value::Object(m) => reverse_map(m),
        Value::Array(items) => items.iter_mut().for_each(reverse_value),
        _ => {}
    }
}

fn reverse_model(m: &mut SemanticModel) {
    reverse_map(&mut m.database_extra);
    reverse_map(&mut m.extra);
    for t in &mut m.tables {
        reverse_map(&mut t.extra);
        t.columns.iter_mut().for_each(|c| reverse_map(&mut c.extra));
        t.measures.iter_mut().for_each(|x| reverse_map(&mut x.extra));
        for p in &mut t.partitions {
            reverse_map(&mut p.extra);
            reverse_map(&mut p.source_extra);
        }
    }
    m.relationships.iter_mut().for_each(|r| reverse_map(&mut r.extra));
    m.expressions.iter_mut().for_each(|e| reverse_map(&mut e.extra));
    m.roles.iter_mut().for_each(|r| reverse_map(&mut r.extra));
}

#[test]
fn every_property_is_written_in_tom_order() {
    let docs = fixture();
    let model = bicraft_tmdl::read_documents(&docs).unwrap();
    assert_same(&docs, &bicraft_tmdl::write_documents(&model));
}

#[test]
fn property_order_does_not_depend_on_input_order() {
    let docs = fixture();
    let mut model = bicraft_tmdl::read_documents(&docs).unwrap();
    reverse_model(&mut model);
    assert_same(&docs, &bicraft_tmdl::write_documents(&model));
}

/// The order a session gives: `column.add {hidden: true}`, then `column.set {is_key: true}` and
/// other later settings, each appended to `extra`. TOM writes `isHidden` before `isUnique` and
/// `isKey`, and `expressionContext` after `summarizeBy`.
#[test]
fn command_order_does_not_change_column_property_order() {
    let mut c = Column {
        name: "C".into(),
        data_type: DataType::Int64,
        is_hidden: true,
        source_column: Some("C".into()),
        summarize_by: Some("none".into()),
        ..Default::default()
    };
    for (k, v) in [
        ("isKey", json!(true)),
        ("isUnique", json!(true)),
        ("isAvailableInMdx", json!(false)),
        ("isNullable", json!(false)),
    ] {
        c.extra.insert(k.into(), v);
    }
    let mut cc = Column {
        name: "CC".into(),
        data_type: DataType::Int64,
        expression: Some("1".into()),
        sort_by_column: Some("C".into()),
        summarize_by: Some("none".into()),
        ..Default::default()
    };
    cc.extra.insert("expressionContext".into(), json!("userContext"));
    let model = SemanticModel {
        name: "M".into(),
        tables: vec![Table {
            name: "T".into(),
            columns: vec![c, cc],
            ..Default::default()
        }],
        ..Default::default()
    };
    let docs = bicraft_tmdl::write_documents(&model);
    let (_, text) = docs.iter().find(|(p, _)| p == "tables/T.tmdl").unwrap();
    assert_eq!(
        text.replace("\r\n", "\n"),
        "table T\n\
         \n\
         \tcolumn C\n\
         \t\tdataType: int64\n\
         \t\tisHidden\n\
         \t\tisUnique\n\
         \t\tisKey\n\
         \t\tisNullable: false\n\
         \t\tisAvailableInMdx: false\n\
         \t\tsummarizeBy: none\n\
         \t\tsourceColumn: C\n\
         \n\
         \tcolumn CC = 1\n\
         \t\tdataType: int64\n\
         \t\tsummarizeBy: none\n\
         \t\texpressionContext: userContext\n\
         \t\tsortByColumn: C\n\
         \n"
    );
}
