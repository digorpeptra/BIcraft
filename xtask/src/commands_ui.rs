//! `cargo xtask commands-ui [--check]`: writes `docs/commands-ui.md`, the placement table of every
//! engine command (ROADMAP M11.1): its ribbon tab and group, context menu or palette, UI label,
//! icon, shortcut and what it needs to be enabled. The table is read from the `COMMANDS` rows of
//! `crates/engine/src/commands.rs` (their `place`, `shortcut` and `enabled` fields), so xtask
//! does not have to build the engine; the engine's test `the_placement_doc_matches_the_registry`
//! checks the rows against the compiled registry. `cargo xtask docs` fails when the file is stale.

use std::collections::BTreeMap;
use std::path::Path;

const COMMANDS: &str = "crates/engine/src/commands.rs";
const DOC: &str = "docs/commands-ui.md";

/// The ribbon tabs in tab-row order (`command_ui::Tab::ALL`), and whether each is contextual.
const TABS: &[(&str, bool)] = &[
    ("File", false),
    ("Home", false),
    ("Insert", false),
    ("Report", false),
    ("Model", false),
    ("Data", false),
    ("Analyze", false),
    ("View", false),
    ("Optimize", false),
    ("Visual", true),
    ("Table", true),
    ("Measure", true),
];

/// The context menus (`command_ui::Menu::ALL`) and their captions.
const MENUS: &[(&str, &str)] = &[
    ("Page", "Page tab"),
    ("Visual", "Visual"),
    ("ModelObject", "Model object"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum At {
    Ribbon { tab: String, group: String },
    Menu(String),
    Palette,
}

#[derive(Debug, Clone)]
struct Row {
    id: String,
    at: At,
    label: String,
    icon: String,
    shortcut: Option<String>,
    needs: Vec<String>,
}

pub fn cmd(args: &[String]) -> Result<(), String> {
    let root = crate::root();
    let check = match args {
        [] => false,
        [a] if a == "--check" => true,
        _ => return Err("usage: cargo xtask commands-ui [--check]".into()),
    };
    if check {
        let problems = self::check(&root);
        if problems.is_empty() {
            eprintln!("commands-ui: {DOC} is up to date");
            return Ok(());
        }
        return Err(problems.join("\n"));
    }
    let text = render(&read_rows(&root)?);
    std::fs::write(root.join(DOC), text).map_err(|e| format!("{DOC}: {e}"))?;
    eprintln!("commands-ui: wrote {DOC}");
    Ok(())
}

/// Problems for `cargo xtask docs`: the doc differs from what the registry gives.
pub fn check(root: &Path) -> Vec<String> {
    let want = match read_rows(root) {
        Ok(rows) => render(&rows),
        Err(e) => return vec![e],
    };
    match std::fs::read_to_string(root.join(DOC)) {
        Ok(have) if have.replace("\r\n", "\n") == want => Vec::new(),
        Ok(_) => vec![format!("{DOC}: stale (run `cargo xtask commands-ui`)")],
        Err(e) => vec![format!("{DOC}: {e} (run `cargo xtask commands-ui`)")],
    }
}

fn read_rows(root: &Path) -> Result<Vec<Row>, String> {
    let text = std::fs::read_to_string(root.join(COMMANDS)).map_err(|e| format!("{COMMANDS}: {e}"))?;
    parse(&text)
}

/// The rows of the `COMMANDS` table: each `    Command {` … `    },` block, its fields at eight
/// spaces of indentation.
fn parse(text: &str) -> Result<Vec<Row>, String> {
    let start = text
        .find("pub static COMMANDS")
        .ok_or_else(|| format!("{COMMANDS}: no COMMANDS table"))?;
    let body = text.get(start..).unwrap_or("");
    let end = body.find("\n];").unwrap_or(body.len());
    let body = body.get(..end).unwrap_or("");
    let mut rows = Vec::new();
    let mut fields: Vec<(String, String)> = Vec::new();
    let mut in_block = false;
    for line in body.lines() {
        if line == "    Command {" {
            in_block = true;
            fields.clear();
            continue;
        }
        if !in_block {
            continue;
        }
        if line == "    }," {
            in_block = false;
            rows.push(row(&fields)?);
            continue;
        }
        let field = line
            .strip_prefix("        ")
            .filter(|l| !l.starts_with(' '))
            .and_then(|l| l.split_once(':'))
            .filter(|(name, _)| name.chars().all(|c| c.is_ascii_lowercase() || c == '_'));
        match (field, fields.last_mut()) {
            (Some((name, rest)), _) => fields.push((name.to_string(), rest.to_string())),
            (None, Some((_, value))) => {
                value.push('\n');
                value.push_str(line);
            }
            (None, None) => {}
        }
    }
    if rows.is_empty() {
        return Err(format!("{COMMANDS}: no command rows found"));
    }
    Ok(rows)
}

fn field<'a>(fields: &'a [(String, String)], name: &str, id: &str) -> Result<&'a str, String> {
    fields
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
        .ok_or_else(|| format!("{COMMANDS}: `{id}` has no `{name}` field"))
}

