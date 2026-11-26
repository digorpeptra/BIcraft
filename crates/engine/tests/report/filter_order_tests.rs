//! `filter.order`: the order of the Filters pane's cards in a section, written as
//! `filterConfig.filterSortOrder` and, in custom order, each card's `ordinal` (public
//! `filterConfiguration` schema), shown in that order by the snapshot and kept by a PBIP, a PBIX
//! and a legacy layout (`docs/formats/pbir.md`, "Filters pane formatting and card order").

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

const SYNTHETIC_METADATA: &[u8] =
    include_bytes!("../../../metadata/tests/fixtures/synthetic_metadata.sqlitedb");

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn shop() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "Sales", "m": "let Source = #table(type table [Region = text, Product = text, Amount = Int64.Type], {{\"East\", \"Bike\", 5}, {\"West\", \"Car\", 7}}) in Source"}),
    );
    run(&mut s, "report.new", json!({}));
    let page = first_page(&s);
    // added out of alphabetical order
    for f in ["Sales[Product]", "Sales[Region]", "Sales[Amount]"] {
        run(&mut s, "page.filter", json!({"page": page, "field": f}));
    }
    s
}

fn first_page(s: &Session) -> String {
    s.report.as_ref().unwrap().pages[0].name.clone()
}

/// The page's cards as the snapshot lists them (column names).
fn shown(s: &Session) -> Vec<String> {
    s.snapshot()["report"]["pages"][0]["filters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["column"].as_str().unwrap().to_string())
        .collect()
}

fn config(s: &Session) -> Value {
    s.report.as_ref().unwrap().pages[0]
        .filter_config
        .clone()
        .unwrap_or(Value::Null)
}

/// Sorted Z to A, then Region moved last: Product, Amount, Region in custom order.
fn reorder(s: &mut Session) {
    let page = first_page(s);
    run(
        s,
        "filter.order",
        json!({"level": "page", "page": page, "sort": "descending"}),
    );
    let out = run(
        s,
        "filter.order",
        json!({"level": "page", "page": page, "field": "Sales[Region]", "to": 2}),
    );
    assert_eq!(out["changed"], true, "{out}");
}

fn check_custom(s: &Session, label: &str) {
    assert_eq!(shown(s), ["Product", "Amount", "Region"], "{label}");
    let cfg = config(s);
    assert_eq!(cfg["filterSortOrder"], "Custom", "{label}: {cfg}");
    let ordinal = |col: &str| {
        cfg["filters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["field"]["Column"]["Property"] == col)
            .map(|c| c["ordinal"].clone())
            .unwrap_or(Value::Null)
    };
    assert_eq!(
        (ordinal("Product"), ordinal("Amount"), ordinal("Region")),
        (json!(0), json!(1), json!(2)),
        "{label}: {cfg}"
    );
    assert_eq!(
        s.snapshot()["report"]["pages"][0]["filter_sort"],
        "custom",
        "{label}"
    );
}

#[test]
fn cards_are_listed_a_to_z_then_sorted_and_moved() {
    let mut s = shop();
    // no filterSortOrder: A to Z, as Desktop lists filters by default
    assert_eq!(shown(&s), ["Amount", "Product", "Region"]);
    assert_eq!(s.snapshot()["report"]["pages"][0]["filter_sort"], "ascending");
    let page = first_page(&s);
    run(
        &mut s,
        "filter.order",
        json!({"level": "page", "page": page, "sort": "descending"}),
    );
    assert_eq!(shown(&s), ["Region", "Product", "Amount"]);
    assert_eq!(config(&s)["filterSortOrder"], "Descending");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(shown(&s), ["Amount", "Product", "Region"]);
    reorder(&mut s);
    check_custom(&s, "session");
    // custom from the shown order keeps it; A to Z sorts again, the ordinals kept
    let out = run(
        &mut s,
        "filter.order",
        json!({"level": "page", "page": page, "sort": "custom"}),
    );
    assert_eq!(out["changed"], false, "{out}");
    run(
        &mut s,
        "filter.order",
        json!({"level": "page", "page": page, "sort": "ascending"}),
    );
    assert_eq!(shown(&s), ["Amount", "Product", "Region"]);
    run(&mut s, "edit.undo", json!({}));
    check_custom(&s, "undo");
    // moving to a position past the end puts the card last
    run(
        &mut s,
        "filter.order",
        json!({"level": "page", "page": page, "field": "Sales[Product]", "to": 50}),
    );
    assert_eq!(shown(&s), ["Amount", "Region", "Product"]);
}

#[test]
fn report_and_visual_sections_are_ordered_too() {
    let mut s = shop();
    for f in ["Sales[Region]", "Sales[Amount]"] {
        run(&mut s, "report.filter", json!({"field": f}));
    }
    run(
        &mut s,
        "filter.order",
        json!({"level": "report", "field": "Sales[Region]", "to": 0}),
    );
    let snap = s.snapshot();
    let report: Vec<&str> = snap["report"]["filters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["column"].as_str().unwrap())
        .collect();
    assert_eq!(report, ["Region", "Amount"]);
    assert_eq!(snap["report"]["filter_sort"], "custom");
    let page = first_page(&s);
    let v = run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "tableEx", "x": 10, "y": 10, "width": 300, "height": 200, "fields": [{"role": "Values", "field": "Sales[Region]"}]}),
    );
    let visual = v["visual"].as_str().unwrap().to_string();
    for f in ["Sales[Region]", "Sales[Amount]"] {
        run(
            &mut s,
            "visual.filter",
            json!({"page": page, "visual": visual, "field": f}),
        );
    }
    run(
        &mut s,
        "filter.order",
        json!({"level": "visual", "page": page, "visual": visual, "sort": "descending"}),
    );
    let vs = s.snapshot()["report"]["pages"][0]["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == visual.as_str())
        .cloned()
        .unwrap();
    assert_eq!(vs["filter_sort"], "descending");
    assert_eq!(vs["filters"][0]["column"], "Region", "{}", vs["filters"]);
}

/// The session after a save as `file` and a fresh open.
fn reopened(s: &mut Session, dir: &TempDir, file: &str) -> Session {
    run(s, "file.save_as", json!({"path": dir.join(file)}));
    let mut back = Session::new();
    run(&mut back, "file.open", json!({"path": dir.join(file)}));
    back
}

#[test]
fn pbip_and_pbix_keep_the_card_order() {
    let dir = TempDir::new("filter-order-files");
    for file in ["Shop.pbip", "Shop.pbix"] {
        let mut s = shop();
        reorder(&mut s);
        let back = reopened(&mut s, &dir, file);
        check_custom(&back, file);
    }
}

fn synthetic(dir: &TempDir, pbir: bool) -> PathBuf {
    let path = dir.join(if pbir { "Pbir.pbix" } else { "Legacy.pbix" });
    let pkg = if pbir {
        bicraft_pbix::synthetic::pbir_pbix(SYNTHETIC_METADATA)
    } else {
        bicraft_pbix::synthetic::pbix(SYNTHETIC_METADATA)
    }
    .unwrap();
    std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
    path
}

fn open(path: &Path) -> Session {
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    s
}

#[test]
fn a_legacy_layout_and_pbir_parts_keep_the_card_order() {
    let dir = TempDir::new("filter-order-legacy");
    for pbir in [false, true] {
        let path = synthetic(&dir, pbir);
        let mut s = open(&path);
        let page = first_page(&s);
        // two columns of the synthetic model, whatever its tables
        let cols: Vec<String> = s.snapshot()["model"]["tables"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|t| {
                let table = t["name"].as_str().unwrap_or("").to_string();
                t["columns"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(move |c| c["name"].as_str().map(|n| format!("{table}[{n}]")))
            })
            .filter(|f| !f.contains("RowNumber"))
            .take(2)
            .collect();
        assert_eq!(cols.len(), 2, "the synthetic model has two columns");
        for f in &cols {
            run(&mut s, "page.filter", json!({"page": page, "field": f}));
        }
        let before = shown(&s);
        // the last shown card first: the custom order
        run(
            &mut s,
            "filter.order",
            json!({"level": "page", "page": page, "field": cols.iter().find(|f| f.ends_with(&format!("[{}]", before[1]))).unwrap(), "to": 0}),
        );
        let want = vec![before[1].clone(), before[0].clone()];
        assert_eq!(shown(&s), want);
        run(&mut s, "file.save", json!({}));
        let back = open(&path);
        let label = if pbir { "pbir parts" } else { "legacy layout" };
        assert_eq!(shown(&back), want, "{label}");
        assert_eq!(config(&back)["filterSortOrder"], "Custom", "{label}");
    }
}

#[test]
fn bad_orders_are_refused() {
    let mut s = shop();
    let page = first_page(&s);
    for bad in [
        json!({"level": "page", "page": page}),
        json!({"level": "section", "page": page, "sort": "ascending"}),
        json!({"level": "page", "page": page, "sort": "random"}),
        json!({"level": "page", "page": page, "field": "Sales[Region]"}),
        json!({"level": "page", "page": page, "to": 1}),
        json!({"level": "page", "page": page, "field": "Sales[Region]", "to": -1}),
        json!({"level": "page", "page": page, "field": "Sales[Region]", "to": "first"}),
        json!({"level": "page", "page": page, "field": "Sales[Region]", "to": 0, "sort": "ascending"}),
        json!({"level": "page", "page": page, "field": "Sales[Nope]", "to": 0}),
        json!({"level": "report", "sort": "ascending"}),
        json!({"level": "report", "page": page, "sort": "ascending"}),
        json!({"level": "visual", "page": page, "visual": "nope", "sort": "ascending"}),
        json!({"level": "page", "page": page, "sort": "ascending", "extra": 1}),
        json!({"page": page, "sort": "ascending"}),
    ] {
        assert!(s.execute("filter.order", &bad).is_err(), "{bad}");
    }
    // a damaged filter list is refused, not crashed on
    s.report.as_mut().unwrap().pages[0].filter_config = Some(json!({"filters": 3}));
    assert!(
        s.execute(
            "filter.order",
            &json!({"level": "page", "page": page, "sort": "ascending"})
        )
        .is_err()
    );
    s.report.as_mut().unwrap().pages[0].filter_config =
        Some(json!({"filters": [1, {"ordinal": "x"}], "filterSortOrder": "Custom"}));
    let _ = s.snapshot();
    let _ = s.execute(
        "filter.order",
        &json!({"level": "page", "page": page, "sort": "custom"}),
    );
    let mut empty = Session::new();
    assert!(
        empty
            .execute("filter.order", &json!({"level": "report", "sort": "ascending"}))
            .is_err()
    );
}
