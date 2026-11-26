//! An explicit `cache_abf`: a project copy without `.pbi` reads the original's cache, read-only.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

const COUNT: &str = "EVALUATE ROW(\"n\", COUNTROWS(Sales))";

/// A project with a `.pbi/cache.abf` (`orig/Shop.pbip`) and the same project without one
/// (`copy/Shop.pbip`); returns (original cache, copy).
fn projects(dir: &TempDir) -> (PathBuf, PathBuf) {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    let rows: Vec<String> = (0..300).map(|i| format!("{{{i}, {}}}", i % 5)).collect();
    run(
        &mut s,
        "table.import",
        json!({"name": "Sales", "m": format!(
            "let Source = #table(type table [Id = Int64.Type, Qty = Int64.Type], {{{}}}) in Source",
            rows.join(", ")
        )}),
    );
    run(&mut s, "file.save_as", json!({"path": dir.join("shop.pbix")}));
    let orig = dir.join("orig").join("Shop.pbip");
    run(&mut s, "file.save_as", json!({"path": orig}));
    let copy = dir.join("copy").join("Shop.pbip");
    run(&mut s, "file.save_as", json!({"path": copy, "cache": false}));
    let cache = dir.join("orig/Shop.SemanticModel/.pbi/cache.abf");
    assert!(cache.is_file());
    assert!(!dir.join("copy/Shop.SemanticModel/.pbi").exists());
    (cache, copy)
}

fn bytes(p: &Path) -> Vec<u8> {
    std::fs::read(p).unwrap()
}

fn sales_rows(stats: &Value) -> Value {
    stats["tables"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "Sales")
        .unwrap()["rows"]
        .clone()
}

#[test]
fn a_copy_reads_the_original_cache_read_only() {
    let dir = TempDir::new("engine-cache-abf");
    let (cache, copy) = projects(&dir);
    let before = bytes(&cache);

    // Without it the copy has no stored data.
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": copy}));
    assert!(s.execute("model.storage_stats", &json!({})).is_err());
    assert_ne!(
        run(&mut s, "dax.evaluate", json!({"query": COUNT}))["data_source"],
        "cache.abf"
    );

    // `cache_abf` on one command only: the session goes back to no cache afterwards.
    let once = run(&mut s, "model.storage_stats", json!({"cache_abf": cache}));
    assert_eq!(once["source"], "cache_abf");
    assert_eq!(sales_rows(&once), 300);
    assert!(s.execute("model.storage_stats", &json!({})).is_err());
    let v = run(
        &mut s,
        "dax.evaluate",
        json!({"query": COUNT, "cache_abf": cache}),
    );
    assert_eq!(v["data_source"], "cache.abf");
    assert_eq!(v["results"][0]["rows"][0]["[n]"], 300);

    // On file.open: every data reader uses it.
    let mut s = Session::new();
    let opened = run(&mut s, "file.open", json!({"path": copy, "cache_abf": cache}));
    assert!(opened["cache_abf"].as_str().unwrap().ends_with("cache.abf"));
    let v = run(&mut s, "dax.evaluate", json!({"query": COUNT}));
    assert_eq!(v["data_source"], "cache.abf");
    assert_eq!(v["results"][0]["rows"][0]["[n]"], 300);
    let stats = run(&mut s, "model.storage_stats", json!({}));
    assert_eq!(stats["source"], "cache_abf");
    assert_eq!(sales_rows(&stats), 300);
    let vpax = dir.join("copy.vpax");
    run(&mut s, "model.export_vpax", json!({"path": vpax}));
    assert!(vpax.is_file());

    // Saving the copy never writes the cache; a save_as onto the original's folder is refused.
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Total Qty", "expression": "SUM(Sales[Qty])"}),
    );
    run(&mut s, "file.save", json!({}));
    assert!(!dir.join("copy/Shop.SemanticModel/.pbi/cache.abf").exists());
    assert!(
        s.execute(
            "file.save_as",
            &json!({"path": dir.join("orig").join("Shop.pbip")})
        )
        .is_err()
    );
    assert_eq!(bytes(&cache), before);

    // file.set_cache_abf null: back to the project's own (none).
    let v = run(&mut s, "file.set_cache_abf", json!({"cache_abf": null}));
    assert_eq!(v["cache_abf"], Value::Null);
    assert!(s.execute("model.storage_stats", &json!({})).is_err());
    let v = run(&mut s, "file.set_cache_abf", json!({"cache_abf": cache}));
    assert!(v["cache_abf"].is_string());
    assert_eq!(sales_rows(&run(&mut s, "model.storage_stats", json!({}))), 300);
    assert_eq!(bytes(&cache), before);
}

#[test]
fn a_missing_cache_is_an_error_not_a_fallback() {
    let dir = TempDir::new("engine-cache-abf-missing");
    let (cache, copy) = projects(&dir);
    let missing = dir.join("nope.abf");
    let mut s = Session::new();
    let e = s
        .execute("file.open", &json!({"path": copy, "cache_abf": missing}))
        .unwrap_err();
    assert!(e.contains("cache_abf"), "{e}");
    assert!(s.source.is_none());
    run(&mut s, "file.open", json!({"path": copy}));
    assert!(
        s.execute("file.set_cache_abf", &json!({"cache_abf": missing}))
            .is_err()
    );
    assert!(s.execute("file.set_cache_abf", &json!({})).is_err());
    assert!(s.execute("file.set_cache_abf", &json!({"cache_abf": 3})).is_err());
    let e = s
        .execute("dax.evaluate", &json!({"query": COUNT, "cache_abf": missing}))
        .unwrap_err();
    assert!(e.contains("cache_abf"), "{e}");

    // A cache that disappears after it was set: the readers fail, they do not fall back.
    let gone = dir.join("gone.abf");
    std::fs::copy(&cache, &gone).unwrap();
    run(&mut s, "file.set_cache_abf", json!({"cache_abf": gone}));
    std::fs::remove_file(&gone).unwrap();
    let e = s.execute("dax.evaluate", &json!({"query": COUNT})).unwrap_err();
    assert!(e.contains("cache_abf"), "{e}");
    assert!(s.execute("model.storage_stats", &json!({})).is_err());

    // Only a PBIP takes one.
    let mut p = Session::new();
    let e = p
        .execute(
            "file.open",
            &json!({"path": dir.join("shop.pbix"), "cache_abf": cache}),
        )
        .unwrap_err();
    assert!(e.contains("PBIP"), "{e}");
    run(&mut p, "file.open", json!({"path": dir.join("shop.pbix")}));
    assert!(
        p.execute("model.storage_stats", &json!({"cache_abf": cache}))
            .is_err()
    );
}
