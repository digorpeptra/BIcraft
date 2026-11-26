//! Tests of visual properties (`visual_props.rs`): title, sort, filters, formatting, the
//! `visual.*` property commands, extract → apply idempotence, and PBIR round trips.

use bicraft_report::{Report, Visual};
use serde_json::{Value, json};

use crate::Session;
use crate::visual_props::{apply_props, extract_props};

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Runs a command that must fail and checks the report and its dirty flag did not change.
fn fails(s: &mut Session, id: &str, params: Value) -> String {
    let before = s.report.clone();
    let dirty = s.report_dirty;
    let err = s
        .execute(id, &params)
        .expect_err(&format!("{id} {params} should fail"));
    assert_eq!(s.report, before, "{id} changed the report although it failed");
    assert_eq!(s.report_dirty, dirty);
    err
}

/// Sales(Amount double, Qty int64, Price decimal, Product, Date) with [Total], Product(Name,
/// Color), 'Date Table'(Date), and a blank report.
fn session() -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        ("table.add", json!({"name": "Sales"})),
        ("table.add", json!({"name": "Product"})),
        ("table.add", json!({"name": "Date Table"})),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Qty", "data_type": "int64"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Price", "data_type": "decimal"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Date", "data_type": "dateTime"}),
        ),
        (
            "column.add",
            json!({"table": "Product", "name": "Name", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Product", "name": "Color", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Date Table", "name": "Date", "data_type": "dateTime"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        ("report.new", json!({})),
    ] {
        run(&mut s, id, p);
    }
    s
}

/// Adds a column chart (Category = Product[Name], Y = [Total]); returns (page, visual).
fn chart(s: &mut Session) -> (String, String) {
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let v = run(
        s,
        "visual.add",
        json!({"page": page, "visual_type": "clusteredColumnChart", "x": 10, "y": 20, "width": 400,
               "height": 300, "fields": [{"role": "Category", "field": "Product[Name]"},
                                         {"role": "Y", "field": "[Total]"}]}),
    );
    (page, v["visual"].as_str().unwrap().to_string())
}

fn visual<'a>(s: &'a Session, page: &str, name: &str) -> &'a Visual {
    s.report
        .as_ref()
        .unwrap()
        .page(page)
        .unwrap()
        .visual(name)
        .unwrap()
}

fn filters(v: &Visual) -> Vec<Value> {
    v.filter_config
        .as_ref()
        .and_then(|c| c["filters"].as_array().cloned())
        .unwrap_or_default()
}

/// `apply_props(extract_props(v))` changes nothing, with and without the model.
fn assert_extract_noop(s: &Session, v: &Visual) {
    let props = extract_props(v);
    for model in [s.model.as_ref(), None] {
        let mut copy = v.clone();
        let changes = apply_props(&mut copy, &props, model).unwrap_or_else(|e| panic!("{props}: {e}"));
        assert!(
            changes.is_empty(),
            "extract -> apply changed {changes:?}\nprops: {props}"
        );
        assert_eq!(&copy, v);
    }
}

/// The PBIR documents of a report, re-read.
fn pbir_round_trip(r: &Report) -> Report {
    let docs = bicraft_pbir::write_documents(r);
    bicraft_pbir::read_documents(&docs).unwrap()
}

fn schema_issues(s: &mut Session) -> Value {
    let v = run(s, "report.validate", json!({}));
    assert_eq!(v["schema_issues"], json!([]), "{v}");
    v
}

// ---------------------------------------------------------------------------------------------
// Title
// ---------------------------------------------------------------------------------------------

#[test]
fn set_title_writes_desktop_literals_and_is_idempotent() {
    let mut s = session();
    let (page, v) = chart(&mut s);
    let out = run(
        &mut s,
        "visual.set_title",
        json!({"page": page, "visual": v, "title": "Sales by product"}),
    );
    assert_eq!(out["changed"], true);
    assert_eq!(out["props"]["title"], "Sales by product");
    let vis = visual(&s, &page, &v);
    assert_eq!(
        vis.visual_extra["visualContainerObjects"]["title"],
        json!([{"properties": {
            "text": {"expr": {"Literal": {"Value": "'Sales by product'"}}},
            "show": {"expr": {"Literal": {"Value": "true"}}},
        }}])
    );
    s.report_dirty = false;
    let again = run(
        &mut s,
        "visual.set_title",
        json!({"page": page, "visual": v, "title": "Sales by product"}),
    );
    assert_eq!(again["changed"], false);
    assert_eq!(again["changes"], json!([]));
    assert!(!s.report_dirty, "a no-op must not dirty the report");

    // Object form, quotes escaped, formatting.
    run(
        &mut s,
        "visual.set_title",
        json!({"page": page, "visual": v, "title": {"text": "Bob's sales", "font_size": 14,
               "color": "#1F77B4", "alignment": "Center", "bold": true}}),
    );
    let props = &visual(&s, &page, &v).visual_extra["visualContainerObjects"]["title"][0]["properties"];
    assert_eq!(props["text"]["expr"]["Literal"]["Value"], "'Bob''s sales'");
    assert_eq!(props["fontSize"]["expr"]["Literal"]["Value"], "14D");
    assert_eq!(
        props["fontColor"],
        json!({"solid": {"color": {"expr": {"Literal": {"Value": "'#1F77B4'"}}}}})
    );
    assert_eq!(props["alignment"]["expr"]["Literal"]["Value"], "'center'");
    let p = run(&mut s, "visual.props", json!({"page": page, "visual": v}));
    assert_eq!(
        p["props"]["title"],
        json!({"text": "Bob's sales", "show": true, "font_size": 14, "color": "#1F77B4",
               "alignment": "center", "bold": true})
    );
    // null removes one property, then the whole title.
    run(
        &mut s,
        "visual.set_title",
        json!({"page": page, "visual": v, "title": {"bold": null}}),
    );
    assert!(
        visual(&s, &page, &v).visual_extra["visualContainerObjects"]["title"][0]["properties"]
            .get("bold")
            .is_none()
    );
    let out = run(
        &mut s,
        "visual.set_title",
        json!({"page": page, "visual": v, "title": null}),
    );
    assert_eq!(out["changed"], true);
    assert!(
        !visual(&s, &page, &v)
            .visual_extra
            .contains_key("visualContainerObjects")
    );
    schema_issues(&mut s);
}

