//! Tests of `project.test` (`citest*.rs`): every assertion and rule kind passing and failing, the
//! four output formats (JUnit structure, SARIF 2.1.0 structure), determinism, server mode, and
//! hostile test files. Fixtures are generated here (a small model with DATATABLE data and a
//! report built with bicraft's own commands).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use crate::{ServerConfig, Session};

/// The fixture model: Sales with data (DATATABLE), three measures, and Raw, an import table
/// whose M source cannot be read.
const MODEL: &str = r##"{
 "name": "CiShop",
 "compatibilityLevel": 1567,
 "model": {
  "culture": "en-US",
  "tables": [
   {
    "name": "Sales",
    "description": "Sales lines",
    "columns": [
     {"type": "calculatedTableColumn", "name": "Category", "dataType": "string", "sourceColumn": "[Category]"},
     {"type": "calculatedTableColumn", "name": "Amount", "dataType": "double", "sourceColumn": "[Amount]"},
     {"type": "calculatedTableColumn", "name": "Day", "dataType": "dateTime", "sourceColumn": "[Day]"}
    ],
    "partitions": [
     {"name": "Sales", "source": {"type": "calculated", "expression": "DATATABLE(\"Category\", STRING, \"Amount\", DOUBLE, \"Day\", DATETIME, {{\"Toys\", 10, \"2024-01-01\"}, {\"Toys\", 15.5, \"2024-01-02\"}, {\"Wheels\", 100, \"2024-01-01\"}})"}}
    ],
    "measures": [
     {"name": "Total Sales", "expression": "SUM(Sales[Amount])", "formatString": "#,0.00", "description": "Sum of the amounts"},
     {"name": "Lines", "expression": "COUNTROWS(Sales)"},
     {"name": "safe ratio", "expression": "IFERROR(DIVIDE([Total Sales], [Lines]), 0)"}
    ]
   },
   {
    "name": "Raw",
    "columns": [{"name": "X", "dataType": "string", "sourceColumn": "X"}],
    "partitions": [
     {"name": "Raw", "mode": "import", "source": {"type": "m", "expression": "let\n    Source = Csv.Document(File.Contents(\"missing.csv\"))\nin\n    Source"}}
    ]
   }
  ]
 }
}"##;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// Builds `<dir>/Shop.pbip`: the fixture model and a report with a card, a chart, a hidden page
/// and a button navigating to it. `broken`: a visual also binds a column that does not exist.
fn project(dir: &Path, broken: bool) -> PathBuf {
    let bim = dir.join("src.bim");
    std::fs::write(&bim, MODEL).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": bim}));
    run(&mut s, "report.new", json!({}));
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let card = run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "card", "x": 10, "y": 10, "width": 200, "height": 100,
               "fields": [{"role": "Values", "field": "[Total Sales]"}]}),
    );
    run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "clusteredColumnChart", "x": 220, "y": 10, "width": 400,
               "height": 300, "fields": [{"role": "Category", "field": "Sales[Category]"},
                                         {"role": "Y", "field": "[Total Sales]"}]}),
    );
    if broken {
        run(
            &mut s,
            "visual.add",
            json!({"page": page, "visual_type": "tableEx", "x": 10, "y": 320, "width": 300, "height": 200,
                   "fields": [{"role": "Values", "field": "Sales[Nope]"}], "force": true}),
        );
    }
    let hidden = run(&mut s, "page.add", json!({"display_name": "Details"}));
    let hidden = hidden["page"].as_str().unwrap().to_string();
    let button = run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "actionButton", "x": 700, "y": 10, "width": 100, "height": 40}),
    );
    {
        let r = s.report.as_mut().unwrap();
        let p = r.pages.iter_mut().find(|p| p.name == hidden).unwrap();
        p.visibility = Some("HiddenInViewMode".into());
        let first = &mut r.pages[0];
        let lit = |v: &str| json!({"expr": {"Literal": {"Value": v}}});
        for v in &mut first.visuals {
            if v.name == button["visual"].as_str().unwrap() {
                v.visual_extra.insert(
                    "visualContainerObjects".into(),
                    json!({"visualLink": [{"properties": {"show": lit("true"), "type": lit("'PageNavigation'"),
                                                           "navigationSection": lit(&format!("'{hidden}'"))}}]}),
                );
            }
            if v.name == card["visual"].as_str().unwrap() {
                v.visual_extra.insert(
                    "visualContainerObjects".into(),
                    json!({"general": [{"properties": {"altText": lit("'Total sales'")}}]}),
                );
                v.visual_extra.insert(
                    "objects".into(),
                    json!({"labels": [{"properties": {"color": {"solid": {"color": lit("'#FF0000'")}}}}]}),
                );
            }
        }
        s.report_dirty = true;
    }
    run(&mut s, "project.new", json!({"dir": dir, "name": "Shop"}));
    dir.join("Shop.pbip")
}

/// Runs `project.test` on `pbip` with `yaml` as its test file next to it.
fn test_yaml(pbip: &Path, yaml: &str, extra: Value) -> Value {
    std::fs::write(pbip.parent().unwrap().join("bicraft.tests.yaml"), yaml).unwrap();
    let mut p = json!({"path": pbip});
    if let (Some(o), Some(e)) = (p.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    run(&mut Session::new(), "project.test", p)
}

fn case<'a>(v: &'a Value, id: &str) -> &'a Value {
    v["suites"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["tests"].as_array().unwrap())
        .find(|t| t["id"] == id)
        .unwrap_or_else(|| panic!("no test {id} in {v:#}"))
}

fn status(v: &Value, id: &str) -> String {
    case(v, id)["status"].as_str().unwrap().to_string()
}

