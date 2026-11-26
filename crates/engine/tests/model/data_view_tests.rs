//! The Data view's engine reads (ROADMAP M11.17): `query.preview` (a Power Query query's first
//! rows through BIcraft's M evaluator, nothing loaded; files only under `data_root`; external
//! connectors only when named, never for a reader, never without the server's network) and
//! `visual.records` (See records of a data point, through the model's relationships). The paging
//! of `table.rows` is tested inside the crate (`preview_rows_tests.rs`). Synthetic data only.

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::{ServerConfig, Session};

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

const SALES: &str = r#"#table(type table [Category = text, Amount = number], {{"Wheels", 100}, {"Toys", 5}, {"Games", 20}, {"Toys", 7}})"#;

fn shop() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(
        &mut s,
        "table.add",
        json!({"name": "Sales", "partition": {"source_type": "m", "expression": SALES}}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "Sales", "name": "Category", "data_type": "string"}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
    );
    run(
        &mut s,
        "expression.add",
        json!({"name": "Big", "expression": "Table.SelectRows(Sales, each [Amount] > 6)"}),
    );
    run(
        &mut s,
        "expression.add",
        json!({"name": "Threshold", "expression": "6"}),
    );
    s
}

#[test]
fn query_preview_shows_a_querys_first_rows_and_loads_nothing() {
    let mut s = shop();
    let rev = s.revision;
    let dirty = s.dirty;
    // a table's M partition
    let v = run(&mut s, "query.preview", json!({"name": "sales"}));
    assert_eq!(v["source"], "table");
    assert_eq!(v["name"], "Sales");
    assert_eq!(
        v["columns"][0],
        json!({"name": "Category", "data_type": "string"})
    );
    assert_eq!(
        v["rows"],
        json!([["Wheels", 100], ["Toys", 5], ["Games", 20], ["Toys", 7]])
    );
    assert_eq!(v["truncated"], false);
    // a query referencing the table's query, with a limit
    let v = run(&mut s, "query.preview", json!({"name": "Big", "limit": 2}));
    assert_eq!(v["source"], "query");
    assert_eq!(v["rows"], json!([["Wheels", 100], ["Games", 20]]));
    assert_eq!(v["row_count"], 2);
    assert_eq!(v["truncated"], true);
    // M text
    let v = run(
        &mut s,
        "query.preview",
        json!({"expression": "Table.FromRecords({[A = 1], [A = Threshold]})"}),
    );
    assert_eq!(v["source"], "expression");
    assert_eq!(v["rows"], json!([[1], [6]]));
    // nothing was loaded or changed
    assert_eq!(s.revision, rev);
    assert_eq!(s.dirty, dirty);
    // a parameter is not a table; unknown names, both or neither are refused
    let e = s
        .execute("query.preview", &json!({"name": "Threshold"}))
        .unwrap_err();
    assert!(e.contains("not a table"), "{e}");
    for bad in [
        json!({}),
        json!({"name": "Nope"}),
        json!({"name": "Sales", "expression": "1"}),
        json!({"expression": "{1"}),
        json!({"name": "Sales", "limit": 0}),
        json!({"name": "Sales", "partition": "Nope"}),
    ] {
        assert!(s.execute("query.preview", &bad).is_err(), "{bad}");
    }
    // a shared run gives the same rows
    let shared = s
        .execute_shared("query.preview", &json!({"name": "Big", "limit": 2}), None)
        .unwrap();
    assert_eq!(shared["rows"], json!([["Wheels", 100], ["Games", 20]]));
}

#[test]
fn query_preview_reads_files_only_under_the_data_root() {
    let dir = TempDir::new("engine-query-preview");
    std::fs::write(dir.join("people.csv"), "Name,Score\nann,1\nbob,2\n").unwrap();
    let mut s = shop();
    let m = r#"let
    Source = Csv.Document(File.Contents("people.csv"), [Delimiter = ",", Encoding = 65001]),
    Promoted = Table.PromoteHeaders(Source, [PromoteAllScalars = true]),
    Typed = Table.TransformColumnTypes(Promoted, {{"Name", type text}, {"Score", Int64.Type}})
in
    Typed"#;
    let e = s.execute("query.preview", &json!({"expression": m})).unwrap_err();
    assert!(e.contains("data_root"), "{e}");
    let v = run(
        &mut s,
        "query.preview",
        json!({"expression": m, "data_root": dir.path()}),
    );
    assert_eq!(v["rows"], json!([["ann", 1], ["bob", 2]]));
    assert_eq!(v["columns"][1]["data_type"], "int64");
}

