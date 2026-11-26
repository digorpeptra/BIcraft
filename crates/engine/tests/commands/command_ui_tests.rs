//! Placement, shortcuts, `enabled` checks and forms of every command (ROADMAP M11.1).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::PathKind;
use bicraft_engine::{At, Caller, Kind, Menu, Need, Tab, UI_SHORTCUTS};
use bicraft_engine::{COMMANDS, Command, ServerConfig, Session, list_commands};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// A model with a table, a measure and a report with one chart.
fn sample() -> Session {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "Sales", "csv": "Product,Amount\nA,1\nB,2\n"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
    );
    run(&mut s, "report.new", json!({}));
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "clusteredColumnChart", "x": 0, "y": 0, "width": 300,
               "height": 200, "fields": [{"role": "Category", "field": "Sales[Product]"},
                                         {"role": "Y", "field": "[Total]"}]}),
    );
    s
}

fn is_kebab(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && !s.ends_with('-')
        && !s.contains("--")
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[test]
fn every_command_has_a_place_with_a_label_and_an_icon() {
    let mut labels: HashMap<String, &str> = HashMap::new();
    for c in COMMANDS {
        let p = c.place;
        assert!(
            !p.label.trim().is_empty() && p.label.len() <= 40,
            "{}: the UI label must be short and not empty: {:?}",
            c.id,
            p.label
        );
        assert!(
            p.label.chars().next().is_some_and(char::is_uppercase),
            "{}: labels start with a capital: {:?}",
            c.id,
            p.label
        );
        assert!(
            is_kebab(p.icon),
            "{}: icon names are kebab-case: {:?}",
            c.id,
            p.icon
        );
        if let At::Ribbon { group, .. } = p.at {
            assert!(!group.trim().is_empty(), "{}: ribbon group", c.id);
        }
        // two buttons with the same text in one place would be ambiguous
        let key = match p.at {
            At::Ribbon { tab, .. } => tab.name(),
            At::Menu(m) => m.name(),
            At::Palette => "palette",
        };
        if let Some(other) = labels.insert(format!("{key}/{}", p.label), c.id) {
            panic!(
                "`{}` and `{other}` have the same label {:?} in {key}",
                c.id, p.label
            );
        }
    }
    // every fixed tab has buttons; the palette stays for reads and automation
    for t in Tab::ALL {
        assert!(
            COMMANDS
                .iter()
                .any(|c| matches!(c.place.at, At::Ribbon { tab, .. } if tab == t)),
            "tab {t:?} has no button"
        );
    }
    for m in Menu::ALL {
        assert!(
            COMMANDS.iter().any(|c| c.place.at == At::Menu(m)),
            "menu {m:?} is empty"
        );
    }
    let palette = COMMANDS.iter().filter(|c| c.place.at == At::Palette).count();
    assert!(palette * 4 < COMMANDS.len(), "{palette} palette-only commands");
}

#[test]
fn shortcuts_are_unique_and_well_formed() {
    let mut seen: HashMap<String, &str> = HashMap::new();
    for (k, what) in UI_SHORTCUTS {
        seen.insert(k.to_ascii_lowercase(), what);
    }
    for c in COMMANDS {
        let Some(k) = c.shortcut else { continue };
        let parts: Vec<&str> = k.split('+').collect();
        let (key, mods) = parts.split_last().unwrap();
        assert!(
            mods.iter().all(|m| ["Ctrl", "Shift", "Alt"].contains(m)),
            "{}: modifiers are Ctrl, Shift, Alt in that order: {k}",
            c.id
        );
        let order: Vec<usize> = mods
            .iter()
            .map(|m| ["Ctrl", "Shift", "Alt"].iter().position(|x| x == m).unwrap())
            .collect();
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{}: {k}", c.id);
        let named = ["Delete", "Escape", "Enter", "PageUp", "PageDown", "Home", "End"];
        let ok = (key.len() == 1 && key.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()))
            || named.contains(key)
            || (key.starts_with('F') && key[1..].parse::<u8>().is_ok_and(|n| (1..=12).contains(&n)));
        assert!(ok, "{}: unknown key in {k}", c.id);
        if let Some(other) = seen.insert(k.to_ascii_lowercase(), c.id) {
            panic!("shortcut {k} of `{}` is already taken by {other}", c.id);
        }
    }
    let ids = |k: &str| COMMANDS.iter().find(|c| c.shortcut == Some(k)).map(|c| c.id);
    assert_eq!(ids("Ctrl+S"), Some("file.save"));
    assert_eq!(ids("Ctrl+Z"), Some("edit.undo"));
    assert_eq!(ids("F5"), Some("model.refresh"));
}

