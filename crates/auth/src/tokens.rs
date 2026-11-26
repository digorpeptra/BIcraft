//! Static API tokens for agents and CI.
//!
//! A token is `bct_` followed by 32 random bytes in base64url (256 bits from the OS generator).
//! Only its SHA-256 is stored. A salted slow hash (argon2) protects *passwords*, which have little
//! entropy; a 256-bit random token cannot be brute-forced whatever the hash, so SHA-256 loses
//! nothing and keeps every request cheap (a slow hash on each request would be a denial-of-service
//! lever). Tokens supplied from elsewhere must have at least [`MIN_TOKEN_LEN`] characters for the
//! same reason. The presented token is hashed and compared with every stored hash in constant
//! time, without stopping at the first match.

use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{Value, json};

use crate::b64;

/// Prefix of generated tokens (lets secret scanners and logs recognize them).
pub const PREFIX: &str = "bct_";
/// Shortest token accepted by [`hash_token`] for externally generated tokens.
pub const MIN_TOKEN_LEN: usize = 32;

/// One configured token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenEntry {
    /// The identity the token stands for (`token:<name>` in the ACL).
    pub name: String,
    pub sha256: [u8; 32],
    pub roles: Vec<String>,
    pub groups: Vec<String>,
}

/// A fresh random token.
pub fn generate() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| "the OS random generator failed".to_string())?;
    Ok(format!("{PREFIX}{}", b64::encode(&bytes)))
}

/// `bytes` random bytes from the OS generator (at most 64), as lowercase hex: one-time login
/// codes and session ids of the viewer's browser login (docs/auth.md, "The viewer in a browser").
pub fn random_hex(bytes: usize) -> Result<String, String> {
    let mut buf = [0u8; 64];
    let out = buf
        .get_mut(..bytes.min(64))
        .ok_or_else(|| "random_hex: bad length".to_string())?;
    SystemRandom::new()
        .fill(out)
        .map_err(|_| "the OS random generator failed".to_string())?;
    Ok(b64::hex(out))
}

/// SHA-256 of a token.
pub fn digest(token: &str) -> [u8; 32] {
    let d = ring::digest::digest(&ring::digest::SHA256, token.as_bytes());
    let mut out = [0u8; 32];
    out.copy_from_slice(d.as_ref());
    out
}

/// The hex SHA-256 to store for `token`; refuses tokens too short to be random enough.
pub fn hash_token(token: &str) -> Result<String, String> {
    let token = token.trim();
    if token.len() < MIN_TOKEN_LEN {
        return Err(format!(
            "a token needs at least {MIN_TOKEN_LEN} characters of random text (use `bicraft serve token create`)"
        ));
    }
    if !token.bytes().all(|b| b.is_ascii_graphic()) {
        return Err("a token is visible ASCII without spaces".into());
    }
    Ok(b64::hex(&digest(token)))
}

/// The tokens-file entry for a token.
pub fn entry_json(name: &str, sha256_hex: &str, roles: &[String], groups: &[String]) -> Value {
    json!({"name": name, "sha256": sha256_hex, "roles": roles, "groups": groups})
}

/// Equal-length comparison whose time does not depend on where the bytes differ.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    std::hint::black_box(diff) == 0
}

/// The entry whose hash matches `token`; every entry is compared.
pub fn find<'a>(entries: &'a [TokenEntry], token: &str) -> Option<&'a TokenEntry> {
    let d = digest(token);
    let mut found = None;
    for e in entries {
        if constant_time_eq(&e.sha256, &d) && found.is_none() {
            found = Some(e);
        }
    }
    found
}

/// Parses a tokens document: `{"tokens": [{"name", "sha256", "roles"?, "groups"?}]}` or the
/// bare array.
pub fn parse(doc: &Value) -> Result<Vec<TokenEntry>, String> {
    let list = match doc {
        Value::Array(a) => a,
        Value::Object(o) => o
            .get("tokens")
            .and_then(Value::as_array)
            .ok_or("a tokens file is {\"tokens\": [...]}")?,
        _ => return Err("a tokens file is {\"tokens\": [...]}".into()),
    };
    let mut out: Vec<TokenEntry> = Vec::with_capacity(list.len().min(10_000));
    for (i, t) in list.iter().enumerate().take(10_000) {
        let o = t.as_object().ok_or(format!("token {i}: not an object"))?;
        for k in o.keys() {
            if !matches!(k.as_str(), "name" | "sha256" | "roles" | "groups" | "comment") {
                return Err(format!("token {i}: unknown field `{k}`"));
            }
        }
        let name = o
            .get("name")
            .and_then(Value::as_str)
            .filter(|n| !n.is_empty() && n.len() <= 128)
            .ok_or(format!("token {i}: `name` is required"))?;
        let hex = o
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or(format!("token `{name}`: `sha256` is required (64 hex digits)"))?;
        let sha: [u8; 32] = b64::unhex(hex)
            .and_then(|v| v.try_into().ok())
            .ok_or(format!("token `{name}`: `sha256` must be 64 hex digits"))?;
        if out.iter().any(|e| e.name == name) {
            return Err(format!("token `{name}` is listed twice"));
        }
        if out.iter().any(|e| e.sha256 == sha) {
            return Err(format!("token `{name}` has the same hash as another token"));
        }
        out.push(TokenEntry {
            name: name.to_string(),
            sha256: sha,
            roles: strings(o.get("roles"), name, "roles")?,
            groups: strings(o.get("groups"), name, "groups")?,
        });
    }
    Ok(out)
}

fn strings(v: Option<&Value>, name: &str, field: &str) -> Result<Vec<String>, String> {
    match v {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(a)) => a
            .iter()
            .map(|x| x.as_str().map(str::to_string))
            .collect::<Option<Vec<_>>>()
            .ok_or(format!("token `{name}`: `{field}` is a list of strings")),
        Some(_) => Err(format!("token `{name}`: `{field}` is a list of strings")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_hash_find() {
        let t = generate().unwrap();
        assert!(t.starts_with(PREFIX));
        assert_eq!(t.len(), PREFIX.len() + 43);
        assert_ne!(t, generate().unwrap());
        let h = hash_token(&t).unwrap();
        assert_eq!(h.len(), 64);
        let doc = json!({"tokens": [entry_json("ci", &h, &["editor".into()], &[])]});
        let entries = parse(&doc).unwrap();
        assert_eq!(find(&entries, &t).unwrap().name, "ci");
        assert!(find(&entries, "bct_wrong").is_none());
        assert!(hash_token("short").is_err());
        assert!(hash_token(&format!("{} x", "a".repeat(40))).is_err());
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    #[test]
    fn bad_files() {
        let h = "0".repeat(64);
        for bad in [
            json!(3),
            json!({"tokens": 3}),
            json!([{"sha256": h}]),
            json!([{"name": "a"}]),
            json!([{"name": "a", "sha256": "xyz"}]),
            json!([{"name": "a", "sha256": h, "roles": "x"}]),
            json!([{"name": "a", "sha256": h, "secret": "x"}]),
            json!([{"name": "a", "sha256": h}, {"name": "a", "sha256": "1".repeat(64)}]),
            json!([{"name": "a", "sha256": h}, {"name": "b", "sha256": h}]),
        ] {
            assert!(parse(&bad).is_err(), "{bad}");
        }
    }
}