#[test]
fn query_preview_runs_connectors_only_when_allowed() {
    let dbx = r#"let
    Source = Databricks.Catalogs("adb-1.azuredatabricks.net", "/sql/1.0/warehouses/1", [Catalog = null, Database = null]),
    T = Source{[Name = "main", Kind = "Database"]}[Data]
in
    T"#;
    let mut s = shop();
    // not named: the connector is not run
    let e = s
        .execute("query.preview", &json!({"expression": dbx}))
        .unwrap_err();
    assert!(e.contains("allow_connectors"), "{e}");
    // unknown connectors are refused
    let e = s
        .execute(
            "query.preview",
            &json!({"expression": dbx, "allow_connectors": ["ftp"]}),
        )
        .unwrap_err();
    assert!(e.contains("unknown connector"), "{e}");
    // a reader may not name connectors, and naming them runs exclusively
    assert!(s.reader_check("query.preview", &json!({"name": "Sales"})).is_ok());
    assert!(
        s.reader_check(
            "query.preview",
            &json!({"name": "Sales", "allow_connectors": ["databricks"]})
        )
        .is_err()
    );
    assert!(s.runs_shared("query.preview", &json!({"name": "Sales"})));
    assert!(!s.runs_shared("query.preview", &json!({"name": "Sales", "allow_connectors": []})));
    assert!(
        s.execute_shared(
            "query.preview",
            &json!({"expression": dbx, "allow_connectors": ["databricks"]}),
            None
        )
        .is_err()
    );
    // a server without network access refuses the connector before any connection
    let dir = TempDir::new("engine-query-preview-server");
    let mut srv = Session::server(ServerConfig::new(dir.path()).unwrap());
    run(&mut srv, "model.new", json!({"name": "M"}));
    let e = srv
        .execute(
            "query.preview",
            &json!({"expression": dbx, "allow_connectors": ["databricks"]}),
        )
        .unwrap_err();
    assert!(e.contains("network"), "{e}");
}

/// Product(Key, Category) 1—* Sales(ProductKey, Amount); a column chart of [Total] by
/// Product[Category].
fn star() -> (Session, String, String) {
    let mut s = Session::new();
    let product =
        r#"#table(type table [Key = Int64.Type, Category = text], {{1, "Toys"}, {2, "Games"}, {3, "Toys"}})"#;
    let sales = r#"#table(type table [ProductKey = Int64.Type, Amount = number], {{1, 10}, {2, 20}, {3, 30}, {1, 40}, {2, 50}})"#;
    for (id, p) in [
        ("model.new", json!({"name": "Star"})),
        (
            "table.add",
            json!({"name": "Product", "partition": {"source_type": "m", "expression": product}}),
        ),
        (
            "column.add",
            json!({"table": "Product", "name": "Key", "data_type": "int64"}),
        ),
        (
            "column.add",
            json!({"table": "Product", "name": "Category", "data_type": "string"}),
        ),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "ProductKey", "data_type": "int64"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "relationship.add",
            json!({"from": "Sales[ProductKey]", "to": "Product[Key]"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        ("report.new", json!({})),
    ] {
        run(&mut s, id, p);
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let v = run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "clusteredColumnChart", "x": 0, "y": 0, "width": 400, "height": 300,
               "fields": [{"role": "Category", "field": "Product[Category]"}, {"role": "Y", "field": "[Total]"}]}),
    );
    let visual = v["visual"].as_str().unwrap().to_string();
    (s, page, visual)
}

