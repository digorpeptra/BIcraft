//! The model half of the diff.

use std::collections::HashMap;

use bicraft_model::{
    Annotation, Column, Measure, NamedExpression, Partition, Relationship, Role, SemanticModel, Table,
};
use serde_json::{Map, Value, json};

use crate::json::{Differ, clip, element_name};
use crate::norm::{self, Same};
use crate::pair::{Item, pair};
use crate::{Area, Change, How, Kind, Options, Prop, bracket, quote_table};

pub(crate) const SECTION_MODEL: u8 = 0;
pub(crate) const SECTION_TABLES: u8 = 1;
pub(crate) const SECTION_RELATIONSHIPS: u8 = 2;
pub(crate) const SECTION_ROLES: u8 = 3;
pub(crate) const SECTION_PERSPECTIVES: u8 = 4;
pub(crate) const SECTION_CULTURES: u8 = 5;
pub(crate) const SECTION_EXPRESSIONS: u8 = 6;
pub(crate) const SECTION_DATA_SOURCES: u8 = 7;
pub(crate) const SECTION_DATA: u8 = 8;

const RANK_TABLE: u8 = 0;
const RANK_COLUMN: u8 = 1;
const RANK_MEASURE: u8 = 2;
const RANK_HIERARCHY: u8 = 3;
const RANK_CALC_ITEM: u8 = 4;
const RANK_PARTITION: u8 = 5;

/// Annotations Desktop rewrites on every save (its version, the tools that touched the file).
pub const NOISY_ANNOTATIONS: &[&str] = &["PBIDesktopVersion", "PBI_ProTooling"];

/// Most names listed in an added or removed table's summary.
const MAX_LISTED: usize = 200;

/// Model-level collections compared as objects of their own, not as properties.
const MODEL_COLLECTIONS: &[&str] = &["perspectives", "cultures", "dataSources"];
/// Table-level collections compared as objects of their own.
const TABLE_COLLECTIONS: &[&str] = &["hierarchies", "calculationGroup"];

#[derive(Clone, Copy)]
pub(crate) enum Lang {
    Dax,
    M,
}

/// Compares two expressions; pushes the property when they differ.
pub(crate) fn expr(
    props: &mut Vec<Prop>,
    name: &str,
    a: Option<&str>,
    b: Option<&str>,
    lang: Lang,
    opts: &Options,
) {
    let text = |s: &str| json!(norm::lf(s));
    match (a, b) {
        (None, None) => {}
        (Some(x), Some(y)) => {
            let same = match lang {
                Lang::Dax => norm::dax(x, y),
                Lang::M => norm::m(x, y),
            };
            match same {
                Same::Identical => {}
                Same::Cosmetic if opts.ignore_whitespace => {}
                Same::Cosmetic => props.push(Prop {
                    cosmetic: true,
                    ..Prop::new(name, Some(text(x)), Some(text(y)))
                }),
                Same::Different => props.push(Prop::new(name, Some(text(x)), Some(text(y)))),
            }
        }
        (x, y) => props.push(Prop::new(name, x.map(text), y.map(text))),
    }
}

pub(crate) fn opt_str(props: &mut Vec<Prop>, name: &str, a: Option<&str>, b: Option<&str>) {
    let a = a.filter(|s| !s.is_empty());
    let b = b.filter(|s| !s.is_empty());
    if a != b {
        props.push(Prop::new(
            name,
            a.map(|s| json!(norm::lf(s))),
            b.map(|s| json!(norm::lf(s))),
        ));
    }
}

pub(crate) fn flag(props: &mut Vec<Prop>, name: &str, a: bool, b: bool) {
    if a != b {
        props.push(Prop::new(name, Some(json!(a)), Some(json!(b))));
    }
}

fn annotations(props: &mut Vec<Prop>, a: &[Annotation], b: &[Annotation]) {
    let map = |v: &[Annotation]| -> std::collections::BTreeMap<String, String> {
        v.iter()
            .filter(|x| !NOISY_ANNOTATIONS.contains(&x.name.as_str()))
            .map(|x| (x.name.clone(), x.value.clone()))
            .collect()
    };
    let (ma, mb) = (map(a), map(b));
    let keys: std::collections::BTreeSet<&String> = ma.keys().chain(mb.keys()).collect();
    for k in keys {
        let (x, y) = (ma.get(k), mb.get(k));
        if x.map(|s| norm::lf(s)) != y.map(|s| norm::lf(s)) {
            props.push(Prop::new(
                format!("annotation {k}"),
                x.map(|s| clip(&json!(norm::lf(s)))),
                y.map(|s| clip(&json!(norm::lf(s)))),
            ));
        }
    }
}

fn extras(
    props: &mut Vec<Prop>,
    prefix: &str,
    a: &Map<String, Value>,
    b: &Map<String, Value>,
    ignore: &[&str],
) {
    let mut d = Differ::new(props, ignore);
    d.maps(prefix, a, b);
    d.finish();
}

fn table_id(t: &str) -> String {
    format!("table {}", quote_table(t))
}
fn member_id(kind: &str, t: &str, m: &str) -> String {
    format!("{kind} {}{}", quote_table(t), bracket(m))
}

struct Ctx<'o> {
    opts: &'o Options,
    out: &'o mut Vec<Change>,
    /// Table names of `a` → their names in `b`.
    tables: HashMap<String, String>,
    /// (table, column) of `a` → the column's name in `b`.
    columns: HashMap<(String, String), String>,
}

