//! `metadata-edit`: the edits of the metadata oracle models (docs/formats/metadata-sqlitedb.md,
//! "Engine oracle").
//!
//! `cargo xtask metadata-edit EDIT IN.json OUT.json` reads bicraft's TMSL of a model, changes
//! objects that metadata-only writing supports and writes the edited TMSL. All names and values
//! are invented for these tests.
//!
//! Paths address the TMSL `model` object: segments separated by `.`, a number indexes a list and
//! `[name]` picks the first list item with that `name`.

use std::path::Path;

use serde_json::{Map, Value, json};

use super::{pyjson, read_json, write_text};

type R = Result<(), String>;

/// One edit of the TMSL `model` object.
type Edit = fn(&mut Value) -> R;

/// The edits, in the order the metadata oracle runs them.
pub const EDITS: [(&str, Edit); 9] = [
    ("add_and_change", add_and_change),
    ("remove", remove),
    ("calendars", calendars),
    ("sources", sources),
    ("aggregations", aggregations),
    ("variation_props", variation_props),
    ("variation_rename", variation_rename),
    ("preview_change", preview_change),
    ("preview_remove", preview_remove),
];

fn step<'a>(v: &'a mut Value, seg: &str) -> Option<&'a mut Value> {
    if let Some(name) = seg.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        return v
            .as_array_mut()?
            .iter_mut()
            .find(|x| x.get("name").and_then(Value::as_str) == Some(name));
    }
    match v {
        Value::Array(a) => a.get_mut(seg.parse::<usize>().ok()?),
        Value::Object(m) => m.get_mut(seg),
        _ => None,
    }
}

fn at<'a>(v: &'a mut Value, path: &str) -> Result<&'a mut Value, String> {
    let mut cur = v;
    for seg in path.split('.').filter(|s| !s.is_empty()) {
        cur = step(cur, seg).ok_or_else(|| format!("no `{seg}` in `{path}`"))?;
    }
    Ok(cur)
}

/// The parent object of `path` and its last key.
fn parent<'a, 'p>(v: &'a mut Value, path: &'p str) -> Result<(&'a mut Map<String, Value>, &'p str), String> {
    let (head, key) = path.rsplit_once('.').unwrap_or(("", path));
    let p = at(v, head)?
        .as_object_mut()
        .ok_or_else(|| format!("`{head}` is not an object"))?;
    Ok((p, key))
}

/// `x[k] = value` (an existing key keeps its place).
fn set(v: &mut Value, path: &str, value: Value) -> R {
    if let Some((head, idx)) = path.rsplit_once('.').filter(|(_, i)| i.parse::<usize>().is_ok()) {
        *at(v, head)?
            .as_array_mut()
            .and_then(|a| a.get_mut(idx.parse::<usize>().ok()?))
            .ok_or_else(|| format!("no `{path}`"))? = value;
        return Ok(());
    }
    let (p, key) = parent(v, path)?;
    p.insert(key.to_string(), value);
    Ok(())
}

/// `list.append(value)`.
fn push(v: &mut Value, path: &str, value: Value) -> R {
    at(v, path)?
        .as_array_mut()
        .ok_or_else(|| format!("`{path}` is not a list"))?
        .push(value);
    Ok(())
}

/// `del x[k]` (fails when missing).
fn del(v: &mut Value, path: &str) -> R {
    let (p, key) = parent(v, path)?;
    p.shift_remove(key)
        .map(|_| ())
        .ok_or_else(|| format!("no `{path}` to remove"))
}

/// `x.pop(k, None)`.
fn pop(v: &mut Value, path: &str) -> R {
    let (p, key) = parent(v, path)?;
    p.shift_remove(key);
    Ok(())
}

/// Keeps the list items whose `name` is not `name`.
fn drop_named(v: &mut Value, path: &str, name: &str) -> R {
    at(v, path)?
        .as_array_mut()
        .ok_or_else(|| format!("`{path}` is not a list"))?
        .retain(|x| x.get("name").and_then(Value::as_str) != Some(name));
    Ok(())
}

fn items(v: &mut Value) -> impl Iterator<Item = &mut Value> {
    v.as_array_mut().into_iter().flatten()
}

