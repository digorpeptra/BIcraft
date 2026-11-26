//! A canonical text form of one file's model and report, for git's `textconv`: objects sorted
//! by name, one property per line, expressions as indented blocks, noise (`$schema`, version,
//! timestamps, generated ids) left out, so that `git diff` of two dumps is a readable,
//! semantic line diff of two binary PBIX files.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use bicraft_model::{Annotation, SemanticModel};
use bicraft_report::Report;
use serde_json::{Map, Value};

use crate::json::{NOISE, flatten_objects};
use crate::model::{NOISY_ANNOTATIONS, is_parameter, role_extra};
use crate::report::{field_text, num};
use crate::{bracket, dq, quote_table};

/// Longest line written for a JSON value (characters).
const MAX_LINE: usize = 2_000;

fn one_line(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

fn json_line(v: &Value) -> String {
    let s = one_line(&serde_json::to_string(&sorted(v, 0)).unwrap_or_default());
    if s.chars().count() > MAX_LINE {
        let mut t: String = s.chars().take(MAX_LINE).collect();
        t.push('…');
        t
    } else {
        s
    }
}

/// The value with object keys sorted and noise keys dropped (bounded depth).
fn sorted(v: &Value, depth: usize) -> Value {
    if depth > crate::json::MAX_DEPTH {
        return v.clone();
    }
    match v {
        Value::Object(m) => {
            let b: BTreeMap<&String, Value> = m
                .iter()
                .filter(|(k, _)| !NOISE.contains(&k.as_str()))
                .map(|(k, x)| (k, sorted(x, depth + 1)))
                .collect();
            Value::Object(b.into_iter().map(|(k, x)| (k.clone(), x)).collect())
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| sorted(x, depth + 1)).collect()),
        _ => v.clone(),
    }
}

struct W {
    s: String,
}

impl W {
    fn line(&mut self, indent: usize, text: &str) {
        let _ = writeln!(self.s, "{:indent$}{}", "", one_line(text));
    }
    fn prop(&mut self, indent: usize, name: &str, v: Option<&str>) {
        if let Some(v) = v.filter(|v| !v.is_empty()) {
            if v.contains('\n') || v.contains('\r') {
                self.block(indent, name, v);
            } else {
                self.line(indent, &format!("{name}: {v}"));
            }
        }
    }
    fn block(&mut self, indent: usize, name: &str, text: &str) {
        self.line(indent, &format!("{name}:"));
        for l in crate::norm::lf(text).lines() {
            let _ = writeln!(self.s, "{:w$}{}", "", l.trim_end(), w = indent + 4);
        }
    }
    fn expr(&mut self, indent: usize, name: &str, text: Option<&str>) {
        if let Some(t) = text {
            self.block(indent, name, t);
        }
    }
    fn flag(&mut self, indent: usize, name: &str, on: bool) {
        if on {
            self.line(indent, name);
        }
    }
    fn annotations(&mut self, indent: usize, a: &[Annotation]) {
        let m: BTreeMap<&str, &str> = a
            .iter()
            .filter(|x| !NOISY_ANNOTATIONS.contains(&x.name.as_str()))
            .map(|x| (x.name.as_str(), x.value.as_str()))
            .collect();
        for (k, v) in m {
            self.prop(indent, &format!("annotation {k}"), Some(v));
        }
    }
    fn extras(&mut self, indent: usize, m: &Map<String, Value>, skip: &[&str]) {
        let keys: BTreeMap<&String, &Value> = m
            .iter()
            .filter(|(k, _)| !NOISE.contains(&k.as_str()) && !skip.contains(&k.as_str()))
            .collect();
        for (k, v) in keys {
            match v {
                Value::String(s) => self.prop(indent, k, Some(s)),
                other => self.line(indent, &format!("{k}: {}", json_line(other))),
            }
        }
    }
}

