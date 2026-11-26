//! Glyph subsets of TrueType fonts (`glyf` outlines) for BIcraft's bundled fallback faces
//! (docs/design.md, "Typography"; docs/render.md, "Fonts").
//!
//! - `cargo xtask font-subset IN.ttf OUT.ttf CHARS`: one subset. Glyph ids are kept as they are:
//!   the glyphs of `CHARS` (and the components of composite glyphs) keep their outlines, every
//!   other glyph becomes empty. `CHARS` is literal characters and `U+XXXX` or `U+XXXX-YYYY`
//!   tokens. This is how the UI's symbol face `crates/ui/fonts/NotoSansSymbols2-subset.ttf` was
//!   made.
//! - `cargo xtask font-subset bundle SRC_DIR`: the renderer's fallback chain
//!   (`crates/render/fonts/`, listed in [`BUNDLE`]) from the upstream Noto fonts unpacked in
//!   `SRC_DIR` (each file's SHA-256 is checked against the release recorded here). Each face
//!   keeps the characters of [`RANGES`] that it has and that neither Selawik nor an earlier face
//!   of the chain has, with its glyphs renumbered (no empty glyph slots). It writes
//!   `crates/render/fonts/SOURCE.txt` and prints what is left uncovered.
//! - `cargo xtask font-subset carlito SRC_DIR`: the renderer's stand-in for Calibri
//!   (`crates/render/fonts/carlito/`, listed in [`CARLITO`]) from the four Carlito faces of
//!   Google Fonts' repository unpacked in `SRC_DIR` (SHA-256 checked). Each face keeps the text
//!   characters of [`CARLITO_RANGES`] it has, glyphs renumbered; it writes
//!   `crates/render/fonts/carlito/SOURCE.txt`.
//!
//! In both, the character map lists the kept characters only (format 4, plus format 12 for
//! characters outside the BMP), `post` becomes format 3 (no glyph names), and the layout tables
//! (`GSUB`, `GPOS`, `GDEF`…) and hinting tables are dropped. `head`, `hhea`, `maxp`, `OS/2`,
//! `name` and `hmtx` are kept (the `name` table keeps the font's copyright and licence strings,
//! as the OFL asks). Characters the font does not have are reported and left out.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

/// Unicode ranges the renderer's chain is asked to cover (inclusive): what report text commonly
/// holds beyond Selawik's Latin, chosen to keep the bundle small (rare and historic characters of
/// these blocks are left out). CJK is not here (a documented gap: docs/gaps.md G43; its
/// characters are drawn as boxes and reported). Emoji: [`EMOJI`].
pub const RANGES: &[(u32, u32, &str)] = &[
    (0x00A0, 0x017F, "Latin-1 Supplement, Latin Extended-A"),
    (0x018F, 0x018F, "Latin Extended-B: schwa"),
    (0x0192, 0x0192, "Latin Extended-B: florin"),
    (0x01A0, 0x01B0, "Latin Extended-B: Vietnamese horned letters"),
    (0x01CD, 0x01DC, "Latin Extended-B: pinyin"),
    (0x0218, 0x021B, "Latin Extended-B: Romanian"),
    (0x0259, 0x0259, "IPA: schwa"),
    (0x02BB, 0x02BC, "Spacing Modifier Letters: apostrophes"),
    (0x02C6, 0x02DD, "Spacing Modifier Letters: accents"),
    (0x0300, 0x0338, "Combining Diacritical Marks"),
    (0x0370, 0x03FF, "Greek and Coptic"),
    (0x0400, 0x045F, "Cyrillic"),
    (0x0490, 0x0491, "Cyrillic: Ukrainian"),
    (0x1E80, 0x1E85, "Latin Extended Additional: Welsh"),
    (0x1E9E, 0x1E9E, "Latin Extended Additional: capital sharp s"),
    (0x1EA0, 0x1EF9, "Latin Extended Additional: Vietnamese"),
    (0x2000, 0x206F, "General Punctuation"),
    (0x2070, 0x209F, "Superscripts and Subscripts"),
    (0x20A0, 0x20C0, "Currency Symbols"),
    (0x2100, 0x214F, "Letterlike Symbols"),
    (0x2150, 0x216B, "Number Forms: fractions, Roman numerals"),
    (0x2190, 0x21FF, "Arrows"),
    (0x2200, 0x22FF, "Mathematical Operators"),
    (0x2300, 0x231F, "Miscellaneous Technical"),
    (0x2325, 0x232B, "Miscellaneous Technical: keys"),
    (0x23CE, 0x23CF, "Miscellaneous Technical: return, eject"),
    (0x23E9, 0x23FA, "Miscellaneous Technical: media controls, clocks"),
    (0x2460, 0x2473, "Enclosed Alphanumerics: circled numbers"),
    (0x24EA, 0x24FF, "Enclosed Alphanumerics: circled numbers"),
    (0x2500, 0x257F, "Box Drawing"),
    (0x2580, 0x259F, "Block Elements"),
    (0x25A0, 0x25FF, "Geometric Shapes"),
    (0x2600, 0x26FF, "Miscellaneous Symbols"),
    (0x2700, 0x27BF, "Dingbats"),
    (
        0x27E8,
        0x27E9,
        "Miscellaneous Mathematical Symbols-A: angle brackets",
    ),
    (0x27F5, 0x27FA, "Supplemental Arrows-A: long arrows"),
    (0x2934, 0x2935, "Supplemental Arrows-B: curved arrows"),
    (
        0x2B00,
        0x2B2F,
        "Miscellaneous Symbols and Arrows: arrows, squares",
    ),
    (0x2B50, 0x2B59, "Miscellaneous Symbols and Arrows: stars, circles"),
    (0xFFFD, 0xFFFD, "Replacement Character"),
    (
        0x1F7E0,
        0x1F7EB,
        "Geometric Shapes Extended: coloured circles and squares",
    ),
];

