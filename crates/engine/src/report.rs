//! Report commands: `report.*`, `page.*`, `visual.*`.
//!
//! They edit the report `file.open` read (or `report.new` started) in place: every lookup and
//! check runs first, and the report is only touched once nothing can fail any more. Visual fields
//! are resolved against the open model (see `refs.rs`). Pages and visuals get 20-hex-digit names,
//! like Power BI Desktop gives them; they become PBIR folder names.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use bicraft_model::SemanticModel;
use bicraft_report::{FieldBinding, FieldRef, Page, Position, Report, Visual};
use serde_json::{Value, json};

use crate::edit::{check_name, new_guid};
use crate::params::{allow, force, opt_bool, opt_str, str_param};
use crate::refs::{
    ParsedField, SiteKind, check_field, field_json, field_text, find_refs, parse_field, raw_documents_ref,
    resolve_field, same_field,
};
use crate::{Session, Source};

/// Page `displayOption` values.
pub(crate) const DISPLAY_OPTIONS: &[&str] = &["FitToPage", "FitToWidth", "ActualSize"];
/// Largest page or visual dimension and coordinate accepted, in pixels.
pub(crate) const MAX_SIZE: f64 = 100_000.0;
/// Most issues `report.validate` lists; the counts are always exact.
const MAX_ISSUES: usize = 500;
/// Longest visual type name accepted (custom visuals use long generated names).
pub(crate) const MAX_VISUAL_TYPE: usize = 200;
/// Most fields one `visual.add` binds.
pub(crate) const MAX_FIELDS: usize = 1000;

/// Visual types built into Power BI Desktop. Others (custom visuals) are accepted with a
/// warning, since a typo would otherwise only show up when Desktop opens the report.
pub(crate) const KNOWN_VISUAL_TYPES: &[&str] = &[
    "actionButton",
    "advancedSlicerVisual",
    "aiNarratives",
    "areaChart",
    "azureMap",
    "barChart",
    "basicShape",
    "bookmarkNavigator",
    "card",
    "cardVisual",
    "clusteredBarChart",
    "clusteredColumnChart",
    "columnChart",
    "decompositionTreeVisual",
    "donutChart",
    "filledMap",
    "funnel",
    "ganttChart",
    "gauge",
    "hundredPercentStackedAreaChart",
    "hundredPercentStackedBarChart",
    "hundredPercentStackedColumnChart",
    "image",
    "keyDriversVisual",
    "kpi",
    "lineChart",
    "lineClusteredColumnComboChart",
    "lineStackedColumnComboChart",
    "listSlicer",
    "map",
    "multiRowCard",
    "pageNavigator",
    "pieChart",
    "pivotTable",
    "pythonVisual",
    "qnaVisual",
    "ribbonChart",
    "scatterChart",
    "scorecard",
    "scriptVisual",
    "shape",
    "shapeMap",
    "slicer",
    "stackedAreaChart",
    "tableEx",
    "textSlicer",
    "textbox",
    "treemap",
    "waterfallChart",
];

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

fn no_report(s: &Session) -> String {
    match (&s.report_error, &s.source) {
        (Some(e), _) => format!("the open file has no readable report: {e}"),
        (None, Some(Source::Bim { .. })) => {
            "a model.bim has no report: run report.new to start one, then save with file.save_as to a .pbip"
                .into()
        }
        (None, None) if s.model.is_some() => "no report is open: run report.new to start one".into(),
        _ => "no report is open (open a PBIX, PBIT or PBIP project with file.open, or run report.new)".into(),
    }
}

pub(crate) fn report_ref(s: &Session) -> Result<&Report, String> {
    s.report.as_ref().ok_or_else(|| no_report(s))
}

/// The open model (if any) and the report, borrowed together.
pub(crate) fn parts(s: &mut Session) -> Result<(Option<&SemanticModel>, &mut Report), String> {
    if s.report.is_none() {
        return Err(no_report(s));
    }
    match s.report.as_mut() {
        Some(r) => Ok((s.model.as_ref(), r)),
        None => Err("no report is open".into()),
    }
}

/// A page by name (exact, then case-insensitive) or display name (case-insensitive, unique).
pub(crate) fn page_index(r: &Report, key: &str) -> Result<usize, String> {
    if let Some(i) = r.pages.iter().position(|p| p.name == key) {
        return Ok(i);
    }
    if let Some(i) = r.pages.iter().position(|p| p.name.eq_ignore_ascii_case(key)) {
        return Ok(i);
    }
    let matches: Vec<usize> = r
        .pages
        .iter()
        .enumerate()
        .filter(|(_, p)| p.display_name.eq_ignore_ascii_case(key))
        .map(|(i, _)| i)
        .collect();
    match matches.as_slice() {
        [i] => Ok(*i),
        [] => Err(format!(
            "no page named `{key}` (pages: {})",
            r.pages
                .iter()
                .map(|p| format!("{} ({})", p.display_name, p.name))
                .collect::<Vec<_>>()
                .join(", ")
        )),
        _ => Err(format!(
            "several pages are called `{key}`: use the page name (report.summary lists them)"
        )),
    }
}

fn page_at(r: &Report, i: usize) -> Result<&Page, String> {
    r.pages
        .get(i)
        .ok_or_else(|| "internal: page index out of range".to_string())
}

pub(crate) fn page_at_mut(r: &mut Report, i: usize) -> Result<&mut Page, String> {
    r.pages
        .get_mut(i)
        .ok_or_else(|| "internal: page index out of range".to_string())
}

pub(crate) fn visual_index(p: &Page, key: &str) -> Result<usize, String> {
    p.visuals
        .iter()
        .position(|v| v.name == key)
        .or_else(|| p.visuals.iter().position(|v| v.name.eq_ignore_ascii_case(key)))
        // the key it was added with (`visual.add {key}`)
        .or_else(|| {
            let named = crate::report_apply::visual_name_for_key(&p.name, key);
            p.visuals.iter().position(|v| v.name == named)
        })
        .ok_or_else(|| {
            format!(
                "page `{}` has no visual named `{key}` (report.summary lists them)",
                p.display_name
            )
        })
}

