//! Bookmarks (Desktop's Bookmarks pane, "Signets" in French): the report's saved states.
//!
//! A bookmark is a `definition/bookmarks/<name>.bookmark.json` document (PBIR schema
//! `bookmark` 2.1.0, [pbir.md](../../../docs/formats/pbir.md)) kept raw in
//! [`bicraft_report::Report::bookmarks`]; its place, display name and group in the report live in
//! `definition/bookmarks/bookmarks.json` (`bookmarksMetadata` 1.0.0: `items` of `{name}`, or
//! `{name, displayName, children: [names]}` for a group), which the report keeps as a resource.
//!
//! The commands capture and restore the state BIcraft's report model carries:
//!
//! - `explorationState.activeSection`: the page shown when the bookmark was captured;
//! - `sections[<page>].visualContainers[<visual>].singleVisual.display.mode`: `"hidden"` for a
//!   visual hidden with the Selection pane (a visual left out shows), `"spotlight"` for the
//!   reader's spotlight; a container-level `display` (which BIcraft wrote before 2026-11-19, not
//!   in the schema) is still read;
//! - `singleVisual.orderBy`: the reader's sort of a visual (`view.interact` `sort`), as
//!   semanticQuery sort clauses;
//! - `filters` at the report (`explorationState.filters`), page (`sections[p].filters`) and visual
//!   (`sections[p].visualContainers[v].filters`) levels, as `{"byName": {<name>: {name, filter}}}`;
//! - each slicer's selection as the session shows it (a click in the viewer included):
//!   `sections[p].visualContainers[v].singleVisual.objects` (`merge.general` with the filter,
//!   `merge.data` with the range properties, or `remove` of `general.filter`; see
//!   `interact_bookmark.rs`). Applying it writes the selection back into the slicer's
//!   `objects` and drops the reader's own selection of that slicer.
//!
//! `options` says what applying the bookmark changes, as Desktop's Data / Display / Current page
//! checkboxes do: `suppressData`, `suppressDisplay`, `suppressActiveSection`, and
//! `applyOnlyToTargetVisuals` with `targetVisualNames` for a bookmark of some visuals.
//!
//! - the data points selected in a visual, as its container's `highlight`
//!   (`interact_highlight.rs`); applying the bookmark selects them again in the session.
//!
//! The other reader state (drill) is not captured: a bookmark captured
//! here holds what the file holds plus the slicers' and data points' selections. A bookmark written by Desktop is applied for
//! the parts this module knows and its other sections are left alone.

use bicraft_report::{NamedDocument, Report, Resource};
use serde_json::{Map, Value, json};

use crate::Session;
use crate::interact::Interactions;
use crate::params::{allow, opt_bool, opt_str, str_param};
use crate::report::{page_index, parts};

/// The `bookmarks.json` resource, relative to the report root.
const META: &str = "definition/bookmarks/bookmarks.json";
/// Public schema URLs (`crates/pbir/schemas/`).
const BOOKMARK_SCHEMA: &str =
    "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/bookmark/2.1.0/schema.json";
const META_SCHEMA: &str = "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/bookmarksMetadata/1.0.0/schema.json";
/// `explorationState.version`, the value this module writes.
const STATE_VERSION: &str = "1.0.0";
/// Most bookmarks (and groups) one report holds.
const MAX_BOOKMARKS: usize = 1_000;
/// Longest bookmark display name, in characters.
const MAX_NAME: usize = 200;
/// Most visuals one capture targets.
const MAX_TARGETS: usize = 1_000;

// ---------------------------------------------------------------------------------------------
// bookmark.list
// ---------------------------------------------------------------------------------------------

/// `bookmark.list`: the report's bookmarks and groups in their saved order.
pub(crate) fn bookmark_list(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &[])?;
    Ok(bookmarks_view(s))
}

/// `bookmark.list` on a shared session (`shared.rs`): it reads, never writes.
pub(crate) fn bookmark_list_shared(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &[])?;
    Ok(bookmarks_view(s))
}

/// The bookmarks of the session's report, for `bookmark.list` and the snapshot: their order,
/// names, display names, group, options and what they captured. Never fails (an unreadable
/// `bookmarks.json` leaves the order it has).
pub(crate) fn bookmarks_view(s: &Session) -> Value {
    let Some(r) = s.report.as_ref() else {
        return json!({"bookmarks": [], "groups": [], "count": 0});
    };
    let meta = meta(r).unwrap_or_else(|_| json!({"items": []}));
    let mut list: Vec<Value> = Vec::new();
    let mut groups: Vec<Value> = Vec::new();
    for item in items(&meta) {
        if let Some(children) = item.get("children").and_then(Value::as_array) {
            let name = item
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let display = group_display(item).unwrap_or_else(|| name.clone());
            let names: Vec<String> = children
                .iter()
                .take(MAX_BOOKMARKS)
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
            groups.push(json!({
                "name": name,
                "display_name": display,
                "children": names,
            }));
            for child in &names {
                push_entry(&mut list, r, child, Some((&name, &display)));
            }
        } else if let Some(name) = item.get("name").and_then(Value::as_str) {
            push_entry(&mut list, r, name, None);
        }
    }
    // hand-edited files: a document no item lists still exists
    for b in &r.bookmarks {
        if !list
            .iter()
            .any(|x| x.get("name").and_then(Value::as_str) == Some(b.name.as_str()))
        {
            push_entry(&mut list, r, &b.name, None);
        }
    }
    let count = list.len();
    json!({"bookmarks": list, "groups": groups, "count": count})
}

