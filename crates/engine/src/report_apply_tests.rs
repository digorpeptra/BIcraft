//! Tests of declarative report building: `report.apply`, `report.check`, `report.extract`.

use bicraft_report::{FieldRef, Report, Visual};
use serde_json::{Value, json};

use crate::Session;
use crate::report_apply::{page_name_for_key, stable_name, visual_name_for_key};

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Runs a command that must fail and checks that neither the report nor its dirty flag changed.
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

fn report(s: &Session) -> &Report {
    s.report.as_ref().unwrap()
}

/// Sales(Amount, Product) with [Total], Product(Name), and a blank one-page report.
fn session() -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        ("table.add", json!({"name": "Sales"})),
        ("table.add", json!({"name": "Product"})),
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
            json!({"table": "Product", "name": "Name", "data_type": "string"}),
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

/// Two pages: an overview with a card and a chart, and a detail page with a table.
fn spec() -> Value {
    json!({"pages": [
        {"key": "overview", "display_name": "Overview", "visuals": [
            {"key": "total", "type": "card", "x": 20, "y": 20, "width": 200, "height": 120,
             "fields": [{"role": "Values", "field": "[Total]"}]},
            {"key": "by-product", "type": "clusteredColumnChart", "x": 240, "y": 20, "width": 600,
             "height": 400, "fields": [{"role": "Category", "field": "Product[Name]"},
                                       {"role": "Y", "field": "[Total]"}]}
        ]},
        {"key": "detail", "display_name": "Detail", "width": 1920, "height": 1080, "visuals": [
            {"key": "table", "type": "tableEx", "x": 0, "y": 0, "width": 1920, "height": 1080,
             "fields": [{"role": "Values", "field": "Product[Name]"},
                        {"role": "Values", "field": "Sum(Sales[Amount])"}]}
        ]}
    ]})
}

fn apply(s: &mut Session, spec: &Value) -> Value {
    run(s, "report.apply", json!({"spec": spec}))
}

/// Applies `spec` once more and checks it is a no-op: no change, same report, same PBIR bytes,
/// dirty flag untouched.
fn assert_noop(s: &mut Session, spec: &Value, params: Value) {
    let before = s.report.clone();
    let docs = bicraft_pbir::write_documents(report(s));
    let dirty = s.report_dirty;
    let mut p = params;
    p["spec"] = spec.clone();
    let out = run(s, "report.apply", p);
    assert_eq!(out["change_count"], 0, "{out}");
    assert_eq!(out["applied"], false);
    assert_eq!(s.report, before);
    assert_eq!(bicraft_pbir::write_documents(report(s)), docs);
    assert_eq!(s.report_dirty, dirty);
}

fn page<'a>(s: &'a Session, key: &str) -> &'a bicraft_report::Page {
    let name = page_name_for_key(key);
    report(s)
        .pages
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no page for key {key}"))
}

fn visual<'a>(s: &'a Session, page_key: &str, key: &str) -> &'a Visual {
    let p = page(s, page_key);
    let name = visual_name_for_key(&p.name, key);
    p.visual(&name)
        .unwrap_or_else(|| panic!("no visual for key {page_key}/{key}"))
}

#[test]
fn stable_names_are_fixed_twenty_hex_digits() {
    for input in ["", "overview", "overview/total", "é/日本"] {
        let n = stable_name(input);
        assert_eq!(n.len(), 20, "{n}");
        assert!(
            n.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "{n}"
        );
        assert_eq!(stable_name(input), n);
    }
    assert_ne!(stable_name("a"), stable_name("b"));
    assert_ne!(stable_name("overview/total"), stable_name("overview/totals"));
    // Pinned values: the names must never change between versions or platforms.
    assert_eq!(stable_name(""), STABLE_EMPTY);
    assert_eq!(stable_name("overview"), STABLE_OVERVIEW);
    assert_eq!(visual_name_for_key("p", "v"), stable_name("p/v"));
    assert_eq!(page_name_for_key("overview"), stable_name("overview"));
}

