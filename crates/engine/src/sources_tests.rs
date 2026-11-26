//! `sources.list` and `sources.rebase` (`sources.rs`): every source kind, path styles, escaping,
//! parameter introduction, computed arguments, byte-identical M, round trips, undo, PBIX, server
//! mode. Fixtures are generated here.

use std::path::Path;

use bicraft_model::SemanticModel;
use serde_json::{Value, json};

use super::*;
use crate::Session;
use crate::tempdir::TempDir;

pub(crate) fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

pub(crate) fn fails(s: &mut Session, id: &str, params: Value) -> String {
    match s.execute(id, &params) {
        Ok(v) => panic!("{id} {params} should fail, got {v}"),
        Err(e) => e,
    }
}

pub(crate) fn model(s: &Session) -> &SemanticModel {
    s.model.as_ref().unwrap()
}

pub(crate) fn partition_m(s: &Session, table: &str) -> String {
    model(s).table(table).unwrap().partitions[0]
        .expression
        .clone()
        .unwrap()
}

pub(crate) fn expression_m(s: &Session, name: &str) -> String {
    model(s)
        .expressions
        .iter()
        .find(|e| e.name == name)
        .unwrap()
        .expression
        .clone()
}

pub(crate) const SALES: &str = "let\n    Source = Excel.Workbook(File.Contents(\"C:\\Data\\Finance\\sales.xlsx\"), null, true),\n    \
                                Sheet = Source{[Item = \"Sales\", Kind = \"Sheet\"]}[Data]\nin\n    Sheet";
pub(crate) const COSTS: &str = "let\n    Path = \"c:/data/finance/csv/costs.csv\",\n    \
                                Source = Csv.Document(File.Contents(Path), [Delimiter = \",\", Encoding = 65001]),\n    \
                                Promoted = Table.PromoteHeaders(Source)\nin\n    Promoted";
pub(crate) const MONTHLY: &str =
    "let\n    Source = Folder.Files(\"\\\\fileserver\\share\\finance\\monthly\\\")\nin\n    Source";
pub(crate) const POSIX: &str =
    "let Source = Csv.Document(File.Contents(\"/mnt/data/finance/x.csv\")) in Source";
pub(crate) const BUDGET: &str = "let\n    Source = Sql.Database(\"dev-sql.contoso.local\", \"FinanceDev\"),\n    \
                                 T = Source{[Schema = \"dbo\", Item = \"Budget\"]}[Data]\nin\n    T";
pub(crate) const LAKE: &str = "let\n    Source = Databricks.Catalogs(Host, HttpPath, [Catalog = null, Database = null])\nin\n    Source";
pub(crate) const RATES: &str =
    "let Source = Json.Document(Web.Contents(\"https://dev.example.com/api/rates\")) in Source";
pub(crate) const COMPUTED: &str =
    "let Source = Csv.Document(File.Contents(Text.Combine({\"C:\\Data\", \"x.csv\"}, \"\\\"))) in Source";
pub(crate) const PLAN: &str =
    "let Source = Excel.Workbook(File.Contents(RootFolder & \"plan.xlsx\")) in Source";
pub(crate) const QUOTED: &str =
    "let Source = Csv.Document(File.Contents(\"C:\\Data\\Finance\\say \"\"hi\"\" #(#)(x).csv\")) in Source";

fn param_m(value: &str, ty: &str) -> String {
    format!("{value} meta [IsParameterQuery = true, Type = \"{ty}\", IsParameterQueryRequired = true]")
}

/// A model with one table per source kind and the parameters they use.
pub(crate) fn fixture() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Finance"}));
    for (name, value, ty) in [
        ("RootFolder", "\"C:\\Data\\Finance\\\"", "Text"),
        ("Host", "\"adb-1.azuredatabricks.net\"", "Text"),
        ("HttpPath", "\"/sql/1.0/warehouses/dev\"", "Text"),
        ("Year", "2025", "Number"),
        ("Live", "false", "Logical"),
        ("Start", "#date(2025, 1, 1)", "Date"),
    ] {
        run(
            &mut s,
            "expression.add",
            json!({"name": name, "expression": param_m(value, ty)}),
        );
    }
    for (table, m) in [
        ("Sales", SALES),
        ("Costs", COSTS),
        ("Monthly", MONTHLY),
        ("Posix", POSIX),
        ("Budget", BUDGET),
        ("Lake", LAKE),
        ("Rates", RATES),
        ("Computed", COMPUTED),
        ("Plan", PLAN),
        ("Quoted", QUOTED),
    ] {
        run(
            &mut s,
            "table.add",
            json!({"name": table, "partition": {"source_type": "m", "expression": m}}),
        );
    }
    s
}