/// One bookmark's entry of [`bookmarks_view`].
fn push_entry(list: &mut Vec<Value>, r: &Report, name: &str, group: Option<(&str, &str)>) {
    let Some(b) = r.bookmarks.iter().find(|b| b.name == name) else {
        // listed in bookmarks.json but its document is gone: the writer drops it
        return;
    };
    let doc = &b.document;
    let options = doc.get("options").cloned().unwrap_or(Value::Null);
    let flag = |k: &str| options.get(k).and_then(Value::as_bool) == Some(true);
    let targets: Vec<Value> = options
        .get("targetVisualNames")
        .and_then(Value::as_array)
        .map(|a| a.iter().take(MAX_TARGETS).cloned().collect())
        .unwrap_or_default();
    let state = doc.get("explorationState");
    let mut hidden = 0_u64;
    let mut filters = 0_u64;
    let mut page = Value::Null;
    if let Some(x) = state {
        page = x.get("activeSection").cloned().unwrap_or(Value::Null);
        filters += count_filters(x.get("filters"));
        for sec in x
            .get("sections")
            .and_then(Value::as_object)
            .map(|m| m.values())
            .into_iter()
            .flatten()
        {
            filters += count_filters(sec.get("filters"));
            for c in sec
                .get("visualContainers")
                .and_then(Value::as_object)
                .map(|m| m.values())
                .into_iter()
                .flatten()
            {
                if display_mode(c) == Some("hidden") {
                    hidden += 1;
                }
                filters += count_filters(c.get("filters"));
            }
        }
    }
    list.push(json!({
        "name": b.name,
        "display_name": bookmark_name(b),
        "group": group.map(|g| g.1),
        "group_name": group.map(|g| g.0),
        "data": !flag("suppressData"),
        "display": !flag("suppressDisplay"),
        "current_page": !flag("suppressActiveSection"),
        "targets": targets,
        "page": page,
        "hidden": hidden,
        "filters": filters,
    }));
}

fn count_filters(state: Option<&Value>) -> u64 {
    state
        .and_then(|x| x.get("byName"))
        .and_then(Value::as_object)
        .map_or(0, |m| m.len() as u64)
}

// ---------------------------------------------------------------------------------------------
// bookmark.capture / bookmark.update
// ---------------------------------------------------------------------------------------------

/// `bookmark.capture`: a new bookmark of the report's current page, hidden visuals and filters.
pub(crate) fn bookmark_capture(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "display_name",
            "group",
            "data",
            "display",
            "current_page",
            "visuals",
            "key",
        ],
    )?;
    let display_name = str_param(p, "display_name")?;
    check_display_name(display_name)?;
    let key = opt_str(p, "key")?;
    if let Some(k) = key {
        crate::report_apply::check_key(k)?;
    }
    let group = opt_str(p, "group")?;
    let data = opt_bool(p, "data")?.unwrap_or(true);
    let display = opt_bool(p, "display")?.unwrap_or(true);
    let current_page = opt_bool(p, "current_page")?.unwrap_or(false);
    let targets = targets_param(p)?;
    let view = s.interactions.clone();
    let shown = shown_report(s);
    let out = {
        let (_, r) = parts(s)?;
        if r.bookmarks.len() >= MAX_BOOKMARKS {
            return Err(format!("a report holds at most {MAX_BOOKMARKS} bookmarks"));
        }
        let page = current_page_name(r)?;
        if let Some(list) = &targets {
            check_targets(r, &page, list)?;
        }
        if r.bookmarks
            .iter()
            .any(|b| bookmark_name(b).eq_ignore_ascii_case(display_name))
        {
            return Err(format!(
                "a bookmark called `{display_name}` already exists; bookmark display names must be unique"
            ));
        }
        if let Some(g) = group {
            find_group(r, g)?;
        }
        let base = match key {
            Some(k) => crate::report_apply::stable_name(&format!("bookmark/{k}")),
            None => crate::report_apply::stable_name(display_name),
        };
        let name = unique_name(r, &base);
        let mut doc = Map::new();
        doc.insert("$schema".into(), json!(BOOKMARK_SCHEMA));
        doc.insert("name".into(), json!(name));
        doc.insert("displayName".into(), json!(display_name));
        doc.insert(
            "options".into(),
            options_json(data, display, current_page, targets.as_deref()),
        );
        doc.insert(
            "explorationState".into(),
            state_of(r, &view, shown.as_ref(), &page, targets.as_deref()),
        );
        let mut meta = meta(r)?;
        place_item(&mut meta, &name, group)?;
        meta_save(r, &meta)?;
        r.bookmarks.push(NamedDocument {
            name: name.clone(),
            document: Value::Object(doc),
        });
        json!({
            "bookmark": name,
            "display_name": display_name,
            "group": group,
            "page": page,
        })
    };
    s.report_dirty = true;
    Ok(out)
}

/// `bookmark.update`: captures the current state into an existing bookmark, keeping its name,
/// display name and options.
pub(crate) fn bookmark_update(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["bookmark"])?;
    let key = str_param(p, "bookmark")?;
    let view = s.interactions.clone();
    let shown = shown_report(s);
    let (name, changed, page) = {
        let (_, r) = parts(s)?;
        let i = bookmark_index(r, key)?;
        let page = current_page_name(r)?;
        let targets = bookmark_targets(r, i);
        let state = state_of(r, &view, shown.as_ref(), &page, targets.as_deref());
        let Some(b) = r.bookmarks.get_mut(i) else {
            return Err("internal: bookmark index out of range".into());
        };
        let changed = b.document.get("explorationState") != Some(&state);
        if changed && let Some(o) = b.document.as_object_mut() {
            o.insert("explorationState".into(), state);
        }
        (b.name.clone(), changed, page)
    };
    if changed {
        s.report_dirty = true;
    }
    Ok(json!({"bookmark": name, "changed": changed, "page": page}))
}

// ---------------------------------------------------------------------------------------------
// bookmark.set / bookmark.remove
// ---------------------------------------------------------------------------------------------

