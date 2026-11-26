//! Synchronized slicers (Desktop's "Sync slicers" pane): a slicer carries a `syncGroup` object in
//! its `visual` (`{"groupName": "...", "fieldChanges": true, "filterChanges": true}`), kept in
//! [`bicraft_report::Visual::visual_extra`] like every unmodelled property, and written back by
//! the PBIR writer (`VISUAL_CONFIG_ORDER`). Slicers of different pages that share a group name
//! filter together, as Desktop's pane decides with its checkboxes.
//!
//! Commands: `slicer.sync_list` (the slicers of the report and their groups) and `slicer.set_sync`
//! (puts slicers in a group, with the fields / filters options, or takes them out).
//!
//! What a group does:
//! - **Filters** (`filterChanges`): the reader's click on a slicer of the group reaches every
//!   slicer of the group, on every page (`interact.rs`, interaction oracle rule 6). Joining a
//!   group reconciles nothing: each slicer keeps its own selection until the next click in the
//!   group, as Desktop leaves the files' selections when a report opens (rule 6).
//! - **Fields** (`fieldChanges`): a field bound, removed or moved on a slicer of the group
//!   (`visual.bind`, `visual.unbind`, `visual.move_field`, `visual.bind_hierarchy`) is given to
//!   every slicer of the group whose Fields option is on, in the same command (one undo step):
//!   [`visual_bind`] and the other wrappers below.

use bicraft_report::Visual;
use serde_json::{Value, json};

use crate::Session;
use crate::params::{allow, opt_bool};
use crate::report::{page_index, parts, visual_index};

/// Most slicers one command changes, and the report lists.
const MAX_SEGMENTS: usize = 1_000;
/// Longest sync group name, in characters.
const MAX_GROUP_NAME: usize = 100;

/// A slicer's sync group: its name and the two options Desktop's pane shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SyncGroup {
    pub name: String,
    pub field_changes: bool,
    pub filter_changes: bool,
}

/// The sync group of `v`, when it is a slicer that has one.
fn sync_of(v: &Visual) -> Option<SyncGroup> {
    let g = v.visual_extra.get("syncGroup")?;
    let name = g.get("groupName").and_then(Value::as_str)?.trim();
    if name.is_empty() {
        return None;
    }
    Some(SyncGroup {
        name: name.to_string(),
        field_changes: g.get("fieldChanges").and_then(Value::as_bool).unwrap_or(true),
        filter_changes: g.get("filterChanges").and_then(Value::as_bool).unwrap_or(true),
    })
}

/// Whether `v` is a slicer.
fn is_slicer(v: &Visual) -> bool {
    v.visual_type.as_deref() == Some("slicer")
}

/// The snapshot's view of a slicer's sync group (`null` when it has none).
pub(crate) fn sync_group_json(v: &Visual) -> Value {
    match sync_of(v) {
        Some(g) => json!({
            "name": g.name,
            "field_changes": g.field_changes,
            "filter_changes": g.filter_changes,
        }),
        None => Value::Null,
    }
}

// ---------------------------------------------------------------------------------------------
// slicer.sync_list
// ---------------------------------------------------------------------------------------------

/// `slicer.sync_list`: every slicer of the report, and the groups they form.
pub(crate) fn slicer_sync_list(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &[])?;
    Ok(sync_view(s))
}

/// `slicer.sync_list` on a shared session (`shared.rs`): it reads, never writes.
pub(crate) fn slicer_sync_list_shared(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &[])?;
    Ok(sync_view(s))
}

