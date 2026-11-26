//! Reading and saving the report of a PBIP project. Reports are read from PBIR (`definition/`,
//! through `bicraft_pbir`) or from the legacy `report.json` (through `bicraft_layout`), and are
//! always saved as PBIR. Saving a legacy project converts it: `definition/` is written first,
//! then `definition.pbir` is raised to a version that makes Desktop read PBIR, and only then is
//! `report.json` removed (see `docs/formats/pbip.md`, "Saving a report").

use std::path::{Path, PathBuf};

use bicraft_report::Report;
use serde_json::Value;

use crate::create::{json_text, replace_file};
use crate::{Error, Project, ReportFormat, read_capped};

/// `definition.pbir` version from which Desktop reads a PBIR `definition/` folder.
const FIRST_PBIR_VERSION: (u32, u32) = (3, 0);

/// Reads the project's report, PBIR or legacy.
pub fn read_report(project: &Project) -> Result<Report, Error> {
    let Some(dir) = &project.report else {
        return Err(Error::NoReport(project.root.clone()));
    };
    match project.report_format {
        ReportFormat::Pbir => bicraft_pbir::read_dir(dir).map_err(|e| Error::Pbir(dir.join("definition"), e)),
        ReportFormat::Legacy => {
            let path = dir.join("report.json");
            let bytes = read_capped(&path)?;
            let (text, _) = bicraft_opc::decode_text(&path.to_string_lossy(), &bytes)
                .map_err(|e| Error::Text(path.clone(), e))?;
            bicraft_layout::read_layout(&text).map_err(|e| Error::Layout(path, e))
        }
        ReportFormat::Missing => Err(Error::NoReport(dir.clone())),
    }
}

/// Saves `report` into the project's report folder as PBIR. A legacy (or empty) report folder is
/// converted: see the module documentation. Returns the report format now on disk (always
/// [`ReportFormat::Pbir`]).
pub fn save_report(project: &Project, report: &Report) -> Result<ReportFormat, Error> {
    let Some(dir) = &project.report else {
        return Err(Error::NoReport(project.root.clone()));
    };
    // Fallible preparation first: a legacy project's `definition.pbir` must be readable before
    // anything is written.
    let upgrade = match project.report_format {
        ReportFormat::Pbir => None,
        ReportFormat::Legacy | ReportFormat::Missing => Some(upgraded_definition(dir)?),
    };
    bicraft_pbir::write_dir(report, dir).map_err(|e| Error::Pbir(dir.join("definition"), e))?;
    if let Some(upgrade) = upgrade {
        let written = dir.join("definition").join("report.json");
        if !written.is_file() {
            return Err(Error::ReportNotWritten(written));
        }
        if let Some((path, text)) = upgrade {
            replace_file(&path, &text)?;
        }
        let legacy = dir.join("report.json");
        if legacy.exists() {
            std::fs::remove_file(&legacy).map_err(|e| Error::Io(legacy, e))?;
        }
    }
    Ok(ReportFormat::Pbir)
}

/// The new text of `definition.pbir` when its `version` is below 3.0 (Desktop then ignores
/// `definition/`), `None` when it already reads PBIR. The new version is always `4.0`, the first
/// version Microsoft documents for a PBIR `definition/` folder (Microsoft Learn, "Power BI Desktop
/// project report folder", `definition.pbir`; docs/formats/packaging-verification.md, P-PBIR-VER),
/// with or without a `.platform` file.
fn upgraded_definition(dir: &Path) -> Result<Option<(PathBuf, String)>, Error> {
    let path = dir.join("definition.pbir");
    let bytes = read_capped(&path)?;
    let (text, _) = bicraft_opc::decode_text(&path.to_string_lossy(), &bytes)
        .map_err(|e| Error::Text(path.clone(), e))?;
    let mut value: Value = serde_json::from_str(&text).map_err(|e| Error::Json(path.clone(), e))?;
    let Some(object) = value.as_object_mut() else {
        return Err(Error::BadDefinition(path, "not a JSON object"));
    };
    let current = object
        .get("version")
        .and_then(Value::as_str)
        .and_then(parse_version);
    if current.is_some_and(|v| v >= FIRST_PBIR_VERSION) {
        return Ok(None);
    }
    let target = "4.0";
    object.insert("version".into(), Value::String(target.into()));
    let text = json_text(&path, &value)?;
    Ok(Some((path, text)))
}