impl Ctx<'_> {
    fn table_in_b<'s>(&'s self, a: &'s str) -> &'s str {
        self.tables.get(a).map_or(a, String::as_str)
    }
    fn column_in_b<'s>(&'s self, t: &'s str, c: &'s str) -> &'s str {
        self.columns
            .get(&(t.to_string(), c.to_string()))
            .map_or(c, String::as_str)
    }
    fn push(&mut self, c: Change) {
        self.out.push(c);
    }
}

pub(crate) fn compare(a: &SemanticModel, b: &SemanticModel, opts: &Options, out: &mut Vec<Change>) {
    let mut cx = Ctx {
        opts,
        out,
        tables: HashMap::new(),
        columns: HashMap::new(),
    };
    model_props(&mut cx, a, b);
    tables(&mut cx, a, b);
    measures(&mut cx, a, b);
    relationships(&mut cx, a, b);
    roles(&mut cx, a, b);
    named_json(
        &mut cx,
        a.extra.get("perspectives"),
        b.extra.get("perspectives"),
        "perspective",
        SECTION_PERSPECTIVES,
    );
    named_json(
        &mut cx,
        a.extra.get("cultures"),
        b.extra.get("cultures"),
        "culture",
        SECTION_CULTURES,
    );
    named_json(
        &mut cx,
        a.extra.get("dataSources"),
        b.extra.get("dataSources"),
        "data_source",
        SECTION_DATA_SOURCES,
    );
    expressions(&mut cx, &a.expressions, &b.expressions);
}

fn model_props(cx: &mut Ctx<'_>, a: &SemanticModel, b: &SemanticModel) {
    let mut c = Change::new(Area::Model, "model", "model".into(), Kind::Changed);
    let p = &mut c.properties;
    opt_str(p, "culture", a.culture.as_deref(), b.culture.as_deref());
    opt_str(
        p,
        "description",
        a.description.as_deref(),
        b.description.as_deref(),
    );
    if a.compatibility_level != b.compatibility_level {
        p.push(Prop::new(
            "compatibility_level",
            a.compatibility_level.map(|v| json!(v)),
            b.compatibility_level.map(|v| json!(v)),
        ));
    }
    annotations(p, &a.annotations, &b.annotations);
    // the model object's name (`Model`) is fixed by the format, not by the author
    let mut ignore: Vec<&str> = MODEL_COLLECTIONS.to_vec();
    ignore.push("name");
    extras(p, "", &a.extra, &b.extra, &ignore);
    // the database object: its name and id are generated per format, the level is above
    extras(
        p,
        "database",
        &a.database_extra,
        &b.database_extra,
        &["name", "id", "compatibilityLevel"],
    );
    if c.has_content() {
        cx.push(c.sorted(SECTION_MODEL, "", 0));
    }
}

fn names<'a>(it: impl Iterator<Item = &'a str>) -> Value {
    let v: Vec<&str> = it.collect();
    if v.len() > MAX_LISTED {
        json!({"count": v.len(), "first": v.iter().take(MAX_LISTED).collect::<Vec<_>>()})
    } else {
        json!(v)
    }
}

fn visible_columns(t: &Table) -> Vec<&Column> {
    t.columns.iter().filter(|c| !c.is_row_number()).collect()
}

fn table_summary(t: &Table) -> Value {
    let mut o = Map::new();
    o.insert(
        "columns".into(),
        names(visible_columns(t).iter().map(|c| c.name.as_str())),
    );
    o.insert(
        "measures".into(),
        names(t.measures.iter().map(|m| m.name.as_str())),
    );
    o.insert(
        "partitions".into(),
        names(t.partitions.iter().map(|p| p.name.as_str())),
    );
    if t.is_hidden {
        o.insert("hidden".into(), json!(true));
    }
    if t.extra.contains_key("calculationGroup") {
        o.insert("calculation_group".into(), json!(true));
    }
    Value::Object(o)
}

fn table_sig(t: &Table) -> Option<String> {
    let mut cols: Vec<&str> = visible_columns(t).iter().map(|c| c.name.as_str()).collect();
    if cols.is_empty() {
        return None;
    }
    cols.sort_unstable();
    let src: Vec<String> = t
        .partitions
        .iter()
        .filter_map(|p| p.expression.as_deref().map(norm::m_key))
        .collect();
    Some(format!("{}\u{1}{}", cols.join("\u{2}"), src.join("\u{2}")))
}

fn tables(cx: &mut Ctx<'_>, a: &SemanticModel, b: &SemanticModel) {
    let items = crate::pair::items_fn(|m: &SemanticModel| {
        m.tables
            .iter()
            .map(|t| Item {
                name: &t.name,
                tag: t.lineage_tag.as_deref(),
                sig: table_sig(t),
            })
            .collect()
    });
    let (ia, ib) = (items(a), items(b));
    let p = pair(&ia, &ib);
    for &(x, y, _) in &p.pairs {
        if let (Some(ta), Some(tb)) = (a.tables.get(x), b.tables.get(y)) {
            cx.tables.insert(ta.name.clone(), tb.name.clone());
        }
    }
    for &(x, y, how) in &p.pairs {
        let (Some(ta), Some(tb)) = (a.tables.get(x), b.tables.get(y)) else {
            continue;
        };
        table_pair(cx, ta, tb, how);
    }
    for &x in &p.removed {
        if let Some(t) = a.tables.get(x) {
            let mut c = Change::new(Area::Model, "table", table_id(&t.name), Kind::Removed);
            c.before = Some(table_summary(t));
            cx.push(c.sorted(SECTION_TABLES, &t.name, RANK_TABLE));
        }
    }
    for &y in &p.added {
        if let Some(t) = b.tables.get(y) {
            let mut c = Change::new(Area::Model, "table", table_id(&t.name), Kind::Added);
            c.after = Some(table_summary(t));
            cx.push(c.sorted(SECTION_TABLES, &t.name, RANK_TABLE));
        }
    }
}

