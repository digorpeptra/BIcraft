//! `bicraft refresh PATH [--allow-connector databricks]... [--table T]... [--external-links]
//! [--data-root DIR] [--backup]`: `model.refresh` on a PBIX file (the new rows go into its
//! `DataModel`) or a PBIP project.
//!
//! The Databricks token comes from `BICRAFT_DATABRICKS_TOKEN`; when it is unset and the console
//! is interactive, it is asked for once, masked, when the first request is made. It is never
//! taken from the command line, printed or written.

use std::io::IsTerminal;
use std::sync::Arc;

use bicraft_engine::{DatabricksToken, Session, TokenSource, token_from_env};
use serde_json::{Value, json};

use super::{Failure, INVALID, OK, open_with, print_json, usage, value};

/// The token of the environment, else a masked prompt on an interactive console.
fn token() -> Result<DatabricksToken, String> {
    match token_from_env() {
        Ok(t) => Ok(t),
        Err(why) if std::io::stdin().is_terminal() && std::io::stderr().is_terminal() => {
            let text = rpassword::prompt_password("Databricks token (input hidden): ")
                .map_err(|e| format!("{why} (the prompt failed: {e})"))?;
            DatabricksToken::new(&text).ok_or(why)
        }
        Err(why) => Err(why),
    }
}

pub(crate) fn refresh(args: &[String]) -> Result<u8, Failure> {
    let mut positional: Vec<String> = Vec::new();
    let mut params = serde_json::Map::new();
    let mut connectors: Vec<Value> = Vec::new();
    let mut tables: Vec<Value> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--allow-connector" => connectors.push(json!(value(&mut it, a)?)),
            "--table" => tables.push(json!(value(&mut it, a)?)),
            "--backup" => {
                params.insert("backup".into(), json!(true));
            }
            "--external-links" => {
                params.insert("external_links".into(), json!(true));
            }
            "--data-root" => {
                params.insert("data_root".into(), json!(value(&mut it, a)?));
            }
            "--token" => {
                return Err(usage(
                    "the token is never passed on the command line: set BICRAFT_DATABRICKS_TOKEN \
                     (or let bicraft ask for it)",
                ));
            }
            other if other.starts_with("--") => return Err(usage(format!("unknown option `{other}`"))),
            other => positional.push(other.to_string()),
        }
    }
    let [path] = positional.as_slice() else {
        return Err(usage(
            "refresh needs one PATH (a .pbix file, a .pbip file or a project folder)",
        ));
    };
    if !connectors.is_empty() {
        params.insert("allow_connectors".into(), Value::Array(connectors));
    }
    if !tables.is_empty() {
        params.insert("tables".into(), Value::Array(tables));
    }
    let mut session = Session::new();
    session.token_source = Some(TokenSource(Arc::new(token)));
    open_with(&mut session, path, None)?;
    let v = session
        .execute("model.refresh", &Value::Object(params))
        .map_err(|e| (INVALID, e))?;
    print_json(&v);
    Ok(OK)
}
