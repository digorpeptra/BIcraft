//! MCP tool definitions (names, descriptions, JSON Schemas) and their implementations on top of
//! the engine. Generic tools (`list_commands`, `run_command`, `batch`) reach every command; the
//! convenience tools map 1:1 to one command each.

use bicraft_engine::Session;
use serde_json::{Map, Value, json};

/// The result of `tools/call`: MCP content blocks, structured content and the `isError` flag.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolResult {
    /// Always a JSON object (MCP requires `structuredContent` to be one).
    pub structured: Value,
    pub is_error: bool,
}

impl ToolResult {
    /// A successful result. Non-object values are wrapped as `{"result": v}`.
    pub fn ok(v: Value) -> Self {
        Self {
            structured: as_object(v),
            is_error: false,
        }
    }

    /// A tool-level error with a plain message.
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            structured: json!({"error": message.into()}),
            is_error: true,
        }
    }

    /// A tool-level error carrying extra JSON (e.g. a batch's partial results).
    pub fn error_with(v: Value) -> Self {
        Self {
            structured: as_object(v),
            is_error: true,
        }
    }

    /// The text shown to the model: the message for plain errors, else the pretty JSON.
    pub fn text(&self) -> String {
        if self.is_error
            && let Some(Value::String(m)) = self.structured.get("error")
            && self.structured.as_object().is_some_and(|o| o.len() == 1)
        {
            return m.clone();
        }
        serde_json::to_string_pretty(&self.structured).unwrap_or_default()
    }

    /// [`ToolResult::to_value`] without copying the structured result (it is moved in).
    pub fn into_value(self) -> Value {
        if self.structured.get("pages").is_some() {
            return self.to_value();
        }
        let text = self.text();
        let mut o = Map::with_capacity(3);
        o.insert(
            "content".into(),
            Value::Array(vec![json!({"type": "text", "text": text})]),
        );
        o.insert("structuredContent".into(), self.structured);
        o.insert("isError".into(), Value::Bool(self.is_error));
        Value::Object(o)
    }

    /// The JSON-RPC response line `{"jsonrpc":"2.0","id":…,"result":…}` carrying this result,
    /// byte for byte what serializing `{"jsonrpc", "id", "result": into_value()}` gives, written
    /// straight from the structured result: no copy of it, and its pretty text escaped as it is
    /// produced instead of being built as a string first. `None` when that is not possible
    /// (a result with image blocks): the caller builds the value.
    pub fn response_line(&self, id: &Value) -> Option<String> {
        if self.structured.get("pages").is_some() {
            return None;
        }
        let mut out: Vec<u8> = Vec::new();
        out.extend_from_slice(br#"{"jsonrpc":"2.0","id":"#);
        serde_json::to_writer(&mut out, id).ok()?;
        out.extend_from_slice(br#","result":{"content":[{"type":"text","text":"#);
        match self.plain_error() {
            Some(m) => serde_json::to_writer(&mut out, m).ok()?,
            None => {
                out.push(b'"');
                serde_json::to_writer_pretty(Escaped(&mut out), &self.structured).ok()?;
                out.push(b'"');
            }
        }
        out.extend_from_slice(br#"}],"structuredContent":"#);
        serde_json::to_writer(&mut out, &self.structured).ok()?;
        out.extend_from_slice(if self.is_error {
            br#","isError":true}}"#
        } else {
            br#","isError":false}}"#
        });
        String::from_utf8(out).ok()
    }

    /// The message of a plain error (`{"error": "…"}` only), shown as the text as it is.
    fn plain_error(&self) -> Option<&str> {
        match self.structured.get("error") {
            Some(Value::String(m))
                if self.is_error && self.structured.as_object().is_some_and(|o| o.len() == 1) =>
            {
                Some(m)
            }
            _ => None,
        }
    }

    pub fn to_value(&self) -> Value {
        // report.render's inline PNGs become image content blocks (not base64 text)
        let mut structured = self.structured.clone();
        let images = take_images(&mut structured);
        if images.is_empty() {
            return json!({
                "content": [{"type": "text", "text": self.text()}],
                "structuredContent": self.structured,
                "isError": self.is_error,
            });
        }
        let text = serde_json::to_string_pretty(&structured).unwrap_or_default();
        let mut content = vec![json!({"type": "text", "text": text})];
        content.extend(images);
        json!({"content": content, "structuredContent": structured, "isError": self.is_error})
    }
}

/// Writes the bytes it is given as the inside of a JSON string, escaped as `serde_json` escapes
/// strings: `"` and `\` with a backslash, `\b \t \n \f \r` by name, other control bytes as
/// `\u00XX` (lower-case hex), everything else (UTF-8 included) as it is.
struct Escaped<'a>(&'a mut Vec<u8>);

impl std::io::Write for Escaped<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut start = 0;
        for (i, &b) in buf.iter().enumerate() {
            let short: &[u8] = match b {
                b'"' => b"\\\"",
                b'\\' => b"\\\\",
                b'\n' => b"\\n",
                b'\r' => b"\\r",
                b'\t' => b"\\t",
                0x08 => b"\\b",
                0x0c => b"\\f",
                0x00..=0x1f => b"",
                _ => continue,
            };
            self.0.extend_from_slice(buf.get(start..i).unwrap_or_default());
            if short.is_empty() {
                let hi = HEX.get(usize::from(b >> 4)).copied().unwrap_or(b'0');
                let lo = HEX.get(usize::from(b & 0xf)).copied().unwrap_or(b'0');
                self.0.extend_from_slice(&[b'\\', b'u', b'0', b'0', hi, lo]);
            } else {
                self.0.extend_from_slice(short);
            }
            start = i.saturating_add(1);
        }
        self.0.extend_from_slice(buf.get(start..).unwrap_or_default());
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Moves `pages[].image` (`{mime, base64}`) out of a result into MCP image blocks.
fn take_images(v: &mut Value) -> Vec<Value> {
    let mut out = Vec::new();
    let Some(pages) = v.get_mut("pages").and_then(Value::as_array_mut) else {
        return out;
    };
    for p in pages {
        let Some(o) = p.as_object_mut() else { continue };
        let Some(img) = o.get("image") else { continue };
        let (Some(mime), Some(data)) = (
            img.get("mime").and_then(Value::as_str),
            img.get("base64").and_then(Value::as_str),
        ) else {
            continue;
        };
        out.push(json!({"type": "image", "mimeType": mime, "data": data}));
        o.insert("image".into(), json!({"mime": mime, "content_block": out.len()}));
    }
    out
}

fn as_object(v: Value) -> Value {
    match v {
        Value::Object(_) => v,
        other => json!({"result": other}),
    }
}

/// `(tool name, engine command)` for the 1:1 convenience tools.
const CONVENIENCE: &[(&str, &str)] = &[
    ("open_file", "file.open"),
    ("inspect_file", "file.inspect"),
    ("model_summary", "model.summary"),
    ("validate_dax", "dax.validate"),
    ("validate_model", "model.validate"),
    ("model_usage", "model.usage"),
    ("model_unused", "model.unused"),
    ("remove_unused", "model.remove_unused"),
    ("new_model", "model.new"),
    ("add_table", "table.add"),
    ("add_column", "column.add"),
    ("add_measure", "measure.add"),
    ("set_measure", "measure.set"),
    ("add_relationship", "relationship.add"),
    ("save", "file.save"),
    ("save_as", "file.save_as"),
    ("convert", "file.convert"),
    ("diff_files", "file.diff"),
    ("import_objects", "objects.import"),
    ("new_project", "project.new"),
    ("report_summary", "report.summary"),
    ("add_page", "page.add"),
    ("add_visual", "visual.add"),
    ("bind_field", "visual.bind"),
    ("validate_report", "report.validate"),
    ("validate_dax_query", "dax.validate_query"),
    ("render_page", "report.render"),
    ("evaluate_dax_query", "dax.evaluate"),
    ("refresh_model", "model.refresh"),
    ("run_tests", "project.test"),
    ("list_sources", "sources.list"),
    ("rebase_sources", "sources.rebase"),
    ("apply_env", "env.apply"),
    ("check_env", "env.check"),
];

const GENERIC: &[&str] = &["list_commands", "run_command", "batch"];

pub(crate) fn is_tool(name: &str) -> bool {
    GENERIC.contains(&name) || CONVENIENCE.iter().any(|(t, _)| *t == name)
}

/// The tool's own static name (`None`: not a tool): logs and metrics never echo a client string.
pub(crate) fn static_tool_name(name: &str) -> Option<&'static str> {
    GENERIC
        .iter()
        .copied()
        .chain(CONVENIENCE.iter().map(|(t, _)| *t))
        .find(|t| *t == name)
}