/// Emoji common in report text (status, trends, money, time, places, hands, faces, hearts…),
/// from Noto Emoji (monochrome).
pub const EMOJI: &str = "🔴🟠🟡🟢🔵🟣🟤🟥🟧🟨🟩🟦🟪🟫⚫⚪⬛⬜◼◻▪▫🔶🔷🔸🔹🔺🔻💠🔘\
✅❌❎⭕❗❓❕❔⚠⛔🚫⭐🌟✨💯🔥💡🎯🏆🥇🥈🥉🏅🎖🚀🎉🎊🎁\
📈📉📊📌📍📎📅📆🗓📋📝📁📂🗂🗃🗑📦📫📧📨✉📞☎📱💻🖥🖨⌨🖱💾🔒🔓🔑🔐⚙🔧🔨🛠🔍🔎🔗🔔🔕📢📣\
💰💵💶💷💴💸💳🪙🏦🛒🛍🏷🧾🚚🚛✈🚗🚢🚆🏠🏢🏭🏬🏥🏫🏪🏨🌍🌎🌏🗺📍🧭\
⏰⏱⏲⏳⌛🕐🕑🕒🕓🕔🕕🕖🕗🕘🕙🕚🕛\
☀🌤⛅☁🌧⛈🌩❄🌡💧🌊⚡🌱🌳🍎🍕☕\
👍👎👏🙌👌✋🤝💪👀👁👤👥🧑👨👩🙋\
😀😃😄😁🙂😊😉😍😎🤔😐😑🙁😞😟😢😭😡😱🤯😴🥳\
❤🧡💛💚💙💜🖤🤍🤎💔\
🔼🔽⏫⏬⏩⏪⏭⏮⏯⏸⏹⏺▶◀🔀🔁🔂🔄🔃↩↪⤴⤵⬆⬇⬅➡↗↘↙↖↕↔\
➕➖➗✖🟰🆕🆗🆙🆒🆓🆘🔝🔜🔚🔛🔟🔢🔣🔤🏁🚩🏳🏴⚑";

/// One upstream release.
pub struct Upstream {
    pub name: &'static str,
    pub repo: &'static str,
    pub release: &'static str,
    pub archive: &'static str,
    pub archive_sha256: &'static str,
    pub licence: &'static str,
}

/// One bundled face: the upstream file, its SHA-256 and the subset written.
pub struct BundleFace {
    pub upstream: &'static Upstream,
    pub path_in_archive: &'static str,
    pub sha256: &'static str,
    pub out: &'static str,
    /// Which of the characters still uncovered this face may take.
    pub only: Only,
}

