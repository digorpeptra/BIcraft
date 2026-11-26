//! JSON Web Keys (RFC 7517/7518/8037) and JWS signature checks (RFC 7515) with `ring`.

use ring::signature::{self, RsaPublicKeyComponents, UnparsedPublicKey};
use serde_json::Value;

use crate::b64;

/// Signature algorithms BIcraft can check. `none` is not one of them, ever.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Alg {
    RS256,
    RS384,
    RS512,
    PS256,
    PS384,
    PS512,
    ES256,
    ES384,
    EdDSA,
    HS256,
    HS384,
    HS512,
}

impl Alg {
    /// The asymmetric algorithms, accepted by default.
    pub const ASYMMETRIC: &'static [Alg] = &[
        Alg::RS256,
        Alg::RS384,
        Alg::RS512,
        Alg::PS256,
        Alg::PS384,
        Alg::PS512,
        Alg::ES256,
        Alg::ES384,
        Alg::EdDSA,
    ];

    pub fn parse(s: &str) -> Option<Alg> {
        Some(match s {
            "RS256" => Alg::RS256,
            "RS384" => Alg::RS384,
            "RS512" => Alg::RS512,
            "PS256" => Alg::PS256,
            "PS384" => Alg::PS384,
            "PS512" => Alg::PS512,
            "ES256" => Alg::ES256,
            "ES384" => Alg::ES384,
            "EdDSA" => Alg::EdDSA,
            "HS256" => Alg::HS256,
            "HS384" => Alg::HS384,
            "HS512" => Alg::HS512,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Alg::RS256 => "RS256",
            Alg::RS384 => "RS384",
            Alg::RS512 => "RS512",
            Alg::PS256 => "PS256",
            Alg::PS384 => "PS384",
            Alg::PS512 => "PS512",
            Alg::ES256 => "ES256",
            Alg::ES384 => "ES384",
            Alg::EdDSA => "EdDSA",
            Alg::HS256 => "HS256",
            Alg::HS384 => "HS384",
            Alg::HS512 => "HS512",
        }
    }

    pub fn is_hmac(self) -> bool {
        matches!(self, Alg::HS256 | Alg::HS384 | Alg::HS512)
    }
}

/// The public material of one key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyMaterial {
    /// RSA modulus and exponent, big-endian.
    Rsa { n: Vec<u8>, e: Vec<u8> },
    /// An uncompressed EC point `04 || x || y` on P-256 or P-384.
    Ec { p384: bool, point: Vec<u8> },
    /// An Ed25519 public key.
    Ed25519(Vec<u8>),
}

/// One verification key from a JWKS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jwk {
    pub kid: Option<String>,
    /// The key's own `alg`, when the issuer pins one.
    pub alg: Option<String>,
    pub material: KeyMaterial,
}

/// Largest RSA modulus accepted (bytes): 8192 bits, as `ring` checks at most.
const MAX_RSA_BYTES: usize = 1024;

impl Jwk {
    /// Parses one JWK; `None` for a key BIcraft cannot use (private parts are ignored, a key whose
    /// `use` is not `sig` is skipped, unknown types and curves are skipped).
    pub fn parse(v: &Value) -> Option<Jwk> {
        let field = |k: &str| v.get(k).and_then(Value::as_str);
        if field("use").is_some_and(|u| u != "sig") {
            return None;
        }
        if let Some(ops) = v.get("key_ops").and_then(Value::as_array)
            && !ops.iter().any(|o| o.as_str() == Some("verify"))
        {
            return None;
        }
        let bytes = |k: &str| field(k).and_then(b64::decode);
        let material = match field("kty")? {
            "RSA" => {
                let n = strip_zeros(bytes("n")?);
                let e = strip_zeros(bytes("e")?);
                if n.is_empty() || n.len() > MAX_RSA_BYTES || e.is_empty() || e.len() > 8 {
                    return None;
                }
                KeyMaterial::Rsa { n, e }
            }
            "EC" => {
                let (p384, len) = match field("crv")? {
                    "P-256" => (false, 32),
                    "P-384" => (true, 48),
                    _ => return None,
                };
                let x = bytes("x")?;
                let y = bytes("y")?;
                if x.len() != len || y.len() != len {
                    return None;
                }
                let mut point = Vec::with_capacity(1 + 2 * len);
                point.push(4);
                point.extend_from_slice(&x);
                point.extend_from_slice(&y);
                KeyMaterial::Ec { p384, point }
            }
            "OKP" => {
                if field("crv")? != "Ed25519" {
                    return None;
                }
                let x = bytes("x")?;
                if x.len() != 32 {
                    return None;
                }
                KeyMaterial::Ed25519(x)
            }
            _ => return None,
        };
        Some(Jwk {
            kid: field("kid").map(str::to_string),
            alg: field("alg").map(str::to_string),
            material,
        })
    }

