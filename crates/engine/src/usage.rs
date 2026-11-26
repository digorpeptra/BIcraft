//! Where model objects are used, what is unused, and removing the unused (`model.usage`,
//! `model.unused`, `model.remove_unused`).
//!
//! One pass over the model and the reports builds a **usage graph** ([`Graph`]):
//!
//! - one node per table, column (engine row-number columns excepted), measure and shared M
//!   expression;
//! - **edges** `a → b`, "a needs b": every DAX expression the model stores
//!   ([`crate::dax_rename::model_sites`]: measures and their KPI / dynamic format string / detail
//!   rows expressions, calculated columns, calculated tables, field parameters through `NAMEOF`,
//!   calculation items) resolved like the rename and removal commands resolve them
//!   ([`Renamer::references`]); a column or measure needs its table; a calculated table that
//!   names a whole table needs every column of it; a column needs its `groupByColumns`; an M
//!   partition or expression needs the queries it names;
//! - **usages**: every other place that uses a node, each a JSON object with a `kind`: the
//!   report (visual roles, filters at every level, drillthrough and tooltip pages, bookmarks,
//!   conditional formatting, sort, report-level measures; `usage_report.rs`), row filters (RLS)
//!   and object-level security, relationships (inactive ones too), hierarchies, sort-by columns,
//!   auto date/time variations and tables, key and default-label columns, aggregations
//!   (`alternateOf`), calculation groups (including measures they name in strings, as
//!   `SELECTEDMEASURENAME() = "…"` does), user-defined functions, perspectives, translations, the
//!   Q&A linguistic schema, and the allow-list.
//!
//! A node is **used** when it has a usage, or a used node needs it (transitively). It is used *by
//! a report* when that chain starts at a report usage; the others are *kept* by a model rule,
//! and `model.unused` says which. An expression that does not parse is matched by name
//! (`[Name]`, `'Table'`), so it can only keep more objects, never fewer.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

use bicraft_dax::deps::{ObjectRef, calculated_table_expression};
use bicraft_dax::rename::Renamer;
use bicraft_model::SemanticModel;
use bicraft_report::Report;
use serde_json::{Map, Value, json};

use crate::Session;
use crate::commands::model;
use crate::dax_rename::{SiteOwner, model_sites};
use crate::edit::{Edit, dax_column, dax_measure, dax_table};
use crate::params::{allow, force, opt_bool, opt_str};
use crate::refs::{ParsedField, parse_field};
use crate::usage_report::{ReportRef, report_measure_refs, report_refs};

/// Most extra reports one call reads.
const MAX_REPORTS: usize = 64;
/// Most allow-list patterns.
const MAX_ALLOW: usize = 10_000;
/// Longest allow-list pattern, in characters.
const MAX_PATTERN_CHARS: usize = 1_000;
/// Most entries of one list in a result (counts stay exact).
const MAX_LISTED: usize = 5_000;

/// Usage kinds that come from a report.
const REPORT_KINDS: &[&str] = &[
    "visual",
    "sort",
    "formatting",
    "visual filter",
    "visual property",
    "page filter",
    "drillthrough",
    "tooltip page",
    "page formatting",
    "page property",
    "report filter",
    "report formatting",
    "report property",
    "bookmark",
    "report measure",
];

/// Usage kinds `keep_metadata: false` ignores: metadata that names objects without using them.
const METADATA_KINDS: &[&str] = &["perspective", "translation", "linguistic schema"];

/// The label of the open file's report in usages.
pub(crate) const OPEN_REPORT: &str = "open";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Kind {
    Table,
    Column,
    Measure,
    Expression,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Table => "table",
            Kind::Column => "column",
            Kind::Measure => "measure",
            Kind::Expression => "expression",
        }
    }
}

#[derive(Debug, Clone)]
struct Node {
    kind: Kind,
    /// The owning table (empty for expressions; the table itself for tables).
    table: String,
    name: String,
    /// A calculated table (its partition is DAX), not a calculation group.
    calculated_table: bool,
}

impl Node {
    fn json(&self) -> Value {
        match self.kind {
            Kind::Table => json!({"kind": "table", "name": self.name}),
            Kind::Expression => json!({"kind": "expression", "name": self.name}),
            _ => json!({"kind": self.kind.name(), "table": self.table, "name": self.name}),
        }
    }

    fn label(&self) -> String {
        match self.kind {
            Kind::Table => format!("table {}", dax_table(&self.name)),
            Kind::Column => format!("column {}", dax_column(&self.table, &self.name)),
            Kind::Measure => format!("measure {}", dax_measure(&self.name)),
            Kind::Expression => format!("expression `{}`", self.name),
        }
    }
}

/// `from → to`, with the expression or property that makes the dependency (`sites[site]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Edge {
    to: usize,
    site: usize,
}

/// `sites[MEMBER]`: a column or measure belongs to its table.
const MEMBER: usize = 0;

type Key = (Kind, String, String);

fn key(kind: Kind, table: &str, name: &str) -> Key {
    (kind, table.to_lowercase(), name.to_lowercase())
}

/// The usage graph of a model and its reports (module docs).
pub(crate) struct Graph {
    nodes: Vec<Node>,
    lookup: HashMap<Key, usize>,
    measures_by_name: HashMap<String, usize>,
    columns_by_name: HashMap<String, Vec<usize>>,
    /// (table, hierarchy, level), lower case → column.
    levels: HashMap<(String, String, String), usize>,
    deps: Vec<Vec<Edge>>,
    rdeps: Vec<Vec<(usize, usize)>>,
    sites: Vec<Value>,
    usages: Vec<Vec<Value>>,
    /// The reports read, in order: `open` (the open file's), then the `reports` parameter's.
    pub(crate) reports: Vec<Value>,
    pub(crate) warnings: Vec<String>,
    unparsed: usize,
    unresolved_report_refs: usize,
}

impl Graph {
    fn find(&self, kind: Kind, table: &str, name: &str) -> Option<usize> {
        self.lookup.get(&key(kind, table, name)).copied()
    }

    fn find_table(&self, name: &str) -> Option<usize> {
        self.find(Kind::Table, name, name)
    }

    fn node(&self, i: usize) -> Option<&Node> {
        self.nodes.get(i)
    }

    fn label(&self, i: usize) -> String {
        self.node(i).map(Node::label).unwrap_or_default()
    }

    fn object(&self, i: usize) -> Value {
        self.node(i).map(Node::json).unwrap_or(Value::Null)
    }

    fn add_node(&mut self, node: Node) -> usize {
        let k = key(node.kind, &node.table, &node.name);
        if let Some(&i) = self.lookup.get(&k) {
            return i;
        }
        let i = self.nodes.len();
        match node.kind {
            Kind::Measure => {
                self.measures_by_name.entry(node.name.to_lowercase()).or_insert(i);
            }
            Kind::Column => self
                .columns_by_name
                .entry(node.name.to_lowercase())
                .or_default()
                .push(i),
            _ => {}
        }
        self.nodes.push(node);
        self.deps.push(Vec::new());
        self.usages.push(Vec::new());
        self.lookup.insert(k, i);
        i
    }

    fn site(&mut self, v: Value) -> usize {
        self.sites.push(v);
        self.sites.len().saturating_sub(1)
    }

    fn edge(&mut self, from: usize, to: usize, site: usize) {
        if from == to {
            return;
        }
        if let Some(e) = self.deps.get_mut(from) {
            e.push(Edge { to, site });
        }
    }

    fn use_node(&mut self, i: usize, usage: Value) {
        if let Some(u) = self.usages.get_mut(i) {
            u.push(usage);
        }
    }

    /// The node of a DAX reference.
    fn resolve_ref(&self, r: &ObjectRef) -> Option<usize> {
        match r {
            ObjectRef::Measure { table, name } => self
                .find(Kind::Measure, table, name)
                .or_else(|| self.measures_by_name.get(&name.to_lowercase()).copied()),
            ObjectRef::Column { table, name } => self.find(Kind::Column, table, name),
            ObjectRef::Table { name } => self.find_table(name),
            ObjectRef::RowFilter { .. } => None,
        }
    }

