//! Tests of the catalog. The data-role expectations are our own black-box observations of Power
//! BI Desktop 2.159 (`docs/formats/visual-roles-bb.md`; the test ids are in the comments).

use super::*;

fn role(t: &str, r: &str) -> &'static DataRole {
    data_role(t, r).unwrap_or_else(|| panic!("{t}.{r}"))
}

fn names(t: &str) -> Vec<&'static str> {
    visual(t)
        .unwrap_or_else(|| panic!("{t}"))
        .data_roles
        .iter()
        .map(|r| r.name.as_str())
        .collect()
}

#[test]
fn catalog_is_sorted_and_found() {
    let names = visual_types();
    assert!(!names.is_empty());
    assert!(names.windows(2).all(|w| w[0] < w[1]));
    for n in &names {
        assert_eq!(visual(n).map(|v| v.name.as_str()), Some(*n));
    }
    assert!(visual("noSuchVisual").is_none());
}

/// Types whose field wells were not established (not checked): types without a field well, and
/// types not yet opened in Desktop.
const UNCHECKED: &[&str] = &[
    "actionButton",
    "aiNarratives",
    "bookmarkNavigator",
    "image",
    "pageNavigator",
    "qnaVisual",
    "shape",
    "textbox",
];

#[test]
fn every_theme_type_with_wells_is_checked() {
    // wells-A: every type of the theme schema was opened in Desktop.
    for t in visual_types() {
        let v = visual(t).unwrap_or_else(|| panic!("{t}"));
        assert_eq!(v.roles_known, !UNCHECKED.contains(&t), "{t}");
        assert_eq!(
            v.roles,
            v.data_roles.iter().map(|r| r.name.clone()).collect::<Vec<_>>(),
            "{t}"
        );
    }
}
#[test]
fn role_names_in_well_order() {
    // wells-A / wells-R: the role names Desktop saved for each well, in the well order.
    for t in [
        "barChart",
        "clusteredBarChart",
        "clusteredColumnChart",
        "columnChart",
        "hundredPercentStackedAreaChart",
        "hundredPercentStackedBarChart",
        "hundredPercentStackedColumnChart",
        "ribbonChart",
        "stackedAreaChart",
    ] {
        assert_eq!(names(t), ["Category", "Y", "Series", "Rows", "Tooltips"], "{t}");
    }
    for t in [
        "lineChart",
        "areaChart",
        "lineClusteredColumnComboChart",
        "lineStackedColumnComboChart",
    ] {
        assert_eq!(
            names(t),
            ["Category", "Y", "Y2", "Series", "Rows", "Tooltips"],
            "{t}"
        );
    }
    for t in ["pieChart", "donutChart"] {
        assert_eq!(names(t), ["Category", "Y", "Series", "Tooltips"], "{t}");
    }
    assert_eq!(
        names("scatterChart"),
        ["Category", "X", "Y", "Series", "Size", "Play", "Tooltips"]
    );
    assert_eq!(
        names("azureMap"),
        [
            "Category",
            "Y",
            "X",
            "Series",
            "Size",
            "Tooltips",
            "PathID",
            "PointOrder"
        ]
    );
    assert_eq!(names("map"), ["Category", "Series", "Y", "X", "Size", "Tooltips"]);
    assert_eq!(names("filledMap"), ["Category", "Series", "Y", "X", "Tooltips"]);
    assert_eq!(names("shapeMap"), ["Category", "Series", "Value", "Tooltips"]);
    assert_eq!(
        names("gauge"),
        ["Y", "MinValue", "MaxValue", "TargetValue", "Tooltips"]
    );
    assert_eq!(names("kpi"), ["Indicator", "TrendLine", "Goal"]);
    assert_eq!(names("card"), ["Values"]);
    assert_eq!(names("cardVisual"), ["Data", "Rows", "Tooltips"]);
    assert_eq!(names("multiRowCard"), ["Values"]);
    assert_eq!(names("tableEx"), ["Values"]);
    assert_eq!(names("pivotTable"), ["Rows", "Columns", "Values"]);
    assert_eq!(names("treemap"), ["Group", "Details", "Values", "Tooltips"]);
    assert_eq!(
        names("waterfallChart"),
        ["Category", "Breakdown", "Y", "Tooltips"]
    );
    assert_eq!(names("funnel"), ["Category", "Y", "Tooltips"]);
    assert_eq!(
        names("decompositionTreeVisual"),
        ["Analyze", "ExplainBy", "Tooltips"]
    );
    assert_eq!(names("keyDriversVisual"), ["Target", "ExplainBy", "Details"]);
    assert_eq!(names("slicer"), ["Values"]);
    assert_eq!(names("textSlicer"), ["Values"]);
    assert_eq!(names("listSlicer"), ["Values", "Tooltips"]);
    assert_eq!(names("advancedSlicerVisual"), ["Values", "Label", "Tooltips"]);
    assert_eq!(names("rdlVisual"), ["parameterValues"]);
    for t in ["pythonVisual", "scriptVisual"] {
        assert_eq!(names(t), ["Values"], "{t}");
    }
}

