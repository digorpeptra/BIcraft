//! `tmdl-model`: the synthetic TMSL model of the TMDL oracle, with every TMDL object kind and the
//! tricky text cases (docs/formats/tmdl.md "TOM oracle").
//!
//! `cargo xtask tmdl-model [COMPATIBILITY_LEVEL] [--out FILE]` (default level 1706, default
//! output stdout). Everything here is written for bicraft's tests (names, expressions, values).

use std::path::Path;

use serde_json::{Value, json};

use super::{merged, pyjson, write_text};

const ML: &str = "VAR x =\n    1\n\nRETURN\n    x + 2";
const M_SRC: &str = "let\n    Source = #table({\"A\"}, {{1}}),\n    #\"Changed Type\" = Table.TransformColumnTypes(Source, {{\"A\", Int64.Type}})\nin\n    #\"Changed Type\"";

fn ann(name: &str, value: &str) -> Value {
    json!({"name": name, "value": value})
}

/// A data column: `dataType` (default string) and `sourceColumn` (default the name) come right
/// after the name, then `extra`.
fn col(name: &str, extra: Value) -> Value {
    let mut extra = match extra {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };
    let dt = extra.shift_remove("dataType").unwrap_or_else(|| json!("string"));
    let sc = extra.shift_remove("sourceColumn").unwrap_or_else(|| json!(name));
    merged(
        json!({"name": name, "dataType": dt, "sourceColumn": sc}),
        Value::Object(extra),
    )
}

fn c(name: &str) -> Value {
    col(name, json!({}))
}

fn calc(name: &str, expression: &str, data_type: &str, extra: Value) -> Value {
    merged(
        json!({"name": name, "type": "calculated", "dataType": data_type, "expression": expression}),
        extra,
    )
}

