//! `Tokenize()`, string indexing and `char` helpers, `outerIt` in nested lambdas, cultures and
//! translation indexers, and inferred measure data types; on a model and rules written for
//! these tests.

use crate::eval::{Interp, V};
use crate::objects::{ModelView, Obj};
use crate::{Options, analyze, linq, parse_rules};

const MODEL: &str = r##"{
  "name": "M", "compatibilityLevel": 1550,
  "model": {
    "culture": "en-US",
    "queryGroups": [{"folder": "Staging"}],
    "cultures": [
      {"name": "en-US"},
      {"name": "fr-FR", "translations": {"model": {"name": "Model", "translatedCaption": "Modèle",
        "tables": [{"name": "Sales", "translatedCaption": "Ventes", "translatedDescription": "Lignes",
          "columns": [{"name": "Qty", "translatedCaption": "Qté"}],
          "measures": [{"name": "Total Qty", "translatedCaption": "Qté totale", "translatedDisplayFolder": "Dossier"}],
          "hierarchies": [{"name": "H", "translatedCaption": "Hiér",
            "levels": [{"name": "L1", "translatedCaption": "N1"}]}]}],
        "perspectives": [{"name": "P", "translatedCaption": "Vue"}]}}}
    ],
    "perspectives": [{"name": "P", "tables": [{"name": "Sales", "includeAll": true}]}],
    "tables": [
      { "name": "Sales", "description": "Sales lines",
        "columns": [
          {"name": "Amount", "dataType": "decimal", "sourceColumn": "Amount"},
          {"name": "Qty", "dataType": "int64", "sourceColumn": "Qty"},
          {"name": "Price", "dataType": "double", "sourceColumn": "Price"},
          {"name": "Label", "dataType": "string", "sourceColumn": "Label"},
          {"name": "Date", "dataType": "dateTime", "sourceColumn": "Date"},
          {"name": "lower name", "dataType": "string", "sourceColumn": "L"},
          {"name": "étape", "dataType": "string", "sourceColumn": "E", "isHidden": true},
          {"type": "calculated", "name": "Unit", "dataType": "double", "expression": "Sales[Amount] / Sales[Qty]"},
          {"type": "calculated", "name": "Half", "dataType": "double", "expression": "Sales[Qty] / 2"},
          {"type": "calculated", "name": "Part", "dataType": "double", "expression": "[Price] / 2.5"}
        ],
        "hierarchies": [{"name": "H", "levels": [
          {"name": "L1", "ordinal": 0, "column": "Label"}, {"name": "L2", "ordinal": 1, "column": "Qty"}]}],
        "measures": [
          {"name": "Total Qty", "expression": "SUM(Sales[Qty])", "displayFolder": "Base"},
          {"name": "Total Amount", "expression": "SUM ( Sales[Amount] )"},
          {"name": "Avg Price", "expression": "AVERAGE(Sales[Price])"},
          {"name": "Ratio", "expression": "DIVIDE([Total Qty], 3)"},
          {"name": "Per Unit", "expression": "[Total Amount] / [Total Qty]"},
          {"name": "Text", "expression": "FORMAT([Total Qty], \"0\")"},
          {"name": "Last Date", "expression": "MAX(Sales[Date])"},
          {"name": "Flag", "expression": "[Total Qty] > 1"},
          {"name": "Pick", "expression": "IF([Flag], 1, 2)"},
          {"name": "Mixed", "expression": "IF([Flag], 1, \"x\")"},
          {"name": "Plus One", "expression": "[Total Qty] + 1"},
          {"name": "Twice", "expression": "VAR t = [Total Qty] RETURN t * 2 // comment"}
        ],
        "partitions": [{"name": "Sales", "source": {"type": "m", "expression": "x"}}] }
    ]
  }
}"##;

fn model() -> bicraft_model::SemanticModel {
    bicraft_tmsl::read(MODEL).unwrap_or_else(|e| panic!("{e}"))
}

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

fn is_err(expr: &str, o: Obj) -> String {
    match eval(expr, o) {
        Err(e) => e,
        other => panic!("{expr}: expected an error, got {other:?}"),
    }
}

const SALES: Obj = Obj::Table(0);
const QTY: Obj = Obj::Column(0, 1);
const LOWER: Obj = Obj::Column(0, 5);
const ETAPE: Obj = Obj::Column(0, 6);
const UNIT: Obj = Obj::Column(0, 7);
const HALF: Obj = Obj::Column(0, 8);
const PART: Obj = Obj::Column(0, 9);
const TOTAL_QTY: Obj = Obj::Measure(0, 0);
const PER_UNIT: Obj = Obj::Measure(0, 4);
const TEXT: Obj = Obj::Measure(0, 5);
const TWICE: Obj = Obj::Measure(0, 11);

