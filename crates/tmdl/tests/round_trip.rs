//! Round trips over synthetic fixtures written in Power BI Desktop's canonical TMDL style.
//!
//! Fixture files are stored with LF line endings (git may convert them); the tests turn them
//! into Desktop's CRLF before use.

use std::path::{Path, PathBuf};

use bicraft_tmdl::{read_documents, read_folder, write_documents, write_folder};

#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;
use tempdir::TempDir;

fn fixture_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// `(relative path, CRLF text)` for every file of a fixture, sorted by path.
fn fixture(name: &str) -> Vec<(String, String)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                let text = std::fs::read_to_string(&path).unwrap();
                out.push((relative, text.replace("\r\n", "\n").replace('\n', "\r\n")));
            }
        }
    }
    let root = fixture_dir(name);
    let mut out = Vec::new();
    walk(&root, &root, &mut out);
    out.sort();
    out
}

/// Asserts two document sets are identical, showing the first differing line.
fn assert_same(expected: &[(String, String)], actual: &[(String, String)]) {
    let paths = |d: &[(String, String)]| d.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>();
    assert_eq!(paths(expected), paths(actual), "file lists differ");
    for ((path, want), (_, got)) in expected.iter().zip(actual) {
        if want == got {
            continue;
        }
        let w: Vec<&str> = want.split("\r\n").collect();
        let g: Vec<&str> = got.split("\r\n").collect();
        let line = w
            .iter()
            .zip(&g)
            .position(|(a, b)| a != b)
            .unwrap_or(w.len().min(g.len()));
        panic!(
            "{path}: first difference at line {}\n  want: {:?}\n  got:  {:?}\n--- got ---\n{got}",
            line + 1,
            w.get(line),
            g.get(line)
        );
    }
}

/// A guarded temp folder and a path inside it that does not exist yet.
fn temp_dir(tag: &str) -> (TempDir, PathBuf) {
    let tmp = TempDir::new(&format!("tmdl-{tag}"));
    let dir = tmp.join("out");
    (tmp, dir)
}

/// `contoso`: a realistic model. `edge`: names needing quotes, fenced expressions, quoted text
/// values, every column/measure/table property. `order`: every child object kind, to pin
/// Desktop's property and child order. `tom`: the TOM oracle's output for the synthetic model of
/// `cargo xtask tmdl-model` (every object kind TOM 17.0.87 writes, docs/formats/tmdl.md
/// "TOM oracle"). `tom-objects`: the same for `crates/metadata/tests/engine/objects.bim` (role
/// members with child objects, field parameters, column permissions).
const FIXTURES: [&str; 5] = ["contoso", "edge", "order", "tom", "tom-objects"];

#[test]
fn canonical_fixtures_round_trip_byte_for_byte() {
    for name in FIXTURES {
        let docs = fixture(name);
        let model = read_documents(&docs).unwrap();
        assert_same(&docs, &write_documents(&model));
    }
}

