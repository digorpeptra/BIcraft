//! Model-level tests over synthetic TMDL written for this crate.

use super::*;
use bicraft_model::DataType;

const DATABASE: &str = "database Contoso\n\tcompatibilityLevel: 1601\n";

const MODEL: &str = "\
model Model
\tculture: fr-FR
\tdefaultPowerBIDataSourceVersion: powerBI_V3
\tdiscourageImplicitMeasures

queryGroup Staging

\tannotation PBI_QueryGroupOrder = 0

annotation PBI_ProTooling = [\"DevMode\"]

ref table 'Order Lines'
ref table Calendar
ref table Missing

ref culture fr-FR
";

const ORDER_LINES: &str = "\
/// Fact table of order lines.
/// Second description line.
table 'Order Lines'
\tlineageTag: 0d6b1f7c-0000-4000-8000-000000000001

\t/// Sum of the line amounts.
\tmeasure 'Sales Amount' = SUMX('Order Lines', 'Order Lines'[Qty] * 'Order Lines'[Unit Price])
\t\tformatString: \"#,##0.00\"
\t\tdisplayFolder: \" Key \"\"Figures\"\"\"
\t\tlineageTag: 11111111-2222-4333-8444-555555555555

\tmeasure 'Sales (LY)' =
\t\t\tVAR ly =
\t\t\t\tCALCULATE ( [Sales Amount], SAMEPERIODLASTYEAR ( Calendar[Date] ) )

\t\t\tRETURN
\t\t\t\tly
\t\tformatString: 0.0
\t\tisHidden

\tmeasure 'Margin %' = ```
\t\t\t\tDIVIDE (
\t\t\t\t    [Sales Amount] - [Cost],
\t\t\t\t    [Sales Amount]
\t\t\t\t)
\t\t\t```
\t\tformatString: 0.0%;-0.0%;0.0%

\t\tformatStringDefinition = SELECTEDMEASUREFORMATSTRING()

\tmeasure 'Owner''s Cut' = [Sales Amount] * 0.1
\t\tisHidden: false

\tcolumn 'Order Key'
\t\tdataType: int64
\t\tisHidden
\t\tformatString: 0
\t\tsummarizeBy: none
\t\tsourceColumn: OrderKey

\t\tannotation SummarizationSetBy = Automatic

\tcolumn Qty
\t\tdataType: int64
\t\tsourceColumn: Qty

\tcolumn 'Unit Price'
\t\tdataType: decimal
\t\tsourceColumn: \"Unit Price\"

\tcolumn 'Order Date'
\t\tdataType: dateTime
\t\tsourceColumn: OrderDate

\tcolumn 'Line Total' = 'Order Lines'[Qty] * 'Order Lines'[Unit Price]
\t\tdataType: double
\t\tisDataTypeInferred

\tcolumn Bucket =
\t\t\tSWITCH (
\t\t\t\tTRUE (),
\t\t\t\t[Qty] > 10, \"Large\",
\t\t\t\t\"Small\"
\t\t\t)
\t\tdataType: string

\thierarchy 'Order Hierarchy'
\t\tlevel 'Order Key'
\t\t\tcolumn: 'Order Key'

\tpartition 'Order Lines' = m
\t\tmode: import
\t\tsource =
\t\t\t\tlet
\t\t\t\t    Source = Csv.Document(File.Contents(\"C:\\data\\lines.csv\")),
\t\t\t\t    measure = Source
\t\t\t\tin
\t\t\t\t    measure

\tannotation PBI_ResultType = Table
";

const CALENDAR: &str = "\
table Calendar
\tisHidden
\tdataCategory: Time

\tcolumn Date
\t\tdataType: dateTime
\t\tisKey

\tpartition Calendar = calculated
\t\tmode: import
\t\tsource = CALENDARAUTO()
";

const RELATIONSHIPS: &str = "\
relationship 3f1c0b2e-0000-4000-8000-00000000000a
\tfromColumn: 'Order Lines'.'Order Date'
\ttoColumn: Calendar.Date

relationship 3f1c0b2e-0000-4000-8000-00000000000b
\tisActive: false
\tcrossFilteringBehavior: bothDirections
\tfromColumn: 'Order Lines'.'Order Key'
\ttoColumn: 'Owner''s Table'.Key

relationship incomplete
\tfromColumn: 'Order Lines'.Qty
";

const ROLE: &str = "\
role Readers
\tmodelPermission: read

\ttablePermission 'Order Lines' = 'Order Lines'[Qty] > 0
";

fn docs() -> Vec<(String, String)> {
    [
        ("database.tmdl", DATABASE),
        ("model.tmdl", MODEL),
        ("relationships.tmdl", RELATIONSHIPS),
        ("roles/Readers.tmdl", ROLE),
        ("tables/Calendar.tmdl", CALENDAR),
        ("tables/Order Lines.tmdl", ORDER_LINES),
    ]
    .into_iter()
    .map(|(p, t)| (p.to_string(), t.to_string()))
    .collect()
}

#[test]
fn reads_a_full_model() {
    let m = read_documents(&docs()).unwrap();
    assert_eq!(m.name, "Contoso");
    assert_eq!(m.compatibility_level, Some(1601));
    assert_eq!(m.culture.as_deref(), Some("fr-FR"));
    let names: Vec<_> = m.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["Order Lines", "Calendar"], "`ref table` order wins");

    let lines = m.table("Order Lines").unwrap();
    assert!(!lines.is_hidden);
    let measures: Vec<_> = lines.measures.iter().map(|x| x.name.as_str()).collect();
    assert_eq!(
        measures,
        ["Sales Amount", "Sales (LY)", "Margin %", "Owner's Cut"]
    );

    let sales = lines.measure("Sales Amount").unwrap();
    assert_eq!(
        sales.expression,
        "SUMX('Order Lines', 'Order Lines'[Qty] * 'Order Lines'[Unit Price])"
    );
    assert_eq!(sales.format_string.as_deref(), Some("#,##0.00"));
    assert!(!sales.is_hidden);

    let ly = lines.measure("Sales (LY)").unwrap();
    assert_eq!(
        ly.expression,
        "VAR ly =\n\tCALCULATE ( [Sales Amount], SAMEPERIODLASTYEAR ( Calendar[Date] ) )\n\nRETURN\n\tly"
    );
    assert_eq!(ly.format_string.as_deref(), Some("0.0"));
    assert!(ly.is_hidden);

    let margin = lines.measure("Margin %").unwrap();
    assert_eq!(
        margin.expression,
        "\tDIVIDE (\n\t    [Sales Amount] - [Cost],\n\t    [Sales Amount]\n\t)"
    );
    assert_eq!(margin.format_string.as_deref(), Some("0.0%;-0.0%;0.0%"));

    let cut = lines.measure("Owner's Cut").unwrap();
    assert_eq!(cut.expression, "[Sales Amount] * 0.1");
    assert!(!cut.is_hidden);

    let cols: Vec<_> = lines.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        cols,
        [
            "Order Key",
            "Qty",
            "Unit Price",
            "Order Date",
            "Line Total",
            "Bucket"
        ],
        "hierarchy levels and partitions are not columns"
    );
    let key = lines.column("Order Key").unwrap();
    assert_eq!(key.data_type, DataType::Int64);
    assert!(key.is_hidden);
    assert_eq!(key.expression, None);
    assert_eq!(lines.column("Unit Price").unwrap().data_type, DataType::Decimal);
    assert_eq!(lines.column("Order Date").unwrap().data_type, DataType::DateTime);
    let total = lines.column("Line Total").unwrap();
    assert_eq!(
        total.expression.as_deref(),
        Some("'Order Lines'[Qty] * 'Order Lines'[Unit Price]")
    );
    assert_eq!(total.data_type, DataType::Double);
    assert_eq!(
        lines.column("Bucket").unwrap().expression.as_deref(),
        Some("SWITCH (\n\tTRUE (),\n\t[Qty] > 10, \"Large\",\n\t\"Small\"\n)")
    );

    let cal = m.table("Calendar").unwrap();
    assert!(cal.is_hidden);
    assert_eq!(cal.columns.len(), 1);

    assert_eq!(m.relationships.len(), 3, "incomplete relationships are kept");
    let r0 = &m.relationships[0];
    assert_eq!(
        (
            r0.from_table.as_str(),
            r0.from_column.as_str(),
            r0.to_table.as_str(),
            r0.to_column.as_str()
        ),
        ("Order Lines", "Order Date", "Calendar", "Date")
    );
    assert!(r0.is_active);
    let r1 = &m.relationships[1];
    assert_eq!(r1.to_table, "Owner's Table");
    assert!(!r1.is_active);
    assert_eq!(r1.cross_filtering_behavior.as_deref(), Some("bothDirections"));
    let r2 = &m.relationships[2];
    assert_eq!((r2.from_column.as_str(), r2.to_table.as_str()), ("Qty", ""));

    // Everything else is kept, TMSL-shaped.
    assert_eq!(lines.extra["hierarchies"][0]["levels"][0]["column"], "Order Key");
    assert_eq!(lines.extra["hierarchies"][0]["levels"][0]["ordinal"], 0);
    assert_eq!(lines.partitions[0].mode.as_deref(), Some("import"));
    assert_eq!(lines.annotations[0].value, "Table");
    assert_eq!(
        lines.measure("Margin %").unwrap().extra["formatStringDefinition"]["expression"],
        "SELECTEDMEASUREFORMATSTRING()"
    );
    assert_eq!(lines.measure("Owner's Cut").unwrap().extra["isHidden"], false);
    assert_eq!(
        lines.column("Line Total").unwrap().extra["isDataTypeInferred"],
        true
    );
    assert_eq!(cal.extra["dataCategory"], "Time");
    assert_eq!(cal.columns[0].extra["isKey"], true);
    assert_eq!(m.extra["defaultPowerBIDataSourceVersion"], "powerBI_V3");
    assert_eq!(m.extra["discourageImplicitMeasures"], true);
    assert_eq!(m.extra["queryGroups"][0]["folder"], "Staging");
    assert_eq!(m.annotations[0].name, "PBI_ProTooling");
    let role = &m.roles[0];
    assert_eq!(role.model_permission.as_deref(), Some("read"));
    assert_eq!(
        role.table_permissions[0].filter_expression.as_deref(),
        Some("'Order Lines'[Qty] > 0")
    );
}

#[test]
fn values_are_typed_like_tmsl() {
    let src = "table T
	column C
		isHidden: false
		displayOrdinal: 3
		ratio: 1.5
		looksNumeric: \"42\"
		leadingZero: 007
		flag
		nothing: null
		formatString: 0
		sortByColumn: 'Other Col'
		sourceColumn: \" padded \"

		variation V
			defaultHierarchy: 'My T'.'H 1'
			relationship: 'rel 1'

	partition Q = query
		source
			query = SELECT 1
			dataSource: 'My DS'
";
    let m = read_documents(&[("t.tmdl".into(), src.into())]).unwrap();
    let c = &m.tables[0].columns[0];
    assert!(!c.is_hidden);
    assert_eq!(c.extra["isHidden"], false, "explicit defaults stay in extra");
    assert_eq!(c.extra["displayOrdinal"], 3);
    assert_eq!(c.extra["ratio"], 1.5);
    assert_eq!(c.extra["looksNumeric"], "42");
    assert_eq!(c.extra["leadingZero"], "007");
    assert_eq!(c.extra["flag"], true);
    assert_eq!(c.extra["nothing"], serde_json::Value::Null);
    assert_eq!(c.format_string.as_deref(), Some("0"));
    assert_eq!(c.sort_by_column.as_deref(), Some("Other Col"));
    assert_eq!(c.source_column.as_deref(), Some(" padded "));
    let v = &c.extra["variations"][0];
    assert_eq!(v["defaultHierarchy"]["table"], "My T");
    assert_eq!(v["defaultHierarchy"]["hierarchy"], "H 1");
    assert_eq!(v["relationship"], "rel 1");
    let p = &m.tables[0].partitions[0];
    assert_eq!((p.source_type.as_str(), p.expression.as_deref()), ("query", None));
    assert_eq!(p.source_extra["query"], "SELECT 1");
    assert_eq!(p.source_extra["dataSource"], "My DS");

    // The writer gives back the same text, in Desktop's property order.
    let out = write_documents(&m);
    let table = &out.iter().find(|(p, _)| p == "tables/T.tmdl").unwrap().1;
    let want = "\t\tisHidden: false\r\n\t\tformatString: 0\r\n\t\tdisplayOrdinal: 3\r\n";
    assert!(table.contains(want), "{table}");
    assert!(table.contains("\t\tlooksNumeric: \"42\"\r\n"), "{table}");
    assert_eq!(read_documents(&out).unwrap(), m);
}

#[test]
fn names_starting_with_a_colon_or_quote_read_like_desktop_writes_them() {
    let src = "table :colon
	column \"q\"
		sortByColumn: \"q\"
		formatString : 0
";
    let m = read_documents(&[("t.tmdl".into(), src.into())]).unwrap();
    let t = &m.tables[0];
    assert_eq!(t.name, ":colon");
    assert_eq!(t.columns[0].name, "\"q\"");
    assert_eq!(t.columns[0].sort_by_column.as_deref(), Some("\"q\""));
    assert_eq!(t.columns[0].format_string.as_deref(), Some("0"));
}

#[test]
fn writer_never_panics_on_odd_extra_values() {
    let mut deep = serde_json::json!("leaf");
    for _ in 0..300 {
        deep = serde_json::json!({ "x": [deep] });
    }
    let mut extra = bicraft_model::Properties::new();
    for (k, v) in [
        ("deep", deep),
        ("numbers", serde_json::json!([1, 2, 3])),
        ("emptyList", serde_json::json!([])),
        ("lines", serde_json::json!(["let", "    x = 1", "in", "    x"])),
        ("mixed", serde_json::json!([{"name": "a"}, 1])),
        ("annotations", serde_json::json!([{"name": "a", "value": 1}])),
        ("weird name", serde_json::json!({"name": 5, "description": 7})),
        (
            "hierarchies",
            serde_json::json!([{"levels": [{"ordinal": "x"}]}, {}]),
        ),
        ("calculationGroup", serde_json::json!("not an object")),
        ("translations", serde_json::json!({"model": 3})),
    ] {
        extra.insert(k.into(), v);
    }
    let model = bicraft_model::SemanticModel {
        name: String::new(),
        tables: vec![bicraft_model::Table {
            name: "T".into(),
            extra: extra.clone(),
            partitions: vec![bicraft_model::Partition::default()],
            columns: vec![bicraft_model::Column::default()],
            measures: vec![bicraft_model::Measure::default()],
            ..Default::default()
        }],
        relationships: vec![bicraft_model::Relationship::default()],
        roles: vec![bicraft_model::Role::default()],
        expressions: vec![bicraft_model::NamedExpression::default()],
        database_extra: extra.clone(),
        extra,
        ..Default::default()
    };
    let docs = write_documents(&model);
    assert!(docs.iter().any(|(p, _)| p == "tables/T.tmdl"));
    // Reading the result back may fail on the odd parts, but never panics.
    let _ = read_documents(&docs);
}

#[test]
fn crlf_bom_and_space_indentation_read_the_same() {
    let tabs = read_documents(&docs()).unwrap();
    let converted: Vec<_> = docs()
        .into_iter()
        .map(|(p, t)| (p, format!("\u{feff}{}", t.replace('\n', "\r\n"))))
        .collect();
    assert_eq!(read_documents(&converted).unwrap(), tabs);

    // Re-indent every line with 2 spaces per level (fenced bodies included: their margin comes
    // from the closing ```).
    let spaced: Vec<_> = docs()
        .into_iter()
        .map(|(p, t)| {
            let lines: Vec<String> = t
                .lines()
                .map(|l| {
                    let body = l.trim_start_matches('\t');
                    let tabs = l.len() - body.len();
                    format!("{}{body}", "  ".repeat(tabs))
                })
                .collect();
            (p, lines.join("\n"))
        })
        .collect();
    let spaced = read_documents(&spaced).unwrap();
    // Indentation kept inside expressions is now made of spaces too.
    let mut expected = tabs;
    for t in &mut expected.tables {
        for mm in &mut t.measures {
            mm.expression = mm.expression.replace('\t', "  ");
        }
        for c in &mut t.columns {
            c.expression = c.expression.as_ref().map(|e| e.replace('\t', "  "));
        }
    }
    assert_eq!(spaced, expected);
}

#[test]
fn partial_declarations_merge_and_duplicates_fail() {
    let docs = vec![
        (
            "tables/A.tmdl".to_string(),
            "table A\n\tcolumn X\n\t\tdataType: string\n".to_string(),
        ),
        (
            "measures.tmdl".to_string(),
            "table A\n\tmeasure M = 1\ntable B\n\tmeasure N = 2\n".to_string(),
        ),
    ];
    let m = read_documents(&docs).unwrap();
    assert_eq!(m.tables.len(), 2);
    let a = m.table("A").unwrap();
    assert_eq!((a.columns.len(), a.measures.len()), (1, 1));

    let dup = vec![
        ("a.tmdl".to_string(), "table A\n\tmeasure M = 1\n".to_string()),
        ("b.tmdl".to_string(), "table a\n\n\tmeasure m = 2\n".to_string()),
    ];
    match read_documents(&dup) {
        Err(Error::Syntax { file, line, .. }) => assert_eq!((file.as_str(), line), ("b.tmdl", 3)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn model_children_may_be_indented_and_wrapped_in_create_or_replace() {
    let src = "createOrReplace\n\n\tdatabase\n\t\tcompatibilityLevel: 1550\n\n\t\tmodel Model\n\t\t\tculture: en-US\n\n\t\t\ttable T\n\t\t\t\tcolumn C\n";
    let m = read_documents(&[("script.tmdl".into(), src.into())]).unwrap();
    assert_eq!(m.compatibility_level, Some(1550));
    assert_eq!(m.name, "", "the database has no name");
    assert_eq!(m.table("T").unwrap().columns.len(), 1);
    let m = read_documents(&[("m.tmdl".into(), "model Sales\n".into())]).unwrap();
    assert_eq!(m.name, "Sales", "without a database, the model name stands in");
}

#[test]
fn errors_name_the_file_and_line() {
    let docs = vec![
        ("model.tmdl".to_string(), "model Model\n".to_string()),
        (
            "tables/Bad.tmdl".to_string(),
            "table Bad\n\n\tmeasure Two Words = 1\n".to_string(),
        ),
    ];
    let err = read_documents(&docs).unwrap_err().to_string();
    assert!(err.starts_with("tables/Bad.tmdl:3: "), "{err}");

    let nameless = vec![("t.tmdl".to_string(), "table T\n\tmeasure = 1\n".to_string())];
    let err = read_documents(&nameless).unwrap_err().to_string();
    assert!(err.starts_with("t.tmdl:2: "), "{err}");
}

#[test]
fn read_folder_sorts_and_filters_files() {
    let tmp = crate::tempdir::TempDir::new("tmdl-folder");
    let root = tmp.join("definition");
    std::fs::create_dir_all(root.join("tables")).unwrap();
    std::fs::create_dir_all(root.join("roles")).unwrap();
    for (p, t) in docs() {
        std::fs::write(root.join(&p), t).unwrap();
    }
    std::fs::write(root.join("notes.txt"), "not tmdl at all ''").unwrap();
    let m = read_folder(&root).unwrap();
    assert_eq!(m, read_documents(&docs()).unwrap());

    std::fs::write(root.join("tables/Broken.TMDL"), [0xff, 0xfe, 0x00]).unwrap();
    assert!(matches!(read_folder(&root), Err(Error::Encoding(f)) if f == "tables/Broken.TMDL"));
    let _ = std::fs::remove_dir_all(&root);
    assert!(matches!(read_folder(&root), Err(Error::Io(..))));
}

#[test]
fn hostile_documents_do_not_panic() {
    let samples = [
        "table",
        "table T\n\tcolumn",
        "relationship r\n\tfromColumn: .\n\ttoColumn: 'a'.'b'.'c'",
        "relationship r\n\tfromColumn: 'a\n",
        "database\n\tcompatibilityLevel: 99999999999999",
        "model\n\tculture",
        "ref table",
        "table T\n\t\t\t\t\t\t\tcolumn C\n",
        "\t\tmeasure M = 1",
    ];
    for s in samples {
        let _ = read_documents(&[("x.tmdl".into(), s.into())]);
    }
    let m = read_documents(&[(
        "x.tmdl".into(),
        "database\n\tcompatibilityLevel: 99999999999999".into(),
    )])
    .unwrap();
    assert_eq!(m.compatibility_level, None);
}

/// Auto date/time: `'Sales'[OrderDate].[Year]` resolves through the column's TMDL `variation`.
#[test]
fn column_variations_resolve_after_a_round_trip() {
    let sales = "table Sales
	column OrderDate
		dataType: dateTime
		sourceColumn: OrderDate

		variation Variation
			isDefault
			relationship: 0b5c2f8e-2222
			defaultHierarchy: LocalDateTable_1.'Date Hierarchy'
";
    let ldt = "table LocalDateTable_1
	isHidden
	showAsVariationsOnly

	column Date
		dataType: dateTime
		isHidden
		sourceColumn: Date

	column Year = YEAR([Date])
		dataType: int64
		isHidden

	hierarchy 'Date Hierarchy'

		level Year
			column: Year
";
    let rels = "relationship 0b5c2f8e-2222
	joinOnDateBehavior: datePartOnly
	fromColumn: Sales.OrderDate
	toColumn: LocalDateTable_1.Date
";
    let m = read_documents(&[
        ("tables/Sales.tmdl".into(), sales.into()),
        ("tables/LocalDateTable_1.tmdl".into(), ldt.into()),
        ("relationships.tmdl".into(), rels.into()),
    ])
    .unwrap();
    let v = m
        .table("Sales")
        .unwrap()
        .column("OrderDate")
        .unwrap()
        .variations();
    assert_eq!(v.len(), 1);
    assert!(v[0].is_default);
    assert_eq!(v[0].relationship.as_deref(), Some("0b5c2f8e-2222"));
    let (t, c) = m.variation_column("Sales", "OrderDate", "Year").unwrap();
    assert_eq!((t.name.as_str(), c.name.as_str()), ("LocalDateTable_1", "Year"));
    let back = read_documents(&write_documents(&m)).unwrap();
    assert_eq!(back, m);
}