/// `bookmark.set`: renames a bookmark, changes its options, or moves it.
pub(crate) fn bookmark_set(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "bookmark",
            "display_name",
            "data",
            "display",
            "current_page",
            "visuals",
            "group",
            "index",
        ],
    )?;
    let key = str_param(p, "bookmark")?;
    let display_name = opt_str(p, "display_name")?;
    if let Some(n) = display_name {
        check_display_name(n)?;
    }
    let data = opt_bool(p, "data")?;
    let display = opt_bool(p, "display")?;
    let current_page = opt_bool(p, "current_page")?;
    let targets = if p.get("visuals").is_some() {
        Some(targets_param(p)?.unwrap_or_default())
    } else {
        None
    };
    let group = match p.get("group") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(g)) if !g.is_empty() => Some(Some(g.as_str())),
        Some(_) => return Err("parameter `group` must be a group name or null".into()),
    };
    let index = match p.get("index") {
        None | Some(Value::Null) => None,
        Some(v) => match v.as_i64() {
            Some(i) if i >= 0 && i <= MAX_BOOKMARKS as i64 => Some(i as usize),
            _ => return Err("parameter `index` must be a whole number, 0 or more".into()),
        },
    };
    let (_, r) = parts(s)?;
    let i = bookmark_index(r, key)?;
    let name = r
        .bookmarks
        .get(i)
        .map(|b| b.name.clone())
        .ok_or("internal: bookmark index out of range")?;
    if let Some(n) = display_name
        && r.bookmarks
            .iter()
            .enumerate()
            .any(|(j, b)| j != i && bookmark_name(b).eq_ignore_ascii_case(n))
    {
        return Err(format!(
            "a bookmark called `{n}` already exists; bookmark display names must be unique"
        ));
    }
    if let Some(Some(g)) = group {
        find_group(r, g)?;
    }
    let mut changed = false;
    // the document: display name and options
    let existing = r
        .bookmarks
        .get(i)
        .map(|b| b.document.clone())
        .ok_or("internal: bookmark index out of range")?;
    let mut doc = existing.clone();
    let opts = doc.get("options").cloned().unwrap_or(Value::Null);
    if let Some(o) = doc.as_object_mut() {
        if let Some(n) = display_name
            && o.get("displayName").and_then(Value::as_str) != Some(n)
        {
            o.insert("displayName".into(), json!(n));
            changed = true;
        }
        if data.is_some() || display.is_some() || current_page.is_some() || targets.is_some() {
            let flag = |k: &str, v: Option<bool>| match v {
                // the parameter says whether the bookmark applies that state
                Some(applies) => !applies,
                // not given: the suppress flag the bookmark already holds
                None => opts.get(k).and_then(Value::as_bool).unwrap_or(false),
            };
            let d = flag("suppressData", data);
            let s2 = flag("suppressDisplay", display);
            let c = flag("suppressActiveSection", current_page);
            let t = targets.as_deref();
            let next = if let Some(list) = t {
                options_json(!d, !s2, !c, Some(list))
            } else {
                let kept: Vec<String> = opts
                    .get("targetVisualNames")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                    .unwrap_or_default();
                let apply_only = opts.get("applyOnlyToTargetVisuals").and_then(Value::as_bool) == Some(true);
                if apply_only && !kept.is_empty() {
                    options_json(!d, !s2, !c, Some(&kept))
                } else {
                    options_json(!d, !s2, !c, None)
                }
            };
            if o.get("options") != Some(&next) {
                o.insert("options".into(), next);
                changed = true;
            }
        }
    }
    if changed && let Some(b) = r.bookmarks.get_mut(i) {
        b.document = doc;
    }
    // the metadata: the group and the position
    if group.is_some() || index.is_some() {
        let mut meta = meta(r)?;
        let before = meta.clone();
        if let Some(g) = group {
            move_to_group(&mut meta, &name, g)?;
        }
        if let Some(at) = index {
            move_to_index(&mut meta, &name, at);
        }
        if meta != before {
            meta_save(r, &meta)?;
            changed = true;
        }
    }
    if changed {
        s.report_dirty = true;
    }
    Ok(json!({"bookmark": name, "changed": changed}))
}

/// `bookmark.remove`: deletes a bookmark, or a group and its bookmarks.
pub(crate) fn bookmark_remove(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["bookmark"])?;
    let key = str_param(p, "bookmark")?;
    let (_, r) = parts(s)?;
    let mut meta = meta(r)?;
    // a group: its children go with it
    let names: Vec<String> = match find_group(r, key) {
        Ok((group, _display)) => {
            let mut names: Vec<String> = items(&meta)
                .iter()
                .find(|i| i.get("name").and_then(Value::as_str) == Some(group.as_str()))
                .and_then(|i| i.get("children"))
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                .unwrap_or_default();
            if names.is_empty() {
                return Err(format!(
                    "bookmark group `{key}` holds no bookmark; use bookmark.ungroup to remove an empty group"
                ));
            }
            names.push(group);
            names
        }
        Err(_) => vec![
            r.bookmarks
                .get(bookmark_index(r, key)?)
                .map(|b| b.name.clone())
                .ok_or("internal: bookmark index out of range")?,
        ],
    };
    let removed: Vec<String> = names
        .iter()
        .filter(|n| n.as_str() != key && r.bookmarks.iter().any(|b| &b.name == *n))
        .cloned()
        .collect();
    if removed.is_empty() && !names.iter().any(|n| n == key) {
        return Err(format!("the report has no bookmark named `{key}`"));
    }
    r.bookmarks.retain(|b| !names.contains(&b.name));
    remove_items(&mut meta, &names);
    meta_save(r, &meta)?;
    s.report_dirty = true;
    Ok(json!({"removed": removed, "group": names.last()}))
}

// ---------------------------------------------------------------------------------------------
// bookmark.group / bookmark.ungroup
// ---------------------------------------------------------------------------------------------