fn find<'v>(list: &'v Value, table: &str, function: &str) -> &'v Value {
    list.as_array()
        .unwrap()
        .iter()
        .find(|v| v["object"]["table"] == table && v["function"] == function)
        .unwrap_or_else(|| panic!("{table} {function} not in {list}"))
}

#[test]
fn path_rules() {
    let maps = |from: &str, to: &str| {
        vec![PathMap {
            from: from.into(),
            to: to.into(),
        }]
    };
    let r = |p: &str, from: &str, to: &str| rebase_path(p, &maps(from, to)).map(|(_, r)| r.value);
    // Windows: case and separators do not matter, whole folder names only
    assert_eq!(
        r("C:\\Data\\a.xlsx", "c:/data", "D:\\x").as_deref(),
        Some("D:\\x\\a.xlsx")
    );
    assert_eq!(r("C:\\Database\\a.xlsx", "C:\\Data", "D:\\x"), None);
    assert_eq!(r("C:\\Data", "C:\\Data\\", "D:\\x\\").as_deref(), Some("D:\\x"));
    assert_eq!(r("C:\\Data\\", "C:\\Data", "D:\\x").as_deref(), Some("D:\\x\\"));
    assert_eq!(
        r("C:\\Data\\a\\b.csv", "C:\\Data", "C:\\").as_deref(),
        Some("C:\\a\\b.csv")
    );
    assert_eq!(r("C:\\Data", "C:\\Data", "C:\\").as_deref(), Some("C:\\"));
    assert_eq!(r("D:\\Data\\a.csv", "C:\\Data", "E:\\"), None);
    // UNC, both ways
    assert_eq!(
        r("\\\\srv\\Share\\fin\\a.csv", "//SRV/share", "C:\\local").as_deref(),
        Some("C:\\local\\fin\\a.csv")
    );
    assert_eq!(
        r("C:\\x\\a.csv", "C:\\x", "\\\\srv\\s").as_deref(),
        Some("\\\\srv\\s\\a.csv")
    );
    // POSIX: exact case; forward slashes kept
    assert_eq!(r("/mnt/Data/a.csv", "/mnt/data", "/srv"), None);
    assert_eq!(
        r("/mnt/data/a/b.csv", "/mnt/data", "/srv/x/").as_deref(),
        Some("/srv/x/a/b.csv")
    );
    assert_eq!(
        r("/mnt/data/a.csv", "/", "/srv").as_deref(),
        Some("/srv/mnt/data/a.csv")
    );
    // Windows to POSIX and back: the new root's separator
    assert_eq!(
        r("C:\\Data\\a\\b.csv", "C:\\Data", "/srv").as_deref(),
        Some("/srv/a/b.csv")
    );
    // longest root wins
    let two = vec![
        PathMap {
            from: "C:\\Data".into(),
            to: "D:\\".into(),
        },
        PathMap {
            from: "C:\\Data\\Finance".into(),
            to: "F:\\fin".into(),
        },
    ];
    assert_eq!(
        rebase_path("C:\\Data\\Finance\\a.csv", &two).map(|(i, _)| i),
        Some(1)
    );
    // not paths
    for p in [
        "a.csv",
        "data\\a.csv",
        "https://x/y",
        "\\\\?\\C:\\x",
        "\\\\srv",
        "",
    ] {
        assert!(parse_path(p).is_none(), "{p}");
    }
    assert!(path_under("C:\\DATA\\x", "c:\\data") && !path_under("C:\\datax", "C:\\data"));
    assert_eq!(with_trailing_sep("C:\\x"), "C:\\x\\");
    assert_eq!(with_trailing_sep("/srv/x/"), "/srv/x/");
    assert_eq!(with_trailing_sep("C:"), "C:\\");
}

