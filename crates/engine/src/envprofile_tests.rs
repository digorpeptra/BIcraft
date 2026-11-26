//! Environment profiles (`envprofile.rs`): `env.apply`, `env.check`, typed values, the profile
//! file (next to the project, given, inline), hostile YAML, server mode.

use serde_json::{Value, json};

use super::*;
use crate::Session;
use crate::sources::tests::{
    BUDGET, COSTS, LAKE, MONTHLY, RATES, SALES, expression_m, fails, fixture, model, partition_m, run,
};
use crate::tempdir::TempDir;

const PROFILES: &str = r#"# environments of the Finance model
version: 1
profiles:
  dev:
    parameters:
      RootFolder: 'C:\Data\Finance\'
  prod:
    description: production
    parameters:
      RootFolder: '\\files\share\fin\'
      Year: 2026
      Live: true
      Start: 2026-01-01
    paths:
      'C:\Data\Finance': '\\files\share\fin'
      '/mnt/data': '/srv/data'
    servers:
      dev-sql.contoso.local: prod-sql.contoso.local
    databases:
      FinanceDev: Finance
    hosts:
      adb-1.azuredatabricks.net: adb-2.azuredatabricks.net
    http_paths:
      /sql/1.0/warehouses/dev: /sql/1.0/warehouses/prod
    urls:
      'https://dev.example.com/': 'https://api.example.com/'
"#;

fn temp(name: &str) -> TempDir {
    TempDir::new(&format!("engine-envprofile-{name}"))
}

fn with_file(name: &str, text: &str) -> (TempDir, String) {
    let dir = temp(name);
    let path = dir.join("bicraft.env.yaml");
    std::fs::write(&path, text).unwrap();
    (dir, path.display().to_string())
}

