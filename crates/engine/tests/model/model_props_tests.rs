//! Tests of the M9.1 properties (`model_props.rs`): dynamic format strings, column properties,
//! annotations, `model.set`, date variations; round trips through PBIP (TMSL and TMDL) and PBIX.

use std::path::Path;

use bicraft_model::{Relationship, SemanticModel, Variation};
use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn fails(s: &mut Session, id: &str, params: Value) -> String {
    match s.execute(id, &params) {
        Ok(v) => panic!("{id} {params} should fail, got {v}"),
        Err(e) => e,
    }
}

fn model(s: &Session) -> &SemanticModel {
    s.model.as_ref().unwrap()
}

/// A model with literal data: Month (name, number, a quarter that does not sort the month),
/// Sales with a date column.
fn session() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Props"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "Month", "m": "let Source = #table(type table [Name = text, No = Int64.Type, Quarter = text], \
               {{\"Jan\", 1, \"Q1\"}, {\"Feb\", 2, \"Q1\"}, {\"Apr\", 4, \"Q2\"}, {\"jan\", 1, \"Q1\"}}) in Source"}),
    );
    run(
        &mut s,
        "table.import",
        json!({"name": "Sales", "m": "let Source = #table(type table [Month = text, Amount = number], \
               {{\"Jan\", 1.5}, {\"Feb\", 2}, {\"Apr\", 3}}) in Source"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])", "format_string": "0.0"}),
    );
    s
}

fn column<'m>(m: &'m SemanticModel, t: &str, c: &str) -> &'m bicraft_model::Column {
    m.table(t).unwrap().column(c).unwrap()
}

fn definition(m: &SemanticModel, measure: &str) -> Option<String> {
    m.measure(measure)
        .unwrap()
        .1
        .extra
        .get("formatStringDefinition")
        .and_then(|d| d.get("expression"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

#[test]
fn dynamic_format_strings_are_set_validated_and_removed() {
    let mut s = session();
    let out = run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Dyn", "expression": "[Total]",
               "format_string": "0", "format_string_definition": "IF([Total] > 2, \"0.00\", \"0\")"}),
    );
    assert_eq!(
        out["format_string_definition"], "IF([Total] > 2, \"0.00\", \"0\")",
        "{out}"
    );
    let m = model(&s);
    assert_eq!(
        definition(m, "Dyn").as_deref(),
        Some("IF([Total] > 2, \"0.00\", \"0\")")
    );
    // the engine refuses a static and a dynamic format string together: as TOM, the static one is
    // cleared and no copy is kept
    let x = m.measure("Dyn").unwrap().1;
    assert_eq!(x.format_string, None);
    assert!(
        x.annotations
            .iter()
            .all(|a| a.name != "custom_StaticFormatString"),
        "{:?}",
        x.annotations
    );
    assert!(out["warnings"].to_string().contains("cleared"), "{out}");

    // invalid DAX: the whole measure.add is refused, nothing is left behind
    let e = fails(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Bad", "expression": "1", "format_string_definition": "[Nope]"}),
    );
    assert!(
        e.contains("format string expression") && e.contains("nothing was changed"),
        "{e}"
    );
    assert!(model(&s).measure("Bad").is_none());
    // a table expression is not a format string
    let e = fails(
        &mut s,
        "measure.set",
        json!({"name": "Dyn", "format_string_definition": "ALL(Sales)"}),
    );
    assert!(e.contains("invalid DAX"), "{e}");
    // force applies it with a warning
    let out = run(
        &mut s,
        "measure.set",
        json!({"name": "Dyn", "format_string_definition": "[Nope]", "force": true}),
    );
    assert!(out["warnings"].to_string().contains("force"), "{out}");

    // set with a rename in the same call
    run(
        &mut s,
        "measure.set",
        json!({"name": "Dyn", "new_name": "Dynamic", "format_string_definition": "\"#,0\""}),
    );
    assert_eq!(definition(model(&s), "Dynamic").as_deref(), Some("\"#,0\""));
    // null removes it and leaves no format string
    run(
        &mut s,
        "measure.set",
        json!({"name": "Dynamic", "format_string_definition": null}),
    );
    let m = model(&s);
    assert_eq!(definition(m, "Dynamic"), None);
    assert!(
        !m.measure("Dynamic")
            .unwrap()
            .1
            .extra
            .contains_key("formatStringDefinition")
    );
    assert_eq!(m.measure("Dynamic").unwrap().1.format_string, None);
    assert!(m.measure("Dynamic").unwrap().1.annotations.is_empty());
    // a static format string alone while the dynamic one stays is refused, nothing changes
    run(
        &mut s,
        "measure.set",
        json!({"name": "Dynamic", "format_string_definition": "\"0\""}),
    );
    let e = fails(
        &mut s,
        "measure.set",
        json!({"name": "Dynamic", "format_string": "0.000"}),
    );
    assert!(e.contains("dynamic format string"), "{e}");
    let x = model(&s).measure("Dynamic").unwrap().1;
    assert_eq!(x.format_string, None);
    assert!(x.annotations.is_empty());
    assert_eq!(definition(model(&s), "Dynamic").as_deref(), Some("\"0\""));
    // removed with a static one in the same call: the static one applies
    run(
        &mut s,
        "measure.set",
        json!({"name": "Dynamic", "format_string_definition": null, "format_string": "0.000"}),
    );
    let x = model(&s).measure("Dynamic").unwrap().1;
    assert_eq!(x.format_string.as_deref(), Some("0.000"));
    assert_eq!(definition(model(&s), "Dynamic"), None);
    // undo brings it back, as one step per command
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(definition(model(&s), "Dynamic").as_deref(), Some("\"0\""));
    assert_eq!(model(&s).measure("Dynamic").unwrap().1.format_string, None);
    // the table is checked when given; unknown parameters are errors
    let e = fails(
        &mut s,
        "measure.set",
        json!({"name": "Dynamic", "table": "Month", "format_string_definition": "\"0\""}),
    );
    assert!(e.contains("not `Month`"), "{e}");
    let e = fails(
        &mut s,
        "measure.set",
        json!({"name": "Dynamic", "format_string_definitoin": "\"0\""}),
    );
    assert!(e.contains("unknown parameter"), "{e}");
}