fn row(fields: &[(String, String)]) -> Result<Row, String> {
    let id_text = field(fields, "id", "?")?;
    let id = strings(id_text)
        .into_iter()
        .next()
        .ok_or_else(|| format!("{COMMANDS}: an id is not a string literal: {id_text}"))?;
    let place = compact(field(fields, "place", &id)?);
    let bad = || format!("{COMMANDS}: `{id}`: cannot read place `{place}`");
    let args = strings(&place);
    let (at, label, icon) = if let Some(rest) = place.strip_prefix("Place::ribbon(Tab::") {
        let tab: String = rest.chars().take_while(char::is_ascii_alphanumeric).collect();
        match args.as_slice() {
            [group, label, icon] => (
                At::Ribbon {
                    tab,
                    group: group.clone(),
                },
                label.clone(),
                icon.clone(),
            ),
            _ => return Err(bad()),
        }
    } else if let Some(rest) = place.strip_prefix("Place::menu(Menu::") {
        let menu: String = rest.chars().take_while(char::is_ascii_alphanumeric).collect();
        match args.as_slice() {
            [label, icon] => (At::Menu(menu), label.clone(), icon.clone()),
            _ => return Err(bad()),
        }
    } else if place.starts_with("Place::palette(") {
        match args.as_slice() {
            [label, icon] => (At::Palette, label.clone(), icon.clone()),
            _ => return Err(bad()),
        }
    } else {
        return Err(bad());
    };
    let shortcut_text = compact(field(fields, "shortcut", &id)?);
    let shortcut = match shortcut_text.as_str() {
        "None," => None,
        _ => Some(
            strings(&shortcut_text)
                .into_iter()
                .next()
                .ok_or_else(|| format!("{COMMANDS}: `{id}`: cannot read shortcut `{shortcut_text}`"))?,
        ),
    };
    let needs = compact(field(fields, "enabled", &id)?)
        .split("Need::")
        .skip(1)
        .map(|n| {
            snake(
                &n.chars()
                    .take_while(char::is_ascii_alphanumeric)
                    .collect::<String>(),
            )
        })
        .collect();
    Ok(Row {
        id,
        at,
        label,
        icon,
        shortcut,
        needs,
    })
}

/// `text` without whitespace outside string literals.
fn compact(text: &str) -> String {
    let mut out = String::new();
    let mut in_str = false;
    let mut escaped = false;
    for c in text.chars() {
        if in_str {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
            }
        } else if c == '"' {
            in_str = true;
            out.push(c);
        } else if !c.is_whitespace() {
            out.push(c);
        }
    }
    out
}

