//! Data source settings (Power Query's *Data source settings* dialog, Microsoft Learn,
//! *Privacy levels in Power Query* and *Authentication with a data source*): the credentials of
//! web sources (`source.credentials {kind: web, url, authentication}`) and the privacy level of
//! each data source (`source.privacy`), held by the session in memory only, and the list of the
//! model's data sources with what is set for each (`source.settings`).
//!
//! Web credentials (anonymous, basic, Web API key, bearer token) are stored for a URL prefix and
//! serve `Web.Contents`, `OData.Feed` and `SharePoint.*` (`bicraft_webdata::auth`). Privacy
//! levels (None, Public, Organizational, Private) are recorded and shown; BIcraft does not run
//! Power Query's formula firewall, so they do not change how queries are evaluated (G69). See
//! docs/formats/data-sources.md, *Credentials and privacy levels*.

use std::collections::BTreeMap;

use bicraft_webdata::auth::{self, Credential, Secret};
use serde_json::{Value, json};

use crate::Session;
use crate::params::{allow, opt_bool, opt_str, str_param};

/// Longest secret or user name accepted, in bytes.
const MAX_SECRET: usize = 16 * 1024;
/// Longest data source name accepted, in bytes.
const MAX_SOURCE: usize = 8 * 1024;
/// Most privacy levels a session keeps.
const MAX_LEVELS: usize = 1000;

/// A privacy level (Learn, *Privacy levels in Power Query*).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivacyLevel {
    None,
    Public,
    Organizational,
    Private,
}

impl PrivacyLevel {
    pub const ALL: [PrivacyLevel; 4] = [
        PrivacyLevel::None,
        PrivacyLevel::Public,
        PrivacyLevel::Organizational,
        PrivacyLevel::Private,
    ];

    pub fn name(self) -> &'static str {
        match self {
            PrivacyLevel::None => "none",
            PrivacyLevel::Public => "public",
            PrivacyLevel::Organizational => "organizational",
            PrivacyLevel::Private => "private",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.name().eq_ignore_ascii_case(s))
    }
}

/// The privacy level of each data source, by its kind and normalized name.
#[derive(Debug, Clone, Default)]
pub struct PrivacyLevels(BTreeMap<(String, String), PrivacyLevel>);

impl PrivacyLevels {
    pub fn get(&self, kind: &str, source: &str) -> Option<PrivacyLevel> {
        self.0.get(&(kind.to_string(), normalize(kind, source))).copied()
    }
}

/// A data source's name as settings key it: URLs as `auth::normalize`, servers in lower case,
/// paths as written.
fn normalize(kind: &str, source: &str) -> String {
    let t = source.trim();
    match kind {
        "web" => auth::normalize(t),
        "sqlserver" => t.to_ascii_lowercase(),
        _ => t.to_string(),
    }
}

fn kind_param(p: &Value) -> Result<&'static str, String> {
    Ok(
        match opt_str(p, "kind")?.map(str::to_ascii_lowercase).as_deref() {
            None | Some("web") | Some("odata") | Some("sharepoint") => "web",
            Some("sqlserver") => "sqlserver",
            Some("file") | Some("folder") => "file",
            Some(k) => return Err(format!("unknown `kind` `{k}` (expected web, sqlserver or file)")),
        },
    )
}

fn secret<'a>(p: &'a Value, key: &str) -> Result<&'a str, String> {
    let v = match p.get(key) {
        None | Some(Value::Null) => "",
        Some(Value::String(s)) => s.as_str(),
        Some(_) => return Err(format!("`{key}` must be text")),
    };
    if v.len() > MAX_SECRET || v.chars().any(|c| c == '\r' || c == '\n' || c == '\0') {
        return Err(format!(
            "`{key}` is longer than {MAX_SECRET} bytes or holds a line break"
        ));
    }
    Ok(v)
}

/// The checked URL of a web source.
fn web_url(p: &Value) -> Result<String, String> {
    let url = str_param(p, "url")?.trim();
    bicraft_webdata::http::check_url(url).map_err(|e| e.message)?;
    if url.len() > MAX_SOURCE {
        return Err(format!("`url` is longer than {MAX_SOURCE} bytes"));
    }
    Ok(url.to_string())
}