fn table_pair(cx: &mut Ctx<'_>, ta: &Table, tb: &Table, how: How) {
    let renamed = ta.name != tb.name;
    let mut c = Change::new(
        Area::Model,
        "table",
        table_id(&tb.name),
        if renamed { Kind::Renamed } else { Kind::Changed },
    );
    if renamed {
        c.from = Some(table_id(&ta.name));
        c.how = Some(how);
    }
    let p = &mut c.properties;
    opt_str(
        p,
        "description",
        ta.description.as_deref(),
        tb.description.as_deref(),
    );
    flag(p, "hidden", ta.is_hidden, tb.is_hidden);
    annotations(p, &ta.annotations, &tb.annotations);
    let mut ignore: Vec<&str> = TABLE_COLLECTIONS.to_vec();
    ignore.push("lineageTag");
    extras(p, "", &ta.extra, &tb.extra, &ignore);
    // the calculation group's own properties (precedence, …); its items are objects
    let cg = |t: &Table| -> Map<String, Value> {
        match t.extra.get("calculationGroup") {
            Some(Value::Object(m)) => m.clone(),
            _ => Map::new(),
        }
    };
    extras(p, "calculationGroup", &cg(ta), &cg(tb), &["calculationItems"]);
    if renamed || c.has_content() {
        cx.push(c.sorted(SECTION_TABLES, &tb.name, RANK_TABLE));
    }
    columns(cx, ta, tb);
    let items = |t: &Table, key: &str| -> Option<Value> {
        match key {
            "hierarchies" => t.extra.get("hierarchies").cloned(),
            _ => t
                .extra
                .get("calculationGroup")
                .and_then(|g| g.get("calculationItems"))
                .cloned(),
        }
    };
    child_json(
        cx,
        ta,
        tb,
        items(ta, "hierarchies"),
        items(tb, "hierarchies"),
        "hierarchy",
        RANK_HIERARCHY,
    );
    child_json(
        cx,
        ta,
        tb,
        items(ta, "items"),
        items(tb, "items"),
        "calculation_item",
        RANK_CALC_ITEM,
    );
    partitions(cx, ta, tb);
}

fn column_summary(c: &Column) -> Value {
    let mut o = Map::new();
    o.insert(
        "data_type".into(),
        serde_json::to_value(c.data_type).unwrap_or(Value::Null),
    );
    if let Some(e) = &c.expression {
        o.insert("expression".into(), json!(norm::lf(e)));
    }
    if let Some(s) = &c.source_column {
        o.insert("source_column".into(), json!(s));
    }
    if let Some(f) = &c.format_string {
        o.insert("format_string".into(), json!(f));
    }
    if c.is_hidden {
        o.insert("hidden".into(), json!(true));
    }
    Value::Object(o)
}

fn columns(cx: &mut Ctx<'_>, ta: &Table, tb: &Table) {
    let (ca, cb) = (visible_columns(ta), visible_columns(tb));
    let items = crate::pair::items_fn(|cols: &[&Column]| {
        cols.iter()
            .map(|c| Item {
                name: c.name.as_str(),
                tag: c.lineage_tag.as_deref(),
                sig: match (&c.source_column, &c.expression) {
                    (Some(s), _) => Some(format!("s\u{1}{s}\u{1}{:?}", c.data_type)),
                    (None, Some(e)) => Some(format!("e\u{1}{}\u{1}{:?}", norm::dax_key(e), c.data_type)),
                    _ => None,
                },
            })
            .collect()
    });
    // `items` borrows from the column vectors; collect them first
    let (ia, ib) = (items(&ca), items(&cb));
    let p = pair(&ia, &ib);
    for &(x, y, _) in &p.pairs {
        if let (Some(a), Some(b)) = (ca.get(x), cb.get(y)) {
            cx.columns
                .insert((ta.name.clone(), a.name.clone()), b.name.clone());
        }
    }
    for &(x, y, how) in &p.pairs {
        let (Some(a), Some(b)) = (ca.get(x), cb.get(y)) else {
            continue;
        };
        let renamed = a.name != b.name;
        let mut c = Change::new(
            Area::Model,
            "column",
            member_id("column", &tb.name, &b.name),
            if renamed { Kind::Renamed } else { Kind::Changed },
        );
        if renamed {
            c.from = Some(member_id("column", &ta.name, &a.name));
            c.how = Some(how);
        }
        column_props(cx, &mut c.properties, a, b);
        if renamed || c.has_content() {
            cx.push(c.sorted(SECTION_TABLES, &tb.name, RANK_COLUMN));
        }
    }
    for &x in &p.removed {
        if let Some(col) = ca.get(x) {
            let mut c = Change::new(
                Area::Model,
                "column",
                member_id("column", &ta.name, &col.name),
                Kind::Removed,
            );
            c.before = Some(column_summary(col));
            cx.push(c.sorted(SECTION_TABLES, &tb.name, RANK_COLUMN));
        }
    }
    for &y in &p.added {
        if let Some(col) = cb.get(y) {
            let mut c = Change::new(
                Area::Model,
                "column",
                member_id("column", &tb.name, &col.name),
                Kind::Added,
            );
            c.after = Some(column_summary(col));
            cx.push(c.sorted(SECTION_TABLES, &tb.name, RANK_COLUMN));
        }
    }
}

