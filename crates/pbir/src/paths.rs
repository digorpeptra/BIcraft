//! Document paths: classification of report-root-relative paths and the rules for names that
//! become file or folder names.

use std::collections::HashSet;

use bicraft_report::Report;

use crate::Error;

pub(crate) const VERSION: &str = "definition/version.json";
pub(crate) const REPORT: &str = "definition/report.json";
pub(crate) const PAGES: &str = "definition/pages/pages.json";
pub(crate) const BOOKMARKS: &str = "definition/bookmarks/bookmarks.json";
pub(crate) const BOOKMARK_SUFFIX: &str = ".bookmark.json";

/// Longest name accepted for one path component (bytes).
const MAX_COMPONENT: usize = 255;

/// What a report-root-relative path holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DocKind {
    Version,
    Report,
    Pages,
    Page {
        page: String,
    },
    Visual {
        page: String,
        visual: String,
    },
    /// `pages/<p>/visuals/<v>/mobile.json`: kept raw.
    Mobile {
        page: String,
        visual: String,
    },
    /// `bookmarks/bookmarks.json`: kept raw.
    Bookmarks,
    Bookmark {
        stem: String,
    },
    /// `reportExtensions.json`: kept raw.
    ReportExtensions,
    /// Anything else under `definition/`: kept raw.
    OtherDefinition,
    /// `StaticResources/…` or `CustomVisuals/…`.
    Resource,
    /// Outside the report content (`definition.pbir`, `.platform`, `.pbi/…`, …).
    Ignored,
}

impl DocKind {
    /// The `report.schemas` key and the public schema name for JSON documents.
    pub(crate) fn schema_kind(&self) -> Option<(&'static str, &'static str)> {
        Some(match self {
            DocKind::Version => ("version", "versionMetadata"),
            DocKind::Report => ("report", "report"),
            DocKind::Pages => ("pages", "pagesMetadata"),
            DocKind::Page { .. } => ("page", "page"),
            DocKind::Visual { .. } => ("visual", "visualContainer"),
            DocKind::Mobile { .. } => ("mobile", "visualContainerMobileState"),
            DocKind::Bookmarks => ("bookmarks", "bookmarksMetadata"),
            DocKind::Bookmark { .. } => ("bookmark", "bookmark"),
            DocKind::ReportExtensions => ("reportExtensions", "reportExtension"),
            _ => return None,
        })
    }
}

/// Normalizes separators (`\` → `/`).
pub(crate) fn normalize(path: &str) -> String {
    path.replace('\\', "/")
}

/// Classifies a normalized path. Paths inside the report content must consist of valid names.
pub(crate) fn classify(path: &str) -> Result<DocKind, Error> {
    let parts: Vec<&str> = path.split('/').collect();
    let Some(&top) = parts.first() else {
        return Ok(DocKind::Ignored);
    };
    if !matches!(top, "definition" | "StaticResources" | "CustomVisuals") || parts.len() < 2 {
        return Ok(DocKind::Ignored);
    }
    for part in &parts {
        check_name(part).map_err(|message| Error::InvalidPath {
            path: path.to_string(),
            message,
        })?;
    }
    if top != "definition" {
        return Ok(DocKind::Resource);
    }
    let kind = match parts.get(1..).unwrap_or_default() {
        ["version.json"] => DocKind::Version,
        ["report.json"] => DocKind::Report,
        ["reportExtensions.json"] => DocKind::ReportExtensions,
        ["pages", "pages.json"] => DocKind::Pages,
        ["pages", page, "page.json"] => DocKind::Page {
            page: page.to_string(),
        },
        ["pages", page, "visuals", visual, "visual.json"] => DocKind::Visual {
            page: page.to_string(),
            visual: visual.to_string(),
        },
        ["pages", page, "visuals", visual, "mobile.json"] => DocKind::Mobile {
            page: page.to_string(),
            visual: visual.to_string(),
        },
        ["bookmarks", "bookmarks.json"] => DocKind::Bookmarks,
        ["bookmarks", file] if file.len() > BOOKMARK_SUFFIX.len() && file.ends_with(BOOKMARK_SUFFIX) => {
            DocKind::Bookmark {
                stem: file
                    .get(..file.len() - BOOKMARK_SUFFIX.len())
                    .unwrap_or_default()
                    .to_string(),
            }
        }
        _ => DocKind::OtherDefinition,
    };
    Ok(kind)
}

/// A name usable as a file or folder name on Windows and Linux.
pub(crate) fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("empty name".into());
    }
    if name.len() > MAX_COMPONENT {
        return Err(format!("name longer than {MAX_COMPONENT} bytes"));
    }
    if name == "." || name == ".." {
        return Err(format!("`{name}` is not a valid name"));
    }
    if let Some(c) = name
        .chars()
        .find(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control())
    {
        return Err(format!("name contains the character {c:?}"));
    }
    if name.ends_with(' ') || name.ends_with('.') || name.starts_with(' ') {
        return Err("name starts with a space or ends with a space or a dot".into());
    }
    let stem = name.split('.').next().unwrap_or_default().to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes().get(3).is_some_and(|b| (b'1'..=b'9').contains(b)));
    if reserved {
        return Err(format!("`{name}` is a reserved device name on Windows"));
    }
    Ok(())
}