#[test]
fn types_without_wells() {
    // wells-A: no field well in Desktop's Build pane. A textbox with a field draws nothing and
    // shows no error (kinds-t04): those types are left unchecked. A scorecard with a field shows
    // an error (kinds-t03): it takes no fields.
    for t in UNCHECKED {
        let v = visual(t).unwrap_or_else(|| panic!("{t}"));
        assert!(
            !v.roles_known && v.data_roles.is_empty() && v.roles.is_empty(),
            "{t}"
        );
    }
    let s = visual("scorecard").unwrap_or_else(|| panic!());
    assert!(s.roles_known && s.data_roles.is_empty());
}
#[test]
fn kinds_follow_what_the_wells_keep() {
    // wells-B refused a measure: columns only.
    for (t, r) in [
        ("clusteredColumnChart", "Category"),
        ("clusteredColumnChart", "Series"),
        ("clusteredColumnChart", "Rows"),
        ("slicer", "Values"),
        ("kpi", "TrendLine"),
        ("keyDriversVisual", "Details"),
        ("cardVisual", "Rows"),
        ("pivotTable", "Columns"),
    ] {
        assert_eq!(role(t, r).kind, RoleKind::Grouping, "{t}.{r}");
    }
    // wells-A summarized a text column, wells-B kept a measure: measures.
    for (t, r) in [
        ("clusteredColumnChart", "Y"),
        ("clusteredColumnChart", "Tooltips"),
        ("card", "Values"),
        ("pivotTable", "Values"),
        ("gauge", "TargetValue"),
        ("map", "Y"),
        ("scatterChart", "Y"),
    ] {
        assert_eq!(role(t, r).kind, RoleKind::Measure, "{t}.{r}");
    }
    // Both kept as they are: either.
    for (t, r) in [
        ("tableEx", "Values"),
        ("multiRowCard", "Values"),
        ("scatterChart", "X"),
        ("keyDriversVisual", "Target"),
    ] {
        assert_eq!(role(t, r).kind, RoleKind::GroupingOrMeasure, "{t}.{r}");
    }
}

#[test]
fn maxima_and_drill_roles() {
    // wells-C: the well keeps one field (it replaces it), or two for the KPI target.
    for (t, r) in [
        ("clusteredColumnChart", "Series"),
        ("card", "Values"),
        ("gauge", "Y"),
        ("textSlicer", "Values"),
        ("shapeMap", "Category"),
        ("waterfallChart", "Y"),
        ("scatterChart", "Size"),
        ("decompositionTreeVisual", "Analyze"),
    ] {
        assert_eq!(role(t, r).max, Some(1), "{t}.{r}");
    }
    assert_eq!(role("kpi", "Goal").max, Some(2));
    // wells-C: five fields kept; with several, the header offers drill-down buttons.
    for (t, r) in [
        ("clusteredColumnChart", "Category"),
        ("lineChart", "Category"),
        ("pieChart", "Category"),
        ("pivotTable", "Rows"),
        ("pivotTable", "Columns"),
        ("treemap", "Group"),
        ("funnel", "Category"),
    ] {
        let d = role(t, r);
        assert!(d.drill && d.max.is_none(), "{t}.{r}");
    }
    // Five fields kept, no drill buttons.
    // Saved with `active` flags (no header buttons): drill roles too.
    for (t, r) in [
        ("slicer", "Values"),
        ("listSlicer", "Values"),
        ("map", "Category"),
    ] {
        assert!(role(t, r).drill, "{t}.{r}");
    }
    for (t, r) in [
        ("tableEx", "Values"),
        ("cardVisual", "Data"),
        ("multiRowCard", "Values"),
    ] {
        let d = role(t, r);
        assert!(!d.drill && d.max.is_none(), "{t}.{r}");
    }
}

#[test]
fn legend_makes_values_single() {
    // wells-A / wells-C: with a legend field, the values well stops taking several fields.
    let v = visual("clusteredColumnChart").unwrap_or_else(|| panic!());
    let holds = |series: u32, y: u32| {
        v.role_conditions.iter().any(|c| {
            c.bounds.iter().all(|b| {
                let n = match b.role.as_str() {
                    "Series" => series,
                    "Y" => y,
                    _ => 0,
                };
                n >= b.min && b.max.is_none_or(|m| n <= m)
            })
        })
    };
    assert!(holds(0, 3));
    assert!(holds(1, 1));
    assert!(!holds(1, 2));
}

#[test]
fn lookups() {
    assert!(data_role("clusteredColumnChart", "category").is_none());
    assert!(data_role("noSuchVisual", "Y").is_none());
    assert_eq!(ROLES_DESKTOP_VERSION, "2.159.883.0");
    assert!(!container_objects().is_empty());
    assert_eq!(suggest("Categry", ["Category", "Series"]), Some("Category"));
}

