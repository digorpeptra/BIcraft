//! The app's fonts, the same in every shell: the four Selawik faces `bicraft-render` bundles
//! (SIL OFL 1.1, metric-compatible with Segoe UI, the viewer's font) as egui's proportional
//! family, with `semibold`, `bold` and `light` families, and Hack (egui's own monospace face) for
//! DAX and badges, the viewer's `ui-monospace` look. The symbols the viewer's page uses (`✓`, `✕`,
//! `✖`, `⚠`, `●`, `▶`…) come from a subset of Noto Sans Symbols 2 (SIL OFL 1.1,
//! `crates/ui/fonts/`), right after Selawik; then the renderer's bundled Noto subsets (Greek,
//! Cyrillic, currency, superscripts, arrows, math, shapes: `crates/render/fonts/SOURCE.txt`, the
//! bytes the report pages use, so nothing more to download), so a measure named `CA Δ N-1`
//! looks the same in an answer grid and on the page; other glyphs come from the faces egui
//! embeds (Ubuntu Light, Hack, Noto Emoji, the emoji icon font; listed in
//! `THIRD_PARTY_CRATES.md`): no font is read from the system, so the desktop and the browser draw
//! the same pixels.
//!
//! [`install`] runs on the first frame of [`crate::App`], whichever shell drives it.

use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily, FontId, TextStyle};

/// Font data keys.
const REGULAR: &str = "Selawik";
const SEMIBOLD: &str = "Selawik-Semibold";
const BOLD: &str = "Selawik-Bold";
const LIGHT: &str = "Selawik-Light";
/// The symbol face: a subset of Noto Sans Symbols 2 (`crates/ui/fonts/SOURCE.txt`).
const SYMBOLS: &str = "NotoSansSymbols2-subset";

/// The subset's bytes.
pub const SYMBOLS_TTF: &[u8] = include_bytes!("../fonts/NotoSansSymbols2-subset.ttf");
/// egui's embedded faces (`epaint_default_fonts`), used as fallbacks.
const FALLBACKS: [&str; 4] = ["Ubuntu-Light", "Hack", "NotoEmoji-Regular", "emoji-icon-font"];
/// The renderer's bundled Noto subsets the UI uses: Noto Sans regular (bold in the semibold and
/// bold families), then Mono (box drawing), Symbols 2, Math and Symbols. egui's complete Noto
/// Emoji covers emoji.
const NOTO_SANS: &str = "NotoSans-subset";
const NOTO_SANS_BOLD: &str = "NotoSans-Bold-subset";
const NOTO_REST: [(&str, &[u8]); 4] = [
    ("NotoSansMono-subset", bicraft_render::NOTO_SANS_MONO),
    (
        "NotoSansSymbols2-Regular-subset",
        bicraft_render::NOTO_SANS_SYMBOLS2,
    ),
    ("NotoSansMath-subset", bicraft_render::NOTO_SANS_MATH),
    ("NotoSansSymbols-subset", bicraft_render::NOTO_SANS_SYMBOLS),
];

/// The semibold family (the viewer's `font-weight: 600`–`650`: card headers, titles).
pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

/// The bold family (`font-weight: 700`).
pub fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

/// The light family (`font-weight: 300`).
pub fn light() -> FontFamily {
    FontFamily::Name("light".into())
}

/// A semibold font of `size` points.
pub fn semibold_px(size: f32) -> FontId {
    FontId::new(size, semibold())
}

