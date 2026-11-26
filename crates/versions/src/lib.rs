//! Every versioned format identifier bicraft writes, in one place.
//!
//! The writers (`bicraft-pbip`, `bicraft-pbir`, `bicraft-pbixwriter`, `bicraft-pbix`,
//! `bicraft-engine`, `bicraft-visuals`) take their `$schema` URLs, file `version` values and the
//! new-model compatibility level from here, and `cargo xtask schema-watch` compares the same table
//! against the schemas Microsoft publishes in <https://github.com/microsoft/json-schemas>
//! (`fabric/…`). See docs/schema-watch.md.
//!
//! Choices and their reasons are in docs/formats/pbip.md ("Versions chosen, and why") and
//! docs/formats/pbix-package.md (§4 and §6).

/// Prefix of every Fabric / Power BI `$schema` URL.
pub const SCHEMA_BASE: &str = "https://developer.microsoft.com/json-schemas/fabric/";

/// A public JSON schema: `{SCHEMA_BASE}{key}/{version}/schema.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Schema {
    /// Folder under `fabric/`, e.g. `item/report/definition/page`.
    pub key: &'static str,
    /// `major.minor.patch`.
    pub version: &'static str,
    /// The files that declare it (for reports).
    pub files: &'static str,
}

impl Schema {
    /// The `$schema` URL.
    pub fn url(&self) -> String {
        format!("{SCHEMA_BASE}{}/{}/schema.json", self.key, self.version)
    }

    /// Last segment of the key (the PBIR schema name, e.g. `page`).
    pub fn name(&self) -> &'static str {
        self.key.rsplit('/').next().unwrap_or(self.key)
    }
}

const fn schema(key: &'static str, version: &'static str, files: &'static str) -> Schema {
    Schema { key, version, files }
}

// --- Schemas bicraft writes ----------------------------------------------------------------

/// `<Name>.pbip`.
pub const PBIP: Schema = schema("pbip/pbipProperties", "1.0.0", "<Name>.pbip");
/// `definition.pbism`.
pub const PBISM: Schema = schema(
    "item/semanticModel/definitionProperties",
    "1.0.0",
    "<Name>.SemanticModel/definition.pbism",
);
/// `definition.pbir`.
pub const PBIR: Schema = schema(
    "item/report/definitionProperties",
    "2.0.0",
    "<Name>.Report/definition.pbir",
);
/// `.platform` (paired with [`PLATFORM_CONFIG_VERSION`]).
pub const PLATFORM: Schema = schema("gitIntegration/platformProperties", "2.0.0", ".platform");
/// PBIR `definition/version.json`.
pub const VERSION_METADATA: Schema = schema(
    "item/report/definition/versionMetadata",
    "1.0.0",
    "definition/version.json",
);
/// PBIR `definition/report.json`.
pub const REPORT: Schema = schema("item/report/definition/report", "3.3.0", "definition/report.json");
/// PBIR `definition/pages/pages.json`.
pub const PAGES_METADATA: Schema = schema(
    "item/report/definition/pagesMetadata",
    "1.1.0",
    "definition/pages/pages.json",
);
/// PBIR `page.json`.
pub const PAGE: Schema = schema("item/report/definition/page", "2.1.0", "pages/<page>/page.json");
/// PBIR `visual.json`.
pub const VISUAL_CONTAINER: Schema = schema(
    "item/report/definition/visualContainer",
    "2.12.0",
    "visuals/<visual>/visual.json",
);
/// PBIR `mobile.json` (phone layout of a visual).
pub const VISUAL_MOBILE_STATE: Schema = schema(
    "item/report/definition/visualContainerMobileState",
    "2.7.0",
    "visuals/<visual>/mobile.json",
);
/// PBIR `bookmarks/bookmarks.json`.
pub const BOOKMARKS_METADATA: Schema = schema(
    "item/report/definition/bookmarksMetadata",
    "1.0.0",
    "definition/bookmarks/bookmarks.json",
);
/// PBIR `<bookmark>.bookmark.json`.
pub const BOOKMARK: Schema = schema(
    "item/report/definition/bookmark",
    "2.1.0",
    "bookmarks/<bookmark>.bookmark.json",
);
/// PBIR `definition/reportExtensions.json` (report measures).
pub const REPORT_EXTENSION: Schema = schema(
    "item/report/definition/reportExtension",
    "1.0.0",
    "definition/reportExtensions.json",
);