/// `bookmark.group`: puts bookmarks in a new group, in the order given.
pub(crate) fn bookmark_group(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["bookmarks", "display_name"])?;
    let display_name = str_param(p, "display_name")?;
    check_display_name(display_name)?;
    let list = names_param(p, "bookmarks")?;
    if list.is_empty() {
        return Err("`bookmarks` must name at least one bookmark".into());
    }
    let (_, r) = parts(s)?;
    let mut meta = meta(r)?;
    let mut names = Vec::new();
    for key in &list {
        let i = bookmark_index(r, key)?;
        let name = r
            .bookmarks
            .get(i)
            .map(|b| b.name.clone())
            .ok_or("internal: bookmark index out of range")?;
        if find_group(r, &name).is_ok() {
            return Err(format!("`{key}` is a bookmark group, not a bookmark"));
        }
        if !names.contains(&name) {
            names.push(name);
        }
    }
    let base = crate::report_apply::stable_name(&format!("group/{display_name}"));
    let group = unique_group_name(&meta, &base);
    // The group takes the place of the first listed bookmark.
    let at = names
        .iter()
        .filter_map(|n| {
            items(&meta)
                .iter()
                .position(|i| i.get("name").and_then(Value::as_str) == Some(n.as_str()))
        })
        .min()
        .unwrap_or_else(|| items(&meta).len());
    remove_items(&mut meta, &names);
    let item = json!({"name": group, "displayName": display_name, "children": names});
    insert_item(&mut meta, at, item);
    meta_save(r, &meta)?;
    s.report_dirty = true;
    Ok(json!({"group": group, "display_name": display_name, "bookmarks": names}))
}

/// `bookmark.ungroup`: removes a group, its bookmarks taking its place in the list.
pub(crate) fn bookmark_ungroup(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["group"])?;
    let key = str_param(p, "group")?;
    let (_, r) = parts(s)?;
    let mut meta = meta(r)?;
    let (group, display) = find_group(r, key)?;
    let at = items(&meta)
        .iter()
        .position(|i| i.get("name").and_then(Value::as_str) == Some(group.as_str()))
        .ok_or("internal: bookmark group not listed")?;
    let children: Vec<String> = items(&meta)
        .get(at)
        .and_then(|i| i.get("children"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    remove_group_item(&mut meta, &group);
    for (k, name) in children.iter().enumerate() {
        insert_item(&mut meta, at + k, json!({"name": name}));
    }
    meta_save(r, &meta)?;
    s.report_dirty = true;
    Ok(json!({"group": group, "display_name": display, "bookmarks": children}))
}

/// `bookmark.rename_group`: renames a bookmark group (its bookmarks keep their names).
pub(crate) fn bookmark_rename_group(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["group", "display_name"])?;
    let key = str_param(p, "group")?;
    let display_name = str_param(p, "display_name")?;
    check_group_name(display_name)?;
    let (_, r) = parts(s)?;
    let (group, _) = find_group(r, key)?;
    let mut meta = meta(r)?;
    let mut changed = false;
    if let Some(Value::Array(list)) = meta.get_mut("items") {
        for item in list.iter_mut() {
            if item.get("name").and_then(Value::as_str) != Some(group.as_str()) {
                continue;
            }
            if item.get("displayName").and_then(Value::as_str) == Some(display_name) {
                continue;
            }
            if let Some(o) = item.as_object_mut() {
                o.insert("displayName".into(), json!(display_name));
                changed = true;
            }
        }
    }
    if changed {
        meta_save(r, &meta)?;
        s.report_dirty = true;
    }
    Ok(json!({"group": group, "display_name": display_name, "changed": changed}))
}

// ---------------------------------------------------------------------------------------------
// bookmark.apply
// ---------------------------------------------------------------------------------------------

/// `bookmark.apply`: restores the state a bookmark captured into the report (one undo step).
/// Desktop's "View" shows it without saving; a session's report is its state, so this writes it.
pub(crate) fn bookmark_apply(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["bookmark"])?;
    let key = str_param(p, "bookmark")?;
    let (_, r) = parts(s)?;
    let i = bookmark_index(r, key)?;
    let (name, doc) = r
        .bookmarks
        .get(i)
        .map(|b| (b.name.clone(), b.document.clone()))
        .ok_or("internal: bookmark index out of range")?;
    let out = apply_document(r, &doc)?;
    let changed = out.get("changed").and_then(Value::as_bool) == Some(true);
    if changed {
        s.report_dirty = true;
    }
    // the slicers it restored show the stored selection again, not a click made since
    let mut view_changed = restore_highlights(s, &doc);
    if let Some(list) = out.get("slicers").and_then(Value::as_array) {
        for x in list {
            if let (Some(pg), Some(v)) = (x["page"].as_str(), x["visual"].as_str()) {
                view_changed |= s.interactions.forget_slicer(pg, v);
            }
        }
    }
    let slicers: Vec<Value> = out
        .get("slicers")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .map(|x| json!({"page": x["page"], "visual": x["visual"]}))
                .collect()
        })
        .unwrap_or_default();
    Ok(json!({
        "bookmark": name,
        "changed": changed,
        "active_page": out.get("active_page").cloned().unwrap_or(Value::Null),
        "page_changed": out.get("page_changed").cloned().unwrap_or(Value::Bool(false)),
        "hidden": out.get("hidden").cloned().unwrap_or(Value::Array(Vec::new())),
        "filters_changed": out.get("filters_changed").cloned().unwrap_or(Value::Bool(false)),
        "slicers": slicers,
        "view_changed": view_changed,
        "warnings": out.get("warnings").cloned().unwrap_or(Value::Array(Vec::new())),
    }))
}

/// The data-point selections a bookmark document holds, restored in the session's view (its
/// options' Data part, the pages it has sections for). True when the view changed.
fn restore_highlights(s: &mut Session, doc: &Value) -> bool {
    let opts = doc.get("options").cloned().unwrap_or(Value::Null);
    if opts.get("suppressData").and_then(Value::as_bool) == Some(true) {
        return false;
    }
    let targets: Option<Vec<String>> =
        (opts.get("applyOnlyToTargetVisuals").and_then(Value::as_bool) == Some(true)).then(|| {
            opts.get("targetVisualNames")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .take(MAX_TARGETS)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default()
        });
    let Some(sections) = doc
        .pointer("/explorationState/sections")
        .and_then(Value::as_object)
    else {
        return false;
    };
    let Some(r) = s.report.as_ref() else {
        return false;
    };
    let mut changed = false;
    for (name, sec) in sections.iter().take(MAX_BOOKMARKS) {
        let Some(page) = r.pages.iter().find(|p| &p.name == name) else {
            continue;
        };
        let containers = sec.get("visualContainers").and_then(Value::as_object);
        let before = s.interactions.state_json();
        crate::interact::highlight::restore(
            &mut s.interactions,
            page,
            containers,
            page.visuals
                .iter()
                .filter(|v| targets.as_ref().is_none_or(|t| t.contains(&v.name))),
        );
        crate::interact::drill::restore(
            &mut s.interactions,
            s.model.as_ref(),
            page,
            containers,
            page.visuals
                .iter()
                .filter(|v| targets.as_ref().is_none_or(|t| t.contains(&v.name))),
        );
        changed |= s.interactions.state_json() != before;
    }
    changed
}

