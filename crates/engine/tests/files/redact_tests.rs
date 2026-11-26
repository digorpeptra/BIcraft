//! M11.37: `file.redact` on PBIX and PBIP files built here from synthetic models: no original
//! secret survives anywhere in the written bytes (each part, the decompressed DataModel, every
//! file of a project; UTF-8 and UTF-16LE), the copy opens again, placeholders are stable.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

const SECRETS: &[&str] = &[
    "alice-private-share",
    "param-secret-host.corp",
    "WarehouseSecretDb",
    "secret-corp.example",
    "tok-SECRET-123",
    "hunter2pass",
    "SecretDsn",
];

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// Every byte of a written copy: the files, each package part, the decompressed DataModel.
fn all_bytes(path: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![path.to_path_buf()];
    while let Some(p) = stack.pop() {
        if p.is_dir() {
            for e in std::fs::read_dir(&p).unwrap() {
                stack.push(e.unwrap().path());
            }
            continue;
        }
        let bytes = std::fs::read(&p).unwrap();
        if matches!(p.extension().and_then(|e| e.to_str()), Some("pbix" | "pbit")) {
            let pkg = bicraft_opc::Package::read_bytes(&bytes).unwrap();
            for part in &pkg.parts {
                if part.name == bicraft_pbix::DATA_MODEL {
                    let (_, abf) = bicraft_pbix::read_data_model_keeping(
                        &part.data,
                        &bicraft_pbix::DataModelLimits::default(),
                    )
                    .unwrap();
                    out.push(("decompressed DataModel".into(), abf.unwrap()));
                }
                out.push((part.name.clone(), part.data.clone()));
            }
        }
        out.push((p.display().to_string(), bytes));
    }
    out
}

fn assert_clean(path: &Path, secrets: &[&str]) {
    let all = all_bytes(path);
    assert!(!all.is_empty());
    for (what, bytes) in &all {
        for s in secrets {
            assert!(
                !contains(bytes, s.as_bytes()) && !contains(bytes, &utf16(s)),
                "`{s}` survives in {what}"
            );
        }
    }
}

fn assert_found(path: &Path, secret: &str) -> bool {
    all_bytes(path)
        .iter()
        .any(|(_, b)| contains(b, secret.as_bytes()) || contains(b, &utf16(secret)))
}

/// A PBIX with a CSV table under a "private" folder and shared queries holding a server
/// parameter, a database, a URL with a bearer token and an ODBC connection string.
fn shop_pbix(dir: &Path, host_in_data: bool) -> PathBuf {
    let share = dir.join("alice-private-share");
    std::fs::create_dir_all(&share).unwrap();
    let csv = share.join("sales.csv");
    let host = if host_in_data {
        "param-secret-host.corp"
    } else {
        "Marseille"
    };
    std::fs::write(
        &csv,
        format!("City,Amount\nParis-Secret-City,10\nLyon,20\n{host},30\n"),
    )
    .unwrap();
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    let m = format!(
        "let Source = Csv.Document(File.Contents(\"{}\"), [Delimiter = \",\", Encoding = 65001]), \
         Promoted = Table.PromoteHeaders(Source), Typed = Table.TransformColumnTypes(Promoted, {{{{\"Amount\", Int64.Type}}}}) in Typed",
        csv.display()
    );
    run(
        &mut s,
        "table.import",
        json!({"name": "Sales", "m": m, "data_root": dir}),
    );
    for (name, e) in [
        (
            "Server",
            "\"param-secret-host.corp\" meta [IsParameterQuery = true, Type = \"Text\", IsParameterQueryRequired = true]",
        ),
        (
            "Warehouse",
            "let Source = Sql.Database(Server, \"WarehouseSecretDb\") in Source",
        ),
        (
            "Api",
            "let Source = Json.Document(Web.Contents(\"https://api.secret-corp.example/v1/data\", [Headers = [Authorization = \"Bearer tok-SECRET-123\"]])) in Source",
        ),
        (
            "Legacy",
            "let Source = Odbc.DataSource(\"dsn=SecretDsn;uid=bob;pwd=hunter2pass\", [HierarchicalNavigation = true]) in Source",
        ),
    ] {
        run(&mut s, "expression.add", json!({"name": name, "expression": e}));
    }
    let pbix = dir.join("Shop.pbix");
    run(&mut s, "file.save_as", json!({"path": pbix, "data_root": dir}));
    pbix
}

fn rows(s: &mut Session, table: &str) -> Value {
    run(s, "table.rows", json!({"table": table}))
}

