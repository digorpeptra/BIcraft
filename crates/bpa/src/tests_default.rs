//! The embedded default rule set on synthetic models: every rule is evaluated, none fires on a
//! model built to follow them, and each fires on a model built to break them.

use std::collections::BTreeSet;

use crate::{Analysis, Options, analyze, default_rules};

fn model(tmsl: &str) -> bicraft_model::SemanticModel {
    bicraft_tmsl::read(tmsl).unwrap_or_else(|e| panic!("bad test model: {e}"))
}

fn run(tmsl: &str) -> Analysis {
    let set = default_rules();
    assert!(set.invalid.is_empty(), "{:?}", set.invalid);
    analyze(&model(tmsl), &set.rules, &Options::default())
}

fn assert_all_evaluated(a: &Analysis) {
    let bad: Vec<String> = a
        .rules
        .iter()
        .filter(|r| r.status != "evaluated")
        .map(|r| format!("{} {}: {:?}", r.rule_id, r.status, r.reason))
        .collect();
    assert!(bad.is_empty(), "rules not fully evaluated:\n{}", bad.join("\n"));
}

/// A small star schema that follows every default rule.
const CLEAN: &str = r##"{
  "name": "Clean", "compatibilityLevel": 1567,
  "model": {
    "defaultPowerBIDataSourceVersion": "powerBI_V3",
    "tables": [
      { "name": "Sales", "description": "Sales facts",
        "columns": [
          {"name": "CustomerKey", "dataType": "int64", "sourceColumn": "CustomerKey", "isHidden": true,
           "isAvailableInMDX": false, "summarizeBy": "none"},
          {"name": "DateKey", "dataType": "int64", "sourceColumn": "DateKey", "isHidden": true,
           "isAvailableInMDX": false, "summarizeBy": "none"},
          {"name": "Amount", "dataType": "decimal", "sourceColumn": "Amount", "isHidden": true,
           "isAvailableInMDX": false, "summarizeBy": "sum"}
        ],
        "measures": [
          {"name": "Total Amount", "expression": "SUM(Sales[Amount])", "formatString": "#,0",
           "description": "Sum of the amounts"},
          {"name": "Amount Share", "expression": "DIVIDE([Total Amount], CALCULATE([Total Amount], ALL(Customer)))",
           "formatString": "#,0.0%;-#,0.0%;#,0.0%", "description": "Share of all customers"}
        ],
        "partitions": [{"name": "Sales", "source": {"type": "m", "expression": "let Source = #table({}, {}) in Source"}}]
      },
      { "name": "Customer", "description": "Customers",
        "columns": [
          {"name": "Customer Id", "dataType": "int64", "sourceColumn": "Id", "isKey": true,
           "summarizeBy": "none", "description": "Customer identifier"},
          {"name": "Customer Name", "dataType": "string", "sourceColumn": "Name", "description": "Full name"}
        ],
        "partitions": [{"name": "Customer", "source": {"type": "m", "expression": "let Source = #table({}, {}) in Source"}}]
      },
      { "name": "Date", "description": "Calendar", "dataCategory": "Time",
        "columns": [
          {"name": "Date Id", "dataType": "int64", "sourceColumn": "DateId", "summarizeBy": "none",
           "description": "yyyymmdd"},
          {"name": "Date", "dataType": "dateTime", "sourceColumn": "Date", "isKey": true,
           "formatString": "mm/dd/yyyy", "description": "The day"}
        ],
        "partitions": [{"name": "Date", "source": {"type": "m", "expression": "let Source = #table({}, {}) in Source"}}]
      }
    ],
    "relationships": [
      {"name": "r1", "fromTable": "Sales", "fromColumn": "CustomerKey", "toTable": "Customer", "toColumn": "Customer Id"},
      {"name": "r2", "fromTable": "Sales", "fromColumn": "DateKey", "toTable": "Date", "toColumn": "Date Id"}
    ]
  }
}"##;

