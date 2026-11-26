//! The rule-file reader and the expression interpreter, on rules written for these tests.

use crate::eval::{Interp, V};
use crate::objects::{ModelView, Obj};
use crate::{Options, analyze, linq, parse_rules, severity_from_text};

const MODEL: &str = r##"{
  "name": "M", "compatibilityLevel": 1500,
  "model": {
    "perspectives": [{"name": "Sales view", "tables": [{"name": "Sales", "measures": [{"name": "Total"}]}]}],
    "tables": [
      { "name": "Sales", "annotations": [{"name": "Rows", "value": "42"}],
        "columns": [
          {"name": "Amount", "dataType": "double", "sourceColumn": "Amount", "isHidden": true},
          {"name": "Key", "dataType": "int64", "sourceColumn": "Key", "summarizeBy": "none"},
          {"name": "Month Name", "dataType": "string", "sourceColumn": "M", "sortByColumn": "Month No"},
          {"name": "Month No", "dataType": "int64", "sourceColumn": "N"}
        ],
        "measures": [
          {"name": "Total", "expression": "SUM(Sales[Amount])", "formatString": "#,0"},
          {"name": "Twice", "expression": "VAR t = [Total] RETURN t * 2 + 'Sales'[Total]"},
          {"name": "Rows", "expression": "COUNTROWS(Sales)", "description": "row\u0001count"}
        ],
        "partitions": [{"name": "Sales", "source": {"type": "m", "expression": "let S = 1 in S"}}] },
      { "name": "Customer",
        "columns": [{"name": "Key", "dataType": "int64", "sourceColumn": "Key", "isKey": true}],
        "partitions": [{"name": "Customer", "source": {"type": "m", "expression": "x"}}] }
    ],
    "relationships": [
      {"name": "r1", "fromTable": "Sales", "fromColumn": "Key", "toTable": "Customer", "toColumn": "Key",
       "crossFilteringBehavior": "bothDirections"}
    ],
    "roles": [{"name": "R", "tablePermissions": [{"name": "Customer", "filterExpression": "Customer[Key] = 1",
               "metadataPermission": "none"}],
               "members": [{"memberName": "a@b.c"}]}]
  }
}"##;

fn model() -> bicraft_model::SemanticModel {
    bicraft_tmsl::read(MODEL).unwrap_or_else(|e| panic!("{e}"))
}

/// Evaluates `expr` on `o`.
fn eval(expr: &str, o: Obj) -> Result<V, String> {
    let m = model();
    let view = ModelView::new(&m);
    let mut i = Interp::new(&view);
    let ex = linq::parse(expr)?;
    i.eval_on(&ex, o)
}

fn is_true(expr: &str, o: Obj) -> bool {
    match eval(expr, o) {
        Ok(V::Bool(b)) => b,
        other => panic!("{expr}: {other:?}"),
    }
}

const AMOUNT: Obj = Obj::Column(0, 0);
const KEY: Obj = Obj::Column(0, 1);
const MONTH_NAME: Obj = Obj::Column(0, 2);
const TOTAL: Obj = Obj::Measure(0, 0);
const TWICE: Obj = Obj::Measure(0, 1);
const ROWS: Obj = Obj::Measure(0, 2);
const SALES: Obj = Obj::Table(0);
const CUSTOMER: Obj = Obj::Table(1);
const REL: Obj = Obj::Relationship(0);

#[test]
fn operators_literals_and_keywords() {
    for e in [
        "1 = 1",
        "1 == 1 and 2 <> 3",
        "true && !false",
        "not (1 > 2) or false",
        "2 * 3 - 1 = 5",
        "7 % 4 = 3 and 7 mod 4 = 3",
        "10 / 4 = 2.5",
        "-1 < 0",
        "\"a\" + 1 = \"a1\"",
        "\"a\" & \"b\" == \"ab\"",
        "'it''s' == \"it's\"",
        "\"say \"\"hi\"\"\".Length = 8",
        "(1 > 0 ? \"y\" : \"n\") == \"y\"",
        "iif(1 > 2, 1, 2) = 2",
        "null == null and (null ?? 3) = 3",
        "1L + 2.5m = 3.5",
    ] {
        assert!(is_true(e, AMOUNT), "{e}");
    }
}