#[test]
fn sort_by_column_follows_desktop_rules() {
    let mut s = session();
    // one month number per month name ("Jan" and "jan" are the same value for the engine)
    let out = run(
        &mut s,
        "column.set",
        json!({"table": "Month", "name": "Name", "sort_by_column": "no"}),
    );
    assert_eq!(out["sort_by_column"], "No", "{out}");
    assert_eq!(
        column(model(&s), "Month", "Name").sort_by_column.as_deref(),
        Some("No")
    );
    // a cycle, itself, another table, a missing column
    let e = fails(
        &mut s,
        "column.set",
        json!({"table": "Month", "name": "No", "sort_by_column": "Name"}),
    );
    assert!(e.contains("cycle"), "{e}");
    let e = fails(
        &mut s,
        "column.set",
        json!({"table": "Month", "name": "No", "sort_by_column": "No"}),
    );
    assert!(e.contains("itself"), "{e}");
    let e = fails(
        &mut s,
        "column.set",
        json!({"table": "Sales", "name": "Month", "sort_by_column": "No"}),
    );
    assert!(e.contains("own table"), "{e}");
    // the quarter has two numbers: refused on the data, with the offending value
    let e = fails(
        &mut s,
        "column.set",
        json!({"table": "Month", "name": "Quarter", "sort_by_column": "No"}),
    );
    assert!(e.contains("more than one") && e.contains("\"Q1\""), "{e}");
    assert_eq!(column(model(&s), "Month", "Quarter").sort_by_column, None);
    let out = run(
        &mut s,
        "column.set",
        json!({"table": "Month", "name": "Quarter", "sort_by_column": "No", "force": true}),
    );
    assert!(out["warnings"].to_string().contains("force"), "{out}");
    run(
        &mut s,
        "column.set",
        json!({"table": "Month", "name": "Quarter", "sort_by_column": null}),
    );
    assert_eq!(column(model(&s), "Month", "Quarter").sort_by_column, None);
    // a calculated column computed from the column cannot sort it
    run(
        &mut s,
        "column.add",
        json!({"table": "Month", "name": "Len", "data_type": "int64", "expression": "LEN(Month[Quarter])"}),
    );
    let e = fails(
        &mut s,
        "column.set",
        json!({"table": "Month", "name": "Quarter", "sort_by_column": "Len"}),
    );
    assert!(e.contains("circular"), "{e}");
    // a calculated column is checked on its computed values, as a data column is
    let e = fails(
        &mut s,
        "column.set",
        json!({"table": "Month", "name": "Len", "sort_by_column": "No"}),
    );
    assert!(e.contains("more than one"), "{e}");
    assert_eq!(column(model(&s), "Month", "Len").sort_by_column, None);
}

