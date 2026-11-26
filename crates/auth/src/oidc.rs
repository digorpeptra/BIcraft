//! OIDC / OAuth 2.1 issuers: discovery, JWKS fetch and caching with key rotation, and the
//! validation of one bearer JWT (docs/auth.md, "OIDC").

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use crate::jwk::{self, Alg, Jwk};
use crate::jwt::{self, JwtError, Parsed};
use crate::{Identity, Secret, Source};

/// Largest discovery or JWKS document read (bytes).
const MAX_DOC_BYTES: u64 = 1024 * 1024;
/// Network timeout of one discovery or JWKS request.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// One accepted issuer.
#[derive(Debug, Clone)]
pub struct IssuerConfig {
    /// The exact `iss` value (and the discovery base).
    pub issuer: String,
    /// Accepted `aud` values besides the server's own resource URLs.
    pub audiences: Vec<String>,
    /// The JWKS URL; discovered from the issuer's metadata when unset.
    pub jwks_uri: Option<String>,
    /// Accepted signature algorithms (default: the asymmetric ones). HMAC algorithms need
    /// [`IssuerConfig::hmac_secret`] and an explicit listing.
    pub algorithms: Vec<Alg>,
    pub hmac_secret: Option<Secret>,
    /// Clock skew allowed on `exp`, `nbf` and `iat` (seconds; default 60, at most 300).
    pub leeway_secs: u64,
    /// Claim naming the user (default `sub`; Entra ID: `oid` is stable across apps).
    pub subject_claim: String,
    /// Claim holding the e-mail (default `email`; Entra ID: `preferred_username` or `upn`).
    pub email_claim: String,
    /// Claim holding the groups (default `groups`). A dotted path reaches nested claims.
    pub groups_claim: Option<String>,
    /// Claim holding the roles (default `roles`; Keycloak: `realm_access.roles`).
    pub roles_claim: Option<String>,
    /// Claim value → BIcraft role names. Values not listed are kept as they are.
    pub role_map: HashMap<String, Vec<String>>,
    /// Scopes every token must carry (403 `insufficient_scope` otherwise).
    pub required_scopes: Vec<String>,
    /// Claims that must hold a value: a string claim equal to it, or an array containing it.
    pub required_claims: Vec<(String, String)>,
    /// Keys are refetched after this time (seconds; default 3600).
    pub jwks_refresh_secs: u64,
    /// An unknown `kid` refetches the keys at most this often (seconds; default 30).
    pub jwks_min_refetch_secs: u64,
    /// Listed in the protected resource metadata's `authorization_servers` (default true).
    pub advertise: bool,
}

impl IssuerConfig {
    /// An issuer with the defaults.
    pub fn new(issuer: impl Into<String>) -> Self {
        Self {
            issuer: issuer.into(),
            audiences: Vec::new(),
            jwks_uri: None,
            algorithms: Alg::ASYMMETRIC.to_vec(),
            hmac_secret: None,
            leeway_secs: 60,
            subject_claim: "sub".into(),
            email_claim: "email".into(),
            groups_claim: Some("groups".into()),
            roles_claim: Some("roles".into()),
            role_map: HashMap::new(),
            required_scopes: Vec::new(),
            required_claims: Vec::new(),
            jwks_refresh_secs: 3600,
            jwks_min_refetch_secs: 30,
            advertise: true,
        }
    }

    /// Checks the configuration (called by [`crate::Authenticator::new`]).
    pub fn check(&self) -> Result<(), String> {
        let who = &self.issuer;
        if !fetchable(&self.issuer) {
            return Err(format!(
                "OIDC issuer `{who}` must be an https:// URL (http:// only for a loopback host)"
            ));
        }
        if let Some(j) = &self.jwks_uri
            && !fetchable(j)
        {
            return Err(format!("jwks_uri of `{who}` must be an https:// URL"));
        }
        if self.algorithms.is_empty() {
            return Err(format!("OIDC issuer `{who}`: no signature algorithm accepted"));
        }
        let hmac = self.algorithms.iter().any(|a| a.is_hmac());
        if hmac && self.hmac_secret.is_none() {
            return Err(format!(
                "OIDC issuer `{who}`: HS256/HS384/HS512 need hmac_secret_file or hmac_secret_env"
            ));
        }
        if !hmac && self.hmac_secret.is_some() {
            return Err(format!(
                "OIDC issuer `{who}`: an HMAC secret is set but no HS* algorithm is listed"
            ));
        }
        if self.hmac_secret.as_ref().is_some_and(|s| s.0.len() < 32) {
            return Err(format!(
                "OIDC issuer `{who}`: the HMAC secret must be at least 32 bytes"
            ));
        }
        if self.leeway_secs > 300 {
            return Err(format!("OIDC issuer `{who}`: leeway_secs is at most 300"));
        }
        Ok(())
    }
}