#[test]
fn list_reports_every_kind_and_form() {
    let mut s = fixture();
    let v = run(&mut s, "sources.list", json!({}));
    let src = &v["sources"];
    let sales = find(src, "Sales", "File.Contents");
    assert_eq!(sales["format"], "Excel.Workbook");
    assert_eq!(sales["arguments"][0]["form"], "literal");
    assert_eq!(sales["arguments"][0]["role"], "path");
    assert_eq!(sales["arguments"][0]["value"], "C:\\Data\\Finance\\sales.xlsx");
    let costs = find(src, "Costs", "File.Contents");
    assert_eq!(costs["arguments"][0]["form"], "step_literal");
    assert_eq!(costs["arguments"][0]["step"], "Path");
    assert_eq!(costs["format"], "Csv.Document");
    assert_eq!(find(src, "Costs", "Csv.Document")["arguments"][0]["form"], "call");
    assert_eq!(
        find(src, "Monthly", "Folder.Files")["arguments"][0]["role"],
        "folder"
    );
    let budget = find(src, "Budget", "Sql.Database");
    assert_eq!(budget["arguments"][0]["role"], "server");
    assert_eq!(budget["arguments"][1]["value"], "FinanceDev");
    let lake = find(src, "Lake", "Databricks.Catalogs");
    assert_eq!(lake["arguments"][0]["form"], "parameter");
    assert_eq!(lake["arguments"][0]["value"], "adb-1.azuredatabricks.net");
    assert_eq!(lake["arguments"][1]["role"], "http_path");
    assert_eq!(find(src, "Rates", "Web.Contents")["arguments"][0]["role"], "url");
    assert_eq!(find(src, "Rates", "Web.Contents")["format"], "Json.Document");
    let computed = find(src, "Computed", "File.Contents");
    assert_eq!(computed["arguments"][0]["form"], "call");
    let plan = find(src, "Plan", "File.Contents");
    assert_eq!(plan["arguments"][0]["form"], "concat");
    assert_eq!(plan["arguments"][0]["value"], "C:\\Data\\Finance\\plan.xlsx");
    assert_eq!(
        find(src, "Quoted", "File.Contents")["arguments"][0]["value"],
        "C:\\Data\\Finance\\say \"hi\" #(x).csv"
    );
    // spans point at the M text
    let m = partition_m(&s, "Sales");
    let sp = &sales["arguments"][0]["span"];
    let (a, b) = (
        sp["start"].as_u64().unwrap() as usize,
        sp["end"].as_u64().unwrap() as usize,
    );
    assert_eq!(&m[a..b], "\"C:\\Data\\Finance\\sales.xlsx\"");
    let params = v["parameters"].as_array().unwrap();
    let root = params.iter().find(|p| p["name"] == "RootFolder").unwrap();
    assert_eq!(root["used_as"], json!(["path"]));
    assert_eq!(root["type"], "Text");
    assert_eq!(
        params.iter().find(|p| p["name"] == "Year").unwrap()["value"],
        2025.0
    );
    assert_eq!(
        params.iter().find(|p| p["name"] == "Start").unwrap()["type"],
        "Date"
    );
    assert_eq!(v["unparsed"], json!([]));
    // read-only: no revision bump
    let before = s.revision;
    run(&mut s, "sources.list", json!({}));
    assert_eq!(s.revision, before);
}

