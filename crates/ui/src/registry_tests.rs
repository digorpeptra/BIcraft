use super::*;

use bicraft_engine::{At as EAt, Kind as EKind, Menu as EMenu, Need as ENeed, Obj as EObj, Tab as ETab};
use serde_json::json;

const TABLE: &str = "crates/ui/src/registry_table.rs";

fn tab(t: ETab) -> &'static str {
    match t {
        ETab::File => "Tab::File",
        ETab::Home => "Tab::Home",
        ETab::Insert => "Tab::Insert",
        ETab::Report => "Tab::Report",
        ETab::Model => "Tab::Model",
        ETab::Data => "Tab::Data",
        ETab::Analyze => "Tab::Analyze",
        ETab::View => "Tab::View",
        ETab::Visual => "Tab::Visual",
        ETab::Table => "Tab::Table",
        ETab::Measure => "Tab::Measure",
    }
}

fn obj(o: EObj) -> String {
    format!("Obj::{o:?}")
}

fn kind(k: EKind) -> String {
    match k {
        EKind::Name(o) => format!("Kind::Name({})", obj(o)),
        EKind::Enum(v) => format!(
            "Kind::Enum(&[{}])",
            v.iter().map(|s| format!("{s:?}")).collect::<Vec<_>>().join(", ")
        ),
        other => format!("Kind::{other:?}"),
    }
}

fn need(n: ENeed) -> String {
    format!("Need::{n:?}")
}

/// The text of `registry_table.rs`, from the engine's compiled registry.
fn generated() -> String {
    let mut s = String::from(
        "//! Generated from `bicraft_engine::COMMANDS` by the test `the_ui_table_is_the_engines_registry`\n\
         //! (`BICRAFT_BLESS=1 cargo test -p bicraft-ui registry` rewrites it). Do not edit by hand.\n\n\
         use super::{At, Cmd, Effect, Kind, Menu, Need, Obj, Param, Tab};\n\n\
         /// Every engine command, in the engine's order.\n\
         pub static COMMANDS: &[Cmd] = &[\n",
    );
    for c in bicraft_engine::COMMANDS {
        let at = match c.place.at {
            EAt::Ribbon { tab: t, group } => format!("At::Ribbon {{ tab: {}, group: {group:?} }}", tab(t)),
            EAt::Menu(m) => format!(
                "At::Menu(Menu::{})",
                match m {
                    EMenu::Page => "Page",
                    EMenu::Visual => "Visual",
                    EMenu::ModelObject => "ModelObject",
                }
            ),
            EAt::Palette => "At::Palette".to_string(),
        };
        let effect = match c.server.effect {
            bicraft_engine::Effect::Mutating => "Effect::Mutating",
            bicraft_engine::Effect::ReadOnly { .. } => "Effect::ReadOnly",
            bicraft_engine::Effect::Opens => "Effect::Opens",
        };
        let needs: Vec<String> = c.enabled.iter().map(|n| need(*n)).collect();
        let form: Vec<String> = c
            .form
            .iter()
            .map(|p| {
                format!(
                    "Param {{ name: {:?}, kind: {}, required: {}, default: {} }}",
                    p.name,
                    kind(p.kind),
                    p.required,
                    match p.default {
                        Some(d) => format!("Some({d:?})"),
                        None => "None".into(),
                    }
                )
            })
            .collect();
        s.push_str(&format!(
            "    Cmd {{\n        id: {:?},\n        label: {:?},\n        params: {:?},\n        at: {at},\n        \
             button: {:?},\n        icon: {:?},\n        shortcut: {},\n        needs: &[{}],\n        \
             form: &[{}],\n        effect: {effect},\n    }},\n",
            c.id,
            c.label,
            c.params,
            c.place.label,
            c.place.icon,
            match c.shortcut {
                Some(k) => format!("Some({k:?})"),
                None => "None".into(),
            },
            needs.join(", "),
            form.join(", "),
        ));
    }
    s.push_str("];\n");
    s
}

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// `registry_table.rs` is the engine's registry: the same ids in the same order, with their
/// labels, places, icons, shortcuts, needs, forms and effects. `BICRAFT_BLESS=1` rewrites it.
#[test]
fn the_ui_table_is_the_engines_registry() {
    let want = generated();
    let path = root().join(TABLE);
    if bicraft_env::var_os("BICRAFT_BLESS").is_some() {
        std::fs::write(&path, &want).unwrap();
    }
    let have = std::fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
    assert!(
        have == want,
        "{TABLE} is stale: run `BICRAFT_BLESS=1 cargo test -p bicraft-ui registry`"
    );
    assert_eq!(COMMANDS.len(), bicraft_engine::COMMANDS.len());
    for (a, b) in COMMANDS.iter().zip(bicraft_engine::COMMANDS) {
        assert_eq!(a.id, b.id);
        assert_eq!(a.button, b.place.label);
    }
}

