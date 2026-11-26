//! Data sources of the model's Power Query (M) and their rewrites (`sources.list`,
//! `sources.rebase`; `env.apply` in `envprofile.rs` uses the same plan).
//!
//! Sources are found by `bicraft_data::m::sources` on tokens, so every rewrite replaces one text
//! literal (or a parameter's value) by its byte range and leaves the rest of the M text byte for
//! byte. Only literals are rewritten: a literal argument, a step bound to a literal, the leading
//! literal of a `&` concatenation, a text parameter's value. Anything computed is reported and
//! never guessed.
//!
//! Paths compare the way the platform that reads them does: Windows drive and UNC paths without
//! regard to case and with `\` and `/` alike, POSIX paths exactly; a root matches whole folder
//! names only (`C:\data` is not a prefix of `C:\database`).

use std::collections::{BTreeMap, BTreeSet};

use bicraft_data::Limits;
use bicraft_data::m::sources::{
    Form, ParamInfo, ParamValue, Part, Role, SourceCall, TextLit, parameter_info, source_calls, text_literal,
};
use bicraft_data::m::{analyze_all, identifier};
use bicraft_model::{NamedExpression, SemanticModel};
use serde_json::{Value, json};

use crate::Session;
use crate::edit::{Edit, apply, new_guid};
use crate::mquery::{is_m, query_names};
use crate::params::{allow, opt_bool, opt_str};

/// Longest text shown for a computed argument.
const MAX_SHOWN: usize = 200;
/// Most entries in one mapping.
pub(crate) const MAX_MAPPINGS: usize = 10_000;

// ---------------------------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Style {
    /// `C:\…`, the drive letter lowercased.
    Drive(char),
    /// `\\server\share\…` or `//server/share/…`.
    Unc,
    /// `/…`.
    Posix,
}

#[derive(Debug, Clone)]
struct PathParts<'a> {
    style: Style,
    comps: Vec<&'a str>,
    trailing: bool,
}

fn is_sep(c: char) -> bool {
    c == '\\' || c == '/'
}

/// An absolute path's parts; `None` for relative paths, URLs and device paths (`\\?\…`).
fn parse_path(s: &str) -> Option<PathParts<'_>> {
    if s.contains("://") {
        return None;
    }
    let mut chars = s.chars();
    let (c0, c1, c2) = (chars.next(), chars.next(), chars.next());
    let (style, rest) = match (c0, c1, c2) {
        (Some(d), Some(':'), None) if d.is_ascii_alphabetic() => (Style::Drive(d.to_ascii_lowercase()), ""),
        (Some(d), Some(':'), Some(c)) if d.is_ascii_alphabetic() && is_sep(c) => {
            (Style::Drive(d.to_ascii_lowercase()), s.get(2..)?)
        }
        (Some(a), Some(b), c) if is_sep(a) && is_sep(b) => {
            if matches!(c, Some('?' | '.')) && s.chars().nth(3).is_some_and(is_sep) {
                return None;
            }
            (Style::Unc, s.get(2..)?)
        }
        (Some('/'), _, _) => (Style::Posix, s.get(1..)?),
        _ => return None,
    };
    let comps: Vec<&str> = rest.split(is_sep).filter(|c| !c.is_empty()).collect();
    if style == Style::Unc && comps.len() < 2 {
        return None;
    }
    Some(PathParts {
        style,
        comps,
        trailing: rest.ends_with(is_sep) && !rest.is_empty(),
    })
}

/// Whether two folder names are the same for the path's platform.
fn same_name(style: &Style, a: &str, b: &str) -> bool {
    match style {
        Style::Posix => a == b,
        _ => a == b || a.to_lowercase() == b.to_lowercase(),
    }
}

/// The parts of `path` below `root`, when `path` is `root` or inside it.
fn below<'p>(root: &PathParts<'_>, path: &PathParts<'p>) -> Option<Vec<&'p str>> {
    if root.style != path.style || root.comps.len() > path.comps.len() {
        return None;
    }
    for (a, b) in root.comps.iter().zip(&path.comps) {
        if !same_name(&root.style, a, b) {
            return None;
        }
    }
    Some(path.comps.get(root.comps.len()..)?.to_vec())
}

/// The separator a new root uses: its own, else `fallback`.
fn sep_of(root: &str, fallback: char) -> char {
    if root.contains('\\') {
        '\\'
    } else if root.contains('/') {
        '/'
    } else {
        fallback
    }
}

/// The separator a path uses (`\` unless it only has `/`).
fn own_sep(path: &str) -> char {
    if !path.contains('\\') && path.contains('/') {
        '/'
    } else {
        '\\'
    }
}

/// `root` without its trailing separators (a root that is only separators stays as it is).
fn trim_root(root: &str) -> &str {
    let t = root.trim_end_matches(is_sep);
    // `\\server\share\` must keep its leading pair; `/` stays `/`
    if t.is_empty() { root } else { t }
}