#[test]
fn apply_sets_parameters_and_maps_sources_in_one_edit() {
    let (_dir, file) = with_file("apply", PROFILES);
    let mut s = fixture();
    let before = model(&s).clone();
    let v = run(&mut s, "env.apply", json!({"profile": "prod", "env_file": file}));
    assert_eq!(v["profile"], "prod");
    assert_eq!(v["changed"], true);
    assert!(expression_m(&s, "RootFolder").starts_with("\"\\\\files\\share\\fin\\\" meta"));
    assert!(expression_m(&s, "Year").starts_with("2026 meta"));
    assert!(expression_m(&s, "Live").starts_with("true meta"));
    assert!(expression_m(&s, "Start").starts_with("#date(2026, 1, 1) meta"));
    assert!(expression_m(&s, "Host").starts_with("\"adb-2.azuredatabricks.net\" meta"));
    assert_eq!(
        partition_m(&s, "Sales"),
        SALES.replace(
            "C:\\Data\\Finance\\sales.xlsx",
            "\\\\files\\share\\fin\\sales.xlsx"
        )
    );
    assert_eq!(
        partition_m(&s, "Costs"),
        COSTS.replace(
            "c:/data/finance/csv/costs.csv",
            "\\\\files\\share\\fin\\csv\\costs.csv"
        )
    );
    assert_eq!(
        partition_m(&s, "Budget"),
        BUDGET
            .replace("dev-sql.contoso.local", "prod-sql.contoso.local")
            .replace("FinanceDev", "Finance")
    );
    assert_eq!(partition_m(&s, "Lake"), LAKE);
    assert_eq!(partition_m(&s, "Monthly"), MONTHLY);
    assert_eq!(
        partition_m(&s, "Rates"),
        RATES.replace("https://dev.example.com/", "https://api.example.com/")
    );
    // the parameter set explicitly wins over the path mapping
    let root = v["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "RootFolder")
        .unwrap();
    assert_eq!(root["after"], "\\\\files\\share\\fin\\");
    // one undo step
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(model(&s), &before);
    // applying twice changes nothing the second time
    run(&mut s, "env.apply", json!({"profile": "prod", "env_file": file}));
    let again = run(&mut s, "env.apply", json!({"profile": "prod", "env_file": file}));
    assert_eq!(again["changed"], false, "{again}");
}

#[test]
fn dry_run_and_inline_profiles() {
    let mut s = fixture();
    let before = model(&s).clone();
    let inline = json!({"version": 1, "profiles": {"t": {"parameters": {"Year": 2030}}}});
    let v = run(
        &mut s,
        "env.apply",
        json!({"profile": "t", "env_file": inline, "dry_run": true}),
    );
    assert_eq!(model(&s), &before);
    assert_eq!(v["parameters"][0]["m_after"], "2030");
    assert_eq!(v["env_file"], Value::Null);
}

#[test]
fn profile_errors_refuse_the_whole_profile() {
    let mut s = fixture();
    let before = model(&s).clone();
    let doc = |p: Value| json!({"version": 1, "profiles": {"p": p}});
    for (env, needle) in [
        (doc(json!({"parameters": {"Nope": 1}})), "not a parameter"),
        (
            doc(json!({"parameters": {"rootfolder": "x"}})),
            "did you mean `RootFolder`",
        ),
        (
            doc(json!({"parameters": {"Year": "2026"}})),
            "Number parameter needs a number",
        ),
        (
            doc(json!({"parameters": {"RootFolder": 3}})),
            "Text parameter needs a text",
        ),
        (doc(json!({"parameters": {"Live": "yes"}})), "true or false"),
        (
            doc(json!({"parameters": {"Start": "2026-02-30"}})),
            "a date YYYY-MM-DD",
        ),
        (doc(json!({"parameters": {"Year": null}})), "null"),
        (doc(json!({"parameters": {"Year": 1, "Nope": 2}})), "Nope"),
        (doc(json!({"server": {}})), "unknown key `server`"),
        (
            doc(json!({"paths": {"relative": "D:\\"}})),
            "not an absolute path",
        ),
        (doc(json!({"servers": {"a": ""}})), "non-empty"),
        (doc(json!([1])), "must be a mapping"),
        (json!({"profiles": {"p": {}}, "extra": 1}), "unknown key `extra`"),
        (json!({"version": 2, "profiles": {"p": {}}}), "version 2"),
        (json!({"version": 1}), "`profiles` must be a mapping"),
        (
            json!({"version": 1, "profiles": {"a": {}, "b": {}}}),
            "unknown profile `p` (the file has: a, b)",
        ),
    ] {
        let e = fails(&mut s, "env.apply", json!({"profile": "p", "env_file": env}));
        assert!(e.contains(needle), "{env}: {e}");
        assert_eq!(model(&s), &before);
    }
    let e = fails(&mut s, "env.apply", json!({"env_file": {"profiles": {}}}));
    assert!(e.contains("profile"), "{e}");
    let e = fails(&mut s, "env.apply", json!({"profile": "p", "env_file": 3}));
    assert!(e.contains("env_file"), "{e}");
    // no open file: no project folder to look in
    let e = fails(&mut s, "env.apply", json!({"profile": "p"}));
    assert!(e.contains("env_file"), "{e}");
}

#[test]
fn typed_values() {
    let t = |ty: &str, v: Value| typed_value(ty, &v).map(|n| n.m);
    assert_eq!(t("Text", json!("a\"b")).unwrap(), "\"a\"\"b\"");
    assert_eq!(t("Number", json!(-2.5)).unwrap(), "-2.5");
    assert_eq!(t("Number", json!(7)).unwrap(), "7");
    assert_eq!(t("Logical", json!(false)).unwrap(), "false");
    assert_eq!(t("Date", json!("2024-02-29")).unwrap(), "#date(2024, 2, 29)");
    assert!(t("Date", json!("2023-02-29")).is_err());
    assert!(t("Date", json!("24-1-1")).is_err());
    assert_eq!(
        t("DateTime", json!("2024-01-02T03:04:05")).unwrap(),
        "#datetime(2024, 1, 2, 3, 4, 5)"
    );
    assert_eq!(
        t("DateTime", json!("2024-01-02 03:04")).unwrap(),
        "#datetime(2024, 1, 2, 3, 4, 0)"
    );
    assert!(t("DateTime", json!("2024-01-02T25:00:00")).is_err());
    assert_eq!(t("Time", json!("23:59:59")).unwrap(), "#time(23, 59, 59)");
    assert_eq!(t("Any", json!("x")).unwrap(), "\"x\"");
    assert_eq!(t("Any", json!(true)).unwrap(), "true");
    assert!(t("Any", json!([1])).is_err());
    assert!(t("Binary", json!("x")).unwrap_err().contains("cannot be set"));
    assert!(t("Duration", json!("x")).is_err());
}

#[test]
fn allowed_values_are_enforced() {
    let mut s = fixture();
    run(
        &mut s,
        "expression.add",
        json!({"name": "Region", "expression": "\"EU\" meta [IsParameterQuery = true, List = {\"EU\", \"US\"}, DefaultValue = \"EU\", Type = \"Text\", IsParameterQueryRequired = true]"}),
    );
    let env = |v: &str| json!({"profiles": {"p": {"parameters": {"Region": v}}}});
    let e = fails(
        &mut s,
        "env.apply",
        json!({"profile": "p", "env_file": env("APAC")}),
    );
    assert!(e.contains("allowed values"), "{e}");
    run(
        &mut s,
        "env.apply",
        json!({"profile": "p", "env_file": env("US")}),
    );
    assert!(expression_m(&s, "Region").starts_with("\"US\" meta"));
}

#[test]
fn check_reports_gaps_and_differences() {
    let (_dir, file) = with_file("check", PROFILES);
    let mut s = fixture();
    let rev = s.revision;
    let v = run(&mut s, "env.check", json!({"profile": "prod", "env_file": file}));
    assert_eq!(s.revision, rev, "read-only");
    assert_eq!(v["ok"], false, "{v}");
    let sources = v["sources"].as_array().unwrap();
    let status = |t: &str, role: &str| {
        sources
            .iter()
            .find(|x| x["object"]["table"] == t && x["role"] == role)
            .map(|x| x["status"].clone())
            .unwrap_or_else(|| panic!("{t} {role}: {v}"))
    };
    assert_eq!(status("Sales", "path"), "differs");
    assert_eq!(status("Monthly", "folder"), "not_covered");
    assert_eq!(status("Computed", "path"), "computed");
    assert_eq!(status("Plan", "path"), "covered");
    assert_eq!(status("Lake", "host"), "covered");
    let params = v["parameters"].as_array().unwrap();
    let p = |n: &str| params.iter().find(|p| p["name"] == n).unwrap().clone();
    assert_eq!(p("Year")["status"], "differs");
    assert_eq!(p("Year")["target"], 2026);
    assert_eq!(p("Host")["status"], "differs");
    assert_eq!(v["summary"]["sources_not_covered"], 1, "{v}");
    assert!(v["warnings"].to_string().contains("computed"));
    // the dev profile covers nothing but RootFolder: the Databricks parameters are gaps
    let dev = run(&mut s, "env.check", json!({"profile": "dev", "env_file": file}));
    assert_eq!(dev["ok"], false);
    let dp = dev["parameters"].as_array().unwrap();
    assert_eq!(
        dp.iter().find(|p| p["name"] == "RootFolder").unwrap()["status"],
        "same"
    );
    assert_eq!(
        dp.iter().find(|p| p["name"] == "Host").unwrap()["status"],
        "not_covered"
    );
    // a profile that covers every source the profile can reach: ok
    let full = PROFILES.replace(
        "      '/mnt/data': '/srv/data'",
        "      '/mnt/data': '/srv/data'\n      '\\\\fileserver\\share': '\\\\files\\share'",
    );
    let (_d2, file2) = with_file("check-full", &full);
    let v = run(&mut s, "env.check", json!({"profile": "prod", "env_file": file2}));
    assert_eq!(v["ok"], true, "{v}");
    // errors make it not ok, without failing the command
    let v = run(
        &mut s,
        "env.check",
        json!({"profile": "p", "env_file": {"profiles": {"p": {"parameters": {"Year": "x"}}}}}),
    );
    assert_eq!(v["ok"], false);
    assert_eq!(v["errors"].as_array().unwrap().len(), 1);
}

#[test]
fn the_profile_file_next_to_the_project() {
    let dir = temp("next");
    let mut s = fixture();
    run(
        &mut s,
        "project.new",
        json!({"dir": dir.path(), "name": "Finance", "format": "tmdl"}),
    );
    let pbip = dir.join("Finance.pbip");
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": pbip}));
    let e = fails(&mut s, "env.check", json!({"profile": "prod"}));
    assert!(e.contains("bicraft.env.yaml"), "{e}");
    std::fs::write(dir.join("bicraft.env.yaml"), PROFILES).unwrap();
    let v = run(&mut s, "env.apply", json!({"profile": "prod"}));
    assert!(v["env_file"].as_str().unwrap().ends_with("bicraft.env.yaml"));
    run(&mut s, "file.save", json!({}));
    let mut again = Session::new();
    run(&mut again, "file.open", json!({"path": pbip}));
    assert!(expression_m(&again, "Year").starts_with("2026 meta"));
    // JSON profiles too
    std::fs::remove_file(dir.join("bicraft.env.yaml")).unwrap();
    std::fs::write(
        dir.join("bicraft.env.json"),
        r#"{"version": 1, "profiles": {"y": {"parameters": {"Year": 2027}}}}"#,
    )
    .unwrap();
    run(&mut again, "env.apply", json!({"profile": "y"}));
    assert!(expression_m(&again, "Year").starts_with("2027 meta"));
}

#[test]
fn hostile_profile_files_are_refused() {
    let mut s = fixture();
    let deep = format!("{}1{}", "[".repeat(200), "]".repeat(200));
    let wide = "a: 1\n".repeat(10);
    for (name, text, needle) in [
        (
            "alias",
            "profiles:\n  p: &x {}\n  q: *x\n".to_string(),
            "anchors and aliases",
        ),
        (
            "deep",
            format!("profiles:\n  p:\n    parameters: {deep}\n"),
            "nested deeper",
        ),
        (
            "dup",
            "profiles:\n  p: {}\n  p: {}\n".to_string(),
            "duplicate key",
        ),
        ("tag", "profiles: !!binary aGk=\n".to_string(), "tag"),
        (
            "docs",
            "profiles: {}\n---\nprofiles: {}\n".to_string(),
            "one YAML document",
        ),
        ("dupwide", wide, "duplicate key"),
    ] {
        let (_dir, file) = with_file(name, &text);
        let e = fails(&mut s, "env.apply", json!({"profile": "p", "env_file": file}));
        assert!(e.contains(needle), "{name}: {e}");
    }
    // too large
    let dir = temp("large");
    let path = dir.join("big.yaml");
    std::fs::write(&path, vec![b'#'; crate::citest_yaml::MAX_TEXT_BYTES + 1]).unwrap();
    let e = fails(&mut s, "env.apply", json!({"profile": "p", "env_file": path}));
    assert!(e.contains("larger than"), "{e}");
    // not UTF-8
    let path = dir.join("bin.yaml");
    std::fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
    let e = fails(&mut s, "env.apply", json!({"profile": "p", "env_file": path}));
    assert!(e.contains("UTF-8"), "{e}");
}

#[test]
fn server_mode_keeps_the_profile_file_in_the_workspace() {
    let dir = temp("server");
    std::fs::write(
        dir.join("bicraft.env.yaml"),
        "profiles:\n  y:\n    parameters:\n      Year: 2025\n",
    )
    .unwrap();
    let outside = temp("server-outside");
    std::fs::write(outside.join("bicraft.env.yaml"), PROFILES).unwrap();
    let mut s = Session::server(crate::sandbox::ServerConfig::new(&dir).unwrap());
    run(&mut s, "model.new", json!({"name": "M"}));
    run(
        &mut s,
        "expression.add",
        json!({"name": "Year", "expression": "2025 meta [IsParameterQuery = true, Type = \"Number\"]"}),
    );
    let e = fails(
        &mut s,
        "env.apply",
        json!({"profile": "prod", "env_file": outside.join("bicraft.env.yaml").display().to_string()}),
    );
    assert!(e.contains("server mode"), "{e}");
    let e = fails(
        &mut s,
        "env.check",
        json!({"profile": "prod", "env_file": "../x.yaml"}),
    );
    assert!(e.contains("server mode"), "{e}");
    let v = run(
        &mut s,
        "env.apply",
        json!({"profile": "y", "env_file": "bicraft.env.yaml", "dry_run": true}),
    );
    assert_eq!(v["changed"], false, "{v}");
}

#[test]
fn list_names_the_profiles() {
    let (_dir, file) = with_file("list", PROFILES);
    let mut s = fixture();
    let before = model(&s).clone();
    let v = run(&mut s, "env.list", json!({"env_file": file}));
    assert_eq!(v["count"], 2);
    assert_eq!(
        v["profiles"][0],
        json!({"name": "dev", "description": null, "parameters": 1})
    );
    assert_eq!(
        v["profiles"][1],
        json!({"name": "prod", "description": "production", "parameters": 4})
    );
    let v = run(&mut s, "env.list", json!({"env_file": {"profiles": {"a": null}}}));
    assert_eq!(v["profiles"][0]["name"], "a");
    assert_eq!(v["env_file"], Value::Null);
    assert_eq!(model(&s), &before, "read-only");
    let e = fails(&mut s, "env.list", json!({"env_file": {"profiles": 1}}));
    assert!(e.contains("profiles"), "{e}");
    let e = fails(&mut s, "env.list", json!({"profile": "x"}));
    assert!(e.contains("profile"), "{e}");
}