/// The names in `unknown parameter `x` (expected: a, b)` of a run with an unknown key.
fn accepted(s: &mut Session, id: &str) -> Option<BTreeSet<String>> {
    let e = s.execute(id, &json!({"__form_probe__": 1})).err()?;
    let rest = e.split_once("(expected: ")?.1;
    let list = rest.rsplit_once(')')?.0;
    Some(match list {
        "none" => BTreeSet::new(),
        l => l.split(", ").map(str::to_string).collect(),
    })
}

#[test]
fn forms_name_the_parameters_each_command_accepts() {
    let mut empty = Session::new();
    let mut full = sample();
    let mut unprobed = Vec::new();
    for c in COMMANDS {
        let names: BTreeSet<&str> = c.form.iter().map(|p| p.name).collect();
        assert_eq!(names.len(), c.form.len(), "{}: a parameter is listed twice", c.id);
        assert!(
            c.form
                .iter()
                .all(|p| !p.name.is_empty()
                    && p.name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_')),
            "{}",
            c.id
        );
        // every name the command's parameter check lists is in the form
        let listed = accepted(&mut empty, c.id).or_else(|| accepted(&mut full, c.id));
        match listed {
            Some(l) => {
                let missing: Vec<&String> = l.iter().filter(|n| !names.contains(n.as_str())).collect();
                assert!(missing.is_empty(), "{}: the form lacks {missing:?}", c.id);
            }
            None => unprobed.push(c.id),
        }
        // and every form parameter is accepted (not reported unknown)
        for p in c.form {
            for s in [&mut empty, &mut full] {
                if let Err(e) = s.execute(c.id, &json!({p.name: {"__wrong__": []}})) {
                    assert!(
                        !e.contains(&format!("unknown parameter `{}`", p.name)),
                        "{}: form parameter `{}` is refused: {e}",
                        c.id,
                        p.name
                    );
                }
            }
        }
    }
    // commands that accept any key (formatting patches, reader actions): checked by hand
    assert_eq!(
        unprobed,
        ["view.interact", "visual.format", "visual.mobile_format"],
        "commands without a parameter list"
    );
}

#[test]
fn path_parameters_are_the_server_policy_paths() {
    for c in COMMANDS {
        for p in c.form {
            let policy = c.server.paths.iter().find(|x| x.name == p.name);
            let want = policy.map(|x| match x.kind {
                PathKind::Many => Kind::Paths,
                PathKind::PathOrText => Kind::PathOrText,
                PathKind::One | PathKind::Clearable | PathKind::Delegated => Kind::Path,
            });
            let is_path = matches!(p.kind, Kind::Path | Kind::Paths | Kind::PathOrText);
            match want {
                Some(k) => assert_eq!(p.kind, k, "{}.{}", c.id, p.name),
                None => assert!(
                    !is_path,
                    "{}.{}: a path the server policy does not resolve",
                    c.id, p.name
                ),
            }
        }
        for x in c.server.paths {
            assert!(
                c.form.iter().any(|p| p.name == x.name),
                "{}: server path `{}` is not in the form",
                c.id,
                x.name
            );
        }
    }
}

#[test]
fn enabled_follows_the_session() {
    let mut s = Session::new();
    let why = |s: &Session, id: &str| s.command_enabled(id, Caller::Editor).err();
    assert_eq!(why(&s, "file.open"), None);
    assert_eq!(why(&s, "model.new"), None);
    assert_eq!(why(&s, "file.save").as_deref(), Some(Need::File.reason()));
    assert_eq!(why(&s, "measure.add").as_deref(), Some(Need::Model.reason()));
    assert_eq!(why(&s, "page.add").as_deref(), Some(Need::Report.reason()));
    assert_eq!(why(&s, "edit.undo").as_deref(), Some(Need::Undo.reason()));
    assert_eq!(
        why(&s, "model.refresh").as_deref(),
        Some(Need::Refreshable.reason())
    );
    assert!(why(&s, "no.such").is_some());
    run(&mut s, "model.new", json!({"name": "M"}));
    assert_eq!(why(&s, "measure.add"), None);
    assert_eq!(why(&s, "report.new"), None);
    // a new model is not a file: save needs save_as first
    assert_eq!(why(&s, "file.save").as_deref(), Some(Need::File.reason()));
    run(&mut s, "table.add", json!({"name": "T"}));
    assert_eq!(why(&s, "edit.undo"), None);
    assert_eq!(why(&s, "edit.redo").as_deref(), Some(Need::Redo.reason()));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(why(&s, "edit.redo"), None);
    run(&mut s, "report.new", json!({}));
    assert_eq!(why(&s, "report.new").as_deref(), Some(Need::NoReport.reason()));
    assert_eq!(why(&s, "page.add"), None);
    assert_eq!(
        why(&s, "model.storage_stats").as_deref(),
        Some(Need::Data.reason())
    );

    // a saved project: file commands, refresh; no cache.abf, so no stored data
    let dir = TempDir::new("engine-command-ui-enabled");
    run(&mut s, "project.new", json!({"dir": dir.path(), "name": "P"}));
    for id in [
        "file.save",
        "file.open_in_desktop",
        "model.refresh",
        "file.set_cache_abf",
    ] {
        assert_eq!(why(&s, id), None, "{id}");
    }
    assert_eq!(
        why(&s, "model.storage_stats").as_deref(),
        Some(Need::Data.reason())
    );

    // read-only callers: only reading commands
    let r = |s: &Session, id: &str| s.command_enabled(id, Caller::Reader).err();
    assert_eq!(r(&s, "model.summary"), None);
    assert!(r(&s, "table.add").unwrap().contains("read-only"));
    assert_eq!(r(&s, "file.open"), None, "no unsaved changes");
    run(&mut s, "table.add", json!({"name": "U"}));
    assert!(r(&s, "file.open").unwrap().contains("unsaved"));

    // server mode: the policy's refusals
    let srv = Session::server(ServerConfig::new(dir.path()).unwrap());
    let e = srv
        .command_enabled("file.open_in_desktop", Caller::Editor)
        .unwrap_err();
    assert!(e.contains("server"), "{e}");
    assert_eq!(srv.command_enabled("file.open", Caller::Editor), Ok(()));
}

#[test]
fn enabled_checks_change_nothing() {
    let mut s = sample();
    run(&mut s, "table.add", json!({"name": "Extra"}));
    let before = (s.revision, s.dirty, s.report_dirty, s.last_command.clone());
    let undo = s.execute("edit.history", &json!({})).unwrap();
    let before_hist = (s.revision, s.last_command.clone());
    for c in COMMANDS {
        for caller in [Caller::Editor, Caller::Reader] {
            let _ = s.command_enabled(c.id, caller);
        }
    }
    let _ = s.available_commands(None);
    let _ = s.reader_commands(None);
    assert_eq!((s.revision, s.last_command.clone()), before_hist);
    assert_eq!(
        (before.0, before.1, before.2),
        (s.revision, s.dirty, s.report_dirty)
    );
    assert_eq!(s.execute("edit.history", &json!({})).unwrap(), undo);
}

#[test]
fn listings_show_place_shortcut_form_and_enabled() {
    let all = serde_json::to_value(list_commands(None)).unwrap();
    let open = all
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "file.open")
        .unwrap();
    assert_eq!(
        open["place"],
        json!({"kind": "ribbon", "tab": "file", "contextual": false, "group": "File", "label": "Open",
               "icon": "folder"})
    );
    assert_eq!(open["shortcut"], "Ctrl+O");
    assert_eq!(
        open["form"][0],
        json!({"name": "path", "kind": "path", "required": true})
    );
    assert_eq!(open["needs"], json!([]));
    assert!(open.get("enabled").is_none(), "no session, no enabled state");
    // the params doc stays
    assert!(open["params"].as_str().unwrap().contains("data_root"));

    let s = Session::new();
    let listed = serde_json::to_value(s.available_commands(None)).unwrap();
    let save = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "file.save")
        .unwrap();
    assert_eq!(save["enabled"], false);
    assert_eq!(save["disabled_reason"], Need::File.reason());
    assert_eq!(save["needs"], json!(["file"]));
    let open = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "file.open")
        .unwrap();
    assert_eq!(open["enabled"], true);
    assert!(open.get("disabled_reason").is_none());
    let menu = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "page.remove")
        .unwrap();
    assert_eq!(menu["place"]["kind"], "menu");
    assert_eq!(menu["place"]["menu"], "page");
    let visual = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "visual.filter")
        .unwrap();
    assert_eq!(visual["place"]["contextual"], true);
    let enum_param = &visual["form"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "op")
        .unwrap();
    assert_eq!(enum_param["kind"], "enum");
    assert!(enum_param["values"].as_array().unwrap().len() > 5);
    let name_param = &visual["form"][0];
    assert_eq!(
        name_param,
        &json!({"name": "page", "kind": "name", "object": "page", "required": true})
    );

    let readers = s.reader_commands(None);
    assert!(readers.iter().all(|c| c.enabled.is_some()));
    assert!(readers.iter().any(|c| c.id == "model.summary"));
    assert!(!readers.iter().any(|c| c.id == "table.add"));
}