/// A page, visual or bookmark name Power BI Desktop loads: one or more word characters
/// (letters, digits, underscores) or hyphens. Desktop ignores a page, visual or bookmark whose
/// name or folder name does not follow this rule (Microsoft Learn, "Power BI Desktop project
/// report folder", PBIR naming convention; docs/formats/packaging-verification.md, fact P-NAME).
pub(crate) fn check_object_name(name: &str) -> Result<(), String> {
    check_name(name)?;
    match name
        .chars()
        .find(|c| !(c.is_alphanumeric() || *c == '_' || *c == '-'))
    {
        Some(c) => Err(format!(
            "name contains {c:?}: Power BI Desktop only loads names of letters, digits, underscores and hyphens"
        )),
        None => Ok(()),
    }
}

/// A resource path the writer may emit: valid names, under the report content folders, and not
/// one of the documents the writer generates itself.
pub(crate) fn resource_path_ok(path: &str) -> bool {
    matches!(
        classify(path),
        Ok(DocKind::Resource
            | DocKind::OtherDefinition
            | DocKind::ReportExtensions
            | DocKind::Bookmarks
            | DocKind::Mobile { .. })
    )
}

pub(crate) fn check_report_names(report: &Report) -> Result<(), Error> {
    let bad = |path: String, message: String| Error::InvalidPath { path, message };
    let mut pages = HashSet::new();
    for page in &report.pages {
        check_object_name(&page.name).map_err(|m| bad(format!("page `{}`", page.name), m))?;
        if !pages.insert(page.name.to_lowercase()) {
            return Err(bad(format!("page `{}`", page.name), "duplicate page name".into()));
        }
        let mut visuals = HashSet::new();
        for visual in &page.visuals {
            let what = format!("visual `{}` on page `{}`", visual.name, page.name);
            check_object_name(&visual.name).map_err(|m| bad(what.clone(), m))?;
            if !visuals.insert(visual.name.to_lowercase()) {
                return Err(bad(what, "duplicate visual name".into()));
            }
        }
    }
    let mut bookmarks = HashSet::new();
    for bookmark in &report.bookmarks {
        let what = format!("bookmark `{}`", bookmark.name);
        check_name(&format!("{}{BOOKMARK_SUFFIX}", bookmark.name))
            .and_then(|()| check_object_name(&bookmark.name))
            .map_err(|m| bad(what.clone(), m))?;
        if !bookmarks.insert(bookmark.name.to_lowercase()) {
            return Err(bad(what, "duplicate bookmark name".into()));
        }
    }
    let mut resources = HashSet::new();
    for resource in &report.resources {
        let path = normalize(&resource.path);
        if !resource_path_ok(&path) {
            return Err(bad(
                resource.path.clone(),
                "resource paths must be valid names under StaticResources/, CustomVisuals/ or definition/ \
                 and must not be a page, visual or bookmark document"
                    .into(),
            ));
        }
        if !resources.insert(path.to_lowercase()) {
            return Err(bad(resource.path.clone(), "duplicate resource path".into()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(check_name("ReportSection1").is_ok());
        assert!(check_name("0a1b2c3d4e5f6a7b8c9d").is_ok());
        for bad in [
            "",
            ".",
            "..",
            "a/b",
            "a\\b",
            "a:b",
            "x?",
            "trail.",
            " lead",
            "CON",
            "com1.json",
            "a\u{1}",
        ] {
            assert!(check_name(bad).is_err(), "{bad:?}");
        }
        assert!(check_name("COM0").is_ok());
        assert!(check_name("console").is_ok());
    }

    /// Regression (packaging-verification.md, P-NAME): Desktop ignores PBIR objects whose names
    /// are not word characters or hyphens, so the writer refuses them.
    #[test]
    fn object_names_follow_the_pbir_convention() {
        for ok in ["ReportSection1", "0a1b2c3d4e5f6a7b8c9d", "pg-1_A", "Seite\u{e4}"] {
            assert!(check_object_name(ok).is_ok(), "{ok:?}");
        }
        for bad in ["pg.1", "pg 1", "a+b", "x@y", ""] {
            assert!(check_object_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn classification() {
        assert_eq!(classify("definition/report.json").ok(), Some(DocKind::Report));
        assert_eq!(
            classify("definition/pages/p/visuals/v/visual.json").ok(),
            Some(DocKind::Visual {
                page: "p".into(),
                visual: "v".into()
            })
        );
        assert_eq!(
            classify("definition/bookmarks/B1.bookmark.json").ok(),
            Some(DocKind::Bookmark { stem: "B1".into() })
        );
        assert_eq!(classify("definition.pbir").ok(), Some(DocKind::Ignored));
        assert_eq!(classify(".pbi/localSettings.json").ok(), Some(DocKind::Ignored));
        assert!(classify("definition/pages/../x/page.json").is_err());
        assert!(classify("StaticResources//x.png").is_err());
    }
}
