//! Authentication of `bicraft-cli serve` and the workspace ACL (docs/auth.md).
//!
//! Three modes, any combination:
//!
//! - **OIDC / OAuth 2.1 resource server** ([`oidc`]), per the MCP authorization spec (revision
//!   2026-07-28): bearer JWTs from configured issuers, keys found by discovery and JWKS (cached,
//!   refetched on rotation), signature (RS*, PS*, ES256/384, EdDSA; HS* only when explicitly
//!   configured; `none` never), `iss`, `exp`/`nbf`/`iat` with leeway, `aud` bound to this server,
//!   optional required scopes and claims, claims mapped to groups and roles.
//! - **Trusted identity header** ([`proxy`]) from an auth proxy, accepted only from configured
//!   proxy addresses.
//! - **Static API tokens** ([`tokens`]), stored as SHA-256, compared in constant time.
//!
//! An authenticated [`Identity`] then needs a grant in the [`acl::Acl`] for the workspace.
//! Nothing here logs or returns a token or a secret.
//!
//! L0: no workspace dependency, no async runtime (discovery and JWKS use a blocking client).

pub mod acl;
pub mod b64;
pub mod jwk;
pub mod jwt;
pub mod oidc;
pub mod proxy;
pub mod tokens;

use std::net::IpAddr;
use std::path::Path;

use serde_json::{Map, Value, json};

pub use acl::{Access, Acl, Grant};
pub use jwk::Alg;
pub use oidc::{Issuer, IssuerConfig};
pub use proxy::{Cidr, ProxyConfig};
pub use tokens::TokenEntry;

/// Where an identity comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Oidc { issuer: String },
    Proxy,
    Token,
}

/// An authenticated caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub source: Source,
    /// The subject claim, the proxy's user header, or the token's name.
    pub subject: String,
    pub email: Option<String>,
    pub groups: Vec<String>,
    pub roles: Vec<String>,
    pub scopes: Vec<String>,
}

impl Identity {
    /// A short label for logs: the mode and the subject (never a credential).
    pub fn label(&self) -> String {
        match &self.source {
            Source::Oidc { .. } => format!("oidc:{}", self.subject),
            Source::Proxy => format!("proxy:{}", self.subject),
            Source::Token => format!("token:{}", self.subject),
        }
    }
}

