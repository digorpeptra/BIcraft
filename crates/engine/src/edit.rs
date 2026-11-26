//! Model editing commands: `model.new`, `table.*`, `column.*`, `measure.*`, `relationship.*`,
//! `role.*`.
//!
//! Every edit works on a clone of the open model. DAX the edit introduces (measures, calculated
//! columns, calculated tables, row filters) is validated against that clone, i.e. against the model
//! as it will be after the change, and the clone replaces the session's model only when everything
//! succeeded. Invalid DAX rejects the edit unless the params say `"force": true`; the diagnostics
//! are then returned as warnings. Names are matched case-insensitively, like DAX does.
//!
//! Renames rewrite every DAX expression that references the renamed object, like Power BI
//! Desktop (`dax_rewritten` in the result; `"update_dax": false` opts out), see
//! [`crate::dax_rename`]. Removals refuse when DAX expressions use the object (listing them),
//! unless `"force": true`, which leaves those expressions as they are. Both report `broken`, the
//! expressions that no longer validate.
//!
//! When the session has a report, renames, removals and moves also report the visual bindings
//! and raw report references they affect (`report` in the result); renames and moves rewrite them
//! with `"update_report": true`.

use std::collections::HashSet;
use std::hash::BuildHasher;
use std::sync::atomic::{AtomicU64, Ordering};

use bicraft_dax::deps::ObjectRef;
use bicraft_dax::{Context, Diagnostic, Severity, Validator, validate};
use bicraft_model::{Column, DataType, Measure, Partition, Role, SemanticModel, Table, TablePermission};
use serde_json::{Map, Value, json};

use crate::Session;
use crate::commands::model;
use crate::dax_rename::{DaxChange, dependants, refusal, rename_in_model};
use crate::params::{allow, change_str, force, opt_bool, opt_str, str_param};
use crate::refs::{ModelChange, apply_change};
use crate::rename_refs::{NameChange, follow as follow_names};

/// Compatibility level of new models: what Power BI Desktop 2.158 gives a new model (it also
/// upgrades a 1567 model to it when saving).
pub(crate) const NEW_MODEL_COMPATIBILITY_LEVEL: u32 = bicraft_versions::NEW_MODEL_COMPATIBILITY_LEVEL;
/// Longest object name accepted.
const MAX_NAME_CHARS: usize = 1024;

// ---------------------------------------------------------------------------------------------
// The edit pipeline
// ---------------------------------------------------------------------------------------------

/// A DAX expression an edit introduces or changes.
struct Check {
    /// What the expression belongs to, for messages, e.g. "measure [Total]".
    object: String,
    expression: String,
    /// Home table (`None` for calculated tables).
    table: Option<String>,
    /// The name of the user-defined function `expression` defines (`( params ) => body`),
    /// validated as a function definition rather than as an expression.
    function: Option<String>,
}

/// What an edit closure produced, besides mutating the model clone.
pub(crate) struct Edit {
    result: Map<String, Value>,
    checks: Vec<Check>,
    /// Report the expressions this edit breaks (renames, removals, moves).
    report_broken: bool,
    /// `(old, new)` table name, so expressions that already failed still match after a rename.
    table_rename: Option<(String, String)>,
    /// Non-DAX warnings.
    pub(crate) notes: Vec<String>,
    /// What the edit means for report bindings.
    report_change: Option<ModelChange>,
    /// A rename (or measure move) to carry into the report-level measures' DAX: the object
    /// (named as before the edit) and what happens to it.
    report_dax: Option<(ObjectRef, DaxChange)>,
    /// Inferred calculated-table columns renamed after their source, carried into the report.
    followers: Vec<crate::rename_inferred::Follower>,
    /// Whether DAX follows the renames (report-level measures included).
    followers_dax: bool,
}

impl Edit {
    pub(crate) fn new(result: Value) -> Self {
        Self {
            result: match result {
                Value::Object(o) => o,
                _ => Map::new(),
            },
            checks: Vec::new(),
            report_broken: false,
            table_rename: None,
            notes: Vec::new(),
            report_change: None,
            report_dax: None,
            followers: Vec::new(),
            followers_dax: false,
        }
    }

    /// Records the inferred columns a rename renamed in turn (`inferred_renamed`), with their
    /// DAX and metadata changes. Call it after the rename's own results are recorded.
    fn inferred(&mut self, f: crate::rename_inferred::Followers, update_dax: bool) {
        if f.renamed.is_empty() {
            return;
        }
        self.result.insert("inferred_renamed".into(), f.summary());
        for (key, list) in [
            ("dax_rewritten", f.dax.rewritten),
            ("dax_not_rewritten", f.dax.unparsable),
            ("metadata_updated", f.metadata),
        ] {
            if list.is_empty() {
                continue;
            }
            match self.result.get_mut(key) {
                Some(Value::Array(a)) => a.extend(list),
                _ => {
                    self.result.insert(key.into(), Value::Array(list));
                }
            }
        }
        self.followers = f.renamed;
        self.followers_dax = update_dax;
    }

    /// Adds `key` to the command's result.
    pub(crate) fn insert(&mut self, key: &str, value: Value) {
        self.result.insert(key.into(), value);
    }

    pub(crate) fn breaking(mut self) -> Self {
        self.report_broken = true;
        self
    }

    /// Records the name-keyed metadata a rename or move updated (`metadata_updated`).
    fn names_followed(&mut self, updated: Vec<Value>) {
        if !updated.is_empty() {
            self.result
                .insert("metadata_updated".into(), Value::Array(updated));
        }
    }

    pub(crate) fn check(&mut self, object: String, expression: &str, table: Option<&str>) {
        self.checks.push(Check {
            object,
            expression: expression.to_string(),
            table: table.map(str::to_string),
            function: None,
        });
    }

    /// Validates the definition of the user-defined function `name` (`( params ) => body`).
    pub(crate) fn check_function(&mut self, name: &str, expression: &str) {
        self.checks.push(Check {
            object: format!("function `{name}`"),
            expression: expression.to_string(),
            table: None,
            function: Some(name.to_string()),
        });
    }
}