    /// The nodes an unparsable expression names: `[Name]` (measures and columns of that name) and
    /// `'Table'`.
    fn mentions(&self, text: &str) -> Vec<usize> {
        let mut out = Vec::new();
        for name in delimited(text, '[', ']') {
            let lower = name.to_lowercase();
            if let Some(&m) = self.measures_by_name.get(&lower) {
                out.push(m);
            }
            if let Some(cols) = self.columns_by_name.get(&lower) {
                out.extend(cols.iter().copied());
            }
        }
        for name in delimited(text, '\'', '\'') {
            if let Some(t) = self.find_table(&name) {
                out.push(t);
            }
        }
        out
    }

    /// The node a report reference names, if the model has it.
    fn resolve_report_ref(&self, r: &ReportRef) -> Option<usize> {
        match r {
            ReportRef::Column { table, name } => self
                .find(Kind::Column, table, name)
                .or_else(|| self.find(Kind::Measure, table, name)),
            ReportRef::Measure { table, name } => self
                .find(Kind::Measure, table, name)
                .or_else(|| self.measures_by_name.get(&name.to_lowercase()).copied())
                .or_else(|| self.find(Kind::Column, table, name)),
            ReportRef::Level {
                table,
                hierarchy,
                level,
            } => self
                .levels
                .get(&(
                    table.to_lowercase(),
                    hierarchy.to_lowercase(),
                    level.to_lowercase(),
                ))
                .copied(),
        }
    }
}

/// The contents of every `open…close` span of `text` (a doubled `close` is an escaped one).
fn delimited(text: &str, open: char, close: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != open {
            continue;
        }
        let mut s = String::new();
        let mut closed = false;
        while let Some(c) = chars.next() {
            if c == close {
                if chars.peek() == Some(&close) {
                    chars.next();
                    s.push(close);
                    continue;
                }
                closed = true;
                break;
            }
            s.push(c);
        }
        if closed && !s.is_empty() {
            out.push(s);
        }
    }
    out
}

fn str_of<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

fn array_of<'a>(v: Option<&'a Value>, k: &str) -> impl Iterator<Item = &'a Value> {
    v.and_then(|v| v.get(k))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

// ---------------------------------------------------------------------------------------------
// Building the graph
// ---------------------------------------------------------------------------------------------

/// Builds the usage graph of `m` and `reports` (`(label, report)`).
pub(crate) fn build(m: &SemanticModel, reports: &[(String, &Report)]) -> Graph {
    let mut g = Graph {
        nodes: Vec::new(),
        lookup: HashMap::new(),
        measures_by_name: HashMap::new(),
        columns_by_name: HashMap::new(),
        levels: HashMap::new(),
        deps: Vec::new(),
        rdeps: Vec::new(),
        sites: vec![json!({"kind": "member"})],
        usages: Vec::new(),
        reports: Vec::new(),
        warnings: Vec::new(),
        unparsed: 0,
        unresolved_report_refs: 0,
    };
    nodes(&mut g, m);
    dax_edges(&mut g, m);
    model_usages(&mut g, m);
    m_edges(&mut g, m);
    let renamer = Renamer::new(m);
    for (label, r) in reports {
        report_usages(&mut g, label, r, &renamer);
    }
    finish(&mut g);
    g
}

fn nodes(g: &mut Graph, m: &SemanticModel) {
    for t in &m.tables {
        let calculated =
            calculated_table_expression(t).is_some() && !t.extra.contains_key("calculationGroup");
        let ti = g.add_node(Node {
            kind: Kind::Table,
            table: t.name.clone(),
            name: t.name.clone(),
            calculated_table: calculated,
        });
        for c in t.columns.iter().filter(|c| !c.is_row_number()) {
            let ci = g.add_node(Node {
                kind: Kind::Column,
                table: t.name.clone(),
                name: c.name.clone(),
                calculated_table: false,
            });
            g.edge(ci, ti, MEMBER);
        }
        for x in &t.measures {
            let mi = g.add_node(Node {
                kind: Kind::Measure,
                table: t.name.clone(),
                name: x.name.clone(),
                calculated_table: false,
            });
            g.edge(mi, ti, MEMBER);
        }
    }
    for e in &m.expressions {
        g.add_node(Node {
            kind: Kind::Expression,
            table: String::new(),
            name: e.name.clone(),
            calculated_table: false,
        });
    }
}

fn dax_edges(g: &mut Graph, m: &SemanticModel) {
    let renamer = Renamer::new(m);
    for s in model_sites(m) {
        let parsed = renamer.references_as_used(s.text(), s.site());
        let mut desc = s.to_json();
        let refs: Vec<(usize, bool)> = match &parsed {
            Some(refs) => refs
                .iter()
                .filter_map(|r| {
                    g.resolve_ref(r)
                        .map(|i| (i, matches!(r, ObjectRef::Table { .. })))
                })
                .collect(),
            None => {
                g.unparsed = g.unparsed.saturating_add(1);
                if let Some(o) = desc.as_object_mut() {
                    o.insert("matched_by_name".into(), json!(true));
                }
                g.mentions(s.text()).into_iter().map(|i| (i, false)).collect()
            }
        };
        let owner = s.owner();
        let from = match owner {
            SiteOwner::Measure { table, name } => g.find(Kind::Measure, table, name),
            SiteOwner::Column { table, name } => g.find(Kind::Column, table, name),
            SiteOwner::Table(t) | SiteOwner::CalculationGroup(t) => g.find_table(t),
            SiteOwner::RowFilter { .. } | SiteOwner::Function(_) => None,
        };
        let calculated_table =
            matches!(owner, SiteOwner::Table(_)) && s.to_json()["kind"] == "calculated table";
        match (from, owner) {
            (Some(from), _) => {
                let site = g.site(desc);
                for (to, whole_table) in refs {
                    g.edge(from, to, site);
                    // a calculated table built on a whole table gets all its columns
                    if whole_table && calculated_table {
                        let cols = table_columns(g, to);
                        for c in cols {
                            g.edge(from, c, site);
                        }
                    }
                }
                if let SiteOwner::CalculationGroup(t) = owner {
                    // SELECTEDMEASURENAME() = "Sales" names a measure in a string
                    for name in delimited(s.text(), '"', '"') {
                        if let Some(&mi) = g.measures_by_name.get(&name.to_lowercase()) {
                            let by_name = g
                                .site(json!({"kind": "calculation group (measure named in a string)",
                                                        "table": t, "name": s.to_json()["name"]}));
                            g.edge(from, mi, by_name);
                        }
                    }
                }
            }
            (None, SiteOwner::RowFilter { role, table }) => {
                let usage = json!({"kind": "row filter", "role": role, "table": table});
                for (to, _) in refs {
                    g.use_node(to, usage.clone());
                }
                if let Some(t) = g.find_table(table) {
                    g.use_node(t, usage);
                }
            }
            (None, SiteOwner::Function(name)) => {
                let usage = json!({"kind": "user-defined function", "name": name});
                for (to, _) in refs {
                    g.use_node(to, usage.clone());
                }
            }
            (None, _) => {}
        }
    }
}

fn table_columns(g: &Graph, table: usize) -> Vec<usize> {
    let Some(t) = g.node(table).filter(|n| n.kind == Kind::Table) else {
        return Vec::new();
    };
    let name = t.table.to_lowercase();
    g.nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.kind == Kind::Column && n.table.to_lowercase() == name)
        .map(|(i, _)| i)
        .collect()
}