#[test]
fn see_records_shows_the_rows_behind_a_data_point() {
    let (mut s, page, visual) = star();
    let v = run(
        &mut s,
        "visual.records",
        json!({"page": page, "visual": visual, "point": [{"c": 0, "v": "Toys"}]}),
    );
    assert_eq!(v["table"], "Sales", "{v}");
    let names: Vec<&str> = v["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["ProductKey", "Amount"]);
    let mut rows = v["rows"].as_array().unwrap().clone();
    rows.sort_by_key(|r| r[1].as_f64().unwrap() as i64);
    let rows: Vec<(i64, f64)> = rows
        .iter()
        .map(|r| (r[0].as_i64().unwrap(), r[1].as_f64().unwrap()))
        .collect();
    assert_eq!(rows, vec![(1, 10.0), (3, 30.0), (1, 40.0)], "{v}");
    assert_eq!(v["filters_applied"], "point");
    // no point: every row; a limit
    let v = run(
        &mut s,
        "visual.records",
        json!({"page": page, "visual": visual, "limit": 2}),
    );
    assert_eq!(v["rows"].as_array().unwrap().len(), 2);
    assert_eq!(v["truncated"], true);
    // a point on a column the visual does not have, a measure column, or a bad value
    for bad in [
        json!({"page": page, "visual": visual, "point": [{"c": 9, "v": "Toys"}]}),
        json!({"page": page, "visual": visual, "point": [{"c": 1, "v": 1}]}),
        json!({"page": page, "visual": visual, "point": [{"c": 0, "v": [1]}]}),
        json!({"page": page, "visual": "nope"}),
        json!({"page": page, "visual": visual, "limit": 0}),
    ] {
        assert!(s.execute("visual.records", &bad).is_err(), "{bad}");
    }
}

/// The amounts of a See records answer, sorted.
fn amounts(v: &Value) -> Vec<f64> {
    let mut out: Vec<f64> = v["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r[1].as_f64().unwrap())
        .collect();
    out.sort_by(f64::total_cmp);
    out
}

/// See records of a mark that stands for several points (a histogram bin, a box, a Venn set of
/// BIcraft's visuals: the hit's `points`) lists the rows of every point, not only the first's
/// (docs/gaps.md G29).
#[test]
fn see_records_of_several_points_lists_every_points_rows() {
    let (mut s, page, visual) = star();
    let toys = json!([{"c": 0, "v": "Toys"}]);
    let games = json!([{"c": 0, "v": "Games"}]);
    let v = run(
        &mut s,
        "visual.records",
        json!({"page": page, "visual": visual, "point": toys, "points": [toys, games]}),
    );
    assert_eq!(amounts(&v), vec![10.0, 20.0, 30.0, 40.0, 50.0], "{v}");
    assert_eq!(v["points"], 2);
    assert_eq!(v["point"][0]["value"], "Toys");
    assert!(v["query"].as_str().unwrap().starts_with("EVALUATE UNION("), "{v}");
    // the points alone, a repeated point counted once, one point as before
    let v = run(
        &mut s,
        "visual.records",
        json!({"page": page, "visual": visual, "points": [games, games]}),
    );
    assert_eq!(amounts(&v), vec![20.0, 50.0]);
    assert_eq!(v["points"], 1);
    // the filters apply to every point's rows
    run(
        &mut s,
        "visual.filter",
        json!({"page": page, "visual": visual, "field": "Sales[Amount]", "op": ">", "value": 25}),
    );
    let v = run(
        &mut s,
        "visual.records",
        json!({"page": page, "visual": visual, "points": [toys, games]}),
    );
    assert_eq!(amounts(&v), vec![30.0, 40.0, 50.0], "{v}");
    for bad in [
        json!({"page": page, "visual": visual, "points": "Toys"}),
        json!({"page": page, "visual": visual, "points": [[{"c": 9, "v": "Toys"}]]}),
        json!({"page": page, "visual": visual, "points": vec![toys.clone(); 1001]}),
    ] {
        assert!(s.execute("visual.records", &bad).is_err(), "{bad}");
    }
}

/// See records applies the report, page and visual filters and the reader's slicer selections
/// (docs/gaps.md G49), and names what it cannot apply to rows.
#[test]
fn see_records_applies_the_filters_and_the_slicers() {
    let (mut s, page, visual) = star();
    let all = json!({"page": page, "visual": visual});
    assert_eq!(amounts(&run(&mut s, "visual.records", all.clone())).len(), 5);
    // a page filter through the relationship: Toys are products 1 and 3
    run(
        &mut s,
        "page.filter",
        json!({"page": page, "field": "Product[Category]", "in": ["Toys"]}),
    );
    let v = run(&mut s, "visual.records", all.clone());
    assert_eq!(amounts(&v), [10.0, 30.0, 40.0], "{v}");
    assert_eq!(v["filters_applied"], "point and filters");
    assert!(
        v["query"]
            .as_str()
            .unwrap()
            .contains("FILTER(ALL('Product'[Category])"),
        "{v}"
    );
    // a report filter and a visual filter on the fact table narrow it further
    run(
        &mut s,
        "report.filter",
        json!({"field": "Sales[Amount]", "op": ">", "value": 15}),
    );
    run(
        &mut s,
        "visual.filter",
        json!({"page": page, "visual": visual, "field": "Sales[ProductKey]", "not_in": [1]}),
    );
    let v = run(&mut s, "visual.records", all.clone());
    assert_eq!(amounts(&v), [30.0], "{v}");
    // with the point, the same
    let v = run(
        &mut s,
        "visual.records",
        json!({"page": page, "visual": visual, "point": [{"c": 0, "v": "Toys"}]}),
    );
    assert_eq!(amounts(&v), [30.0], "{v}");
    // a filter on a measure keeps the points it keeps, the measure computed under the other
    // filters: Toys' total is 30 there (80 without them)
    run(
        &mut s,
        "visual.filter",
        json!({"page": page, "visual": visual, "field": "[Total]", "op": ">", "value": 50}),
    );
    let v = run(&mut s, "visual.records", all.clone());
    assert_eq!(amounts(&v), Vec::<f64>::new(), "{v}");
    assert_eq!(v["not_applied"], json!([]), "{v}");
    assert!(
        v["query"]
            .as_str()
            .unwrap()
            .contains("FILTER(VALUES('Product'[Category]), [Total] > 50)"),
        "{v}"
    );
    run(
        &mut s,
        "visual.filter",
        json!({"page": page, "visual": visual, "field": "[Total]", "op": ">", "value": 20}),
    );
    let v = run(&mut s, "visual.records", all.clone());
    assert_eq!(amounts(&v), [30.0], "{v}");
    // without the other filters: Games (70) is kept, Toys (80) too, by > 75 only Toys
    for (id, p) in [
        ("page.clear_filters", json!({"page": page})),
        ("report.clear_filters", json!({})),
        ("visual.clear_filters", json!({"page": page, "visual": visual})),
        (
            "visual.filter",
            json!({"page": page, "visual": visual, "field": "[Total]", "op": ">", "value": 75}),
        ),
    ] {
        run(&mut s, id, p);
    }
    let v = run(&mut s, "visual.records", all.clone());
    assert_eq!(amounts(&v), [10.0, 30.0, 40.0], "{v}");
    // a slicer's selection in the reader's view: Games only, with the filters gone
    for (id, p) in [
        ("page.clear_filters", json!({"page": page})),
        ("report.clear_filters", json!({})),
        ("visual.clear_filters", json!({"page": page, "visual": visual})),
    ] {
        run(&mut s, id, p);
    }
    let slicer = run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "slicer", "x": 420, "y": 0, "width": 200, "height": 300,
               "fields": [{"role": "Values", "field": "Product[Category]"}]}),
    )["visual"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(amounts(&run(&mut s, "visual.records", all.clone())).len(), 5);
    run(
        &mut s,
        "view.interact",
        json!({"action": "slicer_set", "page": page, "visual": slicer, "values": ["Games"]}),
    );
    let v = run(&mut s, "visual.records", all.clone());
    assert_eq!(amounts(&v), [20.0, 50.0], "{v}");
    // the stored report is not changed by the reader's slicer
    run(&mut s, "view.interact", json!({"action": "reset"}));
    assert_eq!(amounts(&run(&mut s, "visual.records", all)).len(), 5);
}

