//! Calculation groups: `calculation_group.add` / `.set` / `.remove` and `calculation_item.add` /
//! `.set` / `.remove`.
//!
//! A calculation group is a table whose `calculationGroup` object (kept in [`Table::extra`], as
//! TMSL and TMDL write it) holds the calculation items; its rows are the items. The table has the
//! layout Power BI Desktop 2.158 gives a new calculation group: a text column holding the item
//! names (`sourceColumn` `Name`, sorted by the ordinal), a hidden `Ordinal` column (`sourceColumn`
//! `Ordinal`) and one `calculationGroup` partition.
//!
//! Item expressions and format string expressions are validated as DAX against the model
//! (`SELECTEDMEASURE()`, `SELECTEDMEASURENAME()`, `ISSELECTEDMEASURE(...)`,
//! `SELECTEDMEASUREFORMATSTRING()` included). A model with calculation groups must discourage
//! implicit measures: `calculation_group.add` sets the model's `discourageImplicitMeasures`, as
//! Desktop does when it creates the first group.

use bicraft_model::{Column, DataType, Partition, SemanticModel, Table};
use bicraft_vertipaq::CellValue;
use serde_json::{Map, Value, json};

use crate::Session;
use crate::commands::model;
use crate::edit::{Edit, apply, check_name, dax_table, new_guid, table_index};
use crate::params::{allow, change_str, opt_bool, opt_str, str_param};

/// Lowest compatibility level with calculation groups.
const MIN_COMPATIBILITY_LEVEL: u32 = 1500;
/// Most items of one group.
const MAX_ITEMS: usize = 10_000;

fn group(t: &Table) -> Option<&Map<String, Value>> {
    t.extra.get("calculationGroup").and_then(Value::as_object)
}

fn item_name(v: &Value) -> &str {
    v.get("name").and_then(Value::as_str).unwrap_or_default()
}

fn ordinal(v: &Value) -> Option<i64> {
    v.get("ordinal").and_then(Value::as_i64)
}

fn precedence(t: &Table) -> i64 {
    group(t)
        .and_then(|g| g.get("precedence"))
        .and_then(Value::as_i64)
        .unwrap_or(0)
}

/// The calculation group table named `name` (index), or why there is none.
fn group_index(m: &SemanticModel, name: &str) -> Result<usize, String> {
    let i = table_index(m, name)?;
    match m.tables.get(i) {
        Some(t) if group(t).is_some() => Ok(i),
        Some(t) => Err(format!("table `{}` is not a calculation group", t.name)),
        None => Err("internal: table index out of range".into()),
    }
}

fn group_mut(m: &mut SemanticModel, i: usize) -> Result<(&str, &mut Map<String, Value>), String> {
    let t = m.tables.get_mut(i).ok_or("internal: table index out of range")?;
    let g = t
        .extra
        .get_mut("calculationGroup")
        .and_then(Value::as_object_mut)
        .ok_or("internal: not a calculation group")?;
    Ok((t.name.as_str(), g))
}

fn items_mut(g: &mut Map<String, Value>) -> Result<&mut Vec<Value>, String> {
    if !g.get("calculationItems").is_some_and(Value::is_array) {
        g.insert("calculationItems".into(), Value::Array(Vec::new()));
    }
    g.get_mut("calculationItems")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "internal: calculation items are not an array".to_string())
}

fn opt_i64(p: &Value, key: &str) -> Result<Option<i64>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_i64()
            .map(Some)
            .ok_or_else(|| format!("parameter `{key}` must be an integer")),
    }
}

