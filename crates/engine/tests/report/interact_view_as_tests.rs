//! View as (`view.interact {action: "view_as"}`, Desktop's Modeling › View as): the roles' row
//! filters reach every evaluation (visuals, `dax.evaluate`, `table.rows`, `visual.records`),
//! through the relationships, united over several roles, with `USERPRINCIPALNAME()` as the
//! "Other user"; the model, the report, the revision and the dirty flags never change.

use serde_json::{Value, json};

use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Sales(Product, Region, Amount) many-to-one Product(Product, Category) and Region(Region,
/// Manager). Total 1132: East 105 (Bike 100, Ball 5), West 1027 (Car 1000, Kite 7, Ball 20);
/// Kite has no product row. Roles: East (Region = East), Toys (Category = Toys), Cars (Product =
/// Car), Managers (Manager = USERPRINCIPALNAME()), Custom (a filter bicraft cannot evaluate).
fn shop() -> Session {
    let mut s = Session::new();
    let sales = r#"#table({"Product", "Region", "Amount"}, {{"Bike", "East", 100}, {"Car", "West", 1000}, {"Ball", "East", 5}, {"Kite", "West", 7}, {"Ball", "West", 20}})"#;
    let product =
        r#"#table({"Product", "Category"}, {{"Bike", "Wheels"}, {"Car", "Wheels"}, {"Ball", "Toys"}})"#;
    let region =
        r#"#table({"Region", "Manager"}, {{"East", "alice@contoso.test"}, {"West", "bob@contoso.test"}})"#;
    let col = |t: &str, n: &str, ty: &str| ("column.add", json!({"table": t, "name": n, "data_type": ty}));
    let steps = [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "table.add",
            json!({"name": "Product", "partition": {"source_type": "m", "expression": product}}),
        ),
        (
            "table.add",
            json!({"name": "Region", "partition": {"source_type": "m", "expression": region}}),
        ),
        col("Sales", "Product", "string"),
        col("Sales", "Region", "string"),
        col("Sales", "Amount", "double"),
        col("Product", "Product", "string"),
        col("Product", "Category", "string"),
        col("Region", "Region", "string"),
        col("Region", "Manager", "string"),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Every row", "expression": "CALCULATE(SUM(Sales[Amount]), ALL(Sales))"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Regions", "expression": "COUNTROWS(Region)"}),
        ),
        (
            "relationship.add",
            json!({"from": "Sales[Product]", "to": "Product[Product]"}),
        ),
        (
            "relationship.add",
            json!({"from": "Sales[Region]", "to": "Region[Region]"}),
        ),
        ("role.add", json!({"name": "East"})),
        (
            "role.set_filter",
            json!({"role": "East", "table": "Region", "filter_expression": "[Region] = \"East\""}),
        ),
        ("role.add", json!({"name": "Toys"})),
        (
            "role.set_filter",
            json!({"role": "Toys", "table": "Product", "filter_expression": "Product[Category] = \"Toys\""}),
        ),
        ("role.add", json!({"name": "Cars"})),
        (
            "role.set_filter",
            json!({"role": "Cars", "table": "Product", "filter_expression": "[Product] = \"Car\""}),
        ),
        ("role.add", json!({"name": "Managers"})),
        (
            "role.set_filter",
            json!({"role": "Managers", "table": "Region", "filter_expression": "[Manager] = USERPRINCIPALNAME()"}),
        ),
        ("role.add", json!({"name": "Custom"})),
        (
            "role.set_filter",
            json!({"role": "Custom", "table": "Region", "filter_expression": "[Manager] = CUSTOMDATA()"}),
        ),
        ("report.new", json!({})),
    ];
    for (id, p) in steps {
        run(&mut s, id, p);
    }
    s
}

fn card(s: &mut Session, measure: &str) -> String {
    let page = s
        .report
        .as_ref()
        .map(|r| r.pages[0].name.clone())
        .unwrap_or_default();
    let r = run(
        s,
        "visual.add",
        json!({"page": page, "visual_type": "card", "x": 0, "y": 0, "width": 100, "height": 100,
               "fields": [{"role": "Values", "field": format!("[{measure}]")}]}),
    );
    r["visual"].as_str().unwrap_or_default().to_string()
}

fn visual_data(snap: &Value, visual: &str) -> Value {
    for p in snap["report"]["pages"].as_array().cloned().unwrap_or_default() {
        for v in p["visuals"].as_array().cloned().unwrap_or_default() {
            if v["name"] == visual {
                return v["data"].clone();
            }
        }
    }
    panic!("visual {visual} not in the snapshot")
}

