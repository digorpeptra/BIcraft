//! `parameter.what_if` and `parameter.fields`: Power BI Desktop's Modeling › New parameter
//! (Numeric range, Fields), as `docs/formats/parameters.md` describes them.
//!
//! - A what-if (numeric range) parameter is a calculated table `GENERATESERIES(min, max,
//!   increment)` whose one column takes the parameter's name, a measure `<name> Value` that reads
//!   the selected value (`SELECTEDVALUE(…, default)`), and optionally a single-value slicer on a
//!   page.
//! - A field parameter is a calculated table of tuples `{("Label", NAMEOF('T'[c]), 0), …}` with
//!   three columns (the label, the field's name, the order), the group-by column and
//!   `ParameterMetadata` extended property that mark it, and optionally a slicer of its labels.
//!   A visual bound to the parameter shows the fields chosen in that slicer (all of them when
//!   nothing is chosen): the preview resolves it ([`FieldParam`], `preview_field_params.rs`).
//!
//! Each command is one edit: the model change and the slicer are one `edit.undo` step, and
//! nothing changes when any part fails.

use bicraft_dax::ast::Expr;
use bicraft_model::{Column, DataType, Measure, Partition, SemanticModel, Table};
use bicraft_report::{FieldRef, Page};
use serde_json::{Value, json};

use crate::Session;
use crate::edit::{Edit, apply, check_name, dax_column, new_guid};
use crate::params::{allow, opt_bool, opt_str, str_param};
use crate::report::opt_num;

/// The extended property that marks a parameter's column.
pub(crate) const PARAMETER_METADATA: &str = "ParameterMetadata";
/// `ParameterMetadata.kind` of a field parameter's fields column.
const FIELDS_KIND: i64 = 2;
/// Most rows a what-if parameter's series may have.
const MAX_SERIES_ROWS: f64 = 1_000_000.0;
/// Values beyond which Desktop's slider samples the series (Microsoft Learn, what-if parameters).
const SLIDER_VALUES: f64 = 1_000.0;
/// Most fields of one field parameter.
const MAX_FIELDS: usize = 1_000;
/// The default size of the slicers the commands add.
const WHAT_IF_SLICER: (f64, f64) = (240.0, 100.0);
const FIELDS_SLICER: (f64, f64) = (200.0, 180.0);
/// The data types of a what-if parameter (Desktop's Whole number, Decimal number, Fixed decimal).
pub(crate) const WHAT_IF_TYPES: &[&str] = &["whole", "decimal", "fixed"];

/// A table name as DAX, always quoted (`'Sales'`).
fn quoted(table: &str) -> String {
    format!("'{}'", table.replace('\'', "''"))
}

/// A column or measure reference as DAX: `'Sales'[Amount]`.
fn qualified(table: &str, name: &str) -> String {
    format!("{}[{}]", quoted(table), name.replace(']', "]]"))
}

