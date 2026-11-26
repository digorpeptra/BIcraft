//! Report-level measures (G14): the measures a report adds to the model's tables, in PBIR
//! `definition/reportExtensions.json` (a raw resource of the report) or in a legacy layout's
//! `config.modelExtensions` (kept verbatim among the layout's unmapped keys).
//!
//! The documents stay where the readers put them, byte for byte, until a `report_measure.*`
//! command edits them; this module reads them on demand, validates their DAX against the model
//! (with the other report measures in scope), resolves `[Name]` / `Table[Name]` field strings to
//! them, and edits them.
//!
//! A visual binds a report measure as
//! `{"Measure": {"Expression": {"SourceRef": {"Schema": "extension", "Entity": T}}, "Property": N}}`
//! (a `FieldRef::Other`).

use bicraft_dax::types::{ScalarType, ValueType};
use bicraft_dax::{ExprKind, Severity, Validation, Validator};
use bicraft_model::{Measure, SemanticModel, Table};
use bicraft_report::{FieldRef, Report, Resource};
use serde_json::{Map, Value, json};

use crate::Session;
use crate::edit::{check_name, dax_measure};
use crate::params::{allow, change_str, force, opt_bool, opt_str, str_param};
use crate::refs::{ParsedField, SiteKind, find_refs, parse_field, raw_documents_ref};
use crate::report::parts;

/// Where PBIR keeps report measures (relative to the report root).
pub(crate) const EXTENSIONS_PATH: &str = "definition/reportExtensions.json";
/// The legacy layout's key among its unmapped `config` keys.
const LEGACY_KEY: &str = "config.modelExtensions";
const SCHEMA: bicraft_versions::Schema = bicraft_versions::REPORT_EXTENSION;
/// The extension name Desktop writes (and `SourceRef.Schema` uses).
const EXTENSION_NAME: &str = "extension";
/// Most report measures read from one report.
const MAX_MEASURES: usize = 10_000;
/// Most DAX error messages one issue quotes.
const MAX_DIAGNOSTICS: usize = 3;
/// `dataType` values of the PBIR schema (`PrimitiveTypeName`).
const DATA_TYPES: &[&str] = &[
    "Binary",
    "Boolean",
    "Date",
    "DateTime",
    "DateTimeZone",
    "Decimal",
    "Double",
    "Duration",
    "Integer",
    "Json",
    "None",
    "Null",
    "Text",
    "Time",
    "Variant",
];

/// Which document holds the report measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Store {
    Pbir,
    Legacy,
}

impl Store {
    fn label(self) -> &'static str {
        match self {
            Store::Pbir => "reportExtensions.json",
            Store::Legacy => "legacy config.modelExtensions",
        }
    }
}

/// One report-level measure.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReportMeasure {
    /// The extension's name (`SourceRef.Schema` of the bindings).
    pub schema: String,
    /// The model table it extends.
    pub entity: String,
    pub name: String,
    pub expression: String,
    /// The measure object as stored.
    pub raw: Value,
}

impl ReportMeasure {
    fn id(&self) -> String {
        format!("{}{}", quote_table(&self.entity), dax_measure(&self.name))
    }
}

fn quote_table(t: &str) -> String {
    format!("'{}'", t.replace('\'', "''"))
}

/// The document holding report measures and where it is (`Ok(None)`: the report has none).
fn load(r: &Report) -> Result<Option<(Store, Value)>, String> {
    if let Some(res) = r.resources.iter().find(|x| x.path == EXTENSIONS_PATH) {
        let text = std::str::from_utf8(&res.bytes).map_err(|_| format!("{EXTENSIONS_PATH} is not UTF-8"))?;
        let doc: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|e| format!("{EXTENSIONS_PATH} is not valid JSON: {e}"))?;
        if !doc.is_object() {
            return Err(format!("{EXTENSIONS_PATH} is not a JSON object"));
        }
        return Ok(Some((Store::Pbir, doc)));
    }
    let legacy = r
        .extra
        .get(bicraft_layout::UNMAPPED_KEY)
        .and_then(|u| u.get(LEGACY_KEY));
    match legacy {
        None | Some(Value::Null) => Ok(None),
        Some(v @ Value::Array(_)) => Ok(Some((Store::Legacy, v.clone()))),
        Some(Value::String(s)) => serde_json::from_str::<Value>(s)
            .ok()
            .filter(Value::is_array)
            .map(|v| Some((Store::Legacy, v)))
            .ok_or_else(|| "the legacy config.modelExtensions is not a JSON array".to_string()),
        Some(_) => Err("the legacy config.modelExtensions is not a JSON array".into()),
    }
}

/// The extension objects of a document: the PBIR document itself, or each legacy entry.
fn extensions(store: Store, doc: &Value) -> Vec<&Value> {
    match store {
        Store::Pbir => vec![doc],
        Store::Legacy => doc.as_array().map(|a| a.iter().collect()).unwrap_or_default(),
    }
}

fn extensions_mut(store: Store, doc: &mut Value) -> Vec<&mut Value> {
    match store {
        Store::Pbir => vec![doc],
        Store::Legacy => doc
            .as_array_mut()
            .map(|a| a.iter_mut().collect())
            .unwrap_or_default(),
    }
}

fn measures_of(store: Store, doc: &Value) -> Vec<ReportMeasure> {
    let mut out = Vec::new();
    for ext in extensions(store, doc) {
        let schema = ext
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(EXTENSION_NAME)
            .to_string();
        for entity in ext
            .get("entities")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(table) = entity
                .get("name")
                .or_else(|| entity.get("extends"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            for m in entity
                .get("measures")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if out.len() >= MAX_MEASURES {
                    return out;
                }
                let Some(name) = m.get("name").and_then(Value::as_str) else {
                    continue;
                };
                out.push(ReportMeasure {
                    schema: schema.clone(),
                    entity: table.to_string(),
                    name: name.to_string(),
                    expression: m
                        .get("expression")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    raw: m.clone(),
                });
            }
        }
    }
    out
}

/// The report's measures (empty when it has none or their document cannot be read).
pub(crate) fn report_measures(r: &Report) -> Vec<ReportMeasure> {
    match load(r) {
        Ok(Some((store, doc))) => measures_of(store, &doc),
        _ => Vec::new(),
    }
}