fn by_name<T>(items: &[T], name: impl Fn(&T) -> &str) -> Vec<&T> {
    let mut v: Vec<&T> = items.iter().collect();
    v.sort_by(|a, b| name(a).cmp(name(b)));
    v
}

/// The canonical text of a model and a report.
pub fn dump(model: Option<&SemanticModel>, report: Option<&Report>) -> String {
    let mut w = W { s: String::new() };
    if let Some(m) = model {
        dump_model(&mut w, m);
    }
    if let Some(r) = report {
        dump_report(&mut w, r);
    }
    w.s
}

fn dump_model(w: &mut W, m: &SemanticModel) {
    w.line(0, "model");
    w.prop(2, "culture", m.culture.as_deref());
    w.prop(2, "description", m.description.as_deref());
    if let Some(l) = m.compatibility_level {
        w.line(2, &format!("compatibility_level: {l}"));
    }
    w.annotations(2, &m.annotations);
    w.extras(2, &m.extra, &["name"]);
    for t in by_name(&m.tables, |t| &t.name) {
        w.line(0, &format!("table {}", quote_table(&t.name)));
        w.prop(2, "description", t.description.as_deref());
        w.flag(2, "hidden", t.is_hidden);
        w.annotations(2, &t.annotations);
        w.extras(2, &t.extra, &["lineageTag"]);
        for c in by_name(&t.columns, |c| &c.name) {
            if c.is_row_number() {
                continue;
            }
            let ty = serde_json::to_value(c.data_type)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();
            w.line(
                2,
                &format!("column {}{}: {ty}", quote_table(&t.name), bracket(&c.name)),
            );
            w.expr(4, "expression", c.expression.as_deref());
            w.prop(4, "source_column", c.source_column.as_deref());
            w.prop(4, "format_string", c.format_string.as_deref());
            w.prop(4, "display_folder", c.display_folder.as_deref());
            w.prop(
                4,
                "summarize_by",
                c.summarize_by
                    .as_deref()
                    .filter(|s| !s.eq_ignore_ascii_case("default")),
            );
            w.prop(4, "sort_by_column", c.sort_by_column.as_deref());
            w.prop(4, "data_category", c.data_category.as_deref());
            w.prop(4, "description", c.description.as_deref());
            w.flag(4, "hidden", c.is_hidden);
            w.annotations(4, &c.annotations);
            w.extras(4, &c.extra, &["lineageTag"]);
        }
        for x in by_name(&t.measures, |x| &x.name) {
            w.line(
                2,
                &format!("measure {}{}", quote_table(&t.name), bracket(&x.name)),
            );
            w.expr(4, "expression", Some(&x.expression));
            w.prop(4, "format_string", x.format_string.as_deref());
            w.prop(4, "display_folder", x.display_folder.as_deref());
            w.prop(4, "description", x.description.as_deref());
            w.flag(4, "hidden", x.is_hidden);
            w.annotations(4, &x.annotations);
            w.extras(4, &x.extra, &["lineageTag"]);
        }
        for p in by_name(&t.partitions, |p| &p.name) {
            w.line(2, &format!("partition {}", one_line(&p.source_type)));
            w.prop(4, "mode", p.mode.as_deref());
            w.expr(4, "expression", p.expression.as_deref());
            w.extras(4, &p.extra, &[]);
            w.extras(4, &p.source_extra, &[]);
        }
    }
    let mut rels: Vec<String> = m
        .relationships
        .iter()
        .map(|r| {
            let mut s = format!(
                "relationship {}{} -> {}{}",
                quote_table(&r.from_table),
                bracket(&r.from_column),
                quote_table(&r.to_table),
                bracket(&r.to_column)
            );
            if !r.is_active {
                s.push_str(" inactive");
            }
            for v in [
                &r.cross_filtering_behavior,
                &r.from_cardinality,
                &r.to_cardinality,
            ]
            .into_iter()
            .flatten()
            {
                let _ = write!(s, " {v}");
            }
            s
        })
        .collect();
    rels.sort();
    for r in rels {
        w.line(0, &r);
    }
    for r in by_name(&m.roles, |r| &r.name) {
        w.line(0, &format!("role {}", quote_table(&r.name)));
        w.prop(2, "model_permission", r.model_permission.as_deref());
        for p in by_name(&r.table_permissions, |p| &p.table) {
            w.expr(
                2,
                &format!("filter {}", quote_table(&p.table)),
                p.filter_expression.as_deref(),
            );
        }
        w.annotations(2, &r.annotations);
        w.extras(2, &role_extra(r), &[]);
    }
    for e in by_name(&m.expressions, |e| &e.name) {
        let kind = if is_parameter(e) {
            "parameter"
        } else {
            "expression"
        };
        w.line(0, &format!("{kind} {}", quote_table(&e.name)));
        w.expr(2, "expression", Some(&e.expression));
        w.prop(2, "description", e.description.as_deref());
        w.annotations(2, &e.annotations);
        w.extras(2, &e.extra, &["lineageTag"]);
    }
}