/// The command label of a tool call for metrics and logs, always from a fixed set: the engine
/// command a convenience tool or `run_command` runs (an id of [`bicraft_engine::COMMANDS`]),
/// `list_commands`, `batch`, or `unknown` (a `run_command` naming no command). Never a
/// parameter.
pub(crate) fn command_label(name: &str, args: &Value) -> &'static str {
    let engine = |id: &str| bicraft_engine::find_command(id).map_or(UNKNOWN_COMMAND, |c| c.id);
    match name {
        "list_commands" => "list_commands",
        "batch" => "batch",
        "run_command" => args
            .get("command")
            .and_then(Value::as_str)
            .map_or(UNKNOWN_COMMAND, engine),
        _ => CONVENIENCE
            .iter()
            .find(|(t, _)| *t == name)
            .map_or(UNKNOWN_COMMAND, |(_, c)| engine(c)),
    }
}

/// The label of a call naming no known command.
pub(crate) const UNKNOWN_COMMAND: &str = "unknown";

/// The engine command of a convenience tool (`open_file` → `file.open`); `None` for the generic
/// tools and unknown names.
pub(crate) fn tool_command(name: &str) -> Option<&'static str> {
    CONVENIENCE.iter().find(|(t, _)| *t == name).map(|(_, c)| *c)
}

/// Every label [`command_label`] can return.
pub(crate) fn command_labels() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = bicraft_engine::COMMANDS.iter().map(|c| c.id).collect();
    v.extend(["list_commands", "batch", UNKNOWN_COMMAND]);
    v
}

fn string(desc: &str) -> Value {
    json!({"type": "string", "description": desc})
}

fn boolean(desc: &str) -> Value {
    json!({"type": "boolean", "description": desc})
}

/// A string, or `null` to clear the property (`*.set` tools).
fn clearable(desc: &str) -> Value {
    json!({"type": ["string", "null"], "description": format!("{desc} (null or \"\" clears it)")})
}

fn one_of(values: &[&str], desc: &str) -> Value {
    json!({"type": "string", "enum": values, "description": desc})
}

fn number(desc: &str) -> Value {
    json!({"type": "number", "description": desc})
}

fn integer(desc: &str) -> Value {
    json!({"type": "integer", "minimum": 0, "description": desc})
}

const PAGE: &str = "Page name or display name (report_summary lists them)";
const FIELD: &str = "Table[Column], [Measure], or an aggregated column such as Sum(Sales[Amount]) \
                     (Sum, Avg, Count = distinct count, Min, Max, CountNonNull, Median, StandardDeviation, Variance)";
const ROLE: &str = "Data role of the visual, e.g. Values (card, table), Category / Y / Series (legend) / \
                    Tooltips (charts), Rows, Columns, Values (matrix); visual.options lists each type's roles";
const UPDATE_REPORT: &str = "Also rewrite the report's visual bindings and filters that use the old name";

const UPDATE_DAX: &str =
    "Rewrite the DAX expressions that reference the old name (default true; false leaves them broken)";

const FORCE: &str = "Apply even if the DAX does not validate; the diagnostics come back as warnings";
const DATA_TYPES: &[&str] = &[
    "string", "int64", "double", "decimal", "dateTime", "boolean", "binary", "variant",
];
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