/// A DAX string literal.
fn dax_string(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// A number as DAX writes it (no exponent).
fn dax_number(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 {
        format!("{x:.0}")
    } else {
        format!("{x}")
    }
}

/// A `json` extended property `{type, name, value}`.
fn extended_property(value: Value) -> Value {
    json!([{"type": "json", "name": PARAMETER_METADATA, "value": value}])
}

/// The model's table names in use (case-insensitive), measure names too.
fn check_free(m: &SemanticModel, table: &str, measure: Option<&str>) -> Result<(), String> {
    if let Some(t) = m.table(table) {
        return Err(format!("a table named `{}` already exists", t.name));
    }
    if let Some(name) = measure
        && let Some((t, x)) = m.measure(name)
    {
        return Err(format!(
            "a measure named `{}` already exists (in table `{}`); measure names are unique in the model",
            x.name, t.name
        ));
    }
    Ok(())
}

/// The page a slicer goes on: `page` when given, else the active page, else the first.
fn slicer_page(s: &Session, page: Option<&str>) -> Result<usize, String> {
    let r = crate::report::report_ref(s)?;
    if let Some(key) = page {
        return crate::report::page_index(r, key);
    }
    if r.pages.is_empty() {
        return Err("the report has no page to put the slicer on (add one with page.add)".into());
    }
    Ok(r.active_page
        .as_deref()
        .and_then(|a| r.pages.iter().position(|p| p.name == a))
        .unwrap_or(0))
}

/// A free spot of `size` on `page`: the first place on a 20 px grid, rows first, that overlaps no
/// visual; the top left corner when the page is full.
fn free_spot(page: &Page, size: (f64, f64)) -> (f64, f64) {
    let (w, h) = size;
    let overlaps = |x: f64, y: f64| {
        page.visuals.iter().any(|v| {
            let p = &v.position;
            x < p.x + p.width && p.x < x + w && y < p.y + p.height && p.y < y + h
        })
    };
    let step = 20.0;
    let mut y = step;
    while y + h <= page.height {
        let mut x = step;
        while x + w <= page.width {
            if !overlaps(x, y) {
                return (x, y);
            }
            x += step;
        }
        y += step;
    }
    (step, step)
}

/// Whether the command adds a slicer: `add_slicer`, else whenever the session has a report.
fn wants_slicer(s: &Session, p: &Value) -> Result<bool, String> {
    let has_report = s.report.is_some();
    match opt_bool(p, "add_slicer")? {
        Some(true) if !has_report => Err(
            "`add_slicer`: the session has no report to put the slicer on (report.new adds one, or pass \
             add_slicer: false)"
                .into(),
        ),
        Some(b) => Ok(b),
        None => Ok(has_report),
    }
}

/// Adds a slicer of `field` (`'T'[C]`) on page `pi`, with `objects` when given, inside the
/// command's edit. Returns the visual's name.
fn add_slicer(
    s: &mut Session,
    pi: usize,
    field: &str,
    size: (f64, f64),
    objects: Option<Value>,
) -> Result<Value, String> {
    let (page_name, (x, y)) = {
        let r = crate::report::report_ref(s)?;
        let page = r.pages.get(pi).ok_or("internal: page index out of range")?;
        (page.name.clone(), free_spot(page, size))
    };
    let out = crate::report::visual_add(
        s,
        &json!({
            "page": page_name,
            "visual_type": "slicer",
            "x": x,
            "y": y,
            "width": size.0,
            "height": size.1,
            "fields": [{"role": "Values", "field": field}],
        }),
    )?;
    let name = out
        .get("visual")
        .and_then(Value::as_str)
        .ok_or("internal: the slicer has no name")?
        .to_string();
    if let Some(objects) = objects {
        let r = s.report.as_mut().ok_or("internal: the report is gone")?;
        let v = r
            .pages
            .get_mut(pi)
            .and_then(|p| p.visuals.iter_mut().find(|v| v.name == name))
            .ok_or("internal: the slicer is gone")?;
        v.visual_extra.insert("objects".into(), objects);
    }
    Ok(json!({"page": page_name, "visual": name, "position": out.get("position")}))
}

/// Runs `f`; when it fails, puts the model and the report back as they were (the command's
/// parts form one edit).
fn atomic(s: &mut Session, f: impl FnOnce(&mut Session) -> Result<Value, String>) -> Result<Value, String> {
    let saved = (
        s.model.clone(),
        s.model_error.clone(),
        s.dirty,
        s.report.clone(),
        s.report_dirty,
    );
    let r = f(s);
    if r.is_err() {
        (s.model, s.model_error, s.dirty, s.report, s.report_dirty) = saved;
    }
    r
}

// ---------------------------------------------------------------------------------------------
// parameter.what_if
// ---------------------------------------------------------------------------------------------

/// `parameter.what_if {name, min, max, increment, default?, data_type?, add_slicer?, page?}`.
pub(crate) fn what_if(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "name",
            "min",
            "max",
            "increment",
            "default",
            "data_type",
            "add_slicer",
            "page",
        ],
    )?;
    let name = str_param(p, "name")?.to_string();
    check_name("parameter", &name)?;
    let need = |key: &str| opt_num(p, key)?.ok_or_else(|| format!("missing number parameter `{key}`"));
    let (min, max, increment) = (need("min")?, need("max")?, need("increment")?);
    let default = opt_num(p, "default")?;
    if max < min {
        return Err(format!("`max` ({max}) is less than `min` ({min})"));
    }
    if increment <= 0.0 {
        return Err("`increment` must be greater than 0".into());
    }
    let rows = ((max - min) / increment).floor() + 1.0;
    if !rows.is_finite() || rows > MAX_SERIES_ROWS {
        return Err(format!(
            "the series from {min} to {max} by {increment} has more than {MAX_SERIES_ROWS} values"
        ));
    }
    let whole =
        [min, max, increment].iter().all(|x| x.fract() == 0.0) && default.is_none_or(|d| d.fract() == 0.0);
    let data_type = match opt_str(p, "data_type")? {
        None => {
            if whole {
                "whole"
            } else {
                "decimal"
            }
        }
        Some(t) => WHAT_IF_TYPES
            .iter()
            .copied()
            .find(|k| k.eq_ignore_ascii_case(t))
            .ok_or_else(|| format!("unknown data_type `{t}` (expected whole, decimal or fixed)"))?,
    };
    if data_type == "whole" && !whole {
        return Err("a whole-number parameter needs whole min, max, increment and default".into());
    }
    let dt = match data_type {
        "whole" => DataType::Int64,
        "fixed" => DataType::Decimal,
        _ => DataType::Double,
    };
    let measure = format!("{name} Value");
    check_name("measure", &measure)?;
    let slicer = wants_slicer(s, p)?;
    let page = if slicer {
        Some(slicer_page(s, opt_str(p, "page")?)?)
    } else {
        if opt_str(p, "page")?.is_some() {
            return Err("`page` is only used with add_slicer".into());
        }
        None
    };
    let mut warnings = Vec::new();
    if rows > SLIDER_VALUES {
        warnings.push(format!(
            "the series has {rows} values: Power BI Desktop's slider shows at most {SLIDER_VALUES} of them (evenly sampled)"
        ));
    }
    let series = format!(
        "GENERATESERIES({}, {}, {})",
        dax_number(min),
        dax_number(max),
        dax_number(increment)
    );
    let column_ref = qualified(&name, &name);
    let value = match default {
        Some(d) => format!("SELECTEDVALUE({column_ref}, {})", dax_number(d)),
        None => format!("SELECTEDVALUE({column_ref})"),
    };
    atomic(s, |s| {
        let mut out = apply(s, &json!({}), |m| {
            check_free(m, &name, Some(&measure))?;
            let mut edit = Edit::new(json!({}));
            edit.check(format!("calculated table {}", quoted(&name)), &series, None);
            edit.check(format!("measure [{measure}]"), &value, Some(&name));
            let mut column = Column {
                name: name.clone(),
                data_type: dt,
                source_column: Some("[Value]".into()),
                lineage_tag: Some(new_guid()),
                summarize_by: Some("none".into()),
                ..Default::default()
            };
            column.extra.insert("type".into(), json!("calculatedTableColumn"));
            column.extra.insert("isNameInferred".into(), json!(false));
            column.extra.insert("isDataTypeInferred".into(), json!(true));
            column.extra.insert(
                "extendedProperties".into(),
                extended_property(json!({"version": 0})),
            );
            crate::desktop_defaults::new_column(&mut column, true, None);
            m.tables.push(Table {
                name: name.clone(),
                lineage_tag: Some(new_guid()),
                columns: vec![column],
                measures: vec![Measure {
                    name: measure.clone(),
                    expression: value.clone(),
                    lineage_tag: Some(new_guid()),
                    ..Default::default()
                }],
                partitions: vec![Partition {
                    name: name.clone(),
                    mode: Some("import".into()),
                    source_type: "calculated".into(),
                    expression: Some(series.clone()),
                    ..Default::default()
                }],
                ..Default::default()
            });
            Ok(edit)
        })?;
        let mut result = json!({
            "table": name,
            "column": name,
            "expression": series,
            "data_type": data_type,
            "measure": {"name": measure, "expression": value},
        });
        if let Some(pi) = page {
            // Desktop's what-if slicer is a single-value slider (docs/formats/parameters.md)
            let objects =
                json!({"data": [{"properties": {"mode": {"expr": {"Literal": {"Value": "'Single'"}}}}}]});
            result["slicer"] = add_slicer(s, pi, &column_ref, WHAT_IF_SLICER, Some(objects))?;
        }
        if let Some(w) = out.get_mut("warnings").and_then(Value::as_array_mut) {
            w.extend(warnings.into_iter().map(Value::String));
        }
        if let (Some(o), Some(r)) = (result.as_object_mut(), out.as_object()) {
            for k in ["warnings", "diagnostics"] {
                if let Some(v) = r.get(k) {
                    o.insert(k.into(), v.clone());
                }
            }
        }
        Ok(result)
    })
}

