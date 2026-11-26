//! The engine's model tests that need only its public API: model edits, history, import, the
//! model readers and properties, partitions, perspectives, translations, Power Query, data
//! sources, storage statistics and DAX queries. An integration test links the library already
//! built instead of compiling it again under `cfg(test)` (docs/ci.md, "Where engine tests go").
//! Each file keeps the module path it had inside the crate, so the test names do not change.
//! `#[path = "."]` on an inline module keeps its files next to `main.rs` (a path through the
//! module's missing directory, `module/../file.rs`, does not resolve on Linux).

#[path = "../../../../testsupport/tempdir.rs"]
mod tempdir;

mod analyze_tests;
mod apply_define_tests;
mod autodetect_tests;
mod data_view_tests;
mod dax_queries_tests;
mod edit_tests;
mod get_data_tests;
mod groups_tests;
mod history_tests;
mod import_tests;
mod m92_tests;
mod manage_relationships_tests;
mod manage_roles_tests;
mod parameters_tests;
mod perspective_edit_tests;
mod query_folding_tests;
mod quick_tests;
mod refresh_policy_tests;
mod set_properties_tests;
mod sharepoint_tests;
mod sqlserver_tests;
mod storage_mode_tests;
mod tmdl_view_tests;
mod udf_tests;
mod webdata_tests;

#[path = "."]
mod commands {
    #[path = "validate_tests.rs"]
    mod validate_tests;
}
#[path = "."]
mod datasrc {
    #[path = "datasrc_tests.rs"]
    mod session_tests;
}
#[path = "."]
mod model_get {
    #[path = "model_get_tests.rs"]
    mod model_get_tests;
}
#[path = "."]
mod model_props {
    #[path = "model_props_tests.rs"]
    mod tests;
}
#[path = "."]
mod mquery {
    #[path = "mquery_tests.rs"]
    mod mquery_tests;
}
#[path = "."]
mod partition {
    #[path = "partition_tests.rs"]
    mod partition_tests;
}
#[path = "."]
mod perspectives {
    #[path = "perspectives_tests.rs"]
    mod tests;
}
#[path = "."]
mod storage_stats {
    #[path = "storage_stats_tests.rs"]
    mod storage_stats_tests;
}
#[path = "."]
mod translations {
    #[path = "translations_tests.rs"]
    mod tests;
}
#[path = "."]
mod preview {
    #[path = "."]
    mod general {
        #[path = "."]
        mod query {
            #[path = "dax_calendar_tests.rs"]
            mod calendar_tests;
            #[path = "dax_leftovers_tests.rs"]
            mod leftovers_tests;
            #[path = "dax_pick_tests.rs"]
            mod pick_tests;
            #[path = "dax_query_tests.rs"]
            mod tests;
            #[path = "dax_window_tests.rs"]
            mod window_tests;
        }
    }
}
