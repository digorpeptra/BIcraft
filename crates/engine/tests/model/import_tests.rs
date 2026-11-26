//! `objects.import` on generated files: every object kind, dependency closure, conflict modes,
//! renames, cross-format imports, the report side, refusals, dry runs, undo, hostile input and
//! server mode.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn fails(s: &mut Session, params: Value) -> String {
    match s.execute("objects.import", &params) {
        Ok(v) => panic!("objects.import {params} succeeded: {v}"),
        Err(e) => e,
    }
}

fn import(s: &mut Session, params: Value) -> Value {
    run(s, "objects.import", params)
}

const SALES_M: &str = "let Source = #table(type table [ProductKey = Int64.Type, Amount = number], \
                       {{1, 10}, {2, 20}, {1, 5}}) in Source";
const PRODUCT_M: &str = "let Source = #table(type table [ProductKey = Int64.Type, Name = text], \
                         {{1, \"Apple\"}, {2, \"Pear\"}}) in Source";

/// The source model "Shop": Sales and Product with a relationship, measures that depend on each
/// other, a calculated column, a parameter, a calculation group, a function, a role and a report
/// with two pages, a bookmark and a custom theme.
fn shop() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(&mut s, "table.import", json!({"name": "Sales", "m": SALES_M}));
    run(&mut s, "table.import", json!({"name": "Product", "m": PRODUCT_M}));
    run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[ProductKey]", "to": "Product[ProductKey]"}),
    );
    for (name, expr) in [
        ("Total", "SUM(Sales[Amount])"),
        ("Margin", "[Total] * 0.3"),
        ("Apple Margin", "CALCULATE([Margin], Product[Name] = \"Apple\")"),
    ] {
        run(
            &mut s,
            "measure.add",
            json!({"table": "Sales", "name": name, "expression": expr}),
        );
    }
    run(
        &mut s,
        "column.add",
        json!({"table": "Sales", "name": "Double", "data_type": "double", "expression": "Sales[Amount] * 2"}),
    );
    run(
        &mut s,
        "expression.add",
        json!({"name": "Rate", "expression": "0.2 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]"}),
    );
    run(&mut s, "calculation_group.add", json!({"name": "Time Intel"}));
    run(
        &mut s,
        "calculation_item.add",
        json!({"group": "Time Intel", "name": "Current", "expression": "SELECTEDMEASURE()"}),
    );
    run(&mut s, "role.add", json!({"name": "Reader"}));
    run(
        &mut s,
        "role.set_filter",
        json!({"role": "Reader", "table": "Product", "filter_expression": "Product[Name] = \"Apple\""}),
    );
    s.model.as_mut().unwrap().extra.insert(
        "functions".into(),
        json!([{"name": "AddOne", "expression": "(x : SCALAR NUMERIC VAL) => x + 1"}]),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Plus", "expression": "AddOne([Total])"}),
    );
    run(&mut s, "report.new", json!({}));
    run(&mut s, "page.add", json!({"display_name": "Overview"}));
    run(
        &mut s,
        "visual.add",
        json!({"page": "Overview", "visual_type": "card", "x": 10, "y": 10, "width": 200, "height": 100,
               "fields": [{"role": "Values", "field": "[Apple Margin]"}]}),
    );
    run(
        &mut s,
        "visual.add",
        json!({"page": "Overview", "visual_type": "tableEx", "x": 300, "y": 10, "width": 300, "height": 200,
               "fields": [{"role": "Values", "field": "Product[Name]"}]}),
    );
    run(&mut s, "page.add", json!({"display_name": "Detail"}));
    run(
        &mut s,
        "theme.set",
        json!({"theme": {"name": "Shop Colors", "dataColors": ["#112233", "#445566"]}}),
    );
    let overview = s.report.as_ref().unwrap().page("Overview").unwrap().name.clone();
    s.report
        .as_mut()
        .unwrap()
        .bookmarks
        .push(bicraft_report::NamedDocument {
            name: "bm1".into(),
            document: json!({"name": "bm1", "displayName": "Start",
                "explorationState": {"version": "1.3", "activeSection": overview,
                    "sections": {overview.clone(): {"visualContainers": {}}}}}),
        });
    s.report_dirty = true;
    s
}

