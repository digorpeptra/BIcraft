//! Read and write tests on generated legacy layouts (our own content, shaped after the black-box
//! observations listed in docs/formats/legacy-layout-bb.sources.md).

use bicraft_layout::{
    BOOKMARKS_METADATA_KEY, MOBILE_STATE_KEY, UNMAPPED_KEY, read_layout, read_layout_with_summary,
    report_extensions, strip_legacy, write_layout,
};
use bicraft_report::{FieldBinding, FieldRef, Page, Report, Resource, Visual};
use serde_json::{Value, json};

/// Turns a decoded legacy document into its text: the JSON-in-a-string slots become strings.
fn encode(mut doc: Value) -> String {
    fn enc(v: &mut Value, k: &str) {
        if let Some(x) = v.get_mut(k)
            && !x.is_string()
        {
            *x = Value::String(x.to_string());
        }
    }
    enc(&mut doc, "config");
    enc(&mut doc, "filters");
    if let Some(Value::Array(pods)) = doc.get_mut("pods") {
        for p in pods {
            enc(p, "config");
            enc(p, "parameters");
        }
    }
    if let Some(Value::Array(sections)) = doc.get_mut("sections") {
        for s in sections {
            enc(s, "config");
            enc(s, "filters");
            if let Some(Value::Array(cs)) = s.get_mut("visualContainers") {
                for c in cs {
                    for k in ["config", "filters", "query", "dataTransforms"] {
                        enc(c, k);
                    }
                }
            }
        }
    }
    doc.to_string()
}

fn col(alias: &str, p: &str) -> Value {
    json!({"Column": {"Expression": {"SourceRef": {"Source": alias}}, "Property": p}})
}

fn container(name: &str, x: f64, z: f64, single: Value, extra: Value) -> Value {
    let mut c = json!({"x": x, "y": 10.0, "z": z, "width": 200.0, "height": 100.0,
        "config": {"name": name, "layouts": [{"id": 0, "position":
            {"x": x, "y": 10.0, "z": z, "width": 200.0, "height": 100.0, "tabOrder": z}}],
            "singleVisual": single},
        "filters": [], "tabOrder": z});
    if let (Value::Object(m), Value::Object(e)) = (&mut c, extra) {
        for (k, v) in e {
            if k == "config" {
                if let (Some(Value::Object(cm)), Value::Object(ce)) = (m.get_mut("config"), v) {
                    cm.extend(ce);
                }
            } else {
                m.insert(k, v);
            }
        }
    }
    c
}

fn chart() -> Value {
    json!({"visualType": "clusteredColumnChart",
        "projections": {"Category": [{"queryRef": "Shop.Region", "active": true}],
                        "Y": [{"queryRef": "Sum(Shop.Amount)"}, {"queryRef": "Shop.Local"}]},
        "prototypeQuery": {"Version": 2,
            "From": [{"Name": "s", "Entity": "Shop", "Type": 0},
                     {"Name": "s1", "Entity": "Shop", "Schema": "extension", "Type": 0}],
            "Select": [
                {"Column": {"Expression": {"SourceRef": {"Source": "s"}}, "Property": "Region"},
                 "Name": "Shop.Region", "NativeReferenceName": "Region"},
                {"Aggregation": {"Expression": col("s", "Amount"), "Function": 0},
                 "Name": "Sum(Shop.Amount)", "NativeReferenceName": "Sum of Amount"},
                {"Measure": {"Expression": {"SourceRef": {"Source": "s1"}}, "Property": "Local"},
                 "Name": "Shop.Local"}],
            "OrderBy": [{"Direction": 2, "Expression": {"Aggregation": {"Expression": col("s", "Amount"), "Function": 0}}}]},
        "columnProperties": {"Sum(Shop.Amount)": {"displayName": "Sold", "formatString": "0.0"}},
        "showAllRoles": ["Category"],
        "objects": {"legend": [{"properties": {"show": {"expr": {"Literal": {"Value": "true"}}}}}]},
        "vcObjects": {"title": [{"properties": {"text": {"expr": {"Literal": {"Value": "'Chart'"}}}}}]},
        "cachedFilterDisplayItems": [{"displayName": "x"}],
        "mobileObjects": {"legend": []}})
}

