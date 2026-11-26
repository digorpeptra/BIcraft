//! A scripted HTTP/1.1 server on 127.0.0.1 for tests (feature `fake-server`): every request is
//! answered by a handler and recorded. [`shop`] is a small synthetic site and OData v4 service
//! (JSON and CSV files, an HTML page, a 404, entity sets `Products` and `Categories` with
//! `$select`, `$filter` on whole-number comparisons, `$orderby`, `$top` and server-driven paging
//! two entities a page), written for these tests.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// A request the server received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    /// The path and query, as sent.
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        let hex = |j: usize| b.get(j).and_then(|x| char::from(*x).to_digit(16));
        match (c, hex(i + 1), hex(i + 2)) {
            (b'%', Some(h), Some(l)) => {
                out.push(u8::try_from(h * 16 + l).unwrap_or(b'?'));
                i += 3;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl Request {
    /// The path, without the query.
    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or("")
    }

    /// A query parameter, decoded.
    pub fn query(&self, name: &str) -> Option<String> {
        let q = self.target.split_once('?')?.1;
        q.split('&').find_map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (decode(k) == name).then(|| decode(v))
        })
    }

    /// The decoded target (`/odata/Products?$top=2`).
    pub fn decoded(&self) -> String {
        decode(&self.target)
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// An answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn new(status: u16, content_type: &str, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            content_type: content_type.into(),
            body: body.into(),
        }
    }

    pub fn json(body: impl Into<String>) -> Self {
        Self::new(200, "application/json", body.into())
    }

    pub fn status(status: u16) -> Self {
        Self::new(status, "text/plain", format!("status {status}"))
    }
}

/// Answers a request; the second argument is the server's base URL (`http://127.0.0.1:port`).
pub type Handler = Arc<dyn Fn(&Request, &str) -> Reply + Send + Sync>;

/// A running server; dropping it stops it.
pub struct FakeHttp {
    addr: SocketAddr,
    seen: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FakeHttp {
    /// `http://127.0.0.1:port` followed by `path`.
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    /// The requests received so far.
    pub fn seen(&self) -> Vec<Request> {
        match self.seen.lock() {
            Ok(s) => s.clone(),
            Err(p) => p.into_inner().clone(),
        }
    }
}

impl Drop for FakeHttp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_secs(1));
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Starts a server answering with `handler`.
pub fn start(handler: Handler) -> std::io::Result<FakeHttp> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let (s2, stop2) = (Arc::clone(&seen), Arc::clone(&stop));
    let base = format!("http://{addr}");
    let thread = std::thread::spawn(move || {
        for conn in listener.incoming() {
            if stop2.load(Ordering::SeqCst) {
                break;
            }
            let Ok(conn) = conn else { continue };
            let (h, s, b) = (Arc::clone(&handler), Arc::clone(&s2), base.clone());
            std::thread::spawn(move || {
                let _ = serve(conn, &h, &s, &b);
            });
        }
    });
    Ok(FakeHttp {
        addr,
        seen,
        stop,
        thread: Some(thread),
    })
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        301 => "Moved Permanently",
        302 => "Found",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Status",
    }
}

fn serve(conn: TcpStream, h: &Handler, seen: &Mutex<Vec<Request>>, base: &str) -> std::io::Result<()> {
    conn.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut out = conn.try_clone()?;
    let mut r = BufReader::new(conn);
    let mut line = String::new();
    r.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (method, target) = (
        parts.next().unwrap_or("").to_string(),
        parts.next().unwrap_or("").to_string(),
    );
    let mut headers = Vec::new();
    loop {
        let mut l = String::new();
        if r.read_line(&mut l)? == 0 || l.trim().is_empty() || headers.len() > 100 {
            break;
        }
        if let Some((n, v)) = l.split_once(':') {
            headers.push((n.trim().to_string(), v.trim().to_string()));
        }
    }
    let len = headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0)
        .min(1 << 20);
    let mut body = vec![0; len];
    r.read_exact(&mut body)?;
    let req = Request {
        method,
        target,
        headers,
        body,
    };
    if let Ok(mut s) = seen.lock() {
        s.push(req.clone());
    }
    let reply = h(&req, base);
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        reply.status,
        reason(reply.status),
        reply.content_type,
        reply.body.len()
    );
    if (300..400).contains(&reply.status) {
        head.push_str(&format!("Location: {}\r\n", String::from_utf8_lossy(&reply.body)));
    }
    head.push_str("\r\n");
    out.write_all(head.as_bytes())?;
    out.write_all(&reply.body)?;
    out.flush()
}