/// Applies a bookmark document to `r`: the page, the visuals' display state and the filters its
/// options allow. Returns the report of what changed.
pub(crate) fn apply_document(r: &mut Report, doc: &Value) -> Result<Value, String> {
    let state = doc
        .get("explorationState")
        .ok_or("the bookmark holds no explorationState: nothing to apply")?;
    let opts = doc.get("options").cloned().unwrap_or(Value::Null);
    let flag = |k: &str| opts.get(k).and_then(Value::as_bool) == Some(true);
    let targets: Option<Vec<String>> = if flag("applyOnlyToTargetVisuals") {
        Some(
            opts.get("targetVisualNames")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .take(MAX_TARGETS)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
        )
    } else {
        None
    };
    let mut warnings: Vec<String> = Vec::new();
    let mut hidden: Vec<Value> = Vec::new();
    let mut page_changed = false;
    // the page
    let mut active_page = Value::Null;
    if !flag("suppressActiveSection")
        && let Some(want) = state.get("activeSection").and_then(Value::as_str)
    {
        match page_index(r, want) {
            Ok(i) => {
                let name = r.pages.get(i).map(|p| p.name.clone()).unwrap_or_default();
                active_page = json!(name);
                if r.active_page.as_deref() != Some(name.as_str()) {
                    r.active_page = Some(name);
                    page_changed = true;
                }
            }
            Err(_) => warnings.push(format!("the bookmark's page `{want}` no longer exists")),
        }
    }
    // pages: display state and filters
    let mut filters_changed = false;
    let mut slicers: Vec<Value> = Vec::new();
    if let Some(sections) = state.get("sections").and_then(Value::as_object) {
        for (page_name, sec) in sections {
            let Some(page) = r.pages.iter_mut().find(|p| &p.name == page_name) else {
                warnings.push(format!("the bookmark's page `{page_name}` no longer exists"));
                continue;
            };
            if !flag("suppressDisplay")
                && let Some(containers) = sec.get("visualContainers").and_then(Value::as_object)
            {
                for v in &mut page.visuals {
                    if targets.as_ref().is_some_and(|t| !t.contains(&v.name)) {
                        continue;
                    }
                    let shown_hidden = containers.get(&v.name).and_then(display_mode) == Some("hidden");
                    if v.is_hidden != shown_hidden {
                        v.is_hidden = shown_hidden;
                        hidden.push(json!({"page": page_name, "visual": v.name, "hidden": shown_hidden}));
                    }
                }
            }
            if !flag("suppressData") {
                if let Some(f) = sec.get("filters")
                    && restore_filters(&mut page.filter_config, f)
                {
                    filters_changed = true;
                }
                if let Some(containers) = sec.get("visualContainers").and_then(Value::as_object) {
                    for v in &mut page.visuals {
                        if targets.as_ref().is_some_and(|t| !t.contains(&v.name)) {
                            continue;
                        }
                        let Some(c) = containers.get(&v.name) else {
                            continue;
                        };
                        if let Some(f) = c.get("filters")
                            && restore_filters(&mut v.filter_config, f)
                        {
                            filters_changed = true;
                        }
                        if crate::interact::bookmark::holds_slicer(v, c) {
                            let changed = crate::interact::bookmark::restore_slicer(v, c);
                            slicers.push(json!({"page": page_name, "visual": v.name, "changed": changed}));
                        }
                    }
                }
            }
        }
    }
    if !flag("suppressData")
        && let Some(f) = state.get("filters")
        && restore_filters(&mut r.filter_config, f)
    {
        filters_changed = true;
    }
    let slicers_changed = slicers.iter().any(|x| x["changed"] == json!(true));
    let changed = page_changed || !hidden.is_empty() || filters_changed || slicers_changed;
    Ok(json!({
        "changed": changed,
        "active_page": active_page,
        "page_changed": page_changed,
        "hidden": hidden,
        "filters_changed": filters_changed,
        "slicers": slicers,
        "warnings": warnings,
    }))
}

// ---------------------------------------------------------------------------------------------
// State capture
// ---------------------------------------------------------------------------------------------

/// The report as the session shows it: the stored one under the reader's interaction state
/// (the slicer selections clicked), `None` when that state changes nothing.
fn shown_report(s: &Session) -> Option<Report> {
    let r = s.report.as_ref()?;
    s.interactions.effective(r).map(|e| e.data)
}