/// An optional finite number.
pub(crate) fn opt_num(p: &Value, key: &str) -> Result<Option<f64>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_f64() {
            Some(n) if n.is_finite() => Ok(Some(n)),
            _ => Err(format!("parameter `{key}` must be a number")),
        },
    }
}

fn num(p: &Value, key: &str) -> Result<f64, String> {
    opt_num(p, key)?.ok_or_else(|| format!("missing number parameter `{key}`"))
}

pub(crate) fn check_size(key: &str, v: f64) -> Result<f64, String> {
    if v > 0.0 && v <= MAX_SIZE {
        Ok(v)
    } else {
        Err(format!("`{key}` must be greater than 0 and at most {MAX_SIZE}"))
    }
}

pub(crate) fn check_coordinate(key: &str, v: f64) -> Result<f64, String> {
    if (-MAX_SIZE..=MAX_SIZE).contains(&v) {
        Ok(v)
    } else {
        Err(format!("`{key}` must be between -{MAX_SIZE} and {MAX_SIZE}"))
    }
}

pub(crate) fn opt_index(p: &Value, key: &str) -> Result<Option<usize>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .map(Some)
            .ok_or_else(|| format!("parameter `{key}` must be a non-negative integer")),
    }
}

/// A data role name: `Values`, `Category`, `Y`, `Rows`…
pub(crate) fn check_role(role: &str) -> Result<(), String> {
    if role.is_empty() || role.len() > 100 || !role.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!(
            "`{role}` is not a data role name like Values, Category, Y, Legend or Rows"
        ));
    }
    Ok(())
}

/// Data-role findings of an edit: errors fail it (unless `force`, which keeps them as warnings),
/// warnings are passed on.
pub(crate) fn role_verdict(
    issues: Vec<crate::roles::RoleIssue>,
    force: bool,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    let mut errors = Vec::new();
    for i in issues {
        if i.error && !force {
            errors.push(i.message);
        } else if i.error {
            warnings.push(format!("{} (bound anyway: force)", i.message));
        } else {
            warnings.push(i.message);
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} (pass \"force\": true to bind anyway)",
            errors.join("; ")
        ))
    }
}

/// A fresh 20-hex-digit name (Desktop's shape for pages and visuals) that `taken` rejects.
pub(crate) fn new_object_name(taken: impl Fn(&str) -> bool) -> String {
    for _ in 0..64 {
        let hex: String = new_guid()
            .chars()
            .filter(char::is_ascii_hexdigit)
            .enumerate()
            // Skip the GUID's fixed version and variant digits.
            .filter(|(i, _)| *i != 12 && *i != 16)
            .map(|(_, c)| c)
            .take(20)
            .collect();
        if !taken(&hex) {
            return hex;
        }
    }
    // Unreachable in practice; still never return a taken name.
    let mut n = 0u64;
    loop {
        let name = format!("{n:020x}");
        if !taken(&name) {
            return name;
        }
        n = n.saturating_add(1);
    }
}

pub(crate) fn position_json(p: &Position) -> Value {
    json!({"x": p.x, "y": p.y, "z": p.z, "width": p.width, "height": p.height, "tab_order": p.tab_order})
}

pub(crate) fn bindings_json(fields: &[FieldBinding]) -> Vec<Value> {
    fields
        .iter()
        .map(|b| {
            let mut v = field_json(&b.field);
            if let Some(o) = v.as_object_mut() {
                o.insert("role".into(), json!(b.role));
                if let Some(k) = b.show_as {
                    o.insert("show_as".into(), json!(k.as_str()));
                }
            }
            v
        })
        .collect()
}

/// Resolves a field for binding: a model field, else one of the report's own measures
/// (`measures`, see `report_measures.rs`); with `force`, a field the model does not know is bound
/// as written (a parse error still fails).
pub(crate) fn resolve_for_binding(
    model: Option<&SemanticModel>,
    measures: &[crate::report_measures::ReportMeasure],
    text: &str,
    force: bool,
    warnings: &mut Vec<String>,
) -> Result<FieldRef, String> {
    if model.is_none()
        && let Some(f) = crate::report_measures::resolve(measures, text)
    {
        return Ok(f);
    }
    match resolve_field(model, text) {
        Ok(r) => {
            warnings.extend(r.warnings);
            Ok(r.field)
        }
        Err(e) if crate::report_measures::resolve(measures, text).is_some() => {
            crate::report_measures::resolve(measures, text).ok_or(e)
        }
        Err(e) if force && model.is_some() => {
            let r = resolve_field(None, text).map_err(|_| e.clone())?;
            warnings.push(format!("bound although the model does not know it (force): {e}"));
            Ok(r.field)
        }
        Err(e) => Err(if model.is_some() {
            format!("{e} (pass \"force\": true to bind it anyway)")
        } else {
            e
        }),
    }
}

/// Whether `b` is the field `parsed` names (no model needed, so broken bindings can be removed).
pub(crate) fn binding_matches(b: &FieldRef, parsed: &ParsedField) -> bool {
    let eq = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    match (parsed, b) {
        (ParsedField::Measure(name), FieldRef::Measure { measure, .. }) => eq(name, measure),
        (ParsedField::Qualified { table, name }, FieldRef::Column { table: t, column })
        | (
            ParsedField::Qualified { table, name },
            FieldRef::Measure {
                table: t,
                measure: column,
            },
        ) => eq(table, t) && eq(name, column),
        (
            ParsedField::Aggregation {
                function,
                table,
                column,
            },
            FieldRef::Aggregation {
                table: t,
                column: c,
                function: f,
            },
        ) => eq(function, f) && eq(table, t) && eq(column, c),
        _ => false,
    }
}