/// The binding of a report measure.
pub(crate) fn binding(m: &ReportMeasure) -> FieldRef {
    FieldRef::Other {
        raw: json!({"Measure": {
            "Expression": {"SourceRef": {"Schema": m.schema, "Entity": m.entity}},
            "Property": m.name,
        }}),
    }
}

fn find<'a>(measures: &'a [ReportMeasure], table: Option<&str>, name: &str) -> Option<&'a ReportMeasure> {
    measures
        .iter()
        .find(|m| m.name.eq_ignore_ascii_case(name) && table.is_none_or(|t| m.entity.eq_ignore_ascii_case(t)))
}

/// Resolves a `[Name]` or `Table[Name]` field string to a report measure's binding.
pub(crate) fn resolve(measures: &[ReportMeasure], text: &str) -> Option<FieldRef> {
    let m = match parse_field(text).ok()? {
        ParsedField::Measure(name) => find(measures, None, &name),
        ParsedField::Qualified { table, name } => find(measures, Some(&table), &name),
        ParsedField::Aggregation { .. } => None,
    }?;
    Some(binding(m))
}

/// The model with the report measures added to their tables (a table the model lacks is added
/// empty), so they can be validated together.
fn extended_model(model: &SemanticModel, measures: &[ReportMeasure]) -> SemanticModel {
    let mut ext = model.clone();
    for m in measures {
        if ext.measure(&m.name).is_some() {
            continue;
        }
        let measure = Measure {
            name: m.name.clone(),
            expression: m.expression.clone(),
            ..Default::default()
        };
        match ext.table_mut(&m.entity) {
            Some(t) => t.measures.push(measure),
            None => ext.tables.push(Table {
                name: m.entity.clone(),
                measures: vec![measure],
                ..Default::default()
            }),
        }
    }
    ext
}

/// Validates every report measure's DAX against the model and the other report measures.
fn validate_all(model: &SemanticModel, measures: &[ReportMeasure]) -> Vec<Validation> {
    let ext = extended_model(model, measures);
    let mut v = Validator::new(&ext);
    measures
        .iter()
        .map(|m| v.validate_as(&m.expression, Some(&m.entity), ExprKind::Measure))
        .collect()
}

fn error_text(v: &Validation) -> String {
    let errors: Vec<String> = v
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .take(MAX_DIAGNOSTICS)
        .map(|d| format!("{} (line {}, column {})", d.message, d.line, d.column))
        .collect();
    if errors.is_empty() {
        "it does not parse".into()
    } else {
        errors.join("; ")
    }
}

/// Problems of one report measure against the model (other than its DAX).
fn structural_problems(model: &SemanticModel, measures: &[ReportMeasure], i: usize) -> Vec<String> {
    let Some(m) = measures.get(i) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match model.table(&m.entity) {
        None => out.push(format!("its table `{}` is not in the model", m.entity)),
        Some(t) => {
            if t.column(&m.name).is_some() {
                out.push(format!("table `{}` has a column with the same name", t.name));
            }
        }
    }
    if let Some((t, _)) = model.measure(&m.name) {
        out.push(format!(
            "the model already has a measure with this name (in `{}`)",
            t.name
        ));
    }
    if measures
        .iter()
        .take(i)
        .any(|o| o.name.eq_ignore_ascii_case(&m.name))
    {
        out.push("another report measure has the same name".into());
    }
    out
}

/// A legacy layout's report measures: each `dataType` must be a known numeric code, and the
/// document must have the shape BIcraft converts to `reportExtensions.json` (else a PBIR save
/// leaves them out). Warnings: the legacy layout itself keeps them as they are.
fn legacy_issues(doc: &Value, measures: &[ReportMeasure], out: &mut Vec<(bool, Value, String)>) {
    let mut typed = true;
    for m in measures {
        let code = m.raw.get("dataType").unwrap_or(&Value::Null);
        if bicraft_layout::measure_data_type_name(code).is_none() {
            typed = false;
            out.push((
                false,
                json!({"report_measure": m.id()}),
                format!(
                    "report measure {}: its legacy dataType {code} is not a known type code (0 to 13)",
                    m.id()
                ),
            ));
        }
    }
    if typed && !measures.is_empty() && bicraft_layout::report_extensions(doc).is_none() {
        out.push((
            false,
            json!({"part": "report measures"}),
            "the legacy config.modelExtensions has a shape BIcraft does not convert: saving the report \
             as PBIR leaves its report measures out"
                .into(),
        ));
    }
}

