//! [`Session::snapshot`]: the whole session as one JSON document for the live viewer
//! (`docs/view-snapshot.md`, version 1).
//!
//! A snapshot never fails: anything that cannot be computed becomes an `error` field. Lists are
//! capped by [`SnapshotLimits`]; a capped list sets `"truncated": true` on its parent object.

use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;

use bicraft_model::{Column, Measure, Relationship, SemanticModel, Table};
use bicraft_report::{FieldRef, Page, Report, Visual};
use serde_json::{Map, Value, json};

use crate::commands::{model_validation, source_path};
use crate::images::{self, ReportImages};
use crate::preview::{self, PreviewLimits};
use crate::refs::field_text;
use crate::report::report_validation;
use crate::theme::{self, Theme};
use crate::ui_lang::{self, UiLanguage};
use crate::{Session, Source};

/// Version of the snapshot JSON (`docs/view-snapshot.md`).
pub const SNAPSHOT_VERSION: u32 = 1;

/// Caps on the lists a snapshot carries, so a huge model or report cannot produce a huge page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotLimits {
    pub tables: usize,
    pub columns_per_table: usize,
    pub measures_per_table: usize,
    pub relationships: usize,
    pub pages: usize,
    pub visuals_per_page: usize,
    pub fields_per_visual: usize,
    pub issues_per_visual: usize,
    /// Objects listed in `validation.model.objects`.
    pub validation_objects: usize,
    pub diagnostics_per_object: usize,
    /// Issues listed in `validation.report.issues`.
    pub validation_issues: usize,
    /// Longest DAX expression carried, in characters (longer ones are cut and flagged).
    pub expression_chars: usize,
    /// Largest report (in visuals) whose PBIR schemas are checked: the schema check writes and
    /// re-reads every document, too slow to run after every command on big reports
    /// (`validation.report.schema_checked` says whether it ran; `report.validate` always runs it).
    pub schema_check_visuals: usize,
    /// Rows carried in one visual's `data` (M8.2); longer results set `data.truncated`.
    pub data_rows_per_visual: usize,
    /// Largest table (in rows) the row-by-row evaluator reads: the visuals the storage engine
    /// cannot evaluate fall back to it (`preview::grid`); the storage engine reads every row.
    pub data_table_rows: usize,
    /// Time one snapshot may spend computing visual data that is not cached yet; visuals left
    /// over get `data: null` and are computed by a later snapshot.
    pub data_budget_ms: u64,
}

impl Default for SnapshotLimits {
    fn default() -> Self {
        Self {
            tables: 1000,
            columns_per_table: 1000,
            measures_per_table: 2000,
            relationships: 5000,
            pages: 500,
            visuals_per_page: 1000,
            fields_per_visual: 200,
            issues_per_visual: 50,
            validation_objects: 500,
            diagnostics_per_object: 20,
            validation_issues: 500,
            expression_chars: 20_000,
            schema_check_visuals: 100,
            data_rows_per_visual: 1000,
            data_table_rows: 2_000_000,
            data_budget_ms: 1000,
        }
    }
}

impl Session {
    /// Work to run after a command's answer is sent, off the command's path: it starts the
    /// background jobs that build report-derived aggregations (`docs/storage-engine.md`,
    /// "Aggregations"). Cheap when there is nothing to do; servers call it after each answer.
    pub fn background_work(&self) {
        crate::preview::background_work(self);
    }

    /// The session as a version-1 viewer snapshot (`docs/view-snapshot.md`), with the default
    /// [`SnapshotLimits`]. Never fails: errors become fields.
    pub fn snapshot(&self) -> Value {
        self.snapshot_with(&SnapshotLimits::default())
    }

    /// [`Session::snapshot`] with explicit caps.
    pub fn snapshot_with(&self, limits: &SnapshotLimits) -> Value {
        self.snapshot_view_with(&self.interactions, limits)
    }

    /// [`Session::snapshot`] under a view state of its own instead of the session's
    /// [`Session::interactions`]: a server gives each viewer a private state over one shared
    /// session, so one viewer's slicers never change what another sees. The session is not changed.
    pub fn snapshot_view(&self, view: &crate::Interactions) -> Value {
        self.snapshot_view_with(view, &SnapshotLimits::default())
    }

    /// [`Session::snapshot_view`] with explicit caps.
    pub fn snapshot_view_with(&self, view: &crate::Interactions, limits: &SnapshotLimits) -> Value {
        match catch_unwind(AssertUnwindSafe(|| build(self, view, limits))) {
            Ok(v) => v,
            Err(_) => json!({
                "version": SNAPSHOT_VERSION,
                "revision": self.revision,
                "source": source_json(self),
                "dirty": {"model": self.dirty, "report": self.report_dirty, "queries": crate::dax_queries::unsaved(self)},
                "last_command": self.last_command_now(),
                "ui": self.ui_language_now().snapshot_json(),
                "model": null,
                "report": null,
                "validation": {"model": null, "report": null},
                "error": "internal error while building the snapshot",
            }),
        }
    }
}