#[test]
fn every_source_function_is_recognized() {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "K"}));
    let calls = [
        ("Folder.Contents(\"C:\\x\")", "folder"),
        ("Sql.Databases(\"srv\")", "server"),
        ("PostgreSQL.Database(\"pg\", \"db\")", "server"),
        ("MySQL.Database(\"my\", \"db\")", "server"),
        ("Oracle.Database(\"ora\")", "server"),
        (
            "Snowflake.Databases(\"acct.snowflakecomputing.com\", \"WH\")",
            "server",
        ),
        ("AnalysisServices.Database(\"asazure://x\", \"db\")", "server"),
        ("Databricks.Contents(\"h\", \"/p\")", "host"),
        ("DatabricksMultiCloud.Catalogs(\"h\", \"/p\")", "host"),
        ("OData.Feed(\"https://o/x\")", "url"),
        ("SharePoint.Files(\"https://t.sharepoint.com/sites/f\")", "url"),
        ("AzureStorage.Blobs(\"acct\")", "account"),
        (
            "AzureStorage.DataLake(\"https://a.dfs.core.windows.net/c\")",
            "url",
        ),
        ("Odbc.DataSource(\"dsn=x\")", "connection_string"),
        ("Parquet.Document(File.Contents(\"/d/x.parquet\"))", "content"),
    ];
    for (i, (call, role)) in calls.iter().enumerate() {
        run(
            &mut s,
            "table.add",
            json!({"name": format!("T{i}"), "partition": {"source_type": "m", "expression": format!("let Source = {call} in Source")}}),
        );
        let v = run(&mut s, "sources.list", json!({}));
        let first = v["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["object"]["table"] == format!("T{i}"))
            .unwrap_or_else(|| panic!("{call}: {v}"));
        assert_eq!(first["arguments"][0]["role"], *role, "{call}");
    }
}

#[test]
fn rebase_rewrites_literals_only_and_leaves_the_rest_byte_for_byte() {
    let mut s = fixture();
    let before = model(&s).clone();
    let v = run(
        &mut s,
        "sources.rebase",
        json!({"from": "C:\\Data\\Finance", "to": "\\\\files\\share\\fin"}),
    );
    assert_eq!(v["changed"], true);
    assert_eq!(
        partition_m(&s, "Sales"),
        SALES.replace(
            "C:\\Data\\Finance\\sales.xlsx",
            "\\\\files\\share\\fin\\sales.xlsx"
        )
    );
    // the step literal; the old path's case-insensitive match keeps the new root's separator
    assert_eq!(
        partition_m(&s, "Costs"),
        COSTS.replace(
            "c:/data/finance/csv/costs.csv",
            "\\\\files\\share\\fin\\csv\\costs.csv"
        )
    );
    // escaping: quotes doubled again, `#(` escaped again
    assert_eq!(
        partition_m(&s, "Quoted"),
        QUOTED.replace("C:\\Data\\Finance\\", "\\\\files\\share\\fin\\")
    );
    // the RootFolder parameter's value is a path under the root: rebased too
    assert!(expression_m(&s, "RootFolder").starts_with("\"\\\\files\\share\\fin\\\" meta ["));
    // untouched: other roots, POSIX, computed, servers, parameter-relative concatenation
    for t in ["Monthly", "Posix", "Computed", "Budget", "Plan", "Lake", "Rates"] {
        assert_eq!(
            partition_m(&s, t),
            before.table(t).unwrap().partitions[0].expression.clone().unwrap(),
            "{t}"
        );
    }
    let unchanged = v["unchanged"].as_array().unwrap();
    let reason = |t: &str| {
        unchanged
            .iter()
            .find(|u| u["object"]["table"] == t && u["role"] != "content")
            .map(|u| u["reason"].clone())
    };
    assert_eq!(reason("Computed"), Some(json!("computed")));
    assert_eq!(reason("Plan"), Some(json!("parameter")));
    assert_eq!(reason("Monthly"), Some(json!("no_mapping")));
    let sales = find(&v["changes"], "Sales", "File.Contents");
    assert_eq!(sales["before"], "C:\\Data\\Finance\\sales.xlsx");
    assert_eq!(sales["m_after"], "\"\\\\files\\share\\fin\\sales.xlsx\"");
    // the rewritten M still validates
    let m = run(&mut s, "model.validate", json!({}));
    assert!(!m.to_string().contains("m-syntax"), "{m}");
    // one undo step brings everything back
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(model(&s), &before);
}

#[test]
fn dry_run_returns_the_same_plan_and_changes_nothing() {
    let mut s = fixture();
    let before = model(&s).clone();
    let p = json!({"from": "C:\\Data\\Finance", "to": "D:\\fin", "servers": {"DEV-SQL.contoso.local": "prod-sql"}});
    let mut dry = p.clone();
    dry["dry_run"] = json!(true);
    let d = run(&mut s, "sources.rebase", dry);
    assert_eq!(model(&s), &before);
    assert_eq!(d["dry_run"], true);
    let r = run(&mut s, "sources.rebase", p);
    assert_eq!(d["changes"], r["changes"]);
    assert_eq!(d["parameters"], r["parameters"]);
    assert!(partition_m(&s, "Budget").contains("Sql.Database(\"prod-sql\", \"FinanceDev\")"));
    // nothing to do: no change, no undo step
    let again = run(
        &mut s,
        "sources.rebase",
        json!({"from": "C:\\Data\\Finance", "to": "D:\\fin"}),
    );
    assert_eq!(again["changed"], false);
}

