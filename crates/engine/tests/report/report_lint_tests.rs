//! The report rules of `model.analyze` (`report_lint.rs`): references a synthetic report makes
//! to objects renamed or removed in its model, each with its place, severity and, where an
//! existing command removes it, the fix (which is run and checked).

use bicraft_report::{FieldBinding, FieldRef, NamedDocument, Page, Position, Report, Visual};
use serde_json::{Value, json};

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

fn filter(name: &str, field: Value) -> Value {
    json!({"name": name, "field": field, "type": "Categorical"})
}

fn binding(role: &str, field: FieldRef) -> FieldBinding {
    FieldBinding {
        role: role.into(),
        field,
        display_name: None,
        show_as: None,
    }
}

fn column(t: &str, c: &str) -> FieldRef {
    FieldRef::Column {
        table: t.into(),
        column: c.into(),
    }
}

fn measure(t: &str, m: &str) -> FieldRef {
    FieldRef::Measure {
        table: t.into(),
        measure: m.into(),
    }
}

const SIZE: Position = Position {
    x: 0.0,
    y: 0.0,
    z: 0.0,
    width: 300.0,
    height: 200.0,
    tab_order: None,
};

/// Sales(Region, Amount, Cost) with [Total] and [Margin]; a chart, a card and a table on page
/// "Overview", a drillthrough page "Details", report, page and visual filters, a bookmark.
fn session() -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        ("table.add", json!({"name": "Sales"})),
        (
            "column.add",
            json!({"table": "Sales", "name": "Region", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Cost", "data_type": "double"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Margin", "expression": "[Total] - SUM(Sales[Cost])"}),
        ),
    ] {
        run(&mut s, id, p);
    }
    let mut chart = Visual {
        name: "chart".into(),
        position: SIZE,
        visual_type: Some("clusteredColumnChart".into()),
        fields: vec![
            binding("Category", column("Sales", "Region")),
            binding("Y", measure("Sales", "Total")),
        ],
        filter_config: Some(json!({"filters": [filter("f1", col("Sales", "Cost"))]})),
        ..Default::default()
    };
    chart.visual_extra.insert(
        "objects".into(),
        json!({"dataPoint": [{"properties": {"fill": {"solid": {"color": {"expr": {"Measure": {
            "Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Margin"}}}}}}}]}),
    );
    let card = Visual {
        name: "card".into(),
        position: SIZE,
        visual_type: Some("card".into()),
        fields: vec![binding("Values", measure("Sales", "Margin"))],
        ..Default::default()
    };
    let details = Page {
        name: "p2".into(),
        display_name: "Details".into(),
        width: 1280.0,
        height: 720.0,
        filter_config: Some(json!({"filters": [{"name": "d1", "field": col("Sales", "Region"),
            "type": "Categorical", "howCreated": "Drillthrough"}]})),
        ..Default::default()
    };
    s.report = Some(Report {
        filter_config: Some(json!({"filters": [filter("r1", col("Sales", "Region"))]})),
        pages: vec![
            Page {
                name: "p1".into(),
                display_name: "Overview".into(),
                width: 1280.0,
                height: 720.0,
                filter_config: Some(json!({"filters": [filter("pf", col("Sales", "Cost"))]})),
                visuals: vec![chart, card],
                ..Default::default()
            },
            details,
        ],
        bookmarks: vec![NamedDocument {
            name: "Bookmark1".into(),
            document: json!({"name": "Bookmark1", "displayName": "Bookmark 1", "explorationState": {"version": "1.0.0", "activeSection": "p1", "sections": {"p1": {"visualContainers": {},
                "filters": {"byExpr": [{"name": "b1", "expression": meas("Sales", "Margin")}]}}}}}),
        }],
        ..Default::default()
    });
    s
}

fn report_findings(v: &Value) -> Vec<Value> {
    v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["category"] == "Report references")
        .cloned()
        .collect()
}

fn analyze(s: &mut Session) -> Vec<Value> {
    let v = run(s, "model.analyze", json!({"default_rules": false}));
    report_findings(&v)
}