#[test]
fn mdx_key_and_category_rules() {
    let mut s = session();
    run(
        &mut s,
        "column.set",
        json!({"table": "Month", "name": "Name", "sort_by_column": "No"}),
    );
    run(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Month]", "to": "Month[Name]", "to_cardinality": "many"}),
    );
    // the engine refuses it on a sort-by column and a hierarchy level; a sorted column, a
    // relationship key and a variation's default column only warn
    let e = fails(
        &mut s,
        "column.set",
        json!({"table": "Month", "name": "No", "is_available_in_mdx": false}),
    );
    assert!(e.contains("sort-by column"), "{e}");
    for (t, c, why) in [("Month", "Name", "sorted by"), ("Sales", "Month", "relationship")] {
        let out = run(
            &mut s,
            "column.set",
            json!({"table": t, "name": c, "is_available_in_mdx": false}),
        );
        assert!(out["warnings"].to_string().contains(why), "{out}");
        run(&mut s, "edit.undo", json!({}));
    }
    run(
        &mut s,
        "hierarchy.add",
        json!({"table": "Sales", "name": "H", "levels": ["Amount"]}),
    );
    let e = fails(
        &mut s,
        "column.set",
        json!({"table": "Sales", "name": "Amount", "is_available_in_mdx": false}),
    );
    assert!(e.contains("hierarchy"), "{e}");
    let out = run(
        &mut s,
        "column.set",
        json!({"table": "Month", "name": "Quarter", "is_available_in_mdx": false}),
    );
    assert_eq!(out["is_available_in_mdx"], false);
    assert_eq!(
        column(model(&s), "Month", "Quarter").extra["isAvailableInMdx"],
        false
    );
    // and a column without hierarchy cannot be a sort-by column
    let e = fails(
        &mut s,
        "column.set",
        json!({"table": "Month", "name": "Name", "sort_by_column": "Quarter"}),
    );
    assert!(e.contains("attribute hierarchy"), "{e}");
    run(
        &mut s,
        "column.set",
        json!({"table": "Month", "name": "Quarter", "is_available_in_mdx": true}),
    );
    assert!(
        !column(model(&s), "Month", "Quarter")
            .extra
            .contains_key("isAvailableInMdx")
    );

    // keys: unique non-blank values, one per table
    let e = fails(
        &mut s,
        "column.set",
        json!({"table": "Month", "name": "Quarter", "is_key": true}),
    );
    assert!(e.contains("unique") && e.contains("duplicate"), "{e}");
    run(
        &mut s,
        "column.set",
        json!({"table": "Sales", "name": "Amount", "is_key": true}),
    );
    assert_eq!(column(model(&s), "Sales", "Amount").extra["isKey"], true);
    let e = fails(
        &mut s,
        "column.set",
        json!({"table": "Sales", "name": "Month", "is_key": true}),
    );
    assert!(e.contains("one key column"), "{e}");
    run(
        &mut s,
        "column.set",
        json!({"table": "Sales", "name": "Amount", "is_key": false}),
    );
    assert!(!column(model(&s), "Sales", "Amount").extra.contains_key("isKey"));

    // data categories: Desktop's names, case and spaces ignored
    let out = run(
        &mut s,
        "column.set",
        json!({"table": "Sales", "name": "Month", "data_category": "web url"}),
    );
    assert_eq!(out["data_category"], "WebUrl");
    let out = run(
        &mut s,
        "column.set",
        json!({"table": "Sales", "name": "Month", "data_category": "Mood"}),
    );
    assert!(out["warnings"].to_string().contains("not one"), "{out}");
    run(
        &mut s,
        "column.set",
        json!({"table": "Sales", "name": "Month", "data_category": "Uncategorized"}),
    );
    assert_eq!(column(model(&s), "Sales", "Month").data_category, None);

    // column.add takes them too, and a failure removes the added column again
    run(
        &mut s,
        "column.add",
        json!({"table": "Month", "name": "City", "data_type": "string", "data_category": "City", "is_available_in_mdx": false}),
    );
    let c = column(model(&s), "Month", "City");
    assert_eq!(c.data_category.as_deref(), Some("City"));
    assert_eq!(c.extra["isAvailableInMdx"], false);
    let e = fails(
        &mut s,
        "column.add",
        json!({"table": "Month", "name": "X", "data_type": "string", "sort_by_column": "Nope"}),
    );
    assert!(e.contains("Nope"), "{e}");
    assert!(model(&s).table("Month").unwrap().column("X").is_none());
}