#[test]
fn rebase_relative_to_an_existing_parameter() {
    let mut s = fixture();
    let v = run(
        &mut s,
        "sources.rebase",
        json!({"from": "c:\\data\\finance\\", "to": "D:\\fin", "param": "RootFolder"}),
    );
    assert_eq!(
        partition_m(&s, "Sales"),
        SALES.replace("\"C:\\Data\\Finance\\sales.xlsx\"", "RootFolder & \"sales.xlsx\"")
    );
    assert_eq!(
        partition_m(&s, "Costs"),
        COSTS.replace(
            "\"c:/data/finance/csv/costs.csv\"",
            "RootFolder & \"csv\\costs.csv\""
        )
    );
    assert_eq!(expression_m(&s, "RootFolder"), param_m("\"D:\\fin\\\"", "Text"));
    assert_eq!(v["added_parameter"], Value::Null);
    assert_eq!(
        find(&v["changes"], "Sales", "File.Contents")["relative_to"],
        "RootFolder"
    );
    // to defaults to from: parameterize without moving
    let mut s = fixture();
    run(
        &mut s,
        "sources.rebase",
        json!({"from": "C:\\Data\\Finance", "param": "RootFolder"}),
    );
    assert_eq!(
        expression_m(&s, "RootFolder"),
        param_m("\"C:\\Data\\Finance\\\"", "Text")
    );
}

#[test]
fn rebase_introduces_a_new_parameter() {
    let mut s = fixture();
    let v = run(
        &mut s,
        "sources.rebase",
        json!({"from": "C:\\Data", "to": "/srv/data", "param": "Data Root"}),
    );
    assert_eq!(v["added_parameter"]["name"], "Data Root");
    assert_eq!(
        expression_m(&s, "Data Root"),
        "\"/srv/data/\" meta [IsParameterQuery=true, Type=\"Text\", IsParameterQueryRequired=true]"
    );
    assert_eq!(
        partition_m(&s, "Sales"),
        SALES.replace(
            "\"C:\\Data\\Finance\\sales.xlsx\"",
            "#\"Data Root\" & \"Finance/sales.xlsx\""
        )
    );
    let e = model(&s)
        .expressions
        .iter()
        .find(|e| e.name == "Data Root")
        .unwrap();
    assert!(e.lineage_tag.is_some());
    // every query still validates (the new name is known), and the query order lists it
    let m = run(&mut s, "model.validate", json!({}));
    assert!(!m.to_string().contains("m-undefined-name"), "{m}");
    let order = model(&s)
        .annotations
        .iter()
        .find(|a| a.name == crate::mquery::QUERY_ORDER)
        .map(|a| a.value.clone())
        .unwrap_or_default();
    assert!(order.is_empty() || order.contains("Data Root"), "{order}");
    // a second rebase reuses it
    let v = run(
        &mut s,
        "sources.rebase",
        json!({"from": "/srv/data", "to": "/mnt/x", "param": "Data Root"}),
    );
    assert_eq!(v["added_parameter"], Value::Null);
    assert!(expression_m(&s, "Data Root").starts_with("\"/mnt/x/\" meta"));
    // undo removes the parameter with the rewrite
    run(&mut s, "edit.undo", json!({"steps": 2}));
    assert!(!model(&s).expressions.iter().any(|e| e.name == "Data Root"));
}