#[test]
fn a_clean_report_has_no_report_finding_and_every_rule_is_listed() {
    let mut s = session();
    let v = run(&mut s, "model.analyze", json!({"default_rules": false}));
    assert!(report_findings(&v).is_empty(), "{:#}", v["findings"]);
    let rules: Vec<&str> = v["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["rule_id"].as_str().unwrap())
        .collect();
    assert_eq!(rules.len(), 10, "{rules:?}");
    assert!(rules.contains(&"REPORT_BROKEN_FILTER"));
    assert!(
        v["rules"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["status"] == "evaluated")
    );
    // off on request; a model alone has none of them
    let v = run(
        &mut s,
        "model.analyze",
        json!({"default_rules": false, "report_rules": false}),
    );
    assert_eq!(v["summary"]["rules"], 0);
    s.report = None;
    let v = run(&mut s, "model.analyze", json!({"default_rules": false}));
    assert_eq!(v["summary"]["rules"], 0);
}

#[test]
fn a_removed_measure_is_found_in_every_place_with_its_location() {
    let mut s = session();
    // removed without the report following: the card, the chart's formatting and the bookmark
    // still name it
    run(&mut s, "measure.remove", json!({"name": "Margin", "force": true}));
    let f = analyze(&mut s);
    let by_rule = |id: &str| f.iter().filter(|x| x["rule_id"] == id).collect::<Vec<_>>();
    let visual = by_rule("REPORT_BROKEN_VISUAL_FIELD");
    assert_eq!(visual.len(), 1, "{f:#?}");
    let card = visual[0];
    assert_eq!(card["severity"], "error");
    assert_eq!(card["object"], "'Sales'[Margin]");
    assert_eq!(
        card["location"],
        json!({"kind": "visual", "page": "Overview", "page_name": "p1", "visual": "card",
               "visual_type": "card", "role": "Values"})
    );
    assert!(
        card["message"].as_str().unwrap().contains("visual \"card\""),
        "{card}"
    );
    assert_eq!(
        card["fix"],
        json!({"command": "visual.unbind", "params": {"page": "p1", "visual": "card", "role": "Values", "index": 0}})
    );
    let fmt = by_rule("REPORT_BROKEN_FORMATTING");
    assert_eq!(fmt.len(), 1, "{f:#?}");
    assert_eq!(fmt[0]["location"]["visual"], "chart");
    assert_eq!(
        fmt[0]["fix"],
        json!({"command": "report.remove_reference", "params": {"field": "'Sales'[Margin]",
               "place": "formatting", "page": "p1", "visual": "chart"}})
    );
    let bm = by_rule("REPORT_BROKEN_BOOKMARK");
    assert_eq!(bm.len(), 1, "{f:#?}");
    assert_eq!(bm[0]["severity"], "warning");
    assert_eq!(bm[0]["location"]["bookmark"], "Bookmark1");
    assert_eq!(
        bm[0]["fix"],
        json!({"command": "report.remove_reference", "params": {"field": "'Sales'[Margin]",
               "place": "bookmark", "bookmark": "Bookmark1"}})
    );
    // the fix runs and the finding goes
    let fix = card["fix"].clone();
    run(&mut s, fix["command"].as_str().unwrap(), fix["params"].clone());
    let f = analyze(&mut s);
    assert!(
        !f.iter().any(|x| x["rule_id"] == "REPORT_BROKEN_VISUAL_FIELD"),
        "{f:#?}"
    );
    // the formatting fix drops the fill rule (and its entry, left empty), the bookmark fix its
    // filter; one undo step brings a fix back
    let fmt = f
        .iter()
        .find(|x| x["rule_id"] == "REPORT_BROKEN_FORMATTING")
        .unwrap();
    run_fix(&mut s, fmt);
    let chart = &s.report.as_ref().unwrap().pages[0].visuals[0];
    assert!(
        chart.visual_extra.get("objects").is_none(),
        "{:?}",
        chart.visual_extra
    );
    run(&mut s, "edit.undo", json!({}));
    assert!(
        analyze(&mut s)
            .iter()
            .any(|x| x["rule_id"] == "REPORT_BROKEN_FORMATTING")
    );
    let rest = fix_all(&mut s);
    assert!(rest.is_empty(), "{rest:#?}");
    let doc = &s.report.as_ref().unwrap().bookmarks[0].document;
    assert_eq!(
        doc["explorationState"]["sections"]["p1"]["filters"]["byExpr"],
        json!([])
    );
    assert_valid(&mut s);
    // asked again, the commands refuse and change nothing
    for p in [
        json!({"field": "'Sales'[Margin]", "place": "formatting", "page": "p1", "visual": "chart"}),
        json!({"field": "'Sales'[Margin]", "place": "bookmark", "bookmark": "Bookmark1"}),
        json!({"field": "'Sales'[Margin]", "place": "drillthrough", "page": "Details"}),
        json!({"field": "'Sales'[Margin]", "place": "elsewhere"}),
    ] {
        assert!(s.execute("report.remove_reference", &p).is_err(), "{p}");
    }
}

/// A PBIR bookmark's visual filter card, as `bookmark.capture` writes it: keyed by name under
/// `filters.byName`, its field named through the query's `From` alias (found in Desktop checks,
/// 2026-11-23: the bookmark fix said "nothing at that place refers").
#[test]
fn a_bookmark_filter_card_by_name_is_removed() {
    let mut s = session();
    if let Some(r) = s.report.as_mut() {
        r.bookmarks[0].document["explorationState"]["sections"]["p1"]["visualContainers"] = json!({
            "chart": {"filters": {"byName": {"vf": {"name": "vf", "type": "Advanced", "filter": {
                "Version": 2, "From": [{"Name": "s", "Entity": "Sales", "Type": 0}],
                "Where": [{"Condition": {"Comparison": {"ComparisonKind": 1,
                    "Left": {"Measure": {"Expression": {"SourceRef": {"Source": "s"}}, "Property": "Margin"}},
                    "Right": {"Literal": {"Value": "0L"}}}}}]}}}}}});
    }
    run(&mut s, "measure.remove", json!({"name": "Margin", "force": true}));
    let v = run(
        &mut s,
        "report.remove_reference",
        json!({"field": "'Sales'[Margin]", "place": "bookmark", "bookmark": "Bookmark1"}),
    );
    assert_eq!(v["removed"], 2, "{v}");
    let doc = &s.report.as_ref().unwrap().bookmarks[0].document;
    assert_eq!(
        doc["explorationState"]["sections"]["p1"]["visualContainers"]["chart"]["filters"]["byName"],
        json!({})
    );
    assert!(
        !analyze(&mut s)
            .iter()
            .any(|x| x["rule_id"] == "REPORT_BROKEN_BOOKMARK")
    );
}

fn run_fix(s: &mut Session, finding: &Value) {
    let fix = &finding["fix"];
    assert!(fix.is_object(), "no fix: {finding:#}");
    run(s, fix["command"].as_str().unwrap(), fix["params"].clone());
}

/// Runs the first offered fix until none is left (each must remove its finding); returns what
/// remains.
fn fix_all(s: &mut Session) -> Vec<Value> {
    for _ in 0..32 {
        let f = analyze(s);
        let Some(x) = f.iter().find(|x| x.get("fix").is_some()) else {
            return f;
        };
        let before = f.len();
        run_fix(s, x);
        let after = analyze(s).len();
        assert!(after < before, "{x:#} left {after} of {before} findings");
    }
    panic!("fixes do not converge: {:#?}", analyze(s));
}

fn assert_valid(s: &mut Session) {
    let v = run(s, "report.validate", json!({}));
    assert_eq!(v["error_count"], 0, "{v:#}");
}

#[test]
fn a_renamed_column_breaks_filters_drillthrough_and_the_axis_with_a_suggestion() {
    let mut s = session();
    // renamed in the model only (update_report defaults to false)
    run(
        &mut s,
        "column.rename",
        json!({"table": "Sales", "name": "Region", "new_name": "Regions"}),
    );
    run(
        &mut s,
        "column.rename",
        json!({"table": "Sales", "name": "Cost", "new_name": "Unit Cost"}),
    );
    let f = analyze(&mut s);
    let kinds: Vec<(String, String)> = f
        .iter()
        .map(|x| {
            (
                x["rule_id"].as_str().unwrap().to_string(),
                x["location"]["kind"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    for want in [
        ("REPORT_BROKEN_VISUAL_FIELD", "visual"),
        ("REPORT_BROKEN_FILTER", "visual filter"),
        ("REPORT_BROKEN_FILTER", "page filter"),
        ("REPORT_BROKEN_FILTER", "report filter"),
        ("REPORT_BROKEN_DRILLTHROUGH", "drillthrough"),
    ] {
        assert!(
            kinds.contains(&(want.0.to_string(), want.1.to_string())),
            "{want:?} not in {kinds:?}"
        );
    }
    let axis = f.iter().find(|x| x["location"]["kind"] == "visual").unwrap();
    assert_eq!(axis["suggestion"], "'Sales'[Regions]");
    assert!(
        axis["message"]
            .as_str()
            .unwrap()
            .contains("did you mean 'Sales'[Regions]?")
    );
    let cost = f.iter().find(|x| x["object"] == "'Sales'[Cost]").unwrap();
    assert_eq!(cost["suggestion"], "'Sales'[Unit Cost]");
    let drill = f
        .iter()
        .find(|x| x["location"]["kind"] == "drillthrough")
        .unwrap();
    assert_eq!(drill["location"]["page"], "Details");
    assert_eq!(drill["object_type"], "Page");
    // a near field: every fix re-binds to it
    assert_eq!(
        drill["fix"],
        json!({"command": "report.rebind", "params": {"from": "'Sales'[Region]", "to": "'Sales'[Regions]"}})
    );
    assert!(f.iter().all(|x| x["fix"]["command"] == "report.rebind"), "{f:#?}");
    // one re-bind fixes every reference to the field, and is one undo step
    run_fix(&mut s, drill);
    let after = analyze(&mut s);
    assert!(after.iter().all(|x| x["object"] == "'Sales'[Cost]"), "{after:#?}");
    let r = s.report.as_ref().unwrap();
    assert_eq!(r.pages[0].visuals[0].fields[0].field, column("Sales", "Regions"));
    assert_eq!(
        r.pages[1].filter_config.as_ref().unwrap()["filters"][0]["field"],
        col("Sales", "Regions")
    );
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(analyze(&mut s).len(), f.len());
    let rest = fix_all(&mut s);
    assert!(rest.is_empty(), "{rest:#?}");
    assert_valid(&mut s);
    // the field is now in the model: refused; so is a target of another kind or not in the model
    for p in [
        json!({"from": "Sales[Regions]", "to": "Sales[Amount]"}),
        json!({"from": "Sales[Nothing]", "to": "Sales[Amount]"}),
        json!({"from": "Sales[Region]", "to": "Sales[Nope]"}),
        json!({"from": "[Region]", "to": "Sales[Amount]"}),
        json!({"from": "Sales[Region]", "to": "Sum(Sales[Amount])"}),
    ] {
        assert!(s.execute("report.rebind", &p).is_err(), "{p}");
    }
}

#[test]
fn a_drillthrough_field_without_a_near_name_is_removed_from_the_page() {
    let mut s = session();
    // a drillthrough page as page.set_type writes it: filter cards and binding parameters
    run(
        &mut s,
        "page.set_type",
        json!({"page": "Details", "type": "drillthrough", "fields": ["Sales[Region]", "Sales[Cost]"]}),
    );
    run(
        &mut s,
        "column.rename",
        json!({"table": "Sales", "name": "Region", "new_name": "Zone"}),
    );
    let f = analyze(&mut s);
    let drill: Vec<&Value> = f
        .iter()
        .filter(|x| x["rule_id"] == "REPORT_BROKEN_DRILLTHROUGH")
        .collect();
    assert!(!drill.is_empty(), "{f:#?}");
    assert!(drill.iter().all(|x| x.get("suggestion").is_none()), "{drill:#?}");
    assert_eq!(
        drill[0]["fix"],
        json!({"command": "report.remove_reference", "params": {"field": "'Sales'[Region]",
               "place": "drillthrough", "page": "p2"}})
    );
    run_fix(&mut s, drill[0]);
    let page = &s.report.as_ref().unwrap().pages[1];
    let binding = &page.extra["pageBinding"];
    assert_eq!(binding["type"], "Drillthrough");
    assert_eq!(binding["parameters"].as_array().unwrap().len(), 1, "{binding:#}");
    let filters = page.filter_config.as_ref().unwrap()["filters"]
        .as_array()
        .unwrap();
    assert_eq!(filters.len(), 1);
    assert_eq!(filters[0]["field"], col("Sales", "Cost"));
    assert!(
        !analyze(&mut s)
            .iter()
            .any(|x| x["rule_id"] == "REPORT_BROKEN_DRILLTHROUGH")
    );
    let rest = fix_all(&mut s);
    assert!(rest.is_empty(), "{rest:#?}");
    assert_valid(&mut s);
}

#[test]
fn a_removed_table_is_named_as_such() {
    let mut s = session();
    if let Some(r) = s.report.as_mut() {
        r.pages[0].visuals[1].fields = vec![binding("Values", measure("Budget", "Plan"))];
    }
    let f = analyze(&mut s);
    assert_eq!(f.len(), 1, "{f:#?}");
    assert!(
        f[0]["message"]
            .as_str()
            .unwrap()
            .contains("the model has no table 'Budget'"),
        "{}",
        f[0]
    );
}

#[test]
fn stale_query_refs_selectors_and_interactions() {
    let mut s = session();
    if let Some(r) = s.report.as_mut() {
        let chart = &mut r.pages[0].visuals[0];
        chart.visual_extra.insert(
            "query".into(),
            json!({"queryState": {
                "Category": {"projections": [{"field": col("Sales", "Region"), "queryRef": "Sales.Region"}]},
                "Y": {"projections": [{"field": meas("Sales", "Total"), "queryRef": "Sales.Old Total"}]}}}),
        );
        chart.visual_extra.insert(
            "objects".into(),
            json!({"dataPoint": [{"properties": {"fill": {"solid": {"color": {"expr": {"Literal": {"Value": "'#FF0000'"}}}}}},
                                  "selector": {"metadata": "Sales.Old Total"}},
                                 {"properties": {"fill": {"solid": {"color": {"expr": {"Literal": {"Value": "'#00FF00'"}}}}}},
                                  "selector": {"metadata": "Sales.Totl"}}]}),
        );
        r.pages[0].extra.insert(
            "visualInteractions".into(),
            json!([{"source": "chart", "target": "gone", "type": "NoFilter"},
                   {"source": "chart", "target": "card", "type": "NoFilter"}]),
        );
    }
    let f = analyze(&mut s);
    let stale: Vec<&Value> = f
        .iter()
        .filter(|x| x["rule_id"] == "REPORT_STALE_QUERYREF")
        .collect();
    assert_eq!(stale.len(), 1, "{f:#?}");
    assert_eq!(stale[0]["location"]["query_ref"], "Sales.Old Total");
    assert_eq!(stale[0]["suggestion"], "Sales.Total");
    assert_eq!(stale[0]["location"]["role"], "Y");
    let sel: Vec<&Value> = f
        .iter()
        .filter(|x| x["rule_id"] == "REPORT_STALE_SELECTOR")
        .collect();
    assert_eq!(sel.len(), 1, "{f:#?}");
    assert_eq!(sel[0]["object"], "Sales.Totl");
    assert_eq!(sel[0]["location"]["object"], "dataPoint");
    let dangling: Vec<&Value> = f
        .iter()
        .filter(|x| x["rule_id"] == "REPORT_DANGLING_INTERACTION")
        .collect();
    assert_eq!(dangling.len(), 1, "{f:#?}");
    assert_eq!(dangling[0]["object"], "gone");
    assert_eq!(dangling[0]["location"]["end"], "target");
    // the queryRef fix renames the projection and the selector keyed to it
    assert_eq!(
        stale[0]["fix"],
        json!({"command": "visual.fix_query_refs", "params": {"page": "p1", "visual": "chart"}})
    );
    run_fix(&mut s, stale[0]);
    let chart = &s.report.as_ref().unwrap().pages[0].visuals[0];
    assert_eq!(
        chart.visual_extra["query"]["queryState"]["Y"]["projections"][0]["queryRef"],
        "Sales.Total"
    );
    assert_eq!(
        chart.visual_extra["objects"]["dataPoint"][0]["selector"]["metadata"],
        "Sales.Total"
    );
    let f = analyze(&mut s);
    assert!(
        !f.iter().any(|x| x["rule_id"] == "REPORT_STALE_QUERYREF"),
        "{f:#?}"
    );
    assert_valid(&mut s);
    assert!(
        s.execute("visual.fix_query_refs", &json!({"page": "p1", "visual": "chart"}))
            .is_err()
    );
}

#[test]
fn a_stale_query_ref_whose_own_name_is_taken_has_no_fix() {
    let mut s = session();
    if let Some(r) = s.report.as_mut() {
        r.pages[0].visuals[0].visual_extra.insert(
            "query".into(),
            json!({"queryState": {
                "Category": {"projections": [{"field": col("Sales", "Region"), "queryRef": "Sales.Total"}]},
                "Y": {"projections": [{"field": meas("Sales", "Total"), "queryRef": "Sales.Region"}]}}}),
        );
    }
    let f = analyze(&mut s);
    let stale: Vec<&Value> = f
        .iter()
        .filter(|x| x["rule_id"] == "REPORT_STALE_QUERYREF")
        .collect();
    assert_eq!(stale.len(), 2, "{f:#?}");
    assert!(stale.iter().all(|x| x.get("fix").is_none()), "{stale:#?}");
}

#[test]
fn a_field_parameter_listing_a_removed_measure() {
    let mut s = session();
    run(
        &mut s,
        "parameter.fields",
        json!({"name": "Metric", "fields": ["[Total]", "[Margin]"], "add_slicer": false}),
    );
    assert!(analyze(&mut s).is_empty());
    run(&mut s, "measure.remove", json!({"name": "Margin", "force": true}));
    let f = analyze(&mut s);
    let fp: Vec<&Value> = f
        .iter()
        .filter(|x| x["rule_id"] == "REPORT_BROKEN_FIELD_PARAMETER")
        .collect();
    assert_eq!(fp.len(), 1, "{f:#?}");
    assert_eq!(fp[0]["location"]["field_parameter"], "Metric");
    assert_eq!(fp[0]["severity"], "error");
    // no near measure: the row goes (the card and bookmark are fixed too)
    let fp0 = fp[0].clone();
    assert_eq!(fp0["fix"]["command"], "parameter.fix_field");
    assert!(fp0["fix"]["params"].get("to").is_none(), "{fp0:#}");
    run_fix(&mut s, &fp0);
    let expr = metric_expression(&s);
    assert!(
        expr.contains("NAMEOF('Sales'[Total])") && !expr.contains("Margin"),
        "{expr}"
    );
    let rest = fix_all(&mut s);
    assert!(rest.is_empty(), "{rest:#?}");
    assert_valid(&mut s);
    // the only field left cannot go
    assert!(
        s.execute(
            "parameter.fix_field",
            &json!({"name": "Metric", "field": "'Sales'[Total]"})
        )
        .is_err()
    );
}

fn metric_expression(s: &Session) -> String {
    s.model.as_ref().unwrap().table("Metric").unwrap().partitions[0]
        .expression
        .clone()
        .unwrap()
}

#[test]
fn a_field_parameter_field_is_pointed_at_the_near_measure() {
    let mut s = session();
    run(
        &mut s,
        "parameter.fields",
        json!({"name": "Metric", "fields": ["[Total]", "[Margin]"], "add_slicer": false}),
    );
    run(&mut s, "measure.remove", json!({"name": "Margin", "force": true}));
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Margins", "expression": "[Total] - SUM(Sales[Cost])"}),
    );
    let f = analyze(&mut s);
    let fp = f
        .iter()
        .find(|x| x["rule_id"] == "REPORT_BROKEN_FIELD_PARAMETER")
        .unwrap();
    assert_eq!(fp["fix"]["params"]["to"], "'Sales'[Margins]", "{fp:#}");
    run_fix(&mut s, fp);
    let expr = metric_expression(&s);
    assert!(expr.contains("NAMEOF('Sales'[Margins])"), "{expr}");
    assert!(expr.contains("\"Margin\""), "the label stays: {expr}");
    // one undo step puts it back
    run(&mut s, "edit.undo", json!({}));
    assert!(metric_expression(&s).contains("[Margin])"));
    let rest = fix_all(&mut s);
    assert!(rest.is_empty(), "{rest:#?}");
    assert_valid(&mut s);
}

#[test]
fn filters_by_rule_id_severity_and_cap() {
    let mut s = session();
    run(&mut s, "measure.remove", json!({"name": "Margin", "force": true}));
    let v = run(
        &mut s,
        "model.analyze",
        json!({"default_rules": false, "rule_ids": ["report_broken_bookmark"]}),
    );
    assert_eq!(v["summary"]["rules"], 1);
    assert_eq!(report_findings(&v).len(), 1);
    let v = run(
        &mut s,
        "model.analyze",
        json!({"default_rules": false, "severity_min": "error"}),
    );
    assert!(report_findings(&v).iter().all(|x| x["severity"] == "error"));
    let v = run(
        &mut s,
        "model.analyze",
        json!({"default_rules": false, "max_findings": 1}),
    );
    assert_eq!(report_findings(&v).len(), 1);
    assert!(v["findings_omitted"].as_u64().unwrap() >= 2, "{v}");
    assert!(
        s.execute("model.analyze", &json!({"report_rules": "yes"}))
            .is_err()
    );
}
