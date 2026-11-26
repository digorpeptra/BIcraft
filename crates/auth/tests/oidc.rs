//! The bearer JWT validation matrix against a loopback identity provider with locally generated
//! keys: algorithms, signature, times, issuer, audience, scopes and claims, discovery, JWKS
//! caching, unknown kid refetch (rate limited) and key rotation, plus a never-crash fuzz over
//! `Authorization` headers.

#[path = "../../../testsupport/idp.rs"]
mod idp;

use std::net::IpAddr;
use std::sync::atomic::Ordering;

use bicraft_auth::{AuthConfig, Authenticator, Denial, Grant, IssuerConfig, Secret, Source};
use idp::{Idp, Key, hmac_token, now};
use serde_json::{Value, json};

const AUD: &str = "api://bicraft";
const PEER: &str = "203.0.113.9";

fn issuer(idp: &Idp) -> IssuerConfig {
    let mut c = IssuerConfig::new(idp.issuer.clone());
    c.audiences = vec![AUD.into()];
    c.roles_claim = Some("realm_access.roles".into());
    c.jwks_min_refetch_secs = 0;
    c
}

fn auth_with(issuers: Vec<IssuerConfig>, public_url: Option<&str>) -> Authenticator {
    let mut config = AuthConfig {
        issuers,
        public_url: public_url.map(str::to_string),
        ..AuthConfig::default()
    };
    config.acl.grants.push(Grant::parse_text("*=*").unwrap());
    Authenticator::new(config).unwrap()
}

fn auth(idp: &Idp) -> Authenticator {
    auth_with(vec![issuer(idp)], Some("https://bi.example.com"))
}

fn check(a: &Authenticator, token: &str) -> Result<bicraft_auth::Identity, Denial> {
    check_for(a, token, &[])
}

fn check_for(a: &Authenticator, token: &str, resources: &[String]) -> Result<bicraft_auth::Identity, Denial> {
    let value = format!("Bearer {token}");
    let peer: IpAddr = PEER.parse().unwrap();
    a.authenticate(
        peer,
        |n| n.eq_ignore_ascii_case("authorization").then_some(value.as_str()),
        resources,
    )
}

fn refused(a: &Authenticator, token: &str, why: &str) {
    match check(a, token) {
        Err(Denial::InvalidToken(d)) => assert_eq!(d, why),
        other => panic!("expected invalid_token `{why}`, got {other:?}"),
    }
}

fn with(mut claims: Value, k: &str, v: Value) -> Value {
    claims[k] = v;
    claims
}

fn without(mut claims: Value, k: &str) -> Value {
    claims.as_object_mut().unwrap().remove(k);
    claims
}

#[test]
fn valid_tokens_of_every_key_type() {
    let (ec, rsa, ed) = (Key::ec("ec1"), Key::rsa("rsa1"), Key::ed("ed1"));
    let idp = Idp::start(&[&ec, &rsa, &ed]);
    let a = auth(&idp);
    let claims = idp.claims(AUD);
    for key in [&ec, &rsa, &ed] {
        let id = check(&a, &key.token(&claims)).unwrap_or_else(|e| panic!("{}: {e:?}", key.kid()));
        assert_eq!(id.subject, "user-1");
        assert_eq!(id.email.as_deref(), Some("ana@example.com"));
        assert_eq!(id.groups, ["bi-team"]);
        assert_eq!(id.roles, ["bicraft-editor"]);
        assert_eq!(id.scopes, ["openid", "bicraft"]);
        assert_eq!(
            id.source,
            Source::Oidc {
                issuer: idp.issuer.clone()
            }
        );
    }
    // PS256 with the RSA key
    let ps = rsa.token_with(&json!({"alg": "PS256", "kid": "rsa1"}), &claims);
    check(&a, &ps).unwrap();
    // discovery once, keys once: cached afterwards
    assert_eq!(idp.discovery_fetches.load(Ordering::SeqCst), 1);
    assert_eq!(idp.jwks_fetches.load(Ordering::SeqCst), 1);
    // no kid: every key of the right type is tried
    let no_kid = ec.token_with(&json!({"alg": "ES256"}), &claims);
    check(&a, &no_kid).unwrap();
}