#[test]
fn annotations_and_model_properties() {
    let mut s = session();
    let out = run(
        &mut s,
        "model.set_annotation",
        json!({"name": "Owner", "value": "team"}),
    );
    assert_eq!(out["previous"], Value::Null);
    let out = run(
        &mut s,
        "model.set_annotation",
        json!({"name": "Owner", "value": "other"}),
    );
    assert_eq!(out["previous"], "team");
    run(
        &mut s,
        "table.set_annotation",
        json!({"table": "sales", "name": "A", "value": "1"}),
    );
    run(
        &mut s,
        "column.set_annotation",
        json!({"table": "Sales", "column": "amount", "name": "B", "value": "2"}),
    );
    run(
        &mut s,
        "measure.set_annotation",
        json!({"measure": "total", "name": "C", "value": "3"}),
    );
    let m = model(&s);
    let has = |list: &[bicraft_model::Annotation], n: &str, v: &str| {
        list.iter().any(|a| a.name == n && a.value == v)
    };
    assert!(has(&m.annotations, "Owner", "other"));
    assert!(has(&m.table("Sales").unwrap().annotations, "A", "1"));
    assert!(has(&column(m, "Sales", "Amount").annotations, "B", "2"));
    assert!(has(&m.measure("Total").unwrap().1.annotations, "C", "3"));
    run(
        &mut s,
        "measure.set_annotation",
        json!({"measure": "Total", "name": "C", "value": null}),
    );
    assert!(model(&s).measure("Total").unwrap().1.annotations.is_empty());
    let out = run(
        &mut s,
        "table.set_annotation",
        json!({"table": "Sales", "name": "Zzz", "value": null}),
    );
    assert!(out["warnings"].to_string().contains("no annotation"), "{out}");
    let e = fails(&mut s, "model.set_annotation", json!({"name": "X"}));
    assert!(e.contains("value"), "{e}");
    let e = fails(&mut s, "model.set_annotation", json!({"name": "X", "value": 3}));
    assert!(e.contains("string or null"), "{e}");

    run(
        &mut s,
        "model.set",
        json!({"culture": "fr-FR", "discourage_implicit_measures": true, "source_query_culture": "de-DE"}),
    );
    let m = model(&s);
    assert_eq!(m.culture.as_deref(), Some("fr-FR"));
    assert_eq!(m.extra["discourageImplicitMeasures"], true);
    assert_eq!(m.extra["sourceQueryCulture"], "de-DE");
    run(
        &mut s,
        "model.set",
        json!({"discourage_implicit_measures": false, "source_query_culture": null}),
    );
    let m = model(&s);
    assert!(!m.extra.contains_key("discourageImplicitMeasures"));
    assert!(!m.extra.contains_key("sourceQueryCulture"));
    let e = fails(&mut s, "model.set", json!({"culture": "not a culture"}));
    assert!(e.contains("culture name"), "{e}");
    let e = fails(&mut s, "model.set", json!({}));
    assert!(e.contains("at least one"), "{e}");
}