/// A secret value: never printed.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(pub Vec<u8>);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// Why a request is not authenticated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Denial {
    /// No credentials (or another scheme than Bearer): 401 with the challenge.
    Missing,
    /// A bearer token that is not valid here: 401 `invalid_token`.
    InvalidToken(&'static str),
    /// A valid token without the required scopes: 403 `insufficient_scope`.
    InsufficientScope(Vec<String>),
    /// A malformed `Authorization` header, or identity headers from an untrusted address: 400
    /// `invalid_request`.
    InvalidRequest(&'static str),
}

/// The whole authentication configuration.
#[derive(Debug, Clone, Default)]
pub struct AuthConfig {
    /// The server's public origin (`https://bicraft.example.com`): resource identifiers,
    /// metadata URLs and the default audience derive from it.
    pub public_url: Option<String>,
    pub issuers: Vec<IssuerConfig>,
    /// `scopes_supported` of the protected resource metadata and the `scope` of challenges.
    /// Default: the issuers' required scopes.
    pub scopes_supported: Vec<String>,
    pub proxy: Option<ProxyConfig>,
    pub tokens: Vec<TokenEntry>,
    pub acl: Acl,
}

impl AuthConfig {
    /// Whether any mode is configured.
    pub fn any_mode(&self) -> bool {
        !self.issuers.is_empty() || self.proxy.is_some() || !self.tokens.is_empty()
    }

    /// Reads a configuration file (JSON, docs/auth.md). Relative file names inside it
    /// (`tokens_file`, `hmac_secret_file`) are relative to the file's folder.
    pub fn from_file(path: &Path) -> Result<AuthConfig, String> {
        let doc = read_json(path)?;
        let base = path.parent().unwrap_or(Path::new("."));
        let mut c = AuthConfig::default();
        c.merge_json(&doc, base)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(c)
    }

    /// Adds the settings of a configuration document.
    pub fn merge_json(&mut self, doc: &Value, base: &Path) -> Result<(), String> {
        let o = doc.as_object().ok_or("the auth configuration is a JSON object")?;
        known(
            o,
            &[
                "public_url",
                "oidc",
                "scopes_supported",
                "proxy",
                "tokens",
                "tokens_file",
                "acl",
                "comment",
            ],
            "auth configuration",
        )?;
        if let Some(u) = o.get("public_url") {
            self.public_url = Some(as_str(u, "public_url")?.to_string());
        }
        if let Some(list) = o.get("oidc") {
            let list = list.as_array().ok_or("`oidc` is a list of issuers")?;
            for i in list {
                self.issuers.push(issuer_from_json(i, base)?);
            }
        }
        if let Some(s) = o.get("scopes_supported") {
            self.scopes_supported.extend(str_list(s, "scopes_supported")?);
        }
        if let Some(p) = o.get("proxy") {
            self.proxy = Some(proxy_from_json(p)?);
        }
        if let Some(t) = o.get("tokens") {
            self.add_tokens(tokens::parse(t)?)?;
        }
        if let Some(f) = o.get("tokens_file") {
            let path = base.join(as_str(f, "tokens_file")?);
            self.add_tokens(load_tokens_file(&path)?)?;
        }
        if let Some(a) = o.get("acl") {
            let a = a.as_array().ok_or("`acl` is a list of grants")?;
            for g in a {
                self.acl.grants.push(Grant::parse_json(g)?);
            }
        }
        Ok(())
    }

    /// Adds tokens, refusing duplicate names or hashes.
    pub fn add_tokens(&mut self, new: Vec<TokenEntry>) -> Result<(), String> {
        for t in new {
            if self
                .tokens
                .iter()
                .any(|e| e.name == t.name || e.sha256 == t.sha256)
            {
                return Err(format!("token `{}` is configured twice", t.name));
            }
            self.tokens.push(t);
        }
        Ok(())
    }
}

/// Reads a tokens file (`{"tokens": [...]}`).
pub fn load_tokens_file(path: &Path) -> Result<Vec<TokenEntry>, String> {
    tokens::parse(&read_json(path)?).map_err(|e| format!("{}: {e}", path.display()))
}

fn read_json(path: &Path) -> Result<Value, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.len() > 4 * 1024 * 1024 {
        return Err(format!("{}: larger than 4 MiB", path.display()));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("{}: not JSON ({e})", path.display()))
}

fn known(o: &Map<String, Value>, keys: &[&str], what: &str) -> Result<(), String> {
    match o.keys().find(|k| !keys.contains(&k.as_str())) {
        Some(k) => Err(format!("{what}: unknown field `{k}`")),
        None => Ok(()),
    }
}

fn as_str<'a>(v: &'a Value, what: &str) -> Result<&'a str, String> {
    v.as_str().ok_or(format!("`{what}` is a string"))
}

fn str_list(v: &Value, what: &str) -> Result<Vec<String>, String> {
    match v {
        Value::String(s) => Ok(vec![s.clone()]),
        Value::Array(a) => a
            .iter()
            .map(|x| x.as_str().map(str::to_string))
            .collect::<Option<Vec<_>>>()
            .ok_or(format!("`{what}` is a list of strings")),
        _ => Err(format!("`{what}` is a list of strings")),
    }
}

fn number(v: &Value, what: &str) -> Result<u64, String> {
    v.as_u64().ok_or(format!("`{what}` is a non-negative integer"))
}