fn model_usages(g: &mut Graph, m: &SemanticModel) {
    for r in &m.relationships {
        let usage = json!({"kind": "relationship", "name": r.name, "from": dax_column(&r.from_table, &r.from_column),
                           "to": dax_column(&r.to_table, &r.to_column), "active": r.is_active});
        for (t, c) in [(&r.from_table, &r.from_column), (&r.to_table, &r.to_column)] {
            if let Some(i) = g.find(Kind::Column, t, c) {
                g.use_node(i, usage.clone());
            }
        }
    }
    for t in &m.tables {
        table_usages(g, m, t);
    }
    for r in &m.roles {
        for tp in &r.table_permissions {
            let usage = json!({"kind": "object-level security", "role": r.name, "table": tp.table});
            if tp.extra.contains_key("metadataPermission")
                && let Some(i) = g.find_table(&tp.table)
            {
                g.use_node(i, usage.clone());
            }
            if let Some(list) = tp.extra.get("columnPermissions").and_then(Value::as_array) {
                for cp in list {
                    if let Some(i) = str_of(cp, "name").and_then(|c| g.find(Kind::Column, &tp.table, c)) {
                        g.use_node(i, usage.clone());
                    }
                }
            }
        }
    }
    perspective_usages(g, m);
    culture_usages(g, m);
}

fn table_usages(g: &mut Graph, m: &SemanticModel, t: &bicraft_model::Table) {
    let Some(ti) = g.find_table(&t.name) else { return };
    let auto_date = t
        .annotations
        .iter()
        .any(|a| a.name == "__PBI_LocalDateTable" || a.name == "__PBI_TemplateDateTable");
    let calc_group = t.extra.contains_key("calculationGroup");
    let whole_table_rule = if auto_date {
        Some("auto date/time table")
    } else if calc_group {
        Some("calculation group")
    } else {
        None
    };
    if let Some(rule) = whole_table_rule {
        let usage = json!({"kind": rule, "table": t.name});
        g.use_node(ti, usage.clone());
        for c in t.columns.iter().filter(|c| !c.is_row_number()) {
            if let Some(ci) = g.find(Kind::Column, &t.name, &c.name) {
                g.use_node(ci, usage.clone());
            }
        }
    }
    for h in t
        .extra
        .get("hierarchies")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let hname = str_of(h, "name").unwrap_or_default();
        for l in h.get("levels").and_then(Value::as_array).into_iter().flatten() {
            let (Some(level), Some(col)) = (str_of(l, "name"), str_of(l, "column")) else {
                continue;
            };
            if let Some(ci) = g.find(Kind::Column, &t.name, col) {
                g.use_node(
                    ci,
                    json!({"kind": "hierarchy", "table": t.name, "hierarchy": hname, "level": level}),
                );
                g.levels.insert(
                    (t.name.to_lowercase(), hname.to_lowercase(), level.to_lowercase()),
                    ci,
                );
            }
        }
    }
    for c in t.columns.iter().filter(|c| !c.is_row_number()) {
        let Some(ci) = g.find(Kind::Column, &t.name, &c.name) else {
            continue;
        };
        if let Some(target) = c.sort_by_column.as_deref()
            && let Some(si) = g.find(Kind::Column, &t.name, target)
        {
            g.use_node(si, json!({"kind": "sort by", "table": t.name, "column": c.name}));
        }
        for v in c.variations() {
            if let Some(vt) = m.variation_table(&t.name, &c.name, &v)
                && let Some(vi) = g.find_table(&vt.name)
            {
                g.use_node(
                    vi,
                    json!({"kind": "variation", "table": t.name, "column": c.name, "variation": v.name}),
                );
            }
        }
        for (prop, rule) in [
            ("isKey", "key column"),
            ("isDefaultLabel", "default label"),
            ("isDefaultImage", "default image"),
        ] {
            if c.extra.get(prop) == Some(&Value::Bool(true)) {
                g.use_node(ci, json!({"kind": rule, "table": t.name}));
            }
        }
        if let Some(alt) = c.extra.get("alternateOf") {
            g.use_node(ci, json!({"kind": "aggregation", "table": t.name}));
            let base = alt.get("baseColumn");
            let base_ref = match base {
                Some(Value::String(s)) => crate::edit::parse_column_ref(s).ok(),
                Some(o @ Value::Object(_)) => str_of(o, "table")
                    .zip(str_of(o, "column").or_else(|| str_of(o, "name")))
                    .map(|(a, b)| (a.to_string(), b.to_string())),
                _ => None,
            };
            let usage = json!({"kind": "aggregation", "table": t.name, "column": c.name});
            if let Some((bt, bc)) = base_ref
                && let Some(bi) = g.find(Kind::Column, &bt, &bc)
            {
                g.use_node(bi, usage.clone());
            }
            if let Some(bt) = alt.get("baseTable").and_then(Value::as_str)
                && let Some(bi) = g.find_table(bt)
            {
                g.use_node(bi, usage);
            }
        }
        for gb in array_of(c.extra.get("relatedColumnDetails"), "groupByColumns") {
            let target = str_of(gb, "groupingColumn").or_else(|| gb.as_str());
            if let Some(gi) = target.and_then(|x| g.find(Kind::Column, &t.name, x)) {
                let site = g.site(json!({"kind": "group by column", "table": t.name, "name": c.name}));
                g.edge(ci, gi, site);
            }
        }
    }
}

fn perspective_usages(g: &mut Graph, m: &SemanticModel) {
    for p in m
        .extra
        .get("perspectives")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let name = str_of(p, "name").unwrap_or_default();
        let usage = json!({"kind": "perspective", "perspective": name});
        for t in array_of(Some(p), "tables") {
            let Some(table) = str_of(t, "name") else { continue };
            if let Some(i) = g.find_table(table) {
                g.use_node(i, usage.clone());
            }
            for (key, kind) in [("columns", Kind::Column), ("measures", Kind::Measure)] {
                for x in array_of(Some(t), key) {
                    if let Some(i) = str_of(x, "name").and_then(|n| g.find(kind, table, n)) {
                        g.use_node(i, usage.clone());
                    }
                }
            }
        }
    }
}

fn culture_usages(g: &mut Graph, m: &SemanticModel) {
    for c in m
        .extra
        .get("cultures")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let culture = str_of(c, "name").unwrap_or_default();
        let usage = json!({"kind": "translation", "culture": culture});
        let model = c.get("translations").and_then(|t| t.get("model"));
        for t in array_of(model, "tables") {
            let Some(table) = str_of(t, "name") else { continue };
            if let Some(i) = g.find_table(table) {
                g.use_node(i, usage.clone());
            }
            for (key, kind) in [("columns", Kind::Column), ("measures", Kind::Measure)] {
                for x in array_of(Some(t), key) {
                    if let Some(i) = str_of(x, "name").and_then(|n| g.find(kind, table, n)) {
                        g.use_node(i, usage.clone());
                    }
                }
            }
        }
        let Some(lm) = c.get("linguisticMetadata").and_then(|l| l.get("content")) else {
            continue;
        };
        let parsed;
        let content = match lm {
            Value::String(s) => match serde_json::from_str::<Value>(s) {
                Ok(v) => {
                    parsed = v;
                    &parsed
                }
                Err(_) => continue,
            },
            other => other,
        };
        let usage = json!({"kind": "linguistic schema", "culture": culture});
        let mut found = Vec::new();
        conceptual_bindings(content, 0, &mut found);
        for (entity, property) in found {
            let i = match &property {
                Some(p) => g
                    .find(Kind::Column, &entity, p)
                    .or_else(|| g.find(Kind::Measure, &entity, p)),
                None => g.find_table(&entity),
            };
            if let Some(i) = i {
                g.use_node(i, usage.clone());
            }
        }
    }
}

/// `{ConceptualEntity, ConceptualProperty?}` bindings of a linguistic schema.
fn conceptual_bindings(v: &Value, depth: usize, out: &mut Vec<(String, Option<String>)>) {
    if depth > 128 {
        return;
    }
    match v {
        Value::Object(o) => {
            if let Some(e) = o.get("ConceptualEntity").and_then(Value::as_str) {
                let p = o
                    .get("ConceptualProperty")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                out.push((e.to_string(), p));
            }
            for x in o.values() {
                conceptual_bindings(x, depth.saturating_add(1), out);
            }
        }
        Value::Array(a) => {
            for x in a {
                conceptual_bindings(x, depth.saturating_add(1), out);
            }
        }
        _ => {}
    }
}

