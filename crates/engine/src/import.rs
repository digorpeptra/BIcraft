//! Copying objects from another file into the open one (`objects.import`).
//!
//! The source (any format `file.open` reads) is opened read-only in a private session that is
//! never saved. The selection is resolved there, then closed over its DAX and M dependencies
//! (what the target does not already have by name). Conflicts with the target's objects are
//! settled by `on_conflict`; a `rename` renames the source object *in the private session* with
//! the ordinary rename commands, so the DAX, the relationships and the report bindings of the
//! imported set follow it exactly as they do for `table.rename` / `measure.rename`. The renamed
//! objects are then copied into a clone of the target model (and report), the result is checked
//! (DAX newly broken in the target, report fields that do not resolve) and committed as one
//! command, so one `edit.undo` step. `dry_run` returns the same plan without committing.
//!
//! Not copied: perspectives and translations of the source, custom visual packages, report-level
//! measures (an imported visual bound to one is reported as broken unless the target report has
//! it), M queries of renamed tables that other imported queries reference by name.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use bicraft_data::Limits;
use bicraft_data::m::analyze_all;
use bicraft_dax::deps::ObjectRef;
use bicraft_dax::rename::Renamer;
use bicraft_dax::{TokenKind, tokenize};
use bicraft_model::{SemanticModel, Table};
use bicraft_report::{FieldRef, Report, Resource};
use serde_json::{Map, Value, json};

use crate::Session;
use crate::dax_rename::{DaxChange, DaxSite, SiteOwner, model_sites, rename_in_model, rewrite_sites};
use crate::edit::{dax_column, dax_measure, new_guid, newly_broken};
use crate::mquery::is_m;
use crate::params::{allow, opt_bool, opt_str, str_param};
use crate::refs::{ModelChange, SiteKind, apply_change, check_field, check_site, find_refs};

/// Longest selector pattern accepted.
const MAX_PATTERN: usize = 1024;
/// Most selectors per parameter.
const MAX_SELECTORS: usize = 10_000;
/// Longest object name imported (as `check_name` allows).
const MAX_NAME_CHARS: usize = 1024;
/// Most entries listed per result list (counts stay exact).
const MAX_LISTED: usize = 1000;
/// Deepest JSON walked (lineage tags, resource references, id rewrites).
const MAX_DEPTH: usize = 128;
/// Most candidate names tried for a rename.
const MAX_RENAME_TRIES: usize = 1000;
/// Default suffix of `on_conflict: "rename"`.
const DEFAULT_SUFFIX: &str = " (imported)";

const PARAMS: &[&str] = &[
    "source",
    "measures",
    "tables",
    "columns",
    "calculation_groups",
    "functions",
    "expressions",
    "roles",
    "relationships",
    "pages",
    "visuals",
    "target_page",
    "bookmarks",
    "theme",
    "include_dependencies",
    "on_conflict",
    "rename_suffix",
    "into_table",
    "allow_broken",
    "dry_run",
];

// ---------------------------------------------------------------------------------------------
// Objects of the source
// ---------------------------------------------------------------------------------------------

/// A model object of the source, by position (positions do not move when the private session
/// renames objects).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Obj {
    Table(usize),
    Measure(usize, usize),
    /// A calculated column imported into an existing target table.
    Column(usize, usize),
    Function(usize),
    Expression(usize),
    Role(usize),
}

/// A report object of the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum RObj {
    Page(usize),
    Visual(usize, usize),
    Bookmark(usize),
}

/// What an import needs: an object of the source, or a data column (which comes only with its
/// table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Need {
    Obj(Obj),
    DataColumn(usize, usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Fail,
    Skip,
    Replace,
    Rename,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Reason {
    Selected,
    Dependency(String),
}

fn functions(m: &SemanticModel) -> &[Value] {
    m.extra
        .get("functions")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn function_name(f: &Value) -> &str {
    f.get("name").and_then(Value::as_str).unwrap_or_default()
}

fn is_calc_group(t: &Table) -> bool {
    t.extra.get("calculationGroup").is_some()
}

fn eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn table_pos(m: &SemanticModel, name: &str) -> Option<usize> {
    m.tables.iter().position(|t| eq(&t.name, name))
}

fn measure_pos(m: &SemanticModel, name: &str) -> Option<(usize, usize)> {
    m.tables.iter().enumerate().find_map(|(ti, t)| {
        t.measures
            .iter()
            .position(|x| eq(&x.name, name))
            .map(|mi| (ti, mi))
    })
}

fn expression_pos(m: &SemanticModel, name: &str) -> Option<usize> {
    m.expressions.iter().position(|e| eq(&e.name, name))
}

fn function_pos(m: &SemanticModel, name: &str) -> Option<usize> {
    functions(m).iter().position(|f| eq(function_name(f), name))
}

fn role_pos(m: &SemanticModel, name: &str) -> Option<usize> {
    m.roles.iter().position(|r| eq(&r.name, name))
}

/// `{kind, name, table?}` of a source object, named as it is in `m`.
fn describe(m: &SemanticModel, o: Obj) -> Value {
    let table = |ti: usize| m.tables.get(ti).map(|t| t.name.clone()).unwrap_or_default();
    match o {
        Obj::Table(ti) => {
            let kind = if m.tables.get(ti).is_some_and(is_calc_group) {
                "calculation group"
            } else if m
                .tables
                .get(ti)
                .and_then(bicraft_dax::deps::calculated_table_expression)
                .is_some()
            {
                "calculated table"
            } else {
                "table"
            };
            json!({"kind": kind, "name": table(ti)})
        }
        Obj::Measure(ti, mi) => json!({"kind": "measure", "table": table(ti),
            "name": m.tables.get(ti).and_then(|t| t.measures.get(mi)).map(|x| x.name.clone())}),
        Obj::Column(ti, ci) => json!({"kind": "calculated column", "table": table(ti),
            "name": m.tables.get(ti).and_then(|t| t.columns.get(ci)).map(|c| c.name.clone())}),
        Obj::Function(i) => json!({"kind": "function", "name": functions(m).get(i).map(function_name)}),
        Obj::Expression(i) => {
            json!({"kind": "expression", "name": m.expressions.get(i).map(|e| e.name.clone())})
        }
        Obj::Role(i) => json!({"kind": "role", "name": m.roles.get(i).map(|r| r.name.clone())}),
    }
}

/// A short label of a source object for `required_by`.
fn label(m: &SemanticModel, o: Obj) -> String {
    let d = describe(m, o);
    let name = d["name"].as_str().unwrap_or_default();
    match o {
        Obj::Measure(..) => format!("measure {}", dax_measure(name)),
        Obj::Column(..) => format!(
            "calculated column {}",
            dax_column(d["table"].as_str().unwrap_or_default(), name)
        ),
        _ => format!("{} `{name}`", d["kind"].as_str().unwrap_or_default()),
    }
}

fn object_name(m: &SemanticModel, o: Obj) -> String {
    describe(m, o)["name"].as_str().unwrap_or_default().to_string()
}

// ---------------------------------------------------------------------------------------------
// Parameters and selectors
// ---------------------------------------------------------------------------------------------

/// A list parameter: an array of strings, or one string.
fn list(p: &Value, key: &str) -> Result<Vec<String>, String> {
    let items = match p.get(key) {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => {
            if a.len() > MAX_SELECTORS {
                return Err(format!("`{key}` lists more than {MAX_SELECTORS} selectors"));
            }
            a.iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| format!("`{key}` must be a list of strings"))
                })
                .collect::<Result<Vec<_>, _>>()?
        }
        Some(_) => return Err(format!("`{key}` must be a list of strings")),
    };
    for s in &items {
        if s.trim().is_empty() {
            return Err(format!("`{key}`: empty selector"));
        }
        if s.chars().count() > MAX_PATTERN {
            return Err(format!(
                "`{key}`: a selector is longer than {MAX_PATTERN} characters"
            ));
        }
    }
    Ok(items)
}

/// Whether `name` matches `pattern` (`*` and `?`, case-insensitive).
fn matches(pattern: &str, name: &str) -> bool {
    if !pattern.contains(['*', '?']) {
        return name.to_lowercase() == pattern.to_lowercase();
    }
    // Names longer than any accepted name never match a pattern (bounded work).
    name.chars().count() <= MAX_NAME_CHARS && crate::citest_rules::glob(pattern, name)
}

/// `Table[Name]` → (Some(table), name); `[Name]` / `Name` → (None, name). Quotes around the
/// table and `]]` / `''` escapes are undone.
fn split_qualified(sel: &str) -> (Option<String>, String) {
    let s = sel.trim();
    if let (Some(open), true) = (s.find('['), s.ends_with(']')) {
        let table = s.get(..open).unwrap_or_default().trim();
        let name = s
            .get(open.saturating_add(1)..s.len().saturating_sub(1))
            .unwrap_or_default()
            .replace("]]", "]");
        let table = table
            .strip_prefix('\'')
            .and_then(|t| t.strip_suffix('\''))
            .map_or_else(|| table.to_string(), |t| t.replace("''", "'"));
        return ((!table.is_empty()).then_some(table), name);
    }
    let unquoted = s
        .strip_prefix('\'')
        .and_then(|t| t.strip_suffix('\''))
        .map_or_else(|| s.to_string(), |t| t.replace("''", "'"));
    (None, unquoted)
}

fn no_match(kind: &str, sel: &str) -> String {
    format!("no {kind} of the source matches `{sel}`")
}

struct Selection {
    objs: Vec<Obj>,
    robjs: Vec<RObj>,
}

fn select_model(src: &SemanticModel, p: &Value) -> Result<Vec<Obj>, String> {
    let mut out = Vec::new();
    for sel in list(p, "measures")? {
        let (table, name) = split_qualified(&sel);
        let before = out.len();
        for (ti, t) in src.tables.iter().enumerate() {
            if table.as_deref().is_some_and(|tp| !matches(tp, &t.name)) {
                continue;
            }
            for (mi, x) in t.measures.iter().enumerate() {
                if matches(&name, &x.name) {
                    out.push(Obj::Measure(ti, mi));
                }
            }
        }
        if out.len() == before {
            return Err(no_match("measure", &sel));
        }
    }
    for sel in list(p, "tables")? {
        let (_, name) = split_qualified(&sel);
        let before = out.len();
        out.extend(
            src.tables
                .iter()
                .enumerate()
                .filter(|(_, t)| matches(&name, &t.name))
                .map(|(i, _)| Obj::Table(i)),
        );
        if out.len() == before {
            return Err(no_match("table", &sel));
        }
    }
    for sel in list(p, "calculation_groups")? {
        let (_, name) = split_qualified(&sel);
        let before = out.len();
        out.extend(
            src.tables
                .iter()
                .enumerate()
                .filter(|(_, t)| is_calc_group(t) && matches(&name, &t.name))
                .map(|(i, _)| Obj::Table(i)),
        );
        if out.len() == before {
            return Err(no_match("calculation group", &sel));
        }
    }
    for sel in list(p, "columns")? {
        let (Some(table), name) = split_qualified(&sel) else {
            return Err(format!(
                "`columns`: `{sel}` must name its table, e.g. Sales[Margin %] or Sales[*]"
            ));
        };
        let before = out.len();
        let mut data = None;
        for (ti, t) in src.tables.iter().enumerate() {
            if !matches(&table, &t.name) {
                continue;
            }
            for (ci, c) in t.columns.iter().enumerate() {
                if c.is_row_number() || !matches(&name, &c.name) {
                    continue;
                }
                if c.expression.is_some() {
                    out.push(Obj::Column(ti, ci));
                } else {
                    data = Some(dax_column(&t.name, &c.name));
                }
            }
        }
        if out.len() == before {
            return Err(match data {
                Some(d) => format!(
                    "`columns`: {d} is a data column; only calculated columns are imported alone \
                     (import its table with `tables`)"
                ),
                None => no_match("calculated column", &sel),
            });
        }
    }
    for sel in list(p, "functions")? {
        let before = out.len();
        out.extend(
            functions(src)
                .iter()
                .enumerate()
                .filter(|(_, f)| matches(&sel, function_name(f)))
                .map(|(i, _)| Obj::Function(i)),
        );
        if out.len() == before {
            return Err(no_match("user-defined function", &sel));
        }
    }
    for sel in list(p, "expressions")? {
        let before = out.len();
        out.extend(
            src.expressions
                .iter()
                .enumerate()
                .filter(|(_, e)| matches(&sel, &e.name))
                .map(|(i, _)| Obj::Expression(i)),
        );
        if out.len() == before {
            return Err(no_match("shared expression", &sel));
        }
    }
    for sel in list(p, "roles")? {
        let before = out.len();
        out.extend(
            src.roles
                .iter()
                .enumerate()
                .filter(|(_, r)| matches(&sel, &r.name))
                .map(|(i, _)| Obj::Role(i)),
        );
        if out.len() == before {
            return Err(no_match("role", &sel));
        }
    }
    Ok(out)
}

