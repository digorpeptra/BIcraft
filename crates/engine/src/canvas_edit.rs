//! Canvas editing (ROADMAP M11.16, the app's direct manipulation of visuals): copy, paste and
//! duplicate visuals, move and resize several visuals in one step, align and distribute them,
//! group and ungroup them. Each gesture of the app is one command, so one undo step.
//!
//! **The clipboard document** (`visual.copy`'s answer, `visual.paste {clip}`'s input) is
//! `{bicraft: "visuals", version: 1, page: {name, display_name, width, height}, visuals: [...]}`,
//! where each visual is the report crate's visual record (`bicraft_report::Visual`: name,
//! position, type, fields, group, hidden, filters and the unmodelled `visual.json` keys), so a
//! paste keeps everything the visual carries, its formatting and filters included, as Desktop's
//! copy does. The app puts it on the OS clipboard as JSON text. It is untrusted input: every
//! visual is checked (type, position, size, counts), names are always new, groups are re-linked
//! only inside the clip, and keys the typed record writes itself are dropped from the unmodelled
//! ones. Fields the target model lacks are kept as they are (broken references, as Desktop keeps
//! them) and each one is reported in `warnings`.
//!
//! **Groups** (`visualGroup` containers, `parentGroupName` members): member positions are page
//! positions, as PBIR writes them; a group's rectangle is refitted to its members after they move.

use std::collections::{HashMap, HashSet};

use bicraft_report::{FieldRef, Page, Position, Visual};
use serde_json::{Map, Value, json};

use crate::Session;
use crate::params::{allow, opt_str, str_param};
use crate::refs::{check_field, field_text};
use crate::report::{
    KNOWN_VISUAL_TYPES, MAX_FIELDS, MAX_SIZE, MAX_VISUAL_TYPE, check_coordinate, check_size, new_object_name,
    opt_num, outside_page_warning, page_at_mut, page_index, parts, position_json, report_ref,
};

/// The clipboard document's kind (`bicraft`) and version.
pub const CLIP_KIND: &str = "visuals";
pub const CLIP_VERSION: u64 = 1;
/// Most visuals one clipboard document carries (groups and members count).
pub const MAX_CLIP_VISUALS: usize = 500;
/// Most visuals one command lists.
const MAX_LIST: usize = 2000;
/// Longest visual name accepted in a clip (names are only used to re-link groups).
const MAX_NAME: usize = 200;
/// How far a paste moves when the position is taken (report pixels, right and down).
pub const PASTE_OFFSET: f64 = 20.0;
/// Most offset steps a paste tries before it keeps the last position.
const MAX_OFFSET_STEPS: usize = 100;
/// Deepest group nesting followed.
const MAX_DEPTH: usize = 32;
/// Most warnings an answer lists.
const MAX_WARNINGS: usize = 200;
/// Keys of `visual.json` the typed record writes itself: a clip's copies are dropped.
const TYPED_KEYS: &[&str] = &["name", "visual", "parentGroupName", "isHidden", "filterConfig"];
/// Keys of `position` the typed record writes itself.
const TYPED_POSITION: &[&str] = &["x", "y", "z", "width", "height", "tabOrder"];
/// `visual.align {to}` values.
pub const ALIGN_TO: &[&str] = &[
    "left",
    "center",
    "right",
    "top",
    "middle",
    "bottom",
    "distribute_horizontal",
    "distribute_vertical",
];

fn lower(s: &str) -> String {
    s.to_lowercase()
}

/// The visual names of `visual` and `visuals` (a list of names), in order, without repeats, at
/// least one.
fn listed_names(p: &Value) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(v) = opt_str(p, "visual")? {
        out.push(v.to_string());
    }
    match p.get("visuals") {
        None | Some(Value::Null) => {}
        Some(Value::Array(list)) => {
            if list.len() > MAX_LIST {
                return Err(format!("`visuals` lists more than {MAX_LIST} visuals"));
            }
            for x in list {
                let n = x.as_str().ok_or("`visuals` must be a list of visual names")?;
                if !out.iter().any(|o| o.eq_ignore_ascii_case(n)) {
                    out.push(n.to_string());
                }
            }
        }
        Some(_) => return Err("`visuals` must be a list of visual names".into()),
    }
    if out.is_empty() {
        return Err("name the visuals: `visual` or `visuals` (a list of names)".into());
    }
    Ok(out)
}

/// Indices of `names` on `page`, in order.
fn indices(page: &Page, names: &[String]) -> Result<Vec<usize>, String> {
    let mut out = Vec::with_capacity(names.len());
    for n in names {
        let i = crate::report::visual_index(page, n)?;
        if !out.contains(&i) {
            out.push(i);
        }
    }
    Ok(out)
}