/// Ensures calculation groups are allowed and implicit measures discouraged; returns notes.
fn prepare_model(m: &mut SemanticModel) -> Result<Vec<String>, String> {
    let mut notes = Vec::new();
    if let Some(level) = m.compatibility_level
        && level < MIN_COMPATIBILITY_LEVEL
    {
        return Err(format!(
            "calculation groups need compatibility level {MIN_COMPATIBILITY_LEVEL} or more (the model has {level})"
        ));
    }
    if m.extra.get("discourageImplicitMeasures").and_then(Value::as_bool) != Some(true) {
        m.extra
            .insert("discourageImplicitMeasures".into(), Value::Bool(true));
        notes.push(
            "the model now discourages implicit measures (discourageImplicitMeasures), as calculation groups require"
                .into(),
        );
    }
    Ok(notes)
}

/// `calculation_group.add`.
pub(crate) fn calculation_group_add(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "precedence", "column", "description", "hidden"])?;
    let name = str_param(p, "name")?.to_string();
    check_name("table", &name)?;
    let column = opt_str(p, "column")?.unwrap_or("Name").to_string();
    check_name("column", &column)?;
    if column.eq_ignore_ascii_case("Ordinal") {
        return Err(
            "the item column cannot be named `Ordinal` (the group's ordinal column has that name)".into(),
        );
    }
    let wanted = opt_i64(p, "precedence")?;
    let description = opt_str(p, "description")?.map(str::to_string);
    let hidden = opt_bool(p, "hidden")?.unwrap_or(false);
    apply(s, p, |m| {
        if let Some(t) = m.table(&name) {
            return Err(format!("a table named `{}` already exists", t.name));
        }
        let groups: Vec<(String, i64)> = m
            .tables
            .iter()
            .filter(|t| group(t).is_some())
            .map(|t| (t.name.clone(), precedence(t)))
            .collect();
        let precedence = match wanted {
            Some(v) => {
                if v < 0 {
                    return Err("`precedence` must not be negative".into());
                }
                if let Some((other, _)) = groups.iter().find(|(_, x)| *x == v) {
                    return Err(format!(
                        "calculation group `{other}` already has precedence {v}; precedences must differ"
                    ));
                }
                v
            }
            None => groups.iter().map(|(_, x)| x.saturating_add(1)).max().unwrap_or(0),
        };
        let notes = prepare_model(m)?;
        let mut g = Map::new();
        if let Some(d) = &description {
            g.insert("description".into(), json!(d));
        }
        if precedence != 0 {
            g.insert("precedence".into(), json!(precedence));
        }
        g.insert("calculationItems".into(), Value::Array(Vec::new()));
        let mut extra = Map::new();
        extra.insert("calculationGroup".into(), Value::Object(g));
        m.tables.push(Table {
            name: name.clone(),
            lineage_tag: Some(new_guid()),
            is_hidden: hidden,
            columns: vec![
                Column {
                    name: column.clone(),
                    data_type: DataType::String,
                    source_column: Some("Name".into()),
                    sort_by_column: Some("Ordinal".into()),
                    summarize_by: Some("none".into()),
                    lineage_tag: Some(new_guid()),
                    ..Default::default()
                },
                Column {
                    name: "Ordinal".into(),
                    data_type: DataType::Int64,
                    source_column: Some("Ordinal".into()),
                    summarize_by: Some("sum".into()),
                    is_hidden: true,
                    lineage_tag: Some(new_guid()),
                    ..Default::default()
                },
            ],
            partitions: vec![Partition {
                name: name.clone(),
                source_type: "calculationGroup".into(),
                ..Default::default()
            }],
            extra,
            ..Default::default()
        });
        let mut edit = Edit::new(json!({
            "table": name,
            "column": column,
            "precedence": precedence,
        }));
        edit.notes = notes;
        Ok(edit)
    })
}