fn column_props(cx: &Ctx<'_>, p: &mut Vec<Prop>, a: &Column, b: &Column) {
    if a.data_type != b.data_type {
        p.push(Prop::new(
            "data_type",
            serde_json::to_value(a.data_type).ok(),
            serde_json::to_value(b.data_type).ok(),
        ));
    }
    expr(
        p,
        "expression",
        a.expression.as_deref(),
        b.expression.as_deref(),
        Lang::Dax,
        cx.opts,
    );
    opt_str(
        p,
        "source_column",
        a.source_column.as_deref(),
        b.source_column.as_deref(),
    );
    opt_str(
        p,
        "format_string",
        a.format_string.as_deref(),
        b.format_string.as_deref(),
    );
    opt_str(
        p,
        "display_folder",
        a.display_folder.as_deref(),
        b.display_folder.as_deref(),
    );
    opt_str(
        p,
        "summarize_by",
        a.summarize_by
            .as_deref()
            .filter(|s| !s.eq_ignore_ascii_case("default")),
        b.summarize_by
            .as_deref()
            .filter(|s| !s.eq_ignore_ascii_case("default")),
    );
    opt_str(
        p,
        "sort_by_column",
        a.sort_by_column.as_deref(),
        b.sort_by_column.as_deref(),
    );
    opt_str(
        p,
        "data_category",
        a.data_category.as_deref(),
        b.data_category.as_deref(),
    );
    opt_str(
        p,
        "description",
        a.description.as_deref(),
        b.description.as_deref(),
    );
    flag(p, "hidden", a.is_hidden, b.is_hidden);
    annotations(p, &a.annotations, &b.annotations);
    extras(p, "", &a.extra, &b.extra, &["lineageTag"]);
}

fn measure_summary(m: &Measure) -> Value {
    let mut o = Map::new();
    o.insert("expression".into(), json!(norm::lf(&m.expression)));
    if let Some(f) = &m.format_string {
        o.insert("format_string".into(), json!(f));
    }
    if let Some(f) = &m.display_folder {
        o.insert("display_folder".into(), json!(f));
    }
    if m.is_hidden {
        o.insert("hidden".into(), json!(true));
    }
    Value::Object(o)
}

/// Measures are unique in the whole model: matched across tables, so a measure in another
/// table is a move.
fn measures(cx: &mut Ctx<'_>, a: &SemanticModel, b: &SemanticModel) {
    let all = |m: &SemanticModel| -> Vec<(String, Measure)> {
        m.tables
            .iter()
            .flat_map(|t| t.measures.iter().map(move |x| (t.name.clone(), x.clone())))
            .collect()
    };
    let (ma, mb) = (all(a), all(b));
    let items = crate::pair::items_fn(|v: &[(String, Measure)]| {
        v.iter()
            .map(|(_, m)| Item {
                name: &m.name,
                tag: m.lineage_tag.as_deref(),
                sig: Some(norm::dax_key(&m.expression)),
            })
            .collect()
    });
    let (ia, ib) = (items(&ma), items(&mb));
    let p = pair(&ia, &ib);
    for &(x, y, how) in &p.pairs {
        let (Some((ta, a)), Some((tb, b))) = (ma.get(x), mb.get(y)) else {
            continue;
        };
        let renamed = a.name != b.name;
        let moved = cx.table_in_b(ta) != tb.as_str();
        let kind = if renamed {
            Kind::Renamed
        } else if moved {
            Kind::Moved
        } else {
            Kind::Changed
        };
        let mut c = Change::new(Area::Model, "measure", member_id("measure", tb, &b.name), kind);
        if renamed || moved {
            c.from = Some(member_id("measure", ta, &a.name));
            c.how = Some(how);
        }
        let props = &mut c.properties;
        expr(
            props,
            "expression",
            Some(&a.expression),
            Some(&b.expression),
            Lang::Dax,
            cx.opts,
        );
        opt_str(
            props,
            "format_string",
            a.format_string.as_deref(),
            b.format_string.as_deref(),
        );
        opt_str(
            props,
            "display_folder",
            a.display_folder.as_deref(),
            b.display_folder.as_deref(),
        );
        opt_str(
            props,
            "description",
            a.description.as_deref(),
            b.description.as_deref(),
        );
        flag(props, "hidden", a.is_hidden, b.is_hidden);
        annotations(props, &a.annotations, &b.annotations);
        extras(props, "", &a.extra, &b.extra, &["lineageTag"]);
        if renamed || moved || c.has_content() {
            cx.push(c.sorted(SECTION_TABLES, tb, RANK_MEASURE));
        }
    }
    for &x in &p.removed {
        if let Some((t, m)) = ma.get(x) {
            let mut c = Change::new(
                Area::Model,
                "measure",
                member_id("measure", t, &m.name),
                Kind::Removed,
            );
            c.before = Some(measure_summary(m));
            let group = cx.table_in_b(t).to_string();
            cx.push(c.sorted(SECTION_TABLES, &group, RANK_MEASURE));
        }
    }
    for &y in &p.added {
        if let Some((t, m)) = mb.get(y) {
            let mut c = Change::new(
                Area::Model,
                "measure",
                member_id("measure", t, &m.name),
                Kind::Added,
            );
            c.after = Some(measure_summary(m));
            cx.push(c.sorted(SECTION_TABLES, t, RANK_MEASURE));
        }
    }
}