/// Adds an auto date table for `Sales[Date]` the way Desktop lays it out.
fn with_auto_date(s: &mut Session) {
    run(
        s,
        "column.add",
        json!({"table": "Sales", "name": "Date", "data_type": "dateTime", "expression": "DATE(2024, 1, 1)"}),
    );
    run(
        s,
        "table.import",
        json!({"name": "LocalDateTable_1", "hidden": true,
               "dax": "ADDCOLUMNS(CALENDAR(DATE(2024, 1, 1), DATE(2024, 1, 31)), \"Year\", YEAR([Date]))"}),
    );
    let m = s.model.as_mut().unwrap();
    m.relationships.push(Relationship {
        name: "auto-rel".into(),
        from_table: "Sales".into(),
        from_column: "Date".into(),
        to_table: "LocalDateTable_1".into(),
        to_column: "Date".into(),
        is_active: true,
        ..Default::default()
    });
    let t = m.table_mut("Sales").unwrap();
    t.column_mut("Date").unwrap().set_variations(&[Variation {
        name: "Variation".into(),
        relationship: Some("auto-rel".into()),
        default_column: Some(("LocalDateTable_1".into(), "Date".into())),
        is_default: true,
        ..Default::default()
    }]);
}

#[test]
fn removing_variations_takes_the_auto_date_table_along() {
    let mut s = session();
    with_auto_date(&mut s);
    // a variation's default column without hierarchy: a warning
    let out = run(
        &mut s,
        "column.set",
        json!({"table": "LocalDateTable_1", "name": "Date", "is_available_in_mdx": false}),
    );
    assert!(out["warnings"].to_string().contains("variation"), "{out}");
    run(&mut s, "edit.undo", json!({}));
    let e = fails(
        &mut s,
        "column.remove_variation",
        json!({"table": "Sales", "name": "Date", "variation": "Other"}),
    );
    assert!(e.contains("it has: Variation"), "{e}");
    let out = run(
        &mut s,
        "column.remove_variation",
        json!({"table": "Sales", "name": "Date"}),
    );
    assert_eq!(out["removed_variations"], json!(["Variation"]), "{out}");
    assert_eq!(out["removed_tables"], json!(["LocalDateTable_1"]));
    let m = model(&s);
    assert!(m.table("LocalDateTable_1").is_none());
    assert!(m.relationships.iter().all(|r| r.name != "auto-rel"));
    assert!(column(m, "Sales", "Date").variations().is_empty());
    let e = fails(
        &mut s,
        "column.remove_variation",
        json!({"table": "Sales", "name": "Date"}),
    );
    assert!(e.contains("no variations"), "{e}");
    run(&mut s, "edit.undo", json!({}));
    assert!(model(&s).table("LocalDateTable_1").is_some());

    // DAX that uses the auto date table blocks the removal unless force
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Years", "expression": "COUNTROWS(LocalDateTable_1)"}),
    );
    let e = fails(
        &mut s,
        "column.remove_variation",
        json!({"table": "Sales", "name": "Date"}),
    );
    assert!(e.contains("Years"), "{e}");
    run(&mut s, "measure.remove", json!({"name": "Years"}));

    // table.remove: the variations pointing to it go first
    let out = run(&mut s, "table.remove", json!({"name": "LocalDateTable_1"}));
    assert_eq!(
        out["removed_variations"],
        json!([{"table": "Sales", "column": "Date", "variation": "Variation"}]),
        "{out}"
    );
    assert!(column(model(&s), "Sales", "Date").variations().is_empty());
    run(&mut s, "edit.undo", json!({}));

    // column.remove: its auto date table goes with it
    let out = run(&mut s, "column.remove", json!({"table": "Sales", "name": "Date"}));
    assert_eq!(out["removed_tables"], json!(["LocalDateTable_1"]), "{out}");
    assert!(model(&s).table("LocalDateTable_1").is_none());
}

/// Every M9.1 property on the session's model.
fn decorate(s: &mut Session) {
    decorate_metadata(s);
    decorate_storage(s);
}

/// The properties that change the stored data's layout (attribute hierarchies, key flags).
fn decorate_storage(s: &mut Session) {
    run(
        s,
        "column.set",
        json!({"table": "Month", "name": "Quarter", "is_available_in_mdx": false}),
    );
    run(
        s,
        "column.set",
        json!({"table": "Sales", "name": "Amount", "is_key": true}),
    );
}