/// Applies `f` to a clone of the open model, validates, then commits. Nothing in the session
/// changes unless this returns `Ok`.
pub(crate) fn apply(
    s: &mut Session,
    params: &Value,
    f: impl FnOnce(&mut SemanticModel) -> Result<Edit, String>,
) -> Result<Value, String> {
    let force = force(params)?;
    let update_report = opt_bool(params, "update_report")?.unwrap_or(false);
    let current = model(s)?;
    let mut next = current.clone();
    let edit = f(&mut next)?;
    // Translations of objects the edit removed go with them (M10.4).
    let translations_dropped = crate::translations::drop_dangling(&mut next);
    // So do their perspective memberships.
    let perspective_entries_dropped = crate::perspectives::drop_dangling(&mut next);

    let mut diagnostics = Vec::new();
    let mut errors = Vec::new();
    for c in &edit.checks {
        if let Some(name) = &c.function {
            let v = bicraft_dax::udf::validate_function(name, &c.expression, Some(&next));
            if !v.valid {
                errors.push(format!("{}: {}", c.object, summarize(&v.diagnostics)));
            }
            if !v.diagnostics.is_empty() {
                diagnostics.push(json!({"object": c.object, "valid": v.valid, "diagnostics": v.diagnostics}));
            }
            continue;
        }
        let mut v = validate(
            &c.expression,
            &Context {
                model: Some(&next),
                table: c.table.as_deref(),
            },
        );
        // every check is a model object's DAX: the query-only INFO functions are refused there,
        // as the engine does (docs/formats/dax-functions.md, "INFO functions")
        let info = bicraft_dax::query_only_info_calls(&c.expression);
        if !info.is_empty() {
            v.valid = false;
            v.diagnostics.extend(info);
        }
        if !v.valid {
            errors.push(format!("{}: {}", c.object, summarize(&v.diagnostics)));
        }
        if !v.diagnostics.is_empty() {
            diagnostics.push(json!({"object": c.object, "valid": v.valid, "diagnostics": v.diagnostics}));
        }
    }
    if !errors.is_empty() && !force {
        return Err(format!(
            "invalid DAX, nothing was changed: {} (pass \"force\": true to apply anyway)",
            errors.join("; ")
        ));
    }
    let mut warnings = edit.notes;
    warnings.extend(
        errors
            .into_iter()
            .map(|e| format!("applied with invalid DAX (force): {e}")),
    );
    let broken = edit
        .report_broken
        .then(|| newly_broken(current, &next, edit.table_rename.as_ref()));
    let report_dax = match (&edit.report_dax, s.report.as_ref()) {
        (Some((target, change)), Some(r)) => {
            crate::report_measures::rename_references(r, current, target, change)
        }
        _ => (None, Vec::new()),
    };

    s.model = Some(next);
    s.model_error = None;
    s.dirty = true;
    // the model diagram's nodes name tables (`nodeIndex`): they follow a rename
    if let Some((old, new)) = &edit.table_rename
        && old != new
    {
        crate::diagram_layout::rename_table(s, old, new);
    }

    let mut out = edit.result;
    if let (Some(pending), Some(report)) = (report_dax.0, s.report.as_mut()) {
        match crate::report_measures::commit_doc(report, pending) {
            Ok(()) => s.report_dirty = true,
            Err(e) => warnings.push(format!("the report-level measures were not updated: {e}")),
        }
    }
    if !report_dax.1.is_empty()
        && let Some(Value::Array(list)) = out.get_mut("dax_rewritten")
    {
        list.extend(report_dax.1);
    }
    if let (Some(change), Some(report)) = (&edit.report_change, s.report.as_mut()) {
        let (effects, modified) = apply_change(report, change, update_report);
        if modified {
            s.report_dirty = true;
        }
        out.insert("report".into(), effects);
    } else if update_report && s.report.is_none() {
        warnings.push("update_report: no report is open, so there was nothing to update".into());
    }
    if let Some(report) = s.report.as_mut() {
        for (i, f) in edit.followers.iter().enumerate() {
            if edit.followers_dax {
                let target = ObjectRef::Column {
                    table: f.table.clone(),
                    name: f.old.clone(),
                };
                let (pending, listed) = crate::report_measures::rename_references(
                    report,
                    &f.before,
                    &target,
                    &DaxChange::Rename(f.new.clone()),
                );
                if let Some(pending) = pending {
                    match crate::report_measures::commit_doc(report, pending) {
                        Ok(()) => s.report_dirty = true,
                        Err(e) => warnings.push(format!("the report-level measures were not updated: {e}")),
                    }
                }
                if let Some(Value::Array(list)) = out.get_mut("dax_rewritten") {
                    list.extend(listed);
                }
            }
            let change = ModelChange::ColumnRenamed {
                table: f.table.clone(),
                old: f.old.clone(),
                new: f.new.clone(),
            };
            let (effects, modified) = apply_change(report, &change, update_report);
            if modified {
                s.report_dirty = true;
            }
            if let Some(entry) = out
                .get_mut("inferred_renamed")
                .and_then(|v| v.get_mut(i))
                .and_then(Value::as_object_mut)
            {
                entry.insert("report".into(), effects);
            }
        }
    }
    out.insert("warnings".into(), json!(warnings));
    out.insert("diagnostics".into(), json!(diagnostics));
    if !translations_dropped.is_empty() {
        out.insert("translations_dropped".into(), json!(translations_dropped));
    }
    if !perspective_entries_dropped.is_empty() {
        out.insert(
            "perspective_entries_dropped".into(),
            json!(perspective_entries_dropped),
        );
    }
    if let Some(broken) = broken {
        out.insert("broken".into(), json!(broken));
    }
    Ok(Value::Object(out))
}

fn summarize(diags: &[Diagnostic]) -> String {
    let errors: Vec<String> = diags
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| format!("{}:{}: {} [{}]", d.line, d.column, d.message, d.code))
        .collect();
    if errors.is_empty() {
        "the expression is incomplete".into()
    } else {
        errors.join(", ")
    }
}

/// One DAX expression stored in the model.
struct Stored<'a> {
    kind: &'static str,
    table: &'a str,
    name: &'a str,
    expression: &'a str,
    /// Home table for binding (`None` for calculated tables).
    home: Option<&'a str>,
}

/// Every DAX expression in the model: measures, calculated columns, calculated tables and row
/// filters.
fn stored_expressions(m: &SemanticModel) -> Vec<Stored<'_>> {
    let mut out = Vec::new();
    for t in &m.tables {
        for x in &t.measures {
            out.push(Stored {
                kind: "measure",
                table: &t.name,
                name: &x.name,
                expression: &x.expression,
                home: Some(&t.name),
            });
        }
        for c in &t.columns {
            if let Some(e) = &c.expression {
                out.push(Stored {
                    kind: "calculated column",
                    table: &t.name,
                    name: &c.name,
                    expression: e,
                    home: Some(&t.name),
                });
            }
        }
        for p in &t.partitions {
            if p.source_type.eq_ignore_ascii_case("calculated")
                && let Some(e) = &p.expression
            {
                out.push(Stored {
                    kind: "calculated table",
                    table: &t.name,
                    name: &t.name,
                    expression: e,
                    home: None,
                });
            }
        }
    }
    for r in &m.roles {
        for tp in &r.table_permissions {
            if let Some(e) = &tp.filter_expression {
                out.push(Stored {
                    kind: "row filter",
                    table: &tp.table,
                    name: &r.name,
                    expression: e,
                    home: Some(&tp.table),
                });
            }
        }
    }
    out
}

fn is_valid(v: &mut Validator<'_>, e: &Stored<'_>) -> Result<(), Vec<Diagnostic>> {
    let v = v.validate(e.expression, e.home);
    if v.valid { Ok(()) } else { Err(v.diagnostics) }
}