/// One fixture project for a test.
fn fixture(name: &str, broken: bool) -> (TempDir, PathBuf) {
    let dir = TempDir::new(&format!("engine-citest-{name}"));
    let pbip = project(dir.path(), broken);
    (dir, pbip)
}

// ---- YAML reader ------------------------------------------------------------------------------

#[test]
fn yaml_scalars_and_structure() {
    let v = crate::citest_yaml::parse(
        "a: 1\nb: -2.5\nc: true\nd: ~\ne: '1'\nf: \"x\"\ng: [1, two, {h: null}]\ni: |\n  line 1\n  line 2\nj: 1e3\nk: 0x10\n",
    )
    .unwrap();
    assert_eq!(
        v,
        json!({"a": 1, "b": -2.5, "c": true, "d": null, "e": "1", "f": "x",
               "g": [1, "two", {"h": null}], "i": "line 1\nline 2\n", "j": 1000.0, "k": "0x10"})
    );
    assert_eq!(crate::citest_yaml::parse("").unwrap(), Value::Null);
    assert_eq!(
        crate::citest_yaml::parse("\u{feff}x: !!str 12").unwrap(),
        json!({"x": "12"})
    );
}

#[test]
fn yaml_refuses_aliases_duplicates_tags_and_extra_documents() {
    for (text, want) in [
        ("a: &x [1, 2]\nb: *x\n", "anchors and aliases"),
        ("a: 1\na: 2\n", "duplicate key `a`"),
        ("a: !custom 1\n", "unsupported tag"),
        ("a: 1\n---\nb: 2\n", "one YAML document only"),
        ("a: [1, 2\n", "line"),
        ("? [1]\n: x\n", "mapping key must be a scalar"),
    ] {
        let e = crate::citest_yaml::parse(text).unwrap_err();
        assert!(e.contains(want), "{text:?}: {e}");
    }
}

#[test]
fn yaml_limits_depth_nodes_and_size() {
    let deep = format!("{}1{}", "[".repeat(200), "]".repeat(200));
    assert!(crate::citest_yaml::parse(&deep).is_err());
    let mut block = String::new();
    for i in 0..100 {
        block.push_str(&" ".repeat(i * 2));
        block.push_str("- \n");
    }
    let e = crate::citest_yaml::parse(&block).unwrap_err();
    assert!(e.contains("nested deeper"), "{e}");
    let many = "- 1\n".repeat(crate::citest_yaml::MAX_NODES + 1);
    assert!(
        crate::citest_yaml::parse(&many)
            .unwrap_err()
            .contains("more than")
    );
    let big = "x".repeat(crate::citest_yaml::MAX_TEXT_BYTES + 1);
    assert!(
        crate::citest_yaml::parse(&big)
            .unwrap_err()
            .contains("larger than")
    );
}

/// Hostile text never crashes the reader or the schema check (a deterministic pseudo-random
/// sweep of edits to a real test file and of random text).
#[test]
fn yaml_never_crashes_on_hostile_text() {
    let seed_text = "version: 1\ndax:\n  - name: x\n    measure: '[Total Sales]'\n    expect: 25.5\n    filters: [\"Sales[Category] = \\\"Toys\\\"\"]\nmodel: {bpa: {include: ['*']}}\n";
    let alphabet: &[u8] = b"-:[]{}&*!|>'\"#%@`,?\n\t ab1.\\~";
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for round in 0..3000 {
        let mut bytes: Vec<u8> = if round % 2 == 0 {
            seed_text.as_bytes().to_vec()
        } else {
            Vec::new()
        };
        let edits = (next() % 24 + 1) as usize;
        for _ in 0..edits {
            let c = alphabet[(next() % alphabet.len() as u64) as usize];
            let at = if bytes.is_empty() {
                0
            } else {
                (next() % bytes.len() as u64) as usize
            };
            match next() % 3 {
                0 => bytes.insert(at, c),
                1 if !bytes.is_empty() => {
                    bytes[at] = c;
                }
                _ => bytes.truncate(at),
            }
        }
        let text = String::from_utf8_lossy(&bytes);
        if let Ok(doc) = crate::citest_yaml::parse(&text) {
            let _ = crate::citest_spec::parse(&doc);
        }
    }
}

// ---- test-file schema -------------------------------------------------------------------------

#[test]
fn spec_errors_name_their_place() {
    let p = |v: Value| crate::citest_spec::parse(&v).unwrap_err();
    assert!(p(json!({})).contains("version: required"));
    assert!(p(json!({"version": 2})).contains("not 2"));
    assert!(
        p(json!({"version": 1, "dax": [{"measure": "x", "expectt": 1}]}))
            .contains("dax[0]: unknown key `expectt`")
    );
    assert!(p(json!({"version": 1, "dax": [{"name": "a"}]})).contains("needs one of"));
    assert!(
        p(json!({"version": 1, "dax": [{"measure": "x", "query": "EVALUATE {1}"}]})).contains("only one")
    );
    assert!(p(json!({"version": 1, "dax": [{"measure": "x"}, {"measure": "x"}]})).contains("duplicate id"));
    assert!(p(json!({"version": 1, "dax": [{"measure": "x", "type": "money"}]})).contains("dax[0].type"));
    assert!(p(json!({"version": 1, "dax": [{"measure": "x", "tolerance": -1}]})).contains("negative"));
    assert!(
        p(json!({"version": 1, "model": {"rules": [{"kind": "naming", "pattern": "(?<=a)"}]}}))
            .contains("model.rules[0].pattern")
    );
    assert!(
        p(json!({"version": 1, "model": {"rules": [{"kind": "required", "property": "colour"}]}}))
            .contains("model.rules[0].property")
    );
    assert!(p(json!({"version": 1, "model": {"rules": [{"kind": "required", "property": "format_string", "scope": "table"}]}}))
        .contains("not a scope"));
    assert!(
        p(json!({"version": 1, "report": {"rules": [{"kind": "page_size", "width": 1}]}})).contains("both")
    );
    assert!(
        p(json!({"version": 1, "report": {"validate": {"include": ["nope"]}}})).contains("unknown category")
    );
    assert!(p(json!({"version": 1, "report": {"rules": [{"kind": "colours"}]}})).contains("unknown kind"));
    assert!(
        p(json!({"version": 1, "usage": {"no_unused_measures": 3}})).contains("usage.no_unused_measures")
    );
    assert!(p(json!([1])).contains("the test file: expected a mapping"));
}

