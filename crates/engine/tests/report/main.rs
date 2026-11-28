//! The engine's report tests that need only its public API: the report commands and checks,
//! repairs, bound values, usage, themes, interactions, filters and the snapshot. An integration
//! test links the library already built instead of compiling it again under `cfg(test)`
//! (docs/ci.md, "Where engine tests go"). Each file keeps the module path it had inside the
//! crate, so the test names do not change.
//! `#[path = "."]` on an inline module keeps its files next to `main.rs` (a path through the
//! module's missing directory, `module/../file.rs`, does not resolve on Linux).

#[path = "../../../../testsupport/tempdir.rs"]
mod tempdir;

mod analyze_point_tests;
mod bound_values_tests;
mod builtin_visuals_tests;
mod bvis_tests;
mod canvas_edit_tests;
mod chart_pack_tests;
mod custom_visual_tests;
mod filter_order_tests;
mod filter_pane_format_tests;
mod format_painter_tests;
mod gantt_tests;
mod look_minimal_tests;
mod matrix_tests;
mod ols_tests;
mod optimize_tests;
mod own_visual_interactions_tests;
mod page_view_tests;
mod report_lint_tests;
mod report_settings_tests;
mod report_tests;
mod shape_map_tests;
mod spec_visual_tests;
mod usage_tests;
mod visual_project_tests;
mod visual_runtime_tests;
mod waterfall_gauge_tests;

#[path = "."]
mod interact {
    #[path = "interact_drill_tests.rs"]
    mod drill_tests;
    #[path = "interact_tests.rs"]
    mod tests;
    #[path = "interact_view_as_tests.rs"]
    mod view_as_tests;
}
#[path = "."]
mod preview {
    #[path = "preview_filters_tests.rs"]
    mod filters_tests;
}
#[path = "."]
mod report_checks {
    #[path = "report_checks_tests.rs"]
    mod tests;
}
#[path = "."]
mod report_repair {
    #[path = "report_repair_tests.rs"]
    mod tests;
}
#[path = "."]
mod snapshot {
    #[path = "snapshot_tests.rs"]
    mod tests;
    #[path = "."]
    mod details {
        #[path = "snapshot_visual_tests.rs"]
        mod tests;
    }
}
#[path = "."]
mod theme_edit {
    #[path = "theme_edit_tests.rs"]
    mod tests;
}

mod buttons_tests;
mod rich_tests;