const STABLE_EMPTY: &str = "f52a15e9a9b5e89be9d3";
const STABLE_OVERVIEW: &str = "ccdf2dfcf7cae1b83b51";

#[test]
fn apply_creates_then_reapplying_changes_nothing() {
    let mut s = session();
    let blank = report(&s).pages[0].name.clone();
    let out = apply(&mut s, &spec());
    assert_eq!(out["applied"], true);
    assert_eq!(out["summary"]["pages_created"], 2, "{out}");
    assert_eq!(out["summary"]["visuals_created"], 3);
    assert_eq!(out["warning_count"], 0, "{out}");
    let r = report(&s);
    // The blank page stays first (not listed, not pruned); new pages follow in spec order.
    let names: Vec<&str> = r.pages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        [
            blank.as_str(),
            &page_name_for_key("overview"),
            &page_name_for_key("detail")
        ]
    );
    let card = visual(&s, "overview", "total");
    assert_eq!(card.visual_type.as_deref(), Some("card"));
    assert_eq!(
        card.fields[0].field,
        FieldRef::Measure {
            table: "Sales".into(),
            measure: "Total".into()
        }
    );
    let chart = visual(&s, "overview", "by-product");
    assert!(chart.position.z > card.position.z);
    assert!(chart.position.tab_order > card.position.tab_order);
    let detail = page(&s, "detail");
    assert_eq!((detail.width, detail.height), (1920.0, 1080.0));
    // ids map every key to its name.
    assert_eq!(out["ids"][0]["key"], "overview");
    assert_eq!(out["ids"][0]["page"], page_name_for_key("overview"));
    assert_eq!(out["ids"][0]["visuals"][1]["key"], "by-product");

    s.report_dirty = false;
    assert_noop(&mut s, &spec(), json!({}));
    assert_eq!(s.report.as_ref().map(|r| r.pages.len()), Some(3));
    // check agrees: in sync.
    let check = run(&mut s, "report.check", json!({"spec": spec()}));
    assert_eq!(check["in_sync"], true, "{check}");
}

#[test]
fn apply_updates_only_what_differs() {
    let mut s = session();
    apply(&mut s, &spec());
    let before_card = visual(&s, "overview", "total").clone();
    let mut changed = spec();
    changed["pages"][0]["display_name"] = json!("Summary");
    changed["pages"][0]["visuals"][1]["width"] = json!(500);
    changed["pages"][0]["visuals"][1]["fields"] = json!([{"role": "Category", "field": "Sales[Product]"},
                                                         {"role": "Y", "field": "[Total]"}]);
    let out = apply(&mut s, &changed);
    assert_eq!(out["change_count"], 2, "{out}");
    let changes = out["changes"].as_array().unwrap();
    assert_eq!(changes[0]["action"], "update");
    assert_eq!(changes[0]["kind"], "page");
    assert_eq!(changes[0]["properties"], json!(["display_name"]));
    assert_eq!(changes[1]["kind"], "visual");
    assert_eq!(changes[1]["key"], "by-product");
    assert_eq!(changes[1]["properties"], json!(["position", "fields"]));
    assert_eq!(page(&s, "overview").display_name, "Summary");
    let chart = visual(&s, "overview", "by-product");
    assert_eq!(chart.position.width, 500.0);
    assert_eq!(
        chart.fields[0].field,
        FieldRef::Column {
            table: "Sales".into(),
            column: "Product".into()
        }
    );
    // Untouched visuals are byte-identical.
    assert_eq!(visual(&s, "overview", "total"), &before_card);
    assert_noop(&mut s, &changed, json!({}));

    // Omitted properties are left alone; a partial position on an existing visual moves only
    // what it names.
    let partial = json!({"pages": [{"key": "overview", "visuals": [{"key": "total", "x": 30}]}]});
    let out = apply(&mut s, &partial);
    assert_eq!(out["change_count"], 1, "{out}");
    let card = visual(&s, "overview", "total");
    assert_eq!(
        (card.position.x, card.position.y, card.position.width),
        (30.0, 20.0, 200.0)
    );
    assert_eq!(card.fields, before_card.fields);
    // An empty field list clears the bindings.
    let clear = json!({"pages": [{"key": "overview", "visuals": [{"key": "total", "fields": []}]}]});
    apply(&mut s, &clear);
    assert!(visual(&s, "overview", "total").fields.is_empty());
}