/// Lowercased name → lowercased parent group, for the page.
fn parents(page: &Page) -> HashMap<String, String> {
    page.visuals
        .iter()
        .filter_map(|v| v.parent_group.as_ref().map(|g| (lower(&v.name), lower(g))))
        .collect()
}

/// The groups above `name`, nearest first (bounded, cycles cut).
fn ancestors(parent: &HashMap<String, String>, name: &str) -> Vec<String> {
    let mut chain = Vec::new();
    let mut at = lower(name);
    while chain.len() < MAX_DEPTH {
        match parent.get(&at) {
            Some(g) if !chain.contains(g) && *g != lower(name) => {
                chain.push(g.clone());
                at = g.clone();
            }
            _ => break,
        }
    }
    chain
}

/// The lowercased names of the members of the groups in `roots`, at any depth (roots included).
fn with_members(page: &Page, roots: &HashSet<String>) -> HashSet<String> {
    let mut all = roots.clone();
    for _ in 0..MAX_DEPTH {
        let before = all.len();
        for v in &page.visuals {
            if v.parent_group.as_ref().is_some_and(|g| all.contains(&lower(g))) {
                all.insert(lower(&v.name));
            }
        }
        if all.len() == before {
            break;
        }
    }
    all
}

/// The smallest rectangle around `rects` (x, y, right, bottom).
fn bounds(rects: impl Iterator<Item = Position>) -> Option<(f64, f64, f64, f64)> {
    rects.fold(None, |acc, p| {
        let (r, b) = (p.x + p.width, p.y + p.height);
        Some(match acc {
            None => (p.x, p.y, r, b),
            Some((x0, y0, x1, y1)) => (x0.min(p.x), y0.min(p.y), x1.max(r), y1.max(b)),
        })
    })
}

/// Sets every group above the visuals in `touched` (lowercased names) to the rectangle of its
/// members, innermost first. Groups in `keep` (moved themselves) are left alone.
fn refit_groups(page: &mut Page, touched: &HashSet<String>, keep: &HashSet<String>) {
    let parent = parents(page);
    let mut groups: Vec<(usize, String)> = Vec::new();
    for t in touched {
        for (depth, g) in ancestors(&parent, t).into_iter().enumerate() {
            if !keep.contains(&g) && !groups.iter().any(|(_, x)| *x == g) {
                groups.push((depth, g));
            }
        }
    }
    // deepest (closest to the members) first: a group's rectangle uses its inner groups' new ones
    groups.sort_by_key(|(depth, _)| *depth);
    let mut done: Vec<String> = Vec::new();
    for _ in 0..=MAX_DEPTH {
        for (_, g) in &groups {
            if done.contains(g) {
                continue;
            }
            // wait until every inner group of `g` that needs a refit is done
            let inner_pending = groups
                .iter()
                .any(|(_, h)| h != g && !done.contains(h) && parent.get(h).is_some_and(|p| p == g));
            if inner_pending {
                continue;
            }
            let b = bounds(
                page.visuals
                    .iter()
                    .filter(|v| v.parent_group.as_ref().is_some_and(|p| lower(p) == *g))
                    .map(|v| v.position),
            );
            if let (Some((x0, y0, x1, y1)), Some(gv)) =
                (b, page.visuals.iter_mut().find(|v| lower(&v.name) == *g))
            {
                gv.position.x = x0;
                gv.position.y = y0;
                gv.position.width = (x1 - x0).max(1.0);
                gv.position.height = (y1 - y0).max(1.0);
            }
            done.push(g.clone());
        }
        if done.len() == groups.len() {
            break;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Copy, paste, duplicate
// ---------------------------------------------------------------------------------------------

/// The clipboard document of the visuals `names` of page `page` (groups with their members).
fn clip_of(page: &Page, names: &[String]) -> Result<Value, String> {
    let picked = indices(page, names)?;
    let roots: HashSet<String> = picked
        .iter()
        .filter_map(|&i| page.visuals.get(i))
        .map(|v| lower(&v.name))
        .collect();
    let all = with_members(page, &roots);
    if all.len() > MAX_CLIP_VISUALS {
        return Err(format!(
            "{} visuals (groups count with their members): at most {MAX_CLIP_VISUALS} are copied at once",
            all.len()
        ));
    }
    let mut visuals = Vec::with_capacity(all.len());
    for v in page.visuals.iter().filter(|v| all.contains(&lower(&v.name))) {
        let mut v = v.clone();
        // a member copied without its group is a top-level visual of the copy
        if v.parent_group.as_ref().is_some_and(|g| !all.contains(&lower(g))) {
            v.parent_group = None;
        }
        visuals.push(serde_json::to_value(&v).map_err(|e| format!("visual `{}`: {e}", v.name))?);
    }
    Ok(json!({
        "bicraft": CLIP_KIND,
        "version": CLIP_VERSION,
        "page": {
            "name": page.name,
            "display_name": page.display_name,
            "width": page.width,
            "height": page.height,
        },
        "visuals": visuals,
    }))
}

pub(crate) fn visual_copy(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "visuals"])?;
    let key = str_param(p, "page")?;
    let names = listed_names(p)?;
    let r = report_ref(s)?;
    let pi = page_index(r, key)?;
    let page = r.pages.get(pi).ok_or("internal: page index out of range")?;
    let clip = clip_of(page, &names)?;
    let count = clip.get("visuals").and_then(Value::as_array).map_or(0, Vec::len);
    Ok(json!({"page": page.name, "count": count, "clip": clip}))
}

/// `visual.cut {page, visual? | visuals?}`: the clip `visual.copy` gives, and the visuals removed
/// (groups with their members) in the same step.
pub(crate) fn visual_cut(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "visuals"])?;
    let copied = visual_copy(s, p)?;
    let names = listed_names(p)?;
    let (first, rest) = names.split_first().ok_or("no visual to cut")?;
    let page = copied.get("page").cloned().unwrap_or(Value::Null);
    let removed = crate::report::visual_remove(s, &json!({"page": page, "visual": first, "visuals": rest}))?;
    let mut out = copied;
    if let Some(o) = out.as_object_mut() {
        o.insert(
            "removed".into(),
            removed.get("removed").cloned().unwrap_or(Value::Null),
        );
    }
    Ok(out)
}

