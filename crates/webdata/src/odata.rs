//! `OData.Feed(serviceUri, headers, options)`: an OData service (JSON format, OData v4; the v3
//! JSON light names are read too), anonymously.
//!
//! - The service document lists the entity sets: the navigation table (`Name`, `Data`,
//!   `Signature` `"table"`, `Kind` `"Table"`). A feed URL that is an entity set (its
//!   `@odata.context` has a `#` fragment) is that table itself, and nothing folds into it.
//! - `$metadata` (CSDL, [`crate::csdl`]) gives the columns and their M types.
//! - The folded query of a table ([`RemoteQuery`]) becomes the system query options `$select`,
//!   `$filter` (comparisons with `eq ne lt le gt ge`, `null` tests, `and`), `$orderby` and `$top`;
//!   server-driven paging (`@odata.nextLink`) is followed on the feed's own origin.
//!
//! See docs/formats/odata.md.

use std::sync::Arc;
use std::time::Duration;

use bicraft_data::connect::{
    Arg, CmpOp, ConnectError, ConnectErrorKind, Literal, NavEntry, Pred, RemoteColumn, RemoteQuery,
    RemoteRows, RemoteType, RemoteValue, SortKey, Source,
};
use serde_json::Value;

use crate::csdl::{self, Metadata, Property};
use crate::http::{self, Client, Request, net, source, unsupported};

const ACCEPT_JSON: &str = "application/json;odata.metadata=minimal;IEEE754Compatible=true";

/// The checked arguments of an `OData.Feed` call.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// Custom query options (`Query = [...]`), sent with every request.
    pub query: Vec<(String, String)>,
    pub timeout: Option<Duration>,
}

fn texts(a: &Arg, what: &str) -> Result<Vec<(String, String)>, ConnectError> {
    let Arg::Record(fields) = a else {
        return Err(unsupported(format!("OData.Feed: `{what}` must be a record")));
    };
    fields
        .iter()
        .map(|(n, v)| match v {
            Arg::Text(t) => Ok((n.clone(), t.clone())),
            _ => Err(unsupported(format!(
                "OData.Feed: the value of `{what}[{n}]` must be text"
            ))),
        })
        .collect()
}

/// Checks a call's arguments.
pub fn options(args: &[Arg]) -> Result<Options, ConnectError> {
    let url = match args.first() {
        Some(Arg::Text(u)) => u.trim().to_string(),
        _ => return Err(unsupported("OData.Feed: the service URL must be text")),
    };
    if args.len() > 3 {
        return Err(unsupported(
            "OData.Feed takes a URL, a headers record and an options record",
        ));
    }
    http::check_url(&url)?;
    let mut out = Options {
        url,
        headers: Vec::new(),
        query: Vec::new(),
        timeout: None,
    };
    match args.get(1) {
        None | Some(Arg::Null) => {}
        Some(h) => out.headers = texts(h, "headers")?,
    }
    match args.get(2) {
        None | Some(Arg::Null) => {}
        Some(Arg::Record(fields)) => {
            for (name, v) in fields {
                if matches!(v, Arg::Null) {
                    continue;
                }
                match name.as_str() {
                    // both implementations read the same service the same way here
                    "Implementation" | "ODataVersion" | "EnableBatch" | "Concurrent"
                    | "FunctionOverloads" => {}
                    "Headers" => out.headers.extend(texts(v, "Headers")?),
                    "Query" => out.query = texts(v, "Query")?,
                    "Timeout" => match v {
                        Arg::Duration(ms) if *ms > 0 => {
                            out.timeout = Some(Duration::from_millis(ms.unsigned_abs()))
                        }
                        _ => return Err(unsupported("OData.Feed: `Timeout` must be a positive duration")),
                    },
                    "MoreColumns" if matches!(v, Arg::Logical(false)) => {}
                    other => {
                        return Err(unsupported(format!(
                            "OData.Feed: the option `{other}` is not supported by bicraft"
                        )));
                    }
                }
            }
        }
        Some(_) => return Err(unsupported("OData.Feed: the options must be a record")),
    }
    http::check_headers(&out.headers)?;
    if out.query.iter().any(|(k, _)| k.starts_with('$')) {
        return Err(unsupported(
            "OData.Feed: system query options ($select, $filter, …) are not custom query options",
        ));
    }
    Ok(out)
}