// ---------------------------------------------------------------------------------------------
// parameter.fields
// ---------------------------------------------------------------------------------------------

/// `parameter.fields {name, fields: [field | {field, label?}], add_slicer?, page?}`.
pub(crate) fn fields(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "fields", "add_slicer", "page"])?;
    let name = str_param(p, "name")?.to_string();
    check_name("parameter", &name)?;
    let list = match p.get("fields") {
        Some(Value::Array(a)) if !a.is_empty() => a,
        Some(Value::Array(_)) => return Err("`fields` must list at least one field".into()),
        _ => return Err("missing list parameter `fields` (fields like Sales[Region] or [Total])".into()),
    };
    if list.len() > MAX_FIELDS {
        return Err(format!("at most {MAX_FIELDS} fields"));
    }
    let model = crate::commands::model(s)?;
    let mut rows: Vec<(String, FieldRef)> = Vec::with_capacity(list.len());
    for (i, item) in list.iter().enumerate() {
        let (text, label) = match item {
            Value::String(t) => (t.as_str(), None),
            Value::Object(_) => {
                allow(item, &["field", "label"]).map_err(|e| format!("fields[{i}]: {e}"))?;
                (
                    str_param(item, "field").map_err(|e| format!("fields[{i}]: {e}"))?,
                    opt_str(item, "label").map_err(|e| format!("fields[{i}]: {e}"))?,
                )
            }
            _ => return Err(format!("fields[{i}] must be a field or {{field, label}}")),
        };
        let field = crate::refs::resolve_field(Some(model), text)
            .map_err(|e| format!("fields[{i}]: {e}"))?
            .field;
        let own = match &field {
            FieldRef::Column { column, .. } => column.clone(),
            FieldRef::Measure { measure, .. } => measure.clone(),
            _ => {
                return Err(format!(
                    "fields[{i}]: `{text}` is an aggregation; a field parameter lists columns and measures"
                ));
            }
        };
        if rows.iter().any(|(_, f)| *f == field) {
            return Err(format!("fields[{i}]: `{text}` is listed twice"));
        }
        let label = label.map_or(own, str::to_string);
        if label.chars().any(char::is_control) || label.chars().count() > 1024 {
            return Err(format!(
                "fields[{i}]: the label must be at most 1024 characters, without control characters"
            ));
        }
        rows.push((label, field));
    }
    let slicer = wants_slicer(s, p)?;
    let page = if slicer {
        Some(slicer_page(s, opt_str(p, "page")?)?)
    } else {
        if opt_str(p, "page")?.is_some() {
            return Err("`page` is only used with add_slicer".into());
        }
        None
    };
    let tuples: Vec<String> = rows
        .iter()
        .enumerate()
        .map(|(i, (label, f))| {
            let (t, n) = match f {
                FieldRef::Column { table, column } => (table, column),
                FieldRef::Measure { table, measure } => (table, measure),
                _ => (&name, &name),
            };
            format!("({}, NAMEOF({}), {i})", dax_string(label), qualified(t, n))
        })
        .collect();
    let expression = format!("{{\n    {}\n}}", tuples.join(",\n    "));
    let fields_col = format!("{name} Fields");
    let order_col = format!("{name} Order");
    atomic(s, |s| {
        let out = apply(s, &json!({}), |m| {
            check_free(m, &name, None)?;
            let mut edit = Edit::new(json!({}));
            edit.check(format!("calculated table {}", quoted(&name)), &expression, None);
            let col = |n: &str, source: &str, dt: DataType, hidden: bool| {
                let mut c = Column {
                    name: n.to_string(),
                    data_type: dt,
                    source_column: Some(source.into()),
                    lineage_tag: Some(new_guid()),
                    summarize_by: Some("none".into()),
                    is_hidden: hidden,
                    ..Default::default()
                };
                c.extra.insert("type".into(), json!("calculatedTableColumn"));
                c.extra.insert("isNameInferred".into(), json!(false));
                c.extra.insert("isDataTypeInferred".into(), json!(true));
                c
            };
            let mut display = col(&name, "[Value1]", DataType::String, false);
            display.sort_by_column = Some(order_col.clone());
            display.extra.insert(
                "relatedColumnDetails".into(),
                json!({"groupByColumns": [{"groupingColumn": fields_col}]}),
            );
            let mut fields = col(&fields_col, "[Value2]", DataType::String, true);
            fields.sort_by_column = Some(order_col.clone());
            fields.extra.insert(
                "extendedProperties".into(),
                extended_property(json!({"version": 3, "kind": FIELDS_KIND})),
            );
            let order = col(&order_col, "[Value3]", DataType::Int64, true);
            m.tables.push(Table {
                name: name.clone(),
                lineage_tag: Some(new_guid()),
                columns: vec![display, fields, order],
                partitions: vec![Partition {
                    name: name.clone(),
                    mode: Some("import".into()),
                    source_type: "calculated".into(),
                    expression: Some(expression.clone()),
                    ..Default::default()
                }],
                ..Default::default()
            });
            Ok(edit)
        })?;
        let mut result = json!({
            "table": name,
            "columns": [name, fields_col, order_col],
            "expression": expression,
            "fields": rows.iter().map(|(label, f)| json!({"label": label, "field": field_text(f)})).collect::<Vec<_>>(),
        });
        if let Some(pi) = page {
            result["slicer"] = add_slicer(s, pi, &qualified(&name, &name), FIELDS_SLICER, None)?;
        }
        if let (Some(o), Some(r)) = (result.as_object_mut(), out.as_object()) {
            for k in ["warnings", "diagnostics"] {
                if let Some(v) = r.get(k) {
                    o.insert(k.into(), v.clone());
                }
            }
        }
        Ok(result)
    })
}

