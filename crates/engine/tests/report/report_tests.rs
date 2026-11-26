//! Tests of the report commands (`report.*`, `page.*`, `visual.*`), of how model edits report
//! and rewrite visual bindings, and of saving reports. Tests that read or write PBIR on disk wait
//! for the PBIR reader/writer (M2.1) and are ignored until it lands.

use bicraft_report::{FieldRef, Report};
use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::{Session, Source};

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Runs a command that must fail, checks neither the report nor its dirty flag changed, and
/// returns the error.
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

fn temp(name: &str) -> TempDir {
    TempDir::new(&format!("engine-report-{name}"))
}

/// Sales(Amount, Product, Date) with [Total], Product(Product, Name), 'Date Table'(Date), and a
/// blank report.
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
            json!({"table": "Sales", "name": "Product", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Date", "data_type": "dateTime"}),
        ),
        (
            "column.add",
            json!({"table": "Product", "name": "Product", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Product", "name": "Name", "data_type": "string"}),
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

fn report(s: &Session) -> &Report {
    s.report.as_ref().unwrap()
}

fn first_page(s: &Session) -> String {
    report(s).pages[0].name.clone()
}

/// Adds a column chart with Category = Product[Name], Y = [Total]; returns (page, visual).
fn chart(s: &mut Session) -> (String, String) {
    let page = first_page(s);
    let v = run(
        s,
        "visual.add",
        json!({"page": page, "visual_type": "clusteredColumnChart", "x": 10, "y": 20, "width": 400,
               "height": 300, "fields": [{"role": "Category", "field": "Product[Name]"},
                                         {"role": "Y", "field": "[Total]"}]}),
    );
    (page, v["visual"].as_str().unwrap().to_string())
}

fn is_hex_name(n: &str) -> bool {
    n.len() == 20 && n.chars().all(|c| c.is_ascii_hexdigit())
}

#[test]
fn report_new_starts_a_blank_report_once() {
    let mut s = session();
    assert!(s.report_dirty);
    let r = report(&s);
    assert_eq!(r.pages.len(), 1);
    assert_eq!(r.pages[0].display_name, "Page 1");
    assert!(is_hex_name(&r.pages[0].name));
    assert_eq!(r.active_page.as_deref(), Some(r.pages[0].name.as_str()));
    let e = fails(&mut s, "report.new", json!({}));
    assert!(e.contains("already open"), "{e}");
    let info = run(&mut s, "file.inspect", json!({}));
    assert_eq!(info["report"], json!({"pages": 1, "visuals": 0}));
    assert_eq!(info["report_dirty"], true);

    // model.new drops the report.
    run(&mut s, "model.new", json!({"name": "Other"}));
    assert!(s.report.is_none() && !s.report_dirty);
}

#[test]
fn report_commands_need_a_report() {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "M"}));
    for (id, p) in [
        ("report.summary", json!({})),
        ("report.validate", json!({})),
        ("page.add", json!({"display_name": "P"})),
        ("visual.remove", json!({"page": "P", "visual": "v"})),
    ] {
        let e = s.execute(id, &p).unwrap_err();
        assert!(e.contains("report.new"), "{id}: {e}");
    }
    s.report_error = Some("broken file".into());
    let e = s.execute("report.summary", &json!({})).unwrap_err();
    assert!(e.contains("broken file"), "{e}");
}

#[test]
fn pages_are_added_renamed_moved_activated_and_removed() {
    let mut s = session();
    s.report_dirty = false;
    let first = first_page(&s);
    let v = run(
        &mut s,
        "page.add",
        json!({"display_name": "Details", "width": 1920, "height": 1080, "display_option": "fittowidth"}),
    );
    let details = v["page"].as_str().unwrap().to_string();
    assert!(is_hex_name(&details) && details != first);
    assert_eq!(v["index"], 1);
    assert_eq!(v["active"], false);
    assert!(s.report_dirty);
    let page = &report(&s).pages[1];
    assert_eq!((page.width, page.height), (1920.0, 1080.0));
    assert_eq!(page.display_option.as_deref(), Some("FitToWidth"));

    let v = run(&mut s, "page.add", json!({"display_name": "Cover", "index": 0}));
    assert_eq!(v["index"], 0);
    assert_eq!(report(&s).pages[0].display_name, "Cover");

    // Duplicates and bad values are rejected.
    for p in [
        json!({"display_name": "details"}),
        json!({"display_name": ""}),
        json!({"display_name": "X", "width": 0}),
        json!({"display_name": "X", "height": -5}),
        json!({"display_name": "X", "display_option": "Stretch"}),
        json!({"display_name": "X", "index": 9}),
        json!({"display_name": "X", "width": "wide"}),
        json!({"display_name": "X", "name": "abc"}),
    ] {
        fails(&mut s, "page.add", p);
    }

    let v = run(
        &mut s,
        "page.rename",
        json!({"page": "details", "display_name": "Detail view"}),
    );
    assert_eq!(v["previous"], "Details");
    fails(
        &mut s,
        "page.rename",
        json!({"page": "Detail view", "display_name": "Cover"}),
    );
    // Renaming a page to its own name (another case) is fine.
    run(
        &mut s,
        "page.rename",
        json!({"page": "Cover", "display_name": "COVER"}),
    );

    let v = run(&mut s, "page.move", json!({"page": "COVER", "index": 2}));
    assert_eq!(v["order"], json!(["Page 1", "Detail view", "COVER"]));
    fails(&mut s, "page.move", json!({"page": "COVER", "index": 3}));
    fails(&mut s, "page.move", json!({"page": "COVER"}));

    run(&mut s, "page.set_active", json!({"page": details}));
    assert_eq!(report(&s).active_page.as_deref(), Some(details.as_str()));
    fails(&mut s, "page.set_active", json!({"page": "nope"}));

    // Removing the active page activates the first one.
    let v = run(&mut s, "page.remove", json!({"page": "Detail view"}));
    assert_eq!(v["removed"], details);
    assert_eq!(v["active_page"], first);
    run(&mut s, "page.remove", json!({"page": "COVER"}));
    let e = fails(&mut s, "page.remove", json!({"page": first}));
    assert!(e.contains("at least one page"), "{e}");
}

#[test]
fn page_lookup_prefers_names_and_rejects_ambiguous_display_names() {
    let mut s = session();
    let first = first_page(&s);
    // A display name equal to another page's name: the name wins.
    run(&mut s, "page.add", json!({"display_name": first}));
    let v = run(&mut s, "page.set_active", json!({"page": first}));
    assert_eq!(v["display_name"], "Page 1");
    // Two pages with the same display name (from a file): lookups by it are ambiguous.
    s.report.as_mut().unwrap().pages[1].display_name = "Page 1".into();
    let e = fails(
        &mut s,
        "page.rename",
        json!({"page": "page 1", "display_name": "X"}),
    );
    assert!(e.contains("several pages"), "{e}");
}

#[test]
fn visuals_are_added_with_fields_resolved_against_the_model() {
    let mut s = session();
    let page = first_page(&s);
    let v = run(
        &mut s,
        "visual.add",
        json!({"page": "Page 1", "visual_type": "treemap", "x": 0, "y": 0, "width": 600, "height": 400,
        "fields": [
            {"role": "Values", "field": "product[name]"},
            {"role": "Values", "field": "[total]"},
            {"role": "Tooltips", "field": "Sales[Total]"},
            {"role": "Values", "field": "sum(Sales[Amount])"},
            {"role": "Values", "field": "Min('Date Table'[Date])"}
        ]}),
    );
    let name = v["visual"].as_str().unwrap();
    assert!(is_hex_name(name));
    assert_eq!(v["page"], page);
    assert_eq!(v["warnings"], json!([]));
    let texts: Vec<&str> = v["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["field"].as_str().unwrap())
        .collect();
    assert_eq!(
        texts,
        [
            "Product[Name]",
            "[Total]",
            "[Total]",
            "Sum(Sales[Amount])",
            "Min('Date Table'[Date])"
        ][..]
    );
    let visual = &report(&s).pages[0].visuals[0];
    assert_eq!(visual.visual_type.as_deref(), Some("treemap"));
    assert_eq!(
        visual.fields[1].field,
        FieldRef::Measure {
            table: "Sales".into(),
            measure: "Total".into()
        }
    );
    assert_eq!(visual.position.z, 0.0);
    assert_eq!(visual.position.tab_order, Some(0));

    // The next visual stacks on top.
    let (_, second) = chart(&mut s);
    let second = report(&s).pages[0].visual(&second).unwrap();
    assert_eq!(second.position.z, 1000.0);
    assert_eq!(second.position.tab_order, Some(1000));

    // Unknown fields are rejected with a reason, unless forced.
    for (field, reason) in [
        ("Sales[Nope]", "no column or measure"),
        ("Nope[Amount]", "no table"),
        ("[Nope]", "no measure"),
        ("[Amount]", "is a column"),
        ("Product[Total]", "is in table `Sales`"),
        ("Sum(Sales[Total])", "cannot be aggregated"),
        ("Foo(Sales[Amount])", "unknown aggregation"),
        ("Sales Amount", "is not a field"),
    ] {
        let e = fails(
            &mut s,
            "visual.add",
            json!({"page": page, "visual_type": "card", "x": 0, "y": 0, "width": 10, "height": 10,
                   "fields": [{"role": "Values", "field": field}]}),
        );
        assert!(e.contains(reason), "{field}: {e}");
    }
    let v = run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "card", "x": 0, "y": 0, "width": 10, "height": 10,
               "fields": [{"role": "Values", "field": "Sales[Nope]"}], "force": true}),
    );
    assert!(v["warnings"][0].as_str().unwrap().contains("force"), "{v}");
    // A parse error is never forced through.
    fails(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "card", "x": 0, "y": 0, "width": 10, "height": 10,
               "fields": [{"role": "Values", "field": "nonsense"}], "force": true}),
    );
}