/// A clip's visuals, checked: each one a valid record with a sane type, position and size.
fn parse_clip(clip: &Value) -> Result<Vec<Visual>, String> {
    let o = clip.as_object().ok_or(
        "`clip` must be the object visual.copy returns ({bicraft: \"visuals\", version: 1, visuals: [...]})",
    )?;
    if o.get("bicraft").and_then(Value::as_str) != Some(CLIP_KIND) {
        return Err("`clip` is not a BIcraft clipboard of visuals (`bicraft` must be \"visuals\")".into());
    }
    match o.get("version").and_then(Value::as_u64) {
        Some(CLIP_VERSION) => {}
        _ => return Err(format!("`clip.version` must be {CLIP_VERSION}")),
    }
    let list = o
        .get("visuals")
        .and_then(Value::as_array)
        .ok_or("`clip.visuals` must be a list")?;
    if list.is_empty() || list.len() > MAX_CLIP_VISUALS {
        return Err(format!(
            "`clip.visuals` must hold 1 to {MAX_CLIP_VISUALS} visuals"
        ));
    }
    let mut out = Vec::with_capacity(list.len());
    for (i, raw) in list.iter().enumerate() {
        let at = format!("clip.visuals[{i}]");
        let mut v: Visual =
            serde_json::from_value(raw.clone()).map_err(|e| format!("{at} is not a visual: {e}"))?;
        if v.name.is_empty() || v.name.chars().count() > MAX_NAME {
            return Err(format!("{at}.name must be 1 to {MAX_NAME} characters"));
        }
        if let Some(t) = &v.visual_type
            && (t.is_empty()
                || t.len() > MAX_VISUAL_TYPE
                || !t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        {
            return Err(format!("{at}: `{t}` is not a visual type"));
        }
        let pos = v.position;
        check_coordinate(&format!("{at}.position.x"), pos.x)?;
        check_coordinate(&format!("{at}.position.y"), pos.y)?;
        check_coordinate(&format!("{at}.position.z"), pos.z)?;
        check_size(&format!("{at}.position.width"), pos.width)?;
        check_size(&format!("{at}.position.height"), pos.height)?;
        if v.fields.len() > MAX_FIELDS {
            return Err(format!("{at} binds more than {MAX_FIELDS} fields"));
        }
        // what the typed record writes itself is not taken from the unmodelled keys
        for k in TYPED_KEYS {
            v.extra.remove(*k);
        }
        if let Some(Value::Object(pm)) = v.extra.get_mut("position") {
            for k in TYPED_POSITION {
                pm.remove(*k);
            }
        }
        v.visual_extra.remove("visualType");
        match &v.visual_type {
            Some(_) => {
                v.extra.remove("visualGroup");
            }
            None => {
                if !v.extra.get("visualGroup").is_some_and(Value::is_object) {
                    v.extra.insert(
                        "visualGroup".into(),
                        json!({"displayName": "Group", "groupMode": "ScaleMode"}),
                    );
                }
            }
        }
        out.push(v);
    }
    let mut seen = HashSet::new();
    for v in &out {
        if !seen.insert(lower(&v.name)) {
            return Err(format!("`clip.visuals` names `{}` twice", v.name));
        }
    }
    Ok(out)
}

/// Whether a binding resolves on the target: the model's objects or the report's own measures.
fn field_problem(
    model: Option<&bicraft_model::SemanticModel>,
    measures: &[crate::report_measures::ReportMeasure],
    f: &FieldRef,
) -> Option<String> {
    let m = model?;
    let problem = check_field(m, f)?;
    if measures
        .iter()
        .any(|rm| crate::report_measures::binding(rm) == *f)
    {
        return None;
    }
    Some(problem)
}

/// Pastes `visuals` (checked) on page `pi`: new names, groups re-linked inside the clip, on top of
/// the page, at the same place (moved by [`PASTE_OFFSET`] while that place is taken) or with
/// the top-left corner at `at`, kept on the page when they fit.
fn paste_into(
    s: &mut Session,
    pi: usize,
    mut visuals: Vec<Visual>,
    at: Option<(f64, f64)>,
) -> Result<Value, String> {
    let (model, r) = parts(s)?;
    let measures = crate::report_measures::report_measures(r);
    let page = page_at_mut(r, pi)?;
    let mut warnings: Vec<String> = Vec::new();
    // names: always new ones, unique on the page and in the paste
    let mut taken: HashSet<String> = page.visuals.iter().map(|v| lower(&v.name)).collect();
    let mut rename: HashMap<String, String> = HashMap::new();
    for v in &visuals {
        let n = new_object_name(|c| taken.contains(&lower(c)));
        taken.insert(lower(&n));
        rename.insert(lower(&v.name), n);
    }
    let groups: HashSet<String> = visuals
        .iter()
        .filter(|v| v.visual_type.is_none())
        .map(|v| lower(&v.name))
        .collect();
    for v in &mut visuals {
        v.parent_group = v
            .parent_group
            .as_ref()
            .filter(|g| groups.contains(&lower(g)))
            .and_then(|g| rename.get(&lower(g)).cloned());
        if let Some(n) = rename.get(&lower(&v.name)) {
            v.name.clone_from(n);
        }
    }
    // cut group cycles (a hostile clip): a member whose chain comes back to it is top-level
    let parent: HashMap<String, String> = visuals
        .iter()
        .filter_map(|v| v.parent_group.as_ref().map(|g| (lower(&v.name), lower(g))))
        .collect();
    for v in &mut visuals {
        let me = lower(&v.name);
        let mut at_ = me.clone();
        let mut ok = true;
        for step in 0..=MAX_DEPTH {
            match parent.get(&at_) {
                Some(g) if *g == me || step == MAX_DEPTH => {
                    ok = false;
                    break;
                }
                Some(g) => at_ = g.clone(),
                None => break,
            }
        }
        if !ok {
            v.parent_group = None;
        }
    }
    // where: the same place, moved while it is taken; or the top-left corner at `at`
    let Some((x0, y0, x1, y1)) = bounds(visuals.iter().map(|v| v.position)) else {
        return Err("the clip has no visuals".into());
    };
    let (mut dx, mut dy) = match at {
        Some((x, y)) => (x - x0, y - y0),
        None => (0.0, 0.0),
    };
    if at.is_none() {
        let occupied = |dx: f64, dy: f64| {
            visuals.iter().filter(|v| v.visual_type.is_some()).any(|v| {
                page.visuals.iter().any(|o| {
                    (o.position.x - (v.position.x + dx)).abs() < 0.5
                        && (o.position.y - (v.position.y + dy)).abs() < 0.5
                })
            })
        };
        for _ in 0..MAX_OFFSET_STEPS {
            if !occupied(dx, dy) {
                break;
            }
            dx += PASTE_OFFSET;
            dy += PASTE_OFFSET;
        }
    }
    // kept on the page when the whole paste fits
    let (w, h) = (x1 - x0, y1 - y0);
    if page.width > 0.0 && w <= page.width {
        dx = dx.min(page.width - x1).max(-x0);
    }
    if page.height > 0.0 && h <= page.height {
        dy = dy.min(page.height - y1).max(-y0);
    }
    // on top of the page, in the clip's own order
    let top_z = page
        .visuals
        .iter()
        .map(|v| v.position.z)
        .fold(None, |m: Option<f64>, z| Some(m.map_or(z, |m| m.max(z))))
        .unwrap_or(0.0);
    let top_tab = page
        .visuals
        .iter()
        .filter_map(|v| v.position.tab_order)
        .max()
        .unwrap_or(0);
    let mut order: Vec<usize> = (0..visuals.len()).collect();
    order.sort_by(|&a, &b| {
        let z = |i: usize| visuals.get(i).map_or(0.0, |v| v.position.z);
        z(a).total_cmp(&z(b)).then(a.cmp(&b))
    });
    for (rank, &i) in order.iter().enumerate() {
        let Some(v) = visuals.get_mut(i) else { continue };
        let step = (rank as f64 + 1.0) * 1000.0;
        v.position.x = (v.position.x + dx).clamp(-MAX_SIZE, MAX_SIZE);
        v.position.y = (v.position.y + dy).clamp(-MAX_SIZE, MAX_SIZE);
        v.position.z = (top_z + step).min(MAX_SIZE);
        let tab_step = i64::try_from(rank)
            .unwrap_or(i64::MAX / 2)
            .saturating_add(1)
            .saturating_mul(1000);
        v.position.tab_order = Some(top_tab.saturating_add(tab_step));
    }
    // what the target cannot resolve is kept and reported
    if model.is_none() && visuals.iter().any(|v| !v.fields.is_empty()) {
        warnings.push("the session has no model: the fields were pasted unchecked".into());
    }
    for v in &visuals {
        if let Some(t) = &v.visual_type
            && !KNOWN_VISUAL_TYPES.contains(&t.as_str())
        {
            warnings.push(format!(
                "visual `{}`: `{t}` is not a built-in visual type; it must be a custom visual installed in the report",
                v.name
            ));
        }
        for b in &v.fields {
            if let Some(problem) = field_problem(model, &measures, &b.field) {
                warnings.push(format!(
                    "visual `{}`: {} ({}) kept as a broken reference: {problem}",
                    v.name,
                    field_text(&b.field),
                    b.role
                ));
            }
        }
        if let Some(w) = outside_page_warning(page, &v.position)
            && v.parent_group.is_none()
        {
            warnings.push(format!("visual `{}`: {w}", v.name));
        }
    }
    warnings.truncate(MAX_WARNINGS);
    let names: Vec<String> = visuals.iter().map(|v| v.name.clone()).collect();
    let top: Vec<String> = visuals
        .iter()
        .filter(|v| v.parent_group.is_none())
        .map(|v| v.name.clone())
        .collect();
    let selectable: Vec<String> = visuals
        .iter()
        .filter(|v| v.visual_type.is_some() && !v.is_hidden)
        .map(|v| v.name.clone())
        .collect();
    let out = json!({
        "page": page.name,
        "visuals": names,
        "top": top,
        "selectable": selectable,
        "offset": [dx, dy],
        "warnings": warnings,
    });
    page.visuals.extend(visuals);
    s.report_dirty = true;
    Ok(out)
}

pub(crate) fn visual_paste(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "clip", "x", "y"])?;
    let key = str_param(p, "page")?;
    let clip = p
        .get("clip")
        .ok_or("missing parameter `clip` (what visual.copy returned)")?;
    // `clip` may be visual.copy's whole answer
    let clip = clip.get("clip").filter(|c| c.is_object()).unwrap_or(clip);
    let visuals = parse_clip(clip)?;
    let x = opt_num(p, "x")?.map(|v| check_coordinate("x", v)).transpose()?;
    let y = opt_num(p, "y")?.map(|v| check_coordinate("y", v)).transpose()?;
    let at = match (x, y) {
        (Some(x), Some(y)) => Some((x, y)),
        (None, None) => None,
        _ => return Err("give both `x` and `y`, or neither".into()),
    };
    let r = report_ref(s)?;
    let pi = page_index(r, key)?;
    paste_into(s, pi, visuals, at)
}