/// The `$metadata` of [`shop`]'s OData service.
pub const SHOP_METADATA: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx Version="4.0" xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx">
  <edmx:DataServices>
    <Schema Namespace="Shop" xmlns="http://docs.oasis-open.org/odata/ns/edm">
      <EntityType Name="Product">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.Int32" Nullable="false"/>
        <Property Name="Name" Type="Edm.String"/>
        <Property Name="Price" Type="Edm.Decimal"/>
        <Property Name="InStock" Type="Edm.Boolean"/>
        <Property Name="Released" Type="Edm.Date"/>
        <NavigationProperty Name="Category" Type="Shop.Category"/>
      </EntityType>
      <EntityType Name="Category">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.Int32" Nullable="false"/>
        <Property Name="Name" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container">
        <EntitySet Name="Products" EntityType="Shop.Product"/>
        <EntitySet Name="Categories" EntityType="Shop.Category"/>
      </EntityContainer>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;

/// `(ID, Name, Price, InStock, Released)` of the products.
pub const PRODUCTS: &[(i64, &str, &str, bool, &str)] = &[
    (1, "Chai", "18.00", true, "2024-01-15"),
    (2, "Chang", "19.50", false, "2024-02-01"),
    (3, "Aniseed Syrup", "10.00", true, "2024-03-10"),
    (4, "Tofu", "23.25", true, "2024-04-22"),
    (5, "Konbu", "6.00", false, "2024-05-05"),
];

const PAGE: usize = 2;

fn product(p: &(i64, &str, &str, bool, &str)) -> serde_json::Value {
    serde_json::json!({
        "@odata.etag": "W/\"1\"",
        "ID": p.0, "Name": p.1, "Price": p.2, "InStock": p.3, "Released": p.4,
    })
}

/// One `Col op n` comparison of a `$filter` (whole numbers, `null`), `and`ed.
fn keep(e: &serde_json::Value, filter: &str) -> bool {
    filter.split(" and ").all(|c| {
        let c = c.trim_matches(|x| x == '(' || x == ')' || x == ' ');
        let mut it = c.split(' ');
        let (Some(col), Some(op), Some(lit)) = (it.next(), it.next(), it.next()) else {
            return false;
        };
        let v = e.get(col).and_then(|v| match v {
            serde_json::Value::String(s) => s.parse::<f64>().ok(),
            other => other.as_f64(),
        });
        if lit == "null" {
            return match op {
                "eq" => v.is_none(),
                "ne" => v.is_some(),
                _ => false,
            };
        }
        let (Some(v), Ok(n)) = (v, lit.parse::<f64>()) else {
            return op == "ne";
        };
        match op {
            "eq" => v == n,
            "ne" => v != n,
            "lt" => v < n,
            "le" => v <= n,
            "gt" => v > n,
            "ge" => v >= n,
            _ => false,
        }
    })
}

fn entity_set(r: &Request, base: &str, name: &str, all: Vec<serde_json::Value>) -> Reply {
    let mut rows = all;
    if let Some(f) = r.query("$filter") {
        rows.retain(|e| keep(e, &f));
    }
    if let Some(o) = r.query("$orderby") {
        let (col, desc) = match o.split_once(' ') {
            Some((c, d)) => (c.to_string(), d == "desc"),
            None => (o.clone(), false),
        };
        rows.sort_by(|a, b| {
            let x = a.get(&col).map(ToString::to_string).unwrap_or_default();
            let y = b.get(&col).map(ToString::to_string).unwrap_or_default();
            let o = x.len().cmp(&y.len()).then(x.cmp(&y));
            if desc { o.reverse() } else { o }
        });
    }
    if let Some(t) = r.query("$top").and_then(|t| t.parse::<usize>().ok()) {
        rows.truncate(t);
    }
    if let Some(sel) = r.query("$select") {
        let keep: Vec<&str> = sel.split(',').collect();
        for e in &mut rows {
            if let Some(o) = e.as_object_mut() {
                o.retain(|k, _| k.starts_with('@') || keep.contains(&k.as_str()));
            }
        }
    }
    let skip = r
        .query("$skiptoken")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    let page: Vec<serde_json::Value> = rows.iter().skip(skip).take(PAGE).cloned().collect();
    let mut body = serde_json::json!({
        "@odata.context": format!("{base}/odata/$metadata#{name}"),
        "value": page,
    });
    if skip + PAGE < rows.len() {
        let mut next = r.target.clone();
        next = next.split("&$skiptoken=").next().unwrap_or(&next).to_string();
        next = next.split("?$skiptoken=").next().unwrap_or(&next).to_string();
        let sep = if next.contains('?') { '&' } else { '?' };
        body["@odata.nextLink"] = serde_json::json!(format!("{base}{next}{sep}$skiptoken={}", skip + PAGE));
    }
    Reply::new(200, "application/json;odata.metadata=minimal", body.to_string())
}

