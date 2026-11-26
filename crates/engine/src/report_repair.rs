//! `report.repair` and the matching `report.validate` errors: report defects Power BI Desktop
//! cannot open that bicraft can rewrite safely.
//!
//! - **Suffixed duplicate queryRefs.** Before it reused the first queryRef, bicraft gave a second
//!   projection of the same field (a scatter chart's Y and Size) a suffixed queryRef
//!   (`Sum(Sales.Qty)1`), so the query selected the field twice and Desktop 2.158 failed the
//!   visual. An unchanged query is written back as is, so files saved then keep the defect. The
//!   repair points every projection of the field at its first queryRef, which also drops the
//!   duplicate select item of a legacy `prototypeQuery` on save. Projections of different fields
//!   are never touched, whatever their names.

use bicraft_report::{Report, Visual, query};
use serde_json::{Value, json};

use crate::Session;
use crate::params::{allow, opt_str};
use crate::report::{page_index, parts, visual_index};

fn message(visual: &str, kept: &str, duplicate: &str) -> String {
    format!(
        "visual `{visual}` projects the same field under two queryRefs, `{kept}` and \
         `{duplicate}`: its query selects the field twice and Power BI Desktop fails it; \
         run report.repair (or re-bind the visual's fields) to use `{kept}` for both"
    )
}

/// `report.validate` errors: visuals whose query projects one field under two queryRefs.
pub(crate) fn validation_issues(r: &Report) -> Vec<(bool, Value, String)> {
    let mut out = Vec::new();
    for page in &r.pages {
        for v in &page.visuals {
            // The query a writer emits: re-bound fields regenerate (and so repair) it.
            for (kept, duplicate) in query::duplicate_refs(query::effective(v).as_ref()) {
                out.push((
                    true,
                    json!({"page": page.display_name, "visual": v.name, "part": "query",
                           "query_ref": duplicate}),
                    message(&v.name, &kept, &duplicate),
                ));
            }
        }
    }
    out
}

/// Points `metadata` selectors of `v`'s formatting objects at `kept` instead of `duplicate`,
/// unless the object already has an entry with the resulting selector.
fn rename_selectors(v: &mut Visual, kept: &str, duplicate: &str) {
    let Some(objects) = v.visual_extra.get_mut("objects").and_then(Value::as_object_mut) else {
        return;
    };
    for entries in objects.values_mut() {
        let Some(entries) = entries.as_array_mut() else {
            continue;
        };
        let selectors: Vec<Value> = entries
            .iter()
            .filter_map(|e| e.get("selector").cloned())
            .collect();
        for e in entries.iter_mut() {
            let Some(selector) = e.get_mut("selector") else {
                continue;
            };
            if selector.get("metadata").and_then(Value::as_str) != Some(duplicate) {
                continue;
            }
            let mut renamed = selector.clone();
            if let Some(o) = renamed.as_object_mut() {
                o.insert("metadata".into(), json!(kept));
            }
            if !selectors.contains(&renamed) {
                *selector = renamed;
            }
        }
    }
}

/// Repairs one visual; returns the `(kept, duplicate)` pairs it fixed.
/// When the bindings were edited since the query was read, the query is replaced by the one the
/// writers would emit (already deduplicated), so formatting selectors are repaired too.
fn repair_visual(v: &mut Visual) -> Vec<(String, String)> {
    let raw = v.visual_extra.get("query");
    let pairs = query::duplicate_refs(raw);
    if pairs.is_empty() {
        return pairs;
    }
    if query::derive(raw) == v.fields {
        if let Some(q) = v.visual_extra.get_mut("query") {
            query::dedupe_refs(q);
        }
    } else {
        match query::effective(v) {
            Some(q) => {
                v.visual_extra.insert("query".into(), q);
            }
            None => {
                v.visual_extra.shift_remove("query");
            }
        }
    }
    for (kept, duplicate) in &pairs {
        rename_selectors(v, kept, duplicate);
    }
    pairs
}

/// `report.repair {page?, visual?}`.
pub(crate) fn report_repair(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual"])?;
    let page_key = opt_str(p, "page")?;
    let visual_key = opt_str(p, "visual")?;
    if visual_key.is_some() && page_key.is_none() {
        return Err("`visual` needs `page`".into());
    }
    let (_, r) = parts(s)?;
    let pages: Vec<usize> = match page_key {
        Some(k) => vec![page_index(r, k)?],
        None => (0..r.pages.len()).collect(),
    };
    let mut fixed = Vec::new();
    for pi in pages {
        let Some(page) = r.pages.get_mut(pi) else {
            continue;
        };
        let visuals: Vec<usize> = match visual_key {
            Some(k) => vec![visual_index(page, k)?],
            None => (0..page.visuals.len()).collect(),
        };
        let display = page.display_name.clone();
        for vi in visuals {
            let Some(v) = page.visuals.get_mut(vi) else {
                continue;
            };
            for (kept, duplicate) in repair_visual(v) {
                fixed.push(
                    json!({"page": display, "visual": v.name, "kind": "duplicate_query_ref",
                                  "kept": kept, "replaced": duplicate}),
                );
            }
        }
    }
    if !fixed.is_empty() {
        s.report_dirty = true;
    }
    Ok(json!({"count": fixed.len(), "fixed": fixed}))
}
