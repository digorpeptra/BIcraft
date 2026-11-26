//! Relative date and relative time filters on filter cards (`relative` of `visual.filter`,
//! `page.filter`, `report.filter`), Top N kept to visual filters, and the lock and eye of a card
//! (`hidden`, `locked` without a condition): the rows each filter keeps under a pinned clock, the
//! PBIR Desktop's shape, the PBIP and PBIX round trip, one undo step per edit, hostile params.

use serde_json::{Value, json};

use crate::Session;
use crate::tempdir::TempDir;

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

const SALES: &str = "#table(type table [Day = date, Amount = number, Label = text], {\
    {#date(2026, 10, 15), 1, \"a\"}, {#date(2026, 11, 1), 2, \"b\"}, {#date(2026, 11, 15), 3, \"c\"}, \
    {#date(2026, 11, 16), 4, \"d\"}, {#date(2026, 11, 17), 5, \"e\"}, {#date(2026, 11, 18), 6, \"f\"}, \
    {#date(2026, 11, 19), 7, \"g\"}, {#date(2026, 11, 20), 8, \"h\"}, {#date(2026, 12, 5), 9, \"i\"}, \
    {#date(2027, 1, 10), 10, \"j\"}})";

const EVENTS: &str = "#table(type table [At = datetime, N = number], {\
    {#datetime(2026, 11, 17, 10, 0, 0), 1}, {#datetime(2026, 11, 18, 8, 0, 0), 2}, \
    {#datetime(2026, 11, 18, 9, 0, 0), 3}, {#datetime(2026, 11, 18, 10, 0, 0), 4}, \
    {#datetime(2026, 11, 18, 10, 29, 0), 5}, {#datetime(2026, 11, 18, 10, 45, 0), 6}, \
    {#datetime(2026, 11, 18, 12, 0, 0), 7}})";