#[test]
fn bad_titles_are_rejected_without_changes() {
    let mut s = session();
    let (page, v) = chart(&mut s);
    for title in [
        json!(3),
        json!({"text": "x", "size": 3}),
        json!({"color": "blue"}),
        json!({"font_size": 0}),
        json!({"alignment": "middle"}),
        json!({"show": "yes"}),
        json!("x".repeat(5000)),
    ] {
        fails(
            &mut s,
            "visual.set_title",
            json!({"page": page, "visual": v, "title": title}),
        );
    }
    fails(&mut s, "visual.set_title", json!({"page": page, "visual": v}));
    fails(
        &mut s,
        "visual.set_title",
        json!({"page": page, "visual": "nope", "title": "x"}),
    );
    // flat properties beside a title of another shape, or twice
    fails(
        &mut s,
        "visual.set_title",
        json!({"page": page, "visual": v, "title": null, "bold": true}),
    );
    fails(
        &mut s,
        "visual.set_title",
        json!({"page": page, "visual": v, "title": {"bold": true}, "bold": false}),
    );
    fails(
        &mut s,
        "visual.set_title",
        json!({"page": page, "visual": v, "underline": "yes"}),
    );
}

/// The parameters form's shape (M11.22): the text as free text and the properties flat beside
/// it; the font family and underline of the title and the subtitle (Desktop's Format pane).
#[test]
fn set_title_takes_flat_properties_and_the_font() {
    let mut s = session();
    let (page, v) = chart(&mut s);
    run(
        &mut s,
        "visual.set_title",
        json!({"page": page, "visual": v, "title": "Sales", "font_size": 16, "underline": true,
               "font_family": "Segoe UI Semibold"}),
    );
    let props = &visual(&s, &page, &v).visual_extra["visualContainerObjects"]["title"][0]["properties"];
    assert_eq!(props["text"]["expr"]["Literal"]["Value"], "'Sales'");
    assert_eq!(props["show"]["expr"]["Literal"]["Value"], "true");
    assert_eq!(props["fontSize"]["expr"]["Literal"]["Value"], "16D");
    assert_eq!(props["underline"]["expr"]["Literal"]["Value"], "true");
    assert_eq!(
        props["fontFamily"]["expr"]["Literal"]["Value"],
        "'Segoe UI Semibold'"
    );
    let p = run(&mut s, "visual.props", json!({"page": page, "visual": v}));
    assert_eq!(p["props"]["title"]["font_family"], "Segoe UI Semibold");
    assert_eq!(p["props"]["title"]["underline"], true);
    // flat alone: the text stays, the title is hidden
    run(
        &mut s,
        "visual.set_title",
        json!({"page": page, "visual": v, "show": false}),
    );
    let props = &visual(&s, &page, &v).visual_extra["visualContainerObjects"]["title"][0]["properties"];
    assert_eq!(props["show"]["expr"]["Literal"]["Value"], "false");
    assert_eq!(props["text"]["expr"]["Literal"]["Value"], "'Sales'");
    // the subtitle takes the same font keys
    run(
        &mut s,
        "visual.format",
        json!({"page": page, "visual": v, "subtitle": {"text": "by month", "italic": true,
               "font_family": "Arial"}}),
    );
    let sub = &visual(&s, &page, &v).visual_extra["visualContainerObjects"]["subTitle"][0]["properties"];
    assert_eq!(sub["italic"]["expr"]["Literal"]["Value"], "true");
    assert_eq!(sub["fontFamily"]["expr"]["Literal"]["Value"], "'Arial'");
    schema_issues(&mut s);
}

// ---------------------------------------------------------------------------------------------
// Sort
// ---------------------------------------------------------------------------------------------

#[test]
fn set_sort_writes_sort_definition_and_clears() {
    let mut s = session();
    let (page, v) = chart(&mut s);
    let out = run(
        &mut s,
        "visual.set_sort",
        json!({"page": page, "visual": v, "field": "[Total]"}),
    );
    assert_eq!(
        out["props"]["sort"],
        json!({"field": "[Total]", "direction": "desc"})
    );
    let q = &visual(&s, &page, &v).visual_extra["query"];
    assert_eq!(
        q["sortDefinition"],
        json!({"sort": [{"field": {"Measure": {"Expression": {"SourceRef": {"Entity": "Sales"}},
                                               "Property": "Total"}},
                         "direction": "Descending"}]})
    );
    assert_eq!(
        run(
            &mut s,
            "visual.set_sort",
            json!({"page": page, "visual": v, "field": "[total]", "direction": "desc"})
        )["changed"],
        false
    );
    // A column, ascending by default.
    let out = run(
        &mut s,
        "visual.set_sort",
        json!({"page": page, "visual": v, "field": "Product[Name]"}),
    );
    assert_eq!(out["changes"], json!(["sort = Product[Name] asc"]));
    // The snapshot gives the sort in the command's field syntax (the header's Sort axis menu).
    let snap = s.snapshot();
    let sv = snap["report"]["pages"]
        .as_array()
        .and_then(|ps| ps.iter().find(|p| p["name"] == json!(page)))
        .and_then(|p| p["visuals"].as_array())
        .and_then(|vs| vs.iter().find(|x| x["name"] == json!(v)))
        .cloned()
        .unwrap();
    assert_eq!(
        sv["sort"],
        json!([{"field": "Product[Name]", "descending": false}])
    );

    // Survives a PBIR round trip, fields intact.
    let r = pbir_round_trip(s.report.as_ref().unwrap());
    let back = r.page(&page).unwrap().visual(&v).unwrap();
    assert_eq!(back.fields, visual(&s, &page, &v).fields);
    assert_eq!(
        extract_props(back)["sort"],
        json!({"field": "Product[Name]", "direction": "asc"})
    );
    schema_issues(&mut s);

    let out = run(
        &mut s,
        "visual.set_sort",
        json!({"page": page, "visual": v, "field": null}),
    );
    assert_eq!(out["changes"], json!(["sort removed"]));
    assert!(out["props"].get("sort").is_none());
    let snap = s.snapshot();
    let left = snap["report"]["pages"]
        .as_array()
        .and_then(|ps| ps.iter().find(|p| p["name"] == json!(page)))
        .and_then(|p| p["visuals"].as_array())
        .and_then(|vs| vs.iter().find(|x| x["name"] == json!(v)))
        .and_then(|x| x.get("sort"))
        .cloned();
    assert_eq!(left, None);
    // Multi-field sort through props.
    let mut copy = visual(&s, &page, &v).clone();
    let changes = apply_props(
        &mut copy,
        &json!({"sort": [{"field": "Product[Name]", "direction": "asc"}, {"field": "[Total]"}]}),
        s.model.as_ref(),
    )
    .unwrap();
    assert_eq!(changes, ["sort = Product[Name] asc, [Total] desc"]);
    assert_extract_noop(&s, &copy);
}