/// Report-measure findings for `report.validate`: `(error, location, message)`.
pub(crate) fn validation_issues(model: Option<&SemanticModel>, r: &Report) -> Vec<(bool, Value, String)> {
    let mut out = Vec::new();
    let measures = match load(r) {
        Ok(Some((store, doc))) => {
            let measures = measures_of(store, &doc);
            if store == Store::Legacy {
                legacy_issues(&doc, &measures, &mut out);
            }
            measures
        }
        Ok(None) => Vec::new(),
        Err(e) => {
            out.push((true, json!({"part": "report measures"}), e));
            Vec::new()
        }
    };
    if let Some(model) = model
        && !measures.is_empty()
    {
        let results = validate_all(model, &measures);
        for (i, (m, v)) in measures.iter().zip(&results).enumerate() {
            let at = json!({"report_measure": m.id()});
            for p in structural_problems(model, &measures, i) {
                out.push((true, at.clone(), format!("report measure {}: {p}", m.id())));
            }
            if !v.valid {
                out.push((
                    true,
                    at,
                    format!(
                        "the DAX of report measure {} does not validate: {}",
                        m.id(),
                        error_text(v)
                    ),
                ));
            }
        }
    }
    // Bindings and filters on report measures that do not exist.
    for (loc, doc) in raw_documents_ref(r) {
        let bookmark = loc.part.starts_with("bookmark");
        let mut missing: Vec<(String, String)> = Vec::new();
        find_refs(doc, &mut |site| {
            if site.extension
                && site.kind == SiteKind::Measure
                && find(&measures, Some(site.entity), site.property).is_none()
                && !missing
                    .iter()
                    .any(|(e, p)| e == site.entity && p == site.property)
            {
                missing.push((site.entity.to_string(), site.property.to_string()));
            }
        });
        for (entity, property) in missing {
            let mut at = loc.to_json();
            if let Some(o) = at.as_object_mut() {
                o.insert(
                    "field".into(),
                    json!(format!("{}{}", quote_table(&entity), dax_measure(&property))),
                );
            }
            out.push((
                !bookmark,
                at,
                format!(
                    "no report measure {} on table `{entity}` (report measures: report_measure.list)",
                    dax_measure(&property)
                ),
            ));
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

fn measure_json(m: &ReportMeasure, store: Store) -> Value {
    let s = |k: &str| m.raw.get(k).cloned().unwrap_or(Value::Null);
    json!({
        "table": m.entity,
        "name": m.name,
        "expression": m.expression,
        "data_type": s("dataType"),
        "format_string": s("formatString"),
        "description": s("description"),
        "display_folder": s("displayFolder"),
        "hidden": m.raw.get("hidden").and_then(Value::as_bool).unwrap_or(false),
        "store": store.label(),
    })
}

pub(crate) fn report_measure_list(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &[])?;
    let r = crate::report::report_ref(s)?;
    let Some((store, doc)) = load(r)? else {
        return Ok(json!({"count": 0, "measures": [], "store": null}));
    };
    let measures = measures_of(store, &doc);
    let results = s.model.as_ref().map(|m| validate_all(m, &measures));
    let list: Vec<Value> = measures
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let mut j = measure_json(m, store);
            if let (Some(o), Some(rs)) = (j.as_object_mut(), &results)
                && let Some(v) = rs.get(i)
            {
                o.insert("valid".into(), json!(v.valid));
                if !v.diagnostics.is_empty() {
                    o.insert("diagnostics".into(), json!(v.diagnostics));
                }
            }
            j
        })
        .collect();
    Ok(json!({
        "count": list.len(),
        "measures": list,
        "store": store.label(),
        "checked_against_model": results.is_some(),
    }))
}

/// The PBIR `dataType` of a validated expression.
fn data_type_of(v: &Validation) -> &'static str {
    match v.result_type {
        Some(ValueType::Scalar(t)) => match t {
            ScalarType::Integer => "Integer",
            ScalarType::Decimal => "Double",
            ScalarType::Currency => "Decimal",
            ScalarType::Boolean => "Boolean",
            ScalarType::String => "Text",
            ScalarType::DateTime => "DateTime",
            ScalarType::Variant => "Variant",
        },
        _ => "Variant",
    }
}

/// The legacy layout's numeric code of PBIR `dataType` `t`.
fn legacy_code(t: &str) -> Result<i64, String> {
    bicraft_layout::measure_data_type_code(t).ok_or_else(|| {
        format!("data_type `{t}` has no known code in a legacy layout; convert the report to PBIR first")
    })
}

fn check_data_type(t: &str) -> Result<&'static str, String> {
    DATA_TYPES
        .iter()
        .find(|d| d.eq_ignore_ascii_case(t))
        .copied()
        .ok_or_else(|| {
            format!(
                "unknown data_type `{t}` (expected one of {})",
                DATA_TYPES.join(", ")
            )
        })
}

/// Validates `candidate` (replacing the measure at `replace`, or added) with the others in scope.
/// `Err` when the DAX is invalid and `force` is off; the validation otherwise.
fn validate_candidate(
    model: Option<&SemanticModel>,
    mut measures: Vec<ReportMeasure>,
    replace: Option<usize>,
    candidate: ReportMeasure,
    force: bool,
    warnings: &mut Vec<String>,
) -> Result<Option<Validation>, String> {
    let Some(model) = model else {
        warnings.push("no model is open: the DAX was not validated".into());
        return Ok(None);
    };
    let id = candidate.id();
    let i = match replace.filter(|&i| i < measures.len()) {
        Some(i) => {
            if let Some(slot) = measures.get_mut(i) {
                *slot = candidate;
            }
            i
        }
        None => {
            measures.push(candidate);
            measures.len().saturating_sub(1)
        }
    };
    for p in structural_problems(model, &measures, i) {
        if force {
            warnings.push(format!("report measure {id}: {p} (force)"));
        } else {
            return Err(format!("report measure {id}: {p}"));
        }
    }
    let v = validate_all(model, &measures)
        .into_iter()
        .nth(i)
        .ok_or("internal: validation result missing")?;
    if !v.valid {
        let e = format!(
            "the DAX of report measure {id} does not validate: {}",
            error_text(&v)
        );
        if force {
            warnings.push(format!("{e} (kept: force)"));
        } else {
            return Err(format!("{e} (pass \"force\": true to keep it anyway)"));
        }
    }
    Ok(Some(v))
}

/// Writes the document back where it came from (a PBIR document is kept even when it has no
/// measures left, so a project's file is rewritten rather than left stale).
fn store_doc(r: &mut Report, store: Store, doc: Value) -> Result<(), String> {
    match store {
        Store::Pbir => {
            let bytes = serde_json::to_vec_pretty(&doc).map_err(|e| e.to_string())?;
            match r.resources.iter_mut().find(|x| x.path == EXTENSIONS_PATH) {
                Some(res) => res.bytes = bytes,
                None => r.resources.push(Resource {
                    path: EXTENSIONS_PATH.into(),
                    bytes,
                }),
            }
            Ok(())
        }
        Store::Legacy => {
            let unmapped = r
                .extra
                .get_mut(bicraft_layout::UNMAPPED_KEY)
                .and_then(Value::as_object_mut)
                .ok_or("internal: the legacy unmapped keys are missing")?;
            unmapped.insert(LEGACY_KEY.into(), doc);
            Ok(())
        }
    }
}

/// Whether the open report is a legacy layout (it then refuses new PBIR documents).
fn is_legacy(r: &Report) -> bool {
    r.extra
        .keys()
        .any(|k| k.starts_with(bicraft_layout::LEGACY_PREFIX))
}