#[test]
fn glob_matches_case_insensitively() {
    use crate::citest_rules::glob;
    assert!(glob("DAX_*", "dax_columns_fully_qualified"));
    assert!(glob("*", ""));
    assert!(glob("a?c", "ABC"));
    assert!(!glob("a*b", "acd"));
    assert!(glob("*x*y*", "aaxbbycc"));
    assert!(!glob("x", "xx"));
}

// ---- DAX assertions ---------------------------------------------------------------------------

const DAX_TESTS: &str = r#"version: 1
name: CI gates
dax:
  - name: Total sales
    measure: Total Sales
    expect: 125.5
    type: number
  - name: Toys only
    measure: "[Total Sales]"
    filters:
      - Sales[Category] = "Toys"
    expect: 25.5
  - name: Toys wrong
    measure: Total Sales
    filters: ['Sales[Category] = "Toys"']
    expect: 30
  - name: Within tolerance
    expression: SUM(Sales[Amount]) + 0.004
    expect: 125.5
    tolerance: 0.01
  - name: Bounds
    expression: COUNTROWS(Sales)
    min: 1
    max: 2
  - name: Typed text is not a number
    expression: FORMAT(3, "0")
    expect: 3
  - name: Blank
    expression: CALCULATE([Total Sales], Sales[Category] = "None")
    expect: null
    type: blank
  - name: Integer type
    expression: COUNTROWS(Sales)
    type: text
  - name: By category
    query: |
      EVALUATE SUMMARIZECOLUMNS(Sales[Category], "Total", [Total Sales])
      ORDER BY Sales[Category]
    expect:
      columns: [Category, Total]
      row_count: 2
      rows:
        - [Toys, 25.5]
        - {Category: Wheels, Total: 100}
  - name: Unordered rows
    query: EVALUATE SUMMARIZECOLUMNS(Sales[Category], "Total", [Total Sales]) ORDER BY Sales[Category] DESC
    ordered: false
    expect:
      rows: [[Toys, 25.5], [Wheels, 100]]
  - name: Ordered rows differ
    query: EVALUATE SUMMARIZECOLUMNS(Sales[Category], "Total", [Total Sales]) ORDER BY Sales[Category] DESC
    expect:
      rows: [[Toys, 25.5], [Wheels, 100]]
  - name: Dates
    query: EVALUATE ROW("d", MIN(Sales[Day]))
    expect: "2024-01-01"
    type: date
  - name: Unknown measure
    measure: Nope
    expect: 1
  - name: No data
    expression: COUNTROWS(Raw)
    expect: 0
  - name: Data required
    expression: COUNTROWS(Raw)
    require_data: true
    severity: warning
"#;

#[test]
fn dax_assertions_pass_fail_skip_and_error() {
    let (_dir, pbip) = fixture("dax", false);
    let v = test_yaml(&pbip, DAX_TESTS, json!({}));
    for (id, want) in [
        ("dax/total-sales", "passed"),
        ("dax/toys-only", "passed"),
        ("dax/toys-wrong", "failed"),
        ("dax/within-tolerance", "passed"),
        ("dax/bounds", "failed"),
        ("dax/typed-text-is-not-a-number", "failed"),
        ("dax/blank", "passed"),
        ("dax/integer-type", "failed"),
        ("dax/by-category", "passed"),
        ("dax/unordered-rows", "passed"),
        ("dax/ordered-rows-differ", "failed"),
        ("dax/dates", "passed"),
        ("dax/unknown-measure", "error"),
        ("dax/no-data", "skipped"),
        ("dax/data-required", "error"),
    ] {
        assert_eq!(status(&v, id), want, "{id}: {:#}", case(&v, id));
    }
    let wrong = case(&v, "dax/toys-wrong");
    assert_eq!(wrong["message"], "expected 30, got 25.5 (number)");
    assert_eq!(wrong["details"]["actual"], 25.5);
    assert_eq!(wrong["details"]["data_source"], "partitions");
    // located on the test's line in the test file
    assert_eq!(wrong["findings"][0]["location"]["path"], "bicraft.tests.yaml");
    assert_eq!(wrong["findings"][0]["location"]["line"], 13);
    assert!(
        case(&v, "dax/bounds")["message"]
            .as_str()
            .unwrap()
            .contains("at most 2, got 3")
    );
    assert!(
        case(&v, "dax/integer-type")["message"]
            .as_str()
            .unwrap()
            .contains("expected type text, got integer")
    );
    assert!(
        case(&v, "dax/no-data")["message"]
            .as_str()
            .unwrap()
            .contains("require_data")
    );
    // the warning-level error does not gate; the error-level failures do
    assert_eq!(case(&v, "dax/data-required")["gates"], false);
    assert_eq!(v["passed"], false);
    assert_eq!(v["exit_code"], 1);
    assert_eq!(v["summary"]["tests"], 15);
    assert_eq!(v["summary"]["passed"], 7);
    assert_eq!(v["summary"]["warnings"], 1);
}