/// An entity set of the service document.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    name: String,
    url: String,
}

enum Root {
    Service(Vec<Entry>),
    /// The feed is an entity set: its name in `$metadata` and its first page.
    Table {
        set: String,
        first: Value,
    },
}

/// One `OData.Feed` call.
pub struct Feed {
    client: Arc<Client>,
    opts: Options,
    root: Root,
    metadata_url: String,
    metadata: Option<Metadata>,
}

/// The feed URL as a base for relative URLs: without query, ending with `/`.
fn base(url: &str) -> String {
    let mut b = url.split(['?', '#']).next().unwrap_or(url).to_string();
    if !b.ends_with('/') {
        b.push('/');
    }
    b
}

/// The folder of a URL (up to the last `/` of its path), for relative references.
fn folder(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let host_end = path
        .find("://")
        .map(|i| i + 3)
        .and_then(|i| path.get(i..).and_then(|r| r.find('/')).map(|j| i + j));
    match (host_end, path.rfind('/')) {
        (Some(h), Some(last)) if last >= h => path.get(..=last).unwrap_or(path).to_string(),
        _ => format!("{path}/"),
    }
}

fn context(v: &Value) -> Option<&str> {
    v.get("@odata.context")
        .or_else(|| v.get("odata.metadata"))
        .and_then(Value::as_str)
}

fn next_link(v: &Value) -> Option<&str> {
    v.get("@odata.nextLink")
        .or_else(|| v.get("odata.nextLink"))
        .and_then(Value::as_str)
}

/// The message of an OData error body (v4 `error.message`, v3 `odata.error.message.value`).
fn error_message(body: &[u8]) -> Option<String> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let m = v
        .pointer("/error/message")
        .or_else(|| v.pointer("/odata.error/message/value"))?;
    match m {
        Value::String(s) => Some(http::clip(s)),
        Value::Object(_) => m.get("value").and_then(Value::as_str).map(http::clip),
        _ => None,
    }
}

impl Feed {
    /// Opens a feed: reads the document at its URL to tell a service from an entity set.
    pub fn open(client: Arc<Client>, opts: Options) -> Result<Self, ConnectError> {
        let url = http::with_query(&opts.url, &opts.query);
        let doc = get_json(&client, &opts, &url)?;
        let ctx = context(&doc).map(str::to_string);
        let metadata_url = match ctx.as_deref().map(|c| c.split('#').next().unwrap_or(c)) {
            Some(c) if c.starts_with("http://") || c.starts_with("https://") => c.to_string(),
            // relative to the request URL's folder
            Some(c) if !c.is_empty() && !c.starts_with('/') => format!("{}{c}", folder(&opts.url)),
            Some(c) if !c.is_empty() => format!("{}{c}", http::origin(&opts.url)),
            _ => http::join_path(&base(&opts.url), "$metadata"),
        };
        http::check_url(&metadata_url)?;
        if http::origin(&metadata_url) != http::origin(&opts.url) {
            return Err(net(format!(
                "OData.Feed: the metadata of {} is on another host",
                http::clip(&opts.url)
            )));
        }
        let fragment = ctx
            .as_deref()
            .and_then(|c| c.split_once('#'))
            .map(|(_, f)| f.to_string());
        let root = match fragment {
            Some(f) => {
                // `Products`, `Products(ID,Name)`, `Products/$entity`
                let set = f.split(['(', '/']).next().unwrap_or("").to_string();
                if set.is_empty() {
                    return Err(net(format!(
                        "OData.Feed: {} is neither a service document nor an entity set",
                        http::clip(&opts.url)
                    )));
                }
                Root::Table { set, first: doc }
            }
            None => Root::Service(entries(&doc, &opts.url)?),
        };
        Ok(Self {
            client,
            opts,
            root,
            metadata_url,
            metadata: None,
        })
    }