/// `(extension, entity, measure)` indices of the measure `name` (in `table`, if given).
fn locate(
    store: Store,
    doc: &Value,
    table: Option<&str>,
    name: &str,
) -> Result<(usize, usize, usize), String> {
    let mut hits = Vec::new();
    for (xi, ext) in extensions(store, doc).into_iter().enumerate() {
        for (ei, entity) in ext
            .get("entities")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let tname = entity
                .get("name")
                .or_else(|| entity.get("extends"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            if table.is_some_and(|t| !t.eq_ignore_ascii_case(tname)) {
                continue;
            }
            for (mi, m) in entity
                .get("measures")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
            {
                if m.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
                {
                    hits.push((xi, ei, mi));
                }
            }
        }
    }
    match hits.as_slice() {
        [one] => Ok(*one),
        [] => Err(format!(
            "no report measure {}{} (report_measure.list lists them)",
            dax_measure(name),
            table.map(|t| format!(" on table `{t}`")).unwrap_or_default()
        )),
        _ => Err(format!(
            "several report measures are called {}: pass `table`",
            dax_measure(name)
        )),
    }
}

fn measure_at(store: Store, doc: &mut Value, at: (usize, usize, usize)) -> Option<&mut Map<String, Value>> {
    let (xi, ei, mi) = at;
    extensions_mut(store, doc)
        .into_iter()
        .nth(xi)?
        .get_mut("entities")?
        .as_array_mut()?
        .get_mut(ei)?
        .get_mut("measures")?
        .as_array_mut()?
        .get_mut(mi)?
        .as_object_mut()
}

pub(crate) fn report_measure_add(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "table",
            "name",
            "expression",
            "format_string",
            "data_type",
            "description",
            "display_folder",
            "hidden",
            "force",
        ],
    )?;
    let table = str_param(p, "table")?;
    let name = str_param(p, "name")?;
    check_name("measure", name)?;
    let expression = str_param(p, "expression")?;
    let format_string = opt_str(p, "format_string")?;
    let data_type = opt_str(p, "data_type")?.map(check_data_type).transpose()?;
    let description = opt_str(p, "description")?;
    let display_folder = opt_str(p, "display_folder")?;
    let hidden = opt_bool(p, "hidden")?;
    let force = force(p)?;
    let (model, r) = parts(s)?;
    let loaded = load(r)?;
    if loaded.is_none() && is_legacy(r) {
        return Err(
            "this report is a legacy layout without report measures; convert it to PBIR first \
             (file.save_as with report_format \"pbir\") to add report measures"
                .into(),
        );
    }
    let (store, mut doc) = loaded.unwrap_or_else(|| {
        (
            Store::Pbir,
            json!({"$schema": SCHEMA.url(), "name": EXTENSION_NAME, "entities": []}),
        )
    });
    if store == Store::Legacy && (description.is_some() || display_folder.is_some()) {
        return Err(
            "on a legacy layout a report measure takes no description or display_folder (their \
             legacy form was not observed); convert the report to PBIR first (file.save_as with \
             report_format \"pbir\")"
                .into(),
        );
    }
    let measures = measures_of(store, &doc);
    // The model's spelling of the table.
    let table = model
        .and_then(|m| m.table(table))
        .map_or_else(|| table.to_string(), |t| t.name.clone());
    let schema = extensions(store, &doc)
        .first()
        .and_then(|e| e.get("name"))
        .and_then(Value::as_str)
        .unwrap_or(EXTENSION_NAME)
        .to_string();
    let mut warnings = Vec::new();
    let candidate = ReportMeasure {
        schema,
        entity: table.clone(),
        name: name.to_string(),
        expression: expression.to_string(),
        raw: Value::Null,
    };
    if measures.iter().any(|m| m.name.eq_ignore_ascii_case(name)) {
        return Err(format!("a report measure {} already exists", dax_measure(name)));
    }
    let validation = validate_candidate(model, measures, None, candidate, force, &mut warnings)?;
    let data_type = data_type.unwrap_or_else(|| validation.as_ref().map_or("Variant", data_type_of));
    let mut m = Map::new();
    m.insert("name".into(), json!(name));
    if store == Store::Legacy {
        // the shape of the report measures Desktop writes in a legacy layout
        // (docs/formats/legacy-layout-bb.md, "Report measures"), `dataType` as its numeric code
        m.insert("dataType".into(), json!(legacy_code(data_type)?));
        m.insert("expression".into(), json!(expression));
        m.insert("errorMessage".into(), Value::Null);
        m.insert("hidden".into(), json!(hidden.unwrap_or(false)));
        m.insert("formulaOverride".into(), Value::Null);
        if let Some(f) = format_string {
            m.insert(
                "formatInformation".into(),
                json!({"formatString": f, "format": "General"}),
            );
        }
    } else {
        m.insert("dataType".into(), json!(data_type));
        m.insert("expression".into(), json!(expression));
    }
    if store == Store::Pbir
        && let Some(h) = hidden
    {
        m.insert("hidden".into(), json!(h));
    }
    if store == Store::Pbir
        && let Some(f) = format_string
    {
        m.insert("formatString".into(), json!(f));
    }
    if let Some(d) = description {
        m.insert("description".into(), json!(d));
    }
    if let Some(d) = display_folder {
        m.insert("displayFolder".into(), json!(d));
    }
    let ext = extensions_mut(store, &mut doc)
        .into_iter()
        .next()
        .and_then(Value::as_object_mut)
        .ok_or("internal: no extension object")?;
    let entities = ext
        .entry("entities")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or("reportExtensions.json: `entities` is not an array")?;
    let entity = match entities.iter().position(|e| {
        e.get("name")
            .or_else(|| e.get("extends"))
            .and_then(Value::as_str)
            .is_some_and(|n| n.eq_ignore_ascii_case(&table))
    }) {
        Some(i) => entities.get_mut(i).ok_or("internal: entity index")?,
        None => {
            entities.push(if store == Store::Legacy {
                json!({"name": table, "extends": table, "measures": []})
            } else {
                json!({"name": table, "measures": []})
            });
            entities.last_mut().ok_or("internal: entity index")?
        }
    };
    let list = entity
        .as_object_mut()
        .ok_or("reportExtensions.json: an entity is not an object")?
        .entry("measures")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or("reportExtensions.json: `measures` is not an array")?;
    list.push(Value::Object(m.clone()));
    store_doc(r, store, doc)?;
    s.report_dirty = true;
    Ok(json!({
        "table": table,
        "name": name,
        "data_type": m.get("dataType"),
        "valid": validation.as_ref().map(|v| v.valid),
        "diagnostics": validation.map(|v| v.diagnostics).unwrap_or_default(),
        "store": store.label(),
        "warnings": warnings,
    }))
}