/// A comparison `alias.column <kind> literal` of a semantic-query filter (kind 0: equal, 1:
/// greater than).
fn cmp(alias: &str, column: &str, kind: u8, literal: &str) -> Value {
    json!({"Comparison": {"ComparisonKind": kind,
        "Left": {"Column": {"Expression": {"SourceRef": {"Source": alias}}, "Property": column}},
        "Right": {"Literal": {"Value": literal}}}})
}

/// See records applies a condition over columns of several tables (here an `Or` across the
/// dimension and the fact, as a hierarchy slicer or a file Desktop wrote may hold), the
/// dimension's column read through `RELATED` (docs/gaps.md G49).
#[test]
fn see_records_applies_a_condition_over_several_tables() {
    let (mut s, page, visual) = star();
    let filter = json!({"filters": [{"name": "Across", "filter": {
        "Version": 2,
        "From": [{"Name": "p", "Entity": "Product", "Type": 0}, {"Name": "s", "Entity": "Sales", "Type": 0}],
        "Where": [{"Condition": {"Or": {"Left": cmp("p", "Category", 0, "'Games'"), "Right": cmp("s", "Amount", 1, "35D")}}}],
    }}]});
    s.report
        .as_mut()
        .unwrap()
        .pages
        .iter_mut()
        .flat_map(|p| p.visuals.iter_mut())
        .find(|x| x.name == visual)
        .unwrap()
        .filter_config = Some(filter);
    let v = run(&mut s, "visual.records", json!({"page": page, "visual": visual}));
    // Games are 20 and 50; above 35: 40 and 50
    assert_eq!(amounts(&v), [20.0, 40.0, 50.0], "{v}");
    assert_eq!(v["not_applied"], json!([]), "{v}");
    assert!(
        v["query"].as_str().unwrap().contains(
            "FILTER('Sales', (RELATED('Product'[Category]) = \"Games\") || ('Sales'[Amount] > 35))"
        ),
        "{v}"
    );
}

