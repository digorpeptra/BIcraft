//! Glyph coverage of the bundled fonts (no system font): every character of the repository's
//! report fixtures and a list of characters common in report text has a glyph in the chain,
//! except CJK ideographs and kana, a documented gap (docs/gaps.md) that is drawn as a box and
//! reported, never dropped.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use bicraft_render::{Font, FontBook};

/// The documented gap: CJK (drawn as a box, listed by `report.render`).
fn is_cjk(c: char) -> bool {
    matches!(u32::from(c),
        0x2E80..=0x2FFF | 0x3000..=0x30FF | 0x3100..=0x31FF | 0x3200..=0x33FF | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF | 0xA960..=0xA97F | 0xAC00..=0xD7FF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF
        | 0x20000..=0x3FFFF)
}

/// The faces a report's text uses: regular, semibold (titles, headers), bold, light (callouts).
fn fonts() -> Vec<Font> {
    [300, 400, 600, 700]
        .into_iter()
        .map(|w| Font::new("Segoe UI", 12.0).weight(w))
        .chain([Font::new("DIN", 12.0), Font::new("wf_standard-font_light", 12.0)])
        .collect()
}

/// Every character must have a glyph, except CJK, which must be reported missing and drawn
/// as a box.
fn check(chars: &BTreeSet<char>, what: &str) {
    let book = FontBook::bundled_only();
    let text: String = chars.iter().collect();
    for f in fonts() {
        let missing = book.missing(&f, &text);
        let bad: Vec<String> = missing
            .iter()
            .filter(|c| !is_cjk(**c))
            .map(|c| format!("{c} U+{:04X}", u32::from(*c)))
            .collect();
        assert!(bad.is_empty(), "{what}: no glyph in {f:?} for {}", bad.join(", "));
    }
    let cjk: Vec<char> = chars.iter().copied().filter(|c| is_cjk(*c)).collect();
    if !cjk.is_empty() {
        let s: String = cjk.iter().collect();
        let f = Font::new("Segoe UI", 12.0);
        let w = book.measure(&f, &s);
        assert!(w > 0.0, "{what}: CJK takes no room");
        let reported = book.missing_glyphs();
        for c in &cjk {
            assert!(reported.contains(c), "{what}: {c} drawn but not reported");
        }
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

#[test]
fn every_fixture_character_has_a_glyph() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    let crates = std::fs::read_dir(root.join("crates")).unwrap();
    for c in crates.flatten() {
        walk(&c.path().join("tests/fixtures"), &mut files);
    }
    let text_ext = [
        "json", "tmdl", "pbir", "bim", "pbism", "pbip", "platform", "txt", "csv", "m", "dax", "xml", "tmsl",
    ];
    let mut chars = BTreeSet::new();
    let mut read = 0;
    for f in &files {
        let ext = f
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !text_ext.contains(&ext.as_str()) {
            continue;
        }
        let Ok(bytes) = std::fs::read(f) else { continue };
        // UTF-8, or UTF-16LE with its BOM (as Power BI writes some parts)
        let text = if bytes.starts_with(&[0xFF, 0xFE]) {
            let u: Vec<u16> = bytes[2..]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes(*c))
                .collect();
            String::from_utf16_lossy(&u)
        } else {
            String::from_utf8_lossy(&bytes).into_owned()
        };
        read += 1;
        chars.extend(text.chars().filter(|c| !c.is_control() && *c != '\u{feff}'));
    }
    assert!(read > 50, "only {read} fixture files found");
    // the fixtures hold accented Latin, CJK and emoji at least
    assert!(chars.contains(&'é') && chars.contains(&'語'), "{chars:?}");
    check(&chars, "fixtures");
}

#[test]
fn common_report_characters_have_a_glyph() {
    let list = concat!(
        // measure names and labels
        "CA Δ N-1 CA Δ% N-1 YoY Δ ∆ ΔΣΩαβγδεμπσφω ΑΒΓΔΕΖΗΘΙΚΛΜΝΞΟΠΡΣΤΥΦΧΨΩ",
        // arrows and trends
        "▲▼△▽►◄▶◀↑↓→←↗↘↔⇧⇩⇒⇐⬆⬇⬈⬊➔➜",
        // math
        "≥≤±≠×÷∑∏√∞≈≡∂∫∈∉∩∪⊂⊃∀∃¬∧∨‰‱°′″",
        // superscripts, subscripts, fractions
        "⁰¹²³⁴⁵⁶⁷⁸⁹⁺⁻ⁿ₀₁₂₃₄₅₆₇₈₉ m² m³ CO₂ ½¼¾⅓⅔⅛",
        // currency
        "€£¥₹₽₺₩₪₫₱₦₴₸₿¢₡₲₵",
        // status symbols
        "✓✔✗✘✕✖☑☒☐●○◉◆◇■□▪▫★☆⚠⚡⛔✅❌⭕❗❓⓪①②③④⑤",
        // box drawing and blocks (sparklines in text)
        "─│┌┐└┘├┤┬┴┼═║▁▂▃▄▅▆▇█░▒▓",
        // emoji used as indicators
        "🔴🟠🟡🟢🔵🟣⚫⚪🟥🟩🟨📈📉📊🎯🚀🎉👍👎💰🏆🔥⏱⏳",
        // punctuation and letters of European languages
        "–—‘’“”„‹›«»…•·№™©®℃℉ ĀăĄćČďĐęĚğĞİıŁłńňŐőŒœřŚşŠšţťŰűŸŹźŻżŽž ẠạẢảẤấẦầẨẩẪẫẬậ ПриветЁё",
        // the replacement character
        "\u{fffd}",
    );
    let chars: BTreeSet<char> = list.chars().filter(|c| !c.is_whitespace()).collect();
    check(&chars, "common characters");
}

#[test]
fn cjk_is_drawn_as_boxes_and_reported() {
    let book = FontBook::bundled_only();
    let f = Font::new("Segoe UI", 12.0);
    assert_eq!(
        book.missing(&f, "売上 매출 销售"),
        ['売', '上', '매', '출', '销', '售']
    );
    assert!(book.measure(&f, "売上") > 10.0);
    assert_eq!(book.missing_glyphs(), ['上', '売']);
}