/// M partitions and shared expressions need the queries they name (tables or expressions).
fn m_edges(g: &mut Graph, m: &SemanticModel) {
    use bicraft_data::Limits;
    use bicraft_data::m::analyze_all;
    let mut owners: Vec<(usize, Value, bool)> = Vec::new();
    let mut texts = Vec::new();
    for e in &m.expressions {
        let Some(ei) = g.find(Kind::Expression, "", &e.name) else {
            continue;
        };
        if !crate::mquery::is_m(&e.kind) {
            g.use_node(ei, json!({"kind": "non-M expression", "expression_kind": e.kind}));
        } else if is_parameter(&e.expression) {
            // parameters feed data sources, dynamic M parameters and refresh policies
            g.use_node(ei, json!({"kind": "M parameter"}));
        }
    }
    for t in &m.tables {
        let Some(ti) = g.find_table(&t.name) else { continue };
        for p in &t.partitions {
            if let (true, Some(e)) = (crate::mquery::is_m(&p.source_type), p.expression.as_deref()) {
                owners.push((
                    ti,
                    json!({"kind": "M query", "table": t.name, "partition": p.name}),
                    true,
                ));
                texts.push(e.to_string());
            }
        }
    }
    for e in &m.expressions {
        if crate::mquery::is_m(&e.kind)
            && let Some(ei) = g.find(Kind::Expression, "", &e.name)
        {
            owners.push((ei, json!({"kind": "M expression", "name": e.name}), false));
            texts.push(e.expression.clone());
        }
    }
    if texts.is_empty() {
        return;
    }
    let analyses = analyze_all(texts, &Limits::default());
    for ((from, desc, partition), a) in owners.into_iter().zip(analyses) {
        let mut targets: Vec<usize> = a
            .references
            .iter()
            .filter_map(|r| {
                g.find(Kind::Expression, "", &r.name)
                    .or_else(|| g.find_table(&r.name))
            })
            .collect();
        targets.sort_unstable();
        targets.dedup();
        if targets.is_empty() {
            continue;
        }
        for &to in &targets {
            // a table's query keeps the expressions it names whatever the table
            if partition && g.node(to).is_some_and(|n| n.kind == Kind::Expression) {
                g.use_node(to, desc.clone());
            }
        }
        let site = g.site(desc);
        for to in targets {
            g.edge(from, to, site);
        }
    }
}

/// An M parameter: a value with `IsParameterQuery = true` metadata.
fn is_parameter(text: &str) -> bool {
    let compact: String = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase();
    compact.contains("isparameterquery=true")
}

fn report_usages(g: &mut Graph, label: &str, r: &Report, renamer: &Renamer<'_>) {
    let mut found: Vec<(usize, Value)> = Vec::new();
    let mut unresolved = 0usize;
    report_refs(label, r, &mut |rr, usage| match g.resolve_report_ref(&rr) {
        Some(i) => found.push((i, usage)),
        None => unresolved = unresolved.saturating_add(1),
    });
    for (usage, refs, text) in report_measure_refs(label, r, renamer) {
        let targets: Vec<usize> = match refs {
            Some(refs) => refs.iter().filter_map(|x| g.resolve_ref(x)).collect(),
            None => g.mentions(&text),
        };
        for t in targets {
            found.push((t, usage.clone()));
        }
    }
    for (i, u) in found {
        g.use_node(i, u);
    }
    g.unresolved_report_refs = g.unresolved_report_refs.saturating_add(unresolved);
    g.reports.push(json!({
        "report": label,
        "pages": r.pages.len(),
        "visuals": r.pages.iter().map(|p| p.visuals.len()).sum::<usize>(),
        "bookmarks": r.bookmarks.len(),
        "report_measures": crate::report_measures::report_measures(r).len(),
    }));
}

/// Sorts and deduplicates, builds the reverse edges.
fn finish(g: &mut Graph) {
    for u in &mut g.usages {
        let mut keyed: Vec<(UsageKey, Value)> = u.drain(..).map(|v| (usage_key(&v), v)).collect();
        keyed.sort_by(|a, b| a.0.cmp(&b.0));
        keyed.dedup_by(|a, b| a.0 == b.0);
        // a visual field found through its query without a role, and with one: keep the role
        let with_role: HashSet<(String, String, String, String)> = keyed
            .iter()
            .filter(|(_, v)| v["kind"] == "visual" && v.get("role").is_some())
            .map(|(_, v)| visual_key(v))
            .collect();
        u.extend(
            keyed
                .into_iter()
                .filter(|(_, v)| {
                    !(v["kind"] == "visual" && v.get("role").is_none() && with_role.contains(&visual_key(v)))
                })
                .map(|(_, v)| v),
        );
    }
    for e in &mut g.deps {
        e.sort_unstable();
        e.dedup();
    }
    g.rdeps = vec![Vec::new(); g.nodes.len()];
    for (from, edges) in g.deps.iter().enumerate() {
        for e in edges {
            if let Some(r) = g.rdeps.get_mut(e.to) {
                r.push((from, e.site));
            }
        }
    }
    if g.unparsed > 0 {
        g.warnings.push(format!(
            "{} DAX expression(s) do not parse: the objects they name ([Name], 'Table') count as used",
            g.unparsed
        ));
    }
    if g.unresolved_report_refs > 0 {
        g.warnings.push(format!(
            "{} report reference(s) name fields the model does not have (report.validate lists them)",
            g.unresolved_report_refs
        ));
    }
}

fn visual_key(v: &Value) -> (String, String, String, String) {
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    (s("report"), s("page"), s("visual"), s("kind"))
}

fn is_report_usage(u: &Value) -> bool {
    u.get("kind")
        .and_then(Value::as_str)
        .is_some_and(|k| REPORT_KINDS.contains(&k))
}

// ---------------------------------------------------------------------------------------------
// Verdicts
// ---------------------------------------------------------------------------------------------

/// Which nodes are used, and how.
struct Verdict {
    used: Vec<bool>,
    by_report: Vec<bool>,
    /// The node that made a node used (`None` for a node with its own usage).
    parent: Vec<Option<usize>>,
}

fn reach(g: &Graph, seed: impl Fn(&[Value]) -> bool) -> (Vec<bool>, Vec<Option<usize>>) {
    let n = g.nodes.len();
    let mut seen = vec![false; n];
    let mut parent = vec![None; n];
    let mut queue = VecDeque::new();
    for (i, u) in g.usages.iter().enumerate() {
        if seed(u)
            && let Some(s) = seen.get_mut(i)
        {
            *s = true;
            queue.push_back(i);
        }
    }
    while let Some(i) = queue.pop_front() {
        for e in g.deps.get(i).into_iter().flatten() {
            if let Some(s) = seen.get_mut(e.to)
                && !*s
            {
                *s = true;
                if let Some(p) = parent.get_mut(e.to) {
                    *p = Some(i);
                }
                queue.push_back(e.to);
            }
        }
    }
    (seen, parent)
}

fn verdict(g: &Graph) -> Verdict {
    let (used, parent) = reach(g, |u| !u.is_empty());
    let (by_report, _) = reach(g, |u| u.iter().any(is_report_usage));
    Verdict {
        used,
        by_report,
        parent,
    }
}

fn flag(v: &[bool], i: usize) -> bool {
    v.get(i).copied().unwrap_or(false)
}

/// The unused measures, columns, calculated tables (a table's members are not listed again) and
/// shared M expressions.
struct Unused {
    tables: Vec<usize>,
    columns: Vec<usize>,
    measures: Vec<usize>,
    expressions: Vec<usize>,
}

