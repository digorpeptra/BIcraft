//! The pinned corpus (ROADMAP M11.8, docs/corpus.md): files we author ourselves, saved by Power
//! BI Desktop, listed with their SHA-256 in `corpus/manifest.json` and kept outside the
//! repository.
//!
//! - `BICRAFT_CORPUS`: the folder that holds them (`cargo xtask corpus fetch|verify`).
//!
//! Each entry's files are checked against the manifest (size, SHA-256: a mismatch fails), then
//! the entry is opened, summarised (model and report) and compared with the manifest's `expect`,
//! saved as a PBIX and as a PBIP project, and each copy is reopened and must summarise the same.
//! Without the variable, or for an entry whose files are missing, the test prints a note and
//! passes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use serde_json::{Map, Value, json};

fn manifest() -> Value {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/manifest.json");
    let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn local(dir: &Path, rel: &str) -> PathBuf {
    rel.split('/').fold(dir.to_path_buf(), |p, s| p.join(s))
}

fn sha256_hex(data: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, data)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `false` when a file is missing (the entry is skipped); panics on a size or hash mismatch.
fn files_match(dir: &Path, entry: &Value) -> bool {
    for f in entry["files"].as_array().into_iter().flatten() {
        let rel = f["path"].as_str().unwrap();
        let p = local(dir, rel);
        let Ok(data) = std::fs::read(&p) else {
            eprintln!("note: {rel} is missing (cargo xtask corpus fetch); entry skipped");
            return false;
        };
        assert_eq!(data.len() as u64, f["size"].as_u64().unwrap(), "{rel}: size");
        assert_eq!(sha256_hex(&data), f["sha256"].as_str().unwrap(), "{rel}: SHA-256");
    }
    true
}

/// What the manifest pins of a file: model objects by name, report pages and visual types.
fn summary(s: &mut Session) -> Value {
    let m = s.model.as_ref().expect("no model read");
    let mut tables: Vec<&str> = m.tables.iter().map(|t| t.name.as_str()).collect();
    tables.sort_unstable();
    let mut calculated_tables: Vec<&str> = m
        .tables
        .iter()
        .filter(|t| t.partitions.iter().any(|p| p.source_type == "calculated"))
        .map(|t| t.name.as_str())
        .collect();
    calculated_tables.sort_unstable();
    let mut calculated_columns: Vec<String> = m
        .tables
        .iter()
        .flat_map(|t| {
            t.columns
                .iter()
                .filter(|c| c.expression.is_some())
                .map(move |c| format!("{}[{}]", t.name, c.name))
        })
        .collect();
    calculated_columns.sort_unstable();
    let mut measures: Vec<String> = m
        .tables
        .iter()
        .flat_map(|t| t.measures.iter().map(move |x| format!("{}[{}]", t.name, x.name)))
        .collect();
    measures.sort_unstable();
    let mut relationships: Vec<String> = m
        .relationships
        .iter()
        .map(|r| {
            format!(
                "{}[{}] -> {}[{}]{}",
                r.from_table,
                r.from_column,
                r.to_table,
                r.to_column,
                if r.is_active { "" } else { " (inactive)" }
            )
        })
        .collect();
    relationships.sort_unstable();
    let mut roles: Vec<&str> = m.roles.iter().map(|r| r.name.as_str()).collect();
    roles.sort_unstable();
    let mut groups = Map::new();
    for t in &m.tables {
        if let Some(g) = t.extra.get("calculationGroup") {
            let items: Vec<&str> = g["calculationItems"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|i| i["name"].as_str())
                .collect();
            groups.insert(t.name.clone(), json!(items));
        }
    }
    let mut out = json!({
        "tables": tables,
        "calculated_tables": calculated_tables,
        "calculated_columns": calculated_columns,
        "measures": measures,
        "relationships": relationships,
        "roles": roles,
        "calculation_groups": groups,
    });
    if s.report.is_some() {
        let r = s.execute("report.summary", &json!({})).unwrap();
        let pages: Vec<&str> = r["pages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p["display_name"].as_str())
            .collect();
        let types: BTreeMap<String, Value> = r["visual_types"]
            .as_object()
            .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        out["pages"] = json!(pages);
        out["visual_types"] = json!(types);
    }
    out
}

fn open(path: &Path) -> Session {
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": path}))
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    s
}

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("bicraft-corpus-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn corpus_files_open_and_round_trip() {
    let Some(dir) = bicraft_env::var_os("BICRAFT_CORPUS").map(PathBuf::from) else {
        eprintln!("note: BICRAFT_CORPUS is not set; corpus skipped (docs/corpus.md)");
        return;
    };
    let m = manifest();
    let mut checked = 0usize;
    for entry in m["entries"].as_array().unwrap() {
        let name = entry["name"].as_str().unwrap();
        if !files_match(&dir, entry) {
            continue;
        }
        let path = local(&dir, entry["open"].as_str().unwrap());
        let mut s = open(&path);
        let got = summary(&mut s);
        eprintln!("{name}: {got}");
        let expect = &entry["expect"];
        for (k, v) in expect.as_object().unwrap() {
            assert_eq!(&got[k], v, "{name}: `{k}` differs from the manifest");
        }

        let out = temp_dir(name);
        let pbix = out.join("rt.pbix");
        s.execute("file.save_as", &json!({"path": pbix}))
            .unwrap_or_else(|e| panic!("{name}: save as PBIX: {e}"));
        assert_eq!(summary(&mut open(&pbix)), got, "{name}: PBIX round trip");

        let mut s = open(&path);
        let pbip = out.join("rt").join("Rt.pbip");
        s.execute("file.save_as", &json!({"path": pbip}))
            .unwrap_or_else(|e| panic!("{name}: save as PBIP: {e}"));
        assert_eq!(summary(&mut open(&pbip)), got, "{name}: PBIP round trip");

        let _ = std::fs::remove_dir_all(&out);
        checked += 1;
    }
    eprintln!("corpus: {checked} entr(y/ies) checked from {}", dir.display());
}