/// `X:` alone (which names the current folder of a drive, not its root).
fn is_bare_drive(s: &str) -> bool {
    let mut c = s.chars();
    matches!((c.next(), c.next(), c.next()), (Some(d), Some(':'), None) if d.is_ascii_alphabetic())
}

/// `root` with one trailing separator.
pub(crate) fn with_trailing_sep(root: &str) -> String {
    let sep = sep_of(root, '\\');
    let t = trim_root(root);
    if t.ends_with(is_sep) {
        t.to_string()
    } else {
        format!("{t}{sep}")
    }
}

/// A path root mapping (`C:\data` → `\\srv\share\data`).
#[derive(Debug, Clone)]
pub(crate) struct PathMap {
    pub from: String,
    pub to: String,
}

/// How a path is rewritten: the new path, or the parameter-relative remainder.
struct Rebased {
    /// The new path (also what a parameter-relative path evaluates to).
    value: String,
    /// Below the root, joined with the new separator (with a trailing one when the old path had
    /// one); empty for the root itself.
    rest: String,
}

/// Rewrites `path` by the longest matching root of `maps`.
fn rebase_path(path: &str, maps: &[PathMap]) -> Option<(usize, Rebased)> {
    let p = parse_path(path)?;
    let mut best: Option<(usize, usize, Vec<&str>)> = None;
    for (i, m) in maps.iter().enumerate() {
        let Some(root) = parse_path(&m.from) else {
            continue;
        };
        if let Some(rest) = below(&root, &p)
            && best.as_ref().is_none_or(|(_, n, _)| root.comps.len() > *n)
        {
            best = Some((i, root.comps.len(), rest));
        }
    }
    let (i, _, rest) = best?;
    let to = &maps.get(i)?.to;
    let sep = sep_of(to, own_sep(path));
    let mut tail = rest.join(&sep.to_string());
    if p.trailing && !tail.is_empty() {
        tail.push(sep);
    }
    let root = trim_root(to);
    let value = if tail.is_empty() {
        if p.trailing || is_bare_drive(root) {
            with_trailing_sep(to)
        } else {
            root.to_string()
        }
    } else if root.ends_with(is_sep) {
        format!("{root}{tail}")
    } else {
        format!("{root}{sep}{tail}")
    };
    Some((i, Rebased { value, rest: tail }))
}

/// Whether `path` is an absolute path a mapping could rewrite.
pub(crate) fn is_absolute_path(path: &str) -> bool {
    parse_path(path).is_some()
}

/// Whether `path` is `root` or inside it.
pub(crate) fn path_under(path: &str, root: &str) -> bool {
    match (parse_path(path), parse_path(root)) {
        (Some(p), Some(r)) => below(&r, &p).is_some(),
        _ => false,
    }
}

// ---------------------------------------------------------------------------------------------
// The model's M objects, sources and parameters
// ---------------------------------------------------------------------------------------------

/// An M object of the model: a partition (table, partition index) or a shared expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ObjKey {
    Partition(usize, usize),
    Expression(usize),
}

pub(crate) fn object_json(m: &SemanticModel, k: ObjKey) -> Value {
    match k {
        ObjKey::Partition(t, p) => {
            let table = m.tables.get(t);
            json!({
                "kind": "partition",
                "table": table.map(|t| t.name.as_str()),
                "partition": table.and_then(|t| t.partitions.get(p)).map(|p| p.name.as_str()),
            })
        }
        ObjKey::Expression(e) => json!({
            "kind": "expression",
            "name": m.expressions.get(e).map(|e| e.name.as_str()),
        }),
    }
}

fn object_text(m: &SemanticModel, k: ObjKey) -> Option<&str> {
    match k {
        ObjKey::Partition(t, p) => m.tables.get(t)?.partitions.get(p)?.expression.as_deref(),
        ObjKey::Expression(e) => m.expressions.get(e).map(|e| e.expression.as_str()),
    }
}

fn object_text_mut(m: &mut SemanticModel, k: ObjKey) -> Option<&mut String> {
    match k {
        ObjKey::Partition(t, p) => m.tables.get_mut(t)?.partitions.get_mut(p)?.expression.as_mut(),
        ObjKey::Expression(e) => m.expressions.get_mut(e).map(|e| &mut e.expression),
    }
}

/// Every M object of the model with its text.
pub(crate) fn m_objects(m: &SemanticModel) -> Vec<(ObjKey, &str)> {
    let mut out = Vec::new();
    for (ti, t) in m.tables.iter().enumerate() {
        for (pi, p) in t.partitions.iter().enumerate() {
            if let (true, Some(e)) = (is_m(&p.source_type), p.expression.as_deref()) {
                out.push((ObjKey::Partition(ti, pi), e));
            }
        }
    }
    for (ei, e) in m.expressions.iter().enumerate() {
        if is_m(&e.kind) {
            out.push((ObjKey::Expression(ei), e.expression.as_str()));
        }
    }
    out
}