fn order_lines() -> Value {
    json!({
        "name": "Order Lines",
        "description": "Line one\nLine two with trailing space \n\n/// starts like a description",
        "lineageTag": "t-ol",
        "dataCategory": "Time",
        "columns": [
            col("Key", json!({"dataType": "int64", "isKey": true, "isUnique": true, "isNullable": false,
                "summarizeBy": "none", "lineageTag": "c1", "sourceLineageTag": "[dbo].[Key]",
                "formatString": "0", "displayFolder": "Keys", "isAvailableInMdx": false,
                "keepUniqueRows": true, "displayOrdinal": 3, "sourceProviderType": "int",
                "encodingHint": "value", "alignment": "right", "tableDetailPosition": 1,
                "isDefaultLabel": true, "isDefaultImage": false, "dataCategory": "Uncategorized",
                "annotations": [ann("SummarizationSetBy", "User")]})),
            col("It's \"quoted\"", json!({"formatString": " lead", "displayFolder": "trail ",
                "description": "d\r\nwith CRLF"})),
            col("2024", json!({"formatString": "1.50", "displayFolder": "0"})),
            col("table", json!({"formatString": "true", "displayFolder": "null"})),
            col("colon", json!({"formatString": "\"q\""})),
            col("a:b#c[d]", json!({"formatString": ""})),
            c("Ünïcødé ✓"),
            col("-dash", json!({"sourceColumn": "/slash"})),
            col(" lead space", json!({"sourceColumn": " lead space"})),
            col("trail ", json!({"sourceColumn": "trail "})),
            col("Sort", json!({"sortByColumn": "Key"})),
            col("Month Name", json!({"sortByColumn": "It's \"quoted\""})),
            calc("Calc 1", "1", "double", json!({"isDataTypeInferred": true})),
            calc("Calc ML", "\n    VAR a = 1\n    RETURN\n        a", "int64", json!({})),
            calc("Calc trailing", "\"x\" ", "string", json!({})),
            calc("Calc lead ws", "  \"x\"\n  & \"y\"", "string", json!({})),
            calc("Calc nl end", "\"x\"\n", "string", json!({})),
            calc("Calc fence", "\"```\" & \"a\"", "string", json!({})),
            calc("Calc crlf", "\"a\"\r\n& \"b\"", "string", json!({})),
            calc("Calc tabs", "IF(TRUE(),\n\t\"a\",\n\t\"b\")", "string", json!({})),
            calc("Calc blank lines", "1\n\n\n+ 2", "string", json!({})),
            col("Alt", json!({"dataType": "int64",
                "alternateOf": {"summarization": "sum", "baseColumn": {"table": "Product", "column": "Product Key"}}})),
        ],
        "measures": [
            {"name": "Sales Amount", "expression": "SUM('Order Lines'[Key])", "formatString": "#,0.00",
             "description": "Sum.", "displayFolder": "Key Figures\\Sub", "lineageTag": "m1",
             "sourceLineageTag": "sm1", "dataCategory": "WebUrl", "isSimpleMeasure": true,
             "kpi": {"description": "kpi desc", "targetDescription": "td", "targetExpression": "100",
                     "targetFormatString": "0", "statusGraphic": "Shapes", "statusDescription": "sd",
                     "statusExpression": "VAR x = [Sales Amount]\nRETURN IF(x > 100, 1, -1)",
                     "trendGraphic": "Standard Arrow", "trendDescription": "trd", "trendExpression": "1",
                     "annotations": [ann("KA", "1")]},
             "detailRowsDefinition": {"expression": "'Order Lines'"},
             "formatStringDefinition": {"expression": "IF(1, \"0\",\n    \"0.0\")"},
             "extendedProperties": [
                 {"type": "json", "name": "PBI_ProTooling", "value": {"tools": ["DevMode", "CalcGroup"]}},
                 {"type": "string", "name": "S", "value": "text value"},
                 {"type": "json", "name": "J", "value": {"version": 0, "a": [1, 2]}},
                 {"type": "string", "name": "S ML", "value": "a\nb"},
             ],
             "changedProperties": [{"property": "Name"}, {"property": "FormatString"}],
             "annotations": [ann("A", "1"), ann("Empty", ""), ann("Multi", "line1\nline2"),
                             ann("Json", "{\"a\": 1}"), ann("Lead", " lead"), ann("It's", "x"),
                             ann("Trail", "x ")]},
            {"name": "ML", "expression": ML, "isHidden": true},
            {"name": "Empty", "expression": ""},
            {"name": "Fence", "expression": "  lead\n  more  "},
            {"name": "Quote'n", "expression": "1", "formatString": "\"$\"#,0"},
            {"name": "Dot.Name", "expression": "1", "displayFolder": "  "},
            {"name": "Eq=Name", "expression": "1 = 1"},
            {"name": "Hash#", "expression": "-- comment\n1"},
            {"name": "Desc ML", "expression": "1", "description": "a\n\nb"},
            {"name": "Desc trail", "expression": "1", "description": "trailing   "},
            {"name": "Desc lead", "expression": "1", "description": "  lead"},
            {"name": "Typed", "expression": "1", "dataType": "int64"},
        ],
        "hierarchies": [
            {"name": "H 1", "description": "hd", "isHidden": true, "displayFolder": "HF",
             "hideMembers": "hideBlankMembers", "lineageTag": "h1", "sourceLineageTag": "sh1",
             "levels": [
                 {"name": "L 1", "ordinal": 0, "column": "Key", "lineageTag": "l1", "description": "ld"},
                 {"name": "L'2", "ordinal": 1, "column": "It's \"quoted\""},
             ],
             "changedProperties": [{"property": "IsHidden"}],
             "annotations": [ann("HA", "1")]},
        ],
        "partitions": [
            {"name": "Order Lines-1", "mode": "import", "dataView": "full", "queryGroup": "Staging\\Sub",
             "description": "pd", "source": {"type": "m", "expression": M_SRC},
             "annotations": [ann("PA", "1")]},
            {"name": "P2", "mode": "directQuery", "source": {"type": "m", "expression": "#\"Sales Staging\""},
             "dataCoverageDefinition": {"description": "dcd",
                                        "expression": "RELATED('Product'[Category]) = \"Bikes\""}},
        ],
        "annotations": [ann("PBI_ResultType", "Table"), ann("ML", "a\nb")],
        "extendedProperties": [{"type": "string", "name": "TE", "value": "v"}],
        "changedProperties": [{"property": "Name"}],
    })
}