fn region_filter() -> Value {
    json!({"name": "F1", "expression": {"Column": {"Expression": {"SourceRef": {"Entity": "Shop"}}, "Property": "Region"}},
        "filter": {"Version": 2, "From": [{"Name": "s", "Entity": "Shop", "Type": 0}],
            "Where": [{"Condition": {"In": {"Expressions": [col("s", "Region")],
                "Values": [[{"Literal": {"Value": "'A'"}}], [{"Literal": {"Value": "'B'"}}]]}}}]},
        "type": "Categorical", "howCreated": 1,
        "cachedDisplayNames": [
            {"id": {"scopeId": {"Comparison": {"ComparisonKind": 0,
                "Left": {"Column": {"Expression": {"SourceRef": {"Entity": "Shop"}}, "Property": "Region"}},
                "Right": {"Literal": {"Value": "'A'"}}}}}, "displayName": "A"},
            {"id": {"scopeId": {"Comparison": {"ComparisonKind": 0,
                "Left": {"Column": {"Expression": {"SourceRef": {"Entity": "Shop"}}, "Property": "Region"}},
                "Right": {"Literal": {"Value": "'B'"}}}}}, "displayName": "B"}],
        "objects": {"general": [{"properties": {"requireSingleSelect": {"expr": {"Literal": {"Value": "true"}}},
                                                "oddProp": {"expr": {"Literal": {"Value": "1D"}}}}}]}})
}

/// A legacy layout touching most of what the reader maps.
fn sample() -> String {
    let card = json!({"visualType": "card",
        "projections": {"Values": [{"queryRef": "Shop.Total"}]},
        "prototypeQuery": {"Version": 2, "From": [{"Name": "s", "Entity": "Shop", "Type": 0}],
            "Select": [{"Measure": {"Expression": {"SourceRef": {"Source": "s"}}, "Property": "Total"},
                        "Name": "Shop.Total", "NativeReferenceName": "Total"}]},
        "display": {"mode": "hidden"}});
    let text = json!({"visualType": "textbox", "projections": {},
        "objects": {"general": [{"properties": {"paragraphs": [{"textRuns": [{"value": "Hi"}]}]}}]}});
    let mut c1 = container(
        "chart1",
        10.0,
        0.0,
        chart(),
        json!({"id": 3, "query": {"Commands": []}}),
    );
    if let Some(Value::Array(l)) = c1.pointer_mut("/config/layouts") {
        l.push(json!({"id": 1, "position": {"x": 0.0, "y": 5.0, "z": 0.0, "width": 300.0, "height": 120.0, "tabOrder": 0.0}}));
    }
    let doc = json!({
        "id": 0,
        "resourcePackages": [{"resourcePackage": {"name": "SharedResources", "type": 2,
            "items": [{"type": 202, "path": "BaseThemes/Base.json", "name": "Base"},
                      {"type": 999, "path": "odd", "name": "odd"}], "disabled": false}}],
        "sections": [
            {"name": "ReportSection2", "displayName": "Second", "filters": [], "ordinal": 1,
             "visualContainers": [container("tb1", 10.0, 0.0, text, json!({}))],
             "config": {"visibility": 1, "type": 1}, "displayOption": 3, "width": 320.0, "height": 240.0},
            {"name": "ReportSection1", "displayName": "First", "filters": [region_filter()], "ordinal": 0,
             "visualContainers": [
                c1,
                container("card1", 220.0, 1000.0, card, json!({"filters": [
                    {"name": "F2", "expression": col("s", "Region"), "type": "Advanced", "howCreated": 0}]})),
                {"x": 10.0, "y": 300.0, "z": 2000.0, "width": 400.0, "height": 300.0,
                 "config": {"name": "grp1", "layouts": [{"id": 0, "position": {"x": 10.0, "y": 300.0, "z": 2000.0, "width": 400.0, "height": 300.0}}],
                            "singleVisualGroup": {"displayName": "Group", "groupMode": 1}},
                 "filters": []}],
             "config": {"objects": {"background": [{"properties": {"transparency": {"expr": {"Literal": {"Value": "10D"}}}}}]},
                        "relationships": [{"source": "chart1", "target": "card1", "type": 3}],
                        "filterSortOrder": 3, "futureKey": 1},
             "displayOption": 1, "width": 1280.0, "height": 720.0}],
        "config": {"version": "5.61",
            "themeCollection": {"baseTheme": {"name": "Base", "version": "5.61", "type": 2}},
            "activeSectionIndex": 1, "linguisticSchemaSyncVersion": 0,
            "objects": {"outspacePane": [{"properties": {"expanded": {"expr": {"Literal": {"Value": "false"}}},
                                                         "width": {"expr": {"Literal": {"Value": "200L"}}}}}]},
            "settings": {"useStylableVisualContainerHeader": true, "exportDataMode": 1, "queryLimitOption": 6,
                         "hideVisualContainerHeader": false, "useNewFilterPaneExperience": true},
            "defaultDrillFilterOtherVisuals": true,
            "bookmarks": [{"name": "B1", "displayName": "One", "explorationState": {"version": "1.3",
                              "activeSection": "ReportSection1", "sections": {"ReportSection1": {}}}},
                          {"name": "G", "displayName": "Group", "children": [
                              {"name": "B2", "displayName": "Two", "explorationState": {"version": "1.3",
                                  "activeSection": "ReportSection2", "sections": {}}, "options": {"suppressData": true}}]}],
            "modelExtensions": [{"name": "extension", "entities": [{"name": "Shop", "extends": "Shop",
                "measures": [{"name": "Local", "dataType": 3, "expression": "1", "hidden": false,
                              "formatInformation": {"formatString": "0", "format": "General"}}]}]}]},
        "layoutOptimization": 0,
        "filters": [{"name": "FT", "expression": col("s", "Region"), "type": "Categorical", "isTransient": true}],
        "pods": [{"name": "Pod1", "boundSection": "ReportSection2", "config": {}, "parameters": [], "type": 2}],
        "publicCustomVisuals": []
    });
    encode(doc)
}