#[test]
fn rebase_refusals() {
    let mut s = fixture();
    for (p, needle) in [
        (json!({}), "nothing to rewrite"),
        (json!({"to": "D:\\x"}), "need `from`"),
        (json!({"from": "C:\\x"}), "needs `to`"),
        (json!({"from": "relative\\x", "to": "D:\\"}), "absolute path"),
        (json!({"from": "C:\\Data", "param": "Year"}), "type Number"),
        (json!({"from": "C:\\Data", "param": "Sales"}), "table"),
        (json!({"servers": {"a": 1}}), "non-empty"),
        (json!({"servers": ["a"]}), "must be an object"),
        (
            json!({"from": "C:\\Data", "to": "D:\\", "nope": 1}),
            "unknown parameter",
        ),
    ] {
        let e = fails(&mut s, "sources.rebase", p.clone());
        assert!(e.contains(needle), "{p}: {e}");
    }
    // a parameter nothing would use is not added
    let v = run(
        &mut s,
        "sources.rebase",
        json!({"from": "Z:\\none", "param": "Unused"}),
    );
    assert_eq!(v["changed"], false);
    assert!(v["warnings"].to_string().contains("was not added"), "{v}");
    // a relative target is written but warned about
    let v = run(
        &mut s,
        "sources.rebase",
        json!({"from": "C:\\Data", "to": "data", "dry_run": true}),
    );
    assert!(v["warnings"].to_string().contains("absolute path"), "{v}");
}

#[test]
fn servers_databases_hosts_and_urls() {
    let mut s = fixture();
    run(
        &mut s,
        "sources.rebase",
        json!({
            "servers": {"DEV-SQL.CONTOSO.LOCAL": "prod-sql.contoso.local"},
            "databases": {"financedev": "Finance"},
            "hosts": {"adb-1.azuredatabricks.net": "adb-2.azuredatabricks.net"},
            "http_paths": {"/sql/1.0/warehouses/dev": "/sql/1.0/warehouses/prod"},
            "urls": {"https://dev.example.com/": "https://api.example.com/"},
        }),
    );
    assert_eq!(
        partition_m(&s, "Budget"),
        BUDGET
            .replace("dev-sql.contoso.local", "prod-sql.contoso.local")
            .replace("FinanceDev", "Finance")
    );
    // the Databricks parameters are used as host and HTTP path: their values are mapped
    assert!(expression_m(&s, "Host").starts_with("\"adb-2.azuredatabricks.net\" meta"));
    assert!(expression_m(&s, "HttpPath").starts_with("\"/sql/1.0/warehouses/prod\" meta"));
    assert_eq!(partition_m(&s, "Lake"), LAKE);
    assert_eq!(
        partition_m(&s, "Rates"),
        RATES.replace("https://dev.example.com/", "https://api.example.com/")
    );
}

#[test]
fn text_literals_escape_like_m() {
    assert_eq!(text_literal("a\"b"), "\"a\"\"b\"");
    assert_eq!(text_literal("x#(y"), "\"x#(#)(y\"");
    assert_eq!(text_literal("#x"), "\"#x\"");
    assert_eq!(text_literal("a\r\n\tb\u{1}"), "\"a#(cr)#(lf)#(tab)b#(0001)\"");
    // and they read back as the same text
    for t in ["a\"b", "x#(y", "a\r\n\tb\u{1}", "C:\\é\\日本\\", ""] {
        let m = format!("let S = File.Contents({}) in S", text_literal(t));
        let calls = bicraft_data::m::sources::source_calls(&m).unwrap();
        match &calls[0].args[0].form {
            Form::Text(l) => assert_eq!(l.value, t),
            other => panic!("{t}: {other:?}"),
        }
    }
}

fn temp(name: &str) -> TempDir {
    TempDir::new(&format!("engine-sources-{name}"))
}

fn reopen(path: &Path) -> Session {
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    assert_eq!(s.model_error, None);
    s
}

fn m_texts(s: &Session) -> Vec<(String, String)> {
    let m = model(s);
    let mut v: Vec<(String, String)> = m
        .tables
        .iter()
        .flat_map(|t| {
            t.partitions
                .iter()
                .map(move |p| (t.name.clone(), p.expression.clone().unwrap_or_default()))
        })
        .chain(
            m.expressions
                .iter()
                .map(|e| (e.name.clone(), e.expression.clone())),
        )
        .collect();
    v.sort();
    v
}