pub(crate) fn report_measure_set(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "table",
            "name",
            "expression",
            "format_string",
            "data_type",
            "description",
            "display_folder",
            "hidden",
            "force",
        ],
    )?;
    let table = opt_str(p, "table")?;
    let name = str_param(p, "name")?;
    let expression = opt_str(p, "expression")?;
    let format_string = change_str(p, "format_string")?;
    let data_type = change_str(p, "data_type")?;
    let description = change_str(p, "description")?;
    let display_folder = change_str(p, "display_folder")?;
    let hidden = opt_bool(p, "hidden")?;
    let force = force(p)?;
    if expression.is_none()
        && [&format_string, &data_type, &description, &display_folder]
            .iter()
            .all(|c| c.is_none())
        && hidden.is_none()
    {
        return Err(
            "report_measure.set needs at least one of expression, format_string, data_type, \
                    description, display_folder, hidden"
                .into(),
        );
    }
    let data_type = match &data_type {
        Some(Some(t)) => Some(Some(check_data_type(t)?)),
        Some(None) => return Err("`data_type` cannot be removed (the PBIR schema requires it)".into()),
        None => None,
    };
    let (model, r) = parts(s)?;
    let (store, mut doc) =
        load(r)?.ok_or("the report has no report measures (report_measure.add adds one)")?;
    if store == Store::Legacy && (description.is_some() || display_folder.is_some()) {
        return Err(
            "on a legacy layout `description` and `display_folder` cannot be changed; convert the \
             report to PBIR (file.save_as with report_format \"pbir\") for them"
                .into(),
        );
    }
    let legacy_type = match (store, &data_type) {
        (Store::Legacy, Some(Some(t))) => Some(legacy_code(t)?),
        _ => None,
    };
    let at = locate(store, &doc, table, name)?;
    let measures = measures_of(store, &doc);
    let index = measures.iter().position(|m| {
        m.name.eq_ignore_ascii_case(name) && table.is_none_or(|t| m.entity.eq_ignore_ascii_case(t))
    });
    let mut warnings = Vec::new();
    let mut validation = None;
    if let (Some(expr), Some(i)) = (expression, index) {
        let mut candidate = measures.get(i).cloned().ok_or("internal: measure index")?;
        candidate.expression = expr.to_string();
        validation = validate_candidate(model, measures.clone(), Some(i), candidate, force, &mut warnings)?;
    }
    let m = measure_at(store, &mut doc, at).ok_or("internal: measure not found")?;
    let mut changed = Vec::new();
    if let Some(e) = expression {
        m.insert("expression".into(), json!(e));
        changed.push("expression");
        if data_type.is_none()
            && let Some(v) = &validation
        {
            match store {
                Store::Pbir => {
                    m.insert("dataType".into(), json!(data_type_of(v)));
                }
                Store::Legacy => {
                    if let Some(code) = bicraft_layout::measure_data_type_code(data_type_of(v)) {
                        m.insert("dataType".into(), json!(code));
                    }
                }
            }
        }
    }
    if let Some(Some(t)) = data_type {
        match legacy_type {
            Some(code) => m.insert("dataType".into(), json!(code)),
            None => m.insert("dataType".into(), json!(t)),
        };
        changed.push("data_type");
    }
    if store == Store::Legacy
        && let Some(change) = &format_string
    {
        match change {
            Some(f) => {
                m.insert(
                    "formatInformation".into(),
                    json!({"formatString": f, "format": "General"}),
                );
            }
            None => {
                m.remove("formatInformation");
            }
        }
        changed.push("format_string");
    }
    let pbir_format = if store == Store::Pbir { format_string } else { None };
    for (key, change, label) in [
        ("formatString", &pbir_format, "format_string"),
        ("description", &description, "description"),
        ("displayFolder", &display_folder, "display_folder"),
    ] {
        match change {
            Some(Some(v)) => {
                m.insert(key.into(), json!(v));
                changed.push(label);
            }
            Some(None) => {
                m.remove(key);
                changed.push(label);
            }
            None => {}
        }
    }
    if let Some(h) = hidden {
        m.insert("hidden".into(), json!(h));
        changed.push("hidden");
    }
    let out_name = m.get("name").cloned().unwrap_or(Value::Null);
    store_doc(r, store, doc)?;
    s.report_dirty = true;
    Ok(json!({
        "name": out_name,
        "changed": changed,
        "valid": validation.as_ref().map(|v| v.valid),
        "diagnostics": validation.map(|v| v.diagnostics).unwrap_or_default(),
        "store": store.label(),
        "warnings": warnings,
    }))
}

pub(crate) fn report_measure_remove(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["table", "name"])?;
    let table = opt_str(p, "table")?;
    let name = str_param(p, "name")?;
    let (_, r) = parts(s)?;
    let (store, mut doc) = load(r)?.ok_or("the report has no report measures")?;
    let (xi, ei, mi) = locate(store, &doc, table, name)?;
    let measures = measures_of(store, &doc);
    let removed = measures
        .iter()
        .find(|m| m.name.eq_ignore_ascii_case(name) && table.is_none_or(|t| m.entity.eq_ignore_ascii_case(t)))
        .cloned()
        .ok_or("internal: measure not found")?;
    {
        let entities = extensions_mut(store, &mut doc)
            .into_iter()
            .nth(xi)
            .and_then(|e| e.get_mut("entities"))
            .and_then(Value::as_array_mut)
            .ok_or("internal: entities not found")?;
        let entity = entities.get_mut(ei).ok_or("internal: entity not found")?;
        let list = entity
            .get_mut("measures")
            .and_then(Value::as_array_mut)
            .ok_or("internal: measures not found")?;
        if mi < list.len() {
            list.remove(mi);
        }
        let now_empty = list.is_empty();
        if now_empty && store == Store::Pbir && ei < entities.len() {
            entities.remove(ei);
        }
    }
    // Where it is still used: visual bindings and filters.
    let mut used = 0usize;
    for (_, d) in raw_documents_ref(r) {
        find_refs(d, &mut |site| {
            if site.extension
                && site.kind == SiteKind::Measure
                && site.entity.eq_ignore_ascii_case(&removed.entity)
                && site.property.eq_ignore_ascii_case(&removed.name)
            {
                used = used.saturating_add(1);
            }
        });
    }
    let others: Vec<String> = measures
        .iter()
        .filter(|m| m.name != removed.name || m.entity != removed.entity)
        .filter(|m| {
            m.expression
                .contains(&format!("[{}]", removed.name.replace(']', "]]")))
        })
        .map(ReportMeasure::id)
        .collect();
    store_doc(r, store, doc)?;
    s.report_dirty = true;
    let mut warnings = Vec::new();
    if used > 0 {
        warnings.push(format!(
            "{used} reference(s) in visuals or filters still use {}; report.validate lists them",
            removed.id()
        ));
    }
    if !others.is_empty() {
        warnings.push(format!("report measures that may use it: {}", others.join(", ")));
    }
    Ok(json!({
        "removed": removed.id(),
        "store": store.label(),
        "warnings": warnings,
    }))
}