#[test]
fn canonical_fixture_reads_into_typed_fields() {
    let m = read_documents(&fixture("contoso")).unwrap();
    assert_eq!(m.name, "Contoso Sales");
    assert_eq!(m.compatibility_level, Some(1601));
    assert_eq!(m.culture.as_deref(), Some("en-US"));
    assert_eq!(
        m.description.as_deref(),
        Some("Synthetic sales model\nused by bicraft tests.")
    );
    let names: Vec<_> = m.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Order Lines",
            "Calendar",
            "Product",
            "Time Intelligence",
            "2024 Targets",
            "Sales.Archive"
        ]
    );
    let roles: Vec<_> = m.roles.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(roles, ["Sales Reader", "Admins"], "`ref role` order wins");

    let lines = m.table("Order Lines").unwrap();
    assert_eq!(
        lines.description.as_deref(),
        Some("Fact table of order lines.\nOne row per line.")
    );
    let key = lines.column("Order Key").unwrap();
    assert_eq!(key.source_column.as_deref(), Some("OrderKey"));
    assert_eq!(key.summarize_by.as_deref(), Some("none"));
    assert_eq!(key.annotations[0].name, "SummarizationSetBy");
    let total = lines.column("Line Total").unwrap();
    assert_eq!(
        total.expression.as_deref(),
        Some("\n    VAR q = 'Order Lines'[Qty]\n    RETURN\n        q * 'Order Lines'[Unit Price]")
    );
    let sales = lines.measure("Sales Amount").unwrap();
    assert_eq!(sales.display_folder.as_deref(), Some("Key Figures"));
    assert_eq!(sales.description.as_deref(), Some("Sum of the line amounts."));
    assert_eq!(
        sales.extra["kpi"]["statusExpression"],
        "VAR x = [Sales Amount]\nRETURN IF(x > 1000, 1, -1)"
    );
    let part = &lines.partitions[0];
    assert_eq!(
        (part.source_type.as_str(), part.mode.as_deref()),
        ("m", Some("import"))
    );
    assert!(
        part.expression
            .as_deref()
            .unwrap()
            .starts_with("let\n    Source = #\"Sales Staging\",")
    );
    assert_eq!(part.extra["queryGroup"], "Staging");

    let cal = m.table("Calendar").unwrap();
    assert_eq!(
        cal.column("Month Name").unwrap().sort_by_column.as_deref(),
        Some("Month Number")
    );
    let levels = &cal.extra["hierarchies"][0]["levels"];
    assert_eq!(levels[1]["name"], "Month");
    assert_eq!(levels[1]["ordinal"], 1, "level ordinals are restored");
    assert_eq!(levels[1]["column"], "Month Name");
    assert_eq!(cal.partitions[0].source_type, "calculated");

    let ti = m.table("Time Intelligence").unwrap();
    assert_eq!(ti.extra["calculationGroup"]["precedence"], 10);
    assert_eq!(
        ti.extra["calculationGroup"]["calculationItems"][1]["formatStringDefinition"]["expression"],
        "\"#,0\""
    );

    let r = &m.relationships[1];
    assert_eq!(
        (
            r.from_table.as_str(),
            r.from_column.as_str(),
            r.to_column.as_str()
        ),
        ("Order Lines", "Product Key", "Product Key")
    );
    assert_eq!(r.cross_filtering_behavior.as_deref(), Some("bothDirections"));
    assert_eq!(r.extra["relyOnReferentialIntegrity"], true);
    assert!(!m.relationships[2].is_active);
    assert_eq!(m.relationships[2].to_cardinality.as_deref(), Some("many"));

    let param = &m.expressions[0];
    assert_eq!(param.kind, "m");
    assert!(param.expression.contains("IsParameterQuery=true"));
    assert_eq!(param.extra["queryGroup"], "Parameters");

    let reader = &m.roles[0];
    assert_eq!(reader.model_permission.as_deref(), Some("read"));
    assert_eq!(reader.extra["description"], "Can read bike sales.");
    assert_eq!(reader.table_permissions[1].table, "Product");
    assert_eq!(
        reader.table_permissions[1].filter_expression.as_deref(),
        Some("Product[Category] = \"Bikes\"\n    || Product[Category] = \"Accessories\"")
    );
    assert_eq!(reader.extra["members"][0]["memberName"], "user@contoso.com");

    assert_eq!(m.extra["dataAccessOptions"]["legacyRedirects"], true);
    assert_eq!(m.extra["queryGroups"][1]["folder"], "Parameters");
    assert_eq!(m.extra["perspectives"][0]["tables"][1]["includeAll"], true);
    let en = &m.extra["cultures"][0];
    assert_eq!(en["linguisticMetadata"]["contentType"], "json");
    assert_eq!(en["linguisticMetadata"]["content"]["Language"], "en-US");
    let fr = &m.extra["cultures"][1]["translations"]["model"];
    assert_eq!(fr["translatedCaption"], "Modèle");
    assert_eq!(
        fr["tables"][0]["measures"][0]["translatedDisplayFolder"],
        "Chiffres clés"
    );
    assert_eq!(m.annotations.len(), 3);
}

#[test]
fn model_to_tmdl_to_model_is_lossless() {
    for name in FIXTURES {
        let model = read_documents(&fixture(name)).unwrap();
        let again = read_documents(&write_documents(&model)).unwrap();
        assert_eq!(again, model, "{name}");
    }
}