#[test]
fn passing_assertions_pass_the_run() {
    let (_dir, pbip) = fixture("dax-pass", false);
    let v = test_yaml(
        &pbip,
        "version: 1\ndax:\n  - measure: Total Sales\n    expect: 125.5\n",
        json!({}),
    );
    assert_eq!(v["passed"], true, "{v:#}");
    assert_eq!(v["exit_code"], 0);
    assert_eq!(status(&v, "dax/total-sales"), "passed");
    assert_eq!(v["project"], "Shop.pbip");
    assert_eq!(v["tests_file"], "bicraft.tests.yaml");
}

// ---- model rules -------------------------------------------------------------------------------

const MODEL_TESTS: &str = r#"version: 1
model:
  validate: true
  bpa:
    include: [DAX_*, "AVOID_USING_THE_IFERROR_*"]
    exclude: [DAX_MEASURES_UNQUALIFIED]
    min_severity: info
  rules:
    - name: Measures start with a capital
      kind: naming
      scope: measure
      pattern: "^[A-Z]"
    - name: Tables have no spaces
      kind: naming
      scope: table
      forbid_pattern: " "
    - name: Measures are described
      kind: required
      property: description
    - name: Measures have a format
      kind: required
      property: format_string
      where: {name: "Total*"}
    - name: No IFERROR
      kind: forbidden_functions
      functions: [IFERROR, EARLIER]
    - name: No FILTER
      kind: forbidden_functions
      functions: [FILTER]
    - name: Short expressions
      kind: max_expression_length
      max: 30
      severity: warning
    - name: Very short
      kind: max_expression_length
      max: 1000
"#;

#[test]
fn model_rules_pass_and_fail() {
    let (_dir, pbip) = fixture("model", false);
    let v = test_yaml(&pbip, MODEL_TESTS, json!({}));
    for (id, want) in [
        ("model/validate-dax", "passed"),
        ("model/validate-relationships", "passed"),
        ("model/measures-start-with-a-capital", "failed"),
        ("model/tables-have-no-spaces", "passed"),
        ("model/measures-are-described", "failed"),
        ("model/measures-have-a-format", "passed"),
        ("model/no-iferror", "failed"),
        ("model/no-filter", "passed"),
        ("model/short-expressions", "failed"),
        ("model/very-short", "passed"),
    ] {
        assert_eq!(status(&v, id), want, "{id}: {:#}", case(&v, id));
    }
    let naming = case(&v, "model/measures-start-with-a-capital");
    assert_eq!(naming["findings"].as_array().unwrap().len(), 1);
    assert!(
        naming["message"]
            .as_str()
            .unwrap()
            .contains("'Sales'[safe ratio]")
    );
    // located in the TMDL table file, on the measure's line
    let loc = &naming["findings"][0]["location"];
    assert_eq!(loc["path"], "Shop.SemanticModel/definition/tables/Sales.tmdl");
    let tmdl = std::fs::read_to_string(
        pbip.parent()
            .unwrap()
            .join("Shop.SemanticModel/definition/tables/Sales.tmdl"),
    )
    .unwrap();
    let line = tmdl
        .lines()
        .nth(loc["line"].as_u64().unwrap() as usize - 1)
        .unwrap();
    assert!(line.contains("measure 'safe ratio'"), "{line}");
    let described = case(&v, "model/measures-are-described");
    assert_eq!(
        described["findings"].as_array().unwrap().len(),
        2,
        "{described:#}"
    );
    assert!(
        case(&v, "model/no-iferror")["message"]
            .as_str()
            .unwrap()
            .contains("uses IFERROR")
    );
    assert_eq!(case(&v, "model/short-expressions")["severity"], "warning");
    assert_eq!(case(&v, "model/short-expressions")["gates"], false);
    // BPA: only included rules, one test each; excluded ones are absent
    let bpa: Vec<&Value> = v["suites"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "bpa")
        .unwrap()["tests"]
        .as_array()
        .unwrap()
        .iter()
        .collect();
    assert!(!bpa.is_empty());
    assert!(bpa.iter().all(|t| {
        let id = t["id"].as_str().unwrap();
        (id.starts_with("bpa/DAX_") || id == "bpa/AVOID_USING_THE_IFERROR_FUNCTION")
            && id != "bpa/DAX_MEASURES_UNQUALIFIED"
    }));
    // the IFERROR measure breaks Microsoft's "avoid IFERROR" rule
    let iferror = case(&v, "bpa/AVOID_USING_THE_IFERROR_FUNCTION");
    assert_eq!(iferror["status"], "failed", "{iferror:#}");
    assert!(
        iferror["findings"][0]["message"]
            .as_str()
            .unwrap()
            .contains("safe ratio")
    );
}

#[test]
fn model_validation_fails_on_invalid_dax() {
    let dir = TempDir::new("engine-citest-invalid");
    let bim = dir.path().join("model.bim");
    std::fs::write(&bim, MODEL.replace("COUNTROWS(Sales)", "COUNTROWS(Nowhere)")).unwrap();
    std::fs::write(
        dir.path().join("bicraft.tests.json"),
        r#"{"version": 1, "model": {"validate": true}}"#,
    )
    .unwrap();
    let v = run(&mut Session::new(), "project.test", json!({"path": bim}));
    let c = case(&v, "model/validate-dax");
    assert_eq!(c["status"], "failed", "{c:#}");
    assert!(c["message"].as_str().unwrap().contains("'Sales'[Lines]"));
    // located in model.bim on the measure's name
    assert_eq!(c["findings"][0]["location"]["path"], "model.bim");
    let line = c["findings"][0]["location"]["line"].as_u64().unwrap() as usize;
    let text = std::fs::read_to_string(&bim).unwrap();
    assert!(text.lines().nth(line - 1).unwrap().contains("\"Lines\""));
    assert_eq!(v["passed"], false);
}

// ---- report rules ------------------------------------------------------------------------------

