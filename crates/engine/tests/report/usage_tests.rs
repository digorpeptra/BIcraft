//! Tests of `model.usage`, `model.unused` and `model.remove_unused` (`usage.rs`), on generated
//! models and reports only.

use bicraft_report::{FieldBinding, FieldRef, NamedDocument, Page, Report, Visual};
use serde_json::{Map, Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn col(t: &str, c: &str) -> Value {
    json!({"Column": {"Expression": {"SourceRef": {"Entity": t}}, "Property": c}})
}

fn meas(t: &str, m: &str) -> Value {
    json!({"Measure": {"Expression": {"SourceRef": {"Entity": t}}, "Property": m}})
}

fn obj(v: &Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap_or_default()
}

/// Sales(Amount, Cost, Spare, ProductKey, Date) → Product(ProductKey, Name, SortOrder); measures
/// [Total], [Margin], [Margin %] (in a visual), [Old] → [Total], [Older] → [Old], [Orphan].
fn session() -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": "let x = 1 in x"}}),
        ),
        ("table.add", json!({"name": "Product"})),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Cost", "data_type": "double"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Spare", "data_type": "double"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "ProductKey", "data_type": "int64"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Date", "data_type": "dateTime"}),
        ),
        (
            "column.add",
            json!({"table": "Product", "name": "ProductKey", "data_type": "int64"}),
        ),
        (
            "column.add",
            json!({"table": "Product", "name": "Name", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Product", "name": "SortOrder", "data_type": "int64"}),
        ),
        (
            "column.set",
            json!({"table": "Product", "name": "Name", "sort_by_column": "SortOrder"}),
        ),
        (
            "relationship.add",
            json!({"from": "Sales[ProductKey]", "to": "Product[ProductKey]", "name": "Sales_Product"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Margin", "expression": "[Total] - SUM(Sales[Cost])"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Margin %", "expression": "DIVIDE([Margin], [Total])"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Old", "expression": "[Total] * 2"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Older", "expression": "[Old] + 1"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Orphan", "expression": "1"}),
        ),
    ] {
        run(&mut s, id, p);
    }
    s.report = Some(Report {
        pages: vec![Page {
            name: "p1".into(),
            display_name: "Overview".into(),
            visuals: vec![Visual {
                name: "v1".into(),
                visual_type: Some("card".into()),
                fields: vec![FieldBinding {
                    display_name: None,
                    show_as: None,
                    role: "Values".into(),
                    field: FieldRef::Measure {
                        table: "Sales".into(),
                        measure: "Margin %".into(),
                    },
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    });
    s
}

fn names(list: &Value) -> Vec<String> {
    list.as_array()
        .into_iter()
        .flatten()
        .map(|e| match e.get("table") {
            Some(t) if e["kind"] != "table" => {
                format!("{}[{}]", t.as_str().unwrap(), e["name"].as_str().unwrap())
            }
            _ => e["name"].as_str().unwrap().to_string(),
        })
        .collect()
}

fn kinds(list: &Value) -> Vec<String> {
    let mut k: Vec<String> = list
        .as_array()
        .into_iter()
        .flatten()
        .map(|u| u["kind"].as_str().unwrap_or_default().to_string())
        .collect();
    k.sort();
    k.dedup();
    k
}

#[test]
fn transitive_chains_and_verdicts() {
    let mut s = session();
    let rev = s.revision;
    let v = run(&mut s, "model.unused", json!({}));
    assert_eq!(s.revision, rev, "model.unused is read-only");
    assert_eq!(
        names(&v["unused"]["measures"]),
        ["Sales[Old]", "Sales[Older]", "Sales[Orphan]"]
    );
    assert_eq!(
        names(&v["unused"]["columns"]),
        ["Product[Name]", "Sales[Date]", "Sales[Spare]"]
    );
    let old = &v["unused"]["measures"][0];
    assert!(old["why"].as_str().unwrap().contains("[Older]"), "{old}");
    assert_eq!(v["unused"]["measures"][1]["why"], "referenced nowhere");
    // Cost is used through [Margin] → [Margin %] → the visual
    assert!(!names(&v["unused"]["columns"]).contains(&"Sales[Cost]".to_string()));
    // the keys and the sort-by target are kept by rules, not by the report
    let kept: Vec<(String, Value)> = v["kept"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| {
            (
                format!(
                    "{}[{}]",
                    k["table"].as_str().unwrap_or(""),
                    k["name"].as_str().unwrap()
                ),
                k["rules"].clone(),
            )
        })
        .collect();
    assert!(
        kept.contains(&("Sales[ProductKey]".into(), json!(["relationship"]))),
        "{kept:?}"
    );
    assert!(
        kept.contains(&("Product[ProductKey]".into(), json!(["relationship"]))),
        "{kept:?}"
    );
    assert!(
        kept.contains(&("Product[SortOrder]".into(), json!(["sort by"]))),
        "{kept:?}"
    );
    assert!(v["warnings"][0].as_str().unwrap().contains("1 report(s)"));
}

#[test]
fn usage_of_one_object_direct_indirect_and_graph() {
    let mut s = session();
    let v = run(&mut s, "model.usage", json!({"object": "[Total]"}));
    assert_eq!(
        v["object"],
        json!({"kind": "measure", "table": "Sales", "name": "Total"})
    );
    assert_eq!(v["used"], true);
    assert_eq!(v["used_by_report"], true);
    assert_eq!(
        names(&v["direct"]),
        ["Sales[Margin]", "Sales[Margin %]", "Sales[Old]"]
    );
    let indirect = v["indirect"].as_array().unwrap();
    let visual = indirect
        .iter()
        .find(|i| i["usage"]["kind"] == "visual")
        .expect("visual through [Margin %]");
    assert_eq!(visual["usage"]["role"], "Values");
    assert_eq!(visual["usage"]["page"], "Overview");
    assert_eq!(visual["through"], json!(["measure [Margin %]"]));
    let deps: Vec<String> = names(
        &v["dependants"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["object"].clone())
            .collect(),
    );
    assert_eq!(
        deps,
        ["Sales[Margin]", "Sales[Margin %]", "Sales[Old]", "Sales[Older]"]
    );
    assert_eq!(names(&v["depends_on"]), ["Sales[Amount]"]);
    assert!(v["graph"]["edges"].as_array().unwrap().len() >= 5);

    let v = run(&mut s, "model.usage", json!({"object": "Sales[Cost]"}));
    assert_eq!(v["used_by_report"], true);
    let through: Vec<&Value> = v["indirect"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| &i["through"])
        .collect();
    assert!(
        through.contains(&&json!(["measure [Margin]", "measure [Margin %]"])),
        "{through:?}"
    );

    // a table: its members' usages are indirect
    let v = run(&mut s, "model.usage", json!({"object": "Product"}));
    assert_eq!(v["object"]["kind"], "table");
    assert!(
        kinds(
            &v["indirect"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| i["usage"].clone())
                .collect()
        )
        .contains(&"relationship".to_string())
    );

    // all objects
    let v = run(&mut s, "model.usage", json!({"kind": "measure", "graph": false}));
    assert_eq!(v["counts"]["objects"], 6);
    assert!(v.get("graph").is_none());
    assert!(s.execute("model.usage", &json!({"object": "[Nope]"})).is_err());
    assert!(
        s.execute("model.usage", &json!({"object": "[Total]", "kind": "bogus"}))
            .is_err()
    );
}

fn props(v: Value) -> Map<String, Value> {
    obj(&v)
}

#[test]
fn every_report_usage_kind() {
    let mut s = session();
    let report = s.report.as_mut().unwrap();
    report.filter_config = Some(json!({"filters": [{"name": "f", "field": col("Sales", "Date")}]}));
    report.bookmarks = vec![NamedDocument {
        name: "B1".into(),
        document: json!({"explorationState": {"sections": {"p1": {"filters": {"byExpr": [{"expression": col("Sales", "Spare")}]}}}}}),
    }];
    let page = &mut report.pages[0];
    page.filter_config = Some(json!({"filters": [
        {"name": "d", "howCreated": "Drillthrough", "field": col("Product", "Name")},
        {"name": "p", "field": meas("Sales", "Orphan")}]}));
    page.extra = props(
        json!({"pageBinding": {"type": "Tooltip", "parameters": [{"fieldExpr": col("Sales", "Cost")}]}}),
    );
    let v = &mut page.visuals[0];
    v.filter_config = Some(json!({"filters": [{"name": "vf", "field": col("Product", "SortOrder")}]}));
    v.visual_extra = props(json!({
        "query": {
            "queryState": {"Values": {"projections": [{"field": meas("Sales", "Margin %")}]}},
            "sortDefinition": {"sort": [{"field": meas("Sales", "Old")}]},
        },
        "objects": {"dataPoint": [{"properties": {"fill": {"solid": {"color": {"expr": meas("Sales", "Older")}}}}}]},
    }));
    v.fields.push(FieldBinding { display_name: None, show_as: None,
        role: "Tooltips".into(),
        field: FieldRef::Other {
            raw: json!({"HierarchyLevel": {"Expression": {"Hierarchy": {"Expression": {"SourceRef": {"Entity": "Product"}},
                        "Hierarchy": "Products"}}, "Level": "Key"}}),
        },
    });
    let t = s.model.as_mut().unwrap().table_mut("Product").unwrap();
    t.extra.insert(
        "hierarchies".into(),
        json!([{"name": "Products", "levels": [{"name": "Key", "column": "ProductKey", "ordinal": 0}]}]),
    );
    let all = run(&mut s, "model.usage", json!({"graph": false}));
    let usage_of = |table: &str, name: &str| -> Vec<Value> {
        all["objects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["object"]["table"] == table && o["object"]["name"] == name)
            .unwrap_or_else(|| panic!("{table}[{name}]"))["used_by"]
            .as_array()
            .unwrap()
            .clone()
    };
    let has = |table: &str, name: &str, kind: &str| {
        let u = usage_of(table, name);
        assert!(
            u.iter().any(|x| x["kind"] == kind),
            "{table}[{name}] {kind}: {u:?}"
        );
        u
    };
    has("Sales", "Date", "report filter");
    has("Sales", "Spare", "bookmark");
    has("Product", "Name", "drillthrough");
    has("Sales", "Orphan", "page filter");
    has("Sales", "Cost", "tooltip page");
    has("Product", "SortOrder", "visual filter");
    has("Sales", "Old", "sort");
    has("Sales", "Older", "formatting");
    let key = has("Product", "ProductKey", "visual");
    assert!(
        key.iter()
            .any(|x| x["kind"] == "visual" && x["role"] == "Tooltips")
    );
    has("Product", "ProductKey", "hierarchy");
    // the field found both typed and through queryState is listed once, with its role
    let margin = usage_of("Sales", "Margin %");
    assert_eq!(
        margin.iter().filter(|x| x["kind"] == "visual").count(),
        1,
        "{margin:?}"
    );
    assert_eq!(margin[0]["visual_type"], "card");
    let unused = run(&mut s, "model.unused", json!({}));
    assert!(
        unused["unused"]["measures"].as_array().unwrap().is_empty(),
        "{unused}"
    );
}

#[test]
fn model_usage_kinds_and_conservative_rules() {
    let mut s = session();
    for (id, p) in [
        ("table.add", json!({"name": "Date"})),
        (
            "column.add",
            json!({"table": "Date", "name": "Date", "data_type": "dateTime"}),
        ),
        (
            "column.add",
            json!({"table": "Date", "name": "Year", "data_type": "int64"}),
        ),
        (
            "column.add",
            json!({"table": "Date", "name": "Secret", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Date", "name": "Shown", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Date", "name": "Talked", "data_type": "string"}),
        ),
        (
            "relationship.add",
            json!({"from": "Sales[Date]", "to": "Date[Date]", "active": false}),
        ),
        ("role.add", json!({"name": "Region"})),
        (
            "role.set_filter",
            json!({"role": "Region", "table": "Product", "filter_expression": "Product[Name] = \"x\""}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "ByShip", "expression": "CALCULATE([Total], USERELATIONSHIP(Sales[Date], 'Date'[Date]))"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Treat", "expression": "CALCULATE([Total], TREATAS(VALUES('Date'[Year]), Sales[Spare]))"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Fmt", "expression": "\"0.0\""}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Named", "expression": "3"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Picked", "expression": "4"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "InParam", "expression": "5"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Persp", "expression": "6"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Transl", "expression": "7"}),
        ),
        ("calculation_group.add", json!({"name": "Calc"})),
        (
            "calculation_item.add",
            json!({"group": "Calc", "name": "Only", "expression": "IF(ISSELECTEDMEASURE([Picked]) || SELECTEDMEASURENAME() = \"Named\", SELECTEDMEASURE())"}),
        ),
        (
            "table.add",
            json!({"name": "Param", "partition": {"source_type": "calculated",
                   "expression": "{(\"InParam\", NAMEOF([InParam]), 0)}"}}),
        ),
        ("culture.add", json!({"name": "fr-FR"})),
        (
            "translation.set",
            json!({"culture": "fr-FR", "object": {"table": "Sales", "measure": "Transl"}, "property": "caption", "value": "Traduit"}),
        ),
    ] {
        run(&mut s, id, p);
    }
    let m = s.model.as_mut().unwrap();
    // ByShip and Treat in the visual so their references count
    let fmt_measure = m
        .table_mut("Sales")
        .unwrap()
        .measures
        .iter_mut()
        .find(|x| x.name == "Margin %")
        .unwrap();
    fmt_measure
        .extra
        .insert("formatStringDefinition".into(), json!({"expression": "[Fmt]"}));
    m.roles[0].table_permissions.push(bicraft_model::TablePermission {
        table: "Date".into(),
        filter_expression: None,
        extra: obj(&json!({"columnPermissions": [{"name": "Secret", "metadataPermission": "none"}]})),
    });
    m.extra.insert(
        "perspectives".into(),
        json!([{"name": "Lite", "tables": [{"name": "Sales", "measures": [{"name": "Persp"}]}]}]),
    );
    let cultures = m.extra.get_mut("cultures").unwrap().as_array_mut().unwrap();
    cultures[0].as_object_mut().unwrap().insert(
        "linguisticMetadata".into(),
        json!({"content": json!({"Entities": {"date.talked": {"Definition": {"Binding": {"ConceptualEntity": "Date", "ConceptualProperty": "Talked"}}}}}).to_string(),
               "contentType": "json"}),
    );
    let page = &mut s.report.as_mut().unwrap().pages[0];
    for name in ["ByShip", "Treat"] {
        page.visuals[0].fields.push(FieldBinding {
            display_name: None,
            show_as: None,
            role: "Values".into(),
            field: FieldRef::Measure {
                table: "Sales".into(),
                measure: name.into(),
            },
        });
    }
    let v = run(&mut s, "model.unused", json!({}));
    let unused_measures = names(&v["unused"]["measures"]);
    for kept in [
        "Sales[Fmt]",
        "Sales[Named]",
        "Sales[Picked]",
        "Sales[Persp]",
        "Sales[Transl]",
    ] {
        assert!(
            !unused_measures.contains(&kept.to_string()),
            "{kept} flagged: {v}"
        );
    }
    let unused_columns = names(&v["unused"]["columns"]);
    for kept in [
        "Date[Secret]",
        "Date[Talked]",
        "Date[Date]",
        "Sales[Date]",
        "Product[Name]",
        "Date[Year]",
        "Sales[Spare]",
    ] {
        assert!(
            !unused_columns.contains(&kept.to_string()),
            "{kept} flagged: {unused_columns:?}"
        );
    }
    assert!(
        unused_columns.contains(&"Date[Shown]".to_string()),
        "{unused_columns:?}"
    );
    // the field parameter is unused (no report uses it), so is the measure only it names
    assert_eq!(names(&v["unused"]["tables"]), ["Param"]);
    assert!(unused_measures.contains(&"Sales[InParam]".to_string()));
    assert!(v["unused"]["tables"][0]["contains"]["columns"].as_u64().is_some());

    let rules = |table: &str, name: &str| -> Vec<String> {
        let k = v["kept"]
            .as_array()
            .unwrap()
            .iter()
            .find(|k| k["table"] == table && k["name"] == name);
        k.map(|k| {
            k["rules"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r.as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
    };
    assert_eq!(rules("Date", "Secret"), ["object-level security"]);
    assert_eq!(rules("Date", "Talked"), ["linguistic schema"]);
    assert_eq!(rules("Sales", "Persp"), ["perspective"]);
    assert_eq!(rules("Sales", "Transl"), ["translation"]);
    assert_eq!(rules("Product", "Name"), ["row filter"]);
    assert_eq!(rules("Sales", "Picked"), ["needed by a kept object"]);

    // the usage view of the same objects
    let u = run(&mut s, "model.usage", json!({"object": "[Named]"}));
    assert_eq!(
        kinds(&u["direct"]),
        ["calculation group (measure named in a string)"]
    );
    let u = run(&mut s, "model.usage", json!({"object": "[Fmt]"}));
    assert_eq!(kinds(&u["direct"]), ["format string expression"]);
    assert_eq!(u["used_by_report"], true);
    let u = run(&mut s, "model.usage", json!({"object": "'Date'[Date]"}));
    let k = kinds(&u["direct"]);
    assert!(
        k.contains(&"relationship".to_string()) && k.contains(&"measure".to_string()),
        "{k:?}"
    );
    assert!(
        u["direct"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["kind"] == "relationship" && x["active"] == false)
    );
    let u = run(&mut s, "model.usage", json!({"object": "[InParam]"}));
    assert_eq!(kinds(&u["direct"]), ["calculated table"]);

    // M: a partition naming a shared expression, an expression naming another
    let m = s.model.as_mut().unwrap();
    for (name, expression) in [("Root", "\"C:\\\\data\""), ("Src", "let p = Root in p")] {
        m.expressions.push(bicraft_model::NamedExpression {
            name: name.into(),
            kind: "m".into(),
            expression: expression.into(),
            ..Default::default()
        });
    }
    m.table_mut("Sales").unwrap().partitions[0].expression = Some("let s = Src in s".into());
    let u = run(
        &mut s,
        "model.usage",
        json!({"object": "Root", "kind": "expression"}),
    );
    assert_eq!(kinds(&u["direct"]), ["M expression"]);
    let indirect = u["indirect"].as_array().unwrap();
    assert_eq!(indirect[0]["through"], json!(["expression `Src`"]), "{u}");
    assert_eq!(indirect[0]["usage"]["kind"], "M query");
    assert!(
        indirect
            .iter()
            .any(|i| i["through"] == json!(["expression `Src`", "table Sales"])),
        "{u}"
    );
    // what needs Root: Src, the table whose query names Src, then (through the table) its members
    let dependants = names(
        &u["dependants"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["object"].clone())
            .collect(),
    );
    assert_eq!(dependants[..2], ["Src", "Sales"]);
    assert!(dependants.contains(&"Sales[Amount]".to_string()));
    assert_eq!(u["dependants"][1]["depth"], 2);

    // unused shared expressions: Orphan (nothing names it), Chain → Leaf (only Chain names Leaf);
    // a parameter is kept
    let m = s.model.as_mut().unwrap();
    for (name, expression) in [
        ("Orphaned", "1"),
        ("Leaf", "2"),
        ("Chain", "Leaf + 1"),
        (
            "Server",
            "\"srv\" meta [IsParameterQuery = true, Type = \"Text\"]",
        ),
    ] {
        m.expressions.push(bicraft_model::NamedExpression {
            name: name.into(),
            kind: "m".into(),
            expression: expression.into(),
            ..Default::default()
        });
    }
    let v = run(&mut s, "model.unused", json!({}));
    assert_eq!(names(&v["unused"]["expressions"]), ["Chain", "Leaf", "Orphaned"]);
    let leaf = &v["unused"]["expressions"][1];
    assert!(
        leaf["why"].as_str().unwrap().contains("expression `Chain`"),
        "{leaf}"
    );
    let err = s
        .execute("model.remove_unused", &json!({"objects": ["Leaf"]}))
        .unwrap_err();
    assert!(err.contains("Chain"), "{err}");

    // keep_metadata: false, perspectives, translations and the Q&A schema keep nothing
    let strict = run(&mut s, "model.unused", json!({"keep_metadata": false}));
    let measures = names(&strict["unused"]["measures"]);
    assert!(measures.contains(&"Sales[Persp]".to_string()), "{measures:?}");
    assert!(measures.contains(&"Sales[Transl]".to_string()), "{measures:?}");
    assert!(names(&strict["unused"]["columns"]).contains(&"Date[Talked]".to_string()));
    let before = s.model.clone();
    let done = run(
        &mut s,
        "model.remove_unused",
        json!({"objects": ["[Persp]", "[Transl]", "Chain", "Leaf", "Orphaned"], "keep_metadata": false}),
    );
    assert_eq!(
        done["removed"]["expressions"],
        json!(["Chain", "Leaf", "Orphaned"])
    );
    assert_eq!(done["count"], 5);
    assert!(done["perspective_entries_dropped"].is_array(), "{done}");
    assert!(done["translations_dropped"].is_array(), "{done}");
    let m = s.model.as_ref().unwrap();
    assert!(
        m.expressions.iter().all(|e| e.name != "Leaf") && m.expressions.iter().any(|e| e.name == "Server")
    );
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.model, before);
}

#[test]
fn unparsable_expressions_keep_what_they_name() {
    let mut s = session();
    let m = s.model.as_mut().unwrap();
    let t = m.table_mut("Sales").unwrap();
    t.measures
        .iter_mut()
        .find(|x| x.name == "Margin %")
        .unwrap()
        .expression = "DIVIDE([Orphan], ((".into();
    let v = run(&mut s, "model.unused", json!({}));
    assert!(
        !names(&v["unused"]["measures"]).contains(&"Sales[Orphan]".to_string()),
        "{v}"
    );
    assert!(
        v["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("do not parse"))
    );
    let u = run(&mut s, "model.usage", json!({"object": "[Orphan]"}));
    assert_eq!(u["direct"][0]["matched_by_name"], true);
}

#[test]
fn multi_report_union() {
    let dir = TempDir::new("engine-usage-multi");
    // a second report bound to the same model uses [Orphan]
    let mut other = session();
    other.report = None;
    run(&mut other, "report.new", json!({}));
    run(
        &mut other,
        "visual.add",
        json!({"page": "Page 1", "visual_type": "card", "x": 0, "y": 0, "width": 200, "height": 100,
               "fields": [{"role": "Values", "field": "[Orphan]"}]}),
    );
    let path = dir.path().join("Thin.pbip");
    run(
        &mut other,
        "file.save_as",
        json!({"path": path.to_str().unwrap()}),
    );

    let mut s = session();
    let alone = run(&mut s, "model.unused", json!({}));
    assert!(names(&alone["unused"]["measures"]).contains(&"Sales[Orphan]".to_string()));
    let report_dir = dir.path().join("Thin.Report");
    let both = run(
        &mut s,
        "model.unused",
        json!({"reports": [report_dir.to_str().unwrap()]}),
    );
    assert_eq!(names(&both["unused"]["measures"]), ["Sales[Old]", "Sales[Older]"]);
    assert_eq!(both["reports"].as_array().unwrap().len(), 2);
    assert_eq!(both["reports"][1]["report"], "Thin.Report");
    assert!(both["warnings"][0].as_str().unwrap().contains("2 report(s)"));
    let u = run(
        &mut s,
        "model.usage",
        json!({"object": "[Orphan]", "reports": [path.to_str().unwrap()]}),
    );
    assert_eq!(u["direct"][0]["report"], "Thin.pbip");
    assert_eq!(u["direct"][0]["kind"], "visual");
    // a path that is no report is an error, not a silent skip
    let err = s
        .execute(
            "model.unused",
            &json!({"reports": [dir.path().join("missing.pbix").to_str().unwrap()]}),
        )
        .unwrap_err();
    assert!(err.contains("missing.pbix"), "{err}");

    // no report at all: everything a report would use is listed, with a warning
    s.report = None;
    let none = run(&mut s, "model.unused", json!({}));
    assert!(none["warnings"][0].as_str().unwrap().contains("no report"));
    assert!(names(&none["unused"]["measures"]).contains(&"Sales[Margin %]".to_string()));
}

#[test]
fn dry_run_remove_and_undo() {
    let mut s = session();
    let before = s.model.clone();
    let dirty = s.dirty;
    let dry = run(&mut s, "model.remove_unused", json!({"dry_run": true}));
    assert_eq!(dry["dry_run"], true);
    assert_eq!(dry["count"], 6);
    assert_eq!(s.model, before, "dry_run changed the model");
    assert_eq!(s.dirty, dirty);

    let done = run(&mut s, "model.remove_unused", json!({}));
    assert_eq!(done["count"], 6);
    assert_eq!(
        names(&done["removed"]["measures"]),
        ["Sales[Old]", "Sales[Older]", "Sales[Orphan]"]
    );
    assert_eq!(done["broken"], json!([]));
    let m = s.model.as_ref().unwrap();
    assert!(m.measure("Old").is_none() && m.measure("Total").is_some());
    assert!(
        m.table("Sales")
            .unwrap()
            .columns
            .iter()
            .all(|c| c.name != "Spare")
    );
    let after = run(&mut s, "model.unused", json!({}));
    assert_eq!(after["counts"]["measures"], 0);
    // Product[SortOrder] only sorted the removed Product[Name]: now it is unused too
    assert_eq!(names(&after["unused"]["columns"]), ["Product[SortOrder]"]);

    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.model, before, "one undo step restores everything");
}

#[test]
fn remove_subset_keeps_safety() {
    let mut s = session();
    let before = s.model.clone();
    // [Older] (unused, kept) uses [Old]: removing only [Old] would break it
    let err = s
        .execute("model.remove_unused", &json!({"objects": ["[Old]"]}))
        .unwrap_err();
    assert!(err.contains("break") && err.contains("Older"), "{err}");
    assert_eq!(s.model, before);
    // a used object is refused by name
    let err = s
        .execute(
            "model.remove_unused",
            &json!({"objects": ["[Orphan]", "[Total]"]}),
        )
        .unwrap_err();
    assert!(err.contains("[Total]"), "{err}");
    assert_eq!(s.model, before);
    // the preview shows what would break
    let dry = run(
        &mut s,
        "model.remove_unused",
        json!({"objects": ["[Old]"], "dry_run": true}),
    );
    assert_eq!(dry["broken"].as_array().unwrap().len(), 1);
    assert_eq!(s.model, before);
    let ok = run(
        &mut s,
        "model.remove_unused",
        json!({"objects": ["[Old]", "[Older]"]}),
    );
    assert_eq!(ok["count"], 2);
    // an allow-listed object is not unused
    let err = s
        .execute(
            "model.remove_unused",
            &json!({"objects": ["[Orphan]"], "allow": ["[Orphan]"]}),
        )
        .unwrap_err();
    assert!(err.contains("not unused"), "{err}");
}

#[test]
fn remove_unused_calculated_table_and_its_members() {
    let mut s = session();
    run(
        &mut s,
        "table.add",
        json!({"name": "Scratch", "partition": {"source_type": "calculated", "expression": "ROW(\"A\", 1)"}}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "Scratch", "name": "A", "expression": "1", "data_type": "int64"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Scratch", "name": "InScratch", "expression": "[Orphan]"}),
    );
    let v = run(&mut s, "model.unused", json!({}));
    assert_eq!(names(&v["unused"]["tables"]), ["Scratch"]);
    assert!(!names(&v["unused"]["measures"]).contains(&"Scratch[InScratch]".to_string()));
    let done = run(
        &mut s,
        "model.remove_unused",
        json!({"objects": ["'Scratch'", "Scratch[InScratch]", "[Orphan]"]}),
    );
    assert_eq!(done["removed"]["tables"], json!(["Scratch"]));
    assert!(s.model.as_ref().unwrap().table("Scratch").is_none());
}

#[test]
fn deterministic_output() {
    let mut a = session();
    let mut b = session();
    for (id, p) in [
        ("model.unused", json!({})),
        ("model.usage", json!({})),
        ("model.usage", json!({"object": "Sales[Amount]"})),
        ("model.remove_unused", json!({"dry_run": true})),
    ] {
        let x = run(&mut a, id, p.clone());
        let y = run(&mut b, id, p.clone());
        let again = run(&mut a, id, p.clone());
        assert_eq!(x.to_string(), y.to_string(), "{id}");
        assert_eq!(x.to_string(), again.to_string(), "{id}");
    }
}

#[test]
fn hostile_inputs_never_crash() {
    let mut s = session();
    let r = s.report.as_mut().unwrap();
    // deep nesting, aliases that point nowhere, wrong types
    let mut deep = json!(1);
    for _ in 0..120 {
        deep = json!({"x": [deep]});
    }
    r.extra = obj(
        &json!({"deep": deep, "bad": {"Column": {"Expression": {"SourceRef": {"Source": "nope"}}, "Property": 5}},
                         "level": {"HierarchyLevel": {"Expression": 3}}}),
    );
    let m = s.model.as_mut().unwrap();
    m.extra.insert("perspectives".into(), json!([{"tables": "x"}, 3]));
    m.extra.insert(
        "cultures".into(),
        json!([{"name": "x", "linguisticMetadata": {"content": "{not json"}}]),
    );
    m.table_mut("Sales")
        .unwrap()
        .extra
        .insert("hierarchies".into(), json!([{"levels": 3}]));
    run(&mut s, "model.usage", json!({}));
    run(&mut s, "model.unused", json!({"allow": ["**?*"]}));
    assert!(s.execute("model.usage", &json!({"object": 3})).is_err());
    assert!(s.execute("model.unused", &json!({"reports": 3})).is_err());
    assert!(s.execute("model.unused", &json!({"bogus": 1})).is_err());
}

/// Generated large model: 5,000 measures in chains, 500 columns, 200 visuals. Run with
/// `cargo test -p bicraft-engine --release usage_bench -- --ignored --nocapture`.
#[test]
#[ignore = "benchmark"]
fn usage_bench() {
    let (s, _) = large(5_000, 200);
    let mut s = s;
    for (id, p) in [
        ("model.unused", json!({})),
        ("model.usage", json!({"object": "[M0]"})),
        ("model.usage", json!({"graph": false})),
    ] {
        let mut times = Vec::new();
        for _ in 0..5 {
            let t = std::time::Instant::now();
            let v = run(&mut s, id, p.clone());
            times.push(t.elapsed().as_secs_f64() * 1000.0);
            if id == "model.unused" {
                assert!(v["counts"]["measures"].as_u64().unwrap() > 0);
            }
        }
        times.sort_by(f64::total_cmp);
        println!("{id} {p}: median {:.1} ms, worst {:.1} ms", times[2], times[4]);
    }
    let t = std::time::Instant::now();
    let v = run(&mut s, "model.remove_unused", json!({"dry_run": true}));
    println!(
        "model.remove_unused dry_run ({} objects): {:.1} ms",
        v["count"],
        t.elapsed().as_secs_f64() * 1000.0
    );
}

#[test]
fn large_model_counts() {
    let (mut s, expected_unused) = large(300, 20);
    let v = run(&mut s, "model.unused", json!({}));
    assert_eq!(v["counts"]["measures"], expected_unused);
}

/// `measures` measures in chains of 10 ([M(k+1)] uses [M(k)]); the visuals show the head of every
/// other chain, so the other chains are unused. Returns the session and the unused measure count.
fn large(measures: usize, visuals: usize) -> (Session, usize) {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Big"}));
    let mut m = s.model.take().unwrap();
    let mut t = bicraft_model::Table {
        name: "Fact".into(),
        ..Default::default()
    };
    for c in 0..500 {
        t.columns.push(bicraft_model::Column {
            name: format!("C{c}"),
            data_type: bicraft_model::DataType::Double,
            ..Default::default()
        });
    }
    for i in 0..measures {
        let expression = if i % 10 == 0 {
            format!(
                "SUM(Fact[C{}]) + CALCULATE(SUM(Fact[C{}]), Fact[C{}] > 0)",
                i % 500,
                (i + 1) % 500,
                (i + 2) % 500
            )
        } else {
            format!("[M{}] * 1.1 + DIVIDE([M{}], 2)", i - 1, i - 1)
        };
        t.measures.push(bicraft_model::Measure {
            name: format!("M{i}"),
            expression,
            ..Default::default()
        });
    }
    m.tables.push(t);
    s.model = Some(m);
    let chains = measures / 10;
    let mut pages = Vec::new();
    let mut used_chains = std::collections::HashSet::new();
    for p in 0..10 {
        let mut page = Page {
            name: format!("p{p}"),
            display_name: format!("Page {p}"),
            ..Default::default()
        };
        for v in 0..visuals / 10 {
            let chain = ((p * visuals / 10 + v) * 2) % chains.max(1);
            used_chains.insert(chain);
            let head = chain * 10 + 9;
            page.visuals.push(Visual {
                name: format!("v{p}_{v}"),
                visual_type: Some("card".into()),
                fields: vec![FieldBinding {
                    display_name: None,
                    show_as: None,
                    role: "Values".into(),
                    field: FieldRef::Measure {
                        table: "Fact".into(),
                        measure: format!("M{head}"),
                    },
                }],
                filter_config: Some(json!({"filters": [{"field": col("Fact", &format!("C{}", v % 500))}]})),
                ..Default::default()
            });
        }
        pages.push(page);
    }
    s.report = Some(Report {
        pages,
        ..Default::default()
    });
    let unused = (chains - used_chains.len()) * 10;
    (s, unused)
}

#[test]
fn calculated_tables_over_whole_tables_keep_their_columns() {
    let mut s = session();
    run(
        &mut s,
        "table.add",
        json!({"name": "Copy", "partition": {"source_type": "calculated", "expression": "FILTER(Product, TRUE())"}}),
    );
    run(
        &mut s,
        "table.add",
        json!({"name": "One", "partition": {"source_type": "calculated", "expression": "ROW(\"x\", SUM(Sales[Spare]))"}}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Copy", "name": "CopyRows", "expression": "COUNTROWS(Copy) + COUNTROWS(One)"}),
    );
    s.report.as_mut().unwrap().pages[0].visuals[0]
        .fields
        .push(FieldBinding {
            display_name: None,
            show_as: None,
            role: "Values".into(),
            field: FieldRef::Measure {
                table: "Copy".into(),
                measure: "CopyRows".into(),
            },
        });
    let v = run(&mut s, "model.unused", json!({}));
    let cols = names(&v["unused"]["columns"]);
    // FILTER(Product, ...) returns every column of Product: all are kept
    assert!(!cols.contains(&"Product[Name]".to_string()), "{cols:?}");
    // Sales is only a qualifier in ROW(...): its other columns stay unused
    assert!(cols.contains(&"Sales[Date]".to_string()), "{cols:?}");
    assert!(!cols.contains(&"Sales[Spare]".to_string()), "{cols:?}");
    let u = run(&mut s, "model.usage", json!({"object": "[Total]"}));
    assert_eq!(names(&u["depends_on"]), ["Sales[Amount]"]);
}