/// Every button and group has its French text, and every icon a drawing.
#[test]
fn every_command_has_french_text_and_an_icon() {
    let mut missing = Vec::new();
    for c in COMMANDS {
        if fr(c.id).is_none() {
            missing.push(c.id);
        }
        assert!(
            Icon::from_name(c.icon).is_some(),
            "{}: the icon `{}` has no drawing in icons.rs",
            c.id,
            c.icon
        );
        if let At::Ribbon { group, .. } = c.at {
            assert!(group_fr(group).is_some(), "group {group} has no French caption");
        }
    }
    assert!(missing.is_empty(), "no French button text: {missing:?}");
}

/// The enabled state: the snapshot's `commands.disabled` decides, the needs when it is absent,
/// the reader role on top; nothing open disables everything.
#[test]
fn enabled_follows_the_session() {
    let none = Context {
        snap: None,
        open: false,
        reader: false,
    };
    assert!(enabled("file.save", none).is_err());
    assert!(enabled("no.such", none).unwrap_err().contains("unknown"));
    let snap = json!({"source": {"kind": "project"}, "model": {"tables": []}, "report": null,
        "commands": {"disabled": {"edit.undo": "nothing to undo"}}});
    let cx = Context {
        snap: Some(&snap),
        open: true,
        reader: false,
    };
    assert_eq!(enabled("edit.undo", cx).unwrap_err(), "nothing to undo");
    assert!(enabled("page.add", cx).is_ok(), "the session's answer wins");
    let reader = Context { reader: true, ..cx };
    assert!(enabled("measure.add", reader).unwrap_err().contains("read-only"));
    assert!(enabled("model.summary", reader).is_ok());
    // an older snapshot: the needs it shows
    let old = json!({"source": {"kind": "project"}, "model": {"tables": []}, "report": null});
    let cx = Context {
        snap: Some(&old),
        open: true,
        reader: false,
    };
    assert_eq!(enabled("page.add", cx).unwrap_err(), "needs a report");
    assert!(enabled("measure.add", cx).is_ok());
    assert!(enabled("report.new", cx).is_ok());
    // hostile snapshots never panic
    for bad in [
        json!(null),
        json!([1]),
        json!({"commands": {"disabled": 3}}),
        json!({"commands": {"disabled": {"file.save": 7}}}),
    ] {
        let cx = Context {
            snap: Some(&bad),
            open: true,
            reader: false,
        };
        for c in COMMANDS {
            let _ = enabled(c.id, cx);
        }
    }
}

/// The engine's snapshot carries the editor's disabled commands with the reasons
/// `command_enabled` gives.
#[test]
fn the_snapshot_says_which_commands_are_disabled() {
    let mut s = bicraft_engine::Session::new();
    s.execute("model.new", &json!({"name": "M"})).unwrap();
    let snap = s.snapshot();
    let cx = Context {
        snap: Some(&snap),
        open: true,
        reader: false,
    };
    for c in bicraft_engine::COMMANDS {
        let want = s.command_enabled(c.id, bicraft_engine::Caller::Editor);
        assert_eq!(enabled(c.id, cx), want, "{}", c.id);
    }
    assert!(enabled("page.add", cx).is_err());
    assert!(enabled("measure.add", cx).is_ok());
}

#[test]
fn needs_values_reads_the_required_parameters() {
    let c = find("visual.remove").unwrap();
    assert!(c.needs_values(&json!({})));
    assert!(c.needs_values(&json!({"page": "p", "visual": null})));
    assert!(!c.needs_values(&json!({"page": "p", "visual": "v"})));
    assert!(!find("edit.undo").unwrap().needs_values(&json!({})));
    let groups = ribbon_groups(Tab::Home);
    assert!(groups.iter().any(|(g, _)| *g == "Undo"));
    assert_eq!(menu_commands(Menu::Page).len(), 5);
}

/// Every reason the UI can show has its French words; the place paths read as the palette shows
/// them.
#[test]
fn reasons_and_places_in_french() {
    use crate::i18n::Lang;
    for n in [
        Need::File,
        Need::SavedReportFile,
        Need::Project,
        Need::Refreshable,
        Need::Model,
        Need::Report,
        Need::NoReport,
        Need::Data,
        Need::Undo,
        Need::Redo,
    ] {
        let en = reason(n);
        assert_ne!(reason_text(en, Lang::Fr), en, "{n:?}");
        assert_eq!(reason_text(en, Lang::En), en);
    }
    assert_eq!(
        reason_text("not available on a server: it writes files", Lang::Fr),
        "indisponible sur un serveur : it writes files"
    );
    assert_eq!(reason_text("something else", Lang::Fr), "something else");
    let rel = find("relationship.add").unwrap();
    assert_eq!(rel.place_path(Lang::En), "Model › Relationships");
    assert_eq!(rel.place_path(Lang::Fr), "Modèle › Relations");
    assert_eq!(
        find("page.remove").unwrap().place_path(Lang::En),
        "Page tab (menu)"
    );
    assert_eq!(find("model.summary").unwrap().place_path(Lang::En), "Palette");
}
