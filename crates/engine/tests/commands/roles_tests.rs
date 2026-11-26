//! Reader access (`docs/auth.md`, "Roles"): every command's [`Effect`] is pinned here, so a new
//! command (mutating by default) has to be classified on purpose, and the reader checks of
//! [`Session::reader_check`] are exercised on real sessions.

use serde_json::json;

use crate::tempdir::TempDir;
use bicraft_engine::{Access, Effect};
use bicraft_engine::{COMMANDS, READ_ONLY_COMMANDS, ServerConfig, Session, find_command};

/// Commands a reader may run whatever the parameters.
const READ: &[&str] = &[
    "query.list",
    "query.steps",
    "query.apply_step",
    "query.edit_step",
    "query.remove_step",
    "query.new",
    "query.rename",
    "query.dependencies",
    "session.revision",
    "table.rows",
    "table.entered_data",
    "visual.records",
    "recovery.list",
    "file.inspect",
    "file.diff",
    "file.textconv",
    "model.summary",
    "model.validate",
    "model.analyze",
    "model.usage",
    "model.unused",
    "model.get",
    "table.get",
    "column.get",
    "column.values",
    "measure.get",
    "relationship.list",
    "translation.list",
    "model.storage_stats",
    "dax.validate",
    "dax.validate_query",
    "role.check_filter",
    "dax.functions",
    "m.validate",
    "m.complete",
    "m.signature",
    "m.hover",
    "dax.evaluate",
    "dax.format",
    "dax.cancel",
    "dax.queries",
    "tmdl.script",
    "report.summary",
    "report.validate",
    "report.check",
    "report.extract",
    "visual.props",
    "visual.options",
    "visual.copy",
    "theme.get",
    "edit.history",
    "view.interactions",
    "report_measure.list",
    "report.custom_visuals",
    "visual.spec_template",
    "bookmark.list",
    "slicer.sync_list",
    "project.test",
    "about.licenses",
    "sources.list",
    "source.settings",
    "env.list",
    "env.check",
];

/// Commands a reader may run without the listed parameters.
const READ_UNLESS: &[(&str, &[&str])] = &[
    ("report.render", &["out"]),
    ("query.preview", &["allow_connectors"]),
    ("query.folding", &["allow_connectors"]),
    ("query.native", &["allow_connectors"]),
    ("query.diagnose", &["allow_connectors"]),
    ("query.profile", &["allow_connectors"]),
    ("source.navigate", &["server", "url"]),
    ("dax.memory", &["budget", "query_budget"]),
    ("dax.aggregations", &["enabled", "build"]),
    ("m.format", &["apply"]),
];

/// Commands a reader may run while the session holds no unsaved changes.
const OPENS: &[&str] = &["file.open"];