/// Sales(Day, Amount, Label) and Events(At, N); a page "Main" with a table of each; the clock
/// pinned to Wednesday 2026-11-18 10:30.
fn dates() -> (Session, String, String) {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Dates"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": SALES}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Day", "data_type": "dateTime"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Label", "data_type": "string"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        (
            "table.add",
            json!({"name": "Events", "partition": {"source_type": "m", "expression": EVENTS}}),
        ),
        (
            "column.add",
            json!({"table": "Events", "name": "At", "data_type": "dateTime"}),
        ),
        (
            "column.add",
            json!({"table": "Events", "name": "N", "data_type": "double"}),
        ),
        ("report.new", json!({})),
        ("page.add", json!({"display_name": "Main"})),
        ("view.set_as_of", json!({"as_of": "2026-11-18T10:30:00"})),
    ] {
        run(&mut s, id, p);
    }
    let mut add = |fields: Value| {
        run(
            &mut s,
            "visual.add",
            json!({"page": "Main", "visual_type": "tableEx", "x": 0, "y": 0, "width": 300, "height": 300,
                   "fields": fields}),
        )["visual"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let sales =
        add(json!([{"role": "Values", "field": "Sales[Label]"}, {"role": "Values", "field": "[Total]"}]));
    let events = add(json!([{"role": "Values", "field": "Sum(Events[N])"}]));
    (s, sales, events)
}

fn visual_json(s: &Session, visual: &str) -> Value {
    let snap = s.snapshot();
    for p in snap["report"]["pages"].as_array().cloned().unwrap_or_default() {
        for v in p["visuals"].as_array().cloned().unwrap_or_default() {
            if v["name"] == visual {
                return v;
            }
        }
    }
    panic!("visual {visual} not in the snapshot")
}

/// The sales labels the table shows, sorted.
fn labels(s: &Session, visual: &str) -> String {
    let d = visual_json(s, visual)["data"].clone();
    assert_eq!(d["status"], "ok", "{d}");
    let mut out: Vec<String> = d["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r[0].as_str().map(str::to_string))
        .collect();
    out.sort();
    out.join("")
}

/// The events' sum the card shows.
fn events_sum(s: &Session, visual: &str) -> f64 {
    let d = visual_json(s, visual)["data"].clone();
    assert_eq!(d["status"], "ok", "{d}");
    d["rows"][0][0].as_f64().unwrap_or(0.0)
}

fn main_page(s: &Session) -> &bicraft_report::Page {
    s.report
        .as_ref()
        .unwrap()
        .pages
        .iter()
        .find(|p| p.display_name == "Main")
        .unwrap()
}

#[test]
fn relative_date_filters_keep_the_right_days_and_round_trip() {
    let (mut s, sales, _) = dates();
    assert_eq!(labels(&s, &sales), "abcdefghij");
    // labels: a 10-15, b 11-01, c 11-15, d 11-16, e 11-17, f 11-18 (today), g 11-19, h 11-20,
    // i 12-05, j 2027-01-10; weeks start on Sunday 11-15
    for (relative, want) in [
        (json!({"range": "last", "count": 3, "unit": "days"}), "def"),
        (
            json!({"range": "last", "count": 3, "unit": "days", "include_today": false}),
            "cde",
        ),
        (json!({"range": "next", "count": 2, "unit": "days"}), "fg"),
        (
            json!({"range": "next", "count": 2, "unit": "days", "include_today": false}),
            "gh",
        ),
        (json!({"range": "this", "unit": "months"}), "bcdefgh"),
        (json!({"range": "this", "unit": "weeks"}), "cdefgh"),
        (
            json!({"range": "last", "count": 1, "unit": "calendar_months", "include_today": false}),
            "a",
        ),
        // a calendar period with today included runs on to today
        (
            json!({"range": "last", "count": 1, "unit": "calendar_months"}),
            "abcdef",
        ),
        (
            json!({"range": "next", "count": 1, "unit": "calendar months", "include_today": false}),
            "i",
        ),
        (
            json!({"range": "next", "count": 1, "unit": "calendar_months"}),
            "fghi",
        ),
        (
            json!({"range": "next", "count": 2, "unit": "calendar_months", "include_today": false}),
            "ij",
        ),
        (json!({"range": "last", "count": 1, "unit": "years"}), "abcdef"),
        (json!({"range": "next", "count": 1, "unit": "weeks"}), "fgh"),
    ] {
        let out = run(
            &mut s,
            "page.filter",
            json!({"page": "Main", "field": "Sales[Day]", "relative": relative}),
        );
        assert_eq!(out["filters"][0]["type"], "relative", "{relative}");
        assert_eq!(labels(&s, &sales), want, "{relative}");
    }
    // the card reads back as written
    let out = run(
        &mut s,
        "page.filter",
        json!({"page": "Main", "field": "Sales[Day]", "relative": {"range": "last", "count": 10, "unit": "years"}}),
    );
    assert_eq!(
        out["filters"][0]["condition"],
        json!({"relative": {"range": "last", "count": 10, "unit": "years", "include_today": true}})
    );
    // Desktop's shape: type RelativeDate, a Between whose bounds are DateSpan / DateAdd / Now
    let c = &main_page(&s).filter_config.as_ref().unwrap()["filters"][0];
    assert_eq!(c["type"], "RelativeDate");
    let cond = &c["filter"]["Where"][0]["Condition"]["Between"];
    assert_eq!(
        cond["LowerBound"],
        json!({"DateSpan": {"Expression": {"DateAdd": {"Expression": {"DateAdd": {"Expression": {"Now": {}},
            "Amount": 1, "TimeUnit": 0}}, "Amount": -10, "TimeUnit": 3}}, "TimeUnit": 0}})
    );
    assert_eq!(
        cond["UpperBound"],
        json!({"DateSpan": {"Expression": {"Now": {}}, "TimeUnit": 0}})
    );
    // a visual filter and a report filter too
    run(
        &mut s,
        "page.clear_filters",
        json!({"page": "Main", "field": "Sales[Day]"}),
    );
    run(
        &mut s,
        "visual.filter",
        json!({"page": "Main", "visual": sales, "field": "Sales[Day]",
               "relative": {"range": "last", "count": 2, "unit": "calendar_weeks", "include_today": true}}),
    );
    // weeks of 11-01 and 11-08, then this week up to today
    assert_eq!(labels(&s, &sales), "bcdef");
    run(
        &mut s,
        "report.filter",
        json!({"field": "Sales[Day]", "relative": {"range": "this", "unit": "years"}}),
    );
    assert_eq!(labels(&s, &sales), "bcdef");
    let snap = s.snapshot();
    assert_eq!(snap["report"]["filters"][0]["type"], "relative");
    assert_eq!(
        snap["report"]["filters"][0]["condition"]["relative"]["range"],
        "this"
    );
    // the same day later: a new clock moves the window
    run(&mut s, "view.set_as_of", json!({"as_of": "2026-11-21"}));
    assert_eq!(
        labels(&s, &sales),
        "bcdefgh",
        "the last two calendar weeks and this one"
    );
    // one undo step per edit
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.snapshot()["report"]["filters"], json!([]));
    run(&mut s, "edit.redo", json!({}));
    run(&mut s, "view.set_as_of", json!({"as_of": "2026-11-18T10:30:00"}));

    // PBIP and PBIX keep them
    let dir = TempDir::new("filter-relative");
    for path in [dir.join("pbip").join("Dates.pbip"), dir.join("Dates.pbix")] {
        run(&mut s, "file.save_as", json!({"path": path}));
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": path}));
        run(
            &mut back,
            "view.set_as_of",
            json!({"as_of": "2026-11-18T10:30:00"}),
        );
        let r = back.report.as_ref().unwrap();
        let rf = crate::filter_edit::cards(r.filter_config.as_ref());
        assert_eq!(
            rf[0]["condition"],
            json!({"relative": {"range": "this", "count": 1, "unit": "years", "include_today": true}}),
            "{path:?}"
        );
        assert_eq!(labels(&back, &sales), "bcdef", "{path:?}");
    }
}

#[test]
fn relative_time_filters_compare_to_the_minute() {
    let (mut s, _, events) = dates();
    assert_eq!(events_sum(&s, &events), 28.0);
    // events: 1 yesterday 10:00, 2 08:00, 3 09:00, 4 10:00, 5 10:29, 6 10:45, 7 12:00; now 10:30
    for (relative, want) in [
        (
            json!({"range": "last", "count": 2, "unit": "hours"}),
            3.0 + 4.0 + 5.0,
        ),
        (json!({"range": "next", "count": 1, "unit": "hours"}), 6.0),
        (json!({"range": "this", "unit": "hours"}), 4.0 + 5.0 + 6.0),
        (
            json!({"range": "last", "count": 45, "unit": "minutes"}),
            4.0 + 5.0,
        ),
        (
            json!({"range": "next", "count": 90, "unit": "minutes"}),
            6.0 + 7.0,
        ),
    ] {
        let out = run(
            &mut s,
            "visual.filter",
            json!({"page": "Main", "visual": events, "field": "Events[At]", "relative": relative}),
        );
        assert_eq!(out["changed"], true, "{relative}");
        assert_eq!(events_sum(&s, &events), want, "{relative}");
    }
    let v = main_page(&s).visuals.iter().find(|v| v.name == events).unwrap();
    let c = &v.filter_config.as_ref().unwrap()["filters"][0];
    assert_eq!(c["type"], "RelativeTime");
    assert_eq!(
        c["filter"]["Where"][0]["Condition"]["Between"]["UpperBound"],
        json!({"DateAdd": {"Expression": {"Now": {}}, "Amount": 90, "TimeUnit": 6}})
    );
    // a minute later the window moves
    run(&mut s, "view.set_as_of", json!({"as_of": "2026-11-18T10:31:00"}));
    run(
        &mut s,
        "visual.filter",
        json!({"page": "Main", "visual": events, "field": "Events[At]",
               "relative": {"range": "last", "count": 3, "unit": "minutes"}}),
    );
    assert_eq!(events_sum(&s, &events), 5.0);
    run(&mut s, "view.set_as_of", json!({"as_of": "2026-11-18T10:33:00"}));
    let d = visual_json(&s, &events)["data"].clone();
    assert!(
        d["rows"]
            .as_array()
            .is_none_or(|r| r.is_empty() || r[0][0].is_null()),
        "nothing in the last 3 minutes: {d}"
    );
}

#[test]
fn bad_relative_filters_and_page_top_n_are_refused() {
    let (mut s, sales, _) = dates();
    for (id, p, want) in [
        (
            "page.filter",
            json!({"page": "Main", "field": "Sales[Label]", "relative": {"unit": "days"}}),
            "not a date",
        ),
        (
            "visual.filter",
            json!({"page": "Main", "visual": sales, "field": "[Total]", "relative": {"unit": "days"}}),
            "date columns",
        ),
        (
            "page.filter",
            json!({"page": "Main", "field": "Sales[Day]", "relative": {"unit": "fortnights"}}),
            "fortnights",
        ),
        (
            "page.filter",
            json!({"page": "Main", "field": "Sales[Day]", "relative": {"unit": "days", "count": 0}}),
            "count",
        ),
        (
            "page.filter",
            json!({"page": "Main", "field": "Sales[Day]", "relative": "last 3 days"}),
            "object",
        ),
        (
            "page.filter",
            json!({"page": "Main", "field": "Sales[Day]", "in": ["2026-11-18"], "relative": {"unit": "days"}}),
            "exactly one",
        ),
        (
            "report.filter",
            json!({"field": "Sales[Day]", "relative": {"unit": "calendar_days"}}),
            "calendar",
        ),
        // Top N: visual filters only, as in Desktop
        (
            "page.filter",
            json!({"page": "Main", "field": "Sales[Label]", "top": 2, "by": "[Total]"}),
            "visual-level",
        ),
        (
            "report.filter",
            json!({"field": "Sales[Label]", "top": 2, "by": "[Total]"}),
            "visual-level",
        ),
    ] {
        let e = fails(&mut s, id, p.clone());
        assert!(e.contains(want), "{p}: {e}");
    }
    assert!(s.report.as_ref().unwrap().filter_config.is_none());
    assert!(main_page(&s).filter_config.is_none());
    // still on a visual
    run(
        &mut s,
        "visual.filter",
        json!({"page": "Main", "visual": sales, "field": "Sales[Label]", "top": 2, "by": "[Total]"}),
    );
    assert_eq!(labels(&s, &sales), "ij");
}

#[test]
fn lock_and_hide_keep_the_condition_and_round_trip() {
    let (mut s, sales, _) = dates();
    run(
        &mut s,
        "page.filter",
        json!({"page": "Main", "field": "Sales[Day]", "relative": {"range": "last", "count": 3, "unit": "days"}}),
    );
    // the lock: one command, one undo step, the condition kept
    let out = run(
        &mut s,
        "page.filter",
        json!({"page": "Main", "field": "Sales[Day]", "locked": true}),
    );
    assert_eq!(out["changed"], true);
    assert_eq!(out["filters"][0]["locked"], true);
    assert_eq!(out["filters"][0]["hidden"], false);
    assert_eq!(out["filters"][0]["type"], "relative");
    assert_eq!(labels(&s, &sales), "def");
    // the eye
    let out = run(
        &mut s,
        "page.filter",
        json!({"page": "Main", "field": "Sales[Day]", "hidden": true}),
    );
    assert_eq!(out["filters"][0]["hidden"], true);
    assert_eq!(labels(&s, &sales), "def", "a hidden filter still applies");
    let c = &main_page(&s).filter_config.as_ref().unwrap()["filters"][0];
    assert_eq!(c["isHiddenInViewMode"], true);
    assert_eq!(c["isLockedInViewMode"], true);
    // a visual's card and a report card
    run(
        &mut s,
        "visual.filter",
        json!({"page": "Main", "visual": sales, "field": "Sales[Label]", "in": ["d", "e"]}),
    );
    run(
        &mut s,
        "visual.filter",
        json!({"page": "Main", "visual": sales, "field": "Sales[Label]", "locked": true}),
    );
    assert_eq!(labels(&s, &sales), "de");
    run(
        &mut s,
        "report.filter",
        json!({"field": "Sales[Amount]", "hidden": true}),
    );
    assert_eq!(s.snapshot()["report"]["filters"][0]["hidden"], true);
    // unlocking writes false; undo steps back one toggle at a time
    run(
        &mut s,
        "report.filter",
        json!({"field": "Sales[Amount]", "hidden": false}),
    );
    assert_eq!(s.snapshot()["report"]["filters"][0]["hidden"], false);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.snapshot()["report"]["filters"][0]["hidden"], true);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.snapshot()["report"]["filters"], json!([]));
    run(&mut s, "edit.redo", json!({}));
    // round trip
    let dir = TempDir::new("filter-lock");
    for path in [dir.join("pbip").join("Dates.pbip"), dir.join("Dates.pbix")] {
        run(&mut s, "file.save_as", json!({"path": path}));
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": path}));
        let page = back
            .report
            .as_ref()
            .unwrap()
            .pages
            .iter()
            .find(|p| p.display_name == "Main")
            .unwrap();
        let pf = crate::filter_edit::cards(page.filter_config.as_ref());
        assert_eq!(
            (pf[0]["locked"].clone(), pf[0]["hidden"].clone()),
            (json!(true), json!(true)),
            "{path:?}"
        );
        assert_eq!(pf[0]["type"], "relative", "{path:?}");
        let vf = &visual_json(&back, &sales)["filters"];
        assert_eq!(vf[0]["locked"], true, "{path:?}");
    }
    // hostile flags
    for bad in [json!("yes"), json!(1), json!([true])] {
        fails(
            &mut s,
            "page.filter",
            json!({"page": "Main", "field": "Sales[Day]", "locked": bad}),
        );
    }
}
