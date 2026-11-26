//! `model.set_properties`: the Properties pane's edit of one or several selected tables, columns
//! and measures (docs/gaps.md G61). The same properties are set on every object, as one command
//! and so one undo step: description, display folder, is hidden, format, data category,
//! summarize by, sort by column, synonyms and a table's storage mode. Each object's change goes
//! through its own setter (`table.set`, `column.set`, `measure.set`, `table.set_storage_mode`),
//! so every refusal of those applies; synonyms are written by [`crate::synonyms`]. The first
//! refusal puts the session back as it was: nothing is changed.

use serde_json::{Map, Value, json};

use crate::Session;
use crate::commands::model;
use crate::edit::{Edit, apply, table_index};
use crate::model_props::Saved;
use crate::params::{allow, force};

/// Most objects one call sets.
const MAX_OBJECTS: usize = 10_000;

/// The properties, with the kinds of object each applies to.
const PROPS: &[(&str, &[Kind])] = &[
    ("description", &[Kind::Table, Kind::Column, Kind::Measure]),
    ("hidden", &[Kind::Table, Kind::Column, Kind::Measure]),
    ("synonyms", &[Kind::Table, Kind::Column, Kind::Measure]),
    ("display_folder", &[Kind::Column, Kind::Measure]),
    ("format_string", &[Kind::Column, Kind::Measure]),
    ("data_category", &[Kind::Column]),
    ("summarize_by", &[Kind::Column]),
    ("sort_by_column", &[Kind::Column]),
    ("storage_mode", &[Kind::Table]),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Table,
    Column,
    Measure,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Kind::Table => "table",
            Kind::Column => "column",
            Kind::Measure => "measure",
        }
    }
}

/// One object of `objects`: its kind, its table, its column or measure (as written).
#[derive(Debug, Clone)]
struct Obj {
    kind: Kind,
    table: String,
    name: Option<String>,
}

impl Obj {
    fn text(&self) -> String {
        match &self.name {
            Some(n) => format!("{} {}[{}]", self.kind.word(), self.table, n),
            None => format!("table {}", self.table),
        }
    }