    fn metadata(&mut self) -> Result<&Metadata, ConnectError> {
        if self.metadata.is_none() {
            let r = Request {
                url: self.metadata_url.clone(),
                headers: with_header(&self.opts.headers, "Accept", "application/xml"),
                body: None,
                timeout: self.opts.timeout,
                api_key_name: None,
            };
            let resp = self.client.send(&r, usize::MAX, false)?;
            check_status(&resp, &r.url)?;
            let text = std::str::from_utf8(&resp.body)
                .map_err(|_| net("OData.Feed: the metadata is not UTF-8 text"))?;
            let m = csdl::parse(text.trim_start_matches('\u{feff}')).map_err(net)?;
            self.metadata = Some(m);
        }
        self.metadata
            .as_ref()
            .ok_or_else(|| net("OData.Feed: the metadata could not be read"))
    }

    /// The entity set at `path` (`[]` for a feed that is one) and its URL.
    fn set_at(&self, path: &[String]) -> Result<(String, Option<String>), ConnectError> {
        match (&self.root, path) {
            (Root::Table { set, .. }, []) => Ok((set.clone(), None)),
            (Root::Service(list), [name]) => list
                .iter()
                .find(|e| e.name == *name)
                .map(|e| (e.name.clone(), Some(e.url.clone())))
                .ok_or_else(|| source(format!("OData.Feed: the service has no entity set `{name}`"))),
            _ => Err(source(format!("OData.Feed: nothing at {}", path.join(" › ")))),
        }
    }

    fn properties(&mut self, set: &str) -> Result<Vec<Property>, ConnectError> {
        let m = self.metadata()?;
        m.set(set).map(|s| s.columns.clone()).ok_or_else(|| {
            net(format!(
                "OData.Feed: the metadata does not describe the entity set `{set}`"
            ))
        })
    }
}

fn with_header(headers: &[(String, String)], name: &str, value: &str) -> Vec<(String, String)> {
    let mut h: Vec<(String, String)> = headers
        .iter()
        .filter(|(n, _)| !n.eq_ignore_ascii_case(name))
        .cloned()
        .collect();
    h.push((name.to_string(), value.to_string()));
    h
}

fn check_status(r: &http::Response, url: &str) -> Result<(), ConnectError> {
    if (200..300).contains(&r.status) {
        return Ok(());
    }
    let message = error_message(&r.body)
        .map(|m| format!(": {m}"))
        .unwrap_or_default();
    let text = format!(
        "OData.Feed: {} answered {} {}{message}",
        http::clip(url),
        r.status,
        r.reason
    );
    if matches!(r.status, 401 | 403 | 407) {
        return Err(ConnectError::new(
            ConnectErrorKind::Credentials,
            format!("{text}. Set the feed's credentials in Data source settings (source.credentials)"),
        ));
    }
    Err(source(text))
}

fn get_json(client: &Client, opts: &Options, url: &str) -> Result<Value, ConnectError> {
    let mut headers = with_header(&opts.headers, "Accept", ACCEPT_JSON);
    headers.push(("OData-MaxVersion".into(), "4.0".into()));
    let r = Request {
        url: url.to_string(),
        headers,
        body: None,
        timeout: opts.timeout,
        api_key_name: None,
    };
    let resp = client.send(&r, usize::MAX, false)?;
    check_status(&resp, url)?;
    parse_json(&resp.body)
}

/// A JSON response body.
pub fn parse_json(body: &[u8]) -> Result<Value, ConnectError> {
    let body = body.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(body);
    serde_json::from_slice(body).map_err(|e| net(format!("OData.Feed: the response is not JSON: {e}")))
}