/// Expressions that fail in `after` but did not fail in `before`. Identity is (kind, home table,
/// expression text), with the home table mapped through `rename`, so objects that were already
/// broken are not reported again.
pub(crate) fn newly_broken(
    before: &SemanticModel,
    after: &SemanticModel,
    rename: Option<&(String, String)>,
) -> Vec<Value> {
    let key =
        |kind: &str, table: &str, expr: &str| (kind.to_string(), table.to_lowercase(), expr.to_string());
    let mut checker = Validator::new(before);
    let failing_before: HashSet<_> = stored_expressions(before)
        .iter()
        .filter(|e| is_valid(&mut checker, e).is_err())
        .map(|e| {
            let table = match rename {
                Some((old, new)) if e.table.eq_ignore_ascii_case(old) => new.as_str(),
                _ => e.table,
            };
            key(e.kind, table, e.expression)
        })
        .collect();
    let mut checker = Validator::new(after);
    stored_expressions(after)
        .iter()
        .filter(|e| !failing_before.contains(&key(e.kind, e.table, e.expression)))
        .filter_map(|e| {
            is_valid(&mut checker, e).err().map(|diagnostics| {
                json!({"kind": e.kind, "table": e.table, "name": e.name, "expression": e.expression,
                       "diagnostics": diagnostics})
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// DAX through renames and removals (G15)
// ---------------------------------------------------------------------------------------------

/// `update_dax` (default true): renames rewrite the DAX that references the object.
fn update_dax(p: &Value) -> Result<bool, String> {
    Ok(opt_bool(p, "update_dax")?.unwrap_or(true))
}

/// The DAX side of a rename, recorded into the edit's result.
#[derive(Default)]
struct DaxRename {
    /// The renamed or moved object (as named before) and what happens to it, when DAX was
    /// rewritten.
    target: Option<(ObjectRef, DaxChange)>,
    outcome: crate::dax_rename::RenameOutcome,
    /// `update_dax: false` on a real rename.
    skipped: bool,
}

impl DaxRename {
    fn record(self, edit: &mut Edit) {
        if let Some(target) = self.target {
            self.outcome.into_result(&mut edit.result);
            edit.report_dax = Some(target);
        } else if self.skipped {
            edit.notes
                .push("update_dax is false: DAX expressions still use the old name (see `broken`)".into());
        }
    }
}

/// Rewrites the model's DAX for a rename of the object `target` picks (`None`: not a rename,
/// e.g. the same name), before the object itself is renamed.
fn rename_dax(
    m: &mut SemanticModel,
    update: bool,
    target: impl FnOnce(&SemanticModel) -> Option<ObjectRef>,
    new_name: &str,
) -> DaxRename {
    change_dax(m, update, target, DaxChange::Rename(new_name.to_string()))
}

/// [`rename_dax`] for any [`DaxChange`].
fn change_dax(
    m: &mut SemanticModel,
    update: bool,
    target: impl FnOnce(&SemanticModel) -> Option<ObjectRef>,
    change: DaxChange,
) -> DaxRename {
    let Some(target) = target(m) else {
        return DaxRename::default();
    };
    if !update {
        return DaxRename {
            skipped: true,
            ..DaxRename::default()
        };
    }
    let outcome = rename_in_model(m, &target, &change);
    DaxRename {
        target: Some((target, change)),
        outcome,
        skipped: false,
    }
}

/// Refuses a removal that DAX expressions depend on unless `force`; otherwise returns those
/// expressions (`used_by` in the result).
fn check_removal(s: &Session, p: &Value, what: &str, target: &ObjectRef) -> Result<Vec<Value>, String> {
    let m = model(s)?;
    let mut found = dependants(m, target);
    if let Some(r) = s.report.as_ref() {
        found.extend(crate::report_measures::dependants(r, m, target));
    }
    if !found.is_empty() && !force(p)? {
        let labels: Vec<String> = found.iter().map(|(_, l)| l.clone()).collect();
        return Err(refusal(what, &labels));
    }
    Ok(found.into_iter().map(|(v, _)| v).collect())
}

// ---------------------------------------------------------------------------------------------
// Names and helpers
// ---------------------------------------------------------------------------------------------

pub(crate) fn check_name(kind: &str, name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err(format!("{kind} name must not be empty"));
    }
    if name.trim() != name {
        return Err(format!(
            "{kind} name `{name}` must not start or end with whitespace"
        ));
    }
    if name.chars().any(char::is_control) {
        return Err(format!("{kind} name must not contain control characters"));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!("{kind} name is longer than {MAX_NAME_CHARS} characters"));
    }
    Ok(())
}

/// A table name as messages and report field references show it: `Sales` or `'Order Lines'`.
/// Not for DAX the engine parses: a reserved word (`Date`, `Item`, `Typed`) stays bare here, the
/// way Desktop shows fields; DAX writers use `bicraft_dax::table_needs_quotes`.
pub(crate) fn dax_table(name: &str) -> String {
    let mut chars = name.chars();
    let simple = chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if simple {
        name.to_string()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

/// A column or measure reference: `Sales[Amount]`, `[Total]`.
pub(crate) fn dax_column(table: &str, name: &str) -> String {
    format!("{}{}", dax_table(table), dax_measure(name))
}

pub(crate) fn dax_measure(name: &str) -> String {
    format!("[{}]", name.replace(']', "]]"))
}

pub(crate) fn table_index(m: &SemanticModel, name: &str) -> Result<usize, String> {
    m.tables
        .iter()
        .position(|t| t.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| format!("no table named `{name}`"))
}

pub(crate) fn table_at(m: &mut SemanticModel, i: usize) -> Result<&mut Table, String> {
    m.tables
        .get_mut(i)
        .ok_or_else(|| "internal: table index out of range".to_string())
}

/// Index of a user column. Engine-internal row-number columns (TMSL `type: rowNumber`) are not
/// editable and never match.
fn column_index(t: &Table, name: &str) -> Result<usize, String> {
    if let Some(i) = t
        .columns
        .iter()
        .position(|c| !c.is_row_number() && c.name.eq_ignore_ascii_case(name))
    {
        return Ok(i);
    }
    if t.columns
        .iter()
        .any(|c| c.is_row_number() && c.name.eq_ignore_ascii_case(name))
    {
        return Err(format!(
            "`{name}` is an engine-internal row-number column of `{}`; it cannot be edited",
            t.name
        ));
    }
    Err(format!("table `{}` has no column `{name}`", t.name))
}

/// `(table index, measure index)` of a measure anywhere in the model.
pub(crate) fn measure_position(m: &SemanticModel, name: &str) -> Result<(usize, usize), String> {
    m.tables
        .iter()
        .enumerate()
        .find_map(|(ti, t)| {
            t.measures
                .iter()
                .position(|x| x.name.eq_ignore_ascii_case(name))
                .map(|mi| (ti, mi))
        })
        .ok_or_else(|| format!("no measure named `{name}`"))
}

/// A measure name must be unique in the model and differ from every column of its table.
pub(crate) fn check_measure_name_free(
    m: &SemanticModel,
    table: usize,
    name: &str,
    except: Option<(usize, usize)>,
) -> Result<(), String> {
    for (ti, t) in m.tables.iter().enumerate() {
        for (mi, x) in t.measures.iter().enumerate() {
            if x.name.eq_ignore_ascii_case(name) && except != Some((ti, mi)) {
                return Err(format!(
                    "a measure named `{}` already exists (in table `{}`); measure names are unique in the model",
                    x.name, t.name
                ));
            }
        }
    }
    if let Some(t) = m.tables.get(table)
        && let Some(c) = t.column(name)
    {
        return Err(format!(
            "table `{}` has a column named `{}`; a measure cannot share its name with a column of its table",
            t.name, c.name
        ));
    }
    Ok(())
}

/// A column name must be unique among the table's user columns and differ from its measures. It
/// may not reuse the name of an engine-internal row-number column either: the file would then hold
/// two columns with the same name.
fn check_column_name_free(t: &Table, name: &str, except: Option<usize>) -> Result<(), String> {
    if let Some((i, c)) = t
        .columns
        .iter()
        .enumerate()
        .find(|(_, c)| c.name.eq_ignore_ascii_case(name))
        && except != Some(i)
    {
        return Err(if c.is_row_number() {
            format!(
                "`{}` is the name of an engine-internal row-number column of `{}`; pick another name",
                c.name, t.name
            )
        } else {
            format!("table `{}` already has a column named `{}`", t.name, c.name)
        });
    }
    if let Some(x) = t.measure(name) {
        return Err(format!(
            "table `{}` has a measure named `{}`; a column cannot share its name with a measure of its table",
            t.name, x.name
        ));
    }
    Ok(())
}

fn parse_data_type(name: &str) -> Result<DataType, String> {
    match DataType::from_tmsl(name) {
        DataType::Unknown => Err(format!(
            "unknown data_type `{name}` (expected string, int64, double, decimal, dateTime, boolean, binary or variant)"
        )),
        t => Ok(t),
    }
}

/// The data type of calculated column `expression` of `table`, as its DAX gives it (text when
/// the type depends on the rows).
fn inferred_type(s: &Session, table: &str, expression: &str) -> DataType {
    use bicraft_dax::types::{ScalarType, ValueType};
    let v = bicraft_dax::validate_as(
        expression,
        &Context {
            model: s.model.as_ref(),
            table: Some(table),
        },
        bicraft_dax::ExprKind::CalculatedColumn,
    );
    match v.result_type {
        Some(ValueType::Scalar(ScalarType::Integer)) => DataType::Int64,
        Some(ValueType::Scalar(ScalarType::Decimal)) => DataType::Double,
        Some(ValueType::Scalar(ScalarType::Currency)) => DataType::Decimal,
        Some(ValueType::Scalar(ScalarType::Boolean)) => DataType::Boolean,
        Some(ValueType::Scalar(ScalarType::DateTime)) => DataType::DateTime,
        _ => DataType::String,
    }
}

const SUMMARIZE_BY: &[&str] = &[
    "default",
    "none",
    "sum",
    "min",
    "max",
    "count",
    "average",
    "distinctCount",
];

fn parse_summarize_by(v: &str) -> Result<String, String> {
    SUMMARIZE_BY
        .iter()
        .find(|s| s.eq_ignore_ascii_case(v))
        .map(|s| (*s).to_string())
        .ok_or_else(|| {
            format!(
                "unknown summarize_by `{v}` (expected {})",
                SUMMARIZE_BY.join(", ")
            )
        })
}

const MODEL_PERMISSIONS: &[&str] = &["none", "read", "readRefresh", "refresh", "administrator"];

pub(crate) fn parse_model_permission(v: &str) -> Result<String, String> {
    MODEL_PERMISSIONS
        .iter()
        .find(|s| s.eq_ignore_ascii_case(v))
        .map(|s| (*s).to_string())
        .ok_or_else(|| {
            format!(
                "unknown model_permission `{v}` (expected {})",
                MODEL_PERMISSIONS.join(", ")
            )
        })
}

fn non_empty<'a>(key: &str, v: &'a str) -> Result<&'a str, String> {
    if v.trim().is_empty() {
        Err(format!("parameter `{key}` must not be empty"))
    } else {
        Ok(v)
    }
}

/// A fresh random GUID (version 4 layout), as Power BI uses for relationship names and lineage
/// tags. Randomness comes from the standard library's per-process hash keys, so no extra
/// dependency is needed; uniqueness within a model is checked by the callers that need it.
/// During a recovery replay, the one the journal recorded (`recovery.rs`), so replayed edits get
/// the same lineage tags and names.
pub(crate) fn new_guid() -> String {
    crate::recovery::taped_guid(fresh_guid)
}

fn fresh_guid() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let state = std::collections::hash_map::RandomState::new();
    let hi = state.hash_one((n, nanos, 0u8));
    let lo = state.hash_one((n, nanos, 1u8));
    let mut v = (u128::from(hi) << 64) | u128::from(lo);
    v = (v & !(0xF << 76)) | (0x4 << 76);
    v = (v & !(0xC << 60)) | (0x8 << 60);
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        v >> 96,
        (v >> 80) & 0xFFFF,
        (v >> 64) & 0xFFFF,
        (v >> 48) & 0xFFFF,
        v & 0xFFFF_FFFF_FFFF
    )
}

/// An empty model as `model.new` and `project.new` create it, with the model-level settings
/// Power BI Desktop 2.158 gives a new model (`docs/formats/tmdl.md`, *New models*). Without
/// `defaultPowerBIDataSourceVersion: powerBI_V3` Desktop refuses to refresh M partitions
/// ("Partition_<guid> not recognized"). Models read from files are never given these.
pub(crate) fn empty_model(name: &str, culture: &str) -> SemanticModel {
    let mut extra = Map::new();
    extra.insert("defaultPowerBIDataSourceVersion".into(), json!("powerBI_V3"));
    extra.insert("sourceQueryCulture".into(), json!(culture));
    extra.insert(
        "dataAccessOptions".into(),
        json!({"legacyRedirects": true, "returnErrorValuesAsNull": true}),
    );
    extra.insert("valueFilterBehavior".into(), json!("independent"));
    SemanticModel {
        name: name.to_string(),
        compatibility_level: Some(NEW_MODEL_COMPATIBILITY_LEVEL),
        culture: Some(culture.to_string()),
        extra,
        // Auto date/time off, explicitly: without the annotation Desktop applies its own option
        // (on by default) at the first refresh and adds hidden `LocalDateTable_…` tables and
        // date variations to every date column.
        annotations: vec![bicraft_model::Annotation {
            name: "__PBI_TimeIntelligenceEnabled".into(),
            value: "0".into(),
        }],
        ..Default::default()
    }
}

/// Points relationships at a renamed table.
fn rename_table_in_relationships(m: &mut SemanticModel, old: &str, new: &str) {
    for r in &mut m.relationships {
        if r.from_table.eq_ignore_ascii_case(old) {
            r.from_table = new.to_string();
        }
        if r.to_table.eq_ignore_ascii_case(old) {
            r.to_table = new.to_string();
        }
    }
}

/// Removes the relationships that use `table` (and `column`, when given); returns their names.
fn remove_relationships(m: &mut SemanticModel, table: &str, column: Option<&str>) -> Vec<String> {
    let uses =
        |t: &str, c: &str| t.eq_ignore_ascii_case(table) && column.is_none_or(|x| c.eq_ignore_ascii_case(x));
    let mut removed = Vec::new();
    m.relationships.retain(|r| {
        let hit = uses(&r.from_table, &r.from_column) || uses(&r.to_table, &r.to_column);
        if hit {
            removed.push(r.name.clone());
        }
        !hit
    });
    removed
}

/// The hierarchies of a table (kept in `extra` as TMSL JSON): rename the levels' column.
pub(crate) fn rename_column_in_hierarchies(t: &mut Table, old: &str, new: &str) {
    let Some(Value::Array(hierarchies)) = t.extra.get_mut("hierarchies") else {
        return;
    };
    for h in hierarchies {
        let Some(Value::Array(levels)) = h.get_mut("levels") else {
            continue;
        };
        for l in levels {
            if let Some(Value::String(c)) = l.get_mut("column")
                && c.eq_ignore_ascii_case(old)
            {
                *c = new.to_string();
            }
        }
    }
}

/// Drops hierarchy levels built on a removed column, and hierarchies left without levels.
fn remove_column_from_hierarchies(t: &mut Table, column: &str) {
    let Some(Value::Array(hierarchies)) = t.extra.get_mut("hierarchies") else {
        return;
    };
    for h in hierarchies.iter_mut() {
        if let Some(Value::Array(levels)) = h.get_mut("levels") {
            levels.retain(|l| {
                !l.get("column")
                    .and_then(Value::as_str)
                    .is_some_and(|c| c.eq_ignore_ascii_case(column))
            });
            for (i, l) in levels.iter_mut().enumerate() {
                if let Some(o) = l.as_object_mut()
                    && o.contains_key("ordinal")
                {
                    o.insert("ordinal".into(), json!(i));
                }
            }
        }
    }
    hierarchies.retain(|h| {
        h.get("levels")
            .and_then(Value::as_array)
            .is_none_or(|l| !l.is_empty())
    });
    if hierarchies.is_empty() {
        t.extra.remove("hierarchies");
    }
}

// ---------------------------------------------------------------------------------------------
// model.new
// ---------------------------------------------------------------------------------------------

pub(crate) fn model_new(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "culture"])?;
    let name = str_param(p, "name")?;
    check_name("model", name)?;
    let culture = opt_str(p, "culture")?.unwrap_or("en-US");
    if culture.is_empty() || !culture.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(format!("culture `{culture}` is not a culture name like en-US"));
    }
    let discarded = (s.dirty && s.model.is_some()) || (s.report_dirty && s.report.is_some());
    s.source = None;
    s.cache_abf = None;
    s.model = Some(empty_model(name, culture));
    s.model_error = None;
    s.dirty = true;
    s.report = None;
    s.report_error = None;
    s.report_dirty = false;
    s.diagram_layout = None;
    crate::dax_queries::reset(s);
    Ok(json!({
        "name": name,
        "culture": culture,
        "compatibility_level": NEW_MODEL_COMPATIBILITY_LEVEL,
        "discarded_unsaved_changes": discarded,
        "next": "add tables with table.add (and a report with report.new), then save with project.new \
                 {dir, name} or file.save_as {path}",
    }))
}

