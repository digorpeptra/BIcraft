//! `m.validate`, the M part of `model.validate` and `PBI_QueryOrder` (`mquery.rs`).

use serde_json::{Value, json};

use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn codes(v: &Value) -> Vec<String> {
    v["diagnostics"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|d| {
            format!(
                "{}:{}",
                d["code"].as_str().unwrap_or(""),
                d["name"].as_str().unwrap_or("")
            )
        })
        .collect()
}

const DATABRICKS: &str = "let\n    Source = Databricks.Catalogs(Host, HttpPath, [Catalog = null, Database = null]),\n    \
                          Navigation = Source{[Name = \"main\", Kind = \"Database\"]}[Data],\n    \
                          sales = Navigation{[Name = \"sales\", Kind = \"Table\"]}[Data]\nin\n    sales";

fn session() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Q"}));
    s
}

fn query_order(s: &Session) -> Option<Vec<String>> {
    let m = s.model.as_ref()?;
    let a = m.annotations.iter().find(|a| a.name == "PBI_QueryOrder")?;
    serde_json::from_str(&a.value).ok()
}

#[test]
fn m_validate_reports_syntax_undefined_names_and_unknown_functions() {
    let mut s = Session::new();
    // No model: parameters are undefined names, the connector is fine (never evaluated).
    let v = run(&mut s, "m.validate", json!({"expression": DATABRICKS}));
    assert_eq!(v["valid"], false);
    assert_eq!(codes(&v), ["m-undefined-name:Host", "m-undefined-name:HttpPath"]);
    assert_eq!(v["diagnostics"][0]["line"], 2);
    assert_eq!(v["diagnostics"][0]["column"], 34);
    assert_eq!(v["steps"], json!(["Source", "Navigation", "sales"]));
    assert_eq!(v["result_type"], "Table");
    assert_eq!(
        v["references"]["library"],
        json!([{"name": "Databricks.Catalogs", "kind": "connector"}])
    );
    // Names given by the caller.
    let v = run(
        &mut s,
        "m.validate",
        json!({"expression": DATABRICKS, "names": ["Host", "HttpPath"]}),
    );
    assert_eq!(v["valid"], true, "{v}");
    assert_eq!(v["references"]["queries"], json!(["Host", "HttpPath"]));
    // Syntax error with its position.
    let v = run(&mut s, "m.validate", json!({"expression": "let\n  a = 1,\nin a"}));
    assert_eq!(v["syntax_valid"], false);
    assert_eq!(codes(&v), ["m-syntax:"]);
    assert_eq!(v["diagnostics"][0]["line"], 3);
    // Unknown library function: a warning only.
    let v = run(
        &mut s,
        "m.validate",
        json!({"expression": "Table.Frobnicate(#table({}, {}))"}),
    );
    assert_eq!(v["valid"], true);
    assert_eq!(v["diagnostics"][0]["severity"], "warning");
    assert_eq!(codes(&v), ["m-unknown-function:Table.Frobnicate"]);
    // A quoted name is a query, never a library function.
    let v = run(&mut s, "m.validate", json!({"expression": "#\"Sales.Fact\""}));
    assert_eq!(codes(&v), ["m-undefined-name:Sales.Fact"]);
    assert!(
        s.execute("m.validate", &json!({"expression": "1", "bogus": 1}))
            .is_err()
    );
}

#[test]
fn m_validate_knows_the_open_models_queries() {
    let mut s = session();
    run(
        &mut s,
        "expression.add",
        json!({"name": "Host", "expression": "\"adb.example\" meta [IsParameterQuery = true, Type = \"Text\", IsParameterQueryRequired = true]"}),
    );
    run(
        &mut s,
        "expression.add",
        json!({"name": "HttpPath", "expression": "\"/sql\" meta [IsParameterQuery = true, Type = \"Text\", IsParameterQueryRequired = true]"}),
    );
    let v = run(&mut s, "m.validate", json!({"expression": DATABRICKS}));
    assert_eq!(v["valid"], true, "{v}");
    assert_eq!(v["checked_against_model"], true);
    let v = run(
        &mut s,
        "m.validate",
        json!({"expression": DATABRICKS, "shared_expressions": false}),
    );
    assert_eq!(v["valid"], false);
}

