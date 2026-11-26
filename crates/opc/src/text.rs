//! Text encodings. Several PBIX parts (`Report/Layout`, `DataModelSchema`, `Version`, …) are
//! UTF-16LE, usually without a BOM; PBIP/PBIR files are UTF-8.

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16LeBom,
}

impl Encoding {
    fn name(self) -> &'static str {
        match self {
            Encoding::Utf8 | Encoding::Utf8Bom => "UTF-8",
            Encoding::Utf16Le | Encoding::Utf16LeBom => "UTF-16LE",
        }
    }
}

/// Decodes a text part, detecting its encoding. Without a BOM, UTF-16LE is recognised by NUL
/// bytes in odd positions (ASCII-range text is the norm for these parts).
pub fn decode_text(part: &str, bytes: &[u8]) -> Result<(String, Encoding)> {
    let enc = detect(bytes);
    let body = match enc {
        Encoding::Utf8 | Encoding::Utf16Le => bytes,
        Encoding::Utf8Bom => bytes.get(3..).unwrap_or_default(),
        Encoding::Utf16LeBom => bytes.get(2..).unwrap_or_default(),
    };
    let text = match enc {
        Encoding::Utf8 | Encoding::Utf8Bom => String::from_utf8(body.to_vec()).ok(),
        Encoding::Utf16Le | Encoding::Utf16LeBom => {
            if body.len() % 2 != 0 {
                None
            } else {
                let units: Vec<u16> = body
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| u16::from_le_bytes(*c))
                    .collect();
                String::from_utf16(&units).ok()
            }
        }
    };
    text.map(|t| (t, enc))
        .ok_or_else(|| Error::BadText(part.to_string(), enc.name()))
}

/// Encodes text in the given encoding (the inverse of [`decode_text`]).
pub fn encode_text(text: &str, enc: Encoding) -> Vec<u8> {
    match enc {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Utf8Bom => [&[0xEF, 0xBB, 0xBF][..], text.as_bytes()].concat(),
        Encoding::Utf16Le | Encoding::Utf16LeBom => {
            let mut out = Vec::with_capacity(text.len().saturating_mul(2).saturating_add(2));
            if enc == Encoding::Utf16LeBom {
                out.extend_from_slice(&[0xFF, 0xFE]);
            }
            for u in text.encode_utf16() {
                out.extend_from_slice(&u.to_le_bytes());
            }
            out
        }
    }
}

/// Whether a file of a Copilot or verified-answers container (package parts `Copilot/…` and
/// `VerifiedAnswers/…`, project files `<Name>.SemanticModel/Copilot/…`) is text: the `.json` and
/// `.md` files, except the raw resource packages (`pbix-package.md` §2, "Copilot containers").
/// Text files are UTF-16LE without BOM in a package and UTF-8 without BOM in a project.
pub fn is_container_text(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    (lower.ends_with(".json") || lower.ends_with(".md"))
        && !lower.split('/').any(|seg| seg == "resourcepackages")
}

/// A container text file from its package form (UTF-16LE) to its project form (UTF-8, no BOM);
/// `None` when the bytes are not valid UTF-16LE text.
pub fn container_text_to_project(bytes: &[u8]) -> Option<Vec<u8>> {
    let body = bytes.strip_prefix(&[0xFF, 0xFE][..]).unwrap_or(bytes);
    if !body.len().is_multiple_of(2) {
        return None;
    }
    let units: Vec<u16> = body
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    String::from_utf16(&units).ok().map(String::into_bytes)
}

/// A container text file from its project form (UTF-8) to its package form (UTF-16LE, no BOM);
/// `None` when the bytes are not valid UTF-8.
pub fn container_text_to_package(bytes: &[u8]) -> Option<Vec<u8>> {
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes);
    std::str::from_utf8(body)
        .ok()
        .map(|t| encode_text(t, Encoding::Utf16Le))
}

fn detect(bytes: &[u8]) -> Encoding {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Encoding::Utf8Bom;
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return Encoding::Utf16LeBom;
    }
    let sample = bytes.get(..bytes.len().min(256)).unwrap_or_default();
    let odd_nuls = sample.iter().skip(1).step_by(2).filter(|b| **b == 0).count();
    let pairs = sample.len() / 2;
    if pairs > 0 && odd_nuls * 2 >= pairs {
        Encoding::Utf16Le
    } else {
        Encoding::Utf8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_text_files_convert_both_ways() {
        assert!(is_container_text("Copilot/settings.json"));
        assert!(is_container_text("Copilot/Instructions/instructions.MD"));
        assert!(!is_container_text(
            "Copilot/VerifiedAnswers/ResourcePackages/x.json"
        ));
        assert!(!is_container_text("Copilot/x.png"));
        let text = "Réponds en français 中文";
        let pkg = container_text_to_package(text.as_bytes()).unwrap();
        assert_eq!(pkg, encode_text(text, Encoding::Utf16Le));
        assert_eq!(container_text_to_project(&pkg).unwrap(), text.as_bytes());
        let bom = [&[0xEF, 0xBB, 0xBF][..], text.as_bytes()].concat();
        assert_eq!(container_text_to_package(&bom).unwrap(), pkg);
        assert!(container_text_to_project(&[0x41]).is_none());
        assert!(container_text_to_project(&[0x00, 0xD8]).is_none());
        assert!(container_text_to_package(&[0xFF]).is_none());
    }

    #[test]
    fn round_trips_every_encoding() {
        let s = "{\"name\":\"Ventes – Été\"}";
        for enc in [
            Encoding::Utf8,
            Encoding::Utf8Bom,
            Encoding::Utf16Le,
            Encoding::Utf16LeBom,
        ] {
            let bytes = encode_text(s, enc);
            let (back, got) = decode_text("p", &bytes).unwrap();
            assert_eq!(back, s);
            assert_eq!(got, enc);
        }
    }

    #[test]
    fn odd_length_utf16_is_an_error_not_a_panic() {
        assert!(decode_text("p", &[0xFF, 0xFE, 0x41]).is_err());
    }

    #[test]
    fn empty_part_is_empty_utf8() {
        assert_eq!(decode_text("p", &[]).unwrap(), (String::new(), Encoding::Utf8));
    }
}