/// A measure filter on a visual without grouping columns (a card) has no points to keep: it is
/// named, not applied.
#[test]
fn a_measure_filter_on_a_card_is_named_not_applied() {
    let (mut s, page, _) = star();
    let card = run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "card", "x": 0, "y": 320, "width": 200, "height": 100,
               "fields": [{"role": "Values", "field": "[Total]"}]}),
    )["visual"]
        .as_str()
        .unwrap()
        .to_string();
    run(
        &mut s,
        "visual.filter",
        json!({"page": page, "visual": card, "field": "[Total]", "op": ">", "value": 1000}),
    );
    let v = run(&mut s, "visual.records", json!({"page": page, "visual": card}));
    assert_eq!(amounts(&v).len(), 5, "{v}");
    assert!(
        v["not_applied"][0]
            .as_str()
            .is_some_and(|t| t.contains("without grouping columns")),
        "{v}"
    );
}

/// See records of a visual another visual's selection cross-highlights lists the rows of the
/// highlighted part, and names the source (docs/gaps.md G49).
#[test]
fn see_records_follows_a_cross_highlight() {
    let (mut s, page, visual) = star();
    let keys = run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "clusteredBarChart", "x": 420, "y": 0, "width": 300, "height": 300,
               "fields": [{"role": "Category", "field": "Sales[ProductKey]"}, {"role": "Y", "field": "[Total]"}]}),
    )["visual"]
        .as_str()
        .unwrap()
        .to_string();
    let all = json!({"page": page, "visual": visual});
    assert_eq!(amounts(&run(&mut s, "visual.records", all.clone())).len(), 5);
    run(
        &mut s,
        "view.interact",
        json!({"action": "select", "page": page, "visual": keys, "point": [{"c": 0, "v": 2}]}),
    );
    let snap = run(&mut s, "visual.records", all.clone());
    assert_eq!(amounts(&snap), [20.0, 50.0], "{snap}");
    assert_eq!(snap["cross_highlight"], json!(keys), "{snap}");
    // the point's rows under the highlight: Toys has no product 2
    let v = run(
        &mut s,
        "visual.records",
        json!({"page": page, "visual": visual, "point": [{"c": 0, "v": "Toys"}]}),
    );
    assert_eq!(amounts(&v), Vec::<f64>::new(), "{v}");
    // the source's own records are not narrowed by its selection
    let v = run(&mut s, "visual.records", json!({"page": page, "visual": keys}));
    assert_eq!(amounts(&v).len(), 5, "{v}");
    assert_eq!(v["cross_highlight"], Value::Null, "{v}");
    run(&mut s, "view.interact", json!({"action": "reset"}));
    let v = run(&mut s, "visual.records", all);
    assert_eq!(amounts(&v).len(), 5, "{v}");
}