/// A model that breaks every default rule at least once.
const BAD: &str = r##"{
  "name": "Bad ", "compatibilityLevel": 1567,
  "model": {
    "defaultPowerBIDataSourceVersion": "powerBI_V3",
    "dataSources": [
      {"name": "Unused DS", "connectionString": "Provider=X"},
      {"type": "structured", "name": "Structured DS", "connectionDetails": {"protocol": "tds"}}
    ],
    "perspectives": [{"name": "Empty P", "tables": []}],
    "tables": [
      { "name": "Fact",
        "annotations": [{"name": "Vertipaq_RowCount", "value": "30000000"}],
        "columns": [
          {"name": "Price", "dataType": "double", "sourceColumn": "Price"},
          {"name": "CustKey", "dataType": "int64", "sourceColumn": "CustKey", "summarizeBy": "none",
           "annotations": [{"name": "Vertipaq_Cardinality", "value": "200000"}]},
          {"name": "Comment", "dataType": "string", "sourceColumn": "Comment",
           "annotations": [{"name": "LongLengthRowCount", "value": "600000"}]},
          {"name": "Order Date", "dataType": "dateTime", "sourceColumn": "OrderDate",
           "annotations": [{"name": "DateTimeWithHourMinSec", "value": "5"}]},
          {"name": "Order Month", "dataType": "dateTime", "sourceColumn": "OrderMonth"},
          {"name": "Hidden Unused", "dataType": "string", "sourceColumn": "HU", "isHidden": true},
          {"name": "Sorted", "dataType": "string", "sourceColumn": "S", "isHidden": true, "isAvailableInMDX": false},
          {"name": "Label", "dataType": "string", "sourceColumn": "L", "sortByColumn": "Sorted"},
          {"name": "NoSource", "dataType": "string"},
          {"name": "IsActive", "dataType": "int64", "sourceColumn": "IsActive", "summarizeBy": "none"},
          {"name": "Country", "dataType": "string", "sourceColumn": "Country"},
          {"name": "Month Name", "dataType": "string", "sourceColumn": "MonthName"},
          {"name": "Bad\u0001Name", "dataType": "string", "sourceColumn": "B"},
          {"name": "Tab\tName", "dataType": "string", "sourceColumn": "T"},
          {"name": "Ctl", "dataType": "string", "sourceColumn": "C", "description": "x\u0002y"},
          {"name": "Calc1", "dataType": "string", "type": "calculated", "expression": "RELATED(Customer[Name])"},
          {"name": "Calc2", "dataType": "int64", "type": "calculated", "expression": "IFERROR(1, 0)"},
          {"name": "Calc3", "dataType": "int64", "type": "calculated", "expression": " "},
          {"name": "Calc4", "dataType": "int64", "type": "calculated", "expression": "1"},
          {"name": "Calc5", "dataType": "int64", "type": "calculated", "expression": "2"},
          {"name": "calc lower", "dataType": "int64", "type": "calculated", "expression": "3"}
        ],
        "measures": [
          {"name": "Total Price", "expression": "SUM('Fact'[Price])",
           "kpi": {"targetExpression": "[Price]", "statusExpression": "1"}},
          {"name": "Dup A", "expression": "SUM( 'Fact'[Price] )", "formatString": "#,0"},
          {"name": "Ref", "expression": "[Total Price]", "formatString": "#,0"},
          {"name": "Unqualified", "expression": "SUMX('Fact', [Price])", "formatString": "#,0"},
          {"name": "Qualified Measure", "expression": "'Fact'[Total Price] + 1", "formatString": "#,0"},
          {"name": "Intersect", "expression": "COUNTROWS(INTERSECT(VALUES('Fact'[CustKey]), VALUES(Customer[Id])))", "formatString": "#,0"},
          {"name": "Divide", "expression": "SUM('Fact'[Price]) / 2", "formatString": "#,0"},
          {"name": "Filter Col", "expression": "CALCULATE(SUM('Fact'[Price]), FILTER('Fact', 'Fact'[Country] = \"x\"))", "formatString": "#,0"},
          {"name": "Filter Measure", "expression": "CALCULATE(SUM('Fact'[Price]), FILTER('Fact', [Total Price] > 1))", "formatString": "#,0"},
          {"name": "Ratio", "expression": "1 - SUM('Fact'[Price]) / 2", "formatString": "#,0"},
          {"name": "Log", "expression": "EVALUATEANDLOG(1)", "formatString": "#,0"},
          {"name": "Empty", "expression": "", "formatString": "#,0"},
          {"name": "Pct", "expression": "0.5", "formatString": "0%"},
          {"name": "TI", "expression": "TOTALYTD(SUM('Fact'[Price]), 'Fact'[Order Date])", "formatString": "#,0"},
          {"name": "Hidden Unused Measure", "expression": "1", "isHidden": true, "formatString": "#,0"},
          {"name": "lower measure", "expression": "2", "formatString": "#,0"},
          {"name": "Use Rel", "expression": "CALCULATE(1, USERELATIONSHIP('Fact'[Order Date], 'Calendar'[Date]))", "formatString": "#,0"}
        ],
        "partitions": [{"name": "P1", "mode": "directQuery",
          "source": {"type": "m", "expression": "let Source = Table.Join(A, B) in Source"}}]
      },
      { "name": "Customer", "description": "Customers",
        "columns": [
          {"name": "Id", "dataType": "int64", "sourceColumn": "Id", "summarizeBy": "none"},
          {"name": "Name", "dataType": "string", "sourceColumn": "Name"},
          {"name": "Email", "dataType": "string", "sourceColumn": "Email"},
          {"name": "Segment", "dataType": "string", "sourceColumn": "Segment"},
          {"name": "RegionKey", "dataType": "int64", "sourceColumn": "RegionKey", "summarizeBy": "none"},
          {"name": "Country", "dataType": "string", "sourceColumn": "Country", "dataCategory": "Country"}
        ],
        "partitions": [{"name": "Customer", "source": {"type": "m", "expression": "let Source = #table({}, {}) in Source"}}]
      },
      { "name": "Region", "columns": [{"name": "Id", "dataType": "int64", "sourceColumn": "Id"}],
        "partitions": [{"name": "Region", "source": {"type": "m", "expression": "x"}}] },
      { "name": "Segments", "columns": [{"name": "Segment", "dataType": "string", "sourceColumn": "Segment"}],
        "partitions": [{"name": "Segments", "source": {"type": "m", "expression": "x"}}] },
      { "name": "Calendar", "columns": [{"name": "Date", "dataType": "dateTime", "sourceColumn": "Date"}],
        "partitions": [{"name": "Calendar", "source": {"type": "m", "expression": "x"}}] },
      { "name": "LocalDateTable_1", "isHidden": true,
        "columns": [{"name": "Date", "dataType": "dateTime", "type": "calculatedTableColumn", "sourceColumn": "[Date]",
                     "isHidden": true}],
        "partitions": [{"name": "LocalDateTable_1", "source": {"type": "calculated",
                        "expression": "CALENDAR(DATE(2020, 1, 1), DATE(2020, 12, 31))"}}] },
      { "name": "Pivot",
        "columns": [
          {"name": "Jan", "dataType": "int64", "sourceColumn": "Jan"},
          {"name": "Feb", "dataType": "int64", "sourceColumn": "Feb"},
          {"name": "Mar", "dataType": "int64", "sourceColumn": "Mar"},
          {"name": "Apr", "dataType": "int64", "sourceColumn": "Apr"},
          {"name": "May", "dataType": "int64", "sourceColumn": "May"},
          {"name": "Jun", "dataType": "int64", "sourceColumn": "Jun"}
        ],
        "partitions": [{"name": "Pivot", "source": {"type": "m", "expression": "x"}}] },
      { "name": "isolated", "description": "no relationships",
        "columns": [{"name": "A", "dataType": "string", "sourceColumn": "A", "description": "a"}],
        "hierarchies": [{"name": "h", "levels": [{"name": "A", "ordinal": 0, "column": "A"}]}],
        "partitions": [{"name": "isolated", "source": {"type": "m", "expression": "x"}}] },
      { "name": "Legacy", "description": "legacy",
        "columns": [{"name": "X", "dataType": "string", "sourceColumn": "X", "description": "x"}],
        "partitions": [{"name": "Legacy", "source": {"type": "query", "query": "SELECT 1", "dataSource": "Structured DS"}}] },
      { "name": "Time Calc", "description": "empty group",
        "calculationGroup": {"calculationItems": []},
        "columns": [{"name": "Name", "dataType": "string", "sourceColumn": "Name", "description": "n"}],
        "partitions": [{"name": "Time Calc", "source": {"type": "calculationGroup"}}] },
      { "name": "CG2", "description": "group",
        "calculationGroup": {"calculationItems": [
          {"name": "YTD", "expression": "TOTALYTD(SELECTEDMEASURE(), 'Calendar'[Date])"}]},
        "columns": [{"name": "Name", "dataType": "string", "sourceColumn": "Name", "description": "n"}],
        "partitions": [{"name": "CG2", "source": {"type": "calculationGroup"}}] }
    ],
    "relationships": [
      {"name": "r1", "fromTable": "Fact", "fromColumn": "CustKey", "toTable": "Customer", "toColumn": "Id",
       "crossFilteringBehavior": "bothDirections",
       "annotations": [{"name": "Vertipaq_RIViolationInvalidRows", "value": "3"}]},
      {"name": "r2", "fromTable": "Customer", "fromColumn": "RegionKey", "toTable": "Region", "toColumn": "Id"},
      {"name": "r3", "fromTable": "Customer", "fromColumn": "Segment", "toTable": "Segments", "toColumn": "Segment",
       "fromCardinality": "many", "toCardinality": "many", "crossFilteringBehavior": "bothDirections"},
      {"name": "r4", "fromTable": "Fact", "fromColumn": "Order Date", "toTable": "Calendar", "toColumn": "Date",
       "isActive": false},
      {"name": "r5", "fromTable": "Fact", "fromColumn": "Comment", "toTable": "Region", "toColumn": "Id",
       "isActive": false},
      {"name": "r6", "fromTable": "Pivot", "fromColumn": "Jan", "toTable": "Calendar", "toColumn": "Date",
       "isActive": false},
      {"name": "r7", "fromTable": "Legacy", "fromColumn": "X", "toTable": "Customer", "toColumn": "Name",
       "isActive": false},
      {"name": "r8", "fromTable": "Time Calc", "fromColumn": "Name", "toTable": "Customer", "toColumn": "Name",
       "isActive": false},
      {"name": "r9", "fromTable": "CG2", "fromColumn": "Name", "toTable": "Customer", "toColumn": "Name",
       "isActive": false}
    ],
    "roles": [
      {"name": "Dyn", "modelPermission": "read", "tablePermissions": [
        {"name": "Customer", "filterExpression": "UPPER(Customer[Email]) = UPPER(USERPRINCIPALNAME())"},
        {"name": "Calendar", "filterExpression": "TRUE()"}]},
      {"name": "No Members", "modelPermission": "read"}
    ]
  }
}"##;