/// A face's share of the uncovered characters.
pub enum Only {
    All,
    /// These ranges only (inclusive).
    Ranges(&'static [(u32, u32)]),
    /// The characters of [`EMOJI`] only.
    Emoji,
}

const NOTO_SANS: Upstream = Upstream {
    name: "Noto Sans",
    repo: "https://github.com/notofonts/latin-greek-cyrillic",
    release: "NotoSans-v2.015",
    archive: "https://github.com/notofonts/latin-greek-cyrillic/releases/download/NotoSans-v2.015/NotoSans-v2.015.zip",
    archive_sha256: "0c34df072a3fa7efbb7cbf34950e1f971a4447cffe365d3a359e2d4089b958f5",
    licence: "OFL-NotoSans.txt",
};
const NOTO_SANS_MONO: Upstream = Upstream {
    name: "Noto Sans Mono",
    repo: "https://github.com/notofonts/latin-greek-cyrillic",
    release: "NotoSansMono-v2.014",
    archive: "https://github.com/notofonts/latin-greek-cyrillic/releases/download/NotoSansMono-v2.014/NotoSansMono-v2.014.zip",
    archive_sha256: "090cf6c5e03f337a755630ca888b1fef463e64ae7b33ee134e9309c05f978732",
    licence: "OFL-NotoSans.txt (the same text as Noto Sans's)",
};
const NOTO_SYMBOLS2: Upstream = Upstream {
    name: "Noto Sans Symbols 2",
    repo: "https://github.com/notofonts/symbols",
    release: "NotoSansSymbols2-v2.008",
    archive: "https://github.com/notofonts/symbols/releases/download/NotoSansSymbols2-v2.008/NotoSansSymbols2-v2.008.zip",
    archive_sha256: "346c930bbe8eb946701a05c54e9c11a2094dee1d93c387bf1771c0a3e335688f",
    licence: "OFL-NotoSansSymbols2.txt",
};
const NOTO_MATH: Upstream = Upstream {
    name: "Noto Sans Math",
    repo: "https://github.com/notofonts/math",
    release: "NotoSansMath-v3.000",
    archive: "https://github.com/notofonts/math/releases/download/NotoSansMath-v3.000/NotoSansMath-v3.000.zip",
    archive_sha256: "ac351837b41f8a897f020b97fb0f075ad574c1e9669fb5839ada1f92fd748356",
    licence: "OFL-NotoSansMath.txt",
};
const NOTO_EMOJI: Upstream = Upstream {
    name: "Noto Emoji",
    repo: "https://github.com/google/fonts (ofl/notoemoji; upstream googlefonts/emoji-bw)",
    release: "google/fonts commit 8b0a1d0f5983c89bc2b93f1b5fb55f9e252744b5",
    archive: "https://raw.githubusercontent.com/google/fonts/8b0a1d0f5983c89bc2b93f1b5fb55f9e252744b5/ofl/notoemoji/NotoEmoji%5Bwght%5D.ttf",
    archive_sha256: "de6c18832938afc99caf132b39d6a30a19bac7f2e812e28db2535b4608d27551",
    licence: "OFL-NotoEmoji.txt",
};
const NOTO_SYMBOLS: Upstream = Upstream {
    name: "Noto Sans Symbols",
    repo: "https://github.com/notofonts/symbols",
    release: "NotoSansSymbols-v2.003",
    archive: "https://github.com/notofonts/symbols/releases/download/NotoSansSymbols-v2.003/NotoSansSymbols-v2.003.zip",
    archive_sha256: "0c113cdcf6c31d050b80dac39fba2d804a6985281012e76e9220c0a00da007f3",
    licence: "OFL-NotoSansSymbols.txt",
};

/// The renderer's bundled fallback faces, in chain order (after Selawik). Faces of one upstream
/// family in a row (Noto Sans Regular and Bold) share one character set: the first one's.
pub const BUNDLE: &[BundleFace] = &[
    BundleFace {
        upstream: &NOTO_SANS,
        path_in_archive: "NotoSans/unhinted/ttf/NotoSans-Regular.ttf",
        sha256: "f3961a9cde016d41a4879aecda1474d3a36d6bf54fa0e4643de029cc2248b0e8",
        out: "NotoSans-Regular-subset.ttf",
        only: Only::All,
    },
    BundleFace {
        upstream: &NOTO_SANS,
        path_in_archive: "NotoSans/unhinted/ttf/NotoSans-Bold.ttf",
        sha256: "87cb2d84472a7d66da659ee47b6cdb9552326e8c128245231f191b6ac72529d9",
        out: "NotoSans-Bold-subset.ttf",
        only: Only::All,
    },
    BundleFace {
        upstream: &NOTO_SANS_MONO,
        path_in_archive: "NotoSansMono/unhinted/ttf/NotoSansMono-Regular.ttf",
        sha256: "87f8ce0522a6c99b743ee5fc75b4073cfdd575639119672828b7b9944b65b4f4",
        out: "NotoSansMono-Regular-subset.ttf",
        // box drawing and block elements, designed to join (no other face has them all)
        only: Only::Ranges(&[(0x2500, 0x259F)]),
    },
    BundleFace {
        upstream: &NOTO_SYMBOLS2,
        path_in_archive: "NotoSansSymbols2/unhinted/ttf/NotoSansSymbols2-Regular.ttf",
        sha256: "c4a0a80f0041ce4be81e2478faad22776d23edb98ae3f0d19bd37044820ecf9d",
        out: "NotoSansSymbols2-Regular-subset.ttf",
        only: Only::All,
    },
    BundleFace {
        upstream: &NOTO_MATH,
        path_in_archive: "NotoSansMath/unhinted/ttf/NotoSansMath-Regular.ttf",
        sha256: "b127e84699212b6b2ef50aff58e0ebebeec04ffe6db1b9eb9e209c8c3d97b4aa",
        out: "NotoSansMath-Regular-subset.ttf",
        only: Only::All,
    },
    BundleFace {
        upstream: &NOTO_SYMBOLS,
        path_in_archive: "NotoSansSymbols/unhinted/ttf/NotoSansSymbols-Regular.ttf",
        sha256: "6eea9cb4cd39269ea9f95ba5c2735f80ae74049dfc9e1a7c932a5cfc8f0c3030",
        out: "NotoSansSymbols-Regular-subset.ttf",
        only: Only::All,
    },
    BundleFace {
        upstream: &NOTO_EMOJI,
        path_in_archive: "ofl/notoemoji/NotoEmoji[wght].ttf",
        sha256: "de6c18832938afc99caf132b39d6a30a19bac7f2e812e28db2535b4608d27551",
        out: "NotoEmoji-Regular-subset.ttf",
        only: Only::Emoji,
    },
];

pub fn cmd(args: &[String]) -> Result<(), String> {
    match args {
        [b, src] if b == "bundle" => bundle(Path::new(src)),
        [b, src] if b == "carlito" => carlito(Path::new(src)),
        [input, output, chars] => {
            let data = std::fs::read(input).map_err(|e| format!("{input}: {e}"))?;
            let wanted = parse_chars(chars)?;
            let (out, kept, missing) = subset(&data, &wanted, false)?;
            std::fs::write(output, &out).map_err(|e| format!("{output}: {e}"))?;
            eprintln!(
                "font-subset: {} bytes -> {} bytes; kept {}",
                data.len(),
                out.len(),
                list(&kept)
            );
            if !missing.is_empty() {
                eprintln!("font-subset: not in the font: {}", list(&missing));
            }
            Ok(())
        }
        _ => Err("usage: cargo xtask font-subset IN.ttf OUT.ttf CHARS | font-subset bundle SRC_DIR | font-subset carlito SRC_DIR".into()),
    }
}

fn list(v: &[char]) -> String {
    v.iter()
        .map(|c| format!("{c} U+{:04X}", *c as u32))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Characters as compact `U+` ranges.
pub fn ranges_text(chars: &BTreeSet<char>) -> String {
    let mut out = String::new();
    let mut it = chars.iter().map(|c| *c as u32).peekable();
    while let Some(start) = it.next() {
        let mut end = start;
        while it.peek() == Some(&(end + 1)) {
            end += 1;
            it.next();
        }
        if !out.is_empty() {
            out.push(' ');
        }
        if end == start {
            let _ = write!(out, "U+{start:04X}");
        } else {
            let _ = write!(out, "U+{start:04X}-{end:04X}");
        }
    }
    out
}

/// `CHARS`: literal characters, and `U+XXXX` / `U+XXXX-YYYY` tokens.
fn parse_chars(spec: &str) -> Result<BTreeSet<char>, String> {
    let mut out = BTreeSet::new();
    for tok in spec.split_whitespace() {
        if let Some(r) = tok.strip_prefix("U+").or_else(|| tok.strip_prefix("u+")) {
            let (a, b) = r.split_once('-').unwrap_or((r, r));
            let hex = |s: &str| {
                u32::from_str_radix(s.trim_start_matches("U+"), 16)
                    .map_err(|_| format!("bad code point `{tok}`"))
            };
            let (a, b) = (hex(a)?, hex(b)?);
            if b < a || b - a > 0x10000 {
                return Err(format!("bad range `{tok}`"));
            }
            out.extend((a..=b).filter_map(char::from_u32));
        } else {
            out.extend(tok.chars());
        }
    }
    Ok(out)
}

/// `font-subset bundle SRC_DIR`.
fn bundle(src: &Path) -> Result<(), String> {
    let root = crate::root();
    let fonts = root.join("crates/render/fonts");
    let all: BTreeSet<char> = RANGES
        .iter()
        .flat_map(|(a, b, _)| (*a..=*b).filter_map(char::from_u32))
        .chain(EMOJI.chars())
        .filter(|c| !c.is_control() && !c.is_whitespace())
        .collect();
    let emoji: BTreeSet<char> = EMOJI.chars().collect();
    let selawik = std::fs::read(fonts.join("selawk.ttf")).map_err(|e| format!("selawk.ttf: {e}"))?;
    let t = tables(&selawik)?;
    let cmap = t.get(b"cmap").ok_or("selawk.ttf: no cmap")?;
    let mut covered: BTreeSet<char> = char_glyphs(cmap, &all)?.into_keys().collect();
    let mut doc = String::from(
        "The renderer's bundled fallback faces (after Selawik, in this order): glyph subsets of\n\
         Noto fonts, made by `cargo xtask font-subset bundle SRC_DIR` (xtask/src/font_subset.rs:\n\
         the release, file and SHA-256 of each source; the Unicode ranges and emoji asked for). Each face keeps\n\
         the characters of those blocks it has that Selawik and the faces before it lack, with its\n\
         glyphs renumbered; cmap of these characters only; post format 3; layout and hinting tables\n\
         dropped; name table kept. Licence: SIL Open Font License 1.1, no Reserved Font Name;\n\
         copyright The Noto Project Authors (Noto Emoji: Google LLC). The OFL-*.txt files here\n\
         are the upstream OFL.txt files, unmodified.\n",
    );
    let _ = writeln!(
        doc,
        "\nSelawik covers {} of the {} characters asked for.",
        covered.len(),
        all.len()
    );
    let mut group: Option<(&str, BTreeSet<char>)> = None;
    let mut total = 0usize;
    for f in BUNDLE {
        let name = f.path_in_archive.rsplit('/').next().unwrap_or(f.path_in_archive);
        let path = src.join(name);
        let data = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let sha = sha256_hex(&data);
        if sha != f.sha256 {
            return Err(format!(
                "{}: SHA-256 {sha}, expected {} ({})",
                path.display(),
                f.sha256,
                f.upstream.release
            ));
        }
        let wanted = match &group {
            Some((rel, set)) if *rel == f.upstream.release => set.clone(),
            _ => all
                .difference(&covered)
                .copied()
                .filter(|c| match &f.only {
                    Only::All => true,
                    Only::Ranges(r) => r.iter().any(|(a, b)| (*a..=*b).contains(&(*c as u32))),
                    Only::Emoji => emoji.contains(c),
                })
                .collect(),
        };
        let (out, kept, _) = subset(&data, &wanted, true)?;
        let kept: BTreeSet<char> = kept.into_iter().collect();
        if !matches!(&group, Some((rel, _)) if *rel == f.upstream.release) {
            covered.extend(kept.iter().copied());
            group = Some((f.upstream.release, kept.clone()));
        }
        std::fs::write(fonts.join(f.out), &out).map_err(|e| format!("{}: {e}", f.out))?;
        total += out.len();
        eprintln!("font-subset: {} {} chars, {} bytes", f.out, kept.len(), out.len());
        let _ = write!(
            doc,
            "\n{out_name}: a glyph subset of {name} ({fam}).\n\
             Upstream:  {repo}, release {rel}\n\
             Archive:   {arch}\n           SHA-256 {asha}\n\
             Source:    {pia} ({len} bytes)\n           SHA-256 {sha}\n\
             Licence:   {lic}\n\
             Subset:    SHA-256 {osha} ({olen} bytes)\n\
             Characters ({n}): {chars}\n",
            out_name = f.out,
            fam = f.upstream.name,
            repo = f.upstream.repo,
            rel = f.upstream.release,
            arch = f.upstream.archive,
            asha = f.upstream.archive_sha256,
            pia = f.path_in_archive,
            len = data.len(),
            lic = f.upstream.licence,
            osha = sha256_hex(&out),
            olen = out.len(),
            n = kept.len(),
            chars = wrap(&ranges_text(&kept), 88, "           "),
        );
    }
    let left: BTreeSet<char> = all.difference(&covered).copied().collect();
    let _ = writeln!(
        doc,
        "\nCharacters of the blocks asked for that no bundled face has ({}), drawn as a box and\n\
         reported by report.render: {}",
        left.len(),
        wrap(&ranges_text(&left), 88, "")
    );
    std::fs::write(fonts.join("SOURCE.txt"), doc).map_err(|e| format!("SOURCE.txt: {e}"))?;
    eprintln!(
        "font-subset: {total} bytes in all; {} characters asked for, {} uncovered",
        all.len(),
        left.len()
    );
    Ok(())
}

/// Carlito (metric-compatible with Calibri: the same advance widths), from Google Fonts'
/// repository at a pinned commit (the upstream project, googlefonts/carlito, publishes sources
/// only).
const CARLITO_REPO: &str = "https://github.com/google/fonts";
const CARLITO_COMMIT: &str = "07ace6abab87a122865e5cb82c7540b39551edb2";
const CARLITO_VERSION: &str = "Version 1.104";

/// The Carlito faces bundled: file name, SHA-256 of the upstream file, subset written.
pub const CARLITO: &[(&str, &str, &str)] = &[
    (
        "Carlito-Regular.ttf",
        "f6418f708baede9789daef5d458c0f53d2a888af9820e8062934e504fedc6595",
        "Carlito-Regular-subset.ttf",
    ),
    (
        "Carlito-Bold.ttf",
        "bb5d20f79b82599ec72983597437373a80f2d2085fa91fc144fd74e876a594db",
        "Carlito-Bold-subset.ttf",
    ),
    (
        "Carlito-Italic.ttf",
        "0b019225e58d702bfedcbd35c21696769f8ee115cb6343f84c2f240312450d1c",
        "Carlito-Italic-subset.ttf",
    ),
    (
        "Carlito-BoldItalic.ttf",
        "b32928186c119599e03ca6a1ffc680fdcb7fac95772f4b95d989cf6cd3861517",
        "Carlito-BoldItalic-subset.ttf",
    ),
];

/// The characters kept in the Carlito faces (inclusive): Western and Central European Latin
/// text with its punctuation, superscripts, currency and common letterlike symbols, chosen to
/// keep the four faces small. Greek, Cyrillic, Vietnamese and symbols are left to the fallback
/// chain (Noto Sans: other widths than Calibri's, docs/gaps.md).
pub const CARLITO_RANGES: &[(u32, u32)] = &[
    (0x0020, 0x007E),
    (0x00A0, 0x017F),
    (0x0192, 0x0192),
    (0x0218, 0x021B),
    (0x02C6, 0x02C7),
    (0x02D8, 0x02DD),
    (0x2010, 0x2027),
    (0x2030, 0x203A),
    (0x2044, 0x2044),
    (0x2070, 0x2079),
    (0x20A0, 0x20C0),
    (0x2113, 0x2113),
    (0x2116, 0x2116),
    (0x2122, 0x2122),
    (0x2126, 0x2126),
    (0x212E, 0x212E),
    (0x2153, 0x215E),
    (0x2212, 0x2212),
    (0xFB01, 0xFB02),
    (0xFFFD, 0xFFFD),
];

/// The family name of the Carlito subsets. Carlito's licence reserves the name "Carlito", and a
/// subset is a Modified Version under the OFL, so the subsets carry another name.
pub const CARLITO_SUBSET_FAMILY: &str = "BIcraft Metric Sans";

/// A Windows (platform 3, Unicode BMP, en-US) string of a `name` table.
fn name_string(name: &[u8], id: u16) -> Option<String> {
    let count = usize::from(u16_at(name, 2).ok()?);
    let storage = usize::from(u16_at(name, 4).ok()?);
    (0..count).find_map(|i| {
        let r = 6 + 12 * i;
        let rec = (
            u16_at(name, r).ok()?,
            u16_at(name, r + 2).ok()?,
            u16_at(name, r + 4).ok()?,
            u16_at(name, r + 6).ok()?,
        );
        if rec != (3, 1, 0x409, id) {
            return None;
        }
        let len = usize::from(u16_at(name, r + 8).ok()?);
        let at = storage + usize::from(u16_at(name, r + 10).ok()?);
        let b = name.get(at..at + len)?;
        let u: Vec<u16> = b
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_be_bytes(*c))
            .collect();
        String::from_utf16(&u).ok()
    })
}

/// A new `name` table (format 0, Windows en-US records only) for a renamed subset: the
/// original's copyright, version, manufacturer, designer, URLs and licence strings, the new
/// family, full and PostScript names, and a description saying what was done.
fn renamed_name_table(name: &[u8], family: &str, note: &str) -> Result<Vec<u8>, String> {
    let style = name_string(name, 2).unwrap_or_else(|| "Regular".into());
    let full = if style == "Regular" {
        family.to_string()
    } else {
        format!("{family} {style}")
    };
    let ps = format!("{}-{}", family.replace(' ', ""), style.replace(' ', ""));
    let mut recs: Vec<(u16, String)> = Vec::new();
    for id in [0u16, 1, 2, 3, 4, 5, 6, 8, 9, 10, 11, 12, 13, 14] {
        let s = match id {
            1 => Some(family.to_string()),
            3 => Some(format!("{ps};subset")),
            4 => Some(full.clone()),
            6 => Some(ps.clone()),
            10 => Some(note.to_string()),
            _ => name_string(name, id),
        };
        if let Some(s) = s {
            recs.push((id, s));
        }
    }
    let n = u16::try_from(recs.len()).map_err(|_| "too many names")?;
    let mut head = Vec::new();
    let mut store: Vec<u8> = Vec::new();
    head.extend(0u16.to_be_bytes());
    head.extend(n.to_be_bytes());
    head.extend((6 + 12 * n).to_be_bytes());
    for (id, s) in &recs {
        let bytes: Vec<u8> = s.encode_utf16().flat_map(u16::to_be_bytes).collect();
        let len = u16::try_from(bytes.len()).map_err(|_| "long name")?;
        let off = u16::try_from(store.len()).map_err(|_| "long name table")?;
        for v in [3u16, 1, 0x409, *id, len, off] {
            head.extend(v.to_be_bytes());
        }
        store.extend(bytes);
    }
    head.extend(store);
    Ok(head)
}

/// `font-subset carlito SRC_DIR`.
fn carlito(src: &Path) -> Result<(), String> {
    let dir = crate::root().join("crates/render/fonts/carlito");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let wanted: BTreeSet<char> = CARLITO_RANGES
        .iter()
        .flat_map(|(a, b)| (*a..=*b).filter_map(char::from_u32))
        .filter(|c| !c.is_control())
        .collect();
    let mut doc = format!(
        "Carlito, the renderer's stand-in for Calibri (metric-compatible: the same advance widths),\n\
         used when a visual asks for Calibri and no Calibri face is installed. Glyph subsets made by\n\
         `cargo xtask font-subset carlito SRC_DIR` (xtask/src/font_subset.rs: the files, SHA-256 and\n\
         the Unicode ranges kept): glyphs renumbered; cmap of the kept characters only; post format 3;\n\
         layout (kerning) and hinting tables dropped. Licence: SIL Open Font License 1.1, with\n\
         Reserved Font Name \"Carlito\": the subsets are a Modified Version, so their name table\n\
         calls them \"{CARLITO_SUBSET_FAMILY}\" (copyright, version, designer, URL and licence\n\
         strings kept). OFL-Carlito.txt here is the upstream OFL.txt, unmodified.\n\n\
         Upstream:  {CARLITO_REPO}, ofl/carlito at commit {CARLITO_COMMIT} ({CARLITO_VERSION})\n\
         Licence:   OFL-Carlito.txt (ofl/carlito/OFL.txt at the same commit)\n"
    );
    for (name, sha, out_name) in CARLITO {
        let path = src.join(name);
        let data = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let got = sha256_hex(&data);
        if got != *sha {
            return Err(format!("{}: SHA-256 {got}, expected {sha}", path.display()));
        }
        let (out, kept, _) = subset(&data, &wanted, true)?;
        let mut t = tables(&out)?;
        let name_table = t.get(b"name").ok_or("no name table")?;
        let renamed = renamed_name_table(
            name_table,
            CARLITO_SUBSET_FAMILY,
            "A glyph subset of Carlito made by BIcraft, renamed as its Reserved Font Name requires.",
        )?;
        t.insert(*b"name", renamed);
        let out = write(&t)?;
        let kept: BTreeSet<char> = kept.into_iter().collect();
        std::fs::write(dir.join(out_name), &out).map_err(|e| format!("{out_name}: {e}"))?;
        eprintln!(
            "font-subset: {out_name} {} chars, {} bytes",
            kept.len(),
            out.len()
        );
        let _ = write!(
            doc,
            "\n{out_name}: a glyph subset of {name}.\n\
             Source:    https://raw.githubusercontent.com/google/fonts/{CARLITO_COMMIT}/ofl/carlito/{name}\n\
             \x20          {len} bytes, SHA-256 {sha}\n\
             Subset:    SHA-256 {osha} ({olen} bytes)\n\
             Characters ({n}): {chars}\n",
            len = data.len(),
            osha = sha256_hex(&out),
            olen = out.len(),
            n = kept.len(),
            chars = wrap(&ranges_text(&kept), 88, "           "),
        );
    }
    std::fs::write(dir.join("SOURCE.txt"), doc).map_err(|e| format!("SOURCE.txt: {e}"))?;
    Ok(())
}

/// `s` wrapped at spaces to `width` columns, continuation lines prefixed by `indent`.
fn wrap(s: &str, width: usize, indent: &str) -> String {
    let mut out = String::new();
    let mut line = 0usize;
    for w in s.split(' ') {
        if line > 0 && line + 1 + w.len() > width {
            out.push('\n');
            out.push_str(indent);
            line = indent.len();
        } else if line > 0 {
            out.push(' ');
            line += 1;
        }
        out.push_str(w);
        line += w.len();
    }
    out
}

fn u16_at(d: &[u8], o: usize) -> Result<u16, String> {
    d.get(o..o + 2)
        .map(|b| u16::from_be_bytes([b[0], b[1]]))
        .ok_or_else(|| format!("truncated font at {o}"))
}

fn u32_at(d: &[u8], o: usize) -> Result<u32, String> {
    d.get(o..o + 4)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| format!("truncated font at {o}"))
}

