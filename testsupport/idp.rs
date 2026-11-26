//! Test identity provider: locally generated signing keys and a loopback HTTP server serving
//! OpenID Connect discovery and a JWKS that tests can rotate. Shared by `bicraft-auth` and
//! `bicraft-mcp` tests (`#[path = "../../../testsupport/idp.rs"]`). Needs `ring`, `serde_json`
//! and `bicraft-auth` (for base64url).

#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bicraft_auth::b64;
use ring::rand::SystemRandom;
use ring::signature::{self, EcdsaKeyPair, Ed25519KeyPair, KeyPair, RsaKeyPair};
use serde_json::{Value, json};

/// A 2048-bit RSA key generated for these tests only (`openssl genpkey`, PKCS#8 DER, base64).
/// Never used anywhere else; it signs nothing but test tokens.
const TEST_ONLY_RSA_PKCS8: &str = concat!(
    "MIIEvwIBADANBgkqhkiG9w0BAQEFAASCBKkwggSlAgEAAoIBAQCuQ7By0gjjv7s7B2ppQePBkxrXQdkmtjjuhH9adMCqtabc",
    "gNuEq8whExsqrZ58wNc13QTkBoFw8n/EDmR5eCc4DU/bpkZJxjyHRWsu6+szqx9ViiumaUQtD/aSrbXH/yTmT+vLvTBL5gJF",
    "1pYqrt974EJCtvFCI1DWP3GVgYXb8O3Tvo7RD+zEN/I8t9n6UOBCHy1MjAdQ41Rz1x89AZzZ9kxaYe4mhAl83Ie07reyAeCB",
    "xiFzUFIm9kXISooheptCWHiVl7v6yQn0ngv+6rGFX0N43qFitdzeHVfH1nLmEJ5BP732cPD42KYOjsIYrFgSkGENriuoVSmR",
    "Odvk8M3jAgMBAAECggEADq0tvaXZ0NCThLetqxq9A08P2iQ2FTAQoGnxTd16YoY0/OovUr+jS//PcGs551jYZzVrlYbEvQ33",
    "W7w4kxBfATYBzUZ2Mf1rmJrsmYGOCAZ395zQ601/S+pFwnm1zxe6dFkzyy9uU14WP+qAMC0VLlonSnYc4IyX4MtsFgLYErtW",
    "Yiiq5aTc/QaTYry3asAP6IXK8mbw7J7nTCw1qC8RW7bl5kEJ3WUeRdLtMLS1Yz+StOt1QnyE+9sG39plWIB5jUX1SmZz0iA5",
    "4Qb7m0mOwR+3X/8d88w8FtnjUPF0qLnGuGBbLyTluAl3DRVQnFye6LSgGx1vLrky494imLbfjQKBgQDz407GL14M6jxsQ0dw",
    "YuZ9qvU4VLjL4duxp4HxACX26pnvytjAj8Jbpg0O2ChQkvarse/YNG3V4YZm+Iahhs0UF17mzrLfDWyG2XLlR1PpVUshe60m",
    "yC7JauqxrykXwIft1vDJt24jhbeBeOfBGox2jA6uCvk81zrPZ7ObVo/TbQKBgQC26zdnrkxi+cwCleyFWh/ALvWp6h/uZFjU",
    "Zi2gIZlGZnSRMjJquN7ePYj+2duSR+JS0hoiGkbytgDS0+FxsCMrwzhX2oZrxu66YWtES7L6zeQysxOohlfdhzBqLhHEJRK4",
    "tdwK1Wu5BZ2IXv+hBayE0V0u1jPLWejEACbBXPwEjwKBgQDHAhNUanBfL39whBG2Ji2YG7kPZyqHy1u+gkXoxsszyUklPytR",
    "Lb1VlZCQLVfwL4uynHZuwJFjdix6t478HQgYvd1FjyKsGcMV4jf72REik+PDhazLuYtb9a9en+RzKjqAm+RNZS0tNT6BGoxZ",
    "qPU3AqAz6mZn++RUO3V2atLfnQKBgQCvli8DKb9Nqzf4q231vEOKOgm753BraIwUp1jYxCfb+nGfBy56X6ggcbVV3k71pmdM",
    "1T4cylicl7VGq/lXJps8YRNu0egDAV0cMHPFhVzzgDyzpk8SkJM7pZAF/N81B9lm0bu98oyLeSZ1R6vfKBEpFch7M0dYxp+3",
    "teV97AJHZQKBgQDQdW4IL+A/cHGNOTEDkPGX4/jWuM8w/10+klOvY5XXKjbfGoKxvEGXDwskZFyrkx9mUTgLD3PY59CUsJFR",
    "7iXbmVIKHpVcC/q7Kg4T0sgq/K4U8CBYXxKcJc+/qDo2sG7E4Mn9FY674zI7van34B+5tGqR3cLFV/oW9ppuF7NWtg==",
);

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

