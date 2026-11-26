//! Text decoding for the XML parts of an ABF.
//!
//! Observed encodings: the header XML is UTF-16LE without a BOM (the BOM sits at stream offset
//! 0, before the signature); the files-section XML documents are UTF-16LE with a BOM; the
//! virtual directory is UTF-8 without a BOM. Detection is therefore by BOM, then by the shape
//! of the first code unit. Invalid sequences decode lossily: the container only needs the
//! markup and ASCII-ish values, and a stray bad unit should not hide every file.

use crate::Error;

/// Largest XML part we are willing to decode (bytes). Real directories are a few hundred
/// bytes per file, so this allows well over a million files.
pub(crate) const MAX_XML_BYTES: usize = 512 * 1024 * 1024;

/// How an XML part is encoded, as detected by [`detect_encoding`]. The writer re-encodes a
/// patched document the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextEncoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16LeBom,
}

/// Detection by BOM, then by the shape of the first code unit.
pub(crate) fn detect_encoding(bytes: &[u8]) -> TextEncoding {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        TextEncoding::Utf16LeBom
    } else if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        TextEncoding::Utf8Bom
    } else if matches!(bytes, [lo, 0, ..] if *lo != 0) {
        TextEncoding::Utf16Le
    } else {
        TextEncoding::Utf8
    }
}

/// Encodes `text` with `enc`, BOM included when `enc` has one.
pub(crate) fn encode(text: &str, enc: TextEncoding) -> Vec<u8> {
    match enc {
        TextEncoding::Utf8 => text.as_bytes().to_vec(),
        TextEncoding::Utf8Bom => [&[0xEF, 0xBB, 0xBF][..], text.as_bytes()].concat(),
        TextEncoding::Utf16Le | TextEncoding::Utf16LeBom => {
            let bom: &[u8] = if enc == TextEncoding::Utf16LeBom {
                &[0xFF, 0xFE]
            } else {
                &[]
            };
            bom.iter()
                .copied()
                .chain(text.encode_utf16().flat_map(u16::to_le_bytes))
                .collect()
        }
    }
}

pub(crate) fn decode_xml(bytes: &[u8], part: &'static str) -> Result<String, Error> {
    if bytes.len() > MAX_XML_BYTES {
        return Err(Error::Malformed {
            part,
            reason: format!("{} bytes exceeds the XML size limit", bytes.len()),
        });
    }
    Ok(match detect_encoding(bytes) {
        TextEncoding::Utf16LeBom => utf16le_lossy(bytes.get(2..).unwrap_or_default()),
        TextEncoding::Utf8Bom => String::from_utf8_lossy(bytes.get(3..).unwrap_or_default()).into_owned(),
        TextEncoding::Utf16Le => utf16le_lossy(bytes),
        TextEncoding::Utf8 => String::from_utf8_lossy(bytes).into_owned(),
    })
}

/// Decodes UTF-16LE, ignoring a trailing odd byte and replacing unpaired surrogates.
pub(crate) fn utf16le_lossy(bytes: &[u8]) -> String {
    let units = bytes.as_chunks::<2>().0.iter().map(|p| u16::from_le_bytes(*p));
    char::decode_utf16(units)
        .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

/// Length of the prefix of `bytes` before the first UTF-16 NUL code unit (even offsets only).
pub(crate) fn utf16_len_before_nul(bytes: &[u8]) -> usize {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .position(|p| *p == [0, 0])
        .map_or(bytes.len() & !1, |units| units * 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_encodings() {
        assert_eq!(decode_xml(b"\xFF\xFE<\0a\0", "t").unwrap(), "<a");
        assert_eq!(decode_xml(b"<\0a\0", "t").unwrap(), "<a");
        assert_eq!(decode_xml(b"\xEF\xBB\xBF<a", "t").unwrap(), "<a");
        assert_eq!(decode_xml(b"<a>", "t").unwrap(), "<a>");
        assert_eq!(decode_xml(b"", "t").unwrap(), "");
    }

    #[test]
    fn encode_inverts_decode() {
        for bytes in [&b"\xFF\xFE<\0a\0"[..], b"<\0a\0", b"\xEF\xBB\xBF<a", b"<a>", b""] {
            let enc = detect_encoding(bytes);
            assert_eq!(encode(&decode_xml(bytes, "t").unwrap(), enc), bytes);
        }
    }

    #[test]
    fn utf16_is_lossy_and_bounded() {
        assert_eq!(utf16le_lossy(&[0x00, 0xD8, b'a', 0, 7]), "\u{fffd}a");
        assert_eq!(utf16_len_before_nul(b"a\0b\0\0\0c\0"), 4);
        assert_eq!(utf16_len_before_nul(b"a\0b"), 2);
    }
}
