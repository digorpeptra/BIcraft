//! SharePoint.Files, SharePoint.Contents and SharePoint.Tables, and the credentials of web
//! sources, through bicraft's M evaluator against the in-process test servers of
//! `bicraft_webdata::fake` (loopback only, never the internet): the bearer token per site,
//! basic and Web API key credentials, credentials scoped to their URL across redirects, lazy file
//! contents, paged list items, and the caps on hostile answers.

use std::sync::Arc;

use bicraft_data::connect::Connectors;
use bicraft_data::{CellValue, Error, SourceOptions, TableData};
use bicraft_webdata::auth::{Credential, Secret, Store};
use bicraft_webdata::fake::{self, FakeHttp, Reply, Request};
use bicraft_webdata::{ODataConnector, Settings, SharePointConnector, WebConnector};

fn opts(store: &Store, settings: Settings) -> SourceOptions {
    let mut c = Connectors::new();
    c.push(Arc::new(WebConnector::with_credentials(
        settings.clone(),
        Some(store.lookup()),
    )));
    c.push(Arc::new(ODataConnector::with_credentials(
        settings.clone(),
        Some(store.lookup()),
    )));
    c.push(Arc::new(SharePointConnector::new(settings, Some(store.lookup()))));
    SourceOptions {
        connectors: c,
        ..SourceOptions::default()
    }
}

fn load(store: &Store, m: &str) -> Result<TableData, Error> {
    bicraft_data::m::load_m_with(m, &opts(store, Settings::default()))
}

fn cell(v: &CellValue) -> String {
    match v {
        CellValue::Null => "null".into(),
        CellValue::Int(i) => i.to_string(),
        CellValue::Real(x) => x.to_string(),
        CellValue::Decimal(d) => (*d as f64 / 10_000.0).to_string(),
        CellValue::Text(t) => t.clone(),
        CellValue::Bool(b) => b.to_string(),
        CellValue::DateTime(d) => format!("d{d}"),
    }
}

fn rows(t: &TableData) -> Vec<Vec<String>> {
    (0..t.row_count())
        .map(|r| {
            t.columns()
                .iter()
                .map(|c| c.values.get(r).map(cell).unwrap_or_default())
                .collect()
        })
        .collect()
}

fn names(t: &TableData) -> Vec<String> {
    t.columns().iter().map(|c| c.name.clone()).collect()
}

fn lit(s: &str) -> String {
    format!("\"{s}\"")
}

fn site(srv: &FakeHttp) -> String {
    srv.url("/sites/team")
}

fn bearer(store: &Store, url: &str) {
    store.set(
        url,
        Credential::Bearer {
            token: Secret::new(fake::SP_TOKEN),
        },
    );
}

#[test]
fn sharepoint_files_lists_every_file_and_reads_content_when_used() {
    let srv = fake::start_sharepoint().unwrap();
    let store = Store::default();
    bearer(&store, &site(&srv));
    let t = load(
        &store,
        &format!(
            "Table.SelectColumns(SharePoint.Files({}, [ApiVersion = 15]), {{\"Name\", \"Extension\", \"Folder Path\"}})",
            lit(&site(&srv))
        ),
    )
    .unwrap();
    let base = srv.url("");
    assert_eq!(
        rows(&t),
        [
            ["a.csv", ".csv", &format!("{base}/sites/team/Shared Documents/")],
            [
                "Budget.json",
                ".json",
                &format!("{base}/sites/team/Shared Documents/")
            ],
            [
                "q1.csv",
                ".csv",
                &format!("{base}/sites/team/Shared Documents/Reports/")
            ],
            [
                "x.txt",
                ".txt",
                &format!("{base}/sites/team/Shared Documents/Reports/Old/")
            ],
        ]
        .map(|r| r.map(str::to_string))
    );
    // listing the files read no content
    assert!(srv.seen().iter().all(|r| !r.decoded().contains("$value")));
    // Learn's Example 4: one file's content, by name and folder
    let t = load(
        &store,
        &format!(
            "let Source = SharePoint.Files({s}), F = Source{{[Name = \"q1.csv\", #\"Folder Path\" = {p}]}}[Content] \
             in Table.PromoteHeaders(Csv.Document(F))",
            s = lit(&site(&srv)),
            p = lit(&format!("{base}/sites/team/Shared Documents/Reports/"))
        ),
    )
    .unwrap();
    assert_eq!(rows(&t), [["7"]]);
    let reads: Vec<String> = srv
        .seen()
        .iter()
        .map(Request::decoded)
        .filter(|t| t.contains("$value"))
        .collect();
    assert_eq!(reads.len(), 1, "{reads:?}");
    assert!(reads[0].contains("GetFileByServerRelativeUrl('/sites/team/Shared Documents/Reports/q1.csv')"));
    // every request carried the token
    assert!(
        srv.seen()
            .iter()
            .all(|r| r.header("authorization") == Some("Bearer sp-test-token"))
    );
}