#[test]
fn check_and_dry_run_plan_without_changing_anything() {
    let mut s = session();
    let before = s.report.clone();
    let check = run(&mut s, "report.check", json!({"spec": spec()}));
    assert_eq!(check["valid"], true, "{check}");
    assert_eq!(check["in_sync"], false);
    assert_eq!(check["change_count"], 5, "{check}");
    assert_eq!(check["applied"], false);
    let dry = run(&mut s, "report.apply", json!({"spec": spec(), "dry_run": true}));
    assert_eq!(dry["changes"], check["changes"]);
    assert_eq!(dry["dry_run"], true);
    assert_eq!(s.report, before);
}

#[test]
fn any_error_rejects_the_whole_spec() {
    let mut s = session();
    let mut bad = spec();
    bad["pages"][1]["visuals"][0]["fields"][1]["field"] = json!("Sales[Nope]");
    bad["pages"][0]["visuals"][0]["fields"][0]["field"] = json!("[Missing]");
    let check = run(&mut s, "report.check", json!({"spec": bad}));
    assert_eq!(check["valid"], false);
    assert_eq!(check["error_count"], 2, "{check}");
    assert_eq!(check["errors"][0]["at"], "spec.pages[0].visuals[0].fields[0]");
    let e = fails(&mut s, "report.apply", json!({"spec": bad}));
    assert!(
        e.contains("changed nothing") && e.contains("Nope") && e.contains("Missing"),
        "{e}"
    );
    // force binds unknown columns as written.
    let mut forced = spec();
    forced["pages"][1]["visuals"][0]["fields"][1]["field"] = json!("Sales[Nope]");
    let out = run(&mut s, "report.apply", json!({"spec": forced, "force": true}));
    assert!(out["warning_count"].as_u64().unwrap() >= 1, "{out}");
}

