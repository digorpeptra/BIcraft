//! `bicraft-cli sources PATH` (sources.list), `bicraft-cli sources rebase PATH --from A [--to B]
//! [--param NAME] [--server OLD=NEW]... [--dry-run]` (sources.rebase), `bicraft-cli env apply PATH
//! PROFILE [--env-file F] [--dry-run]` (env.apply) and `bicraft-cli env check PATH PROFILE
//! [--env-file F]` (env.check). Data sources and environment profiles: docs/sources-env.md.
//!
//! Rebase and apply save the file unless `--dry-run`. Output is JSON on stdout. Exit codes:
//! 0 done (check: the profile covers the model); 1 check found gaps or errors, or the edit or
//! save failed; 2 usage error, unreadable file or profile file, unknown profile.

use bicraft_engine::Session;
use serde_json::{Map, Value, json};

use super::{Failure, INVALID, OK, USAGE_ERROR, open_with, print_json, usage, value};

pub(crate) const SOURCES_HELP: &str = "usage: bicraft-cli sources PATH
       bicraft-cli sources rebase PATH [--from ROOT] [--to ROOT] [--param NAME]
                                  [--server OLD=NEW]... [--database OLD=NEW]... [--host OLD=NEW]...
                                  [--http-path OLD=NEW]... [--url OLD=NEW]... [--dry-run]
  sources          list the M data sources (JSON): files, folders, databases, Databricks, web
  rebase           move file paths from ROOT to ROOT (or make them relative to the text parameter
                   NAME, added when missing), map servers, databases, hosts, URL prefixes; only
                   literals are rewritten; the file is saved unless --dry-run
Exit codes: 0 done, 1 the edit or the save failed, 2 usage error or unreadable file.";

pub(crate) const ENV_HELP: &str = "usage: bicraft-cli env apply PATH PROFILE [--env-file FILE] [--dry-run]
       bicraft-cli env check PATH PROFILE [--env-file FILE]
  apply   set the profile's parameters and map its paths, servers, hosts, URLs (one edit); the
          file is saved unless --dry-run
  check   what the profile does not cover and what would change (read-only)
  --env-file  the profile file (default: bicraft.env.yaml, .yml or .json next to the project)
Exit codes: 0 done / covered, 1 check found gaps or errors, or the edit or the save failed,
2 usage error, unreadable file or profile file, unknown profile.";

fn help_asked(args: &[String]) -> bool {
    args.iter().any(|a| a == "--help" || a == "-h")
}

/// Runs a mutating command, then saves unless `dry_run`.
fn edit_and_save(session: &mut Session, id: &str, params: Value, dry_run: bool) -> Result<u8, Failure> {
    let mut v = session.execute(id, &params).map_err(|e| (INVALID, e))?;
    if !dry_run && v.get("changed") == Some(&Value::Bool(true)) {
        let saved = session
            .execute("file.save", &json!({}))
            .map_err(|e| (INVALID, format!("{id} succeeded, but file.save failed: {e}")))?;
        v["saved"] = saved;
    }
    print_json(&v);
    Ok(OK)
}

pub(crate) fn sources(args: &[String]) -> Result<u8, Failure> {
    if help_asked(args) {
        outln!("{SOURCES_HELP}");
        return Ok(OK);
    }
    match args.first().map(String::as_str) {
        Some("rebase") => rebase(args.get(1..).unwrap_or(&[])),
        Some(path) if !path.starts_with('-') => {
            if let Some(extra) = args.get(1) {
                return Err(usage(format!(
                    "sources: unexpected argument `{extra}`\n\n{SOURCES_HELP}"
                )));
            }
            let mut session = Session::new();
            open_with(&mut session, path, None)?;
            let v = session
                .execute("sources.list", &json!({}))
                .map_err(|e| (USAGE_ERROR, e))?;
            print_json(&v);
            Ok(OK)
        }
        _ => Err(usage(SOURCES_HELP)),
    }
}