/// Whether a JSON document mentions `text` as a string value anywhere (bounded depth).
fn mentions(v: &Value, text: &str, depth: usize) -> bool {
    if depth > 128 {
        return false;
    }
    match v {
        Value::String(s) => s == text,
        Value::Array(a) => a.iter().any(|x| mentions(x, text, depth.saturating_add(1))),
        Value::Object(o) => o.values().any(|x| mentions(x, text, depth.saturating_add(1))),
        _ => false,
    }
}

// ---------------------------------------------------------------------------------------------
// report.*
// ---------------------------------------------------------------------------------------------

pub(crate) fn report_new(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &[])?;
    if s.report.is_some() {
        return Err("a report is already open; edit it with page.* and visual.* commands".into());
    }
    if let (Some(e), Some(Source::Project { .. })) = (&s.report_error, &s.source) {
        return Err(format!(
            "the project's report could not be read ({e}); a new report would replace it on file.save, \
             so report.new is refused for this project"
        ));
    }
    let name = new_object_name(|_| false);
    let report = bicraft_pbip::blank_report(&name);
    let out = json!({
        "pages": [{"name": name, "display_name": report.pages.first().map(|p| p.display_name.clone())}],
        "active_page": name,
        "next": "add visuals with visual.add, pages with page.add; save with file.save (PBIP), \
                 project.new or file.save_as",
    });
    s.report = Some(report);
    s.report_error = None;
    s.report_dirty = true;
    Ok(out)
}

pub(crate) fn report_summary(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visuals"])?;
    let only = opt_str(p, "page")?;
    let with_visuals = opt_bool(p, "visuals")?.unwrap_or(true);
    let r = report_ref(s)?;
    let only = only.map(|k| page_index(r, k)).transpose()?;
    let mut types: BTreeMap<String, usize> = BTreeMap::new();
    let mut fields: BTreeSet<String> = BTreeSet::new();
    let mut visual_count = 0usize;
    let mut pages = Vec::new();
    for (i, page) in r.pages.iter().enumerate() {
        if only.is_some_and(|o| o != i) {
            continue;
        }
        visual_count = visual_count.saturating_add(page.visuals.len());
        for v in &page.visuals {
            let t = v.visual_type.clone().unwrap_or_else(|| "(group)".into());
            *types.entry(t).or_default() += 1;
            for b in &v.fields {
                fields.insert(field_text(&b.field));
            }
        }
        let active = r
            .active_page
            .as_deref()
            .is_some_and(|a| a.eq_ignore_ascii_case(&page.name));
        let mut entry = json!({
            "index": i,
            "name": page.name,
            "display_name": page.display_name,
            "width": page.width,
            "height": page.height,
            "display_option": page.display_option,
            "visibility": page.visibility,
            "active": active,
            "visual_count": page.visuals.len(),
            "has_filters": page.filter_config.is_some(),
        });
        if with_visuals && let Some(o) = entry.as_object_mut() {
            let visuals: Vec<Value> = page
                .visuals
                .iter()
                .map(|v| {
                    json!({
                        "name": v.name,
                        "visual_type": v.visual_type,
                        "position": position_json(&v.position),
                        "hidden": v.is_hidden,
                        "group": v.parent_group,
                        "fields": bindings_json(&v.fields),
                    })
                })
                .collect();
            o.insert("visuals".into(), json!(visuals));
        }
        pages.push(entry);
    }
    Ok(json!({
        "page_count": r.pages.len(),
        "visual_count": visual_count,
        "active_page": r.active_page,
        "visual_types": types,
        "fields": fields,
        "bookmarks": r.bookmarks.len(),
        "resources": r.resources.len(),
        "has_report_filters": r.filter_config.is_some(),
        "pages": pages,
    }))
}

/// Collects `report.validate` findings.
#[derive(Default)]
struct Issues {
    list: Vec<Value>,
    errors: usize,
    warnings: usize,
}

impl Issues {
    fn push(&mut self, error: bool, mut at: Value, message: String) {
        if error {
            self.errors = self.errors.saturating_add(1);
        } else {
            self.warnings = self.warnings.saturating_add(1);
        }
        if self.list.len() >= MAX_ISSUES {
            return;
        }
        if let Some(o) = at.as_object_mut() {
            o.insert("severity".into(), json!(if error { "error" } else { "warning" }));
            o.insert("message".into(), json!(message));
        }
        self.list.push(at);
    }
}

pub(crate) fn report_validate(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &[])?;
    report_validation(s, true)
}