#[test]
fn a_redacted_pbix_keeps_no_secret_and_its_data() {
    let dir = TempDir::new("redact-pbix");
    let pbix = shop_pbix(&dir, false);
    for s in SECRETS {
        assert!(assert_found(&pbix, s), "the fixture holds `{s}`");
    }
    let out = dir.join("Shop (redacted).pbix");
    let mut s = Session::new();
    let r = run(&mut s, "file.redact", json!({"path": pbix, "out": out}));
    assert!(r["redacted"].as_u64().unwrap() >= 5, "{r}");
    assert_eq!(r["verified"]["survivors"], 0);
    assert!(
        !r.to_string().contains("hunter2pass"),
        "the result names no secret"
    );
    assert_clean(&out, SECRETS);
    // the data is kept, the query's path is a placeholder, the copy opens
    let mut back = Session::new();
    run(&mut back, "file.open", json!({"path": out}));
    let page = rows(&mut back, "Sales");
    assert_eq!(page["total_rows"], 3, "{page}");
    assert!(
        page.to_string().contains("Paris-Secret-City"),
        "data kept without `data`"
    );
    let m = back.model.as_ref().unwrap();
    let q = m.table("Sales").unwrap().partitions[0]
        .expression
        .clone()
        .unwrap();
    assert!(q.contains("C:\\redacted\\path_") && q.contains(".csv"), "{q}");
    let warehouse = &m
        .expressions
        .iter()
        .find(|e| e.name == "Warehouse")
        .unwrap()
        .expression;
    assert!(
        warehouse.contains("Sql.Database(Server, \"database_"),
        "{warehouse}"
    );
    // stable: the same input gives the same placeholders
    let again = dir.join("again.pbix");
    let r2 = run(&mut s, "file.redact", json!({"path": pbix, "out": again}));
    assert_eq!(r["placeholders"], r2["placeholders"]);
    // the open session is not touched
    assert!(s.model.is_none());

    // with data: the text values go too
    let out2 = dir.join("Shop (data).pbix");
    let r = run(
        &mut s,
        "file.redact",
        json!({"path": pbix, "out": out2, "data": true}),
    );
    assert!(r["data_values"].as_u64().unwrap() >= 3, "{r}");
    assert_clean(&out2, &[SECRETS, &["Paris-Secret-City", "Marseille"]].concat());
    let mut back = Session::new();
    run(&mut back, "file.open", json!({"path": out2}));
    let page = rows(&mut back, "Sales");
    assert_eq!(page["total_rows"], 3);
    assert!(page.to_string().contains("value_"), "{page}");
}

#[test]
fn a_secret_left_in_the_data_fails_and_deletes_the_copy() {
    let dir = TempDir::new("redact-survivor");
    let pbix = shop_pbix(&dir, true);
    let out = dir.join("red.pbix");
    let mut s = Session::new();
    let e = s
        .execute("file.redact", &json!({"path": pbix, "out": out}))
        .unwrap_err();
    assert!(e.contains("survives") && e.contains("data: true"), "{e}");
    assert!(!out.exists(), "the copy is deleted");
    run(
        &mut s,
        "file.redact",
        json!({"path": pbix, "out": out, "data": true}),
    );
    assert_clean(&out, SECRETS);
}

#[test]
fn a_redacted_project_keeps_no_secret_in_any_file() {
    let dir = TempDir::new("redact-pbip");
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Hub"}));
    run(
        &mut s,
        "table.add",
        json!({"name": "Files", "partition": {"source_type": "m",
            "expression": "let Source = Folder.Files(\"\\\\fileserver-secret\\alice-private-share\\exports\") in Source"}}),
    );
    run(
        &mut s,
        "expression.add",
        json!({"name": "Api", "expression": "Web.Contents(\"https://api.secret-corp.example/v1\", [ApiKeyName = \"k\", Headers = [Authorization = \"Bearer tok-SECRET-123\"]])"}),
    );
    run(&mut s, "report.new", json!({}));
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    run(
        &mut s,
        "textbox.add",
        json!({"page": page, "text": "Docs: https://wiki.secret-corp.example/page"}),
    );
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    run(&mut s, "file.save_as", json!({"path": src.join("Hub.pbip")}));
    let red = dir.join("red");
    std::fs::create_dir_all(&red).unwrap();
    let r = run(
        &mut s,
        "file.redact",
        json!({"path": src.join("Hub.pbip"), "out": red.join("Hub.pbip")}),
    );
    assert_eq!(r["verified"]["survivors"], 0, "{r}");
    assert!(assert_found(&src, "fileserver-secret"));
    assert_clean(
        &red,
        &[
            "fileserver-secret",
            "alice-private-share",
            "secret-corp.example",
            "tok-SECRET-123",
        ],
    );
    let mut back = Session::new();
    run(&mut back, "file.open", json!({"path": red.join("Hub.pbip")}));
    let rep = serde_json::to_string(back.report.as_ref().unwrap()).unwrap();
    assert!(
        rep.contains("redacted.invalid"),
        "the text box's link is a placeholder"
    );
}

#[test]
fn bad_parameters_are_errors() {
    let dir = TempDir::new("redact-bad");
    let mut s = Session::new();
    for p in [
        json!({}),
        json!({"path": 1, "out": "x.pbix"}),
        json!({"path": "a.pbix"}),
        json!({"path": "a.pbix", "out": "b.pbip"}),
        json!({"path": "a.pbip", "out": "b.pbix"}),
        json!({"path": "a.txt", "out": "b.txt"}),
        json!({"path": "a.pbix", "out": "a.pbix"}),
        json!({"path": dir.join("missing.pbix"), "out": dir.join("o.pbix")}),
        json!({"path": "", "out": ""}),
        json!({"path": "a.pbix", "out": "b.pbix", "data": "yes"}),
        json!({"path": "a.pbix", "out": "b.pbix", "extra": 1}),
    ] {
        assert!(s.execute("file.redact", &p).is_err(), "{p}");
    }
}