/// Every other command: refused to readers. Listed one by one so that adding a command fails
/// this test until it is classified here (and, if readable, in its row of `commands.rs`).
#[rustfmt::skip]
const MUTATING: &[&str] = &[
    "file.set_cache_abf", "file.save", "file.save_as", "file.convert", "file.redact", "file.open_in_desktop", "objects.import",
    // the Performance analyzer: the shared session's recording, editors only
    "perf.results", "perf.start", "perf.refresh", "perf.stop", "perf.clear", "perf.render", "perf.export",
    "project.new", "model.new", "model.set", "model.set_annotation", "table.set_annotation",
    "column.set_annotation", "measure.set_annotation", "column.remove_variation", "model.remove_unused",
    "model.export_vpax", "table.add", "table.import", "table.add_refresh_info", "parameter.what_if", "parameter.fields", "table.set_data", "table.remove",
    "table.rename", "table.set", "table.set_storage_mode", "table.set_refresh_policy", "table.remove_refresh_policy", "partition.set", "table.set_partition", "partition.add",
    "partition.remove", "sources.rebase", "env.apply", "expression.add", "expression.set", "expression.rename",
    "expression.remove", "function.add", "function.set", "function.rename", "function.remove", "tmdl.apply",
    "column.add", "column.group", "column.set", "column.rename", "column.remove",
    "measure.add", "measure.quick", "dax.apply_define", "measure.set", "measure.rename", "measure.remove", "measure.move",
    "hierarchy.add", "hierarchy.remove", "hierarchy.set", "calculation_group.add",
    "calculation_group.set", "calculation_group.remove", "calculation_item.add",
    "calculation_item.set", "calculation_item.remove", "relationship.add",
    "relationship.remove", "relationship.set", "relationship.autodetect", "diagram.move", "diagram.add", "diagram.rename", "diagram.remove", "diagram.remove_tables", "role.add", "role.set_filter", "role.remove", "role.rename", "role.set", "role.set_permission", "perspective.add", "perspective.set", "perspective.remove", "model.set_properties",
    "report.new", "view.set_language", "view.set_culture", "view.set_as_of", "view.interact",
    "culture.add", "culture.remove", "translation.set", "report.repair", "report_measure.add",
    "report_measure.set", "report_measure.remove", "report.apply", "page.add", "page.remove",
    "page.rename", "page.move", "page.set_active", "page.set_display_option", "visual.add", "visual.set_position",
    "visual.place", "page.arrange", "visual.bind", "visual.unbind", "visual.move_field",
    "visual.set_aggregation", "visual.set_type", "visual.remove",
    "visual.set_title", "visual.set_sort", "visual.filter", "visual.clear_filters",
    "page.filter", "page.clear_filters", "report.filter", "report.clear_filters", "filter.order",
    "visual.rename_field", "visual.show_value_as", "visual.show_items_no_data", "visual.calculation.add", "visual.calculation.set",
    "visual.calculation.remove", "visual.bind_hierarchy",
    "textbox.add", "textbox.set", "image.add", "image.set", "shape_map.set_map", "visual.import_custom", "visual.remove_custom", "visual.scaffold", "visual.build", "visual.pack", "visual.export_bicraft_visual", "visual.set_spec", "visual.set_bicraft_visual", "visual.set_data_format", "visual.add_sparkline", "visual.edit_sparkline", "button.add", "shape.add", "visual.set_action", "page.format", "page.format_filter_pane",
    "visual.format", "visual.axis", "visual.analytics", "visual.small_multiples",
    "visual.tooltip_page", "page.set_type", "page.interactions", "report.settings", "report.pause_visuals", "report.refresh_visuals", "report.query_reduction", "visual.order", "visual.drill_filter", "view.keep_drill", "visual.expand", "visual.toggle_node",
    "visual.cut", "visual.paste", "visual.copy_format", "visual.duplicate", "visual.move", "visual.align", "visual.group", "visual.ungroup",
    "visual.mobile", "visual.mobile_format", "visual.series", "visual.conditional", "theme.set", "theme.import",
    "theme.export", "theme.remove", "theme.set_colors", "theme.set_text_classes", "theme.customize",
    "report.export_pdf", "report.print", "visual.export_data", "query.export",
    "theme.set_visual_style", "dax.clear_cache", "model.refresh", "edit.undo", "edit.redo",
    "recovery.apply", "recovery.discard",
    "visual.set_hidden", "visual.rename_group", "visual.tab_order",
    "bookmark.capture", "bookmark.update", "bookmark.set", "bookmark.remove",
    "bookmark.group", "bookmark.ungroup", "bookmark.rename_group", "bookmark.apply",
    "slicer.set_sync",
    "report.rebind", "report.remove_reference", "visual.fix_query_refs", "parameter.fix_field",
    "source.load", "source.credentials", "source.privacy", "table.enter_data", "query.apply",
    "dax.set_queries",
];

fn expected(id: &str) -> Option<Effect> {
    if READ.contains(&id) {
        return Some(Effect::ReadOnly { unless: &[] });
    }
    if let Some((_, unless)) = READ_UNLESS.iter().find(|(c, _)| *c == id) {
        return Some(Effect::ReadOnly { unless });
    }
    if OPENS.contains(&id) {
        return Some(Effect::Opens);
    }
    MUTATING.contains(&id).then_some(Effect::Mutating)
}

