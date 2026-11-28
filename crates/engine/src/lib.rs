//! L5: the session and its commands. Everything the tool can do is a [`Command`]; the CLI and the
//! MCP server only translate their inputs into [`Session::execute`] calls, so both always behave
//! the same.
//!
//! Model edits (`table.*`, `column.*`, `measure.*`, `relationship.*`, `role.*`) validate every DAX
//! expression they introduce against the model as it would be after the edit, and are rejected
//! unless it binds (or the params say `"force": true`). `file.save` / `file.save_as` /
//! `project.new` write the model through the format crates.
//!
//! Report edits (`report.*`, `page.*`, `visual.*`) work on the report `file.open` read next to
//! the model; visual fields are resolved against the model. PBIP projects save it as PBIR.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

mod about;
mod aggs_derive;
mod analyze;
mod bookmarks;
mod bound_values;
mod builtin_visuals;
pub mod bvis;
mod cache_abf;
mod calc_group;
mod call_scope;
mod canvas_edit;
pub mod chart_pack;
mod citest;
mod citest_dax;
mod citest_loc;
mod citest_out;
mod citest_rules;
mod citest_spec;
mod citest_yaml;
mod clock;
mod command_ui;
mod commands;
mod convert;
mod copilot;
mod custom_visuals;
mod datasrc;
mod date_table;
mod dax_define;
mod dax_queries;
mod dax_rename;
mod desktop_defaults;
mod desktop_open;
mod diagram_layout;
mod diff;
mod edit;
mod envprofile;
mod export_data;
mod export_pdf;
mod field_param_queries;
mod filter_edit;
mod filter_pane_format;
mod filter_relative;
mod format_painter;
mod get_data;
mod groups;
mod hierarchy;
mod hierarchy_set;
mod history;
mod images;
mod import;
mod interact;
mod layout;
mod m_assist;
mod model_get;
mod model_props;
mod mquery;
mod obs;
mod optimize;
mod parameters;
mod params;
mod partition;
mod paths;
mod pbix_new;
mod pbix_struct;
mod perf;
mod perspective_edit;
mod perspectives;
mod preview;
mod query_deps;
mod query_diagnose;
mod query_export;
mod query_folding;
mod query_preview;
mod query_profile;
mod query_steps;
mod query_transforms;
mod quick;
pub mod readers;
mod recovery;
mod redact;
mod refresh;
mod refresh_info;
mod refresh_pbix;
mod refresh_policy;
mod refresh_table;
mod refs;
mod relationship_autodetect;
mod relationship_review;
mod relationship_set;
mod rename_inferred;
mod rename_refs;
mod render;
mod report;
mod report_apply;
mod report_checks;
mod report_fix;
mod report_lint;
mod report_measures;
mod report_repair;
mod report_settings;
mod rich;
mod role_manage;
mod role_permission;
mod roles;
mod sandbox;
mod save;
mod see_records;
mod set_properties;
mod shape_maps;
mod shared;
mod slicer_sync;
mod snapshot;
mod source_settings;
mod sources;
mod sparklines;
pub mod spec_visual;
mod sqlserver;
mod storage_mode;
mod storage_stats;
mod synonyms;
mod theme;
mod theme_edit;
mod tmdl_script;
mod translations;
mod udf;
mod ui_lang;
mod usage;
mod usage_report;
pub mod view_drawings;
mod visual_analyze;
mod visual_calcs;
mod visual_catalog;
mod visual_conditional;
mod visual_extras;
mod visual_fields;
mod visual_project;
mod visual_props;
pub mod visual_runtime;
mod visual_series;
mod visual_states;
mod vpax;
pub mod watch;
mod webdata;
mod workbook_import;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use bicraft_model::SemanticModel;
use bicraft_opc::Package;
use serde::Serialize;
use serde_json::Value;