/// A table's named JSON children (hierarchies, calculation items).
fn child_json(
    cx: &mut Ctx<'_>,
    ta: &Table,
    tb: &Table,
    a: Option<Value>,
    b: Option<Value>,
    object: &'static str,
    rank: u8,
) {
    let list = |v: Option<Value>| -> Vec<Value> {
        match v {
            Some(Value::Array(v)) => v,
            _ => Vec::new(),
        }
    };
    let (la, lb) = (list(a), list(b));
    let items = crate::pair::items_fn(|v: &[Value]| {
        v.iter()
            .map(|x| Item {
                name: element_name(x).unwrap_or(""),
                tag: x.get("lineageTag").and_then(Value::as_str),
                sig: x.get("expression").and_then(Value::as_str).map(norm::dax_key),
            })
            .collect()
    });
    let (ia, ib) = (items(&la), items(&lb));
    let p = pair(&ia, &ib);
    for &(x, y, how) in &p.pairs {
        let (Some(va), Some(vb)) = (la.get(x), lb.get(y)) else {
            continue;
        };
        let (na, nb) = (element_name(va).unwrap_or(""), element_name(vb).unwrap_or(""));
        let renamed = na != nb;
        let mut c = Change::new(
            Area::Model,
            object,
            member_id(object, &tb.name, nb),
            if renamed { Kind::Renamed } else { Kind::Changed },
        );
        if renamed {
            c.from = Some(member_id(object, &ta.name, na));
            c.how = Some(how);
        }
        json_object_props(cx.opts, &mut c.properties, va, vb);
        if renamed || c.has_content() {
            cx.push(c.sorted(SECTION_TABLES, &tb.name, rank));
        }
    }
    for &x in &p.removed {
        if let Some(v) = la.get(x) {
            let mut c = Change::new(
                Area::Model,
                object,
                member_id(object, &ta.name, element_name(v).unwrap_or("")),
                Kind::Removed,
            );
            c.before = Some(json_summary(v));
            cx.push(c.sorted(SECTION_TABLES, &tb.name, rank));
        }
    }
    for &y in &p.added {
        if let Some(v) = lb.get(y) {
            let mut c = Change::new(
                Area::Model,
                object,
                member_id(object, &tb.name, element_name(v).unwrap_or("")),
                Kind::Added,
            );
            c.after = Some(json_summary(v));
            cx.push(c.sorted(SECTION_TABLES, &tb.name, rank));
        }
    }
}

/// A JSON object's properties for a summary, with `expression` text normalised.
fn json_summary(v: &Value) -> Value {
    let mut v = clip(v);
    if let Some(Value::String(e)) = v.get_mut("expression") {
        *e = norm::lf(e).into_owned();
    }
    v
}

/// Compares two JSON objects: `expression` (and a format string definition's) as DAX, the
/// rest generically.
fn json_object_props(opts: &Options, p: &mut Vec<Prop>, a: &Value, b: &Value) {
    let s = |v: &Value, ptr: &str| v.pointer(ptr).and_then(Value::as_str).map(str::to_string);
    expr(
        p,
        "expression",
        s(a, "/expression").as_deref(),
        s(b, "/expression").as_deref(),
        Lang::Dax,
        opts,
    );
    expr(
        p,
        "formatStringDefinition.expression",
        s(a, "/formatStringDefinition/expression").as_deref(),
        s(b, "/formatStringDefinition/expression").as_deref(),
        Lang::Dax,
        opts,
    );
    let strip = |v: &Value| -> Map<String, Value> {
        let mut m = v.as_object().cloned().unwrap_or_default();
        m.remove("expression");
        m.remove("name");
        m.remove("lineageTag");
        if let Some(Value::Object(f)) = m.get_mut("formatStringDefinition") {
            f.remove("expression");
            if f.is_empty() {
                m.remove("formatStringDefinition");
            }
        }
        m
    };
    extras(p, "", &strip(a), &strip(b), &[]);
}

fn partition_summary(p: &Partition) -> Value {
    let mut o = Map::new();
    o.insert("source_type".into(), json!(p.source_type));
    if let Some(m) = &p.mode {
        o.insert("mode".into(), json!(m));
    }
    if let Some(e) = &p.expression {
        o.insert("expression".into(), json!(norm::lf(e)));
    }
    Value::Object(o)
}

