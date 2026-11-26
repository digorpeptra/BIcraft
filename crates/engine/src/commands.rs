//! The command table. Add a command by adding a row here plus tests: through `Session::execute`
//! in the engine's integration tests (`crates/engine/tests/`: `commands/tests.rs` for the table,
//! `model/edit_tests.rs` for model edits, `files/save_tests.rs` for saving,
//! `report/report_tests.rs` for report, page and visual commands), inside the crate only when a
//! test needs a private item (docs/ci.md, "Where engine tests go").

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bicraft_dax::deps::{self, ObjectRef};
use bicraft_dax::{Context, ExprKind, Validator};
use bicraft_opc::Package;
use serde_json::{Value, json};

use crate::command_ui::{Kind as K, Menu, Need, Obj, Param as P, Place, Tab};
use crate::params::{allow, str_param};
use crate::sandbox::{PathParam, ServerPolicy};
use crate::{Command, Replay, Session, Source, edit, pbix_struct, report, save, visual_props};

// Value lists shared by several forms (`command_ui.rs`).
const SAVE_FORMATS: &[&str] = &["pbix", "pbit", "pbip-tmdl", "pbip-tmsl", "bim"];
const MODES: &[&str] = &["import", "directQuery", "dual"];
/// The objects `dax.validate {kind}` checks an expression as.
const DAX_KINDS: &[&str] = &["measure", "calculated_column", "calculated_table"];
const DATA_TYPES: &[&str] = &[
    "string", "int64", "double", "decimal", "dateTime", "boolean", "binary", "variant",
];
const SUMMARIZE_BY: &[&str] = &[
    "none",
    "sum",
    "min",
    "max",
    "count",
    "average",
    "distinctCount",
    "default",
];
const CARDINALITY: &[&str] = &["one", "many"];
/// `visual.set_aggregation`'s choices (`none`: the column itself, "Don't summarize").
const AGGREGATION_NAMES: &[&str] = &[
    "none",
    "Sum",
    "Avg",
    "Count",
    "Min",
    "Max",
    "CountNonNull",
    "Median",
    "StandardDeviation",
    "Variance",
];
const FILTER_OPS: &[&str] = &[
    "=",
    "<>",
    ">",
    ">=",
    "<",
    "<=",
    "contains",
    "not_contains",
    "starts_with",
    "not_starts_with",
];
const INTERACT_ACTIONS: &[&str] = &[
    "page",
    "back",
    "slicer_toggle",
    "slicer_select_all",
    "slicer_set",
    "slicer_clear",
    "slicer_range",
    "slicer_text",
    "slicer_relative",
    "select",
    "clear_selection",
    "clear_slicers",
    "apply_slicers",
    "button",
    "bookmark",
    "bookmark_deselect",
    "decomp_path",
    "decomp_expand",
    "decomp_collapse",
    "decomp_scroll",
    "key_influencers",
    "drill_down",
    "drill_up",
    "drill_next",
    "drill_expand",
    "drill_mode",
    "matrix_expand",
    "matrix_collapse",
    "drillthrough",
    "tooltip",
    "focus",
    "show_table",
    "spotlight",
    "sort",
    "tree_toggle",
    "filter",
    "filter_clear",
    "view_as",
    "reset",
];