fn measure(name: &str) -> Obj {
    let m = model();
    let mi = m
        .tables
        .first()
        .and_then(|t| t.measures.iter().position(|x| x.name == name))
        .unwrap_or_else(|| panic!("{name}"));
    Obj::Measure(0, mi)
}

#[test]
fn tokenize_types_text_and_navigation() {
    // `[Total Amount] / [Total Qty]`: measure references around a division.
    assert!(is_true("Tokenize().Count() = 3", PER_UNIT));
    assert!(is_true(
        "Tokenize()[1].Type = DIV and Tokenize()[1].Next.Type = MEASURE and Tokenize()[1].Previous.Text = \"[Total Amount]\"",
        PER_UNIT
    ));
    // The ambiguous names match the resolved types.
    assert!(is_true("Tokenize()[0].Type = COLUMN_OR_MEASURE", PER_UNIT));
    assert!(is_true(
        "Tokenize().First().Previous = null and Tokenize().Last().Next = null",
        PER_UNIT
    ));
    // Qualified columns, integer and real literals.
    assert!(is_true(
        "Tokenize().Where(Type = DIV).All(Next.Type = TABLE and Next.Next.Type = COLUMN)",
        UNIT
    ));
    assert!(is_true(
        "Tokenize().Any(Type = DIV and Next.Type = INTEGER_LITERAL)",
        HALF
    ));
    assert!(is_true(
        "Tokenize().Any(t => t.Type = DIV && t.Next.Type == REAL_LITERAL && t.Previous.Type == COLUMN)",
        PART
    ));
    // Functions, parentheses, strings, variables, keywords; comments are not tokens.
    assert!(is_true(
        "Tokenize()[0].Type = FUNCTION and Tokenize()[0].Text = \"FORMAT\" and Tokenize()[1].Type = OPEN_PARENS \
         and Tokenize().Any(Type = STRING_LITERAL and Text = \"\"\"0\"\"\") and Tokenize().Last().Type = CLOSE_PARENS",
        TEXT
    ));
    assert!(is_true(
        "Tokenize().Select(Type.ToString()).ToList().Count = 8 and Tokenize()[0].Type = VAR \
         and Tokenize()[1].Type = VARIABLE and Tokenize()[2].Type = EQUALS and Tokenize()[4].Type = RETURN \
         and Tokenize()[5].Type = TABLE_OR_VARIABLE and Tokenize()[6].Type = MULT and Tokenize()[6].Index = 6",
        TWICE
    ));
    assert!(is_true(
        "Tokenize().Any(Type = FUNCTION and Text = \"SUM\")",
        TOTAL_QTY
    ));
    // A data column has no DAX: no tokens.
    assert!(is_true("not Tokenize().Any()", QTY));
    // Objects without DAX at all, and unknown token properties, are errors.
    is_err("Tokenize().Any()", Obj::Model);
    is_err("Tokenize()[0].Colour = 1", PER_UNIT);
}

#[test]
fn string_indexing_and_char_helpers() {
    assert!(is_true("char.IsLower(Name[0])", LOWER));
    assert!(is_true(
        "not char.IsLower(Name[0]) and char.IsUpper(Name[0])",
        QTY
    ));
    assert!(is_true("char.IsLower(Name[0]) and Name[0] = \"\u{e9}\"", ETAPE));
    assert!(is_true(
        "char.ToUpper(Name[0]) = \"L\" and char.ToLower(\"Q\") = \"q\"",
        LOWER
    ));
    assert!(is_true(
        "char.IsWhiteSpace(Name[5]) and char.IsLetter(Name[0]) and char.IsDigit(\"7\"[0]) and char.IsControl(char(9))",
        LOWER
    ));
    // Unicode scalar values, not UTF-16 units.
    assert!(is_true("\"\u{1F600}x\"[1] = \"x\"", QTY));
    // Out of range or a bad index: an error on that object, never `false`.
    assert!(is_err("char.IsLower(Name[3])", QTY).contains("outside the string"));
    is_err("Name[-1] = \"Q\"", QTY);
    is_err("Name[0.5] = \"Q\"", QTY);
    is_err("Name[\"a\"] = \"Q\"", QTY);
}