/// A signing key with its JWK.
pub enum Key {
    Ec(EcdsaKeyPair, String),
    Rsa(RsaKeyPair, String),
    Ed(Ed25519KeyPair, String),
}

impl Key {
    pub fn ec(kid: &str) -> Key {
        let rng = SystemRandom::new();
        let alg = &signature::ECDSA_P256_SHA256_FIXED_SIGNING;
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(alg, &rng).unwrap();
        Key::Ec(
            EcdsaKeyPair::from_pkcs8(alg, pkcs8.as_ref(), &rng).unwrap(),
            kid.into(),
        )
    }

    pub fn rsa(kid: &str) -> Key {
        let std_b64 = TEST_ONLY_RSA_PKCS8.replace('+', "-").replace('/', "_");
        let der = b64::decode(&std_b64).unwrap();
        Key::Rsa(RsaKeyPair::from_pkcs8(&der).unwrap(), kid.into())
    }

    pub fn ed(kid: &str) -> Key {
        let rng = SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        Key::Ed(Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap(), kid.into())
    }

    pub fn kid(&self) -> &str {
        match self {
            Key::Ec(_, k) | Key::Rsa(_, k) | Key::Ed(_, k) => k,
        }
    }

    pub fn default_alg(&self) -> &'static str {
        match self {
            Key::Ec(..) => "ES256",
            Key::Rsa(..) => "RS256",
            Key::Ed(..) => "EdDSA",
        }
    }

    pub fn jwk(&self) -> Value {
        match self {
            Key::Ec(p, kid) => {
                let pt = p.public_key().as_ref();
                json!({"kty": "EC", "crv": "P-256", "kid": kid, "use": "sig",
                    "x": b64::encode(&pt[1..33]), "y": b64::encode(&pt[33..65])})
            }
            Key::Rsa(p, kid) => {
                let pk = signature::RsaPublicKeyComponents::<Vec<u8>>::from(p.public());
                json!({"kty": "RSA", "kid": kid, "use": "sig",
                    "n": b64::encode(&pk.n), "e": b64::encode(&pk.e)})
            }
            Key::Ed(p, kid) => json!({"kty": "OKP", "crv": "Ed25519", "kid": kid,
                "x": b64::encode(p.public_key().as_ref())}),
        }
    }

    /// Signs `msg` with `alg` (RS256 or PS256 for the RSA key).
    pub fn sign(&self, alg: &str, msg: &[u8]) -> Vec<u8> {
        let rng = SystemRandom::new();
        match self {
            Key::Ec(p, _) => p.sign(&rng, msg).unwrap().as_ref().to_vec(),
            Key::Ed(p, _) => p.sign(msg).as_ref().to_vec(),
            Key::Rsa(p, _) => {
                let enc: &dyn signature::RsaEncoding = match alg {
                    "PS256" => &signature::RSA_PSS_SHA256,
                    _ => &signature::RSA_PKCS1_SHA256,
                };
                let mut sig = vec![0; p.public().modulus_len()];
                p.sign(enc, &rng, msg, &mut sig).unwrap();
                sig
            }
        }
    }

    /// A signed token with this key's default algorithm and kid.
    pub fn token(&self, claims: &Value) -> String {
        let header = json!({"alg": self.default_alg(), "typ": "at+jwt", "kid": self.kid()});
        self.token_with(&header, claims)
    }

    pub fn token_with(&self, header: &Value, claims: &Value) -> String {
        let alg = header["alg"].as_str().unwrap_or("").to_string();
        let input = format!(
            "{}.{}",
            b64::encode(header.to_string().as_bytes()),
            b64::encode(claims.to_string().as_bytes())
        );
        let sig = self.sign(&alg, input.as_bytes());
        format!("{input}.{}", b64::encode(&sig))
    }
}