fn build(s: &Session, view: &crate::Interactions, limits: &SnapshotLimits) -> Value {
    // this view's View as (a viewer's own state on a server), for every evaluation below
    let _view_as = crate::call_scope::view_as(view.view_as().cloned());
    let model_validation = s.model.as_ref().map(|_| model_validation(s));
    let report_validation = s.report.as_ref().map(|r| {
        let visuals = r
            .pages
            .iter()
            .map(|p| p.visuals.len())
            .fold(0usize, usize::saturating_add);
        report_validation(s, visuals <= limits.schema_check_visuals)
    });

    let model = s
        .model
        .as_ref()
        .map(|m| model_json(m, model_validation.as_ref(), limits))
        .map(|mut v| {
            // where the Model view's cards sit (diagram_layout.rs)
            if let (Some(o), Some(d)) = (v.as_object_mut(), crate::diagram_layout::snapshot(s)) {
                o.insert("diagram".into(), d);
            }
            v
        });
    // the viewer's interaction state (interact.rs): the data are evaluated on one derived
    // report (applied slicer selections, cross-filters), the visuals described from another
    // (pending slicer selections shown)
    let effective = s.report.as_ref().and_then(|r| view.effective(r));
    let ui = s.ui_language_now();
    let report = s.report.as_ref().map(|base| {
        let (data_r, r) = match &effective {
            Some(e) => (&e.data, &e.display),
            None => (base, base),
        };
        let theme = theme::resolve(r);
        let preview = PreviewLimits {
            rows_per_visual: limits.data_rows_per_visual,
            table_rows: preview::STORAGE_TABLE_ROWS,
            fallback_rows: limits.data_table_rows,
            budget: Duration::from_millis(limits.data_budget_ms),
            now: preview::filters::clock(s),
            utc_offset: preview::filters::utc_offset(s),
        };
        // Pause visuals (optimize.rs): a refresh asked for evaluates once
        let refresh = s.preview.take_refresh();
        let on = crate::optimize::paused(base) && !refresh;
        let mut data = preview::report_data(
            s,
            data_r,
            limits.pages,
            limits.visuals_per_page,
            &preview,
            preview::Pause { on, main: true },
        );
        // highlighted visuals: their values under the selection, joined to their full values
        if let Some(e) = effective.as_ref()
            && let Some(h) = e.highlight.as_ref()
        {
            let mut parts = preview::report_data(
                s,
                h,
                limits.pages,
                limits.visuals_per_page,
                &preview,
                preview::Pause { on, main: false },
            );
            parts.retain(|k, _| e.highlight_targets.contains(k));
            crate::interact::merge_highlights(&mut data, &parts);
        }
        // key influencers: BIcraft's estimate only when the viewer or the render asks for it
        let ki_estimate = crate::call_scope::key_influencers_now().unwrap_or(view.key_influencers_estimate());
        let key_influencers = if ki_estimate {
            preview::key_influencers::report_estimates(
                s,
                data_r,
                limits.pages,
                limits.visuals_per_page,
                &preview,
            )
        } else {
            HashMap::new()
        };
        let extras = Extras {
            data,
            key_influencers,
            ki_estimate,
            bound: preview::report_bound(s, data_r, limits.pages, limits.visuals_per_page, &preview),
            theme,
            images: s.report_images(),
            model: s.model.as_ref(),
            ui: &ui,
            captions: display_captions(s),
            session: s,
        };
        report_json(r, report_validation.as_ref(), limits, &extras)
    });

    let mut snap = json!({
        "version": SNAPSHOT_VERSION,
        "revision": s.revision,
        "source": source_json(s),
        "dirty": {"model": s.dirty, "report": s.report_dirty, "queries": crate::dax_queries::unsaved(s)},
        "dax_queries": crate::dax_queries::snapshot(s),
        "last_command": s.last_command_now(),
        "ui": s.ui_language_now().snapshot_json(),
        "display_culture": null,
        "model": model,
        "model_error": s.model_error,
        "report": report,
        "report_error": s.report_error,
        "validation": {
            "model": model_validation.map(|v| model_validation_json(v, limits)),
            "report": report_validation.map(|v| report_validation_json(v, limits)),
        },
        "commands": {"disabled": disabled_commands(s)},
        "bookmarks": crate::bookmarks::bookmarks_view(s),
        "recovery": crate::recovery::snapshot_json(s),
    });
    if let Some(captions) = display_captions(s) {
        captions.apply_snapshot(&mut snap);
    }
    if let Some(r) = s.report.as_ref() {
        view.annotate(r, &mut snap);
        view.annotate_drill(s.model.as_ref(), r, &mut snap);
    }
    snap
}

/// The commands an editor cannot run now, and why ([`Session::command_enabled`]): the app greys
/// their buttons with the reason as tooltip (ROADMAP M11). Read-only callers' refusals come on
/// top in the app, from each command's effect.
fn disabled_commands(s: &Session) -> Map<String, Value> {
    crate::COMMANDS
        .iter()
        .filter_map(|c| {
            s.command_enabled(c.id, crate::Caller::Editor)
                .err()
                .map(|why| (c.id.to_string(), Value::String(why)))
        })
        .collect()
}

/// M10.4: the translations of the session's display culture (`None` when unset, or when the
/// model no longer has that culture: names are shown).
fn display_captions(s: &Session) -> Option<crate::translations::Captions> {
    let (c, m) = (s.display_culture_now()?, s.model.as_ref()?);
    crate::translations::Captions::of(m, &c).ok()
}

fn source_json(s: &Session) -> Value {
    let Some(source) = &s.source else {
        return json!({"path": null, "kind": "none", "format": null, "desktop": s.desktop_target().to_json()});
    };
    let (kind, format) = match source {
        Source::Package { path, .. } => {
            let pbit = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("pbit"));
            // an imported Excel workbook (workbook_import.rs) says so: it is not refreshed or
            // saved in place
            let format = if pbit {
                "pbit"
            } else if crate::workbook_import::is_workbook(path) {
                "xlsx"
            } else {
                "pbix"
            };
            ("package", Some(format))
        }
        Source::Project { project, .. } => (
            "project",
            Some(match project.model_format {
                bicraft_pbip::ModelFormat::Tmdl => "pbip-tmdl",
                bicraft_pbip::ModelFormat::Tmsl => "pbip-tmsl",
                bicraft_pbip::ModelFormat::Missing => "pbip",
            }),
        ),
        Source::Bim { .. } => ("bim", Some("bim")),
    };
    json!({
        "path": source_path(source).display().to_string(),
        "kind": kind,
        "format": format,
        // what the live viewer's "Open in Power BI Desktop" button opens (desktop_open.rs)
        "desktop": s.desktop_target().to_json(),
        // a thin report's live connection (docs/gaps.md G12)
        "live_connection": crate::commands::thin_package(Some(source)).flatten(),
    })
}

/// Sets `"truncated": true` on `o` when `cut`.
fn flag(o: &mut Map<String, Value>, cut: bool) {
    if cut {
        o.insert("truncated".into(), Value::Bool(true));
    }
}

/// At most `max` characters of `text`, and whether it was cut.
fn cap_text(text: &str, max: usize) -> (&str, bool) {
    match text.char_indices().nth(max) {
        Some((i, _)) => (text.get(..i).unwrap_or(text), true),
        None => (text, false),
    }
}

// ---------------------------------------------------------------------------------------------
// model
// ---------------------------------------------------------------------------------------------

