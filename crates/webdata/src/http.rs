//! The HTTP side of both connectors: URLs (relative paths, query strings), one bounded request
//! through `ureq` (blocking, rustls), and the response cache of one connector.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bicraft_data::connect::{ConnectError, ConnectErrorKind};

use crate::Settings;
use crate::auth::{self, Credential, Lookup};

/// Longest URL a request takes, in bytes.
pub const MAX_URL: usize = 64 * 1024;
/// Most headers a request takes.
const MAX_HEADERS: usize = 64;
/// Most bytes one connector keeps in its response cache.
const MAX_CACHED: usize = 64 << 20;

pub(crate) fn net(m: impl Into<String>) -> ConnectError {
    ConnectError::new(ConnectErrorKind::Network, m)
}

pub(crate) fn unsupported(m: impl Into<String>) -> ConnectError {
    ConnectError::new(ConnectErrorKind::Unsupported, m)
}

pub(crate) fn source(m: impl Into<String>) -> ConnectError {
    ConnectError::new(ConnectErrorKind::DataSource, m)
}

/// Whether a byte is kept as is in a query component (RFC 3986 unreserved characters and the
/// sub-delimiters OData writes in its options: `,` `'` `(` `)` `:` `$` `/`).
fn kept(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(
            b,
            b'-' | b'.' | b'_' | b'~' | b',' | b'\'' | b'(' | b')' | b':' | b'$' | b'/'
        )
}

/// `text` percent-encoded for a query name or value (UTF-8, space as `%20`).
pub fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if kept(b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `text` percent-encoded for a path segment (`/` kept: a relative path may hold several).
fn encode_path(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric()
            || matches!(
                b,
                b'-' | b'.'
                    | b'_'
                    | b'~'
                    | b'/'
                    | b'('
                    | b')'
                    | b'\''
                    | b','
                    | b':'
                    | b'@'
                    | b'='
                    | b'%'
                    | b'$'
            )
        {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A checked absolute `http`/`https` URL.
pub fn check_url(url: &str) -> Result<(), ConnectError> {
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err(unsupported(format!(
            "`{}` is not an http or https URL",
            clip(url)
        )));
    }
    if url.len() > MAX_URL {
        return Err(unsupported(format!("the URL is longer than {MAX_URL} bytes")));
    }
    if url.chars().any(|c| c.is_control() || c == ' ') {
        return Err(unsupported(format!(
            "the URL `{}` holds a space or a control character",
            clip(url)
        )));
    }
    let rest = url.split_once("://").map_or("", |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() || host.contains('@') {
        return Err(unsupported(format!(
            "the URL `{}` names no host (or holds a user name)",
            clip(url)
        )));
    }
    Ok(())
}

/// The start of a long text, for messages.
pub fn clip(t: &str) -> String {
    match t.char_indices().nth(200) {
        Some((i, _)) => format!("{}…", t.get(..i).unwrap_or(t)),
        None => t.to_string(),
    }
}

/// `scheme://host[:port]` of a URL (lower case), to keep links on the feed's origin.
pub fn origin(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    format!("{}://{}", scheme.to_ascii_lowercase(), host.to_ascii_lowercase())
}

/// `base` with `relative` appended as a path (one `/` between them), before its query.
pub fn join_path(base: &str, relative: &str) -> String {
    let (path, query) = match base.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (base, None),
    };
    let rel = relative.trim_start_matches('/');
    let mut out = path.to_string();
    if !rel.is_empty() {
        if !out.ends_with('/') {
            out.push('/');
        }
        out.push_str(&encode_path(rel));
    }
    if let Some(q) = query {
        out.push('?');
        out.push_str(q);
    }
    out
}

/// `url` with `pairs` appended to its query string (encoded).
pub fn with_query(url: &str, pairs: &[(String, String)]) -> String {
    if pairs.is_empty() {
        return url.to_string();
    }
    // a fragment is never sent
    let url = url.split('#').next().unwrap_or(url);
    let mut out = url.to_string();
    let mut sep = if url.contains('?') {
        if url.ends_with('?') || url.ends_with('&') {
            ""
        } else {
            "&"
        }
    } else {
        "?"
    };
    for (k, v) in pairs {
        out.push_str(sep);
        out.push_str(&encode(k));
        out.push('=');
        out.push_str(&encode(v));
        sep = "&";
    }
    out
}

/// One request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Request {
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// A body: the request is a POST.
    pub body: Option<Vec<u8>>,
    pub timeout: Option<Duration>,
    /// `Web.Contents`'s `ApiKeyName`: the query parameter a Web API key credential is sent in.
    pub api_key_name: Option<String>,
}

/// What came back.
#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub reason: String,
    pub content_type: String,
    pub body: Arc<Vec<u8>>,
}

/// Header names the client manages itself.
fn managed(name: &str) -> bool {
    [
        "host",
        "content-length",
        "transfer-encoding",
        "connection",
        "upgrade",
        "te",
        "trailer",
    ]
    .iter()
    .any(|m| name.eq_ignore_ascii_case(m))
}