pub use about::licenses_text;
pub use bicraft_databricks::Token as DatabricksToken;
pub use bicraft_report;
pub use bicraft_report::Report;
pub use bicraft_sqlserver::{Credential as SqlCredential, Password as SqlPassword, Settings as SqlSettings};
pub use clock::{LocalClock, MAX_OFFSET_MINUTES, OffsetFn, local_clock, set_local_clock};
pub use command_ui::{At, Caller, Kind, Menu, Need, Obj, Param, Place, Tab, UI_SHORTCUTS};
pub use commands::COMMANDS;
pub use desktop_open::{DESKTOP_EXTENSIONS, DesktopTarget, ExternalOpener, Opener, open_with_default_app};
pub use history::{DEFAULT_MAX_BYTES, DEFAULT_MAX_STEPS, HistoryLimits};
pub use images::SnapshotAsset;
pub use interact::{Interactions, ViewAs};
pub use obs::{EngineGauges, engine_gauges};
pub use recovery::{DEFAULT_MAX_JOURNAL_BYTES, RECOVERY_DIR_ENV, RecoveryConfig, Replay, recovery_offers};
pub use refresh::{SNAPSHOT_FILE, TokenSource, token_from_env};
pub use sandbox::{
    Access, Effect, PathKind, PathParam, Refusal, SERVER_NETWORK_ENV, SERVER_ROOT_ENV, ServerConfig,
    ServerPolicy, server_policy,
};
pub use shared::{SharedFn, runs_shared, shared_fn};
pub use snapshot::{SNAPSHOT_VERSION, SnapshotLimits};
pub use source_settings::{PrivacyLevel, PrivacyLevels};
pub use sqlserver::SqlCredentials;
pub use ui_lang::{SUPPORTED_LANGUAGES, UiLanguage};

/// What `file.open` loaded.
#[derive(Debug, Clone)]
pub enum Source {
    /// PBIX or PBIT package.
    Package { path: PathBuf, package: Package },
    /// PBIP project folder.
    Project {
        path: PathBuf,
        project: bicraft_pbip::Project,
    },
    /// Standalone `model.bim`.
    Bim { path: PathBuf },
}