/// The target "Target": a Sales table only, and a blank report.
fn target() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Target"}));
    run(&mut s, "table.import", json!({"name": "Sales", "m": SALES_M}));
    run(&mut s, "report.new", json!({}));
    s
}

fn project(s: &mut Session, dir: &Path, name: &str) -> PathBuf {
    run(s, "project.new", json!({"dir": dir, "name": name}));
    dir.join(format!("{name}.pbip"))
}

fn shop_pbip(dir: &Path) -> PathBuf {
    project(&mut shop(), &dir.join("src"), "Shop")
}

fn target_pbip(dir: &Path) -> (PathBuf, Session) {
    let path = project(&mut target(), &dir.join("tgt"), "Target");
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    (path, s)
}

fn measure_expr(s: &Session, name: &str) -> Option<String> {
    s.model
        .as_ref()
        .unwrap()
        .measure(name)
        .map(|(_, x)| x.expression.clone())
}

fn action<'a>(out: &'a Value, kind: &str, name: &str) -> &'a str {
    out["objects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["kind"] == kind && (o["name"] == name || o["source_name"] == name))
        .and_then(|o| o["action"].as_str())
        .unwrap_or_else(|| panic!("no {kind} {name} in {out}"))
}

#[test]
fn a_measure_brings_its_dependencies_and_their_relationship() {
    let dir = TempDir::new("import-deps");
    let src = shop_pbip(&dir);
    let (_, mut s) = target_pbip(&dir);
    let out = import(&mut s, json!({"source": src, "measures": ["Apple Margin"]}));
    assert_eq!(out["applied"], true, "{out}");
    assert_eq!(action(&out, "measure", "Apple Margin"), "added");
    assert_eq!(action(&out, "measure", "Margin"), "added");
    assert_eq!(action(&out, "measure", "Total"), "added");
    assert_eq!(action(&out, "table", "Product"), "added");
    let deps = out["dependencies"].as_array().unwrap();
    let margin = deps.iter().find(|d| d["name"] == "Margin").unwrap();
    assert_eq!(margin["required_by"], "measure [Apple Margin]", "{out}");
    assert_eq!(margin["from"], "source");
    let total = deps.iter().find(|d| d["name"] == "Total").unwrap();
    assert_eq!(total["required_by"], "measure [Margin]");
    // Sales is the target's own
    assert!(
        deps.iter().any(|d| d["name"] == "Sales" && d["from"] == "target"),
        "{out}"
    );
    assert_eq!(out["relationships"].as_array().unwrap().len(), 1, "{out}");
    let m = s.model.as_ref().unwrap();
    assert!(m.table("Product").is_some());
    assert_eq!(m.relationships.len(), 1);
    assert!(s.dirty);
    // without dependencies, the measure alone does not validate: refused
    let (_, mut bare) = target_pbip(&TempDir::new("import-deps-bare"));
    let e = fails(
        &mut bare,
        json!({"source": src, "measures": ["Apple Margin"], "include_dependencies": false}),
    );
    assert!(e.contains("would not validate"), "{e}");
    assert!(bare.model.as_ref().unwrap().measure("Apple Margin").is_none());
}