#[test]
fn times_are_checked_with_leeway() {
    let ec = Key::ec("k");
    let idp = Idp::start(&[&ec]);
    let a = auth(&idp);
    let c = idp.claims(AUD);
    let t = now();
    refused(
        &a,
        &ec.token(&with(c.clone(), "exp", json!(t - 120))),
        "token expired",
    );
    check(&a, &ec.token(&with(c.clone(), "exp", json!(t - 30)))).unwrap(); // within 60 s leeway
    refused(&a, &ec.token(&without(c.clone(), "exp")), "token has no expiry");
    refused(
        &a,
        &ec.token(&with(c.clone(), "nbf", json!(t + 600))),
        "token not yet valid",
    );
    check(&a, &ec.token(&with(c.clone(), "nbf", json!(t + 30)))).unwrap();
    refused(
        &a,
        &ec.token(&with(c.clone(), "iat", json!(t + 600))),
        "token issued in the future",
    );
    refused(
        &a,
        &ec.token(&with(c.clone(), "exp", json!("soon"))),
        "malformed token",
    );
    check(&a, &ec.token(&with(c, "exp", json!((t + 600) as f64 + 0.5)))).unwrap();
}

#[test]
fn issuer_and_audience() {
    let ec = Key::ec("k");
    let idp = Idp::start(&[&ec]);
    let a = auth(&idp);
    let c = idp.claims(AUD);
    refused(
        &a,
        &ec.token(&with(c.clone(), "iss", json!("https://evil.example"))),
        "issuer not accepted",
    );
    refused(&a, &ec.token(&without(c.clone(), "iss")), "issuer not accepted");
    refused(
        &a,
        &ec.token(&with(c.clone(), "aud", json!("api://other"))),
        "token not issued for this resource",
    );
    refused(
        &a,
        &ec.token(&without(c.clone(), "aud")),
        "token not issued for this resource",
    );
    check(
        &a,
        &ec.token(&with(c.clone(), "aud", json!(["api://other", AUD]))),
    )
    .unwrap();
    // the public URL and the workspace's resource URL are audiences too (RFC 8707)
    check(
        &a,
        &ec.token(&with(c.clone(), "aud", json!("https://bi.example.com"))),
    )
    .unwrap();
    check(
        &a,
        &ec.token(&with(c.clone(), "aud", json!("HTTPS://BI.EXAMPLE.COM/"))),
    )
    .unwrap();
    let ws = "https://bi.example.com/w/sales/mcp".to_string();
    let t = ec.token(&with(c.clone(), "aud", json!(ws)));
    check_for(&a, &t, std::slice::from_ref(&ws)).unwrap();
    // ... but only for that workspace
    assert!(check_for(&a, &t, &["https://bi.example.com/w/hr/mcp".to_string()]).is_err());
}