fn put_u16(d: &mut [u8], o: usize, v: u16) -> Result<(), String> {
    d.get_mut(o..o + 2)
        .ok_or_else(|| format!("truncated table at {o}"))?
        .copy_from_slice(&v.to_be_bytes());
    Ok(())
}

/// The tables of an sfnt file: tag → bytes.
fn tables(d: &[u8]) -> Result<BTreeMap<[u8; 4], Vec<u8>>, String> {
    let n = usize::from(u16_at(d, 4)?);
    let mut t = BTreeMap::new();
    for i in 0..n {
        let r = 12 + 16 * i;
        let tag: [u8; 4] = d
            .get(r..r + 4)
            .and_then(|b| b.try_into().ok())
            .ok_or("truncated table directory")?;
        let off = u32_at(d, r + 8)? as usize;
        let len = u32_at(d, r + 12)? as usize;
        let bytes = d
            .get(off..off.saturating_add(len))
            .ok_or("table outside the file")?;
        t.insert(tag, bytes.to_vec());
    }
    Ok(t)
}

/// The glyph of each character in a format 4 or 12 Unicode `cmap` subtable.
fn char_glyphs(cmap: &[u8], wanted: &BTreeSet<char>) -> Result<BTreeMap<char, u16>, String> {
    let n = usize::from(u16_at(cmap, 2)?);
    let mut best: Option<(usize, u16)> = None;
    for i in 0..n {
        let r = 4 + 8 * i;
        let (pid, eid) = (u16_at(cmap, r)?, u16_at(cmap, r + 2)?);
        let off = u32_at(cmap, r + 4)? as usize;
        let fmt = u16_at(cmap, off)?;
        let unicode = pid == 0 || (pid == 3 && (eid == 1 || eid == 10));
        if unicode && (fmt == 4 || fmt == 12) && best.is_none_or(|(_, f)| fmt > f) {
            best = Some((off, fmt));
        }
    }
    let (off, fmt) = best.ok_or("no Unicode cmap subtable")?;
    let mut out = BTreeMap::new();
    for &c in wanted {
        let cp = c as u32;
        let gid = if fmt == 12 {
            let groups = u32_at(cmap, off + 12)? as usize;
            let mut g = 0u16;
            for k in 0..groups.min(100_000) {
                let r = off + 16 + 12 * k;
                let (start, end, first) = (u32_at(cmap, r)?, u32_at(cmap, r + 4)?, u32_at(cmap, r + 8)?);
                if (start..=end).contains(&cp) {
                    g = u16::try_from(first + (cp - start)).unwrap_or(0);
                    break;
                }
            }
            g
        } else {
            if cp > 0xFFFF {
                continue;
            }
            let cp = cp as u16;
            let segs = usize::from(u16_at(cmap, off + 6)? / 2);
            let ends = off + 14;
            let starts = ends + 2 * segs + 2;
            let deltas = starts + 2 * segs;
            let ranges = deltas + 2 * segs;
            let mut g = 0u16;
            for s in 0..segs {
                let (end, start) = (u16_at(cmap, ends + 2 * s)?, u16_at(cmap, starts + 2 * s)?);
                if cp < start || cp > end {
                    continue;
                }
                let delta = u16_at(cmap, deltas + 2 * s)?;
                let ro = usize::from(u16_at(cmap, ranges + 2 * s)?);
                g = if ro == 0 {
                    cp.wrapping_add(delta)
                } else {
                    let at = ranges + 2 * s + ro + 2 * usize::from(cp - start);
                    match u16_at(cmap, at)? {
                        0 => 0,
                        v => v.wrapping_add(delta),
                    }
                };
                break;
            }
            g
        };
        if gid != 0 {
            out.insert(c, gid);
        }
    }
    Ok(out)
}