#[derive(Debug, Default)]
pub struct Session {
    pub source: Option<Source>,
    pub model: Option<SemanticModel>,
    /// Why `model` is empty although a file is open (e.g. binary PBIX model), for messages.
    pub model_error: Option<String>,
    /// The model has edits that are not saved to `source` yet (always true for a `model.new`
    /// model until it is saved).
    pub dirty: bool,
    /// The open file's report (pages and visuals), when it has one that could be read.
    pub report: Option<Report>,
    /// Why `report` is empty although a file is open, for messages. Never fatal: the model stays
    /// usable.
    pub report_error: Option<String>,
    /// The report has edits that are not saved yet.
    pub report_dirty: bool,
    /// The model diagram (`DiagramLayout` / `diagramLayout.json`): the open file's, read by
    /// `file.open`, as edited by `diagram.move`; `None`: no layout (`diagram_layout.rs`). Undone
    /// with the model; an edit makes the model dirty (the layout is saved with it).
    pub diagram_layout: Option<String>,
    /// Bumped by one after every successful command that may have changed the session (model,
    /// report, source, dirty flags); read-only commands leave it alone. Watchers (the live viewer)
    /// compare it to know when to refresh.
    pub revision: u64,
    /// The last command [`Session::execute`] ran, successful or not.
    pub last_command: Option<LastCommand>,
    /// The only folder Power Query `File.Contents` may read when partition data is loaded
    /// (structural PBIX saves, `table.import`, the viewer). `None` (the default): no file
    /// access. Set explicitly with a command's `data_root` parameter; `file.open` resets it.
    pub data_root: Option<PathBuf>,
    /// UI language of the generated visual text in [`Session::snapshot`] and `report.render`
    /// (auto titles, "(Blank)", "Total", …; M10.2). en-US by default; `view.set_language` sets
    /// it, `file.open` keeps it.
    pub ui_language: UiLanguage,
    /// The culture whose model translations (captions) [`Session::snapshot`] and `report.render`
    /// show (M10.4); `view.set_culture` sets it, `file.open` keeps it. `None`: stored names.
    pub display_culture: Option<String>,
    /// The date and time the DAX evaluator's `TODAY()`/`NOW()` and relative date filters use
    /// (days since 1899-12-30, the fraction is the time of day); `None`: the clock, in the local
    /// time of the process's clock ([`set_local_clock`]; UTC by default).
    /// `view.set_as_of` sets it; `report.render` and `dax.evaluate` take an `as_of` parameter.
    pub as_of: Option<f64>,
    /// An explicit file a PBIP project's stored data is read from instead of its own
    /// `.pbi/cache.abf` (`file.open {cache_abf}`, `file.set_cache_abf`; `cache_abf.rs`). Read-only:
    /// never written or copied. `file.open` resets it.
    pub cache_abf: Option<PathBuf>,
    /// How `file.open_in_desktop` hands the saved file to the OS (`desktop_open.rs`). `None`
    /// (the default): [`open_with_default_app`]. Tests inject one so nothing is launched.
    pub external_opener: Option<ExternalOpener>,
    /// The live viewer's reader state (`interact.rs`): slicer selections, the data-point
    /// selection, the page shown. [`Session::snapshot`] shows the report under it; it is never
    /// saved. `view.interact` changes it.
    pub interactions: Interactions,
    /// Where `model.refresh` gets the Databricks token when it is allowed to run that connector
    /// (`refresh.rs`). `None` (the default): the environment variable `BICRAFT_DATABRICKS_TOKEN`.
    /// The token is read once per refresh, when the first request is made, and never stored.
    pub token_source: Option<TokenSource>,
    /// Credentials of SQL Server sources for this session (`source.credentials`, `sqlserver.rs`):
    /// in memory only, never saved, journaled or returned.
    pub sql_credentials: SqlCredentials,
    /// Credentials of web sources (`Web.Contents`, `OData.Feed`, `SharePoint.*`) by URL prefix
    /// (`source.credentials {kind: web}`, `source_settings.rs`): in memory only, never saved,
    /// journaled or returned.
    pub web_credentials: bicraft_webdata::auth::Store,
    /// The privacy level of each data source (`source.privacy`): recorded and shown, in memory
    /// for the session.
    pub privacy: PrivacyLevels,
    /// How the SQL Server connector connects; `None` (the default): [`SqlSettings::from_env`]
    /// (`BICRAFT_SQLSERVER_CA_FILE`, `BICRAFT_SQLSERVER_TRUST_SERVER_CERTIFICATE`).
    pub sql_settings: Option<SqlSettings>,
    /// Tests: a fake Databricks warehouse the connector talks to instead of the real host.
    #[cfg(test)]
    pub(crate) test_endpoint: Option<refresh::TestEndpoint>,
    /// Data-preview cache for [`Session::snapshot`] (decoded columns, evaluated visuals).
    preview: preview::PreviewCache,
    /// The Performance analyzer's recording (`perf.rs`): diagnostics, never saved.
    perf: perf::Recorder,
    /// Resolved report images for [`Session::snapshot`] / [`Session::snapshot_assets`].
    images: images::ImageCache,
    /// Undo/redo steps and the atomic batch in progress (`history.rs`).
    history: history::History,
    /// The auto-recovery journal (`recovery.rs`); off unless [`Session::set_recovery`] turned it
    /// on.
    recovery: recovery::Recovery,
    /// Refresh state of the open file's backup, per revision (`refresh_info.rs`).
    refresh: refresh_info::Cache,
    /// Server mode (`sandbox.rs`, `docs/server-mode.md`): the workspace root every path parameter is
    /// resolved under, and the command policy. `None` (the default): local mode, as before. Set
    /// only by [`Session::server`] / [`Session::from_env`], never changed afterwards.
    server: Option<sandbox::ServerConfig>,
    /// Raised from another thread to stop the running evaluation ([`Session::set_cancel`]).
    cancel: Option<Arc<AtomicBool>>,
    /// The DAX query view's tabs of the open file (`dax_queries.rs`).
    pub(crate) dax_queries: dax_queries::State,
    /// What `dax.cancel` moves on to stop the running `dax.evaluate` runs (`dax_queries.rs`).
    dax_cancel: dax_queries::Cancel,
    /// The order of [`Session::last_command`] among the session's runs (shared runs included).
    last_seq: u64,
    /// The last shared run ([`Session::execute_shared`]) and its order.
    last_shared: std::sync::Mutex<Option<(u64, LastCommand)>>,
}