/// The string literals of `text`, unescaped (`\"` and `\\`).
fn strings(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur: Option<String> = None;
    let mut escaped = false;
    for c in text.chars() {
        match cur.as_mut() {
            None if c == '"' => cur = Some(String::new()),
            None => {}
            Some(s) if escaped => {
                s.push(c);
                escaped = false;
            }
            Some(_) if c == '\\' => escaped = true,
            Some(_) if c == '"' => out.extend(cur.take()),
            Some(s) => s.push(c),
        }
    }
    out
}

/// `SavedReportFile` → `saved_report_file` (`command_ui::Need::name`).
fn snake(camel: &str) -> String {
    let mut out = String::new();
    for (i, c) in camel.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn tab_caption(tab: &str) -> String {
    match TABS.iter().find(|(t, _)| *t == tab) {
        Some((t, true)) => format!("{t} (contextual)"),
        _ => tab.to_string(),
    }
}

fn table_row(r: &Row, place: &str, group: &str) -> String {
    format!(
        "| {place} | {group} | `{}` | {} | `{}` | {} | {} |",
        r.id,
        r.label,
        r.icon,
        r.shortcut.as_deref().unwrap_or(""),
        r.needs.join(", ")
    )
}

/// The rows in table order: ribbon tabs in tab order and groups in first-appearance order, then
/// the context menus, then the palette; within each, the registry order.
fn ordered(rows: &[Row]) -> Vec<(String, String, &Row)> {
    let mut out = Vec::new();
    for (tab, _) in TABS {
        let mut groups: Vec<&str> = Vec::new();
        for r in rows {
            if let At::Ribbon { tab: t, group } = &r.at
                && t == tab
                && !groups.contains(&group.as_str())
            {
                groups.push(group);
            }
        }
        for g in groups {
            for r in rows {
                if matches!(&r.at, At::Ribbon { tab: t, group } if t == tab && group == g) {
                    out.push((tab_caption(tab), g.to_string(), r));
                }
            }
        }
    }
    for (menu, caption) in MENUS {
        for r in rows.iter().filter(|r| r.at == At::Menu(menu.to_string())) {
            out.push((format!("Menu: {caption}"), String::new(), r));
        }
    }
    for r in rows.iter().filter(|r| r.at == At::Palette) {
        out.push(("Palette".to_string(), String::new(), r));
    }
    out
}

fn render(rows: &[Row]) -> String {
    let ordered = ordered(rows);
    let mut s = String::new();
    s.push_str(
        "# Command placement: every command has its button\n\n\
         <!-- Generated by `cargo xtask commands-ui` from the `place`, `shortcut` and `enabled` fields of \
         crates/engine/src/commands.rs; `cargo xtask docs` fails when it is stale. Do not edit by hand. -->\n\n\
         Every engine command declares where its button lives in the app (AGENTS.md §9, ROADMAP M11.1): a \
         group of a ribbon tab, a context menu, or the command palette only (Ctrl+Shift+P), with a short UI \
         label and an icon; an optional shortcut; and what the session needs before the button is enabled. \
         The palette lists every command the session's role and server policy allow, whatever its place. \
         `list_commands` and `bicraft-cli describe` show the same fields (`place`, `shortcut`, `needs`, `form`; \
         with a session also `enabled` and `disabled_reason`), see [mcp.md](mcp.md). The types are in \
         `crates/engine/src/command_ui.rs`; the tabs follow docs/design.md §5 and Power BI \
         Desktop's ribbon (File backstage, Home, Insert, then Report, Model, Data, Analyze, View and Optimize, plus the \
         contextual Visual, Table and Measure tabs that appear with a selection).\n\n",
    );
    s.push_str(
        "Needs: `file` an open file; `saved_report_file` a PBIX, PBIT or PBIP file; `project` a PBIP \
         project; `model` a model (open or new); `report` a report; `no_report` no report yet; `data` stored \
         data (a PBIX DataModel or a PBIP cache.abf); `undo` / `redo` a step to undo or redo. Read-only \
         callers (`reader` grants) see only the commands that read, and a server refuses what its policy \
         denies; both come on top of the needs.\n\n",
    );

    // summary
    s.push_str("## Summary\n\n| Place | Groups | Commands |\n|---|---|---|\n");
    let mut by_place: Vec<(String, Vec<String>, usize)> = Vec::new();
    for (place, group, _) in &ordered {
        match by_place.iter_mut().find(|(p, _, _)| p == place) {
            Some((_, groups, n)) => {
                if !group.is_empty() && !groups.contains(group) {
                    groups.push(group.clone());
                }
                *n += 1;
            }
            None => by_place.push((
                place.clone(),
                if group.is_empty() {
                    Vec::new()
                } else {
                    vec![group.clone()]
                },
                1,
            )),
        }
    }
    for (place, groups, n) in &by_place {
        s.push_str(&format!("| {place} | {} | {n} |\n", groups.join(", ")));
    }
    s.push_str(&format!("| **All** | | **{}** |\n\n", rows.len()));

    // shortcuts
    s.push_str(
        "## Shortcuts\n\nOffice and DeckCraft conventions (Ctrl on Windows, Linux and in the browser). \
         Each chord belongs to one command; the UI keeps Ctrl+Shift+P for the palette.\n\n\
         | Shortcut | Command | Label |\n|---|---|---|\n",
    );
    for r in rows.iter().filter(|r| r.shortcut.is_some()) {
        s.push_str(&format!(
            "| {} | `{}` | {} |\n",
            r.shortcut.as_deref().unwrap_or(""),
            r.id,
            r.label
        ));
    }
    s.push('\n');

    // the table
    s.push_str("## Placement table\n\n<!-- PLACEMENT -->\n");
    s.push_str(
        "| Place | Group | Command | Label | Icon | Shortcut | Needs |\n|---|---|---|---|---|---|---|\n",
    );
    for (place, group, r) in &ordered {
        s.push_str(&table_row(r, place, group));
        s.push('\n');
    }
    s.push_str("<!-- /PLACEMENT -->\n\n");

    // icons
    let mut icons: BTreeMap<&str, usize> = BTreeMap::new();
    for r in rows {
        *icons.entry(r.icon.as_str()).or_default() += 1;
    }
    s.push_str(
        "## Icons\n\nThe icon names the commands use (kebab-case, drawn in code by the UI; docs/design.md §6). \
         A name the UI does not draw yet falls back to a generic icon until it is added.\n\n",
    );
    let list: Vec<String> = icons.iter().map(|(i, n)| format!("`{i}` ({n})")).collect();
    s.push_str(&list.join(", "));
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_rows_however_rustfmt_wraps_them() {
        let text = "pub static COMMANDS: &[Command] = &[\n    Command {\n        id: \"a.b\",\n        label: \"x: y\",\n        place: Place::ribbon(\n            Tab::Home,\n            \"File\",\n            \"Open \\\"it\\\"\",\n            \"folder\",\n        ),\n        shortcut: Some(\"Ctrl+O\"),\n        enabled: &[Need::File, Need::SavedReportFile],\n        form: &[],\n    },\n    Command {\n        id: \"c.d\",\n        place: Place::menu(Menu::ModelObject, \"Delete\", \"delete\"),\n        shortcut: None,\n        enabled: &[],\n    },\n];\n";
        let rows = parse(text).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].label, "Open \"it\"");
        assert_eq!(
            rows[0].at,
            At::Ribbon {
                tab: "Home".into(),
                group: "File".into()
            }
        );
        assert_eq!(rows[0].shortcut.as_deref(), Some("Ctrl+O"));
        assert_eq!(rows[0].needs, ["file", "saved_report_file"]);
        assert_eq!(rows[1].at, At::Menu("ModelObject".into()));
        assert!(rows[1].needs.is_empty());
        let doc = render(&rows);
        assert!(
            doc.contains(
                "| Home | File | `a.b` | Open \"it\" | `folder` | Ctrl+O | file, saved_report_file |"
            )
        );
        assert!(doc.contains("| Menu: Model object |  | `c.d` | Delete | `delete` |  |  |"));
    }
}
