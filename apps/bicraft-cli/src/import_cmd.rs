//! `bicraft-cli import SOURCE TARGET [selectors] [options]`: copies objects from SOURCE into TARGET
//! with `objects.import`, then saves TARGET with `file.save` (not with `--dry-run`).

use bicraft_engine::Session;
use serde_json::{Map, Value, json};

use super::{Failure, INVALID, OK, USAGE_ERROR, open_model, print_json, usage};

/// List flags and the `objects.import` parameter each one fills.
const LISTS: &[(&str, &str)] = &[
    ("--measure", "measures"),
    ("--table", "tables"),
    ("--column", "columns"),
    ("--calculation-group", "calculation_groups"),
    ("--function", "functions"),
    ("--expression", "expressions"),
    ("--role", "roles"),
    ("--page", "pages"),
    ("--visual", "visuals"),
    ("--bookmark", "bookmarks"),
];

/// Value flags and their parameter.
const VALUES: &[(&str, &str)] = &[
    ("--on-conflict", "on_conflict"),
    ("--rename-suffix", "rename_suffix"),
    ("--into-table", "into_table"),
    ("--target-page", "target_page"),
];

pub(crate) fn import(args: &[String]) -> Result<u8, Failure> {
    let mut positional: Vec<&str> = Vec::new();
    let mut params = Map::new();
    let (mut json_out, mut backup) = (false, true);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let a = a.as_str();
        if let Some((_, key)) = LISTS.iter().find(|(f, _)| *f == a) {
            let v = it.next().ok_or_else(|| usage(format!("{a} needs a value")))?;
            let list = params.entry(*key).or_insert_with(|| Value::Array(Vec::new()));
            if let Value::Array(l) = list {
                l.push(json!(v));
            }
            continue;
        }
        if let Some((_, key)) = VALUES.iter().find(|(f, _)| *f == a) {
            let v = it.next().ok_or_else(|| usage(format!("{a} needs a value")))?;
            params.insert((*key).into(), json!(v));
            continue;
        }
        match a {
            "--theme" => {
                params.insert("theme".into(), json!(true));
            }
            "--no-dependencies" => {
                params.insert("include_dependencies".into(), json!(false));
            }
            "--no-relationships" => {
                params.insert("relationships".into(), json!(false));
            }
            "--allow-broken" => {
                params.insert("allow_broken".into(), json!(true));
            }
            "--dry-run" => {
                params.insert("dry_run".into(), json!(true));
            }
            "--no-backup" => backup = false,
            "--json" => json_out = true,
            flag if flag.starts_with("--") => {
                return Err(usage(format!("import: unknown argument `{flag}`")));
            }
            path => positional.push(path),
        }
    }
    let [source, target] = positional.as_slice() else {
        return Err(usage("import needs SOURCE and TARGET (see `bicraft-cli help`)"));
    };
    params.insert("source".into(), json!(source));
    let dry_run = params.get("dry_run") == Some(&Value::Bool(true));
    let mut session = Session::new();
    open_model(&mut session, target)?;
    let mut v = match session.execute("objects.import", &Value::Object(params)) {
        Ok(v) => v,
        Err(e) if e.starts_with("nothing was imported") => return Err((INVALID, e)),
        Err(e) => return Err((USAGE_ERROR, e)),
    };
    if !dry_run {
        let saved = session
            .execute("file.save", &json!({"backup": backup}))
            .map_err(|e| (INVALID, format!("imported, but saving {target} failed: {e}")))?;
        v["saved"] = saved;
    }
    if json_out {
        print_json(&v);
    } else {
        text(&v, target);
    }
    let blocked = dry_run && v.get("would_apply") == Some(&Value::Bool(false));
    Ok(if blocked { INVALID } else { OK })
}

/// A short human summary of the result.
fn text(v: &Value, target: &str) {
    let list = |k: &str| v.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
    for o in list("objects") {
        let name = match (o["table"].as_str(), o["name"].as_str()) {
            (Some(t), Some(n)) => format!("{t}[{n}]"),
            (None, Some(n)) => n.to_string(),
            _ => continue,
        };
        let mut line = format!(
            "{:<9} {} {name}",
            o["action"].as_str().unwrap_or_default(),
            o["kind"].as_str().unwrap_or_default()
        );
        if let Some(src) = o["source_name"].as_str() {
            line.push_str(&format!(" (was {src})"));
        }
        if let Some(by) = o["required_by"].as_str() {
            line.push_str(&format!(" (used by {by})"));
        }
        outln!("{line}");
    }
    for r in list("relationships") {
        outln!(
            "added     relationship {} -> {}",
            r["from"].as_str().unwrap_or_default(),
            r["to"].as_str().unwrap_or_default()
        );
    }
    for w in list("warnings") {
        outln!("warning: {}", w.as_str().unwrap_or_default());
    }
    for e in list("errors") {
        outln!("error: {}", e.as_str().unwrap_or_default());
    }
    if v["dry_run"] == true {
        outln!("dry run: {target} was not changed");
    } else {
        outln!("saved {target}");
    }
}