/// Checks headers given in M (`Headers = [...]`).
pub fn check_headers(headers: &[(String, String)]) -> Result<(), ConnectError> {
    if headers.len() > MAX_HEADERS {
        return Err(unsupported(format!("more than {MAX_HEADERS} headers")));
    }
    for (n, v) in headers {
        let token = !n.is_empty()
            && n.len() <= 256
            && n.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b));
        if !token {
            return Err(unsupported(format!("`{}` is not a header name", clip(n))));
        }
        if managed(n) {
            return Err(unsupported(format!("the header `{n}` cannot be set")));
        }
        if v.len() > 16 * 1024 || v.chars().any(|c| c == '\r' || c == '\n' || c == '\0') {
            return Err(unsupported(format!("the value of the header `{n}` is not valid")));
        }
    }
    Ok(())
}

/// The HTTP client of one connector, with its response cache (successful GETs and POSTs, by
/// request, up to [`MAX_CACHED`] bytes), so a query that reads a URL twice sends one request.
///
/// Redirects are followed here, not by `ureq`: each hop's credential is looked up for its own
/// URL ([`Lookup`]), so a credential never follows a redirect to another source, and a header
/// the query set itself named `Authorization` or `Cookie` is dropped when the origin changes.
pub struct Client {
    agent: ureq::Agent,
    settings: Settings,
    credentials: Option<Lookup>,
    cache: Mutex<(HashMap<Request, Response>, usize)>,
}

/// Whether a status is a redirect with a `Location`.
fn redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

/// `location` resolved against `url` (absolute, origin-relative or path-relative).
pub fn resolve(url: &str, location: &str) -> String {
    let l = location.trim();
    let lower = l.get(..8).unwrap_or(l).to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return l.to_string();
    }
    if let Some(rest) = l.strip_prefix("//") {
        let scheme = url.split("://").next().unwrap_or("https");
        return format!("{scheme}://{rest}");
    }
    if l.starts_with('/') {
        return format!("{}{l}", origin_as_written(url));
    }
    let path = url.split(['?', '#']).next().unwrap_or(url);
    match path.rfind('/') {
        Some(i) if i > path.find("://").map_or(0, |j| j + 2) => {
            format!("{}{l}", path.get(..=i).unwrap_or(path))
        }
        _ => format!("{path}/{l}"),
    }
}

/// `scheme://host[:port]` of a URL as written.
fn origin_as_written(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    format!("{scheme}://{host}")
}

impl Client {
    pub fn new(settings: Settings) -> Self {
        Self::with_credentials(settings, None)
    }

    /// A client that sends the credential `credentials` gives each request's URL.
    pub fn with_credentials(settings: Settings, credentials: Option<Lookup>) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .proxy(None)
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_global(Some(settings.timeout))
            .max_response_header_size(64 * 1024)
            .user_agent(concat!("bicraft/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Self {
            agent,
            settings,
            credentials,
            cache: Mutex::new((HashMap::new(), 0)),
        }
    }

