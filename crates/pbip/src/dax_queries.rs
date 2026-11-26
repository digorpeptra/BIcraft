//! The DAX query view's tabs in a project (`docs/formats/dax-queries.md`): one
//! `<Name>.SemanticModel/DAXQueries/<tab name>.dax` file per tab, and
//! `DAXQueries/.pbi/daxQueries.json` with the tab order and the tab shown
//! (`{"version": "1.0.0", "tabOrder": [names], "defaultTab": name}`).
//!
//! A query's text is written as it is (UTF-8 without BOM, its own line endings), so the files are
//! the same bytes on Windows and Linux. Other members of `daxQueries.json` keep their values.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::create::replace_file;
use crate::{Error, Project, read_capped};

/// The folder, relative to the semantic model folder.
pub const DAX_QUERIES: &str = "DAXQueries";
/// The settings file, relative to [`DAX_QUERIES`].
pub const DAX_QUERIES_JSON: &str = ".pbi/daxQueries.json";
/// The `version` written in a new settings file (the value Desktop-saved projects hold).
const SETTINGS_VERSION: &str = "1.0.0";
/// Most tabs read or written.
pub const MAX_QUERY_TABS: usize = 1000;
/// Largest query file read, and all of them together.
pub const MAX_QUERY_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ALL_QUERY_BYTES: u64 = 64 * 1024 * 1024;
/// Longest tab name (it becomes a file name).
pub const MAX_QUERY_NAME: usize = 200;

/// One tab: its name (the file name without `.dax`) and its text.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DaxQuery {
    pub name: String,
    pub text: String,
}

/// The tabs in order and the one shown.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DaxQueries {
    pub tabs: Vec<DaxQuery>,
    pub default_tab: Option<String>,
}

/// Why `name` cannot be a tab name: it becomes a file name, so it must be one on Windows and
/// Linux.
pub fn check_query_name(name: &str) -> Result<(), &'static str> {
    const RESERVED: &[&str] = &["CON", "PRN", "AUX", "NUL"];
    let upper = name.to_ascii_uppercase();
    let stem = upper.split('.').next().unwrap_or("");
    let reserved = RESERVED.contains(&stem)
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.ends_with(|c: char| c.is_ascii_digit()));
    if name.trim().is_empty() {
        return Err("a query tab needs a name");
    }
    if name.chars().count() > MAX_QUERY_NAME {
        return Err("a query tab name is at most 200 characters");
    }
    if name.ends_with(['.', ' '])
        || name.starts_with([' ', '.'])
        || name
            .chars()
            .any(|c| c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
        || reserved
    {
        return Err(
            "a query tab name becomes a file name: no < > : \" / \\ | ? *, no leading or trailing dot or \
             space, not a reserved name (CON, NUL, COM1…)",
        );
    }
    Ok(())
}

fn folder(project: &Project) -> Option<PathBuf> {
    project.semantic_model.as_ref().map(|d| d.join(DAX_QUERIES))
}

/// The `.dax` files directly in the folder: (tab name, path), sorted by name.
fn dax_files(dir: &Path) -> Result<Vec<(String, PathBuf)>, Error> {
    let mut out = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|e| Error::Io(dir.into(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::Io(dir.into(), e))?;
        let path = entry.path();
        let kind = entry.file_type().map_err(|e| Error::Io(path.clone(), e))?;
        if !kind.is_file() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(stem) = name
            .len()
            .checked_sub(4)
            .filter(|&n| name.get(n..).is_some_and(|e| e.eq_ignore_ascii_case(".dax")))
            .and_then(|n| name.get(..n))
        else {
            continue;
        };
        if stem.is_empty() {
            continue;
        }
        if out.len() >= MAX_QUERY_TABS {
            return Err(Error::BadDefinition(dir.into(), "more than 1000 DAX query files"));
        }
        out.push((stem.to_string(), path));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// The settings file as a JSON object (`None`: none, or not an object).
fn read_settings(dir: &Path) -> Result<Option<Map<String, Value>>, Error> {
    let path = dir.join(DAX_QUERIES_JSON);
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = read_capped(&path)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_QUERY_BYTES {
        return Err(Error::TooLarge(path));
    }
    let text = bicraft_opc::decode_text(&path.to_string_lossy(), &bytes)
        .map_err(|e| Error::Text(path.clone(), e))?
        .0;
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(m)) => Ok(Some(m)),
        Ok(_) => Ok(None),
        Err(e) => Err(Error::Json(path, e)),
    }
}