fn schema(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

/// Every tool with its description and input schema, as `tools/list` returns them.
pub fn tool_definitions() -> Vec<Value> {
    let params = json!({
        "type": "object",
        "description": "Command parameters (see list_commands for each command's params doc)",
    });
    vec![
        json!({
            "name": "list_commands",
            "title": "List commands",
            "description": "Every engine command (id, label, params doc; its button: place, shortcut, needs, form; whether it is enabled now and why not), optionally filtered by a substring of the id or label. On a server (bicraft serve), commands the server refuses are not listed.",
            "inputSchema": schema(json!({"filter": string("Case-insensitive substring of the id or label")}), &[]),
            "annotations": {"readOnlyHint": true},
        }),
        json!({
            "name": "run_command",
            "title": "Run a command",
            "description": "Run one engine command by id, e.g. {\"command\": \"dax.validate\", \"params\": {\"expression\": \"SUM(Sales[Amount])\"}}.",
            "inputSchema": schema(json!({"command": string("Command id, e.g. file.open"), "params": params, "expected_revision": integer(EXPECTED_REVISION_DOC)}), &["command"]),
        }),
        json!({
            "name": "batch",
            "title": "Run several commands",
            "description": "Run commands in order on the same session; stops at the first error and reports its index and the results so far. Atomic by default: a failure puts the session back as it was before the batch (files written by save steps stay written), and a committed batch is one edit.undo step.",
            "inputSchema": schema(json!({
                "commands": {
                    "type": "array",
                    "description": "The commands to run, in order",
                    "items": schema(json!({"command": string("Command id"), "params": params}), &["command"]),
                },
                "atomic": boolean("All or nothing (default true); false keeps the steps before a failure applied"),
                "expected_revision": integer(EXPECTED_REVISION_DOC),
            }), &["commands"]),
        }),
        json!({
            "name": "open_file",
            "title": "Open a Power BI file",
            "description": "Open a .pbix/.pbit file, a .pbip file or project folder, or a model.bim, or import the Power Pivot data model of an Excel .xlsx/.xlsm workbook (a new model held in memory: save it with save_as); returns what inspect_file returns.",
            "inputSchema": schema(json!({
                "path": string("Path of the file or project folder"),
                "data_root": string("Optional: the only folder the tables' Power Query sources (File.Contents) may read when bicraft runs them; none by default"),
                "cache_abf": string("Optional, PBIP only: read the project's data from this cache.abf file instead of its own .pbi/cache.abf (read-only, never written or copied; it must exist)"),
            }), &["path"]),
        }),
        json!({
            "name": "inspect_file",
            "title": "Inspect the open file",
            "description": "Describe the open file: kind, parts and formats, and whether its model could be read.",
            "inputSchema": schema(json!({}), &[]),
            "annotations": {"readOnlyHint": true},
        }),
        json!({
            "name": "model_summary",
            "title": "Summarise the model",
            "description": "List the tables, columns, measures and relationships of the open model.",
            "inputSchema": schema(json!({}), &[]),
            "annotations": {"readOnlyHint": true},
        }),
        json!({
            "name": "validate_dax",
            "title": "Validate a DAX expression",
            "description": "Parse a DAX expression and bind it against the open model (syntax and function arity only when no model is open). Returns {valid, checked_against_model, diagnostics}.",
            "inputSchema": schema(json!({
                "expression": string("The DAX expression, e.g. SUM(Sales[Amount])"),
                "table": string("Home table of the expression (for calculated columns and unqualified references)"),
            }), &["expression"]),
            "annotations": {"readOnlyHint": true},
        }),
        json!({
            "name": "validate_model",
            "title": "Validate the model",
            "description": "Validate every measure and calculated column of the open model. Returns {valid, invalid_count, objects_with_diagnostics}.",
            "inputSchema": schema(json!({}), &[]),
            "annotations": {"readOnlyHint": true},
        }),
        json!({
            "name": "model_usage",
            "title": "Where objects are used",
            "description": "Where a measure, column, table or M expression is used: other DAX (direct and transitive, with the chain), calculation items, RLS/OLS, relationships, hierarchies, sort-by, M queries, field parameters, perspectives, translations, and the report (visual roles, visual/page/report filters, drillthrough, tooltip pages, bookmarks, conditional formatting, sort). Without `object`: every object with its direct usages and the dependency graph.",
            "inputSchema": schema(json!({
                "object": string("Table[Column], [Measure], 'Table' or an M expression name (omit for every object)"),
                "kind": one_of(&["table", "column", "measure", "expression"], "Which kind `object` names, when ambiguous; without `object`, filters the list"),
                "reports": {"type": "array", "items": {"type": "string"}, "description": "More reports bound to this model (.pbix/.pbit, .pbip, project or <Name>.Report folders); their usage is added"},
                "graph": boolean("Include the dependency graph {nodes, edges} (default true)"),
            }), &[]),
            "annotations": {"readOnlyHint": true},
        }),
        json!({
            "name": "model_unused",
            "title": "Find unused objects",
            "description": "Measures, columns, calculated tables and shared M expressions used nowhere (model or report, directly or transitively), each with why, plus the objects only a model rule keeps (relationship keys, sort-by targets, RLS/OLS, hierarchy levels, perspectives, translations, Q&A schema, calculation groups, report-level measures). Only the reports read are known: pass every report bound to the model with `reports` (thin reports).",
            "inputSchema": schema(json!({
                "reports": {"type": "array", "items": {"type": "string"}, "description": "More reports bound to this model; their usage is added"},
                "allow": {"type": "array", "items": {"type": "string"}, "description": "Names or * ? patterns never to flag, e.g. [KPI*], Sales[Key]"},
                "kept": boolean("List the objects kept by model rules (default true)"),
                "keep_metadata": boolean("Let perspectives, translations and the Q&A schema keep objects (default true); false lists objects only they name"),
            }), &[]),
            "annotations": {"readOnlyHint": true},
        }),
        json!({
            "name": "remove_unused",
            "title": "Remove unused objects",
            "description": "Remove the unused measures, columns, calculated tables and M expressions model_unused lists (all, or `objects`) in one edit (one edit.undo step). Refused when an object is used, or when removing it would break a DAX expression left behind, unless force. dry_run previews.",
            "inputSchema": schema(json!({
                "objects": {"type": "array", "items": {"type": "string"}, "description": "The objects to remove (default: every unused one)"},
                "reports": {"type": "array", "items": {"type": "string"}, "description": "More reports bound to this model, as for model_unused"},
                "allow": {"type": "array", "items": {"type": "string"}, "description": "As for model_unused"},
                "keep_metadata": boolean("As for model_unused"),
                "dry_run": boolean("Only report what would be removed and what would break"),
                "force": boolean("Remove even when DAX left behind would break"),
            }), &[]),
        }),
        json!({
            "name": "new_model",
            "title": "Start a new model",
            "description": "Start an empty semantic model in memory (replaces the open one). Add tables, columns and measures, then save it with new_project or save_as.",
            "inputSchema": schema(json!({
                "name": string("Model (database) name"),
                "culture": string("Culture, e.g. en-US (the default)"),
            }), &["name"]),
        }),
        json!({
            "name": "add_table",
            "title": "Add a table",
            "description": "Add a table to the open model. Give it a partition (Power Query M, or a DAX calculated table) so Power BI Desktop can load it; a calculated table's DAX is validated first.",
            "inputSchema": schema(json!({
                "name": string("Table name, unique in the model (case-insensitive)"),
                "description": string("Description"),
                "hidden": boolean("Hide the table from report authors"),
                "partition": schema(json!({
                    "source_type": one_of(&["m", "calculated"], "m: a Power Query query; calculated: a DAX table expression"),
                    "expression": string("The M query or DAX table expression"),
                }), &["source_type", "expression"]),
                "force": boolean(FORCE),
            }), &["name"]),
        }),
        json!({
            "name": "add_column",
            "title": "Add a column",
            "description": "Add a data column (mapped to source_column) or, with expression, a calculated column whose DAX is validated against the model first.",
            "inputSchema": schema(json!({
                "table": string("Table to add the column to"),
                "name": string("Column name, unique in the table and different from its measures"),
                "data_type": one_of(DATA_TYPES, "Data type"),
                "source_column": string("Source column of a data column (defaults to name)"),
                "expression": string("DAX expression: makes this a calculated column"),
                "format_string": string("Format string, e.g. 0.00 or yyyy-mm-dd"),
                "display_folder": string("Display folder"),
                "summarize_by": one_of(SUMMARIZE_BY, "Default aggregation"),
                "hidden": boolean("Hide the column"),
                "description": string("Description"),
                "force": boolean(FORCE),
            }), &["table", "name", "data_type"]),
        }),
        json!({
            "name": "add_measure",
            "title": "Add a measure",
            "description": "Add a measure. Its DAX is validated against the model first; an invalid expression is rejected with its diagnostics unless force is true.",
            "inputSchema": schema(json!({
                "table": string("Home table of the measure"),
                "name": string("Measure name, unique in the model and different from the home table's columns"),
                "expression": string("DAX expression, e.g. SUM(Sales[Amount])"),
                "format_string": string("Format string, e.g. #,0.00"),
                "display_folder": string("Display folder"),
                "description": string("Description"),
                "hidden": boolean("Hide the measure"),
                "force": boolean(FORCE),
            }), &["table", "name", "expression"]),
        }),
        json!({
            "name": "set_measure",
            "title": "Change a measure",
            "description": "Change a measure's expression, formatting or visibility, or rename it. A rename rewrites every DAX expression that references the measure, like Power BI Desktop (`dax_rewritten`; update_dax: false opts out), returns the expressions that still fail in `broken`, and the report bindings it affects in `report` (rewritten with update_report: true).",
            "inputSchema": schema(json!({
                "name": string("Current measure name"),
                "table": string("Expected home table (checked when given)"),
                "new_name": string("New name"),
                "expression": string("New DAX expression (validated first)"),
                "format_string": clearable("Format string"),
                "display_folder": clearable("Display folder"),
                "description": clearable("Description"),
                "hidden": boolean("Hide the measure"),
                "force": boolean(FORCE),
                "update_report": boolean(UPDATE_REPORT),
                "update_dax": boolean(UPDATE_DAX),
            }), &["name"]),
        }),
        json!({
            "name": "add_relationship",
            "title": "Add a relationship",
            "description": "Relate two columns, from the many side to the one side. Returns the generated relationship name.",
            "inputSchema": schema(json!({
                "from": string("Many-side column, e.g. Sales[ProductKey] or 'Order Lines'[Product]"),
                "to": string("One-side column, e.g. Product[ProductKey]"),
                "active": boolean("Active relationship (default true; one active relationship per pair of tables)"),
                "cross_filter": one_of(&["single", "both"], "Cross-filter direction (default single)"),
                "from_cardinality": one_of(&["one", "many"], "Cardinality of the from side (default many)"),
                "to_cardinality": one_of(&["one", "many"], "Cardinality of the to side (default one)"),
            }), &["from", "to"]),
        }),
        json!({
            "name": "save",
            "title": "Save",
            "description": "Save the model, and the report if it was edited, back to the open PBIP project (reports are written as PBIR) or the model to a model.bim. PBIX/PBIT cannot be written yet: use save_as with a .pbip path.",
            "inputSchema": schema(json!({
                "data_root": string("Optional: the only folder Power Query sources (File.Contents) may read when a PBIX save runs a table's query; kept for the session"),
            }), &[]),
        }),
        json!({
            "name": "save_as",
            "title": "Save as (or convert)",
            "description": "Save the open model and report to another file, which becomes the open file: a new PBIP project (`out/Name.pbip`, TMDL by default), a .pbit template, a .pbix (only from an open .pbix or a PBIP with .pbi/cache.abf, which holds the data), or the model alone to a model.bim. A legacy report stays legacy unless report_format is pbir. Dropped parts and data notes come back in `warnings`.",
            "inputSchema": schema(json!({
                "path": string("Target: a .pbip path (the project name is the file stem), a .pbix/.pbit file or a .bim file"),
                "format": one_of(&["pbix", "pbit", "pbip-tmdl", "pbip-tmsl", "bim"], "Output format (default: from the extension)"),
                "report_format": one_of(&["keep", "pbir"], "keep (default): the report keeps its form; pbir: convert a legacy report to PBIR"),
                "cache": boolean("PBIP targets: write .pbi/cache.abf from the source's data so Desktop opens the project with data (default true when the source has data)"),
                "overwrite": boolean("Replace an existing .pbix/.pbit/model.bim (projects are never replaced)"),
                "strict": boolean("Refuse (write nothing) when a project path would reach Power BI Desktop's 260-character limit"),
                "data_root": string("Optional: as for save"),
            }), &["path"]),
        }),
        json!({
            "name": "convert",
            "title": "Convert a file",
            "description": "Convert a Power BI file without opening it: PBIX -> PBIP/PBIT, PBIT <-> PBIP, PBIP TMSL <-> TMDL, PBIP (with .pbi/cache.abf) -> PBIX, legacy report -> PBIR on request. The open file is not touched. Same options and result as save_as.",
            "inputSchema": schema(json!({
                "from": string("Source: a .pbix/.pbit file, a .pbip file or project folder, or a model.bim, or an Excel .xlsx/.xlsm workbook with a Power Pivot data model"),
                "to": string("Target path (.pbip, .pbix, .pbit or .bim)"),
                "format": one_of(&["pbix", "pbit", "pbip-tmdl", "pbip-tmsl", "bim"], "Output format (default: from the extension)"),
                "report_format": one_of(&["keep", "pbir"], "keep (default): the report keeps its form; pbir: convert a legacy report to PBIR"),
                "cache": boolean("PBIP targets: write .pbi/cache.abf from the source's data (default true when the source has data)"),
                "overwrite": boolean("Replace an existing .pbix/.pbit/model.bim (projects are never replaced)"),
                "strict": boolean("Refuse (write nothing) when a project path would reach Power BI Desktop's 260-character limit"),
            }), &["from", "to"]),
            "annotations": {"readOnlyHint": false},
        }),
        json!({
            "name": "diff_files",
            "title": "Compare two files",
            "description": "Semantic diff of two Power BI files of any format (PBIX, PBIT, PBIP project, model.bim), compared through the model: tables, columns, measures (DAX compared after normalisation: whitespace/comment-only changes are marked cosmetic), relationships, roles, calculation groups, perspectives, cultures, M queries, parameters; report pages, visuals (type, position, fields per role, filters, formatting), bookmarks, theme. Renames (lineage tag, same expression) and moves are recognised. Reads metadata only; the open file is not touched. Returns {identical, summary, changes[], notes?}, plus a text or markdown rendering on request.",
            "inputSchema": schema(json!({
                "a": string("The file before: a .pbix/.pbit file, a .pbip file or project folder, or a model.bim (null: nothing)"),
                "b": string("The file after (null: nothing)"),
                "scope": one_of(&["all", "model", "report"], "What to compare (default all)"),
                "ignore_formatting": boolean("Leave out visual/page formatting and theme changes"),
                "ignore_positions": boolean("Leave out visual positions, sizes, z-order, tab order and page sizes"),
                "ignore_whitespace": boolean("Leave out DAX/M changes that only touch whitespace, comments or case"),
                "format": one_of(&["json", "text", "markdown"], "json (default); text or markdown add a rendering (markdown suits a pull-request comment)"),
            }), &["a", "b"]),
            "annotations": {"readOnlyHint": true},
        }),
        json!({
            "name": "import_objects",
            "title": "Import objects from another file",
            "description": "Copy measures, tables (calculated tables, calculation groups), calculated columns, user-defined functions, shared M expressions and parameters, roles, report pages, single visuals, bookmarks or the custom theme from another Power BI file (PBIX, PBIT, PBIP, model.bim) into the open one, in one undoable edit. The DAX/M objects the selection uses and the target lacks come along (`dependencies`, each with `required_by`), with the relationships of imported tables. Conflicts: fail (default), skip, replace, or rename (the imported set's DAX and visual bindings follow the new names). Refused, with nothing changed, when imported DAX or visual fields would not resolve in the target, unless allow_broken. dry_run returns the plan only. Applying a template report: pages + theme.",
            "inputSchema": schema(json!({
                "source": string("The file to copy from: a .pbix/.pbit file, a .pbip file or project folder, or a model.bim (opened read-only)"),
                "measures": {"type": "array", "items": {"type": "string"}, "description": "Measures: [Name], Table[Name] or * ? patterns (Sales[*])"},
                "tables": {"type": "array", "items": {"type": "string"}, "description": "Tables with their columns, measures, hierarchies and partitions (names or patterns)"},
                "columns": {"type": "array", "items": {"type": "string"}, "description": "Calculated columns Table[Column], added to the target's table of that name"},
                "calculation_groups": {"type": "array", "items": {"type": "string"}, "description": "Calculation groups (names or patterns)"},
                "functions": {"type": "array", "items": {"type": "string"}, "description": "User-defined DAX functions"},
                "expressions": {"type": "array", "items": {"type": "string"}, "description": "Shared M expressions and parameters"},
                "roles": {"type": "array", "items": {"type": "string"}, "description": "Security roles"},
                "relationships": boolean("Import the source's relationships of imported tables (default true)"),
                "pages": {"type": "array", "items": {"type": "string"}, "description": "Report pages with their visuals (name or display name)"},
                "visuals": {"type": "array", "items": {"type": "string"}, "description": "Single visuals as \"Page/visual name\", added to target_page"},
                "target_page": string("The target page the visuals are added to"),
                "bookmarks": {"type": "array", "items": {"type": "string"}, "description": "Bookmarks (name or display name)"},
                "theme": boolean("Import the source's custom theme"),
                "include_dependencies": boolean("Also import what the selection uses and the target lacks (default true)"),
                "on_conflict": one_of(&["fail", "skip", "replace", "rename"], "When the target has an object of the same name (default fail)"),
                "rename_suffix": string("Suffix of renamed objects (default \" (imported)\")"),
                "into_table": string("Target table of the imported measures"),
                "allow_broken": boolean("Import even when DAX or visual fields do not resolve in the target (listed in broken / broken_fields)"),
                "dry_run": boolean("Only return the plan"),
            }), &["source"]),
            "annotations": {"readOnlyHint": false},
        }),
        json!({
            "name": "new_project",
            "title": "Create a PBIP project",
            "description": "Create a Power BI project <dir>/<name>.pbip holding the open model (or an empty one) and the open report (or a blank one-page report), then open it.",
            "inputSchema": schema(json!({
                "dir": string("Folder to create the project in"),
                "name": string("Project name (it becomes file and folder names)"),
                "format": one_of(&["tmdl", "tmsl"], "Semantic model format (default tmdl)"),
                "strict": boolean("Refuse (write nothing) when a project path would reach Power BI Desktop's 260-character limit"),
            }), &["dir", "name"]),
        }),
        json!({
            "name": "report_summary",
            "title": "Summarise the report",
            "description": "List the report's pages (name, display name, size, active) and their visuals (name, type, position, bound fields), with counts of visual types and the distinct fields used.",
            "inputSchema": schema(json!({
                "page": string("Only this page (name or display name)"),
                "visuals": boolean("List each page's visuals (default true)"),
            }), &[]),
            "annotations": {"readOnlyHint": true},
        }),
        json!({
            "name": "add_page",
            "title": "Add a page",
            "description": "Add a report page; returns its generated name (20 hex digits). Display names must be unique.",
            "inputSchema": schema(json!({
                "display_name": string("Name shown on the page tab"),
                "width": number("Width in pixels (default 1280)"),
                "height": number("Height in pixels (default 720)"),
                "display_option": one_of(&["FitToPage", "FitToWidth", "ActualSize"], "How the page scales (default FitToPage)"),
                "index": integer("0-based position (default: last)"),
            }), &["display_name"]),
        }),
        json!({
            "name": "add_visual",
            "title": "Add a visual",
            "description": "Add a visual to a page at a position, optionally binding fields. Fields are resolved against the model: an unknown table, column or measure is rejected unless force is true. Returns the generated visual name.",
            "inputSchema": schema(json!({
                "page": string(PAGE),
                "visual_type": string("Visual type, e.g. card, clusteredColumnChart, clusteredBarChart, lineChart, pieChart, tableEx, pivotTable, slicer, textbox"),
                "x": number("Left edge in pixels"),
                "y": number("Top edge in pixels"),
                "width": number("Width in pixels"),
                "height": number("Height in pixels"),
                "z": number("Stacking order (default: above every visual of the page)"),
                "fields": {
                    "type": "array",
                    "description": "Fields to bind, in order",
                    "items": schema(json!({"role": string(ROLE), "field": string(FIELD)}), &["role", "field"]),
                },
                "force": boolean("Bind fields the model does not know"),
            }), &["page", "visual_type", "x", "y", "width", "height"]),
        }),
        json!({
            "name": "bind_field",
            "title": "Bind a field to a visual",
            "description": "Bind a column, measure or aggregated column to a data role of a visual (visual.unbind removes it).",
            "inputSchema": schema(json!({
                "page": string(PAGE),
                "visual": string("Visual name (report_summary lists them)"),
                "role": string(ROLE),
                "field": string(FIELD),
                "index": integer("Position within the role (default: last)"),
                "force": boolean("Bind a field the model does not know"),
            }), &["page", "visual", "role", "field"]),
        }),
        json!({
            "name": "validate_report",
            "title": "Validate the report",
            "description": "Check the report: PBIR schemas, structure (pages, names, active page), and every field used by visuals and filters against the model. Returns {valid, error_count, warning_count, checked_against_model, schema_issues, issues}.",
            "inputSchema": schema(json!({}), &[]),
            "annotations": {"readOnlyHint": true},
        }),
        json!({
            "name": "validate_dax_query",
            "title": "Validate a DAX query",
            "description": "Parse a DAX query (DEFINE MEASURE/VAR/TABLE/COLUMN/FUNCTION, one or more EVALUATE with ORDER BY and START AT) and bind it against the open model, as DAX Studio or DAX query view would run it. Nothing is changed. Returns {valid, checked_against_model, diagnostics, definitions, evaluate_count}.",
            "inputSchema": schema(json!({
                "query": string("The DAX query, e.g. EVALUATE SUMMARIZECOLUMNS('Date'[Year], \"Sales\", [Total Sales]) ORDER BY 'Date'[Year]"),
            }), &["query"]),
            "annotations": {"readOnlyHint": true},
        }),
        json!({
            "name": "evaluate_dax_query",
            "title": "Run a DAX query",
            "description": "Run a DAX query (DEFINE MEASURE/VAR/TABLE/COLUMN, one or more EVALUATE with ORDER BY and START AT) over the open file's stored data (a PBIX's DataModel; a PBIP's bicraft data snapshot written by refresh_model while it is newer than .pbi/cache.abf, else its .pbi/cache.abf; else the tables' partitions), like DAX Studio. The query is validated first; a function bicraft's evaluator does not support is an error naming it, never an approximate number. Nothing is changed. Returns {results: [{columns: [{name, type}], rows: [{column: value}], row_count, truncated}], data_source, elapsed_ms}.",
            "inputSchema": schema(json!({
                "query": string("The DAX query, e.g. EVALUATE SUMMARIZECOLUMNS('Date'[Year], \"Sales\", [Total Sales]) ORDER BY 'Date'[Year]"),
                "max_rows": {"type": "integer", "description": "Rows returned per EVALUATE (default 10000, at most 1000000); row_count gives the full count"},
                "timeout_ms": {"type": "integer", "description": "Time budget (default 30000, at most 600000)"},
                "allow_cache_abf": {"type": "boolean", "description": "Read a PBIP's .pbi/cache.abf (default true); false uses only the tables' partitions"},
                "cache_abf": string("Optional, PBIP only: read this cache.abf file instead of the project's own, for this query (read-only)"),
            }), &["query"]),
            "annotations": {"readOnlyHint": true},
        }),
        json!({
            "name": "refresh_model",
            "title": "Refresh a PBIP project's data",
            "description": "Refresh the open PBIP project's import tables with bicraft itself, into its own data snapshot <Name>.SemanticModel/.pbi/bicraft-data.bin (never .pbi/cache.abf): M queries in bicraft's Power Query evaluator, external connectors only when allowed (Databricks through the SQL Statement Execution API of the warehouse in the query's HTTP path; column selections, simple filters and row limits fold into the SQL), then calculated tables and columns in its DAX evaluator. The Databricks token is read by the server from its environment variable BICRAFT_DATABRICKS_TOKEN, set by the user; never pass a token in a parameter. Nothing is written unless every import table loads. evaluate_dax_query and render_page then read the refreshed data. Returns {path, refreshed_at, tables: [{table, origin, rows, statements: [{sql, rows, ms}]}], kept, skipped, not_computed, warnings}.",
            "inputSchema": schema(json!({
                "tables": {"type": "array", "items": {"type": "string"}, "description": "Tables to refresh (default every table); the other tables of the previous snapshot are kept"},
                "allow_connectors": {"type": "array", "items": {"type": "string", "enum": ["databricks", "sqlserver", "web", "odata"]}, "description": "External connectors bicraft may run (default none: a query that reads one fails)"},
                "external_links": boolean("Databricks: fetch large results through presigned cloud-storage links (default false: inline results, network only to the workspace host)"),
                "data_root": string("The only folder File.Contents / Folder.* may read"),
            }), &[]),
        }),
        json!({
            "name": "render_page",
            "title": "Render a report page to an image",
            "description": "Draw a report page as Power BI Desktop would (theme, evaluated data, slicer selections, images, formatting) in pure Rust and return it as a PNG image you can look at; with `out`, write PNG files instead. Returns per-visual status (what was drawn: drawn, preview = made-up values, placeholder = a visual type that is not drawn, blank = no field bound, skipped) and data (what the evaluated dataset holds, language-independent: ok, no_rows, blank_values, partial_blank, not_bound, not_evaluated, with rows, value_cells, blank_cells), plus pages[].visuals_without_data.",
            "inputSchema": schema(json!({
                "page": string("Page name, display name or 0-based index (default: every page)"),
                "out": string("Write PNG files instead of returning images: a .png file for one page, else a folder"),
                "scale": number("Device pixels per report pixel, 0.1 to 8 (default 1)"),
                "system_fonts": boolean("Use the system's fonts such as Segoe UI (default true); false uses only the bundled Selawik"),
                "fail_on_empty": boolean("Return an error naming the visuals whose data is no_rows, blank_values or not_evaluated (default false)"),
                "cache_abf": string("Optional, PBIP only: read this cache.abf file instead of the project's own (read-only)"),
            }), &[]),
            "annotations": {"readOnlyHint": true},
        }),
        json!({
            "name": "run_tests",
            "title": "Run a project's CI quality gates",
            "description": "Run the declarative tests of a Power BI project (bicraft.tests.yaml / .yml / .json next to it, or `tests`): DAX assertions evaluated over its data by bicraft's own engine (measure, expression with filters, or query, compared with an expected value or table, with tolerance and type rules; skipped when the data is not loaded unless require_data), model rules (model.validate, BPA rules with include/exclude and a severity floor, naming patterns, required descriptions or format strings, forbidden DAX functions, expression length), report rules (report.validate findings by category, visuals per page, alt text, page size, theme, navigation to hidden pages). Read-only: with `path` the project is opened in a private session and the open file is left alone. Returns {passed, exit_code, summary, suites: [{name, tests: [{id, status, severity, message, findings: [{level, message, location}]}]}]}; format junit or sarif adds `output` (JUnit XML, SARIF 2.1.0).",
            "inputSchema": schema(json!({
                "path": string("Project to test: a .pbip, project folder, .pbix, .pbit or model.bim (default: the open file)"),
                "tests": {"description": "A test file path, or the tests inline (the same object as the file: {version: 1, dax, model, report, usage})", "type": ["string", "object"]},
                "format": one_of(&["json", "text", "junit", "sarif"], "Also render the results as text, JUnit XML or SARIF 2.1.0 in `output` (default json only)"),
                "fail_on": one_of(&["error", "warning"], "Lowest severity of a failed test that fails the run (default error)"),
                "base_dir": string("Folder that result and SARIF paths are relative to (default: the project folder)"),
            }), &[]),
            "annotations": {"readOnlyHint": true},
        }),
        json!({
            "name": "list_sources",
            "title": "List the model's data sources",
            "description": "Every data source of the model's Power Query (M), found without running it: File.Contents, Folder.Files, Excel.Workbook, Csv.Document, Sql.Database, Databricks.Catalogs, Web.Contents, SharePoint, OData, … with each argument's role (path, folder, server, database, host, http_path, url, …), whether it is a literal, a parameter, a step or computed, its value and its byte span in the M text; plus the parameters with their type, value and how the sources use them.",
            "inputSchema": schema(json!({}), &[]),
            "annotations": {"readOnlyHint": true},
        }),
        json!({
            "name": "rebase_sources",
            "title": "Rewrite source paths, servers and hosts",
            "description": "Move the model's file and folder sources from one root to another (Windows drive, UNC and POSIX paths; whole folder names, case-insensitive on Windows), optionally relative to a text parameter (`param`: paths become `Param & \"rest\"`, the parameter is set or added), and map servers, databases, Databricks hosts and HTTP paths, URL prefixes. Only literals are rewritten; computed arguments are reported in `unchanged`. One edit (one undo step); dry_run previews the exact before/after per source.",
            "inputSchema": schema(json!({
                "from": string("The root the paths start with, e.g. C:\\Data\\Finance"),
                "to": string("The new root (default with `param`: `from`)"),
                "param": string("Make the paths relative to this text parameter (added when missing)"),
                "servers": {"type": "object", "description": "{old server: new server}", "additionalProperties": {"type": "string"}},
                "databases": {"type": "object", "description": "{old database: new database}", "additionalProperties": {"type": "string"}},
                "hosts": {"type": "object", "description": "{old Databricks host: new host}", "additionalProperties": {"type": "string"}},
                "http_paths": {"type": "object", "description": "{old HTTP path: new HTTP path}", "additionalProperties": {"type": "string"}},
                "urls": {"type": "object", "description": "{old URL prefix: new prefix}", "additionalProperties": {"type": "string"}},
                "dry_run": boolean("Only return what would change"),
            }), &[]),
            "annotations": {"readOnlyHint": false},
        }),
        json!({
            "name": "apply_env",
            "title": "Apply an environment profile",
            "description": "Apply a profile of bicraft.env.yaml (next to the project, or `env_file`): parameter values typed against each parameter's type and allowed values, path roots, server/database/host/HTTP path/URL mappings, in one edit. An unknown profile, unknown parameter or wrong type refuses the whole profile. dry_run previews.",
            "inputSchema": schema(json!({
                "profile": string("Profile name, e.g. prod"),
                "env_file": {"description": "A profile file (.yaml, .yml, .json) or the profiles inline ({version: 1, profiles: {...}})", "type": ["string", "object"]},
                "dry_run": boolean("Only return what would change"),
            }), &["profile"]),
            "annotations": {"readOnlyHint": false},
        }),
        json!({
            "name": "check_env",
            "title": "Check an environment profile",
            "description": "Compare a profile of bicraft.env.yaml with the model: which sources and parameters it does not cover, which values would change, which arguments are computed; `ok` is true when there is no error and every literal source and every parameter the sources use is covered. Read-only.",
            "inputSchema": schema(json!({
                "profile": string("Profile name"),
                "env_file": {"description": "A profile file or the profiles inline", "type": ["string", "object"]},
            }), &["profile"]),
            "annotations": {"readOnlyHint": true},
        }),
    ]
}

/// [`tool_definitions`] as a server-mode session sees them (`bicraft serve`): the convenience
/// tools whose command is denied in server mode are left out.
pub fn server_tool_definitions() -> Vec<Value> {
    tool_definitions()
        .into_iter()
        .filter(|t| {
            t.get("name")
                .and_then(Value::as_str)
                .is_none_or(|name| !denied_in_server_mode(name))
        })
        .collect()
}

/// Whether tool `name` maps 1:1 to a command that server mode refuses.
fn denied_in_server_mode(tool: &str) -> bool {
    CONVENIENCE
        .iter()
        .find(|(t, _)| *t == tool)
        .and_then(|(_, command)| bicraft_engine::server_policy(command))
        .is_some_and(|p| matches!(p.access, bicraft_engine::Access::Denied(_)))
}

/// What a caller of a server-mode session may do (`docs/auth.md`, "Roles"): the access level of
/// its ACL grant. A server without authentication serves every caller as [`Role::Editor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Read-only commands only ([`bicraft_engine::Effect`]): no edit, save, write, refresh or
    /// network; `file.open` only while the session holds no unsaved changes.
    Reader,
    /// Every command the server allows.
    Editor,
}

