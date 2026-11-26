//! Tests of the diff on synthetic models and reports: every change kind, renames, moves,
//! noise, determinism and hostile inputs.

use bicraft_model::{
    Annotation, Column, DataType, Measure, NamedExpression, Partition, Relationship, Role, SemanticModel,
    Table, TablePermission,
};
use bicraft_report::{FieldBinding, FieldRef, NamedDocument, Page, Position, Report, Resource, Visual};
use serde_json::{Value, json};

use crate::{Change, Diff, How, Input, Kind, Options, Scope, diff, render};

fn col(name: &str, ty: DataType, tag: &str) -> Column {
    Column {
        name: name.into(),
        data_type: ty,
        source_column: Some(name.into()),
        lineage_tag: Some(tag.into()),
        ..Default::default()
    }
}

fn measure(name: &str, expr: &str, tag: &str) -> Measure {
    Measure {
        name: name.into(),
        expression: expr.into(),
        lineage_tag: Some(tag.into()),
        ..Default::default()
    }
}

fn model() -> SemanticModel {
    SemanticModel {
        name: "M".into(),
        culture: Some("en-US".into()),
        compatibility_level: Some(1567),
        tables: vec![
            Table {
                name: "Sales".into(),
                lineage_tag: Some("t-sales".into()),
                columns: vec![
                    col("Amount", DataType::Decimal, "c-amount"),
                    col("CustomerKey", DataType::Int64, "c-ck"),
                    col("Region", DataType::String, "c-region"),
                ],
                measures: vec![
                    measure("Total", "SUM(Sales[Amount])", "m-total"),
                    measure("Count", "COUNTROWS(Sales)", "m-count"),
                    measure("Avg", "AVERAGE(Sales[Amount])", "m-avg"),
                ],
                partitions: vec![Partition {
                    name: "Sales-1".into(),
                    mode: Some("import".into()),
                    source_type: "m".into(),
                    expression: Some("let\n    Source = Csv.Document(\"x\")\nin\n    Source".into()),
                    ..Default::default()
                }],
                extra: serde_json::from_value(json!({
                    "hierarchies": [{"name": "Geo", "lineageTag": "h-geo", "levels": [{"name": "Region", "column": "Region", "ordinal": 0}]}]
                }))
                .unwrap(),
                ..Default::default()
            },
            Table {
                name: "Customer".into(),
                lineage_tag: Some("t-cust".into()),
                columns: vec![col("CustomerKey", DataType::Int64, "c-cust-ck"), col("Name", DataType::String, "c-name")],
                ..Default::default()
            },
            Table {
                name: "Time Calc".into(),
                lineage_tag: Some("t-tc".into()),
                extra: serde_json::from_value(json!({
                    "calculationGroup": {"precedence": 1, "calculationItems": [
                        {"name": "Current", "expression": "SELECTEDMEASURE()", "ordinal": 0},
                        {"name": "YTD", "expression": "CALCULATE(SELECTEDMEASURE(), DATESYTD('Date'[Date]))", "ordinal": 1}
                    ]}
                }))
                .unwrap(),
                ..Default::default()
            },
        ],
        relationships: vec![Relationship {
            name: "rel-guid-1".into(),
            from_table: "Sales".into(),
            from_column: "CustomerKey".into(),
            to_table: "Customer".into(),
            to_column: "CustomerKey".into(),
            is_active: true,
            ..Default::default()
        }],
        expressions: vec![
            NamedExpression {
                name: "Server".into(),
                kind: "m".into(),
                expression: "\"localhost\" meta [IsParameterQuery=true, Type=\"Text\", IsParameterQueryRequired=true]".into(),
                lineage_tag: Some("e-server".into()),
                ..Default::default()
            },
            NamedExpression {
                name: "Shared".into(),
                kind: "m".into(),
                expression: "let\n    x = 1\nin\n    x".into(),
                ..Default::default()
            },
        ],
        roles: vec![Role {
            name: "EU".into(),
            model_permission: Some("read".into()),
            table_permissions: vec![TablePermission {
                table: "Sales".into(),
                filter_expression: Some("[Region] = \"EU\"".into()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        annotations: vec![
            Annotation {
                name: "PBIDesktopVersion".into(),
                value: "2.158".into(),
            },
            Annotation {
                name: "PBI_QueryOrder".into(),
                value: "[\"Sales\"]".into(),
            },
        ],
        extra: serde_json::from_value(json!({
            "perspectives": [{"name": "Lite", "tables": [{"name": "Sales", "measures": [{"name": "Total"}]}]}],
            "cultures": [{"name": "en-US"}],
        }))
        .unwrap(),
        ..Default::default()
    }
}

fn literal(v: &str) -> Value {
    json!({"expr": {"Literal": {"Value": v}}})
}

fn visual(name: &str, ty: &str, x: f64, fields: &[(&str, &str, &str)]) -> Visual {
    Visual {
        name: name.into(),
        visual_type: Some(ty.into()),
        position: Position {
            x,
            y: 10.0,
            z: 0.0,
            width: 300.0,
            height: 200.0,
            tab_order: Some(0),
        },
        fields: fields
            .iter()
            .map(|(role, t, m)| FieldBinding {
                display_name: None,
                show_as: None,
                role: (*role).into(),
                field: FieldRef::Measure {
                    table: (*t).into(),
                    measure: (*m).into(),
                },
            })
            .collect(),
        visual_extra: serde_json::from_value(
            json!({"objects": {"legend": [{"properties": {"show": literal("true")}}]}}),
        )
        .unwrap(),
        ..Default::default()
    }
}

fn report() -> Report {
    Report {
        schemas: serde_json::from_value(json!({"report": "https://example/schema/1.0.0"})).unwrap(),
        pages: vec![
            Page {
                name: "p1".into(),
                display_name: "Overview".into(),
                width: 1280.0,
                height: 720.0,
                visuals: vec![
                    visual("v1", "card", 0.0, &[("Values", "Sales", "Total")]),
                    visual(
                        "v2",
                        "clusteredColumnChart",
                        400.0,
                        &[("Y", "Sales", "Total"), ("Y", "Sales", "Count")],
                    ),
                    visual("v3", "tableEx", 800.0, &[("Values", "Sales", "Avg")]),
                ],
                filter_config: Some(
                    json!({"filters": [{"name": "f1", "field": {"Column": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Region"}}, "type": "Categorical"}]}),
                ),
                ..Default::default()
            },
            Page {
                name: "p2".into(),
                display_name: "Details".into(),
                width: 1280.0,
                height: 720.0,
                ..Default::default()
            },
        ],
        theme_collection: Some(json!({"baseTheme": {"name": "CY24SU10", "reportVersionAtImport": "5.55"}})),
        bookmarks: vec![NamedDocument {
            name: "b1".into(),
            document: json!({"displayName": "Q1", "explorationState": {"activeSection": "p1"}}),
        }],
        resources: vec![Resource {
            path: "StaticResources/RegisteredResources/theme.json".into(),
            bytes: br##"{"name": "Mine", "dataColors": ["#111111"]}"##.to_vec(),
        }],
        version_extra: serde_json::from_value(json!({"version": "2.0.0"})).unwrap(),
        ..Default::default()
    }
}

fn run(a: &SemanticModel, ra: &Report, b: &SemanticModel, rb: &Report, opts: &Options) -> Diff {
    diff(
        &Input {
            model: Some(a),
            report: Some(ra),
            data: None,
        },
        &Input {
            model: Some(b),
            report: Some(rb),
            data: None,
        },
        opts,
    )
}

fn find<'d>(d: &'d Diff, id: &str) -> &'d Change {
    d.changes
        .iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("no change `{id}` in\n{}", render::text(d, "a", "b")))
}

fn has_prop(c: &Change, name: &str) -> bool {
    c.properties.iter().any(|p| p.property == name)
}

#[test]
fn same_files_have_no_differences() {
    let (m, r) = (model(), report());
    let d = run(&m, &r, &m, &r, &Options::default());
    assert!(d.identical, "{}", render::text(&d, "a", "b"));
    assert_eq!(render::text(&d, "a", "b"), "a: a\nb: b\nNo differences.\n");
}

#[test]
fn noise_is_not_a_difference() {
    let (ma, ra) = (model(), report());
    let (mut mb, mut rb) = (model(), report());
    // Desktop version, CRLF, collection order, $schema and version.json, generated ids
    mb.annotations[0].value = "2.159".into();
    mb.tables[0].partitions[0].expression =
        Some("let\r\n    Source = Csv.Document(\"x\")\r\nin\r\n    Source".into());
    mb.tables[0].partitions[0].name = "Sales-2".into();
    mb.tables.reverse();
    mb.tables[2].columns.reverse();
    mb.relationships[0].name = "rel-guid-2".into();
    rb.schemas = serde_json::from_value(json!({"report": "https://example/schema/2.0.0"})).unwrap();
    rb.version_extra = serde_json::from_value(json!({"version": "2.1.0"})).unwrap();
    rb.theme_collection = Some(json!({"baseTheme": {"name": "CY24SU10", "reportVersionAtImport": "5.60"}}));
    rb.pages[0].visuals.reverse();
    rb.pages[0].name = "p1-regenerated".into();
    rb.active_page = Some("p2".into());
    let d = run(&ma, &ra, &mb, &rb, &Options::default());
    assert!(d.identical, "{}", render::text(&d, "a", "b"));
}

#[test]
fn every_model_change_kind() {
    let (ma, r) = (model(), report());
    let mut mb = model();
    {
        let sales = &mut mb.tables[0];
        sales.description = Some("Fact table".into());
        sales.columns[0].format_string = Some("#,0.00".into());
        sales.columns[2].data_type = DataType::Int64;
        sales.columns.push(Column {
            name: "Margin".into(),
            data_type: DataType::Decimal,
            expression: Some("Sales[Amount] * 0.1".into()),
            ..Default::default()
        });
        sales.measures[0].expression = "SUMX(Sales, Sales[Amount])".into();
        sales.measures[1].expression = "countrows ( 'Sales' ) // rows".into();
        sales.measures.push(measure("Max", "MAX(Sales[Amount])", "m-max"));
        sales.partitions[0].expression = Some("let\n    Source = Csv.Document(\"y\")\nin\n    Source".into());
        sales.extra.insert(
            "hierarchies".into(),
            json!([{"name": "Geo", "lineageTag": "h-geo", "levels": [{"name": "Region", "column": "Region", "ordinal": 0}, {"name": "Name", "column": "Name", "ordinal": 1}]}]),
        );
    }
    {
        let cg = mb.tables[2].extra.get_mut("calculationGroup").unwrap();
        cg["calculationItems"][1]["expression"] =
            json!("CALCULATE(SELECTEDMEASURE(), DATESQTD('Date'[Date]))");
    }
    mb.tables[1].columns.remove(1);
    mb.tables.push(Table {
        name: "Returns".into(),
        columns: vec![col("Qty", DataType::Int64, "c-qty")],
        ..Default::default()
    });
    mb.relationships[0].cross_filtering_behavior = Some("bothDirections".into());
    mb.roles[0].table_permissions[0].filter_expression = Some("[Region] = \"EMEA\"".into());
    mb.roles.push(Role {
        name: "All".into(),
        model_permission: Some("read".into()),
        ..Default::default()
    });
    mb.expressions[0].expression = mb.expressions[0].expression.replace("localhost", "prod");
    mb.expressions.remove(1);
    mb.culture = Some("fr-FR".into());
    mb.annotations[1].value = "[\"Sales\",\"Returns\"]".into();
    mb.extra.insert(
        "perspectives".into(),
        json!([{"name": "Lite", "tables": [{"name": "Sales", "measures": [{"name": "Total"}, {"name": "Max"}]}]}]),
    );
    let d = run(&ma, &r, &mb, &r, &Options::default());
    let text = render::text(&d, "a", "b");
    assert!(has_prop(find(&d, "model"), "culture"), "{text}");
    assert!(has_prop(find(&d, "model"), "annotation PBI_QueryOrder"), "{text}");
    assert!(has_prop(find(&d, "table 'Sales'"), "description"));
    assert!(has_prop(find(&d, "column 'Sales'[Amount]"), "format_string"));
    assert!(has_prop(find(&d, "column 'Sales'[Region]"), "data_type"));
    assert_eq!(find(&d, "column 'Sales'[Margin]").change, Kind::Added);
    assert_eq!(find(&d, "column 'Customer'[Name]").change, Kind::Removed);
    assert_eq!(find(&d, "table 'Returns'").change, Kind::Added);
    let total = find(&d, "measure 'Sales'[Total]");
    assert!(
        total
            .properties
            .iter()
            .any(|p| p.property == "expression" && !p.cosmetic)
    );
    let count = find(&d, "measure 'Sales'[Count]");
    assert!(count.properties.iter().all(|p| p.cosmetic), "{text}");
    assert_eq!(find(&d, "measure 'Sales'[Max]").change, Kind::Added);
    assert!(has_prop(find(&d, "partition 'Sales'/'Sales-1'"), "expression"));
    assert!(
        has_prop(find(&d, "hierarchy 'Sales'[Geo]"), "levels[Name]"),
        "{text}"
    );
    assert!(has_prop(
        find(&d, "calculation_item 'Time Calc'[YTD]"),
        "expression"
    ));
    assert!(has_prop(
        find(&d, "relationship 'Sales'[CustomerKey] -> 'Customer'[CustomerKey]"),
        "cross_filtering_behavior"
    ));
    assert!(has_prop(find(&d, "role 'EU'"), "filter 'Sales'"));
    assert_eq!(find(&d, "role 'All'").change, Kind::Added);
    assert!(has_prop(find(&d, "parameter 'Server'"), "expression"));
    assert_eq!(find(&d, "expression 'Shared'").change, Kind::Removed);
    assert!(
        find(&d, "perspective 'Lite'")
            .properties
            .iter()
            .any(|p| p.property.contains("[Max]")),
        "{text}"
    );
    // ignore_whitespace drops the cosmetic change only
    let quiet = run(
        &ma,
        &r,
        &mb,
        &r,
        &Options {
            ignore_whitespace: true,
            ..Default::default()
        },
    );
    assert!(!quiet.changes.iter().any(|c| c.id == "measure 'Sales'[Count]"));
    assert!(quiet.changes.iter().any(|c| c.id == "measure 'Sales'[Total]"));
}

#[test]
fn renames_and_moves() {
    let (ma, r) = (model(), report());
    let mut mb = model();
    // by lineage tag: a table, a column, a measure, a parameter
    mb.tables[1].name = "Clients".into();
    mb.relationships[0].to_table = "Clients".into();
    mb.tables[0].columns[0].name = "Sales Amount".into();
    mb.tables[0].measures[0].name = "Total Sales".into();
    mb.expressions[0].name = "ServerName".into();
    // by content: no tag, same expression
    mb.tables[0].measures[2].lineage_tag = None;
    mb.tables[0].measures[2].name = "Average".into();
    // a move: the measure goes to another table
    let count = mb.tables[0].measures.remove(1);
    mb.tables[1].measures.push(count);
    // a hierarchy and a calculation item renamed
    mb.tables[0].extra["hierarchies"][0]["name"] = json!("Geography");
    mb.tables[2].extra.get_mut("calculationGroup").unwrap()["calculationItems"][0]["name"] = json!("Now");
    // a page renamed, a visual moved to another page
    let mut rb = report();
    rb.pages[0].display_name = "Summary".into();
    let v3 = rb.pages[0].visuals.remove(2);
    rb.pages[1].visuals.push(v3);
    let d = run(&ma, &r, &mb, &rb, &Options::default());
    let text = render::text(&d, "a", "b");
    let t = find(&d, "table 'Clients'");
    assert_eq!(
        (t.change, t.from.as_deref(), t.how),
        (Kind::Renamed, Some("table 'Customer'"), Some(How::LineageTag))
    );
    // the relationship follows the renamed table: no change
    assert!(!d.changes.iter().any(|c| c.object == "relationship"), "{text}");
    assert_eq!(
        find(&d, "column 'Sales'[Sales Amount]").from.as_deref(),
        Some("column 'Sales'[Amount]")
    );
    assert_eq!(find(&d, "measure 'Sales'[Total Sales]").change, Kind::Renamed);
    let avg = find(&d, "measure 'Sales'[Average]");
    assert_eq!((avg.change, avg.how), (Kind::Renamed, Some(How::Content)));
    let moved = find(&d, "measure 'Clients'[Count]");
    assert_eq!(
        (moved.change, moved.from.as_deref()),
        (Kind::Moved, Some("measure 'Sales'[Count]"))
    );
    assert_eq!(find(&d, "parameter 'ServerName'").change, Kind::Renamed);
    assert_eq!(find(&d, "hierarchy 'Sales'[Geography]").change, Kind::Renamed);
    assert_eq!(
        find(&d, "calculation_item 'Time Calc'[Now]").change,
        Kind::Renamed
    );
    assert_eq!(
        find(&d, "page \"Summary\"").from.as_deref(),
        Some("page \"Overview\"")
    );
    let v = find(&d, "visual v3 on page \"Details\"");
    assert_eq!(
        (v.change, v.from.as_deref()),
        (Kind::Moved, Some("visual v3 on page \"Overview\""))
    );
    assert_eq!(d.summary.added + d.summary.removed, 0, "{text}");
}

#[test]
fn every_report_change_kind() {
    let (m, ra) = (model(), report());
    let mut rb = report();
    {
        let p = &mut rb.pages[0];
        p.width = 1920.0;
        p.visuals[0].position.x = 20.0;
        p.visuals[0].position.tab_order = Some(3);
        p.visuals[1].fields.push(FieldBinding {
            display_name: None,
            show_as: None,
            role: "Category".into(),
            field: FieldRef::Column {
                table: "Customer".into(),
                column: "Name".into(),
            },
        });
        p.visuals[1].fields.swap(0, 1);
        p.visuals[1].visual_extra = serde_json::from_value(json!({"objects": {
            "legend": [{"properties": {"show": literal("false")}}],
            "title": [{"properties": {"text": literal("'Sales'")}}]
        }}))
        .unwrap();
        p.visuals[2].visual_type = Some("matrix".into());
        p.visuals.push(visual("v4", "slicer", 0.0, &[]));
        p.filter_config = Some(
            json!({"filters": [{"name": "f1", "field": {"Column": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Region"}}, "type": "Categorical", "filter": {"Where": []}}]}),
        );
    }
    rb.pages.push(Page {
        name: "p3".into(),
        display_name: "New".into(),
        ..Default::default()
    });
    rb.theme_collection = Some(json!({"baseTheme": {"name": "CY25SU01"}}));
    rb.bookmarks[0].document["explorationState"]["activeSection"] = json!("p2");
    rb.resources[0].bytes = br##"{"name": "Mine", "dataColors": ["#222222"]}"##.to_vec();
    rb.filter_config = Some(
        json!({"filters": [{"name": "rf", "field": {"Measure": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Total"}}}]}),
    );
    let d = run(&m, &ra, &m, &rb, &Options::default());
    let text = render::text(&d, "a", "b");
    assert!(has_prop(find(&d, "page \"Overview\""), "size"), "{text}");
    let v1 = find(&d, "visual v1 on page \"Overview\"");
    assert!(has_prop(v1, "position") && has_prop(v1, "tab_order"), "{text}");
    let v2 = find(&d, "visual v2 on page \"Overview\"");
    assert_eq!(v2.fields.len(), 2, "{text}");
    assert!(v2.fields.iter().any(|f| f.role == "Y" && f.reordered));
    assert!(
        v2.fields
            .iter()
            .any(|f| f.role == "Category" && f.added == ["Customer[Name]"])
    );
    assert!(v2.formatting.iter().any(|f| f.property == "legend.show"));
    assert!(
        v2.formatting
            .iter()
            .any(|f| f.property == "title.text" && f.before.is_none())
    );
    assert!(has_prop(find(&d, "visual v3 on page \"Overview\""), "type"));
    assert_eq!(find(&d, "visual v4 on page \"Overview\"").change, Kind::Added);
    assert_eq!(find(&d, "page \"New\"").change, Kind::Added);
    assert!(
        has_prop(find(&d, "filter Sales[Region] on page \"Overview\""), "filter"),
        "{text}"
    );
    assert_eq!(find(&d, "filter Sales[Total] on report").change, Kind::Added);
    assert!(has_prop(find(&d, "theme"), "themeCollection.baseTheme.name"));
    assert!(
        find(&d, "bookmark \"Q1\"")
            .properties
            .iter()
            .any(|p| p.property == "explorationState.activeSection")
    );
    assert!(
        has_prop(
            find(&d, "resource StaticResources/RegisteredResources/theme.json"),
            "dataColors[0]"
        ),
        "{text}"
    );
    // the options leave positions and formatting out
    let opts = Options {
        ignore_positions: true,
        ignore_formatting: true,
        ..Default::default()
    };
    let q = run(&m, &ra, &m, &rb, &opts);
    assert!(!q.changes.iter().any(|c| c.id == "visual v1 on page \"Overview\""));
    assert!(
        !q.changes
            .iter()
            .any(|c| c.object == "theme" || c.object == "resource")
    );
    assert!(q.changes.iter().all(|c| c.formatting.is_empty()));
    // scopes
    let only_model = run(
        &m,
        &ra,
        &m,
        &rb,
        &Options {
            scope: Scope::Model,
            ..Default::default()
        },
    );
    assert!(only_model.identical);
}

#[test]
fn added_and_deleted_files() {
    let (m, r) = (model(), report());
    let none = Input::default();
    let full = Input {
        model: Some(&m),
        report: Some(&r),
        data: None,
    };
    let added = diff(&none, &full, &Options::default());
    assert!(
        added
            .changes
            .iter()
            .all(|c| matches!(c.change, Kind::Added | Kind::Changed)),
        "{}",
        render::text(&added, "a", "b")
    );
    assert!(added.changes.iter().any(|c| c.id == "table 'Sales'"));
    let removed = diff(&full, &none, &Options::default());
    assert!(
        removed
            .changes
            .iter()
            .all(|c| matches!(c.change, Kind::Removed | Kind::Changed))
    );
    assert_eq!(added.summary.added, removed.summary.removed);
}

#[test]
fn data_snapshots_count_only_without_metadata_changes() {
    use crate::DataStamp;
    let m = model();
    let stamp = |crc| DataStamp {
        size: 100,
        crc32: crc,
    };
    let a = Input {
        model: Some(&m),
        report: None,
        data: Some(stamp(1)),
    };
    let b = Input {
        data: Some(stamp(2)),
        ..a
    };
    let d = diff(&a, &b, &Options::default());
    assert_eq!(d.changes.len(), 1);
    assert_eq!(d.changes[0].object, "data");
    assert!(!d.data.unwrap().same);
    let same = diff(&a, &a, &Options::default());
    assert!(same.identical && same.data.unwrap().same);
}

#[test]
fn output_is_deterministic_and_renders() {
    let (ma, ra) = (model(), report());
    let mut mb = model();
    mb.tables[0].measures[0].expression = "VAR x = SUM(Sales[Amount])\nRETURN\n    x * 2".into();
    mb.tables[0].measures[1].lineage_tag = None;
    mb.tables[0].measures[1].expression = "COUNTROWS(Sales) + 1".into();
    mb.tables[0].measures[1].name = "Rows `quoted` | pipe".into();
    let mut rb = report();
    rb.pages[0].visuals[0].position.y = 99.0;
    let d1 = run(&ma, &ra, &mb, &rb, &Options::default());
    let d2 = run(&ma, &ra, &mb, &rb, &Options::default());
    let j1 = serde_json::to_string(&d1).unwrap();
    assert_eq!(j1, serde_json::to_string(&d2).unwrap());
    let text = render::text(&d1, "a.pbix", "b");
    assert!(
        text.contains("  ~ measure 'Sales'[Total]\n      expression:\n        - SUM(Sales[Amount])\n        + VAR x = SUM(Sales[Amount])\n"),
        "{text}"
    );
    assert!(
        text.contains("position: \"0,10 300x200\" -> \"0,99 300x200\""),
        "{text}"
    );
    assert!(!text.contains('\r'));
    let md = render::markdown(&d1, "a.pbix", "b");
    assert!(md.starts_with("### Power BI diff\n"), "{md}");
    assert!(md.contains("```diff\n"), "{md}");
    assert!(md.contains("`` measure 'Sales'[Rows `quoted` | pipe] ``"), "{md}");
    assert!(!md.contains('\r'));
}

#[test]
fn hostile_inputs_stay_bounded() {
    // many objects, duplicate and empty names, huge expressions, deep extras
    let mut a = model();
    let mut b = model();
    let big = "1 + ".repeat(200_000) + "1";
    a.tables[0].measures.push(measure("Big", &big, "m-big"));
    b.tables[0]
        .measures
        .push(measure("Big", &(big.clone() + " + 2"), "m-big"));
    for i in 0..3000 {
        b.tables.push(Table {
            name: format!("T{i}"),
            ..Default::default()
        });
    }
    b.tables.push(Table::default());
    b.tables.push(Table::default());
    b.tables[0].measures.push(measure("Total", "1", "dup"));
    let mut deep = json!(1);
    for _ in 0..300 {
        deep = json!({"k": [deep]});
    }
    b.extra.insert("deep".into(), deep);
    let r = report();
    let start = std::time::Instant::now();
    let d = run(&a, &r, &b, &r, &Options::default());
    let text = render::text(&d, "a", "b");
    let md = render::markdown(&d, "a", "b");
    assert!(start.elapsed().as_secs() < 30);
    assert!(d.summary.added >= 3000);
    assert!(text.len() < 50 << 20 && md.len() < 50 << 20);
}