pub(crate) fn visual_duplicate(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "visuals"])?;
    let key = str_param(p, "page")?;
    let names = listed_names(p)?;
    let r = report_ref(s)?;
    let pi = page_index(r, key)?;
    let page = r.pages.get(pi).ok_or("internal: page index out of range")?;
    let clip = clip_of(page, &names)?;
    let visuals = parse_clip(&clip)?;
    paste_into(s, pi, visuals, None)
}

// ---------------------------------------------------------------------------------------------
// Move, resize, align, distribute
// ---------------------------------------------------------------------------------------------

/// A new rectangle for one visual.
struct Move {
    index: usize,
    to: Position,
}

/// Applies `moves` (checked, distinct visuals, none inside another one's group): a group's
/// members follow it (moved and scaled with its rectangle), the groups around the moved visuals
/// are refitted. Returns the answer's `moved` rows and the warnings.
fn apply_moves(page: &mut Page, moves: Vec<Move>) -> (Vec<Value>, Vec<String>) {
    let parent = parents(page);
    let mut rows = Vec::new();
    let mut warnings = Vec::new();
    let mut touched: HashSet<String> = HashSet::new();
    let mut moved_groups: HashSet<String> = HashSet::new();
    for m in moves {
        let Some(v) = page.visuals.get(m.index) else {
            continue;
        };
        let from = v.position;
        let name = v.name.clone();
        let key = lower(&name);
        let is_group = v.visual_type.is_none();
        let mut to = m.to;
        to.z = from.z;
        to.tab_order = from.tab_order;
        if from == to {
            continue;
        }
        if let Some(w) = outside_page_warning(page, &to)
            && !parent.contains_key(&key)
        {
            warnings.push(format!("visual `{name}`: {w}"));
        }
        if is_group {
            // members keep their place inside the group's rectangle
            let members = with_members(page, &HashSet::from([key.clone()]));
            let sx = if from.width > 0.0 {
                to.width / from.width
            } else {
                1.0
            };
            let sy = if from.height > 0.0 {
                to.height / from.height
            } else {
                1.0
            };
            for mv in page.visuals.iter_mut() {
                let mk = lower(&mv.name);
                if mk != key && members.contains(&mk) {
                    let p = &mut mv.position;
                    p.x = (to.x + (p.x - from.x) * sx).clamp(-MAX_SIZE, MAX_SIZE);
                    p.y = (to.y + (p.y - from.y) * sy).clamp(-MAX_SIZE, MAX_SIZE);
                    p.width = (p.width * sx).clamp(1.0, MAX_SIZE);
                    p.height = (p.height * sy).clamp(1.0, MAX_SIZE);
                }
            }
            moved_groups.insert(key.clone());
        }
        if let Some(v) = page.visuals.get_mut(m.index) {
            v.position = to;
        }
        touched.insert(key);
        rows.push(json!({"visual": name, "position": position_json(&to), "previous": position_json(&from)}));
    }
    refit_groups(page, &touched, &moved_groups);
    warnings.truncate(MAX_WARNINGS);
    (rows, warnings)
}