#[test]
fn properties_enums_and_strings() {
    assert!(is_true(
        "DataType = \"Double\" and DataType == DataType.Double",
        AMOUNT
    ));
    assert!(is_true("DataType.ToString() = \"Double\"", AMOUNT));
    assert!(is_true(
        "IsHidden and not IsVisible and Table.Name = \"Sales\"",
        AMOUNT
    ));
    assert!(is_true(
        "SummarizeBy = \"Default\" and SummarizeBy <> AggregateFunction.None",
        AMOUNT
    ));
    assert!(is_true("SummarizeBy == AggregateFunction.None", KEY));
    assert!(is_true(
        "Name.ToUpper().Contains(\"MONTH\") and Name.StartsWith(\"Mon\") and Name.EndsWith(\"Name\")",
        MONTH_NAME
    ));
    assert!(is_true(
        "Name.IndexOf(\"name\", \"OrdinalIgnoreCase\") = 6 and Name.IndexOf(\"name\") = -1",
        MONTH_NAME
    ));
    assert!(is_true(
        "Name.Contains(\"name\", StringComparison.OrdinalIgnoreCase)",
        MONTH_NAME
    ));
    assert!(is_true(
        "Name.Substring(0,1) = \"M\" and Name.Replace(\" \", \"\") = \"MonthName\"",
        MONTH_NAME
    ));
    assert!(is_true(
        "\"  x \".Trim() = \"x\" and \"a,b\".Split(\",\").Count() = 2",
        MONTH_NAME
    ));
    assert!(is_true(
        "SortByColumn.Name = \"Month No\" and SortByColumn != null",
        MONTH_NAME
    ));
    assert!(is_true("SortByColumn == null", KEY));
    assert!(is_true(
        "string.IsNullOrWhitespace(Description) and String.IsNullOrEmpty(FormatString)",
        AMOUNT
    ));
    assert!(is_true(
        "ObjectTypeName = \"Data Column\" and DaxObjectFullName = \"'Sales'[Amount]\"",
        AMOUNT
    ));
    assert!(is_true(
        "DaxObjectName = \"[Total]\" and FormatString = \"#,0\"",
        TOTAL
    ));
    assert!(is_true("IsAvailableInMDX and IsKey == false", KEY));
    assert!(is_true("Columns.Any(IsKey == true)", CUSTOMER));
    assert!(is_true(
        "Convert.ToInt64(GetAnnotation(\"Rows\")) = 42 and Convert.ToInt64(GetAnnotation(\"Nope\")) = 0",
        SALES
    ));
    assert!(is_true(
        "HasAnnotation(\"Rows\") and GetAnnotation(\"Nope\") == null",
        SALES
    ));
    assert!(is_true(
        "Math.Max(Convert.ToDecimal(Columns.Count), 1) = 4",
        SALES
    ));
    assert!(is_true(
        "Name.IndexOf(char(32)) = 5 and Name.IndexOf(char(9)) = -1",
        MONTH_NAME
    ));
    assert!(is_true(
        "Description.ToCharArray().Any(char.IsControl(it) and !char.IsWhiteSpace(it))",
        ROWS
    ));
}

#[test]
fn collections_lambdas_and_scopes() {
    assert!(is_true("Columns.Where(DataType = \"Int64\").Count() = 2", SALES));
    assert!(is_true("Columns.Count(c => c.DataType = \"Int64\") = 2", SALES));
    assert!(is_true("Columns.All(Name.Length > 2)", SALES));
    assert!(is_true("Columns.Select(Name).Contains(\"Key\")", SALES));
    assert!(is_true(
        "Columns.First(IsHidden).Name = \"Amount\" and Columns.FirstOrDefault(Name = \"x\") == null",
        SALES
    ));
    assert!(is_true(
        "Columns[1].Name = \"Key\" and Columns[\"key\"].DataType = \"Int64\"",
        SALES
    ));
    assert!(is_true(
        "Partitions.Count = 1 and Partitions[0].Name = Name",
        SALES
    ));
    assert!(is_true(
        "Partitions[0].SourceType.ToString() = \"M\" and Partitions[0].Query.Contains(\"let\")",
        SALES
    ));
    // `outerIt` is the object before the lambda, `current` the rule's object.
    assert!(is_true(
        "Model.AllMeasures.Any(Name = \"Twice\" and it <> outerIt)",
        TOTAL
    ));
    assert!(is_true(
        "Model.Tables.Any(Columns.Any(Name == current.Name and Table.Name != current.Table.Name))",
        KEY
    ));
    assert!(is_true(
        "UsedInRelationships.Any(CrossFilteringBehavior == CrossFilteringBehavior.BothDirections and FromColumn.Name == current.Name)",
        KEY
    ));
    assert!(is_true(
        "UsedInSortBy.Any() and not UsedInHierarchies.Any() and not UsedInVariations.Any()",
        Obj::Column(0, 3)
    ));
    assert!(is_true(
        "FromCardinality == \"Many\" and ToCardinality.ToString() == \"One\" and FromColumn.DataType == ToColumn.DataType",
        REL
    ));
    assert!(is_true(
        "Model.Relationships.Where(CrossFilteringBehavior == CrossFilteringBehavior.BothDirections).Count() / Math.Max(Convert.ToDecimal(Model.Relationships.Count), 1) > 0.3",
        SALES
    ));
    assert!(is_true(
        "RowLevelSecurity.Any(it.Length > 0) and ObjectLevelSecurity.Any(it.ToString() == \"None\")",
        CUSTOMER
    ));
    assert!(is_true(
        "not RowLevelSecurity.Any(it <> null) and ObjectLevelSecurity.All(it.ToString() == \"Default\")",
        SALES
    ));
    assert!(is_true(
        "Model.Roles.Any(Members.Count() = 1 and RowLevelSecurity.Any(it.IndexOf(\"customer[key]\", \"OrdinalIgnoreCase\") >= 0))",
        SALES
    ));
    assert!(is_true("InPerspective[\"Sales view\"]", SALES));
    assert!(is_true("not InPerspective[\"Sales view\"]", CUSTOMER));
    assert!(is_true("InPerspective[\"Sales view\"]", TOTAL));
    assert!(is_true("not InPerspective[\"Sales view\"]", TWICE));
}