#[test]
fn conflicts_fail_by_default_and_follow_on_conflict() {
    let dir = TempDir::new("import-conflicts");
    let src = shop_pbip(&dir);
    let tgt = || {
        let mut s = target();
        run(
            &mut s,
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "1"}),
        );
        s
    };
    let picks = json!(["Total", "Margin"]);
    // fail
    let mut s = tgt();
    let e = fails(&mut s, json!({"source": src, "measures": picks}));
    assert!(e.contains("already exist") && e.contains("[Total]"), "{e}");
    assert_eq!(measure_expr(&s, "Total").as_deref(), Some("1"));
    assert!(measure_expr(&s, "Margin").is_none());
    // skip: Margin uses the target's Total
    let mut s = tgt();
    let out = import(
        &mut s,
        json!({"source": src, "measures": picks, "on_conflict": "skip"}),
    );
    assert_eq!(action(&out, "measure", "Total"), "skipped");
    assert_eq!(action(&out, "measure", "Margin"), "added");
    assert_eq!(measure_expr(&s, "Total").as_deref(), Some("1"));
    assert_eq!(measure_expr(&s, "Margin").as_deref(), Some("[Total] * 0.3"));
    // replace
    let mut s = tgt();
    let out = import(
        &mut s,
        json!({"source": src, "measures": picks, "on_conflict": "replace"}),
    );
    assert_eq!(action(&out, "measure", "Total"), "replaced");
    assert_eq!(measure_expr(&s, "Total").as_deref(), Some("SUM(Sales[Amount])"));
    // rename: Margin follows the new name of Total
    let mut s = tgt();
    let out = import(
        &mut s,
        json!({"source": src, "measures": picks, "on_conflict": "rename"}),
    );
    assert_eq!(action(&out, "measure", "Total"), "renamed");
    assert_eq!(measure_expr(&s, "Total").as_deref(), Some("1"));
    assert_eq!(
        measure_expr(&s, "Total (imported)").as_deref(),
        Some("SUM(Sales[Amount])")
    );
    assert_eq!(
        measure_expr(&s, "Margin").as_deref(),
        Some("[Total (imported)] * 0.3"),
        "{out}"
    );
    // a custom suffix, and a second import picks the next free name
    let out = import(
        &mut s,
        json!({"source": src, "measures": ["Total"], "on_conflict": "rename", "rename_suffix": "_src"}),
    );
    assert_eq!(out["applied"], true);
    assert!(measure_expr(&s, "Total_src").is_some());
    let out = import(
        &mut s,
        json!({"source": src, "measures": ["Total"], "on_conflict": "rename", "rename_suffix": "_src"}),
    );
    assert_eq!(out["applied"], true);
    assert!(measure_expr(&s, "Total_src 2").is_some(), "{out}");
}

#[test]
fn every_model_object_kind_imports() {
    let dir = TempDir::new("import-kinds");
    let src = shop_pbip(&dir);
    let (path, mut s) = target_pbip(&dir);
    let out = import(
        &mut s,
        json!({"source": src, "tables": ["Product"], "calculation_groups": ["Time*"],
               "functions": ["AddOne"], "expressions": ["Rate"], "roles": ["Reader"],
               "columns": ["Sales[Double]"], "measures": ["Plus"]}),
    );
    assert_eq!(out["applied"], true, "{out}");
    assert_eq!(action(&out, "calculation group", "Time Intel"), "added");
    assert_eq!(action(&out, "function", "AddOne"), "added");
    assert_eq!(action(&out, "expression", "Rate"), "added");
    assert_eq!(action(&out, "role", "Reader"), "added");
    assert_eq!(action(&out, "calculated column", "Double"), "added");
    // Plus pulled Total in
    assert_eq!(action(&out, "measure", "Total"), "added");
    run(&mut s, "file.save", json!({}));
    let mut back = Session::new();
    run(&mut back, "file.open", json!({"path": path}));
    let m = back.model.as_ref().unwrap();
    assert!(
        m.table("Time Intel")
            .unwrap()
            .extra
            .get("calculationGroup")
            .is_some()
    );
    assert!(m.expressions.iter().any(|e| e.name == "Rate"));
    assert!(m.roles.iter().any(|r| r.name == "Reader"));
    assert!(m.table("Sales").unwrap().column("Double").is_some());
    assert_eq!(m.extra["functions"][0]["name"], "AddOne");
    assert!(m.measure("Plus").is_some());
    let v = run(&mut back, "model.validate", json!({}));
    assert_eq!(v["invalid_count"], 0, "{v}");
}