#[test]
fn signatures_and_algorithms() {
    let (ec, other) = (Key::ec("k"), Key::ec("k"));
    let rsa = Key::rsa("r");
    let idp = Idp::start(&[&ec, &rsa]);
    let a = auth(&idp);
    let c = idp.claims(AUD);
    // signed by another key under the same kid
    refused(&a, &other.token(&c), "invalid signature");
    // payload changed after signing
    let good = ec.token(&c);
    let mut parts: Vec<String> = good.split('.').map(str::to_string).collect();
    parts[1] = bicraft_auth::b64::encode(with(c.clone(), "sub", json!("admin")).to_string().as_bytes());
    refused(&a, &parts.join("."), "invalid signature");
    // a signature of the right length but wrong bytes
    let sig_len = bicraft_auth::b64::decode(good.rsplit('.').next().unwrap())
        .unwrap()
        .len();
    let forged = format!(
        "{}.{}",
        good.rsplit_once('.').unwrap().0,
        bicraft_auth::b64::encode(&vec![7u8; sig_len])
    );
    refused(&a, &forged, "invalid signature");
    // alg none, never
    let h = bicraft_auth::b64::encode(br#"{"alg":"none"}"#);
    let p = bicraft_auth::b64::encode(c.to_string().as_bytes());
    refused(&a, &format!("{h}.{p}."), "signature algorithm not accepted");
    refused(&a, &format!("{h}.{p}.AAAA"), "signature algorithm not accepted");
    let h = bicraft_auth::b64::encode(br#"{"alg":"NoNe"}"#);
    refused(&a, &format!("{h}.{p}.AAAA"), "signature algorithm not accepted");
    // HS256 signed with the RSA public key as the secret (key confusion): refused by default
    let n = &rsa.jwk()["n"];
    refused(
        &a,
        &hmac_token(n.as_str().unwrap().as_bytes(), &c),
        "signature algorithm not accepted",
    );
    // an algorithm the key does not fit
    refused(
        &a,
        &ec.token_with(&json!({"alg": "RS256", "kid": "k"}), &c),
        "unknown signing key",
    );
    // the issuer can narrow the algorithms
    let mut only_rs = issuer(&idp);
    only_rs.algorithms = vec![bicraft_auth::Alg::RS256];
    let narrow = auth_with(vec![only_rs], None);
    refused(&narrow, &ec.token(&c), "signature algorithm not accepted");
    check(&narrow, &rsa.token(&c)).unwrap();
    // critical headers and JWE are refused
    refused(
        &a,
        &ec.token_with(&json!({"alg": "ES256", "kid": "k", "crit": ["exp"]}), &c),
        "unsupported critical header",
    );
}

#[test]
fn hmac_only_when_explicitly_configured() {
    let ec = Key::ec("k");
    let idp = Idp::start(&[&ec]);
    let secret = b"0123456789abcdef0123456789abcdef-shared".to_vec();
    let mut c = issuer(&idp);
    c.algorithms = vec![bicraft_auth::Alg::HS256];
    c.hmac_secret = Some(Secret(secret.clone()));
    let a = auth_with(vec![c], None);
    let claims = idp.claims(AUD);
    check(&a, &hmac_token(&secret, &claims)).unwrap();
    refused(
        &a,
        &hmac_token(b"another-secret-of-thirty-two-bytes!!", &claims),
        "invalid signature",
    );
    // listing HS256 without a secret, or a short secret, is a configuration error
    let mut bad = issuer(&idp);
    bad.algorithms = vec![bicraft_auth::Alg::HS256];
    let mut config = AuthConfig {
        issuers: vec![bad.clone()],
        ..AuthConfig::default()
    };
    config.acl.grants.push(Grant::parse_text("*=*").unwrap());
    assert!(Authenticator::new(config.clone()).unwrap_err().contains("HS256"));
    bad.hmac_secret = Some(Secret(b"short".to_vec()));
    config.issuers = vec![bad];
    assert!(Authenticator::new(config).unwrap_err().contains("32 bytes"));
}

#[test]
fn unknown_kid_refetches_once_and_rotation_drops_old_keys() {
    let (a_key, b_key) = (Key::ec("a"), Key::ec("b"));
    let idp = Idp::start(&[&a_key]);
    let auth = auth(&idp);
    let c = idp.claims(AUD);
    check(&auth, &a_key.token(&c)).unwrap();
    assert_eq!(idp.jwks_fetches.load(Ordering::SeqCst), 1);
    // the issuer rotates: B is added and A removed
    idp.publish(&[&b_key]);
    // A is still cached (no refetch for a known kid)
    check(&auth, &a_key.token(&c)).unwrap();
    assert_eq!(idp.jwks_fetches.load(Ordering::SeqCst), 1);
    // B is unknown: one refetch finds it
    check(&auth, &b_key.token(&c)).unwrap();
    assert_eq!(idp.jwks_fetches.load(Ordering::SeqCst), 2);
    // A was dropped by that refetch
    refused(&auth, &a_key.token(&c), "unknown signing key");
    assert_eq!(
        idp.jwks_fetches.load(Ordering::SeqCst),
        3,
        "one refetch per unknown kid"
    );
    // a random kid: one refetch, then refused
    let stranger = Key::ec("zzz");
    refused(&auth, &stranger.token(&c), "unknown signing key");
    assert_eq!(idp.jwks_fetches.load(Ordering::SeqCst), 4);
}

#[test]
fn forced_refetches_are_rate_limited() {
    let key = Key::ec("a");
    let idp = Idp::start(&[&key]);
    let mut c = issuer(&idp);
    c.jwks_min_refetch_secs = 30;
    let auth = auth_with(vec![c], None);
    let claims = idp.claims(AUD);
    check(&auth, &key.token(&claims)).unwrap();
    for i in 0..20 {
        let k = Key::ec(&format!("random-{i}"));
        refused(&auth, &k.token(&claims), "unknown signing key");
    }
    // the first unknown kid refetched; the 19 others waited for the interval
    assert_eq!(idp.jwks_fetches.load(Ordering::SeqCst), 2);
}

#[test]
fn scopes_and_required_claims() {
    let ec = Key::ec("k");
    let idp = Idp::start(&[&ec]);
    let mut c = issuer(&idp);
    c.required_scopes = vec!["bicraft".into()];
    c.required_claims = vec![("tid".into(), "tenant-1".into())];
    let a = auth_with(vec![c], None);
    let claims = with(idp.claims(AUD), "tid", json!("tenant-1"));
    check(&a, &ec.token(&claims)).unwrap();
    // Entra ID style `scp`
    let scp = with(without(claims.clone(), "scope"), "scp", json!("bicraft other"));
    check(&a, &ec.token(&scp)).unwrap();
    match check(&a, &ec.token(&with(claims.clone(), "scope", json!("openid")))) {
        Err(Denial::InsufficientScope(s)) => assert_eq!(s, ["bicraft"]),
        other => panic!("{other:?}"),
    }
    refused(
        &a,
        &ec.token(&with(claims.clone(), "tid", json!("tenant-2"))),
        "required claim missing or different",
    );
    refused(
        &a,
        &ec.token(&without(claims, "tid")),
        "required claim missing or different",
    );
}

#[test]
fn discovery_must_name_the_same_issuer() {
    let ec = Key::ec("k");
    let idp = Idp::start(&[&ec]);
    idp.lie.store(true, Ordering::SeqCst);
    let a = auth(&idp);
    refused(&a, &ec.token(&idp.claims(AUD)), "issuer keys unavailable");
    assert_eq!(idp.jwks_fetches.load(Ordering::SeqCst), 0);
    assert_eq!(a.prefetch(), std::slice::from_ref(&idp.issuer));
    // a configured jwks_uri skips discovery
    let mut c = issuer(&idp);
    c.jwks_uri = Some(format!("{}/certs", idp.issuer));
    let direct = auth_with(vec![c], None);
    check(&direct, &ec.token(&idp.claims(AUD))).unwrap();
}

#[test]
fn several_issuers() {
    let (k1, k2) = (Key::ec("one"), Key::rsa("two"));
    let (i1, i2) = (Idp::start(&[&k1]), Idp::start(&[&k2]));
    let a = auth_with(vec![issuer(&i1), issuer(&i2)], None);
    check(&a, &k1.token(&i1.claims(AUD))).unwrap();
    check(&a, &k2.token(&i2.claims(AUD))).unwrap();
    // a token naming issuer 2 but signed by issuer 1's key
    let forged = k1.token(&i2.claims(AUD));
    assert!(check(&a, &forged).is_err());
}

/// xorshift64*: a deterministic generator for the fuzz loop.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

#[test]
fn hostile_authorization_headers_never_crash() {
    let ec = Key::ec("k");
    let idp = Idp::start(&[&ec]);
    let mut c = issuer(&idp);
    c.jwks_min_refetch_secs = 3600; // the fuzz must not hammer the IdP
    let a = auth_with(vec![c], Some("https://bi.example.com"));
    let good = format!("Bearer {}", ec.token(&idp.claims(AUD)));
    let pieces: &[&[u8]] = &[
        b".",
        b"..",
        b"=",
        b"Bearer ",
        b"bearer\t",
        b"Basic ",
        b"\0",
        b"\xff",
        b"\xc3\xa9",
        b"%",
        b"eyJ",
        b"e30",
        b"W10",
        b"bnVsbA",
        b"{",
        b"\"",
        b" ",
        b"bct_",
    ];
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let peer: IpAddr = PEER.parse().unwrap();
    for _ in 0..20_000 {
        let mut v = good.as_bytes().to_vec();
        for _ in 0..1 + rng.below(4) {
            match rng.below(5) {
                0 if !v.is_empty() => {
                    let i = rng.below(v.len());
                    v[i] = rng.next() as u8;
                }
                1 if !v.is_empty() => {
                    let i = rng.below(v.len());
                    v.truncate(i);
                }
                2 => {
                    let p = pieces[rng.below(pieces.len())];
                    let i = rng.below(v.len() + 1);
                    v.splice(i..i, p.iter().copied());
                }
                3 if !v.is_empty() => {
                    let i = rng.below(v.len());
                    let j = (i + rng.below(64)).min(v.len());
                    v.drain(i..j);
                }
                _ => {
                    let n = rng.below(3000);
                    v.extend((0..n).map(|_| rng.next() as u8));
                }
            }
        }
        let s = String::from_utf8_lossy(&v).into_owned();
        let _ = a.authenticate(
            peer,
            |n| n.eq_ignore_ascii_case("authorization").then_some(s.as_str()),
            &[],
        );
    }
    // hand-made shapes: nested JSON, huge numbers, wrong types
    for (h, p) in [
        ("{\"alg\":\"ES256\",\"kid\":7}", "{}"),
        ("[]", "{}"),
        ("{\"alg\":\"ES256\"}", "[]"),
        ("{\"alg\":\"ES256\"}", "{\"iss\":7}"),
        (
            "{\"alg\":\"ES256\"}",
            &format!("{{\"iss\":\"{}\",\"exp\":1e400}}", idp.issuer),
        ),
        (
            "{\"alg\":\"ES256\"}",
            &format!("{{\"iss\":\"{}\",\"aud\":[[[[]]]]}}", idp.issuer),
        ),
        ("{\"alg\":\"ES256\"}", &"[".repeat(5000)),
    ] {
        let t = format!(
            "{}.{}.AAAA",
            bicraft_auth::b64::encode(h.as_bytes()),
            bicraft_auth::b64::encode(p.as_bytes())
        );
        assert!(check(&a, &t).is_err());
    }
    assert!(idp.jwks_fetches.load(Ordering::SeqCst) <= 2);
}
