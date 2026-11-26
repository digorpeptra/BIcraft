//! Credentials of web sources (`Web.Contents`, `OData.Feed`, `SharePoint.*`): the kinds Power
//! Query's Web connector offers that a pure-Rust client can send (Microsoft Learn, *Web
//! connector*, *Authentication with a data source*): anonymous, basic (user name and password),
//! a Web API key (sent in the query parameter `Web.Contents`'s `ApiKeyName` names, or in a header
//! the caller names) and a bearer token (an organizational account's access token obtained
//! elsewhere: BIcraft does not run the sign-in itself).
//!
//! A credential is stored for a URL prefix ([`Store`]), the "level" Desktop asks for when it
//! stores one: a request uses the credential of the longest stored prefix of its URL (whole path
//! segments, scheme and host without regard to case). Secrets live in memory only, are never
//! printed ([`Secret`]'s `Debug`) and never appear in an error message.

use std::sync::{Arc, Mutex};

/// A secret text: never printed.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(s: &str) -> Self {
        Self(s.to_string())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(…)")
    }
}

/// How a request to a web source authenticates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Credential {
    Anonymous,
    /// `Authorization: Basic base64(user:password)`.
    Basic {
        user: String,
        password: Secret,
    },
    /// A Web API key: in the query parameter `ApiKeyName` names, else in `header`.
    Key {
        key: Secret,
        header: Option<String>,
    },
    /// `Authorization: Bearer token`.
    Bearer {
        token: Secret,
    },
}

impl Credential {
    /// The kind, as `source.credentials` names it.
    pub fn kind(&self) -> &'static str {
        match self {
            Credential::Anonymous => "anonymous",
            Credential::Basic { .. } => "basic",
            Credential::Key { .. } => "key",
            Credential::Bearer { .. } => "bearer",
        }
    }
}

/// The credential a URL is sent with (`None`: anonymous).
pub type Lookup = Arc<dyn Fn(&str) -> Option<Credential> + Send + Sync>;

/// Most prefixes a store keeps.
pub const MAX_STORED: usize = 1000;

/// A URL as a stored prefix: scheme and host in lower case, the path as written, without query,
/// fragment or trailing `/`.
pub fn normalize(url: &str) -> String {
    let url = url.trim();
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let (scheme, rest) = path.split_once("://").unwrap_or(("", path));
    let (host, tail) = match rest.find('/') {
        Some(i) => (rest.get(..i).unwrap_or(rest), rest.get(i..).unwrap_or("")),
        None => (rest, ""),
    };
    format!(
        "{}://{}{}",
        scheme.to_ascii_lowercase(),
        host.to_ascii_lowercase(),
        tail.trim_end_matches('/')
    )
}

/// Whether `prefix` (normalized) covers `url` (normalized): equal, or a parent path.
fn covers(prefix: &str, url: &str) -> bool {
    url == prefix || (url.starts_with(prefix) && url.as_bytes().get(prefix.len()) == Some(&b'/'))
}

/// Credentials by URL prefix, shared by the connectors of a session.
#[derive(Clone, Default)]
pub struct Store(Arc<Mutex<Vec<(String, Credential)>>>);

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Store({} sources)", self.list().len())
    }
}

impl Store {
    fn map(&self) -> std::sync::MutexGuard<'_, Vec<(String, Credential)>> {
        match self.0.lock() {
            Ok(m) => m,
            Err(p) => p.into_inner(),
        }
    }

    /// Stores `c` for the prefix `url`; false when the store is full.
    pub fn set(&self, url: &str, c: Credential) -> bool {
        let key = normalize(url);
        let mut m = self.map();
        if let Some(e) = m.iter_mut().find(|(k, _)| *k == key) {
            e.1 = c;
            return true;
        }
        if m.len() >= MAX_STORED {
            return false;
        }
        m.push((key, c));
        true
    }

    pub fn clear(&self, url: &str) -> bool {
        let key = normalize(url);
        let mut m = self.map();
        let before = m.len();
        m.retain(|(k, _)| *k != key);
        m.len() != before
    }

    /// The credential of the longest stored prefix of `url`.
    pub fn get(&self, url: &str) -> Option<Credential> {
        let u = normalize(url);
        self.map()
            .iter()
            .filter(|(k, _)| covers(k, &u))
            .max_by_key(|(k, _)| k.len())
            .map(|(_, c)| c.clone())
    }

    /// The stored prefixes and their kinds (never the secrets), sorted.
    pub fn list(&self) -> Vec<(String, &'static str)> {
        let mut v: Vec<_> = self.map().iter().map(|(k, c)| (k.clone(), c.kind())).collect();
        v.sort();
        v
    }

    /// The store as a [`Lookup`].
    pub fn lookup(&self) -> Lookup {
        let s = self.clone();
        Arc::new(move |url: &str| s.get(url))
    }
}

/// Standard base64 with padding (RFC 4648 §4).
pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let b = [
            c.first().copied().unwrap_or(0),
            c.get(1).copied().unwrap_or(0),
            c.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18u32, 12, 6, 0].iter().enumerate() {
            if i <= c.len() {
                let idx = usize::try_from((n >> shift) & 63).unwrap_or(0);
                out.push(char::from(T.get(idx).copied().unwrap_or(b'A')));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// `message` without `secret_url` (a URL that holds a key): replaced by its part before `?`.
pub fn scrub(message: &str, secret_url: &str) -> String {
    if secret_url.is_empty() || !message.contains(secret_url) {
        return message.to_string();
    }
    let shown = secret_url.split('?').next().unwrap_or("");
    message.replace(secret_url, &format!("{shown}?…"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_and_secrets() {
        let s = Store::default();
        s.set(
            "HTTPS://Contoso.example.com/sites/a/",
            Credential::Bearer {
                token: Secret::new("t1"),
            },
        );
        s.set(
            "https://contoso.example.com/sites/a/sub",
            Credential::Basic {
                user: "u".into(),
                password: Secret::new("hunter2"),
            },
        );
        assert_eq!(
            s.get("https://contoso.example.com/sites/a").map(|c| c.kind()),
            Some("bearer")
        );
        assert_eq!(
            s.get("https://contoso.example.com/sites/a/x?y=1")
                .map(|c| c.kind()),
            Some("bearer")
        );
        assert_eq!(
            s.get("https://contoso.example.com/sites/a/sub/z")
                .map(|c| c.kind()),
            Some("basic")
        );
        // a sibling that shares the text prefix is not covered
        assert!(s.get("https://contoso.example.com/sites/ab").is_none());
        assert!(s.get("http://contoso.example.com/sites/a").is_none());
        assert!(!format!("{s:?} {:?}", s.get("https://contoso.example.com/sites/a/sub")).contains("hunter2"));
        assert!(s.clear("https://contoso.example.com/sites/a/sub/"));
        assert_eq!(s.list().len(), 1);
        assert_eq!(base64(b"user:pass"), "dXNlcjpwYXNz");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b""), "");
        assert_eq!(
            scrub("failed: http://h/p?k=1 x", "http://h/p?k=1"),
            "failed: http://h/p?… x"
        );
    }
}