/// Every schema bicraft writes a `$schema` URL for, with the version it writes for new files.
pub const WRITTEN: &[Schema] = &[
    PBIP,
    PBISM,
    PBIR,
    PLATFORM,
    VERSION_METADATA,
    REPORT,
    PAGES_METADATA,
    PAGE,
    VISUAL_CONTAINER,
    VISUAL_MOBILE_STATE,
    BOOKMARKS_METADATA,
    BOOKMARK,
    REPORT_EXTENSION,
];

/// Power BI schemas bicraft knows about but does not write: local state it never creates,
/// containers it copies verbatim, or PBIR sub-schemas referenced from the written ones (those are
/// also vendored under `crates/pbir/schemas/`). The version is the newest one seen at the last
/// review; `schema-watch` reports newer ones for information only.
pub const KNOWN: &[Schema] = &[
    schema(
        "item/report/localSettings",
        "1.0.0",
        "<Name>.Report/.pbi/localSettings.json",
    ),
    schema(
        "item/semanticModel/localSettings",
        "1.1.0",
        "<Name>.SemanticModel/.pbi/localSettings.json",
    ),
    schema(
        "item/semanticModel/editorSettings",
        "1.0.0",
        "<Name>.SemanticModel/.pbi/editorSettings.json",
    ),
    schema(
        "item/semanticModel/unappliedChanges",
        "1.0.0",
        "<Name>.SemanticModel/.pbi/unappliedChanges.json",
    ),
    schema(
        "item/semanticModel/modelReference",
        "2.0.0",
        "modelReference.json",
    ),
    schema(
        "item/semanticModel/copilot/settings",
        "1.0.0",
        "Copilot/settings.json (copied verbatim)",
    ),
    schema(
        "item/semanticModel/copilot/schema",
        "1.0.0",
        "Copilot/schema.json (copied verbatim)",
    ),
    schema(
        "item/semanticModel/copilot/examplePrompts",
        "1.0.0",
        "Copilot/examplePrompts.json (copied verbatim)",
    ),
    schema(
        "item/version",
        "1.0.0",
        "Copilot/VerifiedAnswers version.json (copied verbatim)",
    ),
    schema(
        "gitIntegration/schedules",
        "1.0.0",
        "Fabric git schedules (not used)",
    ),
    schema(
        "item/report/definition/semanticQuery",
        "1.4.0",
        "referenced from visual.json (vendored)",
    ),
    schema(
        "item/report/definition/filterConfiguration",
        "1.3.0",
        "referenced from report/page/visual (vendored)",
    ),
    schema(
        "item/report/definition/formattingObjectDefinitions",
        "1.5.0",
        "referenced from report/page/visual (vendored)",
    ),
    schema(
        "item/report/definition/visualConfiguration",
        "2.7.0",
        "referenced from visual.json (vendored)",
    ),
];

/// Upstream folders whose new schemas concern bicraft: a schema appearing there that is neither
/// in [`WRITTEN`] nor in [`KNOWN`] needs a review (a new Power BI project file type).
pub const WATCHED_PREFIXES: &[&str] = &[
    "pbip/",
    "gitIntegration/",
    "item/report/",
    "item/semanticModel/",
    "item/version",
];

// --- File versions (not published as schemas; see docs/formats/pbix-package.md) --------------