/// The entity sets of a service document (singletons and function imports are not listed).
fn entries(doc: &Value, feed: &str) -> Result<Vec<Entry>, ConnectError> {
    let Some(list) = doc.get("value").and_then(Value::as_array) else {
        return Err(net(format!(
            "OData.Feed: {} did not return a service document",
            http::clip(feed)
        )));
    };
    let origin = http::origin(feed);
    let mut out = Vec::new();
    for e in list.iter().take(bicraft_data::m::navigate::MAX_ITEMS) {
        let kind = e.get("kind").and_then(Value::as_str).unwrap_or("EntitySet");
        if kind != "EntitySet" {
            continue;
        }
        let (Some(name), Some(url)) = (
            e.get("name").and_then(Value::as_str),
            e.get("url").and_then(Value::as_str),
        ) else {
            continue;
        };
        let url = if url.starts_with("http://") || url.starts_with("https://") {
            if http::origin(url) != origin {
                continue;
            }
            url.to_string()
        } else {
            http::join_path(&base(feed), url)
        };
        out.push(Entry {
            name: name.to_string(),
            url,
        });
    }
    Ok(out)
}

/// A literal in the OData URL syntax.
fn literal(l: &Literal) -> String {
    match l {
        Literal::Logical(b) => b.to_string(),
        Literal::Int(i) => i.to_string(),
        Literal::Text(t) => format!("'{}'", t.replace('\'', "''")),
        Literal::Date { year, month, day } => format!("{year:04}-{month:02}-{day:02}"),
    }
}

fn op(o: CmpOp) -> &'static str {
    match o {
        CmpOp::Eq => "eq",
        CmpOp::Ne => "ne",
        CmpOp::Lt => "lt",
        CmpOp::Le => "le",
        CmpOp::Gt => "gt",
        CmpOp::Ge => "ge",
    }
}

/// A row filter as a `$filter` expression. OData compares with `null` as M does: `eq` and the
/// order comparisons never match a null value, `ne` does.
pub fn filter(p: &Pred) -> String {
    match p {
        Pred::Compare { column, op: o, value } => format!("{column} {} {}", op(*o), literal(value)),
        Pred::IsNull(c) => format!("{c} eq null"),
        Pred::NotNull(c) => format!("{c} ne null"),
        Pred::And(parts) => parts
            .iter()
            .map(|p| format!("({})", filter(p)))
            .collect::<Vec<_>>()
            .join(" and "),
    }
}

/// The system query options of a folded query.
pub fn system_options(q: &RemoteQuery) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(cols) = &q.columns {
        out.push(("$select".to_string(), cols.join(",")));
    }
    if !q.filter.is_empty() {
        let parts: Vec<String> = q.filter.iter().map(filter).collect();
        let text = if parts.len() == 1 {
            parts.concat()
        } else {
            parts
                .iter()
                .map(|p| format!("({p})"))
                .collect::<Vec<_>>()
                .join(" and ")
        };
        out.push(("$filter".to_string(), text));
    }
    if !q.sort.is_empty() {
        let keys: Vec<String> = q
            .sort
            .iter()
            .map(|SortKey { column, descending }| {
                if *descending {
                    format!("{column} desc")
                } else {
                    column.clone()
                }
            })
            .collect();
        out.push(("$orderby".to_string(), keys.join(",")));
    }
    if let Some(n) = q.limit {
        out.push(("$top".to_string(), n.to_string()));
    }
    out
}

fn column(p: &Property) -> RemoteColumn {
    RemoteColumn {
        name: p.name.clone(),
        ty: p.ty,
        // texts: the service's collation is unknown; singles: M compares doubles
        comparable: !matches!(
            p.ty,
            RemoteType::Text
                | RemoteType::Single
                | RemoteType::Binary
                | RemoteType::Any
                | RemoteType::Record
                | RemoteType::Table
        ),
    }
}

impl Source for Feed {
    fn lookup(&mut self, _: &[String], _: &str, _: Option<&str>) -> Result<Option<NavEntry>, ConnectError> {
        Ok(None)
    }