/// The properties a PBIX save writes into the metadata of existing objects.
fn decorate_metadata(s: &mut Session) {
    run(
        s,
        "measure.set",
        json!({"name": "Total", "format_string_definition": "IF([Total] > 2, \"0.00\", \"0\")"}),
    );
    run(
        s,
        "column.set",
        json!({"table": "Month", "name": "Name", "sort_by_column": "No", "data_category": "Place"}),
    );
    run(
        s,
        "model.set_annotation",
        json!({"name": "Owner", "value": "team"}),
    );
    run(
        s,
        "table.set_annotation",
        json!({"table": "Sales", "name": "A", "value": "1"}),
    );
    run(
        s,
        "column.set_annotation",
        json!({"table": "Sales", "column": "Amount", "name": "B", "value": "2"}),
    );
    run(
        s,
        "measure.set_annotation",
        json!({"measure": "Total", "name": "C", "value": "3"}),
    );
    run(
        s,
        "model.set",
        json!({"discourage_implicit_measures": true, "source_query_culture": "fr-FR"}),
    );
}

fn assert_decorated(m: &SemanticModel) {
    assert_metadata(m);
    assert_eq!(column(m, "Month", "Quarter").extra["isAvailableInMdx"], false);
    assert_eq!(column(m, "Sales", "Amount").extra["isKey"], true);
}

fn assert_metadata(m: &SemanticModel) {
    assert_eq!(
        definition(m, "Total").as_deref(),
        Some("IF([Total] > 2, \"0.00\", \"0\")")
    );
    let name = column(m, "Month", "Name");
    assert_eq!(name.sort_by_column.as_deref(), Some("No"));
    assert_eq!(name.data_category.as_deref(), Some("Place"));
    assert!(
        m.annotations
            .iter()
            .any(|a| a.name == "Owner" && a.value == "team")
    );
    assert!(
        m.table("Sales")
            .unwrap()
            .annotations
            .iter()
            .any(|a| a.name == "A")
    );
    assert!(
        column(m, "Sales", "Amount")
            .annotations
            .iter()
            .any(|a| a.name == "B")
    );
    assert!(
        m.measure("Total")
            .unwrap()
            .1
            .annotations
            .iter()
            .any(|a| a.name == "C")
    );
    assert_eq!(m.extra["discourageImplicitMeasures"], true);
    assert_eq!(m.extra["sourceQueryCulture"], "fr-FR");
}

fn reopen(path: &Path) -> Session {
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    assert_eq!(s.model_error, None);
    s
}

#[test]
fn pbip_tmsl_and_tmdl_round_trips() {
    let dir = TempDir::new("m91-pbip");
    for format in ["pbip-tmdl", "pbip-tmsl"] {
        let mut s = session();
        decorate(&mut s);
        with_auto_date(&mut s);
        let path = dir.join(format!("{format}.pbip"));
        run(&mut s, "file.save_as", json!({"path": path, "format": format}));
        let mut back = reopen(&path);
        assert_decorated(model(&back));
        assert_eq!(model(&back), model(&s), "{format}");
        // removing the variation and the dynamic format string, saved again
        run(
            &mut back,
            "column.remove_variation",
            json!({"table": "Sales", "name": "Date"}),
        );
        run(
            &mut back,
            "measure.set",
            json!({"name": "Total", "format_string_definition": null}),
        );
        run(&mut back, "file.save", json!({}));
        let again = reopen(&path);
        let m = model(&again);
        assert!(m.table("LocalDateTable_1").is_none(), "{format}");
        assert!(column(m, "Sales", "Date").variations().is_empty());
        assert_eq!(definition(m, "Total"), None);
        assert_eq!(m.measure("Total").unwrap().1.format_string, None);
    }
}