#[test]
fn spec_shape_errors_are_reported_with_their_path() {
    let mut s = session();
    for (spec, needle) in [
        (json!([]), "spec must be an object"),
        (json!({"pages": {}}), "spec.pages must be an array"),
        (json!({"pagez": []}), "unknown parameter `pagez`"),
        (json!({"pages": [{}]}), "spec.pages[0]: needs a `key`"),
        (json!({"pages": [{"key": "a", "name": "b"}]}), "not both"),
        (json!({"pages": [{"key": ""}]}), "must not be empty"),
        (
            json!({"pages": [{"key": "a", "width": -1}]}),
            "spec.pages[0]: `width`",
        ),
        (
            json!({"pages": [{"key": "a", "width": "wide"}]}),
            "must be a number",
        ),
        (
            json!({"pages": [{"key": "a", "display_option": "Zoom"}]}),
            "unknown display_option",
        ),
        (
            json!({"pages": [{"key": "a", "visuals": {}}]}),
            "visuals must be an array",
        ),
        (
            json!({"pages": [{"key": "a", "visuals": [{"key": "v", "type": "card", "colour": 1}]}]}),
            "unknown parameter `colour`",
        ),
        (
            json!({"pages": [{"key": "a", "visuals": [{"key": "v", "fields": [{"role": "Values"}]}]}]}),
            "missing `field`",
        ),
        (
            json!({"pages": [{"key": "a", "visuals": [{"key": "v", "x": 1e9}]}]}),
            "`x` must be between",
        ),
        (
            json!({"pages": [{"key": "a", "visuals": [{"key": "v", "props": 3}]}]}),
            "props must be an object",
        ),
        (
            json!({"pages": [{"key": "a", "visuals": [{"key": "g", "group": true, "type": "card"}]}]}),
            "has no `type`",
        ),
    ] {
        let e = fails(&mut s, "report.apply", json!({"spec": spec}));
        assert!(e.contains(needle), "{spec}: {e}");
    }
    let e = fails(&mut s, "report.apply", json!({"spec": spec(), "prune": "some"}));
    assert!(e.contains("prune"), "{e}");
    let e = fails(&mut s, "report.apply", json!({}));
    assert!(e.contains("spec"), "{e}");
    let e = fails(&mut s, "report.check", json!({"spec": spec(), "dry_run": true}));
    assert!(e.contains("unknown parameter"), "{e}");
    // Size caps.
    let many: Vec<Value> = (0..2001).map(|i| json!({"key": format!("p{i}")})).collect();
    let e = fails(&mut s, "report.apply", json!({"spec": {"pages": many}}));
    assert!(e.contains("at most"), "{e}");
    let long_key = "k".repeat(201);
    let e = fails(
        &mut s,
        "report.apply",
        json!({"spec": {"pages": [{"key": long_key}]}}),
    );
    assert!(e.contains("longer"), "{e}");
    // No report open.
    let mut empty = Session::new();
    let e = empty
        .execute("report.apply", &json!({"spec": spec()}))
        .unwrap_err();
    assert!(e.contains("report"), "{e}");
}

#[test]
fn identity_conflicts_and_bad_items_are_errors() {
    let mut s = session();
    apply(&mut s, &spec());
    for (bad, needle) in [
        (
            json!({"pages": [{"key": "overview"}, {"key": "overview"}]}),
            "same page",
        ),
        (
            json!({"pages": [{"key": "overview"}, {"name": page_name_for_key("overview")}]}),
            "same page",
        ),
        (
            json!({"pages": [{"key": "x", "visuals": [{"key": "a", "type": "card"},
                                                      {"key": "a", "type": "card"}]}]}),
            "same visual",
        ),
        (json!({"pages": [{"name": "no such/page"}]}), "no page is named"),
        (
            json!({"pages": [{"key": "new", "display_name": "overview"}]}),
            "display name",
        ),
        (
            json!({"pages": [{"key": "x", "visuals": [{"key": "a", "x": 1, "y": 1, "width": 10, "height": 10}]}]}),
            "needs a `type`",
        ),
        (
            json!({"pages": [{"key": "x", "visuals": [{"key": "a", "type": "card", "x": 1, "y": 1}]}]}),
            "all of x, y, width and height",
        ),
        (
            json!({"pages": [{"key": "x", "visuals": [{"key": "a", "type": "bad type!", "x": 1, "y": 1,
                                                       "width": 10, "height": 10}]}]}),
            "not a visual type",
        ),
        (
            json!({"pages": [{"key": "x", "visuals": [{"key": "g", "group": true}]}]}),
            "creating visual groups",
        ),
        (
            json!({"pages": [{"key": "overview", "visuals": [{"key": "total", "group": true}]}]}),
            "not a visual group",
        ),
        (
            json!({"pages": [{"key": "overview", "visuals": [{"key": "by-product", "fields": [
                {"role": "Y", "field": "[Total]"}, {"role": "Y", "field": "[Total]"}]}]}]}),
            "bound twice",
        ),
        (
            json!({"pages": [{"key": "overview", "visuals": [{"key": "total", "fields": [
                {"role": "Bad Role", "field": "[Total]"}]}]}]}),
            "data role",
        ),
        (json!({"pages": [], "active_page": "nowhere"}), "active_page"),
        (json!({"pages": []}), ""),
    ] {
        if needle.is_empty() {
            // An empty spec is a no-op.
            let out = run(&mut s, "report.apply", json!({"spec": bad}));
            assert_eq!(out["change_count"], 0);
            continue;
        }
        let e = fails(&mut s, "report.apply", json!({"spec": bad}));
        assert!(e.contains(needle), "{bad}: {e}");
    }
}