#[test]
fn every_command_has_a_deliberate_classification() {
    let listed = READ.len() + READ_UNLESS.len() + OPENS.len() + MUTATING.len();
    assert_eq!(
        listed,
        COMMANDS.len(),
        "a command is listed twice or missing here"
    );
    for c in COMMANDS {
        let Some(want) = expected(c.id) else {
            panic!(
                "`{}` is not classified for readers: add it to roles_tests.rs (MUTATING unless it \
                 only reads; then mark its row in commands.rs)",
                c.id
            );
        };
        assert_eq!(c.server.effect, want, "{}", c.id);
    }
}

/// `path` parameters a reader may pass to a read-only command because the command only reads
/// them. Deliberate entries only: `project.test {path}` opens the project in a private session
/// (`citest.rs`) that is never saved and is dropped when the command returns, so nothing is written
/// and the caller's session is untouched (`project_test_path_is_only_read` checks both);
/// `source.navigate {path}` lists a file or folder under the workspace and evaluates M over it
/// without touching the session, as `query.preview {data_root}` already lets a reader do
/// (`source_navigate_path_is_only_read`).
const UNGATED_READ_PATHS: &[(&str, &str)] = &[("project.test", "path"), ("source.navigate", "path")];

#[test]
fn source_navigate_path_is_only_read() {
    let c = find_command("source.navigate").unwrap();
    assert!(matches!(
        c.server.effect,
        Effect::ReadOnly {
            unless: ["server", "url"]
        }
    ));
    let dir = TempDir::new("engine-roles-source-navigate");
    std::fs::write(dir.path().join("sales.csv"), "Region,Units\nNorth,3\n").unwrap();
    let before = std::fs::read_dir(dir.path()).unwrap().count();
    let mut s = Session::server(ServerConfig::new(dir.path()).unwrap());
    s.execute("model.new", &json!({"name": "M"})).unwrap();
    let params = json!({"path": "sales.csv"});
    s.reader_check("source.navigate", &params).unwrap();
    let (revision, dirty) = (s.revision, s.dirty);
    let v = s.execute("source.navigate", &params).unwrap();
    assert_eq!(v["items"][0]["name"], "sales");
    assert_eq!((s.revision, s.dirty), (revision, dirty));
    assert!(s.data_root.is_none(), "the session's data root is unchanged");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), before);
}

#[test]
fn project_test_path_is_only_read() {
    let c = find_command("project.test").unwrap();
    let p = c.server.paths.iter().find(|p| p.name == "path").unwrap();
    // resolved by the private session's own file.open, never written
    assert_eq!(p.kind, bicraft_engine::PathKind::Delegated);
    assert!(matches!(c.server.effect, Effect::ReadOnly { unless: [] }));
    // a reader runs it on a server session: the project's files and the session stay as they were
    let dir = TempDir::new("engine-roles-project-test");
    std::fs::write(dir.path().join("model.bim"), r#"{"name": "M", "compatibilityLevel": 1567, "model": {"tables": [{"name": "T", "measures": [{"name": "One", "expression": "1"}]}]}}"#).unwrap();
    std::fs::write(
        dir.path().join("bicraft.tests.json"),
        r#"{"version": 1, "model": {"validate": true}}"#,
    )
    .unwrap();
    let snapshot = |d: &std::path::Path| {
        let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                (
                    e.file_name().to_string_lossy().into_owned(),
                    std::fs::read(e.path()).unwrap(),
                )
            })
            .collect();
        files.sort();
        files
    };
    let before = snapshot(dir.path());
    let mut s = Session::server(ServerConfig::new(dir.path()).unwrap());
    let params = json!({"path": "model.bim"});
    s.reader_check("project.test", &params).unwrap();
    let revision = s.revision;
    let v = s.execute("project.test", &params).unwrap();
    assert_eq!(v["summary"]["tests"], 3);
    assert_eq!(s.revision, revision);
    assert!(s.source.is_none() && s.model.is_none());
    assert_eq!(snapshot(dir.path()), before);
    // listed deliberately
    assert!(UNGATED_READ_PATHS.contains(&("project.test", "path")));
}