/// `calculation_group.set`: precedence, description, the selection expressions.
pub(crate) fn calculation_group_set(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "name",
            "precedence",
            "description",
            "multiple_or_empty_selection_expression",
            "no_selection_expression",
            "force",
        ],
    )?;
    let name = str_param(p, "name")?;
    let wanted = opt_i64(p, "precedence")?;
    let description = change_str(p, "description")?;
    let multiple = change_str(p, "multiple_or_empty_selection_expression")?;
    let none = change_str(p, "no_selection_expression")?;
    if wanted.is_none() && description.is_none() && multiple.is_none() && none.is_none() {
        return Err(
            "calculation_group.set needs at least one of `precedence`, `description`, \
             `multiple_or_empty_selection_expression`, `no_selection_expression`"
                .into(),
        );
    }
    apply(s, p, |m| {
        let i = group_index(m, name)?;
        if let Some(v) = wanted {
            if v < 0 {
                return Err("`precedence` must not be negative".into());
            }
            if let Some(other) = m
                .tables
                .iter()
                .enumerate()
                .find(|(j, t)| *j != i && group(t).is_some() && precedence(t) == v)
                .map(|(_, t)| t.name.clone())
            {
                return Err(format!(
                    "calculation group `{other}` already has precedence {v}; precedences must differ"
                ));
            }
        }
        let (table, g) = group_mut(m, i)?;
        let table = table.to_string();
        let mut checks: Vec<(String, String)> = Vec::new();
        if let Some(v) = wanted {
            if v == 0 {
                g.remove("precedence");
            } else {
                g.insert("precedence".into(), json!(v));
            }
        }
        if let Some(d) = description {
            match d {
                Some(d) => g.insert("description".into(), json!(d)),
                None => g.remove("description"),
            };
        }
        for (key, value, what) in [
            (
                "multipleOrEmptySelectionExpression",
                multiple,
                "multiple or empty selection expression",
            ),
            ("noSelectionExpression", none, "no selection expression"),
        ] {
            match value {
                None => {}
                Some(None) => {
                    g.remove(key);
                }
                Some(Some(expr)) => {
                    checks.push((format!("{what} of {}", dax_table(&table)), expr.clone()));
                    match g.get_mut(key).and_then(Value::as_object_mut) {
                        Some(o) => {
                            o.insert("expression".into(), json!(expr));
                        }
                        None => {
                            g.insert(key.into(), json!({"expression": expr}));
                        }
                    }
                }
            }
        }
        let snapshot = Value::Object(g.clone());
        let mut edit = Edit::new(json!({"table": table, "calculation_group": snapshot}));
        for (object, expr) in &checks {
            edit.check(object.clone(), expr, Some(&table));
        }
        Ok(edit)
    })
}

/// `calculation_group.remove`: the table and its items (refused while DAX uses the table).
pub(crate) fn calculation_group_remove(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "force"])?;
    let name = str_param(p, "name")?;
    group_index(model(s)?, name)?;
    crate::edit::table_remove(s, p)
}

fn check_format_string(edit: &mut Edit, table: &str, item: &str, expr: &str) {
    edit.check(
        format!("format string of calculation item {}[{item}]", dax_table(table)),
        expr,
        Some(table),
    );
}