pub static COMMANDS: &[Command] = &[
    Command {
        id: "file.open",
        label: "Open a PBIX, PBIT, PBIP project or model.bim, or import an Excel Power Pivot workbook",
        params: "{path: string, data_root?: string, cache_abf?: string}: a .pbix/.pbit file, a .pbip file or project folder, a \n                 model.bim, or an Excel workbook (.xlsx/.xlsm) with a Power Pivot data model, imported into a new Power BI \n                 model held in memory (the result's `imported`; save it with file.save_as, the workbook is never written; \n                 docs/formats/powerpivot.md); or (M11.36) a .vpax, .pbids, .pq/.m, .pqt, a Dataflow mashup.pq or \n                 queryMetadata.json, or a CDM model.json, read into a new unsaved model (as model.new: save it \n                 with file.save_as; the result adds `format`, `notes` and, for a .vpax, `statistics`). \n                 `data_root` is the only folder Power Query File.Contents may read when bicraft \n                 runs a table's M query (viewer previews, structural saves); without it (the default, also \
                 after another file.open) no file is read. `cache_abf` (PBIP only): read the project's data \
                 from this file instead of its own .pbi/cache.abf (e.g. a throwaway copy reading the \
                 original's cache); opened read-only, never written or copied; it must exist and be readable",
        run: file_open,
        server: ServerPolicy::paths(&[
            PathParam::one("path"),
            PathParam::clearable("data_root"),
            PathParam::clearable("cache_abf"),
        ])
        .opens(),
        replay: Replay::Base,
        place: Place::ribbon(Tab::File, "File", "Open", "folder"),
        shortcut: Some("Ctrl+O"),
        enabled: &[],
        form: &[
            P::req("path", K::Path),
            P::opt("data_root", K::Path),
            P::opt("cache_abf", K::Path),
        ],
    },
    Command {
        id: "session.revision",
        label: "The session's revision (optimistic concurrency) and unsaved-changes flags",
        params: "{}: returns {revision, dirty, report_dirty}. The revision goes up by one after every \n                 command that may have changed the session (model, report, open file, settings); read-only \n                 commands leave it alone. run_command and batch take `expected_revision` to run only on that \n                 revision (docs/web-deployment.md, \"Concurrency\")",
        run: |s, p| session_revision(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::palette("Session revision", "info"),
        shortcut: None,
        enabled: &[],
        form: &[],
    },
    Command {
        id: "file.inspect",
        label: "Describe the open file: kind, parts, formats, unsaved changes",
        params: "{}",
        run: |s, p| file_inspect(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::File, "Info", "File info", "info"),
        shortcut: None,
        enabled: &[Need::File],
        form: &[],
    },
    Command {
        id: "file.set_cache_abf",
        label: "Read the open PBIP project's data from an explicit cache.abf file (read-only) instead of its own .pbi/cache.abf",
        params: "{cache_abf: string|null (a cache.abf file; null: the project's own .pbi/cache.abf again)}: \
                 returns {cache_abf, project_cache_abf}; the file must exist and be readable (an error, never \
                 a silent fallback); bicraft never writes or copies it, and file.save_as refuses a project \
                 whose cache would land on it. file.open resets it (or sets it with its own cache_abf)",
        run: crate::cache_abf::file_set_cache_abf,
        server: ServerPolicy::paths(&[PathParam::clearable("cache_abf")]),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Data, "Stored data", "Data cache file", "folder"),
        shortcut: None,
        enabled: &[Need::Project],
        form: &[P::req("cache_abf", K::Path)],
    },
    Command {
        id: "file.save",
        label: "Save the model (and the edited report) back to the open file",
        params: "{backup?: bool (default true), data_root?: string}: PBIP projects and model.bim files are \
                 rewritten; PBIX/PBIT files are rewritten in place (atomically, keeping `<name>.bak` unless \
                 backup is false). On a PBIX, structural edits (tables, columns, relationships added, removed \
                 or renamed; table.import/table.set_data data) are written with their VertiPaq data: import \
                 tables run their M query in bicraft's Power Query evaluator (literals, Enter data, common \
                 transforms; Csv.Document/Json.Document/Excel.Workbook files only under `data_root`, which \
                 is kept for the session), calculated tables and columns are evaluated by bicraft's DAX \
                 evaluator; text is stored as Analysis Services stores it; what cannot be written exactly is \
                 refused with the reasons. A PBIP \
                 report with unsaved edits is written as PBIR; a legacy report.json project is \
                 converted to PBIR",
        run: save::file_save,
        server: ServerPolicy::paths(&[PathParam::clearable("data_root")]),
        replay: Replay::Base,
        place: Place::ribbon(Tab::File, "File", "Save", "save"),
        shortcut: Some("Ctrl+S"),
        enabled: &[Need::File],
        form: &[P::flag("backup", true), P::opt("data_root", K::Path)],
    },
    Command {
        id: "file.save_as",
        label: "Save (or convert) the model and report to a PBIX, PBIT, new PBIP project or model.bim; the target becomes the open file",
        params: "{path: string, format?: \"pbix\"|\"pbit\"|\"pbip-tmdl\"|\"pbip-tmsl\"|\"bim\", \
                 report_format?: \"keep\"|\"pbir\", cache?: bool, empty_tables?: bool, overwrite?: bool, \
                 strict?: bool, data_root?: string (as for file.save)}: the \
                 format defaults from the extension (.pbix, .pbit, .bim, otherwise pbip-tmdl); \
                 `out/Sales.pbip` (or `out/Sales`) creates the project Sales in `out` (never over an \
                 existing one); `overwrite` replaces an existing .pbix/.pbit/.bim. Conversions: PBIX → \
                 PBIP/PBIT, PBIT ↔ PBIP, PBIP TMSL ↔ TMDL, PBIP/PBIT/model.bim/new model → PBIX (a \
                 project's .pbi/cache.abf holds the data; without one the DataModel is built from scratch: \
                 M queries run in bicraft's Power Query evaluator, files only under data_root, calculated \
                 tables and columns in its DAX evaluator; a table whose source bicraft cannot run, such as \
                 a database connector, is refused by name unless `empty_tables` writes it without rows for \
                 Desktop to load on refresh). A legacy \
                 report stays legacy (report.json / Report/Layout) unless report_format is \"pbir\". A PBIP \
                 made from a PBIX (or a project with a cache) gets .pbi/cache.abf so Desktop opens it with \
                 data (`cache: false` skips it). The model diagram and the report's StaticResources and \
                 CustomVisuals are carried over; dropped parts are listed in `warnings`. A model.bim holds \
                 no report. Paths Power BI Desktop cannot read (260+ characters) are reported in \
                 `warnings` and `long_paths`; with `strict` they are an error and nothing is written",
        run: save::file_save_as,
        server: ServerPolicy::paths(&[PathParam::one("path"), PathParam::clearable("data_root")]),
        replay: Replay::Base,
        place: Place::ribbon(Tab::File, "File", "Save as", "save-as"),
        shortcut: Some("Ctrl+Shift+S"),
        enabled: &[Need::Model],
        form: &[
            P::req("path", K::Path),
            P::opt("format", K::Enum(SAVE_FORMATS)),
            P::opt("report_format", K::Enum(&["keep", "pbir"])),
            P::opt("cache", K::Bool),
            P::opt("empty_tables", K::Bool),
            P::flag("overwrite", false),
            P::flag("strict", false),
            P::opt("data_root", K::Path),
        ],
    },
    Command {
        id: "file.redact",
        label: "Write a redacted copy of a file (connection strings, credentials, servers, databases, paths, URLs; data on request)",
        params: "{path: string (.pbix/.pbit/.pbip or project folder), out: string (same kind: .pbix/.pbit, or \
                 a new .pbip), data?: bool (default false: also replace the stored text values of the import \
                 tables, PBIX), overwrite?: bool}: opens `path` in a private session, replaces every \
                 connection string, credential, server and database name, file path and URL of the model, the \
                 report and the package's text parts by a stable placeholder (the same value always gets the \
                 same one: server_1.redacted.invalid, database_1, C:\\redacted\\path_1.csv, \
                 https://redacted.invalid/url_1, connection_1, credential_1, value_1), leaves out \
                 SecurityBindings, writes `out` with file.save_as, opens it again and searches all its bytes \
                 (the decompressed DataModel too) for the replaced values: a survivor deletes the copy and \
                 fails. The open file is not touched. Returns {from, out, redacted, kinds, placeholders: \
                 [{kind, placeholder}], data, data_values, removed_parts, verified: {bytes_scanned, \
                 survivors}, notes}; the original values are never returned. Also `bicraft redact IN OUT \
                 [--data]`",
        run: crate::redact::file_redact,
        server: ServerPolicy::paths(&[PathParam::delegated("path"), PathParam::delegated("out")]),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::File, "Export", "Redact a copy", "export"),
        shortcut: None,
        enabled: &[],
        form: &[
            P::req("path", K::Path),
            P::req("out", K::Path),
            P::flag("data", false),
            P::flag("overwrite", false),
        ],
    },
    Command {
        id: "file.convert",
        label: "Convert a file to another format without opening it (PBIX/PBIT/PBIP/model.bim)",
        params: "{from: string, to: string, format?, report_format?, cache?, empty_tables?, overwrite?, \
                 strict?, data_root?}: opens `from` (with `data_root`) in a private session and runs \
                 file.save_as {path: to, ...} there (same rules and result, plus `from`); the open file and \
                 its unsaved edits are not touched",
        run: crate::convert::file_convert,
        server: ServerPolicy::paths(&[
            PathParam::delegated("from"),
            PathParam::delegated("to"),
            PathParam::delegated("data_root"),
        ]),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::File, "Export", "Convert a file", "export"),
        shortcut: None,
        enabled: &[],
        form: &[
            P::req("from", K::Path),
            P::req("to", K::Path),
            P::opt("format", K::Enum(SAVE_FORMATS)),
            P::opt("report_format", K::Enum(&["keep", "pbir"])),
            P::opt("cache", K::Bool),
            P::opt("empty_tables", K::Bool),
            P::flag("overwrite", false),
            P::flag("strict", false),
            P::opt("data_root", K::Path),
        ],
    },
    Command {
        id: "file.diff",
        label: "Compare two files of any format (PBIX, PBIT, PBIP, model.bim) through the model: tables, columns, measures, relationships, roles, pages, visuals, filters, bookmarks",
        params: "{a: string|null, b: string|null (the files before and after; null: nothing, for a file \
                 added or deleted), a_label?, b_label?: string (names shown instead of the paths), scope?: \
                 \"all\"|\"model\"|\"report\", ignore_formatting?, ignore_positions?, ignore_whitespace?: bool, \
                 format?: \"json\" (default)|\"text\"|\"markdown\" (adds `text` or `markdown`)}: reads both \
                 sides metadata only (a PBIX DataModel is never decompressed beyond its metadata; the \
                 data snapshots are compared by size and CRC-32). Objects are matched by name, lineage tag, \
                 case or content, so renames and moves are reported as such; DAX and M changes that only \
                 touch whitespace or comments are marked `cosmetic`. Returns {a, b, identical, summary, \
                 data?, changes: [{area, object, id, change: added|removed|changed|renamed|moved, from?, \
                 how?, label?, properties?, fields?, formatting?, before?, after?}], notes?} \
                 (docs/diff.md). The open file is not touched",
        run: |s, p| crate::diff::file_diff(s, p),
        server: ServerPolicy::paths(&[PathParam::clearable("a"), PathParam::clearable("b")]).read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::File, "Compare", "Compare files", "compare"),
        shortcut: None,
        enabled: &[],
        form: &[
            P::req("a", K::Path),
            P::req("b", K::Path),
            P::opt("a_label", K::Text),
            P::opt("b_label", K::Text),
            P::opt("scope", K::Enum(&["all", "model", "report"])),
            P::flag("ignore_formatting", false),
            P::flag("ignore_positions", false),
            P::flag("ignore_whitespace", false),
            P::opt("format", K::Enum(&["json", "text", "markdown"])),
        ],
    },
    Command {
        id: "file.textconv",
        label: "A canonical text form of a file's model and report (for git textconv and line diffs)",
        params: "{file: string}: a .pbix/.pbit/.pbip/model.bim or project folder, read metadata only; \
                 returns {kind, text}: objects sorted by name, one property per line, expressions as \
                 indented blocks, format noise left out",
        run: |s, p| crate::diff::file_textconv(s, p),
        server: ServerPolicy::paths(&[PathParam::one("file")]).read_only(),
        replay: Replay::Skip,
        place: Place::palette("Text form of a file", "file"),
        shortcut: None,
        enabled: &[],
        form: &[P::req("file", K::Path)],
    },
    Command {
        id: "objects.import",
        label: "Copy measures, tables, columns, calculation groups, functions, M queries, roles, pages, visuals, bookmarks or the theme from another file into the open one",
        params: "{source: string (a .pbix/.pbit/.pbip/model.bim or project folder, opened read-only in a \
                 private session), measures?, tables?, columns?, calculation_groups?, functions?, \
                 expressions?, roles?, pages?, bookmarks?: [string] (names, `Table[Name]`, `*`/`?` \
                 patterns; columns: calculated columns `Table[Column]`), visuals?: [\"Page/visual\" | \
                 {page, visual}] with target_page?: string, theme?: bool (the custom theme), \
                 relationships?: bool (default true: the source's relationships of imported tables), \
                 include_dependencies?: bool (default true: the DAX/M objects the selection uses that \
                 the target lacks, listed in `dependencies` with `required_by`), on_conflict?: \
                 \"fail\" (default)|\"skip\"|\"replace\"|\"rename\", rename_suffix?: string (default \
                 \" (imported)\"), into_table?: string (target table of imported measures), \
                 allow_broken?: bool, dry_run?: bool}: one undoable edit. Renames rewrite the DAX and \
                 report bindings of the imported set; duplicate lineage tags are regenerated. Refused \
                 (nothing changes) when imported DAX or visual fields would not resolve in the target \
                 unless allow_broken. Returns {objects: [{kind, name, table?, action: \
                 added|replaced|renamed|skipped, reason, required_by?, source_name?}], dependencies, \
                 relationships, conflicts, lineage_tags_regenerated, broken, broken_fields, warnings, \
                 applied, would_apply?, errors?}",
        run: crate::import::objects_import,
        server: ServerPolicy::paths(&[PathParam::delegated("source")]),
        replay: Replay::Copy,
        place: Place::ribbon(Tab::File, "Import", "Import from a file", "import"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("source", K::Path),
            P::opt("measures", K::List),
            P::opt("tables", K::List),
            P::opt("columns", K::List),
            P::opt("calculation_groups", K::List),
            P::opt("functions", K::List),
            P::opt("expressions", K::List),
            P::opt("roles", K::List),
            P::flag("relationships", true),
            P::opt("pages", K::List),
            P::opt("visuals", K::List),
            P::opt("target_page", K::Name(Obj::Page)),
            P::opt("bookmarks", K::List),
            P::flag("theme", false),
            P::flag("include_dependencies", true),
            P::opt("on_conflict", K::Enum(&["fail", "skip", "replace", "rename"])),
            P::opt("rename_suffix", K::Text),
            P::opt("into_table", K::Name(Obj::Table)),
            P::flag("allow_broken", false),
            P::flag("dry_run", false),
        ],
    },
    Command {
        id: "file.open_in_desktop",
        label: "Open the session's saved file in Power BI Desktop (the OS default application for .pbix/.pbit/.pbip)",
        params: "{dry_run?: bool (only report what would be opened)}: opens the saved .pbix/.pbit file, or the \
                 project's .pbip file, with the operating system's default application (explorer.exe on \
                 Windows, open on macOS, xdg-open elsewhere; no shell). Never a path from the caller: \
                 only the session's own file. Returns {path, opened, unsaved, warning?}; unsaved edits \
                 are not in the opened file (file.save first). Fails when nothing is saved yet, for a \
                 standalone model.bim, or when no application handles the file. bicraft itself never \
                 needs Power BI Desktop",
        run: crate::desktop_open::file_open_in_desktop,
        server: ServerPolicy::denied(
            "it launches an application on the server (download the saved file instead)",
        ),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Home, "Share", "Open in Desktop", "open-in-desktop"),
        shortcut: None,
        enabled: &[Need::SavedReportFile],
        form: &[P::flag("dry_run", false)],
    },
    Command {
        id: "project.new",
        label: "Create a PBIP project from the open model and report (or empty ones) and open it",
        params: "{dir: string, name: string, format?: \"tmdl\"|\"tmsl\", strict?: bool}: creates \
                 `<dir>/<name>.pbip`, `<name>.SemanticModel/` (TMDL by default) and `<name>.Report/` (the \
                 open report as PBIR, or a blank one-page report). Paths Power BI Desktop cannot read \
                 (260+ characters, including the files Desktop adds) are reported in `warnings` and \
                 `long_paths`; with `strict` they are an error and nothing is written",
        run: save::project_new,
        server: ServerPolicy::paths(&[PathParam::one("dir")]),
        replay: Replay::Base,
        place: Place::ribbon(Tab::File, "File", "New project", "folder"),
        shortcut: None,
        enabled: &[],
        form: &[
            P::req("dir", K::Path),
            P::req("name", K::Text),
            P::opt("format", K::Enum(&["tmdl", "tmsl"])),
            P::flag("strict", false),
        ],
    },
    Command {
        id: "model.new",
        label: "Start a new empty model in memory (save it with project.new or file.save_as)",
        params: "{name: string, culture?: string (default en-US)}",
        run: edit::model_new,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Base,
        place: Place::ribbon(Tab::File, "File", "New model", "plus"),
        shortcut: Some("Ctrl+N"),
        enabled: &[],
        form: &[P::req("name", K::Text), P::opt("culture", K::Text)],
    },
    Command {
        id: "model.set",
        label: "Change the model's culture, implicit-measure policy or source query culture",
        params: "{culture?: string (e.g. en-US), discourage_implicit_measures?: bool, source_query_culture?: \
                 string|null}: a PBIX with data refuses a culture change on save",
        run: crate::model_props::model_set,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Model, "Properties", "Model properties", "settings"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::opt("culture", K::Text),
            P::opt("discourage_implicit_measures", K::Bool),
            P::opt("source_query_culture", K::Text),
        ],
    },
    Command {
        id: "model.set_annotation",
        label: "Set or remove an annotation of the model",
        params: "{name, value: string|null}: null removes it; returns `previous`",
        run: crate::model_props::model_set_annotation,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Model annotation", "edit"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[P::req("name", K::Text), P::req("value", K::Text)],
    },
    Command {
        id: "table.set_annotation",
        label: "Set or remove an annotation of a table",
        params: "{table, name, value: string|null}: null removes it; returns `previous`",
        run: crate::model_props::table_set_annotation,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Table annotation", "edit"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("table", K::Name(Obj::Table)),
            P::req("name", K::Text),
            P::req("value", K::Text),
        ],
    },
    Command {
        id: "column.set_annotation",
        label: "Set or remove an annotation of a column",
        params: "{table, column, name, value: string|null}: null removes it; returns `previous`",
        run: crate::model_props::column_set_annotation,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Column annotation", "edit"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("table", K::Name(Obj::Table)),
            P::req("column", K::Name(Obj::Column)),
            P::req("name", K::Text),
            P::req("value", K::Text),
        ],
    },
    Command {
        id: "measure.set_annotation",
        label: "Set or remove an annotation of a measure",
        params: "{measure, table? (checked when given), name, value: string|null}: null removes it; \
                 returns `previous`",
        run: crate::model_props::measure_set_annotation,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Measure annotation", "edit"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("measure", K::Name(Obj::Measure)),
            P::opt("table", K::Name(Obj::Table)),
            P::req("name", K::Text),
            P::req("value", K::Text),
        ],
    },
    Command {
        id: "column.remove_variation",
        label: "Remove a column's date variations (auto date/time) and the auto date tables left unused",
        params: "{table, name (the column), variation? (default: all of them), force?: bool}: removes the \
                 variations, their relationships and the LocalDateTable_ tables no other variation uses \
                 (`removed_variations`, `removed_relationships`, `removed_tables`); refused while DAX \
                 uses such a table unless force (`used_by`)",
        run: crate::model_props::column_remove_variation,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Table, "Columns", "Remove date variations", "calendar"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("table", K::Name(Obj::Table)),
            P::req("name", K::Name(Obj::Column)),
            P::opt("variation", K::Text),
            P::flag("force", false),
        ],
    },
    Command {
        id: "model.summary",
        label: "List the tables, columns, measures and relationships of the open model",
        params: "{}",
        run: |s, p| model_summary(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::palette("Model summary", "list"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[],
    },
    Command {
        id: "model.get",
        label: "Read the model with every property it holds (tables, columns, measures, hierarchies, partitions, relationships, roles, perspectives, cultures, calculation groups)",
        params: "{detail?: \"summary\" (default) | \"full\", culture?: string (a model culture: each object \
                 gains caption, translation {caption, description, display_folder} and shows its \
                 translated description and display folder; see translation.set)}: summary returns \
                 {name, compatibility_level, \
                 culture, description, default_mode, discourage_implicit_measures, source_query_culture, \
                 counts: {tables, columns, measures, hierarchies, partitions, calculation_groups, \
                 relationships, roles, expressions, perspectives, cultures}, tables: [{name, kind, hidden, \
                 data_category, columns, measures, hierarchies (counts), partitions: [{name, mode, \
                 source_type}]}]}; full returns those model properties plus data_access_options, \
                 default_power_bi_data_source_version, tables: [table.get], relationships: \
                 [relationship.list item], roles: [{name, description, model_permission, members, \
                 table_permissions: [{table, filter_expression, metadata_permission, column_permissions, \
                 annotations, extra}], annotations, extra}], expressions: [{name, kind, expression, \
                 description, query_group, lineage_tag, annotations, extra}], perspectives, cultures, \
                 data_sources, query_groups, functions (TMSL objects as stored), annotations, database, \
                 extra, and, when the open file has stored data (PBIX DataModel or PBIP \
                 .pbi/cache.abf), refresh: {source, last_refresh, last_data_change, last_schema_change, \
                 modified_time, structure_modified_time, partition_count, partitions_not_ready: [{table, \
                 partition, state}]} (times ISO-8601 UTC, null = never) and table.get's refresh blocks. \
                 Every object has `annotations` [{name, value}] and `extra` (unmodelled TMSL \
                 properties by TMSL name); unset properties are null (booleans: their TMSL default)",
        run: |s, p| crate::model_get::model_get(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::palette("Model properties (read)", "model"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::opt("detail", K::Enum(&["summary", "full"])),
            P::opt("culture", K::Name(Obj::Culture)),
        ],
    },
    Command {
        id: "table.get",
        label: "Read one table with every property: columns, measures, hierarchies, partitions, calculation group",
        params: "{name, culture? (as for model.get)}: returns {name, kind: data|calculated|calculation_group, description, hidden, \
                 is_private, data_category, show_as_variations_only, exclude_from_model_refresh, \
                 lineage_tag, source_lineage_tag, columns: [column.get], measures: [measure.get], \
                 hierarchies: [{name, description, hidden, display_folder, lineage_tag, levels: [{name, \
                 column, ordinal, description, lineage_tag, annotations, extra}], annotations, extra}], \
                 partitions: [{name, mode, source_type (m|calculated|query|entity|policyRange|...), \
                 expression (M or DAX text), data_view, query_group, description, data_source, source \
                 (other source properties), annotations, extra}], calculation_group: {precedence, \
                 description, multiple_or_empty_selection_expression, no_selection_expression, items: \
                 [{name, expression, ordinal, format_string_definition, description, annotations, \
                 extra}], annotations, extra} | null, refresh_policy, annotations, extra, and, when the \
                 open file has stored data, refresh: {last_refresh, modified_time, \
                 structure_modified_time} plus partitions[].refresh: {state (Ready|NoData|\
                 CalculationNeeded|SemanticError|EvaluationError|DependencyError|Incomplete|\
                 ForceCalculationNeeded|Unknown), state_code, refreshed_time, modified_time, \
                 error_message} (times ISO-8601 UTC, null = never)}",
        run: |s, p| crate::model_get::table_get(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::palette("Table properties (read)", "table"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("name", K::Name(Obj::Table)),
            P::opt("culture", K::Name(Obj::Culture)),
        ],
    },
    Command {
        id: "column.get",
        label: "Read one column with every property",
        params: "{table, name, culture? (as for model.get)}: returns {name, table, kind: data|calculated|calculated_table_column, \
                 data_type, source_column, expression, format_string, display_folder, summarize_by, \
                 sort_by_column, data_category, description, hidden, is_key, is_unique, is_nullable, \
                 is_available_in_mdx, is_name_inferred, is_data_type_inferred, is_default_label, \
                 is_default_image, keep_unique_rows, encoding_hint, alignment, source_provider_type, \
                 lineage_tag, source_lineage_tag, alternate_of, variations (TMSL), annotations, extra, \
                 relationships: [names of the relationships using it]}",
        run: |s, p| crate::model_get::column_get(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::palette("Column properties (read)", "column"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("table", K::Name(Obj::Table)),
            P::req("name", K::Name(Obj::Column)),
            P::opt("culture", K::Name(Obj::Culture)),
        ],
    },
    Command {
        id: "column.values",
        label: "List a column's distinct values (a basic filter's list)",
        params: "{table, column, search? (case-insensitive), limit? (1-10000, default 10000)}: the \
                 values its stored data or literal partitions hold, sorted, blank first. Returns \
                 {values, count, truncated, available}; available false when there is no data",
        run: |s, p| crate::filter_edit::column_values(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::palette("Column values", "list"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("table", K::Name(Obj::Table)),
            P::req("column", K::Name(Obj::Column)),
            P::opt("search", K::Text),
            P::opt("limit", K::Integer),
        ],
    },
    Command {
        id: "table.rows",
        label: "Read one page of a table's rows, sorted and filtered (the Data view)",
        params: "{table, offset? (default 0), limit? (1-10000, default 100), columns?: [names] (default every \
                 column), sort?: {column, descending?: bool | order?: asc|desc} (one column; view only, the \
                 model is not changed), filter?: [{column, op?: =|<>|>|>=|<|<=|contains|not_contains|\
                 starts_with|not_starts_with|ends_with|not_ends_with|in|not_in|blank|not_blank (default \
                 `in` with values, `=` with value), value? | values?: [..]} | {column, any: [{op, value? | \
                 values?}] (1-8 tests, one of them holds: the Data view's custom filter with Or)}] (every \
                 condition holds; text without regard to case, dates as ISO text)}: rows from the open file's stored data or its partitions, the same data \
                 dax.evaluate reads. Calculated columns the data does not hold are computed by bicraft's \
                 DAX evaluator (for the page; over the table, up to 2 000 000 rows, to sort or filter on \
                 them). Returns {table, available, reason?, columns: [{name, data_type, kind: \
                 data|calculated, format_string, summarize_by, data_category, sort_by_column, hidden, \
                 available, computed?, reason?}], rows: [[values in the columns' order]], offset, limit, \
                 row_count (after the filter), total_rows, sort, filtered, order_cached, elapsed_ms}; blanks \
                 are null, dates ISO text. A sort or filter orders the whole table once (kept for the next \
                 pages until the data or the model changes), within the memory limit",
        run: |s, p| crate::preview::rows::table_rows(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::palette("Table rows", "table"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("table", K::Name(Obj::Table)),
            P::opt("offset", K::Integer),
            P::opt("limit", K::Integer),
            P::opt("columns", K::List),
            P::opt("sort", K::Json),
            P::opt("filter", K::Json),
        ],
    },
    Command {
        id: "measure.get",
        label: "Read one measure with every property",
        params: "{name, table? (checked when given), culture? (as for model.get)}: returns {name, table, expression, format_string, \
                 format_string_definition (the dynamic format string's DAX), display_folder, description, \
                 hidden, data_type, data_category, is_simple_measure, detail_rows_definition, kpi \
                 (TMSL), lineage_tag, source_lineage_tag, annotations, extra}",
        run: |s, p| crate::model_get::measure_get(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::palette("Measure properties (read)", "measure"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("name", K::Name(Obj::Measure)),
            P::opt("table", K::Name(Obj::Table)),
            P::opt("culture", K::Name(Obj::Culture)),
        ],
    },
    Command {
        id: "relationship.list",
        label: "List the relationships with every property",
        params: "{table?: only those touching this table}: returns {count, relationships: [{name, from: \
                 \"T[C]\", to: \"T[C]\", from_table, from_column, to_table, to_column, is_active, \
                 cross_filter: single|both|automatic, cross_filtering_behavior (TMSL), from_cardinality, \
                 to_cardinality (one|many), security_filtering_behavior (oneDirection|bothDirections|\
                 none), rely_on_referential_integrity, join_on_date_behavior, type, annotations, extra}]}",
        run: |s, p| crate::model_get::relationship_list(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(
            Tab::Model,
            "Relationships",
            "Manage relationships",
            "relationship",
        ),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[P::opt("table", K::Name(Obj::Table))],
    },
    Command {
        id: "model.storage_stats",
        label: "VertiPaq storage statistics of the stored data (PBIX DataModel or PBIP .pbi/cache.abf): rows, cardinality, encoding, dictionary/data/hierarchy sizes, relationships, segments, partitions",
        params: "{table?, columns?: bool (default true), segments?: bool (default false), row_number?: \
                 bool (default false: the engine's RowNumber column is left out)}: returns {source, \
                 streamed, total_size, table_count, tables: [{name, rows, total_size, percent_of_model, \
                 columns_size, dictionary_size, data_size, hierarchy_size, user_hierarchies_size, \
                 relationships_size, segment_count, referential_integrity_violations, partitions: [{name, \
                 number, rows, segments, records_per_segment, state, state_code, refreshed_time, \
                 modified_time, error_message}], user_hierarchies: [{name, size}], \
                 columns: [{name, data_type, encoding: hash|value|none, cardinality, selectivity, \
                 is_row_number, dictionary_size, data_size, hierarchy_size, total_size, segment_count, \
                 storage_name, segments?: [{partition, partition_number, segment, rows, used_size, \
                 compression: NOSPLIT|RLE|XM123, bits, bookmark_bits}], hierarchy_segments?: [... + \
                 structure]}]}], relationships: [{name, from, to, size_from, size_to, size, \
                 missing_keys}], refresh: (as model.get full)}; sizes in bytes; times ISO-8601 UTC. Reads only metadata.sqlitedb and the .idfmeta files \
                 (streamed for backups too large for memory). cache_abf?: string (PBIP: read this \
                 cache.abf instead of the project's, for this command only; source is then \"cache_abf\")",
        run: crate::cache_abf::storage_stats,
        server: ServerPolicy::paths(&[PathParam::clearable("cache_abf")]).read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Analyze, "Storage", "Storage statistics", "chart"),
        shortcut: None,
        enabled: &[Need::Data],
        form: &[
            P::opt("table", K::Name(Obj::Table)),
            P::flag("columns", true),
            P::flag("segments", false),
            P::flag("row_number", false),
            P::opt("cache_abf", K::Path),
        ],
    },
    Command {
        id: "model.export_vpax",
        label: "Write a VPAX file (the VertiPaq Analyzer format: the model with its storage statistics, plus Model.bim)",
        params: "{path: string (.vpax), overwrite?: bool, model_bim?: bool (default true)}: needs stored \
                 data, as model.storage_stats; returns {path, bytes, tables, total_size, model_bim, \
                 parts}; cache_abf?: string (PBIP: read this cache.abf instead of the project's). Also \
                 `bicraft vpax IN OUT.vpax [--cache-abf PATH]`",
        run: crate::cache_abf::export_vpax,
        server: ServerPolicy::paths(&[PathParam::one("path"), PathParam::clearable("cache_abf")]),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Analyze, "Storage", "Export VPAX", "export"),
        shortcut: None,
        enabled: &[Need::Data],
        form: &[
            P::req("path", K::Path),
            P::flag("overwrite", false),
            P::flag("model_bim", true),
            P::opt("cache_abf", K::Path),
        ],
    },
    Command {
        id: "dax.validate",
        label: "Validate a DAX expression (against the open model when there is one)",
        params: "{expression: string, table?: string, kind?: measure|calculated_column|calculated_table}: \
                 `table` is the home table of the expression; `kind` adds the evaluation-context rules of \
                 that object (a measure has no row context, a calculated column has its table's, a \
                 calculated table must return a table), as measure.set, column.set and partition.set \
                 check them. Returns {valid, checked_against_model, diagnostics: [{severity, code, \
                 message, span: {start, end} (bytes), line, column}]}",
        run: |s, p| dax_validate(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Analyze, "DAX", "Check DAX", "validate"),
        shortcut: None,
        enabled: &[],
        form: &[
            P::req("expression", K::Dax),
            P::opt("table", K::Name(Obj::Table)),
            P::opt("kind", K::Enum(DAX_KINDS)),
        ],
    },
    Command {
        id: "dax.functions",
        label: "List the DAX functions with their signatures (the formula bar's autocomplete)",
        params: "{query?: string (a part of the name, case ignored)}: returns {functions: [{name, \
                 category, signature (`DIVIDE(numerator, denominator[, alternate_result])`), params: \
                 [{name, optional, repeating}], deprecated, description, description_fr (BIcraft's one-sentence descriptions, English and French)}], keywords: [VAR, RETURN, …]}, sorted by name",
        run: |s, p| dax_functions(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::palette("DAX functions", "list"),
        shortcut: None,
        enabled: &[],
        form: &[P::opt("query", K::Text)],
    },
    Command {
        id: "m.validate",
        label: "Check a Power Query (M) expression without running it: syntax and undefined names",
        params: "{expression: string, shared_expressions?: bool (default true: the open model's queries \
                 and parameters count as defined), names?: [string] (more defined names)}: returns \
                 {valid, syntax_valid, checked_against_model, diagnostics: [{severity, code \
                 (m-syntax|m-undefined-name|m-unknown-function), message, name?, line, column, span}], \
                 steps (the let variables), result_type (Table, Text, Number, Function, ... when the \
                 text tells it), references: {queries, library: [{name, kind: \
                 evaluated|documented|connector}]}}. Nothing is evaluated: connectors are fine. Unknown \
                 library functions are warnings",
        run: |s, p| crate::mquery::m_validate(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Data, "Queries", "Check M", "validate"),
        shortcut: None,
        enabled: &[],
        form: &[
            P::req("expression", K::M),
            P::flag("shared_expressions", true),
            P::opt("names", K::List),
        ],
    },
    Command {
        id: "m.format",
        label: "Format a Power Query (M) expression: one step per line, even spacing and indentation, comments kept",
        params: "{expression?: string, name?: a query (shared expression) or a table (its M partition), \
                 partition?: a partition of that table, apply?: bool (default false: the named query takes \
                 the formatted text, one undo step), max_width?: int (default 100), indent?: int (spaces per \
                 level, default 4, at most 16), tabs?: bool (indent with tabs)}: returns {name, source: \
                 query|table|expression, expression (the formatted text), changed, applied}. Only white space \
                 changes: tokens, comments, texts and quoted identifiers stay as written; the result is \
                 checked to parse to the same tree. Idempotent: formatting the result changes nothing",
        run: |s, p| crate::mquery::m_format(s, p),
        server: ServerPolicy::ALLOWED.read_only_unless(&["apply"]),
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Data, "Queries", "Format M", "format"),
        shortcut: None,
        enabled: &[],
        form: &[
            P::opt("expression", K::M),
            P::opt("name", K::Name(Obj::Expression)),
            P::opt("partition", K::Text),
            P::flag("apply", false),
            P::opt("max_width", K::Integer),
            P::opt("indent", K::Integer),
            P::flag("tabs", false),
        ],
    },
    Command {
        id: "m.complete",
        label: "Suggest what may come at a cursor in Power Query (M): functions, steps, queries, columns, keywords",
        params: "{expression: string, cursor?: int (characters from the start; default the end) | line?, \
                 column?: int (from 1), table?: a table of the model (its columns in scope inside [ ]), \
                 columns?: [string], queries?: [string] (more than the model's), steps?: [string] (more \
                 than the text's own), explicit?: bool (default false: suggestions only on a name being \
                 typed or inside [ ]; true lists everything, as Ctrl+Space)}: returns {replace: {start, \
                 end} (characters a suggestion replaces), items: [{label, insert, kind: \
                 function|constant|keyword|step|query|column, detail (a function's signature, a \
                 constant's or keyword's description)}]}, best matches first, at most 200; none in a \
                 comment or a text. From BIcraft's catalogue of the M functions its evaluator runs",
        run: |s, p| crate::m_assist::m_complete(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Data, "Queries", "M suggestions", "list"),
        shortcut: None,
        enabled: &[],
        form: &[
            P::req("expression", K::M),
            P::opt("cursor", K::Integer),
            P::opt("line", K::Integer),
            P::opt("column", K::Integer),
            P::opt("table", K::Name(Obj::Table)),
            P::opt("columns", K::List),
            P::opt("queries", K::List),
            P::opt("steps", K::List),
            P::flag("explicit", false),
        ],
    },
    Command {
        id: "m.signature",
        label: "Show the signature of the Power Query (M) library call at a cursor and the parameter being typed",
        params: "{expression: string, cursor?: int (characters from the start; default the end) | line?, \
                 column?: int (from 1)}: returns {function: {name, signature, description, params: [{name, \
                 optional}]} | null, argument (its position among the arguments typed, from 0), active \
                 (the parameter it fills, null past the last)}. The innermost call of a library function \
                 around the cursor; a call of anything else is looked through",
        run: |s, p| crate::m_assist::m_signature(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Data, "Queries", "M signature help", "info"),
        shortcut: None,
        enabled: &[],
        form: &[
            P::req("expression", K::M),
            P::opt("cursor", K::Integer),
            P::opt("line", K::Integer),
            P::opt("column", K::Integer),
        ],
    },
    Command {
        id: "m.hover",
        label: "Describe the Power Query (M) name at a cursor: a function's signature, a constant, a keyword, a step, a query or a column",
        params: "{expression: string, cursor?: int (characters from the start; default the end) | line?, \
                 column?: int (from 1), table?, columns?, queries?, steps? (the names in scope, as \
                 m.complete)}: returns {hover: {name, kind: function|constant|keyword|step|query|column, \
                 range: {start, end} (characters), signature (functions), params (functions), \
                 description} | null} for the name on or just before the cursor",
        run: |s, p| crate::m_assist::m_hover(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Data, "Queries", "M name info", "info"),
        shortcut: None,
        enabled: &[],
        form: &[
            P::req("expression", K::M),
            P::opt("cursor", K::Integer),
            P::opt("line", K::Integer),
            P::opt("column", K::Integer),
            P::opt("table", K::Name(Obj::Table)),
            P::opt("columns", K::List),
            P::opt("queries", K::List),
            P::opt("steps", K::List),
        ],
    },
    Command {
        id: "query.preview",
        label: "Preview the first rows of a Power Query (M) query without loading it into the model",
        params: "{name?: a query (shared expression) or a table (its M partition), partition?: a partition \
                 of that table, expression?: M text (instead of name), limit? (1-10000, default 1000), \
                 data_root?: string (the folder file functions may read; default the session's), \
                 allow_connectors?: [\"databricks\" | \"sqlserver\" | \"web\" | \"odata\" | \"sharepoint\"] (default none: a query that reads an \
                 external source fails), timeout_ms? (default 60000, at most 600000), step?: one of the \
                 query's applied steps (query.steps; default the result): the query as it is at that \
                 step, queries?: [{name, expression}] (the Power Query editor's drafts of other queries, \
                 read in place of the model's queries of those names)}: evaluated by bicraft's M evaluator as \
                 Table.FirstN(query, limit + 1), with the model's other queries and parameters and its \
                 culture; nothing is loaded or changed. Naming allow_connectors opens network connections \
                 (refused to readers, and in server mode unless the server allows network access). Returns \
                 {name, source: query|table|expression, columns: [{name, data_type, date_kind? (date|\
                 datetime|time|datetimezone: a date/time column's M type)}], rows: [[values]], \
                 row_count, truncated, connectors, elapsed_ms}",
        run: |s, p| crate::query_preview::query_preview(s, p),
        // the connector checks the server's network setting itself (`refresh::connectors`)
        server: ServerPolicy::paths(&[PathParam::clearable("data_root")])
            .read_only_unless(&["allow_connectors"]),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Home, "Queries", "Preview query", "table"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::opt("name", K::Name(Obj::Expression)),
            P::opt("partition", K::Text),
            P::opt("expression", K::M),
            P::opt("limit", K::Integer),
            P::opt("data_root", K::Path),
            P::opt("allow_connectors", K::List),
            P::opt("timeout_ms", K::Integer),
            P::opt("step", K::Text),
            P::opt("queries", K::Json),
        ],
    },
    Command {
        id: "query.folding",
        label: "Query folding indicators: for each applied step of a Power Query (M) query, how much of the query up to it the source runs",
        params: "{name?: a query or a table (its M partition), partition?, expression?: M text (a draft; \
                 instead of name), data_root?, allow_connectors?: [\"databricks\" | \"sqlserver\" | \
                 \"web\" | \"odata\" | \"sharepoint\"] (default none: a step that reads an external source is unknown), \
                 timeout_ms? (all the steps together; default 60000, at most 600000)}: each step is \
                 evaluated by bicraft's M evaluator without reading any remote table's rows (navigation \
                 listings and column lists may be read). Returns {name, source, steps: [{name, folding: \
                 folded (the query up to the step goes to the source as one request) | partial (the \
                 source runs part of it, bicraft the rest) | not_folded (no source that folds) | unknown \
                 (not evaluated: reason), connector, native_available (query.native shows it), reason}], \
                 connectors (the families the query calls), allow_connectors, elapsed_ms}. Nothing \
                 changes",
        run: |s, p| crate::query_folding::query_folding(s, p),
        server: ServerPolicy::paths(&[PathParam::clearable("data_root")])
            .read_only_unless(&["allow_connectors"]),
        replay: Replay::Skip,
        place: Place::palette("Query folding indicators (Power Query)", "list"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::opt("name", K::Name(Obj::Expression)),
            P::opt("partition", K::Text),
            P::opt("expression", K::M),
            P::opt("data_root", K::Path),
            P::opt("allow_connectors", K::List),
            P::opt("timeout_ms", K::Integer),
        ],
    },
    Command {
        id: "query.native",
        label: "View native query: the SQL statement or URL bicraft would send to the source for a Power Query (M) query at one of its steps",
        params: "{name? | expression? (as query.folding), partition?, step?: one of the query's \
                 applied steps (default the result), data_root?, allow_connectors?, timeout_ms?}: \
                 nothing is sent and no rows are read. Returns {name, step, folding (as query.folding), \
                 connector, kind: sql | url | null, native: the text, read-only (a Databricks statement \
                 lists its parameters as -- :p0 = value (TYPE) comments), or null with reason (the step \
                 does not fold, or is a navigation table)}",
        run: |s, p| crate::query_folding::query_native(s, p),
        server: ServerPolicy::paths(&[PathParam::clearable("data_root")])
            .read_only_unless(&["allow_connectors"]),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Data, "Queries", "View native query", "server"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::opt("name", K::Name(Obj::Expression)),
            P::opt("partition", K::Text),
            P::opt("expression", K::M),
            P::opt("step", K::Text),
            P::opt("data_root", K::Path),
            P::opt("allow_connectors", K::List),
            P::opt("timeout_ms", K::Integer),
        ],
    },
    Command {
        id: "query.profile",
        label: "Profile a Power Query (M) query's columns: column quality, column distribution and a column profile, as the Power Query editor's data profiling",
        params: "{name? | expression? (as query.preview), partition?, step?: an applied step (default \
                 the result), column?: a column to profile in full, full?: bool (default false: the \
                 first 1000 rows, as the editor's \"Column profiling based on top 1000 rows\"; true: the \
                 entire data set), data_root?, allow_connectors?, timeout_ms?, queries? (as \
                 query.preview)}: \
                 evaluated by bicraft's M evaluator as query.preview is; nothing is loaded or \
                 changed. Returns {name, step, source, full, rows (rows profiled), truncated (more \
                 rows than profiled), columns: [{name, data_type, count, valid, errors, empty (null or \
                 empty text), valid_share, error_share, empty_share (0 to 1), distinct, unique \
                 (values seen once), distribution: [{value, count}] (the 30 most frequent)}], column: \
                 null | {name, data_type, statistics: {count, errors, empty, distinct, unique, min, \
                 max; numbers: average, standard_deviation, zero, even, odd; text: empty_string}, \
                 values: [{value, count}] (the 100 most frequent), values_listed}, connectors, \
                 elapsed_ms}",
        run: |s, p| crate::query_profile::query_profile(s, p),
        server: ServerPolicy::paths(&[PathParam::clearable("data_root")])
            .read_only_unless(&["allow_connectors"]),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Data, "Queries", "Column profile", "chart"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::opt("name", K::Name(Obj::Expression)),
            P::opt("partition", K::Text),
            P::opt("expression", K::M),
            P::opt("step", K::Text),
            P::opt("column", K::Text),
            P::flag("full", false),
            P::opt("data_root", K::Path),
            P::opt("allow_connectors", K::List),
            P::opt("timeout_ms", K::Integer),
            P::opt("queries", K::Json),
        ],
    },
    Command {
        id: "query.export",
        label: "Export a Power Query (M) query's results (every row, or the query at one of its steps) to a CSV file, as the Power Query editor's Output data",
        params: "{name?: a query or a table (its M partition), partition?, expression?: M text (a draft; \
                 instead of name), step?: one of the query's applied steps (default the result), \
                 queries?: [{name, expression}] (the editor's drafts of other queries), data_root?, \
                 allow_connectors? (as query.preview), timeout_ms?, out: string (the .csv file)}: \
                 evaluated by bicraft's M evaluator as query.preview is, without its row limit (the \
                 evaluator's row cap applies); written as UTF-8 with a byte order mark, a header row \
                 and CRLF line ends (text starting with = + - @ gets a leading '); nothing in the \
                 model changes. Returns {name, step, source, path, bytes, columns, rows, elapsed_ms}",
        run: |s, p| crate::query_export::query_export(s, p),
        server: ServerPolicy::paths(&[PathParam::one("out"), PathParam::clearable("data_root")]),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Data, "Queries", "Export query results", "export"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::opt("name", K::Name(Obj::Expression)),
            P::opt("partition", K::Text),
            P::opt("expression", K::M),
            P::opt("step", K::Text),
            P::req("out", K::Path),
            P::opt("data_root", K::Path),
            P::opt("allow_connectors", K::List),
            P::opt("timeout_ms", K::Integer),
            P::opt("queries", K::Json),
        ],
    },
    Command {
        id: "query.diagnose",
        label: "Query diagnostics: a Power Query (M) query at one step (or each step) evaluated as a refresh does, with BIcraft's own time, rows and the requests sent to its sources",
        params: "{name? | expression? (as query.folding), partition?, step?: one of the query's applied \
                 steps (default the result), all_steps?: bool (each step in turn; not with step), \
                 data_root?, allow_connectors?, timeout_ms? (all the steps together)}: returns {name, \
                 steps: [{name, elapsed_ms (BIcraft's own, the steps before it included), rows, columns, \
                 error, requests: [{connector, kind: list | query, text (the native query or the \
                 navigation path), rows, elapsed_ms}], requests_total}], allow_connectors, elapsed_ms}. \
                 Nothing is loaded or changed",
        run: |s, p| crate::query_diagnose::query_diagnose(s, p),
        server: ServerPolicy::paths(&[PathParam::clearable("data_root")])
            .read_only_unless(&["allow_connectors"]),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Data, "Queries", "Query diagnostics", "play"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::opt("name", K::Name(Obj::Expression)),
            P::opt("partition", K::Text),
            P::opt("expression", K::M),
            P::opt("step", K::Text),
            P::flag("all_steps", false),
            P::opt("data_root", K::Path),
            P::opt("allow_connectors", K::List),
            P::opt("timeout_ms", K::Integer),
        ],
    },
    Command {
        id: "query.dependencies",
        label: "Query dependencies: the graph of the Power Query queries, the data sources they read and the model tables they load into",
        params: "{queries?: [{name, expression, load?}] (the editor's drafts; load default true for \
                 a table of the model or a new query, false for a shared query; default the model's \
                 queries)}: nothing is evaluated (references and source calls are read from the M \
                 text). Returns {nodes: [{id, kind: source|query|parameter|table|model_table, name, \
                 function? (a source's), loaded?, layer (0 the sources, each query after the queries \
                 it reads, the model tables last)}], edges: [{from, to, kind: source|reference|load}], \
                 layers, unparsed: [{name, error, line, column}] (queries without edges)}",
        run: |s, p| crate::query_deps::query_dependencies(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Data, "Queries", "Query dependencies", "model"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[P::opt("queries", K::Json)],
    },
    Command {
        id: "query.list",
        label: "Transform data: list the model's Power Query (M) queries (tables loaded by an M partition, shared queries and parameters) with their M",
        params: "{}: returns {queries: [{name, source: table|query, expression (the M), partition (a \
                 table's M partition), partitions, hidden, group (a shared query's query group), \
                 parameter (a parameter query)}], data_root}. Nothing changes. The app's Transform data \
                 opens the Power Query editor on them",
        run: |s, p| crate::query_steps::query_list(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Home, "Queries", "Transform data", "edit"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[],
    },
    Command {
        id: "query.steps",
        label: "The applied steps of a Power Query (M) query: the variables of its let, as the Power Query editor lists them",
        params: "{name?: a query or a table (its M partition), partition?, expression?: M text (a draft; \
                 instead of name)}: returns {name, source, expression, is_let, steps: [{name, expression \
                 (the step's M as written), function (the function it calls first, e.g. \
                 Table.SelectRows), settings (the {kind, …} of query.apply_step that writes the step, \
                 read back from its M for the transforms that ask values, else null: the step settings \
                 gear)}], result (the in expression), result_step (the step the result \
                 names, else the last), step: null, connectors (the allow_connectors families the query calls, itself or \
                 through the model's queries)}. A query that is not a let is one step `Source`. \
                 Nothing is evaluated or changed; preview a step with query.preview {step}",
        run: |s, p| crate::query_steps::query_steps(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Data, "Queries", "Applied steps", "list"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::opt("name", K::Name(Obj::Expression)),
            P::opt("partition", K::Text),
            P::opt("expression", K::M),
        ],
    },
    Command {
        id: "query.apply_step",
        label: "Add a Power Query step after another: a transform of the editor's ribbon (filter rows, remove or keep columns and rows, rename, change type, promote headers, sort, remove duplicates, replace values or errors, fill, group by, merge, expand, append, custom, conditional and index columns, split, merge columns, unpivot, pivot, transpose, text, number and date transforms) written as Power Query writes it",
        params: "{name? | expression? (as query.steps), partition?, after?: a step (default the last), \
                 step: {kind, …}, step_name? (default the editor's: Filtered Rows, Changed Type1…)}: \
                 kinds: filter_rows {column, op: equals|not_equals|greater|greater_or_equal|less|\
                 less_or_equal|contains|not_contains|begins_with|ends_with|is_null|is_not_null, value: \
                 text|number|true/false|null}; filter_values {column, values: [text|number|true/false|\
                 null], exclude? (default false: keep the rows with one of the values; true: drop \
                 them)} (a column filter menu's value list); remove_columns|keep_columns|remove_duplicates {columns} \
                 (remove_duplicates without columns: whole rows); rename_columns {renames: [{column, \
                 new_name}]}; change_type {columns: [{column, type: text|whole|decimal|currency|\
                 percentage|date|datetime|datetimezone|time|duration|logical|any}]}; promote_headers {}; \
                 sort {columns: [{column, descending?}]}; replace_values {columns, find, replace}; \
                 group_by {keys, aggregations: [{name, op: count|count_distinct|sum|average|median|min|\
                 max|all_rows, column?}]}; merge {other (a query), keys, other_keys, join?: left_outer|\
                 right_outer|full_outer|inner|left_anti|right_anti, column? (default other)}; expand \
                 {column, columns, prefix? (default true: Other.Column)}; append {others: [query]}; \
                 add_column {name, formula (M for each row, e.g. [Qty] * 2)}; split_column {column, \
                 delimiter, parts? (default 2)}; unpivot {columns, other? (default true: unpivot the \
                 other columns)}; pivot {column, values, aggregate?: none|count|sum|average|median|min|\
                 max}; custom {} (Insert step after: = the step before); remove_top_rows|\
                 remove_bottom_rows|keep_top_rows|keep_bottom_rows {count}; keep_range {first? (1-based, \
                 default 1), count}; remove_alternate_rows {first?, remove?, keep?}; remove_blank_rows {}; \
                 remove_errors|keep_errors {columns? (none: any column)}; replace_errors {columns, \
                 value}; fill_down|fill_up {columns}; reverse_rows|transpose|demote_headers {}; \
                 reorder_columns {columns (the new order)}; duplicate_column {column, name? (default \
                 `C - Copy`)}; add_index {name? (Index), start? (0), step? (1)}; merge_columns {columns, \
                 delimiter?, name? (Merged), add? (true: a new column, the columns kept)}; \
                 conditional_column {name?, rules: [{column, op (as filter_rows), value, output}], \
                 otherwise?}; column_op {op: lower|upper|proper|trim|clean|prefix|suffix (text)|length|\
                 first_chars|last_chars (count)|text_range (start, count)|before_delimiter|\
                 after_delimiter (text)|between_delimiters (text: start delimiter, end: end \
                 delimiter)|year|month|day|quarter|month_name|day_name|day_of_week|\
                 week_of_year|day_of_year|start_of_month|end_of_month|start_of_year|end_of_year|\
                 start_of_quarter|end_of_quarter|start_of_week|end_of_week|start_of_day|end_of_day|\
                 days_in_month|week_of_month|age|date_only|time_only|hour|minute|second|\
                 start_of_hour|end_of_hour|duration_days|duration_hours|duration_minutes|\
                 duration_seconds|total_years|total_days|total_hours|total_minutes|total_seconds|\
                 round (count: decimal places)|round_up|round_down|abs|square_root|exponent|log10|ln|\
                 factorial|sin|cos|tan|asin|acos|atan|is_even|is_odd|sign|add|subtract|multiply|divide|\
                 integer_divide|modulo|percentage|percent_of|power (number), columns, add? (true: a \
                 new column `name` from the first column, else in place)}. A value may be {\"date\": \"YYYY-MM-DD\"} or \
                 {\"datetime\": \"YYYY-MM-DDTHH:MM:SS\"} (#date, #datetime). The step reads `after`; the \
                 step that read `after` next (or the result) reads the new step. The rest of the text \
                 is kept as written. Nothing in the model changes (query.apply writes it). Returns \
                 query.steps's answer for the new text, with step (the new step's name) and kind",
        run: |s, p| crate::query_steps::query_apply_step(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Data, "Queries", "Add step", "plus"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::opt("name", K::Name(Obj::Expression)),
            P::opt("partition", K::Text),
            P::opt("expression", K::M),
            P::opt("after", K::Text),
            P::req("step", K::Json),
            P::opt("step_name", K::Text),
        ],
    },
    Command {
        id: "query.edit_step",
        label: "Change a Power Query step's formula (the formula bar) or its settings (the gear), or rename it; the steps that use it follow the new name",
        params: "{name? | expression? (as query.steps), partition?, step, formula?: M (a leading = is \
                 dropped), settings?: {kind, …} (as query.apply_step's step: the step written anew, \
                 reading the step it read), new_name?}: at least one of formula, settings and new_name; \
                 not both formula and settings. Only that step's text (and \
                 the references to a renamed step) changes; the result must parse. Nothing in the model \
                 changes. Returns query.steps's answer with step (its name now)",
        run: |s, p| crate::query_steps::query_edit_step(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Data, "Queries", "Edit step", "edit"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::opt("name", K::Name(Obj::Expression)),
            P::opt("partition", K::Text),
            P::opt("expression", K::M),
            P::req("step", K::Text),
            P::opt("formula", K::M),
            P::opt("settings", K::Json),
            P::opt("new_name", K::Text),
        ],
    },
    Command {
        id: "query.remove_step",
        label: "Delete a Power Query step; what used it reads the step before",
        params: "{name? | expression? (as query.steps), partition?, step}: the first step goes only when \
                 no step uses it; a query keeps one step. Nothing in the model changes. Returns \
                 query.steps's answer",
        run: |s, p| crate::query_steps::query_remove_step(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::palette("Delete a Power Query step", "delete"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::opt("name", K::Name(Obj::Expression)),
            P::opt("partition", K::Text),
            P::opt("expression", K::M),
            P::req("step", K::Text),
        ],
    },
    Command {
        id: "query.new",
        label: "A new Power Query query for the editor: a blank query, a duplicate of a query or a reference to it",
        params: "{kind?: blank (default) | duplicate | reference, of?: the query duplicated or \
                 referenced, expression?: its M (duplicate: the editor's draft; default the model's), \
                 names?: [the names already taken] (default the model's queries)}: returns {name \
                 (Query1, Query2… for a blank query; `of (2)`, `of (3)`… otherwise), expression (blank: \
                 let Source = \"\" in Source; reference: let Source = of in Source), kind}. Nothing in \
                 the model changes: query.apply loads it",
        run: |s, p| crate::query_steps::query_new(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Data, "Queries", "Blank query", "plus"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::opt("kind", K::Text),
            P::opt("of", K::Text),
            P::opt("expression", K::M),
            P::opt("names", K::List),
        ],
    },
    Command {
        id: "query.rename",
        label: "Rename a query in the Power Query editor's drafts; the other queries that read it follow",
        params: "{name, new_name, queries: [{name, expression}] (the editor's queries, the renamed one \
                 among them)}: the new name must be free among them and the model's calculated tables. \
                 Returns {renamed, name, queries: [{name, expression, references}] (the drafts whose \
                 references changed, under their names after the rename), not_rewritten: [names of \
                 drafts that do not parse]}. Nothing in the model changes: query.apply {renamed} \
                 renames it there",
        run: |s, p| crate::query_steps::query_rename(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Data, "Queries", "Rename draft query", "rename"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("name", K::Text),
            P::req("new_name", K::Text),
            P::req("queries", K::Json),
        ],
    },
    Command {
        id: "query.apply",
        label: "Close & Apply: write the Power Query editor's queries back to the model and reload their tables, as one edit",
        params: "{queries: [{name, expression}] (1 to 200: a shared query, a table loaded by one M \
                 partition, or a new name: a new import table, as source.load), renamed?: [{name, \
                 new_name}] (model queries or tables renamed in the editor, first, in order; the M of \
                 the other queries follows; `queries` names them by their new names), removed?: [name] \
                 (model queries or tables deleted in the editor, last; refused while another M query \
                 reads one), data_root?: string (the \
                 folder file functions may read, kept for the session as table.import does), refresh?: \
                 bool (default true: each changed table, and each table that reads a changed shared \
                 query, is reloaded by bicraft's M evaluator: its columns follow the query's, its \
                 partition keeps its name and mode; false: the text only, as partition.set and \
                 expression.set)}: all or nothing, one edit.undo step; a query left as it was is \
                 skipped. A table of several partitions gets its first M partition's text, not \
                 reloaded (a warning). Returns {queries: [{name, source, changed?, new?, reloaded?, \
                 rows?, columns?}], refreshed, warnings, renamed, removed}",
        run: crate::query_steps::query_apply,
        server: ServerPolicy::paths(&[PathParam::clearable("data_root")]),
        replay: Replay::Copy,
        place: Place::palette("Close & Apply (Power Query)", "check"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::opt("queries", K::Json),
            P::opt("renamed", K::Json),
            P::opt("removed", K::List),
            P::opt("data_root", K::Path),
            P::flag("refresh", true),
        ],
    },
    Command {
        id: "model.validate",
        label: "Validate every DAX object of the open model (measures, calculated columns and tables, \
                row filters): types, evaluation contexts and circular dependencies; and the M of partitions \
                and shared expressions (`m`: syntax, undefined names); and relationships Power BI \
                Desktop does not open (`relationships.invalid`: a one-to-one relationship with \
                single-direction cross filtering)",
        params: "{}",
        run: |s, p| model_validate(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Analyze, "Validate", "Validate model", "validate"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[],
    },
    Command {
        id: "model.analyze",
        label: "Best-practice analysis of the open model: Microsoft's BPA rules (embedded) plus any rule \
                file in the same format (JSON rules with Dynamic LINQ expressions), and, with a report open, \
                the report's references to fields the model no longer has",
        params: "{rules?: string | string[] | rules (a rule file path, several paths, or inline rules: a JSON \
                 array of {ID, Name, Category, Description, Severity, Scope, Expression, FixExpression?, \
                 CompatibilityLevel?}), default_rules?: bool (default true: the embedded Microsoft rules), \
                 rule_ids?: string[], severity_min?: 1-5 | \"info\" | \"warning\" | \"error\", \
                 max_findings?: number (default 10000), report_rules?: bool (default true: with a report \
                 open, the REPORT_* rules: fields missing from the model in visuals, sorts, filters, \
                 drillthrough, formatting, bookmarks, properties and field parameters, stale queryRefs and \
                 formatting selectors, dangling visual interactions; each finding has `location` {kind, \
                 page, page_name, visual?, role?, bookmark?...}, `message`, `suggestion?` and `fix?` \
                 {command, params} when an existing command removes the stale reference)}: returns \
                 findings (rule, severity, object) and, per rule, whether it was evaluated, partly \
                 evaluated or not evaluated (with the reason)",
        run: |s, p| crate::analyze::model_analyze(s, p),
        server: ServerPolicy::paths(&[PathParam::many("rules")]).read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Analyze, "Validate", "Best practices", "check"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::opt("rules", K::Paths),
            P::flag("default_rules", true),
            P::opt("rule_ids", K::List),
            P::opt("severity_min", K::Text),
            P::opt("max_findings", K::Integer),
            P::flag("report_rules", true),
        ],
    },
    Command {
        id: "model.usage",
        label: "Where model objects are used: DAX (direct and transitive), RLS/OLS, relationships, hierarchies, \
                sort-by, M queries, field parameters, calculation groups, perspectives, translations, and the \
                report (visual roles, filters, drillthrough, tooltips, bookmarks, formatting, sort)",
        params: "{object?: string (Table[Column], [Measure], Table[Measure], 'Table' or Table, or an M expression \
                 name), kind?: \"table\"|\"column\"|\"measure\"|\"expression\", reports?: string[] (more reports \
                 bound to this model: .pbix/.pbit, .pbip, project or <Name>.Report folders), graph?: bool \
                 (default true)}: with `object`: {object, used, used_by_report, direct: [usage], indirect: \
                 [{through: [objects], usage}], dependants: [{object, depth, used}], depends_on, \
                 depends_on_all, graph: {nodes, edges: [[from, to, kind]] (from needs to)}}; without: \
                 {objects: [{object, used, used_by_report, direct_count, used_by}], counts, graph} (`kind` \
                 filters). A usage is {kind, ...}: kind is a DAX site (measure, calculated column, \
                 calculated table, kpi target, format string expression, calculation item, ...) or relationship, \
                 row filter, object-level security, hierarchy, sort by, variation, key column, aggregation, \
                 group by column, M query, M expression, perspective, translation, linguistic schema, \
                 user-defined function, calculation group, auto date/time table; report usages (with \
                 `report`: `open` or the report's file name) are visual (page, visual, role), visual/page/report \
                 filter, drillthrough, tooltip page, bookmark, formatting, sort, report measure, \
                 visual/page/report property. Also {reports, warnings}; read-only",
        run: |s, p| crate::usage::model_usage(s, p),
        server: ServerPolicy::paths(&[PathParam::many("reports")]).read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Analyze, "Usage", "Where used", "search"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::opt("object", K::Text),
            P::opt("kind", K::Enum(&["table", "column", "measure", "expression"])),
            P::opt("reports", K::Paths),
            P::flag("graph", true),
        ],
    },
    Command {
        id: "model.unused",
        label: "Measures, columns, calculated tables and shared M expressions used nowhere (model or report, directly or through \
                other objects), with the reason, and the objects only model rules keep",
        params: "{reports?: string[] (more reports bound to this model, as for model.usage), allow?: string[] \
                 (names or * ? patterns never to flag: [Measure], Table[Column], 'Table', [KPI*]), kept?: bool \
                 (default true), keep_metadata?: bool (default true; false: perspectives, translations and \
                 the Q&A schema keep nothing)}: returns {unused: {tables, columns, measures, expressions: \
                 [{kind, table?, name, why}]}, counts, kept: [{object, rules, detail | needed_by}], reports, \
                 warnings}. Conservative: relationship keys (inactive too), sort-by targets, RLS/OLS, \
                 hierarchy levels, key and default-label columns, aggregations, calculation groups (and \
                 measures they name in strings), auto date/time tables, user-defined functions, \
                 perspectives, translations, the Q&A linguistic schema and report-level measures keep what \
                 they name; an expression that does not parse keeps every object it names. A shared M \
                 expression is unused when no table's query names it, directly or through used \
                 expressions; M parameters (IsParameterQuery) are kept. Usage is only known for the reports \
                 read: thin reports elsewhere may use what is listed (see warnings); read-only",
        run: |s, p| crate::usage::model_unused(s, p),
        server: ServerPolicy::paths(&[PathParam::many("reports")]).read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Analyze, "Usage", "Unused objects", "eye-off"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::opt("reports", K::Paths),
            P::opt("allow", K::List),
            P::flag("kept", true),
            P::flag("keep_metadata", true),
        ],
    },
    Command {
        id: "model.remove_unused",
        label: "Remove unused measures, columns, calculated tables and shared M expressions (all, or a chosen \
                subset) in one undoable edit",
        params: "{objects?: string[] (default: everything model.unused lists), reports?: string[], allow?: \
                 string[] and keep_metadata?: bool (as for model.unused), dry_run?: bool, force?: bool}: each \
                 object must still be unused under the same reports and allow-list (else nothing is \
                 removed); the removal runs the mutations of table.remove / column.remove / measure.remove \
                 as one model edit (one edit.undo step), drops perspective and translation entries, and is \
                 refused when it would break a DAX expression or the M of an expression left behind (an \
                 unused object that uses a removed one) unless force; removing expressions is refused on a \
                 PBIX with data, like expression.remove. Returns \
                 {removed: {tables, columns, measures, expressions}, count, broken, removed_relationships, dry_run, \
                 perspective_entries_dropped?, translations_dropped?, reports, warnings}; dry_run returns \
                 the same without changing the session",
        run: crate::usage::model_remove_unused,
        server: ServerPolicy::paths(&[PathParam::many("reports")]),
        replay: Replay::Copy,
        place: Place::ribbon(Tab::Analyze, "Usage", "Remove unused", "delete"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::opt("objects", K::List),
            P::opt("reports", K::Paths),
            P::opt("allow", K::List),
            P::flag("keep_metadata", true),
            P::flag("dry_run", false),
            P::flag("force", false),
        ],
    },
    Command {
        id: "table.add",
        label: "Add a table",
        params: "{name, description?, hidden?: bool, partition?: {source_type: \"m\"|\"calculated\", \
                 expression}, force?: bool}: a calculated partition's expression is validated as DAX",
        run: edit::table_add,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Model, "Calculations", "New table", "table"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("name", K::Text),
            P::opt("description", K::Text),
            P::flag("hidden", false),
            P::opt("partition", K::Json),
            P::flag("force", false),
        ],
    },
    Command {
        id: "source.navigate",
        label: "Get data: list the sheets, tables or items of a local file or folder, with the Power Query (M) query that loads each",
        params: "{path: string (a file, or a folder), kind?: excel|csv|json|folder (default: the \
                 extension, folder for a folder), delimiter?: string (a text file's, one character or \
                 \"tab\"; default found from its first line), data_root?: string (the folder file \
                 functions may read; default the session's when the path is under it, else the \
                 file's own folder, the folder itself for a folder), detect_types?: bool (default \
                 true)}: what Power BI Desktop's Navigator lists: a workbook's sheets, tables and named \
                 ranges (kind Sheet, Table, DefinedName; hidden sheets say so), a text file (File), a \
                 JSON file's list and its record's lists and records (List, Record), a folder's file \
                 list (Folder) and its CSV files appended (Combined). Each item's `m` is the query \
                 Desktop's Load writes: the source, the first row promoted to headers when it reads as \
                 one, a Table.TransformColumnTypes step with types detected from the first 200 rows; \
                 an item whose query does not load has `error`. Nothing changes: preview an item with \
                 query.preview {expression: m, data_root}, load the ones picked with source.load. \
                 Returns {kind, path, data_root, items: [{name, kind, m, hidden, error?}], truncated}. \
                 kind sqlserver {server: string (host, host,port or host\\instance), database: string, \
                 query?: string (a native SQL query)}: the database's user tables and views (kind Table, \
                 View; name schema.item) through the SQL Server connector, each `m` a \
                 Sql.Database(server, database){[Schema, Item]}[Data] query (with `query`: one item Query1, \
                 Sql.Database(server, database, [Query = …])); opens a TLS connection to the server with \
                 the credentials of source.credentials (else BICRAFT_SQLSERVER_USER / _PASSWORD), so it is \
                 refused to readers and in server mode unless the server allows network access; preview \
                 and load with allow_connectors [\"sqlserver\"]. Returns {kind, server, database, items, \
                 truncated, allow_connectors}. kind web {url: string (http or https), delimiter?, \
                 detect_types?}: the resource read once through the Web connector (with the credential \
                 source.credentials {kind: web} holds for the URL, else anonymously): a JSON \
                 document lists as a JSON file does (Json.Document(Web.Contents(url))), other text as a \
                 text file (Csv.Document(Web.Contents(url), …)); an HTML or XML page is refused. kind odata \
                 {url: string (a service root, or an entity set)}: the service's entity sets (kind Table) \
                 through the OData connector, each `m` an OData.Feed(url, null, [Implementation=\"2.0\"])\
                 {[Name, Signature=\"table\"]}[Data] query. kind sharepoint_folder {url: string (a site's \
                 root address)}: the site's files through the SharePoint connector, as a folder's (kind \
                 Folder: SharePoint.Files(url, [ApiVersion = 15]) with its loadable columns; kind Combined: \
                 its CSV files appended). kind sharepoint_list {url}: the site's lists (kind Table, named by \
                 title), each `m` a SharePoint.Tables(url, [Implementation = \"2.0\", ViewMode = \"All\"])\
                 {[Id = …]}[Items] query. These open network connections to the URL, so \
                 they are refused to readers and in server mode unless the server allows network access; \
                 preview and load with allow_connectors [\"web\"], [\"odata\"] or [\"sharepoint\"]. Returns \
                 {kind, url, items, truncated, allow_connectors}",
        run: |s, p| crate::get_data::source_navigate(s, p),
        server: ServerPolicy::paths(&[PathParam::one("path"), PathParam::clearable("data_root")])
            .read_only_unless(&["server", "url"]),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Home, "Data", "Get data", "table"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::opt("path", K::Path),
            P::opt(
                "kind",
                K::Enum(&[
                    "excel",
                    "csv",
                    "json",
                    "folder",
                    "sqlserver",
                    "web",
                    "odata",
                    "sharepoint_folder",
                    "sharepoint_list",
                ]),
            ),
            P::opt("delimiter", K::Text),
            P::opt("data_root", K::Path),
            P::flag("detect_types", true),
            P::opt("server", K::Text),
            P::opt("database", K::Text),
            P::opt("query", K::Text),
            P::opt("url", K::Text),
        ],
    },
    Command {
        id: "source.credentials",
        label: "Data source credentials: the SQL Server login, or a web source's credential, bicraft uses for a source in this session",
        params: "kind sqlserver (the default otherwise) {server: string (as the queries name it: host, host,port or \
                 host\\instance), user: string (a SQL Server login), password?: string, authentication?: sql \
                 (default; windows is refused: Windows integrated authentication is not supported), clear?: \
                 bool (forget the server's credentials)}: for the SQL Server connector (allow_connectors \
                 [\"sqlserver\"]); without credentials here it reads BICRAFT_SQLSERVER_USER and \
                 BICRAFT_SQLSERVER_PASSWORD. Returns {kind, server (normalized), stored, cleared?}. kind web \
                 (the default when `url`, `key`, `header` or `token` is given; odata and sharepoint name the same store) {url: string (http or https: the level the credential \
                 applies to; a request uses the credential of the longest stored prefix of its URL, whole \
                 path segments), authentication?: anonymous (default) | basic {user, password?} | key {key, \
                 header?: string (the header the key is sent in when the query has no ApiKeyName; with \
                 Web.Contents' ApiKeyName the key goes in that query parameter)} | bearer {token: string (an \
                 access token obtained elsewhere, e.g. an organizational account's for SharePoint Online)}; \
                 windows is refused, clear?: bool}: for Web.Contents, OData.Feed and SharePoint.* \
                 (allow_connectors [\"web\"], [\"odata\"], [\"sharepoint\"]); a redirect never carries a \
                 credential to a URL it does not cover. Returns {kind: web, url (normalized), \
                 authentication, stored, cleared?}. Kept in this session's memory only, never written to a \
                 file, the recovery journal, a log or an answer. Not available in server mode (a shared \
                 session would lend them to every user)",
        run: crate::sqlserver::source_credentials,
        server: ServerPolicy::denied(
            "credentials held by a shared session would serve every user of the workspace",
        ),
        replay: Replay::Skip,
        place: Place::palette("Data source credentials", "lock"),
        shortcut: None,
        enabled: &[],
        form: &[
            P::opt("kind", K::Enum(&["sqlserver", "web", "odata", "sharepoint"])),
            P::opt("server", K::Text),
            P::opt("url", K::Text),
            P::opt(
                "authentication",
                K::Enum(&["sql", "anonymous", "basic", "key", "bearer", "windows"]),
            ),
            P::opt("user", K::Text),
            P::opt("password", K::Text),
            P::opt("key", K::Text),
            P::opt("header", K::Text),
            P::opt("token", K::Text),
            P::flag("clear", false),
        ],
    },
    Command {
        id: "source.privacy",
        label: "Set the privacy level of a data source for this session (None, Public, Organizational, Private)",
        params: "{source: string (a URL for kind web, a server for sqlserver, a path for file), level: \
                 none|public|organizational|private, kind?: web (default; odata and sharepoint are the same) \
                 | sqlserver | file, clear?: bool (forget the source's level)}: Power Query's privacy level \
                 of the source (Microsoft Learn, Privacy levels in Power Query), recorded in this session's \
                 memory and shown by source.settings; bicraft does not run Power Query's formula firewall, \
                 so levels do not change how queries combine sources. Returns {kind, source (normalized), \
                 level, cleared?}",
        run: crate::source_settings::source_privacy,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Skip,
        place: Place::palette("Data source privacy level", "lock"),
        shortcut: None,
        enabled: &[],
        form: &[
            P::req("source", K::Text),
            P::opt("level", K::Enum(&["none", "public", "organizational", "private"])),
            P::opt(
                "kind",
                K::Enum(&["web", "odata", "sharepoint", "sqlserver", "file"]),
            ),
            P::flag("clear", false),
        ],
    },
    Command {
        id: "source.settings",
        label: "Data source settings: the model's data sources with their credential kind and privacy level",
        params: "{}: the data sources the model's Power Query (M) names with a literal (URLs of \
                 Web.Contents, OData.Feed and SharePoint.*, SQL Server servers, file and folder paths; as \
                 sources.list finds them) and the sources this session holds a credential or privacy level \
                 for. Returns {sources: [{kind: web|sqlserver|file, source, key (normalized), functions, \
                 credential: anonymous|basic|key|bearer|database|null (the kind only, never the secret), \
                 privacy: none|public|organizational|private|null}], firewall: false (privacy levels are \
                 recorded, not enforced)}. Read-only",
        run: |s, p| crate::source_settings::source_settings(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Home, "Queries", "Data source settings", "lock"),
        shortcut: None,
        enabled: &[],
        form: &[],
    },
    Command {
        id: "source.load",
        label: "Load the items picked in the Navigator: one import table per Power Query (M) query, as one edit",
        params: "{tables: [{name, m}] (1 to 100; `m` usually an item's query from source.navigate), \
                 data_root?: string (the folder file functions may read, kept for the session as \
                 table.import does; pass source.navigate's), allow_connectors?: [\"sqlserver\" | \"web\" | \
                 \"odata\" | \"sharepoint\"] (pass source.navigate's for a SQL Server, Web, OData or SharePoint source)}: table.import {name, m} for each, all or \
                 nothing (one edit.undo step; a failing table leaves the model as it was and names \
                 itself). Returns {tables: [table.import's answer]}",
        run: crate::get_data::source_load,
        server: ServerPolicy::paths(&[PathParam::clearable("data_root")]),
        replay: Replay::Copy,
        place: Place::palette("Load Navigator items", "import"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("tables", K::Json),
            P::opt("data_root", K::Path),
            P::opt("allow_connectors", K::List),
        ],
    },
    Command {
        id: "table.enter_data",
        label: "Enter data: a table typed in as a grid of typed columns, stored as Power BI Desktop's Enter data query",
        params: "{name, columns: [string | {name, type?: auto|text|whole|number|fixed|date|datetime|\
                 logical}] (auto, the default: detected from the cells as Power Query does), rows: \
                 [[text | number | true/false | null]] (shorter rows end in nulls; an empty text is a \
                 null outside text columns), culture?: string (how the texts are written, e.g. fr-FR; \
                 default the model's query culture), replace?: bool (give an existing table these \
                 rows, Edit data), hidden?: bool, description?}: the rows become the compressed JSON \
                 of Desktop's Enter data query (Table.FromRows(Json.Document(Binary.Decompress(…)))) \
                 and one Table.TransformColumnTypes step, each cell written as the model's query \
                 culture reads it back; checked to load the values typed in. One edit (one edit.undo \
                 step). At most 1,000 columns and 1,000,000 cells. Returns table.import's answer with \
                 {types: [{name, type}], culture}",
        run: crate::get_data::table_enter_data,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Home, "Data", "Enter data", "grid"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("name", K::Text),
            P::req("columns", K::Json),
            P::opt("rows", K::Json),
            P::opt("culture", K::Text),
            P::flag("replace", false),
            P::flag("hidden", false),
            P::opt("description", K::Text),
        ],
    },
    Command {
        id: "table.entered_data",
        label: "Edit data: the columns, types and cells of a table entered as data, to edit and give back to table.enter_data",
        params: "{name}: a table whose one partition is Desktop's Enter data query or a #table \
                 literal (table.import {csv} writes one), with at most a type step after it; another \
                 table is an error (edit its query instead). Returns {name, columns: [{name, type}], \
                 rows: [[text | null]] (texts as the model's query culture reads them), culture, \
                 shape: enter_data|table_literal}; table.enter_data {name, columns, rows, culture, \
                 replace: true} writes the edited grid back",
        run: |s, p| crate::get_data::table_entered_data(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::menu(Menu::ModelObject, "Edit data", "edit"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[P::req("name", K::Name(Obj::Table))],
    },
    Command {
        id: "table.import",
        label: "Add an import table with data (CSV, a Power Query (M) query or a DAX table expression), or give an existing table new data",
        params: "{name, csv?: string (a file path or CSV text), m?: string (a Power Query expression), dax?: \
                 string (a DAX table expression: literals, or SUMMARIZECOLUMNS/ADDCOLUMNS/FILTER/... over the \
                 model's data), culture?: string (CSV numbers and dates, e.g. fr-FR; default invariant), delimiter?: string, types?: {column: string|int64|double|decimal|\
                 dateTime|boolean} (CSV; others inferred), replace?: bool, hidden?: bool, description?, \
                 data_root?: string}: exactly one of csv/m/dax. CSV data is stored as an M #table literal \
                 partition (self-contained, refreshes in Desktop). `m` runs in bicraft's Power Query \
                 evaluator with the model's culture and the queries/parameters it names (literals, Enter \
                 data, common transforms; Csv.Document/Json.Document/Excel.Workbook over File.Contents and \
                 Folder.Files/Folder.Contents only under `data_root`, which is then kept for the session; \
                 database and web connectors are not run unless `allow_connectors` names them, e.g. \
                 [\"sqlserver\"], [\"web\"] or [\"odata\"]: then only the query's columns are read, Table.FirstN(query, 0) folded \
                 into the source, and model.refresh loads the rows) and becomes the partition as written; \
                 dax makes a calculated table. `replace` gives an existing table the new data \
                 (columns matched by name keep their properties; calculated columns and measures stay). On a \
                 PBIX, file.save writes the data into the DataModel",
        run: pbix_struct::table_import,
        server: ServerPolicy::paths(&[PathParam::path_or_text("csv"), PathParam::clearable("data_root")]),
        replay: Replay::Copy,
        place: Place::ribbon(Tab::Home, "Data", "Import table", "import"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("name", K::Text),
            P::opt("csv", K::PathOrText),
            P::opt("m", K::M),
            P::opt("dax", K::Dax),
            P::opt("culture", K::Text),
            P::opt("delimiter", K::Text),
            P::opt("types", K::Json),
            P::flag("replace", false),
            P::flag("hidden", false),
            P::opt("description", K::Text),
            P::opt("data_root", K::Path),
            P::opt("allow_connectors", K::List),
        ],
    },
    Command {
        id: "table.add_refresh_info",
        label: "Add a Refresh info table (when the data was last refreshed, for a card or a table visual)",
        params: "{name?: string (default \"Refresh info\"), measure?: bool (default true), measure_name?: \
                 string (default \"Last refresh\"), hidden?: bool (default false)}: an import table with one \
                 row from a Power Query (M) query, columns \"Last refresh (UTC)\" (DateTimeZone.UtcNow) and \
                 \"Last refresh (local)\" (DateTime.LocalNow; UTC in the Power BI service), evaluated at each \
                 refresh in Power BI Desktop, the service and model.refresh; with `measure`, a measure \
                 MAX('<name>'[Last refresh (UTC)]) formatted General Date. Nothing bicraft-specific: the \
                 file opens and refreshes the same way in Desktop. Returns {table, columns, m, measure?}",
        run: crate::refresh_table::add_refresh_info,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Model, "Calculations", "Refresh info table", "calendar"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::opt("name", K::Text).with_default("\"Refresh info\""),
            P::flag("measure", true),
            P::opt("measure_name", K::Text).with_default("\"Last refresh\""),
            P::flag("hidden", false),
        ],
    },
    Command {
        id: "parameter.what_if",
        label: "New what-if (numeric range) parameter: a table of values, a measure reading the selected one, and a slicer",
        params: "{name, min: number, max: number, increment: number (> 0), default?: number, data_type?: \
                 \"whole\"|\"decimal\"|\"fixed\" (default: whole when every number is whole), add_slicer?: \
                 bool (default: when the session has a report), page?: string (default: the active page)}: \
                 adds the calculated table <name> = GENERATESERIES(min, max, increment) with its column \
                 <name> (ParameterMetadata version 0), the measure [<name> Value] = SELECTEDVALUE('<name>'[<name>], \
                 default), and a single-value slicer of the column on the page, as one edit (one \
                 edit.undo step); at most 1,000,000 values. Returns {table, column, expression, data_type, \
                 measure: {name, expression}, slicer?: {page, visual, position}, warnings, diagnostics}",
        run: crate::parameters::what_if,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Model, "Parameters", "Numeric range", "calculator"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("name", K::Text),
            P::req("min", K::Number),
            P::req("max", K::Number),
            P::req("increment", K::Number),
            P::opt("default", K::Number),
            P::opt("data_type", K::Enum(crate::parameters::WHAT_IF_TYPES)),
            P::opt("add_slicer", K::Bool),
            P::opt("page", K::Name(Obj::Page)),
        ],
    },
    Command {
        id: "parameter.fields",
        label: "New field parameter: a table listing columns and measures, and a slicer that picks which ones visuals bound to it show",
        params: "{name, fields: [\"Table[Column]\" | \"[Measure]\" | {field, label?}] (in order; label \
                 defaults to the field's name), add_slicer?: bool (default: when the session has a report), \
                 page?: string (default: the active page)}: adds the calculated table <name> = \
                 {(\"Label\", NAMEOF('T'[c]), 0), …} with the columns <name> (the labels, grouped by the \
                 next), <name> Fields (hidden, ParameterMetadata kind 2) and <name> Order (hidden), and a \
                 slicer of <name> on the page, as one edit (one edit.undo step). A visual bound to \
                 '<name>'[<name>] shows the fields chosen in that slicer, all of them when none is chosen. \
                 Returns {table, columns, expression, fields: [{label, field}], slicer?: {page, visual, \
                 position}, warnings, diagnostics}",
        run: crate::parameters::fields,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Model, "Parameters", "Fields", "list"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("name", K::Text),
            P::req("fields", K::List),
            P::opt("add_slicer", K::Bool),
            P::opt("page", K::Name(Obj::Page)),
        ],
    },
    Command {
        id: "parameter.fix_field",
        label: "Point a field parameter's field at another column or measure, or remove it from the parameter",
        params: "{name (the field parameter's table), field (as its NAMEOF names it: 'T'[c] or \
                 [Measure]; it may be missing from the model), to?: field}: with `to`, that \
                 NAMEOF names `to` (the label and order stay); without, the field's row goes from \
                 the table constructor (refused for the only one). One edit (one edit.undo step); \
                 the new expression is validated. Returns {table, field, expression, change, \
                 warnings, diagnostics}",
        run: crate::report_fix::parameter_fix_field,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Model, "Parameters", "Fix field", "edit"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("name", K::Name(Obj::Table)),
            P::req("field", K::Text),
            P::opt("to", K::Field),
        ],
    },
    Command {
        id: "table.set_data",
        label: "Give an existing table new data (CSV, a Power Query (M) query or a DAX table expression)",
        params: "{name, csv?|m?|dax?, culture?, delimiter?, types?, data_root?}: table.import with replace on an existing \
                 table; on a PBIX, file.save replaces its stored data (its relationships are rebuilt)",
        run: pbix_struct::table_set_data,
        server: ServerPolicy::paths(&[PathParam::path_or_text("csv"), PathParam::clearable("data_root")]),
        replay: Replay::Copy,
        place: Place::ribbon(Tab::Table, "Data", "Replace data", "reload"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("name", K::Name(Obj::Table)),
            P::opt("csv", K::PathOrText),
            P::opt("m", K::M),
            P::opt("dax", K::Dax),
            P::opt("culture", K::Text),
            P::opt("delimiter", K::Text),
            P::opt("types", K::Json),
            P::opt("data_root", K::Path),
        ],
    },
    Command {
        id: "table.remove",
        label: "Remove a table with its relationships and row filters",
        params: "{name, force?: bool}: refused when DAX expressions (of other tables, row filters, report \
                 measures) use the table or its columns and measures, listing them; `force` removes it anyway \
                 and leaves them as they are (`used_by`). Returns the removed relationships and `broken`, the \
                 expressions that no longer validate. Column variations that point to the table (auto \
                 date/time) are removed first (`removed_variations`)",
        run: crate::model_props::table_remove,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::ModelObject, "Delete table", "delete"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[P::req("name", K::Name(Obj::Table)), P::flag("force", false)],
    },
    Command {
        id: "table.rename",
        label: "Rename a table (relationships, roles and every DAX expression follow)",
        params: "{name, new_name, update_dax?: bool (default true), update_report?: bool}: rewrites the \
                 references in measures, calculated columns and tables, row filters, KPIs, format string and \
                 detail rows expressions, calculation items and report measures (`dax_rewritten`; \
                 `'Old'[x]` and `Old[x]` alike, strings and comments untouched; M queries are not DAX and \
                 are left alone); returns `broken`, the expressions that no longer validate",
        run: edit::table_rename,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::ModelObject, "Rename table", "rename"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("name", K::Name(Obj::Table)),
            P::req("new_name", K::Text),
            P::flag("update_dax", true),
            P::flag("update_report", false),
        ],
    },
    Command {
        id: "table.set",
        label: "Change a table's description, visibility or data category (mark it as a date table)",
        params: "{name, description?: string|null, hidden?: bool, data_category?: string|null, key_column?: string|null}: data_category \"Time\" with key_column marks a date table (the column gets isKey); the column must be a date/time column whose values are unique, not blank, one per day without gaps and at one time of day (checked when the session has the data: `data_checked`); leaving \"Time\" clears the key",
        run: edit::table_set,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Table, "Properties", "Table properties", "settings"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("name", K::Name(Obj::Table)),
            P::opt("description", K::Text),
            P::opt("hidden", K::Bool),
            P::opt("data_category", K::Text),
            P::opt("key_column", K::Name(Obj::Column)),
        ],
    },
    Command {
        id: "model.set_properties",
        label: "Set the same properties on one or several tables, columns and measures (the Properties pane's multi-selection)",
        params: "{objects: [{table} | {table, column} | {table, measure}] (1 to 10000, each once), props: \
                 {description?: string|null, hidden?: bool, synonyms?: [string] (Q&A terms in the model \
                 culture's linguistic schema), display_folder?, format_string? (columns and measures), \
                 data_category?: string|null, summarize_by?, sort_by_column?: string|null (columns), \
                 storage_mode?: import|directQuery|dual (tables)}, force?: bool}: every object gets the \
                 same values through its own setter (table.set, column.set, measure.set, \
                 table.set_storage_mode), so their checks apply; a property that does not apply to one \
                 of the objects is refused. All or nothing, one undo step. Returns {objects, props, \
                 warnings}",
        run: crate::set_properties::set_properties,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Set properties of the selection", "settings"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("objects", K::Json),
            P::req("props", K::Json),
            P::flag("force", false),
        ],
    },
    Command {
        id: "table.set_storage_mode",
        label: "Set a table's storage mode: Import, DirectQuery or Dual (composite models)",
        params: "{table, mode: \"import\"|\"directQuery\"|\"dual\", dual_related?: bool (with import: the \
                 DirectQuery tables reached through the one side of its relationships become Dual, as \
                 Desktop's dialog offers)}: writes every partition's `mode`. Refused for a calculated \
                 table (always Import), a table with an incremental refresh policy, a non-M partition, \
                 a source with no connector (Enter data, #table: Import only), and an Import table \
                 whose rows a PBIX stores (Desktop changes only DirectQuery tables). Warns about the \
                 relationships that become limited (Import and DirectQuery tables are in different \
                 source groups) and Dual on a table on the many side. BIcraft evaluates imported data \
                 only: a DirectQuery table is \"not evaluated: DirectQuery\" in visuals. Returns {table, \
                 mode, previous_mode, set_to_dual, dual_candidates, limited_relationships}. One undo step",
        run: crate::storage_mode::table_set_storage_mode,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Table, "Properties", "Storage mode", "server"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("table", K::Name(Obj::Table)),
            P::req("mode", K::Enum(crate::storage_mode::MODES)),
            P::flag("dual_related", false),
        ],
    },
    Command {
        id: "table.set_refresh_policy",
        label: "Set a table's incremental refresh policy (archive and refresh periods, detect data changes, real time)",
        params: "{table, store: {n, unit: day|month|quarter|year} (Archive data starting n units before \
                 the refresh date), refresh: {n, unit} (Incrementally refresh data starting n units \
                 before the refresh date), detect_changes?: column (a date/time column whose maximum \
                 per period is polled), only_complete_periods?: bool, realtime?: bool (the latest data \
                 in real time with DirectQuery: a hybrid table; turns only_complete_periods on)}: writes \
                 the table's TMSL `refreshPolicy` (TMDL, TMSL, PBIX). Needs the RangeStart and RangeEnd \
                 Date/Time parameters (names case-sensitive), an Import table and a Power Query filter \
                 step on both; the error lists what is missing. Raises the compatibility level to 1550 \
                 (1565 for real time). The policy applies when the service refreshes the published model; \
                 the table keeps its partition here. One undo step",
        run: crate::refresh_policy::table_set_refresh_policy,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::ModelObject, "Incremental refresh", "calendar"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("table", K::Name(Obj::Table)),
            P::req("store", K::Json),
            P::req("refresh", K::Json),
            P::opt("detect_changes", K::Name(Obj::Column)),
            P::flag("only_complete_periods", false),
            P::flag("realtime", false),
        ],
    },
    Command {
        id: "table.remove_refresh_policy",
        label: "Remove a table's incremental refresh policy",
        params: "{table}: removes the table's `refreshPolicy`; partitions the service created from it are \
                 kept. One undo step",
        run: crate::refresh_policy::table_remove_refresh_policy,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Remove incremental refresh", "delete"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[P::req("table", K::Name(Obj::Table))],
    },
    Command {
        id: "partition.set",
        label: "Replace a partition's source (M or DAX) in place, keeping the table, columns, relationships and measures",
        params: "{table, partition? (name; default: the table's only partition), expression, kind?: \
                 \"m\"|\"calculated\" (default: the partition's), mode?: import|directQuery|dual, force?: \
                 bool, new_name?: string, update_report?: bool, update_dax?: bool}: M is checked without \
                 running it (m.validate: syntax errors and undefined names refuse unless force; \
                 connectors are fine), DAX is bound as a calculated table. The partition keeps its \
                 name. An M table's columns are not changed; a calculated table's new expression gives \
                 its columns, as Power BI Desktop re-derives them (evaluated over the session's data: a \
                 column of the same source or name keeps its properties, others are added, data \
                 columns the expression no longer gives are removed with their relationships; `columns: {added, \
                 removed, retyped}`; kept, with a note, when the expression cannot be evaluated). \
                 `new_name` also renames the table as table.rename does, in the same undo step \
                 (`renamed`). On a PBIX with data, file.save \
                 reloads the table with the new source, so a source bicraft cannot run (a connector) \
                 is refused: edit a PBIP instead. Returns previous_expression and m_diagnostics",
        run: crate::partition::partition_set,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Data, "Partitions", "Edit partition", "edit"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("table", K::Name(Obj::Table)),
            P::opt("partition", K::Name(Obj::Partition)),
            P::req("expression", K::Code),
            P::opt("kind", K::Enum(&["m", "calculated"])),
            P::opt("mode", K::Enum(MODES)),
            P::flag("force", false),
            P::opt("new_name", K::Text),
            P::flag("update_dax", true),
            P::flag("update_report", false),
        ],
    },
    Command {
        id: "table.set_partition",
        label: "Same as partition.set",
        params: "as partition.set",
        run: crate::partition::partition_set,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Edit partition (table.set_partition)", "edit"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("table", K::Name(Obj::Table)),
            P::opt("partition", K::Name(Obj::Partition)),
            P::req("expression", K::Code),
            P::opt("kind", K::Enum(&["m", "calculated"])),
            P::opt("mode", K::Enum(MODES)),
            P::flag("force", false),
            P::opt("new_name", K::Text),
            P::flag("update_dax", true),
            P::flag("update_report", false),
        ],
    },
    Command {
        id: "partition.add",
        label: "Add a partition to a table, checked like partition.set",
        params: "{table, name? (default <table>-<n>), expression, kind?: \"m\"|\"calculated\" (default: \
                 the kind of the table's partitions), mode?, force?}: a calculated table has one partition",
        run: crate::partition::partition_add,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Data, "Partitions", "New partition", "plus"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("table", K::Name(Obj::Table)),
            P::opt("name", K::Text),
            P::req("expression", K::Code),
            P::opt("kind", K::Enum(&["m", "calculated"])),
            P::opt("mode", K::Enum(MODES)),
            P::flag("force", false),
        ],
    },
    Command {
        id: "partition.remove",
        label: "Remove a partition of a table that has several",
        params: "{table, name}",
        run: crate::partition::partition_remove,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Data, "Partitions", "Delete partition", "delete"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("table", K::Name(Obj::Table)),
            P::req("name", K::Name(Obj::Partition)),
        ],
    },
    Command {
        id: "expression.add",
        label: "Add a shared M expression (a query or parameter that tables' queries can use)",
        params: "{name, expression, kind?: \"m\", description?, query_group?, force?}: checked like \
                 partition.set; names are unique across tables and shared expressions; gets Desktop's \
                 PBI_ResultType annotation when the text tells the result type. Not on a PBIX with data",
        run: crate::partition::expression_add,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Data, "Queries", "New query", "plus"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("name", K::Text),
            P::req("expression", K::M),
            P::opt("kind", K::Enum(&["m"])),
            P::opt("description", K::Text),
            P::opt("query_group", K::Text),
            P::flag("force", false),
        ],
    },
    Command {
        id: "expression.set",
        label: "Change a shared M expression's text, description or query group",
        params: "{name, expression?, description?: string|null, query_group?: string|null, force?}: on a \
                 PBIX with data, refused when a loaded table's query uses the expression",
        run: crate::partition::expression_set,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Data, "Queries", "Edit query", "edit"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("name", K::Name(Obj::Expression)),
            P::opt("expression", K::M),
            P::opt("description", K::Text),
            P::opt("query_group", K::Text),
            P::flag("force", false),
        ],
    },
    Command {
        id: "expression.rename",
        label: "Rename a shared M expression; the M queries that use it follow",
        params: "{name, new_name, update_m?: bool (default true)}: rewrites the references in other M \
                 queries (`m_rewritten`; `m_not_rewritten`: queries that do not parse). Not on a PBIX \
                 with data",
        run: crate::partition::expression_rename,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::ModelObject, "Rename query", "rename"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("name", K::Name(Obj::Expression)),
            P::req("new_name", K::Text),
            P::flag("update_m", true),
        ],
    },
    Command {
        id: "expression.remove",
        label: "Remove a shared M expression",
        params: "{name, force?: bool}: refused while M queries use it (listed), unless force (`used_by`). \
                 Not on a PBIX with data",
        run: crate::partition::expression_remove,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::ModelObject, "Delete query", "delete"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[P::req("name", K::Name(Obj::Expression)), P::flag("force", false)],
    },
    Command {
        id: "function.add",
        label: "Add a DAX user-defined function to the model",
        params: "{name, expression?: \"(params) => body\" | body?: string with parameters?: [{name, type?: \
                 AnyVal|Scalar|Table|AnyRef|CalendarRef|ColumnRef|MeasureRef|TableRef, subtype?: \
                 Variant|Int64|Decimal|Double|String|DateTime|Boolean|Numeric, mode?: val|expr, default?: \
                 DAX}], description?, hidden?: bool, force?}: names have letters, digits, underscores and \
                 periods (namespaces) and are not built-in functions; the definition is validated as \
                 `DEFINE FUNCTION name = …` against the model (`force` keeps invalid DAX); recursion is \
                 refused; raises the compatibility level to 1702. Returns {function: {name, expression, \
                 parameters: [{name, type, subtype, mode, default}], body}}",
        run: crate::udf::function_add,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::ModelObject, "New function", "calculator"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("name", K::Text),
            P::opt("expression", K::Dax),
            P::opt("parameters", K::Json),
            P::opt("body", K::Dax),
            P::opt("description", K::Text),
            P::flag("hidden", false),
            P::flag("force", false),
        ],
    },
    Command {
        id: "function.set",
        label: "Change a DAX user-defined function: its definition, parameters, body, description or hidden",
        params: "{name, expression? | parameters? and/or body? (the part left out is kept), description?: \
                 string|null, hidden?: bool, force?}: validated like function.add; the DAX that calls it \
                 and no longer validates is listed in `broken`",
        run: crate::udf::function_set,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::ModelObject, "Edit function", "edit"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("name", K::Text),
            P::opt("expression", K::Dax),
            P::opt("parameters", K::Json),
            P::opt("body", K::Dax),
            P::opt("description", K::Text),
            P::opt("hidden", K::Bool),
            P::flag("force", false),
        ],
    },
    Command {
        id: "function.rename",
        label: "Rename a DAX user-defined function; the calls in the model's DAX follow",
        params: "{name, new_name}: rewrites `name(` calls in measures, columns, tables, row filters, \
                 calculation items and other functions (`calls_rewritten`)",
        run: crate::udf::function_rename,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::ModelObject, "Rename function", "rename"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[P::req("name", K::Text), P::req("new_name", K::Text)],
    },
    Command {
        id: "function.remove",
        label: "Remove a DAX user-defined function",
        params: "{name, force?: bool}: refused while DAX calls it (listed), unless force (`used_by`)",
        run: crate::udf::function_remove,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::ModelObject, "Delete function", "delete"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[P::req("name", K::Text), P::flag("force", false)],
    },
    Command {
        id: "tmdl.script",
        label: "Script model objects as a TMDL createOrReplace script (the TMDL view's Script TMDL)",
        params: "{objects?: [{kind: model|table|column|measure|hierarchy|partition|function|expression|\
                 relationship|role|perspective|culture, name, table? (column, measure, hierarchy, \
                 partition)}] (default: the whole model)}: returns {script, count} (count: the \
                 objects scripted); a table's members are scripted under `ref table`. Read-only",
        run: |s, p| crate::tmdl_script::tmdl_script(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::palette("Script TMDL", "file"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[P::opt("objects", K::Json)],
    },
    Command {
        id: "tmdl.apply",
        label: "Apply a TMDL script to the model (createOrReplace), in one undo step",
        params: "{script: string, force?: bool}: the script starts with `createOrReplace`; each object it \
                 declares replaces the model's object of that kind and name with all its descendants, \
                 or is added; members under `ref table T` replace or add T's columns, measures, \
                 hierarchies and partitions. A script that does not parse, an unknown `ref` table, or DAX \
                 that does not validate (unless force) changes nothing: the error lists `line L, column \
                 C: message` positions in the script. Returns {applied: [{kind, name, table?, action: \
                 added|replaced}], warnings, diagnostics}",
        run: crate::tmdl_script::tmdl_apply,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Apply TMDL script", "play"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[P::req("script", K::Code), P::flag("force", false)],
    },
    Command {
        id: "sources.list",
        label: "List the data sources of the model's Power Query (M): files, folders, databases, Databricks, web, with their literal arguments",
        params: "{}: every call of a source function (File.Contents, Folder.Files, Excel.Workbook, \
                 Csv.Document, Sql.Database, Databricks.Catalogs, Web.Contents, SharePoint.Files, \
                 OData.Feed, PostgreSQL.Database, …) in M partitions and shared expressions, found \
                 without running the M. Returns {sources: [{object: {kind: partition|expression, table?, \
                 partition?, name?}, function, format? (the Excel.Workbook/Csv.Document/… reading it), \
                 span: {start, end} (bytes of the M text), line, column, arguments: [{role: \
                 path|folder|server|database|host|http_path|url|account|warehouse|connection_string|content, \
                 form: literal|step_literal|parameter|query|step|name|concat|call|computed, value?, \
                 name?, parts?, span, line, column}]}], parameters: [{name, type, value, used_as, \
                 allowed?}], summary, unparsed: [{object, error}]}. Read-only (docs/sources-env.md)",
        run: |s, p| crate::sources::sources_list(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Data, "Sources", "Data sources", "server"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[],
    },
    Command {
        id: "sources.rebase",
        label: "Rewrite the model's source paths from one root folder to another (optionally relative to a text parameter), and map servers, databases, Databricks hosts and URLs",
        params: "{from?: string, to?: string, param?: string, servers?: {old: new}, databases?: {old: \
                 new}, hosts?: {old: new}, http_paths?: {old: new}, urls?: {old prefix: new prefix}, \
                 dry_run?: bool}: paths under `from` (whole folder names; Windows drive and UNC paths \
                 without regard to case, `\\` and `/` alike; POSIX paths exactly) move under `to`. With \
                 `param`, they become `param & \"rest\"` and the text parameter `param` is set to `to` \
                 (default `from`) with a trailing separator, added when missing (IsParameterQuery, \
                 Type Text). Servers, databases and hosts match exactly (case-insensitive), URLs by \
                 prefix. Only literals are rewritten: a literal argument, a step bound to a literal, \
                 the leading literal of `&`, a text parameter's value (paths: anywhere; servers and \
                 the rest: where the sources use it); computed arguments are listed in `unchanged`, \
                 never guessed. The M text outside the rewritten literals stays byte for byte. One \
                 edit (one edit.undo step); `dry_run` returns the same result without changing \
                 anything. Returns {dry_run, changed, changes: [{object, function, role, form, before, \
                 after, span, m_before, m_after, relative_to?}], parameters: [{name, before, after, \
                 m_before, m_after}], added_parameter, unchanged: [{object, function, role, reason}], \
                 warnings}. Refused on a PBIX that stores data (dry_run works)",
        run: crate::sources::sources_rebase,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Data, "Sources", "Change sources", "link"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::opt("from", K::Text),
            P::opt("to", K::Text),
            P::opt("param", K::Text),
            P::opt("servers", K::Json),
            P::opt("databases", K::Json),
            P::opt("hosts", K::Json),
            P::opt("http_paths", K::Json),
            P::opt("urls", K::Json),
            P::flag("dry_run", false),
        ],
    },
    Command {
        id: "env.apply",
        label: "Apply an environment profile of bicraft.env.yaml: parameter values, path roots, server/database/host/URL mappings, in one edit",
        params: "{profile: string, env_file?: string|object (a profile file .yaml/.yml/.json, or the \
                 profiles inline; default bicraft.env.yaml, .yml or .json next to the project), \
                 dry_run?: bool}: the profile's `parameters` are typed against each parameter's Type \
                 (Text, Number, Logical, Date YYYY-MM-DD, DateTime, Time, Any) and its list of allowed \
                 values; `paths`, `servers`, `databases`, `hosts`, `http_paths`, `urls` rewrite \
                 literals as sources.rebase does. An unknown profile, a name that is not a \
                 parameter or a value of the wrong type refuses the whole profile. One edit; \
                 `dry_run` previews. Returns sources.rebase's result plus {profile, env_file} \
                 (docs/sources-env.md)",
        run: crate::envprofile::env_apply,
        server: ServerPolicy::paths(&[PathParam::many("env_file")]),
        replay: Replay::Copy,
        place: Place::ribbon(Tab::Data, "Environments", "Apply environment", "server"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("profile", K::Name(Obj::Profile)),
            P::opt("env_file", K::Paths),
            P::flag("dry_run", false),
        ],
    },
    Command {
        id: "env.list",
        label: "List the environment profiles of the project's profile file",
        params: "{env_file?: string|object (as for env.apply)}: returns {env_file, count, profiles:                  [{name, description, parameters (how many it sets)}]}. Read-only",
        run: |s, p| crate::envprofile::env_list(s, p),
        server: ServerPolicy::paths(&[PathParam::many("env_file")]).read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Data, "Environments", "Environment profiles", "list"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[P::opt("env_file", K::Paths)],
    },
    Command {
        id: "env.check",
        label: "Check an environment profile against the model: which sources and parameters it does not cover, which values differ",
        params: "{profile: string, env_file?: string|object (as for env.apply)}: returns {profile, \
                 env_file, ok (no error and every literal source and every parameter the sources \
                 use is covered), errors, parameters: [{name, type, current, target?, status: \
                 same|differs|covered|not_covered, used_as}], sources: [{object, function, role, \
                 status: same|differs|covered|not_covered|computed, before?, after?, value?, \
                 reason?}], summary, warnings}. Read-only",
        run: |s, p| crate::envprofile::env_check(s, p),
        server: ServerPolicy::paths(&[PathParam::many("env_file")]).read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Data, "Environments", "Check environment", "check"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("profile", K::Name(Obj::Profile)),
            P::opt("env_file", K::Paths),
        ],
    },
    Command {
        id: "column.add",
        label: "Add a data column or a calculated column",
        params: "{table, name, data_type: string|int64|double|decimal|dateTime|boolean|binary|variant \
                 (optional for a calculated column: its expression's type, text when it depends on the \
                 rows), source_column? (data column, default name), expression? (calculated column, validated \
                 as DAX), format_string?, display_folder?, summarize_by?: none|sum|min|max|count|average|\
                 distinctCount|default, hidden?: bool, description?, force?: bool, plus the column.set \
                 properties sort_by_column?, is_available_in_mdx?, is_key?, data_category?}",
        run: crate::model_props::column_add,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Table, "Columns", "New column", "column"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("table", K::Name(Obj::Table)),
            P::req("name", K::Text),
            P::req("data_type", K::Enum(DATA_TYPES)),
            P::opt("source_column", K::Text),
            P::opt("expression", K::Dax),
            P::opt("format_string", K::Text),
            P::opt("display_folder", K::Text),
            P::opt("summarize_by", K::Enum(SUMMARIZE_BY)),
            P::flag("hidden", false),
            P::opt("description", K::Text),
            P::flag("force", false),
            P::opt("sort_by_column", K::Name(Obj::Column)),
            P::flag("is_available_in_mdx", true),
            P::flag("is_key", false),
            P::opt("data_category", K::Text),
        ],
    },
    Command {
        id: "column.group",
        label: "New group or bins on a column: a calculated column of named groups of its values, or of bins",
        params: "{table, column, name? (default \"<column> (groups)\" or \"<column> (bins)\"), kind?: \
                 \"list\"|\"bin\" (default: bin for a number or date column without groups), groups?: \
                 [{name, values: [value | null (blank)]}] (list: values of the column's type, dates as \
                 YYYY-MM-DD), other?: bool (list, default true: the values no group lists go to \"Other\", \
                 else keep their own value; a blank no group lists is \"(Blank)\"), bin_size?: number (> 0; \
                 a date's whole number of units, default 1), bin_count?: integer (numbers: that many equal \
                 bins between the column's minimum and maximum, computed by the DAX), unit?: \
                 \"number\"|\"year\"|\"month\"|\"day\"|\"hour\"|\"minute\"|\"second\" (default: number, year \
                 for a date), replace?: bool (rewrite the group column `name`)}: adds the calculated column \
                 with its SWITCH (list) or bin DAX in Desktop's form, validated, the GroupingMetadata \
                 extended property naming the grouped column and the GroupingDesignState annotation \
                 Desktop's Edit groups reads (none for a list group listing a blank), as one edit. Returns \
                 {table, column, grouped_column, kind, expression, data_type, other?, bin_size?, \
                 bin_count?, unit?, warnings, diagnostics}",
        run: crate::groups::group,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Table, "Groups", "Data groups", "group"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("table", K::Name(Obj::Table)),
            P::req("column", K::Name(Obj::Column)),
            P::opt("name", K::Text),
            P::opt("kind", K::Enum(crate::groups::KINDS)),
            P::opt("groups", K::List),
            P::opt("other", K::Bool),
            P::opt("bin_size", K::Number),
            P::opt("bin_count", K::Number),
            P::opt("unit", K::Enum(crate::groups::UNITS)),
            P::opt("replace", K::Bool),
        ],
    },
    Command {
        id: "column.set",
        label: "Change or rename a column (a rename rewrites the DAX that uses it)",
        params: "{table, name, new_name?, data_type?, source_column? (data columns), expression? \
                 (calculated columns), format_string?, display_folder?, summarize_by?, description? \
                 (string, or null to clear), hidden?: bool, force?: bool, update_dax?: bool (default true), \
                 update_report?: bool}: a rename rewrites every DAX reference to the column \
                 (`dax_rewritten`) and returns `broken`. sort_by_column?: \"Column\"|null (same table; \
                 refused when it has more than one value per value of the column, checked on the data \
                 when there is data, else a warning; no cycles; force applies anyway), \
                 is_available_in_mdx?: bool (false refused while the column is a sort-by column or a \
                 hierarchy level, as the engine does), is_key?: bool (one \
                 per table; unique non-blank values when there is data), data_category?: \
                 Address|Place|City|County|StateOrProvince|PostalCode|Country|Continent|Latitude|Longitude|\
                 WebUrl|ImageUrl|Barcode|null (others kept with a warning)",
        run: crate::model_props::column_set,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Table, "Columns", "Column properties", "settings"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("table", K::Name(Obj::Table)),
            P::req("name", K::Name(Obj::Column)),
            P::opt("new_name", K::Text),
            P::opt("data_type", K::Enum(DATA_TYPES)),
            P::opt("source_column", K::Text),
            P::opt("expression", K::Dax),
            P::opt("format_string", K::Text),
            P::opt("display_folder", K::Text),
            P::opt("summarize_by", K::Enum(SUMMARIZE_BY)),
            P::opt("description", K::Text),
            P::opt("hidden", K::Bool),
            P::flag("force", false),
            P::flag("update_dax", true),
            P::flag("update_report", false),
            P::opt("sort_by_column", K::Name(Obj::Column)),
            P::opt("is_available_in_mdx", K::Bool),
            P::opt("is_key", K::Bool),
            P::opt("data_category", K::Text),
        ],
    },
    Command {
        id: "column.rename",
        label: "Rename a column; every DAX expression that uses it follows",
        params: "{table, name, new_name, update_dax?: bool (default true), update_report?: bool}: column.set \
                 with only new_name; returns `dax_rewritten` and `broken`",
        run: edit::column_rename,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::ModelObject, "Rename column", "rename"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("table", K::Name(Obj::Table)),
            P::req("name", K::Name(Obj::Column)),
            P::req("new_name", K::Text),
            P::flag("update_dax", true),
            P::flag("update_report", false),
        ],
    },
    Command {
        id: "column.remove",
        label: "Remove a column with the relationships that use it",
        params: "{table, name, force?: bool}: refused when DAX expressions use the column, listing them; \
                 `force` removes it anyway (`used_by`). Returns the removed relationships and `broken` \
                 expressions. A calculated table's column is removed by rewriting the table's \
                 expression to select the other columns (`expression_rewritten`). Auto date tables only \
                 its variations used go too (`removed_tables`)",
        run: crate::model_props::column_remove,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::ModelObject, "Delete column", "delete"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("table", K::Name(Obj::Table)),
            P::req("name", K::Name(Obj::Column)),
            P::flag("force", false),
        ],
    },
    Command {
        id: "measure.add",
        label: "Add a measure",
        params: "{table, name, expression (validated as DAX against the model), format_string?, \
                 display_folder?, description?, hidden?: bool, format_string_definition?: string (dynamic \
                 format string, validated as a scalar DAX expression), force?: bool}",
        run: crate::model_props::measure_add,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Home, "Calculations", "New measure", "measure"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("table", K::Name(Obj::Table)),
            P::req("name", K::Text),
            P::req("expression", K::Dax),
            P::opt("format_string", K::Text),
            P::opt("display_folder", K::Text),
            P::opt("description", K::Text),
            P::flag("hidden", false),
            P::opt("format_string_definition", K::Dax),
            P::flag("force", false),
        ],
    },
    Command {
        id: "measure.quick",
        label: "New quick measure: an ordinary measure written from a common calculation's DAX template",
        params: "{kind: average_per_category|variance_per_category|max_per_category|min_per_category|\
                 weighted_average_per_category|filtered_value|difference_from_filtered_value|\
                 percentage_difference_from_filtered_value|sales_from_new_customers|year_to_date|\
                 quarter_to_date|month_to_date|year_over_year_change|quarter_over_quarter_change|\
                 month_over_month_change|rolling_average|running_total|total_for_category_filtered|\
                 total_for_category_unfiltered|addition|subtraction|multiplication|division|\
                 percentage_difference|correlation_coefficient|star_rating|concatenated_list, params: \
                 {base (a measure `[M]`, a column `T[C]` summed, or `Average(T[C])`…), category?, \
                 weight?, filter? and values? (a column and its values), customer? (a column), date? (a \
                 date column; time intelligence uses its auto date/time table's dates `T[C].[Date]` \
                 when it has one), year_end_month?: 1-12 (year_to_date's fiscal year end, default 12), \
                 period?: day|month|quarter|year, before?, after?, field? (a column), descending?, \
                 second?, max_value?, min_value?, stars?, max_values?, separator?}, table? (default: \
                 the base's table), name? (default from the calculation, made unique)}: adds the \
                 measure, validated as DAX, as one edit (docs/formats/quick-measures.md lists each \
                 calculation's parameters and template). Returns {table, measure, kind, expression, \
                 format_string, warnings (incl. a date column that is neither a marked date table's \
                 nor auto date/time), diagnostics}",
        run: crate::quick::quick,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Home, "Calculations", "Quick measure", "calculator"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("kind", K::Enum(crate::quick::KINDS)),
            P::req("params", K::Json),
            P::opt("table", K::Name(Obj::Table)),
            P::opt("name", K::Text),
        ],
    },
    Command {
        id: "measure.set",
        label: "Change or rename a measure (a rename rewrites the DAX that uses it)",
        params: "{name, table? (checked when given), new_name?, expression?, format_string?, \
                 display_folder?, description? (string, or null to clear), hidden?: bool, force?: bool, \
                 update_dax?: bool (default true), update_report?: bool}: a rename rewrites every DAX \
                 reference to the measure (`dax_rewritten`) and returns `broken`. \
                 format_string_definition?: string|null sets the dynamic format string (a scalar DAX \
                 expression, validated) or removes it. The engine refuses a measure with both, so the \
                 static format_string is cleared, as in TOM, and removing it leaves no format string \
                 unless format_string is passed in the same call (format_string alone on a measure \
                 with a dynamic one is refused)",
        run: crate::model_props::measure_set,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Measure, "Properties", "Measure properties", "settings"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("name", K::Name(Obj::Measure)),
            P::opt("table", K::Name(Obj::Table)),
            P::opt("new_name", K::Text),
            P::opt("expression", K::Dax),
            P::opt("format_string", K::Text),
            P::opt("display_folder", K::Text),
            P::opt("description", K::Text),
            P::opt("hidden", K::Bool),
            P::flag("force", false),
            P::flag("update_dax", true),
            P::flag("update_report", false),
            P::opt("format_string_definition", K::Dax),
        ],
    },
    Command {
        id: "measure.rename",
        label: "Rename a measure; every DAX expression that uses it follows",
        params: "{name, new_name, table? (checked when given), update_dax?: bool (default true), \
                 update_report?: bool}: measure.set with only new_name; returns `dax_rewritten` and `broken`",
        run: edit::measure_rename,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::ModelObject, "Rename measure", "rename"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("name", K::Name(Obj::Measure)),
            P::req("new_name", K::Text),
            P::opt("table", K::Name(Obj::Table)),
            P::flag("update_dax", true),
            P::flag("update_report", false),
        ],
    },
    Command {
        id: "measure.remove",
        label: "Remove a measure",
        params: "{name, force?: bool}: refused when DAX expressions use the measure, listing them; `force` \
                 removes it anyway (`used_by`). Returns `broken`, the expressions that no longer validate",
        run: edit::measure_remove,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::ModelObject, "Delete measure", "delete"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[P::req("name", K::Name(Obj::Measure)), P::flag("force", false)],
    },
    Command {
        id: "measure.move",
        label: "Move a measure to another table",
        params: "{name, to_table, force?: bool}",
        run: edit::measure_move,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Measure, "Structure", "Home table", "table"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("name", K::Name(Obj::Measure)),
            P::req("to_table", K::Name(Obj::Table)),
            P::flag("force", false),
            P::flag("update_dax", true),
            P::flag("update_report", false),
        ],
    },
    Command {
        id: "hierarchy.add",
        label: "Add a user hierarchy (drill-down levels) to a table",
        params: "{table, name, levels: [column name | {name?, column}] (top level first), hidden?: bool, description?, display_folder?}: every level is a column of the table with an attribute hierarchy; on a PBIX, file.save writes the hierarchy's node table",
        run: crate::hierarchy::hierarchy_add,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Table, "Hierarchies", "New hierarchy", "hierarchy"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("table", K::Name(Obj::Table)),
            P::req("name", K::Text),
            P::req("levels", K::List),
            P::flag("hidden", false),
            P::opt("description", K::Text),
            P::opt("display_folder", K::Text),
        ],
    },
    Command {
        id: "hierarchy.remove",
        label: "Remove a user hierarchy",
        params: "{table, name}: its perspective memberships go with it",
        run: crate::hierarchy::hierarchy_remove,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::ModelObject, "Delete hierarchy", "delete"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("table", K::Name(Obj::Table)),
            P::req("name", K::Name(Obj::Hierarchy)),
        ],
    },
    Command {
        id: "hierarchy.set",
        label: "Change a user hierarchy: rename it or its levels, reorder or replace its levels, hide it",
        params: "{table, name, new_name?, levels?: [column name | {name?, column}] (the new level layout, top level first; kept columns keep their level's properties), rename_levels?: {old: new}, hidden?: bool, description?: string|null, display_folder?: string|null}: a rename follows perspectives, translations and date variations (`metadata_updated`); on a PBIX, file.save rebuilds a renamed or restructured hierarchy's node table",
        run: crate::hierarchy_set::hierarchy_set,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Table, "Hierarchies", "Edit hierarchy", "edit"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("table", K::Name(Obj::Table)),
            P::req("name", K::Name(Obj::Hierarchy)),
            P::opt("new_name", K::Text),
            P::opt("levels", K::List),
            P::opt("rename_levels", K::Json),
            P::opt("hidden", K::Bool),
            P::opt("description", K::Text),
            P::opt("display_folder", K::Text),
        ],
    },
    Command {
        id: "calculation_group.add",
        label: "Add a calculation group (a table whose rows are calculation items)",
        params: "{name, precedence? (default: one more than the highest), column? (the item column, default Name), description?, hidden?: bool}: creates the table with its item column, a hidden Ordinal column and a calculationGroup partition; sets the model's discourageImplicitMeasures, as Power BI Desktop does (calculation groups require it)",
        run: crate::calc_group::calculation_group_add,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Model, "Calculation groups", "New calculation group", "plus"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("name", K::Text),
            P::opt("precedence", K::Integer),
            P::opt("column", K::Text),
            P::opt("description", K::Text),
            P::flag("hidden", false),
        ],
    },
    Command {
        id: "calculation_group.set",
        label: "Change a calculation group's precedence, description or selection expressions",
        params: "{name, precedence?, description?: string|null, multiple_or_empty_selection_expression?: string|null, no_selection_expression?: string|null, force?: bool}: expressions are validated as DAX",
        run: crate::calc_group::calculation_group_set,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(
            Tab::Model,
            "Calculation groups",
            "Calculation group properties",
            "settings",
        ),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("name", K::Name(Obj::CalculationGroup)),
            P::opt("precedence", K::Integer),
            P::opt("description", K::Text),
            P::opt("multiple_or_empty_selection_expression", K::Dax),
            P::opt("no_selection_expression", K::Dax),
            P::flag("force", false),
        ],
    },
    Command {
        id: "calculation_group.remove",
        label: "Remove a calculation group with its items",
        params: "{name, force?: bool}: table.remove on a calculation group table",
        run: crate::calc_group::calculation_group_remove,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::ModelObject, "Delete calculation group", "delete"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("name", K::Name(Obj::CalculationGroup)),
            P::flag("force", false),
        ],
    },
    Command {
        id: "calculation_item.add",
        label: "Add a calculation item to a calculation group",
        params: "{group, name, expression (DAX, e.g. CALCULATE(SELECTEDMEASURE(), DATESYTD('Date'[Date]))), ordinal? (position; later items move down; default last), format_string_expression?, description?, force?: bool}: expressions are validated as DAX against the model",
        run: crate::calc_group::calculation_item_add,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Model, "Calculation groups", "New calculation item", "plus"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("group", K::Name(Obj::CalculationGroup)),
            P::req("name", K::Text),
            P::req("expression", K::Dax),
            P::opt("ordinal", K::Integer),
            P::opt("format_string_expression", K::Dax),
            P::opt("description", K::Text),
            P::flag("force", false),
        ],
    },
    Command {
        id: "calculation_item.set",
        label: "Change, rename or move a calculation item",
        params: "{group, name, new_name?, expression?, ordinal? (moves the item; ordinals become 0..n), format_string_expression?: string|null, description?: string|null, force?: bool}",
        run: crate::calc_group::calculation_item_set,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Model, "Calculation groups", "Edit calculation item", "edit"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("group", K::Name(Obj::CalculationGroup)),
            P::req("name", K::Name(Obj::CalculationItem)),
            P::opt("new_name", K::Text),
            P::opt("expression", K::Dax),
            P::opt("ordinal", K::Integer),
            P::opt("format_string_expression", K::Dax),
            P::opt("description", K::Text),
            P::flag("force", false),
        ],
    },
    Command {
        id: "calculation_item.remove",
        label: "Remove a calculation item",
        params: "{group, name}",
        run: crate::calc_group::calculation_item_remove,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::ModelObject, "Delete calculation item", "delete"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("group", K::Name(Obj::CalculationGroup)),
            P::req("name", K::Name(Obj::CalculationItem)),
        ],
    },
    Command {
        id: "relationship.add",
        label: "Add a relationship between two columns",
        params: "{from: \"Table[Column]\" (many side), to: \"Table[Column]\" (one side), active?: bool \
                 (default: active when it can be, else added inactive with a `made_inactive` note: \
                 another relationship is active between the two tables, or filter paths would become \
                 ambiguous; true then refuses), cross_filter?: \"single\"|\"both\", from_cardinality?: \
                 \"one\"|\"many\", to_cardinality?: \"one\"|\"many\", security_filtering?: \
                 \"single\"|\"both\"|\"none\", assume_referential_integrity?: bool (DirectQuery or Dual \
                 tables, not many-to-many), name?: string (unique; default a new GUID, as Desktop names \
                 them), check?: bool (default false: true changes nothing and answers the Create \
                 relationship dialog's review: {check, valid, errors: [{code, message}], warnings: \
                 [{code, message}], from, to, from_cardinality, to_cardinality, cross_filter, active, \
                 can_be_active, from_unique, to_unique (null without data), suggested: {cardinality: \
                 many_to_one|one_to_one|one_to_many|many_to_many, cross_filter} | null, data_checked})}: \
                 returns the name. Refused (codes): same_table, duplicate, one_to_many, \
                 security_needs_both, not_unique (a \"one\" side with duplicate or blank values in the \
                 session's data), no_unique_side (neither column unique, not many-to-many), \
                 referential_integrity, active_exists, ambiguous (with active: true); warnings: \
                 many_to_many, data_types, made_inactive, not_checked, too_many_paths. One-to-one cross \
                 filters in both directions (the default for it; \"single\" is refused: Desktop does not \
                 open such a file)",
        run: crate::relationship_review::relationship_add,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Model, "Relationships", "New relationship", "relationship"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("from", K::Field),
            P::req("to", K::Field),
            P::opt("active", K::Bool),
            P::opt("cross_filter", K::Enum(&["single", "both"])),
            P::opt("from_cardinality", K::Enum(CARDINALITY)),
            P::opt("to_cardinality", K::Enum(CARDINALITY)),
            P::opt("security_filtering", K::Enum(&["single", "both", "none"])),
            P::opt("assume_referential_integrity", K::Bool),
            P::opt("name", K::Text),
            P::flag("check", false),
        ],
    },
    Command {
        id: "relationship.remove",
        label: "Remove a relationship",
        params: "{name} or {from: \"Table[Column]\", to: \"Table[Column]\"}",
        run: edit::relationship_remove,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::ModelObject, "Delete relationship", "delete"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::opt("name", K::Name(Obj::Relationship)),
            P::opt("from", K::Field),
            P::opt("to", K::Field),
        ],
    },
    Command {
        id: "relationship.set",
        label: "Change a relationship in place: name, columns, active, cross filter, cardinality, security filter, referential integrity",
        params: "{name} or {from: \"Table[Column]\", to: \"Table[Column]\"}, then any of new_name?: string \
                 (unique; renames it in place, column variations that name it follow), new_from?: \
                 \"Table[Column]\", new_to?: \"Table[Column]\" (other columns or tables for its ends), \
                 active?: bool, cross_filter?: \"single\"|\"both\", from_cardinality?: \"one\"|\"many\", \
                 to_cardinality?: \"one\"|\"many\", security_filtering?: \"single\"|\"both\"|\"none\", \
                 assume_referential_integrity?: bool, check?: bool (true changes nothing and answers the \
                 Edit relationship dialog's review, as relationship.add's check): refused when another \
                 relationship is active between the two tables, when filter paths become ambiguous, for \
                 one-to-many (swap the ends), for a one-to-one with cross_filter \"single\" (a one-to-one \
                 left single-direction is made both, with a note) and when an end that becomes a \"one\" \
                 side (or a \"one\" end moved to another column) has duplicate or blank values (checked \
                 when the session has the data: `data_checked`); the codes are relationship.add's. On a \
                 PBIX, a cardinality or an end change rebuilds the relationship's storage under the same \
                 name; a rename renames its row",
        run: crate::relationship_set::relationship_set,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Model, "Relationships", "Edit relationship", "edit"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::opt("name", K::Name(Obj::Relationship)),
            P::opt("from", K::Field),
            P::opt("to", K::Field),
            P::opt("new_name", K::Text),
            P::opt("new_from", K::Field),
            P::opt("new_to", K::Field),
            P::opt("active", K::Bool),
            P::opt("cross_filter", K::Enum(&["single", "both"])),
            P::opt("from_cardinality", K::Enum(CARDINALITY)),
            P::opt("to_cardinality", K::Enum(CARDINALITY)),
            P::opt("security_filtering", K::Enum(&["single", "both", "none"])),
            P::opt("assume_referential_integrity", K::Bool),
            P::flag("check", false),
        ],
    },
    Command {
        id: "relationship.autodetect",
        label: "Find relationships from matching column names and the data, then add them",
        params: "{apply?: bool (default false: only answer the candidates), only?: [{from: \"Table[Column]\", \
                 to: \"Table[Column]\"}] (with apply: the candidates to add; default all)}: candidates are \
                 two columns of two tables not related yet with the same name (ignoring case) and data \
                 type, where the session's data shows a unique, non-blank side (the \"one\" side; both \
                 unique: one-to-one, both directions; else many-to-one, single direction), one per pair \
                 of tables, none that makes filter paths ambiguous. Returns {candidates: [{from, to, \
                 from_cardinality, to_cardinality, cross_filter, active}], skipped: [{from, to, code (no_data, no_unique_side, ambiguous, \
                 unreadable, too_many_paths), reason}], \
                 applied: false}, or with apply {added: [... with name], skipped, applied: true}: one undo \
                 step, refused when nothing is found or `only` names a pair that is not a candidate",
        run: crate::relationship_autodetect::relationship_autodetect,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::ModelObject, "Autodetect relationships", "relationship"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[P::flag("apply", false), P::opt("only", K::List)],
    },
    Command {
        id: "diagram.move",
        label: "Move a table's card in the model diagram",
        params: "{table, x: number, y: number} or {tables: [{table, x, y}]}, then diagram?: string (a \
                 layout tab's name; default the selected one): sets each card's top-left corner in the model \
                 diagram (PBIX `DiagramLayout`, PBIP `diagramLayout.json`), adding the table's node when the \
                 diagram has none; file.save writes the layout back. Returns {moved, diagram}. One undo \
                 step. The Model view's drag of a card runs it (with the cards the file had no place for)",
        run: crate::diagram_layout::diagram_move,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Move table in diagram", "model"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("table", K::Name(Obj::Table)),
            P::req("x", K::Number),
            P::req("y", K::Number),
            P::opt("diagram", K::Text),
        ],
    },
    Command {
        id: "diagram.add",
        label: "Add a layout tab to the model diagram",
        params: "{name: string (unique, ignoring case), tables?: [table names] (placed in rows of four; \
                 default none)}: a new diagram after the others in the model diagram (PBIX `DiagramLayout`, \
                 PBIP `diagramLayout.json`), made the selected one; a layout other than the one of all \
                 tables shows only its tables (diagram.move adds one). Returns {diagram, tables}. One undo \
                 step",
        run: crate::diagram_layout::diagram_add,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Model, "Layouts", "New layout", "plus"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[P::req("name", K::Text), P::opt("tables", K::List)],
    },
    Command {
        id: "diagram.rename",
        label: "Rename a layout tab of the model diagram",
        params: "{name, new_name (unique, ignoring case)}: selectedDiagram and defaultDiagram follow. One \
                 undo step",
        run: crate::diagram_layout::diagram_rename,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::ModelObject, "Rename layout", "edit"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[P::req("name", K::Text), P::req("new_name", K::Text)],
    },
    Command {
        id: "diagram.remove",
        label: "Delete a layout tab of the model diagram",
        params: "{name}: refused for the layout of all tables (ordinal 0, else the first); selectedDiagram \
                 and defaultDiagram that named it go to that one. One undo step",
        run: crate::diagram_layout::diagram_remove,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::ModelObject, "Delete layout", "delete"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[P::req("name", K::Text)],
    },
    Command {
        id: "diagram.remove_tables",
        label: "Remove tables from a layout tab of the model diagram (the tables stay in the model)",
        params: "{tables: [table names], diagram?: string (a layout tab's name; default the selected \
                 one)}: takes the tables' cards out of that layout; refused for the layout of all tables \
                 (ordinal 0, else the first), which shows every table. Tables the layout does not show \
                 are skipped. Returns {removed, diagram}. One undo step",
        run: crate::diagram_layout::diagram_remove_tables,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::ModelObject, "Remove from layout", "delete"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[P::req("tables", K::List), P::opt("diagram", K::Text)],
    },
    Command {
        id: "role.add",
        label: "Add a security role",
        params: "{name, model_permission?: none|read|readRefresh|refresh|administrator (default read)}",
        run: edit::role_add,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Model, "Security", "New role", "lock"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("name", K::Text),
            P::opt(
                "model_permission",
                K::Enum(&["none", "read", "readRefresh", "refresh", "administrator"]),
            ),
        ],
    },
    Command {
        id: "role.set_filter",
        label: "Set or clear a role's row filter on a table",
        params: "{role, table, filter_expression: string (validated as DAX), or null to clear, force?: bool}",
        run: edit::role_set_filter,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Model, "Security", "Role filter", "filter"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("role", K::Name(Obj::Role)),
            P::req("table", K::Name(Obj::Table)),
            P::req("filter_expression", K::Dax),
            P::flag("force", false),
        ],
    },
    Command {
        id: "role.remove",
        label: "Remove a security role",
        params: "{name}",
        run: edit::role_remove,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::ModelObject, "Delete role", "delete"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[P::req("name", K::Name(Obj::Role))],
    },
    Command {
        id: "role.rename",
        label: "Rename a security role",
        params: "{name, new_name}: unique, case ignored, no comma (as Desktop's Manage security roles \
                 refuses one); View as follows the rename. Returns {role, old_name}. One undo step",
        run: crate::role_manage::role_rename,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::ModelObject, "Rename role", "rename"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[P::req("name", K::Name(Obj::Role)), P::req("new_name", K::Text)],
    },
    Command {
        id: "role.set",
        label: "Change a security role's description or model permission",
        params: "{name, description?: string|null (null or \"\" removes it), model_permission?: \
                 none|read|readRefresh|refresh|administrator}: at least one of the two; the Model view's \
                 Properties pane of a role. Returns {role, description, model_permission}. One undo step",
        run: crate::role_manage::role_set,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::ModelObject, "Role properties", "settings"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("name", K::Name(Obj::Role)),
            P::opt("description", K::Text),
            P::opt(
                "model_permission",
                K::Enum(&["none", "read", "readRefresh", "refresh", "administrator"]),
            ),
        ],
    },
    Command {
        id: "role.check_filter",
        label: "Check a role's row filter without saving it",
        params: "{role?, table, filter_expression: string or null}: validates the DAX as a row-level \
                 security filter of `table` (its row context; the result must be True/False) against \
                 the open model, as the Manage security roles dialog's check mark does; changes nothing. \
                 Returns {valid, empty (no filter: every row), role, table, errors, warnings, \
                 diagnostics: [{severity, code, message, span: {start, end} (bytes), line, column}]}",
        run: |s, p| crate::role_manage::role_check_filter(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::palette("Check role filter", "validate"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::opt("role", K::Name(Obj::Role)),
            P::req("table", K::Name(Obj::Table)),
            P::req("filter_expression", K::Dax),
        ],
    },
    Command {
        id: "role.set_permission",
        label: "Set a role's object-level security on a table or a column (hidden or visible)",
        params: "{role, table, column?: string (a column of the table; default the table itself), \
                 metadata_permission: \"none\" (hidden from the role's members) | \"read\" | null or \
                 \"default\" (the default, read: the setting is removed)}: writes \
                 tablePermissions[].metadataPermission or its columnPermissions[]; a table permission left \
                 with no filter and no permission goes. Refused for a table related to two or more other \
                 tables (securing it would break the relationship chain). View as applies it: what every \
                 chosen role hides fails as a missing object. Returns {role, table, column, object, \
                 metadata_permission}. One undo step",
        run: crate::role_permission::role_set_permission,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::ModelObject, "Manage roles", "lock"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("role", K::Name(Obj::Role)),
            P::req("table", K::Name(Obj::Table)),
            P::opt("column", K::Text),
            P::req("metadata_permission", K::Enum(&["none", "read", "default"])),
        ],
    },
    Command {
        id: "perspective.add",
        label: "Add a perspective: a named subset of the model's tables, columns, measures and hierarchies",
        params: "{name (unique, ignoring case), description?, tables?: [table name (every member) | {table, \
                 all?: bool, columns?: [names], measures?: [names], hierarchies?: [names]}]}: writes \
                 perspectives[] with each member by name. Returns {perspective, members: {tables, columns, \
                 measures, hierarchies}}. One undo step",
        run: crate::perspective_edit::perspective_add,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::ModelObject, "New perspective", "plus"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("name", K::Text),
            P::opt("description", K::Text),
            P::opt("tables", K::List),
        ],
    },
    Command {
        id: "perspective.set",
        label: "Change a perspective: its members, its name or its description",
        params: "{name, new_name?, description? (null or \"\" clears), tables? (as for perspective.add; \
                 replaces the members, null empties it)}: needs one of new_name, description, tables. \
                 Returns {perspective, members, renamed?}. One undo step",
        run: crate::perspective_edit::perspective_set,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::ModelObject, "Manage perspectives", "eye"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("name", K::Text),
            P::opt("new_name", K::Text),
            P::opt("description", K::Text),
            P::opt("tables", K::List),
        ],
    },
    Command {
        id: "perspective.remove",
        label: "Delete a perspective",
        params: "{name}. One undo step",
        run: crate::perspective_edit::perspective_remove,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::ModelObject, "Delete perspective", "delete"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[P::req("name", K::Text)],
    },
    Command {
        id: "report.new",
        label: "Start a blank report (one empty page) for a session that has none, e.g. after model.new",
        params: "{}: saved by project.new / file.save_as to a PBIP; fails when a report is open",
        run: report::report_new,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Report, "Report", "New report", "page"),
        shortcut: None,
        enabled: &[Need::NoReport],
        form: &[],
    },
    Command {
        id: "report.summary",
        label: "List the report's pages and visuals with their types and bound fields",
        params: "{page?: string (name or display name: only that page), visuals?: bool (default true; \
                 false lists pages only)}",
        run: |s, p| report::report_summary(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::palette("Report summary", "list"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[P::opt("page", K::Name(Obj::Page)), P::flag("visuals", true)],
    },
    Command {
        id: "report.render",
        label: "Render report pages to PNG (pure Rust, like the live viewer; no Power BI Desktop)",
        params: "{page?: string|number (name, display name or 0-based index; default every page), out?: string (a .png file for one page; a folder, or name.png → name-<n>.png, for several; without it the PNG comes back inline as base64), scale?: number (0.1–8, default 1), format?: \"png\", font_dirs?: [string], system_fonts?: bool (default true), badges?: bool (default true), ui_language?: string (language tag of the generated text: auto titles, (Blank), Total…; default the session's, see view.set_language), culture?: string (a model culture whose translated captions name tables, columns and measures; default the session's, see view.set_culture; it does not change number and date formatting nor the generated text. A culture name the model has no translations for, or its own culture, draws the stored names and adds a `warnings` entry; a malformed name is refused), format_culture?: string (culture of numbers, dates and axis labels, e.g. \"fr-FR\"; default the model's culture, like Desktop), as_of?: string (\"YYYY-MM-DD\" or \"YYYY-MM-DDTHH:MM:SS\": the date TODAY()/NOW() return; default the session's, see view.set_as_of), text_dump?: bool (also list every drawn text run: string, font, px size, weight, colour, box; a `<page>.json` beside each PNG with `out`), lcd_text?: bool (subpixel, ClearType-like text antialiasing, as the app draws its pages at low DPI; default false: grayscale), cache_abf?: string (PBIP: read this cache.abf instead of the project's, read-only), fail_on_empty?: bool (error naming the visuals without data), key_influencers?: \"placeholder\"|\"estimate\" (key influencers visuals: the placeholder naming their fields, the default, or BIcraft's own estimate from the data, badged and never Power BI's result; default the viewer's state, see view.interact; docs/key-influencers.md)}: draws the page with its theme, evaluated data, slicer selections, images and formatting; returns per-visual status (what was drawn: drawn, preview = made-up values, placeholder = not drawn, blank = no field bound, skipped) and data (what the evaluated dataset holds, whatever the language: ok, no_rows, blank_values, partial_blank, not_bound, not_evaluated; with rows, value_cells, blank_cells), and per page visuals_without_data (no_rows + blank_values + not_evaluated); `warnings` (only when there are some) lists the parameters that were taken but had no effect",
        run: crate::cache_abf::report_render,
        server: ServerPolicy::paths(&[
            PathParam::one("out"),
            PathParam::many("font_dirs"),
            PathParam::clearable("cache_abf"),
        ])
        .read_only_unless(&["out"]),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::File, "Export", "Export to PNG", "export"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::opt("page", K::Name(Obj::Page)),
            P::opt("out", K::Path),
            P::opt("scale", K::Number),
            P::opt("format", K::Enum(&["png"])),
            P::opt("font_dirs", K::Paths),
            P::flag("system_fonts", true),
            P::flag("badges", true),
            P::opt("ui_language", K::Text),
            P::opt("culture", K::Name(Obj::Culture)),
            P::opt("format_culture", K::Text),
            P::opt("as_of", K::Text),
            P::flag("text_dump", false),
            P::flag("lcd_text", false),
            P::opt("cache_abf", K::Path),
            P::flag("fail_on_empty", false),
            P::opt("key_influencers", K::Enum(&["placeholder", "estimate"])),
        ],
    },
    Command {
        id: "report.export_pdf",
        label: "Export report pages to a PDF file (vector shapes, selectable text with embedded fonts), as Desktop's Export to PDF",
        params: "{out: string (the .pdf file), pages?: \"all\" (default: every page not hidden) | \"current\" (the active page) | \"2-4\" (a 1-based range, \"1,3-5\") | a page name, display name or 0-based index | [those] (pages named one by one are exported even when hidden), hidden?: bool (include hidden pages in \"all\" and ranges; default false), title?: string (the document title; default the file's name), scale?: number (1–4, default 2: the resolution of images and of parts placed as images), font_dirs?: [string], system_fonts?: bool (default true), ui_language?: string, format_culture?: string, as_of?: string}: one PDF page per report page at its size (0.75 pt per report pixel), drawn as report.render draws it; returns {path, bytes, title, pages: [{index, page, display_name, width_pt, height_pt, visuals: [{name, type, status}], raster_parts}], fonts: [{name, embedded, glyphs}], skipped_hidden, warnings?}. Also `bicraft export-pdf`",
        run: |s, p| crate::export_pdf::report_export_pdf(s, p),
        server: ServerPolicy::paths(&[PathParam::one("out"), PathParam::many("font_dirs")]),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::File, "Export", "Export to PDF", "export"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("out", K::Path),
            P::opt("pages", K::Json),
            P::flag("hidden", false),
            P::opt("title", K::Text),
            P::opt("scale", K::Number),
            P::opt("font_dirs", K::Paths),
            P::flag("system_fonts", true),
            P::opt("ui_language", K::Text),
            P::opt("format_culture", K::Text),
            P::opt("as_of", K::Text),
        ],
    },
    Command {
        id: "report.print",
        label: "Print report pages: write them to a PDF and hand it to the operating system's PDF application to print",
        params: "{pages?: as report.export_pdf (default every page not hidden; \"current\" the active page), hidden?: bool, title?: string, out?: string (the .pdf file; default a file in the temporary folder's bicraft-print folder; required on a server), dry_run?: bool (write the PDF, open nothing), ui_language?: string}: writes the PDF as report.export_pdf does (outline, tagged structure), then opens it with the default application for .pdf files (explorer.exe on Windows, open on macOS, xdg-open elsewhere; no shell), where it is printed; on a server nothing is opened (the browser prints the workspace file). Returns report.export_pdf's answer and {opened}",
        run: |s, p| crate::export_pdf::report_print(s, p),
        server: ServerPolicy::paths(&[PathParam::one("out")]),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::File, "Export", "Print", "print"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::opt("pages", K::Json),
            P::flag("hidden", false),
            P::opt("title", K::Text),
            P::opt("out", K::Path),
            P::flag("dry_run", false),
            P::opt("ui_language", K::Text),
        ],
    },
    Command {
        id: "visual.export_data",
        label: "Export a visual's data (summarized or underlying) to a CSV or XLSX file, as the visual's More options > Export data",
        params: "{visual: string, page?: string (needed when several pages have a visual of that name), out: string (.csv or .xlsx), format?: \"csv\"|\"xlsx\" (default from out's extension), kind?: \"summarized\" (default: the visual's fields as it aggregates them) | \"underlying\" (aggregated columns grouped instead, measures on each row; only when the report allows it)}: under the visual's filters, slicers and cross-filters; at most 30,000 rows to CSV and 150,000 to XLSX (Power BI's limits; `truncated` says when rows were left out); refused when the report's export setting (report.settings {export_data}) forbids it; returns {path, bytes, format, kind, page, visual, columns, rows, total_rows, limit, truncated, export_data_mode, warnings?}",
        run: |s, p| crate::export_data::visual_export_data(s, p),
        server: ServerPolicy::paths(&[PathParam::one("out")]),
        replay: Replay::Skip,
        place: Place::menu(Menu::Visual, "Export data", "export"),
        shortcut: None,
        enabled: &[Need::Report, Need::Model],
        form: &[
            P::req("visual", K::Name(Obj::Visual)),
            P::opt("page", K::Name(Obj::Page)),
            P::req("out", K::Path),
            P::opt("format", K::Enum(&["csv", "xlsx"])),
            P::opt("kind", K::Enum(&["summarized", "underlying"])),
        ],
    },
    Command {
        id: "view.set_language",
        label: "Set the UI language of generated visual text (auto titles, implicit-measure names, (Blank), Total, slicer All) in the viewer and report.render",
        params: "{ui_language: string (BCP 47 tag, e.g. \"fr-FR\"; phrase tables for en, fr, de, es, it, \
                 nl, pt, ja; other languages use English)}: returns {ui_language, previous, \
                 strings_language, fallback, supported}; the model and report are not changed",
        run: crate::ui_lang::view_set_language,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Skip,
        place: Place::ribbon(Tab::View, "Language", "Visual text language", "language"),
        shortcut: None,
        enabled: &[],
        form: &[P::req("ui_language", K::Text)],
    },
    Command {
        id: "view.set_culture",
        label: "Show the model's translated captions (a culture's translations) in the viewer and report.render",
        params: "{culture: string|null (a culture of the model, or the model's own culture; null shows the \
                 stored names)}: returns {display_culture}; the model and report are not changed",
        run: crate::translations::view_set_culture,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Skip,
        place: Place::ribbon(Tab::View, "Language", "Show translations", "language"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[P::req("culture", K::Name(Obj::Culture))],
    },
    Command {
        id: "view.set_as_of",
        label: "Set the date and time TODAY()/NOW() and relative date filters use in the viewer and report.render",
        params: "{as_of: string|null (\"YYYY-MM-DD\" or \"YYYY-MM-DDTHH:MM:SS\"; null: the system clock, UTC)}: \
                 returns {as_of}; for reproducible previews; the model and report are not changed",
        run: crate::render::view_set_as_of,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Skip,
        place: Place::ribbon(Tab::View, "Preview", "As-of date", "calendar"),
        shortcut: None,
        enabled: &[],
        form: &[P::req("as_of", K::Text)],
    },
    Command {
        id: "view.interact",
        label: "Act on the report as a reader in the live viewer: click slicer items, select data points (cross-filtering), change page, click buttons",
        params: "{action: \"page\" {page} | \"back\" | \"slicer_toggle\" {page, visual, value, ctrl?} |                  \"slicer_select_all\" {page, visual} | \"slicer_set\" {page, visual, values: [], inverted?} |                  \"slicer_clear\" {page, visual} | \"slicer_range\" {page, visual, from?, to?} (numbers, or ISO dates for a                  date column; null: open) | \"slicer_text\" {page, visual, text} | \"slicer_relative\" {page, visual,                  range: last|next|this, duration, period: days|weeks|calendar weeks|months|calendar months|years|calendar                  years} | \"select\" {page, visual, point: [{c, v}] (data column, value) | null, points?: [[{c, v}]] (several points selected together, point their first), ctrl?} |                  \"clear_selection\" {page} | \"clear_slicers\" {page} | \"apply_slicers\" {page} | \"button\" {page,                  visual, destination? (a conditional (fx) destination: the page, bookmark or URL the snapshot evaluated, interaction.button.destination; refused for a fixed one)} (the button's visualLink: PageNavigation, Back, ApplyAllSlicers, ClearAllSlicers, Drillthrough, WebUrl, Bookmark; a drillthrough button needs one selected data point, interaction.button.enabled) | \"bookmark\" {bookmark} (name or display name; in this view only: its page unless suppressActiveSection, its hidden and shown visuals unless suppressDisplay, its report, page and visual filters unless suppressData; a bookmark navigator's buttons send it) | \"bookmark_deselect\" (a bookmark navigator with Allow deselection and no deselection bookmark: no bookmark shows as selected) | \"decomp_path\" {page, visual, level, value, path?} | \"decomp_expand\" {page, visual, level, value, column | ai: high|low, path?, columns?} | \"decomp_collapse\" {page, visual, level} | \"decomp_scroll\" {page, visual, level, start} | {page, visual, shift} (decomposition trees: move the selected path, add a level, remove a level, show a level's members from `start` on (its scroll arrows), scroll a tree wider than the visual by `shift` levels; a new path or level resets the scroll of the levels below) | \"key_influencers\" {mode: \"estimate\"|\"placeholder\"} (key influencers visuals show BIcraft's estimate or the placeholder; kept by reset) | \"drill_down\" {page, visual, point, role?} | \"drill_up\" {page, visual, role?} | \"drill_next\" {page, visual, role?} (go to the next level) | \"drill_expand\" {page, visual, role?} (expand all down one level) | \"drill_mode\" {page, visual, on?} (a click on a point drills into it) | \"matrix_expand\" / \"matrix_collapse\" {page, visual, path: [values, top level shown first] | level: n | all: true, role?} (a matrix's +/-, entire level, all) | \"drillthrough\" {page, visual, point, target} (opens the drillthrough page filtered on the point; Keep all filters carries the visual's filters; cross-report pages refused) | \"tooltip\" {page, visual, point | null} (the report-page tooltip of the hovered point; null hides it) | \"focus\" {page, visual | null} (focus mode: the visual shown alone; null leaves it; a page change leaves it too) | \"show_table\" {page, visual, on?: bool (default true)} (Show as a table: the visual in focus with its data as a table under it; off keeps the focus) | \"spotlight\" {page, visual | null} (Spotlight: the page's other visuals fade; null ends it; a page change and reset too; a bookmark captures and applies it) | \"sort\" {page, visual, field | null, direction?: asc|desc} (the reader's sort, as visual.set_sort takes it, the report unchanged; null goes back to the report's sort; reset drops it; a bookmark carries it as orderBy) | \"tree_toggle\" {page, visual, id} (a BIcraft Org Chart node's collapse button: the reader's toggled nodes replace the stored tree.toggled in this view; visual.toggle_node saves them) | \"filter\" {page?, visual?, field, then in | not_in | op, value | conditions, logic? | relative | top, by, direction: top|bottom (visual only), as visual.filter takes them} (the reader changes a filter card the author did not lock or hide: of the visual with page and visual, of the page with page, of every page with neither; a bound field of a visual counts as a card; changing the card's type needs report.settings allow_change_filter_types; this view only, the report unchanged; reset drops it) | \"filter_clear\" {page?, visual?, field} (the reader clears that card's condition, the card kept) | \"view_as\" {roles: [role names] | null, user?: string} (Modeling › View as: every evaluation applies the roles' row filters, united, through the relationships; USERPRINCIPALNAME() and USERNAME() return `user`; an unknown role is an error; roles null without user stops; cleared by reset) | \"reset\"}:                  Power BI Desktop's click rules (docs/view-snapshot.md, Interactions); returns {action, changed, state}.                  The snapshot shows the report under this state; the model, the report and the files are never changed",
        run: crate::interact::view_interact,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Skip,
        place: Place::palette("Reader action", "chart"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("action", K::Enum(INTERACT_ACTIONS)),
            P::opt("page", K::Name(Obj::Page)),
            P::opt("visual", K::Name(Obj::Visual)),
            P::opt("value", K::Json),
            P::opt("values", K::List),
            P::flag("ctrl", false),
            P::flag("inverted", false),
            P::opt("from", K::Json),
            P::opt("to", K::Json),
            P::opt("text", K::Text),
            P::opt("range", K::Enum(&["last", "next", "this"])),
            P::opt("duration", K::Integer),
            P::opt("period", K::Text),
            P::opt("point", K::Json),
            P::opt("level", K::Integer),
            P::opt("start", K::Integer),
            P::opt("shift", K::Integer),
            P::opt("path", K::List),
            P::opt("column", K::Field),
            P::opt("ai", K::Enum(&["high", "low"])),
            P::opt("columns", K::List),
            P::opt("mode", K::Enum(&["estimate", "placeholder"])),
            P::opt("role", K::Text),
            P::opt("target", K::Name(Obj::Page)),
            P::opt("on", K::Json),
            P::flag("all", false),
            P::opt("bookmark", K::Text),
            P::opt("roles", K::List),
            P::opt("user", K::Text),
            P::opt("field", K::Field),
            P::opt("direction", K::Enum(&["asc", "desc", "top", "bottom"])),
            P::opt("id", K::Text),
            P::opt("in", K::List),
            P::opt("not_in", K::List),
            P::opt("op", K::Enum(FILTER_OPS)),
            P::opt("conditions", K::List),
            P::opt("logic", K::Enum(&["and", "or"])),
            P::opt("relative", K::Json),
            P::opt("top", K::Integer),
            P::opt("by", K::Field),
        ],
    },
    Command {
        id: "view.interactions",
        label: "The live viewer's interaction state (slicer selections, data-point selections, page)",
        params: "{}: returns {seq, page, back, slicers, pending, selections, trees, bookmark, hidden, bookmark_filters, focus, spotlight, sorts}",
        run: |s, p| crate::interact::view_interactions(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::palette("Reader state", "info"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[],
    },
    Command {
        id: "view.keep_drill",
        label: "Keep the reader's drill and matrix expansion in the report, as Power BI Desktop saves them",
        params: "{page?, visual?}: writes the drill state set with view.interact (displayed levels as the \n                 projections' `active` flags, the values drilled into as `howCreated: Drill` filters, a \n                 matrix's `expansionStates`) into the report for every drilled visual, the visuals of `page`, \n                 or `visual`; their reader drill state is dropped. Returns {kept: [{page, visual}], changed}",
        run: crate::interact::view_keep_drill,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Copy,
        place: Place::menu(Menu::Visual, "Keep drill state", "hierarchy"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::opt("page", K::Name(Obj::Page)),
            P::opt("visual", K::Name(Obj::Visual)),
        ],
    },
    Command {
        id: "visual.expand",
        label: "Expand or collapse a matrix's rows or columns in the report (+/-, Expand / Collapse, the header's expand and level buttons), as Power BI Desktop saves it",
        params: "{page, visual, role?: \"Rows\"|\"Columns\" (default: the first with two or more \n                 levels), then one of path: [values, top level shown first] (one member: +/-, \n                 Selection), level: n (an entire level, 0: the top one shown), all: true (All), with \n                 collapse?: bool (default false: expand); or step: \"expand_all\" (Expand all down one \n                 level) | \"next\" (Go to the next level) | \"up\" (Drill up)}: writes the role's \n                 projections' `active` flags and the visual's `expansionStates` (Desktop's PBIR \n                 shapes), saved with the file and undoable; a reader's drill state on the visual is \n                 dropped. Returns {page, visual, changed, drill}",
        run: crate::interact::expand::visual_expand,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::Visual, "Expand or collapse", "hierarchy"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::opt("role", K::Enum(&["Rows", "Columns"])),
            P::opt("path", K::List),
            P::opt("level", K::Integer),
            P::flag("all", false),
            P::flag("collapse", false),
            P::opt("step", K::Enum(&["expand_all", "next", "up"])),
        ],
    },
    Command {
        id: "visual.toggle_node",
        label: "Collapse or expand a node of a BIcraft Org Chart (its round button), as the visual saves it in Power BI Desktop",
        params: "{page, visual, id: the node's id (1-100 characters)}: removes `id` from the visual's \
                 `tree.toggled` list (a JSON list of node ids whose open state differs from the \
                 `tree.levels` default) when it is there, else adds it last (500 kept), as the visual's \
                 TypeScript persists it; saved with the file and undoable (a reader's toggles are \
                 view.interact `tree_toggle`). Returns {page, visual, id, toggled}",
        run: crate::chart_pack::visual_toggle_node,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Collapse or expand an org chart node", "hierarchy"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::req("id", K::Text),
        ],
    },
    Command {
        id: "culture.add",
        label: "Add a culture (translation language) to the model",
        params: "{name: string (e.g. fr-FR)}: an empty culture; fill it with translation.set",
        run: crate::translations::culture_add,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Model, "Translations", "New culture", "language"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[P::req("name", K::Text)],
    },
    Command {
        id: "culture.remove",
        label: "Remove a culture with its translations (and its linguistic schema, if it has one)",
        params: "{name}: returns {removed, translations_removed, linguistic_metadata_removed}",
        run: crate::translations::culture_remove,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::ModelObject, "Delete culture", "delete"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[P::req("name", K::Name(Obj::Culture))],
    },
    Command {
        id: "translation.set",
        label: "Set or remove the translated caption, description or display folder of a model object in one culture",
        params: "{culture, object: {} (the model) | {table} | {table, column} | {table?, measure} | {table, \
                 hierarchy} | {table, hierarchy, level} | {perspective} | {role} | {expression}, property: \
                 \"caption\"|\"description\"|\"display_folder\" (columns, measures and hierarchies only), \
                 value: string | null (null or \"\" removes it)}: renames keep translations attached, \
                 removing an object drops its translations (`translations_dropped`)",
        run: crate::translations::translation_set,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Model, "Translations", "Translate", "edit"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("culture", K::Name(Obj::Culture)),
            P::req("object", K::Json),
            P::req("property", K::Enum(&["caption", "description", "display_folder"])),
            P::req("value", K::Text),
        ],
    },
    Command {
        id: "translation.list",
        label: "List the model's cultures and translated captions, descriptions and display folders",
        params: "{culture?}: returns {cultures: [{name, model_culture, translations, linguistic_metadata}], \
                 count, translations: [{culture, object (as translation.set takes it), property, value}]}",
        run: |s, p| crate::translations::translation_list(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Model, "Translations", "Translations", "list"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[P::opt("culture", K::Name(Obj::Culture))],
    },
    Command {
        id: "report.custom_visuals",
        label: "List the report's custom visuals: embedded ones (CustomVisuals/ files and their report.json package), AppSource and organization references, and the visuals that use each",
        params: "{}: returns {embedded: [{name, guid, display_name, version, api_version, author, data_roles, registered, items, metadata_file, files, used_by, issues, bicraft}], public: [{name, used_by}], organization: [{name, path, disabled, used_by}], count, bicraft: [{name, guid, display_name, version, embedded, embedded_version, current, roles, used_by}] (BIcraft's own visuals, embedded or not; a save drops the package of one no visual uses)}; read-only",
        run: |s, p| crate::custom_visuals::report_custom_visuals(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Report, "Visuals", "Custom visuals", "chart"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[],
    },
    Command {
        id: "visual.import_custom",
        label: "Import a custom visual from a .pbiviz file (Visualizations pane › … › Import a visual from a file)",
        params: "{path | data (base64 or a data: URL)}: a .pbiviz package (a ZIP of package.json and \
                 resources/<guid>.pbiviz.json) of at most 32 MB, read with capped sizes and names \
                 checked; stored as CustomVisuals/<guid>/package.json and resources/… with a \
                 CustomVisual resource package in report.json (PBIR reports; a legacy layout is \
                 refused). The same guid replaces the visual. Its JavaScript is never run: BIcraft \
                 draws its visuals as a placeholder with the package's icon and name. Returns {name, \
                 display_name, version, api_version, author, has_icon, has_javascript, files, skipped, \
                 replaced, used_by, warnings}",
        run: crate::custom_visuals::visual_import_custom,
        server: ServerPolicy::paths(&[PathParam::one("path")]),
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Insert, "Visuals", "Import a visual", "import"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[P::opt("path", K::Path), P::opt("data", K::Text)],
    },
    Command {
        id: "visual.remove_custom",
        label: "Remove a custom visual from the report (Visualizations pane › … › Remove a custom visual)",
        params: "{name (the visual's guid, as report.custom_visuals lists it), force?: bool}: drops \
                 its CustomVisual resource package, its CustomVisuals/<name>/ files and an AppSource \
                 reference of that name. Refused while visuals use it unless force (they stay, drawn \
                 as placeholders). Returns {name, removed_package, removed_files, \
                 removed_public_reference, kept_visuals}",
        run: crate::custom_visuals::visual_remove_custom,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Insert, "Visuals", "Remove a custom visual", "delete"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[P::req("name", K::Text), P::opt("force", K::Bool)],
    },
    Command {
        id: "visual.scaffold",
        label: "Create a custom visual project: pbiviz.json, capabilities.json with the data roles and format options chosen, a TypeScript visual, its CSS and an icon",
        params: "{folder, name (letters, digits, _), display_name?, guid? (default: name + 32 hex \
                 digits), description?, author?, data_roles?: [\"name:Grouping|Measure|GroupingOrMeasure\
                 [:Display name]\" | {name, kind, display_name}] (default category:Grouping and \
                 measure:Measure), format_options?: [\"object.property:bool|color|numeric|text\
                 [:Display name]\" | {object, property, kind, display_name}] (default dataPoint.fill:color \
                 and dataPoint.fontSize:numeric), overwrite?: bool, template?: default | spec (the BIcraft \
                 Spec visual's own sources, roles and spec.json option: no data_roles or format_options)}: writes pbiviz.json, capabilities.json \
                 (a categorical mapping of the roles, the options as format objects), tsconfig.json, \
                 package.json (editor typings only), src/visual.ts, style/visual.css and assets/icon.png \
                 into folder; a folder holding a pbiviz.json is refused unless overwrite. Returns {folder, \
                 name, display_name, guid, data_roles, format_options, files}",
        run: crate::visual_project::visual_scaffold,
        server: ServerPolicy::paths(&[PathParam::one("folder")]),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Insert, "Visuals", "New visual project", "plus"),
        shortcut: None,
        enabled: &[],
        form: &[
            P::req("folder", K::Path),
            P::req("name", K::Text),
            P::opt("display_name", K::Text),
            P::opt("data_roles", K::List),
            P::opt("format_options", K::List),
            P::opt("author", K::Text),
            P::opt("description", K::Text),
            P::opt("guid", K::Text),
            P::flag("overwrite", false),
            P::opt("template", K::Enum(&["default", "spec"])),
        ],
    },
    Command {
        id: "visual.build",
        label: "Compile a custom visual project's TypeScript and bundle it into one JavaScript file (swc, no Node)",
        params: "{folder, out? (default <folder>/dist/visual.js)}: reads pbiviz.json, the .ts and .js \
                 files under src/ (the entry: tsconfig.json files[0], else src/visual.ts), strips the \
                 types, bundles the relative imports with a module registering the visual's class under \
                 its guid in powerbi.visuals.plugins, and writes the script. Package imports (npm), import \
                 cycles, LESS styles and .tsx files are refused; types are not checked. Returns {guid, \
                 entry, modules, out, js_bytes, warnings}",
        run: crate::visual_project::visual_build,
        server: ServerPolicy::paths(&[PathParam::one("folder"), PathParam::clearable("out")]),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Insert, "Visuals", "Compile a visual", "play"),
        shortcut: None,
        enabled: &[],
        form: &[P::req("folder", K::Path), P::opt("out", K::Path)],
    },
    Command {
        id: "visual.pack",
        label: "Package a custom visual project as a .pbiviz file (built with visual.build's compiler)",
        params: "{folder, out? (default <folder>/dist/<guid>.<version>.pbiviz)}: builds the project \
                 as visual.build does and writes a .pbiviz: package.json (visual, author, resources, \
                 metadata) and resources/<guid>.pbiviz.json (visual, apiVersion, author, capabilities, \
                 content: js, css, iconBase64), which visual.import_custom reads. Returns {guid, \
                 display_name, version, api_version, entry, modules, out, bytes, js_bytes, has_icon, \
                 warnings}",
        run: crate::visual_project::visual_pack,
        server: ServerPolicy::paths(&[PathParam::one("folder"), PathParam::clearable("out")]),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Insert, "Visuals", "Package a visual", "export"),
        shortcut: None,
        enabled: &[],
        form: &[P::req("folder", K::Path), P::opt("out", K::Path)],
    },
    Command {
        id: "visual.export_bicraft_visual",
        label: "Export one of BIcraft's visuals as a .pbiviz file, for Desktop's Import a visual from a file and for organizational visuals",
        params: "{name: a BIcraft visual's name or guid (bicraftSpec, bicraftHistogram, bicraftBullet, \
                 bicraftBoxPlot, bicraftDumbbell, bicraftOrgChart, bicraftVenn, bicraftMarimekko, \
                 bicraftCalendarHeatmap, bicraftSankey, bicraftScatterPlus, bicraftLineAreaPlus, bicraftWaterfallPlus, bicraftGaugeKpi; report.custom_visuals `bicraft` lists them) | [names] | \
                 \"all\", out: a .pbiviz file (one visual) or a folder (created when missing; each \
                 visual written as <guid>.<version>.pbiviz)}: writes the package BIcraft embeds in a \
                 report (the same bytes), which visual.import_custom reads back; existing files are \
                 replaced. A list or \"all\" always writes into a folder. Needs no open file and \
                 leaves the session as it is. Returns {out, count, files: [{name, guid, display_name, \
                 version, path, bytes}]}. Also `bicraft visual export`",
        run: crate::builtin_visuals::visual_export_bicraft_visual,
        server: ServerPolicy::paths(&[PathParam::one("out")]),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Insert, "Visuals", "Export a BIcraft visual", "export"),
        shortcut: None,
        enabled: &[],
        form: &[
            P::req("name", K::Enum(crate::builtin_visuals::EXPORT_NAMES)),
            P::req("out", K::Path),
        ],
    },
    Command {
        id: "visual.set_spec",
        label: "Draw a visual from a declarative chart spec: marks, encodings, scales, axes and legend (the BIcraft Spec custom visual, no JavaScript to write)",
        params: "{page, visual, spec: object | its JSON text}: the spec (docs/formats/spec-visual.md; \
                 visual.spec_template gives starting ones) has marks (bar, line, point, area, text, rule, \
                 rect, at most 16) whose x, y, x2, y2, color, size and label read `category` and \
                 `value` / `value.N` (the visual's category and its measures in order), x and y scales \
                 (band, linear, time), axes, gridlines, a palette and a legend; at most 32 KB, checked \
                 with every error's place. Embeds the BIcraft Spec custom visual in the report when it \
                 is missing or older (a PBIR report), turns the visual into one (its columns go to the \
                 category role, its measures and aggregations to values) and stores the spec in its \
                 spec.json property. Power BI draws it with the visual's TypeScript; report.render, PDF \
                 export and the app draw the same layout. Returns {visual, visual_type, converted_from, \
                 installed, marks, values_used, fields, dropped, spec_bytes, warnings}",
        run: crate::spec_visual::visual_set_spec,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Visual, "Type", "Chart spec", "chart"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::req("spec", K::Json),
        ],
    },
    Command {
        id: "visual.spec_template",
        label: "A starting chart spec for the BIcraft Spec visual (bar, line, area, point, scatter, combo, text, rule)",
        params: "{kind? (default bar): bar | line | area | point | scatter | combo | text | rule}: returns \
                 {kind, spec, kinds, visual_type, roles, fields, marks, max_spec_bytes, doc}; read-only. \
                 Pass the spec to visual.set_spec",
        run: |s, p| crate::spec_visual::visual_spec_template(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::palette("Chart spec template", "chart"),
        shortcut: None,
        enabled: &[],
        form: &[P::opt("kind", K::Enum(bicraft_render::spec::TEMPLATES))],
    },
    Command {
        id: "visual.set_bicraft_visual",
        label: "Turn a visual into one of BIcraft's own chart visuals: histogram, bullet chart, box plot, dumbbell or slope chart",
        params: "{page, visual, kind: histogram | bullet | box_plot | dumbbell, options?: {object: {property: \
                 value | null}}}: embeds the visual's package (built from BIcraft's TypeScript by its own \
                 bundler, docs/formats/bicraft-visuals.md) in the report when it is missing or older (a PBIR \
                 report), turns the visual into it (columns go to its grouping roles, measures and \
                 aggregations to its measure roles, in order while they have room; the rest is `dropped`) and \
                 sets its format options, each checked (histogram: bins.count 1-200, bins.size, colors.fill, \
                 labels.show; bullet: bands.first/second/third as % of the target, colors.value, \
                 colors.target, labels.show; box_plot: box.mean, box.outliers, colors.fill; dumbbell: \
                 style.slope, colors.start, colors.end, colors.line, labels.show; null removes one). Roles: \
                 histogram values (column), frequency; bullet category, value, target, bands (up to 4), \
                 maximum; box_plot category, sample (the items), value; dumbbell category, start, end. Power \
                 BI draws it with the visual's script; report.render, PDF export and the app draw the same \
                 layout. Returns {visual, kind, visual_type, display_name, converted_from, installed, fields, \
                 dropped, options, warnings}",
        run: crate::bvis::visual_set_bicraft_visual,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Visual, "Type", "BIcraft chart", "chart"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::req("kind", K::Enum(bicraft_render::bvis::KINDS)),
            P::opt("options", K::Json),
        ],
    },
    Command {
        id: "report.validate",
        label: "Validate the report: PBIR schemas, structure, data roles, report measures, and every field against the model",
        params: "{}: returns {valid, error_count, warning_count, checked_against_model, schema_issues, \
                 issues}; unknown tables, columns and measures in visuals and filters are errors; data \
                 roles are checked per built-in visual type (unknown role, measure in a column-only role, \
                 too many fields: errors; nothing to draw: warning); report-level measures' DAX is \
                 validated; visuals sticking out of their page and overlapping visible visuals are warnings; \
                 a field projected under its queryRef and that queryRef plus a digit suffix (`Sum(T.C)`, `Sum(T.C)1`) is an error that \
                 report.repair fixes",
        run: |s, p| report::report_validate(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Analyze, "Validate", "Validate report", "validate"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[],
    },
    Command {
        id: "report.repair",
        label: "Repair report defects Power BI Desktop cannot open that report.validate reports as fixable",
        params: "{page?, visual? (needs page)}: default every visual. Points every projection of a \
                 field projected under its queryRef plus a digit suffix (a query saved by an older bicraft) at its first \
                 queryRef, so the query selects the field once (legacy layouts lose the duplicate \
                 select item on save); returns {count, fixed: [{page, visual, kind, kept, replaced}]}",
        run: crate::report_repair::report_repair,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Analyze, "Validate", "Repair report", "settings"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::opt("page", K::Name(Obj::Page)),
            P::opt("visual", K::Name(Obj::Visual)),
        ],
    },
    Command {
        id: "report.rebind",
        label: "Re-bind every report reference to a field the model lacks to another field",
        params: "{from: \"Table[Name]\" (a column or measure the model does not have), to: field \
                 (a column or measure of the model)}: every reference of that kind (as `to`'s) \
                 in visual roles, sorts, filters, drillthrough and tooltip bindings, formatting, \
                 bookmarks and properties is pointed at `to`, with the queryRefs and selectors \
                 that name it, as a rename with update_report does; refused when `from` is in the \
                 model, nothing refers to it, or a reference goes through a query alias shared \
                 with other fields. Returns {from, to, kind, affected_bindings, \
                 affected_binding_count, other_references, other_reference_count, query_refs_updated?}",
        run: crate::report_fix::report_rebind,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Report, "Report", "Re-bind field", "link"),
        shortcut: None,
        enabled: &[Need::Report, Need::Model],
        form: &[P::req("from", K::Text), P::req("to", K::Field)],
    },
    Command {
        id: "report.remove_reference",
        label: "Remove what refers to a field at one place: a drillthrough or tooltip field, conditional formatting, a bookmark's state",
        params: "{field, place: drillthrough {page} | formatting {page, visual} | {page} | {} (the \
                 report) | bookmark {bookmark}}: drillthrough removes the page's drillthrough (or \
                 tooltip) filter cards on the field and their binding parameters; formatting \
                 removes the formatted properties naming the field (an entry whose selector names \
                 it goes whole, and an entry or object left empty); bookmark removes each entry \
                 of the bookmark's lists (filters, sorts, formatting) naming it. Refused, changing \
                 nothing, when nothing there names the field or a reference would remain. \
                 Returns {field, place, at, removed}",
        run: crate::report_fix::report_remove_reference,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Report, "Report", "Clear reference", "minus"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("field", K::Text),
            P::req("place", K::Enum(&["drillthrough", "formatting", "bookmark"])),
            P::opt("page", K::Name(Obj::Page)),
            P::opt("visual", K::Name(Obj::Visual)),
            P::opt("bookmark", K::Text),
        ],
    },
    Command {
        id: "visual.fix_query_refs",
        label: "Give a visual's projections whose queryRef names another field their own queryRef",
        params: "{page, visual}: each projection of the visual's query whose queryRef names \
                 another field (REPORT_STALE_QUERYREF) takes the field's own (`Sales.Total`, \
                 `Sum(Sales.Amount)`) unless another projection of the visual has it; formatting \
                 selectors, expansion states, the visual's state in bookmarks and the phone \
                 layout follow. Returns {renamed: [{role, field, from, to}], kept, references_updated}",
        run: crate::report_fix::visual_fix_query_refs,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Visual, "Data", "Fix queryRefs", "settings"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
        ],
    },
    Command {
        id: "report_measure.list",
        label: "List the report-level measures (reportExtensions.json / legacy modelExtensions), validated against the model",
        params: "{}: returns {count, measures: [{table, name, expression, data_type, format_string, \
                 description, display_folder, hidden, store, valid?, diagnostics?}], store}",
        run: |s, p| crate::report_measures::report_measure_list(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Report, "Report measures", "Report measures", "list"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[],
    },
    Command {
        id: "report_measure.add",
        label: "Add a report-level measure (PBIR reportExtensions.json); its DAX is validated against the model",
        params: "{table, name, expression, format_string?, data_type? (default: the expression's type), \
                 description?, display_folder?, hidden?, force?: keep invalid DAX}: visuals bind it as \
                 [name] or table[name]; refused on a legacy layout",
        run: crate::report_measures::report_measure_add,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Report, "Report measures", "New report measure", "measure"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("table", K::Name(Obj::Table)),
            P::req("name", K::Text),
            P::req("expression", K::Dax),
            P::opt("format_string", K::Text),
            P::opt("data_type", K::Text),
            P::opt("description", K::Text),
            P::opt("display_folder", K::Text),
            P::flag("hidden", false),
            P::flag("force", false),
        ],
    },
    Command {
        id: "report_measure.set",
        label: "Change a report-level measure's expression or properties",
        params: "{name, table?, expression?, format_string?, data_type?, description?, display_folder?, \
                 hidden?, force?}: null or \"\" removes format_string, description, display_folder; on a \
                 legacy layout only expression and hidden",
        run: crate::report_measures::report_measure_set,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Report, "Report measures", "Edit report measure", "edit"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("name", K::Name(Obj::ReportMeasure)),
            P::opt("table", K::Name(Obj::Table)),
            P::opt("expression", K::Dax),
            P::opt("format_string", K::Text),
            P::opt("data_type", K::Text),
            P::opt("description", K::Text),
            P::opt("display_folder", K::Text),
            P::opt("hidden", K::Bool),
            P::flag("force", false),
        ],
    },
    Command {
        id: "report_measure.remove",
        label: "Remove a report-level measure",
        params: "{name, table?}: warns about visuals, filters and report measures still using it",
        run: crate::report_measures::report_measure_remove,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Report, "Report measures", "Delete report measure", "delete"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("name", K::Name(Obj::ReportMeasure)),
            P::opt("table", K::Name(Obj::Table)),
        ],
    },
    Command {
        id: "report.apply",
        label: "Make the report match a declarative spec (pages and visuals); re-applying is a no-op",
        params: "{spec: {pages: [{key | name, display_name?, width?, height?, display_option?, hidden?, \
                 layout?, visuals?: [{key | name, type, x?, y?, width?, height?, z?, place?, hidden?, \
                 fields?: [{role, field} | {role, raw}], props?}]}], active_page?}, prune?: \
                 true|\"visuals\"|\"all\", dry_run?: bool, force?: bool}: creates what is missing, \
                 updates what differs, leaves the rest. A `key` (any string) always maps to the same \
                 20-hex name, so re-runs never duplicate; `name` matches an existing object. Omitted \
                 properties are left alone; `fields` replaces the visual's bindings. A visual without \
                 x/y/width/height (or with `place`) is placed by the page `layout`. prune: true removes \
                 unlisted visuals of listed pages, \"all\" also unlisted pages. Everything is checked \
                 first: any error changes nothing. A `type` may be one of BIcraft's visuals by name or \
                 guid, as with visual.add: its roles are checked and its package embedded on first use \
                 (`installed`); a new Spec visual without a spec starts with visual.spec_template's. \
                 Returns {change_count, changes, summary, warnings, ids, installed}",
        run: crate::report_apply::report_apply,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Report, "Spec", "Apply spec", "check"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("spec", K::Json),
            P::opt("prune", K::Json),
            P::flag("dry_run", false),
            P::flag("force", false),
        ],
    },
    Command {
        id: "report.check",
        label: "Compare the report with a declarative spec: the changes report.apply would make, and its errors",
        params: "{spec, prune?, force?}: as report.apply, but changes nothing; returns {valid, in_sync, \
                 change_count, changes: [{action: create|update|remove|reorder|set_active, kind, page, \
                 visual?, key?, properties?}], errors: [{at, message}], warnings, ids}",
        run: |s, p| crate::report_apply::report_check(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Report, "Spec", "Compare with spec", "compare"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("spec", K::Json),
            P::opt("prune", K::Json),
            P::flag("force", false),
        ],
    },
    Command {
        id: "report.extract",
        label: "The report as a declarative spec (pages and visuals by name) that report.apply accepts",
        params: "{page? (name or display name: only that page)}: applying the result to the same \
                 report changes nothing; edit it and apply it to change the report",
        run: |s, p| crate::report_apply::report_extract(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Report, "Spec", "Extract spec", "export"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[P::opt("page", K::Name(Obj::Page))],
    },
    Command {
        id: "page.add",
        label: "Add a report page",
        params: "{display_name, width? (default 1280), height? (default 720), display_option?: \
                 FitToPage|FitToWidth|ActualSize (default FitToPage), index? (default: last), key?}: returns the \
                 generated page name (20 hex digits, as Desktop); with `key`, the name report.apply \
                 gives that key",
        run: report::page_add,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Report, "Pages", "New page", "page"),
        shortcut: Some("Ctrl+M"),
        enabled: &[Need::Report],
        form: &[
            P::req("display_name", K::Text),
            P::opt("width", K::Integer),
            P::opt("height", K::Integer),
            P::opt(
                "display_option",
                K::Enum(&["FitToPage", "FitToWidth", "ActualSize"]),
            ),
            P::opt("index", K::Integer),
            P::opt("key", K::Text),
        ],
    },
    Command {
        id: "page.remove",
        label: "Remove a report page with its visuals",
        params: "{page (name or display name)}: the last page cannot be removed",
        run: report::page_remove,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::Page, "Delete page", "delete"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[P::req("page", K::Name(Obj::Page))],
    },
    Command {
        id: "page.rename",
        label: "Change a page's display name",
        params: "{page (name or display name), display_name}",
        run: report::page_rename,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::Page, "Rename page", "rename"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("display_name", K::Text),
        ],
    },
    Command {
        id: "page.move",
        label: "Move a page to another position",
        params: "{page (name or display name), index: 0-based position}",
        run: report::page_move,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::Page, "Move page", "chevron-right"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[P::req("page", K::Name(Obj::Page)), P::req("index", K::Integer)],
    },
    Command {
        id: "page.set_active",
        label: "Make a page the one the report opens on",
        params: "{page (name or display name)}",
        run: report::page_set_active,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::Page, "Open on this page", "pin"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[P::req("page", K::Name(Obj::Page))],
    },
    Command {
        id: "page.set_display_option",
        label: "Set how a page is scaled: fit to page, fit to width or actual size (View › Page view)",
        params: "{page (name or display name), display_option: FitToPage|FitToWidth|ActualSize}: \
                 written as the page's displayOption. Returns {page, display_option, previous, changed}",
        run: report::page_set_display_option,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::View, "Page view", "Page view", "fit-width"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("display_option", K::Enum(report::DISPLAY_OPTIONS)),
        ],
    },
    Command {
        id: "visual.add",
        label: "Add a visual to a page, optionally with fields",
        params: "{page, visual_type (e.g. card, clusteredColumnChart, tableEx, slicer, lineChart), x, y, \
                 width, height, z?, fields?: [{role, field}], force?: bool}: a field is \"Table[Column]\", \
                 \"[Measure]\" or \"Sum(Table[Column])\" (Sum, Avg, Count, Min, Max, CountNonNull, Median, \
                 StandardDeviation, Variance), resolved against the model (unknown fields are rejected \
                 unless force); roles are the visual's data roles (Values, Category, Y, Legend, Rows...). \
                 One of BIcraft's visuals (`bicraftSpec`; report.custom_visuals `bicraft` lists them) is \
                 named or given by its guid: the visual carries the guid, its roles are checked and its \
                 package is embedded on first use (a missing or older build replaced, `installed`; PBIR \
                 reports only); a Spec visual starts with visual.spec_template's bar spec, which draws \
                 each value bound as its own series (`spec_template`). Returns the generated visual name; with \
                 `key?`, the name report.apply gives that key on that page",
        run: report::visual_add,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Report, "Visuals", "New visual", "chart"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual_type", K::Name(Obj::VisualType)),
            P::req("x", K::Number),
            P::req("y", K::Number),
            P::req("width", K::Number),
            P::req("height", K::Number),
            P::opt("z", K::Integer),
            P::opt("fields", K::List),
            P::flag("force", false),
            P::opt("key", K::Text),
        ],
    },
    Command {
        id: "visual.set_position",
        label: "Move or resize a visual",
        params: "{page, visual, x?, y?, z?, width?, height?}: only the given values change",
        run: report::visual_set_position,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Visual, "Arrange", "Position and size", "fit-small"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::opt("x", K::Number),
            P::opt("y", K::Number),
            P::opt("z", K::Integer),
            P::opt("width", K::Number),
            P::opt("height", K::Number),
        ],
    },
    Command {
        id: "visual.place",
        label: "Put a visual on a grid cell of its page",
        params: "{page, visual, col, row (0-based), col_span? (default 1), row_span? (default 1), \
                 grid?: {columns? (12), rows? (8), margin? (16), gutter? (16)}}: sets x, y, width and \
                 height to whole pixels (a group's members move with it); returns the position and \
                 overlap `warnings`",
        run: crate::layout::visual_place,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Visual, "Arrange", "Snap to grid", "grid"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::req("col", K::Integer),
            P::req("row", K::Integer),
            P::opt("col_span", K::Integer),
            P::opt("row_span", K::Integer),
            P::opt("grid", K::Json),
        ],
    },
    Command {
        id: "page.arrange",
        label: "Lay out a page's visuals on shelves or a grid",
        params: "{page, layout?: \"shelf\" (default) | \"grid\" | {shelf: {margin?, gutter?}} | \
                 {grid: {columns?, rows?, margin?, gutter?}}, visuals?: [name | {visual, place?}], \
                 dry_run?: bool}: without `visuals`, every visible top-level visual in reading order. \
                 `place`: null (default size by visual type), {width?, height?} (px or \"50%\"), \
                 {col, row, col_span?, row_span?} or {col_span?, row_span?} (grid), or {x, y, width, \
                 height}. Fails, naming the visual and the missing room, when something does not fit. \
                 Returns `arranged` [{visual, position, previous}] and overlap `warnings`; `dry_run` \
                 changes nothing",
        run: crate::layout::page_arrange,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Report, "Pages", "Arrange visuals", "grid"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::opt("layout", K::Json),
            P::opt("visuals", K::List),
            P::flag("dry_run", false),
        ],
    },
    Command {
        id: "visual.bind",
        label: "Bind a field to a visual's data role",
        params: "{page, visual, role, field: \"Table[Column]\"|\"[Measure]\"|\"Sum(Table[Column])\", \
                 index? (position in the role, default last), force?: bool}",
        run: crate::slicer_sync::visual_bind,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Visual, "Data", "Add field", "plus"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::req("role", K::Name(Obj::DataRole)),
            P::req("field", K::Field),
            P::opt("index", K::Integer),
            P::flag("force", false),
        ],
    },
    Command {
        id: "visual.unbind",
        label: "Remove a field (or every field) from a visual's data role",
        params: "{page, visual, role, field? | index? (its position in the role; default: every \
                 field of the role)}",
        run: crate::slicer_sync::visual_unbind,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Visual, "Data", "Remove field", "minus"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::req("role", K::Name(Obj::DataRole)),
            P::opt("field", K::Field),
            P::opt("index", K::Integer),
        ],
    },
    Command {
        id: "visual.move_field",
        label: "Move a field of a visual inside its data role, or to another role",
        params: "{page, visual, role, field | index (its position in the role), to_role? (default: \
                 the same role), to_index? (position in the target role, default last), force?: \
                 bool}: one step, as dragging a field in the Build pane",
        run: crate::slicer_sync::visual_move_field,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Visual, "Data", "Move field", "sort"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::req("role", K::Name(Obj::DataRole)),
            P::opt("field", K::Field),
            P::opt("index", K::Integer),
            P::opt("to_role", K::Name(Obj::DataRole)),
            P::opt("to_index", K::Integer),
            P::flag("force", false),
        ],
    },
    Command {
        id: "visual.set_aggregation",
        label: "Summarize a column field of a visual (Sum, Average, Count…) or show it as is",
        params: "{page, visual, role, field | index (its position in the role), aggregation: Sum | \
                 Avg | Count | Min | Max | CountNonNull | Median | StandardDeviation | Variance | \
                 null (don't summarize), force?: bool}",
        run: crate::visual_fields::visual_set_aggregation,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Visual, "Data", "Summarize", "measure"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::req("role", K::Name(Obj::DataRole)),
            P::opt("field", K::Field),
            P::opt("index", K::Integer),
            P::req("aggregation", K::Enum(AGGREGATION_NAMES)),
            P::flag("force", false),
        ],
    },
    Command {
        id: "visual.set_type",
        label: "Change a visual's type; its fields move to the new type's data roles",
        params: "{page, visual, visual_type}: each field keeps its role when the new type has it, \
                 else goes to the first role that takes it and has room; fields no role takes are \
                 dropped (`dropped`); formatting is kept. One of BIcraft's visuals (by name or guid) \
                 takes its own roles and is embedded on first use, as with visual.add (`installed`); \
                 a Spec visual without a spec starts with visual.spec_template's (`spec_template`). \
                 Returns {fields, moved, dropped, warnings}",
        run: crate::visual_fields::visual_set_type,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Visual, "Type", "Change type", "chart"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::req("visual_type", K::Name(Obj::VisualType)),
        ],
    },
    Command {
        id: "visual.rename_field",
        label: "Rename a field for one visual (its display name in titles, axes and headers)",
        params: "{page, visual, role, field? | index?, name: text | null}: the binding is named by its \
                 field text or its position in the role; name null or \"\" restores the field's own \
                 name. Written as the projection's displayName. Returns {fields, renamed}",
        run: crate::visual_fields::visual_rename_field,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Rename for this visual", "rename"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::req("role", K::Name(Obj::DataRole)),
            P::opt("field", K::Field),
            P::opt("index", K::Integer),
            P::req("name", K::Text),
        ],
    },
    Command {
        id: "visual.show_value_as",
        label: "Show a visual's value field as a percentage of the grand total, or of its matrix column or row total",
        params: "{page, visual, role, field? | index?, as: none | percent_of_grand_total | \
                 percent_of_column_total | percent_of_row_total}: the field is a measure or a \
                 summarized column; column and row totals need a matrix with a column in Columns / \
                 Rows. Written as the projection's Arithmetic(field / ScopedEval(field, scope)). \
                 Returns {fields, as}",
        run: crate::visual_fields::visual_show_value_as,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Show value as", "measure"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::req("role", K::Name(Obj::DataRole)),
            P::opt("field", K::Field),
            P::opt("index", K::Integer),
            P::req("as", K::Enum(crate::visual_fields::SHOW_VALUE_AS)),
        ],
    },
    Command {
        id: "visual.show_items_no_data",
        label: "Show or hide the items with no data of a visual's category role",
        params: "{page, visual, role, show: bool}: with show, the role's category values are listed \
                 even where every value is blank (Desktop's \"Show items with no data\"). Written as \
                 the role's showAll in the visual's query. Returns {role, show}",
        run: crate::visual_fields::visual_show_items_no_data,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Show items with no data", "list"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::req("role", K::Name(Obj::DataRole)),
            P::req("show", K::Bool),
        ],
    },
    Command {
        id: "visual.set_data_format",
        label: "Set a field's data format for one visual (Format pane › General › Data format), without changing the model",
        params: "{page, visual, role, field? | index?, type?: auto | general | whole | decimal | \
                 percentage | currency | scientific | date | custom, format?: custom format string \
                 | null, decimals? (0-15, default 2), thousands? (default true, false for a \
                 percentage), currency_symbol? (default $), date_format? (default Short Date; \
                 Short Date, Long Date, General Date, Long Time, Short Time are written d, D, G, \
                 T, t)}: written as the projection's visual-scoped `format` (public PBIR schema) \
                 in the strings Power BI Desktop's card writes (G, 0.00E+000, a three-section $ or \
                 € currency); auto or null removes it and the model's format applies again. The visual's values are displayed with it. Returns \
                 {field, format, fields}",
        run: crate::sparklines::visual_set_data_format,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Visual, "Data", "Data format", "format"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::req("role", K::Name(Obj::DataRole)),
            P::opt("field", K::Field),
            P::opt("index", K::Integer),
            P::opt("type", K::Enum(crate::sparklines::FORMAT_TYPES)),
            P::opt("format", K::Text),
            P::opt("decimals", K::Integer),
            P::opt("thousands", K::Bool),
            P::opt("currency_symbol", K::Text),
            P::opt("date_format", K::Text),
        ],
    },
    Command {
        id: "visual.add_sparkline",
        label: "Add a sparkline to a table or matrix: a small line or column chart of a measure over a column, in each row",
        params: "{page, visual, y: measure or column, summarization? (a column's: Sum by default, \
                 Average, Min, Max, Count, CountNonNull, Median…), x: Table[Column] (typically a \
                 date), apply_calculation_group_to?: point (default, as Desktop's Add dialog) | \
                 sparkline | null, chart_type?: line | column, color?, width?, markers?: [all, \
                 highest, lowest, first, last, negative, isolated] (none given: isolated points \
                 only), marker_color?, \
                 marker_shape?, marker_size?}: tables and matrices take up to 5 sparklines of up to \
                 52 points. Written as a SparklineData projection in Values and an \
                 objects.sparklines entry selected by its queryRef. Returns {index, query_ref, \
                 fields, warnings}",
        run: crate::sparklines::visual_add_sparkline,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Insert, "Sparklines", "Add a sparkline", "chart"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::req("y", K::Field),
            P::opt("summarization", K::Enum(crate::sparklines::SUMMARIZATIONS)),
            P::req("x", K::Field),
            P::opt("apply_calculation_group_to", K::Enum(&["point", "sparkline"])),
            P::opt("chart_type", K::Enum(crate::sparklines::CHART_TYPES)),
            P::opt("color", K::Color),
            P::opt("width", K::Number),
            P::opt("markers", K::List),
            P::opt("marker_color", K::Color),
            P::opt("marker_shape", K::Enum(crate::sparklines::MARKER_SHAPES)),
            P::opt("marker_size", K::Number),
        ],
    },
    Command {
        id: "visual.edit_sparkline",
        label: "Edit a sparkline of a table or matrix: its Y axis, summarization, X axis, chart type, colour, width and markers",
        params: "{page, visual, index? (among the visual's sparklines; needed when it has several), y?, \
                 summarization?, x?, apply_calculation_group_to?, chart_type?, color?, width?, \
                 markers?, marker_color?, marker_shape?, marker_size?}: the sparkline keeps its \
                 queryRef (its look stays with it); null removes a look property. Remove a \
                 sparkline with visual.unbind. Returns {index, query_ref, changes, fields}",
        run: crate::sparklines::visual_edit_sparkline,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Edit sparkline", "chart"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::opt("index", K::Integer),
            P::opt("y", K::Field),
            P::opt("summarization", K::Enum(crate::sparklines::SUMMARIZATIONS)),
            P::opt("x", K::Field),
            P::opt("apply_calculation_group_to", K::Enum(&["point", "sparkline"])),
            P::opt("chart_type", K::Enum(crate::sparklines::CHART_TYPES)),
            P::opt("color", K::Color),
            P::opt("width", K::Number),
            P::opt("markers", K::List),
            P::opt("marker_color", K::Color),
            P::opt("marker_shape", K::Enum(crate::sparklines::MARKER_SHAPES)),
            P::opt("marker_size", K::Number),
        ],
    },
    Command {
        id: "visual.calculation.add",
        label: "Add a visual calculation to a visual: a DAX expression over the visual matrix (RUNNINGSUM, MOVINGAVERAGE, PREVIOUS, COLLAPSE…)",
        params: "{page, visual, expression, name? (default Calculation, Calculation 2…), role? (default the \
                 visual's value role), hidden?, force?}: the expression refers to the visual's fields by \
                 name in brackets ([Sum of Amount]) and to its axes (ROWS, COLUMNS, ROWS COLUMNS); it is \
                 bound against them first (force keeps an invalid one, its diagnostics in warnings). \
                 Written as a NativeVisualCalculation projection. Returns {name, expression, role, \
                 hidden, fields, warnings}",
        run: crate::visual_calcs::add,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::Visual, "New calculation", "calculator"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::req("expression", K::Dax),
            P::opt("name", K::Text),
            P::opt("role", K::Name(Obj::DataRole)),
            P::flag("hidden", false),
            P::flag("force", false),
        ],
    },
    Command {
        id: "visual.calculation.set",
        label: "Change a visual calculation's expression or name, or hide or show any field of a visual",
        params: "{page, visual, name, expression?, new_name?, hidden?, force?}: name is a visual \
                 calculation's name or the name of a field of the visual; expression and new_name apply to \
                 calculations only (a calculation others refer to keeps its name); hidden keeps the field \
                 in the visual matrix but not on the visual (the projection's hidden). Returns {name, \
                 expression, role, hidden, fields, warnings}",
        run: crate::visual_calcs::set,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Edit visual calculation", "calculator"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::req("name", K::Text),
            P::opt("expression", K::Dax),
            P::opt("new_name", K::Text),
            P::opt("hidden", K::Bool),
            P::flag("force", false),
        ],
    },
    Command {
        id: "visual.calculation.remove",
        label: "Remove a visual calculation from a visual",
        params: "{page, visual, name, force?}: refused while another calculation of the visual refers to it \
                 (force removes it anyway). Returns {removed, fields}",
        run: crate::visual_calcs::remove,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Delete visual calculation", "delete"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::req("name", K::Text),
            P::flag("force", false),
        ],
    },
    Command {
        id: "visual.bind_hierarchy",
        label: "Bind a whole hierarchy to a visual's role, one projection per level",
        params: "{page, visual, role, hierarchy, table?, levels?: [level names] (default all, in \
                 order), index?, force?}: each level becomes a HierarchyLevel projection (queryRef \
                 Table.Hierarchy.Level, as Desktop writes it); in a drill role (chart axes, matrix \
                 rows) the first level is active. The role must take columns. Returns {fields, \
                 levels, warnings}",
        run: crate::slicer_sync::visual_bind_hierarchy,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Add hierarchy", "hierarchy"),
        shortcut: None,
        enabled: &[Need::Report, Need::Model],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::req("role", K::Name(Obj::DataRole)),
            P::req("hierarchy", K::Name(Obj::Hierarchy)),
            P::opt("table", K::Name(Obj::Table)),
            P::opt("levels", K::List),
            P::opt("index", K::Integer),
            P::flag("force", false),
        ],
    },
    Command {
        id: "textbox.add",
        label: "Insert a text box (Insert › Text box)",
        params: "{page, x?, y?, width?, height? (default 280 x 110, centred), paragraphs? | text?}: \
                 paragraphs as textbox.set; text is plain text, one paragraph per line. Returns \
                 {page, visual, position, warnings}",
        run: crate::rich::textbox_add,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Insert, "Elements", "Text box", "text-box"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::opt("x", K::Number),
            P::opt("y", K::Number),
            P::opt("width", K::Number),
            P::opt("height", K::Number),
            P::opt("paragraphs", K::Json),
            P::opt("text", K::Text),
        ],
    },
    Command {
        id: "textbox.set",
        label: "Set a text box's rich text: paragraphs of runs with fonts, colours, links and dynamic values",
        params: "{page, visual, paragraphs: [{align?: left|center|right, list?: bullet|number|none, \
                 runs: [{text | value: \"[Measure]\" | \"Sum(Table[Column])\", bold?, italic?, \
                 underline?, font_family?, font_size? (points), color? (#RRGGBB), url? (http, https, \
                 mailto)}]}] | text: plain text}: written as Desktop's paragraphs / textRuns, a value \
                 as a dynamic value (objects.values). Replaces the whole text. Returns {visual, \
                 paragraphs, values, changed, warnings}",
        run: crate::rich::textbox_set,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::Visual, "Edit text", "edit"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::opt("paragraphs", K::Json),
            P::opt("text", K::Text),
        ],
    },
    Command {
        id: "image.add",
        label: "Insert an image stored in the report (Insert › Image)",
        params: "{page, path | data (base64 or a data: URL), name?, x?, y?, width?, height?, scaling?: \
                 normal|fit|fill}: PNG, JPEG, GIF, WebP or SVG by content, at most 16 MB, decoded \
                 before it is stored; it becomes a RegisteredResources item (StaticResources/\
                 RegisteredResources/<name><digits>.<ext>) the image visual names. Default size: \
                 the natural size within half the page, centred. Returns {page, visual, resource, \
                 mime, image_size, position, warnings}",
        run: crate::rich::image_add,
        server: ServerPolicy::paths(&[PathParam::one("path")]),
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Insert, "Elements", "Image", "image"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::opt("path", K::Path),
            P::opt("data", K::Text),
            P::opt("name", K::Text),
            P::opt("x", K::Number),
            P::opt("y", K::Number),
            P::opt("width", K::Number),
            P::opt("height", K::Number),
            P::opt("scaling", K::Enum(&["normal", "fit", "fill"])),
        ],
    },
    Command {
        id: "image.set",
        label: "Replace an image visual's picture or change its scaling",
        params: "{page, visual, path? | data?, name?, scaling?: normal|fit|fill}: a new picture is \
                 checked and stored as for image.add; the old one is dropped when nothing else shows \
                 it. Returns {visual, resource, changed, warnings}",
        run: crate::rich::image_set,
        server: ServerPolicy::paths(&[PathParam::one("path")]),
        replay: Replay::Exact,
        place: Place::palette("Change image", "image"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::opt("path", K::Path),
            P::opt("data", K::Text),
            P::opt("name", K::Text),
            P::opt("scaling", K::Enum(&["normal", "fit", "fill"])),
        ],
    },
    Command {
        id: "shape_map.set_map",
        label: "Add a custom map to a shape map (Format › Map settings › Add a map type)",
        params: "{page, visual, path? | data? (the TopoJSON or GeoJSON text, base64 or a data: URL) | \
                 builtin? (`world_countries`: BIcraft's built-in map of Natural Earth's countries, \
                 without Antarctica, keyed by id = ISO alpha-3 and the properties name, iso_a2, iso_a3, \
                 name_long, admin, formal_en, name_fr; written into the report as an ordinary custom \
                 TopoJSON map that Desktop opens), \
                 name? (the file name shown), remove?: bool}: the map is checked (at most 16 MB, \
                 10,000 shapes, 2 million points; polygons and multipolygons) and stored as a \
                 ShapeMap item of the RegisteredResources package; the visual's `shape` object names \
                 it (`map.geoJson`, `datasourceType` file_upload). remove: true drops the custom map. \
                 Locations match a shape's id or any of its property values, ignoring case \
                 (docs/formats/maps.md). Returns {visual, resource, shapes, points, keys (the first \
                 key of the first 50 shapes), changed}",
        run: crate::shape_maps::set_map,
        server: ServerPolicy::paths(&[PathParam::one("path")]),
        replay: Replay::Exact,
        place: Place::palette("Add a map type", "image"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::opt("path", K::Path),
            P::opt("data", K::Text),
            P::opt("builtin", K::Enum(&["world_countries"])),
            P::opt("name", K::Text),
            P::opt("remove", K::Bool),
        ],
    },
    Command {
        id: "button.add",
        label: "Insert a button or a navigator (Insert › Buttons)",
        params: "{page, kind: left_arrow|right_arrow|reset|back|information|help|qna|bookmark|blank|\
                 apply_all_slicers|clear_all_slicers|page_navigator|bookmark_navigator, x?, y?, \
                 width?, height?}: an actionButton with the kind's icon (and its action for back, \
                 bookmark, Q&A, apply and clear all slicers), or a pageNavigator / \
                 bookmarkNavigator. Default 100 x 40 (navigators 400 x 60), centred. Returns {page, \
                 visual, kind, action, position, warnings}",
        run: crate::rich::button_add,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Insert, "Elements", "Buttons", "button"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("kind", K::Enum(crate::rich::BUTTON_KINDS)),
            P::opt("x", K::Number),
            P::opt("y", K::Number),
            P::opt("width", K::Number),
            P::opt("height", K::Number),
        ],
    },
    Command {
        id: "shape.add",
        label: "Insert a shape (Insert › Shapes)",
        params: "{page, shape: rectangle|rectangleRounded|rectangleRoundedByPixel|oval|line|\
                 triangleIsoc|triangleRight|arrow|arrowChevron|arrowPentagon|pentagon|hexagon|\
                 octagon|parallelogram|trapezoid|pill|heart|speechbubbleRectangle|tabCutCorner|\
                 tabCutTopCorners|tabCutTopCornersByPixel|tabRoundCorner|tabRoundTopCorners, x?, \
                 y?, width?, height?}: a shape visual with that tile shape (objects.shape \
                 tileShape); fill, outline and rotation through visual.format. Default 300 x 200 \
                 (a line 300 x 40), centred. Returns {page, visual, shape, position, warnings}",
        run: crate::rich::shape_add,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Insert, "Elements", "Shapes", "shapes"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("shape", K::Enum(crate::rich::SHAPE_TYPES)),
            P::opt("x", K::Number),
            P::opt("y", K::Number),
            P::opt("width", K::Number),
            P::opt("height", K::Number),
        ],
    },
    Command {
        id: "visual.set_action",
        label: "Set the action of a button, image or shape (Format › Action)",
        params: "{page, visual, type: none|back|bookmark|drillthrough|page_navigation|qna|web_url|\
                 apply_all_slicers|clear_all_slicers, destination? (a page; a drillthrough page for \
                 drillthrough), bookmark?, url? (http, https, mailto), tooltip?, destination_field? \
                 (fx: a measure or a column, read through Min, whose value names the page, the \
                 drillthrough page, the bookmark or the URL; instead of destination / bookmark / \
                 url), tooltip_field? (fx tooltip), enabled_tooltip?, disabled_tooltip? (the tooltip \
                 of an enabled or disabled drillthrough button)}: written as \
                 visualContainerObjects.visualLink; none turns the action off. Returns {visual, \
                 type, destination, conditional, changed, warnings}",
        run: crate::rich::visual_set_action,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Visual, "Interactions", "Action", "link"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::req("type", K::Enum(crate::rich::ACTION_TYPES)),
            P::opt("destination", K::Name(Obj::Page)),
            P::opt("bookmark", K::Text),
            P::opt("url", K::Text),
            P::opt("tooltip", K::Text),
            P::opt("destination_field", K::Text),
            P::opt("tooltip_field", K::Text),
            P::opt("enabled_tooltip", K::Text),
            P::opt("disabled_tooltip", K::Text),
        ],
    },
    Command {
        id: "page.format",
        label: "Format a page: canvas background, wallpaper, page size, vertical alignment",
        params: "{page, background_color?, background_transparency? (0-100), background_image? \
                 (base64 or data: URL; null removes) | background_image_path?, background_scaling?: \
                 normal|fit|fill, the same as wallpaper_color, wallpaper_transparency, wallpaper_image | \
                 wallpaper_image_path, wallpaper_scaling, size?: 16:9|4:3|letter|tooltip|custom, \
                 width?, height? (custom), vertical_alignment?: top|middle}: written as the page's \
                 objects.background, outspace and displayArea and its width and height; images are \
                 checked and stored as for image.add. Returns {page, width, height, images, changed, \
                 warnings}",
        run: crate::rich::page_format,
        server: ServerPolicy::paths(&[
            PathParam::clearable("background_image_path"),
            PathParam::clearable("wallpaper_image_path"),
        ]),
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Report, "Pages", "Page format", "palette"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::opt("background_color", K::Color),
            P::opt("background_transparency", K::Number),
            P::opt("background_image_path", K::Path),
            P::opt("background_image", K::Text),
            P::opt("background_scaling", K::Enum(&["normal", "fit", "fill"])),
            P::opt("wallpaper_color", K::Color),
            P::opt("wallpaper_transparency", K::Number),
            P::opt("wallpaper_image_path", K::Path),
            P::opt("wallpaper_image", K::Text),
            P::opt("wallpaper_scaling", K::Enum(&["normal", "fit", "fill"])),
            P::opt("size", K::Enum(crate::rich::PAGE_SIZE_NAMES)),
            P::opt("width", K::Number),
            P::opt("height", K::Number),
            P::opt("vertical_alignment", K::Enum(&["top", "middle"])),
        ],
    },
    Command {
        id: "page.format_filter_pane",
        label: "Format a page's Filters pane or its filter cards (Format pane › Filter pane, Filter cards)",
        params: "{page, card?: pane|available|applied (default pane), reset?: bool (removes the \
                 card's properties first), background_color?, transparency? (0-100), \
                 foreground_color? (font and icon colour), font_family?, border?: bool, \
                 border_color?, input_box_color?; the pane only: title_size?, header_size?, \
                 search_text_size? (6-72), checkbox_color? (checkbox and Apply), width? \
                 (100-1000); the cards only: text_size? (6-72)}: colours as #RRGGBB, null removes \
                 one property. Written as the page's objects.outspacePane and objects.filterCard \
                 (selector id Available or Applied); the theme's outspacePane and filterCard are \
                 the defaults. Returns {page, card, changed, changes, format: {pane, available, \
                 applied}}",
        run: crate::filter_pane_format::page_format_filter_pane,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        // the Format pane's page cards (nothing selected) are its place, as in Desktop
        place: Place::palette("Format Filters pane", "palette"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::opt("card", K::Enum(&["pane", "available", "applied"])),
            P::opt("reset", K::Bool),
            P::opt("background_color", K::Color),
            P::opt("transparency", K::Number),
            P::opt("foreground_color", K::Color),
            P::opt("font_family", K::Text),
            P::opt("border", K::Bool),
            P::opt("border_color", K::Color),
            P::opt("input_box_color", K::Color),
            P::opt("title_size", K::Number),
            P::opt("header_size", K::Number),
            P::opt("search_text_size", K::Number),
            P::opt("checkbox_color", K::Color),
            P::opt("width", K::Number),
            P::opt("text_size", K::Number),
        ],
    },
    Command {
        id: "visual.remove",
        label: "Remove a visual (a group is removed with its members)",
        params: "{page, visual, visuals?: [names] (more visuals removed in the same step)}",
        run: report::visual_remove,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::Visual, "Delete visual", "delete"),
        shortcut: Some("Delete"),
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::opt("visuals", K::List),
        ],
    },
    Command {
        id: "visual.set_title",
        label: "Set, format or remove a visual's title",
        params: "{page, visual, title?, show?, font_size?, color?, alignment?, bold?, italic?, \
                 underline?, font_family?}: title is \"text\" (sets the text and shows it), {text?, \
                 show?, font_size?, color? (#RRGGBB), alignment? (left|center|right), bold?, italic?, \
                 underline?, font_family?} (null removes one property), or null (removes the title \
                 formatting); the properties may also be given flat beside it (with a text, the title \
                 is shown unless show says). Returns {changed, changes, props}",
        run: visual_props::visual_set_title,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        // the Build pane's Format visual tab (General › Title) is its place, as in Desktop
        place: Place::palette("Title", "edit"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            // the text is free text (a JSON field here drew a code box and no title)
            P::opt("title", K::Text),
            P::opt("show", K::Bool),
            P::opt("font_size", K::Number),
            P::opt("color", K::Color),
            P::opt("alignment", K::Enum(&["left", "center", "right"])),
            P::opt("bold", K::Bool),
            P::opt("italic", K::Bool),
            P::opt("underline", K::Bool),
            P::opt("font_family", K::Text),
        ],
    },
    Command {
        id: "visual.set_sort",
        label: "Sort a visual by a field, or remove its sort",
        params: "{page, visual, field: \"Table[Column]\"|\"[Measure]\"|\"Sum(Table[Column])\"|null, \
                 direction?: \"asc\"|\"desc\" (default desc for measures and aggregations, asc for \
                 columns)}: the field must be bound to the visual or exist in the model; null removes \
                 the sort",
        run: visual_props::visual_set_sort,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::Visual, "Sort", "sort"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::req("field", K::Field),
            P::opt("direction", K::Enum(&["asc", "desc"])),
        ],
    },
    Command {
        id: "visual.records",
        label: "See records: the rows behind a data point of a visual",
        params: "{page, visual, point?: [{c, v}] (the data point as view.interact `select` takes it: a data \
                 column of the visual and its value; none: every row), points?: [[{c, v}]] (a mark that \
                 stands for several points, as view.interact `select` takes them: the rows of every one, at \
                 most 1000), limit? (1-10000, default 1000)}: the \
                 rows of the table the visual summarizes (its first measure's home table, else its first \
                 aggregated or grouping column's table) under the point's values, through the model's \
                 relationships and the report, page and visual filters and the page's slicer selections \
                 (the reader's view state included), computed by bicraft's DAX evaluator (EVALUATE \
                 CALCULATETABLE). A filter on a measure keeps the points it keeps (FILTER over the \
                 visual's grouping columns, under the other filters); a condition over several tables \
                 reads them through RELATED; when another visual's selection cross-highlights this one, \
                 the rows of the highlighted part. What cannot be applied is listed in not_applied. \
                 Returns {page, visual, table, point: [{table, column, value}] (the first), points (how \
                 many), query, columns: [{name, \
                 data_type}], rows: [[values]], row_count, truncated, filters_applied, filters: [DAX \
                 filter arguments], not_applied: [reasons], cross_highlight (the source visual, or null)}",
        run: |s, p| crate::see_records::visual_records(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::menu(Menu::Visual, "See records", "table"),
        shortcut: None,
        enabled: &[Need::Model, Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::opt("point", K::Json),
            P::opt("points", K::Json),
            P::opt("limit", K::Integer),
        ],
    },
    Command {
        id: "visual.filter",
        label: "Add or replace a visual-level filter on a field",
        params: "{page, visual, field, then one of: in: [values] | not_in: [values] (basic, columns) | \
                 op: \"=\"|\"<>\"|\">\"|\">=\"|\"<\"|\"<=\"|\"contains\"|\"not_contains\"|\"starts_with\"|\
                 \"not_starts_with\", value (advanced; value null = blank) | conditions: [{op, value}] \
                 (up to 2), logic?: \"and\"|\"or\" | top: N, by: \"[Measure]\"|\"Sum(T[C])\", \
                 direction?: \"top\"|\"bottom\" (top N) | relative: {range: last|next|this, count?, \
                 unit: days|weeks|months|years|calendar_weeks|calendar_months|calendar_years|minutes|\
                 hours, include_today?} (relative date or time, date columns); hidden?, locked? (in \
                 view mode; without a condition they set the card's lock and eye, the condition kept)}: values are \
                 typed by the column's data type; dates as 2024-01-31. Replaces the visual's filter on \
                 the same field; without a condition, an empty filter card on the field \
                 (Filters pane)",
        run: visual_props::visual_filter,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Visual, "Filters", "Filter", "filter"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::req("field", K::Field),
            P::opt("in", K::List),
            P::opt("not_in", K::List),
            P::opt("op", K::Enum(FILTER_OPS)),
            P::opt("value", K::Json),
            P::opt("conditions", K::List),
            P::opt("logic", K::Enum(&["and", "or"])),
            P::opt("top", K::Integer),
            P::opt("by", K::Field),
            P::opt("direction", K::Enum(&["top", "bottom"])),
            P::opt("relative", K::Json),
            P::opt("hidden", K::Bool),
            P::opt("locked", K::Bool),
        ],
    },
    Command {
        id: "visual.clear_filters",
        label: "Remove a visual's filters (all, or those on one field)",
        params: "{page, visual, field?, keep_cards?: bool (default false)}: without field, every visual-level filter goes; \
                 automatic filter cards of bound fields stay, without condition; keep_cards keeps every \
                 card and clears its condition (Desktop's Clear filter)",
        run: visual_props::visual_clear_filters,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Visual, "Filters", "Clear filters", "close"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::opt("field", K::Field),
            P::flag("keep_cards", false),
        ],
    },
    Command {
        id: "page.filter",
        label: "Add or replace a filter on a page (Filters on this page), or add an empty filter card",
        params: "{page, field: \"Table[Column]\", then one of: in: [values] | not_in: [values] (basic) | \
                 op, value | conditions: [{op, value}] (up to 2), logic?: \"and\"|\"or\" (advanced) | \
                 relative: {range, count?, unit, include_today?} (relative date or time), as \
                 visual.filter; without a condition, an empty filter card on the field (Desktop's \
                 field dropped on the Filters pane); hidden?, locked? (in view mode; without a \
                 condition they set the card's lock and eye)}: columns only (measures and top N are \
                 filtered on a visual, as in Desktop). Replaces the filter on the same field. Returns \
                 {changed, changes, filters: [cards]}",
        run: crate::filter_edit::page_filter,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Report, "Filters", "Page filter", "filter"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("field", K::Field),
            P::opt("in", K::List),
            P::opt("not_in", K::List),
            P::opt("op", K::Enum(FILTER_OPS)),
            P::opt("value", K::Json),
            P::opt("conditions", K::List),
            P::opt("logic", K::Enum(&["and", "or"])),
            P::opt("relative", K::Json),
            P::opt("hidden", K::Bool),
            P::opt("locked", K::Bool),
        ],
    },
    Command {
        id: "page.clear_filters",
        label: "Clear or remove the filters on a page (Filters on this page) (all, or those on one field)",
        params: "{page, field?, keep_cards?: bool (default false)}: keep_cards clears the conditions \
                 and keeps the cards (Desktop's Clear filter); else cards go (Remove filter), automatic \
                 ones stay without condition. Returns {changed, changes, filters: [cards]}",
        run: crate::filter_edit::page_clear_filters,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Report, "Filters", "Clear page filters", "close"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::opt("field", K::Field),
            P::flag("keep_cards", false),
        ],
    },
    Command {
        id: "report.filter",
        label: "Add or replace a filter on every page (Filters on all pages), or add an empty filter card",
        params: "{field: \"Table[Column]\", then one of: in: [values] | not_in: [values] (basic) | \
                 op, value | conditions: [{op, value}] (up to 2), logic?: \"and\"|\"or\" (advanced) | \
                 relative: {range, count?, unit, include_today?} (relative date or time), as \
                 visual.filter; without a condition, an empty filter card on the field (Desktop's \
                 field dropped on the Filters pane); hidden?, locked? (in view mode; without a \
                 condition they set the card's lock and eye)}: columns only (measures and top N are \
                 filtered on a visual, as in Desktop). Replaces the filter on the same field. Returns \
                 {changed, changes, filters: [cards]}",
        run: crate::filter_edit::report_filter,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Report, "Filters", "Report filter", "filter"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("field", K::Field),
            P::opt("in", K::List),
            P::opt("not_in", K::List),
            P::opt("op", K::Enum(FILTER_OPS)),
            P::opt("value", K::Json),
            P::opt("conditions", K::List),
            P::opt("logic", K::Enum(&["and", "or"])),
            P::opt("relative", K::Json),
            P::opt("hidden", K::Bool),
            P::opt("locked", K::Bool),
        ],
    },
    Command {
        id: "filter.order",
        label: "Order the filter cards of a section of the Filters pane: move a card, or sort A to Z, Z to A or custom",
        params: "{level: visual|page|report, page? (visual and page levels), visual? (visual level), \
                 field? and to? (move the card on that field to that position, 0 first, of the \
                 cards as the pane shows them: the custom order), sort?: ascending|descending|custom}: \
                 written as filterConfig.filterSortOrder (Ascending, Descending, Custom) and, in \
                 custom order, every card's ordinal (0, 1, …), as the public filterConfiguration \
                 schema defines them; without filterSortOrder the pane lists cards A to Z. Returns \
                 {changed, changes, filters: [cards in their new order]}",
        run: crate::filter_edit::filter_order,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        // a card dragged in its section, and each section's sort list, are its place, as in Desktop
        place: Place::palette("Order filter cards", "sort"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("level", K::Enum(&["visual", "page", "report"])),
            P::opt("page", K::Name(Obj::Page)),
            P::opt("visual", K::Name(Obj::Visual)),
            P::opt("field", K::Field),
            P::opt("to", K::Integer),
            P::opt("sort", K::Enum(&["ascending", "descending", "custom"])),
        ],
    },
    Command {
        id: "report.clear_filters",
        label: "Clear or remove the filters on every page (Filters on all pages) (all, or those on one field)",
        params: "{field?, keep_cards?: bool (default false)}: keep_cards clears the conditions \
                 and keeps the cards (Desktop's Clear filter); else cards go (Remove filter), automatic \
                 ones stay without condition. Returns {changed, changes, filters: [cards]}",
        run: crate::filter_edit::report_clear_filters,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Report, "Filters", "Clear report filters", "close"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[P::opt("field", K::Field), P::flag("keep_cards", false)],
    },
    Command {
        id: "visual.format",
        label: "Format a visual: background, border, padding, shadow, header, alt text, subtitle, data labels, legend, axes, or any formatting property",
        params: "{page, visual, background?: bool|{show?, color?, transparency? (0-100)}, border?: \
                 bool|{show?, color?, radius?, width?}, padding?: number|{top?, bottom?, left?, right?}, \
                 shadow?: bool|{show?, color?, transparency?}, header?: bool, alt_text?: string, \
                 subtitle?: string|{text?, show?, font_size?, color?, alignment?}, data_labels?: \
                 bool|{show?, font_size?, color?}, legend?: bool|{show?, position? \
                 (Top|Bottom|Left|Right|TopCenter|…), font_size?}, x_axis? / y_axis?: bool|{show?, \
                 title?: bool, font_size?, color?}, objects?: [{object, property, value, selector?, \
                 state?, target?: \"visual\"|\"container\"}]}: null removes a property (or a whole \
                 group); generic values are bool, number (12D), string ('text'), {color}, {literal: \
                 \"12L\"} or a PBIR {expr}/{solid} value. Stateful objects (new card, new slicers, \
                 buttons, navigators, shapes) go to the `default` state entry (selector {id: \
                 \"default\"}), the one Desktop draws; state: \"hover\"|\"press\"|… targets another \
                 state, \"none\" the entry without selector",
        run: visual_props::visual_format,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Visual, "Format", "Format", "format"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::opt("background", K::Json),
            P::opt("border", K::Json),
            P::opt("padding", K::Json),
            P::opt("shadow", K::Json),
            P::opt("header", K::Bool),
            P::opt("alt_text", K::Text),
            P::opt("subtitle", K::Json),
            P::opt("data_labels", K::Json),
            P::opt("legend", K::Json),
            P::opt("x_axis", K::Json),
            P::opt("y_axis", K::Json),
            P::opt("objects", K::List),
        ],
    },
    Command {
        id: "visual.props",
        label: "Read a visual's title, sort, filters and formatting as visual props",
        params: "{page, visual, layout?: \"desktop\"|\"mobile\"}: returns {props}, in the shape \
                 visual.set_title / set_sort / filter / format and report.apply accept; layout \
                 \"mobile\" reads them as the phone layout draws them (the mobile formatting over \
                 the visual's) and lists the formatted objects (mobile_objects)",
        run: |s, p| visual_props::visual_props(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::palette("Visual properties (read)", "info"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::opt("layout", K::Enum(&["desktop", "mobile"])),
        ],
    },
    Command {
        id: "visual.axis",
        label: "Format a chart axis: range, log scale, gridlines, title, display units, labels",
        params: "{page, visual, axis: \"x\"|\"y\"|\"y2\", show?, title?: bool|\"text\", title_text?, min?, \
                 max?, start_at_zero?: true, scale?: \"linear\"|\"log\", gridlines?: bool|{show?, color?, \
                 width?, style?: solid|dashed|dotted}, display_units?: \
                 auto|none|thousands|millions|billions|trillions, precision? (0-15), font_size? (8-60), \
                 color?, invert?, type?: \"categorical\"|\"continuous\" (x), clear?: true}: x is categoryAxis, y valueAxis, y2 the secondary axis \
                 (combo, line and area charts); null removes one setting, clear removes them all. \
                 Returns {changed, changes, props}",
        run: crate::visual_extras::visual_axis,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Axes", "chart"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::req("axis", K::Enum(&["x", "y", "y2"])),
            P::flag("clear", false),
            P::opt("show", K::Bool),
            P::opt("title", K::Json),
            P::opt("title_text", K::Text),
            P::opt("min", K::Number),
            P::opt("max", K::Number),
            P::opt("start_at_zero", K::Bool),
            P::opt("scale", K::Enum(&["linear", "log"])),
            P::opt("gridlines", K::Json),
            P::opt(
                "display_units",
                K::Enum(&["auto", "none", "thousands", "millions", "billions", "trillions"]),
            ),
            P::opt("precision", K::Integer),
            P::opt("font_size", K::Number),
            P::opt("color", K::Color),
            P::opt("invert", K::Bool),
            P::opt("type", K::Enum(&["categorical", "continuous"])),
            P::opt("label_angle", K::Number),
            P::opt("ticks", K::Json),
        ],
    },
    Command {
        id: "visual.analytics",
        label: "Set a chart's analytics: constant, average, min, max, median, percentile, trend and forecast lines, error bars, a line chart's anomalies, a scatter's ratio line and symmetry shading",
        params: "{page, visual, lines: [{kind: constant|average|min|max|median|percentile|trend|forecast|\
                 ratio_line, value? (constant), axis?: \"x\"|\"y\" (constant, default y), measure? (statistical \
                 lines and forecast: the series, default the first measure in Y), percentile? (0-100), label?, \
                 color?, style?: solid|dashed|dotted, width?, transparency? (0-100), position?: front|back, \
                 data_label?: bool, show?, length?, ignore_last?, confidence? (0-1), seasonality? (forecast)} | \
                 {kind: error_bars, measure?, upper?, lower? (a measure or an aggregation; one at least), \
                 relative?: bool, show?, bar?, color?, match_series?, width?, marker?, marker_shape?, band?, \
                 band_style?: fill|line|fillLine, band_color?, transparency?, data_label?, tooltip?} | \
                 {kind: symmetry_shading, show?, label?, upper_color?, lower_color?, transparency?} | \
                 {kind: anomaly_detection (line charts, one value, no legend), measure?, show?, label?, \
                 sensitivity? (0-100), marker?, marker_shape?: circle|square|diamond|triangle|droplet, \
                 marker_size?, marker_color?, marker_transparency?, band?, band_style?: fill|line|none, \
                 band_color?, transparency?} | \
                 {object, raw}] | null}: the complete list (null or [] removes every line); lines keep their \
                 Desktop ids and unmodelled settings. Returns {changed, changes, props}",
        run: crate::visual_extras::visual_analytics,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Visual, "Analytics", "Analytics lines", "chart"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::req("lines", K::List),
        ],
    },
    Command {
        id: "visual.small_multiples",
        label: "Split a chart into small multiples by a column, with grid size and headers",
        params: "{page, visual, field?: \"Table[Column]\"|null, rows? (1-6), columns? (1-6), title?: \
                 bool|{show?, font_size?, color?, alignment?, position?: top|bottom}}: the field is bound \
                 to the Rows role; `field: null` alone removes small multiples (field, grid, headers)",
        run: crate::visual_extras::visual_small_multiples,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Visual, "Data", "Small multiples", "grid"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::opt("field", K::Field),
            P::opt("rows", K::Integer),
            P::opt("columns", K::Integer),
            P::opt("title", K::Json),
        ],
    },
    Command {
        id: "visual.tooltip_page",
        label: "Show a report page as a visual's tooltip, or restore / hide the default tooltip",
        params: "{page, visual, tooltip_page: page name or display name | \"default\" | \"none\" | null}: \
                 warns when the page is not a tooltip page (page.set_type)",
        run: crate::visual_extras::visual_tooltip_page,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Tooltip page", "page"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::req("tooltip_page", K::Name(Obj::Page)),
        ],
    },
    Command {
        id: "page.set_type",
        label: "Make a page a tooltip page, a drillthrough page, or a normal report page",
        params: "{page, type: \"report\"|\"tooltip\"|\"drillthrough\", fields?: [\"Table[Column]\" | \
                 \"Sum(Table[Column])\"] (the drillthrough / tooltip fields, complete list), \
                 keep_all_filters?: bool, cross_report?: bool (drillthrough), back_button?: bool \
                 (drillthrough: adds a Back button; false removes it), width?, height?, hidden?: bool}: \
                 a page becoming a tooltip page is sized 320 x 240 unless width/height are given",
        run: crate::visual_extras::page_set_type,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::Page, "Page type", "page"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("type", K::Enum(&["report", "tooltip", "drillthrough"])),
            P::opt("fields", K::List),
            P::opt("keep_all_filters", K::Bool),
            P::opt("cross_report", K::Bool),
            P::opt("back_button", K::Bool),
            P::opt("width", K::Integer),
            P::opt("height", K::Integer),
            P::opt("hidden", K::Bool),
        ],
    },
    Command {
        id: "page.interactions",
        label: "Set how selecting data in one visual affects another (filter, highlight, none)",
        params: "{page, rules: [{source, target, type: \"filter\"|\"highlight\"|\"none\"|\"default\"}], \
                 clear?: bool}: rules merge into the page's interactions (\"default\" removes a rule; \
                 clear starts from none); a visual group as source or target stands for each \
                 visual inside it. Returns the page's interactions",
        run: crate::visual_extras::page_interactions,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Visual, "Interactions", "Edit interactions", "link"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("rules", K::List),
            P::flag("clear", false),
        ],
    },
    Command {
        id: "slicer.sync_list",
        label: "List the report's slicers, their sync groups and what each synchronizes",
        params: "{}: the slicers of every page with their label, group and Fields / Filters \
                 options, and the groups they form",
        run: crate::slicer_sync::slicer_sync_list,
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::palette("Synchronized slicers", "link"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[],
    },
    Command {
        id: "slicer.set_sync",
        label: "Synchronize slicers across pages: put them in a group or take them out",
        params: "{segments: [{page, visual}] | page and visual, group: a group name or null \
                 (leave the group), field_changes?: bool (default true), filter_changes?: bool \
                 (default true)}: writes the slicers' syncGroup. Returns {segments, group, \
                 field_changes, filter_changes, changed}",
        run: crate::slicer_sync::slicer_set_sync,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Sync slicers", "link"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::opt("segments", K::List),
            P::opt("page", K::Name(Obj::Page)),
            P::opt("visual", K::Name(Obj::Visual)),
            P::req("group", K::Text),
            P::flag("field_changes", true),
            P::flag("filter_changes", true),
        ],
    },
    Command {
        id: "bookmark.list",
        label: "List the report's bookmarks and groups, what each captured",
        params: "{}: the bookmarks in their saved order with their display name, group, Data / \
                 Display / Current page options, the visuals they target, the page and the counts \
                 of hidden visuals and filters they hold",
        run: crate::bookmarks::bookmark_list,
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::palette("Bookmarks", "list"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[],
    },
    Command {
        id: "bookmark.capture",
        label: "Capture the current state as a bookmark (Desktop's Add bookmark)",
        params: "{display_name, group?: a group's name or display name, data?: bool (default \
                 true), display?: bool (default true), current_page?: bool (default false), \
                 visuals?: [names of the active page] (a bookmark of those visuals), key?: string}: \
                 writes definition/bookmarks/<name>.bookmark.json (page, hidden visuals, filters) \
                 and lists it in bookmarks.json. Returns {bookmark, display_name, group, page}",
        run: crate::bookmarks::bookmark_capture,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::View, "Bookmarks", "Add bookmark", "pin"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("display_name", K::Text),
            P::opt("group", K::Text),
            P::flag("data", true),
            P::flag("display", true),
            P::flag("current_page", false),
            P::opt("visuals", K::List),
            P::opt("key", K::Text),
        ],
    },
    Command {
        id: "bookmark.update",
        label: "Capture the current state into an existing bookmark",
        params: "{bookmark}: rewrites the bookmark's explorationState (page, hidden visuals, \
                 filters), keeping its name, display name and options. Returns {bookmark, changed, \
                 page}",
        run: crate::bookmarks::bookmark_update,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Update bookmark", "reload"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[P::req("bookmark", K::Text)],
    },
    Command {
        id: "bookmark.set",
        label: "Rename a bookmark, change what it applies or move it",
        params: "{bookmark, display_name?, data?, display?, current_page?, visuals?: [names]|null, \
                 group?: group name or null (out of its group), index?: 0-based position}: the \
                 document's display name and options, and the bookmark's place in bookmarks.json. \
                 Returns {bookmark, changed}",
        run: crate::bookmarks::bookmark_set,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Edit bookmark", "edit"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("bookmark", K::Text),
            P::opt("display_name", K::Text),
            P::flag("data", true),
            P::flag("display", true),
            P::flag("current_page", false),
            P::opt("visuals", K::List),
            P::opt("group", K::Text),
            P::opt("index", K::Integer),
        ],
    },
    Command {
        id: "bookmark.remove",
        label: "Delete a bookmark, or a group and its bookmarks",
        params: "{bookmark}: a bookmark's name or display name, or a group's (its bookmarks go \
                 with it). Returns {removed, group}",
        run: crate::bookmarks::bookmark_remove,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Delete bookmark", "delete"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[P::req("bookmark", K::Text)],
    },
    Command {
        id: "bookmark.group",
        label: "Group bookmarks in the Bookmarks pane",
        params: "{bookmarks: [names or display names], display_name}: a new group, in the place of \
                 the first listed bookmark. Returns {group, display_name, bookmarks}",
        run: crate::bookmarks::bookmark_group,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Group bookmarks", "group"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[P::req("bookmarks", K::List), P::req("display_name", K::Text)],
    },
    Command {
        id: "bookmark.ungroup",
        label: "Remove a bookmark group, keeping its bookmarks",
        params: "{group: name or display name}: the group goes, its bookmarks take its place. \
                 Returns {group, display_name, bookmarks}",
        run: crate::bookmarks::bookmark_ungroup,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Ungroup bookmarks", "ungroup"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[P::req("group", K::Text)],
    },
    Command {
        id: "bookmark.rename_group",
        label: "Rename a bookmark group in the Bookmarks pane",
        params: "{group: name or display name, display_name}: the group's caption, its bookmarks \
                 keep their names. Returns {group, display_name, changed}",
        run: crate::bookmarks::bookmark_rename_group,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Rename bookmark group", "edit"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[P::req("group", K::Text), P::req("display_name", K::Text)],
    },
    Command {
        id: "bookmark.apply",
        label: "Apply a bookmark: its page, hidden visuals and filters (Desktop's View)",
        params: "{bookmark}: restores what the bookmark captured, as its Data / Display / Current \
                 page options allow, in one undo step. Returns {bookmark, changed, active_page, \
                 page_changed, hidden, filters_changed, warnings}",
        run: crate::bookmarks::bookmark_apply,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::View, "Bookmarks", "View bookmark", "play"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[P::req("bookmark", K::Text)],
    },
    Command {
        id: "report.settings",
        label: "Set the report's settings: interaction defaults (cross-highlight or cross-filter, drilling filters other visuals), what readers may export, persistent filters, personalize visuals, cross-report drillthrough, the Filters pane's visibility to readers and whether readers may change filter types",
        params: "{default_interaction?: \"highlight\"|\"filter\", drill_filters_other_visuals?: bool, \
                 export_data?: \"AllowSummarized\"|\"AllowSummarizedAndUnderlying\"|\"None\", \
                 persistent_filters?: bool, personalize_visuals?: bool, cross_report_drillthrough?: bool, \
                 filter_pane_visible?: bool, allow_change_filter_types?: bool}: Desktop's Report settings \
                 of the current file, in report.json settings (docs/formats/pbir.md, Report settings): \
                 \"filter\" is \"Change default visual interaction from cross highlighting to cross \
                 filtering\" (defaultFilterActionIsDataFilter); export_data is the Export data setting \
                 (exportDataMode; a report without one allows summarized data), which visual.export_data \
                 honours; persistent_filters false is \"Don't allow end users to save filters\" \
                 (isPersistentUserStateDisabled); personalize_visuals is allowInlineExploration; \
                 cross_report_drillthrough is useCrossReportDrillthrough; allow_change_filter_types is \
                 allowChangeFilterTypes; filter_pane_visible false hides the Filters pane from readers \
                 (objects.outspacePane visible, the pane's eye). Without parameters nothing changes. \
                 Returns {changed, default_interaction, drill_filters_other_visuals, export_data, \
                 persistent_filters, personalize_visuals, cross_report_drillthrough, filter_pane_visible, \
                 allow_change_filter_types}",
        run: crate::report_settings::report_settings,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Visual, "Interactions", "Interaction settings", "settings"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::opt("default_interaction", K::Enum(&["highlight", "filter"])),
            P::opt("drill_filters_other_visuals", K::Bool),
            P::opt(
                "export_data",
                K::Enum(&["AllowSummarized", "AllowSummarizedAndUnderlying", "None"]),
            ),
            P::opt("persistent_filters", K::Bool),
            P::opt("personalize_visuals", K::Bool),
            P::opt("cross_report_drillthrough", K::Bool),
            P::opt("filter_pane_visible", K::Bool),
            P::opt("allow_change_filter_types", K::Bool),
        ],
    },
    Command {
        id: "report.pause_visuals",
        label: "Pause or resume the report's visual queries (Optimize: Pause visuals)",
        params: "{paused?: bool (default true)}: Desktop's Optimize › Pause visuals: while paused, a visual whose \
                 data would need an evaluation keeps the data it last showed (marked pending) or shows the pending \
                 status, and nothing is evaluated for it until report.refresh_visuals or a resume (paused: false). \
                 Saved with the report (report.json settings.pauseQueries), as Desktop keeps a paused report paused \
                 when reopened. Returns {paused, changed}",
        run: crate::optimize::pause_visuals,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Analyze, "Optimize", "Pause visuals", "eye-off"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[P::flag("paused", true)],
    },
    Command {
        id: "report.refresh_visuals",
        label: "Refresh visuals: evaluate every visual once on the next view, the report staying paused",
        params: "{}: Desktop's Optimize › Refresh visuals: the next view of the report evaluates every visual once; \
                 a paused report stays paused. The report and the model are not changed. Returns {paused, refresh}",
        run: crate::optimize::refresh_visuals,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Analyze, "Optimize", "Refresh all visuals", "reload"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[],
    },
    Command {
        id: "report.query_reduction",
        label: "Apply an optimization preset or set the query reduction options (cross-highlighting by default, Apply buttons)",
        params: "{preset?: \"query_reduction\"|\"interactivity\", disable_cross_highlighting?: bool, slicers?: \
                 \"instantly\"|\"apply_button\", filters?: \"instantly\"|\"apply_buttons\"|\"single_button\"}: \
                 Desktop's Optimize › Optimization presets, in report.json slowDataSourceSettings: query_reduction \
                 turns the default cross-highlighting and cross-filtering off (Edit interactions rules still apply) \
                 and adds Apply buttons to the slicers and a single Apply button to the Filters pane; \
                 interactivity (Desktop's default) turns them all off; the other parameters set one option each \
                 (Customize) and win over the preset. Returns {changed, preset: query_reduction|interactivity| \
                 customized, disable_cross_highlighting, slicers, filters}",
        run: crate::optimize::query_reduction,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Analyze, "Optimize", "Optimization presets", "settings"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::opt("preset", K::Enum(&["query_reduction", "interactivity"])),
            P::opt("disable_cross_highlighting", K::Bool),
            P::opt("slicers", K::Enum(&["instantly", "apply_button"])),
            P::opt(
                "filters",
                K::Enum(&["instantly", "apply_buttons", "single_button"]),
            ),
        ],
    },
    Command {
        id: "visual.drill_filter",
        label: "Apply a visual's drill down filters to the entire page or to the visual only",
        params: "{page, visual, apply_to: \"page\"|\"visual\"|\"default\"}: Desktop's \"Apply drill down filters \n                 to\" (drillFilterOtherVisuals): \"page\" makes drilling into the visual filter the page's \n                 other visuals, \"visual\" keeps the drill to the visual, \"default\" follows the report's \n                 \"Drilling filters other visuals\" (report.settings). Returns {visual, changed, apply_to, \n                 filters_other_visuals}",
        run: crate::visual_extras::visual_drill_filter,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(
            Tab::Visual,
            "Interactions",
            "Apply drill down filters to",
            "hierarchy",
        ),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::req("apply_to", K::Enum(&["page", "visual", "default"])),
        ],
    },
    Command {
        id: "visual.order",
        label: "Change the layer (and tab) order of visuals",
        params: "{page, visuals: [names, front to back] | visual and/or visuals, to: \"front\"|\"back\"|\
                 \"forward\"|\"backward\"}: visuals of one group (or of the page's top level). Without \
                 `to`, unlisted siblings go behind the listed ones; with `to`, the listed visuals move \
                 together, keeping their order (forward and backward: past the next sibling not \
                 listed). z and tabOrder are redistributed together. Returns {front_to_back, previous}",
        run: crate::visual_extras::visual_order,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Visual, "Arrange", "Layer order", "layers"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::opt("visuals", K::List),
            P::opt("visual", K::Name(Obj::Visual)),
            P::opt("to", K::Enum(&["front", "back", "forward", "backward"])),
        ],
    },
    Command {
        id: "visual.set_hidden",
        label: "Show or hide visuals (the Selection pane's eye)",
        params: "{page, visual? | visuals?: [names], hidden: true|false}: `isHidden`, the visuals kept in \
                 the report but not drawn or tabbed to. A group hides its members with it. Returns {page, \
                 hidden, changed, visuals: [{visual, hidden}]}",
        run: crate::visual_extras::visual_set_hidden,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Visual, "Arrange", "Show or hide", "eye"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::opt("visual", K::Name(Obj::Visual)),
            P::opt("visuals", K::List),
            P::req("hidden", K::Bool),
        ],
    },
    Command {
        id: "visual.rename_group",
        label: "Rename a visual group (the Selection pane's Rename)",
        params: "{page, group, display_name}: the group's `visualGroup.displayName`, 1 to 200 characters. \
                 Groups only: a visual's layer name follows its title. Returns {page, group, display_name, \
                 changed}",
        run: crate::visual_extras::visual_rename_group,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::Visual, "Rename group", "rename"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("group", K::Name(Obj::Visual)),
            P::req("display_name", K::Text),
        ],
    },
    Command {
        id: "visual.tab_order",
        label: "Set the tab order of visuals",
        params: "{page, visuals: [names, in tab order]}: keyboard order (Tab), written as tabOrder 0, 1000, \
                 2000…; unlisted visuals keep theirs. Returns {page, changed, tab_order: [{visual, \
                 tab_order}]}",
        run: crate::visual_extras::visual_tab_order,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Visual, "Arrange", "Tab order", "sort"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[P::req("page", K::Name(Obj::Page)), P::req("visuals", K::List)],
    },
    Command {
        id: "visual.copy",
        label: "Copy visuals to the clipboard (a JSON document visual.paste takes, in this report or another)",
        params: "{page, visual? | visuals?: [names]}: a group is copied with its members. Returns {page, \
                 count, clip}, where `clip` is {bicraft: \"visuals\", version: 1, page: {name, display_name, \
                 width, height}, visuals: [the visuals' records: position, type, fields, formatting, filters]}; \
                 the app puts it on the clipboard as JSON text",
        run: |s, p| crate::canvas_edit::visual_copy(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::menu(Menu::Visual, "Copy", "copy"),
        shortcut: Some("Ctrl+C"),
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::opt("visual", K::Name(Obj::Visual)),
            P::opt("visuals", K::List),
        ],
    },
    Command {
        id: "visual.cut",
        label: "Cut visuals: copy them to the clipboard and remove them in one step",
        params: "{page, visual? | visuals?: [names]}: what visual.copy returns, then the visuals are \
                 removed as visual.remove removes them (a group with its members; a group left \
                 without members goes too). Returns {page, count, clip, removed}",
        run: crate::canvas_edit::visual_cut,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::Visual, "Cut", "cut"),
        shortcut: Some("Ctrl+X"),
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::opt("visual", K::Name(Obj::Visual)),
            P::opt("visuals", K::List),
        ],
    },
    Command {
        id: "visual.paste",
        label: "Paste visuals copied with visual.copy onto a page (of this report or another)",
        params: "{page, clip (visual.copy's `clip`, or its whole answer), x?, y? (the top-left corner; \
                 default: the copied place, moved 20 px right and down while a visual is there)}: the \
                 visuals get new names and go on top of the page, kept on it when they fit; groups keep \
                 their members. Fields the model lacks are kept as broken references and listed in \
                 `warnings`. The clip is checked (at most 500 visuals, types, positions, sizes). Returns \
                 {page, visuals, top, selectable, offset, warnings}",
        run: crate::canvas_edit::visual_paste,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Home, "Clipboard", "Paste", "paste"),
        shortcut: Some("Ctrl+V"),
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("clip", K::Json),
            P::opt("x", K::Number),
            P::opt("y", K::Number),
        ],
    },
    Command {
        id: "visual.copy_format",
        label: "Format painter: copy one visual's formatting onto other visuals",
        params: "{page, from, to: name | [names], to_page? (default page)}: in one step, each target \
                 gets the source's formatting objects (visual and container) the target's type has: \
                 the title but its text, data labels but display units and decimal places, axes but \
                 their start and end, the legend but its title text, colours and labels set for one \
                 field's values only when the target binds that field; fields, filters, size, \
                 position, alt text and the visual's action stay the target's. Returns {page, from, \
                 changed, changes: [{visual, objects}]}",
        run: crate::format_painter::visual_copy_format,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Home, "Clipboard", "Format painter", "format"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("from", K::Name(Obj::Visual)),
            P::req("to", K::List),
            P::opt("to_page", K::Name(Obj::Page)),
        ],
    },
    Command {
        id: "visual.duplicate",
        label: "Duplicate visuals on their page",
        params: "{page, visual? | visuals?: [names]}: as visual.copy then visual.paste on the same page, \
                 in one step: the copies are moved 20 px right and down while their place is taken. \
                 Returns visual.paste's answer",
        run: crate::canvas_edit::visual_duplicate,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::menu(Menu::Visual, "Duplicate", "duplicate"),
        shortcut: Some("Ctrl+D"),
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::opt("visual", K::Name(Obj::Visual)),
            P::opt("visuals", K::List),
        ],
    },
    Command {
        id: "visual.move",
        label: "Move or resize several visuals in one step (a drag on the canvas)",
        params: "{page, visuals: [{visual, x?, y?, width?, height?}]}: only the given values change; a \
                 group's members move and scale with its rectangle, and the groups around a moved \
                 visual are refitted to their members. A visual and its group cannot both be listed. \
                 Returns {page, changed, moved: [{visual, position, previous}], warnings}",
        run: crate::canvas_edit::visual_move,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Move visuals", "move"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[P::req("page", K::Name(Obj::Page)), P::req("visuals", K::Json)],
    },
    Command {
        id: "visual.align",
        label: "Align or distribute visuals",
        params: "{page, visual? | visuals?: [names], to: left|center|right|top|middle|bottom|\
                 distribute_horizontal|distribute_vertical}: one visual aligns to the page, several to \
                 the rectangle around them; distributing (three visuals or more) makes the gaps between \
                 them equal, the outer two staying. Groups move with their members. Returns visual.move's \
                 answer",
        run: crate::canvas_edit::visual_align,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Visual, "Arrange", "Align", "align"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::opt("visual", K::Name(Obj::Visual)),
            P::opt("visuals", K::List),
            P::req("to", K::Enum(crate::canvas_edit::ALIGN_TO)),
        ],
    },
    Command {
        id: "visual.group",
        label: "Group visuals (a visual group, as Desktop's Group)",
        params: "{page, visual? | visuals?: [names] (two or more, of one group or of the page's top \
                 level), display_name? (default: Group 1, Group 2…)}: a new group around them, in their \
                 group. Returns {page, group, display_name, members}",
        run: crate::canvas_edit::visual_group,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Visual, "Arrange", "Group", "group"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::opt("visual", K::Name(Obj::Visual)),
            P::opt("visuals", K::List),
            P::opt("display_name", K::Text),
        ],
    },
    Command {
        id: "visual.ungroup",
        label: "Ungroup a visual group (its members stay where they are)",
        params: "{page, visual (a group, or a visual inside one: its innermost group)}: the members go \
                 to the group's own group or the page's top level. Returns {page, ungrouped, members}",
        run: crate::canvas_edit::visual_ungroup,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Visual, "Arrange", "Ungroup", "ungroup"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
        ],
    },
    Command {
        id: "visual.mobile",
        label: "Place a visual on the page's mobile (phone) layout, or remove it from it",
        params: "{page, visual, x, y, width, height} (a new mobile layout needs all four, and goes on \
                 top of the phone canvas's layers; an existing one takes any of them) | {page, \
                 visual, layout: null} (removes it) | {page, visuals: [{visual, x?, y?, width?, \
                 height?, layout?}]} (several at once, one undo step); snap?: true puts x and \
                 width on the 18-pixel columns and y and height on the 20-pixel rows of Power BI \
                 Desktop's 324-wide phone canvas, inside it: written to \
                 visuals/<visual>/mobile.json (a legacy layout: the container's phone layout)",
        run: crate::visual_extras::visual_mobile,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Visual, "Arrange", "Mobile layout", "mobile"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::opt("visual", K::Name(Obj::Visual)),
            P::opt("x", K::Number),
            P::opt("y", K::Number),
            P::opt("width", K::Number),
            P::opt("height", K::Number),
            P::opt("layout", K::Json),
            P::opt("visuals", K::Json),
            P::opt("snap", K::Bool),
        ],
    },
    Command {
        id: "visual.mobile_format",
        label: "Format a visual on the page's mobile (phone) layout only, or clear its mobile formatting",
        params: "{page, visual, <visual.format's settings>, title?: {…}|null, axes?: {x?, y?, y2?}, \
                 tooltip_page?} | {page, visual, clear: true}: the visual must be on the mobile \
                 layout (visual.mobile); writes the objects and visualContainerObjects of its \
                 mobile.json, which the phone layout draws over the visual's own; clear: true \
                 removes them all (the visual follows its desktop formatting again)",
        run: crate::visual_extras::visual_mobile_format,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Mobile format", "mobile"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::opt("title", K::Json),
            P::opt("background", K::Json),
            P::opt("border", K::Json),
            P::opt("objects", K::Json),
            P::opt("clear", K::Bool),
        ],
    },
    Command {
        id: "visual.series",
        label: "Format one series or one category value of a visual: data color, data labels, line style, any selector-scoped property",
        params: "{page, visual, target: \"[Measure]\" | \"Table[Column]\" (a bound field) | \
                 \"Table[Column]=value\" | {field, value}, color?: \"#hex\"|null, labels?: \
                 bool|{show?, color?, font_size?, position?, display_units?, precision?}|null, line?: \
                 {show?, width?, style?: solid|dashed|dotted, show_marker?, marker_shape?, \
                 marker_size?}|null, objects?: [{object, property, value}], clear?: bool} or {page, \
                 visual, series?: [items like that], colors?: {\"<target>\": \"#hex\"|null}}: \
                 a patch; null removes; writes selector {metadata: queryRef} or {data: [{scopeId}]}",
        run: crate::visual_series::visual_series,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Series format", "palette"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::opt("target", K::Json),
            P::opt("color", K::Color),
            P::opt("labels", K::Json),
            P::opt("line", K::Json),
            P::opt("objects", K::List),
            P::flag("clear", false),
            P::opt("series", K::List),
            P::opt("colors", K::Json),
        ],
    },
    Command {
        id: "visual.conditional",
        label: "Set or remove a conditional format: color scale, rules, field value, icons, data bars, web URL",
        params: "{page, visual, target?: background|font_color|icon|data_bars|web_url (tables), \
                 font_color|category_color (cards), data_color|label_color (charts), \
                 title_text|title_color|title_background|subtitle_text|subtitle_color|\
                 visual_background|border_color (every visual: one value, no series) | {object, \
                 property}, series?: field (the \
                 column or measure formatted; tables need it unless the rule's field is bound), \
                 occurrence?: n (which binding of series when it is bound more than once; 1), \
                 apply_to?: values|values_and_totals|totals, rule: {gradient: {field, min, mid?, max, \
                 empty?: zero|none|#hex}} | {rules: {field, cases: [{op: =|>|>=|<|<=|between, \
                 value|values, color}], else?}} | {field_value: {field, as?: color|value}} | {icons: \
                 {field, cases: [{op, value, icon}] | preset + thresholds: [low, high], else?, \
                 position?: before|after|only, align?: top|middle|bottom}} | {data_bars: {positive?, \
                 negative?, axis?, reverse?, show_bar_only?, min?, max?}} | {web_url: {field}} | null} \
                 or {page, visual, conditional: [items like that]}: stops are \"#hex\" or {color, value}",
        run: crate::visual_conditional::visual_conditional,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Conditional formatting", "palette"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("page", K::Name(Obj::Page)),
            P::req("visual", K::Name(Obj::Visual)),
            P::opt("target", K::Json),
            P::opt("series", K::Field),
            P::opt("occurrence", K::Integer),
            P::opt("apply_to", K::Enum(&["values", "values_and_totals", "totals"])),
            P::opt("rule", K::Json),
            P::opt("conditional", K::List),
        ],
    },
    Command {
        id: "visual.options",
        label: "List the formatting options of built-in visual types: objects, properties, types, allowed values, ranges",
        params: "{visual_type?, object?}: without visual_type, the visual types (with data roles) and \
                 the container objects; with it, that type's objects (target visual|container, \
                 selectors) and their properties ({name, type: bool|number|integer|text|color|enum|\
                 font|image|field|other, min?, max?, values?, conditional?}); with object too, that \
                 object with display names and descriptions. visual.format checks values against it",
        run: |s, p| crate::visual_catalog::visual_options(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::palette("Formatting options", "help"),
        shortcut: None,
        enabled: &[],
        form: &[
            P::opt("visual_type", K::Name(Obj::VisualType)),
            P::opt("object", K::Text),
        ],
    },
    Command {
        id: "theme.get",
        label: "Read the report theme: the custom theme JSON, the base theme, and the resolved colors and fonts",
        params: "{}: returns {custom: {name, type, report_version_at_import, resource, theme_name, theme, \
                 issues} | null, base: {name, type, report_version_at_import, resource} | null, resolved: \
                 {name, data_colors, background, foreground, table_accent, font_family, font_size} | null, \
                 builtin: [id] (the built-in themes of theme.set), schema_version}",
        run: |s, p| crate::theme_edit::theme_get(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::View, "Themes", "Current theme", "palette"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[],
    },
    Command {
        id: "theme.set",
        label: "Install a custom report theme (theme JSON, or one of BIcraft's built-in themes), replacing the current one",
        params: "{theme: object (or JSON text) | builtin: string (a built-in theme id; theme.get lists \
                 them), force?: bool}: validated against Microsoft's report theme \
                 schema (force installs it anyway, problems in `warnings`); written as \
                 StaticResources/RegisteredResources/<name><digits>.json, registered as a CustomTheme \
                 resource and referenced by themeCollection.customTheme, as Desktop does; the previous \
                 custom theme file is dropped. Re-installing the same theme changes nothing",
        run: crate::theme_edit::theme_set,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::View, "Themes", "Themes", "palette"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::opt("theme", K::Json),
            P::opt("builtin", K::Text),
            P::flag("force", false),
        ],
    },
    Command {
        id: "theme.import",
        label: "Install a custom report theme from a theme JSON file",
        params: "{path: string, force?: bool}: as theme.set, reading the file (at most 4 MB)",
        run: crate::theme_edit::theme_import,
        server: ServerPolicy::paths(&[PathParam::one("path")]),
        replay: Replay::Copy,
        place: Place::ribbon(Tab::View, "Themes", "Browse for themes", "folder"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[P::req("path", K::Path), P::flag("force", false)],
    },
    Command {
        id: "theme.export",
        label: "Write the report's custom theme (else its embedded base theme) to a JSON file",
        params: "{path: string, overwrite?: bool}: an existing file is replaced only with overwrite",
        run: crate::theme_edit::theme_export,
        server: ServerPolicy::paths(&[PathParam::one("path")]),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::View, "Themes", "Save current theme", "export"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[P::req("path", K::Path), P::flag("overwrite", false)],
    },
    Command {
        id: "theme.remove",
        label: "Remove the custom theme (back to the base theme) and its resource file",
        params: "{}: no-op when the report has no custom theme",
        run: crate::theme_edit::theme_remove,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::View, "Themes", "Reset theme", "reload"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[],
    },
    Command {
        id: "theme.set_colors",
        label: "Set the theme's data colors and named colors (creates the custom theme when missing)",
        params: "{data_colors?: [\"#RRGGBB\", ...], background?, foreground?, table_accent?, good?, \
                 neutral?, bad?, maximum?, center?, minimum?, force?}: colors are #RGB, #RRGGBB or \
                 #RRGGBBAA; null removes the key (the base theme's value applies). A new custom theme \
                 starts from the embedded base theme's colors",
        run: crate::theme_edit::theme_set_colors,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::View, "Themes", "Theme colors", "palette"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::opt("data_colors", K::List),
            P::opt("background", K::Color),
            P::opt("foreground", K::Color),
            P::opt("table_accent", K::Color),
            P::opt("good", K::Color),
            P::opt("neutral", K::Color),
            P::opt("bad", K::Color),
            P::opt("maximum", K::Color),
            P::opt("center", K::Color),
            P::opt("minimum", K::Color),
            P::flag("force", false),
        ],
    },
    Command {
        id: "theme.set_text_classes",
        label: "Set the theme's text classes (fonts, sizes, colors of titles, labels, callouts, headers)",
        params: "{callout?, title?, header?, label?, largeTitle?, dataTitle?, boldLabel?, largeLabel?, \
                 largeLightLabel?, lightLabel?, semiboldLabel?, smallLabel?, smallLightLabel?, \
                 smallDataLabel?: {font_face?, font_size? (pt), color?, font_weight?} | null, force?}: \
                 null removes a property or a whole class",
        run: crate::theme_edit::theme_set_text_classes,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::View, "Themes", "Text classes", "format"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::opt("callout", K::Json),
            P::opt("title", K::Json),
            P::opt("header", K::Json),
            P::opt("label", K::Json),
            P::opt("largeTitle", K::Json),
            P::opt("dataTitle", K::Json),
            P::opt("boldLabel", K::Json),
            P::opt("largeLabel", K::Json),
            P::opt("largeLightLabel", K::Json),
            P::opt("lightLabel", K::Json),
            P::opt("semiboldLabel", K::Json),
            P::opt("smallLabel", K::Json),
            P::opt("smallLightLabel", K::Json),
            P::opt("smallDataLabel", K::Json),
            P::flag("force", false),
        ],
    },
    Command {
        id: "theme.set_visual_style",
        label: "Set a default formatting property for every visual of a type (theme visualStyles)",
        params: "{visual_type (\"*\" for every visual, or e.g. card, clusteredColumnChart, page), object \
                 (e.g. labels, title, background), property (e.g. fontSize, show, color), value: bool | \
                 number | string | {color: \"#hex\"} | theme JSON value | null (removes it), state? \
                 (the entry's $id, e.g. hover), force?}: checked against the visual catalog when it knows \
                 the type and against the report theme schema",
        run: crate::theme_edit::theme_set_visual_style,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::View, "Themes", "Visual styles", "format"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::req("visual_type", K::Name(Obj::VisualType)),
            P::req("object", K::Text),
            P::req("property", K::Text),
            P::req("value", K::Json),
            P::opt("state", K::Text),
            P::flag("force", false),
        ],
    },
    Command {
        id: "theme.customize",
        label: "Customize the current theme: its name, colors, text classes and visual styles in one step",
        params: "{name?, data_colors?, background?, foreground?, table_accent?, good?, neutral?, bad?, \
                 maximum?, center?, minimum? (as theme.set_colors), text_classes?: {label?, title?, \
                 callout?, header?, …} (as theme.set_text_classes), styles?: [{visual_type, object, \
                 property, value, state?}] (as theme.set_visual_style, at most 256), force?}: one edit \
                 of the custom theme (created from the base theme when missing), one undo step",
        run: crate::theme_edit::theme_customize,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::View, "Themes", "Customize current theme", "format"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::opt("name", K::Text),
            P::opt("data_colors", K::List),
            P::opt("background", K::Color),
            P::opt("foreground", K::Color),
            P::opt("table_accent", K::Color),
            P::opt("good", K::Color),
            P::opt("neutral", K::Color),
            P::opt("bad", K::Color),
            P::opt("maximum", K::Color),
            P::opt("center", K::Color),
            P::opt("minimum", K::Color),
            P::opt("text_classes", K::Json),
            P::opt("styles", K::Json),
            P::flag("force", false),
        ],
    },
    Command {
        id: "dax.validate_query",
        label: "Validate a DAX query (DEFINE / EVALUATE / ORDER BY / START AT) against the open model",
        params: "{query: string}: returns {valid, checked_against_model, diagnostics, definitions: \
                 [{kind, name, table?}], evaluate_count}; query MEASURE, COLUMN and TABLE definitions \
                 are checked as if added to the model (nothing is changed)",
        run: |s, p| dax_validate_query(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Analyze, "DAX", "Check query", "validate"),
        shortcut: None,
        enabled: &[],
        form: &[P::req("query", K::Dax)],
    },
    Command {
        id: "dax.evaluate",
        label: "Run a DAX query (DEFINE / EVALUATE / ORDER BY / START AT) over the open file's stored data and return the rows",
        params: "{query: string, max_rows?: int (rows returned per EVALUATE, default 10000, at most \
                 1000000), timeout_ms?: int (default 30000, at most 600000), as_of?: string (\"YYYY-MM-DD\" or \
                 \"YYYY-MM-DDTHH:MM:SS\": what TODAY()/NOW() return, local time; default the session's clock), \
                 utc_offset_minutes?: int (local time minus UTC, -1080 to 1080, default the process's local \
                 clock: the machine's time zone in the desktop app and the CLI, UTC or --local-offset on a server; \
                 what UTCNOW()/UTCTODAY() subtract from NOW()/TODAY(), as Desktop's machine time zone does), allow_cache_abf?: bool \
                 (default true), cache_abf?: string (PBIP: read this cache.abf instead of the \
                 project's, read-only)}: data comes from a PBIX's DataModel, a PBIP's bicraft data \
                 snapshot (.pbi/bicraft-data.bin, written by model.refresh, while it is newer than \
                 .pbi/cache.abf), else its .pbi/cache.abf (or the session's or the command's cache_abf, \
                 which wins; not read when allow_cache_abf is false), then the tables' partitions (M queries, DAX literal \
                 tables). The query is validated first. DEFINE MEASURE/VAR/TABLE/COLUMN are supported \
                 (FUNCTION is not). Returns {results: [{columns: [{name, type}], rows: [{column: value}], \
                 row_count, truncated}] (one per EVALUATE), data_source, elapsed_ms}: model columns are \
                 named Table[Column], others [Name]; dates are ISO text, blanks null. A function or \
                 construct bicraft's evaluator does not support is an error naming it; nothing is changed",
        run: crate::cache_abf::dax_evaluate,
        server: ServerPolicy::paths(&[PathParam::clearable("cache_abf")]).read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Analyze, "DAX", "Run DAX query", "play"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("query", K::Dax),
            P::opt("max_rows", K::Integer),
            P::opt("timeout_ms", K::Integer),
            P::opt("as_of", K::Text),
            P::opt("utc_offset_minutes", K::Integer),
            P::flag("allow_cache_abf", true),
            P::opt("cache_abf", K::Path),
        ],
    },
    Command {
        id: "dax.apply_define",
        label: "Update the model from a DAX query's DEFINE MEASURE blocks (add new measures, overwrite changed ones)",
        params: "{query: string, measures?: [string] (the names to apply; default every DEFINE MEASURE), \
                 force?: bool}: each `MEASURE Table[Name] = expression` of the query's DEFINE block is \
                 added to Table when the model has no measure of that name, or replaces the expression \
                 of the measure of that name in Table (a measure of that name in another table, an \
                 unknown table or a query that does not parse is an error). One edit, one undo step; \
                 the expressions are validated against the model with every change made (`force` \
                 applies invalid DAX anyway). Returns {changes: [{table, measure, action: \
                 added|updated|unchanged}], added, updated, unchanged, changed, diagnostics?, warnings?}",
        run: crate::dax_define::dax_apply_define,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Analyze, "DAX", "Update model", "measure"),
        shortcut: None,
        enabled: &[Need::Model],
        form: &[
            P::req("query", K::Dax),
            P::opt("measures", K::List),
            P::flag("force", false),
        ],
    },
    Command {
        id: "dax.format",
        label: "Format a DAX query or expression: one clause per line, long calls broken, comments kept",
        params: "{query: string (a DAX query starting with DEFINE or EVALUATE, or a DAX expression),                  max_width?: int (20 to 1000, default 100), indent?: int (spaces per level, 0 to 16, default                  4), style?: query (default) | formula (the formula bar's Format DAX: BIcraft's style after the DAX formatting conventions SQLBI describes publicly: function names and keywords in upper case, `SUM ( T[C] )`, the expression after RETURN on its own line one level in)}: returns {query (the formatted text), changed}. Except the formula style's upper case, only white space changes: tokens,                  comments, texts and names stay as written, and the result is checked to hold the same                  tokens and comments. DEFINE, EVALUATE, ORDER BY and START AT start a line, each definition                  one level in; a call or {…} that does not fit, or holds VAR or a line comment, gets one                  argument per line. Idempotent: formatting the result changes nothing. Text that does not                  parse is an error; nothing is changed",
        run: |s, p| crate::dax_queries::dax_format(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::palette("Format query", "format"),
        shortcut: None,
        enabled: &[],
        form: &[
            P::req("query", K::Dax),
            P::opt("max_width", K::Integer),
            P::opt("indent", K::Integer),
            P::opt("style", K::Enum(&["query", "formula"])),
        ],
    },
    Command {
        id: "dax.cancel",
        label: "Cancel the running DAX queries of this session",
        params: "{}: every dax.evaluate of this session that started before this call stops at its next                  budget check and fails with \"cancelled by the client\"; returns {running} (how many were                  running). Read-only: a server runs it beside the query it stops. Nothing else is changed",
        run: |s, p| crate::dax_queries::dax_cancel(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::palette("Cancel query", "close"),
        shortcut: None,
        enabled: &[],
        form: &[],
    },
    Command {
        id: "dax.queries",
        label: "The DAX query view's tabs of the open file",
        params: "{}: returns {tabs: [{name, text}], default_tab, saved_in: project|session, dirty, generation,                  error}: what file.open read from a PBIP project's DAXQueries folder (one .dax file per tab,                  .pbi/daxQueries.json for the order and the tab shown; docs/formats/dax-queries.md), as                  dax.set_queries changed it. `saved_in` is session for a PBIX, PBIT or model.bim (the tabs                  are not written into those); `generation` changes with every file.open",
        run: |s, p| crate::dax_queries::dax_queries(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::palette("DAX query tabs", "list"),
        shortcut: None,
        enabled: &[],
        form: &[],
    },
    Command {
        id: "dax.set_queries",
        label: "Set the DAX query view's tabs that the next save writes into the project",
        params: "{tabs: [{name: string, text?: string}], default_tab?: string}: replaces the open file's DAX                  query tabs (at most 1000; each name a valid file name on Windows and Linux, unique                  ignoring case, at most 200 characters; each text at most 16 MiB). An unsaved change                  (`dirty.queries` of the snapshot), not a model edit and no undo step: file.save writes                  them into a PBIP project's DAXQueries folder (one `<name>.dax` file per tab, removing the                  files of closed tabs, and .pbi/daxQueries.json), file.save_as and project.new into the new                  project; a PBIX, PBIT or model.bim keeps them for the session. Returns {tabs, changed,                  dirty, saved_in}",
        run: crate::dax_queries::dax_set_queries,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::palette("Keep DAX query tabs", "save"),
        shortcut: None,
        enabled: &[],
        form: &[P::req("tabs", K::Json), P::opt("default_tab", K::Text)],
    },
    Command {
        id: "dax.clear_cache",
        label: "Drop what DAX queries and the viewer keep between them, keeping the open data and its decoded columns",
        params: "{}: clears the storage engine's cache (grouped scans, lookup tables, kept tables and                  iterator results), what it computed from the rows of each column (rows per code) and                  the viewer's cached results, as Analysis Services' ClearCache leaves the data and its                  dictionaries in memory and the caches empty; the next query runs cold. Returns                  {columns, results}: the columns cleared and the results dropped",
        run: crate::preview::clear_cache,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Analyze, "Performance", "Clear cache", "reload"),
        shortcut: None,
        enabled: &[],
        form: &[],
    },
    Command {
        id: "dax.aggregations",
        label: "Report-derived aggregations: what the report's visuals need, the aggregations chosen and built in memory, on or off for the session",
        params: "{enabled?: bool (on or off for this session; default: on unless the environment variable \
                 BICRAFT_AGGREGATIONS is 0), build?: bool (derive and build them now for every fact the report \
                 needs; otherwise they are built on the first query over a fact of at least 100,000 rows)}: \
                 aggregations are derived from the report (visual fields, filters, slicers, measures and the \
                 columns their DAX reads), kept in memory only (never written to any file), answer the storage \
                 engine's scans only when they give exactly the fact's result, and follow every data, model and \
                 report change. Returns {enabled, visuals: [{page, visual, grain, values, facts}], facts: \
                 [{table, rows, visuals, uncovered, notes, build_ms, aggregations: [{name, grain: [{column, by, \
                 classes}], values, rows, estimate, bytes, covers, from, build_ms}]}], total_rows, total_bytes, \
                 build_ms, hits, misses}",
        run: |s, p| crate::preview::dax_aggregations(s, p),
        server: ServerPolicy::ALLOWED.read_only_unless(&["enabled", "build"]),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Analyze, "Performance", "Aggregations", "chart"),
        shortcut: None,
        enabled: &[],
        form: &[P::opt("enabled", K::Bool), P::flag("build", false)],
    },
    Command {
        id: "dax.memory",
        label: "Memory budget for the data DAX queries keep: set it for the session and report what is held",
        params: "{budget?: int | string | null (bytes, or text with a binary K, M, G or T suffix such as \
                 \"300M\"; 0 or \"unlimited\": no budget; null: back to the environment variable \
                 BICRAFT_MEMORY_BUDGET, else no budget), query_budget?: int | string | null (the cap of one \
                 query's formula-engine memory, same forms; null: back to BICRAFT_QUERY_BUDGET, else the memory \
                 budget)}: a query whose tables, groupings, scan states, sort keys and answer rows would exceed \
                 its cap is refused with an error naming the part and the cap (a TOPN over a large grouping, or \
                 an ORDER BY cut by max_rows, is selected block by block instead); under a budget the stored data's decompressed \
                 backup is moved to a temporary file (BICRAFT_SPILL_DIR, else the system's temporary folder) \
                 and read from it, the storage engine's caches take at most a quarter of the budget and are \
                 dropped first, then the least recently used decoded columns not in use by a query are \
                 dropped (read again when needed, with the same results); a column over half the budget is read \
                 segment by segment from the temporary file through a segment cache of at most half the budget; \
                 a column whose values, or one of whose segments, cannot fit is refused with an error naming it. \
                 A budget set before file.open leaves the DataModel in the package file and decompresses it as a \
                 stream to the temporary file. Returns {budget, budget_source, counted_bytes, \
                 peak_counted_bytes, columns, column_bytes, largest_column_bytes, columns_held_elsewhere, storage_cache_bytes, \
                 backup: {where, bytes}, spill_dir, spill_error, evictions, plain_dropped, cache_drops, refused, \
                 lazy_columns, lazy_reads, column_reads, segment_cache: {bytes, protected_bytes, cap, segments, \
                 loads, hits, shared, evictions}, \
                 segment_load_failures, query: {cap, cap_source, last_peak_bytes, peak_bytes, refused}, limit: {limit, \
                 reserved_bytes, peak_reserved_bytes, by_class: {kept, query, background, io}, refused, evictions, \
                 running_queries, session_reserved_bytes, process_peak_bytes}}. Under the process' memory limit \
                 (BICRAFT_MEMORY_LIMIT) there is always a budget (half of it unless set), and every large allocation \
                 is reserved against the limit first: what does not fit frees what sessions keep, then is refused \
                 with an error naming it; running queries share the limit fairly",
        run: |s, p| crate::preview::dax_memory(s, p),
        server: ServerPolicy::ALLOWED.read_only_unless(&["budget", "query_budget"]),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Analyze, "Performance", "Memory budget", "settings"),
        shortcut: None,
        enabled: &[],
        form: &[P::opt("budget", K::Text), P::opt("query_budget", K::Text)],
    },
    // The Performance analyzer (perf.rs): BIcraft's own times only. The recording is the shared
    // session's diagnostics state, so on a server every perf command is for editors only.
    Command {
        id: "perf.results",
        label: "The Performance analyzer's recording: BIcraft's evaluation and render time and DAX query text of each visual evaluated",
        params: "{}: returns {recording, elapsed_ms, events: [{seq, at_ms, page, visual, visual_type, \
                 cause: \"refresh\"|\"view\", evaluation_ms, render_ms|null, total_ms, status, reason?, rows, \
                 query|null, query_notes}], dropped, clock: \"monotonic\", measured_by: \"BIcraft\"}. Times are \
                 BIcraft's own (its evaluator, its renderer), in milliseconds from a monotonic clock",
        run: crate::perf::perf_results,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Analyze, "Performance", "Performance analyzer", "chart"),
        shortcut: None,
        enabled: &[],
        form: &[],
    },
    Command {
        id: "perf.start",
        label: "Start recording BIcraft's evaluation time of each visual (Performance analyzer)",
        params: "{}: while recording, every visual the viewer computes for the page it shows (not answered \
                 from the result cache) is recorded, and perf.refresh evaluates a page afresh; returns \
                 {recording, events}. The model, the report and the file are not changed",
        run: crate::perf::perf_start,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Skip,
        place: Place::palette("Start recording", "play"),
        shortcut: None,
        enabled: &[],
        form: &[],
    },
    Command {
        id: "perf.refresh",
        label: "Refresh visuals: evaluate every visual of a page afresh (without the result cache), each timed and recorded",
        params: "{page?: string (name or display name; default the page shown), visual?: string (only this \
                 visual)}: needs a recording (perf.start); the visuals are evaluated under the viewer's slicer \
                 selections and cross-filters, decoded columns staying cached; returns {page, visuals: [events \
                 as perf.results lists them]}",
        run: crate::perf::perf_refresh,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Skip,
        place: Place::palette("Refresh visuals", "reload"),
        shortcut: None,
        enabled: &[Need::Report],
        form: &[
            P::opt("page", K::Name(Obj::Page)),
            P::opt("visual", K::Name(Obj::Visual)),
        ],
    },
    Command {
        id: "perf.stop",
        label: "Stop recording (Performance analyzer); the events are kept",
        params: "{}: returns {recording, events}",
        run: crate::perf::perf_stop,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Skip,
        place: Place::palette("Stop recording", "close"),
        shortcut: None,
        enabled: &[],
        form: &[],
    },
    Command {
        id: "perf.clear",
        label: "Clear the Performance analyzer's events (a running recording goes on)",
        params: "{}: returns {recording, cleared}",
        run: crate::perf::perf_clear,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Skip,
        place: Place::palette("Clear recording", "delete"),
        shortcut: None,
        enabled: &[],
        form: &[],
    },
    Command {
        id: "perf.render",
        label: "Hand back the render time the app measured for visuals of a page (Performance analyzer)",
        params: "{page: string, times: {visual: milliseconds (0 to 3600000)}}: each time goes to the latest \
                 recorded event of that visual of the page that has none yet; returns {updated}",
        run: crate::perf::perf_render,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Skip,
        place: Place::palette("Record render times", "info"),
        shortcut: None,
        enabled: &[],
        form: &[P::req("page", K::Name(Obj::Page)), P::req("times", K::Json)],
    },
    Command {
        id: "perf.export",
        label: "Export the Performance analyzer's recording as a JSON file of BIcraft's own times",
        params: "{out: string (the .json file)}: writes what perf.results returns; returns {path, bytes, events}",
        run: crate::perf::perf_export,
        server: ServerPolicy::paths(&[PathParam::one("out")]),
        replay: Replay::Skip,
        place: Place::palette("Export recording", "export"),
        shortcut: None,
        enabled: &[],
        form: &[P::req("out", K::Path)],
    },
    Command {
        id: "model.refresh",
        label: "Refresh the open PBIX file's or PBIP project's import tables with bicraft (Power Query evaluator, explicitly allowed connectors such as Databricks): into the PBIX's DataModel, or the project's own data snapshot",
        params: "{tables?: [string] (default: every table), allow_connectors?: [\"databricks\" | \"sqlserver\" | \"web\" | \"odata\" | \"sharepoint\"] (default none: a \
                 query that reads an external source fails, as before), external_links?: bool (default false: \
                 Databricks results inline, network only to the workspace host the query names; true: large \
                 results through presigned cloud-storage links, fetched without the token), data_root?: \
                 string, force?: bool (PBIX, default false: refused while the model has unsaved edits; true: \
                 the file's saved model is refreshed and the edits stay unsaved), backup?: bool (PBIX, \
                 default false: true keeps the previous file as <name>.bak), utc_offset_minutes?: int \
                 (local time minus UTC, -1080 to 1080, default the process's local clock, as in \
                 dax.evaluate: what DateTime.LocalNow() adds to the refresh time and what NOW()/TODAY() \
                 in calculated tables and columns add to UTCNOW()/UTCTODAY())}. On a PBIX the new rows are \
                 written into its DataModel as Power BI Desktop's Refresh writes them (VertiPaq storage, \
                 partitions Ready with the refresh time): every import and calculated table, or the \
                 `tables` named plus the calculated tables and columns that read them (a table whose \
                 calculated columns read them keeps its stored rows and gets those columns recomputed, \
                 origin `stored`); the other tables keep their storage byte for byte. Nothing is written \
                 unless every table loads; the file is written to a temporary sibling, reopened with \
                 bicraft's reader, then replaced atomically; the rows are reserved against the memory \
                 limit (BICRAFT_MEMORY_LIMIT) as they load. A PBIX returns {path, format: pbix, \
                 refreshed_at, tables: [{table, origin (m|dax|stored), rows, columns, partitions, ms}], \
                 kept, skipped, stale: [{table, column, reason}], connectors, statements, bytes, \
                 previous_bytes, backup, verified, phases_ms: {load, encode, write_verify}, elapsed_ms, \
                 warnings}. On a PBIP project: M partitions run in bicraft's Power Query evaluator (column selections, simple \
                 row filters and row limits fold into the source query; DateTimeZone.UtcNow() is the \
                 refresh time), then calculated tables and columns in its DAX evaluator. The Databricks \
                 token is read from the environment variable BICRAFT_DATABRICKS_TOKEN (the CLI asks for \
                 it, masked, when unset) when the first request is made; it is never written, logged or \
                 returned. SQL Server credentials come from source.credentials (this session, in memory) or \
                 BICRAFT_SQLSERVER_USER / BICRAFT_SQLSERVER_PASSWORD, read when a connection opens, never \
                 written, logged or returned. Writes <Name>.SemanticModel/.pbi/bicraft-data.bin (bicraft's own format; \
                 .pbi/cache.abf is never read, written or copied) only when every import table loads; \
                 with `tables`, the other tables of the previous snapshot are kept. dax.evaluate, \
                 report.render and the viewer then read that snapshot while it is newer than the \
                 project's cache.abf and no explicit cache_abf is set. Returns {path, refreshed_at, \
                 bytes, tables: [{table, origin (m|dax), rows, columns, ms, statements: [{host, sql, rows, \
                 ms}]}], kept, skipped, not_computed: [{table, column, reason}], connectors, elapsed_ms, \
                 warnings}",
        run: crate::refresh::model_refresh,
        server: ServerPolicy::network(&[PathParam::clearable("data_root")]),
        replay: Replay::Copy,
        place: Place::ribbon(Tab::Home, "Queries", "Refresh", "reload"),
        shortcut: Some("F5"),
        enabled: &[Need::Refreshable],
        form: &[
            P::opt("tables", K::List),
            P::opt("allow_connectors", K::List),
            P::flag("external_links", false),
            P::opt("data_root", K::Path),
            P::flag("force", false),
            P::flag("backup", false),
            P::opt("utc_offset_minutes", K::Integer),
        ],
    },
    Command {
        id: "edit.undo",
        label: "Undo the last edits (model and report; files written by saves stay)",
        params: "{steps? (default 1)}: returns {undone: [{label, commands, at_ms}], can_undo, can_redo, \
                 dirty, report_dirty}; fails when there is nothing to undo. An atomic batch is one step; \
                 file.open and model.new clear the history",
        run: crate::history::edit_undo,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Home, "Undo", "Undo", "undo"),
        shortcut: Some("Ctrl+Z"),
        enabled: &[Need::Undo],
        form: &[P::opt("steps", K::Integer)],
    },
    Command {
        id: "edit.redo",
        label: "Redo edits undone by edit.undo",
        params: "{steps? (default 1)}: returns {redone: [...], can_undo, can_redo, dirty, report_dirty}; \
                 any new edit clears what can be redone",
        run: crate::history::edit_redo,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Exact,
        place: Place::ribbon(Tab::Home, "Undo", "Redo", "redo"),
        shortcut: Some("Ctrl+Y"),
        enabled: &[Need::Redo],
        form: &[P::opt("steps", K::Integer)],
    },
    Command {
        id: "edit.history",
        label: "List the undo and redo steps",
        params: "{}: returns {undo: [{label, commands, at_ms}] (next to undo first), redo: [...] (next \
                 to redo first), can_undo, can_redo, bytes (estimated snapshot memory), limits: \
                 {max_steps, max_bytes}}",
        run: |s, p| crate::history::edit_history(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Home, "Undo", "History", "list"),
        shortcut: None,
        enabled: &[],
        form: &[],
    },
    Command {
        id: "project.test",
        label: "Run a project's CI quality gates: DAX assertions over its data, model rules (validation, BPA, naming, required properties, forbidden functions), report rules (report.validate, visuals per page, alt text, page size, theme, navigation) from a declarative test file",
        params: "{path?: string (a .pbip, project folder, .pbix, .pbit or model.bim, opened in a private \
                 session: the open file is left alone; default the open file), tests?: string | object (a \
                 test file, or inline tests; default bicraft.tests.yaml / .yml / .json next to the \
                 project), format?: json|text|junit|sarif or a list of them (default json; one other \
                 format adds `output`, the rendered text; a list adds `outputs: {format: text}`), fail_on?: error|warning (default error), base_dir?: string (paths in \
                 results and SARIF are relative to it; default the project folder)}: returns {passed, \
                 exit_code (0 passed, 1 a failed or errored test reaches fail_on), fail_on, summary: \
                 {tests, passed, failed, errors, skipped, warnings}, suites: [{name, tests: [{id, name, \
                 kind, status: passed|failed|error|skipped, severity, gates, message, findings: [{level, \
                 message, location?: {path, line}}], elapsed_ms, details?}]}], notes}. DAX assertions use \
                 bicraft's own evaluator; without loaded data they are skipped (require_data: true fails \
                 them). An unreadable or invalid test file is an error. Test-file format: docs/ci-tests.md",
        run: crate::citest::project_test,
        server: ServerPolicy::paths(&[
            PathParam::delegated("path"),
            PathParam::many("tests"),
            PathParam::clearable("base_dir"),
        ])
        .read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::Analyze, "Tests", "Run tests", "check"),
        shortcut: None,
        enabled: &[],
        form: &[
            P::opt("path", K::Path),
            P::opt("tests", K::Paths),
            P::opt("format", K::Json),
            P::opt("fail_on", K::Enum(&["error", "warning"])),
            P::opt("base_dir", K::Path),
        ],
    },
    Command {
        id: "about.licenses",
        label: "BIcraft's licence and the third-party notices compiled into the binary",
        params: "{}: returns {license, third_party_notices (THIRD_PARTY_NOTICES.md), files: [{name, \
                 text}] (each licence file it refers to)}; needs no open file",
        run: |s, p| crate::about::about_licenses(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::ribbon(Tab::File, "About", "Licences", "info"),
        shortcut: None,
        enabled: &[],
        form: &[],
    },
    Command {
        id: "recovery.list",
        label: "Unsaved edits that auto-recovery kept: this session's journal and those left by other sessions",
        params: "{all?: bool}: returns {enabled, journaled, edits, copy, error, offers: [{id, path, label, \
                 edits, copy, at_ms, created_ms, base: \"same\"|\"changed\"|\"missing\"|\"untitled\", recoverable, why}]}: \
                 the journals of the open file left by a crash, a closed view or a file opened over unsaved \
                 edits (with `all`, every file's, of the same user). `base` compares the file with what it \
                 was when the edits started; a changed file is recoverable only from a recovery copy. \
                 `enabled` is false when the host keeps no journal (docs/recovery.md)",
        run: |s, p| crate::recovery::recovery_list(s, p),
        server: ServerPolicy::ALLOWED.read_only(),
        replay: Replay::Skip,
        place: Place::palette("Recoverable edits", "list"),
        shortcut: None,
        enabled: &[],
        form: &[P::opt("all", K::Bool)],
    },
    Command {
        id: "recovery.apply",
        label: "Recover unsaved edits: replay a journal of the open file through the normal commands",
        params: "{id?: string (from recovery.list; default: the newest recoverable one)}: the open file must \
                 have no unsaved edits. A recovery copy, when the journal has one, replaces the model and \
                 report first; then each journaled command runs again (each its own undo step), stopping at \
                 the first failure. Returns {id, applied, total, complete, failed: {index, command, error} \
                 | null, copy, base, view: {applied, errors}}; the journal's view settings (visual text \
                 language, captions' culture, as-of date, page) are put back after the edits. A model never \
                 saved (`base: \"untitled\"` in recovery.list {all}) is recovered by `id` in any session \
                 without unsaved edits: a new model, its recovery copy, then its lines. The journal is \
                 removed: the recovered edits are journaled again in this session's own",
        run: crate::recovery::recovery_apply,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Base,
        place: Place::palette("Recover unsaved edits", "reload"),
        shortcut: None,
        enabled: &[Need::File],
        form: &[P::opt("id", K::Text)],
    },
    Command {
        id: "recovery.discard",
        label: "Discard edits auto-recovery kept: a journal, those of the open file, or all of them",
        params: "{id?: string, all?: bool, current?: bool}: removes journal `id`, else those left for the \
                 open file, or with `all` every journal of the same user (journals of running sessions \
                 stay). `current` also drops this session's own journal (the user closes without saving). \
                 Returns {discarded, offers}",
        run: crate::recovery::recovery_discard,
        server: ServerPolicy::ALLOWED,
        replay: Replay::Base,
        place: Place::palette("Discard recoverable edits", "delete"),
        shortcut: None,
        enabled: &[],
        form: &[
            P::opt("id", K::Text),
            P::opt("all", K::Bool),
            P::opt("current", K::Bool),
        ],
    },
];

