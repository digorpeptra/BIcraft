//! `SharePoint.Files`, `SharePoint.Contents` and `SharePoint.Tables` over the SharePoint REST
//! API (`{site}/_api/web/…`, JSON), with the credential stored for the site's URL (a bearer
//! token for SharePoint Online, basic for a server that accepts it, [`crate::auth`]).
//!
//! What the functions return follows Microsoft Learn (*SharePoint.Files*, *SharePoint.Contents*,
//! *SharePoint.Tables*, and the SharePoint folder and list connector pages):
//!
//! - `SharePoint.Files(url, options)`: one row per file of the site's document libraries,
//!   subfolders included, columns `Content` (the file's bytes, read only when used), `Name`,
//!   `Extension`, `Date accessed` (null), `Date modified`, `Date created`, `Attributes`, `Folder
//!   Path` (the absolute URL of the folder, ending with `/`);
//! - `SharePoint.Contents(url, options)`: the same columns for the folders and files of the
//!   site's root folder, a folder's `Content` being its own contents table (read when used);
//! - `SharePoint.Tables(url, options)`: one row per list of the site (`Id`, `Title`, `Items`),
//!   `Items` the list's items (read when used, every page), columns of type `any`.
//!
//! Options: `ApiVersion` (14, 15 or `"Auto"`), `Implementation` (null or `"2.0"`), `ViewMode`
//! (`"All"` or `"Default"`, implementation 2.0), `DisableAppendNoteColumns`. BIcraft always uses
//! the REST API of version 15 (`_api`); version 14's other endpoints are not used. See
//! docs/formats/sharepoint.md.

use std::sync::Arc;

use bicraft_data::connect::{
    Arg, ConnectError, ConnectErrorKind, Lazy, LazyContent, LazyRows, NavEntry, RemoteColumn, RemoteQuery,
    RemoteRows, RemoteType, RemoteValue, Source,
};
use serde_json::Value;

use crate::http::{self, Client, Request, source, unsupported};

const ACCEPT: &str = "application/json;odata=nometadata";
/// Deepest folder nesting `SharePoint.Files` follows.
const MAX_DEPTH: usize = 64;
/// Deepest JSON nesting an item's value is converted to (deeper: text).
const MAX_VALUE_DEPTH: usize = 16;

/// Which function a call is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Function {
    Files,
    Contents,
    Tables,
}

impl Function {
    pub fn of(name: &str) -> Option<Self> {
        Some(match name {
            "SharePoint.Files" => Function::Files,
            "SharePoint.Contents" => Function::Contents,
            "SharePoint.Tables" => Function::Tables,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Function::Files => "SharePoint.Files",
            Function::Contents => "SharePoint.Contents",
            Function::Tables => "SharePoint.Tables",
        }
    }
}

/// A call's checked arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub function: Function,
    /// The site URL, without a trailing `/`.
    pub site: String,
    /// `ViewMode = "Default"`: a list's items with its default view's columns only.
    pub default_view: bool,
}