fn unmapped(extra: &serde_json::Map<String, Value>) -> Vec<String> {
    extra
        .get(UNMAPPED_KEY)
        .and_then(Value::as_object)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

#[test]
fn reads_the_report_level() {
    let (r, summary) = read_layout_with_summary(&sample()).unwrap();
    assert_eq!(
        r.pages.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
        ["ReportSection1", "ReportSection2"],
        "pages follow their ordinal"
    );
    assert_eq!(r.active_page.as_deref(), Some("ReportSection2"));
    assert_eq!(
        r.theme_collection,
        Some(json!({"baseTheme": {"name": "Base", "reportVersionAtImport":
            {"visual": "1.8.97", "report": "2.0.97", "page": "1.3.97"}, "type": "SharedResources"}}))
    );
    assert_eq!(
        r.extra["settings"],
        json!({"useStylableVisualContainerHeader": true, "exportDataMode": "AllowSummarized",
               "defaultDrillFilterOtherVisuals": true})
    );
    assert_eq!(
        r.extra["objects"],
        json!({"outspacePane": [{"properties": {"expanded": {"expr": {"Literal": {"Value": "false"}}}}}]})
    );
    assert_eq!(
        r.extra["resourcePackages"],
        json!([{"name": "SharedResources", "type": "SharedResources",
                "items": [{"name": "Base", "path": "BaseThemes/Base.json", "type": "BaseTheme"}]}])
    );
    let un = unmapped(&r.extra);
    for k in [
        "id",
        "layoutOptimization",
        "publicCustomVisuals",
        "filters[0]",
        "config.version",
        "config.linguisticSchemaSyncVersion",
        "config.themeCollection.baseTheme.version",
        "config.objects.outspacePane[0].properties.width",
        "config.settings.queryLimitOption",
        "config.settings.hideVisualContainerHeader",
        "config.settings.useNewFilterPaneExperience",
        "config.modelExtensions",
        "resourcePackages[0].resourcePackage.items[1]",
        "resourcePackages[0].resourcePackage.disabled",
    ] {
        assert!(un.contains(&k.to_string()), "{k} in {un:?}");
    }
    assert!(
        r.filter_config.is_none(),
        "the transient filter is not a PBIR filter"
    );
    // Bookmarks.
    assert_eq!(
        r.extra[BOOKMARKS_METADATA_KEY],
        json!({"items": [{"name": "B1"}, {"name": "G", "displayName": "Group", "children": ["B2"]}]})
    );
    assert_eq!(r.bookmarks.len(), 2);
    assert_eq!(
        r.bookmarks[0].document["explorationState"]["sections"]["ReportSection1"],
        json!({}),
        "kept as read; strip_legacy adds visualContainers"
    );
    assert_eq!(summary.pages, 2);
    assert_eq!(summary.visuals, 4);
    assert!(summary.unmapped.contains(&"report: id".to_string()));
    assert!(
        summary
            .unmapped
            .contains(&"visual ReportSection1/chart1: id".to_string())
    );
    // Report measures.
    let ext = report_extensions(&r.extra[UNMAPPED_KEY]["config.modelExtensions"]).unwrap();
    assert_eq!(ext["entities"][0]["measures"][0]["dataType"], "Double");
}

#[test]
fn reads_pages_and_visuals() {
    let r = read_layout(&sample()).unwrap();
    let first = &r.pages[0];
    assert_eq!(first.display_option.as_deref(), Some("FitToPage"));
    assert_eq!(
        first.extra["visualInteractions"],
        json!([{"source": "chart1", "target": "card1", "type": "NoFilter"}])
    );
    assert_eq!(
        unmapped(&first.extra),
        [
            "filters[0].objects.general[0].properties.oddProp",
            "config.futureKey"
        ]
    );
    let fc = first.filter_config.as_ref().unwrap();
    assert_eq!(fc["filterSortOrder"], "Custom");
    let f = &fc["filters"][0];
    assert_eq!(f["field"]["Column"]["Property"], "Region");
    assert_eq!(f["howCreated"], "User");
    assert_eq!(
        f["filter"]["Where"][0]["Annotations"]["filterExpressionMetadata"]["valueMap"],
        json!([{"0": "A"}, {"0": "B"}])
    );
    assert_eq!(
        f["objects"],
        json!({"general": [{"properties": {"requireSingleSelect": {"expr": {"Literal": {"Value": "true"}}}}}]})
    );
    let second = &r.pages[1];
    assert_eq!(second.visibility.as_deref(), Some("HiddenInViewMode"));
    assert_eq!(second.extra["type"], "Tooltip");
    assert_eq!(
        second.extra["pageBinding"],
        json!({"name": "Pod1", "type": "Tooltip", "parameters": []})
    );

    let chart = first.visual("chart1").unwrap();
    assert_eq!(chart.position.tab_order, Some(0));
    let q = &chart.visual_extra["query"];
    assert_eq!(
        q["queryState"]["Y"]["projections"][0],
        json!({"field": {"Aggregation": {"Expression": {"Column": {"Expression": {"SourceRef": {"Entity": "Shop"}},
               "Property": "Amount"}}, "Function": 0}}, "queryRef": "Sum(Shop.Amount)",
               "nativeQueryRef": "Sum of Amount", "displayName": "Sold", "format": "0.0"})
    );
    assert_eq!(
        q["queryState"]["Y"]["projections"][1]["field"]["Measure"]["Expression"]["SourceRef"],
        json!({"Schema": "extension", "Entity": "Shop"})
    );
    assert_eq!(q["queryState"]["Category"]["showAll"], true);
    assert_eq!(q["sortDefinition"]["sort"][0]["direction"], "Descending");
    assert_eq!(chart.fields.len(), 3);
    assert!(chart.visual_extra.contains_key("visualContainerObjects"));
    assert_eq!(
        chart.extra[MOBILE_STATE_KEY],
        json!({"position": {"x": 0, "y": 5, "z": 0, "height": 120, "width": 300, "tabOrder": 0},
               "objects": {"legend": []}})
    );
    let un = unmapped(&chart.extra);
    for k in ["id", "query", "config.singleVisual.cachedFilterDisplayItems"] {
        assert!(un.contains(&k.to_string()), "{k} in {un:?}");
    }
    let card = first.visual("card1").unwrap();
    assert!(card.is_hidden);
    assert!(
        card.filter_config.as_ref().unwrap()["filters"][0]
            .get("howCreated")
            .is_none()
    );
    assert_eq!(unmapped(&card.extra), ["filters[0].howCreated"]);
    let group = first.visual("grp1").unwrap();
    assert_eq!(
        group.extra["visualGroup"],
        json!({"displayName": "Group", "groupMode": "ScrollMode"})
    );
    assert_eq!(group.visual_type, None);
    assert_eq!(group.position.tab_order, None);
}

#[test]
fn an_unchanged_report_writes_the_original_text() {
    let text = sample();
    let r = read_layout(&text).unwrap();
    assert_eq!(write_layout(&text, &r).unwrap(), text);
    let bom = format!("\u{feff}{text}");
    assert_eq!(write_layout(&bom, &r).unwrap(), bom);
}

/// Every edit below is written, read back unchanged, and leaves what it did not touch as it was.
#[test]
fn edits_round_trip() {
    let text = sample();
    let base = read_layout(&text).unwrap();
    type Edit = fn(&mut Report);
    let edits: Vec<(&str, Edit)> = vec![
        ("rename page", |r| r.pages[0].display_name = "Renamed".into()),
        ("move visual", |r| r.pages[0].visuals[1].position.x = 33.5),
        ("unhide", |r| r.pages[0].visuals[1].is_hidden = false),
        ("active page", |r| r.active_page = Some("ReportSection1".into())),
        ("remove visual", |r| {
            r.pages[0].visuals.remove(0);
        }),
        ("reorder pages", |r| r.pages.swap(0, 1)),
        ("new page", |r| {
            r.pages.push(Page {
                name: "ReportSectionNew".into(),
                display_name: "New".into(),
                display_option: Some("FitToWidth".into()),
                width: 1280.0,
                height: 720.0,
                ..Default::default()
            })
        }),
        ("new visual", |r| {
            let mut v = Visual {
                name: "new1".into(),
                visual_type: Some("tableEx".into()),
                fields: vec![
                    FieldBinding {
                        display_name: None,
                        show_as: None,
                        role: "Values".into(),
                        field: FieldRef::Column {
                            table: "Other".into(),
                            column: "Name".into(),
                        },
                    },
                    FieldBinding {
                        display_name: None,
                        show_as: None,
                        role: "Values".into(),
                        field: FieldRef::Measure {
                            table: "Shop".into(),
                            measure: "Total".into(),
                        },
                    },
                ],
                ..Default::default()
            };
            v.position.width = 300.0;
            v.position.height = 200.0;
            v.position.tab_order = Some(5000);
            r.pages[1].visuals.push(v);
        }),
        ("rebind chart", |r| {
            r.pages[0].visuals[0].fields.push(FieldBinding {
                display_name: None,
                show_as: None,
                role: "Series".into(),
                field: FieldRef::Column {
                    table: "Other".into(),
                    column: "Kind".into(),
                },
            })
        }),
        ("interactions", |r| {
            r.pages[0].extra.insert(
                "visualInteractions".into(),
                json!([{"source": "card1", "target": "chart1", "type": "DataFilter"}]),
            );
        }),
        ("page filter", |r| {
            if let Some(Value::Object(fc)) = &mut r.pages[0].filter_config {
                fc.insert("filterSortOrder".into(), json!("Ascending"));
                if let Some(Value::Array(fs)) = fc.get_mut("filters") {
                    fs[0]["isHiddenInViewMode"] = json!(true);
                }
            }
        }),
        ("report filter", |r| {
            r.filter_config = Some(json!({"filters": [{"name": "R1",
                "field": {"Column": {"Expression": {"SourceRef": {"Entity": "Shop"}}, "Property": "Region"}},
                "type": "Categorical", "howCreated": "User"}]}));
        }),
        ("settings", |r| {
            r.extra["settings"]["exportDataMode"] = json!("AllowSummarized");
            r.extra["settings"]["useEnhancedTooltips"] = json!(true);
        }),
        ("phone layout moved", |r| {
            r.pages[0].visuals[0].extra[MOBILE_STATE_KEY]["position"]["x"] = json!(40);
        }),
        ("phone layout removed", |r| {
            r.pages[0].visuals[0].extra.shift_remove(MOBILE_STATE_KEY);
        }),
        ("bookmark edited", |r| {
            r.bookmarks[1].document["displayName"] = json!("Deux");
        }),
        ("report measures", |r| {
            r.extra[UNMAPPED_KEY]["config.modelExtensions"][0]["entities"][0]["measures"][0]["expression"] =
                json!("2");
        }),
        ("group renamed", |r| {
            r.pages[0].visuals[2].extra["visualGroup"]["displayName"] = json!("Renamed group");
        }),
    ];
    for (what, edit) in edits {
        let mut r = base.clone();
        edit(&mut r);
        // As the engine does: the raw query follows edited bindings.
        for p in &mut r.pages {
            for v in &mut p.visuals {
                if let Some(q) = bicraft_report::query::effective(v) {
                    v.visual_extra.insert("query".into(), q);
                }
            }
        }
        let written = write_layout(&text, &r).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert_ne!(written, text, "{what}");
        let back = read_layout(&written).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert!(back == r, "{what}: {}", first_diff(&back, &r));
        // Written again: stable.
        assert_eq!(write_layout(&written, &back).unwrap(), written, "{what}");
    }
}

#[test]
fn untouched_sections_and_containers_keep_their_text() {
    let text = sample();
    let mut r = read_layout(&text).unwrap();
    r.pages[0].visuals[1].position.y = 77.0;
    let written = write_layout(&text, &r).unwrap();
    let orig: Value = serde_json::from_str(&text).unwrap();
    // The other page, the other containers and the report config keep their exact text.
    let raw = |v: &Value| v.to_string();
    for s in orig["sections"].as_array().unwrap() {
        if s["name"] == "ReportSection2" {
            assert!(written.contains(&raw(s)));
        } else {
            for (i, c) in s["visualContainers"].as_array().unwrap().iter().enumerate() {
                if written.contains(&raw(c)) != (i != 1) {
                    let w: Value = serde_json::from_str(&written).unwrap();
                    let wc = w["sections"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|x| x["name"] == s["name"])
                        .unwrap()["visualContainers"][i]
                        .clone();
                    let dec = |v: &Value| {
                        let mut v = v.clone();
                        for k in ["config", "filters", "query"] {
                            if let Some(t) = v[k].as_str() {
                                v[k] = serde_json::from_str(t).unwrap();
                            }
                        }
                        v
                    };
                    panic!(
                        "container {i}:
{}
{}",
                        dec(c),
                        dec(&wc)
                    );
                }
            }
        }
    }
    assert!(written.contains(&raw(&orig["config"])));
}

#[test]
fn phone_layouts_come_from_mobile_documents_and_other_pbir_documents_are_refused() {
    let text = sample();
    let mut r = read_layout(&text).unwrap();
    r.resources.push(Resource {
        path: "definition/pages/ReportSection1/visuals/card1/mobile.json".into(),
        bytes: br#"{"$schema":"x","position":{"x":1,"y":2,"z":0,"height":50,"width":100}}"#.to_vec(),
    });
    let written = write_layout(&text, &r).unwrap();
    let back = read_layout(&written).unwrap();
    assert_eq!(
        back.pages[0].visuals[1].extra[MOBILE_STATE_KEY],
        json!({"position": {"x": 1, "y": 2, "z": 0, "height": 50, "width": 100}})
    );
    r.resources.push(Resource {
        path: "definition/reportExtensions.json".into(),
        bytes: b"{}".to_vec(),
    });
    let e = write_layout(&text, &r).unwrap_err().to_string();
    assert!(e.contains("definition/reportExtensions.json"), "{e}");
}

#[test]
fn edits_without_a_legacy_form_are_refused_and_named() {
    let text = sample();
    let base = read_layout(&text).unwrap();
    let mut r = base.clone();
    r.theme_collection = Some(json!({"baseTheme": {"name": "Other"}}));
    r.extra["resourcePackages"] = json!([]);
    r.pages[1].extra["pageBinding"]["name"] = json!("Other");
    r.pages[0].extra.insert("annotations".into(), json!([]));
    r.pages[0].visuals[0]
        .extra
        .insert("annotations".into(), json!([]));
    r.pages[0].visuals[0]
        .visual_extra
        .insert("visualContainerObjectsX".into(), json!({}));
    let e = write_layout(&text, &r).unwrap_err().to_string();
    for what in [
        "theme",
        "resourcePackages",
        "page binding",
        "page ReportSection1: annotations",
        "visual ReportSection1/chart1: annotations",
        "visual.visualContainerObjectsX",
    ] {
        assert!(e.contains(what), "{what}: {e}");
    }
}

#[test]
fn strip_legacy_gives_the_pbir_form() {
    let mut r = read_layout(&sample()).unwrap();
    strip_legacy(&mut r);
    let text = serde_json::to_string(&r).unwrap();
    assert!(!text.contains("legacy:"), "{text}");
    assert_eq!(r.extra["settings"]["useEnhancedTooltips"], false);
    assert_eq!(
        r.bookmarks[0].document["explorationState"]["sections"]["ReportSection1"],
        json!({"visualContainers": {}})
    );
    // The chart's category is a drill role: already active; Y is not.
    let q = &r.pages[0].visuals[0].visual_extra["query"]["queryState"];
    assert_eq!(q["Category"]["projections"][0]["active"], true);
    assert!(q["Y"]["projections"][0].get("active").is_none());
}

#[test]
fn hostile_layouts_never_panic() {
    let text = sample();
    let inputs = [
        "",
        "[]",
        "null",
        "{}",
        r#"{"sections": 3}"#,
        r#"{"sections": [3, {"visualContainers": [1, {"config": "{"}]}]}"#,
        r#"{"config": "not json", "sections": [{"config": "[]", "filters": "{}"}]}"#,
        r#"{"config": {"activeSectionIndex": 99, "bookmarks": [{"children": 1}, {"name": "a"}, {"name": "a"}]}}"#,
        r#"{"sections": [{"visualContainers": [{"config": {"singleVisual": {"projections": {"Y": [{"queryRef": 1}]},
            "prototypeQuery": {"From": 1, "Select": [{"Name": "q"}], "OrderBy": [{"Direction": 9}]}}}}]}]}"#,
        r#"{"pods": [{"boundSection": "x", "config": "]", "parameters": 5}], "resourcePackages": [{"a": 1}]}"#,
    ];
    for input in inputs {
        if let Ok(r) = read_layout(input) {
            let _ = write_layout(input, &r);
            let mut edited = r.clone();
            edited.pages.push(Page {
                name: "p".into(),
                ..Default::default()
            });
            let _ = write_layout(input, &edited);
        }
    }
    // Deep nesting and every prefix of a real layout.
    let deep = format!("{}{}", "[".repeat(5000), "]".repeat(5000));
    assert!(read_layout(&deep).is_err());
    for cut in (0..text.len()).step_by(97) {
        if let Some(prefix) = text.get(..cut) {
            let _ = read_layout(prefix);
        }
    }
}