/// `visuals`: `"Page/visual"` strings or `{page, visual}` objects.
fn visual_selectors(p: &Value) -> Result<Vec<(String, String)>, String> {
    let items = match p.get("visuals") {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(a)) => a.clone(),
        Some(v @ (Value::String(_) | Value::Object(_))) => vec![v.clone()],
        Some(_) => {
            return Err(
                "`visuals` must be a list of \"Page/visual\" strings or {page, visual} objects".into(),
            );
        }
    };
    if items.len() > MAX_SELECTORS {
        return Err(format!("`visuals` lists more than {MAX_SELECTORS} selectors"));
    }
    let mut out = Vec::new();
    for v in items {
        let (page, visual) = match &v {
            Value::String(s) => match s.rsplit_once('/') {
                Some((p, v)) => (p.to_string(), v.to_string()),
                None => return Err(format!("`visuals`: `{s}` must be \"Page/visual\"")),
            },
            Value::Object(o) => (
                o.get("page")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                o.get("visual")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            ),
            _ => {
                return Err(
                    "`visuals` must be a list of \"Page/visual\" strings or {page, visual} objects".into(),
                );
            }
        };
        if page.is_empty() || visual.is_empty() {
            return Err("`visuals`: every selector needs a page and a visual".into());
        }
        if page.chars().count() > MAX_PATTERN || visual.chars().count() > MAX_PATTERN {
            return Err(format!(
                "`visuals`: a selector is longer than {MAX_PATTERN} characters"
            ));
        }
        out.push((page, visual));
    }
    Ok(out)
}

fn bookmark_display(doc: &Value) -> &str {
    doc.get("displayName").and_then(Value::as_str).unwrap_or_default()
}

fn select_report(src: Option<&Report>, p: &Value) -> Result<Vec<RObj>, String> {
    let pages = list(p, "pages")?;
    let visuals = visual_selectors(p)?;
    let bookmarks = list(p, "bookmarks")?;
    if pages.is_empty() && visuals.is_empty() && bookmarks.is_empty() {
        return Ok(Vec::new());
    }
    let r = src.ok_or("the source has no report to import pages, visuals or bookmarks from")?;
    let mut out = Vec::new();
    for sel in pages {
        let before = out.len();
        out.extend(
            r.pages
                .iter()
                .enumerate()
                .filter(|(_, pg)| matches(&sel, &pg.name) || matches(&sel, &pg.display_name))
                .map(|(i, _)| RObj::Page(i)),
        );
        if out.len() == before {
            return Err(no_match("page", &sel));
        }
    }
    for (page, visual) in visuals {
        let pi =
            crate::report::page_index(r, &page).map_err(|e| format!("`visuals`: {e} (in the source)"))?;
        let pg = r.pages.get(pi).ok_or("internal: page index out of range")?;
        let picked: Vec<usize> = pg
            .visuals
            .iter()
            .enumerate()
            .filter(|(_, v)| matches(&visual, &v.name))
            .map(|(i, _)| i)
            .collect();
        if picked.is_empty() {
            return Err(no_match("visual", &format!("{page}/{visual}")));
        }
        // groups come with their members, members with their groups
        let mut set: BTreeSet<usize> = picked.into_iter().collect();
        for _ in 0..pg.visuals.len().min(1000) {
            let before = set.len();
            let names: HashSet<String> = set
                .iter()
                .filter_map(|&i| pg.visuals.get(i).map(|v| v.name.clone()))
                .collect();
            let parents: HashSet<String> = set
                .iter()
                .filter_map(|&i| pg.visuals.get(i).and_then(|v| v.parent_group.clone()))
                .collect();
            for (i, v) in pg.visuals.iter().enumerate() {
                if parents.contains(&v.name) || v.parent_group.as_ref().is_some_and(|g| names.contains(g)) {
                    set.insert(i);
                }
            }
            if set.len() == before {
                break;
            }
        }
        out.extend(set.into_iter().map(|vi| RObj::Visual(pi, vi)));
    }
    for sel in bookmarks {
        let before = out.len();
        out.extend(
            r.bookmarks
                .iter()
                .enumerate()
                .filter(|(_, b)| matches(&sel, &b.name) || matches(&sel, bookmark_display(&b.document)))
                .map(|(i, _)| RObj::Bookmark(i)),
        );
        if out.len() == before {
            return Err(no_match("bookmark", &sel));
        }
    }
    let mut seen = HashSet::new();
    out.retain(|o| seen.insert(*o));
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Dependencies
// ---------------------------------------------------------------------------------------------

/// The source's DAX sites by owning object.
struct SourceIndex {
    sites: Vec<DaxSite>,
    by_obj: HashMap<Obj, Vec<usize>>,
    by_table: HashMap<usize, Vec<usize>>,
}

impl SourceIndex {
    fn new(m: &SemanticModel) -> Self {
        let sites = model_sites(m);
        let mut by_obj: HashMap<Obj, Vec<usize>> = HashMap::new();
        let mut by_table: HashMap<usize, Vec<usize>> = HashMap::new();
        for (i, s) in sites.iter().enumerate() {
            let obj = match s.owner() {
                SiteOwner::Measure { table, name } => table_pos(m, table).and_then(|ti| {
                    m.tables
                        .get(ti)
                        .and_then(|t| t.measures.iter().position(|x| eq(&x.name, name)))
                        .map(|mi| Obj::Measure(ti, mi))
                }),
                SiteOwner::Column { table, name } => table_pos(m, table).and_then(|ti| {
                    m.tables
                        .get(ti)
                        .and_then(|t| t.columns.iter().position(|c| eq(&c.name, name)))
                        .map(|ci| Obj::Column(ti, ci))
                }),
                SiteOwner::Table(t) | SiteOwner::CalculationGroup(t) => table_pos(m, t).map(Obj::Table),
                SiteOwner::RowFilter { role, .. } => role_pos(m, role).map(Obj::Role),
                SiteOwner::Function(name) => function_pos(m, name).map(Obj::Function),
            };
            let Some(obj) = obj else { continue };
            by_obj.entry(obj).or_default().push(i);
            if let Obj::Table(ti) | Obj::Measure(ti, _) | Obj::Column(ti, _) = obj {
                by_table.entry(ti).or_default().push(i);
            }
        }
        Self {
            sites,
            by_obj,
            by_table,
        }
    }
}

/// Calls of the user-defined functions of `m` in a DAX text: `name(` with a function's name.
fn function_calls(m: &SemanticModel, text: &str) -> Vec<usize> {
    if functions(m).is_empty() {
        return Vec::new();
    }
    let tokens = tokenize(text);
    let mut out = Vec::new();
    for w in tokens.windows(2) {
        if let [a, b] = w
            && let (TokenKind::Ident(name), TokenKind::LParen) = (&a.kind, &b.kind)
            && let Some(i) = function_pos(m, name)
            && !out.contains(&i)
        {
            out.push(i);
        }
    }
    out
}

/// What a resolved DAX reference of the source needs.
fn need_of_ref(m: &SemanticModel, r: &ObjectRef) -> Option<Need> {
    match r {
        ObjectRef::Measure { name, .. } => measure_pos(m, name).map(|(t, x)| Need::Obj(Obj::Measure(t, x))),
        ObjectRef::Column { table, name } => {
            let ti = table_pos(m, table)?;
            let t = m.tables.get(ti)?;
            let ci = t.columns.iter().position(|c| eq(&c.name, name))?;
            Some(if t.columns.get(ci)?.expression.is_some() {
                Need::Obj(Obj::Column(ti, ci))
            } else {
                Need::DataColumn(ti, ci)
            })
        }
        ObjectRef::Table { name } => table_pos(m, name).map(|t| Need::Obj(Obj::Table(t))),
        ObjectRef::RowFilter { .. } => None,
    }
}

/// The free names of M texts that are queries of the source (shared expressions, M tables).
fn m_needs(m: &SemanticModel, texts: Vec<String>) -> Vec<Need> {
    if texts.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for a in analyze_all(texts, &Limits::default()) {
        for r in a.references {
            let need = if let Some(i) = expression_pos(m, &r.name) {
                Need::Obj(Obj::Expression(i))
            } else if let Some(ti) = table_pos(m, &r.name)
                && m.tables
                    .get(ti)
                    .is_some_and(|t| t.partitions.iter().any(|p| is_m(&p.source_type)))
            {
                Need::Obj(Obj::Table(ti))
            } else {
                continue;
            };
            if !out.contains(&need) {
                out.push(need);
            }
        }
    }
    out
}

fn needs_of(m: &SemanticModel, idx: &SourceIndex, renamer: &Renamer<'_>, o: Obj) -> Vec<Need> {
    let site_ids: &[usize] = match o {
        Obj::Table(ti) => idx.by_table.get(&ti).map_or(&[], Vec::as_slice),
        _ => idx.by_obj.get(&o).map_or(&[], Vec::as_slice),
    };
    let mut out: Vec<Need> = Vec::new();
    let mut push = |n: Need| {
        if !out.contains(&n) {
            out.push(n);
        }
    };
    for &i in site_ids {
        let Some(s) = idx.sites.get(i) else { continue };
        for r in renamer.references(s.text(), s.site()).unwrap_or_default() {
            if let Some(n) = need_of_ref(m, &r) {
                push(n);
            }
        }
        for f in function_calls(m, s.text()) {
            push(Need::Obj(Obj::Function(f)));
        }
    }
    let mut m_texts = Vec::new();
    match o {
        Obj::Table(ti) => {
            if let Some(t) = m.tables.get(ti) {
                for p in &t.partitions {
                    if is_m(&p.source_type)
                        && let Some(e) = &p.expression
                    {
                        m_texts.push(e.clone());
                    }
                }
                // auto date/time: the local date tables the columns' variations point at
                for c in &t.columns {
                    for v in c.variations() {
                        if let Some((table, _)) = &v.default_hierarchy
                            && let Some(dt) = table_pos(m, table)
                        {
                            push(Need::Obj(Obj::Table(dt)));
                        }
                    }
                }
            }
        }
        Obj::Expression(i) => {
            if let Some(e) = m.expressions.get(i)
                && is_m(&e.kind)
            {
                m_texts.push(e.expression.clone());
            }
        }
        Obj::Role(i) => {
            if let Some(r) = m.roles.get(i) {
                for tp in &r.table_permissions {
                    if let Some(t) = table_pos(m, &tp.table) {
                        push(Need::Obj(Obj::Table(t)));
                    }
                }
            }
        }
        _ => {}
    }
    for n in m_needs(m, m_texts) {
        push(n);
    }
    // a table needs nothing of its own
    out.retain(|n| match (o, n) {
        (
            Obj::Table(ti),
            Need::Obj(Obj::Table(t) | Obj::Measure(t, _) | Obj::Column(t, _)) | Need::DataColumn(t, _),
        ) => *t != ti,
        _ => true,
    });
    out
}

/// The model references of report JSON and typed bindings, as needs of the source.
fn report_needs(m: &SemanticModel, docs: &[&Value], fields: &[&FieldRef]) -> Vec<Need> {
    let mut out: Vec<Need> = Vec::new();
    let mut add = |kind: SiteKind, table: &str, name: &str| {
        let r = match kind {
            SiteKind::Table => ObjectRef::Table { name: table.into() },
            SiteKind::Column => ObjectRef::Column {
                table: table.into(),
                name: name.into(),
            },
            SiteKind::Measure => ObjectRef::Measure {
                table: table.into(),
                name: name.into(),
            },
        };
        if let Some(n) = need_of_ref(m, &r)
            && !out.contains(&n)
        {
            out.push(n);
        }
    };
    for f in fields {
        match f {
            FieldRef::Column { table, column } | FieldRef::Aggregation { table, column, .. } => {
                add(SiteKind::Column, table, column);
            }
            FieldRef::Measure { table, measure } => add(SiteKind::Measure, table, measure),
            FieldRef::Other { .. } => {}
        }
    }
    for d in docs {
        find_refs(d, &mut |s| {
            if !s.extension {
                add(s.kind, s.entity, s.property);
            }
        });
    }
    out
}

/// The verbatim JSON of a page (with its visuals), a visual or a bookmark.
fn robj_docs<'r>(r: &'r Report, o: RObj) -> (Vec<&'r Value>, Vec<&'r FieldRef>) {
    let mut docs: Vec<&Value> = Vec::new();
    let mut fields: Vec<&FieldRef> = Vec::new();
    let mut visual = |v: &'r bicraft_report::Visual, docs: &mut Vec<&'r Value>| {
        docs.extend(v.filter_config.as_ref());
        docs.extend(v.visual_extra.values());
        docs.extend(v.extra.values());
        for b in &v.fields {
            match &b.field {
                FieldRef::Other { raw } => docs.push(raw),
                f => fields.push(f),
            }
        }
    };
    match o {
        RObj::Page(pi) => {
            if let Some(p) = r.pages.get(pi) {
                docs.extend(p.filter_config.as_ref());
                docs.extend(p.extra.values());
                for v in &p.visuals {
                    visual(v, &mut docs);
                }
            }
        }
        RObj::Visual(pi, vi) => {
            if let Some(v) = r.pages.get(pi).and_then(|p| p.visuals.get(vi)) {
                visual(v, &mut docs);
            }
        }
        RObj::Bookmark(bi) => {
            if let Some(b) = r.bookmarks.get(bi) {
                docs.push(&b.document);
            }
        }
    }
    (docs, fields)
}