/// Checks a call's arguments.
pub fn options(function: &str, args: &[Arg]) -> Result<Options, ConnectError> {
    let Some(f) = Function::of(function) else {
        return Err(unsupported(format!("`{function}` is not a SharePoint function")));
    };
    let name = f.name();
    let site = match args.first() {
        Some(Arg::Text(u)) => u.trim().to_string(),
        _ => return Err(unsupported(format!("{name}: the site URL must be text"))),
    };
    if args.len() > 2 {
        return Err(unsupported(format!(
            "{name} takes a site URL and an options record"
        )));
    }
    http::check_url(&site)?;
    if site.contains(['?', '#']) {
        return Err(unsupported(format!(
            "{name}: the site URL can't contain a query string or fragment"
        )));
    }
    let mut out = Options {
        function: f,
        site: site.trim_end_matches('/').to_string(),
        default_view: false,
    };
    let mut v2 = false;
    let mut api14 = false;
    let mut view_mode = false;
    match args.get(1) {
        None | Some(Arg::Null) => {}
        Some(Arg::Record(fields)) => {
            for (n, v) in fields {
                if matches!(v, Arg::Null) {
                    continue;
                }
                match (n.as_str(), v) {
                    ("ApiVersion", Arg::Number(x)) if *x == 14.0 || *x == 15.0 => api14 = *x == 14.0,
                    ("ApiVersion", Arg::Text(t)) if t.eq_ignore_ascii_case("auto") => {}
                    ("ApiVersion", _) => {
                        return Err(unsupported(format!(
                            "{name}: `ApiVersion` must be 14, 15 or \"Auto\""
                        )));
                    }
                    ("Implementation", Arg::Text(t)) if t == "2.0" && f != Function::Files => v2 = true,
                    ("Implementation", _) => {
                        return Err(unsupported(format!(
                            "{name}: `Implementation` must be \"2.0\" or null"
                        )));
                    }
                    ("ViewMode", Arg::Text(t)) if f == Function::Tables && (t == "All" || t == "Default") => {
                        out.default_view = t == "Default";
                        view_mode = true;
                    }
                    ("ViewMode", _) => {
                        return Err(unsupported(format!(
                            "{name}: `ViewMode` must be \"All\" or \"Default\" (SharePoint.Tables only)"
                        )));
                    }
                    ("DisableAppendNoteColumns", Arg::Logical(_)) if f == Function::Tables => {}
                    (other, _) => {
                        return Err(unsupported(format!(
                            "{name}: the option `{other}` is not supported by bicraft"
                        )));
                    }
                }
            }
        }
        Some(_) => return Err(unsupported(format!("{name}: the options must be a record"))),
    }
    if v2 && api14 {
        return Err(unsupported(format!(
            "{name}: API version 14 isn't supported with implementation 2.0"
        )));
    }
    if view_mode && !v2 {
        return Err(unsupported(format!(
            "{name}: `ViewMode` is only valid for implementation 2.0"
        )));
    }
    Ok(out)
}

/// The site a call reads, shared by its lazy values.
struct Site {
    client: Arc<Client>,
    /// The site URL, without a trailing `/`.
    url: String,
    origin: String,
}

impl std::fmt::Debug for Site {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Site({})", self.url)
    }
}

/// The elements of a JSON collection: `value` (no metadata), `d.results` (verbose), or a
/// property holding either.
fn items<'a>(v: &'a Value, key: Option<&str>) -> Option<&'a Vec<Value>> {
    let v = v.get("d").unwrap_or(v);
    let v = match key {
        Some(k) => v.get(k)?,
        None => v.get("value").unwrap_or(v),
    };
    v.as_array()
        .or_else(|| v.get("results").and_then(Value::as_array))
}

fn next_link(v: &Value) -> Option<&str> {
    let d = v.get("d").unwrap_or(v);
    ["odata.nextLink", "@odata.nextLink", "__next"]
        .iter()
        .find_map(|k| d.get(*k).and_then(Value::as_str))
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// The message of a SharePoint error body (`error.message.value` or `odata.error.message.value`).
fn error_message(body: &[u8]) -> Option<String> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let m = v
        .pointer("/error/message")
        .or_else(|| v.pointer("/odata.error/message"))?;
    match m {
        Value::String(s) => Some(http::clip(s)),
        _ => m.get("value").and_then(Value::as_str).map(http::clip),
    }
}

/// A server-relative URL as the argument of `GetFolderByServerRelativeUrl('…')`.
fn quoted(server_relative: &str) -> String {
    http::encode(&server_relative.replace('\'', "''"))
}

/// An ISO 8601 UTC time (`2024-01-15T09:30:00Z`) as a datetime.
fn datetime(t: &str) -> RemoteValue {
    let num = |a: usize, b: usize| t.get(a..b).and_then(|s| s.parse::<u32>().ok());
    let (Some(y), Some(mo), Some(d), Some(h), Some(mi), Some(s)) = (
        num(0, 4),
        num(5, 7),
        num(8, 10),
        num(11, 13),
        num(14, 16),
        num(17, 19),
    ) else {
        return RemoteValue::Null;
    };
    let (Ok(year), Ok(month), Ok(day)) = (i32::try_from(y), u8::try_from(mo), u8::try_from(d)) else {
        return RemoteValue::Null;
    };
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || h > 23 || mi > 59 || s > 59 {
        return RemoteValue::Null;
    }
    RemoteValue::DateTime {
        year,
        month,
        day,
        millis: ((h * 60 + mi) * 60 + s) * 1000,
    }
}