/// Fails when a listed visual is inside a listed group (move the group or its member).
fn check_no_nesting(page: &Page, idx: &[usize]) -> Result<(), String> {
    let parent = parents(page);
    let listed: HashSet<String> = idx
        .iter()
        .filter_map(|&i| page.visuals.get(i))
        .map(|v| lower(&v.name))
        .collect();
    for &i in idx {
        let Some(v) = page.visuals.get(i) else { continue };
        if let Some(g) = ancestors(&parent, &v.name)
            .into_iter()
            .find(|g| listed.contains(g))
        {
            return Err(format!(
                "visual `{}` is inside group `{g}`, which is listed too: list the group or its member",
                v.name
            ));
        }
    }
    Ok(())
}

pub(crate) fn visual_move(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visuals"])?;
    let key = str_param(p, "page")?;
    let list = p
        .get("visuals")
        .and_then(Value::as_array)
        .ok_or("`visuals` must be a list of {visual, x?, y?, width?, height?}")?;
    if list.is_empty() || list.len() > MAX_LIST {
        return Err(format!("`visuals` must list 1 to {MAX_LIST} visuals"));
    }
    let mut asked: Vec<(String, [Option<f64>; 4])> = Vec::with_capacity(list.len());
    for (i, item) in list.iter().enumerate() {
        let at = format!("visuals[{i}]");
        if !item.is_object() {
            return Err(format!("{at} must be {{visual, x?, y?, width?, height?}}"));
        }
        allow(item, &["visual", "x", "y", "width", "height"]).map_err(|e| format!("{at}: {e}"))?;
        let name = str_param(item, "visual").map_err(|e| format!("{at}: {e}"))?;
        let x = opt_num(item, "x")?
            .map(|v| check_coordinate("x", v))
            .transpose()?;
        let y = opt_num(item, "y")?
            .map(|v| check_coordinate("y", v))
            .transpose()?;
        let w = opt_num(item, "width")?
            .map(|v| check_size("width", v))
            .transpose()?;
        let h = opt_num(item, "height")?
            .map(|v| check_size("height", v))
            .transpose()?;
        if [x, y, w, h].iter().all(Option::is_none) {
            return Err(format!("{at} needs at least one of x, y, width, height"));
        }
        asked.push((name.to_string(), [x, y, w, h]));
    }
    let (_, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let page = page_at_mut(r, pi)?;
    let mut moves = Vec::with_capacity(asked.len());
    let mut idx = Vec::with_capacity(asked.len());
    for (name, [x, y, w, h]) in asked {
        let i = crate::report::visual_index(page, &name)?;
        if idx.contains(&i) {
            return Err(format!("visual `{name}` is listed twice"));
        }
        idx.push(i);
        let mut to = page
            .visuals
            .get(i)
            .map(|v| v.position)
            .ok_or("internal: visual index out of range")?;
        to.x = x.unwrap_or(to.x);
        to.y = y.unwrap_or(to.y);
        to.width = w.unwrap_or(to.width);
        to.height = h.unwrap_or(to.height);
        moves.push(Move { index: i, to });
    }
    check_no_nesting(page, &idx)?;
    let page_name = page.name.clone();
    let (rows, warnings) = apply_moves(page, moves);
    if !rows.is_empty() {
        s.report_dirty = true;
    }
    Ok(json!({"page": page_name, "changed": !rows.is_empty(), "moved": rows, "warnings": warnings}))
}

pub(crate) fn visual_align(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "visuals", "to"])?;
    let key = str_param(p, "page")?;
    let names = listed_names(p)?;
    let to = str_param(p, "to")?;
    let to = ALIGN_TO
        .iter()
        .find(|t| t.eq_ignore_ascii_case(to))
        .copied()
        .ok_or_else(|| format!("`to` must be one of {}", ALIGN_TO.join(", ")))?;
    let (_, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let page = page_at_mut(r, pi)?;
    let idx = indices(page, &names)?;
    check_no_nesting(page, &idx)?;
    let rects: Vec<(usize, Position)> = idx
        .iter()
        .filter_map(|&i| page.visuals.get(i).map(|v| (i, v.position)))
        .collect();
    let distribute = to.starts_with("distribute");
    if distribute && rects.len() < 3 {
        return Err("distributing needs at least three visuals".into());
    }
    // one visual aligns to the page, several to the rectangle around them
    let (bx0, by0, bx1, by1) = if rects.len() == 1 {
        (0.0, 0.0, page.width, page.height)
    } else {
        bounds(rects.iter().map(|(_, p)| *p)).ok_or("no visuals to align")?
    };
    let mut moves = Vec::with_capacity(rects.len());
    if distribute {
        let horizontal = to == "distribute_horizontal";
        let mut sorted = rects.clone();
        let start = |p: &Position| if horizontal { p.x } else { p.y };
        let size = |p: &Position| if horizontal { p.width } else { p.height };
        sorted.sort_by(|a, b| start(&a.1).total_cmp(&start(&b.1)).then(a.0.cmp(&b.0)));
        let total: f64 = sorted.iter().map(|(_, p)| size(p)).sum();
        let span = if horizontal { bx1 - bx0 } else { by1 - by0 };
        let n = sorted.len() as f64;
        let gap = (span - total) / (n - 1.0);
        let mut cursor = if horizontal { bx0 } else { by0 };
        for (i, mut pos) in sorted {
            if horizontal {
                pos.x = cursor;
            } else {
                pos.y = cursor;
            }
            cursor += size(&pos) + gap;
            moves.push(Move { index: i, to: pos });
        }
    } else {
        for (i, mut pos) in rects {
            match to {
                "left" => pos.x = bx0,
                "center" => pos.x = (bx0 + bx1) / 2.0 - pos.width / 2.0,
                "right" => pos.x = bx1 - pos.width,
                "top" => pos.y = by0,
                "middle" => pos.y = (by0 + by1) / 2.0 - pos.height / 2.0,
                _ => pos.y = by1 - pos.height,
            }
            pos.x = pos.x.clamp(-MAX_SIZE, MAX_SIZE);
            pos.y = pos.y.clamp(-MAX_SIZE, MAX_SIZE);
            moves.push(Move { index: i, to: pos });
        }
    }
    let page_name = page.name.clone();
    let (rows, warnings) = apply_moves(page, moves);
    if !rows.is_empty() {
        s.report_dirty = true;
    }
    Ok(json!({"page": page_name, "to": to, "changed": !rows.is_empty(), "moved": rows, "warnings": warnings}))
}

