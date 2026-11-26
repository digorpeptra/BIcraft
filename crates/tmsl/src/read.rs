//! TMSL → model. See the crate docs for the lossless rules.

use bicraft_model::{
    Annotation, Column, DataType, Measure, NamedExpression, Partition, Properties, Relationship, Role,
    SemanticModel, Table, TablePermission,
};
use serde_json::{Map, Value};

use crate::Error;

type Obj = Map<String, Value>;

pub(crate) fn database(doc: &Value) -> Result<SemanticModel, Error> {
    let db = doc
        .as_object()
        .ok_or(Error::Shape("the document is not a JSON object"))?;
    let model = db
        .get("model")
        .and_then(Value::as_object)
        .ok_or(Error::Shape("missing `model` object"))?;
    let mut d = Props::new(db);
    d.used.push("model");
    let name = d.name("name");
    let compatibility_level = d.take("compatibilityLevel", |v| {
        v.as_u64().and_then(|n| u32::try_from(n).ok())
    });
    let mut m = Props::new(model);
    Ok(SemanticModel {
        name,
        compatibility_level,
        culture: m.string("culture"),
        description: m.string("description"),
        tables: m.collection("tables", |o| Some(table(o))),
        relationships: m.collection("relationships", |o| Some(relationship(o))),
        expressions: m.collection("expressions", |o| Some(named_expression(o))),
        roles: m.collection("roles", |o| Some(role(o))),
        annotations: m.annotations(),
        database_extra: d.extra(),
        extra: m.extra(),
    })
}

fn table(o: &Obj) -> Table {
    let mut p = Props::new(o);
    Table {
        name: p.name("name"),
        description: p.string("description"),
        lineage_tag: p.string("lineageTag"),
        is_hidden: p.flag("isHidden", false),
        columns: p.collection("columns", |o| Some(column(o))),
        measures: p.collection("measures", |o| Some(measure(o))),
        partitions: p.collection("partitions", |o| Some(partition(o))),
        annotations: p.annotations(),
        extra: p.extra(),
    }
}

fn column(o: &Obj) -> Column {
    let mut p = Props::new(o);
    let expression = p.expression("expression");
    // `type: calculated` is implied by an expression; the writer re-derives it.
    if expression.is_some() {
        p.take("type", |v| (v.as_str() == Some("calculated")).then_some(()));
    }
    Column {
        name: p.name("name"),
        data_type: p
            .take("dataType", |v| {
                v.as_str()
                    .map(DataType::from_tmsl)
                    .filter(|t| *t != DataType::Unknown)
            })
            .unwrap_or_default(),
        expression,
        source_column: p.string("sourceColumn"),
        format_string: p.string("formatString"),
        display_folder: p.string("displayFolder"),
        summarize_by: p.string("summarizeBy"),
        sort_by_column: p.string("sortByColumn"),
        data_category: p.string("dataCategory"),
        description: p.string("description"),
        lineage_tag: p.string("lineageTag"),
        is_hidden: p.flag("isHidden", false),
        annotations: p.annotations(),
        extra: p.extra(),
    }
}

fn measure(o: &Obj) -> Measure {
    let mut p = Props::new(o);
    Measure {
        name: p.name("name"),
        expression: p.required_expression("expression"),
        format_string: p.string("formatString"),
        display_folder: p.string("displayFolder"),
        description: p.string("description"),
        lineage_tag: p.string("lineageTag"),
        is_hidden: p.flag("isHidden", false),
        annotations: p.annotations(),
        extra: p.extra(),
    }
}

fn partition(o: &Obj) -> Partition {
    let mut p = Props::new(o);
    let name = p.name("name");
    let mode = p.string("mode");
    // Only a non-empty `source` object is split into typed fields; anything else stays verbatim.
    let source = p.take("source", |v| v.as_object().filter(|s| !s.is_empty()).cloned());
    let (source_type, expression, source_extra) = match &source {
        Some(s) => {
            let mut s = Props::new(s);
            let source_type = s.name("type");
            let expression = s.expression("expression");
            (source_type, expression, s.extra())
        }
        None => (String::new(), None, Properties::new()),
    };
    Partition {
        name,
        mode,
        source_type,
        expression,
        extra: p.extra(),
        source_extra,
    }
}

