//! Environment profiles (`env.apply`, `env.check`): a versioned file `bicraft.env.yaml` next to
//! the project names profiles (dev, test, prod, …); each sets parameter values and maps path
//! roots, servers, databases, Databricks hosts and HTTP paths, and URL prefixes. Applying one is
//! one edit built on the plan of `sources.rebase` (`sources.rs`), so it rewrites literals only.
//!
//! The file is read by the bounded YAML reader of `bicraft.tests.yaml` (`citest_yaml.rs`): size,
//! depth and node caps, no anchors or aliases, no duplicate keys. Format: `docs/sources-env.md`.
//!
//! ```yaml
//! version: 1
//! profiles:
//!   prod:
//!     parameters: {RootFolder: '\\files\finance\', Year: 2026}
//!     paths: {'C:\data\': '\\files\finance\'}
//!     servers: {dev-sql: prod-sql}
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use bicraft_data::m::sources::{ParamValue, text_literal};
use serde_json::{Map, Value, json};

use crate::Session;
use crate::params::{allow, opt_bool, str_param};
use crate::sources::{Mappings, NewValue, Param, PathMap, Plan, commit, mapping, parameters, plan, scan};

/// The profile files looked for next to the project, in order.
pub(crate) const ENV_FILES: &[&str] = &["bicraft.env.yaml", "bicraft.env.yml", "bicraft.env.json"];

/// Keys of a profile.
const PROFILE_KEYS: &[&str] = &[
    "description",
    "parameters",
    "paths",
    "servers",
    "databases",
    "hosts",
    "http_paths",
    "urls",
];

/// Most profiles in one file.
const MAX_PROFILES: usize = 1_000;

/// One profile, read and checked.
pub(crate) struct Profile {
    pub name: String,
    pub parameters: Vec<(String, Value)>,
    pub maps: Mappings,
}

/// Reads the profile document: `env_file` (a path or the profiles inline), else the file next to
/// the open project. Returns the document and the file it came from.
fn load(s: &Session, p: &Value) -> Result<(Value, Option<PathBuf>), String> {
    match p.get("env_file") {
        Some(v @ Value::Object(_)) => Ok((v.clone(), None)),
        Some(Value::String(raw)) if !raw.trim().is_empty() => {
            let path = PathBuf::from(raw);
            let (doc, _) = crate::citest::read_file(&path)?;
            Ok((doc, Some(path)))
        }
        None | Some(Value::Null) => {
            let source = s.source.as_ref().ok_or(
                "no file is open, so there is no project folder to find bicraft.env.yaml in: pass `env_file`",
            )?;
            let dir = crate::citest::project_folder(source);
            let found = ENV_FILES
                .iter()
                .map(|n| dir.join(n))
                .find(|p| p.is_file())
                .ok_or_else(|| {
                    format!(
                        "no profile file: put {} next to the project, or pass `env_file`",
                        ENV_FILES.join(", ")
                    )
                })?;
            if let Some(server) = s.server_config()
                && !server.contains(&found)
            {
                return Err(
                    "server mode: the profile file next to the project leaves the workspace root".into(),
                );
            }
            let (doc, _) = crate::citest::read_file(&found)?;
            Ok((doc, Some(found)))
        }
        Some(_) => Err("parameter `env_file` must be a file path or the profiles inline (an object)".into()),
    }
}

fn file_label(path: Option<&Path>) -> String {
    path.and_then(Path::file_name)
        .map_or_else(|| "env_file".to_string(), |n| n.to_string_lossy().into_owned())
}

/// The profiles of a document, checked at the top level.
fn profiles(doc: &Value) -> Result<&Map<String, Value>, String> {
    let o = doc
        .as_object()
        .ok_or("the profile file must be a mapping with `version` and `profiles`")?;
    for k in o.keys() {
        if k != "version" && k != "profiles" {
            return Err(format!("unknown key `{k}` (expected: version, profiles)"));
        }
    }
    match o.get("version") {
        None => {}
        Some(v) if v.as_u64() == Some(1) => {}
        Some(v) => {
            return Err(format!(
                "version {v} is not supported (this bicraft reads version 1)"
            ));
        }
    }
    let list = o
        .get("profiles")
        .and_then(Value::as_object)
        .ok_or("`profiles` must be a mapping of profile names to profiles")?;
    if list.len() > MAX_PROFILES {
        return Err(format!("more than {MAX_PROFILES} profiles"));
    }
    Ok(list)
}