#[test]
fn visual_add_checks_its_parameters() {
    let mut s = session();
    let ok = json!({"page": "Page 1", "visual_type": "card", "x": 0, "y": 0, "width": 100, "height": 50});
    let with = |k: &str, v: Value| {
        let mut p = ok.clone();
        p.as_object_mut().unwrap().insert(k.into(), v);
        p
    };
    for p in [
        with("page", json!("Nope")),
        with("visual_type", json!("")),
        with("visual_type", json!("bad type")),
        with("width", json!(0)),
        with("height", json!(1e9)),
        with("x", json!("left")),
        with("fields", json!({"role": "Values"})),
        with("fields", json!([{"role": "Values"}])),
        with("fields", json!([{"role": "Bad Role", "field": "[Total]"}])),
        with(
            "fields",
            json!([{"role": "Values", "field": "[Total]", "extra": 1}]),
        ),
        with(
            "fields",
            json!([{"role": "Values", "field": "[Total]"}, {"role": "values", "field": "[total]"}]),
        ),
        with("colour", json!("red")),
    ] {
        fails(&mut s, "visual.add", p);
    }
    let mut missing = ok.clone();
    missing.as_object_mut().unwrap().remove("x");
    fails(&mut s, "visual.add", missing);

    // A custom visual type and a visual off the page are accepted with warnings.
    let v = run(
        &mut s,
        "visual.add",
        with("visual_type", json!("myCustomVisual1234")),
    );
    assert!(
        v["warnings"][0].as_str().unwrap().contains("custom visual"),
        "{v}"
    );
    let v = run(&mut s, "visual.add", with("x", json!(1250)));
    assert!(v["warnings"][0].as_str().unwrap().contains("does not fit"), "{v}");
}