fn extension(name: &str, folder: bool) -> String {
    if folder {
        return String::new();
    }
    match name.rfind('.') {
        Some(i) => name.get(i..).unwrap_or("").to_lowercase(),
        None => String::new(),
    }
}

impl Site {
    fn get(&self, url: &str) -> Result<Value, ConnectError> {
        let r = Request {
            url: url.to_string(),
            headers: vec![("Accept".into(), ACCEPT.into())],
            ..Request::default()
        };
        let resp = self.client.send(&r, usize::MAX, false)?;
        self.check(&resp, url)?;
        let body = resp.body.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&resp.body);
        serde_json::from_slice(body).map_err(|e| {
            http::net(format!(
                "SharePoint: the response of {} is not JSON: {e}",
                http::clip(url)
            ))
        })
    }

    fn check(&self, r: &http::Response, url: &str) -> Result<(), ConnectError> {
        if (200..300).contains(&r.status) {
            return Ok(());
        }
        let message = error_message(&r.body)
            .map(|m| format!(": {m}"))
            .unwrap_or_default();
        let text = format!(
            "SharePoint: {} answered {} {}{message}",
            http::clip(url),
            r.status,
            r.reason
        );
        if matches!(r.status, 401 | 403 | 407) {
            return Err(ConnectError::new(
                ConnectErrorKind::Credentials,
                format!(
                    "{text}. Set the site's credentials in Data source settings (source.credentials): \
                     a bearer token for SharePoint Online"
                ),
            ));
        }
        Err(source(text))
    }

    /// Every element of a paged collection, following next links on the site's origin.
    fn all(&self, first: &str, max: usize) -> Result<Vec<Value>, ConnectError> {
        let mut out = Vec::new();
        let mut url = first.to_string();
        for _ in 0..self.client.settings().max_pages {
            let doc = self.get(&url)?;
            let page = items(&doc, None).ok_or_else(|| {
                http::net(format!(
                    "SharePoint: {} did not return a collection",
                    http::clip(&url)
                ))
            })?;
            if out.len().saturating_add(page.len()) > max {
                return Err(ConnectError::new(
                    ConnectErrorKind::Limit,
                    format!("SharePoint: {} has more than {max} items", http::clip(first)),
                ));
            }
            out.extend(page.iter().cloned());
            match next_link(&doc) {
                None => return Ok(out),
                Some(n) => {
                    let n = http::resolve(&url, n);
                    if http::origin(&n) != self.origin {
                        return Err(http::net(format!(
                            "SharePoint: the next page of {} is on another site",
                            http::clip(first)
                        )));
                    }
                    url = n;
                }
            }
        }
        Err(ConnectError::new(
            ConnectErrorKind::Limit,
            format!(
                "SharePoint: {} has more than {} pages",
                http::clip(first),
                self.client.settings().max_pages
            ),
        ))
    }

    /// A folder with its files and subfolders (`None`: the site's root folder).
    fn folder(&self, server_relative: Option<&str>) -> Result<(Vec<Value>, Vec<Value>), ConnectError> {
        let url = match server_relative {
            None => format!("{}/_api/web/RootFolder?$expand=Folders,Files", self.url),
            Some(p) => format!(
                "{}/_api/web/GetFolderByServerRelativeUrl('{}')?$expand=Folders,Files",
                self.url,
                quoted(p)
            ),
        };
        let doc = self.get(&url)?;
        let files = items(&doc, Some("Files")).cloned().unwrap_or_default();
        let folders = items(&doc, Some("Folders")).cloned().unwrap_or_default();
        Ok((files, folders))
    }

    /// The server-relative root folders of the site's document libraries (lists of template
    /// 101 that are not hidden).
    fn libraries(&self) -> Result<Vec<String>, ConnectError> {
        let url = format!(
            "{}/_api/web/lists?$select=Title,Hidden,BaseTemplate,RootFolder/ServerRelativeUrl&$expand=RootFolder",
            self.url
        );
        let lists = self.all(&url, bicraft_data::m::navigate::MAX_ITEMS)?;
        Ok(lists
            .iter()
            .filter(|l| l.get("BaseTemplate").and_then(Value::as_i64) == Some(101))
            .filter(|l| l.get("Hidden").and_then(Value::as_bool) != Some(true))
            .filter_map(|l| {
                l.get("RootFolder")
                    .map(|r| r.get("d").unwrap_or(r))
                    .and_then(|r| r.get("ServerRelativeUrl"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect())
    }

    /// The `Folder Path` of an item whose server-relative URL is `server_relative`.
    fn folder_path(&self, server_relative: &str) -> String {
        let parent = match server_relative.rfind('/') {
            Some(i) => server_relative.get(..i).unwrap_or(""),
            None => "",
        };
        format!("{}{parent}/", self.origin_as_written())
    }

    fn origin_as_written(&self) -> String {
        let (scheme, rest) = self.url.split_once("://").unwrap_or(("https", &self.url));
        let host = rest.split('/').next().unwrap_or("");
        format!("{scheme}://{host}")
    }
}

fn file_columns(content: RemoteType) -> Vec<RemoteColumn> {
    let col = |name: &str, ty| RemoteColumn {
        name: name.into(),
        ty,
        comparable: false,
    };
    vec![
        col("Content", content),
        col("Name", RemoteType::Text),
        col("Extension", RemoteType::Text),
        col("Date accessed", RemoteType::DateTime),
        col("Date modified", RemoteType::DateTime),
        col("Date created", RemoteType::DateTime),
        col("Attributes", RemoteType::Record),
        col("Folder Path", RemoteType::Text),
    ]
}

/// The bytes of a file, read when its `Content` is used.
#[derive(Debug)]
struct FileContent {
    site: Arc<Site>,
    server_relative: String,
}

impl LazyContent for FileContent {
    fn read(&self, max_bytes: usize) -> Result<Vec<u8>, ConnectError> {
        let url = format!(
            "{}/_api/web/GetFileByServerRelativeUrl('{}')/$value",
            self.site.url,
            quoted(&self.server_relative)
        );
        let r = Request {
            url: url.clone(),
            ..Request::default()
        };
        let resp = self.site.client.send(&r, max_bytes, false)?;
        self.site.check(&resp, &url)?;
        Ok(resp.body.to_vec())
    }
}

/// A folder's contents, read when its `Content` is used (`SharePoint.Contents`).
#[derive(Debug)]
struct FolderRows {
    site: Arc<Site>,
    /// `None`: the site's root folder.
    server_relative: Option<String>,
}

impl LazyRows for FolderRows {
    fn rows(&self, max_rows: usize) -> Result<RemoteRows, ConnectError> {
        let (files, folders) = self.site.folder(self.server_relative.as_deref())?;
        let mut rows = Vec::new();
        for f in &folders {
            let path = text(f, "ServerRelativeUrl").to_string();
            let nested = RemoteValue::Rows(Lazy(Arc::new(FolderRows {
                site: Arc::clone(&self.site),
                server_relative: Some(path.clone()),
            })));
            rows.push(row(&self.site, f, nested, true));
        }
        for f in &files {
            rows.push(file_row(&self.site, f));
        }
        if rows.len() > max_rows {
            return Err(limit_rows(max_rows));
        }
        Ok(RemoteRows {
            columns: file_columns(RemoteType::Any),
            rows,
        })
    }
}

fn limit_rows(max: usize) -> ConnectError {
    ConnectError::new(
        ConnectErrorKind::Limit,
        format!("SharePoint: more than {max} rows"),
    )
}

fn size(f: &Value) -> RemoteValue {
    match f.get("Length") {
        Some(Value::Number(n)) => n.as_i64().map_or(RemoteValue::Null, RemoteValue::Int),
        Some(Value::String(s)) => s.parse::<i64>().map_or(RemoteValue::Null, RemoteValue::Int),
        _ => RemoteValue::Null,
    }
}

fn row(site: &Site, f: &Value, content: RemoteValue, folder: bool) -> Vec<RemoteValue> {
    let name = text(f, "Name");
    let ext = extension(name, folder);
    let kind = if folder { "Folder" } else { "File" };
    let attributes = RemoteValue::Record(vec![
        ("Kind".into(), RemoteValue::Text(kind.into())),
        ("Size".into(), if folder { RemoteValue::Null } else { size(f) }),
        ("Directory".into(), RemoteValue::Logical(folder)),
    ]);
    vec![
        content,
        RemoteValue::Text(name.to_string()),
        RemoteValue::Text(ext),
        RemoteValue::Null,
        datetime(text(f, "TimeLastModified")),
        datetime(text(f, "TimeCreated")),
        attributes,
        RemoteValue::Text(site.folder_path(text(f, "ServerRelativeUrl"))),
    ]
}

fn file_row(site: &Arc<Site>, f: &Value) -> Vec<RemoteValue> {
    let content = RemoteValue::Content(Lazy(Arc::new(FileContent {
        site: Arc::clone(site),
        server_relative: text(f, "ServerRelativeUrl").to_string(),
    })));
    row(site, f, content, false)
}

/// `SharePoint.Files`: every file of the document libraries, depth first, a folder's files
/// before its subfolders.
fn all_files(site: &Arc<Site>, max_rows: usize) -> Result<RemoteRows, ConnectError> {
    let mut rows = Vec::new();
    let mut requests = 0usize;
    for lib in site.libraries()? {
        let mut stack = vec![(lib, 0usize)];
        while let Some((folder, depth)) = stack.pop() {
            requests = requests.saturating_add(1);
            if requests > site.client.settings().max_pages {
                return Err(ConnectError::new(
                    ConnectErrorKind::Limit,
                    format!(
                        "SharePoint.Files: the site has more than {} folders",
                        site.client.settings().max_pages
                    ),
                ));
            }
            let (files, folders) = site.folder(Some(&folder))?;
            for f in &files {
                rows.push(file_row(site, f));
                if rows.len() > max_rows {
                    return Err(limit_rows(max_rows));
                }
            }
            if !folders.is_empty() && depth >= MAX_DEPTH {
                return Err(ConnectError::new(
                    ConnectErrorKind::Limit,
                    format!("SharePoint.Files: folders nest deeper than {MAX_DEPTH}"),
                ));
            }
            for f in folders.iter().rev() {
                let p = text(f, "ServerRelativeUrl");
                if !p.is_empty() {
                    stack.push((p.to_string(), depth.saturating_add(1)));
                }
            }
        }
    }
    Ok(RemoteRows {
        columns: file_columns(RemoteType::Binary),
        rows,
    })
}

/// A list's items, read when `Items` is used.
#[derive(Debug)]
struct ListItems {
    site: Arc<Site>,
    id: String,
    default_view: bool,
}

/// A JSON value of an item as an M value (`any` columns): records for objects, text for lists.
fn json_value(v: &Value, depth: usize) -> RemoteValue {
    match v {
        Value::Null => RemoteValue::Null,
        Value::Bool(b) => RemoteValue::Logical(*b),
        Value::Number(n) => match n.as_i64() {
            Some(i) => RemoteValue::Int(i),
            None => n.as_f64().map_or(RemoteValue::Null, RemoteValue::Double),
        },
        Value::String(s) => RemoteValue::Text(s.clone()),
        Value::Object(o) if depth < MAX_VALUE_DEPTH => RemoteValue::Record(
            o.iter()
                .filter(|(k, _)| !skipped(k))
                .map(|(k, v)| (k.clone(), json_value(v, depth + 1)))
                .collect(),
        ),
        other => RemoteValue::Text(other.to_string()),
    }
}

/// Properties of an item that are protocol metadata, not columns.
fn skipped(key: &str) -> bool {
    key.starts_with("odata.") || key.starts_with("@odata") || key == "__metadata" || key.contains("@odata.")
}

impl LazyRows for ListItems {
    fn rows(&self, max_rows: usize) -> Result<RemoteRows, ConnectError> {
        let list = format!(
            "{}/_api/web/lists(guid'{}')",
            self.site.url,
            http::encode(&self.id)
        );
        let mut select = String::new();
        if self.default_view {
            let doc = self.site.get(&format!("{list}/DefaultView/ViewFields"))?;
            let fields: Vec<String> = items(&doc, Some("Items"))
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(http::encode)
                .collect();
            if !fields.is_empty() {
                select = format!("&$select={}", fields.join(","));
            }
        }
        let all = self
            .site
            .all(&format!("{list}/items?$top=5000{select}"), max_rows)?;
        let mut names: Vec<String> = Vec::new();
        for item in &all {
            if let Some(o) = item.as_object() {
                for k in o.keys() {
                    if !skipped(k) && !names.contains(k) {
                        if names.len() >= 16_384 {
                            return Err(ConnectError::new(
                                ConnectErrorKind::Limit,
                                "SharePoint.Tables: the list has more than 16384 columns",
                            ));
                        }
                        names.push(k.clone());
                    }
                }
            }
        }
        let rows = all
            .iter()
            .map(|item| {
                names
                    .iter()
                    .map(|n| item.get(n).map_or(RemoteValue::Null, |v| json_value(v, 0)))
                    .collect()
            })
            .collect();
        Ok(RemoteRows {
            columns: names
                .into_iter()
                .map(|name| RemoteColumn {
                    name,
                    ty: RemoteType::Any,
                    comparable: false,
                })
                .collect(),
            rows,
        })
    }
}

/// The site's lists that are not hidden: `Id`, `Title`, `Items`.
fn lists(site: &Arc<Site>, default_view: bool) -> Result<RemoteRows, ConnectError> {
    let url = format!("{}/_api/web/lists?$select=Id,Title,Hidden", site.url);
    let all = site.all(&url, bicraft_data::m::navigate::MAX_ITEMS)?;
    let rows = all
        .iter()
        .filter(|l| l.get("Hidden").and_then(Value::as_bool) != Some(true))
        .filter(|l| !text(l, "Id").is_empty())
        .map(|l| {
            let id = text(l, "Id").to_string();
            vec![
                RemoteValue::Text(id.clone()),
                RemoteValue::Text(text(l, "Title").to_string()),
                RemoteValue::Rows(Lazy(Arc::new(ListItems {
                    site: Arc::clone(site),
                    id,
                    default_view,
                }))),
            ]
        })
        .collect();
    let col = |name: &str, ty| RemoteColumn {
        name: name.into(),
        ty,
        comparable: false,
    };
    Ok(RemoteRows {
        columns: vec![
            col("Id", RemoteType::Text),
            col("Title", RemoteType::Text),
            col("Items", RemoteType::Table),
        ],
        rows,
    })
}

/// One SharePoint call: a table (nothing folds into it).
pub struct Call {
    site: Arc<Site>,
    opts: Options,
    rows: Option<RemoteRows>,
}

impl Call {
    pub fn open(client: Arc<Client>, opts: Options) -> Self {
        let origin = http::origin(&opts.site);
        Self {
            site: Arc::new(Site {
                client,
                url: opts.site.clone(),
                origin,
            }),
            opts,
            rows: None,
        }
    }

    fn read(&mut self, max_rows: usize) -> Result<RemoteRows, ConnectError> {
        if let Some(r) = &self.rows {
            return Ok(r.clone());
        }
        let r = match self.opts.function {
            Function::Files => all_files(&self.site, max_rows)?,
            Function::Contents => FolderRows {
                site: Arc::clone(&self.site),
                server_relative: None,
            }
            .rows(max_rows)?,
            Function::Tables => lists(&self.site, self.opts.default_view)?,
        };
        self.rows = Some(r.clone());
        Ok(r)
    }
}

impl Source for Call {
    fn lookup(&mut self, _: &[String], _: &str, _: Option<&str>) -> Result<Option<NavEntry>, ConnectError> {
        Ok(None)
    }

    fn list(&mut self, _: &[String]) -> Result<Vec<NavEntry>, ConnectError> {
        Err(unsupported(format!(
            "{} returns a table",
            self.opts.function.name()
        )))
    }

    fn columns(&mut self, _: &[String]) -> Result<Vec<RemoteColumn>, ConnectError> {
        Ok(match self.opts.function {
            Function::Files => file_columns(RemoteType::Binary),
            Function::Contents => file_columns(RemoteType::Any),
            Function::Tables => lists(&self.site, self.opts.default_view)?.columns,
        })
    }

    fn fetch(&mut self, _: &RemoteQuery, max_rows: usize) -> Result<RemoteRows, ConnectError> {
        let r = self.read(max_rows)?;
        if r.rows.len() > max_rows {
            return Err(limit_rows(max_rows));
        }
        Ok(r)
    }

    fn root_is_table(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(f: Vec<(&str, Arg)>) -> Arg {
        Arg::Record(f.into_iter().map(|(n, v)| (n.to_string(), v)).collect())
    }

    #[test]
    fn checked_options() {
        let site = Arg::Text("https://contoso.example.com/sites/Team/".into());
        let o = options(
            "SharePoint.Files",
            &[site.clone(), rec(vec![("ApiVersion", Arg::Number(15.0))])],
        )
        .unwrap();
        assert_eq!(o.site, "https://contoso.example.com/sites/Team");
        let o = options(
            "SharePoint.Tables",
            &[
                site.clone(),
                rec(vec![
                    ("Implementation", Arg::Text("2.0".into())),
                    ("ViewMode", Arg::Text("Default".into())),
                ]),
            ],
        )
        .unwrap();
        assert!(o.default_view);
        for bad in [
            ("SharePoint.Files", vec![Arg::Text("https://h/s?x=1".into())]),
            (
                "SharePoint.Files",
                vec![site.clone(), rec(vec![("ApiVersion", Arg::Number(16.0))])],
            ),
            (
                "SharePoint.Files",
                vec![
                    site.clone(),
                    rec(vec![("Implementation", Arg::Text("2.0".into()))]),
                ],
            ),
            (
                "SharePoint.Contents",
                vec![
                    site.clone(),
                    rec(vec![
                        ("Implementation", Arg::Text("2.0".into())),
                        ("ApiVersion", Arg::Number(14.0)),
                    ]),
                ],
            ),
            (
                "SharePoint.Tables",
                vec![site.clone(), rec(vec![("ViewMode", Arg::Text("All".into()))])],
            ),
            (
                "SharePoint.Tables",
                vec![site.clone(), rec(vec![("Bogus", Arg::Logical(true))])],
            ),
            ("SharePoint.Tables", vec![Arg::Number(1.0)]),
            ("SharePoint.Other", vec![site.clone()]),
        ] {
            assert!(options(bad.0, &bad.1).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn values() {
        assert_eq!(
            datetime("2024-01-15T09:30:05Z"),
            RemoteValue::DateTime {
                year: 2024,
                month: 1,
                day: 15,
                millis: 34_205_000
            }
        );
        assert_eq!(datetime("garbage"), RemoteValue::Null);
        assert_eq!(quoted("/sites/a/O'Neil docs"), "/sites/a/O''Neil%20docs");
        assert_eq!(extension("Budget.XLSX", false), ".xlsx");
        let deep = (0..40).fold(serde_json::json!(1), |v, _| serde_json::json!({ "a": v }));
        // deep objects end as text, never recursing without bound
        let mut v = json_value(&deep, 0);
        let mut depth = 0;
        while let RemoteValue::Record(f) = v {
            v = f.into_iter().next().map(|(_, x)| x).unwrap_or(RemoteValue::Null);
            depth += 1;
        }
        assert_eq!(depth, MAX_VALUE_DEPTH);
        assert!(matches!(v, RemoteValue::Text(_)));
    }
}