fn sort_key(g: &Graph, i: usize) -> (String, String) {
    g.node(i)
        .map(|n| (n.table.to_lowercase(), n.name.to_lowercase()))
        .unwrap_or_default()
}

fn unused(g: &Graph, v: &Verdict) -> Unused {
    let mut tables = Vec::new();
    let mut columns = Vec::new();
    let mut measures = Vec::new();
    let mut expressions = Vec::new();
    let mut gone_tables = HashSet::new();
    for (i, n) in g.nodes.iter().enumerate() {
        if n.kind == Kind::Table && n.calculated_table && !flag(&v.used, i) {
            tables.push(i);
            gone_tables.insert(n.table.to_lowercase());
        }
    }
    for (i, n) in g.nodes.iter().enumerate() {
        if flag(&v.used, i) || gone_tables.contains(&n.table.to_lowercase()) {
            continue;
        }
        match n.kind {
            Kind::Column => columns.push(i),
            Kind::Measure => measures.push(i),
            Kind::Expression => expressions.push(i),
            Kind::Table => {}
        }
    }
    for list in [&mut tables, &mut columns, &mut measures, &mut expressions] {
        list.sort_by_key(|&i| sort_key(g, i));
    }
    Unused {
        tables,
        columns,
        measures,
        expressions,
    }
}

/// Why an unused node is unused.
fn why_unused(g: &Graph, i: usize) -> String {
    let users: Vec<String> = g
        .rdeps
        .get(i)
        .into_iter()
        .flatten()
        .filter(|(_, site)| *site != MEMBER)
        .map(|(from, _)| g.label(*from))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let is_table = g.node(i).is_some_and(|n| n.kind == Kind::Table);
    if !users.is_empty() {
        let more = users.len().saturating_sub(5);
        let mut list = users.into_iter().take(5).collect::<Vec<_>>().join(", ");
        if more > 0 {
            list.push_str(&format!(" and {more} more"));
        }
        format!("used only by unused objects: {list}")
    } else if is_table {
        "no column or measure of it is used, and nothing references it".into()
    } else {
        "referenced nowhere".into()
    }
}

// ---------------------------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------------------------

/// Reports read (`(label, report)`) and warnings.
type Loaded = (Vec<(String, Report)>, Vec<String>);

/// The `reports` parameter's reports, read (the open file's report is the caller's).
fn load_reports(s: &Session, p: &Value) -> Result<Loaded, String> {
    let mut warnings = Vec::new();
    let paths: Vec<String> = match p.get("reports") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::String(x)) => vec![x.clone()],
        Some(Value::Array(a)) => a
            .iter()
            .map(|x| {
                x.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "`reports` must be an array of paths".to_string())
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err("`reports` must be an array of paths".into()),
    };
    if paths.len() > MAX_REPORTS {
        return Err(format!("`reports`: at most {MAX_REPORTS} reports"));
    }
    if s.report.is_none()
        && let Some(e) = &s.report_error
    {
        warnings.push(format!(
            "the open file's report could not be read ({e}): its usage is unknown"
        ));
    }
    let mut out = Vec::new();
    let mut names: HashSet<String> = HashSet::new();
    names.insert(OPEN_REPORT.into());
    for raw in paths {
        let path = PathBuf::from(&raw);
        let report = read_report(s, &path).map_err(|e| format!("reports: {}: {e}", file_label(&path)))?;
        let base = file_label(&path);
        let mut label = base.clone();
        let mut n = 1usize;
        while !names.insert(label.to_lowercase()) {
            n = n.saturating_add(1);
            label = format!("{base} ({n})");
        }
        out.push((label, report));
    }
    Ok((out, warnings))
}

/// The last component of a path: reports are named by it, never by a full (server) path.
fn file_label(path: &std::path::Path) -> String {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("report")
        .to_string();
    if name == ".pbip" || name.is_empty() {
        "report".into()
    } else {
        name
    }
}

/// Reads the report of a .pbix/.pbit file, a .pbip file, a project folder or a `.Report` folder.
fn read_report(s: &Session, path: &std::path::Path) -> Result<Report, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("pbix" | "pbit") => {
            let package = bicraft_opc::Package::open_deferring(
                path,
                &bicraft_opc::ReadLimits::default(),
                &[bicraft_pbix::DATA_MODEL],
            )
            .map_err(|e| e.to_string())?;
            bicraft_pbix::read_report(&package).map_err(|e| e.to_string())
        }
        _ if path.is_dir() || ext.as_deref() == Some("pbip") => {
            let project = bicraft_pbip::locate(path).map_err(|e| e.to_string())?;
            if let Some(server) = s.server_config() {
                server.check_project(&project)?;
            }
            if project.report.is_none() {
                return Err("no report found (expected a <Name>.Report folder)".into());
            }
            bicraft_pbip::read_report(&project).map_err(|e| e.to_string())
        }
        _ => Err("expected a .pbix, .pbit, .pbip, project folder or <Name>.Report folder".into()),
    }
}

fn coverage_warning(reports: usize) -> String {
    if reports == 0 {
        "no report was read: report usage is unknown, so objects used only by reports are listed as \
         unused. Open the file with its report or pass `reports`"
            .into()
    } else {
        format!(
            "report usage covers {reports} report(s); a semantic model can serve other reports (thin \
             reports bound to it, copies in other workspaces): objects only they use are listed as \
             unused here. Pass every report with `reports`"
        )
    }
}

/// Builds the graph over the session's model and reports.
fn graph_of(s: &Session, p: &Value) -> Result<Graph, String> {
    let m = model(s)?;
    let (extra, mut warnings) = load_reports(s, p)?;
    let mut reports: Vec<(String, &Report)> = Vec::new();
    if let Some(r) = s.report.as_ref() {
        reports.push((OPEN_REPORT.to_string(), r));
    }
    for (label, r) in &extra {
        reports.push((label.clone(), r));
    }
    let mut g = build(m, &reports);
    warnings.push(coverage_warning(reports.len()));
    warnings.append(&mut g.warnings);
    g.warnings = warnings;
    Ok(g)
}

/// Strips `'…'` around a table name.
fn unquote(s: &str) -> String {
    let t = s.trim();
    match t.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')) {
        Some(inner) => inner.replace("''", "'"),
        None => t.to_string(),
    }
}

/// The node an `object` parameter names.
fn resolve_object(g: &Graph, text: &str, kind: Option<&str>) -> Result<usize, String> {
    let missing = || format!("the model has no {} `{text}`", kind.unwrap_or("object"));
    match kind {
        None | Some("table" | "column" | "measure" | "expression") => {}
        Some(k) => {
            return Err(format!(
                "`kind` must be table, column, measure or expression, not `{k}`"
            ));
        }
    }
    if kind == Some("expression") {
        return g.find(Kind::Expression, "", text.trim()).ok_or_else(missing);
    }
    if kind == Some("table") || !text.contains('[') {
        return g
            .find_table(&unquote(text))
            .or_else(|| {
                (kind.is_none())
                    .then(|| g.find(Kind::Expression, "", text.trim()))
                    .flatten()
            })
            .ok_or_else(missing);
    }
    match parse_field(text)? {
        ParsedField::Measure(name) => {
            let measure = g.measures_by_name.get(&name.to_lowercase()).copied();
            let columns = g.columns_by_name.get(&name.to_lowercase());
            match (kind, measure, columns) {
                (Some("column"), _, Some(c)) | (None, None, Some(c)) if c.len() == 1 => {
                    c.first().copied().ok_or_else(missing)
                }
                (Some("column"), _, Some(_)) | (None, None, Some(_)) => Err(format!(
                    "several tables have a column `{name}`: name it as Table[{name}]"
                )),
                (Some("column"), _, None) => Err(missing()),
                (_, Some(m), _) => Ok(m),
                _ => Err(missing()),
            }
        }
        ParsedField::Qualified { table, name } => {
            let column = g.find(Kind::Column, &table, &name);
            let measure = g.find(Kind::Measure, &table, &name);
            match kind {
                Some("measure") => measure,
                Some("column") => column,
                _ => column.or(measure),
            }
            .ok_or_else(missing)
        }
        ParsedField::Aggregation { table, column, .. } => {
            g.find(Kind::Column, &table, &column).ok_or_else(missing)
        }
    }
}