/// The font definitions of the app.
pub fn definitions() -> FontDefinitions {
    let mut fd = FontDefinitions::default();
    for (key, bytes) in [
        (REGULAR, bicraft_render::SELAWIK_REGULAR),
        (SEMIBOLD, bicraft_render::SELAWIK_SEMIBOLD),
        (BOLD, bicraft_render::SELAWIK_BOLD),
        (LIGHT, bicraft_render::SELAWIK_LIGHT),
        (SYMBOLS, SYMBOLS_TTF),
        (NOTO_SANS, bicraft_render::NOTO_SANS_REGULAR),
        (NOTO_SANS_BOLD, bicraft_render::NOTO_SANS_BOLD),
    ]
    .into_iter()
    .chain(NOTO_REST)
    {
        fd.font_data
            .insert(key.to_string(), Arc::new(FontData::from_static(bytes)));
    }
    let chain = |first: &str| -> Vec<String> {
        let noto = if first == SEMIBOLD || first == BOLD {
            NOTO_SANS_BOLD
        } else {
            NOTO_SANS
        };
        [first, SYMBOLS, noto]
            .into_iter()
            .chain(NOTO_REST.map(|(k, _)| k))
            .chain(FALLBACKS)
            .map(str::to_string)
            .collect()
    };
    fd.families.insert(FontFamily::Proportional, chain(REGULAR));
    fd.families.insert(semibold(), chain(SEMIBOLD));
    fd.families.insert(bold(), chain(BOLD));
    fd.families.insert(light(), chain(LIGHT));
    fd.families.insert(
        FontFamily::Monospace,
        [
            "Hack",
            REGULAR,
            SYMBOLS,
            NOTO_SANS,
            NOTO_REST[0].0,
            NOTO_REST[1].0,
            NOTO_REST[2].0,
            NOTO_REST[3].0,
            "Ubuntu-Light",
            "NotoEmoji-Regular",
            "emoji-icon-font",
        ]
        .map(str::to_string)
        .to_vec(),
    );
    fd
}

/// Whether the app's fonts are in use on `ctx` (they apply from the pass after [`install`]).
pub fn installed(ctx: &egui::Context) -> bool {
    ctx.fonts(|f| {
        let fams = &f.definitions().families;
        [semibold(), bold(), light()].iter().all(|x| fams.contains_key(x))
    })
}

/// Installs the fonts and the text styles (headings in semibold) on `ctx`.
pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(definitions());
    ctx.all_styles_mut(|style| {
        let size = style
            .text_styles
            .get(&TextStyle::Heading)
            .map_or(18.0, |f| f.size);
        style
            .text_styles
            .insert(TextStyle::Heading, FontId::new(size, semibold()));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_family_resolves_to_embedded_faces() {
        let fd = definitions();
        for fam in [
            FontFamily::Proportional,
            FontFamily::Monospace,
            semibold(),
            bold(),
            light(),
        ] {
            let faces = fd.families.get(&fam).unwrap();
            assert!(!faces.is_empty());
            for f in faces {
                assert!(fd.font_data.contains_key(f), "{f} of {fam:?} is not embedded");
            }
        }
        assert_eq!(fd.families[&FontFamily::Proportional][0], REGULAR);
        assert_eq!(fd.families[&bold()][0], BOLD);
    }

    /// `CA Δ N-1` in an answer grid: the Δ comes from the Noto Sans subset the pages use (bold in
    /// the bold families), not from egui's Ubuntu Light; arrows and math from the Noto subsets.
    #[test]
    fn greek_arrows_and_math_come_from_the_page_faces() {
        let fd = definitions();
        let first_with = |fam: &FontFamily, c: char| -> String {
            fd.families[fam]
                .iter()
                .find(|k| {
                    skrifa::FontRef::new(&fd.font_data[*k].font)
                        .is_ok_and(|f| skrifa::MetadataProvider::charmap(&f).map(c).is_some())
                })
                .cloned()
                .unwrap_or_default()
        };
        assert_eq!(first_with(&FontFamily::Proportional, 'Δ'), NOTO_SANS);
        assert_eq!(first_with(&bold(), 'Δ'), NOTO_SANS_BOLD);
        assert_eq!(first_with(&semibold(), 'Δ'), NOTO_SANS_BOLD);
        for c in ['≥', '∑', '↗'] {
            let f = first_with(&FontFamily::Proportional, c);
            assert!(f.starts_with("NotoSans"), "{c} from {f}");
        }
    }
}