#[test]
fn prune_removes_what_the_spec_does_not_list() {
    let mut s = session();
    let blank = report(&s).pages[0].name.clone();
    apply(&mut s, &spec());
    run(
        &mut s,
        "visual.add",
        json!({"page": "Overview", "visual_type": "card", "x": 0, "y": 500, "width": 100, "height": 100}),
    );
    run(
        &mut s,
        "visual.add",
        json!({"page": blank, "visual_type": "card", "x": 0, "y": 0, "width": 100, "height": 100}),
    );
    assert_eq!(page(&s, "overview").visuals.len(), 3);
    // Without prune, extra visuals stay.
    assert_noop(&mut s, &spec(), json!({}));
    // prune: true removes the unlisted visual of a listed page, not the unlisted page's visual.
    let out = run(&mut s, "report.apply", json!({"spec": spec(), "prune": true}));
    assert_eq!(out["summary"]["visuals_removed"], 1, "{out}");
    assert_eq!(page(&s, "overview").visuals.len(), 2);
    assert_eq!(report(&s).pages[0].visuals.len(), 1);
    // A page listed without `visuals` keeps its visuals even with prune.
    let unmanaged = json!({"pages": [{"key": "overview"}, {"key": "detail"}]});
    assert_noop(&mut s, &unmanaged, json!({"prune": true}));
    // prune: "all" also removes unlisted pages; the active page moves to a kept one.
    run(&mut s, "page.set_active", json!({"page": blank}));
    let out = run(&mut s, "report.apply", json!({"spec": spec(), "prune": "all"}));
    assert_eq!(out["summary"]["pages_removed"], 1, "{out}");
    let r = report(&s);
    assert_eq!(r.pages.len(), 2);
    assert_eq!(
        r.active_page.as_deref(),
        Some(page_name_for_key("overview").as_str())
    );
    assert_noop(&mut s, &spec(), json!({"prune": "all"}));
    // Pruning every page is refused.
    let e = fails(
        &mut s,
        "report.apply",
        json!({"spec": {"pages": []}, "prune": "all"}),
    );
    assert!(e.contains("no page left"), "{e}");
}

#[test]
fn pruning_keeps_groups_that_hold_listed_visuals() {
    let mut s = session();
    apply(&mut s, &spec());
    let page_name = page_name_for_key("overview");
    let card = visual_name_for_key(&page_name, "total");
    {
        let r = s.report.as_mut().unwrap();
        let p = r.page_mut(&page_name).unwrap();
        p.visuals.push(Visual {
            name: "group1".into(),
            position: bicraft_report::Position {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                width: 300.0,
                height: 200.0,
                tab_order: None,
            },
            ..Default::default()
        });
        p.visual_mut(&card).unwrap().parent_group = Some("group1".into());
    }
    let out = run(&mut s, "report.apply", json!({"spec": spec(), "prune": true}));
    assert_eq!(out["change_count"], 0, "{out}");
    // The group itself can be listed (and moved) with `group: true`.
    let moved = json!({"pages": [{"key": "overview", "visuals": [
        {"name": "group1", "group": true, "x": 10, "y": 10, "width": 300, "height": 200}]}]});
    let out = apply(&mut s, &moved);
    assert_eq!(out["changes"][0]["properties"], json!(["position"]), "{out}");
    let e = fails(
        &mut s,
        "report.apply",
        json!({"spec": {"pages": [{"key": "overview", "visuals": [{"name": "group1", "type": "card"}]}]}}),
    );
    assert!(e.contains("is a visual group"), "{e}");
}