#[test]
fn visuals_move_bind_unbind_and_go() {
    let mut s = session();
    let (page, visual) = chart(&mut s);
    s.report_dirty = false;

    let v = run(
        &mut s,
        "visual.set_position",
        json!({"page": page, "visual": visual, "x": 50, "height": 200}),
    );
    assert_eq!(v["position"]["x"], 50.0);
    assert_eq!(v["position"]["y"], 20.0);
    assert_eq!(v["position"]["height"], 200.0);
    assert!(s.report_dirty);
    fails(
        &mut s,
        "visual.set_position",
        json!({"page": page, "visual": visual}),
    );
    fails(
        &mut s,
        "visual.set_position",
        json!({"page": page, "visual": visual, "width": -1}),
    );
    fails(
        &mut s,
        "visual.set_position",
        json!({"page": page, "visual": "nope", "x": 1}),
    );

    // Bind: appended to its role, or at an index inside the role.
    run(
        &mut s,
        "visual.bind",
        json!({"page": page, "visual": visual, "role": "Y", "field": "Sum(Sales[Amount])"}),
    );
    let v = run(
        &mut s,
        "visual.bind",
        json!({"page": page, "visual": visual, "role": "y", "field": "Count(Sales[Product])", "index": 0}),
    );
    let order: Vec<(String, String)> = v["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["role"].as_str().unwrap().into(),
                f["field"].as_str().unwrap().into(),
            )
        })
        .collect();
    assert_eq!(
        order,
        [
            ("Category".to_string(), "Product[Name]".to_string()),
            ("Y".into(), "Count(Sales[Product])".into()),
            ("Y".into(), "[Total]".into()),
            ("Y".into(), "Sum(Sales[Amount])".into()),
        ]
    );
    let e = fails(
        &mut s,
        "visual.bind",
        json!({"page": page, "visual": visual, "role": "Y", "field": "[Total]"}),
    );
    assert!(e.contains("already bound"), "{e}");
    fails(
        &mut s,
        "visual.bind",
        json!({"page": page, "visual": visual, "role": "Y", "field": "[Nope]"}),
    );
    fails(
        &mut s,
        "visual.bind",
        json!({"page": page, "visual": visual, "role": "Legend", "field": "Product[Name]", "index": 1}),
    );

    // Unbind one field, then a whole role.
    let v = run(
        &mut s,
        "visual.unbind",
        json!({"page": page, "visual": visual, "role": "Y", "field": "sales[total]"}),
    );
    assert_eq!(v["removed"], json!(["[Total]"]));
    fails(
        &mut s,
        "visual.unbind",
        json!({"page": page, "visual": visual, "role": "Y", "field": "[Total]"}),
    );
    let v = run(
        &mut s,
        "visual.unbind",
        json!({"page": page, "visual": visual, "role": "Y"}),
    );
    assert_eq!(v["removed"].as_array().unwrap().len(), 2);
    fails(
        &mut s,
        "visual.unbind",
        json!({"page": page, "visual": visual, "role": "Y"}),
    );

    let v = run(&mut s, "visual.remove", json!({"page": page, "visual": visual}));
    assert_eq!(v["removed"], json!([visual]));
    assert_eq!(v["remaining_visuals"], 0);
    fails(&mut s, "visual.remove", json!({"page": page, "visual": visual}));
}