/// Reads profile `name` of `doc`.
pub(crate) fn profile(doc: &Value, name: &str) -> Result<Profile, String> {
    let list = profiles(doc)?;
    let Some(v) = list.get(name) else {
        let names: Vec<&str> = list.keys().map(String::as_str).collect();
        return Err(format!(
            "unknown profile `{name}` (the file has: {})",
            if names.is_empty() {
                "none".to_string()
            } else {
                names.join(", ")
            }
        ));
    };
    let o = match v {
        Value::Object(o) => o.clone(),
        Value::Null => Map::new(),
        _ => return Err(format!("profile `{name}` must be a mapping")),
    };
    for k in o.keys() {
        if !PROFILE_KEYS.contains(&k.as_str()) {
            return Err(format!(
                "profile `{name}`: unknown key `{k}` (expected: {})",
                PROFILE_KEYS.join(", ")
            ));
        }
    }
    let parameters = match o.get("parameters") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Object(p)) => p.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        Some(_) => return Err(format!("profile `{name}`: `parameters` must be a mapping")),
    };
    let at = |key: &str| format!("profile `{name}`: {key}");
    let paths = mapping(o.get("paths"), &at("paths"))?
        .into_iter()
        .map(|(from, to)| {
            if crate::sources::is_absolute_path(&from) {
                Ok(PathMap { from, to })
            } else {
                Err(format!(
                    "profile `{name}`: paths: `{from}` is not an absolute path (C:\\…, \\\\server\\share\\… or /…)"
                ))
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Profile {
        name: name.to_string(),
        parameters,
        maps: Mappings {
            paths,
            param: None,
            servers: mapping(o.get("servers"), &at("servers"))?,
            databases: mapping(o.get("databases"), &at("databases"))?,
            hosts: mapping(o.get("hosts"), &at("hosts"))?,
            http_paths: mapping(o.get("http_paths"), &at("http_paths"))?,
            urls: mapping(o.get("urls"), &at("urls"))?,
        },
    })
}

// ---------------------------------------------------------------------------------------------
// Typed parameter values
// ---------------------------------------------------------------------------------------------

fn days_in_month(y: i64, m: i64) -> i64 {
    bicraft_locale::calendar::days_in_month(y, m)
}

/// `YYYY-MM-DD`.
fn parse_date(s: &str) -> Option<(i64, i64, i64)> {
    let mut it = s.split('-');
    let (y, m, d) = (it.next()?, it.next()?, it.next()?);
    if it.next().is_some() || y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return None;
    }
    let (y, m, d): (i64, i64, i64) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
    ((1..=9999).contains(&y) && (1..=12).contains(&m) && d >= 1 && d <= days_in_month(y, m))
        .then_some((y, m, d))
}

/// `HH:MM` or `HH:MM:SS`.
fn parse_time(s: &str) -> Option<(i64, i64, i64)> {
    let mut it = s.split(':');
    let (h, m) = (it.next()?, it.next()?);
    let sec = it.next().unwrap_or("00");
    if it.next().is_some() || h.len() != 2 || m.len() != 2 || sec.len() != 2 {
        return None;
    }
    let (h, m, sec): (i64, i64, i64) = (h.parse().ok()?, m.parse().ok()?, sec.parse().ok()?);
    ((0..24).contains(&h) && (0..60).contains(&m) && (0..60).contains(&sec)).then_some((h, m, sec))
}

fn number_m(v: &Value) -> Option<String> {
    if let Some(i) = v.as_i64() {
        return Some(i.to_string());
    }
    if let Some(u) = v.as_u64() {
        return Some(u.to_string());
    }
    v.as_f64().filter(|f| f.is_finite()).map(|f| format!("{f}"))
}

/// The M text of `v` for a parameter of type `ty`.
pub(crate) fn typed_value(ty: &str, v: &Value) -> Result<NewValue, String> {
    let shown = v.clone();
    let wrong = |what: &str| Err(format!("a {ty} parameter needs {what}, not {v}"));
    let m = match (ty.to_ascii_lowercase().as_str(), v) {
        (_, Value::Null) => return Err("null is not a parameter value".into()),
        ("text", Value::String(s)) | ("any", Value::String(s)) => text_literal(s),
        ("text", _) => return wrong("a text (quote it in YAML)"),
        ("number", Value::Number(_)) | ("any", Value::Number(_)) => match number_m(v) {
            Some(n) => n,
            None => return wrong("a finite number"),
        },
        ("number", _) => return wrong("a number"),
        ("logical", Value::Bool(b)) | ("any", Value::Bool(b)) => b.to_string(),
        ("logical", _) => return wrong("true or false"),
        ("date", Value::String(s)) => match parse_date(s) {
            Some((y, m, d)) => format!("#date({y}, {m}, {d})"),
            None => return wrong("a date YYYY-MM-DD"),
        },
        ("date", _) => return wrong("a date YYYY-MM-DD"),
        ("datetime", Value::String(s)) => {
            let parsed = s
                .split_once(['T', ' '])
                .and_then(|(d, t)| Some((parse_date(d)?, parse_time(t)?)));
            match parsed {
                Some(((y, mo, d), (h, mi, sec))) => format!("#datetime({y}, {mo}, {d}, {h}, {mi}, {sec})"),
                None => return wrong("a date and time YYYY-MM-DDTHH:MM:SS"),
            }
        }
        ("datetime", _) => return wrong("a date and time YYYY-MM-DDTHH:MM:SS"),
        ("time", Value::String(s)) => match parse_time(s) {
            Some((h, m, sec)) => format!("#time({h}, {m}, {sec})"),
            None => return wrong("a time HH:MM:SS"),
        },
        ("time", _) => return wrong("a time HH:MM:SS"),
        ("any", _) => return wrong("a text, a number or true/false"),
        (other, _) => {
            return Err(format!(
                "parameters of type {other} cannot be set by a profile (Text, Number, Logical, Date, \
                 DateTime, Time and Any can)"
            ));
        }
    };
    Ok(NewValue { m, shown })
}