fn other_tables() -> Vec<Value> {
    let x_src = "let\n    x = 1\nin\n    x";
    vec![
        json!({
            "name": "Product",
            "lineageTag": "t-p",
            "isHidden": true,
            "isPrivate": true,
            "showAsVariationsOnly": true,
            "excludeFromModelRefresh": true,
            "alternateSourcePrecedence": 2,
            "sourceLineageTag": "[dbo].[Product]",
            "systemManaged": true,
            "excludeFromAutomaticAggregations": true,
            "columns": [
                col("Product Key", json!({"dataType": "int64"})),
                col("Category", json!({"stringIndexingBehavior": "full"})),
                col("Date", json!({"dataType": "dateTime",
                    "variations": [{"name": "Variation", "description": "vd", "isDefault": true,
                                    "relationship": "rel date",
                                    "defaultHierarchy": {"table": "LocalDate", "hierarchy": "Date Hierarchy"},
                                    "annotations": [ann("VA", "1")]}]})),
                col("Bin", json!({"dataType": "binary"})),
                col("Dec", json!({"dataType": "decimal"})),
                col("Bool", json!({"dataType": "boolean"})),
                col("Dbl", json!({"dataType": "double"})),
            ],
            "measures": [{"name": "Count", "expression": "COUNTROWS(Product)"}],
            "partitions": [{"name": "Product", "mode": "import",
                            "source": {"type": "m", "expression": x_src}}],
            "defaultDetailRowsDefinition": {"expression": "SELECTCOLUMNS(Product, \"K\", Product[Product Key])"},
            "refreshPolicy": {"policyType": "basic", "mode": "hybrid", "rollingWindowGranularity": "year",
                              "rollingWindowPeriods": 3, "incrementalGranularity": "day", "incrementalPeriods": 10,
                              "incrementalPeriodsOffset": -1, "pollingExpression": "1",
                              "sourceExpression": ["let", "    x = 1", "in", "    x"]},
        }),
        json!({
            "name": "LocalDate",
            "isHidden": true,
            "showAsVariationsOnly": true,
            "columns": [
                {"name": "Date", "dataType": "dateTime", "isNameInferred": true, "sourceColumn": "[Date]",
                 "type": "calculatedTableColumn", "isDataTypeInferred": true},
                calc("Year", "YEAR([Date])", "int64", json!({})),
            ],
            "hierarchies": [{"name": "Date Hierarchy", "levels": [{"name": "Year", "ordinal": 0, "column": "Year"}]}],
            "partitions": [{"name": "LocalDate", "mode": "import",
                            "source": {"type": "calculated",
                                       "expression": "CALENDAR(DATE(2020,1,1), DATE(2020,12,31))"}}],
        }),
        json!({
            "name": "Time Intelligence",
            "calculationGroup": {
                "description": "cgd",
                "precedence": 10,
                "multipleOrEmptySelectionExpression": {"expression": "1", "description": "m",
                                                       "formatStringDefinition": {"expression": "\"0\""}},
                "noSelectionExpression": {"expression": "SELECTEDMEASURE()"},
                "calculationItems": [
                    {"name": "Current", "expression": "SELECTEDMEASURE()", "description": "cur"},
                    {"name": "YTD",
                     "expression": "CALCULATE(\n    SELECTEDMEASURE(),\n    DATESYTD('LocalDate'[Date])\n)",
                     "formatStringDefinition": {"expression": "\"#,0\""}, "ordinal": 1},
                    {"name": "Out of order", "expression": "1", "ordinal": 5},
                ],
                "annotations": [ann("CGA", "1")],
            },
            "columns": [{"name": "Name", "dataType": "string", "sourceColumn": "Name", "sortByColumn": "Ordinal"},
                        {"name": "Ordinal", "dataType": "int64", "sourceColumn": "Ordinal"}],
            "partitions": [{"name": "Partition", "mode": "import", "source": {"type": "calculationGroup"}}],
        }),
        json!({
            "name": "Entity",
            "columns": [c("A")],
            "partitions": [
                {"name": "E", "mode": "directLake",
                 "source": {"type": "entity", "entityName": "A", "schemaName": "dbo",
                            "expressionSource": "DL"}},
            ],
        }),
        json!({
            "name": "Legacy",
            "columns": [c("A")],
            "partitions": [
                {"name": "Q", "mode": "import",
                 "source": {"type": "query", "query": "SELECT *\nFROM dbo.A", "dataSource": "SQL Legacy"}},
            ],
        }),
        json!({
            "name": "Incremental",
            "columns": [col("A", json!({"dataType": "dateTime"}))],
            "partitions": [
                {"name": "2024", "mode": "import",
                 "source": {"type": "policyRange", "start": "2024-01-01T00:00:00", "end": "2025-01-01T00:00:00",
                            "granularity": "year", "refreshBookmark": "bm"}},
            ],
            "refreshPolicy": {"policyType": "basic", "rollingWindowGranularity": "year", "rollingWindowPeriods": 2,
                              "incrementalGranularity": "day", "incrementalPeriods": 3,
                              "sourceExpression": x_src},
        }),
        json!({
            "name": "Fiscal Calendar",
            "columns": [col("Date", json!({"dataType": "dateTime"})), col("Year", json!({"dataType": "int64"})),
                        c("Month"), col("Month No", json!({"dataType": "int64"})), c("Week Day")],
            "partitions": [{"name": "Fiscal Calendar", "mode": "import",
                            "source": {"type": "m", "expression": x_src}}],
            "calendars": [
                {"name": "Fiscal", "description": "fiscal calendar", "lineageTag": "cal1",
                 "calendarColumnGroups": [
                     {"timeUnit": "year", "primaryColumn": "Year"},
                     {"timeUnit": "monthOfYear", "primaryColumn": "Month No",
                      "associatedColumns": ["Month", "Week Day"]},
                     {"columns": ["Date", "Week Day"]},
                 ]},
            ],
        }),
        json!({
            "name": "Calc Table",
            "columns": [{"name": "Value", "dataType": "int64", "isNameInferred": true, "sourceColumn": "[Value]",
                         "type": "calculatedTableColumn"}],
            "partitions": [{"name": "Calc Table", "mode": "import",
                            "source": {"type": "calculated", "expression": "GENERATESERIES(1, 10)"}}],
        }),
    ]
}