fn value(snap: &Value, visual: &str) -> f64 {
    let d = visual_data(snap, visual);
    assert_eq!(d["status"], "ok", "{d}");
    d["rows"][0][0]
        .as_f64()
        .unwrap_or_else(|| panic!("no number in {d}"))
}

fn view_as(s: &mut Session, roles: Value, user: Value) -> Value {
    run(
        s,
        "view.interact",
        json!({"action": "view_as", "roles": roles, "user": user}),
    )
}

fn scalar(s: &mut Session, query: &str) -> Value {
    let r = run(s, "dax.evaluate", json!({ "query": query }));
    let v = r
        .pointer("/results/0/rows/0")
        .and_then(Value::as_object)
        .and_then(|o| o.values().next())
        .cloned()
        .unwrap_or(Value::Null);
    assert_ne!(v, Value::Null, "{r}");
    v
}

#[test]
fn a_role_filters_every_visual_and_all_does_not_escape_it() {
    let mut s = shop();
    let total = card(&mut s, "Total");
    let every = card(&mut s, "Every row");
    let regions = card(&mut s, "Regions");
    let snap = s.snapshot();
    assert_eq!(value(&snap, &total), 1132.0);
    assert_eq!(value(&snap, &regions), 2.0);
    view_as(&mut s, json!(["East"]), Value::Null);
    let snap = s.snapshot();
    assert_eq!(value(&snap, &total), 105.0);
    // ALL removes filters, never row-level security
    assert_eq!(value(&snap, &every), 105.0);
    assert_eq!(value(&snap, &regions), 1.0);
    assert_eq!(
        snap["interaction"]["view_as"],
        json!({"roles": ["East"], "user": null})
    );
    // stopped: the data as stored again
    view_as(&mut s, Value::Null, Value::Null);
    let snap = s.snapshot();
    assert_eq!(value(&snap, &total), 1132.0);
    assert_eq!(snap["interaction"]["view_as"], Value::Null);
}

#[test]
fn several_roles_unite_their_rows() {
    let mut s = shop();
    let total = card(&mut s, "Total");
    view_as(&mut s, json!(["Toys"]), Value::Null);
    // Ball East 5 + Ball West 20; Kite (no product row) is not kept
    assert_eq!(value(&s.snapshot(), &total), 25.0);
    view_as(&mut s, json!(["East", "Toys"]), Value::Null);
    // East's 105 and Toys' West 20
    assert_eq!(value(&s.snapshot(), &total), 125.0);
}

#[test]
fn filters_cross_relationships_back_only_with_both_directions() {
    let mut s = shop();
    let regions = card(&mut s, "Regions");
    let total = card(&mut s, "Total");
    view_as(&mut s, json!(["Cars"]), Value::Null);
    let snap = s.snapshot();
    assert_eq!(value(&snap, &total), 1000.0);
    // Product filters Sales; Sales does not filter Region one way
    assert_eq!(value(&snap, &regions), 2.0);
    run(
        &mut s,
        "relationship.set",
        json!({"from": "Sales[Region]", "to": "Region[Region]", "cross_filter": "both", "security_filtering": "both"}),
    );
    // both directions: the regions of the Car rows only
    assert_eq!(value(&s.snapshot(), &regions), 1.0);
}

#[test]
fn other_user_is_what_userprincipalname_returns() {
    let mut s = shop();
    let total = card(&mut s, "Total");
    view_as(&mut s, json!(["Managers"]), json!("bob@contoso.test"));
    assert_eq!(value(&s.snapshot(), &total), 1027.0);
    assert_eq!(
        scalar(&mut s, "EVALUATE ROW(\"u\", USERPRINCIPALNAME())"),
        json!("bob@contoso.test")
    );
    assert_eq!(
        scalar(&mut s, "EVALUATE ROW(\"u\", USERNAME())"),
        json!("bob@contoso.test")
    );
    view_as(&mut s, json!(["Managers"]), json!("alice@contoso.test"));
    assert_eq!(value(&s.snapshot(), &total), 105.0);
    // Other user alone: no role filter, the user still answers
    view_as(&mut s, json!([]), json!("carol@contoso.test"));
    assert_eq!(value(&s.snapshot(), &total), 1132.0);
    assert_eq!(
        scalar(&mut s, "EVALUATE ROW(\"u\", USERPRINCIPALNAME())"),
        json!("carol@contoso.test")
    );
}