fn model_json(
    m: &SemanticModel,
    validation: Option<&Result<Value, String>>,
    limits: &SnapshotLimits,
) -> Value {
    // Measures that failed validation (lowercased table and name); `None` when unknown.
    let invalid: Option<HashSet<(String, String)>> = match validation {
        Some(Ok(v)) => Some(
            v.get("objects_with_diagnostics")
                .and_then(Value::as_array)
                .map(|objects| {
                    objects
                        .iter()
                        .filter(|o| o.get("kind").and_then(Value::as_str) == Some("measure"))
                        .filter(|o| o.get("valid") == Some(&Value::Bool(false)))
                        .map(|o| {
                            let text =
                                |k: &str| o.get(k).and_then(Value::as_str).unwrap_or("").to_lowercase();
                            (text("table"), text("name"))
                        })
                        .collect()
                })
                .unwrap_or_default(),
        ),
        _ => None,
    };
    // the Properties pane's Synonyms (the model culture's Q&A schema)
    let synonyms = crate::synonyms::all(m);
    let tables: Vec<Value> = m
        .tables
        .iter()
        .take(limits.tables)
        .map(|t| table_json(m, t, invalid.as_ref(), &synonyms, limits))
        .collect();
    let relationships: Vec<Value> = m
        .relationships
        .iter()
        .take(limits.relationships)
        .map(relationship_json)
        .collect();
    let mut o = Map::new();
    o.insert("name".into(), json!(m.name));
    o.insert("culture".into(), json!(m.culture));
    o.insert("tables".into(), Value::Array(tables));
    o.insert("relationships".into(), Value::Array(relationships));
    // the forms' pickers (names only)
    o.insert(
        "expressions".into(),
        json!(
            m.expressions
                .iter()
                .take(NAMES)
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>()
        ),
    );
    o.insert(
        "roles".into(),
        json!(
            m.roles
                .iter()
                .take(NAMES)
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>()
        ),
    );
    o.insert("cultures".into(), json!(names(m.extra.get("cultures"))));
    // the Model view's Properties pane of a shared expression, a culture, a data source
    o.insert("expression_details".into(), expression_details(m, limits));
    o.insert("culture_details".into(), culture_details(m));
    o.insert("data_sources".into(), data_sources(m));
    // the Model explorer's Perspectives (names only)
    o.insert("perspectives".into(), json!(names(m.extra.get("perspectives"))));
    // the Model explorer's Functions (the DAX user-defined functions) and their edit form
    o.insert("functions".into(), functions_json(m));
    // the Manage roles dialog: each role's row filters and object-level security
    o.insert("security".into(), security_json(m));
    // the Perspectives dialog and the Model view's perspective filter: each one's members
    o.insert("perspective_members".into(), perspective_members(m));
    // the Incremental refresh dialog: the RangeStart and RangeEnd parameters
    o.insert("incremental_refresh".into(), crate::refresh_policy::parameters(m));
    flag(
        &mut o,
        m.tables.len() > limits.tables || m.relationships.len() > limits.relationships,
    );
    Value::Object(o)
}

fn table_json(
    m: &SemanticModel,
    t: &Table,
    invalid: Option<&HashSet<(String, String)>>,
    synonyms: &std::collections::HashMap<crate::synonyms::Key, Vec<String>>,
    limits: &SnapshotLimits,
) -> Value {
    let table_key = t.name.to_lowercase();
    let terms = |p: Option<&str>| synonyms.get(&(table_key.clone(), p.map(str::to_lowercase)));
    let visible: Vec<&Column> = t.columns.iter().filter(|c| !c.is_row_number()).collect();
    let columns: Vec<Value> = visible
        .iter()
        .take(limits.columns_per_table)
        .map(|c| {
            let mut v = json!({
                "name": c.name,
                "data_type": c.data_type.as_tmsl().unwrap_or("unknown"),
                "format_string": c.format_string,
                "calculated": c.expression.is_some(),
                "hidden": c.is_hidden,
                "display_folder": c.display_folder,
                "summarize_by": c.summarize_by,
                "data_category": c.data_category,
                "description": c.description,
                "sort_by_column": c.sort_by_column,
            });
            if c.extra.get("isKey").and_then(Value::as_bool) == Some(true) {
                v["is_key"] = json!(true);
            }
            if let Some(list) = terms(Some(&c.name)) {
                v["synonyms"] = json!(list);
            }
            // a group column (`column.group`): what Edit groups starts from
            if let Some(g) = crate::groups::column_definition(c) {
                v["group"] = g;
            }
            v
        })
        .collect();
    let measures: Vec<Value> = t
        .measures
        .iter()
        .take(limits.measures_per_table)
        .map(|x| {
            let mut v = measure_json(x, &table_key, invalid, limits);
            if let Some(list) = terms(Some(&x.name)) {
                v["synonyms"] = json!(list);
            }
            v
        })
        .collect();
    let mut o = Map::new();
    o.insert("name".into(), json!(t.name));
    if let Some(list) = terms(None) {
        o.insert("synonyms".into(), json!(list));
    }
    o.insert("hidden".into(), json!(t.is_hidden));
    o.insert("description".into(), json!(t.description));
    // a calculated table (the formula bar edits its DAX)
    o.insert(
        "calculated".into(),
        json!(bicraft_dax::deps::calculated_table_expression(t).is_some()),
    );
    // Import, DirectQuery, Dual, or hybrid (an incremental refresh policy with real time)
    o.insert(
        "storage_mode".into(),
        json!(crate::storage_mode::table_mode(m, t)),
    );
    // the Incremental refresh dialog: the table's policy, and whether its query filters on
    // RangeStart and RangeEnd
    if let Some(p) = crate::refresh_policy::summary(t) {
        o.insert("refresh_policy".into(), p);
    }
    o.insert(
        "range_filtered".into(),
        json!(crate::refresh_policy::range_filtered(m, t)),
    );
    o.insert("columns".into(), Value::Array(columns));
    o.insert("measures".into(), Value::Array(measures));
    // the forms' pickers (names only)
    let hierarchies = names(t.extra.get("hierarchies"));
    if !hierarchies.is_empty() {
        o.insert("hierarchies".into(), json!(hierarchies));
        // the Data pane's tree: each hierarchy's levels (the column each one shows)
        o.insert(
            "hierarchy_levels".into(),
            hierarchy_levels(t.extra.get("hierarchies")),
        );
    }
    o.insert(
        "partitions".into(),
        json!(
            t.partitions
                .iter()
                .take(NAMES)
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>()
        ),
    );
    // Edit data: the table was typed in (Enter data or a #table literal)
    if crate::get_data::is_entered_table(t) {
        o.insert("entered_data".into(), json!(true));
    }
    if let Some(g) = t.extra.get("calculationGroup").and_then(Value::as_object) {
        // in ordinal order (the order the group applies and lists them; items without an
        // ordinal last, in the file's order)
        let mut items: Vec<&Value> = g
            .get("calculationItems")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .collect();
        items.sort_by_key(|x| x.get("ordinal").and_then(Value::as_i64).unwrap_or(i64::MAX));
        // the Properties pane of a calculation item: its DAX (clipped as measures' are)
        let details: Vec<Value> = items
            .iter()
            .filter(|v| v.get("name").is_some_and(Value::is_string))
            .take(NAMES)
            .map(|v| {
                let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("");
                let expression = lines_text(v.get("expression"));
                let (expression, cut) = cap_text(&expression, limits.expression_chars);
                // TMSL nests the format string expression as {expression}
                let format = lines_text(
                    v.get("formatStringDefinition")
                        .map(|f| f.get("expression").unwrap_or(f)),
                );
                let (format, format_cut) = cap_text(&format, limits.expression_chars);
                let mut d = json!({
                    "name": text("name"),
                    "expression": expression,
                    "format_string_expression": format,
                    "description": text("description"),
                    "ordinal": v.get("ordinal").and_then(Value::as_i64),
                });
                if cut || format_cut {
                    d["truncated"] = json!(true);
                }
                d
            })
            .collect();
        let items: Vec<&str> = items
            .iter()
            .filter_map(|v| v.get("name").and_then(Value::as_str))
            .take(NAMES)
            .collect();
        o.insert("calculation_items".into(), json!(items));
        o.insert("calculation_item_details".into(), Value::Array(details));
    }
    flag(
        &mut o,
        visible.len() > limits.columns_per_table || t.measures.len() > limits.measures_per_table,
    );
    Value::Object(o)
}