/// Whether some accepted combination of `t` holds for these field counts (kinds ignored).
fn accepts(t: &str, counts: &[(&str, u32)]) -> bool {
    let v = visual(t).unwrap_or_else(|| panic!("{t}"));
    v.role_conditions.is_empty()
        || v.role_conditions.iter().any(|c| {
            c.bounds.iter().all(|b| {
                let n = counts.iter().find(|(r, _)| *r == b.role).map_or(0, |(_, n)| *n);
                n >= b.min && b.max.is_none_or(|m| n <= m)
            })
        })
}

#[test]
fn render_tests_combinations() {
    // render `pair Y=2+Series`, `pair Y2=2+Series`, `all one`: one value in all with a legend.
    assert!(accepts("lineChart", &[("Category", 1), ("Y", 2), ("Y2", 3)]));
    assert!(accepts("lineChart", &[("Series", 1), ("Y2", 1)]));
    assert!(!accepts("lineChart", &[("Series", 1), ("Y", 1), ("Y2", 1)]));
    assert!(!accepts("areaChart", &[("Series", 1), ("Y2", 2)]));
    // Combo charts: the legend limits the columns only.
    assert!(accepts(
        "lineClusteredColumnComboChart",
        &[("Series", 1), ("Y", 1), ("Y2", 3)]
    ));
    assert!(!accepts(
        "lineStackedColumnComboChart",
        &[("Series", 1), ("Y", 2)]
    ));
    // treemap details, funnel category.
    assert!(!accepts("treemap", &[("Details", 1), ("Values", 2)]));
    assert!(!accepts("funnel", &[("Category", 1), ("Y", 2)]));
    assert!(accepts("funnel", &[("Y", 3)]));
    // Key influencers need a target and an explanation (render singles).
    assert!(!accepts("keyDriversVisual", &[("ExplainBy", 1)]));
    assert!(accepts("keyDriversVisual", &[("Target", 1), ("ExplainBy", 2)]));
    // A waterfall breakdown needs a category (render `pair Breakdown+Y`).
    assert!(!accepts("waterfallChart", &[("Breakdown", 1), ("Y", 1)]));
    assert!(accepts(
        "waterfallChart",
        &[("Category", 1), ("Breakdown", 1), ("Y", 1)]
    ));
    // The list slicer needs a value field (render `single Tooltips`).
    assert!(!accepts("listSlicer", &[("Tooltips", 1)]));
}

#[test]
fn kinds_and_relationships_inside_conditions() {
    // kinds-s09/s10 and render `pair X+Play`: with details or a play axis, X and Y summarized.
    let s = visual("scatterChart").unwrap_or_else(|| panic!());
    let summarized = |role: &str| {
        s.role_conditions
            .iter()
            .filter(|c| c.bounds.iter().any(|b| b.min > 0))
            .all(|c| {
                c.bounds
                    .iter()
                    .any(|b| b.role == role && b.kind == Some(RoleKind::Measure))
            })
    };
    assert!(summarized("X") && summarized("Y"));
    // kinds-r01..r12: slicer fields must be related.
    for t in ["slicer", "listSlicer"] {
        let v = visual(t).unwrap_or_else(|| panic!());
        assert!(
            v.role_conditions
                .iter()
                .all(|c| c.bounds.iter().any(|b| b.role == "Values" && b.related)),
            "{t}"
        );
    }
    // Maps: not testable here (kinds-m01..a04), so no combination at all.
    for t in ["map", "filledMap", "azureMap"] {
        assert!(visual(t).is_some_and(|v| v.role_conditions.is_empty()), "{t}");
    }
}

#[test]
fn required_roles_from_render_tests() {
    let req = |t: &str| visual(t).map(|v| v.required_roles.clone()).unwrap_or_default();
    assert_eq!(req("clusteredColumnChart"), [["Y"]]);
    assert_eq!(req("lineChart"), [["Y", "Y2"]]);
    assert_eq!(req("pieChart"), [["Y"]]);
    assert_eq!(req("kpi"), [["Indicator"], ["TrendLine"]]);
    assert!(req("pivotTable").is_empty() && req("scatterChart").is_empty());
}

#[test]
fn gantt_wells_from_desktop_2_159_1231() {
    // bb:gantt-A (role names, kinds of a text or date column), bb:gantt-B (measures kept or
    // refused), bb:gantt-C (maxima), bb:gantt-R (the task row first)
    assert_eq!(
        names("ganttChart"),
        [
            "Task",
            "StartDate",
            "EndDate",
            "Progress",
            "Category",
            "Columns",
            "Tooltips"
        ]
    );
    for (r, kind, max) in [
        ("Task", RoleKind::Grouping, Some(1)),
        ("StartDate", RoleKind::Measure, Some(1)),
        ("EndDate", RoleKind::Measure, Some(1)),
        ("Progress", RoleKind::Measure, Some(1)),
        ("Category", RoleKind::Grouping, Some(3)),
        ("Columns", RoleKind::Measure, None),
        ("Tooltips", RoleKind::Measure, None),
    ] {
        let d = role("ganttChart", r);
        assert_eq!((d.kind, d.max, d.drill), (kind, max, false), "{r}");
    }
    let v = visual("ganttChart").unwrap();
    assert!(v.roles_known);
    assert_eq!(v.required_roles, [["Task"]]);
}