/// Glob match (`*`, `?`), case-insensitive.
fn glob(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        match p.get(pi) {
            Some('*') => {
                star = Some((pi, ti));
                pi = pi.saturating_add(1);
            }
            Some(&c) if c == '?' || Some(&c) == t.get(ti) => {
                pi = pi.saturating_add(1);
                ti = ti.saturating_add(1);
            }
            _ => match star {
                Some((sp, st)) => {
                    pi = sp.saturating_add(1);
                    ti = st.saturating_add(1);
                    star = Some((sp, ti));
                }
                None => return false,
            },
        }
    }
    p.get(pi..).is_some_and(|rest| rest.iter().all(|&c| c == '*'))
}

/// The names an allow-list pattern may match a node by.
fn names_of(n: &Node) -> Vec<String> {
    match n.kind {
        Kind::Table => vec![n.name.clone(), dax_table(&n.name), format!("'{}'", n.name)],
        Kind::Column => vec![
            format!("{}[{}]", n.table, n.name),
            dax_column(&n.table, &n.name),
            format!("'{}'[{}]", n.table, n.name),
        ],
        Kind::Measure => vec![
            dax_measure(&n.name),
            format!("{}[{}]", n.table, n.name),
            dax_column(&n.table, &n.name),
        ],
        Kind::Expression => vec![n.name.clone()],
    }
}

/// Adds the `allow` patterns as usages; returns the patterns that matched nothing.
fn apply_allow(g: &mut Graph, p: &Value) -> Result<Vec<String>, String> {
    let patterns: Vec<String> = match p.get("allow") {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a
            .iter()
            .map(|x| {
                x.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "`allow` must be an array of names or patterns".to_string())
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err("`allow` must be an array of names or patterns".into()),
    };
    if patterns.len() > MAX_ALLOW {
        return Err(format!("`allow`: at most {MAX_ALLOW} patterns"));
    }
    if let Some(long) = patterns.iter().find(|x| x.chars().count() > MAX_PATTERN_CHARS) {
        return Err(format!(
            "`allow`: pattern longer than {MAX_PATTERN_CHARS} characters: {}…",
            long.chars().take(40).collect::<String>()
        ));
    }
    let mut unmatched = Vec::new();
    let mut hits: Vec<(usize, String)> = Vec::new();
    let all_names: Vec<Vec<String>> = g.nodes.iter().map(names_of).collect();
    for pat in &patterns {
        let before = hits.len();
        for (i, names) in all_names.iter().enumerate() {
            if names.iter().any(|name| glob(pat, name)) {
                hits.push((i, pat.clone()));
            }
        }
        if hits.len() == before {
            unmatched.push(pat.clone());
        }
    }
    for (i, pat) in hits {
        g.use_node(i, json!({"kind": "allow-list", "pattern": pat}));
    }
    Ok(unmatched)
}

// ---------------------------------------------------------------------------------------------
// model.usage
// ---------------------------------------------------------------------------------------------

/// The direct usages of node `i`: its own usages, then the expressions and properties of other
/// objects that need it (members of a table excepted).
fn direct_of(g: &Graph, i: usize) -> Vec<Value> {
    let mut out: Vec<Value> = g.usages.get(i).cloned().unwrap_or_default();
    let mut users: Vec<Value> = g
        .rdeps
        .get(i)
        .into_iter()
        .flatten()
        .filter(|(_, site)| *site != MEMBER)
        .filter_map(|(_, site)| g.sites.get(*site).cloned())
        .collect();
    users.sort_by_cached_key(usage_key);
    users.dedup();
    // a table's M query is both a usage of the expression it names and an edge to it
    users.retain(|u| !out.contains(u));
    out.extend(users);
    out
}

fn capped(list: Vec<Value>, key: &str, out: &mut Map<String, Value>) {
    let total = list.len();
    out.insert(
        key.into(),
        Value::Array(list.into_iter().take(MAX_LISTED).collect()),
    );
    if total > MAX_LISTED {
        out.insert(format!("{key}_total"), json!(total));
    }
}

pub(crate) fn model_usage(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["object", "kind", "reports", "graph"])?;
    let object = opt_str(p, "object")?;
    let kind = opt_str(p, "kind")?;
    let with_graph = opt_bool(p, "graph")?;
    let g = graph_of(s, p)?;
    let v = verdict(&g);
    let mut out = Map::new();
    match object {
        Some(text) => {
            let i = resolve_object(&g, text, kind)?;
            one_object(&g, &v, i, with_graph.unwrap_or(true), &mut out);
        }
        None => all_objects(&g, &v, kind, with_graph.unwrap_or(true), &mut out)?,
    }
    out.insert("reports".into(), json!(g.reports));
    out.insert("warnings".into(), json!(g.warnings));
    Ok(Value::Object(out))
}

fn one_object(g: &Graph, v: &Verdict, i: usize, with_graph: bool, out: &mut Map<String, Value>) {
    out.insert("object".into(), g.object(i));
    out.insert("used".into(), json!(flag(&v.used, i)));
    out.insert("used_by_report".into(), json!(flag(&v.by_report, i)));
    capped(direct_of(g, i), "direct", out);

    // dependants: breadth first over the reverse edges, with the chain back to the object
    let n = g.nodes.len();
    let mut via: Vec<Option<usize>> = vec![None; n];
    let mut depth = vec![0usize; n];
    let mut seen = vec![false; n];
    if let Some(s) = seen.get_mut(i) {
        *s = true;
    }
    let mut order = Vec::new();
    let mut queue = VecDeque::from([i]);
    while let Some(x) = queue.pop_front() {
        for (from, _) in g.rdeps.get(x).into_iter().flatten() {
            if let Some(s) = seen.get_mut(*from)
                && !*s
            {
                *s = true;
                if let Some(slot) = via.get_mut(*from) {
                    *slot = Some(x);
                }
                let d = depth.get(x).copied().unwrap_or(0).saturating_add(1);
                if let Some(slot) = depth.get_mut(*from) {
                    *slot = d;
                }
                order.push(*from);
                queue.push_back(*from);
            }
        }
    }
    let chain = |mut x: usize| {
        let mut labels = Vec::new();
        while x != i && labels.len() <= n {
            labels.push(g.label(x));
            match via.get(x).copied().flatten() {
                Some(next) => x = next,
                None => break,
            }
        }
        labels.reverse();
        labels
    };
    let mut indirect = Vec::new();
    let mut dependants = Vec::new();
    for &d in &order {
        let mut o = Map::new();
        o.insert("object".into(), g.object(d));
        o.insert("depth".into(), json!(depth.get(d).copied().unwrap_or(0)));
        o.insert("used".into(), json!(flag(&v.used, d)));
        dependants.push(Value::Object(o));
        for u in g.usages.get(d).into_iter().flatten() {
            indirect.push(json!({"through": chain(d), "usage": u}));
        }
    }
    capped(indirect, "indirect", out);
    capped(dependants, "dependants", out);

    let direct_deps: Vec<usize> = {
        let mut d: Vec<usize> = g
            .deps
            .get(i)
            .into_iter()
            .flatten()
            .filter(|e| e.site != MEMBER)
            .map(|e| e.to)
            .collect();
        d.dedup();
        d
    };
    out.insert(
        "depends_on".into(),
        Value::Array(direct_deps.iter().map(|&d| g.object(d)).collect()),
    );
    let mut all = vec![false; n];
    let mut queue: VecDeque<usize> = direct_deps.iter().copied().collect();
    for &d in &direct_deps {
        if let Some(s) = all.get_mut(d) {
            *s = true;
        }
    }
    let mut closure = Vec::new();
    while let Some(x) = queue.pop_front() {
        closure.push(x);
        for e in g.deps.get(x).into_iter().flatten().filter(|e| e.site != MEMBER) {
            if let Some(s) = all.get_mut(e.to)
                && !*s
                && e.to != i
            {
                *s = true;
                queue.push_back(e.to);
            }
        }
    }
    closure.sort_by_key(|&x| (g.node(x).map(|n| n.kind), sort_key(g, x)));
    capped(
        closure.iter().map(|&d| g.object(d)).collect(),
        "depends_on_all",
        out,
    );
    if with_graph {
        let mut members: Vec<usize> = vec![i];
        members.extend(order.iter().copied());
        members.extend(closure.iter().copied());
        members.sort_unstable();
        members.dedup();
        out.insert("graph".into(), subgraph(g, &members));
    }
}

