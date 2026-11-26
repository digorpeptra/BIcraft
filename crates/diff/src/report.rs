//! The report half of the diff.

use std::collections::{BTreeMap, BTreeSet};

use bicraft_report::{FieldRef, Page, Report, Resource, Visual};
use serde_json::{Map, Value, json};

use crate::json::{Differ, clip, field_label, flatten_objects, formatting};
use crate::model::{flag, opt_str};
use crate::pair::{Item, pair};
use crate::{Area, Change, FieldDelta, How, Kind, Options, Prop, dq};

const SECTION_REPORT: u8 = 0;
const SECTION_PAGES: u8 = 1;
const SECTION_BOOKMARKS: u8 = 2;
const SECTION_RESOURCES: u8 = 3;

const RANK_PAGE: u8 = 0;
const RANK_PAGE_FILTER: u8 = 1;
const RANK_VISUAL: u8 = 2;
const RANK_VISUAL_FILTER: u8 = 3;

/// Formatting maps of a visual, page or report.
const FORMATTING_KEYS: &[&str] = &["objects", "visualContainerObjects", "vcObjects"];

/// Most bytes of a JSON resource (a theme) compared property by property.
const MAX_JSON_RESOURCE_BYTES: usize = 16 << 20;

struct Ctx<'o> {
    opts: &'o Options,
    out: &'o mut Vec<Change>,
}