/// A parameter query of the model.
#[derive(Debug, Clone)]
pub(crate) struct Param {
    pub index: usize,
    pub name: String,
    pub info: ParamInfo,
    /// The declared type, else the type of the value as written.
    pub ty: String,
}

fn value_type(v: &ParamValue) -> &'static str {
    match v {
        ParamValue::Text(_) => "Text",
        ParamValue::Number(_) => "Number",
        ParamValue::Logical(_) => "Logical",
        ParamValue::Null => "Any",
        ParamValue::Other(t) if t.starts_with("#datetimezone(") => "DateTimeZone",
        ParamValue::Other(t) if t.starts_with("#datetime(") => "DateTime",
        ParamValue::Other(t) if t.starts_with("#date(") => "Date",
        ParamValue::Other(t) if t.starts_with("#time(") => "Time",
        ParamValue::Other(t) if t.starts_with("#duration(") => "Duration",
        ParamValue::Other(_) => "Any",
    }
}

pub(crate) fn value_json(v: &ParamValue) -> Value {
    match v {
        ParamValue::Text(s) => json!(s),
        ParamValue::Number(n) => json!(n),
        ParamValue::Logical(b) => json!(b),
        ParamValue::Null => Value::Null,
        ParamValue::Other(t) => json!(t),
    }
}

pub(crate) fn parameters(m: &SemanticModel) -> Vec<Param> {
    m.expressions
        .iter()
        .enumerate()
        .filter(|(_, e)| is_m(&e.kind))
        .filter_map(|(i, e)| {
            let info = parameter_info(&e.expression)?;
            let ty = info
                .declared_type
                .clone()
                .unwrap_or_else(|| value_type(&info.value).to_string());
            Some(Param {
                index: i,
                name: e.name.clone(),
                info,
                ty,
            })
        })
        .collect()
}

/// The source calls of every M object; objects whose text does not tokenize go to `unparsed`.
pub(crate) struct Scan {
    pub calls: Vec<(ObjKey, SourceCall)>,
    pub unparsed: Vec<(ObjKey, String)>,
    pub params: Vec<Param>,
    by_name: std::collections::HashMap<String, usize>,
}

pub(crate) fn scan(m: &SemanticModel) -> Scan {
    let mut calls = Vec::new();
    let mut unparsed = Vec::new();
    for (k, text) in m_objects(m) {
        match source_calls(text) {
            Ok(list) => calls.extend(list.into_iter().map(|c| (k, c))),
            Err(e) => unparsed.push((k, e)),
        }
    }
    let params = parameters(m);
    let by_name = params
        .iter()
        .enumerate()
        .map(|(i, p)| (p.name.clone(), i))
        .collect();
    Scan {
        calls,
        unparsed,
        params,
        by_name,
    }
}

impl Scan {
    pub(crate) fn param(&self, name: &str) -> Option<&Param> {
        self.by_name.get(name).and_then(|&i| self.params.get(i))
    }