/// The project's DAX query tabs, or `None` when it has no `DAXQueries` folder. Tabs follow
/// `tabOrder`; files it does not name come after, by name.
pub fn read_dax_queries(project: &Project) -> Result<Option<DaxQueries>, Error> {
    let Some(dir) = folder(project).filter(|d| d.is_dir()) else {
        return Ok(None);
    };
    let files = dax_files(&dir)?;
    let settings = read_settings(&dir)?.unwrap_or_default();
    let order: Vec<&str> = settings
        .get("tabOrder")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut picked: Vec<&(String, PathBuf)> = Vec::with_capacity(files.len());
    for name in &order {
        if let Some(f) = files.iter().find(|(n, _)| n.eq_ignore_ascii_case(name))
            && !picked.iter().any(|p| std::ptr::eq(*p, f))
        {
            picked.push(f);
        }
    }
    for f in &files {
        if !picked.iter().any(|p| std::ptr::eq(*p, f)) {
            picked.push(f);
        }
    }
    let mut total = 0u64;
    let mut tabs = Vec::with_capacity(picked.len());
    for (name, path) in picked {
        let bytes = read_capped(path)?;
        let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        total = total.saturating_add(len);
        if len > MAX_QUERY_BYTES || total > MAX_ALL_QUERY_BYTES {
            return Err(Error::TooLarge(path.clone()));
        }
        let text = bicraft_opc::decode_text(&path.to_string_lossy(), &bytes)
            .map_err(|e| Error::Text(path.clone(), e))?
            .0;
        tabs.push(DaxQuery {
            name: name.clone(),
            text,
        });
    }
    let default_tab = settings
        .get("defaultTab")
        .and_then(Value::as_str)
        .and_then(|d| tabs.iter().find(|t| t.name.eq_ignore_ascii_case(d)))
        .map(|t| t.name.clone());
    Ok(Some(DaxQueries { tabs, default_tab }))
}