/// `.pbip` `version`: the only supported value.
pub const PBIP_VERSION: &str = "1.0";
/// `definition.pbism` `version`: ≥ 3.0 for a TMDL `definition/` folder; 4.2 is what Desktop
/// 2.157/2.158 writes for a new project.
pub const PBISM_VERSION: &str = "4.2";
/// `definition.pbism` `version` from which Desktop reads the `Copilot/` folder (pbix-package.md
/// §6).
pub const PBISM_COPILOT_VERSION: &str = "6.0";
/// `definition.pbir` `version`: ≥ 3.0 for a PBIR `definition/` folder; 4.0 adds `.platform`.
pub const PBIR_VERSION: &str = "4.0";
/// `.platform` `config.version`, paired with platformProperties 2.x.
pub const PLATFORM_CONFIG_VERSION: &str = "2.0";
/// PBIR `definition/version.json` `version`.
pub const PBIR_DEFINITION_VERSION: &str = "2.0.0";
/// Compatibility level of new models (TMSL / TMDL / ABF): what Desktop 2.158 gives a new model.
pub const NEW_MODEL_COMPATIBILITY_LEVEL: u32 = 1606;
/// PBIX/PBIT `/Version` part: `1.<minor>`, the baseline minor Desktop writes.
pub const PBIX_VERSION_BASELINE_MINOR: u32 = 28;
/// `/Version` minor when the report uses an external dataset.
pub const PBIX_VERSION_EXTERNAL_DATASET_MINOR: u32 = 29;
/// `/Version` minor when the report is PBIR (`Report/definition/`).
pub const PBIX_VERSION_PBIR_MINOR: u32 = 32;
/// `/Version` minor when verified answers are present.
pub const PBIX_VERSION_VERIFIED_ANSWERS_MINOR: u32 = 34;
/// `/Version` minor when Copilot content is present (also Desktop 2.158's maximum).
pub const PBIX_VERSION_COPILOT_MINOR: u32 = 36;
/// Power Query (Mashup) package version written in `Settings` `QueriesSettings.Version`.
pub const MASHUP_PACKAGE_VERSION: &str = "2.130.0.0";
/// Report theme JSON schema vendored in `crates/visuals/schemas/` (published in
/// microsoft/powerbi-desktop-samples, "Report Theme JSON Schema").
pub const REPORT_THEME_SCHEMA_VERSION: &str = "2.157";
/// The Power BI Desktop release the formats were last checked against.
pub const DESKTOP_BASELINE: &str = "2.158.1304.0";

/// A version that is not a published JSON schema, for reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileVersion {
    pub what: &'static str,
    pub value: &'static str,
}

/// The versions above, as text (for `schema-watch` and docs).
pub const FILE_VERSIONS: &[FileVersion] = &[
    FileVersion {
        what: ".pbip version",
        value: PBIP_VERSION,
    },
    FileVersion {
        what: "definition.pbism version",
        value: PBISM_VERSION,
    },
    FileVersion {
        what: "definition.pbism version with Copilot/",
        value: PBISM_COPILOT_VERSION,
    },
    FileVersion {
        what: "definition.pbir version",
        value: PBIR_VERSION,
    },
    FileVersion {
        what: ".platform config.version",
        value: PLATFORM_CONFIG_VERSION,
    },
    FileVersion {
        what: "PBIR version.json version",
        value: PBIR_DEFINITION_VERSION,
    },
    FileVersion {
        what: "new model compatibilityLevel (TMSL/TMDL/ABF)",
        value: "1606",
    },
    FileVersion {
        what: "PBIX /Version (baseline .. Copilot)",
        value: "1.28 .. 1.36",
    },
    FileVersion {
        what: "PBIX Settings QueriesSettings.Version (Mashup)",
        value: MASHUP_PACKAGE_VERSION,
    },
    FileVersion {
        what: "report theme JSON schema",
        value: REPORT_THEME_SCHEMA_VERSION,
    },
    FileVersion {
        what: "Power BI Desktop baseline",
        value: DESKTOP_BASELINE,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    fn triple(v: &str) -> Option<(u32, u32, u32)> {
        let mut it = v.split('.').map(str::parse::<u32>);
        let t = (it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?);
        it.next().is_none().then_some(t)
    }

    #[test]
    fn urls_and_names() {
        assert_eq!(
            PAGE.url(),
            "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/page/2.1.0/schema.json"
        );
        assert_eq!(PAGE.name(), "page");
        assert_eq!(PBIP.name(), "pbipProperties");
    }

    #[test]
    fn table_is_well_formed() {
        let mut keys = std::collections::HashSet::new();
        for s in WRITTEN.iter().chain(KNOWN) {
            assert!(triple(s.version).is_some(), "{s:?}");
            assert!(!s.key.starts_with('/') && !s.key.ends_with('/'), "{s:?}");
            assert!(keys.insert(s.key), "duplicate {}", s.key);
            assert!(
                WATCHED_PREFIXES.iter().any(|p| s.key.starts_with(p)),
                "{} is outside the watched folders",
                s.key
            );
        }
        assert_eq!(
            FILE_VERSIONS
                .iter()
                .find(|v| v.what.contains("compatibilityLevel"))
                .map(|v| v.value),
            Some(NEW_MODEL_COMPATIBILITY_LEVEL.to_string().as_str())
        );
        const { assert!(PBIX_VERSION_BASELINE_MINOR < PBIX_VERSION_COPILOT_MINOR) };
    }
}