// ---------------------------------------------------------------------------------------------
// table.*
// ---------------------------------------------------------------------------------------------

fn parse_partition(v: &Value, table: &str) -> Result<Partition, String> {
    if !v.is_object() {
        return Err("`partition` must be an object {source_type: \"m\"|\"calculated\", expression}".into());
    }
    allow(v, &["source_type", "expression"]).map_err(|e| format!("partition: {e}"))?;
    let source_type = match str_param(v, "source_type").map_err(|e| format!("partition: {e}"))? {
        t if t.eq_ignore_ascii_case("m") => "m",
        t if t.eq_ignore_ascii_case("calculated") => "calculated",
        t => {
            return Err(format!(
                "partition: unknown source_type `{t}` (expected m or calculated)"
            ));
        }
    };
    let expression = non_empty(
        "partition.expression",
        str_param(v, "expression").map_err(|e| format!("partition: {e}"))?,
    )?;
    Ok(Partition {
        name: table.to_string(),
        mode: Some("import".into()),
        source_type: source_type.into(),
        expression: Some(expression.to_string()),
        ..Default::default()
    })
}

pub(crate) fn table_add(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "description", "hidden", "partition", "force"])?;
    let name = str_param(p, "name")?.to_string();
    check_name("table", &name)?;
    let description = opt_str(p, "description")?.map(str::to_string);
    let hidden = opt_bool(p, "hidden")?.unwrap_or(false);
    let partition = match p.get("partition") {
        None | Some(Value::Null) => None,
        Some(v) => Some(parse_partition(v, &name)?),
    };
    apply(s, p, |m| {
        if let Some(t) = m.table(&name) {
            return Err(format!("a table named `{}` already exists", t.name));
        }
        let mut edit = Edit::new(json!({"table": name}));
        match &partition {
            Some(part) if part.source_type == "calculated" => {
                let expr = part.expression.as_deref().unwrap_or_default();
                edit.check(format!("calculated table {}", dax_table(&name)), expr, None);
            }
            Some(_) => {}
            None => edit.notes.push(format!(
                "table `{name}` has no partition: Power BI Desktop needs one to load the model \
                 (pass `partition`, or use the table only to hold measures once it has one)"
            )),
        }
        let mut table = Table {
            name: name.clone(),
            description,
            lineage_tag: Some(new_guid()),
            is_hidden: hidden,
            partitions: partition.into_iter().collect(),
            ..Default::default()
        };
        crate::desktop_defaults::new_table(&mut table);
        m.tables.push(table);
        Ok(edit)
    })
}