#[test]
fn writing_twice_gives_identical_bytes_and_folders_round_trip() {
    let model = read_documents(&fixture("contoso")).unwrap();
    assert_eq!(write_documents(&model), write_documents(&model));

    let (_tmp, dir) = temp_dir("folder");
    write_folder(&model, &dir).unwrap();
    let first = fixture_like(&dir);
    std::fs::create_dir_all(dir.join("tables/nested")).unwrap();
    std::fs::write(dir.join("tables/Stale.tmdl"), "table Stale\n").unwrap();
    std::fs::write(dir.join("tables/nested/Old.TMDL"), "table Old\n").unwrap();
    std::fs::write(dir.join("notes.md"), "keep me").unwrap();
    std::fs::write(dir.join("tables/keep.json"), "{}").unwrap();
    write_folder(&model, &dir).unwrap();
    assert_eq!(fixture_like(&dir), first, "stale .tmdl files are removed");
    assert!(dir.join("notes.md").exists() && dir.join("tables/keep.json").exists());
    assert_eq!(read_folder(&dir).unwrap(), model);
    assert_same(&fixture("contoso"), &first);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The `.tmdl` files of a folder as `(relative path, text)`.
fn fixture_like(dir: &Path) -> Vec<(String, String)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("tmdl")) {
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                out.push((relative, std::fs::read_to_string(&path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// F1 (docs/fuzzing.md, fuzz/artifacts/tmdl_read/roundtrip-fence-in-expression): an expression
/// with a line that is just ``` and whitespace that would ask for a fence is written indented,
/// since the ``` line would close the fence early. It reads back unchanged.
#[test]
fn expression_with_a_fence_line_round_trips() {
    let text = "table T\n\
                \tcolumn C\n\
                \t\tdataType: string\n\
                \t\te =\n\
                \t\t\t\ta\n\
                \t\t\t\t```\n\
                \t\t\t\tb\n\
                \n\
                \tmeasure M =\n\
                \t\t\t\tx\n\
                \t\t\t  ``` \n\
                \t\t\ty\n";
    let docs = vec![("tables/T.tmdl".to_string(), text.to_string())];
    let model = read_documents(&docs).unwrap();
    let measure = &model.tables[0].measures[0];
    assert_eq!(measure.expression, "\tx\n  ``` \ny");
    let written = write_documents(&model);
    let again = read_documents(&written).unwrap();
    assert_eq!(again, model);
    assert_eq!(write_documents(&again), written, "stable");

    // Text no TMDL form can hold exactly (a trailing line break wants a fence, the ``` line
    // forbids one) still writes readable output, minus the whitespace at the very end.
    let mut lossy = model.clone();
    lossy.tables[0].measures[0].expression = " a \n```\n\n".to_string();
    let back = read_documents(&write_documents(&lossy)).unwrap();
    assert_eq!(back.tables[0].measures[0].expression, " a \n```");
}

/// Fuzz finding F5: JSON values can carry keys that are not TMDL keywords (`a b`, `""`, `-x`,
/// U+FFFD). Inside a nested property block the writer wrote them as bare property lines, which
/// do not read back. Such a block is now written as `key: <JSON>` and reads back as that text.
#[test]
fn json_keys_that_are_not_keywords_read_back() {
    // An extended property under an unknown model block: its JSON value goes through the
    // nested property writer.
    let text = "model Model\n\
                \tblock\n\
                \t\textendedProperty Ext =\n\
                \t\t\t\t{\n\
                \t\t\t\t  \"a b\": 1\n\
                \t\t\t\t}\n";
    let docs = vec![("model.tmdl".to_string(), text.to_string())];
    let model = read_documents(&docs).unwrap();
    let written = write_documents(&model);
    let again = read_documents(&written).unwrap();
    assert_eq!(write_documents(&again), written, "stable");

    // The same keys in a column's unknown properties, objects and collections.
    let mut model = read_documents(&[(
        "tables/T.tmdl".to_string(),
        "table T\n\tcolumn C\n\t\tdataType: string\n".to_string(),
    )])
    .unwrap();
    let extra = serde_json::json!({
        "x": {"a b": 1, "ok": {"": [true, null]}},
        "items": [{"name": "n", "-y": 2}],
        "s": [{"name": "n"}],
        "translations": {"cultures": [{"name": "fr", "a.b": "c"}]},
    });
    model.tables[0].columns[0].extra = extra.as_object().unwrap().clone();
    let written = write_documents(&model);
    let again = read_documents(&written).unwrap();
    let back = &again.tables[0].columns[0].extra;
    assert_eq!(back["x"], r#"{"a b":1,"ok":{"":[true,null]}}"#);
    assert_eq!(back["items"], r#"[{"name":"n","-y":2}]"#);
    assert_eq!(write_documents(&again), written, "stable");
}

/// G24: what Desktop's serializer writes reads into the TMSL shape Desktop's own reader gives
/// (compared with TOM's TMDL reader, through the development oracles, when the fixture was recorded).
#[test]
fn tom_fixture_reads_like_tom() {
    let m = read_documents(&fixture("tom")).unwrap();
    let sources = &m.extra["dataSources"];
    // A provider data source: `= provider` in TMDL, no `type` in TMSL.
    assert_eq!(sources[0]["name"], "SQL Legacy");
    assert!(sources[0].get("type").is_none());
    assert_eq!(sources[0]["maxConnections"], 4);
    // A structured one: `type` implied in TMDL; `options`/`credential` are JSON text plus, for
    // the credential, its well-known fields as properties.
    let s = &sources[1];
    assert_eq!(s["type"], "structured");
    assert_eq!(s["options"], serde_json::json!({"commandTimeout": "PT5M"}));
    assert_eq!(
        s["credential"],
        serde_json::json!({"kind": "SQL", "path": "srv;db", "AuthenticationKind": "UsernamePassword", "Username": "u"})
    );
    assert_eq!(s["contextExpression"], "let\n  x = 1\nin\n  x");

    // Role members: bare = external user, `= group`, `= activeDirectory` = Windows member.
    let members = &m.roles[0].extra["members"];
    assert_eq!(
        members,
        &serde_json::json!([
            {"memberName": "user@contoso.com", "identityProvider": "AzureAD", "memberType": "user"},
            {"memberName": "Group A", "identityProvider": "AzureAD", "memberType": "group"},
            {"memberName": "DOMAIN\\user"}
        ])
    );

    // Calendars: repeated `associatedColumn:` / `column:` lines are TMSL name arrays.
    let cal = &m.table("Fiscal Calendar").unwrap().extra["calendars"][0];
    assert_eq!(cal["description"], "fiscal calendar");
    assert_eq!(
        cal["calendarColumnGroups"],
        serde_json::json!([
            {"timeUnit": "year", "primaryColumn": "Year"},
            {"timeUnit": "monthOfYear", "primaryColumn": "Month No", "associatedColumns": ["Month", "Week Day"]},
            {"columns": ["Date", "Week Day"]}
        ])
    );

    // Implied values TOM restores: the calculation group's partition, calculated table column
    // types, positional ordinals, midnight on TMDL dates.
    let ti = m.table("Time Intelligence").unwrap();
    assert_eq!(ti.partitions.len(), 1);
    assert_eq!(
        (
            ti.partitions[0].name.as_str(),
            ti.partitions[0].mode.as_deref(),
            ti.partitions[0].source_type.as_str()
        ),
        ("Partition", Some("import"), "calculationGroup")
    );
    let items = &ti.extra["calculationGroup"]["calculationItems"];
    let names: Vec<_> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| (i["name"].clone(), i["ordinal"].clone()))
        .collect();
    assert_eq!(
        names,
        [
            ("YTD".into(), 0.into()),
            ("Out of order".into(), 1.into()),
            ("Current".into(), 2.into())
        ]
    );
    let calc = m.table("Calc Table").unwrap();
    assert_eq!(calc.columns[0].extra["type"], "calculatedTableColumn");
    assert!(
        m.table("Order Lines")
            .unwrap()
            .columns
            .iter()
            .all(|c| c.extra.get("type").is_none())
    );
    let range = &m.table("Incremental").unwrap().partitions[0];
    assert_eq!(range.source_extra["start"], "2024-01-01T00:00:00");
    assert_eq!(range.source_extra["end"], "2025-01-01T00:00:00");

    // Other kinds and properties of TOM 17.0.87.
    let lines = m.table("Order Lines").unwrap();
    assert_eq!(
        lines.partitions[1].extra["dataCoverageDefinition"],
        serde_json::json!({"description": "dcd", "expression": "RELATED('Product'[Category]) = \"Bikes\""})
    );
    assert_eq!(
        lines.measure("Quote'n").unwrap().format_string.as_deref(),
        Some("\"$\"#,0")
    );
    assert_eq!(
        lines.column("colon").unwrap().format_string.as_deref(),
        Some("\"q\"")
    );
    assert_eq!(m.extra["bindingInfoCollection"][0]["connectionId"], "conn-1");
    assert_eq!(
        m.extra["functions"][0]["expression"],
        "(x : NUMERIC) =>\n    x + 1"
    );
    assert_eq!(m.extra["valueFilterBehavior"], "independent");
    let param = m.expressions.iter().find(|x| x.name == "Param").unwrap();
    assert_eq!(
        param.extra["parameterValuesColumn"],
        serde_json::json!({"table": "Product", "column": "Category"})
    );
    let copy = m.expressions.iter().find(|x| x.name == "DL Copy").unwrap();
    assert_eq!(copy.extra["expressionSource"], "DL");
    assert_eq!(
        m.table("Product").unwrap().column("Category").unwrap().extra["stringIndexingBehavior"],
        "full"
    );
}