/// `report.validate` on a shared borrow (also used by [`Session::snapshot`]). `schemas: false`
/// skips the PBIR schema check (the costly part: every document is written and re-validated);
/// the result says which in `schema_checked`.
pub(crate) fn report_validation(s: &Session, schemas: bool) -> Result<Value, String> {
    // Desktop cannot read files whose full path reaches 260 characters (project files only).
    let long_paths = match (&s.source, &s.report) {
        (Some(Source::Project { project, .. }), Some(r)) => {
            crate::paths::project_long_paths(project, s.model.as_ref(), Some(r))
        }
        _ => Vec::new(),
    };
    let model_error = s.model_error.clone();
    let model = s.model.as_ref();
    let r = report_ref(s)?;
    let mut issues = Issues::default();

    let schema_issues = if schemas {
        // Legacy-only keys (a report read from a legacy layout) are not PBIR: check the PBIR form.
        bicraft_pbir::validate_documents(&bicraft_pbir::write_documents(&crate::convert::pbir_form(r).0))
    } else {
        Vec::new()
    };
    issues.errors = issues.errors.saturating_add(schema_issues.len());

    // Structure.
    if r.pages.is_empty() {
        issues.push(
            true,
            json!({}),
            "the report has no page; Power BI needs at least one".into(),
        );
    }
    let mut page_names = HashSet::new();
    for page in &r.pages {
        let at = json!({"page": page.display_name});
        if !page_names.insert(page.name.to_lowercase()) {
            issues.push(true, at.clone(), format!("duplicate page name `{}`", page.name));
        }
        if !(page.width > 0.0 && page.height > 0.0) {
            issues.push(true, at.clone(), "the page has no size".into());
        }
        let mut visual_names = HashSet::new();
        let groups: HashSet<String> = page
            .visuals
            .iter()
            .filter(|v| v.visual_type.is_none())
            .map(|v| v.name.to_lowercase())
            .collect();
        for v in &page.visuals {
            let at = json!({"page": page.display_name, "visual": v.name});
            for i in crate::roles::check_bindings(model, v.visual_type.as_deref(), &v.fields) {
                let mut at = at.clone();
                if let (Some(o), Some(role)) = (at.as_object_mut(), &i.role) {
                    o.insert("role".into(), json!(role));
                }
                issues.push(i.error, at, i.message);
            }
            if !visual_names.insert(v.name.to_lowercase()) {
                issues.push(true, at.clone(), format!("duplicate visual name `{}`", v.name));
            }
            if let Some(g) = &v.parent_group
                && !groups.contains(&g.to_lowercase())
            {
                issues.push(false, at.clone(), format!("its group `{g}` is not on the page"));
            }
            let pos = &v.position;
            if !(pos.width > 0.0 && pos.height > 0.0) {
                issues.push(false, at.clone(), "the visual has no size".into());
            } else if v.parent_group.is_none()
                && (pos.x >= page.width
                    || pos.y >= page.height
                    || pos.x + pos.width <= 0.0
                    || pos.y + pos.height <= 0.0)
            {
                issues.push(false, at, "the visual lies entirely outside the page".into());
            } else if v.parent_group.is_none()
                && let Some(w) = outside_page_warning(page, pos)
            {
                issues.push(false, at, w);
            }
        }
        crate::layout::overlap_warnings(page, |at, message| issues.push(false, at, message));
    }
    if let Some(active) = &r.active_page
        && !r.pages.iter().any(|p| p.name.eq_ignore_ascii_case(active))
    {
        issues.push(
            true,
            json!({}),
            format!("the active page `{active}` does not exist"),
        );
    }

    for lp in &long_paths {
        issues.push(
            true,
            json!({"path": lp.path, "length": lp.length, "limit": lp.limit}),
            format!(
                "this {} path is {} characters long; Power BI Desktop cannot read paths over {} \
                 characters: move the project to a shorter folder or shorten its name",
                lp.kind, lp.length, lp.limit
            ),
        );
    }

    // Fields against the model.
    if let Some(m) = model {
        let mut seen: HashSet<(String, String, bool, String, String)> = HashSet::new();
        for page in &r.pages {
            for v in &page.visuals {
                for b in &v.fields {
                    if let Some(problem) = check_field(m, &b.field) {
                        let (measure, table, name) = match &b.field {
                            FieldRef::Measure { table, measure } => (true, table, measure),
                            FieldRef::Column { table, column }
                            | FieldRef::Aggregation { table, column, .. } => (false, table, column),
                            FieldRef::Other { .. } => continue,
                        };
                        seen.insert((
                            page.display_name.clone(),
                            v.name.clone(),
                            measure,
                            table.to_lowercase(),
                            name.to_lowercase(),
                        ));
                        issues.push(
                            true,
                            json!({"page": page.display_name, "visual": v.name, "role": b.role,
                                   "field": field_text(&b.field)}),
                            problem,
                        );
                    }
                }
            }
        }
        for (loc, doc) in raw_documents_ref(r) {
            let bookmark = loc.part.starts_with("bookmark");
            let mut found = Vec::new();
            find_refs(doc, &mut |site| {
                if site.kind != SiteKind::Table && !site.extension {
                    found.push((site.kind, site.entity.to_string(), site.property.to_string()));
                }
            });
            for (kind, entity, property) in found {
                let key = (
                    loc.page.clone().unwrap_or_default(),
                    loc.visual.clone().unwrap_or_default(),
                    kind == SiteKind::Measure,
                    entity.to_lowercase(),
                    property.to_lowercase(),
                );
                let field = match kind {
                    SiteKind::Measure => FieldRef::Measure {
                        table: entity,
                        measure: property,
                    },
                    _ => FieldRef::Column {
                        table: entity,
                        column: property,
                    },
                };
                if let Some(problem) = check_field(m, &field)
                    && seen.insert(key)
                {
                    let mut at = loc.to_json();
                    if let Some(o) = at.as_object_mut() {
                        o.insert("field".into(), json!(field_text(&field)));
                    }
                    issues.push(!bookmark, at, problem);
                }
            }
        }
    }

    for (error, at, message) in crate::report_measures::validation_issues(model, r) {
        issues.push(error, at, message);
    }
    for (error, at, message) in crate::report_checks::validation_issues(model, r) {
        issues.push(error, at, message);
    }
    for (error, at, message) in crate::visual_states::validation_issues(r) {
        issues.push(error, at, message);
    }
    for (error, at, message) in crate::report_repair::validation_issues(r) {
        issues.push(error, at, message);
    }

    let mut out = json!({
        "valid": issues.errors == 0,
        "error_count": issues.errors,
        "warning_count": issues.warnings,
        "checked_against_model": model.is_some(),
        "schema_checked": schemas,
        "schema_issues": schema_issues,
        "issues": issues.list,
    });
    if model.is_none()
        && let Some(o) = out.as_object_mut()
    {
        o.insert(
            "note".into(),
            json!(match model_error {
                Some(e) => format!("fields were not checked: the model cannot be read ({e})"),
                None => "fields were not checked: no model is open".into(),
            }),
        );
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// page.*
// ---------------------------------------------------------------------------------------------

fn check_display_name_free(r: &Report, name: &str, except: Option<usize>) -> Result<(), String> {
    if let Some((_, other)) = r
        .pages
        .iter()
        .enumerate()
        .find(|(i, p)| Some(*i) != except && (p.display_name.eq_ignore_ascii_case(name)))
    {
        return Err(format!(
            "a page called `{}` already exists ({}); page display names must be unique",
            other.display_name, other.name
        ));
    }
    Ok(())
}

pub(crate) fn page_add(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "display_name",
            "width",
            "height",
            "display_option",
            "index",
            "key",
        ],
    )?;
    let key = opt_str(p, "key")?;
    if let Some(k) = key {
        crate::report_apply::check_key(k)?;
    }
    let display_name = str_param(p, "display_name")?;
    check_name("page", display_name)?;
    let width = check_size("width", opt_num(p, "width")?.unwrap_or(1280.0))?;
    let height = check_size("height", opt_num(p, "height")?.unwrap_or(720.0))?;
    let display_option = match opt_str(p, "display_option")? {
        None => "FitToPage",
        Some(o) => DISPLAY_OPTIONS
            .iter()
            .find(|d| d.eq_ignore_ascii_case(o))
            .copied()
            .ok_or_else(|| {
                format!(
                    "unknown display_option `{o}` (expected {})",
                    DISPLAY_OPTIONS.join(", ")
                )
            })?,
    };
    let index = opt_index(p, "index")?;
    let (_, r) = parts(s)?;
    check_display_name_free(r, display_name, None)?;
    let index = match index {
        None => r.pages.len(),
        Some(i) if i <= r.pages.len() => i,
        Some(i) => return Err(format!("index {i} is past the end ({} pages)", r.pages.len())),
    };
    let taken = |n: &str| r.pages.iter().any(|p| p.name.eq_ignore_ascii_case(n));
    let name = match key {
        Some(k) => {
            let name = crate::report_apply::page_name_for_key(k);
            if taken(&name) {
                return Err(format!("a page with key `{k}` already exists ({name})"));
            }
            name
        }
        None => new_object_name(taken),
    };
    r.pages.insert(
        index,
        Page {
            name: name.clone(),
            display_name: display_name.to_string(),
            display_option: Some(display_option.to_string()),
            width,
            height,
            ..Default::default()
        },
    );
    if r.active_page.is_none() {
        r.active_page = Some(name.clone());
    }
    let active = r.active_page.as_deref() == Some(name.as_str());
    s.report_dirty = true;
    Ok(json!({"page": name, "display_name": display_name, "index": index, "active": active}))
}