/// Whether the target already has what `need` names (by name); then the source object is not
/// imported and the import uses the target's.
fn in_target(src: &SemanticModel, tgt: &SemanticModel, need: Need) -> bool {
    let tname = |ti: usize| src.tables.get(ti).map(|t| t.name.as_str()).unwrap_or_default();
    match need {
        Need::Obj(Obj::Table(ti)) => tgt.table(tname(ti)).is_some(),
        Need::Obj(Obj::Measure(ti, mi)) => src
            .tables
            .get(ti)
            .and_then(|t| t.measures.get(mi))
            .is_some_and(|x| tgt.measure(&x.name).is_some()),
        Need::Obj(Obj::Column(ti, ci)) | Need::DataColumn(ti, ci) => {
            let cname = src
                .tables
                .get(ti)
                .and_then(|t| t.columns.get(ci))
                .map(|c| c.name.as_str())
                .unwrap_or_default();
            tgt.table(tname(ti)).is_some_and(|t| t.column(cname).is_some())
        }
        Need::Obj(Obj::Expression(i)) => src
            .expressions
            .get(i)
            .is_some_and(|e| expression_pos(tgt, &e.name).is_some() || tgt.table(&e.name).is_some()),
        Need::Obj(Obj::Function(i)) => functions(src)
            .get(i)
            .is_some_and(|f| function_pos(tgt, function_name(f)).is_some()),
        Need::Obj(Obj::Role(i)) => src.roles.get(i).is_some_and(|r| role_pos(tgt, &r.name).is_some()),
    }
}

/// The object the import adds for `need` (`None`: the target has it, or nothing can be added).
fn import_for(src: &SemanticModel, tgt: &SemanticModel, need: Need) -> Option<Obj> {
    if in_target(src, tgt, need) {
        return None;
    }
    let tname = |ti: usize| src.tables.get(ti).map(|t| t.name.as_str()).unwrap_or_default();
    match need {
        // a calculated column of a table the target lacks comes with its table
        Need::Obj(Obj::Column(ti, _)) | Need::DataColumn(ti, _) if tgt.table(tname(ti)).is_none() => {
            Some(Obj::Table(ti))
        }
        // a data column missing from a table the target has: nothing to import (reported broken)
        Need::DataColumn(..) => None,
        Need::Obj(o) => Some(o),
    }
}

struct Closure {
    plan: BTreeMap<Obj, Reason>,
    /// `{kind, name, table?, required_by, from: "source"|"target"}`
    dependencies: Vec<Value>,
}

fn close(
    src: &SemanticModel,
    tgt: &SemanticModel,
    selected: &[Obj],
    report_needs_list: &[(String, Vec<Need>)],
    include: bool,
) -> Closure {
    let idx = SourceIndex::new(src);
    let renamer = Renamer::new(src);
    let mut plan: BTreeMap<Obj, Reason> = BTreeMap::new();
    let mut queue: VecDeque<Obj> = VecDeque::new();
    let mut dependencies = Vec::new();
    for o in selected {
        if plan.insert(*o, Reason::Selected).is_none() {
            queue.push_back(*o);
        }
    }
    let visit = |needs: Vec<Need>,
                 by: &str,
                 plan: &mut BTreeMap<Obj, Reason>,
                 queue: &mut VecDeque<Obj>,
                 dependencies: &mut Vec<Value>| {
        for n in needs {
            match import_for(src, tgt, n) {
                Some(d) => {
                    if let std::collections::btree_map::Entry::Vacant(e) = plan.entry(d) {
                        e.insert(Reason::Dependency(by.to_string()));
                        queue.push_back(d);
                        let mut v = describe(src, d);
                        v["required_by"] = json!(by);
                        v["from"] = json!("source");
                        dependencies.push(v);
                    }
                }
                None => {
                    if let Need::Obj(o) = n
                        && in_target(src, tgt, n)
                    {
                        let mut v = describe(src, o);
                        v["required_by"] = json!(by);
                        v["from"] = json!("target");
                        if !dependencies.contains(&v) {
                            dependencies.push(v);
                        }
                    }
                }
            }
        }
    };
    if include {
        for (by, needs) in report_needs_list {
            visit(needs.clone(), by, &mut plan, &mut queue, &mut dependencies);
        }
        while let Some(o) = queue.pop_front() {
            let needs = needs_of(src, &idx, &renamer, o);
            let by = label(src, o);
            visit(needs, &by, &mut plan, &mut queue, &mut dependencies);
        }
    }
    // measures and columns of an imported table come with it
    let tables: HashSet<usize> = plan
        .keys()
        .filter_map(|o| match o {
            Obj::Table(t) => Some(*t),
            _ => None,
        })
        .collect();
    plan.retain(|o, _| !matches!(o, Obj::Measure(t, _) | Obj::Column(t, _) if tables.contains(t)));
    dependencies.retain(|d| {
        d["from"] != "source"
            || !matches!(d["kind"].as_str(), Some("measure" | "calculated column"))
            || d["table"]
                .as_str()
                .and_then(|t| table_pos(src, t))
                .is_none_or(|t| !tables.contains(&t))
    });
    Closure { plan, dependencies }
}

// ---------------------------------------------------------------------------------------------
// Conflicts
// ---------------------------------------------------------------------------------------------

/// What happens to a target object an import replaces.
#[derive(Debug, Default)]
struct Removals {
    tables: Vec<String>,
    expressions: Vec<String>,
    measures: Vec<String>,
    /// (table, column)
    columns: Vec<(String, String)>,
    functions: Vec<String>,
    roles: Vec<String>,
}

/// A free name: `base + suffix`, then `base + suffix + " 2"`, … that `taken` rejects.
fn free_name(base: &str, suffix: &str, taken: &dyn Fn(&str) -> bool) -> Result<String, String> {
    for n in 1..=MAX_RENAME_TRIES {
        let candidate = if n == 1 {
            format!("{base}{suffix}")
        } else {
            format!("{base}{suffix} {n}")
        };
        crate::edit::check_name("renamed object", &candidate)?;
        if !taken(&candidate) {
            return Ok(candidate);
        }
    }
    Err(format!("no free name found for `{base}`"))
}

fn names_of<'a>(it: impl Iterator<Item = &'a str>) -> HashSet<String> {
    it.map(str::to_lowercase).collect()
}

// ---------------------------------------------------------------------------------------------
// The command
// ---------------------------------------------------------------------------------------------

struct Ctx {
    mode: Mode,
    suffix: String,
    objects: Vec<Value>,
    conflicts: Vec<Value>,
    warnings: Vec<String>,
    errors: Vec<String>,
}

impl Ctx {
    fn conflict(&mut self, what: Value, with: String) {
        let mut v = what;
        v["conflicts_with"] = json!(with);
        self.conflicts.push(v);
    }
}

fn mode_param(p: &Value) -> Result<Mode, String> {
    Ok(match opt_str(p, "on_conflict")? {
        None | Some("fail") => Mode::Fail,
        Some("skip") => Mode::Skip,
        Some("replace") => Mode::Replace,
        Some("rename") => Mode::Rename,
        Some(o) => {
            return Err(format!(
                "unknown on_conflict `{o}` (expected fail, skip, replace or rename)"
            ));
        }
    })
}

fn listed(v: Vec<Value>) -> Value {
    let n = v.len();
    let mut v = v;
    v.truncate(MAX_LISTED);
    if n > MAX_LISTED {
        v.push(json!({"more": n - MAX_LISTED}));
    }
    Value::Array(v)
}

fn check_imported_name(kind: &str, name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err(format!(
            "the source has a {kind} without a name; nothing was imported"
        ));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!(
            "the source's {kind} `{}…` has a name longer than {MAX_NAME_CHARS} characters; nothing was imported",
            name.chars().take(40).collect::<String>()
        ));
    }
    Ok(())
}