fn add_and_change(m: &mut Value) -> R {
    set(m, "extendedProperties.0.value", json!({"a": 2, "b": [1, 2]}))?;
    push(
        m,
        "extendedProperties",
        json!({"name": "ModelStr", "value": "hello"}),
    )?;
    set(m, "queryGroups.0.description", json!("qg2"))?;
    push(
        m,
        "queryGroups",
        json!({"folder": "Extra", "description": "new group", "annotations": [{"name": "N", "value": "1"}]}),
    )?;
    let tr = "cultures.[fr-FR].translations.model";
    set(m, &format!("{tr}.tables.0.translatedCaption"), json!("Ventes 2"))?;
    push(
        m,
        &format!("{tr}.tables.0.columns"),
        json!({"name": "Region", "translatedCaption": "Region (fr)"}),
    )?;
    set(
        m,
        &format!("{tr}.tables.0.measures.0.translatedDisplayFolder"),
        json!("Dossier mesure"),
    )?;
    del(m, &format!("{tr}.perspectives"))?;
    set(m, "cultures.[fr-FR].annotations.0.value", json!("cu2"))?;
    set(
        m,
        "cultures.[en-US].linguisticMetadata.content.DynamicImprovement",
        json!("HighConfidence"),
    )?;
    push(
        m,
        "cultures",
        json!({
            "name": "de-DE",
            "linguisticMetadata": {"content": {"Version": "1.0.0", "Language": "de-DE"}, "contentType": "json"},
            "translations": {"model": {"name": "Model", "tables": [
                {"name": "Sales", "translatedCaption": "Umsatz",
                 "columns": [{"name": "City", "translatedCaption": "Stadt"}]}]}},
        }),
    )?;
    set(m, "perspectives.0.description", json!("persp 2"))?;
    drop_named(m, "perspectives.0.tables.0.columns", "Amount")?;
    push(m, "perspectives.0.tables.0.columns", json!({"name": "City"}))?;
    push(
        m,
        "perspectives",
        json!({"name": "Full", "tables": [
            {"name": "Sales", "columns": [{"name": "Id"}],
             "measures": [{"name": "Dyn", "annotations": [{"name": "X", "value": "y"}]}],
             "hierarchies": [{"name": "Geo"}]},
            {"name": "Time Calc", "includeAll": true},
        ]}),
    )?;
    let ms = "tables.[Sales].measures";
    set(m, &format!("{ms}.[Total].kpi.targetExpression"), json!("200"))?;
    set(m, &format!("{ms}.[Total].kpi.annotations.0.value"), json!("k2"))?;
    del(m, &format!("{ms}.[Total].detailRowsDefinition"))?;
    set(
        m,
        &format!("{ms}.[Dyn].formatStringDefinition.expression"),
        json!("\"#,0.0\""),
    )?;
    set(
        m,
        &format!("{ms}.[Dyn].kpi"),
        json!({"targetExpression": "5", "statusGraphic": "Traffic Light"}),
    )?;
    set(
        m,
        &format!("{ms}.[Dyn].detailRowsDefinition"),
        json!({"expression": "SELECTCOLUMNS(Sales, \"Id\", Sales[Id])"}),
    )?;
    set(
        m,
        &format!("{ms}.[Dyn].changedProperties"),
        json!([{"property": "Expression"}]),
    )?;
    push(
        m,
        ms,
        json!({"name": "NewKpi", "expression": "1", "kpi": {"targetExpression": "2"},
               "formatStringDefinition": {"expression": "\"0\""},
               "annotations": [{"name": "A", "value": "b"}],
               "extendedProperties": [{"name": "E", "value": "f"}]}),
    )?;
    set(
        m,
        "tables.[Sales].defaultDetailRowsDefinition.expression",
        json!("SELECTCOLUMNS(Sales, \"City\", Sales[City])"),
    )?;
    let geo = "tables.[Sales].hierarchies.0";
    set(m, &format!("{geo}.description"), json!("geo 2"))?;
    set(m, &format!("{geo}.displayFolder"), json!("H2"))?;
    set(m, &format!("{geo}.levels.1.description"), json!("city level"))?;
    push(
        m,
        &format!("{geo}.annotations"),
        json!({"name": "H2", "value": "x"}),
    )?;
    set(
        m,
        "tables.[Sales].columns.[Id].extendedProperties.0.value",
        json!("s2"),
    )?;
    set(
        m,
        "tables.[Sales].columns.[Amount].changedProperties",
        json!([{"property": "FormatString"}]),
    )?;
    push(
        m,
        "tables.[Param].columns.[Param].relatedColumnDetails.groupByColumns",
        json!({"groupingColumn": "Param Order"}),
    )?;
    let cg = "tables.[Time Calc].calculationGroup";
    set(m, &format!("{cg}.precedence"), json!(20))?;
    set(m, &format!("{cg}.description"), json!("cg desc 2"))?;
    set(
        m,
        &format!("{cg}.calculationItems.[Current].expression"),
        json!("SELECTEDMEASURE() + 0"),
    )?;
    set(
        m,
        &format!("{cg}.calculationItems.[Current].formatStringDefinition"),
        json!({"expression": "\"0.000\""}),
    )?;
    del(
        m,
        &format!("{cg}.calculationItems.[Double].formatStringDefinition"),
    )?;
    del(m, &format!("{cg}.multipleOrEmptySelectionExpression"))?;
    set(
        m,
        &format!("{cg}.noSelectionExpression.formatStringDefinition"),
        json!({"expression": "\"#\""}),
    )?;
    set(m, "roles.0.members.1.memberName", json!("Everyone 2"))?;
    push(
        m,
        "roles.0.members",
        json!({"memberName": "Users", "memberId": "S-1-5-32-545"}),
    )?;
    push(
        m,
        "roles.0.tablePermissions.0.columnPermissions",
        json!({"name": "Region", "metadataPermission": "read"}),
    )?;
    del(m, "roles.0.tablePermissions.0.annotations")?;
    set(
        m,
        "roles.0.extendedProperties",
        json!([{"type": "json", "name": "RoleJson", "value": {"k": true}}]),
    )?;
    push(
        m,
        "roles",
        json!({"name": "Writer", "modelPermission": "read",
               "members": [{"memberName": "Admins", "memberId": "S-1-5-32-544"}],
               "tablePermissions": [{"name": "Sales", "filterExpression": "TRUE()",
                                     "columnPermissions": [{"name": "Amount", "metadataPermission": "none"}],
                                     "annotations": [{"name": "T", "value": "p"}]}]}),
    )?;
    set(m, "expressions.0.extendedProperties.0.value", json!("e2"))
}