impl From<bicraft_auth::Access> for Role {
    fn from(a: bicraft_auth::Access) -> Self {
        match a {
            bicraft_auth::Access::Reader => Role::Reader,
            bicraft_auth::Access::Editor => Role::Editor,
        }
    }
}

/// [`server_tool_definitions`] for a caller with `role`: readers do not see the convenience
/// tools whose command they may never run (the generic tools stay; `list_commands` lists only
/// what they may run).
pub fn server_tool_definitions_for(role: Role) -> Vec<Value> {
    server_tool_definitions()
        .into_iter()
        .filter(|t| role == Role::Editor || t.get("name").and_then(Value::as_str).is_none_or(readable_tool))
        .collect()
}

/// Whether a reader may call tool `name`: a generic tool, or a convenience tool whose command
/// is readable.
fn readable_tool(name: &str) -> bool {
    match CONVENIENCE.iter().find(|(t, _)| *t == name) {
        Some((_, command)) => bicraft_engine::server_policy(command).is_some_and(|p| p.effect.readable()),
        None => true,
    }
}

/// [`call_tool`] for a caller with `role`. A reader's call is checked before anything runs
/// ([`Session::reader_check`]); a refusal is a tool error like a server-denied command's, and a
/// `batch` with one refused step runs no step at all.
pub fn call_tool_as(session: &mut Session, name: &str, args: &Value, role: Role) -> ToolResult {
    call_tool_role(session, name, args, role).0
}