#[test]
fn sort_fields_must_be_bound_or_in_the_model() {
    let mut s = session();
    let (page, v) = chart(&mut s);
    let err = fails(
        &mut s,
        "visual.set_sort",
        json!({"page": page, "visual": v, "field": "Product[Nope]"}),
    );
    assert!(err.contains("Nope"), "{err}");
    fails(
        &mut s,
        "visual.set_sort",
        json!({"page": page, "visual": v, "field": "[Total]", "direction": "up"}),
    );
    fails(
        &mut s,
        "visual.set_sort",
        json!({"page": page, "visual": v, "field": null, "direction": "asc"}),
    );
    // A model field that is not bound is accepted.
    run(
        &mut s,
        "visual.set_sort",
        json!({"page": page, "visual": v, "field": "Product[Color]", "direction": "asc"}),
    );
    // A visual with no query at all gets one with an empty queryState.
    let mut bare = Visual {
        name: "bare".into(),
        visual_type: Some("tableEx".into()),
        ..Default::default()
    };
    apply_props(
        &mut bare,
        &json!({"sort": {"field": "Product[Name]"}}),
        s.model.as_ref(),
    )
    .unwrap();
    assert_eq!(bare.visual_extra["query"]["queryState"], json!({}));
}

// ---------------------------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------------------------

#[test]
fn basic_filter_has_desktop_shape_and_replaces_in_place() {
    let mut s = session();
    let (page, v) = chart(&mut s);
    let out = run(
        &mut s,
        "visual.filter",
        json!({"page": page, "visual": v, "field": "Product[Color]", "in": ["Red", "O'Neil", "Red"]}),
    );
    assert_eq!(
        out["props"]["filters"],
        json!([{"field": "Product[Color]", "in": ["Red", "O'Neil"]}])
    );
    let f = filters(visual(&s, &page, &v));
    assert_eq!(f.len(), 1);
    let name = f[0]["name"].as_str().unwrap().to_string();
    assert_eq!(name.len(), 20);
    assert_eq!(f[0]["type"], "Categorical");
    assert_eq!(f[0]["howCreated"], "User");
    assert_eq!(
        f[0]["field"],
        json!({"Column": {"Expression": {"SourceRef": {"Entity": "Product"}}, "Property": "Color"}})
    );
    assert_eq!(
        f[0]["filter"],
        json!({"Version": 2,
               "From": [{"Name": "p", "Entity": "Product", "Type": 0}],
               "Where": [{"Condition": {"In": {
                   "Expressions": [{"Column": {"Expression": {"SourceRef": {"Source": "p"}},
                                               "Property": "Color"}}],
                   "Values": [[{"Literal": {"Value": "'Red'"}}], [{"Literal": {"Value": "'O''Neil'"}}]]}}}]})
    );
    // Same filter again: no change.
    let again = run(
        &mut s,
        "visual.filter",
        json!({"page": page, "visual": v, "field": "Product[Color]", "in": ["Red", "O'Neil"]}),
    );
    assert_eq!(again["changed"], false);
    // Replaced in place: the name stays, one filter.
    run(
        &mut s,
        "visual.filter",
        json!({"page": page, "visual": v, "field": "product[color]", "not_in": ["Blue"], "hidden": true}),
    );
    let f = filters(visual(&s, &page, &v));
    assert_eq!(f.len(), 1);
    assert_eq!(f[0]["name"], json!(name));
    assert_eq!(f[0]["isHiddenInViewMode"], true);
    assert!(f[0]["filter"]["Where"][0]["Condition"]["Not"]["Expression"]["In"].is_object());
    assert_eq!(
        extract_props(visual(&s, &page, &v))["filters"],
        json!([{"field": "Product[Color]", "not_in": ["Blue"], "hidden": true}])
    );
    assert_extract_noop(&s, visual(&s, &page, &v));
    let v2 = schema_issues(&mut s);
    assert_eq!(v2["valid"], true, "{v2}");
}