/// The `explorationState` of the report now: its page, its hidden and spotlit visuals, the
/// reader's sorts, its filters and its slicers' selections as `shown` shows them (only the
/// visuals of `targets` when given).
fn state_of(
    r: &Report,
    view: &Interactions,
    shown: Option<&Report>,
    page: &str,
    targets: Option<&[String]>,
) -> Value {
    let mut sections = Map::new();
    for pg in &r.pages {
        let mut containers = Map::new();
        let shown_page = shown.and_then(|s| s.pages.iter().find(|p| p.name == pg.name));
        for v in &pg.visuals {
            if targets.is_some_and(|t| !t.contains(&v.name)) {
                continue;
            }
            let mut st = Map::new();
            let seen = shown_page
                .and_then(|p| p.visuals.iter().find(|x| x.name == v.name))
                .unwrap_or(v);
            // a slicer's selection, then the display mode and the reader's sort
            let mut single = match crate::interact::bookmark::capture_slicer(seen) {
                Some(Value::Object(m)) => m,
                _ => Map::new(),
            };
            if v.is_hidden {
                single.insert("display".into(), json!({"mode": "hidden"}));
            } else if view.spotlight() == Some((pg.name.as_str(), v.name.as_str())) {
                single.insert("display".into(), json!({"mode": "spotlight"}));
            }
            if let Some(o) = view
                .reader_sort(&pg.name, &v.name)
                .and_then(crate::visual_props::order_by_json)
            {
                single.insert("orderBy".into(), o);
            }
            // the reader's drill: the levels shown, a matrix's expansion, the values drilled into
            let drill = crate::interact::drill::capture(view, &pg.name, v);
            if let Some(d) = &drill {
                single.insert("activeProjections".into(), d.active.clone());
                if let Some(e) = &d.expansion {
                    single.insert("expansionStates".into(), e.clone());
                }
                // the drill mode: a formatting-object update of BIcraft's own (interact_drill.rs)
                if d.mode
                    && let Some(merge) = single
                        .entry("objects")
                        .or_insert_with(|| json!({}))
                        .as_object_mut()
                        .map(|o| o.entry("merge").or_insert_with(|| json!({})))
                        .and_then(Value::as_object_mut)
                {
                    merge.insert(
                        crate::interact::drill::DRILL_MODE_OBJECT.into(),
                        crate::interact::drill::mode_object(),
                    );
                }
            }
            if !single.is_empty() {
                st.insert("singleVisual".into(), Value::Object(single));
            }
            // the data points selected in it (interact_highlight.rs)
            if let Some(h) = crate::interact::highlight::capture(view, &pg.name, &v.name) {
                st.insert("highlight".into(), h);
            }
            if let Some(f) = v
                .filter_config
                .as_ref()
                .and_then(|c| c.get("filters"))
                .and_then(Value::as_array)
            {
                st.insert("filters".into(), by_name(f));
            }
            if let Some(d) = drill.filter(|d| !d.filters.is_empty()) {
                let f = st.entry("filters").or_insert_with(|| json!({}));
                if let Some(o) = f.as_object_mut() {
                    o.insert("byExpr".into(), Value::Array(d.filters));
                }
            }
            if !st.is_empty() {
                containers.insert(v.name.clone(), Value::Object(st));
            }
        }
        let mut sec = Map::new();
        sec.insert("visualContainers".into(), Value::Object(containers));
        if let Some(f) = pg
            .filter_config
            .as_ref()
            .and_then(|c| c.get("filters"))
            .and_then(Value::as_array)
        {
            sec.insert("filters".into(), by_name(f));
        }
        sections.insert(pg.name.clone(), Value::Object(sec));
    }
    let mut expl = Map::new();
    expl.insert("version".into(), json!(STATE_VERSION));
    expl.insert("activeSection".into(), json!(page));
    if let Some(f) = r
        .filter_config
        .as_ref()
        .and_then(|c| c.get("filters"))
        .and_then(Value::as_array)
    {
        expl.insert("filters".into(), by_name(f));
    }
    expl.insert("sections".into(), Value::Object(sections));
    Value::Object(expl)
}

/// Bookmark filters of a container: `{"byName": {name: {name, filter}}}`.
fn by_name(filters: &[Value]) -> Value {
    let mut by = Map::new();
    for (i, f) in filters.iter().enumerate() {
        let name = f
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("filter{}", i + 1));
        let mut entry = Map::new();
        entry.insert("name".into(), json!(name));
        if let Some(flt) = f.get("filter") {
            entry.insert("filter".into(), flt.clone());
        }
        by.insert(name, Value::Object(entry));
    }
    json!({"byName": by})
}