/// The profile's parameter values, typed; every problem is collected.
fn values(params: &[Param], prof: &Profile) -> (BTreeMap<String, NewValue>, Vec<String>) {
    let mut out = BTreeMap::new();
    let mut errors = Vec::new();
    for (name, v) in &prof.parameters {
        let Some(p) = params.iter().find(|p| &p.name == name) else {
            let close: Vec<&str> = params
                .iter()
                .filter(|p| p.name.eq_ignore_ascii_case(name))
                .map(|p| p.name.as_str())
                .collect();
            errors.push(if close.is_empty() {
                format!(
                    "profile `{}`: `{name}` is not a parameter of the model",
                    prof.name
                )
            } else {
                format!(
                    "profile `{}`: `{name}` is not a parameter of the model (did you mean `{}`?)",
                    prof.name,
                    close.join("`, `")
                )
            });
            continue;
        };
        match typed_value(&p.ty, v) {
            Ok(nv) => {
                if let Some(list) = &p.info.allowed
                    && !list.contains(&nv.m)
                {
                    errors.push(format!(
                        "profile `{}`: {} is not one of the allowed values of `{name}` ({})",
                        prof.name,
                        nv.m,
                        list.join(", ")
                    ));
                    continue;
                }
                out.insert(name.clone(), nv);
            }
            Err(e) => errors.push(format!("profile `{}`: parameter `{name}`: {e}", prof.name)),
        }
    }
    (out, errors)
}

/// Reads the profile and the model's parameters: `(profile, file, values, plan)`.
fn prepare(s: &Session, p: &Value) -> Result<(Profile, Option<PathBuf>, Plan, Vec<String>), String> {
    let name = str_param(p, "profile")?;
    let (doc, file) = load(s, p)?;
    let label = file_label(file.as_deref());
    let prof = profile(&doc, name).map_err(|e| format!("{label}: {e}"))?;
    let m = crate::commands::model(s)?;
    let params = parameters(m);
    let (vals, errors) = values(&params, &prof);
    let pl = plan(m, &prof.maps, &vals);
    Ok((prof, file, pl, errors))
}

fn file_json(file: Option<&Path>) -> Value {
    file.map_or(Value::Null, |f| json!(f.display().to_string()))
}

/// `env.apply {profile, env_file?, dry_run?}`.
pub(crate) fn env_apply(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["profile", "env_file", "dry_run"])?;
    let dry_run = opt_bool(p, "dry_run")?.unwrap_or(false);
    let (prof, file, pl, errors) = prepare(s, p)?;
    if !errors.is_empty() {
        return Err(format!(
            "the profile was not applied, nothing was changed: {}",
            errors.join("; ")
        ));
    }
    let extra = json!({"profile": prof.name, "env_file": file_json(file.as_deref())});
    commit(s, &json!({}), pl, dry_run, extra)
}

/// `env.check {profile, env_file?}`.
/// `env.list`: the profiles of the profile file (`env_file`, else the one next to the project):
/// their names, descriptions and how many parameters each sets. Read-only; the parameters form's
/// profile picker runs it.
pub(crate) fn env_list(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["env_file"])?;
    let (doc, file) = load(s, p)?;
    let label = file_label(file.as_deref());
    let list = profiles(&doc).map_err(|e| format!("{label}: {e}"))?;
    let rows: Vec<Value> = list
        .iter()
        .map(|(name, v)| {
            json!({
                "name": name,
                "description": v.get("description").and_then(Value::as_str),
                "parameters": v.get("parameters").and_then(Value::as_object).map_or(0, Map::len),
            })
        })
        .collect();
    Ok(json!({"env_file": file_json(file.as_deref()), "count": rows.len(), "profiles": rows}))
}