/// `source.credentials {kind: web | odata | sharepoint, url, authentication, …}`.
pub(crate) fn web_credentials(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "kind",
            "url",
            "authentication",
            "user",
            "password",
            "key",
            "header",
            "token",
            "clear",
        ],
    )?;
    let url = web_url(p)?;
    let prefix = auth::normalize(&url);
    if opt_bool(p, "clear")?.unwrap_or(false) {
        let cleared = s.web_credentials.clear(&url);
        return Ok(json!({"kind": "web", "url": prefix, "stored": false, "cleared": cleared}));
    }
    let method = opt_str(p, "authentication")?
        .unwrap_or("anonymous")
        .to_ascii_lowercase();
    let c = match method.as_str() {
        "anonymous" => Credential::Anonymous,
        "basic" => {
            let user = str_param(p, "user")?;
            if user.is_empty()
                || user.len() > 1024
                || user.contains(':')
                || user.chars().any(char::is_control)
            {
                return Err("`user` must be a user name of 1 to 1024 bytes, without `:`".into());
            }
            Credential::Basic {
                user: user.to_string(),
                password: Secret::new(secret(p, "password")?),
            }
        }
        "key" => {
            let key = secret(p, "key")?;
            if key.is_empty() {
                return Err("a Web API key credential needs `key`".into());
            }
            let header = match opt_str(p, "header")?.map(str::trim) {
                None | Some("") => None,
                Some(h) => {
                    bicraft_webdata::http::check_headers(&[(h.to_string(), String::new())])
                        .map_err(|e| e.message)?;
                    if h.eq_ignore_ascii_case("cookie") {
                        return Err("the key's `header` cannot be Cookie".into());
                    }
                    Some(h.to_string())
                }
            };
            Credential::Key {
                key: Secret::new(key),
                header,
            }
        }
        "bearer" | "organizational" => {
            let token = secret(p, "token")?.trim();
            if token.is_empty() {
                return Err("a bearer credential needs `token` (an access token obtained elsewhere)".into());
            }
            Credential::Bearer {
                token: Secret::new(token),
            }
        }
        "windows" => {
            return Err(
                "Windows authentication is not supported by bicraft's web connectors: use basic, a Web API key \
                 or a bearer token"
                    .into(),
            );
        }
        other => {
            return Err(format!(
                "unknown `authentication` `{other}` (expected anonymous, basic, key or bearer)"
            ));
        }
    };
    let kind = c.kind();
    if !s.web_credentials.set(&url, c) {
        return Err(format!(
            "this session already holds {} web credentials",
            auth::MAX_STORED
        ));
    }
    Ok(json!({"kind": "web", "url": prefix, "authentication": kind, "stored": true}))
}

/// `source.privacy`: the privacy level of a data source for this session.
pub(crate) fn source_privacy(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["kind", "source", "level", "clear"])?;
    let kind = kind_param(p)?;
    let source = str_param(p, "source")?;
    if source.trim().is_empty() || source.len() > MAX_SOURCE || source.contains('\0') {
        return Err(format!(
            "`source` must name a data source of at most {MAX_SOURCE} bytes"
        ));
    }
    if kind == "web" {
        bicraft_webdata::http::check_url(source.trim()).map_err(|e| e.message)?;
    }
    let key = (kind.to_string(), normalize(kind, source));
    if opt_bool(p, "clear")?.unwrap_or(false) {
        let cleared = s.privacy.0.remove(&key).is_some();
        return Ok(json!({"kind": kind, "source": key.1, "level": null, "cleared": cleared}));
    }
    let text = str_param(p, "level")?;
    let level = PrivacyLevel::parse(text).ok_or_else(|| {
        format!("unknown `level` `{text}` (expected none, public, organizational or private)")
    })?;
    if !s.privacy.0.contains_key(&key) && s.privacy.0.len() >= MAX_LEVELS {
        return Err(format!("this session already holds {MAX_LEVELS} privacy levels"));
    }
    s.privacy.0.insert(key.clone(), level);
    Ok(json!({"kind": kind, "source": key.1, "level": level.name()}))
}

/// The kind and name of a data source an argument of `function` names.
fn source_of(function: &str, role: &str, value: &str) -> Option<(&'static str, String)> {
    match role {
        "url" => Some(("web", value.to_string())),
        "server" if function.starts_with("Sql.") => Some(("sqlserver", value.to_string())),
        "path" | "folder" => Some(("file", value.to_string())),
        _ => None,
    }
}