// ---- docs/commands-ui.md ---------------------------------------------------------------------

/// The placement table rows as `cargo xtask commands-ui` writes them (`xtask/src/commands_ui.rs`):
/// ribbon tabs in tab order, groups in the order they first appear, then the context menus, then
/// the palette; within each, the registry order.
fn rows() -> Vec<String> {
    let row = |c: &Command, place: String, group: &str| {
        format!(
            "| {place} | {group} | `{}` | {} | `{}` | {} | {} |",
            c.id,
            c.place.label,
            c.place.icon,
            c.shortcut.unwrap_or(""),
            c.enabled.iter().map(|n| n.name()).collect::<Vec<_>>().join(", ")
        )
    };
    let mut out = Vec::new();
    for t in Tab::ALL {
        let mut groups: Vec<&str> = Vec::new();
        for c in COMMANDS {
            if let At::Ribbon { tab, group } = c.place.at
                && tab == t
                && !groups.contains(&group)
            {
                groups.push(group);
            }
        }
        for g in groups {
            for c in COMMANDS {
                if c.place.at == (At::Ribbon { tab: t, group: g }) {
                    let place = if t.contextual() {
                        format!("{} (contextual)", t.caption())
                    } else {
                        t.caption().to_string()
                    };
                    out.push(row(c, place, g));
                }
            }
        }
    }
    for m in Menu::ALL {
        for c in COMMANDS.iter().filter(|c| c.place.at == At::Menu(m)) {
            out.push(row(c, format!("Menu: {}", m.caption()), ""));
        }
    }
    for c in COMMANDS.iter().filter(|c| c.place.at == At::Palette) {
        out.push(row(c, "Palette".into(), ""));
    }
    out
}