/// An HS256 token (for the explicit-HMAC case and the key-confusion attack).
pub fn hmac_token(secret: &[u8], claims: &Value) -> String {
    let header = json!({"alg": "HS256", "typ": "JWT"});
    let input = format!(
        "{}.{}",
        b64::encode(header.to_string().as_bytes()),
        b64::encode(claims.to_string().as_bytes())
    );
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret);
    let tag = ring::hmac::sign(&key, input.as_bytes());
    format!("{input}.{}", b64::encode(tag.as_ref()))
}

/// The loopback identity provider.
pub struct Idp {
    pub addr: SocketAddr,
    pub issuer: String,
    pub jwks: Arc<Mutex<Value>>,
    pub jwks_fetches: Arc<AtomicUsize>,
    pub discovery_fetches: Arc<AtomicUsize>,
    /// Discovery answers with another `issuer` (a mix-up the client must refuse).
    pub lie: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

impl Drop for Idp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

impl Idp {
    pub fn start(keys: &[&Key]) -> Idp {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let issuer = format!("http://{addr}/realms/test");
        let idp = Idp {
            addr,
            issuer: issuer.clone(),
            jwks: Arc::new(Mutex::new(
                json!({"keys": keys.iter().map(|k| k.jwk()).collect::<Vec<_>>()}),
            )),
            jwks_fetches: Arc::default(),
            discovery_fetches: Arc::default(),
            lie: Arc::default(),
            stop: Arc::default(),
        };
        let (jwks, jf, df, lie, stop) = (
            Arc::clone(&idp.jwks),
            Arc::clone(&idp.jwks_fetches),
            Arc::clone(&idp.discovery_fetches),
            Arc::clone(&idp.lie),
            Arc::clone(&idp.stop),
        );
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let Ok((mut s, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                let _ = s.set_nonblocking(false);
                let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
                let mut buf = Vec::new();
                let mut chunk = [0u8; 1024];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 16384 {
                    match s.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                let text = String::from_utf8_lossy(&buf);
                let path = text.split_whitespace().nth(1).unwrap_or("").to_string();
                let (status, body) = if path == "/realms/test/.well-known/openid-configuration" {
                    df.fetch_add(1, Ordering::SeqCst);
                    let iss = if lie.load(Ordering::SeqCst) {
                        "http://127.0.0.1:1/other".to_string()
                    } else {
                        issuer.clone()
                    };
                    (
                        "200 OK",
                        json!({"issuer": iss, "jwks_uri": format!("{issuer}/certs")}).to_string(),
                    )
                } else if path == "/realms/test/certs" {
                    jf.fetch_add(1, Ordering::SeqCst);
                    ("200 OK", jwks.lock().unwrap().to_string())
                } else {
                    ("404 Not Found", "{}".to_string())
                };
                let _ = write!(
                    s,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        idp
    }

    /// Replaces the published keys.
    pub fn publish(&self, keys: &[&Key]) {
        *self.jwks.lock().unwrap() = json!({"keys": keys.iter().map(|k| k.jwk()).collect::<Vec<_>>()});
    }

    /// Standard valid claims for `aud`.
    pub fn claims(&self, aud: &str) -> Value {
        let now = now();
        json!({"iss": self.issuer, "sub": "user-1", "aud": aud, "exp": now + 600, "iat": now,
            "nbf": now - 5, "email": "ana@example.com", "groups": ["bi-team"],
            "realm_access": {"roles": ["bicraft-editor"]}, "scope": "openid bicraft"})
    }
}