pub(crate) fn table_remove(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "force"])?;
    let name = str_param(p, "name")?;
    let actual = {
        let m = model(s)?;
        m.tables
            .get(table_index(m, name)?)
            .map(|t| t.name.clone())
            .unwrap_or_default()
    };
    let used_by = check_removal(
        s,
        p,
        &format!("table {}", dax_table(&actual)),
        &ObjectRef::Table { name: actual.clone() },
    )?;
    apply(s, p, |m| {
        let (mut result, change) = remove_table_in(m, name)?;
        result.insert("used_by".into(), json!(used_by));
        let mut edit = Edit::new(Value::Object(result)).breaking();
        edit.report_change = Some(change);
        Ok(edit)
    })
}

/// The mutation of `table.remove`, without its checks: the table, its relationships and its row
/// filters. Also used by `model.remove_unused`.
pub(crate) fn remove_table_in(
    m: &mut SemanticModel,
    name: &str,
) -> Result<(Map<String, Value>, ModelChange), String> {
    let i = table_index(m, name)?;
    let table = m.tables.remove(i);
    let relationships = remove_relationships(m, &table.name, None);
    let mut roles = Vec::new();
    for r in &mut m.roles {
        let before = r.table_permissions.len();
        r.table_permissions
            .retain(|tp| !tp.table.eq_ignore_ascii_case(&table.name));
        if r.table_permissions.len() != before {
            roles.push(r.name.clone());
        }
    }
    let mut out = Map::new();
    out.insert("removed".into(), json!(table.name));
    out.insert("removed_relationships".into(), json!(relationships));
    out.insert("removed_row_filters_from_roles".into(), json!(roles));
    Ok((out, ModelChange::TableRemoved { table: table.name }))
}

pub(crate) fn table_rename(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "new_name", "update_report", "update_dax"])?;
    let name = str_param(p, "name")?;
    let new_name = str_param(p, "new_name")?;
    check_name("table", new_name)?;
    let update_dax = update_dax(p)?;
    apply(s, p, |m| {
        let i = table_index(m, name)?;
        if let Some((_, other)) = m
            .tables
            .iter()
            .enumerate()
            .find(|(j, t)| *j != i && t.name.eq_ignore_ascii_case(new_name))
        {
            return Err(format!("a table named `{}` already exists", other.name));
        }
        let dax = rename_dax(
            m,
            update_dax,
            |m| {
                let old = &m.tables.get(i)?.name;
                (old != new_name).then(|| ObjectRef::Table { name: old.clone() })
            },
            new_name,
        );
        let t = table_at(m, i)?;
        let old = std::mem::replace(&mut t.name, new_name.to_string());
        let followed = if old == new_name {
            Vec::new()
        } else {
            follow_names(
                m,
                NameChange::Table {
                    old: &old,
                    new: new_name,
                },
            )
        };
        rename_table_in_relationships(m, &old, new_name);
        for r in &mut m.roles {
            for tp in &mut r.table_permissions {
                if tp.table.eq_ignore_ascii_case(&old) {
                    tp.table = new_name.to_string();
                }
            }
        }
        let followers = if old == new_name {
            crate::rename_inferred::Followers::default()
        } else {
            crate::rename_inferred::follow_inferred(
                m,
                NameChange::Table {
                    old: &old,
                    new: new_name,
                },
                update_dax,
            )
        };
        let mut edit = Edit::new(json!({"renamed": old, "to": new_name})).breaking();
        edit.names_followed(followed);
        edit.report_change = Some(ModelChange::TableRenamed {
            old: old.clone(),
            new: new_name.to_string(),
        });
        edit.table_rename = Some((old, new_name.to_string()));
        dax.record(&mut edit);
        edit.inferred(followers, update_dax);
        Ok(edit)
    })
}

pub(crate) fn table_set(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &["name", "description", "hidden", "data_category", "key_column"],
    )?;
    let name = str_param(p, "name")?;
    let description = change_str(p, "description")?;
    let hidden = opt_bool(p, "hidden")?;
    let date = crate::date_table::plan(s, p, name)?;
    if description.is_none() && hidden.is_none() && date.data_category.is_none() && date.key_column.is_none()
    {
        return Err(
            "table.set needs at least one of `description`, `hidden`, `data_category`, `key_column`".into(),
        );
    }
    apply(s, p, |m| {
        crate::date_table::apply(m, name, &date)?;
        let i = table_index(m, name)?;
        let t = table_at(m, i)?;
        if let Some(d) = description {
            t.description = d;
        }
        if let Some(h) = hidden {
            t.is_hidden = h;
        }
        let key = t
            .columns
            .iter()
            .find(|c| c.extra.get("isKey").and_then(Value::as_bool) == Some(true))
            .map(|c| c.name.clone());
        let mut edit = Edit::new(
            json!({"table": t.name, "description": t.description, "hidden": t.is_hidden,
            "data_category": crate::date_table::data_category(t), "key_column": key,
            "data_checked": date.data_checked}),
        );
        edit.notes = date.notes;
        Ok(edit)
    })
}

// ---------------------------------------------------------------------------------------------
// column.*
// ---------------------------------------------------------------------------------------------

pub(crate) fn column_add(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "table",
            "name",
            "data_type",
            "source_column",
            "expression",
            "format_string",
            "display_folder",
            "summarize_by",
            "hidden",
            "description",
            "force",
        ],
    )?;
    let table = str_param(p, "table")?;
    let name = str_param(p, "name")?;
    check_name("column", name)?;
    let expression = opt_str(p, "expression")?
        .map(|e| non_empty("expression", e))
        .transpose()?;
    // a calculated column without `data_type` takes its expression's (the Data view's New column)
    let data_type = match (opt_str(p, "data_type")?, expression) {
        (Some(t), _) => parse_data_type(t)?,
        (None, Some(e)) => inferred_type(s, table, e),
        (None, None) => return Err("`data_type` is required for a data column".into()),
    };
    let source_column = opt_str(p, "source_column")?;
    if expression.is_some() && source_column.is_some() {
        return Err(
            "pass either `expression` (calculated column) or `source_column` (data column), not both".into(),
        );
    }
    let summarize_by = opt_str(p, "summarize_by")?.map(parse_summarize_by).transpose()?;
    let summarize_given = summarize_by.is_some();
    let mut column = Column {
        name: name.to_string(),
        data_type,
        expression: expression.map(str::to_string),
        source_column: match expression {
            Some(_) => None,
            None => Some(source_column.unwrap_or(name).to_string()),
        },
        format_string: opt_str(p, "format_string")?.map(str::to_string),
        display_folder: opt_str(p, "display_folder")?.map(str::to_string),
        summarize_by,
        description: opt_str(p, "description")?.map(str::to_string),
        lineage_tag: Some(new_guid()),
        is_hidden: opt_bool(p, "hidden")?.unwrap_or(false),
        ..Default::default()
    };
    apply(s, p, |m| {
        let i = table_index(m, table)?;
        // A data column of an M table: Desktop's date/time defaults follow the type its query
        // declares for the source column.
        let date_kind = match (&column.source_column, m.tables.get(i)) {
            (Some(sc), Some(t)) if column.data_type == DataType::DateTime => {
                crate::desktop_defaults::source_date_kind(m, t, sc)
            }
            _ => None,
        };
        crate::desktop_defaults::new_column(&mut column, summarize_given, date_kind);
        let t = table_at(m, i)?;
        check_column_name_free(t, name, None)?;
        let reference = dax_column(&t.name, name);
        let mut edit =
            Edit::new(json!({"table": t.name, "column": name, "calculated": expression.is_some()}));
        if let Some(e) = expression {
            edit.check(format!("calculated column {reference}"), e, Some(&t.name));
        }
        t.columns.push(column);
        Ok(edit)
    })
}