#[test]
fn dax_queries_the_data_view_and_see_records_are_filtered_too() {
    let mut s = shop();
    let total = card(&mut s, "Total");
    let page = s
        .report
        .as_ref()
        .map(|r| r.pages[0].name.clone())
        .unwrap_or_default();
    view_as(&mut s, json!(["East"]), Value::Null);
    assert_eq!(
        scalar(&mut s, "EVALUATE ROW(\"n\", COUNTROWS(ALL(Sales)))").as_f64(),
        Some(2.0)
    );
    let r = run(&mut s, "table.rows", json!({"table": "Sales"}));
    assert_eq!(r["rows"].as_array().map(Vec::len), Some(2), "{r}");
    let r = run(&mut s, "table.rows", json!({"table": "Region"}));
    assert_eq!(r["rows"].as_array().map(Vec::len), Some(1), "{r}");
    let r = run(&mut s, "visual.records", json!({"page": page, "visual": total}));
    assert_eq!(r["rows"].as_array().map(Vec::len), Some(2), "{r}");
}

#[test]
fn view_as_is_view_state_and_refuses_unknown_roles() {
    let mut s = shop();
    let total = card(&mut s, "Total");
    let (revision, dirty, report_dirty) = (s.revision, s.dirty, s.report_dirty);
    let model = serde_json::to_value(s.model.as_ref()).unwrap();
    let report = format!("{:?}", s.report);
    let r = view_as(&mut s, json!(["east"]), Value::Null);
    assert_eq!(r["changed"], true);
    // the model's spelling
    assert_eq!(r["state"]["view_as"]["roles"], json!(["East"]));
    assert_eq!(view_as(&mut s, json!(["East"]), Value::Null)["changed"], false);
    let err = s
        .execute(
            "view.interact",
            &json!({"action": "view_as", "roles": ["East", "Nobody"]}),
        )
        .unwrap_err();
    assert!(err.contains("Nobody"), "{err}");
    assert!(
        s.execute("view.interact", &json!({"action": "view_as", "roles": "East"}))
            .is_err()
    );
    assert!(
        s.execute(
            "view.interact",
            &json!({"action": "view_as", "roles": [], "user": 3})
        )
        .is_err()
    );
    // the refused calls kept the state
    assert_eq!(value(&s.snapshot(), &total), 105.0);
    assert_eq!(
        (s.revision, s.dirty, s.report_dirty),
        (revision, dirty, report_dirty)
    );
    assert_eq!(serde_json::to_value(s.model.as_ref()).unwrap(), model);
    assert_eq!(format!("{:?}", s.report), report);
    // reset stops viewing
    let r = run(&mut s, "view.interact", json!({"action": "reset"}));
    assert_eq!(r["changed"], true);
    assert_eq!(r["state"]["view_as"], Value::Null);
    assert_eq!(value(&s.snapshot(), &total), 1132.0);
}

#[test]
fn a_filter_that_cannot_be_evaluated_fails_closed() {
    let mut s = shop();
    let total = card(&mut s, "Total");
    view_as(&mut s, json!(["Custom"]), Value::Null);
    let d = visual_data(&s.snapshot(), &total);
    assert_ne!(d["status"], "ok", "{d}");
    assert!(d.to_string().contains("View as"), "{d}");
    assert!(
        s.execute("dax.evaluate", &json!({"query": "EVALUATE Sales"}))
            .is_err_and(|e| e.contains("View as"))
    );
}

#[test]
fn each_viewer_has_its_own_view_as() {
    // a server keeps one view state per viewer over a shared session
    let mut s = shop();
    let total = card(&mut s, "Total");
    let mut a = bicraft_engine::Interactions::default();
    let b = bicraft_engine::Interactions::default();
    s.interact_view(&mut a, &json!({"action": "view_as", "roles": ["East"]}))
        .unwrap();
    assert_eq!(value(&s.snapshot_view(&a), &total), 105.0);
    assert_eq!(value(&s.snapshot_view(&b), &total), 1132.0);
    assert_eq!(value(&s.snapshot(), &total), 1132.0);
    // and the session's View as does not reach a viewer without one
    view_as(&mut s, json!(["Toys"]), Value::Null);
    assert_eq!(value(&s.snapshot_view(&b), &total), 1132.0);
    assert_eq!(value(&s.snapshot_view(&a), &total), 105.0);
    assert_eq!(value(&s.snapshot(), &total), 25.0);
}