/// Writes the tabs into the project: one `.dax` file each, the settings file, and removes the
/// `.dax` files of tabs that are gone. Every name is checked before anything is written.
pub fn write_dax_queries(project: &Project, queries: &DaxQueries) -> Result<(), Error> {
    let dir = folder(project).ok_or_else(|| Error::NoModel(project.root.clone()))?;
    if queries.tabs.len() > MAX_QUERY_TABS {
        return Err(Error::BadDefinition(dir, "more than 1000 DAX query tabs"));
    }
    for (i, t) in queries.tabs.iter().enumerate() {
        check_query_name(&t.name).map_err(|why| Error::BadDefinition(dir.join(&t.name), why))?;
        if queries
            .tabs
            .iter()
            .take(i)
            .any(|o| o.name.eq_ignore_ascii_case(&t.name))
        {
            return Err(Error::BadDefinition(
                dir.join(&t.name),
                "two query tabs have this name",
            ));
        }
    }
    if queries.tabs.is_empty() && !dir.is_dir() {
        return Ok(());
    }
    let pbi = dir.join(".pbi");
    std::fs::create_dir_all(&pbi).map_err(|e| Error::Io(pbi.clone(), e))?;
    for t in &queries.tabs {
        replace_file(&dir.join(format!("{}.dax", t.name)), &t.text)?;
    }
    for (name, path) in dax_files(&dir)? {
        if !queries.tabs.iter().any(|t| t.name == name) {
            std::fs::remove_file(&path).map_err(|e| Error::Io(path, e))?;
        }
    }
    let mut settings = read_settings(&dir).ok().flatten().unwrap_or_default();
    if !settings.contains_key("version") {
        settings.insert("version".into(), json!(SETTINGS_VERSION));
    }
    settings.insert(
        "tabOrder".into(),
        Value::Array(queries.tabs.iter().map(|t| json!(t.name)).collect()),
    );
    let default = queries
        .default_tab
        .as_ref()
        .and_then(|d| queries.tabs.iter().find(|t| t.name.eq_ignore_ascii_case(d)))
        .or(queries.tabs.first())
        .map(|t| t.name.clone());
    match default {
        Some(d) => {
            settings.insert("defaultTab".into(), json!(d));
        }
        None => {
            settings.remove("defaultTab");
        }
    }
    let path = dir.join(DAX_QUERIES_JSON);
    let text =
        serde_json::to_string_pretty(&Value::Object(settings)).map_err(|e| Error::Json(path.clone(), e))?;
    replace_file(&path, &text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tempdir::TempDir;
    use crate::{ModelFormat, create_project};
    use bicraft_model::SemanticModel;

    fn project(name: &str) -> (TempDir, Project) {
        let tmp = TempDir::new(&format!("pbip-daxq-{name}"));
        let model = SemanticModel {
            name: "Shop".into(),
            ..Default::default()
        };
        let p = create_project(&tmp.join("out"), "Shop", &model, ModelFormat::Tmdl).unwrap();
        (tmp, p)
    }

    fn q(name: &str, text: &str) -> DaxQuery {
        DaxQuery {
            name: name.into(),
            text: text.into(),
        }
    }

    #[test]
    fn queries_round_trip_byte_for_byte() {
        let (_tmp, p) = project("rt");
        assert_eq!(read_dax_queries(&p).unwrap(), None);
        let queries = DaxQueries {
            tabs: vec![
                q("Sales by year", "// LF\nEVALUATE\n    VALUES('Date'[Year])\n"),
                q("Query 1", "EVALUATE\r\n    {1}\r\n"),
                q("Été", "EVALUATE {\"é\"}"),
            ],
            default_tab: Some("query 1".into()),
        };
        write_dax_queries(&p, &queries).unwrap();
        let dir = p.semantic_model.clone().unwrap().join("DAXQueries");
        assert_eq!(
            std::fs::read(dir.join("Query 1.dax")).unwrap(),
            b"EVALUATE\r\n    {1}\r\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join(".pbi/daxQueries.json")).unwrap(),
            "{\n  \"version\": \"1.0.0\",\n  \"tabOrder\": [\n    \"Sales by year\",\n    \"Query 1\",\n    \"Été\"\n  ],\n  \"defaultTab\": \"Query 1\"\n}"
        );
        let back = read_dax_queries(&p).unwrap().unwrap();
        assert_eq!(back.tabs, queries.tabs);
        assert_eq!(back.default_tab.as_deref(), Some("Query 1"));
        // written again: the same bytes
        let before: Vec<Vec<u8>> = [
            "Sales by year.dax",
            "Query 1.dax",
            "Été.dax",
            ".pbi/daxQueries.json",
        ]
        .iter()
        .map(|f| std::fs::read(dir.join(f)).unwrap())
        .collect();
        write_dax_queries(&p, &back).unwrap();
        let after: Vec<Vec<u8>> = [
            "Sales by year.dax",
            "Query 1.dax",
            "Été.dax",
            ".pbi/daxQueries.json",
        ]
        .iter()
        .map(|f| std::fs::read(dir.join(f)).unwrap())
        .collect();
        assert_eq!(before, after);
        // a tab closed: its file goes, other settings members stay
        let mut json: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(".pbi/daxQueries.json")).unwrap())
                .unwrap();
        json["other"] = json!(7);
        std::fs::write(dir.join(".pbi/daxQueries.json"), json.to_string()).unwrap();
        write_dax_queries(
            &p,
            &DaxQueries {
                tabs: vec![q("Query 1", "EVALUATE {2}")],
                default_tab: None,
            },
        )
        .unwrap();
        assert!(!dir.join("Sales by year.dax").exists());
        let json: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(".pbi/daxQueries.json")).unwrap())
                .unwrap();
        assert_eq!(json["other"], 7);
        assert_eq!(json["defaultTab"], "Query 1");
    }

    #[test]
    fn files_not_in_the_tab_order_come_after_and_bad_files_are_errors() {
        let (_tmp, p) = project("order");
        let dir = p.semantic_model.clone().unwrap().join("DAXQueries");
        std::fs::create_dir_all(dir.join(".pbi")).unwrap();
        std::fs::write(dir.join("b.dax"), "EVALUATE {2}").unwrap();
        std::fs::write(dir.join("a.DAX"), "\u{feff}EVALUATE {1}").unwrap();
        std::fs::write(dir.join("c.dax"), "EVALUATE {3}").unwrap();
        std::fs::write(dir.join("notes.txt"), "x").unwrap();
        std::fs::write(
            dir.join(".pbi/daxQueries.json"),
            r#"{"tabOrder": ["c", "missing", "C"], "defaultTab": "nope"}"#,
        )
        .unwrap();
        let back = read_dax_queries(&p).unwrap().unwrap();
        let names: Vec<&str> = back.tabs.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["c", "a", "b"]);
        assert_eq!(back.tabs[1].text, "EVALUATE {1}");
        assert_eq!(back.default_tab, None);
        std::fs::write(dir.join(".pbi/daxQueries.json"), "{oops").unwrap();
        assert!(read_dax_queries(&p).is_err());
        std::fs::write(dir.join(".pbi/daxQueries.json"), "[]").unwrap();
        assert_eq!(read_dax_queries(&p).unwrap().unwrap().tabs.len(), 3);
        std::fs::write(dir.join("bad.dax"), [0xff, 0xfe, 0x00]).unwrap();
        assert!(read_dax_queries(&p).is_err());
    }

    #[test]
    fn names_that_are_not_file_names_are_refused_before_anything_is_written() {
        let (_tmp, p) = project("names");
        for bad in ["", " x", "x.", "a/b", "a:b", "CON", "com1.x", "..", "a\u{7}"] {
            assert!(check_query_name(bad).is_err(), "{bad:?}");
            let r = write_dax_queries(
                &p,
                &DaxQueries {
                    tabs: vec![q("ok", "EVALUATE {1}"), q(bad, "EVALUATE {1}")],
                    default_tab: None,
                },
            );
            assert!(r.is_err(), "{bad:?}");
        }
        assert!(check_query_name("Query 1").is_ok());
        assert!(check_query_name("CONSOLE").is_ok());
        let dup = DaxQueries {
            tabs: vec![q("A", ""), q("a", "")],
            default_tab: None,
        };
        assert!(write_dax_queries(&p, &dup).is_err());
        assert!(!p.semantic_model.clone().unwrap().join("DAXQueries").exists());
        // no tabs and no folder: nothing is written
        write_dax_queries(&p, &DaxQueries::default()).unwrap();
        assert!(!p.semantic_model.clone().unwrap().join("DAXQueries").exists());
    }
}
