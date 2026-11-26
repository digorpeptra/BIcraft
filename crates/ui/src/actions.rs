//! Every engine command the UI runs, in one place (AGENTS.md §9: every button runs an engine
//! command by id). The Backstage, the page tabs and the document tabs build their calls here and
//! send them to the active document's source ([`crate::source::Request::Command`]); the ids and
//! parameters are the engine's (`crates/engine/src/commands.rs`, `docs/mcp.md`). When the
//! engine's command placement registry (ROADMAP M11.1) lands, the buttons can read their place
//! from it and keep these builders for the parameters.

use serde_json::{Map, Value, json};

/// An engine command and its parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub id: &'static str,
    pub params: Value,
}

fn call(id: &'static str, params: Value) -> Call {
    Call { id, params }
}

/// Every command id the UI's buttons run (a test checks each is registered in the engine).
pub const IDS: &[&str] = &[
    "file.open",
    "file.inspect",
    "file.save",
    "file.save_as",
    "file.convert",
    "file.redact",
    "file.diff",
    "project.new",
    "report.render",
    "report.export_pdf",
    "report.print",
    "visual.export_data",
    "report.settings",
    "report.extract",
    "report.apply",
    "page.add",
    "page.remove",
    "page.rename",
    "page.move",
    "page.set_type",
];

/// The page types `page.set_type` takes, with their labels.
pub const PAGE_TYPES: [(&str, &str, &str); 3] = [
    ("report", "Normal page", "Page normale"),
    ("tooltip", "Tooltip page", "Page d'info-bulle"),
    ("drillthrough", "Drillthrough page", "Page d'extraction"),
];

/// The formats Export > Convert writes (`file.save_as` / `file.convert` `format`), with the
/// extension of the target.
pub const CONVERT_FORMATS: [(&str, &str, &str); 3] = [
    ("pbix", "pbix", "PBIX"),
    ("pbit", "pbit", "PBIT"),
    ("pbip-tmdl", "pbip", "PBIP"),
];

pub fn file_open(path: &str) -> Call {
    call("file.open", json!({"path": path}))
}

pub fn file_inspect() -> Call {
    call("file.inspect", json!({}))
}

pub fn file_save() -> Call {
    call("file.save", json!({}))
}

pub fn file_save_as(path: &str) -> Call {
    call("file.save_as", json!({"path": path}))
}

/// Converts the open file's saved state (`from`) into `to` without touching the open session.
pub fn file_convert(from: &str, to: &str, format: &str) -> Call {
    call("file.convert", json!({"from": from, "to": to, "format": format}))
}

/// Writes a redacted copy of the saved file `from` to `to` (`file.redact`), the stored text values
/// too with `data`.
pub fn file_redact(from: &str, to: &str, data: bool) -> Call {
    call("file.redact", json!({"path": from, "out": to, "data": data}))
}

/// Compares the open file (`a`) with another file (`b`), as text.
pub fn file_diff(a: &str, b: &str) -> Call {
    call("file.diff", json!({"a": a, "b": b, "format": "text"}))
}

/// A blank PBIP project `<dir>/<name>.pbip` (a fresh session has no model or report: both blank).
pub fn project_new(dir: &str, name: &str) -> Call {
    call("project.new", json!({"dir": dir, "name": name}))
}

/// Renders one page to a PNG file.
pub fn report_render(page: &str, out: &str) -> Call {
    call("report.render", json!({"page": page, "out": out}))
}

/// `report.export_pdf`: `pages` as the command takes it ("all", "current", a range, a list).
pub fn report_export_pdf(out: &str, pages: Value, hidden: bool) -> Call {
    call(
        "report.export_pdf",
        json!({"out": out, "pages": pages, "hidden": hidden}),
    )
}

/// `report.print`: the pages as [`report_export_pdf`] takes them; `out` only on a server (the
/// workspace file the browser prints), else the engine picks a temporary file and opens it.
pub fn report_print(pages: Value, hidden: bool, out: Option<&str>) -> Call {
    let mut p = json!({"pages": pages, "hidden": hidden});
    if let Some(o) = out {
        p["out"] = json!(o);
    }
    call("report.print", p)
}

pub fn visual_export_data(page: &str, visual: &str, kind: &str, out: &str) -> Call {
    call(
        "visual.export_data",
        json!({"page": page, "visual": visual, "kind": kind, "out": out}),
    )
}

/// `report.settings {export_data}`: the report's Export data setting.
pub fn report_export_setting(mode: &str) -> Call {
    call("report.settings", json!({"export_data": mode}))
}

/// `report.settings` with the values one control of File › Options changed.
pub fn report_settings(params: Value) -> Call {
    call("report.settings", params)
}

pub fn page_add(display_name: &str, index: usize) -> Call {
    call("page.add", json!({"display_name": display_name, "index": index}))
}

pub fn page_remove(page: &str) -> Call {
    call("page.remove", json!({"page": page}))
}

pub fn page_rename(page: &str, display_name: &str) -> Call {
    call("page.rename", json!({"page": page, "display_name": display_name}))
}

pub fn page_move(page: &str, index: usize) -> Call {
    call("page.move", json!({"page": page, "index": index}))
}