pub(crate) fn column_set(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "table",
            "name",
            "new_name",
            "data_type",
            "source_column",
            "expression",
            "format_string",
            "display_folder",
            "summarize_by",
            "hidden",
            "description",
            "force",
            "update_report",
            "update_dax",
        ],
    )?;
    let table = str_param(p, "table")?;
    let name = str_param(p, "name")?;
    let new_name = opt_str(p, "new_name")?;
    if let Some(n) = new_name {
        check_name("column", n)?;
    }
    let update_dax = update_dax(p)?;
    let data_type = opt_str(p, "data_type")?.map(parse_data_type).transpose()?;
    let expression = opt_str(p, "expression")?
        .map(|e| non_empty("expression", e))
        .transpose()?;
    let source_column = opt_str(p, "source_column")?
        .map(|e| non_empty("source_column", e))
        .transpose()?;
    let format_string = change_str(p, "format_string")?;
    let display_folder = change_str(p, "display_folder")?;
    let summarize_by = match change_str(p, "summarize_by")? {
        Some(Some(v)) => Some(Some(parse_summarize_by(&v)?)),
        other => other,
    };
    let description = change_str(p, "description")?;
    let hidden = opt_bool(p, "hidden")?;
    let changes = [
        new_name.is_some(),
        data_type.is_some(),
        expression.is_some(),
        source_column.is_some(),
        format_string.is_some(),
        display_folder.is_some(),
        summarize_by.is_some(),
        description.is_some(),
        hidden.is_some(),
    ];
    if !changes.contains(&true) {
        return Err("column.set needs at least one property to change".into());
    }
    apply(s, p, |m| {
        let ti = table_index(m, table)?;
        let t = table_at(m, ti)?;
        let ci = column_index(t, name)?;
        if let Some(n) = new_name {
            check_column_name_free(t, n, Some(ci))?;
        }
        let dax = match new_name {
            Some(n) => rename_dax(
                m,
                update_dax,
                |m| {
                    let t = m.tables.get(ti)?;
                    let c = t.columns.get(ci)?;
                    (c.name != n).then(|| ObjectRef::Column {
                        table: t.name.clone(),
                        name: c.name.clone(),
                    })
                },
                n,
            ),
            None => DaxRename::default(),
        };
        // A data column turned into `dateTime`: the date/time type its query declares.
        let date_kind = match m.tables.get(ti).and_then(|t| t.columns.get(ci).map(|c| (t, c))) {
            Some((t, c)) if data_type == Some(DataType::DateTime) && c.expression.is_none() => {
                let sc = source_column.or(c.source_column.as_deref()).unwrap_or(&c.name);
                crate::desktop_defaults::source_date_kind(m, t, sc)
            }
            _ => None,
        };
        let t = table_at(m, ti)?;
        let table_name = t.name.clone();
        let c = t
            .columns
            .get_mut(ci)
            .ok_or_else(|| "internal: column index out of range".to_string())?;
        let old_name = c.name.clone();
        if expression.is_some() && c.expression.is_none() {
            return Err(format!(
                "{} is a data column; remove it and add a calculated column instead",
                dax_column(&table_name, &old_name)
            ));
        }
        if source_column.is_some() && c.expression.is_some() {
            return Err(format!(
                "{} is a calculated column; it has no source column",
                dax_column(&table_name, &old_name)
            ));
        }
        if let Some(n) = new_name {
            if c.name != n {
                // A calculated-table column renamed by hand gets an explicit name (as the engine
                // does).
                c.extra.shift_remove("isNameInferred");
            }
            c.name = n.to_string();
        }
        let old_type = c.data_type;
        if let Some(d) = data_type {
            c.data_type = d;
            if c.expression.is_none() {
                crate::desktop_defaults::data_type_changed(c, old_type, format_string.is_some(), date_kind);
            }
        }
        if let Some(e) = expression {
            c.expression = Some(e.to_string());
        }
        if let Some(sc) = source_column {
            c.source_column = Some(sc.to_string());
        }
        if let Some(v) = format_string {
            c.format_string = v;
        }
        if let Some(v) = display_folder {
            c.display_folder = v;
        }
        if let Some(v) = summarize_by {
            c.summarize_by = v;
        }
        if let Some(v) = description {
            c.description = v;
        }
        if let Some(h) = hidden {
            c.is_hidden = h;
        }
        let current_name = c.name.clone();
        let mut edit = Edit::new(json!({"table": table_name, "column": current_name}));
        dax.record(&mut edit);
        if let Some(e) = expression {
            edit.check(
                format!("calculated column {}", dax_column(&table_name, &current_name)),
                e,
                Some(&table_name),
            );
        }
        if new_name.is_some() && current_name != old_name {
            edit.result.insert("renamed".into(), json!(old_name));
            edit.report_broken = true;
            edit.report_change = Some(ModelChange::ColumnRenamed {
                table: table_name.clone(),
                old: old_name.clone(),
                new: current_name.clone(),
            });
            for other in &mut t.columns {
                if other
                    .sort_by_column
                    .as_deref()
                    .is_some_and(|x| x.eq_ignore_ascii_case(&old_name))
                {
                    other.sort_by_column = Some(current_name.clone());
                }
            }
            rename_column_in_hierarchies(t, &old_name, &current_name);
            let followed = follow_names(
                m,
                NameChange::Column {
                    table: &table_name,
                    old: &old_name,
                    new: &current_name,
                },
            );
            edit.names_followed(followed);
            for r in &mut m.relationships {
                if r.from_table.eq_ignore_ascii_case(&table_name)
                    && r.from_column.eq_ignore_ascii_case(&old_name)
                {
                    r.from_column = current_name.clone();
                }
                if r.to_table.eq_ignore_ascii_case(&table_name) && r.to_column.eq_ignore_ascii_case(&old_name)
                {
                    r.to_column = current_name.clone();
                }
            }
            let followers = crate::rename_inferred::follow_inferred(
                m,
                NameChange::Column {
                    table: &table_name,
                    old: &old_name,
                    new: &current_name,
                },
                update_dax,
            );
            edit.inferred(followers, update_dax);
        }
        Ok(edit)
    })
}

pub(crate) fn column_remove(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["table", "name", "force"])?;
    let table = str_param(p, "table")?;
    let name = str_param(p, "name")?;
    let (actual_table, actual) = {
        let m = model(s)?;
        let t = m
            .tables
            .get(table_index(m, table)?)
            .ok_or_else(|| "internal: table index out of range".to_string())?;
        let c = t
            .columns
            .get(column_index(t, name)?)
            .ok_or_else(|| "internal: column index out of range".to_string())?;
        (t.name.clone(), c.name.clone())
    };
    let used_by = check_removal(
        s,
        p,
        &format!("column {}", dax_column(&actual_table, &actual)),
        &ObjectRef::Column {
            table: actual_table.clone(),
            name: actual.clone(),
        },
    )?;
    apply(s, p, |m| {
        let (mut result, change) = remove_column_in(m, table, name)?;
        result.insert("used_by".into(), json!(used_by));
        let mut edit = Edit::new(Value::Object(result)).breaking();
        edit.report_change = Some(change);
        Ok(edit)
    })
}

/// The mutation of `column.remove`, without its checks: the column, the sort-by settings and
/// hierarchy levels that name it and its relationships; a calculated table's expression selects
/// the other columns. Also used by `model.remove_unused`.
pub(crate) fn remove_column_in(
    m: &mut SemanticModel,
    table: &str,
    name: &str,
) -> Result<(Map<String, Value>, ModelChange), String> {
    let ti = table_index(m, table)?;
    let t = table_at(m, ti)?;
    let ci = column_index(t, name)?;
    // A calculated table infers its columns from its expression: select the others.
    let rewritten = crate::hierarchy::calculated_table_without(t, name);
    let column = t.columns.remove(ci);
    if let (Some(e), Some(p)) = (&rewritten, t.partitions.first_mut()) {
        p.expression = Some(e.clone());
    }
    let table_name = t.name.clone();
    for other in &mut t.columns {
        if other
            .sort_by_column
            .as_deref()
            .is_some_and(|x| x.eq_ignore_ascii_case(&column.name))
        {
            other.sort_by_column = None;
        }
    }
    remove_column_from_hierarchies(t, &column.name);
    let relationships = remove_relationships(m, &table_name, Some(&column.name));
    let mut out = Map::new();
    out.insert("table".into(), json!(table_name));
    out.insert("removed".into(), json!(column.name));
    out.insert("removed_relationships".into(), json!(relationships));
    out.insert("expression_rewritten".into(), json!(rewritten));
    Ok((
        out,
        ModelChange::ColumnRemoved {
            table: table_name,
            column: column.name,
        },
    ))
}

// ---------------------------------------------------------------------------------------------
// measure.*
// ---------------------------------------------------------------------------------------------