fn remove(m: &mut Value) -> R {
    drop_named(m, "tables.[Sales].measures", "Total")?;
    for p in items(at(m, "perspectives")?) {
        let Some(tables) = p.get_mut("tables") else {
            continue;
        };
        for t in items(tables) {
            let Some(t) = t.as_object_mut() else { continue };
            let Some(ms) = t.get_mut("measures") else { continue };
            let Some(a) = ms.as_array_mut() else {
                return Err("perspective measures: not a list".into());
            };
            a.retain(|x| x.get("name").and_then(Value::as_str) != Some("Total"));
            if a.is_empty() {
                t.shift_remove("measures");
            }
        }
    }
    for t in items(at(m, "cultures.[fr-FR].translations.model.tables")?) {
        if let Some(t) = t.as_object_mut() {
            t.shift_remove("measures");
        }
    }
    drop_named(m, "cultures", "en-US")?;
    del(m, "tables.[Sales].defaultDetailRowsDefinition")?;
    for x in items(at(m, "tables.[Sales].measures")?) {
        if let Some(x) = x.as_object_mut() {
            x.shift_remove("formatStringDefinition");
        }
    }
    del(m, "tables.[Param].columns.[Param].relatedColumnDetails")?;
    pop(m, "tables.[Time Calc].calculationGroup.noSelectionExpression")?;
    pop(
        m,
        "tables.[Time Calc].calculationGroup.multipleOrEmptySelectionExpression",
    )?;
    pop(m, "roles.0.members")?;
    pop(m, "roles.0.tablePermissions.0.columnPermissions")?;
    pop(m, "extendedProperties")?;
    pop(m, "tables.[Sales].hierarchies.0.annotations")?;
    pop(m, "tables.[Sales].hierarchies.0.levels.0.annotations")?;
    pop(m, "tables.[Sales].calendars")
}