/// The first JSON path where two reports differ (for assertion messages).
fn first_diff(a: &Report, b: &Report) -> String {
    fn walk(p: &str, a: &Value, b: &Value) -> Option<String> {
        match (a, b) {
            (Value::Object(x), Value::Object(y)) => {
                for k in x.keys().chain(y.keys()) {
                    match (x.get(k), y.get(k)) {
                        (Some(v), Some(w)) => {
                            if let Some(d) = walk(&format!("{p}.{k}"), v, w) {
                                return Some(d);
                            }
                        }
                        (v, w) => return Some(format!("{p}.{k}: {v:?} / {w:?}")),
                    }
                }
                None
            }
            (Value::Array(x), Value::Array(y)) if x.len() == y.len() => x
                .iter()
                .zip(y)
                .enumerate()
                .find_map(|(i, (v, w))| walk(&format!("{p}[{i}]"), v, w)),
            _ if a == b => None,
            _ => Some(format!("{p}: {a} / {b}")),
        }
    }
    let a = serde_json::to_value(a).unwrap();
    let b = serde_json::to_value(b).unwrap();
    walk("", &a, &b).unwrap_or_default()
}

/// The committed fixture (also a fuzz seed for `bicraft-pbix`) reads, writes back unchanged and
/// converts.
#[test]
fn the_fixture_reads() {
    let text = include_str!("fixtures/shop_layout.json");
    let r = read_layout(text).unwrap();
    assert_eq!(r.pages[0].visuals.len(), 10);
    assert_eq!(r.pages[0].visuals[0].fields.len(), 2);
    assert_eq!(write_layout(text, &r).unwrap(), text);
}