fn extras(
    props: &mut Vec<Prop>,
    prefix: &str,
    a: &Map<String, Value>,
    b: &Map<String, Value>,
    ignore: &[&str],
) {
    // what a legacy layout holds and PBIR cannot (`legacy:*`) only counts between two legacy reports
    let legacy_only = |m: &Map<String, Value>, other: &Map<String, Value>| -> Map<String, Value> {
        m.iter()
            .filter(|(k, _)| !k.starts_with("legacy:") || other.contains_key(k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    };
    let (a, b) = (legacy_only(a, b), legacy_only(b, a));
    let mut d = Differ::new(props, ignore);
    d.maps(prefix, &a, &b);
    d.finish();
}

pub(crate) fn compare(a: &Report, b: &Report, opts: &Options, out: &mut Vec<Change>) {
    let mut cx = Ctx { opts, out };
    report_props(&mut cx, a, b);
    pages(&mut cx, a, b);
    bookmarks(&mut cx, a, b);
    resources(&mut cx, &a.resources, &b.resources);
}

fn report_props(cx: &mut Ctx<'_>, a: &Report, b: &Report) {
    let mut c = Change::new(Area::Report, "report", "report".into(), Kind::Changed);
    // page order, by display name, when the same pages are in another order
    let order = |r: &Report| -> Vec<String> { r.pages.iter().map(|p| p.display_name.clone()).collect() };
    let (oa, ob) = (order(a), order(b));
    let set = |v: &[String]| -> BTreeSet<String> { v.iter().cloned().collect() };
    if oa != ob && set(&oa) == set(&ob) {
        c.properties
            .push(Prop::new("page_order", Some(json!(oa)), Some(json!(ob))));
    }
    let mut ignore: Vec<&str> = vec!["themeCollection", "filterConfig"];
    ignore.extend(FORMATTING_KEYS);
    extras(&mut c.properties, "", &a.extra, &b.extra, &ignore);
    // pages.json beyond the order and the active page; version.json is format churn
    extras(
        &mut c.properties,
        "pages",
        &a.pages_extra,
        &b.pages_extra,
        &["pageOrder", "activePageName"],
    );
    if !cx.opts.ignore_formatting {
        for key in FORMATTING_KEYS {
            formatting(
                &flatten_objects(a.extra.get(*key)),
                &flatten_objects(b.extra.get(*key)),
                &mut c.formatting,
            );
        }
    }
    if c.has_content() {
        cx.out.push(c.sorted(SECTION_REPORT, "", 0));
    }
    if !cx.opts.ignore_formatting {
        let mut t = Change::new(Area::Report, "theme", "theme".into(), Kind::Changed);
        let mut d = Differ::new(&mut t.properties, &[]);
        d.values(
            "themeCollection",
            a.theme_collection.as_ref().filter(|v| !is_empty_object(v)),
            b.theme_collection.as_ref().filter(|v| !is_empty_object(v)),
        );
        d.finish();
        if t.has_content() {
            cx.out.push(t.sorted(SECTION_REPORT, "", 1));
        }
    }
    filters(
        cx,
        "report",
        a.filter_config.as_ref(),
        b.filter_config.as_ref(),
        SECTION_REPORT,
        "",
        2,
    );
}

/// The filters of a `filterConfig`, matched by name, then by field.
fn filters(
    cx: &mut Ctx<'_>,
    scope: &str,
    a: Option<&Value>,
    b: Option<&Value>,
    section: u8,
    group: &str,
    rank: u8,
) {
    let list = |v: Option<&Value>| -> Vec<Value> {
        v.and_then(|v| v.get("filters"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let (la, lb) = (list(a), list(b));
    let label = |f: &Value| -> String {
        f.get("field")
            .and_then(field_label)
            .or_else(|| f.get("name").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_default()
    };
    let (labels_a, labels_b): (Vec<String>, Vec<String>) =
        (la.iter().map(label).collect(), lb.iter().map(label).collect());
    let items = crate::pair::items_fn2(|v: &[Value], labels: &[String]| {
        v.iter()
            .zip(labels)
            .map(|(f, l)| Item {
                name: f.get("name").and_then(Value::as_str).unwrap_or(""),
                tag: None,
                sig: Some(l.clone()),
            })
            .collect()
    });
    let (ia, ib) = (items(&la, &labels_a), items(&lb, &labels_b));
    let p = pair(&ia, &ib);
    let id = |l: &str| format!("filter {l} on {scope}");
    let strip = |v: &Value| -> Map<String, Value> {
        let mut m = v.as_object().cloned().unwrap_or_default();
        m.remove("name");
        m
    };
    for &(x, y, _) in &p.pairs {
        let (Some(fa), Some(fb)) = (la.get(x), lb.get(y)) else {
            continue;
        };
        let mut c = Change::new(
            Area::Report,
            "filter",
            id(labels_b.get(y).map_or("", String::as_str)),
            Kind::Changed,
        );
        extras(&mut c.properties, "", &strip(fa), &strip(fb), &[]);
        if c.has_content() {
            cx.out.push(c.sorted(section, group, rank));
        }
    }
    for &x in &p.removed {
        if let Some(f) = la.get(x) {
            let mut c = Change::new(
                Area::Report,
                "filter",
                id(labels_a.get(x).map_or("", String::as_str)),
                Kind::Removed,
            );
            c.before = Some(clip(f));
            cx.out.push(c.sorted(section, group, rank));
        }
    }
    for &y in &p.added {
        if let Some(f) = lb.get(y) {
            let mut c = Change::new(
                Area::Report,
                "filter",
                id(labels_b.get(y).map_or("", String::as_str)),
                Kind::Added,
            );
            c.after = Some(clip(f));
            cx.out.push(c.sorted(section, group, rank));
        }
    }
    // the filter pane's order
    let order = |v: Option<&Value>| v.and_then(|v| v.get("filterSortOrder")).cloned();
    let (sa, sb) = (order(a), order(b));
    if sa != sb {
        let mut c = Change::new(
            Area::Report,
            "filter",
            format!("filters on {scope}"),
            Kind::Changed,
        );
        c.properties.push(Prop::new("filterSortOrder", sa, sb));
        cx.out.push(c.sorted(section, group, rank));
    }
}

fn page_id(p: &Page) -> String {
    format!("page {}", dq(&p.display_name))
}

fn page_summary(p: &Page) -> Value {
    json!({
        "name": p.name,
        "display_name": p.display_name,
        "size": format!("{}x{}", num(p.width), num(p.height)),
        "visuals": p.visuals.len(),
    })
}

/// A number rounded to 2 decimals, without trailing zeros.
pub(crate) fn num(v: f64) -> String {
    if !v.is_finite() {
        return v.to_string();
    }
    let r = (v * 100.0).round() / 100.0;
    let r = if r == 0.0 { 0.0 } else { r };
    format!("{r}")
}

fn pages(cx: &mut Ctx<'_>, a: &Report, b: &Report) {
    let items = crate::pair::items_fn(|r: &Report| {
        r.pages
            .iter()
            .map(|p| Item {
                name: &p.name,
                tag: None,
                // a page whose folder name was regenerated keeps its display name
                sig: Some(p.display_name.clone()),
            })
            .collect()
    });
    let (ia, ib) = (items(a), items(b));
    let p = pair(&ia, &ib);
    let mut loose_a: Vec<(&Page, &Visual)> = Vec::new();
    let mut loose_b: Vec<(&Page, &Visual)> = Vec::new();
    for &(x, y, _) in &p.pairs {
        let (Some(pa), Some(pb)) = (a.pages.get(x), b.pages.get(y)) else {
            continue;
        };
        page_pair(cx, pa, pb, &mut loose_a, &mut loose_b);
    }
    for &x in &p.removed {
        if let Some(pg) = a.pages.get(x) {
            let mut c = Change::new(Area::Report, "page", page_id(pg), Kind::Removed);
            c.before = Some(page_summary(pg));
            cx.out.push(c.sorted(SECTION_PAGES, &pg.display_name, RANK_PAGE));
            loose_a.extend(pg.visuals.iter().map(|v| (pg, v)));
        }
    }
    for &y in &p.added {
        if let Some(pg) = b.pages.get(y) {
            let mut c = Change::new(Area::Report, "page", page_id(pg), Kind::Added);
            c.after = Some(page_summary(pg));
            cx.out.push(c.sorted(SECTION_PAGES, &pg.display_name, RANK_PAGE));
            loose_b.extend(pg.visuals.iter().map(|v| (pg, v)));
        }
    }
    // visuals left over on both sides: the same name on another page is a move
    let paired: BTreeSet<(&str, &str)> = p
        .pairs
        .iter()
        .filter_map(|&(x, y, _)| Some((a.pages.get(x)?.name.as_str(), b.pages.get(y)?.name.as_str())))
        .collect();
    let mut by_name: BTreeMap<&str, Option<usize>> = BTreeMap::new();
    for (i, (_, v)) in loose_b.iter().enumerate() {
        by_name
            .entry(v.name.as_str())
            .and_modify(|e| *e = None)
            .or_insert(Some(i));
    }
    let mut b_used = vec![false; loose_b.len()];
    let mut a_left = Vec::new();
    for (pa, va) in &loose_a {
        match by_name.get(va.name.as_str()) {
            Some(Some(i)) if !b_used.get(*i).copied().unwrap_or(true) => {
                if let (Some(slot), Some((pb, vb))) = (b_used.get_mut(*i), loose_b.get(*i)) {
                    *slot = true;
                    visual_pair(
                        cx,
                        pa,
                        va,
                        pb,
                        vb,
                        !paired.contains(&(pa.name.as_str(), pb.name.as_str())),
                    );
                }
            }
            _ => a_left.push((*pa, *va)),
        }
    }
    // visuals of removed pages are counted in the page; on kept pages each is its own change
    let kept_a: BTreeSet<&str> = p
        .pairs
        .iter()
        .filter_map(|&(x, _, _)| a.pages.get(x))
        .map(|p| p.name.as_str())
        .collect();
    let kept_b: BTreeSet<&str> = p
        .pairs
        .iter()
        .filter_map(|&(_, y, _)| b.pages.get(y))
        .map(|p| p.name.as_str())
        .collect();
    for (pg, v) in a_left {
        if kept_a.contains(pg.name.as_str()) {
            let mut c = Change::new(Area::Report, "visual", visual_id(pg, v), Kind::Removed);
            c.label = Some(visual_label(v));
            c.before = Some(visual_summary(v));
            cx.out
                .push(c.sorted(SECTION_PAGES, &pg.display_name, RANK_VISUAL));
        }
    }
    for (i, (pg, v)) in loose_b.iter().enumerate() {
        if b_used.get(i).copied().unwrap_or(true) || !kept_b.contains(pg.name.as_str()) {
            continue;
        }
        let mut c = Change::new(Area::Report, "visual", visual_id(pg, v), Kind::Added);
        c.label = Some(visual_label(v));
        c.after = Some(visual_summary(v));
        cx.out
            .push(c.sorted(SECTION_PAGES, &pg.display_name, RANK_VISUAL));
    }
}

fn page_pair<'r>(
    cx: &mut Ctx<'_>,
    pa: &'r Page,
    pb: &'r Page,
    loose_a: &mut Vec<(&'r Page, &'r Visual)>,
    loose_b: &mut Vec<(&'r Page, &'r Visual)>,
) {
    let renamed = pa.display_name != pb.display_name;
    let mut c = Change::new(
        Area::Report,
        "page",
        page_id(pb),
        if renamed { Kind::Renamed } else { Kind::Changed },
    );
    if renamed {
        c.from = Some(page_id(pa));
        c.how = Some(How::Name);
    }
    let p = &mut c.properties;
    opt_str(
        p,
        "display_option",
        pa.display_option.as_deref(),
        pb.display_option.as_deref(),
    );
    if !cx.opts.ignore_positions {
        let size = |pg: &Page| format!("{}x{}", num(pg.width), num(pg.height));
        opt_str(p, "size", Some(&size(pa)), Some(&size(pb)));
    }
    opt_str(
        p,
        "visibility",
        pa.visibility.as_deref(),
        pb.visibility.as_deref(),
    );
    let mut ignore = vec!["name", "displayName", "ordinal", "filterConfig"];
    ignore.extend(FORMATTING_KEYS);
    extras(p, "", &pa.extra, &pb.extra, &ignore);
    if !cx.opts.ignore_formatting {
        for key in FORMATTING_KEYS {
            formatting(
                &flatten_objects(pa.extra.get(*key)),
                &flatten_objects(pb.extra.get(*key)),
                &mut c.formatting,
            );
        }
    }
    if renamed || c.has_content() {
        cx.out.push(c.sorted(SECTION_PAGES, &pb.display_name, RANK_PAGE));
    }
    filters(
        cx,
        &page_id(pb),
        pa.filter_config.as_ref(),
        pb.filter_config.as_ref(),
        SECTION_PAGES,
        &pb.display_name,
        RANK_PAGE_FILTER,
    );
    let items = |pg: &'r Page| -> Vec<Item<'r>> {
        pg.visuals
            .iter()
            .map(|v| Item {
                name: &v.name,
                tag: None,
                sig: Some(visual_sig(v)),
            })
            .collect()
    };
    let (ia, ib) = (items(pa), items(pb));
    let pr = pair(&ia, &ib);
    for &(x, y, _) in &pr.pairs {
        if let (Some(va), Some(vb)) = (pa.visuals.get(x), pb.visuals.get(y)) {
            visual_pair(cx, pa, va, pb, vb, false);
        }
    }
    loose_a.extend(
        pr.removed
            .iter()
            .filter_map(|&x| pa.visuals.get(x))
            .map(|v| (pa, v)),
    );
    loose_b.extend(
        pr.added
            .iter()
            .filter_map(|&y| pb.visuals.get(y))
            .map(|v| (pb, v)),
    );
}

pub(crate) fn field_text(f: &FieldRef) -> String {
    match f {
        FieldRef::Column { table, column } => format!("{table}[{column}]"),
        FieldRef::Measure { table, measure } => format!("{table}[{measure}]"),
        FieldRef::Aggregation {
            table,
            column,
            function,
        } => format!("{function}({table}[{column}])"),
        FieldRef::Other { raw } => field_label(raw).unwrap_or_else(|| {
            let s = serde_json::to_string(raw).unwrap_or_default();
            s.chars().take(120).collect()
        }),
    }
}

fn visual_type(v: &Visual) -> String {
    v.visual_type.clone().unwrap_or_else(|| "group".into())
}

fn visual_id(p: &Page, v: &Visual) -> String {
    format!("visual {} on {}", v.name, page_id(p))
}

/// `type (first fields)`, or a group's display name.
fn visual_label(v: &Visual) -> String {
    let fields: Vec<String> = v.fields.iter().take(3).map(|f| field_text(&f.field)).collect();
    let more = if v.fields.len() > 3 { ", …" } else { "" };
    if let Some(name) = v
        .extra
        .get("visualGroup")
        .and_then(|g| g.get("displayName"))
        .and_then(Value::as_str)
    {
        return format!("group {}", dq(name));
    }
    if fields.is_empty() {
        visual_type(v)
    } else {
        format!("{} ({}{more})", visual_type(v), fields.join(", "))
    }
}

fn position(v: &Visual) -> String {
    let p = &v.position;
    format!("{},{} {}x{}", num(p.x), num(p.y), num(p.width), num(p.height))
}

fn visual_sig(v: &Visual) -> String {
    let fields: Vec<String> = v
        .fields
        .iter()
        .map(|f| format!("{}={}", f.role, field_text(&f.field)))
        .collect();
    format!(
        "{}\u{1}{}\u{1}{}",
        visual_type(v),
        fields.join("\u{2}"),
        position(v)
    )
}

fn roles(v: &Visual) -> BTreeMap<&str, Vec<String>> {
    let mut m: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for f in &v.fields {
        m.entry(f.role.as_str()).or_default().push(field_text(&f.field));
    }
    m
}

fn visual_summary(v: &Visual) -> Value {
    let fields: Map<String, Value> = roles(v)
        .into_iter()
        .map(|(r, f)| (r.to_string(), json!(f)))
        .collect();
    json!({"type": visual_type(v), "position": position(v), "fields": fields})
}

fn visual_pair(cx: &mut Ctx<'_>, pa: &Page, va: &Visual, pb: &Page, vb: &Visual, moved: bool) {
    let mut c = Change::new(
        Area::Report,
        "visual",
        visual_id(pb, vb),
        if moved { Kind::Moved } else { Kind::Changed },
    );
    c.label = Some(visual_label(vb));
    if moved {
        c.from = Some(visual_id(pa, va));
        c.how = Some(How::Name);
    }
    let p = &mut c.properties;
    opt_str(p, "type", Some(&visual_type(va)), Some(&visual_type(vb)));
    if !cx.opts.ignore_positions {
        opt_str(p, "position", Some(&position(va)), Some(&position(vb)));
        opt_str(p, "z", Some(&num(va.position.z)), Some(&num(vb.position.z)));
        if va.position.tab_order != vb.position.tab_order {
            p.push(Prop::new(
                "tab_order",
                va.position.tab_order.map(|t| json!(t)),
                vb.position.tab_order.map(|t| json!(t)),
            ));
        }
    }
    opt_str(
        p,
        "parent_group",
        va.parent_group.as_deref(),
        vb.parent_group.as_deref(),
    );
    flag(p, "hidden", va.is_hidden, vb.is_hidden);
    // the visual object beyond fields and formatting; the query's other parts (sorting, …)
    let mut ignore = vec!["query"];
    ignore.extend(FORMATTING_KEYS);
    extras(p, "visual", &va.visual_extra, &vb.visual_extra, &ignore);
    let query = |v: &Visual| -> Map<String, Value> {
        let mut m = v
            .visual_extra
            .get("query")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        m.remove("queryState");
        m
    };
    extras(p, "visual.query", &query(va), &query(vb), &[]);
    extras(
        p,
        "",
        &va.extra,
        &vb.extra,
        &[
            "name",
            "position",
            "visual",
            "filterConfig",
            "parentGroupName",
            "isHidden",
        ],
    );
    // fields, role by role
    let (ra, rb) = (roles(va), roles(vb));
    let names: BTreeSet<&str> = ra.keys().chain(rb.keys()).copied().collect();
    for role in names {
        let (fa, fb) = (
            ra.get(role).cloned().unwrap_or_default(),
            rb.get(role).cloned().unwrap_or_default(),
        );
        if fa == fb {
            continue;
        }
        let mut rest_b = fb.clone();
        let mut removed = Vec::new();
        for f in &fa {
            match rest_b.iter().position(|x| x == f) {
                Some(i) => {
                    rest_b.remove(i);
                }
                None => removed.push(f.clone()),
            }
        }
        let reordered = removed.is_empty() && rest_b.is_empty();
        c.fields.push(FieldDelta {
            role: role.to_string(),
            added: rest_b,
            removed,
            reordered,
        });
    }
    if !cx.opts.ignore_formatting {
        for key in FORMATTING_KEYS {
            formatting(
                &flatten_objects(va.visual_extra.get(*key)),
                &flatten_objects(vb.visual_extra.get(*key)),
                &mut c.formatting,
            );
        }
    }
    if moved || c.has_content() {
        cx.out
            .push(c.sorted(SECTION_PAGES, &pb.display_name, RANK_VISUAL));
    }
    filters(
        cx,
        &visual_id(pb, vb),
        va.filter_config.as_ref(),
        vb.filter_config.as_ref(),
        SECTION_PAGES,
        &pb.display_name,
        RANK_VISUAL_FILTER,
    );
}

fn bookmark_name(d: &bicraft_report::NamedDocument) -> &str {
    d.document
        .get("displayName")
        .and_then(Value::as_str)
        .unwrap_or(&d.name)
}

fn bookmarks(cx: &mut Ctx<'_>, a: &Report, b: &Report) {
    let items = crate::pair::items_fn(|r: &Report| {
        r.bookmarks
            .iter()
            .map(|d| Item {
                name: &d.name,
                tag: None,
                sig: Some(bookmark_name(d).to_string()),
            })
            .collect()
    });
    let (ia, ib) = (items(a), items(b));
    let p = pair(&ia, &ib);
    let id = |d: &bicraft_report::NamedDocument| format!("bookmark {}", dq(bookmark_name(d)));
    for &(x, y, _) in &p.pairs {
        let (Some(da), Some(db)) = (a.bookmarks.get(x), b.bookmarks.get(y)) else {
            continue;
        };
        let renamed = bookmark_name(da) != bookmark_name(db);
        let mut c = Change::new(
            Area::Report,
            "bookmark",
            id(db),
            if renamed { Kind::Renamed } else { Kind::Changed },
        );
        if renamed {
            c.from = Some(id(da));
            c.how = Some(How::Name);
        }
        let obj = |d: &bicraft_report::NamedDocument| d.document.as_object().cloned().unwrap_or_default();
        extras(
            &mut c.properties,
            "",
            &obj(da),
            &obj(db),
            &["name", "displayName"],
        );
        if renamed || c.has_content() {
            cx.out.push(c.sorted(SECTION_BOOKMARKS, "", 0));
        }
    }
    for &x in &p.removed {
        if let Some(d) = a.bookmarks.get(x) {
            let mut c = Change::new(Area::Report, "bookmark", id(d), Kind::Removed);
            c.before = Some(json!({"name": d.name}));
            cx.out.push(c.sorted(SECTION_BOOKMARKS, "", 0));
        }
    }
    for &y in &p.added {
        if let Some(d) = b.bookmarks.get(y) {
            let mut c = Change::new(Area::Report, "bookmark", id(d), Kind::Added);
            c.after = Some(json!({"name": d.name}));
            cx.out.push(c.sorted(SECTION_BOOKMARKS, "", 0));
        }
    }
}

fn resource_path(r: &Resource) -> String {
    r.path.replace('\\', "/")
}

fn resources(cx: &mut Ctx<'_>, a: &[Resource], b: &[Resource]) {
    let (ma, mb) = (resource_map(a), resource_map(b));
    let paths: BTreeSet<&String> = ma.keys().chain(mb.keys()).collect();
    for path in paths {
        let id = format!("resource {path}");
        let size = |r: &Resource| json!(format!("{} bytes", r.bytes.len()));
        match (ma.get(path), mb.get(path)) {
            (Some(x), Some(y)) => {
                if x.bytes == y.bytes {
                    continue;
                }
                let mut c = Change::new(Area::Report, "resource", id, Kind::Changed);
                let parsed = |r: &Resource| -> Option<Value> {
                    if r.bytes.len() > MAX_JSON_RESOURCE_BYTES
                        || !path.to_ascii_lowercase().ends_with(".json")
                    {
                        return None;
                    }
                    let text = String::from_utf8_lossy(&r.bytes);
                    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
                };
                match (parsed(x), parsed(y)) {
                    (Some(ja), Some(jb)) => {
                        if cx.opts.ignore_formatting {
                            continue;
                        }
                        let mut d = Differ::new(&mut c.properties, &[]);
                        d.values("", Some(&ja), Some(&jb));
                        d.finish();
                    }
                    _ => c
                        .properties
                        .push(Prop::new("bytes", Some(size(x)), Some(size(y)))),
                }
                if c.has_content() {
                    cx.out.push(c.sorted(SECTION_RESOURCES, "", 0));
                }
            }
            (Some(x), None) => {
                let mut c = Change::new(Area::Report, "resource", id, Kind::Removed);
                c.before = Some(size(x));
                cx.out.push(c.sorted(SECTION_RESOURCES, "", 0));
            }
            (None, Some(y)) => {
                let mut c = Change::new(Area::Report, "resource", id, Kind::Added);
                c.after = Some(size(y));
                cx.out.push(c.sorted(SECTION_RESOURCES, "", 0));
            }
            (None, None) => {}
        }
    }
}

fn resource_map(v: &[Resource]) -> BTreeMap<String, &Resource> {
    v.iter().map(|r| (resource_path(r), r)).collect()
}

fn is_empty_object(v: &Value) -> bool {
    v.as_object().is_some_and(Map::is_empty)
}