/// Most names of one kind a snapshot lists for the forms' pickers.
const NAMES: usize = 1000;
/// Most `visualInteractions` rules a page lists.
const MAX_INTERACTION_RULES: usize = 10_000;

/// The `name`s of a list of objects (at most [`NAMES`]).
/// The roles' table permissions: `[{name, model_permission, tables: [{table, filter_expression,
/// metadata_permission, columns: [{name, metadata_permission}]}]}]`, at most [`NAMES`] of each.
fn security_json(m: &SemanticModel) -> Value {
    let text = |v: Option<&Value>| v.and_then(Value::as_str).map(str::to_string);
    Value::Array(
        m.roles
            .iter()
            .take(NAMES)
            .map(|r| {
                let tables: Vec<Value> = r
                    .table_permissions
                    .iter()
                    .take(NAMES)
                    .map(|tp| {
                        let columns: Vec<Value> = tp
                            .extra
                            .get("columnPermissions")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .take(NAMES)
                            .map(|c| {
                                json!({
                                    "name": text(c.get("name")),
                                    "metadata_permission": text(c.get("metadataPermission")),
                                })
                            })
                            .collect();
                        json!({
                            "table": tp.table,
                            "filter_expression": tp.filter_expression,
                            "metadata_permission": text(tp.extra.get("metadataPermission")),
                            "columns": columns,
                        })
                    })
                    .collect();
                json!({
                    "name": r.name,
                    "model_permission": r.model_permission,
                    "description": text(r.extra.get("description")),
                    "tables": tables,
                })
            })
            .collect(),
    )
}

/// The perspectives' members: `[{name, tables: [{name, columns, measures, hierarchies}]}]`
/// (names; a TMSL `includeAll` table lists every member it has now), at most [`NAMES`] of each.
fn perspective_members(m: &SemanticModel) -> Value {
    let persp = m.extra.get("perspectives").and_then(Value::as_array);
    Value::Array(
        persp
            .into_iter()
            .flatten()
            .take(NAMES)
            .map(|p| {
                let tables: Vec<Value> = p
                    .get("tables")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .take(NAMES)
                    .map(|t| {
                        let name = t.get("name").and_then(Value::as_str).unwrap_or_default();
                        let all = t.get("includeAll").and_then(Value::as_bool) == Some(true);
                        let model_table = m.tables.iter().find(|x| x.name.eq_ignore_ascii_case(name));
                        match (all, model_table) {
                            (true, Some(mt)) => json!({
                                "name": name,
                                "columns": mt.columns.iter().filter(|c| !c.is_row_number()).take(NAMES).map(|c| c.name.as_str()).collect::<Vec<_>>(),
                                "measures": mt.measures.iter().take(NAMES).map(|x| x.name.as_str()).collect::<Vec<_>>(),
                                "hierarchies": names(mt.extra.get("hierarchies")),
                            }),
                            _ => json!({
                                "name": name,
                                "columns": names(t.get("columns")),
                                "measures": names(t.get("measures")),
                                "hierarchies": names(t.get("hierarchies")),
                            }),
                        }
                    })
                    .collect();
                json!({"name": p.get("name").and_then(Value::as_str), "tables": tables})
            })
            .collect(),
    )
}

/// The model's DAX user-defined functions: `[{name, expression, description, hidden}]`, at most
/// [`NAMES`], each definition clipped to [`FUNCTION_TEXT`] bytes.
fn functions_json(m: &SemanticModel) -> Value {
    let list = m.extra.get("functions").and_then(Value::as_array);
    Value::Array(
        list.into_iter()
            .flatten()
            .filter_map(|f| {
                let name = f.get("name").and_then(Value::as_str)?;
                let mut expr = bicraft_dax::udf::expression_text(f.get("expression").unwrap_or(&Value::Null))
                    .unwrap_or_default();
                if expr.len() > FUNCTION_TEXT {
                    let mut end = FUNCTION_TEXT;
                    while !expr.is_char_boundary(end) {
                        end = end.saturating_sub(1);
                    }
                    expr.truncate(end);
                }
                Some(json!({
                    "name": name,
                    "expression": expr,
                    "description": f.get("description").and_then(Value::as_str).unwrap_or(""),
                    "hidden": f.get("isHidden").and_then(Value::as_bool).unwrap_or(false),
                }))
            })
            .take(NAMES)
            .collect(),
    )
}

/// The text of a TMSL expression property: a string, or an array of lines (empty otherwise).
fn lines_text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(lines)) => lines
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// The shared expressions: `[{name, kind: query|parameter, expression (clipped as measures'),
/// description, query_group}]`, at most [`NAMES`]. A parameter is an M expression with the
/// `IsParameterQuery=true` meta record Desktop writes (docs/formats/model-objects.md).
fn expression_details(m: &SemanticModel, limits: &SnapshotLimits) -> Value {
    Value::Array(
        m.expressions
            .iter()
            .take(NAMES)
            .map(|e| {
                let compact: String = e.expression.chars().filter(|c| !c.is_whitespace()).collect();
                let parameter = compact.contains("IsParameterQuery=true");
                let (expression, cut) = cap_text(&e.expression, limits.expression_chars);
                let mut v = json!({
                    "name": e.name,
                    "kind": if parameter { "parameter" } else { "query" },
                    "expression": expression,
                    "description": e.description,
                    "query_group": e.extra.get("queryGroup").and_then(Value::as_str),
                });
                if cut {
                    v["truncated"] = json!(true);
                }
                v
            })
            .collect(),
    )
}

/// The cultures: `[{name, model (the model's own culture), linguistic_metadata (it holds a Q&A
/// schema)}]`, at most [`NAMES`].
fn culture_details(m: &SemanticModel) -> Value {
    let model_culture = m.culture.as_deref().unwrap_or("");
    Value::Array(
        m.extra
            .get("cultures")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|c| {
                let name = c.get("name").and_then(Value::as_str)?;
                Some(json!({
                    "name": name,
                    "model": name.eq_ignore_ascii_case(model_culture),
                    "linguistic_metadata": c.get("linguisticMetadata").is_some(),
                }))
            })
            .take(NAMES)
            .collect(),
    )
}