fn dump_report(w: &mut W, r: &Report) {
    w.line(0, "report");
    let order: Vec<&str> = r.pages.iter().map(|p| p.display_name.as_str()).collect();
    w.line(2, &format!("page_order: {}", json_line(&Value::from(order))));
    if let Some(t) = &r.theme_collection {
        w.line(2, &format!("theme: {}", json_line(t)));
    }
    if let Some(f) = &r.filter_config {
        w.line(2, &format!("filters: {}", json_line(f)));
    }
    w.extras(2, &r.extra, &[]);
    for p in by_name(&r.pages, |p| &p.display_name) {
        w.line(0, &format!("page {}", dq(&p.display_name)));
        w.line(2, &format!("size: {}x{}", num(p.width), num(p.height)));
        w.prop(2, "display_option", p.display_option.as_deref());
        w.prop(2, "visibility", p.visibility.as_deref());
        if let Some(f) = &p.filter_config {
            w.line(2, &format!("filters: {}", json_line(f)));
        }
        w.extras(2, &p.extra, &["name", "displayName", "ordinal"]);
        let mut visuals: Vec<&bicraft_report::Visual> = p.visuals.iter().collect();
        visuals.sort_by(|a, b| {
            (num(a.position.y), num(a.position.x), &a.name).cmp(&(
                num(b.position.y),
                num(b.position.x),
                &b.name,
            ))
        });
        for v in visuals {
            let pos = &v.position;
            w.line(
                2,
                &format!(
                    "visual {} {} at {},{} {}x{}",
                    v.visual_type.as_deref().unwrap_or("group"),
                    v.name,
                    num(pos.x),
                    num(pos.y),
                    num(pos.width),
                    num(pos.height)
                ),
            );
            for f in &v.fields {
                w.line(4, &format!("{}: {}", f.role, field_text(&f.field)));
            }
            w.flag(4, "hidden", v.is_hidden);
            w.prop(4, "parent_group", v.parent_group.as_deref());
            if let Some(f) = &v.filter_config {
                w.line(4, &format!("filters: {}", json_line(f)));
            }
            for key in ["objects", "visualContainerObjects"] {
                for (k, x) in flatten_objects(v.visual_extra.get(key)) {
                    w.line(4, &format!("format {k}: {}", json_line(&x)));
                }
            }
            w.extras(
                4,
                &v.visual_extra,
                &["objects", "visualContainerObjects", "query"],
            );
        }
    }
    for b in by_name(&r.bookmarks, |b| &b.name) {
        let name = b
            .document
            .get("displayName")
            .and_then(Value::as_str)
            .unwrap_or(&b.name);
        w.line(0, &format!("bookmark {}", dq(name)));
    }
    for res in by_name(&r.resources, |x| &x.path) {
        w.line(
            0,
            &format!(
                "resource {} ({} bytes)",
                res.path.replace('\\', "/"),
                res.bytes.len()
            ),
        );
    }
}