fn file_open(s: &mut Session, params: &Value) -> Result<Value, String> {
    allow(params, &["path", "data_root", "cache_abf"])?;
    let data_root = crate::datasrc::data_root_param(params)?.flatten();
    let cache_abf = crate::cache_abf::param(params)?.flatten();
    let path = PathBuf::from(str_param(params, "path")?);
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    // VPAX, PBIDS, M, Power Query templates, Dataflows, CDM model.json: a new model (M11.36)
    if crate::readers::kind_of_path(&path).is_some() && !path.is_dir() {
        if cache_abf.is_some() {
            return Err("`cache_abf` applies to a PBIP project only".into());
        }
        return crate::readers::open(s, &path, data_root);
    }
    // the backup decompressed to read a PBIX model: the first query reads the data from it
    let mut backup = None;
    // under a memory budget (`docs/beyond-ram.md`, lot 2): the DataModel stays in the file and is
    // decompressed as a stream to a temporary file, from which the model and the data are read
    let mut streamed = None;
    let budget = s.preview.budget();
    // an Excel workbook: its Power Pivot model, imported into a PBIX held in memory
    let mut imported = None;
    let (source, model, report) = match ext.as_deref() {
        Some(e) if crate::workbook_import::EXTENSIONS.contains(&e) => {
            let wb = crate::workbook_import::open(&path)?;
            imported = Some(wb.info);
            (
                Source::Package {
                    path,
                    package: wb.package,
                },
                Ok(wb.model),
                Ok(Some(wb.report)),
            )
        }
        Some("pbix" | "pbit") => {
            let package = match budget {
                Some(_) => Package::open_deferring(
                    &path,
                    &bicraft_opc::ReadLimits::default(),
                    &[bicraft_pbix::DATA_MODEL],
                ),
                None => Package::open(&path),
            }
            .map_err(|e| format!("{}: {e}", path.display()))?;
            let spilled = match package.get(bicraft_pbix::DATA_MODEL) {
                Some(part)
                    if budget.is_some()
                        && part.is_deferred()
                        && part.len() <= bicraft_opc::MAX_PART_BYTES
                        && package.get(bicraft_pbix::DATA_MODEL_SCHEMA).is_none() =>
                {
                    let mut open = || {
                        part.reader()
                            .map(|r| Box::new(r) as Box<dyn std::io::Read + Send + '_>)
                            .map_err(|e| e.to_string())
                    };
                    crate::preview::open_streamed(&mut open).ok()
                }
                _ => None,
            };
            let model = match spilled {
                Some((store, sqlitedb)) => {
                    let m = bicraft_pbix::model_from_sqlitedb(&sqlitedb).map_err(|e| e.to_string());
                    if m.is_ok() {
                        streamed = Some(store);
                    }
                    m
                }
                // without a budget, or the stream failed: the usual reader (and its errors)
                None => bicraft_pbix::read_model_keeping(&package, &bicraft_pbix::DataModelLimits::default())
                    .map(|(model, abf)| {
                        backup = abf;
                        model
                    })
                    .map_err(|e| e.to_string()),
            };
            let report = bicraft_pbix::read_report(&package)
                .map(Some)
                .map_err(|e| e.to_string());
            (Source::Package { path, package }, model, report)
        }
        Some("bim") => {
            let model = bicraft_pbip::read_bim(&path).map_err(|e| e.to_string())?;
            (Source::Bim { path }, Ok(model), Ok(None))
        }
        _ if path.is_dir() || ext.as_deref() == Some("pbip") => {
            let project = bicraft_pbip::locate(&path).map_err(|e| e.to_string())?;
            if let Some(server) = s.server_config() {
                server.check_project(&project)?;
            }
            let model = bicraft_pbip::read_model(&project).map_err(|e| e.to_string());
            let report = bicraft_pbip::read_report(&project)
                .map(Some)
                .map_err(|e| e.to_string());
            (Source::Project { path, project }, model, report)
        }
        _ => {
            return Err(format!(
                "{}: expected a .pbix, .pbit, .pbip, model.bim, project folder, or an Excel workbook (.xlsx, .xlsm) \n                 with a Power Pivot data model (or .vpax, .pbids, .pq, .m, .pqt, model.json)",
                path.display()
            ));
        }
    };
    if cache_abf.is_some() {
        crate::cache_abf::require_project(Some(&source))?;
    }
    let (model, model_error) = match model {
        Ok(m) => (Some(m), None),
        Err(e) => (None, Some(e)),
    };
    // a thin report: its model is published in the Power BI service (docs/gaps.md G12)
    let model_error = match thin_package(Some(&source)) {
        Some(live) if model.is_none() => Some(thin_model_error(live.as_ref())),
        _ => model_error,
    };
    let (report, report_error) = match report {
        Ok(r) => (r, None),
        Err(e) => (None, Some(e)),
    };
    s.source = Some(source);
    if let Some(Source::Package { package, .. }) = s.source.as_ref()
        && let Some(part) = package.get(bicraft_pbix::DATA_MODEL)
    {
        if let Some(abf) = backup {
            s.preview.seed_backup(part, abf);
        } else if let Some(store) = streamed {
            s.preview.seed_store(part, store);
        }
    }
    s.model = model;
    s.model_error = model_error;
    s.dirty = false;
    s.report = report;
    s.report_error = report_error;
    s.report_dirty = false;
    s.diagram_layout = crate::diagram_layout::at_open(s);
    crate::dax_queries::at_open(s);
    s.data_root = data_root;
    s.cache_abf = cache_abf;
    let mut out = file_inspect(s, &Value::Null)?;
    if let (Some(info), Some(o)) = (imported, out.as_object_mut()) {
        o.insert("imported".into(), info);
    }
    Ok(out)
}

/// When `source` is a thin report (a package without `DataModel` or `DataModelSchema`): its live
/// connection, `None` inside when its `Connections` part does not read.
pub(crate) fn thin_package(source: Option<&Source>) -> Option<Option<bicraft_pbix::LiveConnection>> {
    match source? {
        Source::Package { package, .. }
            if package.get(bicraft_pbix::DATA_MODEL).is_none()
                && package.get(bicraft_pbix::DATA_MODEL_SCHEMA).is_none() =>
        {
            Some(bicraft_pbix::live_connection(package))
        }
        _ => None,
    }
}

/// The model error of a thin report: where its model is, and that BIcraft does not open it, a
/// known limitation not pursued by the maintainers' decision (docs/gaps.md G12, 2026-11-24).
pub(crate) fn thin_model_error(live: Option<&bicraft_pbix::LiveConnection>) -> String {
    let via = match live {
        Some(c) => {
            let mut parts = Vec::new();
            if let Some(t) = &c.connection_type {
                parts.push(t.clone());
            }
            if let Some(d) = &c.data_source {
                parts.push(format!("to {d}"));
            }
            if let Some(m) = &c.catalog {
                parts.push(format!("model {m}"));
            }
            if parts.is_empty() {
                String::new()
            } else {
                format!(" ({})", parts.join(", "))
            }
        }
        None => " (its Connections part could not be read)".to_string(),
    };
    format!(
        "a thin report: its semantic model is not in the file but published in the Power BI service, \
         reached through a live connection{via}. BIcraft reads and edits the report; it does not sign in \
         to Power BI, so the published model is not opened, queried or refreshed. This is a known \
         limitation, not pursued by the maintainers' decision (docs/gaps.md G12)"
    )
}

pub(crate) fn source_path(source: &Source) -> &Path {
    match source {
        Source::Package { path, .. } | Source::Project { path, .. } | Source::Bim { path } => path,
    }
}

/// `session.revision`.
pub(crate) fn session_revision(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &[])?;
    Ok(json!({"revision": s.revision, "dirty": s.dirty, "report_dirty": s.report_dirty}))
}