pub(crate) fn objects_import(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, PARAMS)?;
    let source = str_param(p, "source")?.to_string();
    let mode = mode_param(p)?;
    let suffix = opt_str(p, "rename_suffix")?.unwrap_or(DEFAULT_SUFFIX).to_string();
    if suffix.is_empty() || suffix.chars().count() > 100 || suffix.chars().any(char::is_control) {
        return Err("rename_suffix must be 1 to 100 characters without control characters".into());
    }
    let include = opt_bool(p, "include_dependencies")?.unwrap_or(true);
    let with_relationships = opt_bool(p, "relationships")?.unwrap_or(true);
    let allow_broken = opt_bool(p, "allow_broken")?.unwrap_or(false);
    let dry_run = opt_bool(p, "dry_run")?.unwrap_or(false);
    let theme = opt_bool(p, "theme")?.unwrap_or(false);
    let into_table = opt_str(p, "into_table")?.map(str::to_string);
    let target_page = opt_str(p, "target_page")?.map(str::to_string);
    let tgt = crate::commands::model(s)?.clone();

    // the source, in a private session (server mode: it resolves `source` itself)
    let mut private = s.private();
    private
        .execute("file.open", &json!({"path": source}))
        .map_err(|e| format!("cannot open the source {source}: {e}"))?;
    // edits on the private copy must not be refused because of the source's container
    private.source = None;
    let src0 = private
        .model
        .clone()
        .ok_or_else(|| format!("the source {source} has no readable model"))?;

    let objs = select_model(&src0, p)?;
    let robjs = select_report(private.report.as_ref(), p)?;
    let sel = Selection { objs, robjs };
    if sel.objs.is_empty() && sel.robjs.is_empty() && !theme {
        return Err(
            "nothing to import: name measures, tables, columns, calculation_groups, functions, \
                    expressions, roles, pages, visuals, bookmarks or theme"
                .into(),
        );
    }
    let visual_count = sel.robjs.iter().filter(|o| matches!(o, RObj::Visual(..))).count();
    if visual_count > 0 && target_page.is_none() {
        return Err("`visuals` needs `target_page`: the target page the visuals are added to".into());
    }
    if !sel.robjs.is_empty() || theme {
        crate::report::report_ref(s).map_err(|e| format!("the target has no report to import into: {e}"))?;
    }

    // the report's needs
    let mut rneeds: Vec<(String, Vec<Need>)> = Vec::new();
    if let Some(r) = private.report.as_ref() {
        for o in &sel.robjs {
            let (docs, fields) = robj_docs(r, *o);
            let by = robj_label(r, *o);
            rneeds.push((by, report_needs(&src0, &docs, &fields)));
        }
    }
    let closure = close(&src0, &tgt, &sel.objs, &rneeds, include);

    let mut ctx = Ctx {
        mode,
        suffix,
        objects: Vec::new(),
        conflicts: Vec::new(),
        warnings: Vec::new(),
        errors: Vec::new(),
    };

    for o in closure.plan.keys() {
        let kind = describe(&src0, *o)["kind"]
            .as_str()
            .unwrap_or("object")
            .to_string();
        check_imported_name(&kind, &object_name(&src0, *o))?;
        if let Obj::Table(ti) = o
            && let Some(t) = src0.tables.get(*ti)
        {
            for c in &t.columns {
                check_imported_name("column", &c.name)?;
            }
            for x in &t.measures {
                check_imported_name("measure", &x.name)?;
            }
        }
    }

    // conflicts, decided on the source's names before any rename
    let decided = decide(&src0, &tgt, &closure.plan, into_table.as_deref(), &mut ctx)?;

    // renames and moves on the private copy
    apply_renames(&mut private, &src0, &decided)?;
    let src = private
        .model
        .clone()
        .ok_or("internal: the private model went away")?;

    if !decided.expressions_added.is_empty()
        && let Err(e) = crate::partition::refuse_on_pbix(s, "importing a shared expression")
    {
        return Err(e);
    }

    // the new target model
    let mut next = tgt.clone();
    let mut regenerated = 0usize;
    build_model(&src, &mut next, &decided, &mut ctx, &mut regenerated)?;
    let relationships = if with_relationships {
        import_relationships(&src, &mut next, &decided, &mut ctx)
    } else {
        Vec::new()
    };
    prune_relationships(&mut next, &mut ctx);

    // the report
    let mut next_report: Option<Report> = None;
    let mut broken_fields: Vec<Value> = Vec::new();
    if !sel.robjs.is_empty() || theme {
        let src_report = private.report.as_ref();
        let target = crate::report::report_ref(s)?.clone();
        let (r, broken) = build_report(
            &private,
            src_report,
            target,
            &sel.robjs,
            theme,
            target_page.as_deref(),
            &next,
            &mut ctx,
            s,
        )?;
        broken_fields = broken;
        next_report = Some(r);
    }

    // checks
    let broken_dax = newly_broken(&tgt, &next, None);
    if !broken_dax.is_empty() {
        let names: Vec<String> = broken_dax
            .iter()
            .take(20)
            .map(|b| {
                format!(
                    "{} {}",
                    b["kind"].as_str().unwrap_or_default(),
                    dax_column(
                        b["table"].as_str().unwrap_or_default(),
                        b["name"].as_str().unwrap_or_default()
                    )
                )
            })
            .collect();
        let msg = format!(
            "{} DAX expression(s) would not validate in the target: {}",
            broken_dax.len(),
            names.join("; ")
        );
        if allow_broken {
            ctx.warnings.push(format!("{msg} (allow_broken)"));
        } else {
            ctx.errors.push(msg);
        }
    }
    if !broken_fields.is_empty() {
        let names: Vec<String> = broken_fields
            .iter()
            .take(20)
            .map(|b| b["field"].as_str().unwrap_or_default().to_string())
            .collect();
        let msg = format!(
            "{} report field reference(s) of the imported report objects do not resolve in the target model: {}",
            broken_fields.len(),
            names.join(", ")
        );
        if allow_broken {
            ctx.warnings.push(format!("{msg} (allow_broken)"));
        } else {
            ctx.errors.push(msg);
        }
    }
    if mode == Mode::Fail && !ctx.conflicts.is_empty() {
        let names: Vec<String> = ctx
            .conflicts
            .iter()
            .take(20)
            .map(|c| c["conflicts_with"].as_str().unwrap_or_default().to_string())
            .collect();
        ctx.errors.insert(
            0,
            format!(
                "{} object(s) already exist in the target: {} (on_conflict: skip, replace or rename)",
                ctx.conflicts.len(),
                names.join("; ")
            ),
        );
    }

    let mut out = Map::new();
    out.insert("source".into(), json!(source));
    out.insert("dry_run".into(), json!(dry_run));
    out.insert("objects".into(), listed(ctx.objects));
    out.insert("dependencies".into(), listed(closure.dependencies));
    out.insert("relationships".into(), listed(relationships));
    out.insert("conflicts".into(), listed(ctx.conflicts));
    out.insert("lineage_tags_regenerated".into(), json!(regenerated));
    out.insert("broken".into(), listed(broken_dax));
    out.insert("broken_fields".into(), listed(broken_fields));
    out.insert("warnings".into(), json!(ctx.warnings));
    if dry_run {
        out.insert("applied".into(), json!(false));
        out.insert("would_apply".into(), json!(ctx.errors.is_empty()));
        out.insert("errors".into(), json!(ctx.errors));
        return Ok(Value::Object(out));
    }
    if !ctx.errors.is_empty() {
        return Err(format!("nothing was imported: {}", ctx.errors.join("; ")));
    }
    // commit
    let model_changed = next != tgt;
    if model_changed {
        s.model = Some(next);
        s.model_error = None;
        s.dirty = true;
    }
    if let Some(r) = next_report
        && s.report.as_ref() != Some(&r)
    {
        s.report = Some(r);
        s.report_dirty = true;
    }
    out.insert("applied".into(), json!(true));
    Ok(Value::Object(out))
}

fn robj_label(r: &Report, o: RObj) -> String {
    match o {
        RObj::Page(pi) => format!(
            "page `{}`",
            r.pages
                .get(pi)
                .map(|p| p.display_name.as_str())
                .unwrap_or_default()
        ),
        RObj::Visual(pi, vi) => {
            let p = r.pages.get(pi);
            format!(
                "visual `{}` of page `{}`",
                p.and_then(|p| p.visuals.get(vi))
                    .map(|v| v.name.as_str())
                    .unwrap_or_default(),
                p.map(|p| p.display_name.as_str()).unwrap_or_default()
            )
        }
        RObj::Bookmark(bi) => format!(
            "bookmark `{}`",
            r.bookmarks
                .get(bi)
                .map(|b| bookmark_display(&b.document).to_string())
                .unwrap_or_default()
        ),
    }
}

// ---------------------------------------------------------------------------------------------
// Deciding
// ---------------------------------------------------------------------------------------------

/// The plan after conflicts: what is added, under which name, and what the target loses.
#[derive(Debug, Default)]
struct Decided {
    /// Source tables to add (position) → replaces the target table of this name.
    tables: Vec<(usize, Option<String>)>,
    /// Standalone measures: (source position, destination table).
    measures: Vec<((usize, usize), String)>,
    /// Measures of imported tables left out (skip).
    dropped_measures: HashSet<(usize, usize)>,
    columns: Vec<(usize, usize)>,
    functions: Vec<usize>,
    expressions_added: Vec<usize>,
    roles: Vec<usize>,
    removals: Removals,
    /// Source object → its new name (rename).
    renames: Vec<(Obj, String)>,
    /// Imported table names as they end up.
    table_names: HashSet<String>,
}

fn entry(src: &SemanticModel, o: Obj, reason: Option<&Reason>, action: &str) -> Value {
    let mut v = describe(src, o);
    v["action"] = json!(action);
    match reason {
        Some(Reason::Selected) => v["reason"] = json!("selected"),
        Some(Reason::Dependency(by)) => {
            v["reason"] = json!("dependency");
            v["required_by"] = json!(by);
        }
        None => v["reason"] = json!("with its table"),
    }
    v
}