/// The slicers and sync groups of the session's report, for `slicer.sync_list` and the snapshot's
/// pane. Never fails.
pub(crate) fn sync_view(s: &Session) -> Value {
    let Some(r) = s.report.as_ref() else {
        return json!({"segments": [], "groups": [], "count": 0});
    };
    let mut segments: Vec<Value> = Vec::new();
    let mut groups: Vec<(String, SyncGroup, Vec<Value>)> = Vec::new();
    let mut total = 0_usize;
    for page in &r.pages {
        for v in &page.visuals {
            if !is_slicer(v) {
                continue;
            }
            total += 1;
            if segments.len() >= MAX_SEGMENTS {
                continue;
            }
            let sync = sync_of(v);
            segments.push(json!({
                "page": page.name,
                "page_display": page.display_name,
                "visual": v.name,
                "label": crate::snapshot::visual_title(v).unwrap_or_else(|| "slicer".to_string()),
                "group": sync.as_ref().map(|g| json!(g.name)).unwrap_or(Value::Null),
                "field_changes": sync.as_ref().is_none_or(|g| g.field_changes),
                "filter_changes": sync.as_ref().is_none_or(|g| g.filter_changes),
            }));
            let Some(g) = sync else { continue };
            let at = groups
                .iter()
                .position(|(name, _, _)| name.eq_ignore_ascii_case(&g.name));
            let entry = json!({"page": page.name, "visual": v.name});
            match at {
                Some(i) => {
                    if let Some((_, _, members)) = groups.get_mut(i) {
                        members.push(entry);
                    }
                }
                None => groups.push((g.name.clone(), g, vec![entry])),
            }
        }
    }
    let groups: Vec<Value> = groups
        .iter()
        .map(|(name, g, members)| {
            json!({
                "name": name,
                "field_changes": g.field_changes,
                "filter_changes": g.filter_changes,
                "segments": members,
            })
        })
        .collect();
    json!({"segments": segments, "groups": groups, "count": total})
}

// ---------------------------------------------------------------------------------------------
// slicer.set_sync
// ---------------------------------------------------------------------------------------------

/// `slicer.set_sync`: puts slicers in a sync group, or takes them out (`group: null`).
pub(crate) fn slicer_set_sync(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "page",
            "visual",
            "segments",
            "group",
            "field_changes",
            "filter_changes",
        ],
    )?;
    let group = match p.get("group") {
        Some(Value::String(g)) => {
            check_group_name(g)?;
            Some(g.trim().to_string())
        }
        Some(Value::Null) => None,
        Some(_) => return Err("parameter `group` must be a group name or null".into()),
        None => return Err("missing parameter `group`".into()),
    };
    let field_changes = opt_bool(p, "field_changes")?.unwrap_or(true);
    let filter_changes = opt_bool(p, "filter_changes")?.unwrap_or(true);
    let keys = targets(p)?;
    let mut out = Vec::new();
    let mut changed = false;
    // a group that exists under another case keeps its spelling: the pane lists groups without
    // case, and the reader's propagation compares names as written
    let group = group.map(|g| {
        s.report
            .iter()
            .flat_map(|r| r.pages.iter())
            .flat_map(|pg| pg.visuals.iter())
            .filter(|v| is_slicer(v))
            .filter_map(sync_of)
            .find(|x| x.name.eq_ignore_ascii_case(&g))
            .map_or(g, |x| x.name)
    });
    {
        let (_, r) = parts(s)?;
        for (page_key, visual_key) in &keys {
            let pi = page_index(r, page_key)?;
            let page = r.pages.get_mut(pi).ok_or("internal: page index out of range")?;
            let vi = visual_index(page, visual_key)?;
            let page_name = page.name.clone();
            let visual = page
                .visuals
                .get_mut(vi)
                .ok_or("internal: visual index out of range")?;
            if !is_slicer(visual) {
                return Err(format!(
                    "visual `{visual_key}` of page `{}` is not a slicer: only slicers synchronize",
                    page.display_name
                ));
            }
            let before = visual.visual_extra.get("syncGroup").cloned();
            let next = group.as_ref().map(|g| {
                json!({
                    "groupName": g,
                    "fieldChanges": field_changes,
                    "filterChanges": filter_changes,
                })
            });
            if group.is_none() {
                visual.visual_extra.remove("syncGroup");
            } else if let Some(next) = next {
                visual.visual_extra.insert("syncGroup".into(), next);
            }
            let after = visual.visual_extra.get("syncGroup").cloned();
            if before != after {
                changed = true;
            }
            out.push(json!({"page": page_name, "visual": visual.name}));
        }
    }
    if changed {
        s.report_dirty = true;
    }
    Ok(json!({
        "segments": out,
        "group": group,
        "field_changes": field_changes,
        "filter_changes": filter_changes,
        "changed": changed,
    }))
}

// ---------------------------------------------------------------------------------------------
// Field changes (`fieldChanges`)
// ---------------------------------------------------------------------------------------------

/// `visual.bind`, then the slicer's group follows its fields ([`follow_fields`]).
pub(crate) fn visual_bind(s: &mut Session, p: &Value) -> Result<Value, String> {
    let out = crate::report::visual_bind(s, p)?;
    Ok(follow_fields(s, p, out))
}

/// `visual.unbind`, then the slicer's group follows its fields.
pub(crate) fn visual_unbind(s: &mut Session, p: &Value) -> Result<Value, String> {
    let out = crate::report::visual_unbind(s, p)?;
    Ok(follow_fields(s, p, out))
}