// ---------------------------------------------------------------------------------------------
// Group, ungroup
// ---------------------------------------------------------------------------------------------

/// A group's display name (`visualGroup.displayName`).
fn group_display(v: &Visual) -> Option<&str> {
    v.extra
        .get("visualGroup")
        .and_then(|g| g.get("displayName"))
        .and_then(Value::as_str)
}

pub(crate) fn visual_group(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual", "visuals", "display_name"])?;
    let key = str_param(p, "page")?;
    let names = listed_names(p)?;
    let display = opt_str(p, "display_name")?;
    if let Some(d) = display
        && (d.trim().is_empty() || d.chars().count() > MAX_NAME)
    {
        return Err(format!("`display_name` must be 1 to {MAX_NAME} characters"));
    }
    let (_, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let page = page_at_mut(r, pi)?;
    let idx = indices(page, &names)?;
    if idx.len() < 2 {
        return Err("grouping needs at least two visuals".into());
    }
    check_no_nesting(page, &idx)?;
    let members: Vec<&Visual> = idx.iter().filter_map(|&i| page.visuals.get(i)).collect();
    let parent = members.first().and_then(|v| v.parent_group.clone());
    if members
        .iter()
        .any(|v| v.parent_group.as_deref().map(lower) != parent.as_deref().map(lower))
    {
        return Err(
            "the visuals are not in the same group: group visuals of one group (or of the page's top level)"
                .into(),
        );
    }
    let (x0, y0, x1, y1) = bounds(members.iter().map(|v| v.position)).ok_or("no visuals to group")?;
    let z = members
        .iter()
        .map(|v| v.position.z)
        .fold(f64::NEG_INFINITY, f64::max)
        .max(-MAX_SIZE);
    let tab = members.iter().filter_map(|v| v.position.tab_order).min();
    let display = match display {
        Some(d) => d.trim().to_string(),
        None => {
            let used: HashSet<String> = page.visuals.iter().filter_map(group_display).map(lower).collect();
            let mut n = 1usize;
            while used.contains(&lower(&format!("Group {n}"))) && n < 100_000 {
                n = n.saturating_add(1);
            }
            format!("Group {n}")
        }
    };
    let name = new_object_name(|c| page.visuals.iter().any(|v| v.name.eq_ignore_ascii_case(c)));
    let member_names: Vec<String> = members.iter().map(|v| v.name.clone()).collect();
    let mut extra = Map::new();
    extra.insert(
        "visualGroup".into(),
        json!({"displayName": display, "groupMode": "ScaleMode"}),
    );
    let group = Visual {
        name: name.clone(),
        position: Position {
            x: x0,
            y: y0,
            z,
            width: (x1 - x0).max(1.0),
            height: (y1 - y0).max(1.0),
            tab_order: tab,
        },
        parent_group: parent,
        extra,
        ..Default::default()
    };
    for &i in &idx {
        if let Some(v) = page.visuals.get_mut(i) {
            v.parent_group = Some(name.clone());
        }
    }
    let at = idx
        .iter()
        .max()
        .map_or(page.visuals.len(), |m| m.saturating_add(1));
    page.visuals.insert(at.min(page.visuals.len()), group);
    let out = json!({"page": page.name, "group": name, "display_name": display, "members": member_names});
    s.report_dirty = true;
    Ok(out)
}

pub(crate) fn visual_ungroup(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual"])?;
    let key = str_param(p, "page")?;
    let visual = str_param(p, "visual")?;
    let (_, r) = parts(s)?;
    let pi = page_index(r, key)?;
    let page = page_at_mut(r, pi)?;
    let vi = crate::report::visual_index(page, visual)?;
    let v = page
        .visuals
        .get(vi)
        .ok_or("internal: visual index out of range")?;
    // a member stands for its innermost group
    let gi = if v.visual_type.is_none() {
        vi
    } else {
        let g = v
            .parent_group
            .clone()
            .ok_or_else(|| format!("visual `{}` is not a group and is in no group", v.name))?;
        crate::report::visual_index(page, &g)?
    };
    let g = page
        .visuals
        .get(gi)
        .ok_or("internal: visual index out of range")?;
    if g.visual_type.is_some() {
        return Err(format!("`{}` is not a group", g.name));
    }
    let (gname, up) = (g.name.clone(), g.parent_group.clone());
    let mut members = Vec::new();
    for v in page.visuals.iter_mut() {
        if v.parent_group
            .as_ref()
            .is_some_and(|p| p.eq_ignore_ascii_case(&gname))
        {
            v.parent_group.clone_from(&up);
            members.push(v.name.clone());
        }
    }
    page.visuals.remove(gi);
    let out = json!({"page": page.name, "ungrouped": gname, "members": members});
    s.report_dirty = true;
    Ok(out)
}