fn partitions(cx: &mut Ctx<'_>, ta: &Table, tb: &Table) {
    let items = crate::pair::items_fn(|t: &Table| {
        t.partitions
            .iter()
            .map(|p| Item {
                name: &p.name,
                tag: None,
                sig: None,
            })
            .collect()
    });
    let (ia, ib) = (items(ta), items(tb));
    let mut p = pair(&ia, &ib);
    // partition names are generated (`Sales-<guid>`): one left on each side is the same one
    if let ([x], [y]) = (p.removed.as_slice(), p.added.as_slice()) {
        p.pairs.push((*x, *y, How::Content));
        p.removed.clear();
        p.added.clear();
    }
    let id = |t: &str, n: &str| format!("partition {}/{}", quote_table(t), quote_table(n));
    for &(x, y, _) in &p.pairs {
        let (Some(a), Some(b)) = (ta.partitions.get(x), tb.partitions.get(y)) else {
            continue;
        };
        let mut c = Change::new(Area::Model, "partition", id(&tb.name, &b.name), Kind::Changed);
        let props = &mut c.properties;
        opt_str(props, "mode", a.mode.as_deref(), b.mode.as_deref());
        opt_str(props, "source_type", Some(&a.source_type), Some(&b.source_type));
        let lang = if b.source_type.eq_ignore_ascii_case("calculated") {
            Lang::Dax
        } else {
            Lang::M
        };
        expr(
            props,
            "expression",
            a.expression.as_deref(),
            b.expression.as_deref(),
            lang,
            cx.opts,
        );
        extras(props, "", &a.extra, &b.extra, &[]);
        extras(props, "source", &a.source_extra, &b.source_extra, &[]);
        if c.has_content() {
            cx.push(c.sorted(SECTION_TABLES, &tb.name, RANK_PARTITION));
        }
    }
    for &x in &p.removed {
        if let Some(part) = ta.partitions.get(x) {
            let mut c = Change::new(Area::Model, "partition", id(&ta.name, &part.name), Kind::Removed);
            c.before = Some(partition_summary(part));
            cx.push(c.sorted(SECTION_TABLES, &tb.name, RANK_PARTITION));
        }
    }
    for &y in &p.added {
        if let Some(part) = tb.partitions.get(y) {
            let mut c = Change::new(Area::Model, "partition", id(&tb.name, &part.name), Kind::Added);
            c.after = Some(partition_summary(part));
            cx.push(c.sorted(SECTION_TABLES, &tb.name, RANK_PARTITION));
        }
    }
}

fn endpoint(t: &str, c: &str) -> String {
    format!("{}{}", quote_table(t), bracket(c))
}

fn rel_id(from: &str, to: &str) -> String {
    format!("relationship {from} -> {to}")
}

fn rel_summary(r: &Relationship) -> Value {
    let mut o = Map::new();
    o.insert("from".into(), json!(endpoint(&r.from_table, &r.from_column)));
    o.insert("to".into(), json!(endpoint(&r.to_table, &r.to_column)));
    o.insert("active".into(), json!(r.is_active));
    if let Some(v) = &r.cross_filtering_behavior {
        o.insert("cross_filtering_behavior".into(), json!(v));
    }
    if let Some(v) = &r.from_cardinality {
        o.insert("from_cardinality".into(), json!(v));
    }
    if let Some(v) = &r.to_cardinality {
        o.insert("to_cardinality".into(), json!(v));
    }
    Value::Object(o)
}

fn relationships(cx: &mut Ctx<'_>, a: &SemanticModel, b: &SemanticModel) {
    // `a`'s endpoints in `b`'s names, so a renamed table does not change its relationships
    let ends_a: Vec<(String, String)> = a
        .relationships
        .iter()
        .map(|r| {
            (
                endpoint(
                    cx.table_in_b(&r.from_table),
                    cx.column_in_b(&r.from_table, &r.from_column),
                ),
                endpoint(
                    cx.table_in_b(&r.to_table),
                    cx.column_in_b(&r.to_table, &r.to_column),
                ),
            )
        })
        .collect();
    let ends_b: Vec<(String, String)> = b
        .relationships
        .iter()
        .map(|r| {
            (
                endpoint(&r.from_table, &r.from_column),
                endpoint(&r.to_table, &r.to_column),
            )
        })
        .collect();
    let items = crate::pair::items_fn2(|rs: &[Relationship], ends: &[(String, String)]| {
        rs.iter()
            .zip(ends)
            .map(|(r, (f, t))| Item {
                name: &r.name,
                tag: None,
                sig: Some(format!("{f}\u{1}{t}")),
            })
            .collect()
    });
    let (ia, ib) = (items(&a.relationships, &ends_a), items(&b.relationships, &ends_b));
    let p = pair(&ia, &ib);
    for &(x, y, _) in &p.pairs {
        let (Some(ra), Some(rb), Some(ea), Some(eb)) = (
            a.relationships.get(x),
            b.relationships.get(y),
            ends_a.get(x),
            ends_b.get(y),
        ) else {
            continue;
        };
        // relationship names are generated ids, never shown: a new name is not a rename
        let mut c = Change::new(Area::Model, "relationship", rel_id(&eb.0, &eb.1), Kind::Changed);
        let props = &mut c.properties;
        opt_str(props, "from", Some(&ea.0), Some(&eb.0));
        opt_str(props, "to", Some(&ea.1), Some(&eb.1));
        flag(props, "active", ra.is_active, rb.is_active);
        opt_str(
            props,
            "cross_filtering_behavior",
            ra.cross_filtering_behavior.as_deref(),
            rb.cross_filtering_behavior.as_deref(),
        );
        opt_str(
            props,
            "from_cardinality",
            ra.from_cardinality.as_deref(),
            rb.from_cardinality.as_deref(),
        );
        opt_str(
            props,
            "to_cardinality",
            ra.to_cardinality.as_deref(),
            rb.to_cardinality.as_deref(),
        );
        annotations(props, &ra.annotations, &rb.annotations);
        extras(props, "", &ra.extra, &rb.extra, &[]);
        if c.has_content() {
            cx.push(c.sorted(SECTION_RELATIONSHIPS, "", 0));
        }
    }
    for &x in &p.removed {
        if let (Some(r), Some(e)) = (a.relationships.get(x), ends_a.get(x)) {
            let mut c = Change::new(Area::Model, "relationship", rel_id(&e.0, &e.1), Kind::Removed);
            c.before = Some(rel_summary(r));
            cx.push(c.sorted(SECTION_RELATIONSHIPS, "", 0));
        }
    }
    for &y in &p.added {
        if let (Some(r), Some(e)) = (b.relationships.get(y), ends_b.get(y)) {
            let mut c = Change::new(Area::Model, "relationship", rel_id(&e.0, &e.1), Kind::Added);
            c.after = Some(rel_summary(r));
            cx.push(c.sorted(SECTION_RELATIONSHIPS, "", 0));
        }
    }
}