    fn list(&mut self, path: &[String]) -> Result<Vec<NavEntry>, ConnectError> {
        match (&self.root, path) {
            (Root::Service(list), []) => Ok(list
                .iter()
                .map(|e| NavEntry {
                    name: e.name.clone(),
                    kind: "Table".into(),
                    leaf: true,
                    fields: vec![("Signature".into(), "table".into())],
                })
                .collect()),
            _ => Err(source(format!(
                "OData.Feed: nothing to list at {}",
                path.join(" › ")
            ))),
        }
    }

    fn columns(&mut self, path: &[String]) -> Result<Vec<RemoteColumn>, ConnectError> {
        let (set, _) = self.set_at(path)?;
        Ok(self.properties(&set)?.iter().map(column).collect())
    }

    fn folds_sort(&self) -> bool {
        true
    }

    fn root_is_table(&self) -> bool {
        matches!(self.root, Root::Table { .. })
    }

    fn native(&mut self, q: &RemoteQuery) -> Result<Option<String>, ConnectError> {
        let (_, url) = self.set_at(&q.table)?;
        Ok(Some(match url {
            Some(u) => {
                let mut pairs = self.opts.query.clone();
                pairs.extend(system_options(q));
                http::with_query(&u, &pairs)
            }
            // an entity-set URL given to OData.Feed: sent as it is
            None => http::with_query(&self.opts.url, &self.opts.query),
        }))
    }

    fn fetch(&mut self, q: &RemoteQuery, max_rows: usize) -> Result<RemoteRows, ConnectError> {
        let (set, url) = self.set_at(&q.table)?;
        let props = self.properties(&set)?;
        let wanted: Vec<Property> = match &q.columns {
            None => props.clone(),
            Some(names) => names
                .iter()
                .map_while(|n| props.iter().find(|p| p.name == *n).cloned())
                .collect(),
        };
        let columns: Vec<RemoteColumn> = wanted.iter().map(column).collect();
        let incomplete = q.columns.as_ref().is_some_and(|n| n.len() != wanted.len());
        if incomplete || q.limit == Some(0) {
            // a column the set does not have (the evaluator says which), or no rows asked for
            return Ok(RemoteRows {
                columns,
                rows: Vec::new(),
            });
        }
        let (first, mut at) = match (&self.root, url) {
            (_, Some(u)) => {
                let mut pairs = self.opts.query.clone();
                pairs.extend(system_options(q));
                let u = http::with_query(&u, &pairs);
                (get_json(&self.client, &self.opts, &u)?, u)
            }
            (Root::Table { first, .. }, None) => (first.clone(), self.opts.url.clone()),
            (Root::Service(_), None) => {
                return Err(source("OData.Feed: the service document is not a table"));
            }
        };
        let origin = http::origin(&self.opts.url);
        let limit = q
            .limit
            .and_then(|l| usize::try_from(l).ok())
            .unwrap_or(usize::MAX);
        let mut rows = Vec::new();
        let mut page = first;
        let mut pages = 0usize;
        loop {
            pages = pages.saturating_add(1);
            let values = page.get("value").and_then(Value::as_array).ok_or_else(|| {
                net(format!(
                    "OData.Feed: the entity set `{set}` did not return a list of entities"
                ))
            })?;
            for e in values {
                if rows.len() >= limit {
                    break;
                }
                if rows.len() >= max_rows {
                    return Err(ConnectError::new(
                        ConnectErrorKind::Limit,
                        format!("OData.Feed: the entity set `{set}` has more than {max_rows} rows"),
                    ));
                }
                rows.push(row(e, &wanted)?);
            }
            if rows.len() >= limit {
                break;
            }
            let Some(next) = next_link(&page) else { break };
            let next = if next.starts_with("http://") || next.starts_with("https://") {
                next.to_string()
            } else if next.starts_with('/') {
                format!("{origin}{next}")
            } else {
                format!("{}{next}", folder(&at))
            };
            http::check_url(&next)?;
            if http::origin(&next) != origin {
                return Err(net(format!(
                    "OData.Feed: the next page of `{set}` is on another host ({})",
                    http::clip(&next)
                )));
            }
            if pages >= self.client.settings().max_pages {
                return Err(ConnectError::new(
                    ConnectErrorKind::Limit,
                    format!("OData.Feed: the entity set `{set}` has more than {pages} pages"),
                ));
            }
            page = get_json(&self.client, &self.opts, &next)?;
            at = next;
        }
        Ok(RemoteRows { columns, rows })
    }
}