/// A format 4 subtable with one segment per BMP character.
fn cmap4(map: &BTreeMap<char, u16>) -> Result<Vec<u8>, String> {
    let segs: Vec<(u16, u16)> = map
        .iter()
        .filter_map(|(c, g)| {
            u16::try_from(*c as u32)
                .ok()
                .filter(|cp| *cp != 0xFFFF)
                .map(|cp| (cp, *g))
        })
        .collect();
    let n = segs.len() + 1; // + the final 0xFFFF segment
    let seg_x2 = u16::try_from(2 * n).map_err(|_| "too many characters")?;
    let mut search = 1u16;
    let mut sel = 0u16;
    while usize::from(search) * 2 <= n {
        search *= 2;
        sel += 1;
    }
    let search_range = search * 2;
    let len = u16::try_from(16 + 8 * n).map_err(|_| "too many characters")?;
    let mut sub = Vec::new();
    for v in [4u16, len, 0, seg_x2, search_range, sel, seg_x2 - search_range] {
        sub.extend(v.to_be_bytes());
    }
    for (cp, _) in &segs {
        sub.extend(cp.to_be_bytes());
    }
    sub.extend(0xFFFFu16.to_be_bytes());
    sub.extend(0u16.to_be_bytes()); // reservedPad
    for (cp, _) in &segs {
        sub.extend(cp.to_be_bytes());
    }
    sub.extend(0xFFFFu16.to_be_bytes());
    for (cp, g) in &segs {
        sub.extend(g.wrapping_sub(*cp).to_be_bytes());
    }
    sub.extend(1u16.to_be_bytes());
    for _ in 0..n {
        sub.extend(0u16.to_be_bytes());
    }
    Ok(sub)
}