/// [`call_tool_as`], also saying whether the role refused the call (nothing ran): metrics and
/// request logs count a reader's refusal apart from a command that failed.
pub(crate) fn call_tool_role(
    session: &mut Session,
    name: &str,
    args: &Value,
    role: Role,
) -> (ToolResult, bool) {
    if role == Role::Editor {
        return (call_tool(session, name, args), false);
    }
    if let Some(r) = reader_precondition(args) {
        return (r, true);
    }
    let empty = Map::new();
    let map = args.as_object().unwrap_or(&empty);
    let refusal = match name {
        "list_commands" => {
            let filter = match map.get("filter") {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) => Some(s.as_str()),
                Some(_) => return (ToolResult::error("`filter` must be a string"), false),
            };
            return (
                match serde_json::to_value(session.reader_commands(filter)) {
                    Ok(list) => ToolResult::ok(json!({"commands": list})),
                    Err(e) => ToolResult::error(format!("cannot list commands: {e}")),
                },
                false,
            );
        }
        // a malformed step is reported by call_tool, before anything runs
        "run_command" => step(map)
            .ok()
            .and_then(|(id, params)| session.reader_check(&id, &params).err())
            .map(ToolResult::error),
        "batch" => batch_refusal(session, map),
        _ => CONVENIENCE
            .iter()
            .find(|(t, _)| *t == name)
            .and_then(|(_, command)| session.reader_check(command, args).err())
            .map(ToolResult::error),
    };
    match refusal {
        Some(r) => (r, true),
        None => (call_tool(session, name, args), false),
    }
}