/// The rows of one page of an entity set (a response body), and its next link (the readers the
/// fuzz targets exercise).
pub fn page_rows(
    body: &[u8],
    columns: &[Property],
) -> Result<(Vec<Vec<RemoteValue>>, Option<String>), ConnectError> {
    let page = parse_json(body)?;
    let values = page
        .get("value")
        .and_then(Value::as_array)
        .ok_or_else(|| net("OData.Feed: the page holds no list of entities"))?;
    let rows = values
        .iter()
        .map(|e| row(e, columns))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((rows, next_link(&page).map(str::to_string)))
}

/// One entity as a row of `columns` (a missing property is null).
pub fn row(e: &Value, columns: &[Property]) -> Result<Vec<RemoteValue>, ConnectError> {
    let Some(obj) = e.as_object() else {
        return Err(net("OData.Feed: an entity is not a JSON object"));
    };
    columns
        .iter()
        .map(|p| {
            value(obj.get(&p.name).unwrap_or(&Value::Null), p.ty).ok_or_else(|| {
                net(format!(
                    "OData.Feed: the value `{}` of the {} property `{}` does not read as its type",
                    http::clip(&obj.get(&p.name).map(Value::to_string).unwrap_or_default()),
                    p.edm,
                    p.name
                ))
            })
        })
        .collect()
}

/// A JSON property value as a value of M type `ty` (`None`: it does not read as one).
pub fn value(v: &Value, ty: RemoteType) -> Option<RemoteValue> {
    if v.is_null() {
        return Some(RemoteValue::Null);
    }
    Some(match ty {
        RemoteType::Text => RemoteValue::Text(v.as_str()?.to_string()),
        RemoteType::Logical => RemoteValue::Logical(v.as_bool()?),
        RemoteType::Int8 | RemoteType::Int16 | RemoteType::Int32 | RemoteType::Int64 => {
            let i = match v {
                Value::Number(n) => n.as_i64()?,
                Value::String(s) => s.trim().parse::<i64>().ok()?,
                _ => return None,
            };
            let ok = match ty {
                RemoteType::Int8 => i8::try_from(i).is_ok(),
                // Edm.Byte (0-255) and Edm.Int16
                RemoteType::Int16 => i16::try_from(i).is_ok(),
                RemoteType::Int32 => i32::try_from(i).is_ok(),
                _ => true,
            };
            if !ok {
                return None;
            }
            RemoteValue::Int(i)
        }
        RemoteType::Single | RemoteType::Double => RemoteValue::Double(match v {
            Value::Number(n) => n.as_f64()?,
            Value::String(s) => match s.as_str() {
                "INF" => f64::INFINITY,
                "-INF" => f64::NEG_INFINITY,
                "NaN" => f64::NAN,
                other => other.trim().parse::<f64>().ok()?,
            },
            _ => return None,
        }),
        RemoteType::Decimal => match v {
            Value::Number(n) => RemoteValue::Decimal(n.to_string()),
            Value::String(s) if s.trim().parse::<f64>().is_ok_and(f64::is_finite) => {
                RemoteValue::Decimal(s.trim().to_string())
            }
            _ => return None,
        },
        RemoteType::Date => {
            let (year, month, day) = date(v.as_str()?)?;
            RemoteValue::Date { year, month, day }
        }
        RemoteType::DateTime => {
            let s = v.as_str()?;
            let (year, month, day, millis) = match s.strip_prefix("/Date(").and_then(|r| r.strip_suffix(")/"))
            {
                // `/Date(ms)/` or `/Date(ms+offset)/`: ms are UTC, the offset only informs
                Some(ms) => {
                    let sign = usize::from(ms.starts_with('-'));
                    let end = ms
                        .get(sign..)
                        .and_then(|b| b.find(['+', '-']))
                        .map_or(ms.len(), |i| i + sign);
                    from_unix_ms(ms.get(..end)?)?
                }
                None => {
                    let (d, t) = s.split_once('T')?;
                    let (y, m, dd) = date(d)?;
                    let t = t.trim_end_matches('Z');
                    (y, m, dd, time(t)?)
                }
            };
            RemoteValue::DateTime {
                year,
                month,
                day,
                millis,
            }
        }
        RemoteType::DateTimeZone => {
            let s = v.as_str()?;
            let (d, t) = s.split_once('T')?;
            let (year, month, day) = date(d)?;
            let (t, offset) = if let Some(t) = t.strip_suffix('Z') {
                (t, 0i16)
            } else {
                let at = t.rfind(['+', '-'])?;
                let (t, o) = t.split_at(at);
                let sign: i16 = if o.starts_with('-') { -1 } else { 1 };
                let (h, m) = o.get(1..)?.split_once(':')?;
                let (h, m) = (two(h)?, two(m)?);
                (t, sign * (i16::from(h) * 60 + i16::from(m)))
            };
            RemoteValue::DateTimeZone {
                year,
                month,
                day,
                millis: time(t)?,
                offset_minutes: offset,
            }
        }
        RemoteType::Time => RemoteValue::Time {
            millis: time(v.as_str()?)?,
        },
        RemoteType::Binary => RemoteValue::Binary(base64(v.as_str()?)?),
        RemoteType::Any | RemoteType::Record | RemoteType::Table => return None,
    })
}