#[test]
fn dependencies_and_references() {
    // ReferencedBy enumerates referencing objects; DependsOn its Key/Value entries.
    assert!(is_true(
        "ReferencedBy.Count = 1 and ReferencedBy.AllMeasures.Count() = 2",
        AMOUNT
    ));
    assert!(is_true("ReferencedBy.AllMeasures.Any(Name = \"Twice\")", AMOUNT));
    assert!(is_true("ReferencedBy.Count = 0", TWICE));
    assert!(is_true(
        "DependsOn.Any(Key.ObjectType = \"Column\" and Value.All(FullyQualified))",
        TOTAL
    ));
    assert!(is_true(
        "DependsOn.Any(Key.ObjectType = \"Measure\" and Value.Any(FullyQualified))",
        TWICE
    ));
    assert!(is_true(
        "DependsOn.Any(Key.ObjectType = \"Measure\" and Value.Any(not FullyQualified))",
        TWICE
    ));
    assert!(is_true(
        "DependsOn.Count() = 1 and DependsOn.First().Value.Count() = 2",
        TWICE
    ));
    assert!(is_true("DependsOn.Any(Key.ObjectType = \"Table\")", ROWS));
    assert!(
        is_true("not DependsOn.Any(Key.Name = \"t\")", TWICE),
        "VAR names are not tables"
    );
}