/// `https://…`, or `http://` to a loopback host (local identity providers and tests).
pub fn fetchable(url: &str) -> bool {
    if url.len() > 2048 || url.contains('#') {
        return false;
    }
    if let Some(rest) = strip_scheme(url, "https://") {
        return !host_of(rest).is_empty();
    }
    strip_scheme(url, "http://").is_some_and(|rest| loopback_host(host_of(rest)))
}

fn strip_scheme<'a>(url: &'a str, scheme: &str) -> Option<&'a str> {
    let head = url.get(..scheme.len())?;
    head.eq_ignore_ascii_case(scheme)
        .then(|| url.get(scheme.len()..))
        .flatten()
}

/// The host of `host[:port]/path…` (brackets kept for IPv6).
fn host_of(rest: &str) -> &str {
    let authority = rest.split(['/', '?']).next().unwrap_or_default();
    if authority.contains('@') {
        return "";
    }
    if authority.starts_with('[') {
        return authority.split(']').next().map_or("", |h| {
            authority.get(..h.len().saturating_add(1)).unwrap_or_default()
        });
    }
    authority.split(':').next().unwrap_or_default()
}

fn loopback_host(host: &str) -> bool {
    let h = host.to_ascii_lowercase();
    h == "localhost" || h == "[::1]" || h.parse::<std::net::Ipv4Addr>().is_ok_and(|ip| ip.is_loopback())
}

/// Seconds since the Unix epoch.
pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

#[derive(Default)]
struct KeyCache {
    jwks_uri: Option<String>,
    keys: Vec<Jwk>,
    fetched: Option<Instant>,
    last_forced: Option<Instant>,
    /// The last failed refresh: an unreachable issuer is retried at most every
    /// `min(jwks_min_refetch_secs, 10)` seconds, not on every request.
    last_failure: Option<Instant>,
}

/// An issuer at run time: its configuration and its key cache.
pub struct Issuer {
    pub config: IssuerConfig,
    cache: Mutex<KeyCache>,
    https: ureq::Agent,
    loopback: ureq::Agent,
}