/// The refusal of a reader's batch: the first step a reader may not run, in the shape of a
/// failed batch (nothing ran, nothing to roll back). `None` when every step is allowed or the
/// batch is malformed (then [`batch`] reports it without running anything).
fn batch_refusal(session: &Session, args: &Map<String, Value>) -> Option<ToolResult> {
    let commands = args.get("commands").and_then(Value::as_array)?;
    for (i, c) in commands.iter().enumerate() {
        let (id, params) = step(c.as_object()?).ok()?;
        if let Err(e) = session.reader_check(&id, &params) {
            return Some(ToolResult::error_with(json!({
                "error": format!("commands[{i}] (`{id}`) refused: {e}; no command ran"),
                "failed_index": i,
                "failed_command": id,
                "rolled_back": false,
                "results": [],
            })));
        }
    }
    None
}

/// Runs tool `name` with `args` on `session`. Unknown tools and bad arguments are tool errors.
pub fn call_tool(session: &mut Session, name: &str, args: &Value) -> ToolResult {
    if let Err(r) = precondition(session.revision, name, args) {
        return r;
    }
    let empty = Map::new();
    let args = args.as_object().unwrap_or(&empty);
    match name {
        "list_commands" => {
            let filter = match args.get("filter") {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) => Some(s.as_str()),
                Some(_) => return ToolResult::error("`filter` must be a string"),
            };
            match serde_json::to_value(session.available_commands(filter)) {
                Ok(list) => ToolResult::ok(json!({"commands": list})),
                Err(e) => ToolResult::error(format!("cannot list commands: {e}")),
            }
        }
        "run_command" => match step(args) {
            Ok((id, params)) => run(session, &id, &params),
            Err(e) => ToolResult::error(e),
        },
        "batch" => batch(session, args),
        _ => match CONVENIENCE.iter().find(|(t, _)| *t == name) {
            Some((_, command)) => run(session, command, &Value::Object(args.clone())),
            None => ToolResult::error(format!("unknown tool `{name}` (see tools/list)")),
        },
    }
}

