//! Get data and Enter data (G58, G59): the Navigator's read of a local source
//! (`source.navigate`), its Load of the checked items as one edit (`source.load`), and typed-in
//! tables (`table.enter_data`, read back by `table.entered_data`), docs/mcp.md "Get data".
//!
//! The queries come from `bicraft_data::m::navigate` and `bicraft_data::m::entered`; the tables
//! are made by `table.import`'s code (`pbix_struct::import`), so they get the same columns,
//! partitions and Desktop defaults.

use std::path::{Path, PathBuf};

use bicraft_data::m::entered::{self, Cell, EnteredType};
use bicraft_data::m::navigate::{NavKind, NavOptions};
use bicraft_data::{Culture, Limits, SourceOptions};
use serde_json::{Value, json};

use crate::Session;
use crate::params::{allow, opt_bool, opt_str, str_param};

/// Most tables one `source.load` makes.
const MAX_LOAD: usize = 100;
/// Longest query `source.load` takes, in bytes.
const MAX_QUERY: usize = 1 << 20;

/// The model's query culture (`sourceQueryCulture`, else `culture`), `en-US` without one.
fn culture_name(s: &Session) -> String {
    s.model
        .as_ref()
        .and_then(crate::datasrc::query_culture)
        .unwrap_or_else(|| "en-US".to_string())
}

fn delimiter(p: &Value) -> Result<Option<char>, String> {
    let Some(d) = opt_str(p, "delimiter")? else {
        return Ok(None);
    };
    if d.eq_ignore_ascii_case("tab") || d == "\\t" {
        return Ok(Some('\t'));
    }
    let mut chars = d.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c != '"' && c != '\n' && c != '\r' => Ok(Some(c)),
        _ => Err("`delimiter` must be one character (or \"tab\")".into()),
    }
}

/// The folder file functions may read for `path`: the call's `data_root`, else the session's
/// when the path is under it, else the file's own folder (the folder itself for a folder).
fn nav_root(s: &Session, p: &Value, path: &Path, is_dir: bool) -> Result<PathBuf, String> {
    let own = || -> Result<PathBuf, String> {
        if is_dir {
            Ok(path.to_path_buf())
        } else {
            path.parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| format!("`{}` has no folder", path.display()))
        }
    };
    match crate::datasrc::data_root_param(p)? {
        Some(Some(r)) => Ok(r),
        Some(None) => own(),
        None => match &s.data_root {
            Some(r) if under(path, r) => Ok(r.clone()),
            _ => own(),
        },
    }
}

fn under(path: &Path, root: &Path) -> bool {
    match (path.canonicalize(), root.canonicalize()) {
        (Ok(a), Ok(b)) => a.starts_with(b),
        _ => false,
    }
}

/// `source.navigate`: the items of a local source and the query of each.
pub(crate) fn source_navigate(s: &Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "path",
            "kind",
            "delimiter",
            "data_root",
            "detect_types",
            "server",
            "database",
            "query",
            "url",
        ],
    )?;
    let kind = opt_str(p, "kind")?.map(str::to_ascii_lowercase);
    let sql = kind.as_deref() == Some("sqlserver");
    if sql || p.get("server").is_some() {
        return navigate_sql(s, p);
    }
    match kind.as_deref() {
        Some(k @ ("web" | "odata" | "sharepoint_folder" | "sharepoint_list")) => {
            return navigate_url(s, p, k);
        }
        _ if p.get("url").is_some() => {
            return Err("a `url` is read with kind web, odata, sharepoint_folder or sharepoint_list".into());
        }
        _ => {}
    }
    let text = str_param(p, "path")?;
    if text.trim().is_empty() || text.contains('\0') {
        return Err("`path` must name a file or a folder".into());
    }
    let path = std::path::absolute(text).map_err(|e| format!("path `{text}`: {e}"))?;
    let meta = std::fs::metadata(&path).map_err(|e| format!("path `{text}`: {e}"))?;
    let kind = match opt_str(p, "kind")? {
        Some(k) => NavKind::from_name(k)
            .ok_or_else(|| format!("unknown `kind` `{k}` (expected excel, csv, json or folder)"))?,
        None if meta.is_dir() => NavKind::Folder,
        None => NavKind::from_extension(text).ok_or_else(|| {
            format!("`{text}`: pass `kind` (excel, csv, json or folder); the extension does not say")
        })?,
    };
    if (kind == NavKind::Folder) != meta.is_dir() {
        return Err(if meta.is_dir() {
            format!("`{text}` is a folder: use kind folder")
        } else {
            format!("`{text}` is a file, not a folder")
        });
    }
    let nav = NavOptions {
        delimiter: delimiter(p)?,
        no_types: !opt_bool(p, "detect_types")?.unwrap_or(true),
    };
    let root = nav_root(s, p, &path, meta.is_dir())?;
    let culture = culture_name(s);
    let opts = SourceOptions {
        limits: Limits::default(),
        file_roots: vec![root.clone()],
        culture: Some(culture),
        ..SourceOptions::default()
    };
    let path_text = path.display().to_string();
    let items = bicraft_data::m::navigate::navigate(kind, &path_text, &opts, &nav)
        .map_err(|e| crate::datasrc::describe(&e, Some(&root)))?;
    let truncated = items.len() >= bicraft_data::m::navigate::MAX_ITEMS;
    let items: Vec<Value> = items
        .into_iter()
        .map(|i| {
            let mut v = json!({"name": i.name, "kind": i.kind, "m": i.m, "hidden": i.hidden});
            if let Some(e) = i.error {
                v["error"] = json!(e);
            }
            v
        })
        .collect();
    Ok(json!({
        "kind": kind.name(),
        "path": path_text,
        "data_root": root.display().to_string(),
        "items": items,
        "truncated": truncated,
    }))
}