pub(crate) fn file_inspect(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &[])?;
    let model = s
        .model
        .as_ref()
        .map(|m| json!({"name": m.name, "tables": m.tables.len()}));
    let report = s.report.as_ref().map(|r| {
        json!({
            "pages": r.pages.len(),
            "visuals": r.pages.iter().map(|p| p.visuals.len()).sum::<usize>(),
        })
    });
    let dirty = s.dirty || s.report_dirty;
    let Some(source) = s.source.as_ref() else {
        return match model {
            Some(model) => Ok(json!({
                "path": null,
                "details": {"type": "new", "note": "not saved yet: use project.new or file.save_as"},
                "model": model,
                "model_error": null,
                "report": report,
                "report_error": s.report_error,
                "dirty": dirty,
                "report_dirty": s.report_dirty,
            })),
            None => Err("no file is open (run file.open or model.new first)".into()),
        };
    };
    let mut ignored = Vec::new();
    let copilot = crate::copilot::summary(&crate::copilot::source_files(s, &mut ignored));
    let details = match source {
        Source::Package { package, .. } => {
            json!({"type": "package", "inventory": bicraft_pbix::inventory(package), "copilot": copilot})
        }
        Source::Project { project, .. } => json!({"type": "project", "project": project, "copilot": copilot}),
        Source::Bim { .. } => json!({"type": "model.bim"}),
    };
    Ok(json!({
        "path": source_path(source).display().to_string(),
        "details": details,
        "model": model,
        "model_error": s.model_error,
        "report": report,
        "report_error": s.report_error,
        "dirty": dirty,
        "report_dirty": s.report_dirty,
        "data_root": s.data_root.as_ref().map(|r| r.display().to_string()),
        "cache_abf": s.cache_abf.as_ref().map(|p| p.display().to_string()),
        "live_connection": thin_package(Some(source)).flatten(),
    }))
}