/// The model's data sources (TMSL `dataSources`: legacy provider and structured sources):
/// `[{name, type, provider, description}]`, at most [`NAMES`]. Connection strings, accounts and
/// credentials are left out (they may hold secrets).
fn data_sources(m: &SemanticModel) -> Value {
    Value::Array(
        m.extra
            .get("dataSources")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|d| {
                let text = |k: &str| d.get(k).and_then(Value::as_str);
                let name = text("name")?;
                Some(json!({
                    "name": name,
                    "type": text("type").unwrap_or("provider"),
                    "provider": text("provider"),
                    "protocol": d.pointer("/connectionDetails/protocol").and_then(Value::as_str),
                    "description": text("description"),
                }))
            })
            .take(NAMES)
            .collect(),
    )
}

/// Longest function definition the snapshot carries, in bytes.
const FUNCTION_TEXT: usize = 64 * 1024;

fn names(list: Option<&Value>) -> Vec<&str> {
    list.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| v.get("name").and_then(Value::as_str))
        .take(NAMES)
        .collect()
}

/// Most levels listed per hierarchy.
const LEVELS: usize = 64;

/// A table's hierarchies with their levels (TMSL `hierarchies[].levels[]`, in ordinal order):
/// `[{name, hidden, display_folder, levels: [{name, column}]}]`, at most [`NAMES`].
fn hierarchy_levels(list: Option<&Value>) -> Value {
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    let out: Vec<Value> = list
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|h| {
            let name = text(h, "name")?;
            let mut levels: Vec<(i64, Value)> = h
                .get("levels")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(LEVELS)
                .enumerate()
                .filter_map(|(i, l)| {
                    let n = text(l, "name")?;
                    let column = text(l, "column").unwrap_or_else(|| n.clone());
                    let ordinal = l
                        .get("ordinal")
                        .and_then(Value::as_i64)
                        .unwrap_or_else(|| i64::try_from(i).unwrap_or(i64::MAX));
                    Some((ordinal, json!({"name": n, "column": column})))
                })
                .collect();
            levels.sort_by_key(|(o, _)| *o);
            Some(json!({
                "name": name,
                "hidden": h.get("isHidden").and_then(Value::as_bool).unwrap_or(false),
                "display_folder": text(h, "displayFolder"),
                "description": text(h, "description"),
                "levels": levels.into_iter().map(|(_, l)| l).collect::<Vec<_>>(),
            }))
        })
        .take(NAMES)
        .collect();
    Value::Array(out)
}

fn measure_json(
    x: &Measure,
    table_key: &str,
    invalid: Option<&HashSet<(String, String)>>,
    limits: &SnapshotLimits,
) -> Value {
    let (expression, cut) = cap_text(&x.expression, limits.expression_chars);
    let valid = invalid.map(|set| !set.contains(&(table_key.to_string(), x.name.to_lowercase())));
    let mut v = json!({
        "name": x.name,
        "expression": expression,
        "format_string": x.format_string,
        "hidden": x.is_hidden,
        "display_folder": x.display_folder,
        "valid": valid,
        "description": x.description,
    });
    if cut && let Some(o) = v.as_object_mut() {
        o.insert("expression_truncated".into(), Value::Bool(true));
    }
    v
}

fn relationship_json(r: &Relationship) -> Value {
    let cross_filter = match r.cross_filtering_behavior.as_deref() {
        Some(b) if b.eq_ignore_ascii_case("bothDirections") => "both",
        Some(b) if b.eq_ignore_ascii_case("automatic") => "automatic",
        _ => "single",
    };
    json!({
        "name": r.name,
        "from_table": r.from_table,
        "from_column": r.from_column,
        "to_table": r.to_table,
        "to_column": r.to_column,
        "active": r.is_active,
        "cross_filter": cross_filter,
        "from_cardinality": r.from_cardinality.as_deref().unwrap_or("many"),
        "to_cardinality": r.to_cardinality.as_deref().unwrap_or("one"),
        "security_filtering": match r.extra.get("securityFilteringBehavior").and_then(Value::as_str) {
            Some(b) if b.eq_ignore_ascii_case("bothDirections") => "both",
            Some(b) if b.eq_ignore_ascii_case("none") => "none",
            _ => "single",
        },
        "assume_referential_integrity": r.extra.get("relyOnReferentialIntegrity").and_then(Value::as_bool) == Some(true),
    })
}

fn model_validation_json(v: Result<Value, String>, limits: &SnapshotLimits) -> Value {
    let v = match v {
        Ok(v) => v,
        Err(e) => return json!({"error": e}),
    };
    let all = v
        .get("objects_with_diagnostics")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let objects: Vec<Value> = all
        .iter()
        .take(limits.validation_objects)
        .map(|o| {
            let diags = o
                .get("diagnostics")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let mut entry = Map::new();
            for k in ["table", "kind", "name", "valid", "role"] {
                if let Some(x) = o.get(k) {
                    entry.insert(k.into(), x.clone());
                }
            }
            entry.insert(
                "diagnostics".into(),
                Value::Array(
                    diags
                        .iter()
                        .take(limits.diagnostics_per_object)
                        .cloned()
                        .collect(),
                ),
            );
            flag(&mut entry, diags.len() > limits.diagnostics_per_object);
            Value::Object(entry)
        })
        .collect();
    let mut o = Map::new();
    o.insert("valid".into(), v.get("valid").cloned().unwrap_or(Value::Null));
    o.insert(
        "invalid_count".into(),
        v.get("invalid_count").cloned().unwrap_or(Value::Null),
    );
    o.insert(
        "checked_count".into(),
        v.get("checked_count").cloned().unwrap_or(Value::Null),
    );
    o.insert("objects".into(), Value::Array(objects));
    o.insert(
        "cycles".into(),
        v.get("cycles").cloned().unwrap_or_else(|| json!([])),
    );
    flag(&mut o, all.len() > limits.validation_objects);
    Value::Object(o)
}

// ---------------------------------------------------------------------------------------------
// report
// ---------------------------------------------------------------------------------------------

/// `report.validate` issues grouped by (page display name, visual name); page-level issues
/// (no visual) use an empty visual name.
type IssueMap<'a> = HashMap<(&'a str, &'a str), Vec<Value>>;

