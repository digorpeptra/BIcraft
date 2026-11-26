//! M10.2: UI-language phrases, auto titles and field labels in the snapshot.

use serde_json::{Value, json};

use super::*;
use crate::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn session() -> Session {
    let mut s = Session::new();
    let steps = [
        ("model.new", json!({"name": "M"})),
        ("table.add", json!({"name": "Sales"})),
        (
            "column.add",
            json!({"table": "Sales", "name": "Region", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Category", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        ("report.new", json!({})),
        ("page.add", json!({"display_name": "Main"})),
    ];
    for (id, p) in steps {
        run(&mut s, id, p);
    }
    s
}

fn add_visual(s: &mut Session, visual_type: &str, fields: Value) -> String {
    let r = run(
        s,
        "visual.add",
        json!({"page": "Main", "visual_type": visual_type, "x": 0, "y": 0, "width": 300, "height": 200,
               "fields": fields}),
    );
    r.get("visual")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| panic!("no visual name in {r}"))
}

fn visual(snap: &Value, name: &str) -> Value {
    for p in snap["report"]["pages"].as_array().cloned().unwrap_or_default() {
        for v in p["visuals"].as_array().cloned().unwrap_or_default() {
            if v["name"] == name {
                return v;
            }
        }
    }
    panic!("visual {name} not in the snapshot")
}

#[test]
fn tags_are_parsed_and_normalized() {
    let fr = UiLanguage::parse("fr_fr").unwrap();
    assert_eq!(fr.tag(), "fr-FR");
    assert_eq!(UiLanguage::parse(" PT-br ").unwrap().tag(), "pt-BR");
    assert_eq!(UiLanguage::parse("zh-hant-tw").unwrap().tag(), "zh-Hant-TW");
    assert_eq!(UiLanguage::default().tag(), "en-US");
    for bad in ["", "f", "fr-", "français", "fr FR", "1a", &"x".repeat(50)] {
        assert!(UiLanguage::parse(bad).is_err(), "{bad:?}");
    }
    // A language without a table is accepted and falls back to English.
    let sv = UiLanguage::parse("sv-SE").unwrap();
    let j = sv.snapshot_json();
    assert_eq!(j["language"], "sv-SE");
    assert_eq!(j["strings_language"], "en");
    assert_eq!(j["fallback"], true);
    assert_eq!(j["strings"]["blank"], "(Blank)");
}

#[test]
fn every_language_has_a_complete_table() {
    for code in SUPPORTED_LANGUAGES {
        let l = UiLanguage::parse(code).unwrap();
        let j = l.snapshot_json();
        assert_eq!(j["strings_language"], *code);
        assert!(j.get("fallback").is_none());
        let strings = j["strings"].as_object().unwrap();
        for (k, v) in strings {
            if k == "aggregations" {
                for (a, t) in v.as_object().unwrap() {
                    assert!(t.as_str().unwrap().contains("{0}"), "{code} {a}");
                }
            } else {
                assert!(!v.as_str().unwrap().is_empty(), "{code} {k}");
            }
        }
        let by = strings["title_by"].as_str().unwrap();
        assert!(by.contains("{v}") && by.contains("{c}"), "{code}");
        // Every aggregation code the report model uses has a name.
        for f in [
            "Sum",
            "Avg",
            "Count",
            "Min",
            "Max",
            "CountNonNull",
            "Median",
            "StandardDeviation",
            "Variance",
        ] {
            assert!(l.aggregation(f, "X").unwrap().contains('X'), "{code} {f}");
        }
        assert!(l.aggregation("Bogus", "X").is_none());
    }
}

#[test]
fn auto_titles_per_language() {
    let vals = ["Total".to_string()];
    let cats = ["Region".to_string(), "Category".to_string()];
    let t = |tag: &str| UiLanguage::parse(tag).unwrap().auto_title(&vals, &cats);
    assert_eq!(t("en-US"), "Total by Region and Category");
    assert_eq!(t("fr-FR"), "Total par Region et Category");
    assert_eq!(t("de-DE"), "Total nach Region und Category");
    assert_eq!(t("es-ES"), "Total por Region y Category");
    assert_eq!(t("it-IT"), "Total per Region e Category");
    assert_eq!(t("nl-NL"), "Total per Region en Category");
    assert_eq!(t("pt-BR"), "Total por Region e Category");
    assert_eq!(t("ja-JP"), "RegionとCategoryごとのTotal");
    let en = UiLanguage::default();
    // Desktop 2.158 (French UI): "A, B et C par D".
    let three = ["A".to_string(), "B".to_string(), "C".to_string()];
    assert_eq!(
        UiLanguage::parse("fr")
            .unwrap()
            .auto_title(&three, &["D".to_string()]),
        "A, B et C par D"
    );
    // Values only, categories only, nothing.
    assert_eq!(en.auto_title(&vals, &[]), "Total");
    assert_eq!(en.auto_title(&[], &cats), "Region, Category");
    assert_eq!(en.auto_title(&[], &[]), "");
    // At most three values and two categories.
    let many: Vec<String> = (0..10).map(|i| format!("V{i}")).collect();
    assert_eq!(en.auto_title(&many, &many), "V0, V1 and V2 by V0 and V1");
    // Capped length.
    let long = ["x".repeat(5000)];
    assert!(en.auto_title(&long, &long).chars().count() <= MAX_LABEL);
}

#[test]
fn snapshot_carries_ui_strings_labels_and_auto_titles() {
    let mut s = session();
    let chart = add_visual(
        &mut s,
        "clusteredColumnChart",
        json!([
            {"role": "Category", "field": "Sales[Region]"},
            {"role": "Series", "field": "Sales[Category]"},
            {"role": "Y", "field": "Sum(Sales[Amount])"},
        ]),
    );
    let card = add_visual(&mut s, "card", json!([{"role": "Values", "field": "[Total]"}]));
    let slicer = add_visual(
        &mut s,
        "slicer",
        json!([{"role": "Values", "field": "Sales[Region]"}]),
    );

    let snap = s.snapshot();
    assert_eq!(snap["ui"]["language"], "en-US");
    assert_eq!(
        snap["ui"]["strings"]["multiple_selections"],
        "Multiple selections"
    );
    let c = visual(&snap, &chart);
    assert_eq!(c["auto_title"], "Sum of Amount by Region and Category");
    let labels: Vec<&str> = c["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["label"].as_str().unwrap())
        .collect();
    assert!(labels.contains(&"Sum of Amount"), "{labels:?}");
    assert_eq!(visual(&snap, &card)["auto_title"], "Total");
    assert_eq!(visual(&snap, &slicer)["auto_title"], "Region");

    let r = run(&mut s, "view.set_language", json!({"ui_language": "fr-fr"}));
    assert_eq!(r["ui_language"], "fr-FR");
    assert_eq!(r["previous"], "en-US");
    let snap = s.snapshot();
    assert_eq!(snap["ui"]["language"], "fr-FR");
    assert_eq!(snap["ui"]["strings"]["blank"], "(Vide)");
    assert_eq!(snap["ui"]["strings"]["all"], "Tout");
    assert_eq!(
        visual(&snap, &chart)["auto_title"],
        "Somme de Amount par Region et Category"
    );

    // Bad values are refused and leave the language alone.
    assert!(
        s.execute("view.set_language", &json!({"ui_language": 3}))
            .is_err()
    );
    assert!(
        s.execute("view.set_language", &json!({"ui_language": "?"}))
            .is_err()
    );
    assert!(s.execute("view.set_language", &json!({})).is_err());
    assert_eq!(s.ui_language.tag(), "fr-FR");
}

#[test]
fn renamed_fields_and_hierarchy_levels_keep_their_names() {
    let mut s = session();
    let chart = add_visual(
        &mut s,
        "clusteredColumnChart",
        json!([
            {"role": "Category", "field": "Sales[Region]"},
            {"role": "Y", "field": "Sum(Sales[Amount])"},
        ]),
    );
    // Rename the Y projection the way Desktop does (`displayName`) and add a date level.
    if let Some(r) = s.report.as_mut() {
        for v in r.pages.iter_mut().flat_map(|p| p.visuals.iter_mut()) {
            if v.name != chart {
                continue;
            }
            v.visual_extra.insert(
                "query".into(),
                json!({"queryState": {
                    "Category": {"projections": [{"queryRef": "Sales.Region"}]},
                    "Y": {"projections": [{"queryRef": "Sum(Sales.Amount)", "displayName": "Revenue"}]},
                }}),
            );
            // the reader takes the projection's name into the binding
            v.fields[1].display_name = Some("Revenue".into());
        }
    }
    let snap = s.snapshot();
    assert_eq!(visual(&snap, &chart)["auto_title"], "Revenue by Region");

    let level = json!({"HierarchyLevel": {"Expression": {"Hierarchy": {
        "Expression": {"SourceRef": {"Entity": "Dates"}}, "Hierarchy": "Calendar"}}, "Level": "Année"}});
    assert_eq!(hierarchy_level(&level).as_deref(), Some("Année"));
    assert_eq!(hierarchy_level(&json!({"Column": {}})), None);
    let lang = UiLanguage::default();
    assert_eq!(
        field_label(&lang, &FieldRef::Other { raw: level }, None).as_deref(),
        Some("Année")
    );
    assert_eq!(
        field_label(&lang, &FieldRef::Other { raw: json!(null) }, None),
        None
    );
    // An unknown aggregation keeps the function form.
    let odd = FieldRef::Aggregation {
        table: "T".into(),
        column: "C".into(),
        function: "Weird".into(),
    };
    assert_eq!(field_label(&lang, &odd, None).as_deref(), Some("Weird(C)"));
}

#[test]
fn render_param_is_checked_and_leaves_the_session_language() {
    let mut s = session();
    assert!(param(&json!({"ui_language": ["fr"]})).is_err());
    assert_eq!(param(&json!({})).unwrap(), None);
    assert!(
        s.execute("report.render", &json!({"ui_language": "not a tag"}))
            .is_err()
    );
    assert_eq!(s.ui_language.tag(), "en-US");
}

/// English UI strings seen in Power BI Desktop 2.158 with its application language set to
/// English (M10.2 oracle): every implicit-measure name (both counts read "Count of"), titles,
/// "(Blank)", "Total", slicer and waterfall texts. The prompts are BIcraft's own wording: each
/// language has them, in its own words.
#[test]
fn english_strings_match_desktop_in_english() {
    let en = UiLanguage::parse("en-US").unwrap();
    let snap = en.snapshot_json();
    let st = &snap["strings"];
    assert_eq!(st["blank"], "(Blank)");
    assert_eq!(st["total"], "Total");
    assert_eq!(st["all"], "All");
    assert_eq!(st["select_all"], "Select all");
    assert_eq!(st["multiple_selections"], "Multiple selections");
    assert_eq!(st["increase"], "Increase");
    assert_eq!(st["decrease"], "Decrease");
    assert_eq!(st["goal"], "Goal");
    for (f, want) in [
        ("Sum", "Sum of Amount"),
        ("Avg", "Average of Amount"),
        ("Min", "Min of Amount"),
        ("Max", "Max of Amount"),
        ("CountNonNull", "Count of Amount"),
        ("Count", "Count of Amount"),
        ("Median", "Median of Amount"),
        ("StandardDeviation", "Standard deviation of Amount"),
        ("Variance", "Variance of Amount"),
    ] {
        assert_eq!(en.aggregation(f, "Amount").as_deref(), Some(want));
    }
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    assert_eq!(
        en.auto_title(
            &s(&["Average of Amount", "Min of Amount", "Max of Amount"]),
            &s(&["Month"])
        ),
        "Average of Amount, Min of Amount and Max of Amount by Month"
    );
    assert_eq!(
        en.auto_title(&s(&["Total"]), &s(&["Region", "Category"])),
        "Total by Region and Category"
    );
    let fr = UiLanguage::parse("fr-FR").unwrap().snapshot_json();
    for key in ["empty_visual", "no_filters_applied", "open_in_desktop"] {
        assert_ne!(st[key], fr["strings"][key], "{key}");
    }
    // every language has the two new phrases
    for tag in SUPPORTED_LANGUAGES {
        let j = UiLanguage::parse(tag).unwrap().snapshot_json();
        assert!(
            j["strings"]["empty_visual"]
                .as_str()
                .is_some_and(|x| !x.is_empty()),
            "{tag}"
        );
        assert!(
            j["strings"]["no_filters_applied"]
                .as_str()
                .is_some_and(|x| !x.is_empty()),
            "{tag}"
        );
        // the live viewer's "open in Desktop" button label
        assert!(
            j["strings"]["open_in_desktop"]
                .as_str()
                .is_some_and(|x| x.contains("Power BI Desktop")),
            "{tag}"
        );
    }
}