// ---------------------------------------------------------------------------------------------
// Model renames and removals (G15)
// ---------------------------------------------------------------------------------------------

/// A rewritten report-measure document, stored by [`commit_doc`].
pub(crate) struct PendingDoc {
    store: Store,
    doc: Value,
}

/// Rewrites the report measures' DAX for a model rename (`target` named as it is before the
/// rename, resolved against `model`, the model before the rename); a renamed table also renames
/// the entities that extend it. `None` when nothing changes; the list has one entry per rewritten
/// measure.
pub(crate) fn rename_references(
    r: &Report,
    model: &SemanticModel,
    target: &bicraft_dax::deps::ObjectRef,
    change: &crate::dax_rename::DaxChange,
) -> (Option<PendingDoc>, Vec<Value>) {
    use crate::dax_rename::DaxChange;
    use bicraft_dax::deps::ObjectRef;
    use bicraft_dax::rename::{Renamer, Rewrite, Site};
    let Ok(Some((store, mut doc))) = load(r) else {
        return (None, Vec::new());
    };
    let renamer = Renamer::new(model);
    let renamed_table = match (target, change) {
        (ObjectRef::Table { name }, DaxChange::Rename(new_name)) => Some((name.as_str(), new_name.as_str())),
        _ => None,
    };
    let mut changed = false;
    let mut listed = Vec::new();
    for ext in extensions_mut(store, &mut doc) {
        let Some(entities) = ext.get_mut("entities").and_then(Value::as_array_mut) else {
            continue;
        };
        for entity in entities {
            let Some(e) = entity.as_object_mut() else { continue };
            let key = if e.contains_key("name") { "name" } else { "extends" };
            let table = e.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
            for m in e
                .get_mut("measures")
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
            {
                let Some(expr) = m.get("expression").and_then(Value::as_str) else {
                    continue;
                };
                let site = Site {
                    home: Some(&table),
                    row_context: false,
                    udf: false,
                };
                if let Rewrite::Rewritten { text, references, .. } =
                    change.rewrite(&renamer, expr, site, target)
                    && references > 0
                    && let Some(o) = m.as_object_mut()
                {
                    listed.push(json!({
                        "kind": "report measure",
                        "table": table,
                        "name": o.get("name").and_then(Value::as_str).unwrap_or_default(),
                        "references": references,
                    }));
                    o.insert("expression".into(), Value::String(text));
                    changed = true;
                }
            }
            if let Some((old, new_name)) = renamed_table
                && table.eq_ignore_ascii_case(old)
            {
                e.insert(key.into(), Value::String(new_name.to_string()));
                changed = true;
            }
        }
    }
    (changed.then_some(PendingDoc { store, doc }), listed)
}

/// Stores a document from [`rename_references`].
pub(crate) fn commit_doc(r: &mut Report, pending: PendingDoc) -> Result<(), String> {
    store_doc(r, pending.store, pending.doc)
}