#[test]
fn outer_it_and_translations() {
    // Depth 1: `outerIt` is the object the rule runs on.
    assert!(is_true(
        "Model.Cultures.Where(c => c.Name <> Model.Culture).Any(outerIt.TranslatedNames[it] = \"Qté\")",
        QTY
    ));
    assert!(is_true(
        "Model.Cultures.Any(Name != Model.Culture and string.IsNullOrEmpty(outerIt.TranslatedNames[it]))",
        Obj::Column(0, 2)
    ));
    // Depth 2: the `it` of the enclosing lambda (Dynamic LINQ's `outerIt`); `current` stays the
    // rule's object.
    assert!(is_true(
        "Model.Tables.Any(Columns.Any(outerIt.Name = \"Sales\" and current.Name = \"Qty\"))",
        QTY
    ));
    // Top level: no enclosing lambda.
    is_err("outerIt.Name = \"Qty\"", QTY);
    // Indexers by culture object or name, enumeration over the model's cultures.
    assert!(is_true(
        "TranslatedNames[\"fr-FR\"] = \"Qté totale\" and TranslatedNames[\"en-US\"] = \"\" \
         and TranslatedDisplayFolders[\"fr-FR\"] = \"Dossier\" and TranslatedDescriptions[\"fr-FR\"] = \"\" \
         and TranslatedNames.Count = 2 and TranslatedNames.Any(it = \"\")",
        TOTAL_QTY
    ));
    assert!(is_true(
        "TranslatedNames[Model.Cultures[1]] = \"Ventes\" and TranslatedDescriptions[\"FR-fr\"] = \"Lignes\"",
        SALES
    ));
    assert!(is_true("TranslatedNames[\"fr-FR\"] = \"Modèle\"", Obj::Model));
    assert!(is_true(
        "TranslatedNames[\"fr-FR\"] = \"Hiér\" and Levels[0].TranslatedNames[\"fr-FR\"] = \"N1\" \
         and Levels[1].TranslatedNames[\"fr-FR\"] = \"\"",
        Obj::Hierarchy(0, 0)
    ));
    assert!(is_true(
        "TranslatedNames[\"fr-FR\"] = \"Vue\"",
        Obj::Perspective(0)
    ));
    assert!(is_true(
        "Model.Cultures.Count = 2 and Model.Cultures[\"fr-FR\"].Name = \"fr-FR\"",
        QTY
    ));
    is_err("TranslatedNames[\"de-DE\"] = \"\"", QTY);
    is_err("TranslatedDisplayFolders[\"fr-FR\"] = \"\"", SALES);
    is_err("TranslatedNames[\"fr-FR\"] = \"\"", Obj::Partition(0, 0));
}

#[test]
fn misplaced_collection_calls_and_json_arrays_are_errors() {
    // A predicate on a non-collection is reported as such (not as a misplaced `outerIt`).
    assert!(is_err("Name.Any(outerIt.Name = \"x\")", QTY).contains("needs a collection"));
    // A JSON array of objects is not read as text.
    assert!(is_err("Model.QueryGroups = \"\"", QTY).contains("collection"));
}

#[test]
fn measure_data_types_are_inferred() {
    for (name, ty) in [
        ("Total Qty", "Int64"),
        ("Total Amount", "Decimal"),
        ("Avg Price", "Double"),
        ("Ratio", "Double"),
        ("Text", "String"),
        ("Last Date", "DateTime"),
        ("Flag", "Boolean"),
        ("Pick", "Int64"),
        ("Mixed", "Variant"),
        ("Plus One", "Int64"),
    ] {
        let r = eval("DataType", measure(name));
        assert_eq!(r, Ok(V::Enum(ty.into())), "{name}");
        assert!(is_true(&format!("DataType = \"{ty}\""), measure(name)), "{name}");
    }
}

/// Our own rule file, one rule per construct.
const RULES: &str = r#"[
  {"ID": "DIV_BY_REFERENCE", "Name": "Division by a reference", "Severity": 3,
   "Scope": "Measure, CalculatedColumn, CalculatedTable",
   "Expression": "Tokenize().Any(Type = DIV and Next.Type != INTEGER_LITERAL and Next.Type != REAL_LITERAL)"},
  {"ID": "LOWER_FIRST", "Name": "Names start upper case", "Severity": 1,
   "Scope": "DataColumn, CalculatedColumn, Measure", "Expression": "IsVisible and char.IsLower(Name[0])"},
  {"ID": "UNTRANSLATED", "Name": "Visible names are translated", "Severity": 1,
   "Scope": "Table, Measure, DataColumn, Hierarchy, Level",
   "Expression": "Model.Cultures.Any(Name <> Model.Culture and outerIt.TranslatedNames[it] = \"\")"},
  {"ID": "NUMERIC_NO_FORMAT", "Name": "Numeric measures have a format", "Severity": 2, "Scope": "Measure",
   "Expression": "string.IsNullOrEmpty(FormatString) and (DataType = \"Int64\" or DataType = \"Double\" or DataType = \"Decimal\")"},
  {"ID": "CULTURES", "Name": "Culture names", "Severity": 1, "Scope": "Culture", "Expression": "Name.Contains(\"-\") = false"}
]"#;