#[test]
fn removing_a_group_removes_its_members() {
    let mut s = session();
    let (page, a) = chart(&mut s);
    let (_, b) = chart(&mut s);
    let (_, c) = chart(&mut s);
    {
        let p = &mut s.report.as_mut().unwrap().pages[0];
        p.visuals.push(bicraft_report::Visual {
            name: "group1".into(),
            ..Default::default()
        });
        p.visuals.push(bicraft_report::Visual {
            name: "group2".into(),
            parent_group: Some("group1".into()),
            ..Default::default()
        });
        p.visual_mut(&a).unwrap().parent_group = Some("group1".into());
        p.visual_mut(&b).unwrap().parent_group = Some("group2".into());
    }
    let e = fails(
        &mut s,
        "visual.bind",
        json!({"page": page, "visual": "group1", "role": "Y", "field": "[Total]"}),
    );
    assert!(e.contains("group"), "{e}");
    let v = run(&mut s, "visual.remove", json!({"page": page, "visual": "GROUP1"}));
    assert_eq!(v["removed"].as_array().unwrap().len(), 4);
    let left: Vec<&str> = report(&s).pages[0]
        .visuals
        .iter()
        .map(|v| v.name.as_str())
        .collect();
    assert_eq!(left, [c.as_str()]);
}

#[test]
fn summary_lists_pages_visuals_types_and_fields() {
    let mut s = session();
    chart(&mut s);
    chart(&mut s);
    run(&mut s, "page.add", json!({"display_name": "Empty"}));
    let v = run(&mut s, "report.summary", json!({}));
    assert_eq!(v["page_count"], 2);
    assert_eq!(v["visual_count"], 2);
    assert_eq!(v["visual_types"], json!({"clusteredColumnChart": 2}));
    assert_eq!(v["fields"], json!(["Product[Name]", "[Total]"]));
    assert_eq!(v["pages"][0]["active"], true);
    assert_eq!(v["pages"][0]["visuals"][0]["fields"][0]["role"], "Category");
    assert_eq!(v["pages"][0]["visuals"][0]["fields"][1]["kind"], "measure");
    assert_eq!(v["pages"][0]["visuals"][0]["fields"][1]["table"], "Sales");

    let v = run(
        &mut s,
        "report.summary",
        json!({"page": "empty", "visuals": false}),
    );
    assert_eq!(v["pages"].as_array().unwrap().len(), 1);
    assert_eq!(v["visual_count"], 0);
    assert!(v["pages"][0].get("visuals").is_none());
    assert!(s.execute("report.summary", &json!({"page": "nope"})).is_err());
}