    /// The parameters the sources use, with the roles they are used in.
    pub(crate) fn param_roles(&self) -> BTreeMap<String, BTreeSet<&'static str>> {
        let mut out: BTreeMap<String, BTreeSet<&'static str>> = BTreeMap::new();
        for (_, c) in &self.calls {
            for a in &c.args {
                let names: Vec<&str> = match &a.form {
                    Form::Name { name, step: false } => vec![name.as_str()],
                    Form::Concat(parts) => parts
                        .iter()
                        .filter_map(|p| match p {
                            Part::Name(n, _, _) => Some(n.as_str()),
                            Part::Text(_) => None,
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                for n in names {
                    if self.param(n).is_some() {
                        out.entry(n.to_string()).or_default().insert(a.role.name());
                    }
                }
            }
        }
        out
    }
}

fn span(start: usize, end: usize) -> Value {
    json!({"start": start, "end": end})
}

fn shown(text: &str, start: usize, end: usize) -> String {
    let s = text.get(start..end).unwrap_or_default();
    match s.char_indices().nth(MAX_SHOWN) {
        Some((cut, _)) => format!("{}…", s.get(..cut).unwrap_or_default()),
        None => s.to_string(),
    }
}

/// What a name in a source argument is: a step of the query, a parameter, another query.
fn name_kind(scan: &Scan, queries: &BTreeSet<String>, name: &str, step: bool) -> &'static str {
    if step {
        "step"
    } else if scan.param(name).is_some() {
        "parameter"
    } else if queries.contains(name) {
        "query"
    } else {
        "name"
    }
}

/// An argument for results: its form, value and position.
pub(crate) fn arg_json(
    scan: &Scan,
    queries: &BTreeSet<String>,
    text: &str,
    a: &bicraft_data::m::sources::Arg,
) -> Value {
    let mut v = json!({
        "role": a.role.name(),
        "span": span(a.start, a.end),
        "line": a.line,
        "column": a.column,
    });
    match &a.form {
        Form::Text(l) => {
            v["form"] = json!("literal");
            v["value"] = json!(l.value);
        }
        Form::StepText(step, l) => {
            v["form"] = json!("step_literal");
            v["step"] = json!(step);
            v["value"] = json!(l.value);
            v["literal_span"] = span(l.start, l.end);
        }
        Form::Name { name, step } => {
            let kind = name_kind(scan, queries, name, *step);
            v["form"] = json!(kind);
            v["name"] = json!(name);
            if let Some(p) = scan.param(name) {
                v["value"] = value_json(&p.info.value);
            }
        }
        Form::Concat(parts) => {
            v["form"] = json!("concat");
            let mut resolved = Some(String::new());
            let list: Vec<Value> = parts
                .iter()
                .map(|p| match p {
                    Part::Text(l) => {
                        if let Some(r) = resolved.as_mut() {
                            r.push_str(&l.value);
                        }
                        json!({"literal": l.value})
                    }
                    Part::Name(n, _, _) => {
                        let kind = name_kind(scan, queries, n, false);
                        match scan.param(n).map(|p| &p.info.value) {
                            Some(ParamValue::Text(t)) => {
                                if let Some(r) = resolved.as_mut() {
                                    r.push_str(t);
                                }
                            }
                            _ => resolved = None,
                        }
                        let mut o = serde_json::Map::new();
                        o.insert(kind.into(), json!(n));
                        Value::Object(o)
                    }
                })
                .collect();
            v["parts"] = Value::Array(list);
            if let Some(r) = resolved {
                v["value"] = json!(r);
            }
        }
        Form::Call(f) => {
            v["form"] = json!("call");
            v["function"] = json!(f);
        }
        Form::Computed => {
            v["form"] = json!("computed");
            v["text"] = json!(shown(text, a.start, a.end));
        }
    }
    v
}

fn all_query_names(m: &SemanticModel) -> BTreeSet<String> {
    query_names(m).into_iter().collect()
}

/// `sources.list {}`.
pub(crate) fn sources_list(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &[])?;
    let m = crate::commands::model(s)?;
    let sc = scan(m);
    let queries = all_query_names(m);
    let roles = sc.param_roles();
    let mut forms: BTreeMap<String, usize> = BTreeMap::new();
    let mut sources = Vec::new();
    for (k, c) in &sc.calls {
        let text = object_text(m, *k).unwrap_or_default();
        let args: Vec<Value> = c.args.iter().map(|a| arg_json(&sc, &queries, text, a)).collect();
        for a in &args {
            if let Some(f) = a.get("form").and_then(Value::as_str) {
                let n = forms.entry(f.to_string()).or_default();
                *n = n.saturating_add(1);
            }
        }
        let mut v = json!({
            "object": object_json(m, *k),
            "function": c.function,
            "span": span(c.start, c.end),
            "line": c.line,
            "column": c.column,
            "arguments": args,
        });
        if let Some(f) = &c.format {
            v["format"] = json!(f);
        }
        sources.push(v);
    }
    let params: Vec<Value> = sc
        .params
        .iter()
        .map(|p| {
            let mut v = json!({
                "name": p.name,
                "type": p.ty,
                "value": value_json(&p.info.value),
                "used_as": roles.get(&p.name).map(|r| r.iter().collect::<Vec<_>>()).unwrap_or_default(),
            });
            if let Some(a) = &p.info.allowed {
                v["allowed"] = json!(a);
            }
            v
        })
        .collect();
    let unparsed: Vec<Value> = sc
        .unparsed
        .iter()
        .map(|(k, e)| json!({"object": object_json(m, *k), "error": e}))
        .collect();
    Ok(json!({
        "sources": sources,
        "parameters": params,
        "summary": {"sources": sc.calls.len(), "parameters": sc.params.len(), "arguments": forms},
        "unparsed": unparsed,
    }))
}

// ---------------------------------------------------------------------------------------------
// Plans: the literals a rebase or a profile rewrites
// ---------------------------------------------------------------------------------------------

/// What to rewrite.
#[derive(Debug, Clone, Default)]
pub(crate) struct Mappings {
    pub paths: Vec<PathMap>,
    /// Paths under the (single) path root become `<param> & "rest"` (`sources.rebase {param}`).
    pub param: Option<String>,
    pub servers: Vec<(String, String)>,
    pub databases: Vec<(String, String)>,
    pub hosts: Vec<(String, String)>,
    pub http_paths: Vec<(String, String)>,
    /// URL prefixes.
    pub urls: Vec<(String, String)>,
}