#[test]
fn default_rules_parse() {
    let set = default_rules();
    assert!(set.invalid.is_empty(), "{:?}", set.invalid);
    assert_eq!(set.rules.len(), 71);
    for r in &set.rules {
        crate::linq::parse(&r.expression).unwrap_or_else(|e| panic!("{}: {e}", r.id));
        assert_eq!(r.source, crate::DEFAULT_SOURCE);
    }
}

#[test]
fn clean_model_passes_every_default_rule() {
    let a = run(CLEAN);
    assert_eq!(a.rules.len(), 71);
    assert_all_evaluated(&a);
    let hits: Vec<String> = a
        .findings
        .iter()
        .map(|f| format!("{} on {}", f.rule_id, f.object))
        .collect();
    assert!(hits.is_empty(), "unexpected findings:\n{}", hits.join("\n"));
}

#[test]
fn bad_model_breaks_every_default_rule() {
    let a = run(BAD);
    assert_all_evaluated(&a);
    let fired: BTreeSet<&str> = a.findings.iter().map(|f| f.rule_id.as_str()).collect();
    let missing: Vec<&str> = a
        .rules
        .iter()
        .map(|r| r.rule_id.as_str())
        .filter(|id| !fired.contains(id))
        .collect();
    assert!(missing.is_empty(), "rules that did not fire: {missing:?}");
    let has = |rule: &str, object: &str| a.findings.iter().any(|f| f.rule_id == rule && f.object == object);
    // A sample of exact objects, where the rule's logic is not obvious.
    assert!(has("DAX_COLUMNS_FULLY_QUALIFIED", "[Unqualified]"));
    assert!(has("DAX_COLUMNS_FULLY_QUALIFIED", "KPI of [Total Price]"));
    assert!(!has("DAX_COLUMNS_FULLY_QUALIFIED", "[Total Price]"));
    assert!(has("DAX_MEASURES_UNQUALIFIED", "[Qualified Measure]"));
    assert!(!has("DAX_MEASURES_UNQUALIFIED", "[Ref]"));
    assert!(has("AVOID_DUPLICATE_MEASURES", "[Dup A]"));
    assert!(has("AVOID_DUPLICATE_MEASURES", "[Total Price]"));
    assert!(has(
        "MEASURES_SHOULD_NOT_BE_DIRECT_REFERENCES_OF_OTHER_MEASURES",
        "[Ref]"
    ));
    assert!(has(
        "INACTIVE_RELATIONSHIPS_THAT_ARE_NEVER_ACTIVATED",
        "'Fact'[Comment] -> 'Region'[Id]"
    ));
    assert!(!has(
        "INACTIVE_RELATIONSHIPS_THAT_ARE_NEVER_ACTIVATED",
        "'Fact'[Order Date] -> 'Calendar'[Date]"
    ));
    assert!(has("SNOWFLAKE_SCHEMA_ARCHITECTURE", "'Customer'"));
    assert!(!has("SNOWFLAKE_SCHEMA_ARCHITECTURE", "'Fact'"));
    assert!(has("HIDE_FOREIGN_KEYS", "'Fact'[CustKey]"));
    assert!(has("MARK_PRIMARY_KEYS", "'Customer'[Id]"));
    assert!(has("UNNECESSARY_COLUMNS", "'Fact'[Hidden Unused]"));
    assert!(!has("UNNECESSARY_COLUMNS", "'Fact'[Sorted]"));
    assert!(has(
        "REMOVE_REDUNDANT_COLUMNS_IN_RELATED_TABLES",
        "'Fact'[Country]"
    ));
    assert!(has("AVOID_INVALID_NAME_CHARACTERS", "'Fact'[Bad\u{1}Name]"));
    assert!(!has("AVOID_INVALID_NAME_CHARACTERS", "'Fact'[Tab\tName]"));
    assert!(has("SPECIAL_CHARS_IN_OBJECT_NAMES", "'Fact'[Tab\tName]"));
    assert!(has("HIDE_FACT_TABLE_COLUMNS", "'Fact'[Price]"));
    assert!(has(
        "AVOID_THE_USERELATIONSHIP_FUNCTION_AND_RLS_AGAINST_THE_SAME_TABLE",
        "'Calendar'"
    ));
    assert!(!has(
        "AVOID_THE_USERELATIONSHIP_FUNCTION_AND_RLS_AGAINST_THE_SAME_TABLE",
        "'Customer'"
    ));
    assert!(has(
        "AVOID_USING_MANY-TO-MANY_RELATIONSHIPS_ON_TABLES_USED_FOR_DYNAMIC_ROW_LEVEL_SECURITY",
        "'Customer'"
    ));
    assert!(has("PERSPECTIVES_WITH_NO_OBJECTS", "perspective Empty P"));
    assert!(has(
        "REMOVE_DATA_SOURCES_NOT_REFERENCED_BY_ANY_PARTITIONS",
        "data source Unused DS"
    ));
    assert!(!has(
        "REMOVE_DATA_SOURCES_NOT_REFERENCED_BY_ANY_PARTITIONS",
        "data source Structured DS"
    ));
}

#[test]
fn severity_and_id_filters() {
    let set = default_rules();
    let m = model(BAD);
    let only = analyze(
        &m,
        &set.rules,
        &Options {
            rule_ids: Some(vec!["avoid_floating_point_data_types".into()]),
            ..Options::default()
        },
    );
    assert_eq!(only.rules.len(), 1);
    assert!(
        only.findings
            .iter()
            .all(|f| f.rule_id == "AVOID_FLOATING_POINT_DATA_TYPES")
    );
    let errors = analyze(
        &m,
        &set.rules,
        &Options {
            severity_min: 3,
            ..Options::default()
        },
    );
    assert!(errors.rules.iter().all(|r| r.severity == "error"));
    assert!(errors.findings.iter().all(|f| f.severity_level >= 3));
    let capped = analyze(
        &m,
        &set.rules,
        &Options {
            max_findings: 5,
            ..Options::default()
        },
    );
    assert_eq!(capped.findings.len(), 5);
    assert!(capped.findings_omitted > 0);
}