    fn same(&self, o: &Obj) -> bool {
        self.kind == o.kind
            && self.table.eq_ignore_ascii_case(&o.table)
            && match (&self.name, &o.name) {
                (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                (None, None) => true,
                _ => false,
            }
    }
}

fn parse_object(v: &Value) -> Result<Obj, String> {
    let o = v
        .as_object()
        .ok_or("each of `objects` must be {table}, {table, column} or {table, measure}")?;
    allow(v, &["table", "column", "measure"]).map_err(|e| format!("objects: {e}"))?;
    let text = |k: &str| -> Result<Option<String>, String> {
        match o.get(k) {
            None => Ok(None),
            Some(Value::String(s)) if !s.is_empty() => Ok(Some(s.clone())),
            Some(_) => Err(format!("objects: `{k}` must be a non-empty string")),
        }
    };
    let table = text("table")?.ok_or("objects: each object needs `table`")?;
    match (text("column")?, text("measure")?) {
        (Some(_), Some(_)) => Err("objects: an object has `column` or `measure`, not both".into()),
        (Some(c), None) => Ok(Obj {
            kind: Kind::Column,
            table,
            name: Some(c),
        }),
        (None, Some(m)) => Ok(Obj {
            kind: Kind::Measure,
            table,
            name: Some(m),
        }),
        (None, None) => Ok(Obj {
            kind: Kind::Table,
            table,
            name: None,
        }),
    }
}

/// The objects, checked against the model (resolved to the model's spelling).
fn objects(s: &Session, p: &Value) -> Result<Vec<Obj>, String> {
    let list = p
        .get("objects")
        .and_then(Value::as_array)
        .ok_or("missing list parameter `objects`")?;
    if list.is_empty() {
        return Err("`objects` is empty".into());
    }
    if list.len() > MAX_OBJECTS {
        return Err(format!("at most {MAX_OBJECTS} objects per call"));
    }
    let m = model(s)?;
    let mut out: Vec<Obj> = Vec::with_capacity(list.len());
    for v in list {
        let o = parse_object(v)?;
        let t = m
            .tables
            .get(table_index(m, &o.table)?)
            .ok_or("internal: table index out of range")?;
        let name = match (o.kind, &o.name) {
            (Kind::Column, Some(n)) => Some(
                t.columns
                    .iter()
                    .filter(|c| !c.is_row_number())
                    .find(|c| c.name.eq_ignore_ascii_case(n))
                    .map(|c| c.name.clone())
                    .ok_or_else(|| format!("no column `{n}` in table `{}`", t.name))?,
            ),
            (Kind::Measure, Some(n)) => Some(
                t.measures
                    .iter()
                    .find(|x| x.name.eq_ignore_ascii_case(n))
                    .map(|x| x.name.clone())
                    .ok_or_else(|| format!("no measure `{n}` in table `{}`", t.name))?,
            ),
            _ => None,
        };
        let o = Obj {
            kind: o.kind,
            table: t.name.clone(),
            name,
        };
        if out.iter().any(|x| x.same(&o)) {
            return Err(format!("{} is listed twice in `objects`", o.text()));
        }
        out.push(o);
    }
    Ok(out)
}

/// The properties, each checked to apply to every object.
fn props(p: &Value, objs: &[Obj]) -> Result<Map<String, Value>, String> {
    let props = p
        .get("props")
        .and_then(Value::as_object)
        .ok_or("missing object parameter `props`")?;
    if props.is_empty() {
        return Err(format!(
            "`props` needs at least one of {}",
            PROPS
                .iter()
                .map(|(k, _)| format!("`{k}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for (k, _) in props {
        let Some((_, kinds)) = PROPS.iter().find(|(n, _)| n == k) else {
            return Err(format!(
                "unknown property `{k}` (expected: {})",
                PROPS.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
            ));
        };
        if let Some(o) = objs.iter().find(|o| !kinds.contains(&o.kind)) {
            return Err(format!("`{k}` does not apply to {}", o.text()));
        }
    }
    Ok(props.clone())
}

/// `model.set_properties {objects, props, force?}`.
pub(crate) fn set_properties(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["objects", "props", "force"])?;
    let force = force(p)?;
    let objs = objects(s, p)?;
    let props = props(p, &objs)?;
    let synonyms = match props.get("synonyms") {
        Some(v) => Some(crate::synonyms::parse(v)?),
        None => None,
    };
    let saved = Saved::take(s);
    match set_all(s, &objs, &props, synonyms.as_deref(), force) {
        Ok(v) => Ok(v),
        Err(e) => {
            saved.restore(s);
            Err(e)
        }
    }
}

fn set_all(
    s: &mut Session,
    objs: &[Obj],
    props: &Map<String, Value>,
    synonyms: Option<&[String]>,
    force: bool,
) -> Result<Value, String> {
    let mut warnings: Vec<Value> = Vec::new();
    let mut keep = |v: &Value, o: &Obj| {
        for w in v.get("warnings").and_then(Value::as_array).into_iter().flatten() {
            if let Some(w) = w.as_str() {
                warnings.push(json!(format!("{}: {w}", o.text())));
            }
        }
    };
    for o in objs {
        // the setter's own properties
        let mut own = Map::new();
        for (k, v) in props {
            if k == "synonyms" || k == "storage_mode" {
                continue;
            }
            own.insert(k.clone(), v.clone());
        }
        let r = match (o.kind, &o.name) {
            (Kind::Table, _) => {
                if own.is_empty() {
                    None
                } else {
                    own.insert("name".into(), json!(o.table));
                    Some(crate::edit::table_set(s, &Value::Object(own)))
                }
            }
            (Kind::Column, Some(n)) => {
                if own.is_empty() {
                    None
                } else {
                    own.insert("table".into(), json!(o.table));
                    own.insert("name".into(), json!(n));
                    if force {
                        own.insert("force".into(), json!(true));
                    }
                    Some(crate::model_props::column_set(s, &Value::Object(own)))
                }
            }
            (Kind::Measure, Some(n)) => {
                if own.is_empty() {
                    None
                } else {
                    own.insert("table".into(), json!(o.table));
                    own.insert("name".into(), json!(n));
                    if force {
                        own.insert("force".into(), json!(true));
                    }
                    Some(crate::model_props::measure_set(s, &Value::Object(own)))
                }
            }
            _ => None,
        };
        if let Some(r) = r {
            let v = r.map_err(|e| format!("{}: {e}", o.text()))?;
            keep(&v, o);
        }
        if let Some(mode) = props.get("storage_mode") {
            let v = crate::storage_mode::table_set_storage_mode(s, &json!({"table": o.table, "mode": mode}))
                .map_err(|e| format!("{}: {e}", o.text()))?;
            keep(&v, o);
        }
    }
    if let Some(list) = synonyms {
        let p = Value::Object(Map::new());
        apply(s, &p, |m| {
            for o in objs {
                crate::synonyms::set(m, &o.table, o.name.as_deref(), list)
                    .map_err(|e| format!("{}: {e}", o.text()))?;
            }
            Ok(Edit::new(json!({})))
        })?;
    }
    let names: Vec<Value> = objs
        .iter()
        .map(|o| match (o.kind, &o.name) {
            (Kind::Column, Some(n)) => json!({"table": o.table, "column": n}),
            (Kind::Measure, Some(n)) => json!({"table": o.table, "measure": n}),
            _ => json!({"table": o.table}),
        })
        .collect();
    Ok(json!({
        "objects": names,
        "props": props.keys().collect::<Vec<_>>(),
        "warnings": warnings,
    }))
}
