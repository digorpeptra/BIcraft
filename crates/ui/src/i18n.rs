//! The app's own words in English and French. The text inside visuals (auto titles, "(Blank)",
//! "Total", slicer words) comes from the snapshot in the session's UI language
//! (`docs/view-snapshot.md`, "UI language"), as in the viewer; this is only the chrome.

/// The chrome's language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Fr,
}

impl Lang {
    /// French for `fr`, `fr-FR`, `fr_CA`…, English otherwise.
    pub fn from_tag(tag: &str) -> Lang {
        let primary = tag.split(['-', '_']).next().unwrap_or("");
        if primary.eq_ignore_ascii_case("fr") {
            Lang::Fr
        } else {
            Lang::En
        }
    }

    /// The phrase in this language.
    pub fn t(self, en: &'static str, fr: &'static str) -> &'static str {
        match self {
            Lang::En => en,
            Lang::Fr => fr,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags() {
        assert_eq!(Lang::from_tag("fr-FR"), Lang::Fr);
        assert_eq!(Lang::from_tag("FR_ca"), Lang::Fr);
        assert_eq!(Lang::from_tag("en-US"), Lang::En);
        assert_eq!(Lang::from_tag(""), Lang::En);
        assert_eq!(Lang::Fr.t("Open", "Ouvrir"), "Ouvrir");
    }
}
