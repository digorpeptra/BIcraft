//! Where a report uses model objects, for `model.usage` / `model.unused` (`usage.rs`).
//!
//! Reads the report only through the report model ([`bicraft_report::Report`]): typed visual
//! bindings, plus the raw PBIR JSON the report keeps verbatim (filters, queries, formatting,
//! bookmarks, page bindings), walked like [`crate::refs::find_refs`] but keeping track of *where*
//! in the document each reference sits, so it can be classified:
//!
//! | Where | Usage kind |
//! |---|---|
//! | a visual binding / `query.queryState.<Role>` | `visual` (with `role`) |
//! | `sortDefinition` | `sort` |
//! | `objects`, `visualContainerObjects` (conditional formatting, titles from fields) | `formatting` |
//! | a visual's `filterConfig` | `visual filter` |
//! | a page's `filterConfig` | `page filter`, or `drillthrough` for filters Desktop created for drillthrough |
//! | a page's `pageBinding` | `drillthrough` or `tooltip page` |
//! | the report's `filterConfig` | `report filter` |
//! | a bookmark | `bookmark` |
//! | anything else kept verbatim | `visual property`, `page property`, `report property` |
//!
//! Report-level measures (`reportExtensions.json`) are listed separately ([`report_measure_refs`]).

use bicraft_dax::deps::ObjectRef;
use bicraft_dax::rename::{Renamer, Site};
use bicraft_report::{FieldRef, Report};
use serde_json::{Map, Value, json};

use crate::refs::field_parts;

/// Deepest JSON nesting walked (serde_json already stops at 128).
const MAX_DEPTH: usize = 256;

/// A model reference found in a report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReportRef {
    Column {
        table: String,
        name: String,
    },
    Measure {
        table: String,
        name: String,
    },
    /// A level of a user hierarchy (`HierarchyLevel`).
    Level {
        table: String,
        hierarchy: String,
        level: String,
    },
}

/// Where in a document the walk is.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Class {
    Default,
    QueryState,
    Field(Option<String>),
    Filter,
    Drillthrough,
    TooltipPage,
    Sort,
    Formatting,
    Bookmark,
}

/// The class of `key`'s value inside a value of class `c`.
fn descend(c: &Class, key: &str) -> Class {
    match (c, key) {
        (Class::Bookmark | Class::Drillthrough | Class::TooltipPage, _) => c.clone(),
        (_, "sortDefinition") => Class::Sort,
        (_, "objects" | "visualContainerObjects") => Class::Formatting,
        (Class::QueryState, role) => Class::Field(Some(role.to_string())),
        (Class::Default, "queryState") => Class::QueryState,
        (Class::Default, "prototypeQuery") => Class::Field(None),
        (Class::Default, "filterConfig" | "filters") => Class::Filter,
        _ => c.clone(),
    }
}

/// Which part of the report a document belongs to.
#[derive(Clone, Copy)]
enum Scope<'a> {
    Report,
    Page(&'a str),
    Visual {
        page: &'a str,
        visual: &'a str,
        visual_type: Option<&'a str>,
    },
}

fn usage(report: &str, scope: Scope<'_>, class: &Class, bookmark: Option<&str>) -> Value {
    let mut o = Map::new();
    let kind = match (scope, class) {
        (_, Class::Bookmark) => "bookmark",
        (Scope::Visual { .. }, Class::Field(_)) => "visual",
        (Scope::Visual { .. }, Class::Sort) => "sort",
        (Scope::Visual { .. }, Class::Formatting) => "formatting",
        (Scope::Visual { .. }, Class::Filter | Class::Drillthrough) => "visual filter",
        (Scope::Visual { .. }, _) => "visual property",
        (Scope::Page(_), Class::Filter) => "page filter",
        (Scope::Page(_), Class::Drillthrough) => "drillthrough",
        (Scope::Page(_), Class::TooltipPage) => "tooltip page",
        (Scope::Page(_), Class::Formatting) => "page formatting",
        (Scope::Page(_), _) => "page property",
        (Scope::Report, Class::Filter | Class::Drillthrough) => "report filter",
        (Scope::Report, Class::Formatting) => "report formatting",
        (Scope::Report, _) => "report property",
    };
    o.insert("kind".into(), json!(kind));
    o.insert("report".into(), json!(report));
    match scope {
        Scope::Report => {}
        Scope::Page(page) => {
            o.insert("page".into(), json!(page));
        }
        Scope::Visual {
            page,
            visual,
            visual_type,
        } => {
            o.insert("page".into(), json!(page));
            o.insert("visual".into(), json!(visual));
            if let Some(t) = visual_type {
                o.insert("visual_type".into(), json!(t));
            }
        }
    }
    if let Class::Field(Some(role)) = class {
        o.insert("role".into(), json!(role));
    }
    if let Some(b) = bookmark {
        o.insert("bookmark".into(), json!(b));
    }
    Value::Object(o)
}

/// `{Expression: {Hierarchy: {Expression: {SourceRef}, Hierarchy}}, Level}` → (table, hierarchy,
/// level). Hierarchies of auto date/time variations (`PropertyVariationSource`) are column
/// references, found as such.
fn hierarchy_level(h: &Map<String, Value>, scope: &[(String, String)]) -> Option<ReportRef> {
    let level = h.get("Level").and_then(Value::as_str)?;
    let hier = h.get("Expression")?.get("Hierarchy")?;
    let name = hier.get("Hierarchy").and_then(Value::as_str)?;
    let source = hier.get("Expression")?.get("SourceRef")?;
    let table = match source.get("Entity").and_then(Value::as_str) {
        Some(e) => e.to_string(),
        None => {
            let alias = source.get("Source").and_then(Value::as_str)?;
            scope.iter().rev().find(|(n, _)| n == alias)?.1.clone()
        }
    };
    Some(ReportRef::Level {
        table,
        hierarchy: name.to_string(),
        level: level.to_string(),
    })
}