pub(crate) fn page_remove(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page"])?;
    let key = str_param(p, "page")?;
    let (_, r) = parts(s)?;
    let i = page_index(r, key)?;
    if r.pages.len() <= 1 {
        return Err("a report needs at least one page: add another page before removing this one".into());
    }
    let name = page_at(r, i)?.name.clone();
    let warnings: Vec<String> = r
        .bookmarks
        .iter()
        .filter(|b| mentions(&b.document, &name, 0))
        .map(|b| format!("bookmark `{}` refers to the removed page", b.name))
        .collect();
    let page = r.pages.remove(i);
    if r.active_page
        .as_deref()
        .is_none_or(|a| a.eq_ignore_ascii_case(&page.name))
    {
        r.active_page = r.pages.first().map(|p| p.name.clone());
    }
    let out = json!({
        "removed": page.name,
        "display_name": page.display_name,
        "visuals_removed": page.visuals.len(),
        "active_page": r.active_page,
        "warnings": warnings,
    });
    s.report_dirty = true;
    Ok(out)
}

pub(crate) fn page_rename(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "display_name"])?;
    let key = str_param(p, "page")?;
    let display_name = str_param(p, "display_name")?;
    check_name("page", display_name)?;
    let (_, r) = parts(s)?;
    let i = page_index(r, key)?;
    check_display_name_free(r, display_name, Some(i))?;
    let page = page_at_mut(r, i)?;
    let previous = std::mem::replace(&mut page.display_name, display_name.to_string());
    let out = json!({"page": page.name, "display_name": display_name, "previous": previous});
    s.report_dirty = true;
    Ok(out)
}

pub(crate) fn page_move(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "index"])?;
    let key = str_param(p, "page")?;
    let to = opt_index(p, "index")?.ok_or("missing integer parameter `index`")?;
    let (_, r) = parts(s)?;
    let from = page_index(r, key)?;
    if to >= r.pages.len() {
        return Err(format!(
            "index {to} is out of range: the report has {} pages (0-based)",
            r.pages.len()
        ));
    }
    let page = r.pages.remove(from);
    let name = page.name.clone();
    r.pages.insert(to, page);
    let order: Vec<&str> = r.pages.iter().map(|p| p.display_name.as_str()).collect();
    let out = json!({"page": name, "from": from, "to": to, "order": order});
    s.report_dirty = true;
    Ok(out)
}

pub(crate) fn page_set_active(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page"])?;
    let key = str_param(p, "page")?;
    let (_, r) = parts(s)?;
    let i = page_index(r, key)?;
    let page = page_at(r, i)?;
    let (name, display) = (page.name.clone(), page.display_name.clone());
    r.active_page = Some(name.clone());
    s.report_dirty = true;
    Ok(json!({"active_page": name, "display_name": display}))
}