/// M8.2 additions: the resolved theme and the visuals' data, keyed by (page name, visual name).
struct Extras<'m> {
    theme: Option<Theme>,
    /// Wallpaper, backgrounds and image visuals (`images.rs`).
    images: Option<std::sync::Arc<ReportImages>>,
    data: HashMap<(String, String), Value>,
    /// Values of measure-bound properties (`bound_values.rs`), keyed like `data`.
    bound: HashMap<(String, String), Value>,
    /// For the visual details that name fields (`snapshot_visual.rs`).
    model: Option<&'m SemanticModel>,
    /// Language of the generated text (`auto_title`, field `label`; M10.2).
    ui: &'m UiLanguage,
    /// Translated captions of the display culture (M10.4), used in field labels and auto titles.
    captions: Option<crate::translations::Captions>,
    /// For range slicers' exact domains (`snapshot_visual.rs`).
    session: &'m Session,
    /// Estimates of the key influencers visuals, keyed like `data` (`preview_key_influencers.rs`).
    key_influencers: HashMap<(String, String), Value>,
    /// Whether key influencers visuals show the estimate.
    ki_estimate: bool,
}

fn issue_map(validation: Option<&Result<Value, String>>) -> IssueMap<'_> {
    let mut map: IssueMap<'_> = HashMap::new();
    let Some(Ok(v)) = validation else {
        return map;
    };
    let Some(issues) = v.get("issues").and_then(Value::as_array) else {
        return map;
    };
    for i in issues {
        let Some(page) = i.get("page").and_then(Value::as_str) else {
            continue;
        };
        let visual = i.get("visual").and_then(Value::as_str).unwrap_or("");
        let mut message = i.get("message").and_then(Value::as_str).unwrap_or("").to_string();
        if let Some(field) = i.get("field").and_then(Value::as_str) {
            message = format!("{field}: {message}");
        }
        map.entry((page, visual)).or_default().push(json!({
            "severity": i.get("severity").cloned().unwrap_or(Value::Null),
            "message": message,
        }));
    }
    map
}

fn report_json(
    r: &Report,
    validation: Option<&Result<Value, String>>,
    limits: &SnapshotLimits,
    extras: &Extras<'_>,
) -> Value {
    let issues = issue_map(validation);
    let pages: Vec<Value> = r
        .pages
        .iter()
        .take(limits.pages)
        .map(|p| page_json(r, p, &issues, limits, extras))
        .collect();
    let active = r
        .active_page
        .clone()
        .or_else(|| r.pages.first().map(|p| p.name.clone()));
    let mut o = Map::new();
    o.insert("active_page".into(), json!(active));
    o.insert("theme".into(), theme::theme_json(extras.theme.as_ref()));
    o.insert("pages".into(), Value::Array(pages));
    // the forms' pickers: the report's own measures (names only)
    let measures: Vec<Value> = crate::report_measures::report_measures(r)
        .iter()
        .take(NAMES)
        .map(|m| json!({"table": m.entity, "name": m.name}))
        .collect();
    o.insert("measures".into(), Value::Array(measures));
    // the report settings the interactions follow (docs/view-snapshot.md, "Interactions")
    // and the other report settings (`report.settings`, report_settings.rs)
    let mut settings = crate::report_settings::settings_json(r);
    // what a visual's Export data may write (`visual.export_data`)
    settings.insert(
        "export_data_mode".into(),
        json!(crate::export_data::mode_of(r).as_str()),
    );
    o.insert("settings".into(), Value::Object(settings));
    o.insert(
        "export_data_mode".into(),
        json!(crate::export_data::mode_of(r).as_str()),
    );
    // the Build pane's type picker: the built-in types that take fields
    o.insert("visual_types".into(), visual_types());
    // and the report's own custom visuals, with their icons (docs/view-snapshot.md)
    o.insert(
        "custom_visuals".into(),
        images::custom_visuals_json(extras.images.as_deref()),
    );
    // BIcraft's own visuals, embedded or not (ROADMAP M12.4)
    o.insert(
        "bicraft_visuals".into(),
        images::bicraft_visuals_json(extras.images.as_deref()),
    );
    // the Filters pane: filters on all pages (docs/view-snapshot.md, "Filters pane")
    o.insert(
        "filters".into(),
        crate::filter_edit::cards(r.filter_config.as_ref()),
    );
    o.insert(
        "filter_sort".into(),
        json!(crate::filter_edit::sort_mode(r.filter_config.as_ref())),
    );
    flag(&mut o, r.pages.len() > limits.pages);
    Value::Object(o)
}