/// A format 12 subtable with one group per character.
fn cmap12(map: &BTreeMap<char, u16>) -> Vec<u8> {
    let n = map.len() as u32;
    let mut sub = Vec::new();
    sub.extend(12u16.to_be_bytes());
    sub.extend(0u16.to_be_bytes());
    sub.extend((16 + 12 * n).to_be_bytes());
    sub.extend(0u32.to_be_bytes());
    sub.extend(n.to_be_bytes());
    for (c, g) in map {
        let cp = *c as u32;
        sub.extend(cp.to_be_bytes());
        sub.extend(cp.to_be_bytes());
        sub.extend(u32::from(*g).to_be_bytes());
    }
    sub
}

/// The `cmap` of `map`: format 4 (platform 0/3 and 3/1), plus format 12 (0/4 and 3/10) when a
/// character is outside the BMP.
fn new_cmap(map: &BTreeMap<char, u16>) -> Result<Vec<u8>, String> {
    let f4 = cmap4(map)?;
    let wide = map.keys().any(|c| *c as u32 > 0xFFFF);
    let records: Vec<(u16, u16, bool)> = if wide {
        vec![(0, 3, false), (0, 4, true), (3, 1, false), (3, 10, true)]
    } else {
        vec![(0, 3, false), (3, 1, false)]
    };
    let head = 4 + 8 * records.len();
    let f12 = wide.then(|| cmap12(map));
    let f12_at = head + f4.len();
    let mut cmap = Vec::new();
    cmap.extend(0u16.to_be_bytes());
    cmap.extend((records.len() as u16).to_be_bytes());
    for (pid, eid, twelve) in &records {
        cmap.extend(pid.to_be_bytes());
        cmap.extend(eid.to_be_bytes());
        cmap.extend((if *twelve { f12_at } else { head } as u32).to_be_bytes());
    }
    cmap.extend(f4);
    if let Some(f) = f12 {
        cmap.extend(f);
    }
    Ok(cmap)
}