fn pair(flag: &str, v: &str) -> Result<(String, String), Failure> {
    match v.split_once('=') {
        Some((a, b)) if !a.is_empty() && !b.is_empty() => Ok((a.to_string(), b.to_string())),
        _ => Err(usage(format!("{flag} needs OLD=NEW, not `{v}`"))),
    }
}

fn rebase(args: &[String]) -> Result<u8, Failure> {
    let mut path = None;
    let mut params = Map::new();
    let mut maps: [(&str, &str, Map<String, Value>); 5] = [
        ("--server", "servers", Map::new()),
        ("--database", "databases", Map::new()),
        ("--host", "hosts", Map::new()),
        ("--http-path", "http_paths", Map::new()),
        ("--url", "urls", Map::new()),
    ];
    let mut dry_run = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--from" => {
                params.insert("from".into(), json!(value(&mut it, a)?));
            }
            "--to" => {
                params.insert("to".into(), json!(value(&mut it, a)?));
            }
            "--param" => {
                params.insert("param".into(), json!(value(&mut it, a)?));
            }
            "--dry-run" => dry_run = true,
            flag if maps.iter().any(|(f, _, _)| *f == flag) => {
                let (old, new) = pair(flag, value(&mut it, flag)?)?;
                if let Some((_, _, m)) = maps.iter_mut().find(|(f, _, _)| *f == flag) {
                    m.insert(old, json!(new));
                }
            }
            other if other.starts_with("--") => {
                return Err(usage(format!(
                    "sources rebase: unknown option `{other}`\n\n{SOURCES_HELP}"
                )));
            }
            other => {
                if path.replace(other).is_some() {
                    return Err(usage("sources rebase takes one PATH"));
                }
            }
        }
    }
    let path = path.ok_or_else(|| usage(format!("sources rebase needs a PATH\n\n{SOURCES_HELP}")))?;
    for (_, key, m) in maps {
        if !m.is_empty() {
            params.insert(key.into(), Value::Object(m));
        }
    }
    params.insert("dry_run".into(), json!(dry_run));
    let mut session = Session::new();
    open_with(&mut session, path, None)?;
    edit_and_save(&mut session, "sources.rebase", Value::Object(params), dry_run)
}

pub(crate) fn env(args: &[String]) -> Result<u8, Failure> {
    if help_asked(args) {
        outln!("{ENV_HELP}");
        return Ok(OK);
    }
    let action = args.first().map(String::as_str);
    if !matches!(action, Some("apply" | "check")) {
        return Err(usage(ENV_HELP));
    }
    let mut positional = Vec::new();
    let mut env_file = None;
    let mut dry_run = false;
    let mut it = args.get(1..).unwrap_or(&[]).iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--env-file" => env_file = Some(value(&mut it, a)?),
            "--dry-run" if action == Some("apply") => dry_run = true,
            other if other.starts_with("--") => {
                return Err(usage(format!("env: unknown option `{other}`\n\n{ENV_HELP}")));
            }
            other => positional.push(other),
        }
    }
    let [path, profile] = positional.as_slice() else {
        return Err(usage(format!("env needs PATH and PROFILE\n\n{ENV_HELP}")));
    };
    let mut params = json!({"profile": profile});
    if let Some(f) = env_file {
        params["env_file"] = json!(f);
    }
    let mut session = Session::new();
    open_with(&mut session, path, None)?;
    if action == Some("check") {
        let v = session
            .execute("env.check", &params)
            .map_err(|e| (USAGE_ERROR, e))?;
        print_json(&v);
        return Ok(if v.get("ok") == Some(&Value::Bool(true)) {
            OK
        } else {
            INVALID
        });
    }
    // an unreadable profile file or unknown profile is a usage error, as for `check`
    params["dry_run"] = json!(true);
    session
        .execute("env.apply", &params)
        .map_err(|e| (USAGE_ERROR, e))?;
    params["dry_run"] = json!(dry_run);
    edit_and_save(&mut session, "env.apply", params, dry_run)
}