    /// The credential of `url` (anonymous when none is stored).
    fn credential(&self, url: &str) -> Credential {
        self.credentials
            .as_ref()
            .and_then(|l| l(url))
            .unwrap_or(Credential::Anonymous)
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    fn cached(&self, r: &Request) -> Option<Response> {
        let c = match self.cache.lock() {
            Ok(c) => c,
            Err(p) => p.into_inner(),
        };
        c.0.get(r).cloned()
    }

    fn keep(&self, r: &Request, resp: &Response) {
        let mut c = match self.cache.lock() {
            Ok(c) => c,
            Err(p) => p.into_inner(),
        };
        let size = resp.body.len().saturating_add(r.url.len());
        if c.1.saturating_add(size) <= MAX_CACHED {
            c.1 = c.1.saturating_add(size);
            c.0.insert(r.clone(), resp.clone());
        }
    }

    /// Sends `r` (or answers from the cache unless `fresh`); a body over `max_bytes` (and the
    /// client's own cap) is [`ConnectErrorKind::Limit`]. Any status comes back: the caller
    /// decides what an error is.
    pub fn send(&self, r: &Request, max_bytes: usize, fresh: bool) -> Result<Response, ConnectError> {
        check_url(&r.url)?;
        if !fresh && let Some(hit) = self.cached(r) {
            return Ok(hit);
        }
        let mut hop = r.clone();
        let mut hops = 0u32;
        loop {
            let (status, location, out) = self.send_once(&hop, max_bytes)?;
            match location {
                Some(loc) if redirect(status) => {
                    hops = hops.saturating_add(1);
                    if hops > self.settings.max_redirects {
                        return Err(net(format!(
                            "{} redirected more than {} times",
                            clip(&r.url),
                            self.settings.max_redirects
                        )));
                    }
                    let next = resolve(&hop.url, &loc);
                    check_url(&next)?;
                    if origin(&next) != origin(&hop.url) {
                        hop.headers.retain(|(n, _)| {
                            !n.eq_ignore_ascii_case("authorization") && !n.eq_ignore_ascii_case("cookie")
                        });
                    }
                    if status == 303 || (matches!(status, 301 | 302) && hop.body.is_some()) {
                        hop.body = None;
                    }
                    hop.url = next;
                }
                _ => {
                    if (200..300).contains(&out.status) {
                        self.keep(r, &out);
                    }
                    return Ok(out);
                }
            }
        }
    }

    /// One request, without following redirects: its status, `Location` and response.
    fn send_once(
        &self,
        r: &Request,
        max_bytes: usize,
    ) -> Result<(u16, Option<String>, Response), ConnectError> {
        let mut url = r.url.clone();
        let mut headers = r.headers.clone();
        let credential = self.credential(&r.url);
        if r.api_key_name.is_some() && !matches!(credential, Credential::Key { .. }) {
            return Err(ConnectError::new(
                ConnectErrorKind::Credentials,
                format!(
                    "Web.Contents: `ApiKeyName` needs a Web API key credential for {} (Data source settings, \
                     source.credentials)",
                    clip(&r.url)
                ),
            ));
        }
        match credential {
            Credential::Anonymous => {}
            Credential::Basic { user, password } => {
                headers.push((
                    "Authorization".into(),
                    format!(
                        "Basic {}",
                        auth::base64(format!("{user}:{}", password.expose()).as_bytes())
                    ),
                ));
            }
            Credential::Bearer { token } => {
                headers.push(("Authorization".into(), format!("Bearer {}", token.expose())));
            }
            Credential::Key { key, header } => match (&r.api_key_name, header) {
                (Some(name), _) => url = with_query(&url, &[(name.clone(), key.expose().to_string())]),
                (None, Some(h)) => headers.push((h, key.expose().to_string())),
                (None, None) => {
                    return Err(ConnectError::new(
                        ConnectErrorKind::Credentials,
                        format!(
                            "the Web API key of {} needs the query's ApiKeyName option or a header name \
                             (source.credentials)",
                            clip(&r.url)
                        ),
                    ));
                }
            },
        }
        check_headers(&headers)?;
        let limit = u64::try_from(max_bytes)
            .unwrap_or(u64::MAX)
            .min(self.settings.max_response_bytes);
        let sent = match &r.body {
            Some(body) => {
                let mut b = self.agent.post(&url);
                for (n, v) in &headers {
                    b = b.header(n.as_str(), v.as_str());
                }
                if let Some(t) = r.timeout {
                    b = b.config().timeout_global(Some(t)).build();
                }
                b.send(body.as_slice())
            }
            None => {
                let mut b = self.agent.get(&url);
                for (n, v) in &headers {
                    b = b.header(n.as_str(), v.as_str());
                }
                if let Some(t) = r.timeout {
                    b = b.config().timeout_global(Some(t)).build();
                }
                b.call()
            }
        };
        // the URL may hold a key: messages name the request's own URL
        let resp = sent.map_err(|e| {
            net(format!(
                "the request to {} failed: {}",
                clip(&r.url),
                auth::scrub(&e.to_string(), &url)
            ))
        })?;
        let status = resp.status();
        let location = resp
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let body = resp
            .into_body()
            .into_with_config()
            .limit(limit)
            .read_to_vec()
            .map_err(|e| match e {
                ureq::Error::BodyExceedsLimit(n) => ConnectError::new(
                    ConnectErrorKind::Limit,
                    format!("the response of {} is larger than {n} bytes", clip(&r.url)),
                ),
                e => net(format!("reading the response of {} failed: {e}", clip(&r.url))),
            })?;
        let out = Response {
            status: status.as_u16(),
            reason: status.canonical_reason().unwrap_or("").to_string(),
            content_type,
            body: Arc::new(body),
        };
        Ok((out.status, location, out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(join_path("http://h/api", "v1/x y"), "http://h/api/v1/x%20y");
        assert_eq!(join_path("http://h/api/?a=1", "/v1"), "http://h/api/v1?a=1");
        assert_eq!(
            with_query("http://h/p?a=1", &[("q".into(), "a b&c".into())]),
            "http://h/p?a=1&q=a%20b%26c"
        );
        assert_eq!(
            with_query("http://h/p", &[("$filter".into(), "Id gt 5".into())]),
            "http://h/p?$filter=Id%20gt%205"
        );
        assert!(check_url("ftp://h/x").is_err());
        assert!(check_url("http://user@h/x").is_err());
        assert!(check_url("http:///x").is_err());
        assert!(check_url("https://h.example.com/x?y=1").is_ok());
        assert_eq!(origin("HTTP://H:80/a/b"), "http://h:80");
        assert!(check_headers(&[("Host".into(), "x".into())]).is_err());
        assert!(check_headers(&[("X-A".into(), "a\r\nb".into())]).is_err());
        assert!(check_headers(&[("Accept".into(), "text/csv".into())]).is_ok());
        assert_eq!(resolve("http://h/a/b?x", "c"), "http://h/a/c");
        assert_eq!(resolve("http://h/a/b", "/z"), "http://h/z");
        assert_eq!(resolve("https://h/a", "//k/q"), "https://k/q");
        assert_eq!(resolve("http://h/a", "HTTPS://o/p"), "HTTPS://o/p");
    }
}