fn two(s: &str) -> Option<u8> {
    if s.len() == 2 && s.bytes().all(|b| b.is_ascii_digit()) {
        s.parse().ok()
    } else {
        None
    }
}

/// `YYYY-MM-DD` (a year of four or more digits, maybe negative).
pub fn date(s: &str) -> Option<(i32, u8, u8)> {
    let (neg, body) = match s.strip_prefix('-') {
        Some(b) => (true, b),
        None => (false, s),
    };
    let mut it = body.split('-');
    let (y, m, d) = (it.next()?, it.next()?, it.next()?);
    if it.next().is_some() || y.len() < 4 || y.len() > 6 || !y.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let y: i32 = y.parse().ok()?;
    let (m, d) = (two(m)?, two(d)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some((if neg { -y } else { y }, m, d))
}

/// `hh:mm[:ss[.fraction]]` in milliseconds (the fraction cut to milliseconds).
pub fn time(s: &str) -> Option<u32> {
    let mut it = s.split(':');
    let h = u32::from(two(it.next()?)?);
    let m = u32::from(two(it.next()?)?);
    let (sec, ms) = match it.next() {
        None => (0, 0),
        Some(rest) => {
            let (sec, frac) = rest.split_once('.').unwrap_or((rest, ""));
            if !frac.bytes().all(|b| b.is_ascii_digit()) || frac.len() > 12 {
                return None;
            }
            let ms = frac
                .bytes()
                .chain(std::iter::repeat(b'0'))
                .take(3)
                .fold(0u32, |a, b| a * 10 + u32::from(b - b'0'));
            (u32::from(two(sec)?), ms)
        }
    };
    if it.next().is_some() || h > 23 || m > 59 || sec > 59 {
        return None;
    }
    Some(((h * 60 + m) * 60 + sec) * 1000 + ms)
}

/// Milliseconds since 1970-01-01 UTC as a civil date and time of day.
fn from_unix_ms(ms: &str) -> Option<(i32, u8, u8, u32)> {
    let ms: i64 = ms.parse().ok()?;
    let days = ms.div_euclid(86_400_000);
    let of_day = u32::try_from(ms.rem_euclid(86_400_000)).ok()?;
    // days to civil (proleptic Gregorian), H. Hinnant's algorithm
    let z = days.checked_add(719_468)?;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    Some((
        i32::try_from(y).ok()?,
        u8::try_from(m).ok()?,
        u8::try_from(d).ok()?,
        of_day,
    ))
}

/// Base64 (standard or URL-safe alphabet, padding optional): OData v4 JSON writes binaries
/// base64url-encoded, v3 standard.
pub fn base64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3 + 3);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for c in s.trim_end_matches('=').bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xFF) as u8);
        }
        acc &= (1 << bits) - 1;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn values() {
        assert_eq!(
            value(&json!("2024-02-29"), RemoteType::Date),
            Some(RemoteValue::Date {
                year: 2024,
                month: 2,
                day: 29
            })
        );
        assert_eq!(
            value(
                &json!("2024-02-29T10:20:30.1234567+02:00"),
                RemoteType::DateTimeZone
            ),
            Some(RemoteValue::DateTimeZone {
                year: 2024,
                month: 2,
                day: 29,
                millis: 37_230_123,
                offset_minutes: 120
            })
        );
        assert_eq!(
            value(&json!("1999-12-31T23:59:59Z"), RemoteType::DateTimeZone),
            Some(RemoteValue::DateTimeZone {
                year: 1999,
                month: 12,
                day: 31,
                millis: 86_399_000,
                offset_minutes: 0
            })
        );
        assert_eq!(
            value(&json!("/Date(86400000)/"), RemoteType::DateTime),
            Some(RemoteValue::DateTime {
                year: 1970,
                month: 1,
                day: 2,
                millis: 0
            })
        );
        assert_eq!(
            value(&json!("9007199254740993"), RemoteType::Int64),
            Some(RemoteValue::Int(9_007_199_254_740_993))
        );
        assert_eq!(value(&json!(70000), RemoteType::Int16), None);
        assert_eq!(
            value(&json!("12.50"), RemoteType::Decimal),
            Some(RemoteValue::Decimal("12.50".into()))
        );
        assert_eq!(
            value(&json!("INF"), RemoteType::Double),
            Some(RemoteValue::Double(f64::INFINITY))
        );
        assert_eq!(
            value(&json!("AQID_w"), RemoteType::Binary),
            Some(RemoteValue::Binary(vec![1, 2, 3, 255]))
        );
        assert_eq!(
            value(&json!("13:45:00.5"), RemoteType::Time),
            Some(RemoteValue::Time { millis: 49_500_500 })
        );
        assert_eq!(value(&json!(1), RemoteType::Text), None);
        assert_eq!(value(&json!(null), RemoteType::Text), Some(RemoteValue::Null));
        assert_eq!(value(&json!("2024-13-01"), RemoteType::Date), None);
    }

    #[test]
    fn folded_options() {
        let q = RemoteQuery {
            table: vec!["Products".into()],
            columns: Some(vec!["ID".into(), "Price".into()]),
            filter: vec![
                Pred::Compare {
                    column: "Price".into(),
                    op: CmpOp::Gt,
                    value: Literal::Int(5),
                },
                Pred::And(vec![
                    Pred::NotNull("ID".into()),
                    Pred::Compare {
                        column: "Name".into(),
                        op: CmpOp::Eq,
                        value: Literal::Text("O'Hara".into()),
                    },
                ]),
            ],
            sort: vec![
                SortKey {
                    column: "Price".into(),
                    descending: true,
                },
                SortKey {
                    column: "ID".into(),
                    descending: false,
                },
            ],
            limit: Some(10),
        };
        let o = system_options(&q);
        assert_eq!(
            o,
            [
                ("$select".to_string(), "ID,Price".to_string()),
                (
                    "$filter".to_string(),
                    "(Price gt 5) and ((ID ne null) and (Name eq 'O''Hara'))".to_string()
                ),
                ("$orderby".to_string(), "Price desc,ID".to_string()),
                ("$top".to_string(), "10".to_string()),
            ]
        );
    }
}