/// `calculation_item.add`.
pub(crate) fn calculation_item_add(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "group",
            "name",
            "expression",
            "ordinal",
            "format_string_expression",
            "description",
            "force",
        ],
    )?;
    let group_name = str_param(p, "group")?;
    let name = str_param(p, "name")?.to_string();
    check_name("calculation item", &name)?;
    let expression = str_param(p, "expression")?.to_string();
    let wanted = opt_i64(p, "ordinal")?;
    let format = opt_str(p, "format_string_expression")?.map(str::to_string);
    let description = opt_str(p, "description")?.map(str::to_string);
    apply(s, p, |m| {
        let i = group_index(m, group_name)?;
        let notes = prepare_model(m)?;
        let (table, g) = group_mut(m, i)?;
        let table = table.to_string();
        let items = items_mut(g)?;
        if items.iter().any(|x| item_name(x).eq_ignore_ascii_case(&name)) {
            return Err(format!(
                "calculation group `{table}` already has an item named `{name}`"
            ));
        }
        if items.len() >= MAX_ITEMS {
            return Err(format!("a calculation group holds at most {MAX_ITEMS} items"));
        }
        let next = items
            .iter()
            .filter_map(ordinal)
            .map(|o| o.saturating_add(1))
            .max()
            .unwrap_or(0);
        let ord = match wanted {
            Some(o) if o < 0 => return Err("`ordinal` must not be negative".into()),
            Some(o) => {
                // Make room: items at or after the position move down by one.
                for x in items.iter_mut() {
                    if let Some(xo) = ordinal(x)
                        && xo >= o
                        && let Some(obj) = x.as_object_mut()
                    {
                        obj.insert("ordinal".into(), json!(xo.saturating_add(1)));
                    }
                }
                o.min(next)
            }
            None => next,
        };
        let mut item = Map::new();
        item.insert("name".into(), json!(name));
        if let Some(d) = &description {
            item.insert("description".into(), json!(d));
        }
        item.insert("expression".into(), json!(expression));
        item.insert("ordinal".into(), json!(ord));
        if let Some(f) = &format {
            item.insert("formatStringDefinition".into(), json!({"expression": f}));
        }
        let at = items
            .iter()
            .position(|x| ordinal(x).is_some_and(|xo| xo > ord))
            .unwrap_or(items.len());
        items.insert(at, Value::Object(item));
        let mut edit = Edit::new(json!({"group": table, "item": name, "ordinal": ord}));
        edit.check(
            format!("calculation item {}[{name}]", dax_table(&table)),
            &expression,
            Some(&table),
        );
        if let Some(f) = &format {
            check_format_string(&mut edit, &table, &name, f);
        }
        edit.notes = notes;
        Ok(edit)
    })
}

/// `calculation_item.set`.
pub(crate) fn calculation_item_set(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "group",
            "name",
            "new_name",
            "expression",
            "ordinal",
            "format_string_expression",
            "description",
            "force",
        ],
    )?;
    let group_name = str_param(p, "group")?;
    let name = str_param(p, "name")?;
    let new_name = opt_str(p, "new_name")?.map(str::to_string);
    if let Some(n) = &new_name {
        check_name("calculation item", n)?;
    }
    let expression = opt_str(p, "expression")?.map(str::to_string);
    let wanted = opt_i64(p, "ordinal")?;
    let format = change_str(p, "format_string_expression")?;
    let description = change_str(p, "description")?;
    if new_name.is_none()
        && expression.is_none()
        && wanted.is_none()
        && format.is_none()
        && description.is_none()
    {
        return Err(
            "calculation_item.set needs at least one of `new_name`, `expression`, `ordinal`, \
             `format_string_expression`, `description`"
                .into(),
        );
    }
    apply(s, p, |m| {
        let i = group_index(m, group_name)?;
        let (table, g) = group_mut(m, i)?;
        let table = table.to_string();
        let items = items_mut(g)?;
        let k = items
            .iter()
            .position(|x| item_name(x).eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("calculation group `{table}` has no item named `{name}`"))?;
        if let Some(n) = &new_name
            && items
                .iter()
                .enumerate()
                .any(|(j, x)| j != k && item_name(x).eq_ignore_ascii_case(n))
        {
            return Err(format!(
                "calculation group `{table}` already has an item named `{n}`"
            ));
        }
        let mut notes = Vec::new();
        let mut checks: Vec<(String, String)> = Vec::new();
        if let Some(o) = wanted {
            if o < 0 {
                return Err("`ordinal` must not be negative".into());
            }
            // Moves the item: the others keep their order around it, ordinals 0..n.
            let item = items.remove(k);
            let mut sorted: Vec<Value> = std::mem::take(items);
            sorted.sort_by_key(|x| ordinal(x).unwrap_or(i64::MAX));
            let at = usize::try_from(o).unwrap_or(usize::MAX).min(sorted.len());
            sorted.insert(at, item);
            for (n, x) in sorted.iter_mut().enumerate() {
                if let Some(obj) = x.as_object_mut() {
                    obj.insert("ordinal".into(), json!(n));
                }
            }
            *items = sorted;
        }
        let k = items
            .iter()
            .position(|x| item_name(x).eq_ignore_ascii_case(name))
            .ok_or("internal: calculation item lost")?;
        let item = items
            .get_mut(k)
            .and_then(Value::as_object_mut)
            .ok_or("internal: calculation item is not an object")?;
        if let Some(n) = &new_name {
            item.insert("name".into(), json!(n));
            notes.push(
                "DAX and report filters that compare the item column with the old name as text are not rewritten"
                    .into(),
            );
        }
        let shown = new_name
            .clone()
            .unwrap_or_else(|| item_name(&Value::Object(item.clone())).to_string());
        if let Some(e) = &expression {
            item.insert("expression".into(), json!(e));
            checks.push((
                format!("calculation item {}[{shown}]", dax_table(&table)),
                e.clone(),
            ));
        }
        match &format {
            None => {}
            Some(None) => {
                item.remove("formatStringDefinition");
            }
            Some(Some(f)) => {
                match item
                    .get_mut("formatStringDefinition")
                    .and_then(Value::as_object_mut)
                {
                    Some(o) => {
                        o.insert("expression".into(), json!(f));
                    }
                    None => {
                        item.insert("formatStringDefinition".into(), json!({"expression": f}));
                    }
                }
            }
        }
        match &description {
            None => {}
            Some(None) => {
                item.remove("description");
            }
            Some(Some(d)) => {
                item.insert("description".into(), json!(d));
            }
        }
        let snapshot = Value::Object(item.clone());
        let mut edit = Edit::new(json!({"group": table, "item": snapshot}));
        for (object, expr) in &checks {
            edit.check(object.clone(), expr, Some(&table));
        }
        if let Some(Some(f)) = &format {
            check_format_string(&mut edit, &table, &shown, f);
        }
        edit.notes = notes;
        Ok(edit)
    })
}