/// The report measures a removal of `target` would break: those referencing it, and (for a
/// table) those extending it. `(json, label)` pairs.
pub(crate) fn dependants(
    r: &Report,
    model: &SemanticModel,
    target: &bicraft_dax::deps::ObjectRef,
) -> Vec<(Value, String)> {
    use bicraft_dax::deps::ObjectRef;
    use bicraft_dax::rename::{Renamer, Site};
    let renamer = Renamer::new(model);
    report_measures(r)
        .iter()
        .filter(|m| {
            let extends = matches!(target, ObjectRef::Table { name } if name.eq_ignore_ascii_case(&m.entity));
            extends
                || renamer
                    .references(
                        &m.expression,
                        Site {
                            home: Some(&m.entity),
                            row_context: false,
                            udf: false,
                        },
                    )
                    .is_some_and(|refs| refs.iter().any(|x| crate::dax_rename::refers_to(x, target)))
        })
        .map(|m| {
            (
                json!({"kind": "report measure", "table": m.entity, "name": m.name}),
                format!("report measure {}", m.id()),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pbir_report(doc: &Value) -> Report {
        Report {
            resources: vec![Resource {
                path: EXTENSIONS_PATH.into(),
                bytes: serde_json::to_vec(doc).unwrap(),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn reads_pbir_and_legacy_documents() {
        let doc = json!({"$schema": SCHEMA.url(), "name": "extension", "entities": [
            {"name": "Sales", "measures": [{"name": "Local", "dataType": "Double", "expression": "1"}]}]});
        let r = pbir_report(&doc);
        let ms = report_measures(&r);
        assert_eq!(ms.len(), 1);
        assert_eq!((ms[0].entity.as_str(), ms[0].name.as_str()), ("Sales", "Local"));
        assert!(resolve(&ms, "[local]").is_some());
        assert!(resolve(&ms, "Sales[Local]").is_some());
        assert!(resolve(&ms, "Other[Local]").is_none());

        let mut legacy = Report::default();
        let mut unmapped = Map::new();
        unmapped.insert(
            LEGACY_KEY.into(),
            json!([{"name": "extension", "entities": [{"name": "Sales", "extends": "Sales",
                "measures": [{"name": "L", "dataType": 3, "expression": "2"}]}]}]),
        );
        legacy
            .extra
            .insert(bicraft_layout::UNMAPPED_KEY.into(), Value::Object(unmapped));
        let ms = report_measures(&legacy);
        assert_eq!(ms.len(), 1);
        assert_eq!(ms[0].expression, "2");
    }

    #[test]
    fn hostile_documents_never_panic() {
        for bytes in [
            &b"\xff\xfe"[..],
            b"[]",
            b"{\"entities\": 3}",
            b"{\"entities\": [{\"measures\": [1, {\"name\": 2}]}]}",
            b"{\"entities\": [{\"name\": \"T\", \"measures\": {}}]}",
        ] {
            let r = Report {
                resources: vec![Resource {
                    path: EXTENSIONS_PATH.into(),
                    bytes: bytes.to_vec(),
                }],
                ..Default::default()
            };
            let _ = report_measures(&r);
            let _ = validation_issues(Some(&SemanticModel::default()), &r);
        }
    }
}

#[cfg(test)]
mod command_tests {
    use serde_json::{Value, json};

    use crate::Session;

    fn session() -> Session {
        let mut s = Session::new();
        for (id, p) in [
            ("model.new", json!({"name": "Shop"})),
            ("table.add", json!({"name": "Sales"})),
            (
                "column.add",
                json!({"table": "Sales", "name": "Region", "data_type": "string"}),
            ),
            (
                "column.add",
                json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
            ),
            (
                "measure.add",
                json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
            ),
            ("report.new", json!({})),
        ] {
            s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
        }
        s
    }

    fn run(s: &mut Session, id: &str, p: Value) -> Value {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
    }

    #[test]
    fn add_list_bind_set_validate_remove() {
        let mut s = session();
        let out = run(
            &mut s,
            "report_measure.add",
            json!({"table": "sales", "name": "Double", "expression": "[Total] * 2", "format_string": "0.0"}),
        );
        assert_eq!(out["table"], "Sales");
        assert_eq!(out["data_type"], "Double", "{out}");
        assert_eq!(out["valid"], true);
        // Report measures see each other.
        run(
            &mut s,
            "report_measure.add",
            json!({"table": "Sales", "name": "Quad", "expression": "[Double] * 2"}),
        );
        let e = s
            .execute(
                "report_measure.add",
                &json!({"table": "Sales", "name": "Bad", "expression": "[Nope] + 1"}),
            )
            .unwrap_err();
        assert!(e.contains("does not validate"), "{e}");
        let e = s
            .execute(
                "report_measure.add",
                &json!({"table": "Sales", "name": "Total", "expression": "1"}),
            )
            .unwrap_err();
        assert!(e.contains("already has a measure"), "{e}");
        let list = run(&mut s, "report_measure.list", json!({}));
        assert_eq!(list["count"], 2, "{list}");
        assert_eq!(list["measures"][0]["format_string"], "0.0");
        assert_eq!(list["measures"][1]["valid"], true);

        // Visuals bind them by name.
        let v = run(
            &mut s,
            "visual.add",
            json!({"page": "Page 1", "visual_type": "clusteredColumnChart", "x": 0, "y": 0, "width": 300,
                   "height": 200, "fields": [{"role": "Category", "field": "Sales[Region]"},
                   {"role": "Y", "field": "[Double]"}]}),
        );
        let raw = &s.report.as_ref().unwrap().pages[0].visuals[0].fields[1].field;
        assert!(
            serde_json::to_string(raw)
                .unwrap()
                .contains("\"Schema\":\"extension\""),
            "{raw:?}"
        );
        let vname = v["visual"].as_str().unwrap().to_string();
        run(
            &mut s,
            "visual.bind",
            json!({"page": "Page 1", "visual": vname, "role": "Tooltips", "field": "Sales[Quad]"}),
        );
        let val = run(&mut s, "report.validate", json!({}));
        assert_eq!(val["error_count"], 0, "{val}");

        // A broken expression is reported by report.validate.
        run(
            &mut s,
            "report_measure.set",
            json!({"name": "Quad", "expression": "[Gone]", "force": true}),
        );
        let val = run(&mut s, "report.validate", json!({}));
        assert!(val["issues"].to_string().contains("does not validate"), "{val}");
        run(
            &mut s,
            "report_measure.set",
            json!({"name": "Quad", "expression": "[Double] * 4", "format_string": null}),
        );

        // Removing a bound measure warns, and validate reports the dangling binding.
        let out = run(&mut s, "report_measure.remove", json!({"name": "Quad"}));
        assert!(out["warnings"].to_string().contains("still use"), "{out}");
        let val = run(&mut s, "report.validate", json!({}));
        assert!(
            val["issues"].to_string().contains("no report measure [Quad]"),
            "{val}"
        );
        assert_eq!(run(&mut s, "report_measure.list", json!({}))["count"], 1);
    }

    /// The legacy `config.modelExtensions` of the open report.
    fn legacy_doc(s: &Session) -> Value {
        s.report.as_ref().unwrap().extra[bicraft_layout::UNMAPPED_KEY][super::LEGACY_KEY].clone()
    }

    fn set_legacy_doc(s: &mut Session, doc: Value) {
        let mut unmapped = serde_json::Map::new();
        unmapped.insert(super::LEGACY_KEY.into(), doc);
        s.report
            .as_mut()
            .unwrap()
            .extra
            .insert(bicraft_layout::UNMAPPED_KEY.into(), Value::Object(unmapped));
    }

    #[test]
    fn legacy_layouts_take_and_check_report_measures() {
        let mut s = session();
        set_legacy_doc(
            &mut s,
            json!([{"name": "extension", "entities": [{"name": "Sales", "extends": "Sales",
                "measures": [{"name": "Old", "dataType": 3, "expression": "[Total]", "errorMessage": null,
                    "hidden": false, "formulaOverride": null}]}]}]),
        );
        // added in the shape Desktop writes, the type as its numeric code
        let out = run(
            &mut s,
            "report_measure.add",
            json!({"table": "Sales", "name": "Triple", "expression": "[Total] * 3", "format_string": "0.0"}),
        );
        assert_eq!(out["store"], "legacy config.modelExtensions", "{out}");
        let doc = legacy_doc(&s);
        let measures = &doc[0]["entities"][0]["measures"];
        assert_eq!(measures.as_array().map(Vec::len), Some(2), "{doc}");
        assert_eq!(
            measures[1],
            json!({"name": "Triple", "dataType": 3, "expression": "[Total] * 3", "errorMessage": null,
                   "hidden": false, "formulaOverride": null,
                   "formatInformation": {"formatString": "0.0", "format": "General"}})
        );
        // a new table's entity names the table it extends
        run(&mut s, "table.add", json!({"name": "Other"}));
        run(
            &mut s,
            "report_measure.add",
            json!({"table": "Other", "name": "Flag", "expression": "TRUE()"}),
        );
        let doc = legacy_doc(&s);
        assert_eq!(doc[0]["entities"][1]["extends"], "Other", "{doc}");
        assert_eq!(doc[0]["entities"][1]["measures"][0]["dataType"], 5, "{doc}");
        // set: the type and the format string in their legacy places
        run(
            &mut s,
            "report_measure.set",
            json!({"name": "Triple", "data_type": "Integer", "format_string": "#,0"}),
        );
        let m = &legacy_doc(&s)[0]["entities"][0]["measures"][1];
        assert_eq!(m["dataType"], 4, "{m}");
        assert_eq!(m["formatInformation"]["formatString"], "#,0", "{m}");
        // the whole document still converts to PBIR, with the types spelled
        let pbir = bicraft_layout::report_extensions(&legacy_doc(&s)).unwrap();
        assert_eq!(
            pbir["entities"][0]["measures"][1]["dataType"], "Integer",
            "{pbir}"
        );
        assert_eq!(
            pbir["entities"][0]["measures"][1]["formatString"], "#,0",
            "{pbir}"
        );
        for p in [
            json!({"table": "Sales", "name": "D", "expression": "1", "description": "x"}),
            json!({"table": "Sales", "name": "J", "expression": "1", "data_type": "Json"}),
        ] {
            let e = s.execute("report_measure.add", &p).unwrap_err();
            assert!(e.contains("legacy"), "{p}: {e}");
        }
        let val = run(&mut s, "report.validate", json!({}));
        let issues = val["issues"].to_string();
        assert!(!issues.contains("dataType") && !issues.contains("shape"), "{val}");

        // report.validate: an unknown type code, and a document a PBIR save would leave out
        set_legacy_doc(
            &mut s,
            json!([{"name": "extension", "entities": [{"name": "Sales", "extends": "Sales",
                "measures": [{"name": "Odd", "dataType": 99, "expression": "1"}]}]}]),
        );
        let val = run(&mut s, "report.validate", json!({}));
        let issues = val["issues"].to_string();
        assert!(
            issues.contains("legacy dataType 99 is not a known type code"),
            "{val}"
        );
        set_legacy_doc(
            &mut s,
            json!([{"name": "extension", "entities": [{"name": "Sales", "extends": "Sales",
                "measures": [{"name": "Odd", "dataType": 3, "expression": "1", "surprise": 1}]}]}]),
        );
        let val = run(&mut s, "report.validate", json!({}));
        let issues = val["issues"].to_string();
        assert!(issues.contains("saving the report as PBIR leaves"), "{val}");
        assert_eq!(val["error_count"], 0, "{val}");
    }

    #[test]
    fn report_measures_survive_a_pbip_round_trip() {
        let mut s = session();
        run(
            &mut s,
            "report_measure.add",
            json!({"table": "Sales", "name": "Double", "expression": "[Total] * 2"}),
        );
        let tmp = crate::tempdir::TempDir::new("report-measures");
        let dir = tmp.path();
        run(&mut s, "file.save_as", json!({"path": dir.join("Shop.pbip")}));
        let mut fresh = Session::new();
        run(&mut fresh, "file.open", json!({"path": dir.join("Shop.pbip")}));
        let list = run(&mut fresh, "report_measure.list", json!({}));
        assert_eq!(list["count"], 1, "{list}");
        assert_eq!(list["measures"][0]["valid"], true);
        let val = run(&mut fresh, "report.validate", json!({}));
        assert!(val["schema_issues"].as_array().unwrap().is_empty(), "{val}");
    }

    #[test]
    fn model_renames_follow_into_report_measures() {
        let mut s = session();
        run(
            &mut s,
            "report_measure.add",
            json!({"table": "Sales", "name": "Double", "expression": "[Total] * 2 + SUM(Sales[Amount]) // [Total]"}),
        );
        let out = run(
            &mut s,
            "measure.rename",
            json!({"name": "Total", "new_name": "Revenue"}),
        );
        assert_eq!(
            out["dax_rewritten"],
            json!([{"kind": "report measure", "table": "Sales", "name": "Double", "references": 1}]),
            "{out}"
        );
        let out = run(
            &mut s,
            "column.rename",
            json!({"table": "Sales", "name": "Amount", "new_name": "Net"}),
        );
        assert_eq!(out["dax_rewritten"].as_array().map(Vec::len), Some(2), "{out}");
        // The table rename renames the entity the report measure extends, too.
        run(
            &mut s,
            "table.rename",
            json!({"name": "Sales", "new_name": "Fact Sales"}),
        );
        let r = s.report.as_ref().unwrap();
        let ms = super::report_measures(r);
        assert_eq!(ms.len(), 1);
        assert_eq!(ms[0].entity, "Fact Sales");
        assert_eq!(
            ms[0].expression,
            "[Revenue] * 2 + SUM('Fact Sales'[Net]) // [Total]"
        );
        assert!(s.report_dirty);
        let val = run(&mut s, "report_measure.list", json!({}));
        assert_eq!(val["measures"][0]["valid"], true, "{val}");

        // Removing what a report measure uses is refused, listing it.
        let e = s
            .execute("measure.remove", &json!({"name": "Revenue"}))
            .unwrap_err();
        assert!(e.contains("report measure 'Fact Sales'[Double]"), "{e}");
        let e = s
            .execute("table.remove", &json!({"name": "Fact Sales"}))
            .unwrap_err();
        assert!(e.contains("report measure"), "{e}");
        let out = run(
            &mut s,
            "measure.remove",
            json!({"name": "Revenue", "force": true}),
        );
        assert_eq!(out["used_by"][0]["kind"], "report measure");
    }
}
