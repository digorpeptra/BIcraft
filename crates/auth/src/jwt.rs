//! Compact JWS parsing and the registered-claim checks of a JWT access token (RFC 7519, RFC 9068).

use serde_json::{Map, Value};

use crate::b64;
use crate::jwk::Alg;

/// Largest token accepted, in bytes (the request head is capped anyway).
pub const MAX_TOKEN_BYTES: usize = 16 * 1024;

/// A parsed, not yet verified, token.
#[derive(Debug)]
pub struct Parsed<'a> {
    /// `header.payload`, the signed bytes.
    pub signing_input: &'a str,
    pub alg_name: String,
    pub kid: Option<String>,
    pub signature: Vec<u8>,
    pub claims: Map<String, Value>,
}

/// Why a token is refused. The text is safe to send back (`error_description`): no token part,
/// no claim value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JwtError {
    Malformed,
    /// `alg` is `none`, unknown, or not allowed for this issuer.
    AlgorithmRefused,
    /// A `crit` header names extensions BIcraft does not understand.
    CriticalHeader,
    UnknownIssuer,
    UnknownKey,
    BadSignature,
    Expired,
    /// `exp` is missing: access tokens must expire.
    NoExpiry,
    NotYetValid,
    IssuedInFuture,
    WrongAudience,
    /// A claim the issuer's configuration requires is missing or different.
    ClaimMismatch,
    /// The issuer's keys cannot be fetched now.
    KeysUnavailable,
}

impl JwtError {
    pub fn description(self) -> &'static str {
        match self {
            JwtError::Malformed => "malformed token",
            JwtError::AlgorithmRefused => "signature algorithm not accepted",
            JwtError::CriticalHeader => "unsupported critical header",
            JwtError::UnknownIssuer => "issuer not accepted",
            JwtError::UnknownKey => "unknown signing key",
            JwtError::BadSignature => "invalid signature",
            JwtError::Expired => "token expired",
            JwtError::NoExpiry => "token has no expiry",
            JwtError::NotYetValid => "token not yet valid",
            JwtError::IssuedInFuture => "token issued in the future",
            JwtError::WrongAudience => "token not issued for this resource",
            JwtError::ClaimMismatch => "required claim missing or different",
            JwtError::KeysUnavailable => "issuer keys unavailable",
        }
    }
}

/// Splits and decodes a compact JWS (`header.payload.signature`). The payload must be a JSON
/// object (JWE and unsecured JWTs are refused).
pub fn parse(token: &str) -> Result<Parsed<'_>, JwtError> {
    if token.is_empty() || token.len() > MAX_TOKEN_BYTES {
        return Err(JwtError::Malformed);
    }
    let mut parts = token.split('.');
    let (Some(h), Some(p), Some(s), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return Err(JwtError::Malformed);
    };
    let signing_input = token
        .get(..h.len().saturating_add(1).saturating_add(p.len()))
        .ok_or(JwtError::Malformed)?;
    let header: Value = b64::decode(h)
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or(JwtError::Malformed)?;
    let Value::Object(header) = header else {
        return Err(JwtError::Malformed);
    };
    let alg_name = header
        .get("alg")
        .and_then(Value::as_str)
        .ok_or(JwtError::Malformed)?
        .to_string();
    if alg_name.eq_ignore_ascii_case("none") {
        return Err(JwtError::AlgorithmRefused);
    }
    if header.contains_key("crit") || header.contains_key("enc") {
        return Err(JwtError::CriticalHeader);
    }
    let kid = match header.get("kid") {
        None => None,
        Some(Value::String(k)) => Some(k.clone()),
        Some(_) => return Err(JwtError::Malformed),
    };
    let claims: Value = b64::decode(p)
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or(JwtError::Malformed)?;
    let Value::Object(claims) = claims else {
        return Err(JwtError::Malformed);
    };
    let signature = b64::decode(s).ok_or(JwtError::Malformed)?;
    if signature.is_empty() {
        return Err(JwtError::Malformed);
    }
    Ok(Parsed {
        signing_input,
        alg_name,
        kid,
        signature,
        claims,
    })
}