/// What [`Session::execute`] ran last.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LastCommand {
    pub id: String,
    pub ok: bool,
    pub error: Option<String>,
    /// Unix time in milliseconds when the command finished.
    pub at_ms: u64,
}

/// Commands that never change the session: they do not bump [`Session::revision`].
pub const READ_ONLY_COMMANDS: &[&str] = &[
    "session.revision",
    "file.inspect",
    "dax.format",
    "dax.define_query",
    "dax.cancel",
    "dax.queries",
    "file.convert",
    "file.redact",
    "file.diff",
    "file.textconv",
    "file.open_in_desktop",
    "model.summary",
    "model.validate",
    "model.analyze",
    "model.usage",
    "model.unused",
    "model.get",
    "table.get",
    "column.get",
    "column.values",
    "table.rows",
    "query.preview",
    "query.profile",
    "query.dependencies",
    "query.list",
    "query.steps",
    "query.apply_step",
    "query.edit_step",
    "query.remove_step",
    "query.new",
    "query.rename",
    "query.folding",
    "query.native",
    "query.diagnose",
    "source.navigate",
    "source.settings",
    "table.entered_data",
    "visual.records",
    "visual.analyze",
    "visual.explain_anomaly",
    "measure.get",
    "relationship.list",
    "translation.list",
    "model.storage_stats",
    "model.export_vpax",
    "dax.validate",
    "dax.validate_query",
    "role.check_filter",
    "dax.functions",
    "m.validate",
    "m.complete",
    "m.signature",
    "m.hover",
    "dax.evaluate",
    "tmdl.script",
    "report.summary",
    "report.validate",
    "report.check",
    "report.extract",
    "report.render",
    // exports write a file, never the session's model or report
    "report.export_pdf",
    "report.print",
    "visual.export_data",
    "query.export",
    "visual.props",
    "visual.options",
    "visual.copy",
    "theme.get",
    "theme.export",
    // custom visual projects: files in a project folder, never the session
    "visual.scaffold",
    "visual.build",
    "visual.pack",
    // BIcraft's visuals written as .pbiviz files, never the session
    "visual.export_bicraft_visual",
    // a starting spec for the BIcraft Spec visual
    "visual.spec_template",
    "edit.history",
    // the viewer's interaction state (slicer selections, cross-filtering, page) is not the
    // model, the report or the source: no revision bump, no undo step
    "view.interact",
    "view.interactions",
    // read-only listings of the report
    "report_measure.list",
    "report.custom_visuals",
    "slicer.sync_list",
    "bookmark.list",
    // CI gates: reads the open file, or opens `path` in a private session
    "project.test",
    "about.licenses",
    // data sources and environment profiles: listings and checks
    "sources.list",
    "env.list",
    "env.check",
    // auto-recovery: the listing of recoverable journals
    "recovery.list",
    // the Performance analyzer's recording is diagnostics, not the model, the report or the
    // source: no revision bump, no undo step (`perf.rs`)
    "perf.start",
    "perf.stop",
    "perf.clear",
    "perf.refresh",
    "perf.render",
    "perf.results",
    "perf.export",
    // Refresh visuals asks the next view to evaluate once: the report is not changed
    "report.refresh_visuals",
    // a visual's Refresh, likewise
    "visual.refresh",
];

/// Whether `id` may change the session (every command but [`READ_ONLY_COMMANDS`]).
pub fn is_mutating(id: &str) -> bool {
    !READ_ONLY_COMMANDS.contains(&id)
}