/// The subset font, the characters kept and those the font lacks.
type Subset = (Vec<u8>, Vec<char>, Vec<char>);

/// Glyph ids a composite glyph refers to, with the offset of each id in the glyph's bytes.
fn components(gd: &[u8]) -> Result<Vec<(usize, u16)>, String> {
    let mut out = Vec::new();
    if gd.len() < 10 || i16::from_be_bytes([gd[0], gd[1]]) >= 0 {
        return Ok(out);
    }
    let mut o = 10;
    for _ in 0..1000 {
        let flags = u16_at(gd, o)?;
        out.push((o + 2, u16_at(gd, o + 2)?));
        o += 4 + if flags & 1 != 0 { 4 } else { 2 };
        o += match flags {
            f if f & 0x80 != 0 => 8,
            f if f & 0x40 != 0 => 4,
            f if f & 0x08 != 0 => 2,
            _ => 0,
        };
        if flags & 0x20 == 0 {
            break;
        }
    }
    Ok(out)
}

/// The subset of `d` for the characters `wanted`; `compact` renumbers the kept glyphs
/// (`.notdef` first, then in their old order) instead of leaving empty slots.
pub fn subset(d: &[u8], wanted: &BTreeSet<char>, compact: bool) -> Result<Subset, String> {
    let mut t = tables(d)?;
    let get = |t: &BTreeMap<[u8; 4], Vec<u8>>, tag: &[u8; 4]| {
        t.get(tag).cloned().ok_or_else(|| {
            format!(
                "no `{}` table (only TrueType outlines are supported)",
                String::from_utf8_lossy(tag)
            )
        })
    };
    let head = get(&t, b"head")?;
    let maxp = get(&t, b"maxp")?;
    let loca = get(&t, b"loca")?;
    let glyf = get(&t, b"glyf")?;
    let cmap = get(&t, b"cmap")?;
    let glyphs = usize::from(u16_at(&maxp, 4)?);
    let long = u16_at(&head, 50)? == 1;
    let at = |i: usize| -> Result<usize, String> {
        Ok(if long {
            u32_at(&loca, 4 * i)? as usize
        } else {
            2 * usize::from(u16_at(&loca, 2 * i)?)
        })
    };
    let glyph = |g: u16| -> Result<&[u8], String> {
        let (s, e) = (at(usize::from(g))?, at(usize::from(g) + 1)?);
        glyf.get(s..e).ok_or_else(|| "glyph outside glyf".to_string())
    };
    let mut map = char_glyphs(&cmap, wanted)?;
    map.retain(|_, g| usize::from(*g) < glyphs);
    let kept: Vec<char> = map.keys().copied().collect();
    let missing: Vec<char> = wanted.iter().copied().filter(|c| !map.contains_key(c)).collect();
    // the glyphs to keep: .notdef, the characters', and the components of composites
    let mut keep: BTreeSet<u16> = std::iter::once(0).chain(map.values().copied()).collect();
    let mut todo: Vec<u16> = keep.iter().copied().collect();
    while let Some(g) = todo.pop() {
        for (_, comp) in components(glyph(g)?)? {
            if usize::from(comp) < glyphs && keep.insert(comp) {
                todo.push(comp);
            }
        }
    }
    // old id → new id
    let renum: BTreeMap<u16, u16> = if compact {
        keep.iter().enumerate().map(|(i, g)| (*g, i as u16)).collect()
    } else {
        (0..glyphs).map(|g| (g as u16, g as u16)).collect()
    };
    let order: Vec<u16> = if compact {
        keep.iter().copied().collect()
    } else {
        (0..glyphs).map(|g| g as u16).collect()
    };
    let mut new_glyf = Vec::new();
    let mut new_loca = Vec::new();
    for g in &order {
        new_loca.extend((new_glyf.len() as u32).to_be_bytes());
        if keep.contains(g) {
            let mut gd = glyph(*g)?.to_vec();
            for (o, comp) in components(&gd.clone())? {
                put_u16(&mut gd, o, renum.get(&comp).copied().unwrap_or(0))?;
            }
            new_glyf.extend(gd);
            while new_glyf.len() % 4 != 0 {
                new_glyf.push(0);
            }
        }
    }
    new_loca.extend((new_glyf.len() as u32).to_be_bytes());
    let mut head = head;
    put_u16(&mut head, 50, 1)?;
    head.get_mut(8..12).ok_or("short head")?.copy_from_slice(&[0; 4]);
    let mut post = get(&t, b"post")?;
    post.truncate(32);
    post.get_mut(0..4)
        .ok_or("short post")?
        .copy_from_slice(&0x0003_0000u32.to_be_bytes());
    if compact {
        let n = u16::try_from(order.len()).map_err(|_| "too many glyphs")?;
        let mut hhea = get(&t, b"hhea")?;
        let hmtx = get(&t, b"hmtx")?;
        let nh = usize::from(u16_at(&hhea, 34)?).max(1);
        let mut new_hmtx = Vec::new();
        for g in &order {
            let g = usize::from(*g);
            let adv = u16_at(&hmtx, 4 * g.min(nh - 1))?;
            let lsb = if g < nh {
                u16_at(&hmtx, 4 * g + 2)?
            } else {
                u16_at(&hmtx, 4 * nh + 2 * (g - nh))?
            };
            new_hmtx.extend(adv.to_be_bytes());
            new_hmtx.extend(lsb.to_be_bytes());
        }
        put_u16(&mut hhea, 34, n)?;
        let mut maxp = maxp;
        put_u16(&mut maxp, 4, n)?;
        t.insert(*b"hhea", hhea);
        t.insert(*b"hmtx", new_hmtx);
        t.insert(*b"maxp", maxp);
        for g in map.values_mut() {
            *g = renum.get(g).copied().unwrap_or(0);
        }
    }
    let keep_tables: [&[u8; 4]; 6] = [b"OS/2", b"hhea", b"hmtx", b"maxp", b"name", b"head"];
    t.retain(|tag, _| keep_tables.contains(&tag));
    t.insert(*b"head", head);
    t.insert(*b"post", post);
    t.insert(*b"cmap", new_cmap(&map)?);
    t.insert(*b"glyf", new_glyf);
    t.insert(*b"loca", new_loca);
    let out = write(&t)?;
    Ok((out, kept, missing))
}