pub(crate) fn env_check(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["profile", "env_file"])?;
    let (prof, file, pl, errors) = prepare(s, p)?;
    let m = crate::commands::model(s)?;
    let sc = scan(m);
    let roles = sc.param_roles();
    let set: BTreeSet<&str> = prof.parameters.iter().map(|(n, _)| n.as_str()).collect();
    let changed_params: BTreeSet<&str> = pl
        .parameters
        .iter()
        .filter_map(|v| v.get("name").and_then(Value::as_str))
        .collect();
    // parameters
    let mut parameters = Vec::new();
    let mut gaps = 0usize;
    for prm in &sc.params {
        let used: Vec<&str> = roles
            .get(&prm.name)
            .map(|r| r.iter().copied().collect())
            .unwrap_or_default();
        let covered_by_mapping = match &prm.info.value {
            ParamValue::Text(t) => {
                used.iter()
                    .filter_map(|r| crate::sources::role_named(r))
                    .any(|r| prof.maps.covers(r, t))
                    || (crate::sources::is_absolute_path(t)
                        && prof.maps.covers(bicraft_data::m::sources::Role::Path, t))
            }
            _ => false,
        };
        let status = if set.contains(prm.name.as_str()) {
            if changed_params.contains(prm.name.as_str()) {
                "differs"
            } else {
                "same"
            }
        } else if covered_by_mapping {
            if changed_params.contains(prm.name.as_str()) {
                "differs"
            } else {
                "covered"
            }
        } else {
            "not_covered"
        };
        if status == "not_covered" && !used.is_empty() {
            gaps = gaps.saturating_add(1);
        }
        let mut v = json!({
            "name": prm.name,
            "type": prm.ty,
            "current": crate::sources::value_json(&prm.info.value),
            "status": status,
            "used_as": used,
        });
        if let Some(c) = pl
            .parameters
            .iter()
            .find(|c| c.get("name").and_then(Value::as_str) == Some(prm.name.as_str()))
        {
            v["target"] = c.get("after").cloned().unwrap_or(Value::Null);
        }
        parameters.push(v);
    }
    // sources
    let mut sources = Vec::new();
    for c in &pl.changes {
        let mut v = c.clone();
        v["status"] = json!("differs");
        sources.push(v);
    }
    let mut computed = 0usize;
    for u in &pl.unchanged {
        let mut v = u.clone();
        let reason = u.get("reason").and_then(Value::as_str).unwrap_or_default();
        let status = match reason {
            "no_mapping" => {
                let role = u
                    .get("role")
                    .and_then(Value::as_str)
                    .and_then(crate::sources::role_named);
                let value = u.get("value").and_then(Value::as_str).unwrap_or_default();
                if role.is_some_and(|r| prof.maps.covers(r, value)) {
                    "same"
                } else {
                    "not_covered"
                }
            }
            "parameter" => {
                let name = u.get("parameter").and_then(Value::as_str).unwrap_or_default();
                let covered = set.contains(name)
                    || parameters.iter().any(|p| {
                        p.get("name").and_then(Value::as_str) == Some(name)
                            && p.get("status").and_then(Value::as_str) != Some("not_covered")
                    });
                if covered { "covered" } else { "not_covered" }
            }
            _ => "computed",
        };
        match status {
            "not_covered" => gaps = gaps.saturating_add(1),
            "computed" => computed = computed.saturating_add(1),
            _ => {}
        }
        v["status"] = json!(status);
        sources.push(v);
    }
    let count = |list: &[Value], st: &str| {
        list.iter()
            .filter(|v| v.get("status").and_then(Value::as_str) == Some(st))
            .count()
    };
    let summary = json!({
        "sources": sources.len(),
        "sources_differ": count(&sources, "differs"),
        "sources_not_covered": count(&sources, "not_covered"),
        "sources_computed": computed,
        "parameters": parameters.len(),
        "parameters_differ": count(&parameters, "differs"),
        "parameters_not_covered": count(&parameters, "not_covered"),
    });
    let mut warnings = Vec::new();
    if computed > 0 {
        warnings.push(format!(
            "{computed} source argument(s) are computed: no profile can rewrite them (they are listed with status `computed`)"
        ));
    }
    Ok(json!({
        "profile": prof.name,
        "env_file": file_json(file.as_deref()),
        "ok": errors.is_empty() && gaps == 0,
        "errors": errors,
        "parameters": parameters,
        "sources": sources,
        "summary": summary,
        "warnings": warnings,
    }))
}

#[cfg(test)]
#[path = "envprofile_tests.rs"]
mod tests;