pub fn model(level: i64) -> Value {
    let mut tables = vec![order_lines()];
    tables.extend(other_tables());
    let relationships = json!([
        {"name": "rel 1", "fromTable": "Order Lines", "fromColumn": "Key", "toTable": "Product",
         "toColumn": "Product Key", "crossFilteringBehavior": "bothDirections",
         "relyOnReferentialIntegrity": true, "securityFilteringBehavior": "bothDirections",
         "annotations": [ann("RA", "1")]},
        {"name": "rel date", "fromTable": "Product", "fromColumn": "Date", "toTable": "LocalDate",
         "toColumn": "Date", "joinOnDateBehavior": "datePartOnly", "isActive": false},
        {"name": "d0e1f2a3-many", "fromTable": "Order Lines", "fromColumn": "It's \"quoted\"",
         "toTable": "Calc Table", "toColumn": "Value", "fromCardinality": "many",
         "toCardinality": "many", "crossFilteringBehavior": "oneDirection"},
    ]);
    let data_sources = json!([
        {"type": "provider", "name": "SQL Legacy",
         "connectionString": "Provider=SQLNCLI11;Data Source=srv;Initial Catalog=db",
         "impersonationMode": "impersonateServiceAccount", "provider": "System.Data.SqlClient",
         "timeout": 30, "isolation": "snapshot", "maxConnections": 4, "description": "legacy ds"},
        {"type": "structured", "name": "SQL/srv;db",
         "connectionDetails": {"protocol": "tds", "address": {"server": "srv", "database": "db"}},
         "credential": {"AuthenticationKind": "UsernamePassword", "kind": "SQL", "path": "srv;db",
                        "Username": "u", "EncryptConnection": false},
         "options": {"commandTimeout": "PT5M"}, "contextExpression": ["let", "  x = 1", "in", "  x"]},
    ]);
    let expressions = json!([
        {"name": "Sales Staging", "kind": "m", "expression": M_SRC, "lineageTag": "e1",
         "queryGroup": "Staging", "description": "staging query",
         "annotations": [ann("PBI_ResultType", "Table")]},
        {"name": "Param", "kind": "m",
         "expression": "\"x\" meta [IsParameterQuery=true, Type=\"Text\", IsParameterQueryRequired=true]",
         "queryGroup": "Parameters", "remoteParameterName": "RP",
         "parameterValuesColumn": {"table": "Product", "column": "Category"}},
        {"name": "DL", "kind": "m", "expression": "let\n    d = Sql.Database(\"s\", \"d\")\nin\n    d",
         "sourceLineageTag": "se", "mAttributes": "[ Source = 1 ]"},
        {"name": "DL Copy", "kind": "m", "expression": "1", "expressionSource": "DL"},
    ]);
    let perspectives = json!([
        {"name": "Sales View", "description": "pd",
         "tables": [
             {"name": "Order Lines", "columns": [{"name": "Key"}], "measures": [{"name": "Sales Amount"}],
              "hierarchies": [{"name": "H 1"}], "annotations": [ann("PTA", "1")]},
             {"name": "Product", "includeAll": true},
         ],
         "annotations": [ann("PA", "1")]},
    ]);
    let roles = json!([
        {"name": "Sales Reader", "description": "Can read.", "modelPermission": "read",
         "members": [
             {"memberName": "user@contoso.com", "identityProvider": "AzureAD", "memberType": "user"},
             {"memberName": "Group A", "memberType": "group", "identityProvider": "AzureAD",
              "memberId": "abc"},
             {"memberName": "DOMAIN\\user", "memberId": "S-1-5-21"},
         ],
         "tablePermissions": [
             {"name": "Order Lines", "filterExpression": "'Order Lines'[Key] > 0",
              "columnPermissions": [{"name": "Key", "metadataPermission": "none"},
                                    {"name": "It's \"quoted\"", "metadataPermission": "read"}],
              "annotations": [ann("TPA", "1")]},
             {"name": "Product",
              "filterExpression": "Product[Category] = \"Bikes\"\n    || Product[Category] = \"Other\""},
             {"name": "LocalDate", "metadataPermission": "none"},
         ],
         "annotations": [ann("RA", "1")]},
        {"name": "Admins", "modelPermission": "administrator"},
    ]);
    let cultures = json!([
        {"name": "en-US", "linguisticMetadata": {"content": {"Version": "1.0.0", "Language": "en-US"},
                                                 "contentType": "json"}},
        {"name": "fr-FR",
         "translations": {"model": {
             "name": "Model", "translatedCaption": "Modèle", "translatedDescription": "multi\nline",
             "tables": [{"name": "Order Lines", "translatedCaption": "Lignes", "translatedDescription": "desc",
                         "columns": [{"name": "Key", "translatedCaption": "Clé",
                                      "translatedDisplayFolder": "Clés"},
                                     {"name": "It's \"quoted\"", "translatedCaption": " lead"}],
                         "measures": [{"name": "Sales Amount", "translatedCaption": "Montant",
                                       "translatedDisplayFolder": "Chiffres clés"}],
                         "hierarchies": [{"name": "H 1", "translatedCaption": "H",
                                          "levels": [{"name": "L 1", "translatedCaption": "N 1"}]}]}],
             "perspectives": [{"name": "Sales View", "translatedCaption": "Vue"}],
             "roles": [{"name": "Sales Reader", "translatedCaption": "Lecteur",
                        "translatedDescription": "Peut lire."}],
             "expressions": [{"name": "Param", "translatedCaption": "Paramètre"}]}},
         "linguisticMetadata": {"content": "<xml/>", "contentType": "xml"}},
    ]);
    json!({
        "name": "Oracle Model",
        "compatibilityLevel": level,
        "model": {
            "description": "model\ndescription",
            "culture": "en-US",
            "defaultPowerBIDataSourceVersion": "powerBI_V3",
            "discourageImplicitMeasures": true,
            "sourceQueryCulture": "fr-FR",
            "collation": "Latin1_General_CI_AS",
            "dataSourceVariablesOverrideBehavior": "allow",
            "dataSourceDefaultMaxConnections": 5,
            "mAttributes": "[ a = 1 ]",
            "discourageCompositeModels": true,
            "disableAutoExists": 1,
            "maxParallelismPerRefresh": 3,
            "maxParallelismPerQuery": 2,
            "directLakeBehavior": "automatic",
            "valueFilterBehavior": "independent",
            "selectionExpressionBehavior": "nonVisual",
            "metadataAccessPolicy": "basic",
            "dataAccessOptions": {"legacyRedirects": true, "returnErrorValuesAsNull": true},
            "tables": tables,
            "relationships": relationships,
            "dataSources": data_sources,
            "expressions": expressions,
            "queryGroups": [{"folder": "Staging", "description": "qg", "annotations": [ann("QA", "1")]},
                            {"folder": "Staging\\Sub"}, {"folder": "Parameters"}],
            "perspectives": perspectives,
            "roles": roles,
            "cultures": cultures,
            "functions": [
                {"name": "AddOne", "description": "Adds one.", "expression": "(x : NUMERIC) =>\n    x + 1",
                 "isHidden": true, "lineageTag": "f1", "annotations": [ann("FA", "1")]},
                {"name": "Fn.Two", "expression": "() => 2"},
            ],
            "bindingInfoCollection": [
                {"type": "dataBindingHint", "name": "B1", "description": "bd", "connectionId": "conn-1"},
            ],
            "annotations": [ann("PBI_QueryOrder", "[\"Sales Staging\",\"Param\"]"),
                            ann("__PBI_TimeIntelligenceEnabled", "1")],
            "extendedProperties": [{"type": "string", "name": "MEP", "value": "m"}],
        },
    })
}