#[test]
fn pages_follow_the_spec_order_and_active_page() {
    let mut s = session();
    let blank = report(&s).pages[0].name.clone();
    apply(&mut s, &spec());
    let swapped = json!({"pages": [{"key": "detail"}, {"key": "overview"}], "active_page": "detail"});
    let out = apply(&mut s, &swapped);
    let actions: Vec<&str> = out["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["action"].as_str().unwrap())
        .collect();
    assert_eq!(actions, ["reorder", "set_active"], "{out}");
    let names: Vec<String> = report(&s).pages.iter().map(|p| p.name.clone()).collect();
    assert_eq!(
        names,
        [blank, page_name_for_key("detail"), page_name_for_key("overview")]
    );
    assert_eq!(
        report(&s).active_page.as_deref(),
        Some(page_name_for_key("detail").as_str())
    );
    assert_noop(&mut s, &swapped, json!({}));
}

#[test]
fn page_add_and_visual_add_take_the_same_keys() {
    let mut s = session();
    let p = run(
        &mut s,
        "page.add",
        json!({"display_name": "Overview", "key": "overview"}),
    );
    assert_eq!(p["page"], page_name_for_key("overview"));
    let v = run(
        &mut s,
        "visual.add",
        json!({"page": "Overview", "key": "total", "visual_type": "card", "x": 20, "y": 20,
               "width": 200, "height": 120, "fields": [{"role": "Values", "field": "[Total]"}]}),
    );
    assert_eq!(
        v["visual"],
        visual_name_for_key(&page_name_for_key("overview"), "total")
    );
    let e = fails(
        &mut s,
        "page.add",
        json!({"display_name": "Other", "key": "overview"}),
    );
    assert!(e.contains("already exists"), "{e}");
    let e = fails(
        &mut s,
        "visual.add",
        json!({"page": "Overview", "key": "total", "visual_type": "card", "x": 0, "y": 0,
               "width": 1, "height": 1}),
    );
    assert!(e.contains("already has"), "{e}");
    // The spec recognises them: only the chart is new.
    let mut only_overview = spec();
    only_overview["pages"].as_array_mut().unwrap().truncate(1);
    let out = apply(&mut s, &only_overview);
    assert_eq!(out["change_count"], 1, "{out}");
    assert_eq!(out["changes"][0]["key"], "by-product");
}

#[test]
fn extract_then_apply_changes_nothing() {
    let mut s = session();
    apply(&mut s, &spec());
    run(
        &mut s,
        "visual.add",
        json!({"page": "Detail", "visual_type": "slicer", "x": 5.5, "y": 7.25, "width": 100.125,
               "height": 50, "fields": [{"role": "Values", "field": "Sales[Ghost]"}], "force": true}),
    );
    // Hidden page and visual, display option, and a raw field survive.
    {
        let r = s.report.as_mut().unwrap();
        r.pages[1].visibility = Some("HiddenInViewMode".into());
        r.pages[1].display_option = Some("FitToWidth".into());
        let v = &mut r.pages[2].visuals[0];
        v.is_hidden = true;
        v.fields.push(bicraft_report::FieldBinding {
            display_name: None,
            show_as: None,
            role: "Values".into(),
            field: FieldRef::Other {
                raw: json!({"HierarchyLevel": {"Level": "Year"}}),
            },
        });
    }
    let extracted = run(&mut s, "report.extract", json!({}));
    assert_eq!(extracted["pages"].as_array().unwrap().len(), 3);
    assert_eq!(extracted["pages"][1]["hidden"], true);
    assert_eq!(
        extracted["pages"][2]["visuals"][0]["fields"][2]["raw"]["HierarchyLevel"]["Level"],
        "Year"
    );
    assert!(extracted["active_page"].is_string());
    assert_noop(&mut s, &extracted, json!({}));
    assert_noop(&mut s, &extracted, json!({"prune": "all"}));
    // One page only.
    let one = run(&mut s, "report.extract", json!({"page": "Detail"}));
    assert_eq!(one["pages"].as_array().unwrap().len(), 1);
    assert!(one.get("active_page").is_none());
    assert_noop(&mut s, &one, json!({"prune": true}));
    // An extracted spec rebuilds the report in a fresh session.
    let mut fresh = session();
    run(
        &mut fresh,
        "report.apply",
        json!({"spec": extracted, "prune": "all", "force": true}),
    );
    let names = |s: &Session| -> Vec<String> { report(s).pages.iter().map(|p| p.name.clone()).collect() };
    assert_eq!(names(&fresh), names(&s));
    let visuals = |r: &Report| -> Vec<Value> {
        r.pages
            .iter()
            .flat_map(|p| &p.visuals)
            .map(|v| {
                json!([
                    v.name,
                    v.visual_type,
                    v.fields,
                    v.is_hidden,
                    v.position.x,
                    v.position.width
                ])
            })
            .collect()
    };
    assert_eq!(visuals(report(&fresh)), visuals(report(&s)));
}

#[test]
fn extract_then_apply_changes_nothing_on_a_pbir_report() {
    let tmp = crate::tempdir::TempDir::new("engine-apply-pbir");
    let dir = tmp.path();
    let mut s = session();
    apply(&mut s, &spec());
    run(
        &mut s,
        "project.new",
        json!({"dir": dir, "name": "Shop", "format": "tmsl"}),
    );
    let mut fresh = Session::new();
    run(&mut fresh, "file.open", json!({"path": dir.join("Shop.pbip")}));
    assert!(!fresh.report_dirty);
    let extracted = run(&mut fresh, "report.extract", json!({}));
    assert_noop(&mut fresh, &extracted, json!({"prune": "all"}));
    // The original spec is still in sync after the PBIR round trip.
    assert_noop(&mut fresh, &spec(), json!({}));
    // And saving after a no-op leaves the files alone (nothing is dirty).
    assert!(!fresh.report_dirty);
}

#[test]
fn reports_without_a_model_match_fields_by_text() {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(&mut s, "table.add", json!({"name": "Sales"}));
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Total", "expression": "1"}),
    );
    run(&mut s, "report.new", json!({}));
    let spec = json!({"pages": [{"key": "p", "visuals": [{"key": "c", "type": "card", "x": 0, "y": 0,
        "width": 10, "height": 10, "fields": [{"role": "Values", "field": "[Total]"}]}]}]});
    apply(&mut s, &spec);
    s.model = None;
    // `[Total]` cannot be resolved without a model, but it is what the visual already binds.
    assert_noop(&mut s, &spec, json!({}));
}