/// `major.minor` of a version string such as `1.0` or `4.0`.
fn parse_version(text: &str) -> Option<(u32, u32)> {
    let mut parts = text.trim().split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().map_or(Some(0), |m| m.parse().ok())?;
    Some((major, minor))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locate;
    use crate::tempdir::TempDir;

    fn legacy_project(name: &str, platform: bool) -> TempDir {
        let root = TempDir::new(&format!("pbip-report-{name}"));
        std::fs::create_dir_all(root.join("Shop.Report")).unwrap();
        std::fs::create_dir_all(root.join("Shop.SemanticModel")).unwrap();
        std::fs::write(root.join("Shop.pbip"), "{}").unwrap();
        std::fs::write(
            root.join("Shop.SemanticModel/model.bim"),
            r#"{"name":"Shop","model":{"tables":[]}}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("Shop.Report/definition.pbir"),
            r#"{"version": "1.0", "datasetReference": {"byPath": {"path": "../Shop.SemanticModel"}}}"#,
        )
        .unwrap();
        std::fs::write(root.join("Shop.Report/report.json"), r#"{"sections": []}"#).unwrap();
        if platform {
            std::fs::write(root.join("Shop.Report/.platform"), "{}").unwrap();
        }
        root
    }

    #[test]
    fn versions_parse() {
        assert_eq!(parse_version("1.0"), Some((1, 0)));
        assert_eq!(parse_version(" 4 "), Some((4, 0)));
        assert_eq!(parse_version("4.0.1"), Some((4, 0)));
        assert_eq!(parse_version("x"), None);
    }

    #[test]
    fn definition_upgrade_is_always_4_0() {
        // Regression (packaging-verification.md, P-PBIR-VER): 3.0 was written without a
        // `.platform` file; Microsoft documents PBIR from 4.0 only.
        for (platform, expected) in [(true, "4.0"), (false, "4.0")] {
            let root = legacy_project(&format!("upgrade-{platform}"), platform);
            let dir = root.join("Shop.Report");
            let (path, text) = upgraded_definition(&dir).unwrap().unwrap();
            assert_eq!(path, dir.join("definition.pbir"));
            let v: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(v["version"], expected);
            // Other properties are kept, in order.
            assert_eq!(v["datasetReference"]["byPath"]["path"], "../Shop.SemanticModel");
            let keys: Vec<_> = v.as_object().unwrap().keys().collect();
            assert_eq!(keys, ["version", "datasetReference"]);
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    #[test]
    fn pbir_definitions_are_left_alone() {
        let root = legacy_project("pbir-def", true);
        let dir = root.join("Shop.Report");
        std::fs::write(dir.join("definition.pbir"), r#"{"version": "4.0"}"#).unwrap();
        assert!(upgraded_definition(&dir).unwrap().is_none());
        std::fs::write(dir.join("definition.pbir"), "[1]").unwrap();
        assert!(matches!(upgraded_definition(&dir), Err(Error::BadDefinition(..))));
        std::fs::remove_file(dir.join("definition.pbir")).unwrap();
        assert!(matches!(upgraded_definition(&dir), Err(Error::Io(..))));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_reports_are_explained() {
        let root = legacy_project("missing", false);
        let mut project = locate(&root.join("Shop.pbip")).unwrap();
        assert_eq!(project.report_format, ReportFormat::Legacy);
        project.report = None;
        assert!(matches!(read_report(&project), Err(Error::NoReport(_))));
        assert!(matches!(
            save_report(&project, &Report::default()),
            Err(Error::NoReport(_))
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The legacy `report.json` is only removed once `definition/` was really written.
    #[test]
    fn a_legacy_report_is_kept_when_nothing_was_written() {
        let root = legacy_project("kept", true);
        let project = locate(&root.join("Shop.pbip")).unwrap();
        let empty_writer = !bicraft_pbir::write_documents(&Report::default())
            .iter()
            .any(|(p, _)| p == "definition/report.json");
        if empty_writer {
            assert!(matches!(
                save_report(&project, &Report::default()),
                Err(Error::ReportNotWritten(_))
            ));
            assert!(root.join("Shop.Report/report.json").is_file());
            let def = std::fs::read_to_string(root.join("Shop.Report/definition.pbir")).unwrap();
            assert!(def.contains("\"1.0\""), "{def}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn saving_a_legacy_project_converts_it_to_pbir() {
        let root = legacy_project("convert", true);
        let project = locate(&root.join("Shop.pbip")).unwrap();
        let report = Report {
            pages: vec![bicraft_report::Page {
                name: "abc".into(),
                display_name: "Overview".into(),
                width: 1280.0,
                height: 720.0,
                ..Default::default()
            }],
            active_page: Some("abc".into()),
            ..Default::default()
        };
        assert_eq!(save_report(&project, &report).unwrap(), ReportFormat::Pbir);
        assert!(!root.join("Shop.Report/report.json").exists());
        let def: Value =
            serde_json::from_str(&std::fs::read_to_string(root.join("Shop.Report/definition.pbir")).unwrap())
                .unwrap();
        assert_eq!(def["version"], "4.0");
        let project = locate(&root.join("Shop.pbip")).unwrap();
        assert_eq!(project.report_format, ReportFormat::Pbir);
        let back = read_report(&project).unwrap();
        assert_eq!(back.pages.len(), 1);
        assert_eq!(back.pages[0].display_name, "Overview");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reads_a_legacy_report() {
        let root = legacy_project("legacy-read", true);
        let project = locate(&root.join("Shop.pbip")).unwrap();
        let report = read_report(&project).unwrap();
        assert!(report.pages.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
