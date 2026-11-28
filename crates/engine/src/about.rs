//! `about.licenses`: BIcraft's licence and the third-party notices compiled into the binary.
//!
//! The binary embeds third-party material (the MIT XPress9 port, the MIT PBIR and report theme
//! schemas, the MIT best-practice rules, the OFL Selawik fonts, the OFL Carlito and Noto subsets (the
//! renderer's fallback faces and the UI's symbols), Unicode CLDR data, the MIT DashCraft port of
//! M library functions and the M formatter). Their licences require the notices to travel with every
//! copy, so the binary carries them itself: `bicraft-cli licenses`, the MCP `run_command`, and the
//! container image all reach this text.

use serde_json::{Value, json};

use crate::Session;

/// The project's licence expression.
pub const LICENSE: &str = "MIT OR Apache-2.0";

/// `THIRD_PARTY_NOTICES.md` at the repository root.
pub const THIRD_PARTY_NOTICES: &str = include_str!("../../../THIRD_PARTY_NOTICES.md");

/// `(name, text)` of each licence file the notices refer to.
pub const LICENSE_FILES: &[(&str, &str)] = &[
    ("LICENSE-MIT", include_str!("../../../LICENSE-MIT")),
    ("LICENSE-APACHE", include_str!("../../../LICENSE-APACHE")),
    (
        "licenses/xpress9/LICENSE-MIT-microsoft-xpress9",
        include_str!("../../xpress9/LICENSE-MIT-microsoft-xpress9"),
    ),
    (
        "licenses/selawik-font/OFL-Selawik.txt",
        include_str!("../../render/fonts/OFL-Selawik.txt"),
    ),
    (
        "licenses/noto-sans-symbols2/OFL-NotoSansSymbols2.txt",
        include_str!("../../ui/fonts/OFL-NotoSansSymbols2.txt"),
    ),
    (
        "licenses/noto-sans/OFL-NotoSans.txt",
        include_str!("../../render/fonts/OFL-NotoSans.txt"),
    ),
    (
        "licenses/noto-sans-math/OFL-NotoSansMath.txt",
        include_str!("../../render/fonts/OFL-NotoSansMath.txt"),
    ),
    (
        "licenses/noto-sans-symbols/OFL-NotoSansSymbols.txt",
        include_str!("../../render/fonts/OFL-NotoSansSymbols.txt"),
    ),
    (
        "licenses/noto-emoji/OFL-NotoEmoji.txt",
        include_str!("../../render/fonts/OFL-NotoEmoji.txt"),
    ),
    (
        "licenses/carlito/OFL-Carlito.txt",
        include_str!("../../render/fonts/carlito/OFL-Carlito.txt"),
    ),
    (
        "licenses/bpa-rules/LICENSE",
        include_str!("../../bpa/rules/LICENSE"),
    ),
    (
        "licenses/pbir-schemas/LICENSE",
        include_str!("../../pbir/schemas/LICENSE"),
    ),
    (
        "licenses/report-theme-schema/LICENSE",
        include_str!("../../visuals/schemas/LICENSE"),
    ),
    (
        "licenses/unicode-cldr/LICENSE-UNICODE",
        include_str!("../../locale/LICENSE-UNICODE"),
    ),
    (
        "licenses/dashcraft/LICENSE-MIT-dashcraft",
        include_str!("../../data/LICENSE-MIT-dashcraft"),
    ),
    (
        "licenses/d3/LICENSE-BSD-d3",
        include_str!("../../render/LICENSE-BSD-d3"),
    ),
    (
        "licenses/d3/LICENSE-BSD-d3-array",
        include_str!("../../render/LICENSE-BSD-d3-array"),
    ),
    (
        "licenses/deckcraft/LICENSE-MIT-deckcraft",
        include_str!("../../ui/LICENSE-MIT-deckcraft"),
    ),
];

/// The whole text: the notices, then each licence file under a heading.
pub fn licenses_text() -> String {
    let mut out = String::with_capacity(64 * 1024);
    out.push_str(&format!("BIcraft is licensed under {LICENSE}.\n\n"));
    out.push_str(THIRD_PARTY_NOTICES);
    for (name, text) in LICENSE_FILES {
        out.push_str(&format!("\n\n----- {name} -----\n\n"));
        out.push_str(text);
    }
    out
}

pub(crate) fn about_licenses(_: &Session, p: &Value) -> Result<Value, String> {
    crate::params::allow(p, &[])?;
    let files: Vec<Value> = LICENSE_FILES
        .iter()
        .map(|(name, text)| json!({"name": name, "text": text}))
        .collect();
    Ok(json!({
        "license": LICENSE,
        "third_party_notices": THIRD_PARTY_NOTICES,
        "files": files,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_licence_is_embedded() {
        let text = licenses_text();
        for needle in [
            "MIT OR Apache-2.0",
            "Copyright (c) Microsoft Corporation",
            "SIL OPEN FONT LICENSE",
            "UNICODE LICENSE",
            "Apache License",
            "Michael Bostock",
        ] {
            assert!(text.to_uppercase().contains(&needle.to_uppercase()), "{needle}");
        }
        let s = Session::new();
        let v = about_licenses(&s, &json!({})).unwrap();
        assert_eq!(v["license"], LICENSE);
        assert_eq!(v["files"].as_array().map(Vec::len), Some(LICENSE_FILES.len()));
        assert!(about_licenses(&s, &json!({"x": 1})).is_err());
    }
}