#[test]
fn validate_cross_checks_fields_and_filters_against_the_model() {
    let mut s = session();
    let (page, visual) = chart(&mut s);
    let v = run(&mut s, "report.validate", json!({}));
    assert_eq!(v["valid"], true, "{v}");
    assert_eq!(v["checked_against_model"], true);
    assert_eq!(v["error_count"], 0);

    // Break a binding and add filters referencing unknown fields.
    s.model
        .as_mut()
        .unwrap()
        .table_mut("Sales")
        .unwrap()
        .measures
        .clear();
    {
        let r = s.report.as_mut().unwrap();
        r.filter_config = Some(json!({"filters": [{"name": "f1",
            "field": {"Column": {"Expression": {"SourceRef": {"Entity": "Product"}}, "Property": "Colour"}}}]}));
        r.pages[0].visuals[0].filter_config = Some(json!({"filters": [{"filter": {
            "From": [{"Name": "s", "Entity": "Sales"}],
            "Where": [{"Condition": {"Comparison": {"Left": {"Column": {"Expression": {"SourceRef": {"Source": "s"}}, "Property": "Amount"}}}}},
                      {"Condition": {"Comparison": {"Left": {"Measure": {"Expression": {"SourceRef": {"Source": "s"}}, "Property": "Margin"}}}}}]}}]}));
        // The raw query mirrors the typed binding: not reported twice.
        r.pages[0].visuals[0].visual_extra.insert(
            "query".into(),
            json!({"queryState": {"Y": {"projections": [{"field": {"Measure": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Total"}}}]}}}),
        );
        // Report-level measures (`Schema: extension`) are not model objects; this one exists in
        // reportExtensions.json.
        r.resources.push(bicraft_report::Resource {
            path: "definition/reportExtensions.json".into(),
            bytes: br#"{"name": "extension", "entities": [{"name": "Sales", "measures": [{"name": "Local", "dataType": "Double", "expression": "1"}]}]}"#.to_vec(),
        });
        r.pages[0].extra.insert(
            "objects".into(),
            json!({"x": {"Measure": {"Expression": {"SourceRef": {"Schema": "extension", "Entity": "Sales"}}, "Property": "Local"}}}),
        );
        r.bookmarks.push(bicraft_report::NamedDocument {
            name: "bm".into(),
            document: json!({"Column": {"Expression": {"SourceRef": {"Entity": "Gone"}}, "Property": "X"}}),
        });
    }
    let v = run(&mut s, "report.validate", json!({}));
    assert_eq!(v["valid"], false);
    let issues = v["issues"].as_array().unwrap();
    let messages: Vec<String> = issues
        .iter()
        .map(|i| format!("{} {} {}", i["severity"], i["field"], i["message"]))
        .collect();
    // The hand-written filters are not schema-complete: those findings are `schema_issues`.
    let errors = issues.iter().filter(|i| i["severity"] == "error").count();
    let warnings = issues.iter().filter(|i| i["severity"] == "warning").count();
    assert_eq!((errors, warnings), (3, 1), "{messages:#?}");
    let schema = v["schema_issues"].as_array().unwrap().len();
    assert_eq!(v["error_count"], 3 + schema);
    let total = issues.iter().find(|i| i["field"] == "[Total]").unwrap();
    assert_eq!(total["page"], "Page 1");
    assert_eq!(total["visual"], visual);
    assert_eq!(total["role"], "Y");
    assert!(total["message"].as_str().unwrap().contains("no measure"));
    assert!(
        issues
            .iter()
            .any(|i| i["field"] == "Product[Colour]" && i["part"] == "filters" && i["page"].is_null())
    );
    assert!(
        issues
            .iter()
            .any(|i| i["field"] == "[Margin]" && i["visual"] == visual)
    );
    assert!(
        issues
            .iter()
            .any(|i| i["field"] == "Gone[X]" && i["severity"] == "warning")
    );
    assert!(!issues.iter().any(|i| i["field"] == "Sales[Amount]"));
    // Validation never changes the report.
    assert_eq!(report(&s).pages[0].name, page);

    // Structural problems.
    {
        let r = s.report.as_mut().unwrap();
        r.active_page = Some("missing".into());
        let dup = r.pages[0].visuals[0].clone();
        r.pages[0].visuals.push(dup);
    }
    let v = run(&mut s, "report.validate", json!({}));
    let text = v["issues"].to_string();
    assert!(text.contains("active page `missing`"), "{text}");
    assert!(text.contains("duplicate visual name"), "{text}");

    // Without a model, fields are not checked.
    s.model = None;
    s.model_error = Some("binary".into());
    let v = run(&mut s, "report.validate", json!({}));
    assert_eq!(v["checked_against_model"], false);
    assert!(v["note"].as_str().unwrap().contains("binary"));
}

#[test]
fn model_removals_report_broken_bindings() {
    let mut s = session();
    chart(&mut s);
    s.report_dirty = false;
    let v = run(&mut s, "measure.remove", json!({"name": "Total"}));
    assert_eq!(v["report"]["affected_bindings"][0]["field"], "[Total]");
    assert!(v["report"]["hint"].as_str().unwrap().contains("visual.unbind"));
    let v = run(
        &mut s,
        "column.remove",
        json!({"table": "Product", "name": "Name"}),
    );
    assert_eq!(v["report"]["affected_binding_count"], 1);
    let v = run(&mut s, "table.remove", json!({"name": "Product"}));
    assert_eq!(v["report"]["affected_binding_count"], 1);
    // Removals never change the report.
    assert!(!s.report_dirty);
    assert_eq!(report(&s).pages[0].visuals[0].fields.len(), 2);
    // update_report is only accepted where it can do something.
    assert!(
        s.execute("measure.remove", &json!({"name": "X", "update_report": true}))
            .unwrap_err()
            .contains("unknown parameter")
    );
}

#[test]
fn update_report_without_a_report_is_a_warning() {
    let mut s = session();
    s.report = None;
    let v = run(
        &mut s,
        "table.rename",
        json!({"name": "Product", "new_name": "Item", "update_report": true}),
    );
    assert!(v.get("report").is_none());
    assert!(v["warnings"][0].as_str().unwrap().contains("no report"));
}

#[test]
fn fields_without_a_model_are_bound_unchecked() {
    let mut s = session();
    s.model = None;
    let v = run(
        &mut s,
        "visual.add",
        json!({"page": "Page 1", "visual_type": "card", "x": 0, "y": 0, "width": 10, "height": 10,
               "fields": [{"role": "Values", "field": "Sales[Anything]"}]}),
    );
    assert!(v["warnings"][0].as_str().unwrap().contains("not checked"), "{v}");
    let e = fails(
        &mut s,
        "visual.bind",
        json!({"page": "Page 1", "visual": v["visual"], "role": "Values", "field": "[Total]"}),
    );
    assert!(e.contains("no model is open"), "{e}");
}

#[test]
fn saving_a_report_to_a_bim_warns() {
    let dir = temp("bim");
    let mut s = session();
    let v = run(&mut s, "file.save_as", json!({"path": dir.join("model.bim")}));
    assert!(v["warnings"][0].as_str().unwrap().contains("no report"), "{v}");
    assert!(s.report.is_some());
    let v = run(&mut s, "file.save", json!({}));
    assert!(v["warnings"][0].as_str().unwrap().contains("no report"), "{v}");
    assert!(s.report_dirty);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Opening a project reads its report; a report that cannot be read is reported, never fatal.
#[test]
fn opening_a_project_reads_the_report_or_explains_why_not() {
    let dir = temp("open");
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(
        &mut s,
        "project.new",
        json!({"dir": dir.path(), "name": "Shop", "format": "tmsl"}),
    );
    assert!(!s.report_dirty);
    let mut fresh = Session::new();
    let info = run(&mut fresh, "file.open", json!({"path": dir.join("Shop.pbip")}));
    assert!(fresh.model.is_some());
    assert!(fresh.report.is_some() != fresh.report_error.is_some(), "{info}");
    assert_eq!(info["report_error"].is_null(), fresh.report.is_some());
    // A missing report folder is explained.
    std::fs::remove_dir_all(dir.join("Shop.Report")).unwrap();
    let info = run(&mut fresh, "file.open", json!({"path": dir.join("Shop.pbip")}));
    assert!(fresh.report.is_none());
    assert!(
        info["report_error"].as_str().unwrap().contains("Report"),
        "{info}"
    );
    // report.new would replace an unreadable project report on save: refused.
    fresh.report_error = Some("unreadable".into());
    let e = fresh.execute("report.new", &json!({})).unwrap_err();
    assert!(e.contains("refused"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_new_project_saves_and_reopens_the_report() {
    let dir = temp("roundtrip");
    let mut s = session();
    let (page, visual) = chart(&mut s);
    run(&mut s, "page.add", json!({"display_name": "Second"}));
    run(
        &mut s,
        "project.new",
        json!({"dir": dir.path(), "name": "Shop", "format": "tmsl"}),
    );
    assert!(!s.report_dirty);

    let mut fresh = Session::new();
    run(&mut fresh, "file.open", json!({"path": dir.join("Shop.pbip")}));
    assert_eq!(fresh.report_error, None);
    let r = report(&fresh);
    assert_eq!(r.pages.len(), 2);
    let v = r.page(&page).unwrap().visual(&visual).unwrap();
    assert_eq!(v.fields, report(&s).pages[0].visuals[0].fields);
    assert_eq!(run(&mut fresh, "report.validate", json!({}))["valid"], true);

    // Edit and save in place; the edit survives a reopen.
    run(&mut fresh, "page.remove", json!({"page": "Second"}));
    run(
        &mut fresh,
        "visual.unbind",
        json!({"page": page, "visual": visual, "role": "Y"}),
    );
    let v = run(&mut fresh, "file.save", json!({}));
    assert_eq!(v["report_saved"], true);
    assert!(!fresh.report_dirty);
    let mut again = Session::new();
    run(&mut again, "file.open", json!({"path": dir.join("Shop.pbip")}));
    let r = report(&again);
    assert_eq!(r.pages.len(), 1);
    assert_eq!(r.pages[0].visuals[0].fields.len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_model_only_project_gets_the_blank_report_in_memory() {
    let dir = temp("blank");
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    let info = run(
        &mut s,
        "project.new",
        json!({"dir": dir.path(), "name": "Shop", "format": "tmsl"}),
    );
    assert_eq!(info["report"], json!({"pages": 1, "visuals": 0}));
    run(&mut s, "page.add", json!({"display_name": "Two"}));
    run(&mut s, "file.save", json!({}));
    let mut fresh = Session::new();
    run(&mut fresh, "file.open", json!({"path": dir.join("Shop.pbip")}));
    assert_eq!(report(&fresh).pages.len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn saving_a_legacy_project_converts_its_report() {
    let dir = temp("legacy");
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(
        &mut s,
        "project.new",
        json!({"dir": dir.path(), "name": "Shop", "format": "tmsl"}),
    );
    // Turn the project into a legacy one: report.json and a version 1.0 definition.pbir.
    let report_dir = dir.join("Shop.Report");
    std::fs::remove_dir_all(report_dir.join("definition")).unwrap();
    std::fs::write(
        report_dir.join("report.json"),
        r#"{"config":"{}","layoutOptimization":0,"sections":[{"name":"ReportSection","displayName":"Page 1","width":1280,"height":720,"displayOption":1,"visualContainers":[],"config":"{}","filters":"[]"}]}"#,
    )
    .unwrap();
    std::fs::write(
        report_dir.join("definition.pbir"),
        r#"{"version": "1.0", "datasetReference": {"byPath": {"path": "../Shop.SemanticModel"}}}"#,
    )
    .unwrap();
    let mut legacy = Session::new();
    run(&mut legacy, "file.open", json!({"path": dir.join("Shop.pbip")}));
    assert_eq!(legacy.report_error, None);
    run(&mut legacy, "page.add", json!({"display_name": "New"}));
    let v = run(&mut legacy, "file.save", json!({}));
    assert_eq!(v["report_converted_to_pbir"], true);
    assert!(!report_dir.join("report.json").exists());
    assert!(matches!(
        &legacy.source,
        Some(Source::Project { project, .. }) if project.report_format == bicraft_pbip::ReportFormat::Pbir
    ));
    let mut fresh = Session::new();
    run(&mut fresh, "file.open", json!({"path": dir.join("Shop.pbip")}));
    assert_eq!(report(&fresh).pages.len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}