#[allow(clippy::too_many_lines)]
fn decide(
    src: &SemanticModel,
    tgt: &SemanticModel,
    plan: &BTreeMap<Obj, Reason>,
    into_table: Option<&str>,
    ctx: &mut Ctx,
) -> Result<Decided, String> {
    let mut d = Decided::default();
    let mode = ctx.mode;
    let suffix = ctx.suffix.clone();
    let mut chosen: HashSet<String> = HashSet::new();
    let src_queries = names_of(
        src.tables
            .iter()
            .map(|t| t.name.as_str())
            .chain(src.expressions.iter().map(|e| e.name.as_str())),
    );
    let tgt_queries = names_of(
        tgt.tables
            .iter()
            .map(|t| t.name.as_str())
            .chain(tgt.expressions.iter().map(|e| e.name.as_str())),
    );
    let renamed = |o: Obj,
                   base: &str,
                   taken: &dyn Fn(&str) -> bool,
                   chosen: &mut HashSet<String>,
                   d: &mut Decided|
     -> Result<String, String> {
        let name = free_name(base, &suffix, &|n| taken(n) || chosen.contains(&n.to_lowercase()))?;
        chosen.insert(name.to_lowercase());
        d.renames.push((o, name.clone()));
        Ok(name)
    };

    // tables
    for (o, reason) in plan {
        let Obj::Table(ti) = *o else { continue };
        let Some(t) = src.tables.get(ti) else { continue };
        let clash = tgt
            .table(&t.name)
            .map(|x| format!("table `{}`", x.name))
            .or_else(|| expression_pos(tgt, &t.name).map(|_| format!("shared expression `{}`", t.name)));
        let Some(clash) = clash else {
            d.tables.push((ti, None));
            d.table_names.insert(t.name.to_lowercase());
            ctx.objects.push(entry(src, *o, Some(reason), "added"));
            continue;
        };
        ctx.conflict(describe(src, *o), clash.clone());
        match mode {
            Mode::Fail | Mode::Skip => ctx.objects.push(entry(src, *o, Some(reason), "skipped")),
            Mode::Replace => {
                if tgt.table(&t.name).is_none() {
                    return Err(format!(
                        "table `{}` cannot replace the target's {clash} (only a table replaces a table)",
                        t.name
                    ));
                }
                d.tables.push((ti, Some(t.name.clone())));
                d.removals.tables.push(t.name.clone());
                d.table_names.insert(t.name.to_lowercase());
                ctx.objects.push(entry(src, *o, Some(reason), "replaced"));
            }
            Mode::Rename => {
                let taken = |n: &str| {
                    let l = n.to_lowercase();
                    src_queries.contains(&l) || tgt_queries.contains(&l)
                };
                let name = renamed(*o, &t.name, &taken, &mut chosen, &mut d)?;
                d.tables.push((ti, None));
                d.table_names.insert(name.to_lowercase());
                let mut v = entry(src, *o, Some(reason), "renamed");
                v["source_name"] = json!(t.name);
                v["name"] = json!(name);
                ctx.objects.push(v);
            }
        }
    }
    // shared expressions
    for (o, reason) in plan {
        let Obj::Expression(i) = *o else { continue };
        let Some(e) = src.expressions.get(i) else { continue };
        let clash = expression_pos(tgt, &e.name)
            .map(|_| format!("shared expression `{}`", e.name))
            .or_else(|| {
                tgt.table(&e.name)
                    .filter(|t| !d.removals.tables.iter().any(|r| eq(r, &t.name)))
                    .map(|t| format!("table `{}`", t.name))
            })
            .or_else(|| {
                d.table_names
                    .contains(&e.name.to_lowercase())
                    .then(|| format!("imported table `{}`", e.name))
            });
        let Some(clash) = clash else {
            d.expressions_added.push(i);
            ctx.objects.push(entry(src, *o, Some(reason), "added"));
            continue;
        };
        ctx.conflict(describe(src, *o), clash.clone());
        match mode {
            Mode::Fail | Mode::Skip => ctx.objects.push(entry(src, *o, Some(reason), "skipped")),
            Mode::Replace => {
                if expression_pos(tgt, &e.name).is_none() {
                    return Err(format!(
                        "shared expression `{}` cannot replace the target's {clash}",
                        e.name
                    ));
                }
                d.removals.expressions.push(e.name.clone());
                d.expressions_added.push(i);
                ctx.objects.push(entry(src, *o, Some(reason), "replaced"));
            }
            Mode::Rename => {
                let taken = |n: &str| {
                    let l = n.to_lowercase();
                    src_queries.contains(&l) || tgt_queries.contains(&l)
                };
                let name = renamed(*o, &e.name, &taken, &mut chosen, &mut d)?;
                d.expressions_added.push(i);
                let mut v = entry(src, *o, Some(reason), "renamed");
                v["source_name"] = json!(e.name);
                v["name"] = json!(name);
                ctx.objects.push(v);
            }
        }
    }
    // measures: standalone ones, then those of imported tables (model-wide unique names)
    let removed_table = |name: &str, d: &Decided| d.removals.tables.iter().any(|r| eq(r, name));
    let mut measures: Vec<(Obj, Option<&Reason>, String)> = Vec::new();
    for (o, reason) in plan {
        if let Obj::Measure(ti, _) = *o {
            let home = src.tables.get(ti).map(|t| t.name.clone()).unwrap_or_default();
            let dest = into_table.map_or(home, str::to_string);
            measures.push((*o, Some(reason), dest));
        }
    }
    for &(ti, _) in &d.tables.clone() {
        if let Some(t) = src.tables.get(ti) {
            for mi in 0..t.measures.len() {
                measures.push((Obj::Measure(ti, mi), None, t.name.clone()));
            }
        }
    }
    let src_measures = names_of(
        src.tables
            .iter()
            .flat_map(|t| t.measures.iter().map(|x| x.name.as_str())),
    );
    let tgt_measures: HashSet<String> = tgt
        .tables
        .iter()
        .filter(|t| !removed_table(&t.name, &d))
        .flat_map(|t| t.measures.iter().map(|x| x.name.to_lowercase()))
        .collect();
    for (o, reason, dest) in measures {
        let Obj::Measure(ti, mi) = o else { continue };
        let Some(x) = src.tables.get(ti).and_then(|t| t.measures.get(mi)) else {
            continue;
        };
        let standalone = reason.is_some();
        if standalone {
            let dest_ok = tgt.table(&dest).is_some() || d.table_names.contains(&dest.to_lowercase());
            if !dest_ok {
                ctx.errors.push(format!(
                    "measure {} has no home in the target: it has no table `{dest}` (pass into_table, \
                     or import the table)",
                    dax_measure(&x.name)
                ));
                continue;
            }
        }
        let column_clash = standalone && tgt.table(&dest).and_then(|t| t.column(&x.name)).is_some();
        let measure_clash = tgt_measures.contains(&x.name.to_lowercase());
        if !column_clash && !measure_clash {
            if standalone {
                d.measures.push(((ti, mi), dest.clone()));
                let mut v = entry(src, o, reason, "added");
                if !eq(
                    &dest,
                    src.tables.get(ti).map(|t| t.name.as_str()).unwrap_or_default(),
                ) {
                    v["into_table"] = json!(dest);
                }
                ctx.objects.push(v);
            }
            continue;
        }
        let clash = if column_clash {
            format!("column {} of the target", dax_column(&dest, &x.name))
        } else {
            let (t, m) = tgt
                .measure(&x.name)
                .map(|(t, m)| (t.name.clone(), m.name.clone()))
                .unwrap_or_default();
            format!("measure {} (table `{t}`) of the target", dax_measure(&m))
        };
        ctx.conflict(describe(src, o), clash.clone());
        match mode {
            Mode::Fail | Mode::Skip => {
                if !standalone {
                    d.dropped_measures.insert((ti, mi));
                }
                ctx.objects.push(entry(src, o, reason, "skipped"));
            }
            Mode::Replace => {
                if column_clash {
                    return Err(format!(
                        "measure {} cannot replace the target's {clash}",
                        dax_measure(&x.name)
                    ));
                }
                d.removals.measures.push(x.name.clone());
                if standalone {
                    d.measures.push(((ti, mi), dest.clone()));
                }
                ctx.objects.push(entry(src, o, reason, "replaced"));
            }
            Mode::Rename => {
                let dest_cols: HashSet<String> = tgt
                    .table(&dest)
                    .into_iter()
                    .chain(src.table(&dest))
                    .flat_map(|t| t.columns.iter().map(|c| c.name.to_lowercase()))
                    .collect();
                let taken = |n: &str| {
                    let l = n.to_lowercase();
                    src_measures.contains(&l) || tgt_measures.contains(&l) || dest_cols.contains(&l)
                };
                let name = renamed(o, &x.name, &taken, &mut chosen, &mut d)?;
                if standalone {
                    d.measures.push(((ti, mi), dest.clone()));
                }
                let mut v = entry(src, o, reason, "renamed");
                v["source_name"] = json!(x.name);
                v["name"] = json!(name);
                ctx.objects.push(v);
            }
        }
    }
    // calculated columns into existing tables
    for (o, reason) in plan {
        let Obj::Column(ti, ci) = *o else { continue };
        let Some(t) = src.tables.get(ti) else { continue };
        let Some(c) = t.columns.get(ci) else { continue };
        let Some(dest) = tgt.table(&t.name) else {
            ctx.errors.push(format!(
                "calculated column {} has no home: the target has no table `{}`",
                dax_column(&t.name, &c.name),
                t.name
            ));
            continue;
        };
        let clash = dest
            .column(&c.name)
            .map(|x| format!("column {} of the target", dax_column(&dest.name, &x.name)))
            .or_else(|| {
                dest.measure(&c.name)
                    .map(|x| format!("measure {} of the target", dax_measure(&x.name)))
            });
        let Some(clash) = clash else {
            d.columns.push((ti, ci));
            ctx.objects.push(entry(src, *o, Some(reason), "added"));
            continue;
        };
        ctx.conflict(describe(src, *o), clash.clone());
        match mode {
            Mode::Fail | Mode::Skip => ctx.objects.push(entry(src, *o, Some(reason), "skipped")),
            Mode::Replace => {
                if dest.column(&c.name).is_none() {
                    return Err(format!(
                        "calculated column {} cannot replace the target's {clash}",
                        dax_column(&t.name, &c.name)
                    ));
                }
                d.removals.columns.push((dest.name.clone(), c.name.clone()));
                d.columns.push((ti, ci));
                ctx.objects.push(entry(src, *o, Some(reason), "replaced"));
            }
            Mode::Rename => {
                let names: HashSet<String> = [Some(dest), src.tables.get(ti)]
                    .into_iter()
                    .flatten()
                    .flat_map(|t| {
                        t.columns
                            .iter()
                            .map(|c| c.name.to_lowercase())
                            .chain(t.measures.iter().map(|m| m.name.to_lowercase()))
                    })
                    .collect();
                let name = renamed(
                    *o,
                    &c.name,
                    &|n| names.contains(&n.to_lowercase()),
                    &mut chosen,
                    &mut d,
                )?;
                d.columns.push((ti, ci));
                let mut v = entry(src, *o, Some(reason), "renamed");
                v["source_name"] = json!(c.name);
                v["name"] = json!(name);
                ctx.objects.push(v);
            }
        }
    }
    // functions and roles
    for (o, reason) in plan {
        let (name, clash) = match *o {
            Obj::Function(i) => {
                let n = functions(src)
                    .get(i)
                    .map(function_name)
                    .unwrap_or_default()
                    .to_string();
                let c = function_pos(tgt, &n).map(|_| format!("function `{n}` of the target"));
                (n, c)
            }
            Obj::Role(i) => {
                let n = src.roles.get(i).map(|r| r.name.clone()).unwrap_or_default();
                let c = role_pos(tgt, &n).map(|_| format!("role `{n}` of the target"));
                (n, c)
            }
            _ => continue,
        };
        let add = |d: &mut Decided| match *o {
            Obj::Function(i) => d.functions.push(i),
            Obj::Role(i) => d.roles.push(i),
            _ => {}
        };
        let Some(clash) = clash else {
            add(&mut d);
            ctx.objects.push(entry(src, *o, Some(reason), "added"));
            continue;
        };
        ctx.conflict(describe(src, *o), clash.clone());
        match mode {
            Mode::Fail | Mode::Skip => ctx.objects.push(entry(src, *o, Some(reason), "skipped")),
            Mode::Replace => {
                match *o {
                    Obj::Function(_) => d.removals.functions.push(name.clone()),
                    _ => d.removals.roles.push(name.clone()),
                }
                add(&mut d);
                ctx.objects.push(entry(src, *o, Some(reason), "replaced"));
            }
            Mode::Rename => {
                let names: HashSet<String> = match *o {
                    Obj::Function(_) => names_of(
                        functions(src)
                            .iter()
                            .chain(functions(tgt).iter())
                            .map(function_name),
                    ),
                    _ => names_of(src.roles.iter().chain(tgt.roles.iter()).map(|r| r.name.as_str())),
                };
                let new = renamed(
                    *o,
                    &name,
                    &|n| names.contains(&n.to_lowercase()),
                    &mut chosen,
                    &mut d,
                )?;
                add(&mut d);
                let mut v = entry(src, *o, Some(reason), "renamed");
                v["source_name"] = json!(name);
                v["name"] = json!(new);
                ctx.objects.push(v);
            }
        }
    }
    Ok(d)
}

/// The DAX text of `text` with every call of function `old` renamed to `new`.
pub(crate) fn rename_calls(text: &str, old: &str, new: &str) -> Option<String> {
    let tokens = tokenize(text);
    let mut spans = Vec::new();
    for w in tokens.windows(2) {
        if let [a, b] = w
            && let (TokenKind::Ident(name), TokenKind::LParen) = (&a.kind, &b.kind)
            && eq(name, old)
        {
            spans.push((a.span.start, a.span.end));
        }
    }
    if spans.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0usize;
    for (s, e) in spans {
        out.push_str(text.get(at..s)?);
        out.push_str(new);
        at = e;
    }
    out.push_str(text.get(at..)?);
    Some(out)
}