fn agent(https_only: bool) -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .https_only(https_only)
        .max_redirects(if https_only { 3 } else { 0 })
        .proxy(None)
        .timeout_global(Some(FETCH_TIMEOUT))
        .user_agent(concat!("bicraft/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

impl Issuer {
    pub fn new(config: IssuerConfig) -> Self {
        Self {
            cache: Mutex::new(KeyCache {
                jwks_uri: config.jwks_uri.clone(),
                ..KeyCache::default()
            }),
            config,
            https: agent(true),
            loopback: agent(false),
        }
    }

    /// GETs a JSON document (capped, 200 only).
    fn fetch(&self, url: &str) -> Option<Value> {
        if !fetchable(url) {
            return None;
        }
        let agent = if strip_scheme(url, "https://").is_some() {
            &self.https
        } else {
            &self.loopback
        };
        let resp = agent.get(url).header("Accept", "application/json").call().ok()?;
        if resp.status().as_u16() != 200 {
            return None;
        }
        let bytes = resp
            .into_body()
            .into_with_config()
            .limit(MAX_DOC_BYTES)
            .read_to_vec()
            .ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// The JWKS URL from the issuer's metadata: OpenID Connect discovery
    /// (`{issuer}/.well-known/openid-configuration`), then RFC 8414
    /// (`/.well-known/oauth-authorization-server{path}`). The document's `issuer` must be
    /// identical to the configured one.
    fn discover(&self) -> Option<String> {
        let issuer = &self.config.issuer;
        let base = issuer.trim_end_matches('/');
        let mut urls = vec![format!("{base}/.well-known/openid-configuration")];
        if let Some(rfc8414) = rfc8414_url(issuer) {
            urls.push(rfc8414);
        }
        for url in urls {
            let Some(doc) = self.fetch(&url) else { continue };
            if doc.get("issuer").and_then(Value::as_str) != Some(issuer.as_str()) {
                continue;
            }
            if let Some(j) = doc.get("jwks_uri").and_then(Value::as_str)
                && fetchable(j)
            {
                return Some(j.to_string());
            }
        }
        None
    }

    /// Refreshes the cached keys (discovery first when needed). On failure the old keys stay.
    fn refresh(&self, cache: &mut KeyCache) -> bool {
        if cache.jwks_uri.is_none() {
            cache.jwks_uri = self.discover();
        }
        let doc = cache.jwks_uri.clone().and_then(|uri| self.fetch(&uri));
        let Some(doc) = doc else {
            cache.last_failure = Some(Instant::now());
            return false;
        };
        cache.keys = jwk::parse_set(&doc);
        cache.fetched = Some(Instant::now());
        cache.last_failure = None;
        true
    }

    /// Whether a refresh may be tried now (backoff after a failure).
    fn may_retry(&self, cache: &KeyCache) -> bool {
        let backoff = Duration::from_secs(self.config.jwks_min_refetch_secs.min(10));
        cache.last_failure.is_none_or(|t| t.elapsed() >= backoff)
    }

    /// Fetches the keys now (startup warm-up); whether keys are available.
    pub fn prefetch(&self) -> bool {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        self.refresh(&mut cache) && !cache.keys.is_empty()
    }

    /// Verifies the signature of `parsed` with this issuer's keys.
    fn verify_signature(&self, parsed: &Parsed<'_>, alg: Alg) -> Result<(), JwtError> {
        let message = parsed.signing_input.as_bytes();
        if alg.is_hmac() {
            let secret = self
                .config
                .hmac_secret
                .as_ref()
                .ok_or(JwtError::AlgorithmRefused)?;
            return if jwk::verify_hmac(alg, &secret.0, message, &parsed.signature) {
                Ok(())
            } else {
                Err(JwtError::BadSignature)
            };
        }
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let refresh_every = Duration::from_secs(self.config.jwks_refresh_secs);
        if cache.fetched.is_none_or(|t| t.elapsed() >= refresh_every) && self.may_retry(&cache) {
            self.refresh(&mut cache);
        }
        let candidates = |keys: &[Jwk]| -> Vec<Jwk> {
            keys.iter()
                .filter(|k| match &parsed.kid {
                    Some(kid) => k.kid.as_deref() == Some(kid.as_str()),
                    None => true,
                })
                .filter(|k| k.fits(alg))
                .cloned()
                .collect()
        };
        let mut keys = candidates(&cache.keys);
        if keys.is_empty() {
            // a rotated key: refetch once, rate limited so random kids cannot flood the issuer
            let min = Duration::from_secs(self.config.jwks_min_refetch_secs);
            if cache.last_forced.is_none_or(|t| t.elapsed() >= min) && self.may_retry(&cache) {
                cache.last_forced = Some(Instant::now());
                self.refresh(&mut cache);
                keys = candidates(&cache.keys);
            }
        }
        if keys.is_empty() {
            return Err(if cache.fetched.is_none() {
                JwtError::KeysUnavailable
            } else {
                JwtError::UnknownKey
            });
        }
        drop(cache);
        if keys.iter().any(|k| k.verify(alg, message, &parsed.signature)) {
            Ok(())
        } else {
            Err(JwtError::BadSignature)
        }
    }

    /// Validates a token whose unverified `iss` named this issuer: algorithm, signature, times,
    /// audience (one of `audiences`, or of the issuer's own list) and required claims.
    pub fn validate(&self, parsed: &Parsed<'_>, audiences: &[String]) -> Result<Identity, JwtError> {
        let c = &self.config;
        let alg = jwt::algorithm(parsed, &c.algorithms)?;
        self.verify_signature(parsed, alg)?;
        let claims = &parsed.claims;
        if claims.get("iss").and_then(Value::as_str) != Some(c.issuer.as_str()) {
            return Err(JwtError::UnknownIssuer);
        }
        let leeway = i64::try_from(c.leeway_secs).unwrap_or(300);
        jwt::check_times(claims, now_secs(), leeway)?;
        let aud = jwt::audiences(claims);
        let accepted =
            |a: &str| c.audiences.iter().any(|x| x == a) || audiences.iter().any(|x| same_resource(x, a));
        if !aud.iter().any(|a| accepted(a)) {
            return Err(JwtError::WrongAudience);
        }
        for (name, want) in &c.required_claims {
            if !jwt::claim_strings(claims, name, false).iter().any(|v| v == want) {
                return Err(JwtError::ClaimMismatch);
            }
        }
        Ok(self.identity(claims))
    }

    fn identity(&self, claims: &Map<String, Value>) -> Identity {
        let c = &self.config;
        let first = |path: &str| jwt::claim_strings(claims, path, false).into_iter().next();
        let subject = first(&c.subject_claim).unwrap_or_default();
        let email = first(&c.email_claim);
        let groups = c
            .groups_claim
            .as_deref()
            .map(|p| jwt::claim_strings(claims, p, true))
            .unwrap_or_default();
        let mut roles = Vec::new();
        if let Some(p) = c.roles_claim.as_deref() {
            for v in jwt::claim_strings(claims, p, true) {
                match c.role_map.get(&v) {
                    Some(mapped) => roles.extend(mapped.iter().cloned()),
                    None => roles.push(v),
                }
            }
        }
        Identity {
            source: Source::Oidc {
                issuer: c.issuer.clone(),
            },
            subject,
            email,
            groups,
            roles,
            scopes: jwt::scopes(claims),
        }
    }
}

/// The RFC 8414 metadata URL of `issuer` (`/.well-known/oauth-authorization-server` inserted
/// between the host and the path).
fn rfc8414_url(issuer: &str) -> Option<String> {
    let scheme_end = issuer.find("://")?.checked_add(3)?;
    let rest = issuer.get(scheme_end..)?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (rest.get(..i)?, rest.get(i..)?),
        None => (rest, ""),
    };
    let scheme = issuer.get(..scheme_end)?;
    Some(format!(
        "{scheme}{authority}/.well-known/oauth-authorization-server{}",
        path.trim_end_matches('/')
    ))
}

/// Two resource URIs name the same resource: equal after lowercasing the scheme and host and
/// dropping one trailing `/` (the MCP spec asks servers to accept uppercase schemes and hosts).
pub fn same_resource(a: &str, b: &str) -> bool {
    normalize_resource(a) == normalize_resource(b)
}

fn normalize_resource(u: &str) -> String {
    let u = u.strip_suffix('/').unwrap_or(u);
    match u.find("://") {
        Some(i) => {
            let start = i.saturating_add(3);
            let rest = u.get(start..).unwrap_or_default();
            let end = rest.find('/').unwrap_or(rest.len());
            format!(
                "{}{}{}",
                u.get(..start).unwrap_or_default().to_ascii_lowercase(),
                rest.get(..end).unwrap_or_default().to_ascii_lowercase(),
                rest.get(end..).unwrap_or_default()
            )
        }
        None => u.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert!(fetchable("https://login.example.com/tenant/v2.0"));
        assert!(fetchable("http://127.0.0.1:8080/realms/x"));
        assert!(fetchable("http://localhost/x"));
        assert!(fetchable("http://[::1]:9/x"));
        assert!(!fetchable("http://idp.example.com"));
        assert!(!fetchable("http://127.0.0.1.evil.example/x"));
        assert!(!fetchable("https://user@host/x"));
        assert!(!fetchable("ftp://x"));
        assert!(!fetchable("https://"));
        assert_eq!(
            rfc8414_url("https://a.example/tenant1/").as_deref(),
            Some("https://a.example/.well-known/oauth-authorization-server/tenant1")
        );
        assert_eq!(
            rfc8414_url("https://a.example").as_deref(),
            Some("https://a.example/.well-known/oauth-authorization-server")
        );
        assert!(same_resource(
            "HTTPS://BI.example.com/w/a/mcp",
            "https://bi.example.com/w/a/mcp/"
        ));
        assert!(!same_resource(
            "https://bi.example.com/w/A/mcp",
            "https://bi.example.com/w/a/mcp"
        ));
    }
}