fn issuer_from_json(v: &Value, base: &Path) -> Result<IssuerConfig, String> {
    let o = v.as_object().ok_or("an OIDC issuer is an object")?;
    known(
        o,
        &[
            "issuer",
            "audiences",
            "jwks_uri",
            "algorithms",
            "hmac_secret_file",
            "hmac_secret_env",
            "leeway_secs",
            "subject_claim",
            "email_claim",
            "groups_claim",
            "roles_claim",
            "role_map",
            "required_scopes",
            "required_claims",
            "jwks_refresh_secs",
            "jwks_min_refetch_secs",
            "advertise",
            "comment",
        ],
        "OIDC issuer",
    )?;
    let issuer = o
        .get("issuer")
        .and_then(Value::as_str)
        .ok_or("an OIDC issuer needs `issuer`")?;
    let mut c = IssuerConfig::new(issuer);
    if let Some(a) = o.get("audiences") {
        c.audiences = str_list(a, "audiences")?;
    }
    if let Some(j) = o.get("jwks_uri") {
        c.jwks_uri = Some(as_str(j, "jwks_uri")?.to_string());
    }
    if let Some(a) = o.get("algorithms") {
        c.algorithms = str_list(a, "algorithms")?
            .iter()
            .map(|s| Alg::parse(s).ok_or(format!("unknown or refused algorithm `{s}`")))
            .collect::<Result<_, _>>()?;
    }
    match (o.get("hmac_secret_file"), o.get("hmac_secret_env")) {
        (Some(_), Some(_)) => return Err("give hmac_secret_file or hmac_secret_env, not both".into()),
        (Some(f), None) => {
            let path = base.join(as_str(f, "hmac_secret_file")?);
            let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let trimmed = trim_ascii_end(&bytes);
            c.hmac_secret = Some(Secret(trimmed.to_vec()));
        }
        (None, Some(e)) => {
            let name = as_str(e, "hmac_secret_env")?;
            let value = std::env::var(name).map_err(|_| format!("hmac_secret_env: `{name}` is not set"))?;
            c.hmac_secret = Some(Secret(value.into_bytes()));
        }
        (None, None) => {}
    }
    if let Some(n) = o.get("leeway_secs") {
        c.leeway_secs = number(n, "leeway_secs")?;
    }
    if let Some(s) = o.get("subject_claim") {
        c.subject_claim = as_str(s, "subject_claim")?.to_string();
    }
    if let Some(s) = o.get("email_claim") {
        c.email_claim = as_str(s, "email_claim")?.to_string();
    }
    for (key, slot) in [
        ("groups_claim", &mut c.groups_claim),
        ("roles_claim", &mut c.roles_claim),
    ] {
        match o.get(key) {
            None => {}
            Some(Value::Null) => *slot = None,
            Some(v) => *slot = Some(as_str(v, key)?.to_string()),
        }
    }
    if let Some(m) = o.get("role_map") {
        let m = m
            .as_object()
            .ok_or("`role_map` maps claim values to role names")?;
        for (k, v) in m {
            c.role_map.insert(k.clone(), str_list(v, "role_map")?);
        }
    }
    if let Some(s) = o.get("required_scopes") {
        c.required_scopes = str_list(s, "required_scopes")?;
    }
    if let Some(m) = o.get("required_claims") {
        let m = m
            .as_object()
            .ok_or("`required_claims` maps claim names to values")?;
        for (k, v) in m {
            c.required_claims
                .push((k.clone(), as_str(v, "required_claims")?.to_string()));
        }
    }
    if let Some(n) = o.get("jwks_refresh_secs") {
        c.jwks_refresh_secs = number(n, "jwks_refresh_secs")?.max(60);
    }
    if let Some(n) = o.get("jwks_min_refetch_secs") {
        c.jwks_min_refetch_secs = number(n, "jwks_min_refetch_secs")?;
    }
    if let Some(a) = o.get("advertise") {
        c.advertise = a.as_bool().ok_or("`advertise` is true or false")?;
    }
    Ok(c)
}

fn trim_ascii_end(b: &[u8]) -> &[u8] {
    let n = b.iter().rev().take_while(|c| c.is_ascii_whitespace()).count();
    b.get(..b.len().saturating_sub(n)).unwrap_or_default()
}