/// A process-wide order of command runs ([`Session::last_command_now`]).
fn next_seq() -> u64 {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// A user-visible operation. Ids are dotted (`file.open`, `dax.validate`).
pub struct Command {
    pub id: &'static str,
    pub label: &'static str,
    /// Human-readable parameter documentation, shown by `describe` and `list_commands`.
    pub params: &'static str,
    pub run: fn(&mut Session, &Value) -> Result<Value, String>,
    /// What the command may do in server mode, and which parameters are filesystem paths
    /// (`sandbox.rs`). Every command states it, so a new one has to decide.
    pub server: ServerPolicy,
    /// Where its button lives in the app: a ribbon tab and group, a context menu, or the palette
    /// only, with a short label and an icon (`command_ui.rs`, `docs/commands-ui.md`).
    pub place: Place,
    /// Its key chord (`Ctrl+S`), unique across the table; `None` for most.
    pub shortcut: Option<&'static str>,
    /// What the session needs before the button is enabled ([`Session::command_enabled`]).
    pub enabled: &'static [Need],
    /// The parameters as a form schema (names, kinds, required), for the palette's form.
    pub form: &'static [Param],
    /// How the auto-recovery journal records it (`recovery.rs`): replayed from its parameters,
    /// a full copy after it (outside inputs), not at all, or a new journal base.
    pub replay: Replay,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommandInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub params: &'static str,
    /// Where the command's button lives (`command_ui.rs`).
    pub place: Place,
    pub shortcut: Option<&'static str>,
    /// The parameters as a form schema.
    pub form: &'static [Param],
    /// What the session needs before the button is enabled ([`Need::name`]).
    pub needs: Vec<&'static str>,
    /// Whether the command can run now, when listed for a session
    /// ([`Session::available_commands`], [`Session::reader_commands`]); absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Why not, when `enabled` is false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
}

impl Command {
    pub fn info(&self) -> CommandInfo {
        CommandInfo {
            id: self.id,
            label: self.label,
            params: self.params,
            place: self.place,
            shortcut: self.shortcut,
            form: self.form,
            needs: self.enabled.iter().map(|n| n.name()).collect(),
            enabled: None,
            disabled_reason: None,
        }
    }

    /// [`Command::info`] with `enabled` / `disabled_reason` for `caller` on session `s`.
    pub fn info_for(&self, s: &Session, caller: Caller) -> CommandInfo {
        let state = s.command_enabled(self.id, caller);
        CommandInfo {
            enabled: Some(state.is_ok()),
            disabled_reason: state.err(),
            ..self.info()
        }
    }
}

pub fn find_command(id: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|c| c.id == id)
}

pub fn list_commands(filter: Option<&str>) -> Vec<CommandInfo> {
    let filter = filter.map(str::to_ascii_lowercase);
    COMMANDS
        .iter()
        .filter(|c| {
            filter
                .as_deref()
                .is_none_or(|f| c.id.contains(f) || c.label.to_ascii_lowercase().contains(f))
        })
        .map(Command::info)
        .collect()
}