/// Runs the renames (and `into_table` moves) on the private copy of the source, so DAX,
/// relationships and report bindings of the source follow them.
fn apply_renames(private: &mut Session, src0: &SemanticModel, d: &Decided) -> Result<(), String> {
    // tables first: later renames name objects by their table
    let mut order: Vec<&(Obj, String)> = d.renames.iter().collect();
    order.sort_by_key(|(o, _)| !matches!(o, Obj::Table(_)));
    for (o, new) in order {
        let cur = private.model.as_ref().ok_or("internal: no private model")?;
        let r = match *o {
            Obj::Table(_) => private.execute(
                "table.rename",
                &json!({"name": object_name(cur, *o), "new_name": new, "update_report": true}),
            ),
            Obj::Measure(..) => private.execute(
                "measure.rename",
                &json!({"name": object_name(cur, *o), "new_name": new, "update_report": true}),
            ),
            Obj::Column(ti, _) => private.execute(
                "column.rename",
                &json!({"table": cur.tables.get(ti).map(|t| t.name.clone()), "name": object_name(cur, *o),
                        "new_name": new, "update_report": true}),
            ),
            Obj::Expression(_) => private.execute(
                "expression.rename",
                &json!({"name": object_name(cur, *o), "new_name": new}),
            ),
            Obj::Function(i) => {
                let old = object_name(cur, *o);
                let m = private.model.as_mut().ok_or("internal: no private model")?;
                rewrite_sites(m, |s| rename_calls(s.text(), &old, new));
                if let Some(f) = m
                    .extra
                    .get_mut("functions")
                    .and_then(|f| f.get_mut(i))
                    .and_then(Value::as_object_mut)
                {
                    f.insert("name".into(), json!(new));
                }
                Ok(Value::Null)
            }
            Obj::Role(i) => {
                if let Some(r) = private.model.as_mut().and_then(|m| m.roles.get_mut(i)) {
                    r.name.clone_from(new);
                }
                Ok(Value::Null)
            }
        };
        r.map_err(|e| format!("renaming the source's {} to `{new}` failed: {e}", label(src0, *o)))?;
    }
    // into_table: `Old[Measure]` qualifiers follow the move
    for ((ti, mi), dest) in &d.measures {
        let m = private.model.as_mut().ok_or("internal: no private model")?;
        let Some(t) = m.tables.get(*ti) else { continue };
        if eq(&t.name, dest) {
            continue;
        }
        let Some(x) = t.measures.get(*mi) else { continue };
        let target = ObjectRef::Measure {
            table: t.name.clone(),
            name: x.name.clone(),
        };
        let change = ModelChange::MeasureMoved {
            measure: x.name.clone(),
            from: t.name.clone(),
            to: dest.clone(),
        };
        rename_in_model(m, &target, &DaxChange::MoveMeasure(dest.clone()));
        if let Some(r) = private.report.as_mut() {
            apply_change(r, &change, true);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Building the target
// ---------------------------------------------------------------------------------------------

/// Every lineage tag held in `v` (a `lineageTag` key at any depth).
fn json_tags(v: &Value, out: &mut HashSet<String>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    match v {
        Value::Object(o) => {
            for (k, x) in o {
                if k == "lineageTag"
                    && let Some(t) = x.as_str()
                {
                    out.insert(t.to_lowercase());
                } else {
                    json_tags(x, out, depth.saturating_add(1));
                }
            }
        }
        Value::Array(a) => {
            for x in a {
                json_tags(x, out, depth.saturating_add(1));
            }
        }
        _ => {}
    }
}

fn model_tags(m: &SemanticModel) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut add = |t: &Option<String>| {
        if let Some(t) = t {
            out.insert(t.to_lowercase());
        }
    };
    for t in &m.tables {
        add(&t.lineage_tag);
        for c in &t.columns {
            add(&c.lineage_tag);
        }
        for x in &t.measures {
            add(&x.lineage_tag);
        }
    }
    for e in &m.expressions {
        add(&e.lineage_tag);
    }
    for t in &m.tables {
        for v in t.extra.values() {
            json_tags(v, &mut out, 0);
        }
    }
    if let Some(f) = m.extra.get("functions") {
        json_tags(f, &mut out, 0);
    }
    out
}

/// Gives `tag` a new value when the target already has it.
fn fresh_tag(tag: &mut Option<String>, taken: &mut HashSet<String>, n: &mut usize) {
    if let Some(t) = tag {
        if taken.contains(&t.to_lowercase()) {
            let g = new_guid();
            taken.insert(g.clone());
            *t = g;
            *n = n.saturating_add(1);
        } else {
            taken.insert(t.to_lowercase());
        }
    }
}

fn fresh_json_tags(v: &mut Value, taken: &mut HashSet<String>, n: &mut usize, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    match v {
        Value::Object(o) => {
            for (k, x) in o.iter_mut() {
                if k == "lineageTag" {
                    if let Value::String(t) = x {
                        let mut tag = Some(t.clone());
                        fresh_tag(&mut tag, taken, n);
                        if let Some(t2) = tag {
                            *t = t2;
                        }
                    }
                } else {
                    fresh_json_tags(x, taken, n, depth.saturating_add(1));
                }
            }
        }
        Value::Array(a) => {
            for x in a {
                fresh_json_tags(x, taken, n, depth.saturating_add(1));
            }
        }
        _ => {}
    }
}

fn fresh_table_tags(t: &mut Table, taken: &mut HashSet<String>, n: &mut usize) {
    fresh_tag(&mut t.lineage_tag, taken, n);
    for c in &mut t.columns {
        fresh_tag(&mut c.lineage_tag, taken, n);
    }
    for x in &mut t.measures {
        fresh_tag(&mut x.lineage_tag, taken, n);
    }
    for v in t.extra.values_mut() {
        fresh_json_tags(v, taken, n, 0);
    }
}

fn build_model(
    src: &SemanticModel,
    next: &mut SemanticModel,
    d: &Decided,
    ctx: &mut Ctx,
    regenerated: &mut usize,
) -> Result<(), String> {
    let r = &d.removals;
    // removals (tables are replaced in place below)
    next.expressions
        .retain(|e| !r.expressions.iter().any(|n| eq(n, &e.name)));
    for t in &mut next.tables {
        t.measures.retain(|x| !r.measures.iter().any(|n| eq(n, &x.name)));
        t.columns.retain(|c| {
            !r.columns
                .iter()
                .any(|(tn, cn)| eq(tn, &t.name) && eq(cn, &c.name))
        });
    }
    if let Some(Value::Array(f)) = next.extra.get_mut("functions") {
        f.retain(|f| !r.functions.iter().any(|n| eq(n, function_name(f))));
    }
    next.roles.retain(|x| !r.roles.iter().any(|n| eq(n, &x.name)));
    let replaced: Vec<String> = r.tables.clone();
    let mut taken = model_tags(next);
    // tags of replaced tables go away with them
    for name in &replaced {
        if let Some(t) = next.table(name) {
            let gone = model_tags(&SemanticModel {
                tables: vec![t.clone()],
                ..Default::default()
            });
            for g in gone {
                taken.remove(&g);
            }
        }
    }
    for (ti, replaces) in &d.tables {
        let mut t = src
            .tables
            .get(*ti)
            .cloned()
            .ok_or("internal: table index out of range")?;
        let mut mi = 0usize;
        t.measures.retain(|_| {
            let keep = !d.dropped_measures.contains(&(*ti, mi));
            mi = mi.saturating_add(1);
            keep
        });
        fresh_table_tags(&mut t, &mut taken, regenerated);
        match replaces.as_deref().and_then(|n| table_pos(next, n)) {
            Some(i) => {
                if let Some(slot) = next.tables.get_mut(i) {
                    *slot = t;
                }
            }
            None => next.tables.push(t),
        }
    }
    for ((ti, mi), dest) in &d.measures {
        let mut x = src
            .tables
            .get(*ti)
            .and_then(|t| t.measures.get(*mi))
            .cloned()
            .ok_or("internal: measure index out of range")?;
        fresh_tag(&mut x.lineage_tag, &mut taken, regenerated);
        let Some(i) = table_pos(next, dest) else {
            ctx.errors.push(format!(
                "the target has no table `{dest}` for measure {}",
                dax_measure(&x.name)
            ));
            continue;
        };
        if let Some(t) = next.tables.get_mut(i) {
            t.measures.push(x);
        }
    }
    for (ti, ci) in &d.columns {
        let st = src.tables.get(*ti).ok_or("internal: table index out of range")?;
        let mut c = st
            .columns
            .get(*ci)
            .cloned()
            .ok_or("internal: column index out of range")?;
        fresh_tag(&mut c.lineage_tag, &mut taken, regenerated);
        if c.sort_by_column
            .as_deref()
            .is_some_and(|s| next.table(&st.name).is_some_and(|t| t.column(s).is_none()))
        {
            ctx.warnings.push(format!(
                "{}: its sort-by column is not in the target table and was dropped",
                dax_column(&st.name, &c.name)
            ));
            c.sort_by_column = None;
        }
        if let Some(t) = table_pos(next, &st.name).and_then(|i| next.tables.get_mut(i)) {
            t.columns.push(c);
        }
    }
    for i in &d.expressions_added {
        let mut e = src
            .expressions
            .get(*i)
            .cloned()
            .ok_or("internal: expression index out of range")?;
        fresh_tag(&mut e.lineage_tag, &mut taken, regenerated);
        next.expressions.push(e);
    }
    if !d.functions.is_empty() {
        let mut added = Vec::new();
        for i in &d.functions {
            let mut f = functions(src)
                .get(*i)
                .cloned()
                .ok_or("internal: function index out of range")?;
            fresh_json_tags(&mut f, &mut taken, regenerated, 0);
            added.push(f);
        }
        match next.extra.get_mut("functions") {
            Some(Value::Array(a)) => a.extend(added),
            _ => {
                next.extra.insert("functions".into(), Value::Array(added));
            }
        }
    }
    for i in &d.roles {
        let mut role = src
            .roles
            .get(*i)
            .cloned()
            .ok_or("internal: role index out of range")?;
        role.table_permissions.retain(|tp| {
            let keep = next.table(&tp.table).is_some();
            if !keep {
                ctx.warnings.push(format!(
                    "role `{}`: its permission on table `{}` was dropped (no such table in the target)",
                    role.name, tp.table
                ));
            }
            keep
        });
        next.roles.push(role);
    }
    Ok(())
}

fn rel_key(t1: &str, c1: &str, t2: &str, c2: &str) -> (String, String, String, String) {
    (
        t1.to_lowercase(),
        c1.to_lowercase(),
        t2.to_lowercase(),
        c2.to_lowercase(),
    )
}

fn has_column(m: &SemanticModel, t: &str, c: &str) -> bool {
    m.table(t).is_some_and(|t| t.column(c).is_some())
}

/// The source's relationships that touch an imported table and whose both ends exist.
fn import_relationships(
    src: &SemanticModel,
    next: &mut SemanticModel,
    d: &Decided,
    ctx: &mut Ctx,
) -> Vec<Value> {
    let mut out = Vec::new();
    let imported = |t: &str| d.table_names.contains(&t.to_lowercase());
    let mut renamed: Vec<(String, String)> = Vec::new();
    for r in &src.relationships {
        if !imported(&r.from_table) && !imported(&r.to_table) {
            continue;
        }
        if !has_column(next, &r.from_table, &r.from_column) || !has_column(next, &r.to_table, &r.to_column) {
            continue;
        }
        let k = rel_key(&r.from_table, &r.from_column, &r.to_table, &r.to_column);
        let k2 = rel_key(&r.to_table, &r.to_column, &r.from_table, &r.from_column);
        if next.relationships.iter().any(|x| {
            let kx = rel_key(&x.from_table, &x.from_column, &x.to_table, &x.to_column);
            kx == k || kx == k2
        }) {
            continue;
        }
        let mut rel = r.clone();
        if next.relationships.iter().any(|x| eq(&x.name, &rel.name)) || rel.name.is_empty() {
            let new = new_guid();
            renamed.push((rel.name.clone(), new.clone()));
            rel.name = new;
        }
        let pair = |a: &str, b: &str| {
            let (a, b) = (a.to_lowercase(), b.to_lowercase());
            if a <= b { (a, b) } else { (b, a) }
        };
        let this = pair(&rel.from_table, &rel.to_table);
        if rel.is_active
            && next
                .relationships
                .iter()
                .any(|x| x.is_active && pair(&x.from_table, &x.to_table) == this)
        {
            rel.is_active = false;
            ctx.warnings.push(format!(
                "relationship {} → {} imported inactive: the target already has an active relationship \
                 between these tables",
                dax_column(&rel.from_table, &rel.from_column),
                dax_column(&rel.to_table, &rel.to_column)
            ));
        }
        out.push(
            json!({"name": rel.name, "from": dax_column(&rel.from_table, &rel.from_column),
            "to": dax_column(&rel.to_table, &rel.to_column), "active": rel.is_active}),
        );
        next.relationships.push(rel);
    }
    // column variations name their relationship: follow the new names
    if !renamed.is_empty() {
        for t in &mut next.tables {
            if !imported(&t.name) {
                continue;
            }
            for c in &mut t.columns {
                if let Some(Value::Array(vs)) = c.extra.get_mut("variations") {
                    for v in vs {
                        if let Some(Value::String(rn)) = v.get_mut("relationship")
                            && let Some((_, new)) = renamed.iter().find(|(o, _)| o == rn)
                        {
                            rn.clone_from(new);
                        }
                    }
                }
            }
        }
    }
    out
}

/// Drops the target relationships whose columns a replacement removed.
fn prune_relationships(next: &mut SemanticModel, ctx: &mut Ctx) {
    let before = next.relationships.len();
    let mut dropped = Vec::new();
    let keep: Vec<bool> = next
        .relationships
        .iter()
        .map(|r| {
            has_column(next, &r.from_table, &r.from_column) && has_column(next, &r.to_table, &r.to_column)
        })
        .collect();
    let mut i = 0usize;
    next.relationships.retain(|r| {
        let k = keep.get(i).copied().unwrap_or(true);
        i = i.saturating_add(1);
        if !k {
            dropped.push(format!(
                "{} → {}",
                dax_column(&r.from_table, &r.from_column),
                dax_column(&r.to_table, &r.to_column)
            ));
        }
        k
    });
    if next.relationships.len() != before {
        ctx.warnings.push(format!(
            "relationships removed because a replaced object no longer has their columns: {}",
            dropped.join(", ")
        ));
    }
}

// ---------------------------------------------------------------------------------------------
// The report
// ---------------------------------------------------------------------------------------------

/// Replaces string values and object keys equal to a key of `map`.
fn rewrite_ids(v: &mut Value, map: &HashMap<String, String>, depth: usize) {
    if depth > MAX_DEPTH || map.is_empty() {
        return;
    }
    match v {
        Value::String(s) => {
            if let Some(n) = map.get(s.as_str()) {
                s.clone_from(n);
            }
        }
        Value::Array(a) => {
            for x in a {
                rewrite_ids(x, map, depth.saturating_add(1));
            }
        }
        Value::Object(o) => {
            if o.keys().any(|k| map.contains_key(k)) {
                let old = std::mem::take(o);
                for (k, x) in old {
                    let k = map.get(&k).cloned().unwrap_or(k);
                    o.insert(k, x);
                }
            }
            for x in o.values_mut() {
                rewrite_ids(x, map, depth.saturating_add(1));
            }
        }
        _ => {}
    }
}

/// `(package, item)` of every `ResourcePackageItem` in `v`.
fn resource_items(v: &Value, out: &mut Vec<(String, String)>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    match v {
        Value::Object(o) => {
            if let Some(item) = o.get("ResourcePackageItem")
                && let (Some(p), Some(i)) = (
                    item.get("PackageName").and_then(Value::as_str),
                    item.get("ItemName").and_then(Value::as_str),
                )
            {
                let k = (p.to_string(), i.to_string());
                if !out.contains(&k) {
                    out.push(k);
                }
            }
            for x in o.values() {
                resource_items(x, out, depth.saturating_add(1));
            }
        }
        Value::Array(a) => {
            for x in a {
                resource_items(x, out, depth.saturating_add(1));
            }
        }
        _ => {}
    }
}

/// Renames `ItemName`s of `ResourcePackageItem`s in `v`.
fn rename_items(v: &mut Value, map: &HashMap<(String, String), String>, depth: usize) {
    if depth > MAX_DEPTH || map.is_empty() {
        return;
    }
    match v {
        Value::Object(o) => {
            if let Some(Value::Object(item)) = o.get_mut("ResourcePackageItem") {
                let key = (
                    item.get("PackageName")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    item.get("ItemName")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                );
                if let Some(n) = map.get(&key) {
                    item.insert("ItemName".into(), json!(n));
                }
            }
            for x in o.values_mut() {
                rename_items(x, map, depth.saturating_add(1));
            }
        }
        Value::Array(a) => {
            for x in a {
                rename_items(x, map, depth.saturating_add(1));
            }
        }
        _ => {}
    }
}

/// The resource packages of a report (PBIR `{name, items}`, or the legacy wrapper).
fn package<'a>(r: &'a Report, name: &str) -> Option<&'a Value> {
    r.extra
        .get("resourcePackages")
        .and_then(Value::as_array)?
        .iter()
        .map(|p| p.get("resourcePackage").unwrap_or(p))
        .find(|p| p.get("name").and_then(Value::as_str).is_some_and(|n| eq(n, name)))
}