const REPORT_TESTS: &str = r#"version: 1
report:
  validate:
    exclude: [schema]
  rules:
    - kind: fields_exist
    - kind: bookmarks_valid
    - name: At most 2 visuals
      kind: max_visuals_per_page
      max: 2
    - name: At most 10 visuals
      kind: max_visuals_per_page
      max: 10
    - kind: alt_text
    - name: Alt text on cards
      kind: alt_text
      exclude_types: [clusteredColumnChart, actionButton, tableEx]
    - name: 16 by 9
      kind: page_size
      width: 1280
      height: 720
    - name: Full HD
      kind: page_size
      allowed: [{width: 1920, height: 1080}]
    - name: Corporate theme
      kind: theme
      custom_theme: Corporate
    - name: Theme colors only
      kind: theme
      forbid_hardcoded_colors: true
      severity: warning
    - kind: no_hidden_page_navigation
"#;

#[test]
fn report_rules_pass_and_fail() {
    let (_dir, pbip) = fixture("report", true);
    let v = test_yaml(&pbip, REPORT_TESTS, json!({}));
    for (id, want) in [
        ("report/validate-fields", "failed"),
        ("report/validate-bookmarks", "passed"),
        ("report/fields-exist", "failed"),
        ("report/bookmarks-valid", "passed"),
        ("report/at-most-2-visuals", "failed"),
        ("report/at-most-10-visuals", "passed"),
        ("report/alt-text", "failed"),
        ("report/alt-text-on-cards", "passed"),
        ("report/16-by-9", "passed"),
        ("report/full-hd", "failed"),
        ("report/corporate-theme", "failed"),
        ("report/theme-colors-only", "failed"),
        ("report/no-hidden-page-navigation", "failed"),
    ] {
        assert_eq!(status(&v, id), want, "{id}: {:#}", case(&v, id));
    }
    let fields = case(&v, "report/fields-exist");
    assert!(fields["message"].as_str().unwrap().contains("Nope"), "{fields:#}");
    // located in the visual's PBIR file
    let path = fields["findings"][0]["location"]["path"].as_str().unwrap();
    assert!(
        path.starts_with("Shop.Report/definition/pages/") && path.ends_with("/visual.json"),
        "{path}"
    );
    assert!(
        case(&v, "report/alt-text")["findings"][0]["message"]
            .as_str()
            .unwrap()
            .contains("no alt text")
    );
    assert!(
        case(&v, "report/no-hidden-page-navigation")["message"]
            .as_str()
            .unwrap()
            .contains("hidden page 'Details'")
    );
    assert!(
        case(&v, "report/theme-colors-only")["message"]
            .as_str()
            .unwrap()
            .contains("#FF0000")
    );
}

#[test]
fn report_rules_without_a_report_are_errors() {
    let dir = TempDir::new("engine-citest-noreport");
    let bim = dir.path().join("model.bim");
    std::fs::write(&bim, MODEL).unwrap();
    let v = run(
        &mut Session::new(),
        "project.test",
        json!({"path": bim, "tests": {"version": 1, "report": {"rules": [{"kind": "alt_text"}]}}}),
    );
    assert_eq!(status(&v, "report/report"), "error");
    assert_eq!(v["passed"], false);
    assert_eq!(v["tests_file"], Value::Null);
}