fn checksum(b: &[u8]) -> u32 {
    b.chunks(4).fold(0u32, |s, c| {
        let mut w = [0u8; 4];
        w[..c.len()].copy_from_slice(c);
        s.wrapping_add(u32::from_be_bytes(w))
    })
}

/// An sfnt file of `t` (tables in tag order, 4-byte aligned, `head` checksum adjustment set).
fn write(t: &BTreeMap<[u8; 4], Vec<u8>>) -> Result<Vec<u8>, String> {
    let n = u16::try_from(t.len()).map_err(|_| "too many tables")?;
    let mut p = 1u16;
    let mut sel = 0u16;
    while p * 2 <= n {
        p *= 2;
        sel += 1;
    }
    let mut out = Vec::new();
    out.extend(0x0001_0000u32.to_be_bytes());
    for v in [n, p * 16, sel, n * 16 - p * 16] {
        out.extend(v.to_be_bytes());
    }
    let mut off = 12 + 16 * t.len();
    let mut body = Vec::new();
    let mut head_at = None;
    for (tag, b) in t {
        out.extend(tag);
        out.extend(checksum(b).to_be_bytes());
        out.extend((off as u32).to_be_bytes());
        out.extend((b.len() as u32).to_be_bytes());
        if tag == b"head" {
            head_at = Some(off);
        }
        body.extend(b);
        while body.len() % 4 != 0 {
            body.push(0);
        }
        off = 12 + 16 * t.len() + body.len();
    }
    out.extend(body);
    let adj = 0xB1B0_AFBAu32.wrapping_sub(checksum(&out));
    let h = head_at.ok_or("no head table")?;
    out.get_mut(h + 8..h + 12)
        .ok_or("short head")?
        .copy_from_slice(&adj.to_be_bytes());
    Ok(out)
}

/// SHA-256 (FIPS 180-4) of `data`, lower-case hex: checks the upstream fonts and records the
/// subsets in SOURCE.txt.
pub fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend(bits.to_be_bytes());
    for block in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (i, c) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*c);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, v) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(v);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_values() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn chars_and_ranges() {
        let s = parse_chars("ab U+0394 U+2190-2192").unwrap();
        assert_eq!(ranges_text(&s), "U+0061-0062 U+0394 U+2190-2192");
        assert!(parse_chars("U+0010-0001").is_err());
    }

    #[test]
    fn compact_subset_of_the_bundled_symbol_face() {
        // the UI's symbol subset: renumbering keeps each character's outline and advance
        let root = crate::root();
        let src = std::fs::read(root.join("crates/ui/fonts/NotoSansSymbols2-subset.ttf")).unwrap();
        let wanted: BTreeSet<char> = ['✓', '●', '▲'].into_iter().collect();
        let (out, kept, missing) = subset(&src, &wanted, true).unwrap();
        assert_eq!(kept.len(), 3);
        assert!(missing.is_empty());
        assert!(out.len() < src.len() / 4, "{}", out.len());
        let t = tables(&out).unwrap();
        let map = char_glyphs(&t[b"cmap"], &wanted).unwrap();
        assert_eq!(
            map.values().copied().collect::<BTreeSet<u16>>(),
            [1u16, 2, 3].into()
        );
        assert_eq!(u16_at(&t[b"maxp"], 4).unwrap(), 4);
        // wide characters get a format 12 subtable
        let m: BTreeMap<char, u16> = [('a', 1), ('\u{1F7E2}', 2)].into_iter().collect();
        let c = new_cmap(&m).unwrap();
        let all: BTreeSet<char> = m.keys().copied().collect();
        assert_eq!(char_glyphs(&c, &all).unwrap(), m);
    }
}