#[test]
fn a_calculated_column_without_a_data_type_takes_its_expressions() {
    let mut s = shop();
    for (name, dax, want) in [
        ("Double", "Sales[Amount] * 2", "double"),
        ("Label", "Sales[Category] & \"!\"", "string"),
        ("Big", "Sales[Amount] > 10", "boolean"),
        ("One", "1", "int64"),
    ] {
        run(
            &mut s,
            "column.add",
            json!({"table": "Sales", "name": name, "expression": dax}),
        );
        let c = run(&mut s, "column.get", json!({"table": "Sales", "name": name}));
        assert_eq!(c["data_type"], want, "{name}");
    }
    // a data column still needs one
    let e = s
        .execute("column.add", &json!({"table": "Sales", "name": "X"}))
        .unwrap_err();
    assert!(e.contains("data_type"), "{e}");
    // the Data view shows the new column computed
    let v = run(
        &mut s,
        "table.rows",
        json!({"table": "Sales", "columns": ["Amount", "Double", "Big"]}),
    );
    assert_eq!(v["rows"][0], json!([100, 200, true]), "{v}");
}

#[test]
fn see_records_computes_calculated_columns_the_data_does_not_hold() {
    let (mut s, page, visual) = star();
    run(
        &mut s,
        "column.add",
        json!({"table": "Sales", "name": "Double", "expression": "Sales[Amount] * 2"}),
    );
    let v = run(
        &mut s,
        "visual.records",
        json!({"page": page, "visual": visual, "point": [{"c": 0, "v": "Games"}]}),
    );
    let names: Vec<&str> = v["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["ProductKey", "Amount", "Double"], "{v}");
    let mut doubles: Vec<f64> = v["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r[2].as_f64().unwrap())
        .collect();
    doubles.sort_by(f64::total_cmp);
    assert_eq!(doubles, [40.0, 100.0], "{v}");
}

/// `query.preview` of M text reading a shared expression from a record initializer: the
/// parameter's value, whatever its body (number, parameter `meta`, text), shadowed by a `let`
/// variable or a field of the same record, and through another query built on it; the session's
/// revision does not move. (A failure once reported as "9 instead of 6" here was the revision
/// counter of the test above, not the previewed value.)
#[test]
fn query_preview_reads_a_shared_expression_in_a_record() {
    let number = "6 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]";
    for (body, six) in [("6", json!(6)), (number, json!(6)), ("\"6\"", json!("6"))] {
        let mut s = Session::new();
        run(&mut s, "model.new", json!({"name": "P"}));
        run(
            &mut s,
            "expression.add",
            json!({"name": "Threshold", "expression": body}),
        );
        let rev = s.revision;
        let preview =
            |s: &mut Session, q: &str| run(s, "query.preview", json!({"expression": q}))["rows"].clone();
        let one = if six.is_string() { json!("1") } else { json!(1) };
        assert_eq!(
            preview(&mut s, "Table.FromRecords({[A = 1], [A = Threshold]})"),
            json!([[one], [six]]),
            "{body}"
        );
        assert_eq!(
            preview(
                &mut s,
                "let Threshold = 3 in Table.FromRecords({[A = 1], [A = Threshold]})"
            ),
            json!([[1], [3]]),
            "{body}"
        );
        assert_eq!(
            preview(&mut s, "Table.FromRecords({[Threshold = 3, A = Threshold]})"),
            json!([[3, 3]]),
            "{body}"
        );
        assert_eq!(s.revision, rev, "{body}: a preview changes nothing");
    }
    // a query built on the parameter
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "P"}));
    run(
        &mut s,
        "expression.add",
        json!({"name": "Threshold", "expression": "6"}),
    );
    run(
        &mut s,
        "expression.add",
        json!({"name": "Limit", "expression": "Threshold + 3"}),
    );
    let v = run(
        &mut s,
        "query.preview",
        json!({"expression": "Table.FromRecords({[A = Threshold, B = Limit]})"}),
    );
    assert_eq!(v["rows"], json!([[6, 9]]));
}
