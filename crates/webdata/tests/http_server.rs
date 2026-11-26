//! `Web.Contents` and `OData.Feed` through bicraft's M evaluator, against the in-process test
//! server of `bicraft_webdata::fake` (never the internet): options, errors, caps, the cache, the
//! OData navigation table, folding into the system query options and paging.

use std::sync::Arc;

use bicraft_data::connect::Connectors;
use bicraft_data::{CellValue, Error, SourceOptions, TableData};
use bicraft_webdata::fake::{self, FakeHttp, Reply, Request};
use bicraft_webdata::{ODataConnector, Settings, WebConnector};

fn opts(settings: Settings) -> SourceOptions {
    let mut c = Connectors::new();
    c.push(Arc::new(WebConnector::new(settings.clone())));
    c.push(Arc::new(ODataConnector::new(settings)));
    SourceOptions {
        connectors: c,
        ..SourceOptions::default()
    }
}

fn load(m: &str) -> Result<TableData, Error> {
    bicraft_data::m::load_m_with(m, &opts(Settings::default()))
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

fn shop() -> FakeHttp {
    fake::start_shop().unwrap()
}

fn lit(s: &str) -> String {
    format!("\"{s}\"")
}

/// The decoded targets of the requests to `path`.
fn targets(srv: &FakeHttp, path: &str) -> Vec<String> {
    srv.seen()
        .iter()
        .filter(|r| r.path() == path)
        .map(Request::decoded)
        .collect()
}

#[test]
fn json_and_csv_over_http() {
    let srv = shop();
    let t = load(&format!(
        "Table.FromRecords(Json.Document(Web.Contents({})))",
        lit(&srv.url("/data.json"))
    ))
    .unwrap();
    assert_eq!(names(&t), ["id", "name", "score"]);
    assert_eq!(rows(&t), [["1", "Ann", "9.5"], ["2", "Bob", "7"]]);
    let t = load(&format!(
        "Table.PromoteHeaders(Csv.Document(Web.Contents({}), [Delimiter=\",\", Encoding=65001]))",
        lit(&srv.url("/data.csv"))
    ))
    .unwrap();
    assert_eq!(names(&t), ["id", "name"]);
    assert_eq!(rows(&t), [["1", "Ann"], ["2", "Bob"]]);
}

#[test]
fn options_build_the_request() {
    let srv = fake::start(Arc::new(|r: &Request, _: &str| {
        Reply::json(format!(
            "[{{\"target\": \"{}\", \"accept\": \"{}\", \"method\": \"{}\", \"body\": \"{}\"}}]",
            r.decoded(),
            r.header("accept").unwrap_or(""),
            r.method,
            String::from_utf8_lossy(&r.body)
        ))
    }))
    .unwrap();
    let t = load(&format!(
        "Table.FromRecords(Json.Document(Web.Contents({}, [RelativePath = \"v1/orders\", \
         Query = [top = \"10\", tag = {{\"a\", \"b c\"}}], Headers = [Accept = \"application/json\"], \
         Timeout = #duration(0, 0, 0, 30)])))",
        lit(&srv.url("/api"))
    ))
    .unwrap();
    assert_eq!(
        rows(&t),
        [[
            "/api/v1/orders?top=10&tag=a&tag=b c",
            "application/json",
            "GET",
            ""
        ]]
    );
    // Content makes it a POST
    let t = load(&format!(
        "Table.FromRecords(Json.Document(Web.Contents({}, [Content = Text.ToBinary(\"x=1\")])))",
        lit(&srv.url("/post"))
    ))
    .unwrap();
    assert_eq!(rows(&t), [["/post", "*/*", "POST", "x=1"]]);
}

#[test]
fn status_errors_are_catchable_and_manual_status_reads_the_body() {
    let srv = shop();
    let url = lit(&srv.url("/missing"));
    let e = load(&format!("Table.FromRecords(Json.Document(Web.Contents({url})))")).unwrap_err();
    let text = e.to_string();
    assert!(
        text.contains("Web.Contents failed to get contents from") && text.contains("(404)"),
        "{text}"
    );
    let t = load(&format!(
        "#table({{\"r\"}}, {{{{try Text.FromBinary(Web.Contents({url})) catch (e) => e[Reason]}}}})"
    ))
    .unwrap();
    assert_eq!(rows(&t), [["DataSource.Error"]]);
    let t = load(&format!(
        "#table({{\"r\"}}, {{{{Text.FromBinary(Web.Contents({url}, [ManualStatusHandling = {{404}}]))}}}})"
    ))
    .unwrap();
    assert_eq!(rows(&t), [["status 404"]]);
}

#[test]
fn bodies_are_capped_and_requests_cached() {
    let srv = shop();
    let small = Settings {
        max_response_bytes: 16,
        ..Settings::default()
    };
    let r = bicraft_data::m::load_m_with(
        &format!(
            "Table.FromRecords(Json.Document(Web.Contents({})))",
            lit(&srv.url("/data.json"))
        ),
        &opts(small),
    );
    assert!(matches!(r, Err(Error::LimitExceeded(_))), "{r:?}");
    // read twice in one query: one request
    let before = targets(&srv, "/data.csv").len();
    let url = lit(&srv.url("/data.csv"));
    load(&format!(
        "#table({{\"a\", \"b\"}}, {{{{Binary.Length(Web.Contents({url})), Binary.Length(Web.Contents({url}))}}}})"
    ))
    .unwrap();
    assert_eq!(targets(&srv, "/data.csv").len(), before + 1);
}

#[test]
fn refusals() {
    let srv = shop();
    // not allowed: nothing reaches the network
    let r = bicraft_data::m::load_m_with(
        &format!(
            "Table.FromRecords(Json.Document(Web.Contents({})))",
            lit(&srv.url("/data.json"))
        ),
        &SourceOptions::default(),
    );
    assert!(matches!(r, Err(Error::ExternalSource(_))), "{r:?}");
    assert!(srv.seen().is_empty());
    for m in [
        "Web.Contents(\"file:///etc/passwd\")".to_string(),
        format!("Web.Contents({}, [Unknown = 1])", lit(&srv.url("/"))),
        format!("Web.Contents({}, [Headers = [Host = \"x\"]])", lit(&srv.url("/"))),
    ] {
        let r = load(&format!("#table({{\"a\"}}, {{{{Binary.Length({m})}}}})"));
        assert!(matches!(r, Err(Error::UnsupportedSource(_))), "{m}: {r:?}");
    }
    // ApiKeyName needs a stored Web API key: nothing is sent without one
    let before = srv.seen().len();
    let r = load(&format!(
        "Binary.Length(Web.Contents({}, [ApiKeyName = \"k\"]))",
        lit(&srv.url("/"))
    ));
    assert!(
        matches!(&r, Err(Error::Connector(m)) if m.contains("Web API key")),
        "{r:?}"
    );
    assert_eq!(srv.seen().len(), before);
    let r = load(&format!(
        "Table.FirstN(OData.Feed({}), 1)",
        lit(&srv.url("/private"))
    ));
    assert!(matches!(r, Err(Error::Connector(_))), "{r:?}");
}

fn feed(srv: &FakeHttp) -> String {
    format!(
        "OData.Feed({}, null, [Implementation=\"2.0\"])",
        lit(&srv.url("/odata/"))
    )
}

#[test]
fn odata_navigation_table_lists_entity_sets() {
    let srv = shop();
    let t = load(&format!(
        "Table.SelectColumns({}, {{\"Name\", \"Signature\", \"Kind\"}})",
        feed(&srv)
    ))
    .unwrap();
    assert_eq!(
        rows(&t),
        [["Products", "table", "Table"], ["Categories", "table", "Table"]]
    );
}

#[test]
fn odata_reads_every_page_with_the_metadata_types() {
    let srv = shop();
    let t = load(&format!(
        "let Source = {}, P = Source{{[Name=\"Products\",Signature=\"table\"]}}[Data] in P",
        feed(&srv)
    ))
    .unwrap();
    assert_eq!(names(&t), ["ID", "Name", "Price", "InStock", "Released"]);
    let r = rows(&t);
    assert_eq!(r.len(), 5);
    assert_eq!(r[1][..4], ["2", "Chang", "19.5", "false"]);
    // three pages of two
    let p = targets(&srv, "/odata/Products");
    assert_eq!(p.len(), 3, "{p:?}");
    assert!(p[2].ends_with("$skiptoken=4"), "{p:?}");
    // the metadata is read once
    assert_eq!(targets(&srv, "/odata/$metadata").len(), 1);
}

#[test]
fn odata_folds_select_filter_sort_and_top() {
    let srv = shop();
    let t = load(&format!(
        "let Source = {}, P = Source{{[Name=\"Products\",Signature=\"table\"]}}[Data], \
         S = Table.SelectColumns(P, {{\"ID\", \"Price\"}}), \
         F = Table.SelectRows(S, each [ID] > 1 and [Price] <> null), \
         O = Table.Sort(F, {{{{\"ID\", Order.Descending}}}}), \
         T = Table.FirstN(O, 2) in T",
        feed(&srv)
    ))
    .unwrap();
    assert_eq!(rows(&t), [["5", "6"], ["4", "23.25"]]);
    let p = targets(&srv, "/odata/Products");
    assert_eq!(
        p,
        ["/odata/Products?$select=ID,Price&$filter=(ID gt 1) and (Price ne null)&$orderby=ID desc&$top=2"]
    );
}

#[test]
fn odata_text_filters_run_locally() {
    let srv = shop();
    let t = load(&format!(
        "let Source = {}, P = Source{{[Name=\"Products\",Signature=\"table\"]}}[Data], \
         F = Table.SelectRows(P, each [Name] = \"Tofu\") in Table.SelectColumns(F, {{\"ID\"}})",
        feed(&srv)
    ))
    .unwrap();
    assert_eq!(rows(&t), [["4"]]);
    assert!(
        targets(&srv, "/odata/Products")
            .iter()
            .all(|t| !t.contains("$filter"))
    );
}

#[test]
fn odata_no_rows_asked_reads_only_the_metadata() {
    let srv = shop();
    let t = load(&format!(
        "Table.FirstN({}{{[Name=\"Categories\",Signature=\"table\"]}}[Data], 0)",
        feed(&srv)
    ))
    .unwrap();
    assert_eq!(names(&t), ["ID", "Name"]);
    assert_eq!(t.row_count(), 0);
    assert!(targets(&srv, "/odata/Categories").is_empty());
}

#[test]
fn odata_feed_of_an_entity_set_is_a_table() {
    let srv = shop();
    let t = load(&format!("OData.Feed({})", lit(&srv.url("/odata/Categories")))).unwrap();
    assert_eq!(names(&t), ["ID", "Name"]);
    assert_eq!(rows(&t), [["1", "Beverages"], ["2", "Condiments"]]);
}