/// Longest native query `source.navigate` takes, in bytes.
const MAX_NATIVE_QUERY: usize = 256 << 10;

/// `source.navigate {kind: sqlserver}`: the tables and views of a SQL Server database through
/// the SQL Server connector (network access to the server named; the credentials of the
/// session's `source.credentials`, else of the environment).
fn navigate_sql(s: &Session, p: &Value) -> Result<Value, String> {
    if p.get("path").is_some() || p.get("delimiter").is_some() {
        return Err("a SQL Server source takes `server` and `database`, not `path`".into());
    }
    let server = str_param(p, "server")?.trim();
    let database = str_param(p, "database")?.trim();
    if server.is_empty() || database.is_empty() {
        return Err("`server` and `database` must name a SQL Server and one of its databases".into());
    }
    let query = opt_str(p, "query")?.map(str::trim).filter(|q| !q.is_empty());
    if query.is_some_and(|q| q.len() > MAX_NATIVE_QUERY) {
        return Err(format!("the query is longer than {MAX_NATIVE_QUERY} bytes"));
    }
    let (connectors, _) = crate::refresh::connectors(s, &["sqlserver".to_string()], false)?;
    let opts = SourceOptions {
        limits: Limits::default(),
        culture: Some(culture_name(s)),
        connectors,
        ..SourceOptions::default()
    };
    let items = bicraft_data::m::navigate::sql_server(server, database, query, &opts)
        .map_err(|e| crate::datasrc::describe(&e, None))?;
    let truncated = items.len() >= bicraft_data::m::navigate::MAX_ITEMS;
    let items: Vec<Value> = items
        .into_iter()
        .map(|i| json!({"name": i.name, "kind": i.kind, "m": i.m, "hidden": i.hidden}))
        .collect();
    Ok(json!({
        "kind": "sqlserver",
        "server": server,
        "database": database,
        "items": items,
        "truncated": truncated,
        "allow_connectors": ["sqlserver"],
    }))
}

/// Longest URL `source.navigate` takes, in bytes.
const MAX_URL: usize = 8 << 10;

/// `source.navigate {kind: web | odata, url}`: a web resource read through the Web connector
/// (JSON or delimited text), or an OData feed's entity sets through the OData connector (network
/// access to the URL named, anonymously).
fn navigate_url(s: &Session, p: &Value, kind: &str) -> Result<Value, String> {
    for k in ["path", "server", "database", "query"] {
        if p.get(k).is_some() {
            return Err(format!("a {kind} source takes `url`, not `{k}`"));
        }
    }
    let url = str_param(p, "url")?.trim();
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) || url.len() > MAX_URL {
        return Err(format!(
            "`url` must be an http:// or https:// address of at most {MAX_URL} bytes"
        ));
    }
    let family = if kind.starts_with("sharepoint") {
        "sharepoint"
    } else {
        kind
    };
    let (connectors, _) = crate::refresh::connectors(s, &[family.to_string()], false)?;
    let opts = SourceOptions {
        limits: Limits::default(),
        culture: Some(culture_name(s)),
        connectors,
        ..SourceOptions::default()
    };
    let nav = NavOptions {
        delimiter: delimiter(p)?,
        no_types: !opt_bool(p, "detect_types")?.unwrap_or(true),
    };
    let items = if kind == "web" {
        bicraft_data::m::navigate::web(url, &opts, &nav)
    } else if kind == "sharepoint_folder" {
        bicraft_data::m::navigate::sharepoint_folder(url, &opts, &nav)
    } else if kind == "sharepoint_list" {
        bicraft_data::m::navigate::sharepoint_list(url, &opts)
    } else {
        if p.get("delimiter").is_some() {
            return Err("an OData feed takes no `delimiter`".into());
        }
        bicraft_data::m::navigate::odata(url, &opts)
    }
    .map_err(|e| crate::datasrc::describe(&e, None))?;
    let truncated = items.len() >= bicraft_data::m::navigate::MAX_ITEMS;
    let items: Vec<Value> = items
        .into_iter()
        .map(|i| {
            let mut v = json!({"name": i.name, "kind": i.kind, "m": i.m, "hidden": i.hidden});
            if let Some(e) = i.error {
                v["error"] = json!(e);
            }
            v
        })
        .collect();
    Ok(json!({
        "kind": kind,
        "url": url,
        "items": items,
        "truncated": truncated,
        "allow_connectors": [family],
    }))
}