/// View › Page view: how the page is scaled (its `displayOption`, kept with the page).
pub(crate) fn page_set_display_option(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "display_option"])?;
    let key = str_param(p, "page")?;
    let o = str_param(p, "display_option")?;
    let option = DISPLAY_OPTIONS
        .iter()
        .find(|d| d.eq_ignore_ascii_case(o))
        .copied()
        .ok_or_else(|| {
            format!(
                "unknown display_option `{o}` (expected {})",
                DISPLAY_OPTIONS.join(", ")
            )
        })?;
    let (_, r) = parts(s)?;
    let i = page_index(r, key)?;
    let page = page_at_mut(r, i)?;
    let previous = page.display_option.replace(option.to_string());
    let changed = previous.as_deref() != Some(option);
    let out = json!({"page": page.name, "display_option": option, "previous": previous, "changed": changed});
    if changed {
        s.report_dirty = true;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// visual.*
// ---------------------------------------------------------------------------------------------

/// `[{role, field}]` → `(role, field text)`.
fn parse_field_list(v: &Value) -> Result<Vec<(String, String)>, String> {
    let items = v
        .as_array()
        .ok_or("`fields` must be an array of {role, field} objects")?;
    if items.len() > MAX_FIELDS {
        return Err(format!("at most {MAX_FIELDS} fields can be bound at once"));
    }
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            if !item.is_object() {
                return Err(format!("fields[{i}] must be an object {{role, field}}"));
            }
            allow(item, &["role", "field"]).map_err(|e| format!("fields[{i}]: {e}"))?;
            let role = str_param(item, "role").map_err(|e| format!("fields[{i}]: {e}"))?;
            check_role(role).map_err(|e| format!("fields[{i}]: {e}"))?;
            let field = str_param(item, "field").map_err(|e| format!("fields[{i}]: {e}"))?;
            Ok((role.to_string(), field.to_string()))
        })
        .collect()
}

/// A warning when `pos` sticks out of `page` by more than [`crate::layout::TOLERANCE`], saying by
/// how much on each side.
pub(crate) fn outside_page_warning(page: &Page, pos: &Position) -> Option<String> {
    let tol = crate::layout::TOLERANCE;
    let sides: Vec<String> = [
        (-pos.x, "left"),
        (-pos.y, "top"),
        (pos.x + pos.width - page.width, "right"),
        (pos.y + pos.height - page.height, "bottom"),
    ]
    .iter()
    .filter(|(by, _)| *by > tol)
    .map(|(by, side)| format!("{} px past the {side} edge", by.ceil()))
    .collect();
    (!sides.is_empty()).then(|| {
        format!(
            "the visual does not fit on page `{}` ({} x {}): it sticks out {}",
            page.display_name,
            page.width,
            page.height,
            sides.join(", ")
        )
    })
}

pub(crate) fn visual_add(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "page",
            "visual_type",
            "x",
            "y",
            "z",
            "width",
            "height",
            "fields",
            "force",
            "key",
        ],
    )?;
    let visual_key = opt_str(p, "key")?;
    if let Some(k) = visual_key {
        crate::report_apply::check_key(k)?;
    }
    let key = str_param(p, "page")?;
    let visual_type = str_param(p, "visual_type")?;
    if visual_type.is_empty()
        || visual_type.len() > MAX_VISUAL_TYPE
        || !visual_type.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(format!(
            "`{visual_type}` is not a visual type like card, clusteredColumnChart, tableEx or slicer"
        ));
    }
    // one of BIcraft's visuals (by name or guid): its visuals carry the guid
    let builtin = crate::builtin_visuals::find(visual_type);
    let visual_type = builtin.map_or(visual_type, |b| b.guid);
    let x = check_coordinate("x", num(p, "x")?)?;
    let y = check_coordinate("y", num(p, "y")?)?;
    let z = opt_num(p, "z")?.map(|z| check_coordinate("z", z)).transpose()?;
    let width = check_size("width", num(p, "width")?)?;
    let height = check_size("height", num(p, "height")?)?;
    let requested = match p.get("fields") {
        None | Some(Value::Null) => Vec::new(),
        Some(v) => parse_field_list(v)?,
    };
    let force = force(p)?;
    let (model, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let mut warnings = Vec::new();
    if builtin.is_none() && !KNOWN_VISUAL_TYPES.contains(&visual_type) {
        warnings.push(format!(
            "`{visual_type}` is not a built-in visual type; it must be a custom visual installed in the report"
        ));
    }
    let measures = crate::report_measures::report_measures(r);
    let mut fields: Vec<FieldBinding> = Vec::with_capacity(requested.len());
    for (role, text) in &requested {
        let own = match builtin {
            None => None,
            Some(b) => match b.role(role) {
                Some(r) => Some(r),
                None => {
                    let names = b.role_names();
                    let message = format!(
                        "{} has no role `{role}` (its roles: {})",
                        b.display_name,
                        names.join(", ")
                    );
                    if !force {
                        return Err(format!("{message}; pass force: true to keep it"));
                    }
                    warnings.push(message);
                    None
                }
            },
        };
        let role = own
            .or_else(|| crate::roles::canonical_role(Some(visual_type), role))
            .map_or_else(|| role.clone(), str::to_string);
        let role = &role;
        let field = resolve_for_binding(model, &measures, text, force, &mut warnings)?;
        if fields
            .iter()
            .any(|b| b.role.eq_ignore_ascii_case(role) && same_field(&b.field, &field))
        {
            return Err(format!("{text} is bound twice to role {role}"));
        }
        fields.push(FieldBinding {
            display_name: None,
            show_as: None,
            role: role.clone(),
            field,
        });
    }
    role_verdict(
        crate::roles::check_bindings(model, Some(visual_type), &fields),
        force,
        &mut warnings,
    )?;
    let page = page_at_mut(r, pi)?;
    let z = z.unwrap_or_else(|| {
        page.visuals
            .iter()
            .map(|v| v.position.z)
            .fold(None, |m: Option<f64>, z| Some(m.map_or(z, |m| m.max(z))))
            .map_or(0.0, |m| (m + 1000.0).min(MAX_SIZE))
    });
    let tab_order = page
        .visuals
        .iter()
        .filter_map(|v| v.position.tab_order)
        .max()
        .map_or(0, |t| t.saturating_add(1000));
    let position = Position {
        x,
        y,
        z,
        width,
        height,
        tab_order: Some(tab_order),
    };
    warnings.extend(outside_page_warning(page, &position));
    let taken = |n: &str| page.visuals.iter().any(|v| v.name.eq_ignore_ascii_case(n));
    let name = match visual_key {
        Some(k) => {
            let name = crate::report_apply::visual_name_for_key(&page.name, k);
            if taken(&name) {
                return Err(format!(
                    "page `{}` already has a visual with key `{k}` ({name})",
                    page.display_name
                ));
            }
            name
        }
        None => new_object_name(taken),
    };
    let mut out = json!({
        "page": page.name,
        "display_name": page.display_name,
        "visual": name,
        "visual_type": visual_type,
        "position": position_json(&position),
        "fields": bindings_json(&fields),
        "warnings": warnings,
    });
    let mut visual = Visual {
        name,
        position,
        visual_type: Some(visual_type.to_string()),
        fields,
        ..Default::default()
    };
    // a Spec visual starts with visual.spec_template's spec for its fields
    if let (Some(kind), Some(o)) = (
        crate::spec_visual::default_spec(&mut visual)?,
        out.as_object_mut(),
    ) {
        o.insert("spec_template".into(), json!(kind));
    }
    // everything was checked: BIcraft's visual goes in with its package (on first use)
    if let Some(b) = builtin {
        let installed = crate::builtin_visuals::ensure(s, b)?;
        if let Some(o) = out.as_object_mut() {
            o.insert("installed".into(), json!(installed));
        }
    }
    let (_, r) = parts(s)?;
    page_at_mut(r, pi)?.visuals.push(visual);
    s.report_dirty = true;
    Ok(out)
}