#[test]
fn pbix_save_round_trip() {
    let dir = TempDir::new("m91-pbix");
    let path = dir.join("props.pbix");
    let mut s = session();
    run(&mut s, "file.save_as", json!({"path": path}));
    // edits of the existing objects: the metadata-only save
    let mut s = reopen(&path);
    decorate_metadata(&mut s);
    run(&mut s, "file.save", json!({"backup": false}));
    let back = reopen(&path);
    assert_metadata(model(&back));
    // the attribute hierarchy of a stored column lives in the VertiPaq data: refused; the key flag is
    // metadata
    let mut storage = reopen(&path);
    decorate_storage(&mut storage);
    let e = fails(&mut storage, "file.save", json!({"backup": false}));
    assert!(e.contains("isAvailableInMdx"), "{e}");
    let mut key = reopen(&path);
    run(
        &mut key,
        "column.set",
        json!({"table": "Sales", "name": "Amount", "is_key": true}),
    );
    run(&mut key, "file.save", json!({"backup": false}));
    let keyed = reopen(&path);
    assert_eq!(column(model(&keyed), "Sales", "Amount").extra["isKey"], true);
    // the row-number column is no longer the key: the engine refuses two key columns
    let sales = model(&keyed).table("Sales").unwrap();
    let rn = sales.columns.iter().find(|c| c.is_row_number()).unwrap();
    assert_ne!(rn.extra.get("isKey"), Some(&json!(true)));
    // the sort-by check reads the stored data
    let mut back = back;
    let e = fails(
        &mut back,
        "column.set",
        json!({"table": "Month", "name": "Quarter", "sort_by_column": "No"}),
    );
    assert!(e.contains("more than one"), "{e}");
    // removal of the dynamic format string and the annotations, then a culture change (refused)
    run(
        &mut back,
        "measure.set",
        json!({"name": "Total", "format_string_definition": null}),
    );
    run(
        &mut back,
        "model.set_annotation",
        json!({"name": "Owner", "value": null}),
    );
    run(&mut back, "file.save", json!({"backup": false}));
    let again = reopen(&path);
    assert_eq!(definition(model(&again), "Total"), None);
    assert!(model(&again).annotations.iter().all(|a| a.name != "Owner"));
    let mut again = again;
    let out = run(&mut again, "model.set", json!({"culture": "fr-FR"}));
    assert!(out["warnings"].to_string().contains("culture"), "{out}");
    let e = fails(&mut again, "file.save", json!({"backup": false}));
    assert!(e.contains("culture"), "{e}");
}

#[test]
fn pbix_new_columns_and_auto_date_tables() {
    let dir = TempDir::new("m91-pbix-new");
    let path = dir.join("dates.pbix");
    let mut s = session();
    with_auto_date(&mut s);
    run(&mut s, "file.save_as", json!({"path": path}));
    let mut s = reopen(&path);
    assert_eq!(column(model(&s), "Sales", "Date").variations().len(), 1);
    // a new column with its properties, and the auto date table removed: the structural save
    run(
        &mut s,
        "column.add",
        json!({"table": "Month", "name": "Code", "data_type": "string", "expression": "UPPER(Month[Name])",
               "is_available_in_mdx": false, "data_category": "Place"}),
    );
    run(
        &mut s,
        "column.remove_variation",
        json!({"table": "Sales", "name": "Date"}),
    );
    run(&mut s, "file.save", json!({"backup": false}));
    let back = reopen(&path);
    let m = model(&back);
    assert!(m.table("LocalDateTable_1").is_none());
    assert!(column(m, "Sales", "Date").variations().is_empty());
    let code = column(m, "Month", "Code");
    assert_eq!(code.extra["isAvailableInMdx"], false);
    assert_eq!(code.data_category.as_deref(), Some("Place"));
}

/// An annotation that parks a static format string (earlier versions wrote one) is left
/// untouched; no new annotation is written.
#[test]
fn legacy_static_format_annotation_is_left_alone() {
    let mut s = session();
    run(
        &mut s,
        "measure.set",
        json!({"name": "Total", "format_string_definition": "\"0\""}),
    );
    run(
        &mut s,
        "measure.set_annotation",
        json!({"measure": "Total", "name": "custom_StaticFormatString", "value": "0.0"}),
    );
    for definition_text in [json!("\"#,0\""), Value::Null] {
        run(
            &mut s,
            "measure.set",
            json!({"name": "Total", "format_string_definition": definition_text}),
        );
        let x = model(&s).measure("Total").unwrap().1;
        assert_eq!(x.format_string, None);
        assert_eq!(x.annotations.len(), 1, "{:?}", x.annotations);
        assert_eq!(x.annotations[0].value, "0.0");
    }
}