#[test]
fn model_validate_reports_m_errors() {
    let mut s = session();
    run(
        &mut s,
        "expression.add",
        json!({"name": "Host", "expression": "\"h\""}),
    );
    run(
        &mut s,
        "table.add",
        json!({"name": "Sales", "partition": {"source_type": "m", "expression": DATABRICKS}}),
    );
    let v = run(&mut s, "model.validate", json!({}));
    assert_eq!(v["valid"], false);
    assert_eq!(v["m"]["checked_count"], 2);
    assert_eq!(v["m"]["invalid_count"], 1);
    let o = &v["m"]["objects_with_diagnostics"][0];
    assert_eq!(
        (o["kind"].as_str(), o["table"].as_str()),
        (Some("partition"), Some("Sales"))
    );
    assert_eq!(codes(o), ["m-undefined-name:HttpPath"]);
    run(
        &mut s,
        "expression.add",
        json!({"name": "HttpPath", "expression": "\"p\""}),
    );
    let v = run(&mut s, "model.validate", json!({}));
    assert_eq!(v["valid"], true, "{v}");
    assert_eq!(v["m"]["objects_with_diagnostics"], json!([]));
}

#[test]
fn query_order_follows_adds_renames_and_removals() {
    let mut s = session();
    assert_eq!(query_order(&s), None);
    let lit = |c: &str| format!("let Source = #table({{\"{c}\"}}, {{{{1}}}}) in Source");
    run(
        &mut s,
        "table.add",
        json!({"name": "Sales", "partition": {"source_type": "m", "expression": lit("A")}}),
    );
    assert_eq!(query_order(&s), Some(vec!["Sales".into()]));
    run(&mut s, "table.import", json!({"name": "Product", "m": lit("B")}));
    run(
        &mut s,
        "expression.add",
        json!({"name": "Host", "expression": "\"h\""}),
    );
    // A calculated table is not a query.
    run(
        &mut s,
        "table.add",
        json!({"name": "Calc", "partition": {"source_type": "calculated", "expression": "Sales"}}),
    );
    assert_eq!(
        query_order(&s),
        Some(vec!["Sales".into(), "Product".into(), "Host".into()])
    );
    run(
        &mut s,
        "table.rename",
        json!({"name": "Sales", "new_name": "Fact Sales"}),
    );
    run(
        &mut s,
        "expression.rename",
        json!({"name": "Host", "new_name": "Server"}),
    );
    assert_eq!(
        query_order(&s),
        Some(vec!["Fact Sales".into(), "Product".into(), "Server".into()])
    );
    run(&mut s, "table.remove", json!({"name": "Product"}));
    assert_eq!(query_order(&s), Some(vec!["Fact Sales".into(), "Server".into()]));
    // The serialized form is Desktop's: compact JSON.
    let m = s.model.as_ref().unwrap();
    let a = m.annotations.iter().find(|a| a.name == "PBI_QueryOrder").unwrap();
    assert_eq!(a.value, r#"["Fact Sales","Server"]"#);
    // Undo puts the list back with the model.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(
        query_order(&s),
        Some(vec!["Fact Sales".into(), "Product".into(), "Server".into()])
    );
    // A failed command changes nothing.
    assert!(
        s.execute(
            "expression.rename",
            &json!({"name": "Server", "new_name": "Fact Sales"})
        )
        .is_err()
    );
    assert_eq!(
        query_order(&s),
        Some(vec!["Fact Sales".into(), "Product".into(), "Server".into()])
    );
}