/// `{nodes: [object], edges: [[from, to, kind]]}` over `members` (sorted node indexes); `from`
/// needs `to`.
fn subgraph(g: &Graph, members: &[usize]) -> Value {
    let pos: HashMap<usize, usize> = members.iter().enumerate().map(|(k, &i)| (i, k)).collect();
    let mut edges = Vec::new();
    for (k, &i) in members.iter().enumerate() {
        for e in g.deps.get(i).into_iter().flatten() {
            if let Some(&to) = pos.get(&e.to) {
                let kind = g
                    .sites
                    .get(e.site)
                    .and_then(|s| s.get("kind"))
                    .cloned()
                    .unwrap_or(Value::Null);
                edges.push(json!([k, to, kind]));
            }
        }
    }
    edges.dedup();
    json!({
        "nodes": members.iter().map(|&i| g.object(i)).collect::<Vec<_>>(),
        "edges": edges,
    })
}

fn all_objects(
    g: &Graph,
    v: &Verdict,
    kind: Option<&str>,
    with_graph: bool,
    out: &mut Map<String, Value>,
) -> Result<(), String> {
    let filter = match kind {
        None => None,
        Some("table") => Some(Kind::Table),
        Some("column") => Some(Kind::Column),
        Some("measure") => Some(Kind::Measure),
        Some("expression") => Some(Kind::Expression),
        Some(k) => {
            return Err(format!(
                "`kind` must be table, column, measure or expression, not `{k}`"
            ));
        }
    };
    let mut objects = Vec::new();
    let (mut used, mut by_report) = (0usize, 0usize);
    for (i, n) in g.nodes.iter().enumerate() {
        if filter.is_some_and(|k| k != n.kind) {
            continue;
        }
        let direct = direct_of(g, i);
        used = used.saturating_add(usize::from(flag(&v.used, i)));
        by_report = by_report.saturating_add(usize::from(flag(&v.by_report, i)));
        objects.push(json!({
            "object": n.json(),
            "used": flag(&v.used, i),
            "used_by_report": flag(&v.by_report, i),
            "direct_count": direct.len(),
            "used_by": direct,
        }));
    }
    out.insert(
        "counts".into(),
        json!({"objects": objects.len(), "used": used, "used_by_report": by_report}),
    );
    capped(objects, "objects", out);
    if with_graph {
        let members: Vec<usize> = (0..g.nodes.len()).collect();
        out.insert("graph".into(), subgraph(g, &members));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// model.unused
// ---------------------------------------------------------------------------------------------

/// The `model.unused` analysis.
struct Analysis {
    g: Graph,
    v: Verdict,
    unused: Unused,
    unmatched_allow: Vec<String>,
}

fn analysis(s: &Session, p: &Value) -> Result<Analysis, String> {
    let keep_metadata = opt_bool(p, "keep_metadata")?.unwrap_or(true);
    let mut g = graph_of(s, p)?;
    if !keep_metadata {
        for u in &mut g.usages {
            u.retain(|x| !x["kind"].as_str().is_some_and(|k| METADATA_KINDS.contains(&k)));
        }
    }
    let unmatched_allow = apply_allow(&mut g, p)?;
    let v = verdict(&g);
    let unused = unused(&g, &v);
    Ok(Analysis {
        g,
        v,
        unused,
        unmatched_allow,
    })
}

fn unused_entry(g: &Graph, i: usize) -> Value {
    let mut o = match g.object(i) {
        Value::Object(o) => o,
        _ => Map::new(),
    };
    o.insert("why".into(), json!(why_unused(g, i)));
    if let Some(n) = g.node(i)
        && n.kind == Kind::Table
    {
        let name = n.table.to_lowercase();
        let count = |k: Kind| {
            g.nodes
                .iter()
                .filter(|x| x.kind == k && x.table.to_lowercase() == name)
                .count()
        };
        o.insert(
            "contains".into(),
            json!({"columns": count(Kind::Column), "measures": count(Kind::Measure)}),
        );
    }
    Value::Object(o)
}

/// The objects no report uses that a model rule keeps, with the rule.
fn kept(g: &Graph, v: &Verdict) -> Vec<Value> {
    let mut out = Vec::new();
    let mut idx: Vec<usize> = (0..g.nodes.len())
        .filter(|&i| {
            g.node(i).is_some_and(|n| match n.kind {
                Kind::Measure | Kind::Column => true,
                Kind::Table => n.calculated_table,
                Kind::Expression => false,
            }) && flag(&v.used, i)
                && !flag(&v.by_report, i)
        })
        .collect();
    idx.sort_by_key(|&i| (g.node(i).map(|n| n.kind), sort_key(g, i)));
    for i in idx {
        let own = g.usages.get(i).map(Vec::as_slice).unwrap_or_default();
        let mut o = match g.object(i) {
            Value::Object(o) => o,
            _ => Map::new(),
        };
        if own.is_empty() {
            let parent = v.parent.get(i).copied().flatten();
            o.insert("rules".into(), json!(["needed by a kept object"]));
            if let Some(p) = parent {
                o.insert("needed_by".into(), g.object(p));
            }
        } else {
            let mut rules: Vec<&str> = own.iter().filter_map(|u| u["kind"].as_str()).collect();
            rules.sort_unstable();
            rules.dedup();
            o.insert("rules".into(), json!(rules));
            o.insert("detail".into(), own.first().cloned().unwrap_or(Value::Null));
        }
        out.push(Value::Object(o));
    }
    out
}

pub(crate) fn model_unused(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["reports", "allow", "kept", "keep_metadata"])?;
    let with_kept = opt_bool(p, "kept")?.unwrap_or(true);
    let a = analysis(s, p)?;
    let g = &a.g;
    let mut out = Map::new();
    let list = |ids: &[usize]| Value::Array(ids.iter().map(|&i| unused_entry(g, i)).collect());
    out.insert(
        "unused".into(),
        json!({
            "tables": list(&a.unused.tables),
            "columns": list(&a.unused.columns),
            "measures": list(&a.unused.measures),
            "expressions": list(&a.unused.expressions),
        }),
    );
    let kept = kept(g, &a.v);
    out.insert(
        "counts".into(),
        json!({
            "tables": a.unused.tables.len(),
            "columns": a.unused.columns.len(),
            "measures": a.unused.measures.len(),
            "expressions": a.unused.expressions.len(),
            "kept": kept.len(),
            "measures_in_model": g.nodes.iter().filter(|n| n.kind == Kind::Measure).count(),
            "columns_in_model": g.nodes.iter().filter(|n| n.kind == Kind::Column).count(),
        }),
    );
    if with_kept {
        capped(kept, "kept", &mut out);
    }
    let mut warnings = g.warnings.clone();
    if !a.unmatched_allow.is_empty() {
        warnings.push(format!(
            "allow-list pattern(s) matching nothing: {}",
            a.unmatched_allow.join(", ")
        ));
    }
    out.insert("reports".into(), json!(g.reports));
    out.insert("warnings".into(), json!(warnings));
    Ok(Value::Object(out))
}

// ---------------------------------------------------------------------------------------------
// model.remove_unused
// ---------------------------------------------------------------------------------------------

pub(crate) fn model_remove_unused(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &["objects", "reports", "allow", "keep_metadata", "dry_run", "force"],
    )?;
    let dry_run = opt_bool(p, "dry_run")?.unwrap_or(false);
    let force = force(p)?;
    let a = analysis(s, p)?;
    let g = &a.g;
    let unused_set: HashSet<usize> = a
        .unused
        .tables
        .iter()
        .chain(&a.unused.columns)
        .chain(&a.unused.measures)
        .chain(&a.unused.expressions)
        .copied()
        .collect();
    // members of an unused table may be named on their own
    let in_unused_table = |i: usize| {
        g.node(i).is_some_and(|n| {
            a.unused.tables.iter().any(|&t| {
                g.node(t)
                    .is_some_and(|tn| tn.table.eq_ignore_ascii_case(&n.table))
            })
        })
    };
    let selected: Vec<usize> = match p.get("objects") {
        None | Some(Value::Null) => {
            let mut all: Vec<usize> = a.unused.tables.clone();
            all.extend(&a.unused.columns);
            all.extend(&a.unused.measures);
            all.extend(&a.unused.expressions);
            all
        }
        Some(Value::Array(items)) => {
            let mut out = Vec::new();
            let mut used = Vec::new();
            for item in items {
                let text = item
                    .as_str()
                    .ok_or("`objects` must be an array of names (Table[Column], [Measure], 'Table')")?;
                let i = resolve_object(g, text, None)?;
                if unused_set.contains(&i) || in_unused_table(i) {
                    out.push(i);
                } else {
                    let first = g
                        .usages
                        .get(i)
                        .and_then(|u| u.first())
                        .map(|u| format!(" ({})", u["kind"].as_str().unwrap_or("used")))
                        .unwrap_or_default();
                    used.push(format!("{}{first}", g.label(i)));
                }
            }
            if !used.is_empty() {
                return Err(format!(
                    "not unused, nothing was removed: {} (model.usage shows where they are used)",
                    used.join("; ")
                ));
            }
            out
        }
        Some(_) => return Err("`objects` must be an array of names".into()),
    };
    // a table's members go with it
    let mut tables: Vec<usize> = selected
        .iter()
        .copied()
        .filter(|&i| g.node(i).is_some_and(|n| n.kind == Kind::Table))
        .collect();
    tables.sort_by_key(|&i| sort_key(g, i));
    tables.dedup();
    let table_names: HashSet<String> = tables
        .iter()
        .filter_map(|&i| g.node(i).map(|n| n.table.to_lowercase()))
        .collect();
    let mut members: Vec<usize> = selected
        .iter()
        .copied()
        .filter(|&i| {
            g.node(i).is_some_and(|n| {
                matches!(n.kind, Kind::Column | Kind::Measure | Kind::Expression)
                    && !table_names.contains(&n.table.to_lowercase())
            })
        })
        .collect();
    members.sort_by_key(|&i| (g.node(i).map(|n| n.kind), sort_key(g, i)));
    members.dedup();
    let plan: Vec<Node> = tables
        .iter()
        .chain(&members)
        .filter_map(|&i| g.node(i).cloned())
        .collect();
    let mut warnings = g.warnings.clone();
    if !a.unmatched_allow.is_empty() {
        warnings.push(format!(
            "allow-list pattern(s) matching nothing: {}",
            a.unmatched_allow.join(", ")
        ));
    }
    let reports = g.reports.clone();
    if plan.is_empty() {
        return Ok(
            json!({"removed": {"tables": [], "columns": [], "measures": [], "expressions": []}, "count": 0,
                         "dry_run": dry_run, "broken": [], "reports": reports, "warnings": warnings}),
        );
    }

    if plan.iter().any(|n| n.kind == Kind::Expression) {
        crate::partition::refuse_on_pbix(s, "removing a shared expression")?;
        // M is not validated by the edit: an expression left behind must not name a removed one
        let removed: HashSet<usize> = tables.iter().chain(&members).copied().collect();
        let mut left: Vec<String> = Vec::new();
        for &i in members
            .iter()
            .filter(|&&i| g.node(i).is_some_and(|n| n.kind == Kind::Expression))
        {
            for (from, _) in g.rdeps.get(i).into_iter().flatten() {
                if !removed.contains(from) {
                    left.push(format!("{} (uses {})", g.label(*from), g.label(i)));
                }
            }
        }
        if !left.is_empty() && !force {
            left.sort();
            left.dedup();
            return Err(format!(
                "removing these expressions would break the M of: {}. Nothing was removed; include those objects too, or pass \"force\": true",
                left.join("; ")
            ));
        }
    }
    let saved = (s.model.clone(), s.model_error.clone(), s.dirty);
    let apply_params = json!({"force": force});
    let result = crate::edit::apply(s, &apply_params, |m| {
        let mut relationships = Vec::new();
        let mut removed = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for n in &plan {
            let (done, _change) = match n.kind {
                Kind::Table => crate::edit::remove_table_in(m, &n.name)?,
                Kind::Column => crate::edit::remove_column_in(m, &n.table, &n.name)?,
                Kind::Measure => crate::edit::remove_measure_in(m, &n.name)?,
                Kind::Expression => {
                    let before = m.expressions.len();
                    m.expressions.retain(|e| !e.name.eq_ignore_ascii_case(&n.name));
                    if m.expressions.len() == before {
                        return Err(format!("internal: no expression `{}`", n.name));
                    }
                    removed.3.push(json!(n.name));
                    continue;
                }
            };
            if let Some(Value::Array(r)) = done.get("removed_relationships") {
                relationships.extend(r.iter().cloned());
            }
            match n.kind {
                Kind::Table => removed.0.push(json!(n.name)),
                Kind::Column => removed.1.push(json!({"table": n.table, "name": n.name})),
                _ => removed.2.push(json!({"table": n.table, "name": n.name})),
            }
        }
        let count = removed.0.len() + removed.1.len() + removed.2.len() + removed.3.len();
        Ok(Edit::new(json!({
            "removed": {"tables": removed.0, "columns": removed.1, "measures": removed.2, "expressions": removed.3},
            "count": count,
            "removed_relationships": relationships,
        }))
        .breaking())
    });
    let mut out = match result {
        Ok(Value::Object(o)) => o,
        Ok(_) => Map::new(),
        Err(e) => {
            restore(s, saved);
            return Err(e);
        }
    };
    let broken = out.get("broken").and_then(Value::as_array).map_or(0, Vec::len);
    if broken > 0 && !force && !dry_run {
        let names: Vec<String> = out
            .get("broken")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
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
        restore(s, saved);
        return Err(format!(
            "removing these objects would break {broken} DAX expression(s): {}. Nothing was removed; \
             include those objects too, or pass \"force\": true",
            names.join("; ")
        ));
    }
    if dry_run {
        restore(s, saved);
    }
    if let Some(Value::Array(w)) = out.get_mut("warnings") {
        let mut all: Vec<Value> = warnings.into_iter().map(Value::String).collect();
        all.append(w);
        *w = all;
    }
    out.insert("dry_run".into(), json!(dry_run));
    out.insert("reports".into(), json!(reports));
    Ok(Value::Object(out))
}

fn restore(s: &mut Session, saved: (Option<SemanticModel>, Option<String>, bool)) {
    s.model = saved.0;
    s.model_error = saved.1;
    s.dirty = saved.2;
}

#[cfg(test)]
pub(crate) fn glob_for_tests(pattern: &str, text: &str) -> bool {
    glob(pattern, text)
}

/// Sort key of a usage: kind, report, page, visual, table, name (case-insensitive), then the
/// whole object.
type UsageKey = ([String; 6], String);

fn usage_key(v: &Value) -> UsageKey {
    let s = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_lowercase()
    };
    (
        [
            s("kind"),
            s("report"),
            s("page"),
            s("visual"),
            s("table"),
            s("name"),
        ],
        v.to_string(),
    )
}