/// `visual.move_field`, then the slicer's group follows its fields.
pub(crate) fn visual_move_field(s: &mut Session, p: &Value) -> Result<Value, String> {
    let out = crate::visual_fields::visual_move_field(s, p)?;
    Ok(follow_fields(s, p, out))
}

/// `visual.bind_hierarchy`, then the slicer's group follows its fields.
pub(crate) fn visual_bind_hierarchy(s: &mut Session, p: &Value) -> Result<Value, String> {
    let out = crate::visual_fields::visual_bind_hierarchy(s, p)?;
    Ok(follow_fields(s, p, out))
}

/// Desktop's "Sync field changes to other slicers": once a command changed the fields of the
/// visual `p.page` / `p.visual`, when it is a slicer of a group with Fields on, every other
/// slicer of that group (by name, without case) whose Fields option is on takes the same fields.
/// Their selections are left as they are. The answer gains `synced`, the slicers changed.
/// Never fails: the command itself already succeeded.
fn follow_fields(s: &mut Session, p: &Value, mut out: Value) -> Value {
    let (Some(page_key), Some(visual_key)) = (
        p.get("page").and_then(Value::as_str),
        p.get("visual").and_then(Value::as_str),
    ) else {
        return out;
    };
    let Some(r) = s.report.as_mut() else {
        return out;
    };
    let Some(page) = page_index(r, page_key).ok().and_then(|i| r.pages.get(i)) else {
        return out;
    };
    let Some(src) = visual_index(page, visual_key)
        .ok()
        .and_then(|i| page.visuals.get(i))
    else {
        return out;
    };
    let Some(group) = sync_of(src).filter(|g| g.field_changes && is_slicer(src)) else {
        return out;
    };
    let (src_page, src_visual, fields) = (page.name.clone(), src.name.clone(), src.fields.clone());
    let mut synced = Vec::new();
    for page in &mut r.pages {
        for v in &mut page.visuals {
            if (page.name == src_page && v.name == src_visual) || !is_slicer(v) {
                continue;
            }
            let follows =
                sync_of(v).is_some_and(|g| g.field_changes && g.name.eq_ignore_ascii_case(&group.name));
            if follows && v.fields != fields {
                v.fields = fields.clone();
                synced.push(json!({"page": page.name, "visual": v.name}));
            }
        }
    }
    if !synced.is_empty() {
        s.report_dirty = true;
    }
    if let Some(o) = out.as_object_mut() {
        o.insert("synced".into(), Value::Array(synced));
    }
    out
}

/// The `(page, visual)` pairs to change: `segments`, or `page` and `visual`.
fn targets(p: &Value) -> Result<Vec<(String, String)>, String> {
    if let Some(list) = p.get("segments") {
        let Some(a) = list.as_array() else {
            return Err("parameter `segments` must be a list of {page, visual}".into());
        };
        if a.is_empty() {
            return Err("`segments` must name at least one slicer".into());
        }
        if a.len() > MAX_SEGMENTS {
            return Err(format!("`segments` lists more than {MAX_SEGMENTS} slicers"));
        }
        let mut out = Vec::new();
        for item in a {
            let Some(o) = item.as_object() else {
                return Err("parameter `segments` must be a list of {page, visual}".into());
            };
            let page = o
                .get("page")
                .and_then(Value::as_str)
                .ok_or("each `segments` entry needs a `page`")?;
            let visual = o
                .get("visual")
                .and_then(Value::as_str)
                .ok_or("each `segments` entry needs a `visual`")?;
            let pair = (page.to_string(), visual.to_string());
            if !out.contains(&pair) {
                out.push(pair);
            }
        }
        return Ok(out);
    }
    let page = p
        .get("page")
        .and_then(Value::as_str)
        .ok_or("missing parameter `page`")?;
    let visual = p
        .get("visual")
        .and_then(Value::as_str)
        .ok_or("missing parameter `visual`")?;
    Ok(vec![(page.to_string(), visual.to_string())])
}

fn check_group_name(name: &str) -> Result<(), String> {
    crate::edit::check_name("sync group", name.trim())?;
    if name.trim().chars().count() > MAX_GROUP_NAME {
        return Err(format!(
            "a sync group name is longer than {MAX_GROUP_NAME} characters"
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "slicer_sync_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "slicer_sync_view_tests.rs"]
mod view_tests;