#[test]
fn a_renamed_table_takes_its_references_along() {
    let dir = TempDir::new("import-rename-table");
    let src = shop_pbip(&dir);
    let mut s = target();
    run(&mut s, "table.import", json!({"name": "Product", "m": PRODUCT_M}));
    let out = import(
        &mut s,
        json!({"source": src, "measures": ["Apple Margin"], "tables": ["Product"], "on_conflict": "rename"}),
    );
    assert_eq!(action(&out, "table", "Product"), "renamed", "{out}");
    let m = s.model.as_ref().unwrap();
    assert!(m.table("Product (imported)").is_some());
    assert_eq!(
        measure_expr(&s, "Apple Margin").as_deref(),
        Some("CALCULATE([Margin], 'Product (imported)'[Name] = \"Apple\")")
    );
    // the relationship follows the new table name
    let rels = out["relationships"].as_array().unwrap();
    assert_eq!(rels[0]["to"], "'Product (imported)'[ProductKey]", "{out}");
}

#[test]
fn a_replaced_table_keeps_its_place_and_its_relationships() {
    let dir = TempDir::new("import-replace-table");
    let src = shop_pbip(&dir);
    let mut s = target();
    run(&mut s, "table.import", json!({"name": "Product", "m": PRODUCT_M}));
    run(&mut s, "table.import", json!({"name": "Other", "m": PRODUCT_M}));
    run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[ProductKey]", "to": "Product[ProductKey]"}),
    );
    let out = import(
        &mut s,
        json!({"source": src, "tables": ["Product"], "on_conflict": "replace"}),
    );
    assert_eq!(action(&out, "table", "Product"), "replaced");
    let m = s.model.as_ref().unwrap();
    assert_eq!(m.tables[1].name, "Product");
    // the existing relationship is not doubled
    assert_eq!(m.relationships.len(), 1, "{out}");
}

#[test]
fn into_table_moves_measures_and_their_qualifiers() {
    let dir = TempDir::new("import-into");
    let src = shop_pbip(&dir);
    let mut s = target();
    run(
        &mut s,
        "table.import",
        json!({"name": "Measures", "m": "#table(type table [X = Int64.Type], {{1}})"}),
    );
    let out = import(
        &mut s,
        json!({"source": src, "measures": ["Total"], "into_table": "Measures"}),
    );
    assert_eq!(out["objects"][0]["into_table"], "Measures", "{out}");
    let m = s.model.as_ref().unwrap();
    assert!(m.table("Measures").unwrap().measure("Total").is_some());
    // a missing home table is an error naming into_table
    let mut t = Session::new();
    run(&mut t, "model.new", json!({"name": "Empty"}));
    let e = fails(
        &mut t,
        json!({"source": src, "measures": ["Total"], "include_dependencies": false}),
    );
    assert!(e.contains("into_table"), "{e}");
}

#[test]
fn pages_visuals_bookmarks_and_theme() {
    let dir = TempDir::new("import-report");
    let src = shop_pbip(&dir);
    let (path, mut s) = target_pbip(&dir);
    let out = import(
        &mut s,
        json!({"source": src, "pages": ["Overview"], "bookmarks": ["Start"], "theme": true}),
    );
    assert_eq!(out["applied"], true, "{out}");
    assert_eq!(action(&out, "page", "Overview"), "added");
    assert_eq!(action(&out, "bookmark", "Start"), "added");
    assert_eq!(action(&out, "theme", "Shop Colors"), "added");
    // the page's fields pulled their measures and Product in
    assert_eq!(action(&out, "measure", "Apple Margin"), "added");
    assert_eq!(action(&out, "table", "Product"), "added");
    assert!(s.report_dirty && s.dirty);
    run(&mut s, "file.save", json!({}));
    let mut back = Session::new();
    run(&mut back, "file.open", json!({"path": path}));
    let r = back.report.as_ref().unwrap();
    let page = r.page("Overview").unwrap();
    assert_eq!(page.visuals.len(), 2);
    assert_eq!(r.bookmarks.len(), 1);
    assert_eq!(
        r.bookmarks[0].document["explorationState"]["activeSection"],
        page.name.as_str()
    );
    let theme = run(&mut back, "theme.get", json!({}));
    assert_eq!(theme["custom"]["theme_name"], "Shop Colors", "{theme}");
    let v = run(&mut back, "report.validate", json!({}));
    assert_eq!(v["valid"], true, "{v}");

    // the same page again: a conflict on its display name; rename gives a new page and id
    let e = fails(&mut back, json!({"source": src, "pages": ["Overview"]}));
    assert!(e.contains("page `Overview`"), "{e}");
    let out = import(
        &mut back,
        json!({"source": src, "pages": ["Overview"], "on_conflict": "rename"}),
    );
    assert_eq!(action(&out, "page", "Overview"), "renamed");
    let r = back.report.as_ref().unwrap();
    let a = r.page("Overview").unwrap().name.clone();
    let b = r.page("Overview (imported)").unwrap().name.clone();
    assert_ne!(a, b);
    // the theme a second time: kept unless replaced
    let out = import(
        &mut back,
        json!({"source": src, "theme": true, "on_conflict": "skip"}),
    );
    assert_eq!(action(&out, "theme", "Shop Colors"), "skipped");
}