fn relationship(o: &Obj) -> Relationship {
    let mut p = Props::new(o);
    Relationship {
        name: p.name("name"),
        from_table: p.name("fromTable"),
        from_column: p.name("fromColumn"),
        to_table: p.name("toTable"),
        to_column: p.name("toColumn"),
        is_active: p.flag("isActive", true),
        cross_filtering_behavior: p.string("crossFilteringBehavior"),
        from_cardinality: p.string("fromCardinality"),
        to_cardinality: p.string("toCardinality"),
        annotations: p.annotations(),
        extra: p.extra(),
    }
}

fn named_expression(o: &Obj) -> NamedExpression {
    let mut p = Props::new(o);
    NamedExpression {
        name: p.name("name"),
        kind: p.name("kind"),
        expression: p.required_expression("expression"),
        description: p.string("description"),
        lineage_tag: p.string("lineageTag"),
        annotations: p.annotations(),
        extra: p.extra(),
    }
}

fn role(o: &Obj) -> Role {
    let mut p = Props::new(o);
    Role {
        name: p.name("name"),
        model_permission: p.string("modelPermission"),
        table_permissions: p.collection("tablePermissions", |o| {
            let mut t = Props::new(o);
            Some(TablePermission {
                table: t.name("name"),
                filter_expression: t.expression("filterExpression"),
                extra: t.extra(),
            })
        }),
        annotations: p.annotations(),
        extra: p.extra(),
    }
}

/// TMSL stores expressions as one string or as an array of lines (joined with `\n`).
pub(crate) fn expression_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Array(lines) if !lines.is_empty() => {
            let mut text = String::new();
            for (i, line) in lines.iter().enumerate() {
                if i > 0 {
                    text.push('\n');
                }
                text.push_str(line.as_str()?);
            }
            Some(text)
        }
        _ => None,
    }
}

/// A JSON object being split into typed fields. A property is consumed only when its typed field
/// can hold it and the writer will emit it again; everything else ends up in `extra`.
struct Props<'a> {
    src: &'a Obj,
    used: Vec<&'static str>,
}

impl<'a> Props<'a> {
    fn new(src: &'a Obj) -> Self {
        Self {
            src,
            used: Vec::new(),
        }
    }

    /// Consumes `key` when `f` accepts its value.
    fn take<T>(&mut self, key: &'static str, f: impl FnOnce(&Value) -> Option<T>) -> Option<T> {
        let value = f(self.src.get(key)?)?;
        self.used.push(key);
        Some(value)
    }

    /// A required string (`String` field): only a non-empty string is consumed.
    fn name(&mut self, key: &'static str) -> String {
        self.take(key, |v| v.as_str().filter(|s| !s.is_empty()).map(str::to_string))
            .unwrap_or_default()
    }

    /// An optional string (`Option<String>` field).
    fn string(&mut self, key: &'static str) -> Option<String> {
        self.take(key, |v| v.as_str().map(str::to_string))
    }

    fn expression(&mut self, key: &'static str) -> Option<String> {
        self.take(key, expression_text)
    }

    fn required_expression(&mut self, key: &'static str) -> String {
        self.take(key, |v| expression_text(v).filter(|s| !s.is_empty()))
            .unwrap_or_default()
    }

    /// A boolean whose `default` the writer omits; an explicit default stays in `extra`.
    fn flag(&mut self, key: &'static str, default: bool) -> bool {
        self.take(key, |v| v.as_bool().filter(|b| *b != default))
            .unwrap_or(default)
    }

    /// A child collection: consumed only when it is a non-empty array whose every element is an
    /// object that `f` accepts. Otherwise it is kept verbatim in `extra`.
    fn collection<T>(&mut self, key: &'static str, f: impl Fn(&Obj) -> Option<T>) -> Vec<T> {
        self.take(key, |v| {
            let items = v.as_array().filter(|a| !a.is_empty())?;
            items.iter().map(|i| i.as_object().and_then(&f)).collect()
        })
        .unwrap_or_default()
    }

    /// `annotations`: modelled only when every entry is exactly `{name, value}` with strings.
    fn annotations(&mut self) -> Vec<Annotation> {
        self.collection("annotations", |o| {
            if o.len() != 2 {
                return None;
            }
            Some(Annotation {
                name: o.get("name")?.as_str()?.to_string(),
                value: o.get("value")?.as_str()?.to_string(),
            })
        })
    }

    fn extra(self) -> Properties {
        self.src
            .iter()
            .filter(|(k, _)| !self.used.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}