fn role_summary(r: &Role) -> Value {
    let mut o = Map::new();
    if let Some(m) = &r.model_permission {
        o.insert("model_permission".into(), json!(m));
    }
    let filters: Map<String, Value> = r
        .table_permissions
        .iter()
        .map(|t| {
            (
                t.table.clone(),
                json!(t.filter_expression.as_deref().map(norm::lf)),
            )
        })
        .collect();
    o.insert("table_permissions".into(), Value::Object(filters));
    Value::Object(o)
}

fn roles(cx: &mut Ctx<'_>, a: &SemanticModel, b: &SemanticModel) {
    let items = crate::pair::items_fn(|m: &SemanticModel| {
        m.roles
            .iter()
            .map(|r| Item {
                name: &r.name,
                tag: None,
                sig: None,
            })
            .collect()
    });
    let (ia, ib) = (items(a), items(b));
    let p = pair(&ia, &ib);
    let id = |n: &str| format!("role {}", quote_table(n));
    for &(x, y, how) in &p.pairs {
        let (Some(ra), Some(rb)) = (a.roles.get(x), b.roles.get(y)) else {
            continue;
        };
        let renamed = ra.name != rb.name;
        let mut c = Change::new(
            Area::Model,
            "role",
            id(&rb.name),
            if renamed { Kind::Renamed } else { Kind::Changed },
        );
        if renamed {
            c.from = Some(id(&ra.name));
            c.how = Some(how);
        }
        let props = &mut c.properties;
        opt_str(
            props,
            "model_permission",
            ra.model_permission.as_deref(),
            rb.model_permission.as_deref(),
        );
        let (pa, pb) = (perms(ra), perms(rb));
        let mut tables: Vec<String> = pa.keys().map(|t| cx.table_in_b(t).to_string()).collect();
        tables.extend(pb.keys().cloned());
        tables.sort();
        tables.dedup();
        for t in tables {
            let in_a = pa
                .iter()
                .find(|(k, _)| cx.table_in_b(k) == t.as_str())
                .map(|(_, v)| *v);
            let in_b = pb.get(&t).copied();
            let name = format!("filter {}", quote_table(&t));
            match (in_a, in_b) {
                (Some(x), Some(y)) => {
                    expr(
                        props,
                        &name,
                        x.filter_expression.as_deref(),
                        y.filter_expression.as_deref(),
                        Lang::Dax,
                        cx.opts,
                    );
                    extras(props, &name, &x.extra, &y.extra, &[]);
                }
                (x, y) => props.push(Prop::new(
                    name,
                    x.map(|v| json!(v.filter_expression.as_deref().map(norm::lf))),
                    y.map(|v| json!(v.filter_expression.as_deref().map(norm::lf))),
                )),
            }
        }
        annotations(props, &ra.annotations, &rb.annotations);
        extras(props, "", &role_extra(ra), &role_extra(rb), &[]);
        if renamed || c.has_content() {
            cx.push(c.sorted(SECTION_ROLES, "", 0));
        }
    }
    for &x in &p.removed {
        if let Some(r) = a.roles.get(x) {
            let mut c = Change::new(Area::Model, "role", id(&r.name), Kind::Removed);
            c.before = Some(role_summary(r));
            cx.push(c.sorted(SECTION_ROLES, "", 0));
        }
    }
    for &y in &p.added {
        if let Some(r) = b.roles.get(y) {
            let mut c = Change::new(Area::Model, "role", id(&r.name), Kind::Added);
            c.after = Some(role_summary(r));
            cx.push(c.sorted(SECTION_ROLES, "", 0));
        }
    }
}