#[test]
fn own_rule_file_is_fully_evaluated() {
    let m = model();
    let set = parse_rules(RULES, "constructs.json").unwrap_or_else(|e| panic!("{e}"));
    let a = analyze(&m, &set.rules, &Options::default());
    let count = |id: &str| {
        let r = a
            .rules
            .iter()
            .find(|r| r.rule_id == id)
            .unwrap_or_else(|| panic!("{id}"));
        assert_eq!(r.status, "evaluated", "{id}: {:?}", r.reason);
        r.findings
    };
    // Unit (two column references) and Per Unit (two measures).
    assert_eq!(count("DIV_BY_REFERENCE"), 2);
    // `lower name` (étape is hidden).
    assert_eq!(count("LOWER_FIRST"), 1);
    // Translated: table Sales, column Qty, measure Total Qty, hierarchy H, level L1.
    // 1 table + 12 measures + 7 data columns + 1 hierarchy + 2 levels = 23 objects, 5 translated.
    assert_eq!(count("UNTRANSLATED"), 18);
    // Every numeric measure (no format strings in this model): 12 minus Text, Last Date, Flag
    // and Mixed.
    assert_eq!(count("NUMERIC_NO_FORMAT"), 8);
    assert_eq!(count("CULTURES"), 0);
}

/// Measure data types as the Analysis Services engine of Power BI Desktop 2.158.1304.0 reported
/// them (`TMSCHEMA_MEASURES.DataType`) for these measures, added to table `Sales` of the DAX
/// oracle model (`crates/dax/tests/oracle/model.bim`).
const ENGINE_TYPES: &[(&str, &str)] = &[
    (r#"SUM(Sales[Quantity])"#, "Int64"),
    (r#"SUM(Sales[UnitPrice])"#, "Decimal"),
    (r#"SUM(Sales[Discount])"#, "Double"),
    (r#"AVERAGE(Sales[Quantity])"#, "Double"),
    (r#"AVERAGE(Sales[UnitPrice])"#, "Decimal"),
    (r#"MIN(Customer[Name])"#, "String"),
    (r#"MAX(Sales[OrderDate])"#, "DateTime"),
    (r#"COUNTROWS(Sales)"#, "Int64"),
    (r#"DISTINCTCOUNT(Sales[CustomerKey])"#, "Int64"),
    (r#"DIVIDE([Total Quantity], 2)"#, "Double"),
    (r#"DIVIDE([Total Sales], [Order Count])"#, "Decimal"),
    (r#"[Total Sales] / [Order Count]"#, "Decimal"),
    (r#"[Total Quantity] / 2"#, "Double"),
    (r#"[Total Sales]"#, "Decimal"),
    (r#"[Order Count] + 1"#, "Int64"),
    (r#"[Order Count] * 1.5"#, "Double"),
    (r#"[Total Sales] * 2"#, "Decimal"),
    (r#"[Total Sales] + SUM(Sales[Discount])"#, "Double"),
    (r#"IF([Order Count] > 1, 1, 2)"#, "Int64"),
    (r#"IF([Order Count] > 1, 1, 2.5)"#, "Double"),
    (r#"IF([Order Count] > 1, 1, "x")"#, "Variant"),
    (r#"IF([Order Count] > 1, [Total Quantity])"#, "Int64"),
    (r#"IF([Order Count] > 1, [Total Sales], 0)"#, "Decimal"),
    (r#"IF([Order Count] > 1, BLANK(), [Total Sales])"#, "Decimal"),
    (
        r#"SWITCH(TRUE(), [Order Count] > 1, 1, [Order Count] > 2, 2, 3)"#,
        "Int64",
    ),
    (r#"SWITCH([Order Count], 1, "a", 2, "b", "c")"#, "String"),
    (r#"SWITCH([Order Count], 1, 1, 2, 2.5, 3)"#, "Double"),
    (r#"FORMAT([Total Sales], "0.00")"#, "String"),
    (r#""n=" & [Order Count]"#, "String"),
    (r#"[Total Sales] > 100"#, "Boolean"),
    (r#"TODAY()"#, "DateTime"),
    (r#"CALCULATE([Total Sales], Sales[Channel] = "Web")"#, "Decimal"),
    (r#"SUMX(Sales, Sales[Quantity] * Sales[UnitPrice])"#, "Decimal"),
    (r#"SUMX(Sales, Sales[Quantity] * 2)"#, "Int64"),
    (r#"SELECTEDVALUE(Customer[Name])"#, "String"),
    (r#"ROUND([Total Sales], 0)"#, "Decimal"),
    (r#"INT([Total Sales])"#, "Decimal"),
    (r#"VALUE("12")"#, "Double"),
    (r#"BLANK()"#, "Int64"),
    (r#"VAR x = [Total Quantity] RETURN x * 2"#, "Int64"),
    (
        r#"VAR x = [Total Quantity] RETURN DIVIDE(x, [Order Count])"#,
        "Double",
    ),
    (r#"[Sales LY]"#, "Decimal"),
    (r#"DIVIDE([Total Sales] - [Sales LY], [Sales LY])"#, "Double"),
    (r#"COUNT(Customer[Name])"#, "Int64"),
    (r#"MAX([Order Count], 2.5)"#, "Double"),
    (r#"COALESCE([Total Quantity], 0)"#, "Int64"),
    (r#"[Total Quantity] ^ 2"#, "Double"),
    (r#"MAX(Sales[OrderDate]) - MIN(Sales[OrderDate])"#, "DateTime"),
    (r#"MAX(Sales[OrderDate]) + 1"#, "DateTime"),
    (r#"TRUE()"#, "Boolean"),
    (r#"AND([Order Count] > 1, [Total Quantity] > 2)"#, "Boolean"),
    (r#"ISBLANK([Total Sales])"#, "Boolean"),
    (r#"-[Total Sales]"#, "Decimal"),
    (r#"ABS([Total Quantity])"#, "Int64"),
    (r#"RANKX(ALL(Customer), [Total Sales])"#, "Int64"),
    (r#"AVERAGEX(Customer, [Total Sales])"#, "Decimal"),
    (r#"MAXX(Customer, [Order Count])"#, "Int64"),
    (r#"CURRENCY(1)"#, "Decimal"),
    (r#"CONVERT([Total Quantity], DOUBLE)"#, "Double"),
    (
        r#"DATEDIFF(MIN(Sales[OrderDate]), MAX(Sales[OrderDate]), DAY)"#,
        "Int64",
    ),
    (r#"YEAR(MAX(Sales[OrderDate]))"#, "Int64"),
    (r#"LEFT(SELECTEDVALUE(Customer[Name]), 2)"#, "String"),
    (r#"CONCATENATEX(Customer, Customer[Name], ",")"#, "String"),
    (r#"SUMX(Sales, IF(Sales[IsReturn], 1, 0))"#, "Int64"),
    (r#"IF([Order Count] > 1, SUM(Sales[Discount]), 0)"#, "Double"),
    (
        r#"IF([Order Count] > 1, [Total Sales], SUM(Sales[Discount]))"#,
        "Double",
    ),
    (r#"IF([Order Count] > 1, MAX(Sales[OrderDate]))"#, "DateTime"),
    (r#"IF([Order Count] > 1, TRUE(), FALSE())"#, "Boolean"),
];

#[test]
fn measure_data_types_match_the_engine() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../dax/tests/oracle/model.bim");
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut m = bicraft_tmsl::read(&text).unwrap_or_else(|e| panic!("{e}"));
    let ti = m
        .tables
        .iter()
        .position(|t| t.name == "Sales")
        .unwrap_or_else(|| panic!("Sales"));
    let first = m.tables[ti].measures.len();
    for (k, (expr, _)) in ENGINE_TYPES.iter().enumerate() {
        let mut x = m.tables[ti].measures[0].clone();
        x.name = format!("Probe {k}");
        x.expression = (*expr).to_string();
        m.tables[ti].measures.push(x);
    }
    let view = ModelView::new(&m);
    let mut interp = Interp::new(&view);
    let ex = linq::parse("DataType").unwrap_or_else(|e| panic!("{e}"));
    let mut wrong = Vec::new();
    for (k, (expr, want)) in ENGINE_TYPES.iter().enumerate() {
        let got = interp.eval_on(&ex, Obj::Measure(ti, first + k));
        if got != Ok(V::Enum((*want).into())) {
            wrong.push(format!("{expr}: engine {want}, bicraft {got:?}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {} differ:
{}",
        wrong.len(),
        ENGINE_TYPES.len(),
        wrong.join(
            "
"
        )
    );
}
