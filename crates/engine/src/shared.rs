//! Shared runs: read-only commands on a shared `&Session` (`docs/web-deployment.md`,
//! "Concurrency"). `bicraft-cli serve` runs them under a read lock of the workspace's session, so
//! several run at once while a command that may change the session waits for them.
//!
//! [`SHARED`] gives each read-only command (by its server policy, [`Effect::ReadOnly`]) a
//! function taking `&Session`: the compiler, not a list, proves it cannot change the session.
//! What such a run still updates lives behind its own synchronization (the preview caches,
//! `preview.rs` "forks"; the refresh-state cache; [`Session::last_command_now`]).
//!
//! A run is shared ([`Session::runs_shared`]) when its command is read-only, names none of the
//! parameters that make it write (`unless`, e.g. `report.render {out}`), and names none of the
//! parameters that make it swap session fields for its duration (`cache_abf`: such a run goes
//! through [`Session::execute`], exclusively).

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use serde_json::Value;

use crate::sandbox::Effect;
use crate::{LastCommand, Session, find_command};

/// A shared run of a command.
pub type SharedFn = fn(&Session, &Value) -> Result<Value, String>;

/// (command id, shared run, parameters that make a run exclusive).
type Entry = (&'static str, SharedFn, &'static [&'static str]);

/// Parameters with which a read-only command runs exclusively (`cache_abf.rs`: the session's
/// explicit cache file is swapped for the run).
const SWAPS: &[&str] = &["cache_abf"];

/// Read-only commands that always run exclusively, with why.
const EXCLUSIVE_READS: &[(&str, &str)] = &[(
    "project.test",
    "runs its gates as commands of its own (`Session::execute`: DAX assertions, validations), on \n     the open session or on a private one",
)];

/// Every read-only command's shared run (a test checks it matches [`Effect::ReadOnly`]).
static SHARED: &[Entry] = &[
    ("session.revision", crate::commands::session_revision, &[]),
    ("about.licenses", crate::about::about_licenses, &[]),
    ("recovery.list", crate::recovery::recovery_list, &[]),
    ("file.inspect", crate::commands::file_inspect, &[]),
    ("file.diff", crate::diff::file_diff, &[]),
    ("file.textconv", crate::diff::file_textconv, &[]),
    ("model.summary", crate::commands::model_summary, &[]),
    ("model.get", crate::model_get::model_get, &[]),
    ("tmdl.script", crate::tmdl_script::tmdl_script, &[]),
    ("table.get", crate::model_get::table_get, &[]),
    ("column.get", crate::model_get::column_get, &[]),
    ("column.values", crate::filter_edit::column_values, &[]),
    ("table.rows", crate::preview::rows::table_rows, &[]),
    ("measure.get", crate::model_get::measure_get, &[]),
    ("relationship.list", crate::model_get::relationship_list, &[]),
    (
        "model.storage_stats",
        crate::cache_abf::storage_stats_shared,
        SWAPS,
    ),
    ("dax.validate", crate::commands::dax_validate, &[]),
    ("m.validate", crate::mquery::m_validate, &[]),
    ("m.format", crate::mquery::m_format_shared, &[]),
    ("m.complete", crate::m_assist::m_complete, &[]),
    ("m.signature", crate::m_assist::m_signature, &[]),
    ("m.hover", crate::m_assist::m_hover, &[]),
    ("query.preview", crate::query_preview::query_preview, &[]),
    ("query.profile", crate::query_profile::query_profile, &[]),
    ("query.folding", crate::query_folding::query_folding, &[]),
    ("query.native", crate::query_folding::query_native, &[]),
    ("query.diagnose", crate::query_diagnose::query_diagnose, &[]),
    ("query.dependencies", crate::query_deps::query_dependencies, &[]),
    ("query.list", crate::query_steps::query_list, &[]),
    ("query.steps", crate::query_steps::query_steps, &[]),
    ("query.apply_step", crate::query_steps::query_apply_step, &[]),
    ("query.edit_step", crate::query_steps::query_edit_step, &[]),
    ("query.remove_step", crate::query_steps::query_remove_step, &[]),
    ("query.new", crate::query_steps::query_new, &[]),
    ("query.rename", crate::query_steps::query_rename, &[]),
    ("source.navigate", crate::get_data::source_navigate, &[]),
    ("source.settings", crate::source_settings::source_settings, &[]),
    ("table.entered_data", crate::get_data::table_entered_data, &[]),
    ("sources.list", crate::sources::sources_list, &[]),
    ("env.list", crate::envprofile::env_list, &[]),
    ("env.check", crate::envprofile::env_check, &[]),
    ("model.validate", crate::commands::model_validate, &[]),
    ("model.analyze", crate::analyze::model_analyze, &[]),
    ("model.usage", crate::usage::model_usage, &[]),
    ("model.unused", crate::usage::model_unused, &[]),
    ("report.summary", crate::report::report_summary, &[]),
    ("view.interactions", crate::interact::view_interactions, &[]),
    ("translation.list", crate::translations::translation_list, &[]),
    (
        "report.custom_visuals",
        crate::custom_visuals::report_custom_visuals,
        &[],
    ),
    ("report.validate", crate::report::report_validate, &[]),
    (
        "report_measure.list",
        crate::report_measures::report_measure_list,
        &[],
    ),
    ("report.check", crate::report_apply::report_check, &[]),
    ("report.extract", crate::report_apply::report_extract, &[]),
    ("bookmark.list", crate::bookmarks::bookmark_list_shared, &[]),
    (
        "slicer.sync_list",
        crate::slicer_sync::slicer_sync_list_shared,
        &[],
    ),
    ("report.render", crate::cache_abf::report_render_shared, SWAPS),
    ("visual.props", crate::visual_props::visual_props, &[]),
    ("visual.records", crate::see_records::visual_records, &[]),
    ("visual.analyze", crate::visual_analyze::visual_analyze, &[]),
    (
        "visual.explain_anomaly",
        crate::visual_analyze::visual_explain_anomaly,
        &[],
    ),
    ("visual.options", crate::visual_catalog::visual_options, &[]),
    (
        "visual.spec_template",
        crate::spec_visual::visual_spec_template,
        &[],
    ),
    ("visual.copy", crate::canvas_edit::visual_copy, &[]),
    ("theme.get", crate::theme_edit::theme_get, &[]),
    ("dax.validate_query", crate::commands::dax_validate_query, &[]),
    ("dax.queries", crate::dax_queries::dax_queries, &[]),
    ("dax.format", crate::dax_queries::dax_format, &[]),
    ("dax.define_query", crate::dax_define::dax_define_query, &[]),
    ("dax.cancel", crate::dax_queries::dax_cancel, &[]),
    ("role.check_filter", crate::role_manage::role_check_filter, &[]),
    ("dax.functions", crate::commands::dax_functions, &[]),
    ("dax.evaluate", crate::cache_abf::dax_evaluate_shared, SWAPS),
    ("dax.aggregations", crate::preview::dax_aggregations, &[]),
    ("dax.memory", crate::preview::dax_memory, &[]),
    ("edit.history", crate::history::edit_history, &[]),
];

/// The shared run of command `id`, if it has one.
pub fn shared_fn(id: &str) -> Option<SharedFn> {
    SHARED.iter().find(|(i, _, _)| *i == id).map(|(_, f, _)| *f)
}

fn names_any(params: &Value, keys: &[&str]) -> bool {
    keys.iter()
        .any(|k| params.as_object().is_some_and(|o| o.contains_key(*k)))
}

impl Session {
    /// Whether command `id` with `params` may run on a shared session
    /// ([`Session::execute_shared`]); otherwise it needs [`Session::execute`] (exclusive).
    pub fn runs_shared(&self, id: &str, params: &Value) -> bool {
        runs_shared(id, params)
    }
}

/// [`Session::runs_shared`] without a session (the answer depends on the command and its
/// parameters only).
pub fn runs_shared(id: &str, params: &Value) -> bool {
    if EXCLUSIVE_READS.iter().any(|(i, _)| *i == id) {
        return false;
    }
    let (Some(cmd), Some((_, _, swaps))) = (find_command(id), SHARED.iter().find(|(i, _, _)| *i == id))
    else {
        return false;
    };
    match cmd.server.effect {
        Effect::ReadOnly { unless } => !names_any(params, unless) && !names_any(params, swaps),
        Effect::Mutating | Effect::Opens => false,
    }
}

impl Session {
    /// Runs a read-only command on a shared session: what [`Session::execute`] does for it (the
    /// server policy's checks, a panic caught as an error, the outcome recorded), without
    /// `&mut`. `cancel` is this run's cancel flag (several runs at once each have their own). A
    /// run [`Session::runs_shared`] refuses is an error, and nothing runs.
    pub fn execute_shared(
        &self,
        id: &str,
        params: &Value,
        cancel: Option<Arc<AtomicBool>>,
    ) -> Result<Value, String> {
        if !self.runs_shared(id, params) {
            return Err(format!(
                "`{id}` with these parameters cannot run on a shared session"
            ));
        }
        let (Some(cmd), Some(run)) = (find_command(id), shared_fn(id)) else {
            return Err(format!("unknown command `{id}` (see `list_commands`)"));
        };
        let _cancel = crate::call_scope::cancel(cancel);
        let result = match crate::sandbox::prepare(self.server.as_ref(), cmd, params) {
            Err(e) => Err(e),
            Ok(params) => {
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(self, &params))) {
                    Ok(r) => r,
                    Err(_) => Err(format!("internal error in `{id}`")),
                }
            }
        };
        self.record_shared(LastCommand {
            id: id.to_string(),
            ok: result.is_ok(),
            error: result.as_ref().err().cloned(),
            at_ms: crate::now_ms(),
        });
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every read-only command has a shared run, and only read-only commands have one.
    #[test]
    fn shared_runs_match_the_read_only_policy() {
        for c in crate::COMMANDS {
            let read_only = matches!(c.server.effect, Effect::ReadOnly { .. });
            let exclusive = EXCLUSIVE_READS.iter().any(|(id, _)| *id == c.id);
            assert_eq!(read_only, shared_fn(c.id).is_some() || exclusive, "{}", c.id);
            assert!(!(exclusive && shared_fn(c.id).is_some()), "{}", c.id);
        }
        for (id, _, _) in SHARED {
            assert!(find_command(id).is_some(), "{id}");
        }
    }

    #[test]
    fn runs_shared_follows_unless_and_swaps() {
        let s = Session::new();
        assert!(s.runs_shared("dax.evaluate", &serde_json::json!({"query": "EVALUATE {1}"})));
        assert!(!s.runs_shared("dax.evaluate", &serde_json::json!({"cache_abf": null})));
        assert!(s.runs_shared("report.render", &serde_json::json!({})));
        assert!(!s.runs_shared("report.render", &serde_json::json!({"out": "a.png"})));
        assert!(!s.runs_shared("dax.memory", &serde_json::json!({"budget": "1G"})));
        assert!(s.runs_shared("dax.memory", &serde_json::json!({})));
        assert!(!s.runs_shared("measure.add", &serde_json::json!({})));
        assert!(!s.runs_shared("file.open", &serde_json::json!({})));
        assert!(!s.runs_shared("no.such", &serde_json::json!({})));
    }
}