fn package_item<'a>(pkg: &'a Value, item: &str) -> Option<&'a Value> {
    pkg.get("items")?
        .as_array()?
        .iter()
        .find(|i| i.get("name").and_then(Value::as_str).is_some_and(|n| eq(n, item)))
}

fn resource_of<'a>(r: &'a Report, pkg: &str, path: &str) -> Option<&'a Resource> {
    let want = format!(
        "staticresources/{}/{}",
        pkg.to_lowercase(),
        path.to_lowercase().replace('\\', "/")
    );
    r.resources
        .iter()
        .find(|x| x.path.to_lowercase().replace('\\', "/") == want)
}

/// Copies the registered images (and other package items) the imported JSON names. Returns the
/// item renames to apply to the imported documents.
fn copy_resources(
    src: &Report,
    next: &mut Report,
    wanted: &[(String, String)],
    ctx: &mut Ctx,
) -> HashMap<(String, String), String> {
    let mut renames = HashMap::new();
    for (pkg_name, item_name) in wanted {
        let Some(item) = package(src, pkg_name).and_then(|p| package_item(p, item_name)) else {
            continue;
        };
        let path = item
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or(item_name)
            .to_string();
        let Some(res) = resource_of(src, pkg_name, &path) else {
            ctx.warnings.push(format!(
                "resource `{item_name}` of package `{pkg_name}` is missing from the source"
            ));
            continue;
        };
        let existing = package(next, pkg_name)
            .and_then(|p| package_item(p, item_name))
            .cloned();
        let mut new_name = item_name.clone();
        let mut new_path = path.clone();
        if let Some(ex) = existing {
            let ex_path = ex.get("path").and_then(Value::as_str).unwrap_or(item_name);
            if resource_of(next, pkg_name, ex_path).is_some_and(|x| x.bytes == res.bytes) {
                continue; // the same file is already there
            }
            let (stem, ext) = match item_name.rsplit_once('.') {
                Some((s, e)) => (s.to_string(), format!(".{e}")),
                None => (item_name.clone(), String::new()),
            };
            let mut n = 2usize;
            loop {
                let cand = format!("{stem}_{n}{ext}");
                if package(next, pkg_name)
                    .and_then(|p| package_item(p, &cand))
                    .is_none()
                    && resource_of(next, pkg_name, &cand).is_none()
                {
                    new_name.clone_from(&cand);
                    new_path = cand;
                    break;
                }
                n = n.saturating_add(1);
                if n > MAX_RENAME_TRIES {
                    break;
                }
            }
            renames.insert((pkg_name.clone(), item_name.clone()), new_name.clone());
        }
        let mut new_item = item.clone();
        if let Some(o) = new_item.as_object_mut() {
            o.insert("name".into(), json!(new_name));
            o.insert("path".into(), json!(new_path));
        }
        // register it
        let packages = next
            .extra
            .entry("resourcePackages")
            .or_insert_with(|| Value::Array(Vec::new()));
        if !packages.is_array() {
            *packages = Value::Array(Vec::new());
        }
        if let Value::Array(list) = packages {
            let found = list.iter_mut().find_map(|p| {
                let p = if p.get("resourcePackage").is_some() {
                    p.get_mut("resourcePackage")?
                } else {
                    p
                };
                p.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|n| eq(n, pkg_name))
                    .then_some(p)
            });
            match found {
                Some(p) => match p.get_mut("items") {
                    Some(Value::Array(items)) => items.push(new_item),
                    _ => {
                        if let Some(o) = p.as_object_mut() {
                            o.insert("items".into(), Value::Array(vec![new_item]));
                        }
                    }
                },
                None => {
                    let mut pkg = package(src, pkg_name)
                        .cloned()
                        .unwrap_or_else(|| json!({"name": pkg_name}));
                    if let Some(o) = pkg.as_object_mut() {
                        o.insert("items".into(), Value::Array(vec![new_item]));
                    }
                    list.push(pkg);
                }
            }
        }
        next.resources.push(Resource {
            path: format!("StaticResources/{pkg_name}/{new_path}"),
            bytes: res.bytes.clone(),
        });
    }
    renames
}