#[test]
fn readable_commands_are_safe() {
    for c in COMMANDS.iter().filter(|c| c.server.effect.readable()) {
        // never the network, never a command the server refuses
        assert_eq!(c.server.access, Access::Allowed, "{}", c.id);
        match c.server.effect {
            Effect::ReadOnly { unless } => {
                // writing parameters exist in the params doc
                for p in unless {
                    assert!(c.params.contains(p), "{}: `{p}`", c.id);
                }
                // a path parameter a reader may give is read, not written: only `out` writes,
                // and it is gated
                for p in c.server.paths {
                    if (p.name == "out" || p.name == "path") && !UNGATED_READ_PATHS.contains(&(c.id, p.name))
                    {
                        assert!(unless.contains(&p.name), "{}: `{}` must be gated", c.id, p.name);
                    }
                }
                // reads unconditionally: the session's revision does not move
                if unless.is_empty() {
                    assert!(READ_ONLY_COMMANDS.contains(&c.id), "{}", c.id);
                }
            }
            Effect::Opens | Effect::Mutating => {}
        }
    }
}

fn server_session(dir: &TempDir) -> Session {
    Session::server(ServerConfig::new(dir.path()).unwrap())
}

#[test]
fn reader_checks() {
    let dir = TempDir::new("roles-checks");
    let mut s = server_session(&dir);
    // read-only commands pass, writing parameters do not
    assert!(s.reader_check("model.summary", &json!({})).is_ok());
    assert!(s.reader_check("report.render", &json!({"page": 0})).is_ok());
    let e = s
        .reader_check("report.render", &json!({"out": "x.png"}))
        .unwrap_err();
    assert!(e.contains("`out`"), "{e}");
    // null counts as given (dax.memory {budget: null} resets the budget)
    assert!(s.reader_check("dax.memory", &json!({"budget": null})).is_err());
    assert!(s.reader_check("dax.memory", &json!({})).is_ok());
    assert!(
        s.reader_check("dax.aggregations", &json!({"build": true}))
            .is_err()
    );
    // mutating commands are refused
    for id in [
        "file.save",
        "measure.add",
        "model.refresh",
        "theme.export",
        "view.interact",
    ] {
        let e = s.reader_check(id, &json!({})).unwrap_err();
        assert!(e.contains("not available to a reader"), "{id}: {e}");
    }
    // unknown ids pass: running them reports them
    assert!(s.reader_check("no.such", &json!({})).is_ok());
    // file.open: allowed on a clean session, refused once there are unsaved changes
    assert!(s.reader_check("file.open", &json!({"path": "a.bim"})).is_ok());
    s.execute("model.new", &json!({"name": "M"})).unwrap();
    s.execute("table.add", &json!({"name": "T"})).unwrap();
    assert!(s.dirty);
    let e = s
        .reader_check("file.open", &json!({"path": "a.bim"}))
        .unwrap_err();
    assert!(e.contains("unsaved"), "{e}");
    s.execute("file.save_as", &json!({"path": "a.bim"})).unwrap();
    assert!(!s.dirty);
    assert!(s.reader_check("file.open", &json!({"path": "a.bim"})).is_ok());
}

#[test]
fn reader_listing() {
    let dir = TempDir::new("roles-list");
    let s = server_session(&dir);
    let ids: Vec<&str> = s.reader_commands(None).iter().map(|c| c.id).collect();
    assert!(ids.contains(&"model.summary"));
    assert!(ids.contains(&"report.render"));
    assert!(ids.contains(&"file.open"));
    assert!(!ids.contains(&"file.save"));
    assert!(!ids.contains(&"measure.add"));
    assert!(
        ids.iter()
            .all(|id| find_command(id).is_some_and(|c| c.server.effect.readable()))
    );
    let filtered: Vec<&str> = s.reader_commands(Some("measure")).iter().map(|c| c.id).collect();
    assert!(filtered.contains(&"measure.get"), "{filtered:?}");
    assert!(!filtered.contains(&"measure.add"), "{filtered:?}");
}