/// Model-level named JSON collections: perspectives, cultures, data sources.
fn named_json(cx: &mut Ctx<'_>, a: Option<&Value>, b: Option<&Value>, object: &'static str, section: u8) {
    let list = |v: Option<&Value>| -> Vec<Value> {
        match v {
            Some(Value::Array(v)) => v.clone(),
            _ => Vec::new(),
        }
    };
    let (la, lb) = (list(a), list(b));
    let id = |n: &str| format!("{object} {}", quote_table(n));
    let items = crate::pair::items_fn(|v: &[Value]| {
        v.iter()
            .map(|x| Item {
                name: element_name(x).unwrap_or(""),
                tag: None,
                sig: None,
            })
            .collect()
    });
    let (ia, ib) = (items(&la), items(&lb));
    let p = pair(&ia, &ib);
    for &(x, y, how) in &p.pairs {
        let (Some(va), Some(vb)) = (la.get(x), lb.get(y)) else {
            continue;
        };
        let (na, nb) = (element_name(va).unwrap_or(""), element_name(vb).unwrap_or(""));
        let renamed = na != nb;
        let mut c = Change::new(
            Area::Model,
            object,
            id(nb),
            if renamed { Kind::Renamed } else { Kind::Changed },
        );
        if renamed {
            c.from = Some(id(na));
            c.how = Some(how);
        }
        let strip = |v: &Value| -> Map<String, Value> {
            let mut m = v.as_object().cloned().unwrap_or_default();
            m.remove("name");
            m
        };
        extras(&mut c.properties, "", &strip(va), &strip(vb), &[]);
        if renamed || c.has_content() {
            cx.push(c.sorted(section, "", 0));
        }
    }
    for &x in &p.removed {
        if let Some(v) = la.get(x) {
            let mut c = Change::new(
                Area::Model,
                object,
                id(element_name(v).unwrap_or("")),
                Kind::Removed,
            );
            c.before = Some(clip(v));
            cx.push(c.sorted(section, "", 0));
        }
    }
    for &y in &p.added {
        if let Some(v) = lb.get(y) {
            let mut c = Change::new(
                Area::Model,
                object,
                id(element_name(v).unwrap_or("")),
                Kind::Added,
            );
            c.after = Some(clip(v));
            cx.push(c.sorted(section, "", 0));
        }
    }
}

/// A Power Query parameter: its M ends with `meta [IsParameterQuery = true, …]`.
pub(crate) fn is_parameter(e: &NamedExpression) -> bool {
    let key: String = e
        .expression
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    key.contains("isparameterquery=true")
}

fn expressions(cx: &mut Ctx<'_>, a: &[NamedExpression], b: &[NamedExpression]) {
    let items = crate::pair::items_fn(|v: &[NamedExpression]| {
        v.iter()
            .map(|e| Item {
                name: &e.name,
                tag: e.lineage_tag.as_deref(),
                sig: None,
            })
            .collect()
    });
    let (ia, ib) = (items(a), items(b));
    let p = pair(&ia, &ib);
    let object = |e: &NamedExpression| {
        if is_parameter(e) {
            "parameter"
        } else {
            "expression"
        }
    };
    let id = |e: &NamedExpression| format!("{} {}", object(e), quote_table(&e.name));
    let summary = |e: &NamedExpression| -> Value {
        let mut o = Map::new();
        o.insert("kind".into(), json!(e.kind));
        o.insert("expression".into(), json!(norm::lf(&e.expression)));
        Value::Object(o)
    };
    for &(x, y, how) in &p.pairs {
        let (Some(ea), Some(eb)) = (a.get(x), b.get(y)) else {
            continue;
        };
        let renamed = ea.name != eb.name;
        let mut c = Change::new(
            Area::Model,
            object(eb),
            id(eb),
            if renamed { Kind::Renamed } else { Kind::Changed },
        );
        if renamed {
            c.from = Some(id(ea));
            c.how = Some(how);
        }
        let props = &mut c.properties;
        opt_str(props, "kind", Some(&ea.kind), Some(&eb.kind));
        let lang = if eb.kind.eq_ignore_ascii_case("m") {
            Lang::M
        } else {
            Lang::Dax
        };
        expr(
            props,
            "expression",
            Some(&ea.expression),
            Some(&eb.expression),
            lang,
            cx.opts,
        );
        opt_str(
            props,
            "description",
            ea.description.as_deref(),
            eb.description.as_deref(),
        );
        annotations(props, &ea.annotations, &eb.annotations);
        extras(props, "", &ea.extra, &eb.extra, &["lineageTag"]);
        if renamed || c.has_content() {
            cx.push(c.sorted(SECTION_EXPRESSIONS, "", 0));
        }
    }
    for &x in &p.removed {
        if let Some(e) = a.get(x) {
            let mut c = Change::new(Area::Model, object(e), id(e), Kind::Removed);
            c.before = Some(summary(e));
            cx.push(c.sorted(SECTION_EXPRESSIONS, "", 0));
        }
    }
    for &y in &p.added {
        if let Some(e) = b.get(y) {
            let mut c = Change::new(Area::Model, object(e), id(e), Kind::Added);
            c.after = Some(summary(e));
            cx.push(c.sorted(SECTION_EXPRESSIONS, "", 0));
        }
    }
}

fn perms(r: &Role) -> std::collections::BTreeMap<String, &bicraft_model::TablePermission> {
    r.table_permissions.iter().map(|t| (t.table.clone(), t)).collect()
}

/// A role's other properties, its members as TMDL and TMSL both mean them: a member with neither
/// an identity provider nor a member type is a Windows member; a member with a type is external
/// (TMSL schema), its provider `AzureAD` when absent (TMDL writes it without an
/// `identityProvider` line and reads the default back), and `auto` is an absent type.
pub(crate) fn role_extra(r: &Role) -> Map<String, Value> {
    let mut extra = r.extra.clone();
    if let Some(Value::Array(members)) = extra.get_mut("members") {
        for m in members.iter_mut() {
            if let Some(o) = m.as_object_mut() {
                if o.contains_key("memberType") && !o.contains_key("identityProvider") {
                    o.insert("identityProvider".into(), Value::String("AzureAD".into()));
                }
                if o.get("memberType").and_then(Value::as_str) == Some("auto") {
                    o.remove("memberType");
                }
            }
        }
    }
    extra
}