#[test]
fn the_placement_doc_matches_the_registry() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/commands-ui.md");
    let text = std::fs::read_to_string(&path).unwrap();
    let got: Vec<String> = text
        .lines()
        .skip_while(|l| !l.starts_with("<!-- PLACEMENT -->"))
        .take_while(|l| !l.starts_with("<!-- /PLACEMENT -->"))
        .filter(|l| l.starts_with("| ") && l.contains(" | `"))
        .map(str::to_string)
        .collect();
    let want = rows();
    assert_eq!(
        got.len(),
        want.len(),
        "docs/commands-ui.md: one row per command (`cargo xtask commands-ui`)"
    );
    let diff: BTreeMap<usize, (&String, Option<&String>)> = want
        .iter()
        .enumerate()
        .filter(|(i, w)| got.get(*i) != Some(*w))
        .map(|(i, w)| (i, (w, got.get(i))))
        .take(5)
        .collect();
    assert!(
        diff.is_empty(),
        "docs/commands-ui.md is stale (run `cargo xtask commands-ui`): {diff:#?}"
    );
}

#[test]
fn declared_defaults_fit_their_kind_and_the_params_doc() {
    let mut flags = 0;
    for c in COMMANDS {
        for p in c.form {
            let Some(d) = p.default else { continue };
            assert!(
                !p.required,
                "{}.{}: a required parameter has no default",
                c.id, p.name
            );
            let v: Value =
                serde_json::from_str(d).unwrap_or_else(|e| panic!("{}.{}: {d}: {e}", c.id, p.name));
            let fits = match p.kind {
                Kind::Bool => v.is_boolean(),
                Kind::Integer => v.is_i64() || v.is_u64(),
                Kind::Number => v.is_number(),
                Kind::Enum(values) => v.as_str().is_some_and(|s| values.contains(&s)),
                Kind::Json => true,
                _ => v.is_string(),
            };
            assert!(fits, "{}.{}: default {d} does not fit {:?}", c.id, p.name, p.kind);
            if p.kind == Kind::Bool {
                flags += 1;
            }
        }
        // the params doc's "(default true)" and the declaration agree
        for p in c.form.iter().filter(|p| p.kind == Kind::Bool) {
            for (word, value) in [("true", "true"), ("false", "false")] {
                if c.params.contains(&format!("{}?: bool (default {word}", p.name)) {
                    assert_eq!(
                        p.default,
                        Some(value),
                        "{}.{}: the doc says default {word}",
                        c.id,
                        p.name
                    );
                }
            }
        }
    }
    assert!(flags > 90, "optional booleans with a declared default: {flags}");
    // listed with the form
    let listed = serde_json::to_value(list_commands(Some("file.save"))).unwrap();
    let save = listed
        .as_array()
        .and_then(|a| a.iter().find(|c| c["id"] == "file.save"))
        .cloned()
        .unwrap_or_default();
    assert_eq!(save["form"][0]["default"], json!(true), "{save}");
}