impl Mappings {
    /// The exact mappings for a role, and whether they compare without regard to case.
    fn exact(&self, role: Role) -> Option<(&[(String, String)], bool)> {
        match role {
            Role::Server => Some((&self.servers, true)),
            Role::Database => Some((&self.databases, true)),
            Role::Host => Some((&self.hosts, true)),
            Role::HttpPath => Some((&self.http_paths, false)),
            _ => None,
        }
    }

    /// The new value of a literal in `role`: `(value, M text replacing the literal)`; `None`
    /// when no mapping matches or the value would not change.
    fn map_literal(&self, role: Role, old: &str, as_prefix: bool) -> Option<(String, Option<String>)> {
        match role {
            Role::Path | Role::Folder => {
                let (_, r) = rebase_path(old, &self.paths)?;
                match &self.param {
                    Some(param) => {
                        let id = identifier(param);
                        let m = if r.rest.is_empty() {
                            id
                        } else {
                            format!("{id} & {}", text_literal(&r.rest))
                        };
                        Some((r.value, Some(m)))
                    }
                    None => (r.value != old).then_some((r.value, None)),
                }
            }
            Role::Url => {
                let (from, to) = self
                    .urls
                    .iter()
                    .filter(|(f, _)| !f.is_empty() && old.starts_with(f.as_str()))
                    .max_by_key(|(f, _)| f.len())?;
                let new = format!("{to}{}", old.get(from.len()..)?);
                (new != old).then_some((new, None))
            }
            _ if as_prefix => None,
            r => {
                let (list, fold) = self.exact(r)?;
                let (_, to) = list.iter().find(|(f, _)| {
                    if fold {
                        f.to_lowercase() == old.to_lowercase()
                    } else {
                        f == old
                    }
                })?;
                (to != old).then(|| (to.clone(), None))
            }
        }
    }

    /// Whether a value in `role` is one the mappings know (a source or a target).
    pub(crate) fn covers(&self, role: Role, value: &str) -> bool {
        match role {
            Role::Path | Role::Folder => self
                .paths
                .iter()
                .any(|m| path_under(value, &m.from) || path_under(value, &m.to)),
            Role::Url => self.urls.iter().any(|(f, t)| {
                (!f.is_empty() && value.starts_with(f.as_str()))
                    || (!t.is_empty() && value.starts_with(t.as_str()))
            }),
            r => self.exact(r).is_some_and(|(list, fold)| {
                list.iter().any(|(f, t)| {
                    if fold {
                        f.to_lowercase() == value.to_lowercase() || t.to_lowercase() == value.to_lowercase()
                    } else {
                        f == value || t == value
                    }
                })
            }),
        }
    }
}

/// A replacement in one M object.
#[derive(Debug, Clone)]
struct Replace {
    key: ObjKey,
    start: usize,
    end: usize,
    text: String,
}

/// A new parameter value: its M text and its value for results.
#[derive(Debug, Clone)]
pub(crate) struct NewValue {
    pub m: String,
    pub shown: Value,
}

/// The rewrites of a rebase or a profile, computed on the model without changing it.
#[derive(Debug, Default)]
pub(crate) struct Plan {
    replaces: Vec<Replace>,
    /// One entry per rewritten source argument.
    pub changes: Vec<Value>,
    /// One entry per parameter whose value changes.
    pub parameters: Vec<Value>,
    /// Location arguments left as they are, and why.
    pub unchanged: Vec<Value>,
    /// The parameter to add: `(name, M text)`.
    pub new_param: Option<(String, String)>,
    pub warnings: Vec<String>,
}

/// The literal of an argument that a rewrite may replace, and whether it is the leading part of
/// a concatenation (only path and URL prefixes are rewritten there).
fn rewritable(form: &Form) -> Option<(&TextLit, bool)> {
    match form {
        Form::Text(l) | Form::StepText(_, l) => Some((l, false)),
        Form::Concat(parts) => match parts.first() {
            Some(Part::Text(l)) => Some((l, true)),
            _ => None,
        },
        _ => None,
    }
}