#[test]
fn round_trips_through_pbip_bim_and_pbit() {
    for format in ["tmdl", "tmsl"] {
        let dir = temp(format);
        let mut s = fixture();
        run(
            &mut s,
            "project.new",
            json!({"dir": dir.path(), "name": "Finance", "format": format}),
        );
        let pbip = dir.join("Finance.pbip");
        let mut s = reopen(&pbip);
        run(
            &mut s,
            "sources.rebase",
            json!({"from": "C:\\Data\\Finance", "to": "\\\\files\\fin", "param": "FinanceRoot"}),
        );
        run(&mut s, "file.save", json!({}));
        let again = reopen(&pbip);
        assert_eq!(m_texts(&again), m_texts(&s), "{format}");
        assert!(m_texts(&again).iter().any(|(n, _)| n == "FinanceRoot"));
    }
    let dir = temp("bim-pbit");
    for name in ["model.bim", "Finance.pbit"] {
        let mut s = fixture();
        run(
            &mut s,
            "sources.rebase",
            json!({"from": "C:\\Data", "to": "/srv/data"}),
        );
        let path = dir.join(name);
        run(&mut s, "file.save_as", json!({"path": path}));
        let again = reopen(&path);
        assert_eq!(m_texts(&again), m_texts(&s), "{name}");
        // and a rebase of the reopened file works the same
        let mut again = again;
        let v = run(
            &mut again,
            "sources.rebase",
            json!({"from": "/srv/data", "to": "/x", "dry_run": true}),
        );
        assert_eq!(v["changed"], true, "{name}");
    }
}

#[test]
fn a_pbix_with_data_is_refused_but_previewed() {
    let dir = temp("pbix");
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "P"}));
    run(
        &mut s,
        "table.add",
        json!({"name": "Budget", "partition": {"source_type": "m", "expression": BUDGET}}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "Budget", "name": "Amount", "data_type": "double"}),
    );
    let path = dir.join("p.pbix");
    run(
        &mut s,
        "file.save_as",
        json!({"path": path, "empty_tables": true}),
    );
    let mut s = reopen(&path);
    let p = json!({"servers": {"dev-sql.contoso.local": "prod"}});
    let e = fails(&mut s, "sources.rebase", p.clone());
    assert!(e.contains("PBIP"), "{e}");
    let mut dry = p;
    dry["dry_run"] = json!(true);
    let v = run(&mut s, "sources.rebase", dry);
    assert_eq!(v["changes"].as_array().unwrap().len(), 1, "{v}");
}

#[test]
fn server_mode_rewrites_m_paths_without_resolving_them() {
    let dir = temp("server");
    let mut s = Session::server(crate::sandbox::ServerConfig::new(&dir).unwrap());
    run(&mut s, "model.new", json!({"name": "M"}));
    run(
        &mut s,
        "table.add",
        json!({"name": "Sales", "partition": {"source_type": "m", "expression": SALES}}),
    );
    // `from` and `to` are text in M, not files of the server: absolute paths are fine
    let v = run(
        &mut s,
        "sources.rebase",
        json!({"from": "C:\\Data", "to": "/srv/data"}),
    );
    assert_eq!(v["changed"], true);
    run(&mut s, "sources.list", json!({}));
    assert!(s.command_available("sources.rebase") && s.command_available("sources.list"));
}

#[test]
fn hostile_m_is_reported_never_a_crash() {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "H"}));
    for (i, m) in [
        "File.Contents(",
        "File.Contents((((((",
        "File.Contents(\"unterminated",
        "File.Contents(,,,)",
        "Folder.Files)))",
        &format!("File.Contents({}\"x\"{})", "(".repeat(5000), ")".repeat(5000)),
        &"let a = ".repeat(2000),
        "x meta [IsParameterQuery = true, Type =",
    ]
    .iter()
    .enumerate()
    {
        let name = format!("H{i}");
        s.model
            .as_mut()
            .unwrap()
            .expressions
            .push(bicraft_model::NamedExpression {
                name,
                kind: "m".into(),
                expression: (*m).to_string(),
                ..Default::default()
            });
    }
    let v = run(&mut s, "sources.list", json!({}));
    assert!(!v["unparsed"].as_array().unwrap().is_empty(), "{v}");
    run(
        &mut s,
        "sources.rebase",
        json!({"from": "C:\\", "to": "D:\\", "dry_run": true}),
    );
}