pub(crate) fn model(s: &Session) -> Result<&bicraft_model::SemanticModel, String> {
    s.model.as_ref().ok_or_else(|| match &s.model_error {
        Some(e) => format!("the open file has no readable model: {e}"),
        None => "no model is open (run file.open or model.new first)".into(),
    })
}

pub(crate) fn model_summary(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &[])?;
    let m = model(s)?;
    Ok(json!({
        "name": m.name,
        "compatibility_level": m.compatibility_level,
        "culture": m.culture,
        "tables": m.tables.iter().map(|t| json!({
            "name": t.name,
            "hidden": t.is_hidden,
            "columns": t.columns.iter().filter(|c| !c.is_row_number()).map(|c| json!({"name": c.name, "data_type": c.data_type, "calculated": c.expression.is_some()})).collect::<Vec<_>>(),
            "measures": t.measures.iter().map(|x| &x.name).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "relationships": m.relationships,
        "roles": m.roles.iter().map(|r| json!({
            "name": r.name,
            "model_permission": r.model_permission,
            "row_filters": r.table_permissions.iter()
                .filter_map(|tp| tp.filter_expression.as_ref().map(|f| json!({"table": tp.table, "filter_expression": f})))
                .collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    }))
}

pub(crate) fn dax_validate(s: &Session, params: &Value) -> Result<Value, String> {
    allow(params, &["expression", "table", "kind"])?;
    let expression = str_param(params, "expression")?;
    let table = crate::params::opt_str(params, "table")?;
    let kind = match crate::params::opt_str(params, "kind")? {
        None => ExprKind::Expression,
        Some("measure") => ExprKind::Measure,
        Some("calculated_column") => ExprKind::CalculatedColumn,
        Some("calculated_table") => ExprKind::CalculatedTable,
        Some(other) => {
            return Err(format!(
                "unknown kind {other:?}: measure, calculated_column or calculated_table"
            ));
        }
    };
    let ctx = Context {
        model: s.model.as_ref(),
        table,
    };
    let v = bicraft_dax::validate_as(expression, &ctx, kind);
    Ok(json!({"valid": v.valid, "checked_against_model": s.model.is_some(), "diagnostics": v.diagnostics}))
}

/// The words of DAX expressions that are not functions (the formula bar's autocomplete).
const DAX_KEYWORDS: &[&str] = &["VAR", "RETURN", "IN", "NOT", "ASC", "DESC"];

/// `dax.functions`: the catalog with each signature as text.
pub(crate) fn dax_functions(_: &Session, params: &Value) -> Result<Value, String> {
    allow(params, &["query"])?;
    let query = crate::params::opt_str(params, "query")?.map(str::to_uppercase);
    let functions: Vec<Value> = bicraft_dax::all_functions()
        .iter()
        .filter(|f| query.as_deref().is_none_or(|q| f.name.contains(q)))
        .map(|f| {
            let params: Vec<Value> = f
                .params
                .iter()
                .map(|p| json!({"name": p.name, "optional": p.optional, "repeating": p.repeating}))
                .collect();
            let (en, fr) = bicraft_dax::function_description(f.name).unwrap_or_default();
            json!({
                "name": f.name,
                "category": f.category,
                "signature": bicraft_dax::signature_text(f),
                "params": params,
                "deprecated": f.deprecated,
                "description": en,
                "description_fr": fr,
            })
        })
        .collect();
    Ok(json!({"functions": functions, "keywords": DAX_KEYWORDS}))
}

/// One DAX object of the model, as `model.validate` reports it.
pub(crate) struct DaxObject<'m> {
    /// `measure`, `calculated column`, `calculated table` or `row filter`.
    pub label: &'static str,
    kind: ExprKind,
    pub table: &'m str,
    /// Object name (the role's name for a row filter).
    pub name: &'m str,
    pub expression: &'m str,
    /// Home table for binding (`None` for calculated tables).
    home: Option<&'m str>,
    id: ObjectRef,
}

pub(crate) fn dax_objects(m: &bicraft_model::SemanticModel) -> Vec<DaxObject<'_>> {
    let mut out = Vec::new();
    for t in &m.tables {
        for x in &t.measures {
            out.push(DaxObject {
                label: "measure",
                kind: ExprKind::Measure,
                table: &t.name,
                name: &x.name,
                expression: &x.expression,
                home: Some(&t.name),
                id: ObjectRef::Measure {
                    table: t.name.clone(),
                    name: x.name.clone(),
                },
            });
        }
        for c in &t.columns {
            let Some(expression) = c.expression.as_deref() else {
                continue;
            };
            out.push(DaxObject {
                label: "calculated column",
                kind: ExprKind::CalculatedColumn,
                table: &t.name,
                name: &c.name,
                expression,
                home: Some(&t.name),
                id: ObjectRef::Column {
                    table: t.name.clone(),
                    name: c.name.clone(),
                },
            });
        }
        if let Some(expression) = deps::calculated_table_expression(t) {
            out.push(DaxObject {
                label: "calculated table",
                kind: ExprKind::CalculatedTable,
                table: &t.name,
                name: &t.name,
                expression,
                home: None,
                id: ObjectRef::Table { name: t.name.clone() },
            });
        }
        // Calculation items (and their format strings), validated like measures of the group.
        let items = t
            .extra
            .get("calculationGroup")
            .and_then(|g| g.get("calculationItems"))
            .and_then(Value::as_array);
        for item in items.into_iter().flatten() {
            let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
            let format = item
                .get("formatStringDefinition")
                .and_then(|f| f.get("expression"))
                .and_then(Value::as_str);
            let exprs = [
                ("calculation item", item.get("expression").and_then(Value::as_str)),
                ("calculation item format string", format),
            ];
            for (label, expression) in exprs {
                let Some(expression) = expression else {
                    continue;
                };
                out.push(DaxObject {
                    label,
                    kind: ExprKind::Measure,
                    table: &t.name,
                    name,
                    expression,
                    home: Some(&t.name),
                    id: ObjectRef::Table { name: t.name.clone() },
                });
            }
        }
    }
    for r in &m.roles {
        for tp in &r.table_permissions {
            let Some(expression) = tp.filter_expression.as_deref() else {
                continue;
            };
            out.push(DaxObject {
                label: "row filter",
                kind: ExprKind::RowFilter,
                table: &tp.table,
                name: &r.name,
                expression,
                home: Some(&tp.table),
                id: ObjectRef::RowFilter {
                    role: r.name.clone(),
                    table: tp.table.clone(),
                },
            });
        }
    }
    out
}

pub(crate) fn model_validate(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &[])?;
    model_validation(s)
}

/// `model.validate` on a shared borrow (also used by [`Session::snapshot`]).
pub(crate) fn model_validation(s: &Session) -> Result<Value, String> {
    let m = model(s)?;
    let graph = deps::dependencies(m);
    let cycles = deps::cycles_of(&graph);
    // Each object is in at most one cycle; keyed so the pass stays linear in big cycles.
    let mut cycle_diags: HashMap<ObjectRef, Vec<bicraft_dax::Diagnostic>> = HashMap::new();
    for (id, d) in deps::cycle_diagnostics_of(&graph) {
        cycle_diags.entry(id).or_default().push(d);
    }
    let mut results = Vec::new();
    let mut invalid = 0usize;
    let mut checked = 0usize;
    // One name index and type cache for the whole pass (linear in the model, gap G5).
    let mut validator = Validator::new(m);
    for o in dax_objects(m) {
        checked += 1;
        let v = validator.validate_as(o.expression, o.home, o.kind);
        let mut diagnostics = v.diagnostics;
        let mut valid = v.valid;
        if let Some(ds) = cycle_diags.remove(&o.id) {
            valid &= ds.is_empty();
            diagnostics.extend(ds);
        }
        if !valid {
            invalid += 1;
        }
        if !diagnostics.is_empty() {
            let mut entry = json!({"table": o.table, "kind": o.label, "name": o.name, "valid": valid,
                "diagnostics": diagnostics, "result_type": v.result_type});
            if o.kind == ExprKind::RowFilter {
                entry["role"] = json!(o.name);
            }
            results.push(entry);
        }
    }
    let (m_checked, m_invalid, m_results) = crate::mquery::model_m_validation(m);
    let relationships = crate::relationship_set::relationship_problems(m);
    Ok(json!({
        "valid": invalid == 0 && m_invalid == 0 && relationships.is_empty(),
        "relationships": {
            "valid": relationships.is_empty(),
            "checked_count": m.relationships.len(),
            "invalid": relationships,
        },
        "invalid_count": invalid,
        "checked_count": checked,
        "objects_with_diagnostics": results,
        "cycles": cycles,
        "m": {
            "valid": m_invalid == 0,
            "checked_count": m_checked,
            "invalid_count": m_invalid,
            "objects_with_diagnostics": m_results,
        },
    }))
}

pub(crate) fn dax_validate_query(s: &Session, params: &Value) -> Result<Value, String> {
    allow(params, &["query"])?;
    let query = str_param(params, "query")?;
    let ctx = Context {
        model: s.model.as_ref(),
        table: None,
    };
    let (v, parsed) = bicraft_dax::validate_query_parsed(query, &ctx);
    let definitions: Vec<Value> = parsed
        .as_ref()
        .map(|q| {
            q.defines
                .iter()
                .map(|d| match d {
                    bicraft_dax::ast::Define::Measure { table, name, .. }
                    | bicraft_dax::ast::Define::Column { table, name, .. } => {
                        json!({"kind": d.kind(), "table": table, "name": name})
                    }
                    _ => json!({"kind": d.kind(), "name": d.name()}),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(json!({
        "valid": v.valid,
        "checked_against_model": s.model.is_some(),
        "diagnostics": v.diagnostics,
        "definitions": definitions,
        "evaluate_count": parsed.as_ref().map_or(0, |q| q.statements.len()),
    }))
}