pub(crate) fn visual_set_position(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "x", "y", "z", "width", "height"])?;
    let key = str_param(p, "page")?;
    let visual = str_param(p, "visual")?;
    let x = opt_num(p, "x")?.map(|v| check_coordinate("x", v)).transpose()?;
    let y = opt_num(p, "y")?.map(|v| check_coordinate("y", v)).transpose()?;
    let z = opt_num(p, "z")?.map(|v| check_coordinate("z", v)).transpose()?;
    let width = opt_num(p, "width")?.map(|v| check_size("width", v)).transpose()?;
    let height = opt_num(p, "height")?
        .map(|v| check_size("height", v))
        .transpose()?;
    if [x, y, z, width, height].iter().all(Option::is_none) {
        return Err("visual.set_position needs at least one of x, y, z, width, height".into());
    }
    let (_, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let page = page_at_mut(r, pi)?;
    let vi = visual_index(page, visual)?;
    let mut pos = page
        .visuals
        .get(vi)
        .map(|v| v.position)
        .ok_or("internal: visual index out of range")?;
    pos.x = x.unwrap_or(pos.x);
    pos.y = y.unwrap_or(pos.y);
    pos.z = z.unwrap_or(pos.z);
    pos.width = width.unwrap_or(pos.width);
    pos.height = height.unwrap_or(pos.height);
    let warnings: Vec<String> = outside_page_warning(page, &pos).into_iter().collect();
    let v = page
        .visuals
        .get_mut(vi)
        .ok_or("internal: visual index out of range")?;
    v.position = pos;
    let out =
        json!({"page": page.name, "visual": v.name, "position": position_json(&pos), "warnings": warnings});
    s.report_dirty = true;
    Ok(out)
}

pub(crate) fn visual_bind(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "role", "field", "index", "force"])?;
    let key = str_param(p, "page")?;
    let visual = str_param(p, "visual")?;
    let role = str_param(p, "role")?;
    check_role(role)?;
    let text = str_param(p, "field")?;
    let index = opt_index(p, "index")?;
    let force = force(p)?;
    let (model, r) = parts(s)?;
    let measures = crate::report_measures::report_measures(r);
    let pi = page_index(r, key)?;
    let page = page_at_mut(r, pi)?;
    let vi = visual_index(page, visual)?;
    let mut warnings = Vec::new();
    let field = resolve_for_binding(model, &measures, text, force, &mut warnings)?;
    let v = page
        .visuals
        .get_mut(vi)
        .ok_or("internal: visual index out of range")?;
    if v.visual_type.is_none() {
        return Err(format!("`{}` is a visual group; groups have no fields", v.name));
    }
    let in_role: Vec<usize> = v
        .fields
        .iter()
        .enumerate()
        .filter(|(_, b)| b.role.eq_ignore_ascii_case(role))
        .map(|(i, _)| i)
        .collect();
    if in_role
        .iter()
        .filter_map(|&i| v.fields.get(i))
        .any(|b| same_field(&b.field, &field))
    {
        return Err(format!("{} is already bound to role {role}", field_text(&field)));
    }
    // Keep the role's spelling when the visual already uses it, else Desktop's.
    let role = in_role
        .first()
        .and_then(|&i| v.fields.get(i))
        .map(|b| b.role.clone())
        .or_else(|| crate::roles::canonical_role(v.visual_type.as_deref(), role).map(str::to_string))
        .unwrap_or_else(|| role.to_string());
    let at = match index {
        Some(i) if i < in_role.len() => in_role.get(i).copied().unwrap_or(v.fields.len()),
        Some(i) if i > in_role.len() => {
            return Err(format!(
                "index {i} is past the end of role {role} ({} fields)",
                in_role.len()
            ));
        }
        _ => in_role.last().map_or(v.fields.len(), |&l| l.saturating_add(1)),
    };
    let text_out = field_text(&field);
    let mut after = v.fields.clone();
    after.insert(
        at,
        FieldBinding {
            display_name: None,
            show_as: None,
            role: role.clone(),
            field,
        },
    );
    // Judged on what the binding adds: issues the visual already had are not this command's.
    let before = crate::roles::check_bindings(model, v.visual_type.as_deref(), &v.fields);
    let added = crate::roles::new_issues(
        &before,
        crate::roles::check_bindings(model, v.visual_type.as_deref(), &after),
    );
    role_verdict(added, force, &mut warnings)?;
    v.fields = after;
    let out = json!({
        "page": page.name,
        "visual": v.name,
        "role": role,
        "field": text_out,
        "fields": bindings_json(&v.fields),
        "warnings": warnings,
    });
    s.report_dirty = true;
    Ok(out)
}

