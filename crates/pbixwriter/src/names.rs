//! Part names. System.IO.Packaging stores each part under its escaped URI (RFC 3986 percent
//! encoding of everything outside the path characters), so the zip entry of
//! `/Report/StaticResources/My Logo.png` is `Report/StaticResources/My%20Logo.png`.

/// Rejects paths that cannot become a package part: empty, absolute, backslashes, empty, `.` or
/// `..` segments, segments ending in `.` (forbidden by OPC), control characters.
pub(crate) fn check_relative_path(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("empty path".into());
    }
    if path.starts_with('/') {
        return Err("absolute path".into());
    }
    if path.contains('\\') {
        return Err("backslash in path (use `/`)".into());
    }
    if path.chars().any(char::is_control) {
        return Err("control character in path".into());
    }
    for seg in path.split('/') {
        if seg.is_empty() {
            return Err("empty path segment".into());
        }
        if seg.ends_with('.') {
            return Err("path segment ending in `.`".into());
        }
    }
    Ok(())
}

/// Bytes kept as-is in a part URI path: RFC 3986 unreserved, sub-delims, `:`, `@` and `/`.
fn is_path_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@/".contains(&b)
}

/// The zip entry name of a part path (percent-encoded UTF-8).
pub(crate) fn encode_part_name(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for &b in path.as_bytes() {
        if is_path_byte(b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A GUID-shaped name derived from `seed` (FNV-1a), so the same input always gives the same name.
pub(crate) fn stable_guid(seed: &str) -> String {
    let hash = |basis: u64| {
        seed.bytes().fold(basis, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01B3)
        })
    };
    let hi = hash(0xCBF2_9CE4_8422_2325);
    let lo = hash(0x6C62_272E_07BB_0142);
    // Version 4 / RFC 4122 variant bits, so tools that parse it see a well-formed GUID.
    let hi = (hi & 0xFFFF_FFFF_FFFF_0FFF) | 0x0000_0000_0000_4000;
    let lo = (lo & 0x3FFF_FFFF_FFFF_FFFF) | 0x8000_0000_0000_0000;
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        hi >> 32,
        (hi >> 16) & 0xFFFF,
        hi & 0xFFFF,
        lo >> 48,
        lo & 0xFFFF_FFFF_FFFF
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_like_a_part_uri() {
        assert_eq!(
            encode_part_name("Report/definition/pages/abc/page.json"),
            "Report/definition/pages/abc/page.json"
        );
        assert_eq!(encode_part_name("a b/é%.png"), "a%20b/%C3%A9%25.png");
    }

    #[test]
    fn guid_shape() {
        let g = stable_guid("x");
        assert_eq!(g.len(), 36);
        assert_eq!(g.as_bytes().get(14), Some(&b'4'));
        assert_eq!(g, stable_guid("x"));
        assert_ne!(g, stable_guid("y"));
    }
}