fn calendars(m: &mut Value) -> R {
    let cal = "tables.[Sales].calendars.0";
    set(m, &format!("{cal}.description"), json!("cal 2"))?;
    let g = format!("{cal}.calendarColumnGroups");
    set(m, &format!("{g}.0.primaryColumn"), json!("Amount"))?;
    set(m, &format!("{g}.1.associatedColumns"), json!(["City", "Id"]))?;
    set(m, &format!("{g}.1.timeUnit"), json!("month"))?;
    set(m, &format!("{g}.2.columns"), json!(["Double"]))?;
    push(
        m,
        "tables.[Sales].calendars",
        json!({"name": "Second", "calendarColumnGroups": [{"timeUnit": "month", "primaryColumn": "Region"}]}),
    )?;
    push(
        m,
        "cultures.[en-US].linguisticMetadata.annotations",
        json!({"name": "Lm2", "value": "z"}),
    )?;
    let member = items(at(m, "roles.0.members")?)
        .find(|x| x.get("annotations").is_some())
        .ok_or("no role member with annotations")?;
    set(member, "annotations.0.value", json!("mv2"))?;
    pop(member, "extendedProperties")?;
    push(
        m,
        "roles.0.tablePermissions.0.columnPermissions.0.annotations",
        json!({"name": "Cp2", "value": "q"}),
    )?;
    set(m, "perspectives.0.extendedProperties.0.value", json!("pe2"))?;
    del(m, "perspectives.0.tables.0.hierarchies.0.annotations")?;
    push(
        m,
        "tables.[Sales].measures.0.kpi.extendedProperties",
        json!({"type": "json", "name": "KpiJson", "value": {"list": [1, 2]}}),
    )
}

fn sources(m: &mut Value) -> R {
    set(m, "functions.0.expression", json!("(x) => x * 3"))?;
    set(m, "functions.0.description", json!("triples"))?;
    set(m, "functions.0.annotations.0.value", json!("f2"))?;
    push(
        m,
        "functions",
        json!({"name": "AddOne", "expression": "(x) => x + 1", "isHidden": true,
               "lineageTag": "55555555-5555-4555-8555-555555555555"}),
    )?;
    set(
        m,
        "dataSources.[SQL/srv;db].description",
        json!("structured ds 2"),
    )?;
    push(
        m,
        "dataSources.[SQL/srv;db].annotations",
        json!({"name": "Ds2", "value": "x"}),
    )?;
    set(m, "dataSources.[LegacyDS].timeout", json!(60))?;
    set(m, "dataSources.[LegacyDS].maxConnections", json!(4))?;
    let rp = "tables.[Inc].refreshPolicy";
    set(m, &format!("{rp}.incrementalPeriods"), json!(12))?;
    del(m, &format!("{rp}.pollingExpression"))?;
    set(m, &format!("{rp}.incrementalPeriodsOffset"), json!(-2))?;
    pop(m, "tables.[Imp].changedProperties")?;
    push(
        m,
        "tables.[Imp].columns.[A].changedProperties",
        json!({"property": "IsHidden"}),
    )?;
    pop(m, "tables.[Calc].measures.0.changedProperties")
}

fn aggregations(m: &mut Value) -> R {
    let agg = "tables.[Agg].columns";
    set(m, &format!("{agg}.[S].alternateOf.summarization"), json!("max"))?;
    set(
        m,
        &format!("{agg}.[C].alternateOf"),
        json!({"baseColumn": {"table": "Det", "column": "K"}, "summarization": "count"}),
    )?;
    del(m, &format!("{agg}.[G].alternateOf"))?;
    set(
        m,
        "dataSources.[WithOptions].options",
        json!({"commandTimeout": "PT2M"}),
    )?;
    set(
        m,
        "dataSources.[WithOptions].contextExpression",
        json!("let y = 2 in y"),
    )?;
    set(m, "dataSources.[Acct].account", json!("dom\\other"))
}

fn variation_props(m: &mut Value) -> R {
    let v = "tables.[Sales].columns.[OrderDate].variations.0";
    set(m, &format!("{v}.description"), json!("auto date"))?;
    set(
        m,
        &format!("{v}.annotations"),
        json!([{"name": "VarNote", "value": "v"}]),
    )?;
    let h = "tables.[LocalDateTable_0001].hierarchies.0";
    set(m, &format!("{h}.description"), json!("dates"))?;
    set(m, &format!("{h}.levels.0.description"), json!("year level"))
}

fn variation_rename(m: &mut Value) -> R {
    set(
        m,
        "tables.[Sales].columns.[OrderDate].variations.0.name",
        json!("Variation2"),
    )
}