/// `source.load`: one import table per item, as one edit (all or nothing).
pub(crate) fn source_load(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["tables", "data_root", "allow_connectors"])?;
    let list = p
        .get("tables")
        .and_then(Value::as_array)
        .ok_or("`tables` must be a list of {name, m}")?;
    if list.is_empty() || list.len() > MAX_LOAD {
        return Err(format!("`tables` holds 1 to {MAX_LOAD} tables"));
    }
    let mut calls = Vec::with_capacity(list.len());
    for (i, t) in list.iter().enumerate() {
        allow(t, &["name", "m"]).map_err(|e| format!("tables[{i}]: {e}"))?;
        let name = str_param(t, "name").map_err(|e| format!("tables[{i}]: {e}"))?;
        let m = str_param(t, "m").map_err(|e| format!("tables[{i}]: {e}"))?;
        if m.len() > MAX_QUERY {
            return Err(format!("tables[{i}]: the query is longer than {MAX_QUERY} bytes"));
        }
        if calls
            .iter()
            .any(|c: &Value| c["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name)))
        {
            return Err(format!("two tables are named `{name}`"));
        }
        let mut c = json!({"name": name, "m": m});
        for k in ["data_root", "allow_connectors"] {
            if let Some(v) = p.get(k) {
                c[k] = v.clone();
            }
        }
        calls.push(c);
    }
    let saved = (
        s.model.clone(),
        s.model_error.clone(),
        s.dirty,
        s.data_root.clone(),
    );
    let mut done = Vec::with_capacity(calls.len());
    for c in &calls {
        match crate::pbix_struct::import(s, c, false, false) {
            Ok(v) => done.push(v),
            Err(e) => {
                (s.model, s.model_error, s.dirty, s.data_root) = saved;
                return Err(format!("table `{}`: {e}", c["name"].as_str().unwrap_or("")));
            }
        }
    }
    Ok(json!({ "tables": done }))
}

/// A column of `table.enter_data`: `"Name"` or `{name, type?}` (`auto` or no type: detected).
fn column(v: &Value, i: usize) -> Result<(String, Option<EnteredType>), String> {
    let (name, ty) = match v {
        Value::String(n) => (n.as_str(), None),
        Value::Object(_) => {
            allow(v, &["name", "type"]).map_err(|e| format!("columns[{i}]: {e}"))?;
            (
                str_param(v, "name").map_err(|e| format!("columns[{i}]: {e}"))?,
                opt_str(v, "type").map_err(|e| format!("columns[{i}]: {e}"))?,
            )
        }
        _ => return Err(format!("columns[{i}] must be a name or {{name, type}}")),
    };
    let ty = match ty {
        None => None,
        Some(t) if t.eq_ignore_ascii_case("auto") => None,
        Some(t) => Some(EnteredType::from_name(t).ok_or_else(|| {
            format!(
                "columns[{i}]: unknown type `{t}` (expected auto, text, whole, number, fixed, date, datetime or logical)"
            )
        })?),
    };
    Ok((name.to_string(), ty))
}

fn cell(v: &Value, r: usize, c: usize) -> Result<Cell, String> {
    Ok(match v {
        Value::Null => Cell::Null,
        Value::String(t) => Cell::Text(t.clone()),
        Value::Bool(b) => Cell::Logical(*b),
        Value::Number(n) => Cell::Number(
            n.as_f64()
                .ok_or_else(|| format!("rows[{r}][{c}] is not a number a cell can hold"))?,
        ),
        _ => {
            return Err(format!(
                "rows[{r}][{c}] must be text, a number, true/false or null"
            ));
        }
    })
}

/// `table.enter_data`: a table typed in, stored as Desktop's Enter data query.
pub(crate) fn table_enter_data(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "name",
            "columns",
            "rows",
            "culture",
            "replace",
            "hidden",
            "description",
        ],
    )?;
    let name = str_param(p, "name")?.to_string();
    crate::edit::check_name("table", &name)?;
    let cols = p
        .get("columns")
        .and_then(Value::as_array)
        .ok_or("`columns` must be a list of names or {name, type}")?;
    if cols.len() > entered::MAX_COLUMNS {
        return Err(format!("at most {} columns", entered::MAX_COLUMNS));
    }
    let columns = cols
        .iter()
        .enumerate()
        .map(|(i, c)| column(c, i))
        .collect::<Result<Vec<_>, _>>()?;
    let rows = match p.get("rows") {
        None | Some(Value::Null) => &Vec::new(),
        Some(Value::Array(a)) => a,
        Some(_) => return Err("`rows` must be a list of rows (lists of cells)".into()),
    };
    if rows.len().saturating_mul(columns.len().max(1)) > entered::MAX_CELLS {
        return Err(format!(
            "an entered table holds at most {} cells",
            entered::MAX_CELLS
        ));
    }
    let mut cells = Vec::with_capacity(rows.len());
    for (r, row) in rows.iter().enumerate() {
        let row = row
            .as_array()
            .ok_or_else(|| format!("rows[{r}] must be a list of cells"))?;
        cells.push(
            row.iter()
                .enumerate()
                .map(|(c, v)| cell(v, r, c))
                .collect::<Result<Vec<_>, _>>()?,
        );
    }
    let query = culture_name(s);
    let input = match opt_str(p, "culture")? {
        Some(c) => c.to_string(),
        None => query.clone(),
    };
    let e = entered::build(
        &columns,
        &cells,
        Culture::from_name(&input),
        Culture::from_name(&query),
    )
    .map_err(|e| e.to_string())?;
    // the query must load the values typed in
    let opts = SourceOptions {
        culture: Some(query.clone()),
        ..SourceOptions::default()
    };
    let loaded =
        bicraft_data::m::load_m_with(&e.m, &opts).map_err(|err| format!("internal check failed: {err}"))?;
    let values: Vec<_> = loaded.columns().iter().map(|c| c.values.clone()).collect();
    if values != e.values {
        return Err("internal check failed: the Enter data query does not load the values typed in".into());
    }
    let replace = opt_bool(p, "replace")?.unwrap_or(false);
    let mut call = json!({"name": name, "m": e.m});
    for k in ["hidden", "description"] {
        if let Some(v) = p.get(k) {
            call[k] = v.clone();
        }
    }
    // no file is read: the session's data root is left alone
    let mut out = crate::pbix_struct::import(s, &call, replace, false)?;
    out["types"] = json!(
        e.columns
            .iter()
            .map(|(n, t)| json!({"name": n, "type": t.name()}))
            .collect::<Vec<_>>()
    );
    out["culture"] = json!(query);
    Ok(out)
}