/// Restores a container's filters from a bookmark's `FiltersState`. True when they changed.
pub(crate) fn restore_filters(cfg: &mut Option<Value>, state: &Value) -> bool {
    let Some(by) = state.get("byName").and_then(Value::as_object) else {
        return false;
    };
    let existing: Vec<Value> = cfg
        .as_ref()
        .and_then(|c| c.get("filters"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut filters: Vec<Value> = Vec::new();
    for (name, entry) in by {
        let mut item = existing
            .iter()
            .find(|f| f.get("name").and_then(Value::as_str) == Some(name.as_str()))
            .cloned()
            .unwrap_or_else(|| json!({"name": name}));
        if let Some(o) = item.as_object_mut() {
            match entry.get("filter") {
                Some(f) => {
                    o.insert("filter".into(), f.clone());
                }
                None => {
                    o.remove("filter");
                }
            }
        }
        filters.push(item);
    }
    let mut next = if filters.is_empty() {
        None
    } else {
        Some(json!({"filters": filters}))
    };
    // a filterConfig may carry more than its filters: keep it
    if let (Some(n), Some(c)) = (next.as_mut(), cfg.as_ref())
        && let (Some(no), Some(co)) = (n.as_object_mut(), c.as_object())
    {
        for (k, v) in co {
            if k != "filters" {
                no.insert(k.clone(), v.clone());
            }
        }
    }
    if *cfg != next {
        *cfg = next;
        true
    } else {
        false
    }
}

/// The `options` object a capture or an option change writes.
fn options_json(data: bool, display: bool, current_page: bool, targets: Option<&[String]>) -> Value {
    let mut o = Map::new();
    o.insert("suppressData".into(), json!(!data));
    o.insert("suppressDisplay".into(), json!(!display));
    o.insert("suppressActiveSection".into(), json!(!current_page));
    if let Some(list) = targets {
        o.insert("applyOnlyToTargetVisuals".into(), json!(true));
        o.insert(
            "targetVisualNames".into(),
            Value::Array(list.iter().take(MAX_TARGETS).map(|n| json!(n)).collect()),
        );
    }
    Value::Object(o)
}

/// The visuals a bookmark applies to, when it captured some.
fn bookmark_targets(r: &Report, i: usize) -> Option<Vec<String>> {
    let doc = r.bookmarks.get(i)?.document.as_object()?;
    let opts = doc.get("options")?;
    if opts.get("applyOnlyToTargetVisuals").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    Some(
        opts.get("targetVisualNames")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .take(MAX_TARGETS)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
    )
}

// ---------------------------------------------------------------------------------------------
// bookmarks.json and names
// ---------------------------------------------------------------------------------------------

/// The parsed `bookmarks.json` (an empty document when the report has none).
fn meta(r: &Report) -> Result<Value, String> {
    match r.resources.iter().find(|x| x.path == META) {
        None => Ok(json!({"$schema": META_SCHEMA, "items": []})),
        Some(res) => match serde_json::from_slice::<Value>(&res.bytes) {
            Ok(v) if v.is_object() => Ok(v),
            Ok(_) => Err(format!("{META} is not a JSON object")),
            Err(e) => Err(format!("{META} cannot be read: {e}")),
        },
    }
}

fn meta_save(r: &mut Report, doc: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(doc).map_err(|e| format!("{META} cannot be written: {e}"))?;
    match r.resources.iter_mut().find(|x| x.path == META) {
        Some(res) => res.bytes = bytes,
        None => r.resources.push(Resource {
            path: META.to_string(),
            bytes,
        }),
    }
    Ok(())
}

fn items(doc: &Value) -> &[Value] {
    match doc.get("items") {
        Some(Value::Array(a)) => a.as_slice(),
        _ => &[],
    }
}

fn insert_item(doc: &mut Value, at: usize, item: Value) {
    let Some(map) = doc.as_object_mut() else {
        return;
    };
    if !map.get("items").is_some_and(Value::is_array) {
        map.insert("items".into(), Value::Array(Vec::new()));
    }
    if let Some(Value::Array(a)) = map.get_mut("items") {
        let at = at.min(a.len());
        a.insert(at, item);
    }
}

/// Removes the items named in `names` (a group goes with its bookmarks).
fn remove_items(doc: &mut Value, names: &[String]) {
    let Some(map) = doc.as_object_mut() else {
        return;
    };
    let Some(Value::Array(list)) = map.get_mut("items") else {
        return;
    };
    list.retain_mut(|item| {
        if let Some(children) = item.get_mut("children").and_then(Value::as_array_mut) {
            children.retain(|c| c.as_str().is_some_and(|n| !names.iter().any(|x| x == n)));
            // keeping a group whose bookmarks all went would leave it empty
            if children.is_empty() {
                let group_gone = item
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|n| names.iter().any(|x| x == n));
                return !group_gone;
            }
            true
        } else {
            !item
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|n| names.iter().any(|x| x == n))
        }
    });
}

/// Removes the group item named `group` from the list, keeping its bookmarks in it.
fn remove_group_item(doc: &mut Value, group: &str) {
    let Some(map) = doc.as_object_mut() else {
        return;
    };
    let Some(Value::Array(list)) = map.get_mut("items") else {
        return;
    };
    list.retain(|item| {
        !(item.get("children").is_some() && item.get("name").and_then(Value::as_str) == Some(group))
    });
}

fn group_display(item: &Value) -> Option<String> {
    item.get("displayName")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| item.get("name").and_then(Value::as_str).map(str::to_string))
}

/// A group by name or display name: `(name, display name)`.
fn find_group(r: &Report, key: &str) -> Result<(String, String), String> {
    let meta = meta(r)?;
    for item in items(&meta) {
        if item.get("children").and_then(Value::as_array).is_none() {
            continue;
        }
        let Some(name) = item.get("name").and_then(Value::as_str) else {
            continue;
        };
        let display = group_display(item).unwrap_or_else(|| name.to_string());
        if name == key || name.eq_ignore_ascii_case(key) || display.eq_ignore_ascii_case(key) {
            return Ok((name.to_string(), display));
        }
    }
    Err(format!(
        "the report has no bookmark group named `{key}` (bookmark.list lists them)"
    ))
}

/// Puts bookmark `name` at the end of the list, or in group `group`.
fn place_item(doc: &mut Value, name: &str, group: Option<&str>) -> Result<(), String> {
    match group {
        None => {
            let at = items(doc).len();
            insert_item(doc, at, json!({"name": name}));
            Ok(())
        }
        Some(key) => {
            let Some(map) = doc.as_object_mut() else {
                return Err("bookmarks.json is not a JSON object".into());
            };
            let Some(Value::Array(list)) = map.get_mut("items") else {
                return Err("bookmarks.json has no items".into());
            };
            for item in list.iter_mut() {
                if item.get("children").and_then(Value::as_array).is_none() {
                    continue;
                }
                let Some(gname) = item.get("name").and_then(Value::as_str) else {
                    continue;
                };
                let display = group_display(item).unwrap_or_else(|| gname.to_string());
                if (gname == key || gname.eq_ignore_ascii_case(key) || display.eq_ignore_ascii_case(key))
                    && let Some(children) = item.get_mut("children").and_then(Value::as_array_mut)
                {
                    children.push(json!(name));
                    return Ok(());
                }
            }
            Err(format!(
                "the report has no bookmark group named `{key}` (bookmark.list lists them)"
            ))
        }
    }
}

/// Moves bookmark `name` into group `group` (`None`: to the end of the list).
fn move_to_group(doc: &mut Value, name: &str, group: Option<&str>) -> Result<(), String> {
    remove_items(doc, &[name.to_string()]);
    place_item(doc, name, group)
}

/// Moves bookmark `name` to position `at` of its container: its group's children, or the list.
fn move_to_index(doc: &mut Value, name: &str, at: usize) {
    let holder = items(doc)
        .iter()
        .find(|item| {
            item.get("children")
                .and_then(Value::as_array)
                .is_some_and(|c| c.iter().any(|x| x.as_str() == Some(name)))
        })
        .and_then(|item| item.get("name").and_then(Value::as_str))
        .map(str::to_string);
    match holder {
        Some(group) => move_child(doc, &group, name, at),
        None => {
            remove_items(doc, &[name.to_string()]);
            let at = at.min(items(doc).len());
            insert_item(doc, at, json!({"name": name}));
        }
    }
}