#[test]
fn usage_rules_run_model_unused_end_to_end() {
    let (_dir, pbip) = fixture("usage", false);
    let v = test_yaml(
        &pbip,
        "version: 1\nusage:\n  no_unused_measures: error\n",
        json!({}),
    );
    assert!(crate::find_command("model.unused").is_some());
    // the visuals show [Total Sales]; [safe ratio] is used nowhere and [Lines] only by it
    let c = case(&v, "usage/no-unused-measures");
    assert_eq!(c["status"], "failed", "{v:#}");
    let mut messages: Vec<&str> = c["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["message"].as_str().unwrap())
        .collect();
    messages.sort_unstable();
    assert_eq!(messages.len(), 2, "{messages:?}");
    assert!(
        messages[0].contains("[Lines]") && messages[0].contains("[safe ratio]"),
        "{messages:?}"
    );
    assert!(messages[1].contains("[safe ratio]") && messages[1].contains("referenced nowhere"));
    assert_eq!(v["passed"], false);

    // warning level: reported, the run still passes
    let v = test_yaml(
        &pbip,
        "version: 1\nusage:\n  no_unused_measures: warning\n",
        json!({}),
    );
    assert_eq!(
        case(&v, "usage/no-unused-measures")["findings"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(v["passed"], true, "{v:#}");
}

// ---- outputs ------------------------------------------------------------------------------------

/// Durations are the only varying part of a run's output.
fn without_times(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("time=\"") {
        out.push_str(&rest[..i + 6]);
        rest = &rest[i + 6..];
        let end = rest.find('"').unwrap();
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

fn strip_elapsed(v: &mut Value) {
    match v {
        Value::Object(o) => {
            o.remove("elapsed_ms");
            o.values_mut().for_each(strip_elapsed);
        }
        Value::Array(a) => a.iter_mut().for_each(strip_elapsed),
        _ => {}
    }
}

const ALL_TESTS: &str = r#"version: 1
name: "Shop <gates> & checks"
dax:
  - measure: Total Sales
    expect: 125.5
  - name: Wrong
    measure: Total Sales
    expect: 1
  - name: No data
    expression: COUNTROWS(Raw)
model:
  validate: false
  bpa: {include: [AVOID_USING_THE_IFERROR_FUNCTION]}
  rules:
    - name: Described
      kind: required
      property: description
      severity: warning
report:
  validate: false
  rules:
    - kind: fields_exist
"#;

#[test]
fn junit_output_has_the_junit_structure() {
    let (_dir, pbip) = fixture("junit", true);
    let v = test_yaml(&pbip, ALL_TESTS, json!({"format": "junit"}));
    let xml = v["output"].as_str().unwrap();
    let doc = roxmltree::Document::parse(xml).unwrap();
    let root = doc.root_element();
    assert_eq!(root.tag_name().name(), "testsuites");
    assert_eq!(root.attribute("name"), Some("Shop <gates> & checks"));
    let attr = |n: roxmltree::Node<'_, '_>, a: &str| n.attribute(a).unwrap().parse::<usize>().unwrap();
    let suites: Vec<_> = root.children().filter(|n| n.is_element()).collect();
    assert_eq!(
        suites
            .iter()
            .map(|s| s.attribute("name").unwrap())
            .collect::<Vec<_>>(),
        ["dax", "model", "bpa", "report"]
    );
    let (mut tests, mut failures, mut errors, mut skipped) = (0, 0, 0, 0);
    for s in &suites {
        assert_eq!(s.tag_name().name(), "testsuite");
        let cases: Vec<_> = s.children().filter(|n| n.is_element()).collect();
        assert_eq!(attr(*s, "tests"), cases.len());
        let count = |tag: &str| {
            cases
                .iter()
                .filter(|c| c.children().any(|x| x.tag_name().name() == tag))
                .count()
        };
        assert_eq!(attr(*s, "failures"), count("failure"));
        assert_eq!(attr(*s, "errors"), count("error"));
        assert_eq!(attr(*s, "skipped"), count("skipped"));
        for c in &cases {
            assert_eq!(c.tag_name().name(), "testcase");
            assert!(c.attribute("name").is_some() && c.attribute("classname").is_some());
            assert!(c.attribute("time").unwrap().parse::<f64>().is_ok());
            for x in c.children().filter(|n| n.is_element()) {
                assert!(["failure", "error", "skipped", "system-out"].contains(&x.tag_name().name()));
                if x.tag_name().name() != "system-out" {
                    assert!(x.attribute("message").is_some());
                }
            }
        }
        tests += cases.len();
        failures += attr(*s, "failures");
        errors += attr(*s, "errors");
        skipped += attr(*s, "skipped");
    }
    assert_eq!(attr(root, "tests"), tests);
    assert_eq!(attr(root, "failures"), failures);
    assert_eq!(attr(root, "errors"), errors);
    assert_eq!(attr(root, "skipped"), skipped);
    assert_eq!(failures, 2, "{xml}"); // Wrong, fields_exist (the BPA rule is a warning)
    assert_eq!(skipped, 1);
    // the warning-level rule passes with its findings in system-out
    assert!(xml.contains("WARNING (warning)"));
}

/// SARIF 2.1.0 checked structurally (the OASIS schema is not vendored, see docs/ci-tests.md):
/// required properties, levels, rule indexes, relative URIs.
#[test]
fn sarif_output_has_the_sarif_structure() {
    let (_dir, pbip) = fixture("sarif", true);
    let v = test_yaml(&pbip, ALL_TESTS, json!({"format": "sarif"}));
    let doc: Value = serde_json::from_str(v["output"].as_str().unwrap()).unwrap();
    assert_eq!(doc["version"], "2.1.0");
    assert_eq!(doc["$schema"], crate::citest_out::SARIF_SCHEMA);
    let runs = doc["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1);
    let driver = &runs[0]["tool"]["driver"];
    assert_eq!(driver["name"], "bicraft");
    let rules = driver["rules"].as_array().unwrap();
    let results = runs[0]["results"].as_array().unwrap();
    assert!(!results.is_empty());
    let mut ids = std::collections::HashSet::new();
    for r in rules {
        assert!(ids.insert(r["id"].as_str().unwrap()), "duplicate rule id");
        assert!(r["shortDescription"]["text"].is_string());
        assert!(["error", "warning", "note"].contains(&r["defaultConfiguration"]["level"].as_str().unwrap()));
    }
    for r in results {
        let i = r["ruleIndex"].as_u64().unwrap() as usize;
        assert_eq!(rules[i]["id"], r["ruleId"]);
        assert!(["error", "warning", "note", "none"].contains(&r["level"].as_str().unwrap()));
        assert!(!r["message"]["text"].as_str().unwrap().is_empty());
        for l in r["locations"].as_array().into_iter().flatten() {
            let pl = &l["physicalLocation"];
            let uri = pl["artifactLocation"]["uri"].as_str().unwrap();
            assert!(
                !uri.contains('\\') && !uri.starts_with('/') && !uri.contains(' '),
                "{uri}"
            );
            if let Some(line) = pl["region"]["startLine"].as_u64() {
                assert!(line >= 1);
            }
        }
    }
    let ids: Vec<&str> = results.iter().map(|r| r["ruleId"].as_str().unwrap()).collect();
    assert!(
        ids.contains(&"dax/wrong")
            && ids.contains(&"bpa/AVOID_USING_THE_IFERROR_FUNCTION")
            && ids.contains(&"report/fields-exist")
    );
    let dax = results.iter().find(|r| r["ruleId"] == "dax/wrong").unwrap();
    assert_eq!(
        dax["locations"][0]["physicalLocation"]["artifactLocation"]["uri"],
        "bicraft.tests.yaml"
    );
    assert_eq!(dax["locations"][0]["physicalLocation"]["region"]["startLine"], 6);
    let described = results.iter().find(|r| r["ruleId"] == "model/described").unwrap();
    assert_eq!(described["level"], "warning");
    assert!(
        described["locations"][0]["physicalLocation"]["artifactLocation"]["uri"]
            .as_str()
            .unwrap()
            .starts_with("Shop.SemanticModel/definition/tables/")
    );
}

#[test]
fn outputs_are_deterministic() {
    let (_dir, pbip) = fixture("determinism", true);
    let once = || {
        let mut v = test_yaml(
            &pbip,
            ALL_TESTS,
            json!({"format": ["json", "text", "junit", "sarif"]}),
        );
        let outputs = v["outputs"].clone();
        v.as_object_mut().unwrap().remove("outputs");
        strip_elapsed(&mut v);
        let mut all = vec![v.to_string()];
        for f in ["text", "junit", "sarif"] {
            all.push(without_times(outputs[f].as_str().unwrap()));
        }
        all
    };
    let first = once();
    assert_eq!(first, once());
    let text = &first[1];
    assert!(text.contains("FAIL  wrong  Wrong"), "{text}");
    assert!(text.contains("SKIP  no-data"), "{text}");
    assert!(text.contains("WARN  described"), "{text}");
    assert!(text.contains(": FAILED (fail on error)"), "{text}");
}

#[test]
fn fail_on_warning_gates_warnings_too() {
    let (_dir, pbip) = fixture("failon", false);
    let yaml = "version: 1\nmodel:\n  validate: false\n  rules:\n    - kind: required\n      property: description\n      severity: warning\n";
    assert_eq!(test_yaml(&pbip, yaml, json!({}))["passed"], true);
    let v = test_yaml(&pbip, yaml, json!({"fail_on": "warning"}));
    assert_eq!(v["passed"], false);
    assert_eq!(v["fail_on"], "warning");
}

// ---- the command --------------------------------------------------------------------------------

#[test]
fn the_open_session_is_left_alone() {
    let (_dir, pbip) = fixture("readonly", false);
    let other = TempDir::new("engine-citest-other");
    let bim = other.path().join("model.bim");
    std::fs::write(&bim, MODEL).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": bim}));
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Extra", "expression": "1"}),
    );
    let revision = s.revision;
    std::fs::write(
        pbip.parent().unwrap().join("bicraft.tests.yaml"),
        "version: 1\ndax:\n  - measure: Total Sales\n    expect: 125.5\n",
    )
    .unwrap();
    let v = run(&mut s, "project.test", json!({"path": pbip}));
    assert_eq!(v["passed"], true);
    assert_eq!(s.revision, revision);
    assert!(s.dirty);
    assert_eq!(
        crate::commands::source_path(s.source.as_ref().unwrap()),
        bim.as_path()
    );
    // without `path`: the open file, with its unsaved edits (inline tests)
    let v = run(
        &mut s,
        "project.test",
        json!({"tests": {"version": 1, "dax": [{"measure": "Extra", "expect": 1}]}}),
    );
    assert_eq!(v["passed"], true, "{v:#}");
    assert_eq!(s.revision, revision);
    assert!(crate::READ_ONLY_COMMANDS.contains(&"project.test"));
}

#[test]
fn configuration_errors_are_command_errors() {
    let (dir, pbip) = fixture("config", false);
    let mut s = Session::new();
    let e = s.execute("project.test", &json!({"path": pbip})).unwrap_err();
    assert!(e.contains("no test file"), "{e}");
    std::fs::write(
        dir.path().join("bicraft.tests.yaml"),
        "version: 1\ndax: [{measure: x, colour: red}]\n",
    )
    .unwrap();
    let e = s.execute("project.test", &json!({"path": pbip})).unwrap_err();
    assert!(
        e.contains("bicraft.tests.yaml: dax[0]: unknown key `colour`"),
        "{e}"
    );
    std::fs::write(dir.path().join("bicraft.tests.yaml"), "version: 1\ndax: [\n").unwrap();
    let e = s.execute("project.test", &json!({"path": pbip})).unwrap_err();
    assert!(e.contains("not valid YAML"), "{e}");
    std::fs::write(
        dir.path().join("bicraft.tests.yaml"),
        "version: 1\nmodel:\n  bpa: {rules: [missing.json]}\n",
    )
    .unwrap();
    let e = s.execute("project.test", &json!({"path": pbip})).unwrap_err();
    assert!(e.contains("model.bpa") && e.contains("missing.json"), "{e}");
    for bad in [
        json!({"format": "xml"}),
        json!({"fail_on": "info"}),
        json!({"tests": 3}),
        json!({"nope": 1}),
    ] {
        let mut p = bad.clone();
        p["path"] = json!(pbip);
        assert!(s.execute("project.test", &p).is_err(), "{bad}");
    }
    // a JSON test file, and BPA rules from a file next to it
    std::fs::remove_file(dir.path().join("bicraft.tests.yaml")).unwrap();
    std::fs::write(
        dir.path().join("rules.json"),
        r#"[{"ID": "NO_RATIO", "Name": "No ratio", "Category": "Naming", "Severity": 3, "Scope": "Measure", "Expression": "Name.Contains(\"ratio\")"}]"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("bicraft.tests.json"),
        r#"{"version": 1, "model": {"validate": false, "bpa": {"default_rules": false, "rules": ["rules.json"]}}}"#,
    )
    .unwrap();
    let v = run(&mut s, "project.test", json!({"path": pbip}));
    assert_eq!(status(&v, "bpa/NO_RATIO"), "failed");
    assert_eq!(v["tests_file"], "bicraft.tests.json");
}

#[test]
fn server_mode_resolves_paths_under_the_root() {
    let root = TempDir::new("engine-citest-server");
    let ws = root.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    project(&ws, false);
    std::fs::write(ws.join("rules.json"), "[]").unwrap();
    std::fs::write(
        ws.join("bicraft.tests.yaml"),
        "version: 1\ndax:\n  - measure: Total Sales\n    expect: 125.5\nmodel:\n  validate: false\n  bpa: {default_rules: false, rules: [rules.json]}\n",
    )
    .unwrap();
    let mut s = Session::server(ServerConfig::new(&ws).unwrap());
    let v = run(
        &mut s,
        "project.test",
        json!({"path": "Shop.pbip", "format": "sarif"}),
    );
    assert_eq!(v["passed"], true, "{v:#}");
    assert_eq!(v["project"], "Shop.pbip");
    // no server path in the answer
    let root_text = std::fs::canonicalize(&ws).unwrap().display().to_string();
    let text = v.to_string();
    for p in [ws.display().to_string(), root_text] {
        assert!(
            !text.contains(&p.replace('\\', "\\\\")) && !text.contains(&p.replace('\\', "/")),
            "{text}"
        );
    }
    for bad in ["../x.pbip", "/etc/x.pbip", "C:/x.pbip"] {
        let e = s.execute("project.test", &json!({"path": bad})).unwrap_err();
        assert!(e.contains("server mode"), "{bad}: {e}");
    }
    let e = s
        .execute(
            "project.test",
            &json!({"path": "Shop.pbip", "tests": "../t.yaml"}),
        )
        .unwrap_err();
    assert!(e.contains("server mode"), "{e}");
    std::fs::write(
        ws.join("bicraft.tests.yaml"),
        "version: 1\nmodel:\n  bpa: {rules: [../outside.json]}\n",
    )
    .unwrap();
    let e = s
        .execute("project.test", &json!({"path": "Shop.pbip"}))
        .unwrap_err();
    assert!(e.contains("refused in server mode"), "{e}");
    // readers may run it
    assert!(
        s.reader_check("project.test", &json!({"path": "Shop.pbip"}))
            .is_ok()
    );
}

#[test]
fn hostile_test_files_never_crash_the_command() {
    let (dir, pbip) = fixture("hostile", false);
    let mut s = Session::new();
    let cases = [
        "version: 1\ndax: [{query: \"EVALUATE\"}]\n".to_string(),
        format!(
            "version: 1\ndax: [{{expression: \"{}1{}\"}}]\n",
            "(".repeat(5000),
            ")".repeat(5000)
        ),
        "version: 1\ndax: [{measure: \"]]]['\"}]\n".to_string(),
        "version: 1\ndax: [{query: \"EVALUATE ROW(\\\"a\\\", 1)\", expect: {rows: [[1, 2, 3]]}}]\n"
            .to_string(),
        "version: 1\ndax: [{query: \"EVALUATE ROW(\\\"a\\\", 1)\", expect: {rows: [{b: 1}]}}]\n".to_string(),
        "version: 1\ndax: [{measure: x, filters: [\"))))\"]}]\n".to_string(),
        "version: 1\nmodel: {rules: [{kind: naming, pattern: \"(a*)*b\"}]}\n".to_string(),
        "version: 1\nreport: {rules: [{kind: max_visuals_per_page, max: 18446744073709551615}]}\n"
            .to_string(),
        "version: 18446744073709551616\n".to_string(),
        "version: 1\ndefaults: {timeout_ms: 0}\n".to_string(),
        "version: 1\ndax: [{expression: \"1\", expect: 1e308, tolerance: 1e308}]\n".to_string(),
        "\u{0}\u{1}version: 1".to_string(),
        "[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[".to_string(),
    ];
    for text in &cases {
        std::fs::write(dir.path().join("bicraft.tests.yaml"), text).unwrap();
        // an error or a result, never a panic ("internal error")
        match s.execute(
            "project.test",
            &json!({"path": pbip, "format": ["junit", "sarif", "text"]}),
        ) {
            Ok(v) => {
                assert!(v["summary"]["tests"].is_u64());
                roxmltree::Document::parse(v["outputs"]["junit"].as_str().unwrap()).unwrap();
                serde_json::from_str::<Value>(v["outputs"]["sarif"].as_str().unwrap()).unwrap();
            }
            Err(e) => assert!(!e.contains("internal error"), "{text:?}: {e}"),
        }
    }
    // not UTF-8, too large, a folder
    std::fs::write(dir.path().join("bicraft.tests.yaml"), [0xff, 0xfe, 0x00]).unwrap();
    assert!(
        s.execute("project.test", &json!({"path": pbip}))
            .unwrap_err()
            .contains("UTF-8")
    );
    let big = dir.path().join("big.yaml");
    std::fs::write(&big, vec![b' '; crate::citest_yaml::MAX_TEXT_BYTES + 1]).unwrap();
    assert!(
        s.execute("project.test", &json!({"path": pbip, "tests": big}))
            .unwrap_err()
            .contains("larger than")
    );
    assert!(
        s.execute("project.test", &json!({"path": pbip, "tests": dir.path()}))
            .unwrap_err()
            .contains("not a file")
    );
}

/// The hook reads `model.unused`'s result shape; only measures are judged.
#[test]
fn usage_hook_reads_model_unused_measures() {
    let v = json!({
        "unused": {
            "measures": [{"table": "Sales", "name": "Old KPI", "why": "no DAX, visual or filter uses it"}],
            "columns": [{"table": "Sales", "name": "Note", "why": "x"}],
            "tables": [{"name": "Tmp", "why": "x"}],
            "expressions": [{"name": "P", "why": "x"}],
        },
        "kept": [],
        "warnings": [],
    });
    let f = crate::citest_rules::unused_measures(&v, crate::citest::Level::Warning);
    assert_eq!(f.len(), 1);
    assert_eq!(
        f[0].message,
        "measure 'Sales'[Old KPI] is not used (no DAX, visual or filter uses it)"
    );
    assert_eq!(f[0].level, crate::citest::Level::Warning);
    assert!(
        crate::citest_rules::unused_measures(&json!({"unused": []}), crate::citest::Level::Error).is_empty()
    );
}