#[test]
fn advanced_filters_type_values_by_column() {
    let mut s = session();
    let (page, v) = chart(&mut s);
    let cond = |s: &Session| filters(visual(s, &page, &v))[0]["filter"]["Where"][0]["Condition"].clone();
    run(
        &mut s,
        "visual.filter",
        json!({"page": page, "visual": v, "field": "Sales[Amount]", "op": ">", "value": 100}),
    );
    assert_eq!(
        cond(&s),
        json!({"Comparison": {"ComparisonKind": 1,
            "Left": {"Column": {"Expression": {"SourceRef": {"Source": "s"}}, "Property": "Amount"}},
            "Right": {"Literal": {"Value": "100D"}}}})
    );
    assert_eq!(filters(visual(&s, &page, &v))[0]["type"], "Advanced");
    run(
        &mut s,
        "visual.filter",
        json!({"page": page, "visual": v, "field": "Sales[Qty]", "op": "<=", "value": 5}),
    );
    run(
        &mut s,
        "visual.filter",
        json!({"page": page, "visual": v, "field": "Sales[Price]", "op": ">=", "value": 2.5}),
    );
    run(
        &mut s,
        "visual.filter",
        json!({"page": page, "visual": v, "field": "Sales[Date]",
               "conditions": [{"op": ">=", "value": "2024-01-01"}, {"op": "<", "value": "2025-01-01T00:00"}]}),
    );
    run(
        &mut s,
        "visual.filter",
        json!({"page": page, "visual": v, "field": "[Total]", "op": "<>", "value": null}),
    );
    run(
        &mut s,
        "visual.filter",
        json!({"page": page, "visual": v, "field": "Product[Name]",
               "conditions": [{"op": "contains", "value": "Bike"}, {"op": "not_starts_with", "value": "X"}],
               "logic": "or"}),
    );
    let all = filters(visual(&s, &page, &v));
    assert_eq!(all.len(), 6);
    let lit = |i: usize, path: &[&str]| {
        let mut x = &all[i]["filter"]["Where"][0]["Condition"];
        for p in path {
            x = &x[*p];
        }
        x.clone()
    };
    assert_eq!(lit(1, &["Comparison", "Right", "Literal", "Value"]), "5L");
    assert_eq!(lit(1, &["Comparison", "ComparisonKind"]), 4);
    assert_eq!(lit(2, &["Comparison", "Right", "Literal", "Value"]), "2.5M");
    assert_eq!(
        lit(3, &["And", "Left", "Comparison", "Right", "Literal", "Value"]),
        "datetime'2024-01-01T00:00:00'"
    );
    assert_eq!(
        lit(3, &["And", "Right", "Comparison", "Right", "Literal", "Value"]),
        "datetime'2025-01-01T00:00:00'"
    );
    assert_eq!(
        lit(
            4,
            &["Not", "Expression", "Comparison", "Right", "Literal", "Value"]
        ),
        "null"
    );
    assert_eq!(
        all[4]["field"],
        json!({"Measure": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Total"}})
    );
    assert!(lit(5, &["Or", "Left", "Contains"]).is_object());
    assert!(lit(5, &["Or", "Right", "Not", "Expression", "StartsWith"]).is_object());
    let props = extract_props(visual(&s, &page, &v));
    assert_eq!(
        props["filters"][3],
        json!({"field": "Sales[Date]", "conditions": [
            {"op": ">=", "value": "2024-01-01T00:00:00"}, {"op": "<", "value": "2025-01-01T00:00:00"}],
            "logic": "and"})
    );
    assert_eq!(
        props["filters"][4],
        json!({"field": "[Total]", "op": "<>", "value": null})
    );
    assert_extract_noop(&s, visual(&s, &page, &v));
    let report = schema_issues(&mut s);
    assert_eq!(report["valid"], true, "{report}");

    // Survives PBIR save and re-read.
    let r = pbir_round_trip(s.report.as_ref().unwrap());
    let back = r.page(&page).unwrap().visual(&v).unwrap();
    assert_eq!(back.filter_config, visual(&s, &page, &v).filter_config);
    assert_eq!(extract_props(back), props);
}

#[test]
fn top_n_filter_has_desktop_shape() {
    let mut s = session();
    let (page, v) = chart(&mut s);
    run(
        &mut s,
        "visual.filter",
        json!({"page": page, "visual": v, "field": "Product[Name]", "top": 5, "by": "[Total]"}),
    );
    let f = &filters(visual(&s, &page, &v))[0];
    assert_eq!(f["type"], "TopN");
    assert_eq!(
        f["filter"],
        json!({"Version": 2,
            "From": [
                {"Name": "subquery", "Expression": {"Subquery": {"Query": {
                    "Version": 2,
                    "From": [{"Name": "p", "Entity": "Product", "Type": 0},
                             {"Name": "s", "Entity": "Sales", "Type": 0}],
                    "Select": [{"Column": {"Expression": {"SourceRef": {"Source": "p"}}, "Property": "Name"},
                                "Name": "field"}],
                    "OrderBy": [{"Direction": 2, "Expression": {"Measure": {
                        "Expression": {"SourceRef": {"Source": "s"}}, "Property": "Total"}}}],
                    "Top": 5}}}, "Type": 2},
                {"Name": "p", "Entity": "Product", "Type": 0}],
            "Where": [{"Condition": {"In": {
                "Expressions": [{"Column": {"Expression": {"SourceRef": {"Source": "p"}}, "Property": "Name"}}],
                "Table": {"SourceRef": {"Source": "subquery"}}}}}]})
    );
    assert_eq!(
        extract_props(visual(&s, &page, &v))["filters"],
        json!([{"field": "Product[Name]", "top": 5, "by": "[Total]", "direction": "top"}])
    );
    // Bottom 3 by an aggregation of the same table as the field.
    run(
        &mut s,
        "visual.filter",
        json!({"page": page, "visual": v, "field": "Sales[Date]", "top": 3, "by": "Sum(Sales[Amount])",
               "direction": "bottom"}),
    );
    let f = &filters(visual(&s, &page, &v))[1];
    let q = &f["filter"]["From"][0]["Expression"]["Subquery"]["Query"];
    assert_eq!(q["From"].as_array().unwrap().len(), 1);
    assert_eq!(q["OrderBy"][0]["Direction"], 1);
    assert_eq!(q["OrderBy"][0]["Expression"]["Aggregation"]["Function"], 0);
    assert_extract_noop(&s, visual(&s, &page, &v));
    let report = schema_issues(&mut s);
    assert_eq!(report["valid"], true, "{report}");
    // A column cannot rank, a measure cannot be kept.
    fails(
        &mut s,
        "visual.filter",
        json!({"page": page, "visual": v, "field": "Product[Name]", "top": 5, "by": "Sales[Amount]"}),
    );
    fails(
        &mut s,
        "visual.filter",
        json!({"page": page, "visual": v, "field": "[Total]", "top": 5, "by": "[Total]"}),
    );
    fails(
        &mut s,
        "visual.filter",
        json!({"page": page, "visual": v, "field": "Product[Name]", "top": 0, "by": "[Total]"}),
    );
}

#[test]
fn clear_filters_keeps_auto_cards_and_others() {
    let mut s = session();
    let (page, v) = chart(&mut s);
    // An automatic card of a bound field, with a condition, plus a card without one.
    {
        let vis = s
            .report
            .as_mut()
            .unwrap()
            .page_mut(&page)
            .unwrap()
            .visual_mut(&v)
            .unwrap();
        vis.filter_config = Some(json!({"filters": [
            {"name": "auto1", "field": {"Column": {"Expression": {"SourceRef": {"Entity": "Product"}},
                                                   "Property": "Name"}},
             "type": "Categorical", "howCreated": "Auto"},
            {"name": "auto2", "field": {"Measure": {"Expression": {"SourceRef": {"Entity": "Sales"}},
                                                    "Property": "Total"}},
             "type": "Advanced", "howCreated": "Auto"},
        ]}));
    }
    // Filtering a field that has an automatic card fills that card.
    run(
        &mut s,
        "visual.filter",
        json!({"page": page, "visual": v, "field": "Product[Name]", "in": ["A"]}),
    );
    run(
        &mut s,
        "visual.filter",
        json!({"page": page, "visual": v, "field": "Product[Color]", "in": ["Red"]}),
    );
    let f = filters(visual(&s, &page, &v));
    assert_eq!(f.len(), 3);
    assert_eq!(f[0]["name"], "auto1");
    assert_eq!(f[0]["howCreated"], "Auto");
    assert!(f[0]["filter"].is_object());
    assert_eq!(
        f[0].as_object().unwrap().keys().collect::<Vec<_>>(),
        ["name", "field", "type", "filter", "howCreated"]
    );
    // Clear one field.
    let out = run(
        &mut s,
        "visual.clear_filters",
        json!({"page": page, "visual": v, "field": "Product[Color]"}),
    );
    assert_eq!(out["changes"], json!(["filter on Product[Color] removed"]));
    fails(
        &mut s,
        "visual.clear_filters",
        json!({"page": page, "visual": v, "field": "Product[Color]"}),
    );
    // Clear all: the automatic cards stay, without condition.
    let out = run(&mut s, "visual.clear_filters", json!({"page": page, "visual": v}));
    assert_eq!(out["changes"], json!(["filter on Product[Name] cleared"]));
    let f = filters(visual(&s, &page, &v));
    assert_eq!(f.len(), 2);
    assert!(f.iter().all(|c| c.get("filter").is_none()));
    assert_eq!(
        run(&mut s, "visual.clear_filters", json!({"page": page, "visual": v}))["changed"],
        false
    );
    // A visual whose only filters were ours loses its filterConfig.
    let (_, w) = chart(&mut s);
    run(
        &mut s,
        "visual.filter",
        json!({"page": page, "visual": w, "field": "Product[Color]", "in": ["Red"]}),
    );
    run(&mut s, "visual.clear_filters", json!({"page": page, "visual": w}));
    assert!(visual(&s, &page, &w).filter_config.is_none());
}

#[test]
fn bad_filters_are_rejected_without_changes() {
    let mut s = session();
    let (page, v) = chart(&mut s);
    let base = |extra: Value| {
        let mut p = json!({"page": page, "visual": v});
        for (k, x) in extra.as_object().unwrap() {
            p[k] = x.clone();
        }
        p
    };
    for extra in [
        json!({"field": "Product[Name]", "value": "a"}),
        json!({"field": "Product[Name]", "in": []}),
        json!({"field": "Product[Name]", "in": ["a"], "op": "=", "value": "a"}),
        json!({"field": "Product[Name]", "in": ["a"], "value": "a"}),
        json!({"field": "[Total]", "in": [1]}),
        json!({"field": "Sales[Qty]", "in": [1.5]}),
        json!({"field": "Sales[Amount]", "in": ["x"]}),
        json!({"field": "Sales[Date]", "in": ["yesterday"]}),
        json!({"field": "Sales[Amount]", "op": "~", "value": 1}),
        json!({"field": "Sales[Amount]", "op": ">", "value": null}),
        json!({"field": "Sales[Amount]", "op": "contains", "value": 1}),
        json!({"field": "Sales[Amount]", "op": ">"}),
        json!({"field": "Sales[Amount]", "conditions": [{"op": ">", "value": 1}, {"op": ">", "value": 1},
                                                         {"op": ">", "value": 1}]}),
        json!({"field": "Sales[Amount]", "op": ">", "value": 1, "logic": "xor"}),
        json!({"field": "Sales[Amount]", "op": ">", "value": 1, "by": "[Total]"}),
        json!({"field": "Product[Nope]", "in": ["a"]}),
        json!({"field": "Product[Name]", "in": ["a"], "hidden": "yes"}),
        json!({"field": "Product[Name]", "in": vec![json!("a"); 1001]}),
    ] {
        fails(&mut s, "visual.filter", base(extra));
    }
    // Through props: duplicates and bad raw items.
    let mut copy = visual(&s, &page, &v).clone();
    let before = copy.clone();
    for props in [
        json!({"filters": [{"field": "Product[Name]", "in": ["a"]}, {"field": "product[name]", "in": ["b"]}]}),
        json!({"filters": [{"raw": {"field": {}}}]}),
        json!({"filters": [{"raw": {"name": "x"}, "field": "Product[Name]"}]}),
        json!({"filters": {"field": "Product[Name]"}}),
        json!({"filters": [{"field": "Product[Name]", "in": ["a"]}], "colour": 1}),
    ] {
        assert!(
            apply_props(&mut copy, &props, s.model.as_ref()).is_err(),
            "{props}"
        );
        assert_eq!(copy, before);
    }
}

#[test]
fn raw_filters_are_kept_verbatim_and_matched_by_name() {
    let s = session();
    let raw = json!({"name": "rel", "field": {"Column": {"Expression": {"SourceRef": {"Entity": "Sales"}},
                                                         "Property": "Date"}},
                     "type": "RelativeDate", "filter": {"Version": 2, "From": [], "Where": []},
                     "howCreated": "User"});
    let mut v = Visual {
        name: "v".into(),
        visual_type: Some("card".into()),
        filter_config: Some(json!({"filters": [raw.clone()]})),
        ..Default::default()
    };
    let props = extract_props(&v);
    assert_eq!(props["filters"], json!([{"raw": raw}]));
    assert_extract_noop(&s, &v);
    // Adding a typed filter next to it keeps it.
    let mut list = props["filters"].as_array().unwrap().clone();
    list.push(json!({"field": "Product[Color]", "in": ["Red"]}));
    let changes = apply_props(&mut v, &json!({"filters": list}), s.model.as_ref()).unwrap();
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!(filters(&v)[0], raw);
    // Dropping it from the list removes it.
    let changes = apply_props(
        &mut v,
        &json!({"filters": [{"field": "Product[Color]", "in": ["Red"]}]}),
        s.model.as_ref(),
    )
    .unwrap();
    assert_eq!(changes, ["filter on Sales[Date] removed"]);
}

// ---------------------------------------------------------------------------------------------
// Format
// ---------------------------------------------------------------------------------------------

#[test]
fn format_writes_container_and_visual_objects() {
    let mut s = session();
    let (page, v) = chart(&mut s);
    let out = run(
        &mut s,
        "visual.format",
        json!({"page": page, "visual": v,
               "background": {"show": true, "color": "#FFFFFF", "transparency": 20},
               "border": {"show": true, "color": "#CCCCCC", "radius": 8},
               "padding": 4,
               "shadow": true,
               "header": false,
               "alt_text": "Sales by product",
               "subtitle": "FY 2024",
               "data_labels": {"show": true, "font_size": 9},
               "legend": {"show": true, "position": "bottom"},
               "x_axis": {"title": false},
               "y_axis": {"show": true, "title": true, "font_size": 10, "color": "#333"}}),
    );
    assert!(out["changes"].as_array().unwrap().len() >= 18, "{out}");
    let vis = visual(&s, &page, &v);
    let c = &vis.visual_extra["visualContainerObjects"];
    let lit = |x: &Value| x["expr"]["Literal"]["Value"].clone();
    assert_eq!(lit(&c["background"][0]["properties"]["show"]), "true");
    assert_eq!(lit(&c["background"][0]["properties"]["transparency"]), "20D");
    assert_eq!(lit(&c["border"][0]["properties"]["radius"]), "8D");
    for side in ["top", "bottom", "left", "right"] {
        assert_eq!(lit(&c["padding"][0]["properties"][side]), "4D");
    }
    assert_eq!(lit(&c["dropShadow"][0]["properties"]["show"]), "true");
    assert_eq!(lit(&c["visualHeader"][0]["properties"]["show"]), "false");
    assert_eq!(
        lit(&c["general"][0]["properties"]["altText"]),
        "'Sales by product'"
    );
    assert_eq!(lit(&c["subTitle"][0]["properties"]["text"]), "'FY 2024'");
    let o = &vis.visual_extra["objects"];
    assert_eq!(lit(&o["labels"][0]["properties"]["fontSize"]), "9D");
    assert_eq!(lit(&o["legend"][0]["properties"]["position"]), "'Bottom'");
    assert_eq!(lit(&o["categoryAxis"][0]["properties"]["showAxisTitle"]), "false");
    assert_eq!(lit(&o["valueAxis"][0]["properties"]["fontSize"]), "10D");
    // Desktop's key order for the visual object.
    let keys: Vec<&String> = vis.visual_extra.keys().collect();
    let pos = |k: &str| keys.iter().position(|x| *x == k).unwrap();
    assert!(pos("objects") < pos("visualContainerObjects"));
    let props = run(&mut s, "visual.props", json!({"page": page, "visual": v}))["props"].clone();
    assert_eq!(
        props["format"]["padding"],
        json!({"top": 4, "bottom": 4, "left": 4, "right": 4})
    );
    assert_eq!(props["format"]["alt_text"], "Sales by product");
    assert_eq!(props["format"]["subtitle"], "FY 2024");
    assert_eq!(
        props["format"]["legend"],
        json!({"show": true, "position": "Bottom"})
    );
    assert_extract_noop(&s, visual(&s, &page, &v));
    let report = schema_issues(&mut s);
    assert_eq!(report["valid"], true, "{report}");
    // Re-applying is a no-op; null removes a group and cleans up.
    assert_eq!(
        run(
            &mut s,
            "visual.format",
            json!({"page": page, "visual": v, "padding": 4, "shadow": true})
        )["changed"],
        false
    );
    run(
        &mut s,
        "visual.format",
        json!({"page": page, "visual": v, "padding": null, "alt_text": null}),
    );
    let c = &visual(&s, &page, &v).visual_extra["visualContainerObjects"];
    assert!(c.get("padding").is_none());
    assert!(c.get("general").is_none());
}

#[test]
fn generic_objects_escape_hatch() {
    let mut s = session();
    let (page, v) = chart(&mut s);
    run(
        &mut s,
        "visual.format",
        json!({"page": page, "visual": v, "objects": [
            {"object": "title", "property": "fontFamily", "value": "Segoe UI"},
            {"object": "general", "property": "keepLayerOrder", "value": true},
            {"object": "dataPoint", "property": "fill", "value": {"color": "#FF0000"},
             "selector": {"metadata": "Sales.Total"}},
            {"object": "smallMultiplesLayout", "property": "rowCount", "value": {"literal": "3L"}},
            {"object": "valueAxis", "property": "start", "value": {"expr": {"Measure": {
                "Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Total"}}}},
            {"object": "general", "property": "responsive", "value": false, "target": "visual"},
        ]}),
    );
    let vis = visual(&s, &page, &v);
    let c = &vis.visual_extra["visualContainerObjects"];
    let o = &vis.visual_extra["objects"];
    assert_eq!(
        c["title"][0]["properties"]["fontFamily"]["expr"]["Literal"]["Value"],
        "'Segoe UI'"
    );
    assert_eq!(
        c["general"][0]["properties"]["keepLayerOrder"]["expr"]["Literal"]["Value"],
        "true"
    );
    assert_eq!(
        o["dataPoint"][0],
        json!({"properties": {"fill": {"solid": {"color": {"expr": {"Literal": {"Value": "'#FF0000'"}}}}}},
               "selector": {"metadata": "Sales.Total"}})
    );
    assert_eq!(
        o["smallMultiplesLayout"][0]["properties"]["rowCount"]["expr"]["Literal"]["Value"],
        "3L"
    );
    assert!(o["valueAxis"][0]["properties"]["start"]["expr"]["Measure"].is_object());
    assert_eq!(
        o["general"][0]["properties"]["responsive"]["expr"]["Literal"]["Value"],
        "false"
    );
    let report = schema_issues(&mut s);
    assert_eq!(report["valid"], true, "{report}");
    // Same edits again: nothing changes (a selector entry is found again).
    let again = run(
        &mut s,
        "visual.format",
        json!({"page": page, "visual": v, "objects": [
            {"object": "dataPoint", "property": "fill", "value": {"color": "#ff0000"},
             "selector": {"metadata": "Sales.Total"}},
            {"object": "smallMultiplesLayout", "property": "rowCount", "value": 3}]}),
    );
    assert_eq!(again["changed"], false, "{again}");
    // Removing the selector entry's only property removes the entry and the object.
    run(
        &mut s,
        "visual.format",
        json!({"page": page, "visual": v, "objects": [
            {"object": "dataPoint", "property": "fill", "value": null,
             "selector": {"metadata": "Sales.Total"}}]}),
    );
    assert!(
        visual(&s, &page, &v).visual_extra["objects"]
            .get("dataPoint")
            .is_none()
    );
    // Errors.
    for item in [
        json!({"object": "title", "property": "colour", "value": 1, "target": "container"}),
        json!({"object": "nope", "property": "x", "value": 1, "target": "container"}),
        json!({"object": "a b", "property": "x", "value": 1}),
        json!({"object": "a", "property": "x"}),
        json!({"object": "a", "property": "x", "value": [1]}),
        json!({"object": "a", "property": "x", "value": {"what": 1}}),
        json!({"object": "a", "property": "x", "value": {"color": "red"}}),
        json!({"object": "a", "property": "x", "value": 1, "target": "page"}),
        json!({"object": "a", "property": "x", "value": 1, "selector": "s"}),
        // Checked against the visual type's formatting catalog.
        json!({"object": "legnd", "property": "show", "value": true}),
        json!({"object": "legend", "property": "positon", "value": "Top"}),
        json!({"object": "legend", "property": "position", "value": "Middle"}),
        json!({"object": "labels", "property": "fontSize", "value": 4}),
        json!({"object": "labels", "property": "show", "value": "yes"}),
        json!({"object": "dataPoint", "property": "fill", "value": "#FF0000"}),
        json!({"object": "categoryAxis", "property": "innerPadding", "value": 2.5}),
        json!({"object": "general", "property": "paragraphs", "value": "x", "target": "visual"}),
    ] {
        fails(
            &mut s,
            "visual.format",
            json!({"page": page, "visual": v, "objects": [item]}),
        );
    }
    for bad in [
        json!({"border": "yes"}),
        json!({"background": {"transparency": 120}}),
        json!({"legend": {"position": "middle"}}),
        json!({"legend": {"font_size": 5}}),
        json!({"padding": -1}),
        json!({"margin": 1}),
        json!({}),
    ] {
        let mut p = bad.clone();
        p["page"] = json!(page);
        p["visual"] = json!(v);
        fails(&mut s, "visual.format", p);
    }
}

#[test]
fn formatting_is_checked_against_the_catalog() {
    let s = session();
    let mut table = Visual {
        name: "t".into(),
        visual_type: Some("tableEx".into()),
        ..Default::default()
    };
    // A table has no legend: refused, with where to look.
    let e = apply_props(&mut table, &json!({"format": {"legend": true}}), s.model.as_ref()).unwrap_err();
    assert!(e.contains("`tableEx` has no visual object `legend`"), "{e}");
    let e = apply_props(
        &mut table,
        &json!({"format": {"objects": [{"object": "valuess", "property": "fontSize", "value": 10}]}}),
        s.model.as_ref(),
    )
    .unwrap_err();
    assert!(e.contains("did you mean `values`"), "{e}");
    let e = apply_props(
        &mut table,
        &json!({"format": {"objects": [{"object": "values", "property": "fontSize", "value": 5}]}}),
        s.model.as_ref(),
    )
    .unwrap_err();
    assert!(e.contains("at least 8"), "{e}");
    assert_eq!(table.visual_extra.len(), 0);
    // Container formatting is fine on any type; removing anything is always allowed.
    let ok = json!({"format": {"border": true, "objects": [
        {"object": "values", "property": "fontSize", "value": 11},
        {"object": "grid", "property": "gridVertical", "value": true},
        {"object": "legend", "property": "show", "value": null}]}});
    assert_eq!(apply_props(&mut table, &ok, s.model.as_ref()).unwrap().len(), 3);
    // A value the visual already has is not re-checked (files may hold what the catalog lacks).
    table.visual_extra["objects"]["values"][0]["properties"]["fontSize"] =
        json!({"expr": {"Literal": {"Value": "5D"}}});
    let same = json!({"format": {"objects": [{"object": "values", "property": "fontSize", "value": 5}]}});
    assert!(
        apply_props(&mut table, &same, s.model.as_ref())
            .unwrap()
            .is_empty()
    );
    // Custom visuals are not checked.
    let mut custom = Visual {
        name: "c".into(),
        visual_type: Some("myCustomVisual0123".into()),
        ..Default::default()
    };
    let any = json!({"format": {"legend": {"font_size": 300},
                                "objects": [{"object": "whatever", "property": "x", "value": 1}]}});
    assert_eq!(apply_props(&mut custom, &any, s.model.as_ref()).unwrap().len(), 2);
}

#[test]
fn format_warns_on_custom_visuals() {
    let mut s = session();
    let (page, v) = chart(&mut s);
    let out = run(
        &mut s,
        "visual.format",
        json!({"page": page, "visual": v, "legend": {"position": "TopRight"}}),
    );
    assert_eq!(out["warnings"], json!([]));
    let pages = &mut s.report.as_mut().unwrap().pages;
    let target = pages
        .iter_mut()
        .flat_map(|p| p.visuals.iter_mut())
        .find(|x| x.name == v)
        .unwrap();
    target.visual_type = Some("myCustomVisual0123".into());
    let out = run(
        &mut s,
        "visual.format",
        json!({"page": page, "visual": v, "border": true}),
    );
    assert!(
        out["warnings"][0].as_str().unwrap().contains("not checked"),
        "{out}"
    );
}

#[test]
fn groups_and_non_objects_are_rejected() {
    let s = session();
    let mut group = Visual {
        name: "g".into(),
        ..Default::default()
    };
    assert!(apply_props(&mut group, &json!({"title": "x"}), s.model.as_ref()).is_err());
    assert_eq!(
        apply_props(&mut group, &json!({}), s.model.as_ref()).unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(extract_props(&group), json!({}));
    let mut v = Visual {
        name: "v".into(),
        visual_type: Some("card".into()),
        ..Default::default()
    };
    assert!(apply_props(&mut v, &json!([1]), None).is_err());
    assert!(apply_props(&mut v, &json!(null), None).unwrap().is_empty());
    // Hostile existing JSON: errors, never panics, nothing changes.
    v.visual_extra.insert("visualContainerObjects".into(), json!([]));
    v.visual_extra.insert("query".into(), json!("q"));
    v.filter_config = Some(json!({"filters": 3}));
    let before = v.clone();
    for props in [
        json!({"title": "x"}),
        json!({"sort": {"field": "Product[Name]"}}),
        json!({"filters": []}),
    ] {
        assert!(apply_props(&mut v, &props, s.model.as_ref()).is_err(), "{props}");
        assert_eq!(v, before);
    }
    assert_eq!(extract_props(&v), json!({}));
}

#[test]
fn visual_props_is_read_only() {
    let mut s = session();
    let (page, v) = chart(&mut s);
    let rev = s.revision;
    let out = run(&mut s, "visual.props", json!({"page": page, "visual": v}));
    assert_eq!(out["props"], json!({}));
    assert_eq!(out["visual_type"], "clusteredColumnChart");
    assert_eq!(s.revision, rev);
    assert!(crate::find_command("visual.props").is_some());
}

// ---------------------------------------------------------------------------------------------
// Idempotence and round trips
// ---------------------------------------------------------------------------------------------

/// A tiny deterministic generator (no dependency).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) as usize) % n.max(1)
    }
}

/// Random props from a pool of valid pieces.
fn random_props(g: &mut Lcg) -> Value {
    let titles = [
        json!("Revenue"),
        json!({"text": "It's here", "show": false}),
        json!({"font_size": 12, "color": "#abc", "alignment": "right", "italic": true}),
        json!(null),
    ];
    let sorts = [
        json!({"field": "[Total]"}),
        json!({"field": "Product[Name]", "direction": "desc"}),
        json!([{"field": "Product[Color]"}, {"field": "Sum(Sales[Amount])", "direction": "asc"}]),
        json!(null),
    ];
    let filter_pool = [
        json!({"field": "Product[Color]", "in": ["Red", "Blue"]}),
        json!({"field": "Product[Name]", "not_in": ["X"], "locked": true}),
        json!({"field": "Sales[Amount]", "op": ">=", "value": 10.5}),
        json!({"field": "Sales[Qty]", "conditions": [{"op": ">", "value": 1}, {"op": "<", "value": 9}], "logic": "or"}),
        json!({"field": "Sales[Date]", "op": "<", "value": "2024-06-30"}),
        json!({"field": "[Total]", "op": "=", "value": null}),
        json!({"field": "Date Table[Date]", "top": 7, "by": "[Total]", "direction": "bottom"}),
        json!({"field": "Product[Name]", "op": "contains", "value": "a'b"}),
    ];
    let formats = [
        json!({"background": {"color": "#FFFFFF", "transparency": 0}}),
        json!({"border": true, "padding": 6, "header": true}),
        json!({"shadow": {"show": true, "transparency": 50}, "alt_text": "alt"}),
        json!({"data_labels": false, "legend": {"position": "TopCenter", "font_size": 11}}),
        json!({"x_axis": {"show": false}, "y_axis": {"title": true}, "subtitle": {"text": "s", "show": false}}),
        json!({"objects": [{"object": "dataPoint", "property": "fill", "value": {"color": "#123456"},
                            "selector": {"id": "x"}}]}),
        json!({"border": null}),
    ];
    let mut props = serde_json::Map::new();
    if g.next(2) == 0 {
        props.insert("title".into(), titles[g.next(titles.len())].clone());
    }
    if g.next(2) == 0 {
        props.insert("sort".into(), sorts[g.next(sorts.len())].clone());
    }
    if g.next(2) == 0 {
        let mut list = Vec::new();
        for (i, f) in filter_pool.iter().enumerate() {
            // Only one filter per field.
            let field = f["field"].as_str().unwrap();
            if g.next(3) == 0 && !list.iter().any(|x: &Value| x["field"] == field) {
                list.push(filter_pool[i].clone());
            }
        }
        props.insert("filters".into(), Value::Array(list));
    }
    if g.next(2) == 0 {
        props.insert("format".into(), formats[g.next(formats.len())].clone());
    }
    Value::Object(props)
}

#[test]
fn extract_then_apply_is_a_no_op_on_varied_visuals() {
    let mut s = session();
    let (page, v) = chart(&mut s);
    let mut g = Lcg(42);
    for round in 0..300 {
        let props = random_props(&mut g);
        let mut vis = visual(&s, &page, &v).clone();
        if round % 3 == 0 {
            // Another type that has every formatting object the pool uses (a table has no
            // legend or axes, which the formatting catalog refuses).
            vis.visual_type = Some("lineChart".into());
        }
        if round % 5 == 0 {
            vis.fields.clear();
        }
        // Several rounds of edits stack up.
        for _ in 0..3 {
            apply_props(&mut vis, &random_props(&mut g), s.model.as_ref())
                .unwrap_or_else(|e| panic!("round {round}: {e}"));
        }
        apply_props(&mut vis, &props, s.model.as_ref())
            .unwrap_or_else(|e| panic!("round {round} {props}: {e}"));
        // Applying the same props twice is a no-op.
        let mut twice = vis.clone();
        assert!(
            apply_props(&mut twice, &props, s.model.as_ref())
                .unwrap()
                .is_empty(),
            "round {round}: {props}"
        );
        assert_extract_noop(&s, &vis);
        // And so is the extract of the PBIR round trip.
        let mut r = s.report.clone().unwrap();
        *r.page_mut(&page).unwrap().visual_mut(&v).unwrap() = vis.clone();
        let back = pbir_round_trip(&r);
        let back_v = back.page(&page).unwrap().visual(&v).unwrap();
        assert_eq!(extract_props(back_v), extract_props(&vis), "round {round}");
        if round % 25 == 0 {
            let issues = bicraft_pbir::validate_documents(&bicraft_pbir::write_documents(&r));
            assert!(issues.is_empty(), "round {round}: {issues:?}");
        }
    }
}

#[test]
fn committed_fixture_extracts_and_reapplies_without_model() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../pbir/tests/fixtures/sales.Report");
    let r = bicraft_pbir::read_dir(&dir).unwrap();
    let s = Session::new();
    let mut checked = 0;
    for p in &r.pages {
        for v in &p.visuals {
            assert_extract_noop(&s, v);
            checked += 1;
        }
    }
    assert!(checked > 5);
    let chart = r
        .pages
        .iter()
        .flat_map(|p| &p.visuals)
        .find(|v| v.name == "c0ffee00000000000002")
        .unwrap();
    let props = extract_props(chart);
    assert_eq!(props["title"], json!({"text": "Sales by year"}));
    assert_eq!(props["sort"], json!({"field": "Date[Year]", "direction": "asc"}));
    assert_eq!(props["filters"], json!([{"field": "Date[Year]", "in": ["2024"]}]));
    assert_eq!(
        props["format"],
        json!({
            "legend": {"show": true},
            "objects": [{"object": "legend", "property": "show", "value": true}],
        })
    );
    // Without a model, a title and an extra filter resolve against the visual's own fields.
    let mut v = chart.clone();
    let mut list = props["filters"].as_array().unwrap().clone();
    list.push(json!({"field": "Sum(Sales[Amount])", "op": ">", "value": 0}));
    let changes = apply_props(&mut v, &json!({"filters": list}), None).unwrap();
    assert_eq!(changes.len(), 1, "{changes:?}");
}

