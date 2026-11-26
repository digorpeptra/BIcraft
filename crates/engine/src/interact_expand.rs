//! `visual.expand`: a matrix's expansion and a visual's displayed levels edited in the report,
//! as Power BI Desktop saves them when an author expands a matrix in its editing view
//! (docs/formats/pbir.md, "Matrix expansion and per-level subtotals"): the row header's +/-
//! (a member), right-click Expand / Collapse › Selection, Entire level, All, and the visual
//! header's "Expand all down one level", "Go to the next level" and "Drill up". It writes the
//! role's projections' `active` flags and the visual's `expansionStates`, so it is saved with
//! the file (PBIP and PBIX) and undone like any edit. The rules are the reader's
//! (`interact_drill.rs`), applied to the visual as the file stores it; a reader's own drill
//! state on that visual (`view.interact`) is dropped so the file's shows.

use serde_json::{Value, json};

use super::{desktop_drill_names, drill, find_visual};
use crate::Session;

/// The header steps.
const STEPS: &[&str] = &["expand_all", "next", "up"];

fn opt_bool(p: &Value, key: &str) -> Result<bool, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(format!("`{key}` must be true or false")),
    }
}

/// `visual.expand {page, visual, role?, path? | level? | all?, collapse?, step?}`.
pub(crate) fn visual_expand(s: &mut Session, p: &Value) -> Result<Value, String> {
    crate::params::allow(
        p,
        &[
            "page", "visual", "role", "path", "level", "all", "collapse", "step",
        ],
    )?;
    let collapse = opt_bool(p, "collapse")?;
    let all = opt_bool(p, "all")?;
    let role = match p.get("role") {
        None | Some(Value::Null) => None,
        Some(Value::String(r)) if r.eq_ignore_ascii_case("rows") || r.eq_ignore_ascii_case("columns") => {
            Some(r.as_str())
        }
        Some(_) => return Err("`role` must be \"Rows\" or \"Columns\"".into()),
    };
    let step = match p.get("step") {
        None | Some(Value::Null) => None,
        Some(Value::String(x)) if STEPS.contains(&x.as_str()) => Some(x.as_str()),
        Some(_) => return Err(format!("`step` must be one of {}", STEPS.join(", "))),
    };
    let given = |k: &str| p.get(k).is_some_and(|x| !x.is_null());
    let target = usize::from(given("path")) + usize::from(given("level")) + usize::from(all);
    match (step, target) {
        (Some(_), 0) if !collapse => {}
        (Some(_), _) => {
            return Err("`step` takes no `path`, `level`, `all` or `collapse`".into());
        }
        (None, 1) => {}
        (None, _) => {
            return Err(
                "give one of `path` (a member, top level first), `level` (an entire level, 0: the top \
                 level shown), `all: true`, or a `step`"
                    .into(),
            );
        }
    }
    let report = s.report.as_ref().ok_or("the session has no report")?;
    let (page, v) = find_visual(report, p)?;
    let h = drill::hierarchy(s.model.as_ref(), v, role).ok_or_else(|| {
        format!(
            "visual `{}` has no hierarchy to expand (two or more columns on a matrix's rows or \
             columns, or on a chart's axis)",
            v.name
        )
    })?;
    let mut d = drill::stored(v, &h);
    match step {
        Some("expand_all") => drill::expand_all(&mut d)?,
        Some("next") => drill::next(&mut d)?,
        Some(_) => {
            if d.hier.matrix {
                d.exp = Some(drill::current_exp(&d, v));
            }
            drill::up(&mut d)?;
        }
        None => drill::matrix(&mut d, v, !collapse, p)?,
    }
    let key = (page.name.clone(), v.name.clone());
    let vis = s
        .report
        .as_mut()
        .and_then(|r| r.pages.iter_mut().find(|pg| pg.name == key.0))
        .and_then(|pg| pg.visuals.iter_mut().find(|x| x.name == key.1))
        .ok_or("internal: the visual went missing")?;
    let before = vis.clone();
    drill::apply(vis, &d);
    desktop_drill_names(vis, &key.0, &key.1);
    let changed = *vis != before;
    if s.interactions.drills.remove(&key).is_some() {
        s.interactions.seq = s.interactions.seq.saturating_add(1);
    }
    if changed {
        s.report_dirty = true;
    }
    Ok(json!({
        "page": key.0,
        "visual": key.1,
        "changed": changed,
        "drill": drill::drill_json(&d, false),
    }))
}