pub(crate) fn measure_add(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "table",
            "name",
            "expression",
            "format_string",
            "display_folder",
            "description",
            "hidden",
            "force",
        ],
    )?;
    let table = str_param(p, "table")?;
    let name = str_param(p, "name")?;
    check_name("measure", name)?;
    let expression = non_empty("expression", str_param(p, "expression")?)?;
    let measure = Measure {
        name: name.to_string(),
        expression: expression.to_string(),
        format_string: opt_str(p, "format_string")?.map(str::to_string),
        display_folder: opt_str(p, "display_folder")?.map(str::to_string),
        description: opt_str(p, "description")?.map(str::to_string),
        lineage_tag: Some(new_guid()),
        is_hidden: opt_bool(p, "hidden")?.unwrap_or(false),
        ..Default::default()
    };
    apply(s, p, |m| {
        let ti = table_index(m, table)?;
        check_measure_name_free(m, ti, name, None)?;
        let t = table_at(m, ti)?;
        let mut edit = Edit::new(json!({"table": t.name, "measure": name}));
        edit.check(
            format!("measure {}", dax_measure(name)),
            expression,
            Some(&t.name),
        );
        t.measures.push(measure);
        Ok(edit)
    })
}

pub(crate) fn measure_set(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "table",
            "name",
            "new_name",
            "expression",
            "format_string",
            "display_folder",
            "description",
            "hidden",
            "force",
            "update_report",
            "update_dax",
        ],
    )?;
    let table = opt_str(p, "table")?;
    let name = str_param(p, "name")?;
    let new_name = opt_str(p, "new_name")?;
    if let Some(n) = new_name {
        check_name("measure", n)?;
    }
    let update_dax = update_dax(p)?;
    let expression = opt_str(p, "expression")?
        .map(|e| non_empty("expression", e))
        .transpose()?;
    let format_string = change_str(p, "format_string")?;
    let display_folder = change_str(p, "display_folder")?;
    let description = change_str(p, "description")?;
    let hidden = opt_bool(p, "hidden")?;
    let changes = [
        new_name.is_some(),
        expression.is_some(),
        format_string.is_some(),
        display_folder.is_some(),
        description.is_some(),
        hidden.is_some(),
    ];
    if !changes.contains(&true) {
        return Err("measure.set needs at least one property to change".into());
    }
    apply(s, p, |m| {
        let (ti, mi) = measure_position(m, name)?;
        if let Some(n) = new_name {
            check_measure_name_free(m, ti, n, Some((ti, mi)))?;
        }
        let t = table_at(m, ti)?;
        if let Some(expected) = table
            && !t.name.eq_ignore_ascii_case(expected)
        {
            return Err(format!(
                "measure `{name}` is in table `{}`, not `{expected}`",
                t.name
            ));
        }
        let dax = match new_name {
            Some(n) => rename_dax(
                m,
                update_dax,
                |m| {
                    let t = m.tables.get(ti)?;
                    let x = t.measures.get(mi)?;
                    (x.name != n).then(|| ObjectRef::Measure {
                        table: t.name.clone(),
                        name: x.name.clone(),
                    })
                },
                n,
            ),
            None => DaxRename::default(),
        };
        let t = table_at(m, ti)?;
        let table_name = t.name.clone();
        let x = t
            .measures
            .get_mut(mi)
            .ok_or_else(|| "internal: measure index out of range".to_string())?;
        let old_name = x.name.clone();
        if let Some(n) = new_name {
            x.name = n.to_string();
        }
        if let Some(e) = expression {
            x.expression = e.to_string();
        }
        if let Some(v) = format_string {
            x.format_string = v;
        }
        if let Some(v) = display_folder {
            x.display_folder = v;
        }
        if let Some(v) = description {
            x.description = v;
        }
        if let Some(h) = hidden {
            x.is_hidden = h;
        }
        let mut edit = Edit::new(json!({"table": table_name, "measure": x.name}));
        dax.record(&mut edit);
        if let Some(e) = expression {
            edit.check(format!("measure {}", dax_measure(&x.name)), e, Some(&table_name));
        }
        if x.name != old_name {
            let new_name = x.name.clone();
            edit.result.insert("renamed".into(), json!(old_name));
            edit.report_broken = true;
            let followed = follow_names(
                m,
                NameChange::Measure {
                    table: &table_name,
                    old: &old_name,
                    new: &new_name,
                },
            );
            edit.names_followed(followed);
            edit.report_change = Some(ModelChange::MeasureRenamed {
                table: table_name.clone(),
                old: old_name,
                new: new_name,
            });
        }
        Ok(edit)
    })
}

/// `measure.rename`: `measure.set` with only a new name.
pub(crate) fn measure_rename(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "new_name", "table", "update_dax", "update_report"])?;
    str_param(p, "new_name")?;
    measure_set(s, p)
}

/// `column.rename`: `column.set` with only a new name.
pub(crate) fn column_rename(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["table", "name", "new_name", "update_dax", "update_report"])?;
    str_param(p, "new_name")?;
    column_set(s, p)
}

pub(crate) fn measure_remove(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "force"])?;
    let name = str_param(p, "name")?;
    let (actual_table, actual) = {
        let m = model(s)?;
        let (ti, mi) = measure_position(m, name)?;
        let t = m
            .tables
            .get(ti)
            .ok_or_else(|| "internal: table index out of range".to_string())?;
        let x = t
            .measures
            .get(mi)
            .ok_or_else(|| "internal: measure index out of range".to_string())?;
        (t.name.clone(), x.name.clone())
    };
    let used_by = check_removal(
        s,
        p,
        &format!("measure {}", dax_measure(&actual)),
        &ObjectRef::Measure {
            table: actual_table,
            name: actual,
        },
    )?;
    apply(s, p, |m| {
        let (mut result, change) = remove_measure_in(m, name)?;
        result.insert("used_by".into(), json!(used_by));
        let mut edit = Edit::new(Value::Object(result)).breaking();
        edit.report_change = Some(change);
        Ok(edit)
    })
}

/// The mutation of `measure.remove`, without its checks. Also used by `model.remove_unused`.
pub(crate) fn remove_measure_in(
    m: &mut SemanticModel,
    name: &str,
) -> Result<(Map<String, Value>, ModelChange), String> {
    let (ti, mi) = measure_position(m, name)?;
    let t = table_at(m, ti)?;
    if mi >= t.measures.len() {
        return Err("internal: measure index out of range".into());
    }
    let x = t.measures.remove(mi);
    let mut out = Map::new();
    out.insert("table".into(), json!(t.name));
    out.insert("removed".into(), json!(x.name));
    Ok((
        out,
        ModelChange::MeasureRemoved {
            table: t.name.clone(),
            measure: x.name,
        },
    ))
}

pub(crate) fn measure_move(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "to_table", "force", "update_report", "update_dax"])?;
    let name = str_param(p, "name")?;
    let to_table = str_param(p, "to_table")?;
    let update_dax = update_dax(p)?;
    apply(s, p, |m| {
        let (ti, mi) = measure_position(m, name)?;
        let target = table_index(m, to_table)?;
        check_measure_name_free(m, target, name, Some((ti, mi)))?;
        let to_name = table_at(m, target)?.name.clone();
        let dax = change_dax(
            m,
            update_dax,
            |m| {
                let t = m.tables.get(ti)?;
                let x = t.measures.get(mi)?;
                (ti != target).then(|| ObjectRef::Measure {
                    table: t.name.clone(),
                    name: x.name.clone(),
                })
            },
            DaxChange::MoveMeasure(to_name.clone()),
        );
        let source = table_at(m, ti)?;
        if mi >= source.measures.len() {
            return Err("internal: measure index out of range".into());
        }
        let x = source.measures.remove(mi);
        let from = source.name.clone();
        let followed = if ti == target {
            Vec::new()
        } else {
            follow_names(
                m,
                NameChange::MeasureMoved {
                    name: &x.name,
                    from: &from,
                    to: &to_name,
                },
            )
        };
        let t = table_at(m, target)?;
        let mut edit =
            Edit::new(json!({"measure": x.name, "from_table": from, "to_table": t.name})).breaking();
        edit.names_followed(followed);
        dax.record(&mut edit);
        if !from.eq_ignore_ascii_case(&t.name) {
            edit.report_change = Some(ModelChange::MeasureMoved {
                measure: x.name.clone(),
                from: from.clone(),
                to: t.name.clone(),
            });
        }
        edit.check(
            format!("measure {}", dax_measure(&x.name)),
            &x.expression,
            Some(&t.name),
        );
        t.measures.push(x);
        Ok(edit)
    })
}

// ---------------------------------------------------------------------------------------------
// relationship.*
// ---------------------------------------------------------------------------------------------

