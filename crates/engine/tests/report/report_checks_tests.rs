//! `report.validate`: filter types against conditions and fields, hierarchy and date variation
//! references, bookmark targets.

use bicraft_report::NamedDocument;
use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

const BIM: &str = r#"{"name":"M","model":{"tables":[
  {"name":"Sales","columns":[
     {"name":"Product","dataType":"string"},
     {"name":"Amount","dataType":"double"},
     {"name":"Qty","dataType":"int64"},
     {"name":"Date","dataType":"dateTime","variations":[{"name":"Variation","relationship":"r1",
        "defaultHierarchy":{"table":"LocalDateTable_1","hierarchy":"Date Hierarchy"},"isDefault":true}]}],
   "measures":[{"name":"Total","expression":"SUM(Sales[Amount])"}]},
  {"name":"LocalDateTable_1","isHidden":true,"columns":[
     {"name":"Date","dataType":"dateTime"},{"name":"Year","dataType":"int64"},{"name":"Month","dataType":"string"}],
   "hierarchies":[{"name":"Date Hierarchy","levels":[{"name":"Year","ordinal":0,"column":"Year"},
                                                     {"name":"Month","ordinal":1,"column":"Month"}]}]},
  {"name":"Calendar","columns":[{"name":"Year","dataType":"int64"},{"name":"Month","dataType":"int64"}],
   "hierarchies":[{"name":"YM","levels":[{"name":"Year","ordinal":0,"column":"Year"},
                                         {"name":"Month","ordinal":1,"column":"Month"}]}]}],
 "relationships":[{"name":"r1","fromTable":"Sales","fromColumn":"Date","toTable":"LocalDateTable_1","toColumn":"Date"}]}}"#;

/// The model above, a report with two pages, a table visual on the first and a card on the
/// second. Returns the session, the page names and the visual names.
fn session(name: &str) -> (Session, TempDir, [String; 2], [String; 2]) {
    let dir = TempDir::new(&format!("report-checks-{name}"));
    let path = dir.join("model.bim");
    std::fs::write(&path, BIM).unwrap();
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": path})).unwrap();
    s.execute("report.new", &json!({})).unwrap();
    s.execute("page.add", &json!({"display_name": "Two"})).unwrap();
    let v1 = s
        .execute(
            "visual.add",
            &json!({"page": "Page 1", "visual_type": "tableEx", "x": 0, "y": 0, "width": 300, "height": 200,
                    "fields": [{"role": "Values", "field": "Sales[Product]"}]}),
        )
        .unwrap();
    let v2 = s
        .execute(
            "visual.add",
            &json!({"page": "Two", "visual_type": "card", "x": 0, "y": 0, "width": 300, "height": 200,
                    "fields": [{"role": "Values", "field": "[Total]"}]}),
        )
        .unwrap();
    let r = s.report.as_ref().unwrap();
    let pages = [r.pages[0].name.clone(), r.pages[1].name.clone()];
    let visuals = [
        v1["visual"].as_str().unwrap().to_string(),
        v2["visual"].as_str().unwrap().to_string(),
    ];
    (s, dir, pages, visuals)
}

fn col(src: &str, prop: &str) -> Value {
    json!({"Column": {"Expression": {"SourceRef": {"Source": src}}, "Property": prop}})
}

fn ecol(entity: &str, prop: &str) -> Value {
    json!({"Column": {"Expression": {"SourceRef": {"Entity": entity}}, "Property": prop}})
}

fn emeasure(entity: &str, prop: &str) -> Value {
    json!({"Measure": {"Expression": {"SourceRef": {"Entity": entity}}, "Property": prop}})
}

fn lit(v: &str) -> Value {
    json!({"Literal": {"Value": v}})
}

fn relative(c: Value) -> Value {
    json!({"Between": {"Expression": c,
        "LowerBound": {"DateSpan": {"Expression": {"DateAdd": {"Expression": {"Now": {}}, "Amount": -1, "TimeUnit": 5}}, "TimeUnit": 0}},
        "UpperBound": {"DateSpan": {"Expression": {"Now": {}}, "TimeUnit": 0}}}})
}