fn page_json(
    r: &Report,
    p: &Page,
    issues: &IssueMap<'_>,
    limits: &SnapshotLimits,
    extras: &Extras<'_>,
) -> Value {
    let visuals: Vec<Value> = p
        .visuals
        .iter()
        .take(limits.visuals_per_page)
        .map(|v| {
            let found = issues
                .get(&(p.display_name.as_str(), v.name.as_str()))
                .map(Vec::as_slice)
                .unwrap_or_default();
            let mut o = visual_json(v, found, limits, extras.ui, extras.captions.as_ref());
            if let Some(o) = o.as_object_mut() {
                // series formats index the fields of the displayed drill level, as the data
                let drilled = preview::at_drill_level(v);
                let fv = drilled.as_ref().unwrap_or(v);
                o.insert("format".into(), theme::visual_format(fv, extras.theme.as_ref()));
                o.insert(
                    "data".into(),
                    extras
                        .data
                        .get(&(p.name.clone(), v.name.clone()))
                        .cloned()
                        .unwrap_or(Value::Null),
                );
                let st = bicraft_render::data_state_of(
                    o.get("fields").unwrap_or(&Value::Null),
                    o.get("data").unwrap_or(&Value::Null),
                );
                let mut ds = json!({"state": st.kind.as_str()});
                crate::render::put_counts(&mut ds, &st);
                o.insert("data_state".into(), ds);
                images::visual_fields(o, p, v, extras.images.as_deref());
                details::add(o, v, extras.model);
                // a shape map's custom map, decoded (docs/formats/maps.md)
                if let Some(m) = crate::shape_maps::snapshot(r, v) {
                    o.insert("shape_map".into(), m);
                }
                crate::bound_values::apply(
                    o,
                    v,
                    extras.bound.get(&(p.name.clone(), v.name.clone())),
                    extras.model,
                );
                details::add_domain(o, v, extras.session);
                // the phone layout's place and, when it formats the visual, its format
                // (docs/view-snapshot.md, "Mobile layout")
                if let Some((mut m, merged)) = crate::visual_extras::mobile_snapshot(r, &p.name, v) {
                    if let (Some(mv), Some(mo)) = (merged, m.as_object_mut()) {
                        let fv = preview::at_drill_level(&mv).unwrap_or(mv);
                        let mut f = Map::new();
                        f.insert("format".into(), theme::visual_format(&fv, extras.theme.as_ref()));
                        details::add(&mut f, &fv, extras.model);
                        mo.insert("format".into(), f.remove("format").unwrap_or(Value::Null));
                        mo.insert("title".into(), json!(visual_title(&fv)));
                    }
                    o.insert("mobile".into(), m);
                }
                if preview::key_influencers::is_key_influencers(v) {
                    let mut ki = preview::key_influencers::describe(v, extras.model, extras.ki_estimate);
                    if extras.ki_estimate
                        && let Some(m) = ki.as_object_mut()
                    {
                        let e = extras.key_influencers.get(&(p.name.clone(), v.name.clone()));
                        m.insert(
                            "estimate".into(),
                            e.cloned().unwrap_or_else(
                                || json!({"status": "pending", "reason": "not computed yet (time budget)"}),
                            ),
                        );
                    }
                    o.insert("key_influencers".into(), ki);
                }
            }
            o
        })
        .collect();
    let page_issues = issues
        .get(&(p.display_name.as_str(), ""))
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut o = Map::new();
    o.insert("name".into(), json!(p.name));
    o.insert("display_name".into(), json!(p.display_name));
    o.insert("width".into(), json!(p.width));
    o.insert("height".into(), json!(p.height));
    o.insert(
        "hidden".into(),
        json!(p.visibility.as_deref() == Some("HiddenInViewMode")),
    );
    // `page.type` as page.set_type names it (the app's page tabs show and keep it)
    let page_type = match p.extra.get("type").and_then(Value::as_str) {
        Some("Tooltip") => "tooltip",
        Some("Drillthrough") => "drillthrough",
        _ => "report",
    };
    o.insert("page_type".into(), json!(page_type));
    o.insert(
        "background".into(),
        theme::page_background(p, extras.theme.as_ref()),
    );
    images::page_fields(&mut o, p, extras.theme.as_ref(), extras.images.as_deref());
    // page format (page.format): the size's name and the vertical alignment when set
    o.insert("size".into(), json!(crate::rich::size_name(p)));
    if let Some(a) = crate::rich::vertical_alignment(p) {
        o.insert("vertical_alignment".into(), json!(a));
    }
    // View › Page view (page.set_display_option): how the canvas scales the page
    if let Some(d) = &p.display_option {
        o.insert("display_option".into(), json!(d));
    }
    o.insert(
        "filters".into(),
        crate::filter_edit::cards(p.filter_config.as_ref()),
    );
    o.insert(
        "filter_sort".into(),
        json!(crate::filter_edit::sort_mode(p.filter_config.as_ref())),
    );
    // the Filters pane's formatting (page.format_filter_pane), the theme's defaults included
    if let Some(f) = crate::filter_pane_format::snapshot(p, extras.theme.as_ref()) {
        o.insert("filter_pane_format".into(), f);
    }
    o.insert("visuals".into(), Value::Array(visuals));
    // the Edit interactions mode: the page's explicit rules (docs/view-snapshot.md, "Interactions")
    o.insert(
        "interactions".into(),
        crate::interact::page_rules(p, MAX_INTERACTION_RULES),
    );
    o.insert(
        "issues".into(),
        Value::Array(
            page_issues
                .iter()
                .take(limits.issues_per_visual)
                .cloned()
                .collect(),
        ),
    );
    flag(
        &mut o,
        p.visuals.len() > limits.visuals_per_page || page_issues.len() > limits.issues_per_visual,
    );
    Value::Object(o)
}

fn visual_json(
    v: &Visual,
    issues: &[Value],
    limits: &SnapshotLimits,
    ui: &UiLanguage,
    captions: Option<&crate::translations::Captions>,
) -> Value {
    let pos = &v.position;
    // Labels and the auto title name objects by their translated captions (M10.4).
    let translated = captions.and_then(|c| c.translate_visual(v));
    let titled = translated.as_ref().unwrap_or(v);
    let labels = ui_lang::field_labels(ui, titled);
    let hidden = crate::visual_calcs::hidden_fields(v);
    let refs = crate::visual_calcs::field_names(v);
    let formats = {
        let q = bicraft_report::query::effective(v);
        let f = bicraft_report::query::projection_formats(q.as_ref());
        if f.len() == v.fields.len() { f } else { Vec::new() }
    };
    let mut sparkline_index = 0usize;
    let fields: Vec<Value> = v
        .fields
        .iter()
        .zip(&labels)
        .take(limits.fields_per_visual)
        .enumerate()
        .map(|(i, (b, label))| {
            let (kind, table, name, function) = match &b.field {
                FieldRef::Column { table, column } => ("column", Some(table), Some(column), None),
                FieldRef::Measure { table, measure } => ("measure", Some(table), Some(measure), None),
                FieldRef::Aggregation {
                    table,
                    column,
                    function,
                } => ("aggregation", Some(table), Some(column), Some(function)),
                FieldRef::Other { .. } => ("other", None, None, None),
            };
            let level = match &b.field {
                FieldRef::Other { raw } => bicraft_report::query::hierarchy_level(raw),
                _ => None,
            };
            let mut f = json!({
                "role": b.role,
                "kind": kind,
                "table": table,
                "name": name,
                "function": function,
                "text": field_text(&b.field),
                "label": label,
            });
            if let Some((t, h, l)) = level {
                f["kind"] = json!("hierarchy_level");
                f["table"] = json!(t);
                f["hierarchy"] = json!(h);
                f["name"] = json!(l);
            }
            if let Some(c) = bicraft_report::query::visual_calc(&b.field) {
                f["kind"] = json!("visual_calculation");
                f["name"] = json!(c.name);
                f["expression"] = json!(c.expression);
            }
            if let Some(sp) = bicraft_report::query::sparkline(&b.field) {
                f["kind"] = json!("sparkline");
                f["name"] = json!(bicraft_report::query::default_native_name(&b.field));
                f["sparkline"] = json!({
                    "index": sparkline_index,
                    "y": field_text(&sp.measure),
                    "x": sp.groupings.first().map(field_text),
                });
                sparkline_index = sparkline_index.saturating_add(1);
            }
            if let Some(Some(fmt)) = formats.get(i) {
                f["format"] = json!(fmt);
            }
            if hidden.get(i).copied().unwrap_or(false) {
                f["hidden"] = json!(true);
            }
            // the name visual calculations refer to the field by
            if let Some(r) = refs.get(i).and_then(|n| n.first()) {
                f["ref"] = json!(r);
            }
            if let Some(d) = &b.display_name {
                f["display_name"] = json!(d);
            }
            if let Some(k) = b.show_as {
                f["show_as"] = json!(k.as_str());
            }
            if bicraft_report::query::show_all(v.visual_extra.get("query"), &b.role) {
                f["show_all"] = json!(true);
            }
            f
        })
        .collect();
    let mut o = Map::new();
    o.insert("name".into(), json!(v.name));
    o.insert("type".into(), json!(v.visual_type));
    o.insert("x".into(), json!(pos.x));
    o.insert("y".into(), json!(pos.y));
    o.insert("z".into(), json!(pos.z));
    o.insert("width".into(), json!(pos.width));
    o.insert("height".into(), json!(pos.height));
    o.insert("tab_order".into(), json!(pos.tab_order));
    o.insert("title".into(), json!(visual_title(v)));
    // the alt text (General › Alt text), when set: the PDF's figure stands for it
    if let Some(alt) = crate::citest_rules::alt_text(v) {
        o.insert("alt_text".into(), json!(alt));
    }
    // a hierarchical axis is titled by its displayed level ("Sales by Category", not "… and
    // Product")
    let auto_title = match preview::at_drill_level(titled) {
        Some(d) => ui_lang::visual_auto_title(ui, &d, &ui_lang::field_labels(ui, &d)),
        None => ui_lang::visual_auto_title(ui, titled, &labels),
    };
    o.insert("auto_title".into(), json!(auto_title));
    o.insert("parent_group".into(), json!(v.parent_group));
    o.insert("hidden".into(), json!(v.is_hidden));
    o.insert("sync_group".into(), crate::slicer_sync::sync_group_json(v));
    o.insert("fields".into(), Value::Array(fields));
    // the sort the visual's query defines (`query.sortDefinition`), in `visual.set_sort`'s field
    // syntax: the visual header's Sort axis menu checks it
    if let Some(Ok(sorts)) = crate::visual_props::current_sort(v) {
        let sort: Vec<Value> = sorts
            .iter()
            .take(limits.fields_per_visual)
            .map(|(f, desc)| json!({"field": field_text(f), "descending": desc}))
            .collect();
        o.insert("sort".into(), Value::Array(sort));
    }
    o.insert("interactivity".into(), crate::interact::interactivity(v));
    o.insert(
        "filters".into(),
        crate::filter_edit::cards(v.filter_config.as_ref()),
    );
    o.insert(
        "filter_sort".into(),
        json!(crate::filter_edit::sort_mode(v.filter_config.as_ref())),
    );
    if let Some(roles) = visual_roles(v.visual_type.as_deref()) {
        o.insert("roles".into(), roles);
    }
    o.insert(
        "issues".into(),
        Value::Array(issues.iter().take(limits.issues_per_visual).cloned().collect()),
    );
    flag(
        &mut o,
        v.fields.len() > limits.fields_per_visual || issues.len() > limits.issues_per_visual,
    );
    Value::Object(o)
}