#[test]
fn regular_expressions() {
    assert!(is_true(r#"RegEx.IsMatch(Expression, "(?i)sum\s*\(")"#, TOTAL));
    assert!(!is_true(r#"RegEx.IsMatch(Expression, "sum\s*\(")"#, TOTAL));
    assert!(is_true(
        r#"RegEx.IsMatch(Expression, "sum", RegexOptions.IgnoreCase)"#,
        TOTAL
    ));
    assert!(is_true(
        r#"RegEx.IsMatch(Expression,"\)\s*\/(?!\/)(?!\*)") == false"#,
        TOTAL
    ));
    assert!(is_true(
        r#"RegEx.IsMatch(Expression,"[0-9]+\s*[-+]\s*(?i)DIVIDE\s*\(") == false"#,
        TOTAL
    ));
    assert!(is_true(
        r#"RegEx.IsMatch(Expression,"(?i)SUM\s*\(\s*\'*" + Table.Name + "\'*\[")"#,
        TOTAL
    ));
    // A pattern whose case matters before `(?i)` is refused, not guessed.
    let e = eval(r#"RegEx.IsMatch(Expression, "S(?i)UM")"#, TOTAL);
    assert!(e.is_err(), "{e:?}");
    // Unsupported syntax (lookbehind) is an error.
    assert!(eval(r#"RegEx.IsMatch(Expression, "(?<=a)b")"#, TOTAL).is_err());
}

#[test]
fn failures_are_errors_not_false() {
    for e in [
        "NoSuchProperty = 1",
        "Name = 1",
        "Name.Frobnicate()",
        "SortByColumn.Name = \"x\"",
        "1 / 0 = 1",
        "Columns.Any(1)",
        "outerIt = 1",
        "Name",
        "DataType > 2",
    ] {
        let o = if e.contains("Columns") { SALES } else { KEY };
        let r = eval(e, o);
        assert!(r.is_err() || !matches!(r, Ok(V::Bool(_))), "{e}: {r:?}");
    }
}

#[test]
fn parse_errors_and_bounds() {
    for e in [
        "", "(", "1 +", "a.", "\"open", "iif(1,2)", "new X()", "a ? b", "#",
    ] {
        assert!(linq::parse(e).is_err(), "{e}");
    }
    // Deep nesting and long operator chains are refused instead of overflowing the stack.
    assert!(linq::parse(&format!("{}1{}", "(".repeat(10_000), ")".repeat(10_000))).is_err());
    assert!(linq::parse(&"!".repeat(10_000)).is_err());
    assert!(linq::parse(&vec!["a"; 5_000].join(" or ")).is_err());
    assert!(linq::parse(&vec!["a"; 5_000].join(".")).is_err());
    assert!(linq::parse(&vec!["1"; 50].join(" or ")).is_ok());
    assert!(linq::parse(&"1+".repeat(30_000)).is_err());
}

#[test]
fn rule_file_reading_and_outcomes() {
    let file = r#"[
      {"ID": "T_FLOAT", "Name": "Float", "Category": "Perf", "Severity": 2,
       "Scope": "DataColumn, CalculatedColumn", "Expression": "DataType = \"Double\"",
       "FixExpression": "DataType = DataType.Decimal", "CompatibilityLevel": 1200},
      {"ID": "T_PARTIAL", "Severity": 1, "Scope": "DataColumn", "Expression": "SortByColumn.Name = \"Month No\""},
      {"ID": "T_BAD_SYNTAX", "Severity": 3, "Scope": "Measure", "Expression": "Name ="},
      {"ID": "T_SCOPE", "Severity": 1, "Scope": "Variation", "Expression": "true"},
      {"ID": "T_MIXED_SCOPE", "Severity": 1, "Scope": "Model, Variation", "Expression": "true"},
      {"ID": "T_FUTURE", "Severity": 1, "Scope": "Model", "Expression": "true", "CompatibilityLevel": 1600},
      {"id": "t_lower_keys", "severity": "warning", "scope": "Measure", "expression": "IsHidden"},
      {"Name": "no id"},
      42
    ]"#;
    let set = parse_rules(file, "mine.json").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(set.rules.len(), 7);
    assert_eq!(set.invalid.len(), 2);
    assert_eq!(
        set.rules[0].fix_expression.as_deref(),
        Some("DataType = DataType.Decimal")
    );
    assert_eq!(set.rules[6].severity, 2);
    let m = model();
    let a = analyze(&m, &set.rules, &Options::default());
    let status = |id: &str| {
        a.rules
            .iter()
            .find(|r| r.rule_id == id)
            .map(|r| (r.status, r.reason.clone().unwrap_or_default()))
            .unwrap_or_else(|| panic!("{id}"))
    };
    assert_eq!(status("T_FLOAT").0, "evaluated");
    assert!(
        a.findings
            .iter()
            .any(|f| f.rule_id == "T_FLOAT" && f.object == "'Sales'[Amount]")
    );
    let (s, why) = status("T_PARTIAL");
    assert_eq!(s, "partial", "{why}");
    assert!(why.contains("null"), "{why}");
    assert!(
        a.findings
            .iter()
            .any(|f| f.rule_id == "T_PARTIAL" && f.object == "'Sales'[Month Name]")
    );
    assert_eq!(status("T_BAD_SYNTAX").0, "not_evaluated");
    assert_eq!(status("T_SCOPE").0, "not_evaluated");
    assert_eq!(status("T_MIXED_SCOPE").0, "partial");
    assert_eq!(status("T_FUTURE").0, "skipped");
    assert_eq!(status("t_lower_keys").0, "evaluated");
    assert!(a.rules.iter().all(|r| r.source == "mine.json"));

    assert!(parse_rules("{}", "x").is_err());
    assert!(parse_rules("not json", "x").is_err());
    assert!(parse_rules("\u{feff}{\"rules\": []}", "x").is_ok_and(|s| s.rules.is_empty()));
    assert_eq!(severity_from_text("High"), Some(3));
    assert_eq!(severity_from_text("9"), None);
}

#[test]
fn step_budget_stops_runaway_rules() {
    // Nested quantifiers over every column, many times: bounded by the step budget.
    let mut tables = Vec::new();
    for t in 0..20 {
        let cols: Vec<String> = (0..20)
            .map(|c| format!(r#"{{"name": "C{c}", "dataType": "int64", "sourceColumn": "C{c}"}}"#))
            .collect();
        tables.push(format!(r#"{{"name": "T{t}", "columns": [{}]}}"#, cols.join(",")));
    }
    let tmsl = format!(
        r#"{{"name": "Big", "model": {{"tables": [{}]}}}}"#,
        tables.join(",")
    );
    let m = bicraft_tmsl::read(&tmsl).unwrap_or_else(|e| panic!("{e}"));
    let rule = r#"[{"ID": "HEAVY", "Severity": 1, "Scope": "DataColumn",
       "Expression": "Model.AllColumns.Any(Model.AllColumns.Any(Model.AllColumns.Any(Name = \"none\")))"}]"#;
    let set = parse_rules(rule, "heavy").unwrap_or_else(|e| panic!("{e}"));
    let a = analyze(&m, &set.rules, &Options::default());
    let r = &a.rules[0];
    assert_eq!(r.status, "not_evaluated", "{:?}", r.reason);
    assert!(r.reason.as_deref().is_some_and(|w| w.contains("steps")));
}