pub(crate) fn visual_unbind(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "role", "field", "index"])?;
    let key = str_param(p, "page")?;
    let visual = str_param(p, "visual")?;
    let role = str_param(p, "role")?;
    let parsed = opt_str(p, "field")?.map(parse_field).transpose()?;
    let index = opt_index(p, "index")?;
    if parsed.is_some() && index.is_some() {
        return Err("give `field` or `index`, not both".into());
    }
    let (_, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let page = page_at_mut(r, pi)?;
    let vi = visual_index(page, visual)?;
    let v = page
        .visuals
        .get_mut(vi)
        .ok_or("internal: visual index out of range")?;
    // the positions in `v.fields` of the role's bindings, then of those that go
    let in_role: Vec<usize> = v
        .fields
        .iter()
        .enumerate()
        .filter(|(_, b)| b.role.eq_ignore_ascii_case(role))
        .map(|(i, _)| i)
        .collect();
    let doomed: Vec<usize> = match index {
        Some(i) => match in_role.get(i) {
            Some(&at) => vec![at],
            None => {
                return Err(format!(
                    "index {i} is past the end of role {role} of visual `{}` ({} fields)",
                    v.name,
                    in_role.len()
                ));
            }
        },
        None => in_role
            .iter()
            .copied()
            .filter(|&i| {
                v.fields
                    .get(i)
                    .is_some_and(|b| parsed.as_ref().is_none_or(|f| binding_matches(&b.field, f)))
            })
            .collect(),
    };
    let removed: Vec<String> = doomed
        .iter()
        .filter_map(|&i| v.fields.get(i))
        .map(|b| field_text(&b.field))
        .collect();
    if removed.is_empty() {
        return Err(match p.get("field").and_then(Value::as_str) {
            Some(f) => format!("{f} is not bound to role {role} of visual `{}`", v.name),
            None => format!("role {role} of visual `{}` has no fields", v.name),
        });
    }
    v.fields = std::mem::take(&mut v.fields)
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !doomed.contains(i))
        .map(|(_, b)| b)
        .collect();
    let out = json!({
        "page": page.name,
        "visual": v.name,
        "role": role,
        "removed": removed,
        "fields": bindings_json(&v.fields),
    });
    s.report_dirty = true;
    Ok(out)
}

pub(crate) fn visual_remove(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "visuals"])?;
    let key = str_param(p, "page")?;
    let visual = str_param(p, "visual")?;
    let more: Vec<&str> = match p.get("visuals") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(list)) if list.len() <= 2000 => list
            .iter()
            .map(|x| x.as_str().ok_or("`visuals` must be a list of visual names"))
            .collect::<Result<_, _>>()?,
        Some(_) => return Err("`visuals` must be a list of at most 2000 visual names".into()),
    };
    let (_, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let page = page_at_mut(r, pi)?;
    // The visuals and, for a group, every member at any depth.
    let mut doomed: HashSet<String> = HashSet::new();
    for name in std::iter::once(visual).chain(more) {
        let vi = visual_index(page, name)?;
        let root = page
            .visuals
            .get(vi)
            .map(|v| v.name.to_lowercase())
            .ok_or("internal: visual index out of range")?;
        doomed.insert(root);
    }
    loop {
        let before = doomed.len();
        for v in &page.visuals {
            if v.parent_group
                .as_ref()
                .is_some_and(|g| doomed.contains(&g.to_lowercase()))
            {
                doomed.insert(v.name.to_lowercase());
            }
        }
        if doomed.len() == before {
            break;
        }
    }
    // A group whose last members go is removed too (and so, in turn, the groups around it): no
    // empty group is left on the page.
    loop {
        let before = doomed.len();
        let emptied: Vec<String> = page
            .visuals
            .iter()
            .filter(|g| g.visual_type.is_none() && !doomed.contains(&g.name.to_lowercase()))
            .filter(|g| {
                let key = g.name.to_lowercase();
                let mut members = page
                    .visuals
                    .iter()
                    .filter(|v| v.parent_group.as_ref().is_some_and(|p| p.to_lowercase() == key))
                    .peekable();
                members.peek().is_some() && members.all(|v| doomed.contains(&v.name.to_lowercase()))
            })
            .map(|g| g.name.to_lowercase())
            .collect();
        doomed.extend(emptied);
        if doomed.len() == before {
            break;
        }
    }
    let removed: Vec<String> = page
        .visuals
        .iter()
        .filter(|v| doomed.contains(&v.name.to_lowercase()))
        .map(|v| v.name.clone())
        .collect();
    // the groups that keep members are refitted to them, as after a move
    let shrunk: HashSet<String> = page
        .visuals
        .iter()
        .filter(|v| doomed.contains(&v.name.to_lowercase()))
        .filter_map(|v| v.parent_group.as_ref().map(|g| g.to_lowercase()))
        .filter(|g| !doomed.contains(g))
        .collect();
    page.visuals.retain(|v| !doomed.contains(&v.name.to_lowercase()));
    let touched: HashSet<String> = page
        .visuals
        .iter()
        .filter(|v| {
            v.parent_group
                .as_ref()
                .is_some_and(|g| shrunk.contains(&g.to_lowercase()))
        })
        .map(|v| v.name.to_lowercase())
        .collect();
    crate::canvas_edit::refit_groups(page, &touched, &HashSet::new());
    let out = json!({"page": page.name, "removed": removed, "remaining_visuals": page.visuals.len()});
    s.report_dirty = true;
    Ok(out)
}