fn proxy_from_json(v: &Value) -> Result<ProxyConfig, String> {
    let o = v.as_object().ok_or("`proxy` is an object")?;
    known(
        o,
        &[
            "trusted",
            "user_header",
            "email_header",
            "groups_header",
            "groups_separator",
            "comment",
        ],
        "proxy",
    )?;
    let trusted = str_list(o.get("trusted").unwrap_or(&Value::Null), "proxy.trusted")
        .map_err(|_| "`proxy.trusted` lists the proxy addresses or CIDR ranges".to_string())?
        .iter()
        .map(|s| Cidr::parse(s))
        .collect::<Result<Vec<_>, _>>()?;
    let mut c = ProxyConfig::new(trusted);
    if let Some(h) = o.get("user_header") {
        c.user_header = as_str(h, "user_header")?.to_string();
    }
    for (key, slot) in [
        ("email_header", &mut c.email_header),
        ("groups_header", &mut c.groups_header),
    ] {
        match o.get(key) {
            None => {}
            Some(Value::Null) => *slot = None,
            Some(v) => *slot = Some(as_str(v, key)?.to_string()),
        }
    }
    if let Some(s) = o.get("groups_separator") {
        c.groups_separator = as_str(s, "groups_separator")?.to_string();
    }
    Ok(c)
}

/// `scheme://host[:port]` with no path, query or fragment.
fn valid_origin(u: &str) -> bool {
    let rest = match u.split_once("://") {
        Some((s, r)) if s.eq_ignore_ascii_case("https") || s.eq_ignore_ascii_case("http") => r,
        _ => return false,
    };
    !rest.is_empty()
        && !rest.contains(['/', '?', '#', '@', ' ', '"', '\\'])
        && rest.bytes().all(|b| b.is_ascii_graphic())
}

/// Largest header value read for identities (bytes).
const MAX_IDENTITY_HEADER: usize = 4096;

/// The authenticator: configured modes plus the ACL.
pub struct Authenticator {
    public_url: Option<String>,
    issuers: Vec<Issuer>,
    scopes_supported: Vec<String>,
    proxy: Option<ProxyConfig>,
    tokens: Vec<TokenEntry>,
    acl: Acl,
}

impl std::fmt::Debug for Authenticator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.summary())
    }
}

impl Authenticator {
    /// Checks the configuration: at least one mode and one grant, valid issuers and proxy.
    pub fn new(config: AuthConfig) -> Result<Self, String> {
        if !config.any_mode() {
            return Err("no authentication mode is configured".into());
        }
        if config.acl.grants.is_empty() {
            return Err(
                "no ACL grant: nobody could open a workspace (add --grant SUBJECT=WORKSPACES, e.g. \
                 --grant 'group:bi-team=*')"
                    .into(),
            );
        }
        let public_url = match config.public_url {
            Some(u) => {
                let u = u.trim_end_matches('/').to_string();
                if !valid_origin(&u) {
                    return Err(format!(
                        "public URL `{u}` must be scheme://host[:port] with no path (https://bicraft.example.com)"
                    ));
                }
                Some(u)
            }
            None => None,
        };
        for i in &config.issuers {
            i.check()?;
        }
        for (n, i) in config.issuers.iter().enumerate() {
            if config
                .issuers
                .iter()
                .skip(n.saturating_add(1))
                .any(|j| j.issuer == i.issuer)
            {
                return Err(format!("OIDC issuer `{}` is configured twice", i.issuer));
            }
        }
        if let Some(p) = &config.proxy {
            p.check()?;
        }
        let mut scopes = config.scopes_supported;
        if scopes.is_empty() {
            for i in &config.issuers {
                for s in &i.required_scopes {
                    if !scopes.contains(s) {
                        scopes.push(s.clone());
                    }
                }
            }
        }
        if let Some(bad) = scopes.iter().find(|s| !scope_token(s)) {
            return Err(format!("`{bad}` is not a valid scope name"));
        }
        Ok(Self {
            public_url,
            issuers: config.issuers.into_iter().map(Issuer::new).collect(),
            scopes_supported: scopes,
            proxy: config.proxy,
            tokens: config.tokens,
            acl: config.acl,
        })
    }

    pub fn public_url(&self) -> Option<&str> {
        self.public_url.as_deref()
    }

    pub fn has_oidc(&self) -> bool {
        !self.issuers.is_empty()
    }

    /// Header names the server must accept once only (identity headers and `Authorization`).
    pub fn unique_headers(&self) -> Vec<String> {
        let mut v = vec!["authorization".to_string()];
        if let Some(p) = &self.proxy {
            v.extend(p.header_names().iter().map(|h| h.to_ascii_lowercase()));
        }
        v
    }