fn walk(
    v: &Value,
    scope: &mut Vec<(String, String)>,
    class: &Class,
    depth: usize,
    f: &mut dyn FnMut(ReportRef, &Class),
) {
    if depth > MAX_DEPTH {
        return;
    }
    match v {
        Value::Array(items) => {
            for item in items {
                walk(item, scope, class, depth.saturating_add(1), f);
            }
        }
        Value::Object(o) => {
            let before = scope.len();
            if let Some(Value::Array(from)) = o.get("From") {
                for item in from {
                    if let (Some(name), Some(entity)) = (
                        item.get("Name").and_then(Value::as_str),
                        item.get("Entity").and_then(Value::as_str),
                    ) {
                        scope.push((name.to_string(), entity.to_string()));
                    }
                }
            }
            let drillthrough = matches!(class, Class::Filter)
                && o.get("howCreated")
                    .and_then(Value::as_str)
                    .is_some_and(|h| h.eq_ignore_ascii_case("drillthrough"));
            let here = if drillthrough {
                Class::Drillthrough
            } else {
                class.clone()
            };
            for (key, measure) in [
                ("Column", false),
                ("Measure", true),
                ("PropertyVariationSource", false),
            ] {
                if let Some(Value::Object(inner)) = o.get(key)
                    && let Some((table, name, _, extension)) = field_parts(inner, scope)
                    && !extension
                {
                    let r = if measure {
                        ReportRef::Measure { table, name }
                    } else {
                        ReportRef::Column { table, name }
                    };
                    f(r, &here);
                }
            }
            if let Some(Value::Object(h)) = o.get("HierarchyLevel")
                && let Some(r) = hierarchy_level(h, scope)
            {
                f(r, &here);
            }
            for (k, child) in o {
                if child.is_array() || child.is_object() {
                    walk(child, scope, &descend(&here, k), depth.saturating_add(1), f);
                }
            }
            scope.truncate(before);
        }
        _ => {}
    }
}

fn walk_doc(v: &Value, class: &Class, f: &mut dyn FnMut(ReportRef, &Class)) {
    let mut scope = Vec::new();
    walk(v, &mut scope, class, 0, f);
}

/// Calls `f` for every model reference of `r` (report-level measures excepted) with its usage
/// (`report` names the report in each usage).
pub(crate) fn report_refs(report: &str, r: &Report, f: &mut dyn FnMut(ReportRef, Value)) {
    let emit = |f: &mut dyn FnMut(ReportRef, Value),
                scope: Scope<'_>,
                bookmark: Option<&str>,
                v: &Value,
                class: Class| {
        walk_doc(v, &class, &mut |found, c| {
            f(found, usage(report, scope, c, bookmark))
        });
    };
    if let Some(v) = &r.filter_config {
        emit(f, Scope::Report, None, v, Class::Filter);
    }
    for (k, v) in r.extra.iter().chain(r.pages_extra.iter()) {
        emit(f, Scope::Report, None, v, descend(&Class::Default, k));
    }
    for b in &r.bookmarks {
        emit(f, Scope::Report, Some(&b.name), &b.document, Class::Bookmark);
    }
    for page in &r.pages {
        let display = if page.display_name.is_empty() {
            page.name.as_str()
        } else {
            page.display_name.as_str()
        };
        if let Some(v) = &page.filter_config {
            emit(f, Scope::Page(display), None, v, Class::Filter);
        }
        for (k, v) in &page.extra {
            let class = if k == "pageBinding" {
                let tooltip = v
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|t| t.eq_ignore_ascii_case("tooltip"));
                if tooltip {
                    Class::TooltipPage
                } else {
                    Class::Drillthrough
                }
            } else {
                descend(&Class::Default, k)
            };
            emit(f, Scope::Page(display), None, v, class);
        }
        for visual in &page.visuals {
            let scope = Scope::Visual {
                page: display,
                visual: &visual.name,
                visual_type: visual.visual_type.as_deref(),
            };
            for b in &visual.fields {
                let class = Class::Field(Some(b.role.clone()));
                let found = match &b.field {
                    FieldRef::Column { table, column } | FieldRef::Aggregation { table, column, .. } => {
                        ReportRef::Column {
                            table: table.clone(),
                            name: column.clone(),
                        }
                    }
                    FieldRef::Measure { table, measure } => ReportRef::Measure {
                        table: table.clone(),
                        name: measure.clone(),
                    },
                    FieldRef::Other { raw } => {
                        emit(f, scope, None, raw, class);
                        continue;
                    }
                };
                f(found, usage(report, scope, &class, None));
            }
            if let Some(v) = &visual.filter_config {
                emit(f, scope, None, v, Class::Filter);
            }
            for (k, v) in visual.visual_extra.iter().chain(visual.extra.iter()) {
                emit(f, scope, None, v, descend(&Class::Default, k));
            }
        }
    }
}

/// The model objects each report-level measure of `r` references: `(measure usage, refs)`.
/// A measure whose DAX does not parse comes with `None`.
pub(crate) fn report_measure_refs(
    report: &str,
    r: &Report,
    renamer: &Renamer<'_>,
) -> Vec<(Value, Option<Vec<ObjectRef>>, String)> {
    crate::report_measures::report_measures(r)
        .iter()
        .map(|m| {
            let refs = renamer.references_as_used(
                &m.expression,
                Site {
                    home: Some(&m.entity),
                    row_context: false,
                    udf: false,
                },
            );
            (
                json!({"kind": "report measure", "report": report, "table": m.entity, "name": m.name}),
                refs,
                m.expression.clone(),
            )
        })
        .collect()
}