#[test]
fn visuals_without_a_position_are_placed_by_the_layout() {
    let mut s = session();
    let spec = json!({"pages": [{"key": "grid", "visuals": [
        {"key": "a", "type": "card", "fields": [{"role": "Values", "field": "[Total]"}]},
        {"key": "b", "type": "card"},
        {"key": "c", "type": "tableEx", "x": 0, "y": 600, "width": 1280, "height": 100}
    ]}]});
    let out = apply(&mut s, &spec);
    assert_eq!(out["summary"]["visuals_created"], 3, "{out}");
    let a = visual(&s, "grid", "a").position;
    let b = visual(&s, "grid", "b").position;
    assert!(a.width > 0.0 && b.width > 0.0);
    assert_ne!((a.x, a.y), (b.x, b.y));
    assert_eq!(visual(&s, "grid", "c").position.y, 600.0);
    assert_noop(&mut s, &spec, json!({}));
}

#[test]
fn props_are_applied_and_extracted() {
    let mut s = session();
    let mut with_props = spec();
    with_props["pages"][0]["visuals"][0]["props"] = json!({"title": "Revenue"});
    let out = apply(&mut s, &with_props);
    assert_eq!(out["summary"]["visuals_created"], 3, "{out}");
    assert_noop(&mut s, &with_props, json!({}));
    let extracted = run(&mut s, "report.extract", json!({}));
    assert!(
        extracted["pages"][1]["visuals"][0]["props"].is_object(),
        "{extracted}"
    );
    assert_noop(&mut s, &extracted, json!({}));
}