/// The synthetic site: `/data.json`, `/data.csv`, `/page.html`, `/missing` (404), `/private`
/// (401), and the OData service at `/odata/`.
pub fn shop(r: &Request, base: &str) -> Reply {
    match r.path() {
        "/data.json" => {
            Reply::json(r#"[{"id": 1, "name": "Ann", "score": 9.5}, {"id": 2, "name": "Bob", "score": 7}]"#)
        }
        "/nested.json" => Reply::json(r#"{"orders": [{"id": 1}, {"id": 2}], "meta": {"count": 2}}"#),
        "/data.csv" => Reply::new(200, "text/csv", "id,name\r\n1,Ann\r\n2,Bob\r\n"),
        "/page.html" => Reply::new(
            200,
            "text/html",
            "<!doctype html><html><body><table></table></body></html>",
        ),
        "/echo" => Reply::new(200, "application/octet-stream", r.body.clone()),
        "/private" => Reply::status(401),
        "/odata" | "/odata/" => Reply::new(
            200,
            "application/json;odata.metadata=minimal",
            serde_json::json!({
                "@odata.context": format!("{base}/odata/$metadata"),
                "value": [
                    {"name": "Products", "kind": "EntitySet", "url": "Products"},
                    {"name": "Categories", "kind": "EntitySet", "url": "Categories"},
                    {"name": "Me", "kind": "Singleton", "url": "Me"},
                ],
            })
            .to_string(),
        ),
        "/odata/$metadata" => Reply::new(200, "application/xml", SHOP_METADATA),
        "/odata/Products" => entity_set(r, base, "Products", PRODUCTS.iter().map(product).collect()),
        "/odata/Categories" => entity_set(
            r,
            base,
            "Categories",
            vec![
                serde_json::json!({"ID": 1, "Name": "Beverages"}),
                serde_json::json!({"ID": 2, "Name": "Condiments"}),
            ],
        ),
        _ => Reply::status(404),
    }
}

/// A server running [`shop`].
pub fn start_shop() -> std::io::Result<FakeHttp> {
    start(Arc::new(shop))
}

/// The bearer token [`sharepoint`] accepts.
pub const SP_TOKEN: &str = "sp-test-token";
/// The id of [`sharepoint`]'s `Tasks` list.
pub const SP_TASKS: &str = "11111111-2222-3333-4444-555555555555";

/// The files of [`sharepoint`]'s site: (server-relative URL, content).
pub const SP_FILES: &[(&str, &str)] = &[
    (
        "/sites/team/Shared Documents/a.csv",
        "id,name\r\n1,Ann\r\n2,Bob\r\n",
    ),
    ("/sites/team/Shared Documents/Budget.json", r#"{"total": 42}"#),
    ("/sites/team/Shared Documents/Reports/q1.csv", "id\r\n7\r\n"),
    ("/sites/team/Shared Documents/Reports/Old/x.txt", "old"),
    ("/sites/team/Style Library/hidden.css", "x"),
];

fn sp_folder(path: &str) -> Option<serde_json::Value> {
    let prefix = format!("{path}/");
    let mut files = Vec::new();
    let mut folders: Vec<String> = Vec::new();
    for (f, body) in SP_FILES {
        let Some(rest) = f.strip_prefix(&prefix) else {
            continue;
        };
        match rest.split_once('/') {
            None => files.push(serde_json::json!({
                "Name": rest,
                "ServerRelativeUrl": f,
                "Length": body.len().to_string(),
                "TimeCreated": "2024-01-15T09:30:00Z",
                "TimeLastModified": "2024-02-01T10:00:00Z",
            })),
            Some((dir, _)) => {
                let d = format!("{path}/{dir}");
                if !folders.contains(&d) {
                    folders.push(d);
                }
            }
        }
    }
    if !SP_FILES.iter().any(|(f, _)| f.starts_with(&prefix)) {
        return None;
    }
    let folders: Vec<serde_json::Value> = folders
        .iter()
        .map(|d| {
            serde_json::json!({
                "Name": d.rsplit('/').next().unwrap_or(""),
                "ServerRelativeUrl": d,
                "TimeCreated": "2024-01-10T08:00:00Z",
                "TimeLastModified": "2024-01-11T08:00:00Z",
            })
        })
        .collect();
    Some(serde_json::json!({
        "Name": path.rsplit('/').next().unwrap_or(""),
        "ServerRelativeUrl": path,
        "Files": files,
        "Folders": folders,
    }))
}

/// A synthetic SharePoint site at `/sites/team` (REST API, JSON without metadata), written for
/// these tests: the document libraries `Documents` and the hidden `Style Library`, folders and
/// files ([`SP_FILES`]), the list `Tasks` (three items, two a page, its default view) and the
/// hidden list `Hidden`. Every request needs `Authorization: Bearer` [`SP_TOKEN`], else 401.
pub fn sharepoint(r: &Request, base: &str) -> Reply {
    if r.header("authorization") != Some(format!("Bearer {SP_TOKEN}").as_str()) {
        return Reply::new(
            401,
            "application/json",
            r#"{"error":{"code":"-2147024891","message":{"lang":"en-US","value":"Access denied."}}}"#,
        );
    }
    let path = decode(r.path());
    let Some(api) = path.strip_prefix("/sites/team/_api/web") else {
        return Reply::status(404);
    };
    let tasks = format!("/lists(guid'{SP_TASKS}')");
    let json = |v: serde_json::Value| Reply::json(v.to_string());
    if api == "/lists" {
        return json(serde_json::json!({"value": [
            {"Id": "aaaaaaaa-0000-0000-0000-000000000001", "Title": "Documents", "Hidden": false,
             "BaseTemplate": 101, "RootFolder": {"ServerRelativeUrl": "/sites/team/Shared Documents"}},
            {"Id": "aaaaaaaa-0000-0000-0000-000000000002", "Title": "Style Library", "Hidden": true,
             "BaseTemplate": 101, "RootFolder": {"ServerRelativeUrl": "/sites/team/Style Library"}},
            {"Id": SP_TASKS, "Title": "Tasks", "Hidden": false, "BaseTemplate": 100,
             "RootFolder": {"ServerRelativeUrl": "/sites/team/Lists/Tasks"}},
            {"Id": "aaaaaaaa-0000-0000-0000-000000000003", "Title": "Hidden", "Hidden": true,
             "BaseTemplate": 100, "RootFolder": {"ServerRelativeUrl": "/sites/team/Lists/Hidden"}},
        ]}));
    }
    if api == "/RootFolder" {
        return json(
            serde_json::json!({"Name": "", "ServerRelativeUrl": "/sites/team", "Files": [],
            "Folders": [{"Name": "Shared Documents", "ServerRelativeUrl": "/sites/team/Shared Documents",
                         "TimeCreated": "2024-01-01T00:00:00Z", "TimeLastModified": "2024-01-02T00:00:00Z"}]}),
        );
    }
    if let Some(p) = api
        .strip_prefix("/GetFolderByServerRelativeUrl('")
        .and_then(|p| p.strip_suffix("')"))
    {
        return match sp_folder(&p.replace("''", "'")) {
            Some(v) => json(v),
            None => Reply::new(
                404,
                "application/json",
                r#"{"error":{"message":{"value":"File Not Found."}}}"#,
            ),
        };
    }
    if let Some(p) = api
        .strip_prefix("/GetFileByServerRelativeUrl('")
        .and_then(|p| p.strip_suffix("')/$value"))
    {
        let p = p.replace("''", "'");
        return match SP_FILES.iter().find(|(f, _)| *f == p) {
            Some((_, body)) => Reply::new(200, "application/octet-stream", *body),
            None => Reply::status(404),
        };
    }
    if api == format!("{tasks}/DefaultView/ViewFields") {
        return json(serde_json::json!({"Items": ["Title", "Done"]}));
    }
    if api == format!("{tasks}/items") {
        let all = [
            serde_json::json!({"odata.type": "SP.Data.TasksListItem", "Id": 1, "Title": "Plan", "Done": true, "Owner": {"Title": "Ann"}}),
            serde_json::json!({"odata.type": "SP.Data.TasksListItem", "Id": 2, "Title": "Build", "Done": false, "Owner": {"Title": "Bob"}}),
            serde_json::json!({"odata.type": "SP.Data.TasksListItem", "Id": 3, "Title": "Ship", "Done": false, "Owner": null}),
        ];
        let select = r.query("$select");
        let skip = r
            .query("$skiptoken")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0);
        let page: Vec<serde_json::Value> = all
            .iter()
            .skip(skip)
            .take(2)
            .map(|v| match &select {
                Some(s) => {
                    let keep: Vec<&str> = s.split(',').collect();
                    let o: serde_json::Map<String, serde_json::Value> = v
                        .as_object()
                        .into_iter()
                        .flatten()
                        .filter(|(k, _)| keep.contains(&k.as_str()))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    serde_json::Value::Object(o)
                }
                None => v.clone(),
            })
            .collect();
        let mut body = serde_json::json!({ "value": page });
        if skip + 2 < all.len() {
            let mut next = format!("{base}/sites/team/_api/web{tasks}/items?$skiptoken={}", skip + 2);
            if let Some(s) = &select {
                next.push_str(&format!("&$select={s}"));
            }
            body["odata.nextLink"] = serde_json::json!(next);
        }
        return json(body);
    }
    Reply::status(404)
}

/// A server running [`sharepoint`].
pub fn start_sharepoint() -> std::io::Result<FakeHttp> {
    start(Arc::new(sharepoint))
}