#[test]
fn query_order_is_left_alone_when_absent_on_a_model_with_queries_or_not_a_list() {
    let mut s = session();
    let lit = "let Source = #table({\"A\"}, {{1}}) in Source";
    run(
        &mut s,
        "table.add",
        json!({"name": "A", "partition": {"source_type": "m", "expression": lit}}),
    );
    run(
        &mut s,
        "model.set_annotation",
        json!({"name": "PBI_QueryOrder", "value": null}),
    );
    run(
        &mut s,
        "table.add",
        json!({"name": "B", "partition": {"source_type": "m", "expression": lit}}),
    );
    assert_eq!(query_order(&s), None);
    run(
        &mut s,
        "model.set_annotation",
        json!({"name": "PBI_QueryOrder", "value": "not json"}),
    );
    run(
        &mut s,
        "table.add",
        json!({"name": "C", "partition": {"source_type": "m", "expression": lit}}),
    );
    let m = s.model.as_ref().unwrap();
    assert_eq!(
        m.annotations
            .iter()
            .find(|a| a.name == "PBI_QueryOrder")
            .map(|a| a.value.as_str()),
        Some("not json")
    );
    // Queries Desktop lists that bicraft does not know (another case) are kept in place.
    run(
        &mut s,
        "model.set_annotation",
        json!({"name": "PBI_QueryOrder", "value": r#"["X","a","b","C"]"#}),
    );
    run(&mut s, "table.rename", json!({"name": "B", "new_name": "Bee"}));
    let m = s.model.as_ref().unwrap();
    assert_eq!(
        m.annotations
            .iter()
            .find(|a| a.name == "PBI_QueryOrder")
            .map(|a| a.value.as_str()),
        Some(r#"["X","a","Bee","C"]"#)
    );
}

/// Desktop appends a table that becomes a query later (an M partition replacing a calculated
/// one) at the end of `PBI_QueryOrder`, and drops it when it stops being one; extra M
/// partitions of a query table change nothing, and a rename that only changes case follows.
#[test]
fn query_order_follows_partition_changes_and_case_renames() {
    let mut s = session();
    let lit = |c: &str| format!("let Source = #table({{\"{c}\"}}, {{{{1}}}}) in Source");
    run(
        &mut s,
        "table.add",
        json!({"name": "Calc", "partition": {"source_type": "calculated", "expression": "{1}"}}),
    );
    assert_eq!(query_order(&s), None);
    run(
        &mut s,
        "table.add",
        json!({"name": "A", "partition": {"source_type": "m", "expression": lit("A")}}),
    );
    run(
        &mut s,
        "expression.add",
        json!({"name": "P", "expression": "\"p\""}),
    );
    assert_eq!(query_order(&s), Some(vec!["A".into(), "P".into()]));
    // The calculated table becomes a query: appended after the expressions.
    run(
        &mut s,
        "table.set_partition",
        json!({"table": "Calc", "expression": lit("C"), "kind": "m"}),
    );
    assert_eq!(query_order(&s), Some(vec!["A".into(), "P".into(), "Calc".into()]));
    // A second M partition and its removal: the same queries.
    run(
        &mut s,
        "partition.add",
        json!({"table": "A", "name": "A2", "expression": lit("A")}),
    );
    run(&mut s, "partition.remove", json!({"table": "A", "name": "A2"}));
    assert_eq!(query_order(&s), Some(vec!["A".into(), "P".into(), "Calc".into()]));
    // Back to DAX: no longer a query.
    run(
        &mut s,
        "partition.set",
        json!({"table": "Calc", "expression": "{2}", "kind": "calculated"}),
    );
    assert_eq!(query_order(&s), Some(vec!["A".into(), "P".into()]));
    // A rename that only changes case keeps the place and takes the new spelling.
    run(&mut s, "table.rename", json!({"name": "A", "new_name": "a"}));
    run(&mut s, "expression.rename", json!({"name": "P", "new_name": "p"}));
    assert_eq!(query_order(&s), Some(vec!["a".into(), "p".into()]));
}

/// Queries without lineage tags (models from older files): a rename that only changes case
/// still takes the new spelling.
#[test]
fn query_order_follows_case_renames_without_lineage_tags() {
    let mut s = session();
    let lit = "let Source = #table({\"A\"}, {{1}}) in Source";
    run(
        &mut s,
        "table.add",
        json!({"name": "Sales", "partition": {"source_type": "m", "expression": lit}}),
    );
    run(
        &mut s,
        "expression.add",
        json!({"name": "Host", "expression": "\"h\""}),
    );
    let m = s.model.as_mut().unwrap();
    m.tables[0].lineage_tag = None;
    m.expressions[0].lineage_tag = None;
    run(
        &mut s,
        "table.rename",
        json!({"name": "Sales", "new_name": "SALES"}),
    );
    run(
        &mut s,
        "expression.rename",
        json!({"name": "Host", "new_name": "host"}),
    );
    assert_eq!(query_order(&s), Some(vec!["SALES".into(), "host".into()]));
}

#[test]
fn m_format_formats_text_and_applies_to_a_query_in_one_undo_step() {
    let mut s = session();
    // An expression: the formatted text only, nothing changes.
    let revision = s.revision;
    let v = run(
        &mut s,
        "m.format",
        json!({"expression": "let a=1, // one\n b=a+1 in b"}),
    );
    assert_eq!(
        v["expression"],
        "let\n    a = 1, // one\n    b = a + 1\nin\n    b"
    );
    assert_eq!(
        (v["changed"].as_bool(), v["applied"].as_bool()),
        (Some(true), Some(false))
    );
    assert_eq!(s.revision, revision, "a format without apply is not an edit");
    // Idempotent.
    let again = run(&mut s, "m.format", json!({"expression": v["expression"]}));
    assert_eq!(again["changed"], false);
    // Options.
    let v = run(
        &mut s,
        "m.format",
        json!({"expression": "let a=1 in a", "indent": 2}),
    );
    assert_eq!(v["expression"], "let\n  a = 1\nin\n  a");
    // A query and a table partition, applied: one undo step each.
    run(
        &mut s,
        "expression.add",
        json!({"name": "Q1", "expression": "let x=1 in x"}),
    );
    run(
        &mut s,
        "table.add",
        json!({"name": "T", "partition": {"source_type": "m", "expression": "#table({\"a\"},{{1}})"}}),
    );
    let v = run(&mut s, "m.format", json!({"name": "Q1", "apply": true}));
    assert_eq!(
        (v["source"].as_str(), v["applied"].as_bool()),
        (Some("query"), Some(true))
    );
    let q = |s: &Session| s.model.as_ref().map(|m| m.expressions[0].expression.clone());
    assert_eq!(q(&s).as_deref(), Some("let\n    x = 1\nin\n    x"));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(q(&s).as_deref(), Some("let x=1 in x"));
    let v = run(&mut s, "m.format", json!({"name": "T", "apply": true}));
    assert_eq!(
        (v["source"].as_str(), v["applied"].as_bool()),
        (Some("table"), Some(true))
    );
    let part = |s: &Session| {
        s.model
            .as_ref()
            .and_then(|m| m.tables[0].partitions[0].expression.clone())
    };
    assert_eq!(part(&s).as_deref(), Some("#table({\"a\"}, {{1}})"));
    // Already formatted: nothing applied, no undo step.
    let revision = s.revision;
    let v = run(&mut s, "m.format", json!({"name": "T", "apply": true}));
    assert_eq!(
        (v["changed"].as_bool(), v["applied"].as_bool()),
        (Some(false), Some(false))
    );
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(part(&s).as_deref(), Some("#table({\"a\"},{{1}})"));
    assert!(s.revision > revision);
}

#[test]
fn m_format_refuses_bad_input() {
    let mut s = session();
    for p in [
        json!({}),
        json!({"expression": "let a = in a"}),
        json!({"expression": "1", "apply": true}),
        json!({"expression": "1", "max_width": -1}),
        json!({"expression": "1", "indent": 17}),
        json!({"expression": "1", "bogus": 1}),
        json!({"name": "Nope"}),
    ] {
        assert!(s.execute("m.format", &p).is_err(), "{p}");
    }
    let huge = format!("{}1{}", "(".repeat(5000), ")".repeat(5000));
    assert!(s.execute("m.format", &json!({"expression": huge})).is_err());
}

/// `m.complete`, `m.signature` and `m.hover` (M11.35): the catalogue's functions, the text's
/// steps, the model's queries and a table's columns, with cursors in characters.
#[test]
fn m_language_services_complete_sign_and_describe() {
    let mut s = session();
    run(
        &mut s,
        "table.import",
        json!({"name": "Sales", "m": "#table(type table [Qty = Int64.Type, City = text], {{1, \"Paris\"}})"}),
    );
    run(
        &mut s,
        "expression.add",
        json!({"name": "Cities Data", "expression": "1"}),
    );
    let revision = s.revision;
    let v = run(&mut s, "m.complete", json!({"expression": "é = Table.SelectR"}));
    assert_eq!(
        v["replace"],
        json!({"start": 4, "end": 17}),
        "characters, not bytes"
    );
    assert_eq!(v["items"][0]["label"], "Table.SelectRows");
    assert_eq!(v["items"][0]["insert"], "Table.SelectRows(");
    assert_eq!(v["items"][0]["kind"], "function");
    assert_eq!(v["items"][0]["detail"], "Table.SelectRows(table, condition)");
    // the model's queries, quoted
    let v = run(&mut s, "m.complete", json!({"expression": "Cities"}));
    assert_eq!(v["items"][0]["insert"], "#\"Cities Data\"");
    assert_eq!(v["items"][0]["kind"], "query");
    // a table's columns inside [ ], by line and column
    let text = "let\n    S = Sales,\n    K = Table.SelectRows(S, each [Q)\nin\n    K";
    let v = run(
        &mut s,
        "m.complete",
        json!({"expression": text, "line": 3, "column": 36, "table": "Sales"}),
    );
    assert_eq!(
        v["items"],
        json!([{"label": "Qty", "insert": "Qty]", "kind": "column", "detail": ""}])
    );
    // the text's own steps
    let v = run(
        &mut s,
        "m.complete",
        json!({"expression": "let Source = 1, b = Sou"}),
    );
    assert_eq!(v["items"][0]["label"], "Source");
    // nothing in a text
    let v = run(&mut s, "m.complete", json!({"expression": "\"Table.Sel"}));
    assert_eq!(v["items"], json!([]));
    // the signature and the parameter being typed
    let v = run(
        &mut s,
        "m.signature",
        json!({"expression": "Text.PadStart([City], 5, ", "cursor": 25}),
    );
    assert_eq!(v["function"]["name"], "Text.PadStart");
    assert_eq!(
        v["function"]["params"][2],
        json!({"name": "character", "optional": true})
    );
    assert_eq!((v["argument"].as_u64(), v["active"].as_u64()), (Some(2), Some(2)));
    let v = run(&mut s, "m.signature", json!({"expression": "1 + 2"}));
    assert!(v["function"].is_null());
    // hover
    let v = run(
        &mut s,
        "m.hover",
        json!({"expression": "Table.RowCount(Sales)", "cursor": 3}),
    );
    assert_eq!(v["hover"]["kind"], "function");
    assert_eq!(v["hover"]["range"], json!({"start": 0, "end": 14}));
    assert_eq!(v["hover"]["signature"], "Table.RowCount(table)");
    assert_eq!(
        v["hover"]["params"],
        json!([{"name": "table", "optional": false}])
    );
    let v = run(
        &mut s,
        "m.hover",
        json!({"expression": "Table.RowCount(Sales)", "cursor": 17}),
    );
    assert_eq!(
        (v["hover"]["name"].as_str(), v["hover"]["kind"].as_str()),
        (Some("Sales"), Some("query"))
    );
    let v = run(
        &mut s,
        "m.hover",
        json!({"expression": "each [City]", "cursor": 7, "table": "Sales"}),
    );
    assert_eq!(v["hover"]["kind"], "column");
    let v = run(&mut s, "m.hover", json!({"expression": "1 + 2", "cursor": 2}));
    assert!(v["hover"].is_null());
    assert_eq!(s.revision, revision, "the language services change nothing");
}

#[test]
fn m_language_services_refuse_bad_input() {
    let mut s = session();
    for id in ["m.complete", "m.signature", "m.hover"] {
        for p in [
            json!({}),
            json!({"expression": 1}),
            json!({"expression": "a", "cursor": -1}),
            json!({"expression": "a", "cursor": 1, "line": 1}),
            json!({"expression": "a", "line": 0}),
            json!({"expression": "a", "line": 3, "column": 1}),
            json!({"expression": "a", "bogus": 1}),
            json!({"expression": "x".repeat((1 << 20) + 1)}),
        ] {
            assert!(s.execute(id, &p).is_err(), "{id} {p}");
        }
        // a cursor past the end is the end
        assert!(
            s.execute(id, &json!({"expression": "Text.", "cursor": 99}))
                .is_ok()
        );
    }
    for p in [
        json!({"expression": "a", "table": "Nope"}),
        json!({"expression": "a", "columns": [1]}),
        json!({"expression": "a", "steps": "x"}),
    ] {
        assert!(s.execute("m.complete", &p).is_err(), "{p}");
    }
}