/// `relationship.set {new_name}`: renamed in place, the variation that names it follows; saved
/// to PBIP (TMSL, TMDL) and PBIX (the row is renamed, the storage kept).
#[test]
fn relationship_rename_follows_into_variations_and_saves() {
    let rel_of = |m: &SemanticModel| {
        column(m, "Sales", "Date").variations()[0]
            .relationship
            .clone()
            .unwrap()
    };
    let mut s = session();
    with_auto_date(&mut s);
    let count = model(&s).relationships.len();
    let out = run(
        &mut s,
        "relationship.set",
        json!({"name": "auto-rel", "new_name": "Sales Date"}),
    );
    assert_eq!(out["name"], "Sales Date", "{out}");
    assert_eq!(out["renamed"], "auto-rel", "{out}");
    let m = model(&s);
    assert_eq!(m.relationships.len(), count);
    assert!(m.relationships.iter().any(|r| r.name == "Sales Date"));
    assert!(m.relationships.iter().all(|r| r.name != "auto-rel"));
    assert_eq!(rel_of(m), "Sales Date");
    // the auto date variation still resolves
    let v = run(
        &mut s,
        "dax.validate",
        json!({"expression": "MAX(Sales[Date].[Year])", "table": "Sales"}),
    );
    assert_eq!(v["valid"], true, "{v}");
    for format in ["pbip-tmsl", "pbip-tmdl"] {
        let dir = TempDir::new(&format!("m92-rel-rename-{format}"));
        let path = dir.join("Rel.pbip");
        let mut p = Session::new();
        p.model = s.model.clone();
        let fmt = format.trim_start_matches("pbip-");
        run(
            &mut p,
            "project.new",
            json!({"dir": dir.path(), "name": "Rel", "format": fmt}),
        );
        let back = reopen(&path);
        assert_eq!(rel_of(model(&back)), "Sales Date", "{format}");
        assert_eq!(back.model, s.model, "{format}");
    }
    // PBIX: renamed in a saved file
    let dir = TempDir::new("m92-rel-rename-pbix");
    let path = dir.join("rel.pbix");
    let mut p = session();
    with_auto_date(&mut p);
    run(&mut p, "file.save_as", json!({"path": path}));
    let mut p = reopen(&path);
    run(
        &mut p,
        "relationship.set",
        json!({"name": "auto-rel", "new_name": "Sales Date", "active": true}),
    );
    run(&mut p, "file.save", json!({"backup": false}));
    let back = reopen(&path);
    let m = model(&back);
    assert!(m.relationships.iter().any(|r| r.name == "Sales Date"));
    assert!(m.relationships.iter().all(|r| r.name != "auto-rel"));
    assert_eq!(rel_of(m), "Sales Date");
}

/// The snapshot's details for the Model view's Properties pane (G61): shared expressions (query or
/// parameter, text, description, query group), cultures, data sources without their connection
/// strings.
#[test]
fn the_snapshot_details_shared_expressions_cultures_and_data_sources() {
    let mut s = session();
    run(
        &mut s,
        "expression.add",
        json!({"name": "Base", "expression": "\"C:\\\\data\" meta [IsParameterQuery = true, Type = \"Text\", IsParameterQueryRequired = true]"}),
    );
    run(
        &mut s,
        "expression.add",
        json!({"name": "Lookup", "expression": "#table({\"A\"}, {{1}})", "description": "Codes", "query_group": "Helpers"}),
    );
    run(&mut s, "culture.add", json!({"name": "fr-FR"}));
    s.model.as_mut().unwrap().extra.insert(
        "dataSources".into(),
        json!([{"name": "Warehouse", "type": "structured", "connectionDetails": {"protocol": "tds"},
                "credential": {"password": "secret-value"}, "connectionString": "Password=secret-value"}]),
    );
    let m = s.snapshot()["model"].clone();
    let e = &m["expression_details"];
    assert_eq!(e[0]["name"], "Base");
    assert_eq!(e[0]["kind"], "parameter");
    assert_eq!(e[1]["kind"], "query");
    assert_eq!(e[1]["description"], "Codes");
    assert_eq!(e[1]["query_group"], "Helpers");
    assert_eq!(e[1]["expression"], "#table({\"A\"}, {{1}})");
    assert_eq!(m["culture_details"][0]["name"], "fr-FR");
    assert_eq!(m["culture_details"][0]["model"], false);
    let d = &m["data_sources"][0];
    assert_eq!(d["name"], "Warehouse");
    assert_eq!(d["type"], "structured");
    assert_eq!(d["protocol"], "tds");
    assert!(
        !m["data_sources"].to_string().contains("secret-value"),
        "no secret in the snapshot"
    );
}