/// A field as the commands write it: `Sales[Region]`, `[Total]`.
fn field_text(f: &FieldRef) -> String {
    match f {
        FieldRef::Column { table, column } => dax_column(table, column),
        FieldRef::Measure { measure, .. } => crate::edit::dax_measure(measure),
        _ => String::new(),
    }
}

// ---------------------------------------------------------------------------------------------
// Reading a field parameter
// ---------------------------------------------------------------------------------------------

/// One field of a field parameter: its label, the field, the name `NAMEOF` gives, its order.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ParamField {
    pub label: String,
    pub field: FieldRef,
    pub name: String,
    pub order: f64,
}

/// A field parameter of the model: its table, its three columns and its fields in order.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FieldParam {
    pub table: String,
    /// The label column (the one visuals and the slicer bind).
    pub display: String,
    /// The hidden column of the fields' names (`ParameterMetadata` kind 2).
    pub fields: String,
    /// The hidden order column, when known.
    pub order: Option<String>,
    pub rows: Vec<ParamField>,
}

/// Whether column `c` carries `ParameterMetadata` with kind 2 (a field parameter's fields).
pub(crate) fn is_fields_column(c: &Column) -> bool {
    c.extra
        .get("extendedProperties")
        .and_then(Value::as_array)
        .is_some_and(|list| {
            list.iter().any(|e| {
                e.get("name").and_then(Value::as_str) == Some(PARAMETER_METADATA)
                    && e.pointer("/value/kind").and_then(Value::as_i64) == Some(FIELDS_KIND)
            })
        })
}