/// Real reports, local only (skipped without `BICRAFT_TESTDATA`): extract → apply is a no-op.
#[test]
fn extract_then_apply_changes_nothing_on_real_reports() {
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
    for path in files {
        let mut s = Session::new();
        if s.execute("file.open", &json!({"path": path})).is_err() || s.report.is_none() {
            eprintln!("a file could not be opened with a report: skipped");
            continue;
        }
        let extracted = run(&mut s, "report.extract", json!({}));
        let start = std::time::Instant::now();
        assert_noop(&mut s, &extracted, json!({"prune": "all"}));
        let r = report(&s);
        eprintln!(
            "real report: {} pages, {} visuals, extract -> apply no-op in {:?}",
            r.pages.len(),
            r.pages.iter().map(|p| p.visuals.len()).sum::<usize>(),
            start.elapsed()
        );
    }
}

/// `cargo test -p bicraft-engine --release report_apply_timing -- --ignored --nocapture`
#[test]
#[ignore = "timing; run in release"]
fn report_apply_timing() {
    let mut s = session();
    let pages: Vec<Value> = (0..50)
        .map(|p| {
            let visuals: Vec<Value> = (0..10)
                .map(|v| {
                    json!({"key": format!("v{v}"), "type": "clusteredColumnChart",
                           "x": (v % 5) * 250, "y": (v / 5) * 350, "width": 240, "height": 340,
                           "fields": [{"role": "Category", "field": "Product[Name]"},
                                      {"role": "Y", "field": "[Total]"}]})
                })
                .collect();
            json!({"key": format!("section {p}"), "visuals": visuals})
        })
        .collect();
    let spec = json!({"pages": pages});
    let start = std::time::Instant::now();
    let out = apply(&mut s, &spec);
    let first = start.elapsed();
    assert_eq!(out["summary"]["visuals_created"], 500);
    let start = std::time::Instant::now();
    let out = apply(&mut s, &spec);
    let again = start.elapsed();
    assert_eq!(out["change_count"], 0);
    eprintln!("report.apply 50 pages x 10 visuals: first {first:?}, re-apply (no-op) {again:?}");
}

/// The example of docs/mcp.md ("Declarative reports"), with this model's names.
#[test]
fn the_documented_example_applies_and_is_idempotent() {
    let mut s = session();
    let spec = json!({"pages": [
      {"key": "overview", "display_name": "Overview", "width": 1280, "height": 720,
       "layout": "grid",
       "visuals": [
         {"key": "revenue", "type": "card", "place": {"col": 0, "row": 0, "col_span": 3, "row_span": 2},
          "fields": [{"role": "Values", "field": "[Total]"}], "props": {"title": "Revenue"}},
         {"key": "by-category", "type": "clusteredColumnChart", "x": 320, "y": 16, "width": 600,
          "height": 320, "fields": [{"role": "Category", "field": "Product[Name]"},
                                    {"role": "Y", "field": "[Total]"}]}
       ]}
     ],
     "active_page": "overview"});
    let out = apply(&mut s, &spec);
    assert_eq!(out["summary"]["visuals_created"], 2, "{out}");
    assert_eq!(
        report(&s).active_page.as_deref(),
        Some(page_name_for_key("overview").as_str())
    );
    let card = visual(&s, "overview", "revenue").position;
    assert_eq!((card.x, card.y), (16.0, 16.0));
    assert!(card.width > 0.0 && card.height > 0.0);
    assert_noop(&mut s, &spec, json!({}));
    // check is read-only: it does not bump the session revision.
    let revision = s.revision;
    run(&mut s, "report.check", json!({"spec": spec}));
    run(&mut s, "report.extract", json!({}));
    assert_eq!(s.revision, revision);
}