/// `calculation_item.remove`.
pub(crate) fn calculation_item_remove(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["group", "name"])?;
    let group_name = str_param(p, "group")?;
    let name = str_param(p, "name")?;
    apply(s, p, |m| {
        let i = group_index(m, group_name)?;
        let (table, g) = group_mut(m, i)?;
        let table = table.to_string();
        let items = items_mut(g)?;
        let k = items
            .iter()
            .position(|x| item_name(x).eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("calculation group `{table}` has no item named `{name}`"))?;
        let removed = items.remove(k);
        Ok(Edit::new(json!({
            "group": table,
            "removed": item_name(&removed),
            "remaining": items.len(),
        })))
    })
}

/// Values of data columns, by column name.
type StoredColumns = Vec<(String, Vec<CellValue>)>;

/// The rows a PBIX stores for a calculation group table (one per item, in ordinal order): the
/// values of its data columns by `sourceColumn` (`Name`: the item names, `Ordinal`: their
/// ordinals), for `file.save`'s structural writer.
pub(crate) fn stored_rows(t: &Table) -> Result<(usize, StoredColumns), String> {
    let rows = bicraft_pbix::structural::calculation_rows(t);
    let mut cols = Vec::new();
    for c in t
        .columns
        .iter()
        .filter(|c| c.expression.is_none() && !c.is_row_number())
    {
        let values: Vec<CellValue> = match c.source_column.as_deref() {
            Some(s) if s.eq_ignore_ascii_case("Name") => {
                rows.iter().map(|(n, _)| CellValue::Text(n.clone())).collect()
            }
            Some(s) if s.eq_ignore_ascii_case("Ordinal") => {
                rows.iter().map(|(_, o)| CellValue::Int(*o)).collect()
            }
            _ => {
                return Err(format!(
                    "column `{}` of calculation group `{}` is neither its Name nor its Ordinal column",
                    c.name, t.name
                ));
            }
        };
        cols.push((c.name.clone(), values));
    }
    Ok((rows.len(), cols))
}