/// Why a report reference of an imported object does not resolve in `m`.
fn field_problems(
    m: &SemanticModel,
    report: &Report,
    docs: &[&Value],
    fields: &[&FieldRef],
    at: &str,
    out: &mut Vec<Value>,
) {
    let ext = crate::report_measures::report_measures(report);
    let mut push = |field: String, reason: String| {
        let v = json!({"at": at, "field": field, "reason": reason});
        if !out.contains(&v) {
            out.push(v);
        }
    };
    for f in fields {
        if let Some(reason) = check_field(m, f) {
            let field = match f {
                FieldRef::Column { table, column } | FieldRef::Aggregation { table, column, .. } => {
                    dax_column(table, column)
                }
                FieldRef::Measure { table, measure } => dax_column(table, measure),
                FieldRef::Other { .. } => String::new(),
            };
            push(field, reason);
        }
    }
    for d in docs {
        find_refs(d, &mut |s| {
            if s.extension {
                if s.kind == SiteKind::Measure
                    && !ext
                        .iter()
                        .any(|x| eq(&x.entity, s.entity) && eq(&x.name, s.property))
                {
                    push(
                        dax_column(s.entity, s.property),
                        "no report-level measure of this name in the target report".into(),
                    );
                }
                return;
            }
            if let Some(reason) = check_site(m, s.kind, s.entity, s.property) {
                let field = if s.kind == SiteKind::Table {
                    s.entity.to_string()
                } else {
                    dax_column(s.entity, s.property)
                };
                push(field, reason);
            }
        });
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn build_report(
    private: &Session,
    src: Option<&Report>,
    mut next: Report,
    robjs: &[RObj],
    theme: bool,
    target_page: Option<&str>,
    model: &SemanticModel,
    ctx: &mut Ctx,
    s: &Session,
) -> Result<(Report, Vec<Value>), String> {
    let mode = ctx.mode;
    let suffix = ctx.suffix.clone();
    let mut page_ids: HashMap<String, String> = HashMap::new();
    // imported objects in `next`: page positions, (page, visual name), bookmark positions
    let mut new_pages: Vec<usize> = Vec::new();
    let mut new_visuals: Vec<(usize, String)> = Vec::new();
    let mut new_bookmarks: Vec<usize> = Vec::new();
    let mut wanted_resources = Vec::new();
    let empty = Report::default();
    let src = src.unwrap_or(&empty);

    for o in robjs {
        let RObj::Page(pi) = *o else { continue };
        let Some(pg) = src.pages.get(pi) else { continue };
        let mut page = pg.clone();
        let desc = json!({"kind": "page", "name": pg.display_name, "page": pg.name});
        if let Some(other) = next.pages.iter().find(|p| eq(&p.display_name, &pg.display_name)) {
            ctx.conflict(
                desc.clone(),
                format!("page `{}` of the target", other.display_name),
            );
            match mode {
                Mode::Fail | Mode::Skip => {
                    let mut v = desc;
                    v["action"] = json!("skipped");
                    ctx.objects.push(v);
                    continue;
                }
                Mode::Replace => {
                    let old = other.name.clone();
                    next.pages.retain(|p| p.name != old);
                    let mut v = desc;
                    v["action"] = json!("replaced");
                    ctx.objects.push(v);
                }
                Mode::Rename => {
                    let names: HashSet<String> = next
                        .pages
                        .iter()
                        .chain(src.pages.iter())
                        .map(|p| p.display_name.to_lowercase())
                        .collect();
                    page.display_name =
                        free_name(&pg.display_name, &suffix, &|n| names.contains(&n.to_lowercase()))?;
                    let mut v = desc;
                    v["action"] = json!("renamed");
                    v["source_name"] = json!(pg.display_name);
                    v["name"] = json!(page.display_name);
                    ctx.objects.push(v);
                }
            }
        } else {
            let mut v = desc;
            v["action"] = json!("added");
            ctx.objects.push(v);
        }
        if next.pages.iter().any(|p| eq(&p.name, &page.name)) {
            let taken: HashSet<String> = next.pages.iter().map(|p| p.name.to_lowercase()).collect();
            let fresh = crate::report::new_object_name(|n| taken.contains(&n.to_lowercase()));
            page_ids.insert(page.name.clone(), fresh.clone());
            page.name = fresh;
        }
        for d in robj_docs(src, *o).0 {
            resource_items(d, &mut wanted_resources, 0);
        }
        next.pages.push(page);
        new_pages.push(next.pages.len().saturating_sub(1));
    }
    // pages that point at each other (drillthrough, tooltips) follow the new ids
    for &i in &new_pages {
        if let Some(p) = next.pages.get_mut(i) {
            for v in p.extra.values_mut() {
                rewrite_ids(v, &page_ids, 0);
            }
            for vis in &mut p.visuals {
                for v in vis.visual_extra.values_mut().chain(vis.extra.values_mut()) {
                    rewrite_ids(v, &page_ids, 0);
                }
            }
        }
    }
    // visuals onto the target page
    let visuals: Vec<(usize, usize)> = robjs
        .iter()
        .filter_map(|o| match o {
            RObj::Visual(p, v) => Some((*p, *v)),
            _ => None,
        })
        .collect();
    if !visuals.is_empty() {
        let key = target_page.ok_or("`visuals` needs `target_page`")?;
        let ti = crate::report::page_index(&next, key).map_err(|e| format!("target_page: {e}"))?;
        let mut names: HashMap<String, String> = HashMap::new();
        let mut added = Vec::new();
        for (pi, vi) in &visuals {
            let Some(v) = src.pages.get(*pi).and_then(|p| p.visuals.get(*vi)) else {
                continue;
            };
            let page = next.pages.get(ti).ok_or("internal: page index out of range")?;
            let mut v = v.clone();
            if page.visuals.iter().any(|x| eq(&x.name, &v.name))
                || added
                    .iter()
                    .any(|x: &bicraft_report::Visual| eq(&x.name, &v.name))
            {
                let taken: HashSet<String> = page
                    .visuals
                    .iter()
                    .chain(added.iter())
                    .map(|x| x.name.to_lowercase())
                    .collect();
                let fresh = crate::report::new_object_name(|n| taken.contains(&n.to_lowercase()));
                names.insert(v.name.clone(), fresh.clone());
                v.name = fresh;
            }
            for d in robj_docs(src, RObj::Visual(*pi, *vi)).0 {
                resource_items(d, &mut wanted_resources, 0);
            }
            added.push(v);
        }
        for v in &mut added {
            if let Some(g) = v.parent_group.as_ref().and_then(|g| names.get(g)) {
                v.parent_group = Some(g.clone());
            }
            if let Some(warn) = next
                .pages
                .get(ti)
                .and_then(|p| crate::report::outside_page_warning(p, &v.position))
            {
                ctx.warnings.push(warn);
            }
            let page_name = next
                .pages
                .get(ti)
                .map(|p| p.display_name.clone())
                .unwrap_or_default();
            ctx.objects
                .push(json!({"kind": "visual", "name": v.name, "page": page_name,
                "visual_type": v.visual_type, "action": "added"}));
            new_visuals.push((ti, v.name.clone()));
        }
        if let Some(p) = next.pages.get_mut(ti) {
            p.visuals.extend(added);
        }
    }
    // bookmarks
    for o in robjs {
        let RObj::Bookmark(bi) = *o else { continue };
        let Some(b) = src.bookmarks.get(bi) else { continue };
        let mut b = b.clone();
        let display = bookmark_display(&b.document).to_string();
        let desc = json!({"kind": "bookmark", "name": display});
        if let Some(other) = next
            .bookmarks
            .iter()
            .find(|x| !display.is_empty() && eq(bookmark_display(&x.document), &display))
        {
            ctx.conflict(
                desc.clone(),
                format!("bookmark `{}` of the target", bookmark_display(&other.document)),
            );
            match mode {
                Mode::Fail | Mode::Skip => {
                    let mut v = desc;
                    v["action"] = json!("skipped");
                    ctx.objects.push(v);
                    continue;
                }
                Mode::Replace => {
                    let old = other.name.clone();
                    next.bookmarks.retain(|x| x.name != old);
                    let mut v = desc;
                    v["action"] = json!("replaced");
                    ctx.objects.push(v);
                }
                Mode::Rename => {
                    let names: HashSet<String> = next
                        .bookmarks
                        .iter()
                        .chain(src.bookmarks.iter())
                        .map(|x| bookmark_display(&x.document).to_lowercase())
                        .collect();
                    let new = free_name(&display, &suffix, &|n| names.contains(&n.to_lowercase()))?;
                    if let Some(o) = b.document.as_object_mut() {
                        o.insert("displayName".into(), json!(new));
                    }
                    let mut v = desc;
                    v["action"] = json!("renamed");
                    v["source_name"] = json!(display);
                    v["name"] = json!(new);
                    ctx.objects.push(v);
                }
            }
        } else {
            let mut v = desc;
            v["action"] = json!("added");
            ctx.objects.push(v);
        }
        rewrite_ids(&mut b.document, &page_ids, 0);
        if next.bookmarks.iter().any(|x| eq(&x.name, &b.name)) {
            let taken: HashSet<String> = next.bookmarks.iter().map(|x| x.name.to_lowercase()).collect();
            let fresh = crate::report::new_object_name(|n| taken.contains(&n.to_lowercase()));
            b.name.clone_from(&fresh);
            if let Some(o) = b.document.as_object_mut()
                && o.contains_key("name")
            {
                o.insert("name".into(), json!(fresh));
            }
        }
        if let Some(active) = b
            .document
            .get("explorationState")
            .and_then(|e| e.get("activeSection"))
            .and_then(Value::as_str)
            && !next.pages.iter().any(|p| p.name == active)
        {
            ctx.warnings.push(format!(
                "bookmark `{display}` shows page `{active}`, which is not in the target (import the page too)"
            ));
        }
        next.bookmarks.push(b);
        new_bookmarks.push(next.bookmarks.len().saturating_sub(1));
    }
    // registered images the imported objects show
    if !wanted_resources.is_empty() {
        let renames = copy_resources(src, &mut next, &wanted_resources, ctx);
        if !renames.is_empty() {
            for &i in &new_pages {
                if let Some(p) = next.pages.get_mut(i) {
                    for v in p.extra.values_mut() {
                        rename_items(v, &renames, 0);
                    }
                    for vis in &mut p.visuals {
                        for v in vis.visual_extra.values_mut().chain(vis.extra.values_mut()) {
                            rename_items(v, &renames, 0);
                        }
                    }
                }
            }
            for (pi, name) in &new_visuals {
                if let Some(vis) = next.pages.get_mut(*pi).and_then(|p| p.visual_mut(name)) {
                    for v in vis.visual_extra.values_mut().chain(vis.extra.values_mut()) {
                        rename_items(v, &renames, 0);
                    }
                }
            }
        }
    }
    // theme
    if theme {
        let got = private_theme(private)?;
        let has_custom = crate::report::report_ref(s)
            .ok()
            .and_then(|r| r.theme_collection.as_ref())
            .and_then(|t| t.get("customTheme"))
            .is_some();
        let name = got
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let desc = json!({"kind": "theme", "name": name});
        let install = if has_custom {
            ctx.conflict(desc.clone(), "the target's custom theme".into());
            matches!(mode, Mode::Replace | Mode::Rename)
        } else {
            true
        };
        let mut v = desc;
        if install {
            let mut scratch = s.private();
            scratch.report = Some(next);
            let r = scratch.execute("theme.set", &json!({"theme": got, "force": true}));
            next = scratch
                .report
                .take()
                .ok_or("internal: the scratch report went away")?;
            let r = r.map_err(|e| format!("installing the source's theme failed: {e}"))?;
            if let Some(w) = r.get("warnings").and_then(Value::as_array) {
                ctx.warnings
                    .extend(w.iter().filter_map(Value::as_str).map(|w| format!("theme: {w}")));
            }
            v["action"] = json!(if has_custom { "replaced" } else { "added" });
        } else {
            v["action"] = json!("skipped");
        }
        ctx.objects.push(v);
    }
    // field references of what was imported, against the new model
    let mut broken = Vec::new();
    for &i in &new_pages {
        let at = format!(
            "page `{}`",
            next.pages
                .get(i)
                .map(|p| p.display_name.as_str())
                .unwrap_or_default()
        );
        let (docs, fields) = robj_docs(&next, RObj::Page(i));
        field_problems(model, &next, &docs, &fields, &at, &mut broken);
    }
    for (pi, name) in &new_visuals {
        let Some(vi) = next
            .pages
            .get(*pi)
            .and_then(|p| p.visuals.iter().position(|v| &v.name == name))
        else {
            continue;
        };
        let at = format!("visual `{name}`");
        let (docs, fields) = robj_docs(&next, RObj::Visual(*pi, vi));
        field_problems(model, &next, &docs, &fields, &at, &mut broken);
    }
    for &i in &new_bookmarks {
        let at = format!(
            "bookmark `{}`",
            next.bookmarks
                .get(i)
                .map(|b| bookmark_display(&b.document))
                .unwrap_or_default()
        );
        let (docs, fields) = robj_docs(&next, RObj::Bookmark(i));
        field_problems(model, &next, &docs, &fields, &at, &mut broken);
    }
    if next.active_page.is_none() {
        next.active_page = next.pages.first().map(|p| p.name.clone());
    }
    Ok((next, broken))
}

/// The source's custom theme document.
fn private_theme(private: &Session) -> Result<Value, String> {
    if private.report.is_none() {
        return Err("the source has no report, so no theme to import".into());
    }
    let got = crate::theme_edit::theme_get(private, &json!({}))?;
    match got.get("custom").and_then(|c| c.get("theme")) {
        Some(t @ Value::Object(_)) => Ok(t.clone()),
        _ => Err(
            "the source has no readable custom theme to import (its base theme is built into Power BI)"
                .into(),
        ),
    }
}