/// Real reports, local only (skipped without `BICRAFT_TESTDATA`): every visual's extract
/// re-applies as a no-op, with the file's model.
#[test]
fn extract_then_apply_is_a_no_op_on_real_visuals() {
    let Some(root) = bicraft_env::var_os("BICRAFT_TESTDATA").map(std::path::PathBuf::from) else {
        eprintln!("BICRAFT_TESTDATA unset: skipping");
        return;
    };
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&root)
        .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    files.push(root.join("sample1").join("report.pbix"));
    files.retain(|p| {
        p.is_file()
            && p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("pbix") || e.eq_ignore_ascii_case("pbit"))
    });
    files.sort();
    let (mut visuals, mut with_props, mut typed, mut raw) = (0, 0, 0, 0);
    for path in files {
        let mut s = Session::new();
        if s.execute("file.open", &json!({"path": path})).is_err() || s.report.is_none() {
            eprintln!("a file could not be opened with a report: skipped");
            continue;
        }
        for p in &s.report.as_ref().unwrap().pages {
            for v in &p.visuals {
                assert_extract_noop(&s, v);
                visuals += 1;
                let props = extract_props(v);
                if props.as_object().is_some_and(|o| !o.is_empty()) {
                    with_props += 1;
                }
                for f in props["filters"].as_array().into_iter().flatten() {
                    if f.get("raw").is_some() {
                        raw += 1;
                    } else {
                        typed += 1;
                    }
                }
            }
        }
    }
    eprintln!("real visuals: {visuals} checked, {with_props} with props; filters: {typed} typed, {raw} raw");
}
