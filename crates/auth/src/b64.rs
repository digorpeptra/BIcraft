//! Base64url without padding (RFC 4648 §5), as JWS and JWK use it, and hex.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Encodes `bytes` as unpadded base64url.
pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3).saturating_mul(4));
    for chunk in bytes.chunks(3) {
        let b = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let chars = chunk.len().saturating_add(1);
        for i in 0..chars {
            let shift = 18u32.saturating_sub(6u32.saturating_mul(i as u32));
            let idx = ((n >> shift) & 63) as usize;
            if let Some(&c) = ALPHABET.get(idx) {
                out.push(char::from(c));
            }
        }
    }
    out
}

/// Decodes unpadded base64url (padding `=` is tolerated at the end, as some issuers send it in
/// JWKs); `None` for any other character or an impossible length. Non-canonical trailing bits are
/// refused, so one byte string has one encoding (no signature malleability through the text).
pub fn decode(s: &str) -> Option<Vec<u8>> {
    let s = s.trim_end_matches('=');
    let bytes = s.as_bytes();
    if bytes.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3 + 2);
    for chunk in bytes.chunks(4) {
        let mut n = 0u32;
        for &c in chunk {
            n = (n << 6) | sextet(c)?;
        }
        match chunk.len() {
            4 => {
                out.extend_from_slice(&[(n >> 16) as u8, (n >> 8) as u8, n as u8]);
            }
            3 => {
                if n & 0b11 != 0 {
                    return None;
                }
                let n = n >> 2;
                out.extend_from_slice(&[(n >> 8) as u8, n as u8]);
            }
            2 => {
                if n & 0b1111 != 0 {
                    return None;
                }
                out.push((n >> 4) as u8);
            }
            _ => return None,
        }
    }
    Some(out)
}

fn sextet(c: u8) -> Option<u32> {
    Some(u32::from(match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'-' => 62,
        b'_' => 63,
        _ => return None,
    }))
}

/// Lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len().saturating_mul(2));
    for &b in bytes {
        for nibble in [b >> 4, b & 15] {
            if let Some(&d) = DIGITS.get(usize::from(nibble)) {
                s.push(char::from(d));
            }
        }
    }
    s
}

/// Decodes hex (either case); `None` when malformed.
pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if !b.len().is_multiple_of(2) {
        return None;
    }
    b.chunks(2)
        .map(|p| {
            let hi = char::from(*p.first()?).to_digit(16)?;
            let lo = char::from(*p.get(1)?).to_digit(16)?;
            u8::try_from(hi * 16 + lo).ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_rejects() {
        for len in 0..40usize {
            let data: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            let e = encode(&data);
            assert!(!e.contains('='));
            assert_eq!(decode(&e).as_deref(), Some(&data[..]), "{len}");
        }
        assert_eq!(decode("aGk").as_deref(), Some(&b"hi"[..]));
        assert_eq!(decode("aGk=").as_deref(), Some(&b"hi"[..]));
        assert!(decode("a").is_none());
        assert!(decode("aG+=").is_none());
        assert!(decode("aGl").is_none(), "non-canonical trailing bits");
        assert!(decode("a b").is_none());
        assert_eq!(hex(&[0, 255, 16]), "00ff10");
        assert_eq!(unhex("00FF10"), Some(vec![0, 255, 16]));
        assert!(unhex("0").is_none());
        assert!(unhex("zz").is_none());
    }
}