#[test]
fn single_visuals_land_on_the_target_page() {
    let dir = TempDir::new("import-visuals");
    let src = shop_pbip(&dir);
    let (_, mut s) = target_pbip(&dir);
    let src_report = {
        let mut x = Session::new();
        run(&mut x, "file.open", json!({"path": src}));
        x.report.unwrap()
    };
    let card = src_report.page("Overview").unwrap().visuals[0].name.clone();
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let e = fails(
        &mut s,
        json!({"source": src, "visuals": [format!("Overview/{card}")]}),
    );
    assert!(e.contains("target_page"), "{e}");
    let out = import(
        &mut s,
        json!({"source": src, "visuals": [{"page": "Overview", "visual": card}], "target_page": page}),
    );
    assert_eq!(out["applied"], true, "{out}");
    let p = &s.report.as_ref().unwrap().pages[0];
    assert_eq!(p.visuals.len(), 1);
    assert_eq!(p.visuals[0].name, card);
    // again: a new visual id, no conflict
    let out = import(
        &mut s,
        json!({"source": src, "visuals": [format!("Overview/{card}")], "target_page": page}),
    );
    assert_eq!(out["applied"], true, "{out}");
    let p = &s.report.as_ref().unwrap().pages[0];
    assert_eq!(p.visuals.len(), 2);
    assert_ne!(p.visuals[0].name, p.visuals[1].name);
}

#[test]
fn broken_visual_fields_are_refused_unless_allowed() {
    let dir = TempDir::new("import-broken");
    let src = shop_pbip(&dir);
    let (_, mut s) = target_pbip(&dir);
    let before = s.report.clone();
    let p = json!({"source": src, "pages": ["Overview"], "include_dependencies": false});
    let e = fails(&mut s, p.clone());
    assert!(e.contains("do not resolve") && e.contains("Apple Margin"), "{e}");
    assert_eq!(s.report, before);
    // a dry run lists them
    let mut dry = p.clone();
    dry["dry_run"] = json!(true);
    let out = import(&mut s, dry);
    assert_eq!(out["would_apply"], false);
    assert!(!out["broken_fields"].as_array().unwrap().is_empty(), "{out}");
    let mut allowed = p;
    allowed["allow_broken"] = json!(true);
    let out = import(&mut s, allowed);
    assert_eq!(out["applied"], true);
    assert!(!out["warnings"].as_array().unwrap().is_empty(), "{out}");
}

#[test]
fn a_dry_run_changes_nothing_and_undo_restores() {
    let dir = TempDir::new("import-dry-undo");
    let src = shop_pbip(&dir);
    let (_, mut s) = target_pbip(&dir);
    let model = s.model.clone();
    let report = s.report.clone();
    let out = import(
        &mut s,
        json!({"source": src, "measures": ["Apple Margin"], "pages": ["Overview"], "dry_run": true}),
    );
    assert_eq!(out["applied"], false);
    assert_eq!(out["would_apply"], true, "{out}");
    assert_eq!(action(&out, "measure", "Margin"), "added");
    assert_eq!(s.model, model);
    assert_eq!(s.report, report);
    assert!(!s.dirty && !s.report_dirty);
    let h = run(&mut s, "edit.history", json!({}));
    assert_eq!(h["undo"].as_array().map_or(0, Vec::len), 0, "{h}");

    import(
        &mut s,
        json!({"source": src, "measures": ["Apple Margin"], "pages": ["Overview"], "theme": true}),
    );
    assert_ne!(s.model, model);
    let h = run(&mut s, "edit.history", json!({}));
    assert_eq!(h["undo"].as_array().unwrap().len(), 1, "one undo step: {h}");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.model, model);
    assert_eq!(s.report, report);
}