    /// The modes and counts, for the startup line (no secret).
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if !self.issuers.is_empty() {
            let names: Vec<&str> = self.issuers.iter().map(|i| i.config.issuer.as_str()).collect();
            parts.push(format!("OIDC ({})", names.join(", ")));
        }
        if let Some(p) = &self.proxy {
            parts.push(format!(
                "trusted header {} from {} range(s)",
                p.user_header,
                p.trusted.len()
            ));
        }
        if !self.tokens.is_empty() {
            parts.push(format!("{} API token(s)", self.tokens.len()));
        }
        format!("{}; {} ACL grant(s)", parts.join(" + "), self.acl.grants.len())
    }

    /// Fetches every issuer's keys now; the issuers whose keys are unavailable.
    pub fn prefetch(&self) -> Vec<String> {
        self.issuers
            .iter()
            .filter(|i| !i.config.algorithms.iter().all(|a| a.is_hmac()) && !i.prefetch())
            .map(|i| i.config.issuer.clone())
            .collect()
    }

    /// Authenticates a request from `peer`. `header` reads a request header (case-insensitive);
    /// `resources` are this request's own resource identifiers (the workspace endpoint URL and
    /// the server's public URL), accepted as token audiences.
    pub fn authenticate<'h>(
        &self,
        peer: IpAddr,
        header: impl Fn(&str) -> Option<&'h str>,
        resources: &[String],
    ) -> Result<Identity, Denial> {
        if let Some(p) = &self.proxy
            && let Some(user) = header(&p.user_header)
        {
            if !p.trusts(peer) {
                return Err(Denial::InvalidRequest(
                    "identity header from an untrusted address",
                ));
            }
            return proxy_identity(p, user, &header);
        }
        let Some(auth) = header("authorization") else {
            return Err(Denial::Missing);
        };
        let token = match bearer(auth) {
            Bearer::Token(t) => t,
            Bearer::OtherScheme => return Err(Denial::Missing),
            Bearer::Malformed => {
                return Err(Denial::InvalidRequest("malformed Authorization header"));
            }
        };
        if let Some(e) = tokens::find(&self.tokens, token) {
            return Ok(Identity {
                source: Source::Token,
                subject: e.name.clone(),
                email: None,
                groups: e.groups.clone(),
                roles: e.roles.clone(),
                scopes: Vec::new(),
            });
        }
        if self.issuers.is_empty() || token.starts_with(tokens::PREFIX) {
            return Err(Denial::InvalidToken("unknown token"));
        }
        let parsed = jwt::parse(token).map_err(|e| Denial::InvalidToken(e.description()))?;
        let iss = parsed.claims.get("iss").and_then(Value::as_str);
        let Some(issuer) = self
            .issuers
            .iter()
            .find(|i| Some(i.config.issuer.as_str()) == iss)
        else {
            return Err(Denial::InvalidToken(jwt::JwtError::UnknownIssuer.description()));
        };
        let mut audiences: Vec<String> = resources.to_vec();
        if let Some(u) = &self.public_url {
            audiences.push(u.clone());
        }
        let id = issuer
            .validate(&parsed, &audiences)
            .map_err(|e| Denial::InvalidToken(e.description()))?;
        let required = &issuer.config.required_scopes;
        if !required.iter().all(|s| id.scopes.contains(s)) {
            return Err(Denial::InsufficientScope(required.clone()));
        }
        Ok(id)
    }

    /// The access `id` has on `workspace` (`None`: closed; answer as if it did not exist).
    pub fn access(&self, id: &Identity, workspace: &str) -> Option<Access> {
        self.acl.access(id, workspace)
    }

    /// The protected resource metadata (RFC 9728) of `resource`. `None` without an OIDC issuer
    /// (the MCP spec requires at least one authorization server in it).
    pub fn resource_metadata(&self, resource: &str) -> Option<Value> {
        let servers: Vec<&str> = self
            .issuers
            .iter()
            .filter(|i| i.config.advertise)
            .map(|i| i.config.issuer.as_str())
            .collect();
        if servers.is_empty() {
            return None;
        }
        let mut doc = json!({
            "resource": resource,
            "authorization_servers": servers,
            "bearer_methods_supported": ["header"],
            "resource_name": "BIcraft",
        });
        if !self.scopes_supported.is_empty()
            && let Some(o) = doc.as_object_mut()
        {
            o.insert("scopes_supported".into(), json!(self.scopes_supported));
        }
        Some(doc)
    }

    /// The status and `WWW-Authenticate` value for a denial (RFC 6750 §3, RFC 9728 §5.1, MCP
    /// authorization "Error Handling"). `metadata_url` is the resource metadata URL, given when
    /// OIDC is configured.
    pub fn challenge(&self, denial: &Denial, metadata_url: Option<&str>) -> (u16, String) {
        let mut params: Vec<(&str, String)> = Vec::new();
        let status = match denial {
            Denial::Missing => 401,
            Denial::InvalidToken(why) => {
                params.push(("error", "invalid_token".into()));
                params.push(("error_description", (*why).into()));
                401
            }
            Denial::InsufficientScope(scopes) => {
                params.push(("error", "insufficient_scope".into()));
                params.push(("scope", scopes.join(" ")));
                params.push(("error_description", "the token lacks a required scope".into()));
                403
            }
            Denial::InvalidRequest(why) => {
                params.push(("error", "invalid_request".into()));
                params.push(("error_description", (*why).into()));
                400
            }
        };
        let metadata = metadata_url.filter(|_| self.has_oidc());
        match metadata {
            Some(url) => params.insert(0, ("resource_metadata", url.to_string())),
            None => params.insert(0, ("realm", "bicraft".to_string())),
        }
        if matches!(denial, Denial::Missing | Denial::InvalidToken(_))
            && metadata.is_some()
            && !self.scopes_supported.is_empty()
        {
            params.push(("scope", self.scopes_supported.join(" ")));
        }
        let rendered: Vec<String> = params
            .iter()
            .map(|(k, v)| format!("{k}=\"{}\"", quote(v)))
            .collect();
        (status, format!("Bearer {}", rendered.join(", ")))
    }
}