/// Parses `Table[Column]` or `'Table Name'[Column]` (DAX quoting: `''` and `]]` escape).
pub(crate) fn parse_column_ref(text: &str) -> Result<(String, String), String> {
    let bad = || format!("`{text}` is not a column reference like Sales[Amount] or 'Order Lines'[Qty]");
    let s = text.trim();
    let (table, rest) = if let Some(body) = s.strip_prefix('\'') {
        let mut table = String::new();
        let mut chars = body.char_indices().peekable();
        let mut end = None;
        while let Some((i, c)) = chars.next() {
            if c == '\'' {
                if chars.peek().is_some_and(|(_, n)| *n == '\'') {
                    chars.next();
                    table.push('\'');
                } else {
                    end = Some(i + 1);
                    break;
                }
            } else {
                table.push(c);
            }
        }
        let end = end.ok_or_else(bad)?;
        (table, body.get(end..).ok_or_else(bad)?)
    } else {
        let open = s.find('[').ok_or_else(bad)?;
        (
            s.get(..open).ok_or_else(bad)?.trim().to_string(),
            s.get(open..).ok_or_else(bad)?,
        )
    };
    let column = rest
        .trim_start()
        .strip_prefix('[')
        .and_then(|r| r.strip_suffix(']'))
        .ok_or_else(bad)?
        .replace("]]", "]");
    if table.is_empty() || column.is_empty() {
        return Err(bad());
    }
    Ok((table, column))
}

/// The refusal of a one-to-one relationship with single-direction cross filtering.
pub(crate) const ONE_TO_ONE_SINGLE: &str = "a one-to-one relationship cross filters in both directions (Power BI Desktop does not open a \
     file with a single-direction one): leave cross_filter out or pass \"both\"";

pub(crate) fn relationship_remove(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "from", "to"])?;
    let name = opt_str(p, "name")?;
    let from = opt_str(p, "from")?;
    let to = opt_str(p, "to")?;
    apply(s, p, |m| {
        let index = match (name, from, to) {
            (Some(n), None, None) => m
                .relationships
                .iter()
                .position(|r| r.name.eq_ignore_ascii_case(n))
                .ok_or_else(|| format!("no relationship named `{n}`"))?,
            (None, Some(f), Some(t)) => {
                let (ft, fc) = parse_column_ref(f)?;
                let (tt, tc) = parse_column_ref(t)?;
                let matches: Vec<usize> = m
                    .relationships
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| {
                        r.from_table.eq_ignore_ascii_case(&ft)
                            && r.from_column.eq_ignore_ascii_case(&fc)
                            && r.to_table.eq_ignore_ascii_case(&tt)
                            && r.to_column.eq_ignore_ascii_case(&tc)
                    })
                    .map(|(i, _)| i)
                    .collect();
                match matches.as_slice() {
                    [i] => *i,
                    [] => return Err(format!("no relationship from {f} to {t}")),
                    _ => {
                        return Err(format!(
                            "several relationships go from {f} to {t}; remove one by name"
                        ));
                    }
                }
            }
            _ => return Err("pass either `name`, or both `from` and `to`".into()),
        };
        if index >= m.relationships.len() {
            return Err("internal: relationship index out of range".into());
        }
        let r = m.relationships.remove(index);
        Ok(Edit::new(json!({
            "removed": r.name,
            "from": dax_column(&r.from_table, &r.from_column),
            "to": dax_column(&r.to_table, &r.to_column),
        }))
        .breaking())
    })
}

// ---------------------------------------------------------------------------------------------
// role.*
// ---------------------------------------------------------------------------------------------

fn role_index(m: &SemanticModel, name: &str) -> Result<usize, String> {
    m.roles
        .iter()
        .position(|r| r.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| format!("no role named `{name}`"))
}

pub(crate) fn role_add(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "model_permission"])?;
    let name = str_param(p, "name")?;
    crate::role_manage::check_role_name(name)?;
    let permission = parse_model_permission(opt_str(p, "model_permission")?.unwrap_or("read"))?;
    apply(s, p, |m| {
        if let Some(r) = m.roles.iter().find(|r| r.name.eq_ignore_ascii_case(name)) {
            return Err(format!("a role named `{}` already exists", r.name));
        }
        m.roles.push(Role {
            name: name.to_string(),
            model_permission: Some(permission.clone()),
            ..Default::default()
        });
        Ok(Edit::new(json!({"role": name, "model_permission": permission})))
    })
}

pub(crate) fn role_set_filter(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["role", "table", "filter_expression", "force"])?;
    let role = str_param(p, "role")?;
    let table = str_param(p, "table")?;
    let filter = match p.get("filter_expression") {
        None => return Err("missing parameter `filter_expression` (null or \"\" removes the filter)".into()),
        Some(Value::Null) => None,
        Some(Value::String(f)) if f.trim().is_empty() => None,
        Some(Value::String(f)) => Some(f.as_str()),
        Some(_) => return Err("parameter `filter_expression` must be a string or null".into()),
    };
    apply(s, p, |m| {
        let ti = table_index(m, table)?;
        let table_name = m.tables.get(ti).map(|t| t.name.clone()).unwrap_or_default();
        let ri = role_index(m, role)?;
        let r = m
            .roles
            .get_mut(ri)
            .ok_or_else(|| "internal: role index out of range".to_string())?;
        let mut edit = Edit::new(json!({"role": r.name, "table": table_name, "filter_expression": filter}));
        let existing = r
            .table_permissions
            .iter()
            .position(|tp| tp.table.eq_ignore_ascii_case(&table_name));
        match (filter, existing) {
            (Some(f), Some(i)) => {
                if let Some(tp) = r.table_permissions.get_mut(i) {
                    tp.table = table_name.clone();
                    tp.filter_expression = Some(f.to_string());
                }
            }
            (Some(f), None) => r.table_permissions.push(TablePermission {
                table: table_name.clone(),
                filter_expression: Some(f.to_string()),
                ..Default::default()
            }),
            (None, Some(i)) => {
                // Keep the permission when it carries other settings (e.g. metadataPermission).
                let keep = r.table_permissions.get(i).is_some_and(|tp| !tp.extra.is_empty());
                if keep {
                    if let Some(tp) = r.table_permissions.get_mut(i) {
                        tp.filter_expression = None;
                    }
                } else {
                    r.table_permissions.remove(i);
                }
            }
            (None, None) => {}
        }
        if let Some(f) = filter {
            edit.check(
                format!("row filter of role `{}` on {}", r.name, dax_table(&table_name)),
                f,
                Some(&table_name),
            );
        }
        Ok(edit)
    })
}

pub(crate) fn role_remove(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name"])?;
    let name = str_param(p, "name")?;
    apply(s, p, |m| {
        let i = role_index(m, name)?;
        let r = m.roles.remove(i);
        Ok(Edit::new(json!({"removed": r.name})))
    })
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn column_references_parse_with_dax_quoting() {
        assert_eq!(
            parse_column_ref("Sales[Amount]").unwrap(),
            ("Sales".into(), "Amount".into())
        );
        assert_eq!(
            parse_column_ref(" 'Order Lines'[Qty] ").unwrap(),
            ("Order Lines".into(), "Qty".into())
        );
        assert_eq!(
            parse_column_ref("'It''s'[a]]b]").unwrap(),
            ("It's".into(), "a]b".into())
        );
        for bad in [
            "Sales",
            "[Amount]",
            "Sales[]",
            "'Sales[Amount]",
            "Sales Amount]",
            "'S'x",
        ] {
            assert!(parse_column_ref(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn guids_are_well_formed_and_distinct() {
        let a = new_guid();
        let b = new_guid();
        assert_ne!(a, b);
        assert_eq!(a.len(), 36);
        let parts: Vec<_> = a.split('-').map(str::len).collect();
        assert_eq!(parts, [8, 4, 4, 4, 12]);
        assert_eq!(a.as_bytes()[14], b'4');
        assert!(matches!(a.as_bytes()[19], b'8' | b'9' | b'a' | b'b'));
    }

    #[test]
    fn dax_names_are_quoted_when_needed() {
        assert_eq!(dax_table("Sales"), "Sales");
        assert_eq!(dax_table("Order Lines"), "'Order Lines'");
        assert_eq!(dax_table("It's"), "'It''s'");
        assert_eq!(dax_column("Sales", "a]b"), "Sales[a]]b]");
    }

    #[test]
    fn names_are_checked() {
        assert!(check_name("table", "Sales").is_ok());
        for bad in ["", "  ", " Sales", "Sales ", "a\nb"] {
            assert!(check_name("table", bad).is_err(), "{bad:?}");
        }
        assert!(check_name("table", &"x".repeat(MAX_NAME_CHARS + 1)).is_err());
    }
}