    /// Whether this key can check `alg` (its type fits, and its own `alg`, when set, is `alg`).
    pub fn fits(&self, alg: Alg) -> bool {
        if self.alg.as_deref().is_some_and(|a| a != alg.name()) {
            return false;
        }
        matches!(
            (&self.material, alg),
            (
                KeyMaterial::Rsa { .. },
                Alg::RS256 | Alg::RS384 | Alg::RS512 | Alg::PS256 | Alg::PS384 | Alg::PS512,
            ) | (KeyMaterial::Ec { p384: false, .. }, Alg::ES256)
                | (KeyMaterial::Ec { p384: true, .. }, Alg::ES384)
                | (KeyMaterial::Ed25519(_), Alg::EdDSA)
        )
    }

    /// Checks `sig` over `message` with this key and `alg`.
    pub fn verify(&self, alg: Alg, message: &[u8], sig: &[u8]) -> bool {
        if !self.fits(alg) {
            return false;
        }
        match &self.material {
            KeyMaterial::Rsa { n, e } => {
                let params: &signature::RsaParameters = match alg {
                    Alg::RS256 => &signature::RSA_PKCS1_2048_8192_SHA256,
                    Alg::RS384 => &signature::RSA_PKCS1_2048_8192_SHA384,
                    Alg::RS512 => &signature::RSA_PKCS1_2048_8192_SHA512,
                    Alg::PS256 => &signature::RSA_PSS_2048_8192_SHA256,
                    Alg::PS384 => &signature::RSA_PSS_2048_8192_SHA384,
                    Alg::PS512 => &signature::RSA_PSS_2048_8192_SHA512,
                    _ => return false,
                };
                RsaPublicKeyComponents { n, e }
                    .verify(params, message, sig)
                    .is_ok()
            }
            KeyMaterial::Ec { p384, point } => {
                let a: &dyn signature::VerificationAlgorithm = if *p384 {
                    &signature::ECDSA_P384_SHA384_FIXED
                } else {
                    &signature::ECDSA_P256_SHA256_FIXED
                };
                UnparsedPublicKey::new(a, point).verify(message, sig).is_ok()
            }
            KeyMaterial::Ed25519(x) => UnparsedPublicKey::new(&signature::ED25519, x)
                .verify(message, sig)
                .is_ok(),
        }
    }
}

/// Checks an HMAC signature (HS256/384/512) with a configured shared secret, in constant time.
pub fn verify_hmac(alg: Alg, secret: &[u8], message: &[u8], sig: &[u8]) -> bool {
    let a = match alg {
        Alg::HS256 => ring::hmac::HMAC_SHA256,
        Alg::HS384 => ring::hmac::HMAC_SHA384,
        Alg::HS512 => ring::hmac::HMAC_SHA512,
        _ => return false,
    };
    let key = ring::hmac::Key::new(a, secret);
    ring::hmac::verify(&key, message, sig).is_ok()
}

fn strip_zeros(mut v: Vec<u8>) -> Vec<u8> {
    let zeros = v.iter().take_while(|&&b| b == 0).count();
    v.drain(..zeros);
    v
}

/// Largest number of keys kept from one JWKS.
pub const MAX_KEYS: usize = 64;

/// The usable keys of a JWKS document (`{"keys": [...]}`); unusable entries are skipped.
pub fn parse_set(doc: &Value) -> Vec<Jwk> {
    doc.get("keys")
        .and_then(Value::as_array)
        .map(|keys| keys.iter().filter_map(Jwk::parse).take(MAX_KEYS).collect())
        .unwrap_or_default()
}