/// `table.entered_data`: the columns, types and cells of a table typed in, for Edit data.
pub(crate) fn table_entered_data(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name"])?;
    let name = str_param(p, "name")?;
    let model = s.model.as_ref().ok_or("no model is open (file.open first)")?;
    let t = model
        .tables
        .iter()
        .find(|t| t.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| format!("no table named `{name}`"))?;
    let [part] = t.partitions.as_slice() else {
        return Err(format!(
            "table `{}` has {} partitions, not one",
            t.name,
            t.partitions.len()
        ));
    };
    let m = part
        .expression
        .as_deref()
        .filter(|_| part.source_type.eq_ignore_ascii_case("m"))
        .ok_or_else(|| format!("table `{}` is not loaded by a Power Query (M) query", t.name))?;
    if !entered::is_entered(m) {
        return Err(format!(
            "table `{}` was not entered as data (Enter data or a #table literal): edit its query instead",
            t.name
        ));
    }
    let culture = culture_name(s);
    let opts = SourceOptions {
        culture: Some(culture.clone()),
        ..SourceOptions::default()
    };
    let e = entered::read(m, &opts).map_err(|e| format!("table `{}`: {e}", t.name))?;
    Ok(json!({
        "name": t.name,
        "columns": e.columns.iter().map(|(n, ty)| json!({"name": n, "type": ty.name()})).collect::<Vec<_>>(),
        "rows": e.texts,
        "culture": culture,
        "shape": if m.contains("Binary.Decompress") { "enter_data" } else { "table_literal" },
    }))
}

/// Whether table `t`'s data was typed in (the snapshot's `entered_data`, for Edit data).
pub(crate) fn is_entered_table(t: &bicraft_model::Table) -> bool {
    match t.partitions.as_slice() {
        [p] => {
            p.source_type.eq_ignore_ascii_case("m")
                && p.expression.as_deref().is_some_and(entered::is_entered)
        }
        _ => false,
    }
}