fn run(session: &mut Session, id: &str, params: &Value) -> ToolResult {
    match session.execute(id, params) {
        Ok(v) => ToolResult::ok(v),
        Err(e) => ToolResult::error(e),
    }
}

/// `{command, params?}` → `(id, params)`; missing params become `{}`.
fn step(o: &Map<String, Value>) -> Result<(String, Value), String> {
    let id = o
        .get("command")
        .and_then(Value::as_str)
        .ok_or("missing string `command`")?;
    let params = match o.get("params") {
        None | Some(Value::Null) => json!({}),
        Some(p @ Value::Object(_)) => p.clone(),
        Some(_) => return Err(format!("`params` of `{id}` must be an object")),
    };
    Ok((id.to_string(), params))
}

fn batch(session: &mut Session, args: &Map<String, Value>) -> ToolResult {
    let Some(commands) = args.get("commands").and_then(Value::as_array) else {
        return ToolResult::error("missing array `commands`");
    };
    let atomic = match args.get("atomic") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => return ToolResult::error("`atomic` must be true or false"),
    };
    let mut steps = Vec::with_capacity(commands.len());
    for (i, c) in commands.iter().enumerate() {
        let parsed = c
            .as_object()
            .ok_or_else(|| "each entry must be an object {command, params?}".to_string())
            .and_then(step);
        match parsed {
            Ok(s) => steps.push(s),
            Err(e) => {
                return ToolResult::error_with(json!({
                    "error": format!("commands[{i}]: {e}"),
                    "failed_index": i,
                    "results": [],
                }));
            }
        }
    }
    let outcome = if atomic {
        session.batch_atomic(&steps)
    } else {
        session.batch(&steps)
    };
    match outcome {
        // moved in: `json!` would copy the results
        Ok(results) => ToolResult::ok(Value::Object(Map::from_iter([(
            "results".to_string(),
            Value::Array(results),
        )]))),
        Err((i, e, results)) => {
            let command = steps.get(i).map_or("?", |s| s.0.as_str());
            let undone = if atomic { "; the batch was rolled back" } else { "" };
            ToolResult::error_with(json!({
                "error": format!("commands[{i}] (`{command}`) failed: {e}{undone}"),
                "failed_index": i,
                "failed_command": command,
                "rolled_back": atomic,
                "results": results,
            }))
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Optimistic concurrency and shared runs (`docs/web-deployment.md`, "Concurrency")

/// The stable code of a failed `expected_revision` precondition.
pub const REVISION_CONFLICT: &str = "revision_conflict";

/// The argument of `run_command` and `batch` naming the revision the call expects.
pub(crate) const EXPECTED_REVISION: &str = "expected_revision";

/// Checks the `expected_revision` precondition of a `run_command` or `batch` call against the
/// session's revision: `Err` (a tool error, nothing runs) when it is malformed or differs.
pub(crate) fn precondition(revision: u64, name: &str, args: &Value) -> Result<(), ToolResult> {
    let expected = match args.get(EXPECTED_REVISION) {
        None | Some(Value::Null) => return Ok(()),
        Some(v) => v,
    };
    if name != "run_command" && name != "batch" {
        return Err(ToolResult::error(format!(
            "`{EXPECTED_REVISION}` is an argument of run_command and batch only"
        )));
    }
    let Some(expected) = expected.as_u64() else {
        return Err(ToolResult::error(format!(
            "`{EXPECTED_REVISION}` must be a non-negative integer (a revision from session.revision \
             or a result's _meta)"
        )));
    };
    if expected == revision {
        return Ok(());
    }
    Err(ToolResult::error_with(json!({
        "error": format!(
            "revision conflict: the session is at revision {revision}, not {expected}; nothing ran \
             (read the current state, then retry with {EXPECTED_REVISION} {revision})"
        ),
        "code": REVISION_CONFLICT,
        "expected_revision": expected,
        "current_revision": revision,
    })))
}

/// The engine commands a tool call names: its one command, or a batch's steps.
fn named_commands(name: &str, args: &Value) -> Vec<String> {
    if name == "batch" {
        let steps = args.get("commands").and_then(Value::as_array);
        return steps
            .into_iter()
            .flatten()
            .filter_map(|c| c.get("command").and_then(Value::as_str).map(str::to_string))
            .collect();
    }
    single_command(name, args).map(|(id, _)| id).into_iter().collect()
}

/// Whether the call writes the session's open file over itself (`file.save`, alone or in a
/// batch): the per-file revision check applies (docs/mcp.md, "Document views").
pub(crate) fn saves_in_place(name: &str, args: &Value) -> bool {
    named_commands(name, args).iter().any(|c| c == "file.save")
}

/// Whether the call refreshes the open PBIX (`model.refresh` on a package, alone or in a batch):
/// the new data is written into the file in place, so the per-file revision check applies as for
/// [`saves_in_place`], and a successful one is a new revision of the file.
pub(crate) fn refreshes_in_place(name: &str, args: &Value, session: &Session) -> bool {
    matches!(session.source, Some(bicraft_engine::Source::Package { .. }))
        && named_commands(name, args).iter().any(|c| c == "model.refresh")
}

/// Whether the call writes the session's file (`file.save` or `file.save_as`, alone or in a
/// batch): a successful one is a new revision of that file.
pub(crate) fn saves(name: &str, args: &Value) -> bool {
    named_commands(name, args)
        .iter()
        .any(|c| c == "file.save" || c == "file.save_as")
}

/// The stable code of a refused save over a file another session saved since ([`saves_in_place`]).
pub const FILE_CONFLICT: &str = "file_conflict";

/// Whether `result` is a failed `expected_revision` precondition.
pub(crate) fn is_conflict(result: &ToolResult) -> bool {
    result.is_error && result.structured.get("code").and_then(Value::as_str) == Some(REVISION_CONFLICT)
}

/// The one engine command a tool call runs, with its parameters (`None`: `batch`,
/// `list_commands`, an unknown tool or a malformed `run_command`).
fn single_command(name: &str, args: &Value) -> Option<(String, Value)> {
    let empty = Map::new();
    let map = args.as_object().unwrap_or(&empty);
    match name {
        "run_command" => step(map).ok(),
        "batch" | "list_commands" => None,
        _ => CONVENIENCE
            .iter()
            .find(|(t, _)| *t == name)
            .map(|(_, c)| ((*c).to_string(), Value::Object(map.clone()))),
    }
}

/// Whether tool call `name` with `args` may run on a shared session ([`call_tool_shared`]):
/// `list_commands`, or one command that runs shared (`bicraft_engine::runs_shared`). A `batch`
/// always runs exclusively.
pub(crate) fn runs_shared(name: &str, args: &Value) -> bool {
    name == "list_commands"
        || single_command(name, args).is_some_and(|(id, p)| bicraft_engine::runs_shared(&id, &p))
}

/// [`call_tool_role`] on a shared session, for a call [`runs_shared`] accepts (anything else is
/// a tool error and nothing runs). `cancel` is this call's flag.
pub(crate) fn call_tool_shared(
    session: &Session,
    name: &str,
    args: &Value,
    role: Role,
    cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> (ToolResult, bool) {
    if let Err(r) = precondition(session.revision, name, args) {
        return (r, false);
    }
    if name == "list_commands" {
        let filter = match args.get("filter") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.as_str()),
            Some(_) => return (ToolResult::error("`filter` must be a string"), false),
        };
        let list = if role == Role::Editor {
            session.available_commands(filter)
        } else {
            session.reader_commands(filter)
        };
        return (
            match serde_json::to_value(list) {
                Ok(list) => ToolResult::ok(json!({"commands": list})),
                Err(e) => ToolResult::error(format!("cannot list commands: {e}")),
            },
            false,
        );
    }
    let Some((id, params)) = single_command(name, args) else {
        return (
            ToolResult::error(format!("`{name}` cannot run on a shared session")),
            false,
        );
    };
    if role == Role::Reader {
        if let Some(r) = reader_precondition(args) {
            return (r, true);
        }
        if let Err(e) = session.reader_check(&id, &params) {
            return (ToolResult::error(e), true);
        }
    }
    let r = match session.execute_shared(&id, &params, cancel) {
        Ok(v) => ToolResult::ok(v),
        Err(e) => ToolResult::error(e),
    };
    (r, false)
}

/// What the schemas of run_command and batch say about `expected_revision`.
const EXPECTED_REVISION_DOC: &str = "Optional precondition: run only if the session is at this revision \
     (session.revision, or `io.bicraft/revision` in a result's _meta on a server); otherwise the call \
     fails with code `revision_conflict` and nothing runs. Without it, the last writer wins";

/// A reader's call naming `expected_revision`: refused (a reader reads the saved state and
/// changes nothing; the precondition is for editors).
fn reader_precondition(args: &Value) -> Option<ToolResult> {
    args.get(EXPECTED_REVISION).filter(|v| !v.is_null()).map(|_| {
        ToolResult::error(format!(
            "`{EXPECTED_REVISION}` is for editors: a reader reads the saved state and changes nothing"
        ))
    })
}