fn preview_change(m: &mut Value) -> R {
    let s = "tables.[Sales].sets";
    set(
        m,
        &format!("{s}.[Big sales].expression"),
        json!("FILTER(VALUES(Sales[Region]), [Total] > 20)"),
    )?;
    set(m, &format!("{s}.[Big sales].isDynamic"), json!(false))?;
    set(m, &format!("{s}.[Big sales].annotations.0.value"), json!("s2"))?;
    del(m, &format!("{s}.[All regions].isHidden"))?;
    set(
        m,
        &format!("{s}.[All regions].description"),
        json!("every region"),
    )?;
    push(
        m,
        s,
        json!({"name": "Top regions", "expression": "TOPN(2, VALUES(Sales[Region]), [Total])",
               "displayFolder": "Sets", "annotations": [{"name": "New", "value": "n"}]}),
    )?;
    push(m, "perspectives.0.tables.0.sets", json!({"name": "All regions"}))?;
    set(
        m,
        "tables.[Sales].excludedArtifacts.0.reference",
        json!({"name": "Discount", "v": 2}),
    )?;
    push(
        m,
        "tables.[Sales].excludedArtifacts",
        json!({"artifactType": 9, "reference": {"name": "Geo"}}),
    )?;
    push(
        m,
        "expressions.0.excludedArtifacts",
        json!({"artifactType": 3, "reference": {"name": "archive"}}),
    )?;
    push(
        m,
        "excludedArtifacts",
        json!({"artifactType": 7, "reference": {"name": "Other"}}),
    )?;
    set(m, "bindingInfoCollection.0.description", json!("binding hint 2"))?;
    set(
        m,
        "bindingInfoCollection.0.connectionId",
        json!("00000000-0000-0000-0000-0000000000bb"),
    )?;
    push(
        m,
        "bindingInfoCollection.0.annotations",
        json!({"name": "Bind2", "value": "x"}),
    )?;
    push(
        m,
        "bindingInfoCollection",
        json!({"type": "dataBindingHint", "name": "Second binding", "connectionId": "c-2"}),
    )?;
    set(
        m,
        "analyticsAIMetadata.0.measureAnalysisDefinition",
        json!({"measures": ["Total"], "version": 2}),
    )?;
    push(
        m,
        "analyticsAIMetadata",
        json!({"name": "More", "measureAnalysisDefinition": {"x": [1, 2]}}),
    )
}

fn preview_remove(m: &mut Value) -> R {
    drop_named(m, "tables.[Sales].sets", "Big sales")?;
    del(m, "perspectives.0.tables.0.sets")?;
    let ex = at(m, "tables.[Sales].excludedArtifacts")?
        .as_array_mut()
        .ok_or("excludedArtifacts: not a list")?;
    if ex.is_empty() {
        return Err("excludedArtifacts: empty".into());
    }
    ex.remove(0);
    del(m, "expressions.0.excludedArtifacts")?;
    del(m, "excludedArtifacts")?;
    del(m, "bindingInfoCollection")?;
    del(m, "analyticsAIMetadata")
}

/// Applies `edit` to a TMSL document (`{"model": ...}`).
pub fn apply(edit: &str, doc: &mut Value) -> R {
    let f = EDITS
        .iter()
        .find(|(n, _)| *n == edit)
        .map(|(_, f)| *f)
        .ok_or_else(|| {
            let names: Vec<&str> = EDITS.iter().map(|(n, _)| *n).collect();
            format!("unknown edit `{edit}` (expected one of {})", names.join(", "))
        })?;
    let m = doc.get_mut("model").ok_or("no `model` in the TMSL")?;
    f(m).map_err(|e| format!("{edit}: {e}"))
}

pub fn cmd(args: &[String]) -> R {
    let [edit, src, dst] = args else {
        return Err("usage: cargo xtask metadata-edit EDIT IN.json OUT.json".into());
    };
    let mut doc = read_json(Path::new(src))?;
    apply(edit, &mut doc)?;
    write_text(Path::new(dst), &pyjson::dumps(&doc, Some(1), true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_paths() {
        let mut m = json!({"tables": [{"name": "Sales", "measures": [{"name": "Total", "kpi": {}}]}],
                           "roles": [{"name": "R", "members": [{"memberName": "a"}]}]});
        set(
            &mut m,
            "tables.[Sales].measures.[Total].kpi.targetExpression",
            json!("1"),
        )
        .ok();
        set(&mut m, "roles.0.members.0", json!({"memberName": "b"})).ok();
        assert_eq!(m["tables"][0]["measures"][0]["kpi"]["targetExpression"], "1");
        assert_eq!(m["roles"][0]["members"][0]["memberName"], "b");
        assert!(del(&mut m, "tables.[Sales].nothing").is_err());
        assert!(pop(&mut m, "tables.[Sales].nothing").is_ok());
        assert!(set(&mut m, "tables.[Other].x", json!(1)).is_err());
        let mut doc = json!({"model": m});
        assert!(
            apply("remove", &mut doc).is_err(),
            "missing objects fail, no panic"
        );
        assert!(apply("nope", &mut doc).is_err());
    }
}