/// The model's text: two-space indentation, UTF-8, final newline.
pub fn text(level: i64) -> String {
    pyjson::dumps(&model(level), Some(2), false) + "\n"
}

pub fn cmd(args: &[String]) -> Result<(), String> {
    let mut level = 1706;
    let mut out = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => out = Some(it.next().ok_or("--out needs a file")?),
            x => level = x.parse().map_err(|_| format!("bad compatibility level `{x}`"))?,
        }
    }
    let t = text(level);
    match out {
        Some(p) => write_text(Path::new(p), &t),
        None => {
            print!("{t}");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tmdl_oracle_model_shape() {
        let m = model(1706);
        assert_eq!(m["compatibilityLevel"], 1706);
        let t = text(1600);
        assert!(t.starts_with("{\n  \"name\": \"Oracle Model\",\n  \"compatibilityLevel\": 1600,"));
        assert!(t.contains("\"Ünïcødé ✓\"") && t.ends_with("}\n"));
        let keys: Vec<&str> = m["model"]
            .as_object()
            .map(|o| o.keys().map(String::as_str).collect())
            .unwrap_or_default();
        assert_eq!(keys.len(), 30);
        assert!(
            keys.iter().skip(19).take(6).eq([
                "relationships",
                "dataSources",
                "expressions",
                "queryGroups",
                "perspectives",
                "roles"
            ]
            .iter())
        );
        // col(): dataType and sourceColumn right after the name, the rest after them
        assert_eq!(
            pyjson::dumps(&col("-dash", json!({"sourceColumn": "/slash"})), None, false),
            r#"{"name": "-dash", "dataType": "string", "sourceColumn": "/slash"}"#
        );
    }
}