/// The data roles of a built-in visual type whose roles are known, in Desktop's order (the
/// Build pane's wells): `[{name, kind: grouping|measure|grouping_or_measure, max, drill}]`.
/// BIcraft's own visuals (custom visuals to Power BI) give the roles their package declares.
/// `None` for groups, other custom visuals and types whose roles are not known.
fn visual_roles(visual_type: Option<&str>) -> Option<Value> {
    let name = visual_type?;
    let own;
    let t = match bicraft_visuals::visual(name).filter(|t| t.roles_known) {
        Some(t) => t,
        None => {
            own = crate::builtin_visuals::find(name)?.visual_type();
            &own
        }
    };
    Some(Value::Array(
        t.data_roles
            .iter()
            .map(|r| {
                json!({
                    "name": r.name,
                    "kind": r.kind,
                    "max": r.max,
                    "drill": r.drill,
                })
            })
            .collect(),
    ))
}

/// The built-in visual types that take fields (known roles, at least one), by name, with their
/// display names: the Build pane's type picker.
fn visual_types() -> Value {
    Value::Array(
        bicraft_visuals::visual_types()
            .into_iter()
            .filter_map(bicraft_visuals::visual)
            .filter(|t| t.roles_known && !t.data_roles.is_empty())
            .map(|t| json!({"name": t.name, "display_name": t.display_name}))
            .collect(),
    )
}

/// The visual's title text when it is a literal (`visualContainerObjects.title` or the older
/// `objects.title`), or a group's display name.
pub(crate) fn visual_title(v: &Visual) -> Option<String> {
    for key in ["visualContainerObjects", "objects"] {
        let literal = v
            .visual_extra
            .get(key)
            .and_then(|o| o.get("title"))
            .and_then(|t| t.get(0))
            .and_then(|t| t.get("properties"))
            .and_then(|p| p.get("text"))
            .and_then(|t| t.get("expr"))
            .and_then(|e| e.get("Literal"))
            .and_then(|l| l.get("Value"))
            .and_then(Value::as_str);
        if let Some(text) = literal {
            return Some(unquote(text));
        }
    }
    v.extra
        .get("visualGroup")
        .and_then(|g| g.get("displayName"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// A PBIR string literal (`'It''s'`) as text (`It's`); other literals are returned as written.
fn unquote(text: &str) -> String {
    match text.strip_prefix('\'').and_then(|t| t.strip_suffix('\'')) {
        Some(inner) => inner.replace("''", "'"),
        None => text.to_string(),
    }
}

fn report_validation_json(v: Result<Value, String>, limits: &SnapshotLimits) -> Value {
    let v = match v {
        Ok(v) => v,
        Err(e) => return json!({"error": e}),
    };
    let all = v
        .get("issues")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let schema = v
        .get("schema_issues")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let count = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
    let (errors, warnings) = (count("error_count"), count("warning_count"));
    // Schema issues count as errors but are listed separately by report.validate.
    let listed_total = u64::try_from(all.len().saturating_add(schema.len())).unwrap_or(u64::MAX);
    let mut issues: Vec<Value> = all.iter().take(limits.validation_issues).cloned().collect();
    let room = limits.validation_issues.saturating_sub(issues.len());
    issues.extend(schema.iter().take(room).map(|i| {
        let mut i = i.clone();
        if let Some(o) = i.as_object_mut() {
            o.entry("severity").or_insert_with(|| json!("error"));
            o.insert("schema".into(), Value::Bool(true));
        }
        i
    }));
    let mut o = Map::new();
    o.insert("valid".into(), v.get("valid").cloned().unwrap_or(Value::Null));
    o.insert("error_count".into(), json!(errors));
    o.insert("warning_count".into(), json!(warnings));
    for k in ["checked_against_model", "schema_checked"] {
        o.insert(k.into(), v.get(k).cloned().unwrap_or(Value::Null));
    }
    if let Some(note) = v.get("note") {
        o.insert("note".into(), note.clone());
    }
    let cut =
        u64::try_from(issues.len()).unwrap_or(u64::MAX) < errors.saturating_add(warnings).max(listed_total);
    o.insert("issues".into(), Value::Array(issues));
    flag(&mut o, cut);
    Value::Object(o)
}

#[path = "snapshot_visual.rs"]
mod details;

#[cfg(test)]
#[path = "snapshot_tests.rs"]
mod tests;