/// Builds the plan for `maps` and the explicit parameter values `values` (name → new value).
pub(crate) fn plan(m: &SemanticModel, maps: &Mappings, values: &BTreeMap<String, NewValue>) -> Plan {
    let sc = scan(m);
    let queries = all_query_names(m);
    let mut out = Plan::default();
    let mut seen: BTreeSet<(ObjKey, usize)> = BTreeSet::new();
    for (k, c) in &sc.calls {
        let text = object_text(m, *k).unwrap_or_default();
        for a in c.args.iter().filter(|a| a.role.is_location()) {
            let entry = |extra: Value| {
                let mut v = json!({
                    "object": object_json(m, *k),
                    "function": c.function,
                    "role": a.role.name(),
                    "line": a.line,
                    "column": a.column,
                });
                if let (Some(o), Value::Object(e)) = (v.as_object_mut(), extra) {
                    o.extend(e);
                }
                v
            };
            let Some((lit, prefix)) = rewritable(&a.form) else {
                let why = match &a.form {
                    Form::Name { name, step } => match name_kind(&sc, &queries, name, *step) {
                        "parameter" => json!({"reason": "parameter", "parameter": name}),
                        kind => json!({"reason": "computed", "form": kind, "name": name}),
                    },
                    Form::Concat(parts) => match parts.iter().find_map(|p| match p {
                        Part::Name(n, _, _) if sc.param(n).is_some() => Some(n),
                        _ => None,
                    }) {
                        Some(n) => json!({"reason": "parameter", "parameter": n}),
                        None => {
                            json!({"reason": "computed", "form": "concat", "text": shown(text, a.start, a.end)})
                        }
                    },
                    _ => json!({"reason": "computed", "text": shown(text, a.start, a.end)}),
                };
                out.unchanged.push(entry(why));
                continue;
            };
            let Some((value, m_text)) = maps.map_literal(a.role, &lit.value, prefix) else {
                out.unchanged
                    .push(entry(json!({"reason": "no_mapping", "value": lit.value})));
                continue;
            };
            let relative = m_text.is_some();
            let new_m = m_text.unwrap_or_else(|| text_literal(&value));
            if !seen.insert((*k, lit.start)) {
                continue;
            }
            let mut change = entry(json!({
                "form": if matches!(a.form, Form::StepText(..)) { "step_literal" } else if prefix { "concat" } else { "literal" },
                "before": lit.value,
                "after": value,
                "span": span(lit.start, lit.end),
                "m_before": text.get(lit.start..lit.end).unwrap_or_default(),
                "m_after": new_m,
            }));
            if relative {
                change["relative_to"] = json!(maps.param);
            }
            out.changes.push(change);
            out.replaces.push(Replace {
                key: *k,
                start: lit.start,
                end: lit.end,
                text: new_m,
            });
        }
    }
    // parameters: explicit values first, then the mappings of the roles they are used in
    let roles = sc.param_roles();
    for p in &sc.params {
        let old_m = m
            .expressions
            .get(p.index)
            .and_then(|e| e.expression.get(p.info.value_start..p.info.value_end))
            .unwrap_or_default()
            .to_string();
        let new = if let Some(v) = values.get(&p.name) {
            Some((v.m.clone(), v.shown.clone()))
        } else if let ParamValue::Text(t) = &p.info.value {
            let used: Vec<Role> = roles
                .get(&p.name)
                .map(|r| r.iter().filter_map(|n| role_named(n)).collect())
                .unwrap_or_default();
            let mut found = None;
            // a path value is rebased wherever it is used; other roles map where they are used
            if is_absolute_path(t)
                && let Some((_, r)) = rebase_path(t, &maps.paths)
            {
                let v = if t.ends_with(is_sep) {
                    with_trailing_sep(&r.value)
                } else {
                    r.value
                };
                found = Some(v);
            }
            for role in used {
                if found.is_none()
                    && !matches!(role, Role::Path | Role::Folder)
                    && let Some((v, _)) = maps.map_literal(role, t, false)
                {
                    found = Some(v);
                }
            }
            found.filter(|v| v != t).map(|v| (text_literal(&v), json!(v)))
        } else {
            None
        };
        let Some((new_m, shown_v)) = new else {
            continue;
        };
        if new_m == old_m {
            continue;
        }
        out.parameters.push(json!({
            "name": p.name,
            "type": p.ty,
            "before": value_json(&p.info.value),
            "after": shown_v,
            "m_before": old_m,
            "m_after": new_m,
        }));
        out.replaces.push(Replace {
            key: ObjKey::Expression(p.index),
            start: p.info.value_start,
            end: p.info.value_end,
            text: new_m,
        });
    }
    out
}

pub(crate) fn role_named(n: &str) -> Option<Role> {
    [
        Role::Path,
        Role::Folder,
        Role::Server,
        Role::Database,
        Role::Host,
        Role::HttpPath,
        Role::Url,
        Role::Account,
        Role::Warehouse,
        Role::ConnectionString,
    ]
    .into_iter()
    .find(|r| r.name() == n)
}

impl Plan {
    pub(crate) fn is_empty(&self) -> bool {
        self.replaces.is_empty() && self.new_param.is_none()
    }