#[test]
fn sharepoint_contents_navigates_folders() {
    let srv = fake::start_sharepoint().unwrap();
    let store = Store::default();
    bearer(&store, &srv.url("/"));
    let m = format!(
        "let Source = SharePoint.Contents({}, [ApiVersion = 15]), Docs = Source{{[Name = \"Shared Documents\"]}}[Content], \
         Reports = Docs{{[Name = \"Reports\"]}}[Content] in Table.SelectColumns(Reports, {{\"Name\", \"Extension\"}})",
        lit(&site(&srv))
    );
    let t = load(&store, &m).unwrap();
    assert_eq!(rows(&t), [["Old", ""], ["q1.csv", ".csv"]]);
    let t = load(
        &store,
        &format!(
            "Table.SelectColumns(SharePoint.Contents({}), {{\"Name\"}})",
            lit(&site(&srv))
        ),
    )
    .unwrap();
    assert_eq!(rows(&t), [["Shared Documents"]]);
}

#[test]
fn sharepoint_tables_reads_every_page_of_a_list() {
    let srv = fake::start_sharepoint().unwrap();
    let store = Store::default();
    bearer(&store, &site(&srv));
    let lists = load(
        &store,
        &format!(
            "Table.SelectColumns(SharePoint.Tables({}, [Implementation = \"2.0\", ViewMode = \"All\"]), {{\"Id\", \"Title\"}})",
            lit(&site(&srv))
        ),
    )
    .unwrap();
    // document libraries are lists too; hidden lists are left out
    assert_eq!(
        rows(&lists),
        [
            ["aaaaaaaa-0000-0000-0000-000000000001", "Documents"],
            [fake::SP_TASKS, "Tasks"]
        ]
    );
    let m = |mode: &str| {
        format!(
            "let Source = SharePoint.Tables({}, [Implementation = \"2.0\", ViewMode = \"{mode}\"]), \
             Items = Source{{[Id = \"{}\"]}}[Items] in Table.RemoveColumns(Items, List.Intersect({{Table.ColumnNames(Items), {{\"Owner\"}}}}))",
            lit(&site(&srv)),
            fake::SP_TASKS
        )
    };
    let t = load(&store, &m("All")).unwrap();
    assert_eq!(names(&t), ["Id", "Title", "Done"]);
    assert_eq!(
        rows(&t),
        [
            ["1", "Plan", "true"],
            ["2", "Build", "false"],
            ["3", "Ship", "false"]
        ]
    );
    let t = load(&store, &m("Default")).unwrap();
    assert_eq!(names(&t), ["Title", "Done"]);
    assert_eq!(t.row_count(), 3);
    // a lookup column is a record
    let t = load(
        &store,
        &format!(
            "let I = SharePoint.Tables({}, [Implementation = \"2.0\"]){{[Id = \"{}\"]}}[Items] in \
             Table.FromRecords({{[o = I{{0}}[Owner][Title]]}})",
            lit(&site(&srv)),
            fake::SP_TASKS
        ),
    )
    .unwrap();
    assert_eq!(rows(&t), [["Ann"]]);
}

#[test]
fn sharepoint_without_credentials_is_a_credentials_error() {
    let srv = fake::start_sharepoint().unwrap();
    let store = Store::default();
    // a token for another site is not sent
    bearer(&store, &srv.url("/sites/other"));
    let e = load(
        &store,
        &format!(
            "Table.SelectColumns(SharePoint.Files({}), {{\"Name\"}})",
            lit(&site(&srv))
        ),
    )
    .unwrap_err()
    .to_string();
    assert!(e.contains("401") && e.contains("credentials"), "{e}");
    assert!(srv.seen().iter().all(|r| r.header("authorization").is_none()));
}

/// A server that answers 200 only to the credential it expects, and redirects `/out` to `to`.
fn guarded(to: Option<String>) -> FakeHttp {
    fake::start(Arc::new(move |r: &Request, _: &str| {
        let ok = |what: &str| Reply::json(format!("[{{\"via\": \"{what}\"}}]"));
        match r.path() {
            "/basic" if r.header("authorization") == Some("Basic dXNlcjpwYXNz") => ok("basic"),
            "/key" if r.query("api_key").as_deref() == Some("k123") => ok("query"),
            "/key" if r.header("X-Api-Key") == Some("k123") => ok("header"),
            "/bearer" if r.header("authorization") == Some("Bearer tok") => ok("bearer"),
            "/open" => ok(if r.header("authorization").is_some() {
                "leaked"
            } else {
                "anonymous"
            }),
            "/out" => match &to {
                Some(t) => Reply::new(302, "text/plain", t.clone()),
                None => Reply::status(404),
            },
            "/loop" => Reply::new(302, "text/plain", "/loop"),
            _ => Reply::status(401),
        }
    }))
    .unwrap()
}