#[test]
fn cross_format_imports() {
    let dir = TempDir::new("import-formats");
    // PBIX source → PBIP target
    let pbix = dir.join("Shop.pbix");
    let mut shop_s = shop();
    // the PBIX writer runs M: keep the model it can write
    run(
        &mut shop_s,
        "calculation_group.remove",
        json!({"name": "Time Intel"}),
    );
    run(&mut shop_s, "file.save_as", json!({"path": pbix}));
    let (path, mut s) = target_pbip(&dir);
    let out = import(
        &mut s,
        json!({"source": pbix, "measures": ["Apple Margin"], "pages": ["Overview"]}),
    );
    assert_eq!(out["applied"], true, "{out}");
    run(&mut s, "file.save", json!({}));
    let mut back = Session::new();
    run(&mut back, "file.open", json!({"path": path}));
    assert!(measure_expr(&back, "Apple Margin").is_some());
    assert!(back.report.as_ref().unwrap().page("Overview").is_some());

    // PBIP source → PBIX target, saved with its data
    let src = shop_pbip(&dir);
    let tpbix = dir.join("Target.pbix");
    run(&mut target(), "file.save_as", json!({"path": tpbix}));
    let mut t = Session::new();
    run(&mut t, "file.open", json!({"path": tpbix}));
    let out = import(&mut t, json!({"source": src, "measures": ["Margin"]}));
    assert_eq!(out["applied"], true, "{out}");
    run(&mut t, "file.save", json!({"backup": false}));
    let mut back = Session::new();
    run(&mut back, "file.open", json!({"path": tpbix}));
    assert_eq!(measure_expr(&back, "Margin").as_deref(), Some("[Total] * 0.3"));
    // shared expressions are not written into a PBIX's DataModel
    let e = fails(&mut back, json!({"source": src, "expressions": ["Rate"]}));
    assert!(e.contains("shared expression"), "{e}");

    // model.bim source → PBIP target
    let bim = dir.join("Shop.bim");
    run(
        &mut Session::new(),
        "file.convert",
        json!({"from": src, "to": bim}),
    );
    let (_, mut s) = target_pbip(&TempDir::new("import-formats-bim"));
    let out = import(
        &mut s,
        json!({"source": bim, "tables": ["Product"], "roles": ["Reader"]}),
    );
    assert_eq!(out["applied"], true, "{out}");
    let e = fails(&mut s, json!({"source": bim, "pages": ["Overview"]}));
    assert!(e.contains("no report"), "{e}");
}

#[test]
fn lineage_tags_are_regenerated_when_they_would_repeat() {
    let dir = TempDir::new("import-tags");
    let src = shop_pbip(&dir);
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": src}));
    // the file into itself: every tag already exists
    let out = import(
        &mut s,
        json!({"source": src, "tables": ["Product"], "on_conflict": "rename"}),
    );
    assert!(out["lineage_tags_regenerated"].as_u64().unwrap() > 0, "{out}");
    let m = s.model.as_ref().unwrap();
    let a = m.table("Product").unwrap().lineage_tag.clone();
    let b = m.table("Product (imported)").unwrap().lineage_tag.clone();
    assert!(a.is_some() && a != b);
    // the duplicate gets its own relationship to Sales, under a new name
    assert_eq!(m.relationships.len(), 2, "{out}");
    assert_ne!(m.relationships[0].name, m.relationships[1].name);
}