/// The column grouped by `fields` (`relatedColumnDetails.groupByColumns`), the label column.
fn grouped_by<'t>(t: &'t Table, fields: &str) -> Option<&'t Column> {
    t.columns.iter().find(|c| {
        c.extra
            .get("relatedColumnDetails")
            .and_then(|d| d.get("groupByColumns"))
            .and_then(Value::as_array)
            .is_some_and(|g| {
                g.iter().any(|x| {
                    x.get("groupingColumn")
                        .and_then(Value::as_str)
                        .is_some_and(|n| n.eq_ignore_ascii_case(fields))
                })
            })
    })
}

/// The field parameter that table `table` is, when it is one whose expression is the tuple
/// table Desktop writes (`{("Label", NAMEOF(field), order), …}`); `None` otherwise.
pub(crate) fn field_param(m: &SemanticModel, table: &str) -> Option<FieldParam> {
    let t = m.table(table)?;
    let fields = t.columns.iter().find(|c| is_fields_column(c))?;
    let display = grouped_by(t, &fields.name)?;
    let expression = t
        .partitions
        .iter()
        .find(|p| p.source_type.eq_ignore_ascii_case("calculated"))?
        .expression
        .as_deref()?;
    let (Some(Expr::TableCtor { rows, .. }), _) = bicraft_dax::parse_expression(expression) else {
        return None;
    };
    // the column each tuple position feeds (`[Value1]`, `[Value2]`, `[Value3]`)
    let position = |c: &Column| -> Option<usize> {
        let s = c.source_column.as_deref()?;
        let n: usize = s.strip_prefix("[Value")?.strip_suffix(']')?.parse().ok()?;
        n.checked_sub(1)
    };
    let label_at = position(display).unwrap_or(0);
    let name_at = position(fields).unwrap_or(1);
    let order_col = fields
        .sort_by_column
        .as_deref()
        .and_then(|o| t.column(o))
        .or_else(|| t.columns.iter().find(|c| position(c) == Some(2)));
    let order_at = order_col.and_then(position).unwrap_or(2);
    let mut out = Vec::with_capacity(rows.len());
    for (i, row) in rows.iter().enumerate() {
        let Expr::Tuple { items, .. } = row else {
            return None;
        };
        let label = match items.get(label_at)? {
            Expr::Str { value, .. } => value.clone(),
            _ => return None,
        };
        let (field, name) = nameof_field(m, items.get(name_at)?)?;
        let order = match items.get(order_at) {
            Some(Expr::Number { text, .. }) => text.parse().unwrap_or(i as f64),
            _ => i as f64,
        };
        out.push(ParamField {
            label,
            field,
            name,
            order,
        });
    }
    out.sort_by(|a, b| a.order.total_cmp(&b.order));
    Some(FieldParam {
        table: t.name.clone(),
        display: display.name.clone(),
        fields: fields.name.clone(),
        order: order_col.map(|c| c.name.clone()),
        rows: out,
    })
}