fn via(store: &Store, url: &str, options: &str) -> Result<String, Error> {
    let t = load(
        store,
        &format!(
            "Table.FromRecords(Json.Document(Web.Contents({}{options})))",
            lit(url)
        ),
    )?;
    Ok(rows(&t).concat().concat())
}

#[test]
fn web_contents_sends_the_credential_stored_for_its_url() {
    let srv = guarded(None);
    let store = Store::default();
    let e = via(&store, &srv.url("/basic"), "").unwrap_err().to_string();
    assert!(e.contains("401"), "{e}");
    store.set(
        &srv.url("/basic"),
        Credential::Basic {
            user: "user".into(),
            password: Secret::new("pass"),
        },
    );
    assert_eq!(via(&store, &srv.url("/basic"), "").unwrap(), "basic");
    store.set(
        &srv.url("/key"),
        Credential::Key {
            key: Secret::new("k123"),
            header: None,
        },
    );
    assert_eq!(
        via(&store, &srv.url("/key"), ", [ApiKeyName = \"api_key\"]").unwrap(),
        "query"
    );
    // a key without ApiKeyName nor header name is refused before anything is sent
    let n = srv.seen().len();
    let e = via(&store, &srv.url("/key"), "").unwrap_err().to_string();
    assert!(e.contains("ApiKeyName"), "{e}");
    assert_eq!(srv.seen().len(), n);
    store.set(
        &srv.url("/key"),
        Credential::Key {
            key: Secret::new("k123"),
            header: Some("X-Api-Key".into()),
        },
    );
    assert_eq!(via(&store, &srv.url("/key"), "").unwrap(), "header");
    store.set(
        &srv.url("/bearer"),
        Credential::Bearer {
            token: Secret::new("tok"),
        },
    );
    assert_eq!(via(&store, &srv.url("/bearer"), "").unwrap(), "bearer");
    // no message holds a secret
    let e = via(&store, &srv.url("/basic/x"), "").unwrap_err().to_string();
    assert!(!e.contains("pass") && !e.contains("dXNlcjpwYXNz"), "{e}");
}

#[test]
fn credentials_do_not_follow_a_redirect_to_another_source() {
    let other = guarded(None);
    let srv = guarded(Some(other.url("/open")));
    let store = Store::default();
    store.set(
        &srv.url("/"),
        Credential::Bearer {
            token: Secret::new("tok"),
        },
    );
    assert_eq!(via(&store, &srv.url("/out"), "").unwrap(), "anonymous");
    assert!(other.seen().iter().all(|r| r.header("authorization").is_none()));
    // nor a header the query set itself
    assert_eq!(
        via(
            &Store::default(),
            &srv.url("/out"),
            ", [Headers = [Authorization = \"Bearer mine\"]]"
        )
        .unwrap(),
        "anonymous"
    );
    // endless redirects stop
    let e = via(&store, &srv.url("/loop"), "").unwrap_err().to_string();
    assert!(e.contains("redirected more than"), "{e}");
}

#[test]
fn hostile_answers_are_capped() {
    // a list whose next link never ends
    let srv = fake::start(Arc::new(|r: &Request, base: &str| {
        if r.path().ends_with("/_api/web/lists") {
            return Reply::json(format!(
                "{{\"value\": [], \"odata.nextLink\": \"{base}/sites/x/_api/web/lists?p=1\"}}"
            ));
        }
        if r.path().ends_with("/RootFolder") {
            return Reply::json(format!(
                "{{\"Files\": [{{\"Name\": \"{}\"}}], \"Folders\": []}}",
                "a".repeat(5000)
            ));
        }
        Reply::status(404)
    }))
    .unwrap();
    let store = Store::default();
    let settings = Settings {
        max_pages: 5,
        max_response_bytes: 4096,
        ..Settings::default()
    };
    let o = opts(&store, settings);
    let e = bicraft_data::m::load_m_with(
        &format!(
            "Table.SelectColumns(SharePoint.Tables({}), {{\"Id\"}})",
            lit(&srv.url("/sites/x"))
        ),
        &o,
    )
    .unwrap_err()
    .to_string();
    assert!(e.contains("more than 5 pages"), "{e}");
    // the same next link is answered from the cache: at most one request a page
    assert!(srv.seen().len() <= 5);
    let e = bicraft_data::m::load_m_with(
        &format!(
            "Table.SelectColumns(SharePoint.Contents({}), {{\"Name\"}})",
            lit(&srv.url("/sites/x"))
        ),
        &o,
    )
    .unwrap_err()
    .to_string();
    assert!(e.contains("larger than"), "{e}");
}