#[test]
fn hostile_inputs_are_refused_cleanly() {
    let dir = TempDir::new("import-hostile");
    // a cycle of measures and a huge name, in a model.bim written directly
    let bim = dir.join("Bad.bim");
    let huge = "M".repeat(5000);
    std::fs::write(
        &bim,
        serde_json::to_vec(
            &json!({"name": "Bad", "compatibilityLevel": 1600, "model": {"tables": [
            {"name": "T", "columns": [{"name": "A", "dataType": "double", "sourceColumn": "A"}],
             "measures": [{"name": "X", "expression": "[Y] + 1"}, {"name": "Y", "expression": "[X] + 1"},
                          {"name": huge, "expression": "1"}]}]}}),
        )
        .unwrap(),
    )
    .unwrap();
    let mut s = target();
    run(
        &mut s,
        "table.import",
        json!({"name": "T", "m": "#table(type table [A = number], {{1}})"}),
    );
    // the cycle: the closure ends; the result is an answer, never an internal error
    match s.execute(
        "objects.import",
        &json!({"source": bim, "measures": ["X"], "dry_run": true}),
    ) {
        Ok(v) => assert!(v["objects"].as_array().unwrap().len() >= 2, "{v}"),
        Err(e) => assert!(!e.contains("internal"), "{e}"),
    }
    let e = fails(&mut target(), json!({"source": bim, "tables": ["T"]}));
    assert!(e.contains("longer than"), "{e}");
    // huge selectors, bad parameters, missing files
    let e = fails(&mut s, json!({"source": bim, "measures": ["x".repeat(5000)]}));
    assert!(e.contains("longer than"), "{e}");
    let e = fails(&mut s, json!({"source": bim, "measures": ["Nope"]}));
    assert!(e.contains("no measure of the source matches"), "{e}");
    let e = fails(
        &mut s,
        json!({"source": bim, "measures": ["X"], "on_conflict": "merge"}),
    );
    assert!(e.contains("on_conflict"), "{e}");
    let e = fails(&mut s, json!({"source": bim}));
    assert!(e.contains("nothing to import"), "{e}");
    let e = fails(
        &mut s,
        json!({"source": dir.join("missing.pbix"), "measures": ["X"]}),
    );
    assert!(e.contains("cannot open the source"), "{e}");
    let e = fails(&mut s, json!({"source": bim, "columns": ["T[A]"]}));
    assert!(e.contains("data column"), "{e}");
    let e = fails(&mut s, json!({"source": bim, "bogus": 1}));
    assert!(e.contains("unknown parameter"), "{e}");
    let e = fails(
        &mut s,
        json!({"source": bim, "measures": ["X"], "rename_suffix": ""}),
    );
    assert!(e.contains("rename_suffix"), "{e}");
    // nothing changed
    assert!(s.model.as_ref().unwrap().measure("X").is_none());
}

#[test]
fn server_mode_resolves_the_source_under_the_root() {
    let dir = TempDir::new("import-server");
    let src = project(&mut shop(), &dir.join("in"), "Shop");
    assert!(src.exists());
    let mut s = Session::server(bicraft_engine::ServerConfig::new(dir.path()).unwrap());
    run(&mut s, "model.new", json!({"name": "T"}));
    run(&mut s, "table.import", json!({"name": "Sales", "m": SALES_M}));
    let out = import(&mut s, json!({"source": "in/Shop.pbip", "measures": ["Margin"]}));
    assert_eq!(out["applied"], true, "{out}");
    for bad in [
        "../Shop.pbip",
        "/etc/passwd",
        "C:/Windows/win.ini",
        "\\\\server\\share\\x.pbix",
    ] {
        let e = fails(&mut s, json!({"source": bad, "measures": ["Margin"]}));
        assert!(e.contains("server mode"), "{bad}: {e}");
    }
}

/// A page background held in the report's registered resources.
fn with_image(s: &mut Session, page: &str, bytes: &[u8]) {
    let r = s.report.as_mut().unwrap();
    r.extra.insert(
        "resourcePackages".into(),
        json!([{"name": "RegisteredResources", "type": "RegisteredResources",
                "items": [{"name": "logo1.png", "path": "logo1.png", "type": "Image"}]}]),
    );
    r.resources.push(bicraft_report::Resource {
        path: "StaticResources/RegisteredResources/logo1.png".into(),
        bytes: bytes.to_vec(),
    });
    r.page_mut(page).unwrap().extra.insert(
        "objects".into(),
        json!({"background": [{"properties": {"image": {"image": {
            "name": {"expr": {"Literal": {"Value": "'logo1.png'"}}},
            "url": {"expr": {"ResourcePackageItem": {"PackageName": "RegisteredResources",
                "PackageType": 1, "ItemName": "logo1.png"}}}}}}}]}),
    );
    s.report_dirty = true;
}