/// The error text of a DAX evaluation stopped by [`Session::set_cancel`]'s flag.
pub const CANCELLED: &str = "cancelled by the client";

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets (or clears) the flag another thread raises to stop the running command. The DAX
    /// evaluator checks it where it checks its time budget (`dax.evaluate` then fails with
    /// [`CANCELLED`]); other commands run to completion. The MCP HTTP transport raises it when
    /// the client closes the request's response stream.
    pub fn set_cancel(&mut self, flag: Option<Arc<AtomicBool>>) {
        self.cancel = flag;
    }

    /// Whether the cancel flag ([`Session::set_cancel`]) is raised.
    pub fn is_cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|f| f.load(std::sync::atomic::Ordering::Relaxed))
            || call_scope::cancelled()
            || self.dax_cancelled()
    }

    /// The last command run on this session, shared runs ([`Session::execute_shared`])
    /// included: what the live viewer shows.
    pub fn last_command_now(&self) -> Option<LastCommand> {
        let shared = self
            .last_shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        match shared {
            Some((seq, c)) if seq > self.last_seq => Some(c),
            _ => self.last_command.clone(),
        }
    }

    fn record_shared(&self, c: LastCommand) {
        let seq = next_seq();
        *self
            .last_shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((seq, c));
    }

    /// Runs one command. A panic inside a command is caught and reported as an error. This is a
    /// safety net, not a licence to panic. Commands do all fallible work first and mutate the
    /// session last, so an error or panic leaves the session as it was without cloning it (a
    /// session can hold a multi-gigabyte package).
    ///
    /// Records the outcome in [`Session::last_command`] and bumps [`Session::revision`] when a
    /// command that may change the session succeeds.
    pub fn execute(&mut self, id: &str, params: &Value) -> Result<Value, String> {
        // `recovery.apply` replays commands through this function: each is its own history step
        // and journal line, so the apply itself is neither
        let replaying = id == "recovery.apply";
        // the relationship dialogs' review (`check: true`) changes nothing: no undo step, no
        // journal line, no new revision (the preview's caches stay)
        // and so does `m.format` without `apply` (it only returns the formatted text), and the
        // visual calculation window's live preview (`preview: true`)
        let probe = (matches!(id, "relationship.add" | "relationship.set")
            && params.get("check") == Some(&Value::Bool(true)))
            || (id == "m.format" && params.get("apply") != Some(&Value::Bool(true)))
            || (matches!(id, "visual.calculation.add" | "visual.calculation.set")
                && params.get("preview") == Some(&Value::Bool(true)));
        let before = if replaying || probe {
            None
        } else {
            history::before(self, id)
        };
        let journal = if replaying || probe {
            None
        } else {
            recovery::before(self, id, params)
        };
        let queries = mquery::before(self, id);
        let result = match find_command(id) {
            None => Err(format!("unknown command `{id}` (see `list_commands`)")),
            Some(cmd) => match sandbox::prepare(self.server.as_ref(), cmd, params) {
                Err(e) => Err(e),
                Ok(params) => match catch_unwind(AssertUnwindSafe(|| (cmd.run)(self, &params))) {
                    Ok(result) => result,
                    Err(_) => Err(format!("internal error in `{id}`")),
                },
            },
        };
        if result.is_ok() {
            mquery::after(self, queries);
        }
        // a visual bound to a field parameter follows the model (part of the command's undo
        // step); opening, saving and undoing leave the report as it is
        if result.is_ok()
            && !probe
            && is_mutating(id)
            && !["file.", "project.", "edit.", "recovery."]
                .iter()
                .any(|p| id.starts_with(p))
            && field_param_queries::sync(self.model.as_ref(), self.report.as_mut())
        {
            self.report_dirty = true;
        }
        // a report read from a legacy layout (opened, or read back after a save) has its field
        // parameter bindings as plain fields: read them as bound to the parameter again (no edit)
        if result.is_ok()
            && !probe
            && ["file.", "project."].iter().any(|p| id.starts_with(p))
            && convert::source_is_legacy(self)
        {
            field_param_queries::adopt_legacy(self.model.as_ref(), self.report.as_mut());
        }
        history::after(self, id, before, result.is_ok());
        if result.is_ok() && is_mutating(id) && !probe {
            self.revision = self.revision.saturating_add(1);
            self.preview.revised();
        }
        recovery::after(self, id, params, journal, result.is_ok());
        self.last_seq = next_seq();
        self.last_command = Some(LastCommand {
            id: id.to_string(),
            ok: result.is_ok(),
            error: result.as_ref().err().cloned(),
            at_ms: now_ms(),
        });
        result
    }

    /// Runs commands in order and stops at the first error, returning the results so far. The
    /// steps before the error stay applied (each is its own undo step); see
    /// [`Session::batch_atomic`] for all-or-nothing.
    pub fn batch(&mut self, steps: &[(String, Value)]) -> Result<Vec<Value>, (usize, String, Vec<Value>)> {
        let mut out = Vec::with_capacity(steps.len());
        for (i, (id, params)) in steps.iter().enumerate() {
            match self.execute(id, params) {
                Ok(v) => out.push(v),
                Err(e) => return Err((i, e, out)),
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod filter_edit_tests;
#[cfg(test)]
mod filter_relative_tests;
#[cfg(test)]
mod m92_tests;
#[cfg(test)]
mod query_deps_tests;
#[cfg(test)]
mod query_profile_tests;
#[cfg(test)]
mod query_steps_tests;
#[cfg(test)]
mod query_transforms_tests;
#[cfg(test)]
mod report_apply_tests;
#[cfg(test)]
mod report_tests;
#[cfg(test)]
mod save_tests;
#[cfg(test)]
#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;
#[cfg(test)]
mod usage_tests;
#[cfg(test)]
mod visual_conditional_tests;
#[cfg(test)]
mod visual_fields_tests;
#[cfg(test)]
mod visual_props_tests;
#[cfg(test)]
mod visual_series_tests;