/// The allowed algorithm named by the header, or the refusal.
pub fn algorithm(parsed: &Parsed<'_>, allowed: &[Alg]) -> Result<Alg, JwtError> {
    Alg::parse(&parsed.alg_name)
        .filter(|a| allowed.contains(a))
        .ok_or(JwtError::AlgorithmRefused)
}

/// A NumericDate claim as whole seconds (fractions are allowed by RFC 7519 and truncated).
fn date(claims: &Map<String, Value>, name: &str) -> Result<Option<i64>, JwtError> {
    match claims.get(name) {
        None => Ok(None),
        Some(Value::Number(n)) => n
            .as_i64()
            .or_else(|| n.as_f64().filter(|f| f.is_finite()).map(|f| f as i64))
            .map(Some)
            .ok_or(JwtError::Malformed),
        Some(_) => Err(JwtError::Malformed),
    }
}

/// Checks `exp` (required), `nbf` and `iat` against `now` with `leeway` seconds.
pub fn check_times(claims: &Map<String, Value>, now: i64, leeway: i64) -> Result<(), JwtError> {
    let exp = date(claims, "exp")?.ok_or(JwtError::NoExpiry)?;
    if now >= exp.saturating_add(leeway) {
        return Err(JwtError::Expired);
    }
    if let Some(nbf) = date(claims, "nbf")?
        && now < nbf.saturating_sub(leeway)
    {
        return Err(JwtError::NotYetValid);
    }
    if let Some(iat) = date(claims, "iat")?
        && iat > now.saturating_add(leeway)
    {
        return Err(JwtError::IssuedInFuture);
    }
    Ok(())
}

/// The `aud` values (a string or an array of strings).
pub fn audiences(claims: &Map<String, Value>) -> Vec<&str> {
    match claims.get("aud") {
        Some(Value::String(s)) => vec![s.as_str()],
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).take(64).collect(),
        _ => Vec::new(),
    }
}

/// The granted scopes: `scope` (RFC 9068, space separated) or `scp` (Entra ID and Okta: a space
/// separated string or an array).
pub fn scopes(claims: &Map<String, Value>) -> Vec<String> {
    let mut out = Vec::new();
    for name in ["scope", "scp"] {
        match claims.get(name) {
            Some(Value::String(s)) => out.extend(s.split_whitespace().map(str::to_string)),
            Some(Value::Array(a)) => out.extend(a.iter().filter_map(Value::as_str).map(str::to_string)),
            _ => {}
        }
    }
    out.truncate(256);
    out
}

/// A claim by path: the whole name first (namespaced claims such as
/// `https://example.com/roles` contain dots), then a dotted path into nested objects
/// (`realm_access.roles`, Keycloak).
pub fn claim<'a>(claims: &'a Map<String, Value>, path: &str) -> Option<&'a Value> {
    if let Some(v) = claims.get(path) {
        return Some(v);
    }
    let mut parts = path.split('.').take(16);
    let mut cur = claims.get(parts.next()?)?;
    for p in parts {
        cur = cur.as_object()?.get(p)?;
    }
    Some(cur)
}

/// A claim's values as strings: a string (split on spaces when `split` is set), an array of
/// strings, a number or a boolean as text.
pub fn claim_strings(claims: &Map<String, Value>, path: &str, split: bool) -> Vec<String> {
    let one = |v: &Value| match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    };
    match claim(claims, path) {
        Some(Value::Array(a)) => a.iter().filter_map(one).take(1024).collect(),
        Some(Value::String(s)) if split => s.split_whitespace().map(str::to_string).take(1024).collect(),
        Some(v) => one(v).into_iter().collect(),
        None => Vec::new(),
    }
}
