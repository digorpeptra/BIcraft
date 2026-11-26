//! Tests of `TODAY()`/`NOW()`/`DATEDIFF` with `as_of`, table total rows evaluated in the
//! grand-total filter context (`ALLSELECTED`, `REMOVEFILTERS`, `RANKX`, `SAMEPERIODLASTYEAR`),
//! measure-bound visual properties (`bound_values.rs`) and the `preview` status of visuals whose
//! bound values or conditional formatting cannot be evaluated. Synthetic model only.
//!
//! The same total-row patterns (`ROLLUPADDISSUBTOTAL` grand totals of share, dense rank and
//! year-on-year measures) are in the `dax.evaluate` oracle corpus (`tests/dax_queries`, ids
//! `totals-*`), recorded from Power BI Desktop 2.158's engine.

use bicraft_report::{Position, Visual};
use serde_json::{Map, Value, json};

use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Days of 2024-2025 (`Calendar`), spend rows (`Sales`) in four categories of two families.
fn model() -> Session {
    let mut s = Session::new();
    let mut days = Vec::new();
    for y in 2024..=2025 {
        for m in 1..=12 {
            let n = match m {
                2 if y == 2024 => 29,
                2 => 28,
                4 | 6 | 9 | 11 => 30,
                _ => 31,
            };
            for d in 1..=n {
                days.push(format!("{{#date({y},{m},{d})}}"));
            }
        }
    }
    let calendar = format!("#table({{\"Date\"}}, {{{}}})", days.join(", "));
    let sales = r#"#table({"Date", "Category", "Amount"}, {
        {#date(2024,3,1), "Rent", 500}, {#date(2024,3,9), "Food", 100},
        {#date(2025,3,1), "Rent", 600}, {#date(2025,3,9), "Food", 150},
        {#date(2025,4,2), "Fuel", 80}, {#date(2025,5,5), "Toys", 20}})"#;
    let category = r#"#table({"Category", "Family"}, {
        {"Rent", "Home"}, {"Food", "Home"}, {"Fuel", "Car"}, {"Toys", "Fun"}})"#;
    let mut steps = vec![
        ("model.new", json!({"name": "Spend"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "table.add",
            json!({"name": "Category", "partition": {"source_type": "m", "expression": category}}),
        ),
        (
            "table.add",
            json!({"name": "Calendar", "partition": {"source_type": "m", "expression": calendar}}),
        ),
    ];
    for (t, c, ty) in [
        ("Sales", "Date", "dateTime"),
        ("Sales", "Category", "string"),
        ("Sales", "Amount", "double"),
        ("Category", "Category", "string"),
        ("Category", "Family", "string"),
        ("Calendar", "Date", "dateTime"),
    ] {
        steps.push(("column.add", json!({"table": t, "name": c, "data_type": ty})));
    }
    for (from, to) in [
        ("Sales[Date]", "Calendar[Date]"),
        ("Sales[Category]", "Category[Category]"),
    ] {
        steps.push(("relationship.add", json!({"from": from, "to": to})));
    }
    for (name, expr, fmt) in [
        ("Spend", "SUM(Sales[Amount])", "#,0"),
        (
            "Share",
            "VAR v = [Spend] VAR t = CALCULATE([Spend], REMOVEFILTERS(Category)) RETURN DIVIDE(v, t)",
            "0.0 %",
        ),
        (
            "Rank",
            "IF(NOT ISBLANK([Spend]), RANKX(ALLSELECTED(Category[Category]), [Spend], , DESC, DENSE))",
            "0",
        ),
        (
            "YoY",
            "VAR a = [Spend] VAR b = CALCULATE([Spend], SAMEPERIODLASTYEAR('Calendar'[Date])) RETURN DIVIDE(a - b, b)",
            "+0.0 %;-0.0 %",
        ),
        ("Age", "INT(TODAY() - MAX(Sales[Date]))", "#,0"),
        (
            "Pill",
            "\"● \" & [Age] & \" days · data of \" & FORMAT(MAX(Sales[Date]), \"dd/MM/yyyy\")",
            "",
        ),
        ("Tint", "IF([Age] > 30, \"#C62828\", \"#2E7D32\")", ""),
        ("Period", "FORMAT(MIN('Calendar'[Date]), \"MMMM yyyy\")", ""),
        ("Bad", "RAND()", ""),
    ] {
        steps.push((
            "measure.add",
            json!({"table": "Sales", "name": name, "expression": expr, "format_string": fmt}),
        ));
    }
    steps.extend([
        ("report.new", json!({})),
        ("page.add", json!({"display_name": "Main"})),
    ]);
    for (id, p) in steps {
        run(&mut s, id, p);
    }
    s
}

fn add_visual(s: &mut Session, visual_type: &str, fields: &[&str]) -> String {
    let fields: Vec<Value> = fields
        .iter()
        .map(|f| json!({"role": "Values", "field": f}))
        .collect();
    let r = run(
        s,
        "visual.add",
        json!({"page": "Main", "visual_type": visual_type, "x": 0, "y": 0, "width": 400, "height": 200,
               "fields": fields}),
    );
    r["visual"].as_str().map(str::to_string).unwrap_or_default()
}

/// Adds a field-less visual with these `objects` to page "Main".
fn add_raw(s: &mut Session, name: &str, visual_type: &str, objects: Value) {
    let mut extra = Map::new();
    extra.insert("objects".into(), objects);
    let v = Visual {
        name: name.into(),
        position: Position {
            x: 0.0,
            y: 300.0,
            z: 0.0,
            width: 500.0,
            height: 60.0,
            tab_order: None,
        },
        visual_type: Some(visual_type.into()),
        fields: Vec::new(),
        parent_group: None,
        is_hidden: false,
        filter_config: None,
        visual_extra: extra,
        extra: Map::new(),
    };
    let report = s.report.as_mut().expect("report");
    let page = report
        .pages
        .iter_mut()
        .find(|p| p.display_name == "Main")
        .expect("page");
    page.visuals.push(v);
}

fn visual(s: &Session, name: &str) -> Value {
    let snap = s.snapshot();
    for p in snap["report"]["pages"].as_array().cloned().unwrap_or_default() {
        for v in p["visuals"].as_array().cloned().unwrap_or_default() {
            if v["name"] == name {
                return v;
            }
        }
    }
    panic!("visual {name} not in the snapshot")
}

fn measure(name: &str) -> Value {
    json!({"expr": {"Measure": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": name}}})
}

fn statuses(s: &mut Session) -> Vec<(String, String, String)> {
    let r = run(s, "report.render", json!({"page": "Main", "system_fonts": false}));
    r["pages"][0]["visuals"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|v| {
            (
                v["name"].as_str().unwrap_or("").to_string(),
                v["status"].as_str().unwrap_or("").to_string(),
                v["note"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect()
}

#[test]
fn today_now_and_datediff_follow_as_of() {
    let mut s = model();
    let q = "EVALUATE ROW(\"t\", TODAY(), \"n\", NOW(), \"u\", UTCTODAY(), \
             \"d\", DATEDIFF(DATE(2026,1,1), TODAY(), DAY), \"m\", DATEDIFF(DATE(2025,12,31), TODAY(), MONTH), \
             \"w\", DATEDIFF(DATE(2026,9,12), DATE(2026,9,13), WEEK), \"y\", DATEDIFF(TODAY(), DATE(2024,1,1), YEAR), \
             \"h\", DATEDIFF(DATE(2026,9,18), NOW(), HOUR), \"b\", DATEDIFF(BLANK(), TODAY(), DAY))";
    let r = run(
        &mut s,
        "dax.evaluate",
        json!({"query": q, "as_of": "2026-09-18T10:30:00"}),
    );
    let row = &r["results"][0]["rows"][0];
    assert_eq!(row["[t]"], "2026-09-18T00:00:00", "{r}");
    assert_eq!(row["[n]"], "2026-09-18T10:30:00", "{r}");
    assert_eq!(row["[u]"], "2026-09-18T00:00:00", "{r}");
    assert_eq!(row["[d]"], 260, "{r}");
    assert_eq!(row["[m]"], 9, "{r}");
    // 2026-09-13 is a Sunday: a week boundary
    assert_eq!(row["[w]"], 1, "{r}");
    assert_eq!(row["[y]"], -2, "{r}");
    assert_eq!(row["[h]"], 10, "{r}");
    assert_eq!(row["[b]"], Value::Null, "{r}");
    // without as_of: the clock, so TODAY() is some date
    let r = run(
        &mut s,
        "dax.evaluate",
        json!({"query": "EVALUATE ROW(\"t\", TODAY())"}),
    );
    assert!(r["results"][0]["rows"][0]["[t]"].is_string(), "{r}");
    for bad in ["2026-02-30", "18/09/2026", "2026-09-18T25:00", "x"] {
        let e = s
            .execute(
                "dax.evaluate",
                &json!({"query": "EVALUATE ROW(\"t\", 1)", "as_of": bad}),
            )
            .unwrap_err();
        assert!(e.contains("as_of"), "{bad}: {e}");
    }
}

#[test]
fn view_as_of_drives_cards_and_is_reset_by_null() {
    let mut s = model();
    let r = run(&mut s, "view.set_as_of", json!({"as_of": "2025-06-04"}));
    assert_eq!(r["as_of"], "2025-06-04");
    let v = add_visual(&mut s, "card", &["[Age]"]);
    // last sale 2025-05-05: 30 days before 2025-06-04
    assert_eq!(visual(&s, &v)["data"]["rows"], json!([[30]]));
    run(&mut s, "view.set_as_of", json!({"as_of": "2025-06-05T08:00:00"}));
    assert_eq!(
        visual(&s, &v)["data"]["rows"],
        json!([[31]]),
        "the cache follows as_of"
    );
    // report.render's as_of applies to that render only
    let r = run(
        &mut s,
        "report.render",
        json!({"page": "Main", "system_fonts": false, "as_of": "2025-05-05"}),
    );
    assert_eq!(r["pages"][0]["visuals"][0]["status"], "drawn", "{r}");
    assert_eq!(s.as_of.map(f64::floor), Some(45_813.0));
    let r = run(&mut s, "view.set_as_of", json!({"as_of": null}));
    assert_eq!(r["as_of"], Value::Null);
    assert!(s.execute("view.set_as_of", &json!({})).is_err());
    assert!(
        s.execute("view.set_as_of", &json!({"as_of": "2025-13-01"}))
            .is_err()
    );
}

#[test]
fn table_total_row_is_evaluated_in_the_grand_total_context() {
    let mut s = model();
    run(&mut s, "view.set_as_of", json!({"as_of": "2025-12-31"}));
    let v = add_visual(
        &mut s,
        "tableEx",
        &["Category[Category]", "[Spend]", "[Share]", "[Rank]", "[YoY]"],
    );
    let d = visual(&s, &v)["data"].clone();
    assert_eq!(d["status"], "ok", "{d}");
    let t = d["totals"].as_array().cloned().unwrap_or_default();
    assert_eq!(t.len(), 5, "{d}");
    assert_eq!(t[0], Value::Null);
    assert_eq!(t[1], 1450);
    // % share of the total is 100 %, the total ranks first among the categories
    assert_eq!(t[2], 1);
    assert_eq!(t[3], 1);
    // the whole calendar (2024-2025) against its year-earlier dates (2024 only): (1450 - 600) / 600
    let yoy = t[4].as_f64().unwrap_or(f64::NAN);
    assert!((yoy - 850.0 / 600.0).abs() < 1e-9, "{d}");
    // the rows keep their own values; the total row is not their sum (ranks add up to 1+2+3+4)
    let rows = d["rows"].as_array().cloned().unwrap_or_default();
    let ranks: f64 = rows.iter().filter_map(|r| r[3].as_f64()).sum();
    assert_eq!(ranks, 10.0, "{d}");

    // the renderer draws the engine's totals ("Total", 100.0%, 1), not the sum of the ranks
    let r = run(
        &mut s,
        "report.render",
        json!({"page": "Main", "system_fonts": false, "text_dump": true}),
    );
    let texts: Vec<String> = r["pages"][0]["texts"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|t| t["t"].as_str().map(str::to_string))
        .collect();
    // the percent pattern follows the culture (a no-break space before % in some): compare without spaces
    let squeeze = |t: &String| t.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    assert!(texts.iter().any(|t| squeeze(t) == "100.0%"), "{texts:?}");
    assert!(
        !texts.iter().any(|t| t == "10"),
        "rank total must not be the sum: {texts:?}"
    );
}

#[test]
fn measure_bound_text_and_button_values_are_inserted() {
    let mut s = model();
    run(&mut s, "view.set_as_of", json!({"as_of": "2025-06-30"}));
    add_raw(
        &mut s,
        "tb",
        "textbox",
        json!({
            "general": [{"properties": {"paragraphs": [{"textRuns": [
                {"value": "Spend of "},
                {"value": {"propertyIdentifier": {"objectName": "values", "propertyName": "expr"},
                           "selector": {"id": "P"}}},
                {"value": ", transfers excluded."}
            ]}]}}],
            "values": [{"properties": {"expr": measure("Period")}, "selector": {"id": "P"}}]
        }),
    );
    add_raw(
        &mut s,
        "bt",
        "actionButton",
        json!({
            "text": [{"properties": {"show": {"expr": {"Literal": {"Value": "true"}}}, "text": measure("Pill")},
                      "selector": {"id": "default"}}],
            "fill": [{"properties": {"show": {"expr": {"Literal": {"Value": "true"}}},
                                     "fillColor": {"solid": {"color": measure("Tint")}}},
                      "selector": {"id": "default"}}]
        }),
    );
    let tb = visual(&s, "tb");
    let runs = tb["text"]["paragraphs"][0]["runs"].clone();
    assert_eq!(runs[1]["text"], "January 2024", "{tb}");
    assert_eq!(runs[1]["bound"], "P");
    assert!(tb.get("preview_reason").is_none(), "{tb}");
    let bt = visual(&s, "bt");
    assert_eq!(bt["button"]["text"], "● 56 days · data of 05/05/2025", "{bt}");
    assert_eq!(bt["button"]["fill"], "#C62828", "{bt}");
    let st = statuses(&mut s);
    assert!(st.iter().all(|(_, s, _)| s == "drawn"), "{st:?}");
}

#[test]
fn unevaluated_bound_values_and_formatting_are_preview_not_drawn() {
    let mut s = model();
    add_raw(
        &mut s,
        "bt",
        "actionButton",
        json!({
            "text": [{"properties": {"text": measure("Bad")}, "selector": {"id": "default"}}],
            "fill": [{"properties": {"fillColor": {"solid": {"color": measure("Bad")}}},
                      "selector": {"id": "default"}}]
        }),
    );
    add_raw(
        &mut s,
        "tb",
        "textbox",
        json!({
            "general": [{"properties": {"paragraphs": [{"textRuns": [
                {"value": {"propertyIdentifier": {"objectName": "values", "propertyName": "expr"},
                           "selector": {"id": "X"}}}
            ]}]}}],
            "values": [{"properties": {"expr": measure("Bad")}, "selector": {"id": "X"}}]
        }),
    );
    let t = add_visual(&mut s, "tableEx", &["Category[Category]", "[Spend]"]);
    run(
        &mut s,
        "visual.conditional",
        json!({"page": "Main", "visual": t, "target": "background", "series": "[Spend]",
               "rule": {"field_value": {"field": "[Bad]"}}}),
    );
    let bt = visual(&s, "bt");
    assert!(
        bt["preview_reason"].as_str().is_some_and(|r| r.contains("RAND")),
        "{bt}"
    );
    let d = visual(&s, &t)["data"].clone();
    assert_eq!(d["status"], "ok", "{d}");
    assert!(
        d["formatting_ignored_reason"]
            .as_str()
            .is_some_and(|r| r.contains("RAND")),
        "{d}"
    );
    let st = statuses(&mut s);
    for name in ["bt", "tb", t.as_str()] {
        let (_, status, note) = st.iter().find(|(n, _, _)| n == name).cloned().unwrap_or_default();
        assert_eq!(status, "preview", "{name}: {st:?}");
        assert!(note.contains("RAND"), "{name}: {note}");
    }
}

#[test]
fn matrix_subtotals_are_evaluated_per_level() {
    let mut s = model();
    let r = run(
        &mut s,
        "visual.add",
        json!({"page": "Main", "visual_type": "pivotTable", "x": 0, "y": 0, "width": 400, "height": 200,
               "fields": [{"role": "Rows", "field": "Category[Family]"}, {"role": "Rows", "field": "Category[Category]"},
                          {"role": "Values", "field": "[Spend]"}, {"role": "Values", "field": "[Share]"}]}),
    );
    let v = r["visual"].as_str().unwrap_or_default().to_string();
    let d = visual(&s, &v)["data"].clone();
    assert_eq!(d["status"], "ok", "{d}");
    // collapsed to Family (no expansion state), as Desktop 2.158 opens it
    assert_eq!(d["row_levels"], 1, "{d}");
    let levels = d["subtotals"].as_array().cloned().unwrap_or_default();
    let level = |r: u64| {
        levels
            .iter()
            .find(|l| l["rows"] == r && l["columns"] == 0)
            .map(|l| l["data"].clone())
            .unwrap_or_else(|| panic!("level {r} missing: {d}"))
    };
    // the grand total and the Home subtotal: Share is evaluated (1, 1350/1450), not summed
    assert_eq!(level(0), json!([[null, null, 1450, 1]]), "{d}");
    let fam = level(1);
    let home = fam
        .as_array()
        .and_then(|a| a.iter().find(|r| r[0] == "Home"))
        .cloned()
        .unwrap_or_default();
    assert_eq!(home[1], Value::Null, "{d}");
    assert_eq!(home[2], 1350, "{d}");
    let share = home[3].as_f64().unwrap_or(f64::NAN);
    assert!((share - 1350.0 / 1450.0).abs() < 1e-9, "{d}");
    // the leaf level is the visual's own rows, not repeated
    assert!(!levels.iter().any(|l| l["rows"] == 2), "{d}");
}

#[test]
fn matrices_differing_only_in_expansion_get_their_own_data() {
    // regression: the preview cache keyed results by fields, query and objects only, so a
    // matrix took the expansion (row levels, toggled members) of another with the same query
    let mut s = model();
    let fields = json!([{"role": "Rows", "field": "Category[Family]"}, {"role": "Rows", "field": "Category[Category]"},
                        {"role": "Values", "field": "[Spend]"}]);
    let mut names = Vec::new();
    for x in [0, 420] {
        let r = run(
            &mut s,
            "visual.add",
            json!({"page": "Main", "visual_type": "pivotTable", "x": x, "y": 0, "width": 400,
                   "height": 200, "fields": fields}),
        );
        names.push(r["visual"].as_str().unwrap_or_default().to_string());
    }
    let lit = |s: &str| json!({"Literal": {"Value": s}});
    for (name, toggled) in names.iter().zip(["'Home'", "'Car'"]) {
        let vis = s
            .report
            .as_mut()
            .and_then(|r| r.page_mut("Main"))
            .and_then(|p| p.visual_mut(name))
            .unwrap_or_else(|| panic!("{name}"));
        let mut query = bicraft_report::query::effective(vis).unwrap_or_default();
        for p in query["queryState"]["Rows"]["projections"]
            .as_array_mut()
            .into_iter()
            .flatten()
        {
            p["active"] = json!(true);
        }
        vis.visual_extra.insert("query".into(), query);
        vis.visual_extra.insert(
            "expansionStates".into(),
            json!([{"roles": ["Rows"], "levels": [{"queryRefs": ["Category.Family"], "isCollapsed": true},
                   {"queryRefs": ["Category.Category"], "isCollapsed": true}],
                   "root": {"children": [{"identityValues": [lit(toggled)], "isToggled": true}]}}]),
        );
    }
    s.revision += 1;
    for (name, family) in names.iter().zip(["Home", "Car"]) {
        let d = visual(&s, name)["data"].clone();
        assert_eq!(d["row_expansion"]["toggled"], json!([[family]]), "{name}: {d}");
        // the Category level is evaluated for the toggled member
        assert!(
            d["subtotals"]
                .as_array()
                .is_some_and(|l| l.iter().any(|l| l["rows"] == 1)),
            "{d}"
        );
    }
}