fn container(name: &str, field: Value, kind: &str, conditions: Vec<Value>) -> Value {
    let wh: Vec<Value> = conditions.into_iter().map(|c| json!({"Condition": c})).collect();
    json!({"name": name, "field": field, "type": kind,
           "filter": {"Version": 2, "From": [{"Name": "s", "Entity": "Sales", "Type": 0}], "Where": wh}})
}

fn hl(entity: &str, hierarchy: &str, level: &str) -> Value {
    json!({"HierarchyLevel": {"Expression": {"Hierarchy": {"Expression": {"SourceRef": {"Entity": entity}},
        "Hierarchy": hierarchy}}, "Level": level}})
}

fn variation_hl(variation: &str, hierarchy: &str, level: &str) -> Value {
    json!({"HierarchyLevel": {"Expression": {"Hierarchy": {"Expression": {"PropertyVariationSource": {
        "Expression": {"SourceRef": {"Entity": "Sales"}}, "Name": variation, "Property": "Date"}},
        "Hierarchy": hierarchy}}, "Level": level}})
}

/// `(severity, filter or part, message)` of every issue, schema issues aside.
fn issues(s: &mut Session) -> Vec<(String, String, String)> {
    let v = s.execute("report.validate", &json!({})).unwrap();
    v["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            let at = i["filter"]
                .as_str()
                .or(i["part"].as_str())
                .unwrap_or("")
                .to_string();
            (
                i["severity"].as_str().unwrap().to_string(),
                at,
                i["message"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn find<'a>(all: &'a [(String, String, String)], at: &str, text: &str) -> &'a (String, String, String) {
    all.iter()
        .find(|(_, a, m)| a == at && m.contains(text))
        .unwrap_or_else(|| panic!("no issue at {at} containing `{text}`: {all:#?}"))
}

#[test]
fn filter_types_are_checked_against_their_field_and_conditions() {
    let (mut s, dir, _, _) = session("filters");
    {
        let r = s.report.as_mut().unwrap();
        let v = &mut r.pages[0].visuals[0];
        v.filter_config = Some(json!({"filters": [
            container("ok", ecol("Sales", "Product"), "Categorical",
                vec![json!({"In": {"Expressions": [col("s", "Product")], "Values": [[lit("'A'")], [lit("null")]]}})]),
            container("okRel", ecol("Sales", "Date"), "RelativeDate", vec![relative(col("s", "Date"))]),
            container("okAdv", emeasure("Sales", "Total"), "Advanced",
                vec![json!({"Comparison": {"ComparisonKind": 1, "Left": {"Measure": {"Expression": {"SourceRef": {"Source": "s"}}, "Property": "Total"}}, "Right": lit("1D")}})]),
            container("relText", ecol("Sales", "Product"), "RelativeDate", vec![relative(col("s", "Product"))]),
            container("catRange", ecol("Sales", "Amount"), "Categorical",
                vec![json!({"Comparison": {"ComparisonKind": 1, "Left": col("s", "Amount"), "Right": lit("6D")}})]),
            container("topNoSub", ecol("Sales", "Product"), "TopN",
                vec![json!({"In": {"Expressions": [col("s", "Product")], "Values": [[lit("'A'")]]}})]),
            container("catMeasure", emeasure("Sales", "Total"), "Categorical",
                vec![json!({"In": {"Expressions": [{"Measure": {"Expression": {"SourceRef": {"Source": "s"}}, "Property": "Total"}}], "Values": [[lit("1D")]]}})]),
            container("containsNum", ecol("Sales", "Amount"), "Advanced",
                vec![json!({"Contains": {"Left": col("s", "Amount"), "Right": lit("'1'")}})]),
            container("textOnInt", ecol("Sales", "Qty"), "Categorical",
                vec![json!({"In": {"Expressions": [col("s", "Qty")], "Values": [[lit("'abc'")]]}})]),
            container("advNow", ecol("Sales", "Date"), "Advanced", vec![relative(col("s", "Date"))]),
            container("relNoNow", ecol("Sales", "Date"), "RelativeDate",
                vec![json!({"Comparison": {"ComparisonKind": 2, "Left": col("s", "Date"), "Right": lit("datetime'2024-01-01T00:00:00'")}})]),
            // No condition yet: an empty filter card.
            {"name": "empty", "field": ecol("Sales", "Qty"), "type": "Categorical"},
            {"name": "pass", "type": "Passthrough", "filter": {"Version": 2, "From": [], "Where": [{"Condition": {"Now": {}}}]}},
        ]}));
        r.pages[0].filter_config = Some(json!({"filters": [
            container("pageMeasure", emeasure("Sales", "Total"), "Advanced",
                vec![json!({"Comparison": {"ComparisonKind": 1, "Left": {"Measure": {"Expression": {"SourceRef": {"Source": "s"}}, "Property": "Total"}}, "Right": lit("1D")}})]),
        ]}));
    }
    let all = issues(&mut s);
    for ok in ["ok", "okRel", "okAdv", "empty", "pass"] {
        assert!(!all.iter().any(|(_, a, _)| a == ok), "{ok}: {all:#?}");
    }
    assert_eq!(find(&all, "relText", "needs a date column").0, "error");
    assert_eq!(find(&all, "catRange", "range comparisons").0, "warning");
    assert_eq!(find(&all, "topNoSub", "Top N subquery").0, "warning");
    assert_eq!(find(&all, "catMeasure", "cannot filter a measure").0, "warning");
    assert_eq!(find(&all, "containsNum", "applies to text").0, "error");
    assert_eq!(
        find(&all, "textOnInt", "a text value is compared with Sales[Qty]").0,
        "error"
    );
    assert_eq!(find(&all, "advNow", "relative dates").0, "warning");
    assert_eq!(find(&all, "relNoNow", "relative to now").0, "warning");
    assert_eq!(
        find(&all, "pageMeasure", "offers measures only as visual filters").0,
        "warning"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hierarchies_levels_and_date_variations_are_resolved() {
    let (mut s, dir, _, _) = session("hierarchies");
    {
        let r = s.report.as_mut().unwrap();
        let v = &mut r.pages[0].visuals[0];
        v.visual_extra.insert(
            "query".into(),
            json!({"queryState": {"Values": {"projections": [
                {"field": hl("Calendar", "YM", "Year")},
                {"field": hl("Calendar", "Nope", "Year")},
                {"field": hl("Calendar", "ym", "Week")},
                {"field": variation_hl("Variation", "Date Hierarchy", "Month")},
                {"field": variation_hl("Other", "Date Hierarchy", "Year")},
                {"field": variation_hl("Variation", "Fiscal", "Year")},
                // A missing table is a field issue, reported once by the field check.
                {"field": hl("Gone", "H", "L")},
            ]}}}),
        );
        v.filter_config = Some(json!({"filters": [
            {"name": "relLevel", "field": variation_hl("Variation", "Date Hierarchy", "Month"), "type": "RelativeDate"},
            container("relLevel2", variation_hl("Variation", "Date Hierarchy", "Month"), "RelativeDate",
                vec![relative(variation_hl("Variation", "Date Hierarchy", "Month"))]),
        ]}));
        r.bookmarks.push(NamedDocument {
            name: "bm".into(),
            document: json!({"explorationState": {"version": "1.3", "activeSection": r.pages[0].name,
                "sections": {}, "filters": {"byExpr": [{"name": "f", "type": "Categorical",
                    "expression": hl("Calendar", "Nope", "Year")}]}}}),
        });
    }
    let all = issues(&mut s);
    let props = "properties";
    assert_eq!(
        find(&all, props, "table `Calendar` has no hierarchy `Nope`").0,
        "error"
    );
    assert_eq!(
        find(
            &all,
            props,
            "hierarchy `YM` of table `Calendar` has no level `Week`"
        )
        .0,
        "error"
    );
    assert_eq!(
        find(&all, props, "Sales[Date] has no date variation `Other`").0,
        "error"
    );
    assert_eq!(
        find(&all, props, "table `LocalDateTable_1` has no hierarchy `Fiscal`").0,
        "error"
    );
    assert_eq!(find(&all, "bookmark bm", "no hierarchy `Nope`").0, "warning");
    assert!(
        !all.iter()
            .any(|(_, _, m)| m.contains("`Gone`") && m.contains("hierarchy")),
        "{all:#?}"
    );
    // Month of the date hierarchy is a text column: not a date for a relative date filter.
    assert_eq!(find(&all, "relLevel", "needs a date column").0, "error");
    assert_eq!(find(&all, "relLevel2", "needs a date column").0, "error");
    // Each problem once, although the same JSON is walked through several documents.
    let nope = all
        .iter()
        .filter(|(_, a, m)| a == props && m.contains("no hierarchy `Nope`"))
        .count();
    assert_eq!(nope, 1, "{all:#?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A column of a date variation (`Column` over a `PropertyVariationSource`, as a visual bound to
/// an auto date/time level without its hierarchy): the variation's name and the column of its
/// date table are both checked.
#[test]
fn date_variation_columns_are_resolved() {
    let (mut s, dir, _, _) = session("variation-columns");
    let vcol = |variation: &str, prop: &str| {
        json!({"Column": {"Expression": {"PropertyVariationSource": {
            "Expression": {"SourceRef": {"Entity": "Sales"}}, "Name": variation, "Property": "Date"}},
            "Property": prop}})
    };
    {
        let r = s.report.as_mut().unwrap();
        let v = &mut r.pages[0].visuals[0];
        v.visual_extra.insert(
            "query".into(),
            json!({"queryState": {"Values": {"projections": [
                {"field": vcol("Variation", "Year")},
                {"field": vcol("Variation", "Quarter")},
                {"field": vcol("Wrong", "Year")},
            ]}}}),
        );
        r.bookmarks.push(NamedDocument {
            name: "bm".into(),
            document: json!({"explorationState": {"version": "1.3", "activeSection": r.pages[0].name,
                "sections": {}, "filters": {"byExpr": [{"name": "f", "type": "Categorical",
                    "expression": vcol("Variation", "Day")}]}}}),
        });
    }
    let all = issues(&mut s);
    let props = "properties";
    assert_eq!(
        find(
            &all,
            props,
            "date variation `Variation` of Sales[Date] has no column `Quarter`"
        )
        .0,
        "error"
    );
    assert_eq!(
        find(&all, props, "Sales[Date] has no date variation `Wrong`").0,
        "error"
    );
    assert_eq!(find(&all, "bookmark bm", "has no column `Day`").0, "warning");
    assert!(!all.iter().any(|(_, _, m)| m.contains("`Year`")), "{all:#?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bookmark_targets_are_checked_and_only_warned_about() {
    let (mut s, dir, pages, visuals) = session("bookmarks");
    let [p1, p2] = pages;
    let [_, v2] = visuals;
    {
        let r = s.report.as_mut().unwrap();
        let bm = |name: &str, es: Value, options: Option<Value>| {
            let mut d = json!({"displayName": name, "name": name, "explorationState": es});
            if let Some(o) = options {
                d["options"] = o;
            }
            NamedDocument {
                name: name.into(),
                document: d,
            }
        };
        r.bookmarks = vec![
            bm(
                "ok",
                json!({"version": "1.3", "activeSection": p2, "sections": {p2.clone(): {"visualContainers": {v2.clone(): {"singleVisual": {"visualType": "card"}}}}}}),
                None,
            ),
            bm(
                "noPage",
                json!({"version": "1.3", "activeSection": "gone", "sections": {"gone": {"visualContainers": {}}}}),
                None,
            ),
            bm(
                "noVisual",
                json!({"version": "1.3", "activeSection": p2, "sections": {p2.clone(): {"visualContainers": {"ghost": {}}, "visualContainerGroups": {"grp": {}}}}}),
                None,
            ),
            bm(
                "wrongType",
                json!({"version": "1.3", "activeSection": p2, "sections": {p2.clone(): {"visualContainers": {v2.clone(): {"singleVisual": {"visualType": "tableEx"}}}}}}),
                None,
            ),
            bm(
                "target",
                json!({"version": "1.3", "activeSection": p1, "sections": {}}),
                Some(json!({"targetVisualNames": [v2.clone()]})),
            ),
            bm(
                "filters",
                json!({"version": "1.3", "activeSection": p1, "sections": {p1.clone(): {"visualContainers": {},
                "filters": {"byName": {"x": {"type": "RelativeDate", "expression": ecol("Sales", "Product")}}}}},
                "filters": {"byExpr": [{"type": "Categorical", "expression": emeasure("Sales", "Total")}]}}),
                None,
            ),
        ];
    }
    let all = issues(&mut s);
    assert!(!all.iter().any(|(_, a, _)| a == "bookmark ok"), "{all:#?}");
    find(&all, "bookmark noPage", "active page `gone` does not exist");
    find(&all, "bookmark noPage", "page `gone`, which does not exist");
    find(
        &all,
        "bookmark noVisual",
        "visual `ghost`, which is not on page `Two`",
    );
    find(&all, "bookmark noVisual", "group `grp`");
    find(&all, "bookmark wrongType", "as a tableEx, but it is a card");
    find(&all, "bookmark target", "target visual");
    find(&all, "bookmark filters", "needs a date column");
    find(&all, "bookmark filters", "cannot filter a measure");
    for (sev, at, m) in &all {
        if at.starts_with("bookmark") {
            assert_eq!(sev, "warning", "{at}: {m}");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hostile_shapes_do_not_crash() {
    let (mut s, dir, _, _) = session("hostile");
    {
        let r = s.report.as_mut().unwrap();
        let mut deep = json!({"Now": {}});
        for _ in 0..100 {
            deep = json!({"Not": {"Expression": deep}});
        }
        r.filter_config = Some(json!({"filters": [
            1, "x", null, {"type": 3}, {"type": "Categorical", "field": 2, "filter": []},
            {"type": "Categorical", "field": {"Column": {"Expression": {"PropertyVariationSource": 1}, "Property": 2}},
             "filter": {"From": "x", "Where": [{"Condition": deep}, 7, {"Condition": {"In": {"Expressions": 1, "Values": [[1], 2]}}}]}},
            {"type": "TopN", "field": {"HierarchyLevel": {"Expression": {"Hierarchy": 3}, "Level": 4}},
             "filter": {"Where": [{"Condition": {"In": {"Table": 1}}}]}},
            {"type": "Advanced", "field": ecol("Sales", "Qty"),
             "filter": {"Where": [{"Condition": {"Comparison": {"Left": ecol("Sales", "Qty"), "Right": {"Literal": {"Value": "D"}}}}}]}},
        ]}));
        r.bookmarks.push(NamedDocument {
            name: "bad".into(),
            document: json!({"options": {"targetVisualNames": [1, null]}, "explorationState": {"activeSection": 5,
                "sections": {"x": 1, "y": {"visualContainers": [1]}}, "filters": {"byName": [], "byExpr": {}}}}),
        });
        r.bookmarks.push(NamedDocument {
            name: "bad2".into(),
            document: json!([1, 2]),
        });
    }
    let all = issues(&mut s);
    assert!(all.iter().any(|(_, a, _)| a == "bookmark bad"), "{all:#?}");
    let _ = std::fs::remove_dir_all(&dir);
}
