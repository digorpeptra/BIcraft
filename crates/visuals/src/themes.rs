//! BIcraft's own built-in report themes (View › Themes gallery, `theme.set {builtin}`).
//!
//! Original work of the BIcraft contributors (ATTRIBUTION.md): theme JSON files under `themes/`
//! that follow the public report theme schema ([`crate::THEME_SCHEMA`]). They are not copies of
//! Power BI's themes, whose files are Microsoft assets (AGENTS.md §2). Every theme is checked for
//! contrast: text colours read at 4.5:1 on the theme's background (7:1 for High contrast) and the
//! data, sentiment and divergent colours at 3:1 (WCAG 2.1, 1.4.3 and 1.4.11).

/// One built-in theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinTheme {
    /// The stable id (`theme.set {builtin: id}`).
    pub id: &'static str,
    /// The theme JSON.
    pub json: &'static str,
}

/// The built-in themes, in the gallery's order.
pub const BUILTIN_THEMES: &[BuiltinTheme] = &[
    BuiltinTheme {
        id: "bicraft",
        json: include_str!("../themes/bicraft.json"),
    },
    BuiltinTheme {
        id: "harbor",
        json: include_str!("../themes/harbor.json"),
    },
    BuiltinTheme {
        id: "orchard",
        json: include_str!("../themes/orchard.json"),
    },
    BuiltinTheme {
        id: "ember",
        json: include_str!("../themes/ember.json"),
    },
    BuiltinTheme {
        id: "slate",
        json: include_str!("../themes/slate.json"),
    },
    BuiltinTheme {
        id: "clear-sight",
        json: include_str!("../themes/clear-sight.json"),
    },
    BuiltinTheme {
        id: "dusk",
        json: include_str!("../themes/dusk.json"),
    },
    BuiltinTheme {
        id: "high-contrast",
        json: include_str!("../themes/high-contrast.json"),
    },
];

/// The built-in theme `id`.
pub fn builtin_theme(id: &str) -> Option<&'static BuiltinTheme> {
    BUILTIN_THEMES.iter().find(|t| t.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn lum(hex: &str) -> f64 {
        let h = hex.trim_start_matches('#');
        let ch = |i: usize| {
            let v = f64::from(u8::from_str_radix(&h[i..i + 2], 16).unwrap()) / 255.0;
            if v <= 0.039_28 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(0) + 0.7152 * ch(2) + 0.0722 * ch(4)
    }

    fn contrast(a: &str, b: &str) -> f64 {
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// Every theme parses, is named as BIcraft's, and meets its contrast.
    #[test]
    fn themes_are_ours_and_accessible() {
        assert_eq!(BUILTIN_THEMES.len(), 8);
        for t in BUILTIN_THEMES {
            let v: Value = serde_json::from_str(t.json).unwrap();
            let name = v["name"].as_str().unwrap();
            assert!(name.starts_with("BIcraft"), "{}: {name}", t.id);
            let bg = v["background"].as_str().unwrap();
            let text_min = if t.id == "high-contrast" { 7.0 } else { 4.5 };
            for key in ["foreground", "foregroundNeutralSecondary", "hyperlink"] {
                let c = contrast(v[key].as_str().unwrap(), bg);
                assert!(c >= text_min, "{}: {key} is {c:.2}:1", t.id);
            }
            for (k, class) in v["textClasses"].as_object().unwrap() {
                let c = contrast(class["color"].as_str().unwrap(), bg);
                assert!(c >= text_min, "{}: textClasses.{k} is {c:.2}:1", t.id);
            }
            let colors = v["dataColors"].as_array().unwrap();
            assert!(colors.len() >= 8, "{}", t.id);
            for key in [
                "tableAccent",
                "good",
                "neutral",
                "bad",
                "maximum",
                "center",
                "minimum",
            ] {
                let c = contrast(v[key].as_str().unwrap(), bg);
                assert!(c >= 3.0, "{}: {key} is {c:.2}:1", t.id);
            }
            for c in colors {
                let c = c.as_str().unwrap();
                let r = contrast(c, bg);
                assert!(r >= 3.0, "{}: data colour {c} is {r:.2}:1", t.id);
            }
            assert_eq!(builtin_theme(t.id), Some(t));
        }
        assert_eq!(builtin_theme("Default"), None);
    }
}