    /// Applies the replacements to `m` (the model the plan was made from). Every rewritten text
    /// must still tokenize.
    pub(crate) fn apply_to(&self, m: &mut SemanticModel) -> Result<(), String> {
        let mut by_key: BTreeMap<ObjKey, Vec<&Replace>> = BTreeMap::new();
        for r in &self.replaces {
            by_key.entry(r.key).or_default().push(r);
        }
        for (k, mut list) in by_key {
            list.sort_by_key(|r| r.start);
            let text = object_text_mut(m, k).ok_or("internal: an M object of the plan is gone")?;
            let mut out = String::with_capacity(text.len());
            let mut at = 0usize;
            for r in list {
                if r.start < at {
                    return Err("internal: overlapping rewrites".into());
                }
                out.push_str(
                    text.get(at..r.start)
                        .ok_or("internal: rewrite outside the text")?,
                );
                out.push_str(&r.text);
                at = r.end;
            }
            out.push_str(text.get(at..).ok_or("internal: rewrite outside the text")?);
            source_calls(&out).map_err(|e| format!("internal: a rewritten M text does not parse: {e}"))?;
            *text = out;
        }
        if let Some((name, expression)) = &self.new_param {
            let a = analyze_all(vec![expression.clone()], &Limits::default())
                .into_iter()
                .next()
                .unwrap_or_default();
            let mut e = NamedExpression {
                name: name.clone(),
                kind: "m".into(),
                expression: expression.clone(),
                lineage_tag: Some(new_guid()),
                ..Default::default()
            };
            crate::desktop_defaults::m_expression(&mut e.annotations, &a);
            m.expressions.push(e);
        }
        Ok(())
    }
}

/// A text parameter's M (`"value" meta [IsParameterQuery=true, Type="Text", …]`), as Desktop
/// writes a new one.
pub(crate) fn text_parameter_m(value: &str) -> String {
    format!(
        "{} meta [IsParameterQuery=true, Type=\"Text\", IsParameterQueryRequired=true]",
        text_literal(value)
    )
}

/// Refuses rewrites on a PBIX whose DataModel holds the rows: its stored data would no longer
/// match the queries.
pub(crate) fn refuse_pbix_with_data(s: &Session) -> Result<(), String> {
    if crate::partition::pbix_with_data(s) {
        return Err(
            "this PBIX stores the rows its queries loaded; rewriting their sources would leave data \
             that no longer matches the queries. Save it as a PBIP project (file.save_as {path: \
             \"out/Name.pbip\"}), rewrite the sources there and refresh it in Power BI Desktop (or \
             pass dry_run: true to preview)"
                .into(),
        );
    }
    Ok(())
}

/// Results shared by `sources.rebase` and `env.apply`.
pub(crate) fn plan_result(plan: &Plan, dry_run: bool) -> Value {
    json!({
        "dry_run": dry_run,
        "changed": !plan.is_empty(),
        "changes": plan.changes,
        "parameters": plan.parameters,
        "added_parameter": plan.new_param.as_ref().map(|(n, e)| json!({"name": n, "expression": e})),
        "unchanged": plan.unchanged,
        "warnings": plan.warnings,
    })
}

/// Commits a plan as one edit (one undo step), or only returns it with `dry_run`.
pub(crate) fn commit(
    s: &mut Session,
    p: &Value,
    plan: Plan,
    dry_run: bool,
    mut extra: Value,
) -> Result<Value, String> {
    let mut result = plan_result(&plan, dry_run);
    if let (Some(r), Some(e)) = (result.as_object_mut(), extra.as_object_mut()) {
        r.append(e);
    }
    if dry_run || plan.is_empty() {
        return Ok(result);
    }
    refuse_pbix_with_data(s)?;
    apply(s, &strip(p), |m| {
        plan.apply_to(m)?;
        Ok(Edit::new(result))
    })
}

/// The parameters without the keys `edit::apply` reads itself (`force`, `update_report`).
fn strip(p: &Value) -> Value {
    match p {
        Value::Object(o) => {
            let mut o = o.clone();
            o.retain(|k, _| k == "force");
            Value::Object(o)
        }
        _ => json!({}),
    }
}

// ---------------------------------------------------------------------------------------------
// sources.rebase
// ---------------------------------------------------------------------------------------------

/// A `{old: new}` mapping parameter.
pub(crate) fn mapping(v: Option<&Value>, what: &str) -> Result<Vec<(String, String)>, String> {
    match v {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Object(o)) => {
            if o.len() > MAX_MAPPINGS {
                return Err(format!("`{what}`: more than {MAX_MAPPINGS} entries"));
            }
            o.iter()
                .map(|(k, v)| match v {
                    Value::String(s) if !k.is_empty() && !s.is_empty() => Ok((k.clone(), s.clone())),
                    _ => Err(format!(
                        "`{what}`: `{k}` must map a non-empty name to a non-empty text"
                    )),
                })
                .collect()
        }
        Some(_) => Err(format!("`{what}` must be an object {{old: new}}")),
    }
}