/// `NAMEOF('T'[x])`: the column or measure it names, and the name it gives (`'T'[x]`).
fn nameof_field(m: &SemanticModel, e: &Expr) -> Option<(FieldRef, String)> {
    let Expr::Call { name, args, .. } = e else {
        return None;
    };
    if !name.eq_ignore_ascii_case("NAMEOF") {
        return None;
    }
    let Some(Some(Expr::Column {
        table,
        name,
        variation: None,
        ..
    })) = args.first()
    else {
        return None;
    };
    if let Some(t) = table.as_deref().and_then(|t| m.table(t)) {
        if let Some(c) = t.column(name) {
            let field = FieldRef::Column {
                table: t.name.clone(),
                column: c.name.clone(),
            };
            return Some((field, qualified(&t.name, &c.name)));
        }
        if let Some(x) = t.measure(name) {
            let field = FieldRef::Measure {
                table: t.name.clone(),
                measure: x.name.clone(),
            };
            return Some((field, qualified(&t.name, &x.name)));
        }
        return None;
    }
    let (t, x) = m.measure(name)?;
    if table.is_some() {
        return None;
    }
    Some((
        FieldRef::Measure {
            table: t.name.clone(),
            measure: x.name.clone(),
        },
        qualified(&t.name, &x.name),
    ))
}