/// `source.settings`: the model's data sources (literal arguments of its source functions) and
/// the stored web credentials, each with its credential kind (never the secret) and privacy
/// level.
pub(crate) fn source_settings(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &[])?;
    // (kind, normalized) -> (shown, functions)
    let mut found: BTreeMap<(String, String), (String, Vec<String>)> = BTreeMap::new();
    if s.model.is_some() {
        let listed = crate::sources::sources_list(s, &json!({}))?;
        for src in listed
            .get("sources")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let function = src.get("function").and_then(Value::as_str).unwrap_or("");
            for a in src
                .get("arguments")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let (Some(role), Some(value)) = (
                    a.get("role").and_then(Value::as_str),
                    a.get("value").and_then(Value::as_str),
                ) else {
                    continue;
                };
                if let Some((kind, shown)) = source_of(function, role, value) {
                    let e = found
                        .entry((kind.to_string(), normalize(kind, &shown)))
                        .or_insert_with(|| (shown, Vec::new()));
                    if !e.1.iter().any(|f| f == function) {
                        e.1.push(function.to_string());
                    }
                }
            }
        }
    }
    for (prefix, _) in s.web_credentials.list() {
        found
            .entry(("web".to_string(), auth::normalize(&prefix)))
            .or_insert_with(|| (prefix.clone(), Vec::new()));
    }
    for (kind, name) in s.privacy.0.keys() {
        found
            .entry((kind.clone(), name.clone()))
            .or_insert_with(|| (name.clone(), Vec::new()));
    }
    let sources: Vec<Value> = found
        .into_iter()
        .map(|((kind, key), (shown, functions))| {
            let credential = match kind.as_str() {
                "web" => s.web_credentials.get(&shown).map(|c| c.kind()),
                "sqlserver" => s.sql_credentials.get(&shown).map(|_| "database"),
                _ => None,
            };
            json!({
                "kind": kind,
                "source": shown,
                "key": key,
                "functions": functions,
                "credential": credential,
                "privacy": s.privacy.0.get(&(kind.clone(), key.clone())).map(|l| l.name()),
            })
        })
        .collect();
    Ok(json!({"sources": sources, "firewall": false}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_privacy_and_listing() {
        let mut s = Session::new();
        let fails = |s: &mut Session, f: fn(&mut Session, &Value) -> Result<Value, String>, p: Value| {
            assert!(f(s, &p).is_err(), "{p}");
        };
        for p in [
            json!({"kind": "web"}),
            json!({"kind": "web", "url": "ftp://x/y"}),
            json!({"kind": "web", "url": "https://h/", "authentication": "basic"}),
            json!({"kind": "web", "url": "https://h/", "authentication": "basic", "user": "a:b"}),
            json!({"kind": "web", "url": "https://h/", "authentication": "key"}),
            json!({"kind": "web", "url": "https://h/", "authentication": "key", "key": "k", "header": "Host"}),
            json!({"kind": "web", "url": "https://h/", "authentication": "bearer", "token": " "}),
            json!({"kind": "web", "url": "https://h/", "authentication": "windows"}),
            json!({"kind": "web", "url": "https://h/", "authentication": "bearer", "token": "a\nb"}),
            json!({"kind": "web", "url": "https://h/", "bogus": 1}),
            json!({"kind": "web", "url": 3}),
        ] {
            fails(&mut s, web_credentials, p);
        }
        let r = web_credentials(
            &mut s,
            &json!({"kind": "sharepoint", "url": "https://Contoso.example.com/sites/a/", "authentication": "bearer", "token": "tok-secret"}),
        )
        .unwrap();
        assert_eq!(r["url"], "https://contoso.example.com/sites/a");
        assert!(!r.to_string().contains("tok-secret"));
        for p in [
            json!({"source": "https://h/"}),
            json!({"source": "https://h/", "level": "secret"}),
            json!({"source": "not a url", "level": "public"}),
            json!({"kind": "bogus", "source": "x", "level": "public"}),
            json!({"source": "", "level": "public"}),
        ] {
            fails(&mut s, source_privacy, p);
        }
        source_privacy(
            &mut s,
            &json!({"source": "https://contoso.example.com/sites/a", "level": "Organizational"}),
        )
        .unwrap();
        source_privacy(
            &mut s,
            &json!({"kind": "sqlserver", "source": "SRV", "level": "private"}),
        )
        .unwrap();
        let v = source_settings(&s, &json!({})).unwrap();
        let list = v["sources"].as_array().unwrap();
        assert_eq!(list.len(), 2, "{v}");
        let web = list.iter().find(|x| x["kind"] == "web").unwrap();
        assert_eq!(web["credential"], "bearer");
        assert_eq!(web["privacy"], "organizational");
        assert!(!v.to_string().contains("tok-secret"));
        assert_eq!(s.privacy.get("sqlserver", "srv"), Some(PrivacyLevel::Private));
        assert_eq!(
            source_privacy(
                &mut s,
                &json!({"kind": "sqlserver", "source": "srv", "clear": true})
            )
            .unwrap()["cleared"],
            true
        );
        assert!(source_settings(&s, &json!({"x": 1})).is_err());
    }
}