/// Checks a parameter name for `sources.rebase {param}`: an existing text parameter, or a name
/// free for a new one. Returns whether it exists.
fn check_param_target(m: &SemanticModel, sc: &Scan, name: &str) -> Result<bool, String> {
    if let Some(p) = sc.param(name) {
        if !p.ty.eq_ignore_ascii_case("text") && !p.ty.eq_ignore_ascii_case("any") {
            return Err(format!(
                "parameter `{name}` is of type {}; a root folder parameter must be Text",
                p.ty
            ));
        }
        return Ok(true);
    }
    if m.expressions.iter().any(|e| e.name.eq_ignore_ascii_case(name)) {
        return Err(format!(
            "`{name}` is a query of the model, not a parameter (IsParameterQuery): choose another name"
        ));
    }
    if m.tables.iter().any(|t| t.name.eq_ignore_ascii_case(name)) {
        return Err(format!(
            "`{name}` is a table of the model: choose another parameter name"
        ));
    }
    crate::edit::check_name("parameter", name)?;
    Ok(false)
}

/// `sources.rebase {from?, to?, param?, servers?, databases?, hosts?, http_paths?, urls?, dry_run?}`.
pub(crate) fn sources_rebase(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "from",
            "to",
            "param",
            "servers",
            "databases",
            "hosts",
            "http_paths",
            "urls",
            "dry_run",
        ],
    )?;
    let from = opt_str(p, "from")?.filter(|f| !f.is_empty());
    let to = opt_str(p, "to")?.filter(|t| !t.is_empty());
    let param = opt_str(p, "param")?.filter(|n| !n.is_empty());
    let dry_run = opt_bool(p, "dry_run")?.unwrap_or(false);
    let mut maps = Mappings {
        servers: mapping(p.get("servers"), "servers")?,
        databases: mapping(p.get("databases"), "databases")?,
        hosts: mapping(p.get("hosts"), "hosts")?,
        http_paths: mapping(p.get("http_paths"), "http_paths")?,
        urls: mapping(p.get("urls"), "urls")?,
        ..Default::default()
    };
    let mut warnings = Vec::new();
    match (from, to, param) {
        (None, None, None) => {}
        (None, _, _) => {
            return Err("`to` and `param` need `from` (the root folder the paths start with)".into());
        }
        (Some(_), None, None) => {
            return Err(
                "`from` needs `to` (the new root), or `param` to make the paths relative to a parameter"
                    .into(),
            );
        }
        (Some(f), t, _) => {
            if !is_absolute_path(f) {
                return Err(format!(
                    "`from` must be an absolute path (C:\\…, \\\\server\\share\\… or /…), not `{f}`"
                ));
            }
            let t = t.unwrap_or(f);
            if !is_absolute_path(t) {
                warnings.push(format!(
                    "`{t}` is not an absolute path: Power Query reads files by absolute path, so \
                     Power BI Desktop will refuse the rewritten sources until it is one"
                ));
            }
            maps.paths.push(PathMap {
                from: f.to_string(),
                to: t.to_string(),
            });
        }
    }
    if maps.paths.is_empty()
        && maps.servers.is_empty()
        && maps.databases.is_empty()
        && maps.hosts.is_empty()
        && maps.http_paths.is_empty()
        && maps.urls.is_empty()
    {
        return Err(
            "nothing to rewrite: pass `from` and `to` (or `param`), or one of servers, databases, \
             hosts, http_paths, urls"
                .into(),
        );
    }
    let m = crate::commands::model(s)?;
    let sc = scan(m);
    let mut values = BTreeMap::new();
    let mut new_param = None;
    if let (Some(name), Some(pm)) = (param, maps.paths.first()) {
        let root = with_trailing_sep(&pm.to);
        if check_param_target(m, &sc, name)? {
            values.insert(
                name.to_string(),
                NewValue {
                    m: text_literal(&root),
                    shown: json!(root),
                },
            );
        } else {
            new_param = Some((name.to_string(), text_parameter_m(&root)));
        }
        maps.param = Some(name.to_string());
    }
    let mut plan = plan(m, &maps, &values);
    // a new parameter nothing uses is not added
    if new_param.is_some() && plan.changes.iter().any(|c| c.get("relative_to").is_some()) {
        plan.new_param = new_param;
    } else if let Some((n, _)) = new_param {
        plan.warnings.push(format!(
            "no source path is under `{}`: parameter `{n}` was not added",
            from.unwrap_or_default()
        ));
    }
    plan.warnings.extend(warnings);
    for (k, e) in &sc.unparsed {
        plan.warnings.push(format!(
            "{} does not parse and was left alone: {e}",
            object_json(m, *k)
        ));
    }
    commit(s, p, plan, dry_run, json!({}))
}

#[cfg(test)]
#[path = "sources_tests.rs"]
pub(crate) mod tests;