#[test]
fn registered_images_come_along_and_never_overwrite() {
    let dir = TempDir::new("import-images");
    let mut src_s = shop();
    with_image(&mut src_s, "Overview", b"\x89PNG source");
    let src = project(&mut src_s, &dir.join("src"), "Shop");
    let mut s = target();
    let first = s.report.as_ref().unwrap().pages[0].display_name.clone();
    with_image(&mut s, &first, b"\x89PNG target");
    let out = import(&mut s, json!({"source": src, "pages": ["Overview"]}));
    assert_eq!(out["applied"], true, "{out}");
    let r = s.report.as_ref().unwrap();
    let img = &r.page("Overview").unwrap().extra["objects"]["background"][0]["properties"]["image"]["image"]
        ["url"]["expr"]["ResourcePackageItem"]["ItemName"];
    assert_eq!(img, "logo1_2.png");
    let res = r
        .resources
        .iter()
        .find(|x| x.path.ends_with("logo1_2.png"))
        .unwrap();
    assert_eq!(res.bytes, b"\x89PNG source");
    let target_logo = r
        .resources
        .iter()
        .find(|x| x.path.ends_with("/logo1.png"))
        .unwrap();
    assert_eq!(target_logo.bytes, b"\x89PNG target");
    let items = r.extra["resourcePackages"][0]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
}

#[test]
fn a_second_active_path_between_two_tables_is_imported_inactive() {
    let dir = TempDir::new("import-inactive");
    let src = shop_pbip(&dir);
    let mut s = target();
    run(&mut s, "table.import", json!({"name": "Product", "m": PRODUCT_M}));
    run(
        &mut s,
        "column.add",
        json!({"table": "Sales", "name": "Key2", "data_type": "int64",
        "expression": "Sales[ProductKey]"}),
    );
    run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Key2]", "to": "Product[ProductKey]"}),
    );
    // Product replaced: the source's Sales[ProductKey] → Product[ProductKey] comes too
    let out = import(
        &mut s,
        json!({"source": src, "tables": ["Product"], "on_conflict": "replace"}),
    );
    let m = s.model.as_ref().unwrap();
    assert_eq!(m.relationships.len(), 2, "{out}");
    assert!(!m.relationships[1].is_active, "{out}");
    assert!(out["warnings"].to_string().contains("inactive"), "{out}");
}

#[test]
fn a_template_report_is_applied_with_pages_and_theme() {
    let dir = TempDir::new("import-template");
    let src = shop_pbip(&dir);
    let (_, mut s) = target_pbip(&dir);
    run(
        &mut s,
        "theme.set",
        json!({"theme": {"name": "Old", "dataColors": ["#000000"]}}),
    );
    let pages_before = s.report.as_ref().unwrap().pages.len();
    let out = import(
        &mut s,
        json!({"source": src, "pages": ["*"], "theme": true, "on_conflict": "rename"}),
    );
    assert_eq!(out["applied"], true, "{out}");
    assert_eq!(action(&out, "theme", "Shop Colors"), "replaced");
    let r = s.report.as_ref().unwrap();
    assert_eq!(r.pages.len(), pages_before + 3, "{out}");
    // the shared page name was renamed, the others kept theirs
    assert!(r.page("Page 1 (imported)").is_some(), "{out}");
    assert!(r.page("Overview").is_some() && r.page("Detail").is_some());
    let theme = run(&mut s, "theme.get", json!({}));
    assert_eq!(theme["custom"]["theme_name"], "Shop Colors");
    let v = run(&mut s, "report.validate", json!({}));
    assert_eq!(v["valid"], true, "{v}");
}