/// RFC 6749 scope token: visible ASCII except `"` and `\`.
fn scope_token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b == b'!' || (b'#'..=b'[').contains(&b) || (b']'..=b'~').contains(&b))
}

/// A quoted-string body: `"` and `\` escaped, controls and non-ASCII dropped.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            ' '..='~' => out.push(c),
            _ => {}
        }
    }
    out
}

enum Bearer<'a> {
    Token(&'a str),
    OtherScheme,
    Malformed,
}

/// `Bearer <token68>` (scheme case-insensitive, RFC 6750 §2.1).
fn bearer(value: &str) -> Bearer<'_> {
    let value = value.trim();
    let (scheme, rest) = match value.split_once([' ', '\t']) {
        Some((s, r)) => (s, r.trim()),
        None => (value, ""),
    };
    if !scheme.eq_ignore_ascii_case("bearer") {
        return Bearer::OtherScheme;
    }
    let token68 =
        |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'+' | b'/' | b'=');
    if rest.is_empty() || rest.len() > jwt::MAX_TOKEN_BYTES || !rest.bytes().all(token68) {
        return Bearer::Malformed;
    }
    Bearer::Token(rest)
}

fn proxy_identity<'h>(
    p: &ProxyConfig,
    user: &str,
    header: &impl Fn(&str) -> Option<&'h str>,
) -> Result<Identity, Denial> {
    let clean = |v: &str| -> Option<String> {
        let v = v.trim();
        (!v.is_empty() && v.len() <= MAX_IDENTITY_HEADER && !v.chars().any(char::is_control))
            .then(|| v.to_string())
    };
    let subject = clean(user).ok_or(Denial::InvalidRequest("empty or invalid identity header"))?;
    let email = p.email_header.as_deref().and_then(header).and_then(clean);
    let groups = p
        .groups_header
        .as_deref()
        .and_then(header)
        .map(|g| {
            g.split(p.groups_separator.as_str())
                .filter_map(clean)
                .take(1024)
                .collect()
        })
        .unwrap_or_default();
    Ok(Identity {
        source: Source::Proxy,
        subject,
        email,
        groups,
        roles: Vec::new(),
        scopes: Vec::new(),
    })
}

#[cfg(test)]
mod tests;