/// Sets a page's type (`report`, `tooltip`, `drillthrough`) and, when given, whether it is hidden.
/// Hiding or showing a page keeps its type: the caller passes the current one.
pub fn page_set_type(page: &str, kind: &str, hidden: Option<bool>) -> Call {
    let mut p = json!({"page": page, "type": kind});
    if let (Some(h), Some(o)) = (hidden, p.as_object_mut()) {
        o.insert("hidden".into(), json!(h));
    }
    call("page.set_type", p)
}

/// First step of Duplicate page: the page as a declarative spec.
pub fn page_extract(page: &str) -> Call {
    call("report.extract", json!({"page": page}))
}

/// Second step of Duplicate page: the spec `report.extract` gave for one page, applied as a new
/// page named `display_name` at the end (its identity is the `key`, so the page and its visuals get
/// new names; visual keys hash with the new page's name). Positions, types, fields and the
/// properties the spec carries are copied.
pub fn duplicate_from_spec(spec: &Value, key: &str, display_name: &str) -> Result<Call, String> {
    let page = spec
        .get("pages")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(Value::as_object)
        .ok_or("the page could not be read")?;
    let mut out = Map::new();
    for (k, v) in page {
        match k.as_str() {
            "name" => {}
            "display_name" => {}
            "visuals" => {
                let visuals: Vec<Value> = v
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(5000)
                    .map(|vis| match vis.as_object() {
                        Some(o) => {
                            let mut o = o.clone();
                            if let Some(n) = o.remove("name") {
                                o.insert("key".into(), n);
                            }
                            Value::Object(o)
                        }
                        None => vis.clone(),
                    })
                    .collect();
                out.insert("visuals".into(), Value::Array(visuals));
            }
            _ => {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    out.insert("key".into(), json!(key));
    out.insert("display_name".into(), json!(display_name));
    Ok(call(
        "report.apply",
        json!({"spec": {"pages": [Value::Object(out)]}}),
    ))
}

/// The display name of a copy of `name` that no page in `taken` has: `name (2)`, `name (3)`…
pub fn copy_name(name: &str, taken: &[String]) -> String {
    let base: String = name.chars().take(100).collect();
    (2..10_000)
        .map(|n| format!("{base} ({n})"))
        .find(|c| !taken.iter().any(|t| t == c))
        .unwrap_or_else(|| format!("{base} (copy)"))
}

/// A new page's display name that no page has: `Page N`.
pub fn new_page_name(prefix: &str, taken: &[String]) -> String {
    (taken.len().saturating_add(1)..taken.len().saturating_add(10_000))
        .map(|n| format!("{prefix} {n}"))
        .find(|c| !taken.iter().any(|t| t == c))
        .unwrap_or_else(|| format!("{prefix} +"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every id the UI runs is a command of the engine.
    #[test]
    fn every_id_is_an_engine_command() {
        for id in IDS {
            assert!(
                bicraft_engine::COMMANDS.iter().any(|c| c.id == *id),
                "{id} is not an engine command"
            );
        }
        for c in [
            file_open("a"),
            file_inspect(),
            file_save(),
            file_save_as("a"),
            file_convert("a", "b", "pbix"),
            file_diff("a", "b"),
            project_new("d", "n"),
            report_render("p", "o.png"),
            report_export_pdf("o.pdf", json!("all"), false),
            report_print(json!("current"), false, None),
            visual_export_data("p", "v", "summarized", "o.csv"),
            report_export_setting("None"),
            page_add("P", 0),
            page_remove("p"),
            page_rename("p", "q"),
            page_move("p", 1),
            page_set_type("p", "report", Some(true)),
            page_extract("p"),
        ] {
            assert!(IDS.contains(&c.id), "{} missing from IDS", c.id);
        }
    }

    #[test]
    fn names_and_duplicates() {
        let taken = vec![
            "Page 1".to_string(),
            "Page 2".to_string(),
            "Sales (2)".to_string(),
        ];
        assert_eq!(new_page_name("Page", &taken), "Page 4");
        assert_eq!(copy_name("Sales", &taken), "Sales (3)");
        let spec = json!({"pages": [{"name": "abc", "display_name": "Sales", "width": 1280,
            "visuals": [{"name": "v1", "type": "card", "x": 1}, 7]}]});
        let c = duplicate_from_spec(&spec, "copy-1", "Sales (2)").unwrap();
        assert_eq!(c.id, "report.apply");
        let p = &c.params["spec"]["pages"][0];
        assert_eq!(p["key"], "copy-1");
        assert_eq!(p["display_name"], "Sales (2)");
        assert!(p.get("name").is_none());
        assert_eq!(p["visuals"][0]["key"], "v1");
        assert!(p["visuals"][0].get("name").is_none());
        assert_eq!(p["width"], 1280);
        for bad in [
            json!({}),
            json!({"pages": []}),
            json!({"pages": [1]}),
            json!(null),
        ] {
            assert!(duplicate_from_spec(&bad, "k", "n").is_err());
        }
        assert_eq!(
            page_set_type("p", "tooltip", None).params,
            json!({"page": "p", "type": "tooltip"})
        );
    }
}