/// Moves bookmark `name` to position `at` among the children of `group`.
fn move_child(doc: &mut Value, group: &str, name: &str, at: usize) {
    let Some(map) = doc.as_object_mut() else {
        return;
    };
    let Some(Value::Array(list)) = map.get_mut("items") else {
        return;
    };
    for item in list.iter_mut() {
        if item.get("name").and_then(Value::as_str) != Some(group) {
            continue;
        }
        if let Some(Value::Array(children)) = item.get_mut("children") {
            children.retain(|c| c.as_str() != Some(name));
            let at = at.min(children.len());
            children.insert(at, json!(name));
        }
        return;
    }
}

pub(crate) fn bookmark_name(b: &NamedDocument) -> String {
    b.document
        .get("displayName")
        .and_then(Value::as_str)
        .filter(|d| !d.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| b.name.clone())
}

pub(crate) fn bookmark_index(r: &Report, key: &str) -> Result<usize, String> {
    r.bookmarks
        .iter()
        .position(|b| b.name == key || b.name.eq_ignore_ascii_case(key))
        .or_else(|| {
            r.bookmarks
                .iter()
                .position(|b| bookmark_name(b).eq_ignore_ascii_case(key))
        })
        .ok_or_else(|| format!("the report has no bookmark named `{key}` (bookmark.list lists them)"))
}

fn unique_name(r: &Report, base: &str) -> String {
    let taken = |n: &str| r.bookmarks.iter().any(|b| b.name.eq_ignore_ascii_case(n));
    if !taken(base) {
        return base.to_string();
    }
    for k in 2..=MAX_BOOKMARKS {
        let n = format!("{base}{k}");
        if !taken(&n) {
            return n;
        }
    }
    format!("{base}{}", r.bookmarks.len().saturating_add(1))
}

fn unique_group_name(doc: &Value, base: &str) -> String {
    let taken = |n: &str| {
        items(doc).iter().any(|i| {
            i.get("name")
                .and_then(Value::as_str)
                .is_some_and(|x| x.eq_ignore_ascii_case(n))
        })
    };
    if !taken(base) {
        return base.to_string();
    }
    for k in 2..=MAX_BOOKMARKS {
        let n = format!("{base}{k}");
        if !taken(&n) {
            return n;
        }
    }
    format!("{base}{}", items(doc).len().saturating_add(1))
}

fn check_display_name(name: &str) -> Result<(), String> {
    crate::edit::check_name("bookmark", name)?;
    if name.chars().count() > MAX_NAME {
        return Err(format!("a bookmark name is longer than {MAX_NAME} characters"));
    }
    Ok(())
}

fn check_group_name(name: &str) -> Result<(), String> {
    crate::edit::check_name("bookmark group", name)?;
    if name.chars().count() > MAX_NAME {
        return Err(format!(
            "a bookmark group name is longer than {MAX_NAME} characters"
        ));
    }
    Ok(())
}

/// The page a capture starts from: the active one, else the first.
pub(crate) fn current_page_name(r: &Report) -> Result<String, String> {
    r.active_page
        .clone()
        .filter(|n| r.pages.iter().any(|p| &p.name == n))
        .or_else(|| r.pages.first().map(|p| p.name.clone()))
        .ok_or_else(|| "the report has no page to capture".to_string())
}

/// `visuals`: the visuals of the active page a bookmark applies to.
fn targets_param(p: &Value) -> Result<Option<Vec<String>>, String> {
    let Some(v) = p.get("visuals") else {
        return Ok(None);
    };
    if v.is_null() {
        return Ok(None);
    }
    let Some(a) = v.as_array() else {
        return Err("parameter `visuals` must be a list of visual names".into());
    };
    if a.is_empty() {
        return Ok(None);
    }
    if a.len() > MAX_TARGETS {
        return Err(format!("`visuals` lists more than {MAX_TARGETS} visuals"));
    }
    let mut out = Vec::new();
    for x in a {
        let Some(n) = x.as_str() else {
            return Err("parameter `visuals` must be a list of visual names".into());
        };
        if !out.contains(&n.to_string()) {
            out.push(n.to_string());
        }
    }
    Ok(Some(out))
}

fn check_targets(r: &Report, page: &str, targets: &[String]) -> Result<(), String> {
    let Some(pg) = r.pages.iter().find(|p| p.name == page) else {
        return Err("the report has no page to capture".into());
    };
    for t in targets {
        if !pg.visuals.iter().any(|v| &v.name == t) {
            return Err(format!(
                "page `{}` has no visual named `{t}` (report.summary lists them)",
                pg.display_name
            ));
        }
    }
    Ok(())
}

/// `names`: a non-empty list of strings.
fn names_param(p: &Value, key: &str) -> Result<Vec<String>, String> {
    let Some(a) = p.get(key).and_then(Value::as_array) else {
        return Err(format!("missing list parameter `{key}`"));
    };
    if a.len() > MAX_BOOKMARKS {
        return Err(format!("`{key}` lists more than {MAX_BOOKMARKS} names"));
    }
    let mut out = Vec::new();
    for x in a {
        let Some(n) = x.as_str() else {
            return Err(format!("`{key}` must be a list of names"));
        };
        out.push(n.to_string());
    }
    Ok(out)
}

/// How a bookmark's visual container shows its visual: `singleVisual.display.mode` (the PBIR
/// schema's place), else a container-level `display.mode`.
pub(crate) fn display_mode(container: &Value) -> Option<&str> {
    container
        .pointer("/singleVisual/display/mode")
        .or_else(|| container.pointer("/display/mode"))
        .and_then(Value::as_str)
}

#[cfg(test)]
#[path = "bookmarks_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "bookmarks_slicer_tests.rs"]
mod slicer_tests;
