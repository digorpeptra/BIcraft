//! Report interactions of the live viewer (`docs/view-snapshot.md`, "Interactions").
//!
//! The viewer is a report reader: a click on a slicer item, a data point, a page tab or a button
//! changes the *interaction state* held here, in the session, and the next snapshot shows the
//! report under that state. Nothing is ever written to the model, the report or the files: the
//! state lives as long as the session (`view.interact {action: "reset"}` drops it).
//!
//! The state is applied by deriving reports from the session's report for each snapshot
//! ([`Interactions::effective`]):
//! - the **data report**, which the preview evaluates: each slicer carries its *applied*
//!   selection (as `objects.general.filter`, the place Desktop stores it, so the existing filter
//!   rules apply: other slicers filter it, never its own selection, `NoFilter` interactions are
//!   honoured), and the page's data-point selection is added as a visual filter of every visual it
//!   filters;
//! - the **highlight report**: the visuals the selection highlights, with the selection as a
//!   filter, whose results become each row's highlighted part ([`merge_highlights`]; Desktop asks
//!   for both values in one query with `IGNORE`, which the DAX evaluator does not take yet);
//! - the **display report**, which the snapshot describes: the data report, except that a slicer
//!   shows its *pending* selection on a page with an "Apply all slicers" button.
//!
//! Desktop's rules reproduced here (the interaction oracle, docs/interaction-oracle.md, rule
//! numbers in parentheses):
//! - slicer items (8): a plain click selects that item alone, or clears the selection when it was
//!   the only one selected; Ctrl+click adds or removes the item; "Select all" clears the
//!   selection; a click on a synced slicer reaches every slicer of its group, on every page,
//!   hidden ones too (6). Not in the oracle: single select (`selection.strictSingleSelect`)
//!   always selects the one item; without "Multi-select with Ctrl" (`selection.singleSelect:
//!   false`) a click toggles;
//! - data points (12, 13): a click selects the point (the visual's grouping values: category,
//!   legend, a table row), a plain click on one of several selected points keeps only that one, a
//!   click on the only selected point clears it, Ctrl+click adds or removes points; one selection
//!   per page. The source visual is not re-evaluated; every other visual of the page is reached
//!   (9, 14): highlighted when it can highlight (the types the highlight probe saw highlight,
//!   BIcraft's own visuals included when they have a measure role: [`can_highlight`]) unless the
//!   page's `visualInteractions` say `DataFilter` (or the report cross-filters by default,
//!   `settings.defaultFilterActionIsDataFilter`, and no `HighlightFilter` rule asks for the
//!   highlight), filtered otherwise, left alone with `NoFilter`;
//! - "Apply all slicers" (`visualLink` `ApplyAllSlicers`, 17): while the page has such a button,
//!   slicer changes are pending until it is clicked, Import models included; "Clear all slicers"
//!   (`ClearAllSlicers`, 16) clears every slicer of the page (applied and pending) and keeps the
//!   data-point selection;
//! - page navigation (`pageNavigator`, `visualLink` `PageNavigation`, `Back`): changes the shown
//!   page; each page keeps its slicers and selection.

use std::collections::HashMap;

use bicraft_model::{DataType, SemanticModel};
use bicraft_report::{FieldRef, Page, Report, Visual};
use serde_json::{Map, Value, json};

use crate::Session;
use crate::preview::filters;

/// Most values one slicer selection or one data-point selection holds.
const MAX_VALUES: usize = 10_000;
/// Most points of one data-point selection.
const MAX_POINTS: usize = 1_000;
/// Most grouping values in one point.
const MAX_POINT_COLUMNS: usize = 16;
/// Pages kept in the "Back" history.
const MAX_BACK: usize = 64;
/// Longest text a text slicer takes.
const MAX_TEXT: usize = 1_000;
/// Name of the visual-filter containers the data-point selection adds (data report only).
const SELECTION_FILTER: &str = "bicraft-interaction";
/// The `visualLink` types the viewer acts on.
const BUTTON_ACTIONS: [&str; 8] = [
    "PageNavigation",
    "Back",
    "ApplyAllSlicers",
    "ClearAllSlicers",
    "Drillthrough",
    "WebUrl",
    "Bookmark",
    "Qna",
];
/// `objects.data` properties a range or relative-date slicer displays (cleared with it).
const RANGE_PROPS: [&str; 4] = ["numericStart", "numericEnd", "startDate", "endDate"];
const RELATIVE_PROPS: [&str; 4] = [
    "relativeRange",
    "relativeDuration",
    "relativePeriod",
    "relativeTimePeriod",
];

/// A slicer's selection as the viewer set it.
#[derive(Debug, Clone, PartialEq, Default)]
struct SlicerState {
    /// `objects.general.filter.filter` (`None`: nothing selected).
    filter: Option<Value>,
    /// `objects.data` properties set (`Some`) or removed (`None`).
    data: Vec<(String, Option<Value>)>,
    /// The selected values (`inverted`: the excluded ones), for the next toggle.
    values: Vec<Value>,
    inverted: bool,
}

/// A data-point selection on one page.
#[derive(Debug, Clone, PartialEq)]
struct Selection {
    /// The visual the points were clicked in.
    visual: String,
    /// Each point: its grouping values.
    points: Vec<Vec<Pv>>,
}

/// One grouping value of a selected point.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Pv {
    /// The visual's data column.
    pub(crate) c: usize,
    pub(crate) table: String,
    pub(crate) column: String,
    /// The model column's type (how the value is written as a literal).
    pub(crate) ty: DataType,
    pub(crate) value: Value,
}

/// The viewer's interaction state (see the module docs). Empty by default.
#[derive(Debug, Clone, Default)]
pub struct Interactions {
    /// The page the reader is on (`None`: the report's active page).
    page: Option<String>,
    /// Pages left by navigation, for `Back`.
    back: Vec<String>,
    /// Applied slicer selections, by (page, visual).
    applied: HashMap<(String, String), SlicerState>,
    /// Selections waiting for "Apply all slicers", by (page, visual).
    pending: HashMap<(String, String), SlicerState>,
    /// The data-point selection of each page.
    selections: HashMap<String, Selection>,
    /// Decomposition trees whose levels or path the reader changed, by (page, visual): the
    /// state the evaluator reads instead of the stored `expansionStates`
    /// ([`crate::preview::DECOMP_STATE`]).
    trees: HashMap<(String, String), Value>,
    /// Bumped by every change (the viewer's `interaction.seq`).
    seq: u64,
    /// The reader's drill state of each drilled visual, by (page, visual) ([`drill`]).
    drills: HashMap<(String, String), drill::Drill>,
    /// The filters of each drillthrough page, set by the last drillthrough to it.
    through: HashMap<String, drill::Through>,
    /// The report-page tooltip shown (the hovered point).
    tip: Option<drill::Tip>,
    /// Key influencers visuals show BIcraft's estimate instead of the placeholder
    /// (`view.interact {action: "key_influencers"}`; kept by `reset`).
    key_influencers: bool,
    /// Visibility set by a bookmark, by (page, visual): `true` hidden ([`bookmark`]).
    hidden: HashMap<(String, String), bool>,
    /// `filterConfig`s set by a bookmark, by container ([`bookmark::Scope`]).
    filters: HashMap<bookmark::Scope, Option<Value>>,
    /// The last bookmark applied (a bookmark navigator shows it selected).
    bookmark: Option<String>,
    /// The visual shown in focus mode, and whether its data table is shown under it (Show as a
    /// table): `view.interact` `focus` / `show_table`. Display only: the data and display reports
    /// do not change, so it is left out of [`Interactions::is_empty`]; a page change leaves it.
    focus: Option<Focus>,
    /// The visual in spotlight, (page, visual): `view.interact` `spotlight`. Display only (the
    /// renderer dims the page's other visuals); a page change leaves it ([`reader`]).
    spotlight: Option<(String, String)>,
    /// The reader's sorts, by (page, visual): `view.interact` `sort`, a bookmark's
    /// `singleVisual.orderBy`. The data report's visual takes it as its `query.sortDefinition`;
    /// the report is not changed ([`reader`]).
    sorts: HashMap<(String, String), Vec<(FieldRef, bool)>>,
    /// View as (Desktop's Modeling › View as): the model roles whose row filters every
    /// evaluation applies, and the user `USERPRINCIPALNAME()` returns (`None`: the data as
    /// stored; [`crate::preview`], "Row-level security").
    view_as: Option<ViewAs>,
    /// The reader's toggled nodes of BIcraft Org Charts, by (page, visual): `view.interact`
    /// `tree_toggle`. The data report's visual takes the list as its `tree.toggled`; the report is
    /// not changed ([`reader`]).
    nodes: HashMap<(String, String), Vec<String>>,
}

/// Most roles one View as names.
const MAX_VIEW_AS_ROLES: usize = 256;
/// Longest "Other user" name.
const MAX_VIEW_AS_USER: usize = 256;

/// The roles and the user of View as (`view.interact {action: "view_as"}`): view state, never
/// written to the model or the report.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct ViewAs {
    /// Model role names, as the model spells them, without duplicates, in the order given.
    pub roles: Vec<String>,
    /// "Other user": what `USERPRINCIPALNAME()` and `USERNAME()` return (`None`: empty text).
    pub user: Option<String>,
}

impl ViewAs {
    pub(crate) fn to_json(&self) -> Value {
        json!({"roles": self.roles, "user": self.user})
    }
}

/// The reader's focus mode ([`Interactions::focus`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Focus {
    page: String,
    visual: String,
    table: bool,
}

/// The data and display reports of [`Interactions::effective`].
pub(crate) struct Effective {
    pub data: Report,
    pub display: Report,
    /// The highlight report: on pages with a selection, the slicers and the visuals the
    /// selection highlights, those filtered by it (`None`: nothing is highlighted). Its values
    /// are each highlighted row's part ([`merge_highlights`]).
    pub highlight: Option<Report>,
    /// The (page, visual) the highlight report highlights (its slicers only filter).
    pub highlight_targets: Vec<(String, String)>,
}

impl Interactions {
    /// The page the reader moved to (`None`: the report's active page).
    pub fn current_page(&self) -> Option<&str> {
        self.page.as_deref()
    }

    /// `"estimate"` when key influencers visuals show BIcraft's estimate, else `"placeholder"`.
    pub(crate) fn key_influencers_mode(&self) -> &'static str {
        if self.key_influencers {
            "estimate"
        } else {
            "placeholder"
        }
    }

    /// Whether key influencers visuals show BIcraft's estimate.
    pub(crate) fn key_influencers_estimate(&self) -> bool {
        self.key_influencers
    }

    /// The View as state (`None`: the data as stored).
    pub fn view_as(&self) -> Option<&ViewAs> {
        self.view_as.as_ref()
    }

    /// `role.rename`: View as keeps naming the renamed role.
    pub(crate) fn rename_view_as_role(&mut self, old: &str, new: &str) {
        if let Some(v) = self.view_as.as_mut() {
            for r in &mut v.roles {
                if r.eq_ignore_ascii_case(old) {
                    *r = new.to_string();
                }
            }
        }
    }

    /// The visual whose data-point selection cross-filters or cross-highlights page `page`.
    pub(crate) fn selection_source(&self, page: &str) -> Option<&str> {
        self.selections.get(page).map(|s| s.visual.as_str())
    }

    /// Whether nothing is set (the snapshot then shows the report as stored).
    pub fn is_empty(&self) -> bool {
        self.page.is_none()
            && self.applied.is_empty()
            && self.pending.is_empty()
            && self.selections.is_empty()
            && self.trees.is_empty()
            && self.drills.is_empty()
            && self.through.is_empty()
            && self.tip.is_none()
            && self.hidden.is_empty()
            && self.filters.is_empty()
            && self.bookmark.is_none()
            && self.sorts.is_empty()
            && self.nodes.is_empty()
    }

    /// The data and display reports for this state; `None` when the state changes nothing.
    pub(crate) fn effective(&self, r: &Report) -> Option<Effective> {
        if self.is_empty() {
            return None;
        }
        let mut data = r.clone();
        if let Some(p) = self
            .page
            .as_ref()
            .filter(|p| r.pages.iter().any(|x| &x.name == *p))
        {
            data.active_page = Some(p.clone());
        }
        bookmark::apply_overrides(self, &mut data);
        for page in &mut data.pages {
            for v in &mut page.visuals {
                if let Some(st) = self.applied.get(&(page.name.clone(), v.name.clone())) {
                    put_slicer(v, st);
                }
                if let Some(t) = self.trees.get(&(page.name.clone(), v.name.clone())) {
                    v.visual_extra
                        .insert(crate::preview::DECOMP_STATE.into(), t.clone());
                }
            }
        }
        drill::apply_all(self, &mut data);
        reader::apply_sorts(self, &mut data);
        reader::apply_nodes(self, &mut data);
        let mut display = data.clone();
        for page in &mut display.pages {
            for v in &mut page.visuals {
                if let Some(st) = self.pending.get(&(page.name.clone(), v.name.clone())) {
                    put_slicer(v, st);
                }
            }
        }
        // the highlight report starts from the data report before its cross-filters: same
        // slicer selections, the selection added to its highlight targets only
        let cf = cross_filter_default(r);
        let off = crate::optimize::interactions_off(r);
        let mut highlight: Option<Report> = None;
        let mut highlight_targets = Vec::new();
        for page in &data.pages {
            let Some(sel) = self.selections.get(&page.name) else {
                continue;
            };
            let targets: Vec<&Visual> = page
                .visuals
                .iter()
                .filter(|t| {
                    reaches(page, &sel.visual, t, off) && mode_of(page, &sel.visual, t, cf) == Mode::Highlight
                })
                .collect();
            if targets.is_empty() {
                continue;
            }
            highlight_targets.extend(targets.iter().map(|t| (page.name.clone(), t.name.clone())));
            let mut hp = page.clone();
            hp.visuals
                .retain(|v| filters::is_slicer(v) || targets.iter().any(|t| t.name == v.name));
            add_selection_filters(&mut hp, sel, &page.visuals, Some(Mode::Highlight), cf, off);
            let h = highlight.get_or_insert_with(|| Report {
                pages: Vec::new(),
                active_page: data.active_page.clone(),
                filter_config: data.filter_config.clone(),
                ..Report::default()
            });
            h.pages.push(hp);
        }
        for page in &mut data.pages {
            if let Some(sel) = self.selections.get(&page.name) {
                let all = page.visuals.clone();
                add_selection_filters(page, sel, &all, Some(Mode::Filter), cf, off);
            }
        }
        Some(Effective {
            data,
            display,
            highlight,
            highlight_targets,
        })
    }

    /// Adds the interaction fields to a snapshot built from [`Interactions::effective`]: the
    /// top-level `interaction` object, each source visual's `interaction.selected`, each filtered
    /// visual's `interaction.filtered_by`, and `slicer.pending` on slicers with pending changes.
    pub(crate) fn annotate(&self, r: &Report, snap: &mut Value) {
        let cf = cross_filter_default(r);
        let off = crate::optimize::interactions_off(r);
        let pending_pages: Vec<&str> = r
            .pages
            .iter()
            .filter(|p| self.pending.keys().any(|(pg, _)| *pg == p.name))
            .map(|p| p.name.as_str())
            .collect();
        let apply_pages: Vec<&str> = r
            .pages
            .iter()
            .filter(|p| has_button(p, "ApplyAllSlicers"))
            .map(|p| p.name.as_str())
            .collect();
        if let Some(o) = snap.as_object_mut() {
            o.insert(
                "interaction".into(),
                json!({
                    "seq": self.seq,
                    "page": self.page,
                    "can_go_back": !self.back.is_empty(),
                    "pending_pages": pending_pages,
                    "apply_pages": apply_pages,
                    "key_influencers": self.key_influencers_mode(),
                    "bookmark": self.bookmark,
                    "focus": self.focus_json(r),
                    "show_table": self.focus_json(r).is_some() && self.focus.as_ref().is_some_and(|f| f.table),
                    "spotlight": reader::spotlight_json(self, r),
                    "view_as": self.view_as.as_ref().map(ViewAs::to_json),
                }),
            );
        }
        let Some(pages) = snap.pointer_mut("/report/pages").and_then(Value::as_array_mut) else {
            return;
        };
        for p in pages {
            let Some(page_name) = p.get("name").and_then(Value::as_str).map(str::to_string) else {
                continue;
            };
            let Some(page) = r.pages.iter().find(|x| x.name == page_name) else {
                continue;
            };
            let sel = self.selections.get(&page_name);
            let Some(visuals) = p.get_mut("visuals").and_then(Value::as_array_mut) else {
                continue;
            };
            for v in visuals {
                let Some(name) = v.get("name").and_then(Value::as_str).map(str::to_string) else {
                    continue;
                };
                let mut info = Map::new();
                // what a click on a button does (the viewer only sends clicks it can act on)
                if let Some(link) = page.visuals.iter().find(|t| t.name == name).and_then(visual_link)
                    && BUTTON_ACTIONS.contains(&link.kind.as_str())
                {
                    info.insert("button".into(), button_info(r, sel, link, v));
                }
                if let Some(sel) = sel {
                    if sel.visual == name {
                        info.insert("selected".into(), selected_json(sel, v));
                    } else if let Some(t) = page
                        .visuals
                        .iter()
                        .find(|t| t.name == name && reaches(page, &sel.visual, t, off))
                    {
                        info.insert("filtered_by".into(), json!(sel.visual));
                        info.insert("mode".into(), json!(mode_of(page, &sel.visual, t, cf).as_str()));
                    }
                }
                if self.pending.contains_key(&(page_name.clone(), name.clone()))
                    && let Some(s) = v.get_mut("slicer").and_then(Value::as_object_mut)
                {
                    s.insert("pending".into(), json!(true));
                }
                if !info.is_empty()
                    && let Some(o) = v.as_object_mut()
                {
                    o.insert("interaction".into(), Value::Object(info));
                }
            }
        }
    }

    /// The state as JSON (`view.interactions`).
    pub fn state_json(&self) -> Value {
        let slicers = |m: &HashMap<(String, String), SlicerState>| -> Vec<Value> {
            let mut list: Vec<Value> = m
                .iter()
                .map(|((p, v), st)| {
                    json!({"page": p, "visual": v, "values": st.values, "inverted": st.inverted,
                           "filter": st.filter})
                })
                .collect();
            list.sort_by_key(|x| x.to_string());
            list
        };
        let mut selections: Vec<Value> = self
            .selections
            .iter()
            .map(|(p, s)| {
                let points: Vec<Value> = s
                    .points
                    .iter()
                    .map(|pt| {
                        Value::Array(
                            pt.iter()
                                .map(
                                    |x| json!({"c": x.c, "table": x.table, "column": x.column, "v": x.value}),
                                )
                                .collect(),
                        )
                    })
                    .collect();
                json!({"page": p, "visual": s.visual, "points": points})
            })
            .collect();
        selections.sort_by_key(|x| x.to_string());
        let mut trees: Vec<Value> = self
            .trees
            .iter()
            .map(|((p, v), t)| json!({"page": p, "visual": v, "state": t}))
            .collect();
        trees.sort_by_key(|x| x.to_string());
        let (hidden, bookmark_filters) = bookmark::state_json(self);
        json!({
            "seq": self.seq,
            "page": self.page,
            "back": self.back,
            "slicers": slicers(&self.applied),
            "pending": slicers(&self.pending),
            "selections": selections,
            "key_influencers": self.key_influencers_mode(),
            "view_as": self.view_as.as_ref().map(ViewAs::to_json),
            "trees": trees,
            "drills": drill::drills_json(&self.drills),
            "drillthrough": drill::through_json(&self.through),
            "tooltip": self.tip.as_ref().map(drill::tip_json),
            "bookmark": self.bookmark,
            "hidden": hidden,
            "bookmark_filters": bookmark_filters,
            "focus": self.focus.as_ref().map(|f| json!({"page": f.page, "visual": f.visual, "show_table": f.table})),
            "spotlight": self.spotlight.as_ref().map(|(p, v)| json!({"page": p, "visual": v})),
            "sorts": reader::sorts_json(self),
        })
    }

    /// The focused visual `{page, visual}` when it is still in `r` (`null` otherwise).
    fn focus_json(&self, r: &Report) -> Option<Value> {
        let f = self.focus.as_ref()?;
        r.pages
            .iter()
            .find(|p| p.name == f.page)?
            .visuals
            .iter()
            .any(|v| v.name == f.visual)
            .then(|| json!({"page": f.page, "visual": f.visual}))
    }
}

// ---------------------------------------------------------------------------------------------
// Commands

/// `view.interact {action, …}`: one reader action (see the command's params doc).
pub(crate) fn view_interact(s: &mut Session, p: &Value) -> Result<Value, String> {
    let mut view = std::mem::take(&mut s.interactions);
    let result = s.interact_view(&mut view, p);
    s.interactions = view;
    result
}

impl Session {
    /// Runs one reader action (`view.interact`'s parameters) on a view state of its own instead
    /// of the session's [`Session::interactions`], and returns `view.interact`'s answer. The
    /// session is not changed (nor its revision or last command): a server keeps one state per
    /// viewer over a shared session ([`Session::snapshot_view`]). On error `view` is unchanged.
    pub fn interact_view(&self, view: &mut Interactions, p: &Value) -> Result<Value, String> {
        interact_on(self, view, p)
    }
}

fn interact_on(s: &Session, current: &mut Interactions, p: &Value) -> Result<Value, String> {
    let action = p
        .get("action")
        .and_then(Value::as_str)
        .ok_or("`action` is required (see the command's params)")?;
    if action == "view_as" {
        return view_as_action(s, current, p);
    }
    if action == "reset" {
        let changed = !current.is_empty()
            || current.focus.is_some()
            || current.spotlight.is_some()
            || current.view_as.is_some();
        if current.view_as.is_some() {
            s.preview.drop_secured();
        }
        let seq = current.seq.saturating_add(1);
        *current = Interactions {
            seq,
            key_influencers: current.key_influencers,
            ..Interactions::default()
        };
        return Ok(json!({"action": action, "changed": changed, "state": current.state_json()}));
    }
    let report = s.report.as_ref().ok_or("the session has no report")?;
    let mut next = current.clone();
    let mut result = Map::new();
    match action {
        "page" => {
            let page = find_page(report, p.get("page"))?;
            next.go_to(report, &page.name);
        }
        "back" => {
            next.go_back(report);
        }
        "slicer_toggle" | "slicer_select_all" | "slicer_set" | "slicer_clear" | "slicer_range"
        | "slicer_text" | "slicer_relative" => {
            let (page, v) = find_visual(report, p)?;
            if !filters::is_slicer(v) {
                return Err(format!("visual `{}` is not a slicer", v.name));
            }
            let field = slicer_field(v).ok_or("the slicer has no column field")?;
            let key = (page.name.clone(), v.name.clone());
            let deferred = has_button(page, "ApplyAllSlicers");
            let current = next
                .pending
                .get(&key)
                .or_else(|| next.applied.get(&key))
                .cloned()
                .unwrap_or_else(|| stored_state(v));
            let model = s.model.as_ref();
            let today = filters::clock(s).floor();
            let st = slicer_action(action, p, v, &field, current, model, today)?;
            // the slicer, then its synced copies on every page, hidden ones too (oracle rule 6:
            // a click propagates; the files are not reconciled when the report opens)
            let mut targets = vec![(page, v)];
            if let Some(group) = sync_group(v) {
                for p2 in &report.pages {
                    for v2 in &p2.visuals {
                        if !(p2.name == page.name && v2.name == v.name)
                            && filters::is_slicer(v2)
                            && sync_group(v2).as_deref() == Some(group.as_str())
                        {
                            targets.push((p2, v2));
                        }
                    }
                }
            }
            for (tp, tv) in targets {
                let key = (tp.name.clone(), tv.name.clone());
                if has_button(tp, "ApplyAllSlicers") {
                    next.pending.insert(key, st.clone());
                } else {
                    next.pending.remove(&key);
                    next.applied.insert(key, st.clone());
                }
            }
            if deferred {
                result.insert("pending".into(), json!(true));
            }
        }
        "select" => {
            let (page, v) = find_visual(report, p)?;
            let ctrl = p.get("ctrl").and_then(Value::as_bool).unwrap_or(false);
            // the visual as shown (its drill level): the point's columns index its data
            let shown_full = next.shown_visual(page, v);
            match p.get("point").filter(|x| !x.is_null()) {
                None => {
                    // a click on the visual's background clears its own selection
                    if next
                        .selections
                        .get(&page.name)
                        .is_some_and(|x| x.visual == v.name)
                    {
                        next.selections.remove(&page.name);
                    }
                }
                Some(point) => {
                    // several points selected together (a histogram bin, a box, a Venn set, a
                    // Sankey node of BIcraft's visuals: every row of it), else the one point
                    let group: Vec<Vec<Pv>> = match p.get("points").and_then(Value::as_array) {
                        Some(list) if list.len() > 1 => {
                            if list.len() > MAX_POINTS {
                                return Err(format!("`points` holds at most {MAX_POINTS} points"));
                            }
                            list.iter()
                                .map(|x| resolve_point(&shown_full, x, s.model.as_ref()))
                                .collect::<Result<_, _>>()?
                        }
                        _ => vec![resolve_point(&shown_full, point, s.model.as_ref())?],
                    };
                    let point = group
                        .first()
                        .cloned()
                        .ok_or("a point has 1 grouping value or more")?;
                    let key = (page.name.clone(), v.name.clone());
                    // drill mode: a click drills into the point (at the lowest level it selects)
                    let mut drilled = false;
                    if let Some(mut d) = next.drills.get(&key).filter(|d| d.mode).cloned() {
                        let shown = displayed(s.model.as_ref(), &shown_full);
                        if drill::down(&mut d, &shown, &point).is_ok() {
                            next.drills.insert(key, d);
                            if next
                                .selections
                                .get(&page.name)
                                .is_some_and(|x| x.visual == v.name)
                            {
                                next.selections.remove(&page.name);
                            }
                            result.insert("drilled".into(), json!(true));
                            drilled = true;
                        }
                    }
                    if !drilled {
                        if group.len() > 1 {
                            select_points(&mut next, &page.name, &v.name, group, ctrl);
                        } else {
                            select_point(&mut next, &page.name, &v.name, point, ctrl);
                        }
                    }
                }
            }
        }
        "drill_down" | "drill_up" | "drill_next" | "drill_expand" | "drill_mode" | "matrix_expand"
        | "matrix_collapse" => {
            let (page, v) = find_visual(report, p)?;
            let role = p.get("role").and_then(Value::as_str);
            let key = (page.name.clone(), v.name.clone());
            let model = s.model.as_ref();
            let mut d = match next.drills.get(&key) {
                Some(d) if role.is_none_or(|r| d.hier.role.eq_ignore_ascii_case(r)) => d.clone(),
                _ => {
                    let h = drill::hierarchy(model, v, role).ok_or_else(|| {
                        format!(
                            "visual `{}` has no drill hierarchy (two or more columns in a drill role: an axis, \
                             matrix rows or columns)",
                            v.name
                        )
                    })?;
                    drill::stored(v, &h)
                }
            };
            let shown_full = next.shown_visual(page, v);
            let shown = displayed(s.model.as_ref(), &shown_full);
            match action {
                "drill_down" => {
                    let point = p.get("point").filter(|x| !x.is_null()).ok_or(
                        "`point` is required (the data point drilled into); drill_mode turns drill mode on",
                    )?;
                    let point = resolve_point(&shown_full, point, model)?;
                    drill::down(&mut d, &shown, &point)?;
                }
                "drill_up" => drill::up(&mut d)?,
                "drill_next" => drill::next(&mut d)?,
                "drill_expand" => drill::expand_all(&mut d)?,
                "drill_mode" => {
                    d.mode = match p.get("on") {
                        None | Some(Value::Null) => !d.mode,
                        Some(Value::Bool(b)) => *b,
                        Some(_) => return Err("`on` must be true or false".into()),
                    };
                }
                _ => drill::matrix(&mut d, &shown_full, action == "matrix_expand", p)?,
            }
            // a selection in the visual names points of levels it may no longer show
            if !matches!(action, "drill_mode" | "matrix_expand" | "matrix_collapse")
                && next
                    .selections
                    .get(&page.name)
                    .is_some_and(|x| x.visual == v.name)
            {
                next.selections.remove(&page.name);
            }
            next.drills.insert(key, d);
        }
        "drillthrough" => {
            let (page, v) = find_visual(report, p)?;
            let target = find_page(report, p.get("target"))?;
            if drill::page_kind(target) != Some("Drillthrough") {
                return Err(format!(
                    "page `{}` is not a drillthrough page",
                    target.display_name
                ));
            }
            if drill::cross_report(target) {
                return Err(
                    "cross-report drillthrough is a Power BI service feature; the viewer does not follow it"
                        .into(),
                );
            }
            let point = p
                .get("point")
                .filter(|x| !x.is_null())
                .ok_or("`point` is required (the data point drilled through)")?;
            let shown_full = next.shown_visual(page, v);
            let point = resolve_point(&shown_full, point, s.model.as_ref())?;
            next.drill_through(report, page, &v.name, &point, target)?;
            result.insert("page".into(), json!(target.name));
        }
        "tooltip" => match p.get("point").filter(|x| !x.is_null()) {
            None => next.tip = None,
            Some(pt) => {
                let (page, v) = find_visual(report, p)?;
                let normal = s.model.as_ref().and_then(|m| crate::preview::normalized(m, v));
                let target = drill::tooltip_page(report, normal.as_ref().unwrap_or(v))
                    .ok_or("the visual has no report-page tooltip")?;
                let shown_full = next.shown_visual(page, v);
                let point = resolve_point(&shown_full, pt, s.model.as_ref())?;
                // a line chart's legend value does not filter its tooltip page (Microsoft Learn)
                let shown = displayed(s.model.as_ref(), &shown_full);
                let skip: Vec<usize> = if v.visual_type.as_deref() == Some("lineChart") {
                    shown
                        .fields
                        .iter()
                        .enumerate()
                        .filter(|(_, b)| {
                            b.role.eq_ignore_ascii_case("Series") || b.role.eq_ignore_ascii_case("Legend")
                        })
                        .map(|(i, _)| i)
                        .collect()
                } else {
                    Vec::new()
                };
                let mut filters = drill::point_filters(&point, &skip);
                let mut base = next.clone();
                base.tip = None;
                filters.extend(match base.effective(report) {
                    Some(e) => drill::context(&e.data, &page.name, &v.name),
                    None => drill::context(report, &page.name, &v.name),
                });
                next.tip = Some(drill::Tip {
                    page: page.name.clone(),
                    visual: v.name.clone(),
                    point: pt.clone(),
                    target: target.name.clone(),
                    filters,
                });
            }
        },
        "key_influencers" => {
            next.key_influencers = match p.get("mode").and_then(Value::as_str) {
                Some("estimate") => true,
                Some("placeholder") => false,
                _ => return Err("`mode` must be \"estimate\" or \"placeholder\"".into()),
            };
        }
        "decomp_path" | "decomp_expand" | "decomp_collapse" | "decomp_scroll" => {
            let (page, v) = find_visual(report, p)?;
            if v.visual_type.as_deref() != Some("decompositionTreeVisual") {
                return Err(format!("visual `{}` is not a decomposition tree", v.name));
            }
            let key = (page.name.clone(), v.name.clone());
            let start = next
                .trees
                .get(&key)
                .cloned()
                .unwrap_or_else(|| crate::preview::decomp_state(v));
            let t = crate::preview::decomp_action(v, start, action, p)?;
            // a tree changed in the view: its old selection no longer names its nodes (a scroll
            // shows the same ones)
            if action != "decomp_scroll"
                && next
                    .selections
                    .get(&page.name)
                    .is_some_and(|x| x.visual == v.name)
            {
                next.selections.remove(&page.name);
            }
            next.trees.insert(key, t);
        }
        "bookmark_deselect" => {
            // a bookmark navigator with Allow deselection and no deselection bookmark: no
            // bookmark shows as selected any more; the view is left as it is
            next.bookmark = None;
        }
        "bookmark" => {
            let b = p
                .get("bookmark")
                .and_then(Value::as_str)
                .ok_or("`bookmark` is required (a bookmark's name or display name)")?;
            result.extend(bookmark::apply(&mut next, report, s.model.as_ref(), b)?);
        }
        "focus" => match p.get("visual").filter(|x| !x.is_null()) {
            None => {
                crate::params::allow(p, &["action", "page", "visual"])?;
                next.focus = None;
            }
            Some(_) => {
                crate::params::allow(p, &["action", "page", "visual"])?;
                let (page, v) = focusable(report, p)?;
                // focusing the visual already shown as a table keeps its table
                let table = next
                    .focus
                    .as_ref()
                    .is_some_and(|f| f.page == page && f.visual == v && f.table);
                next.focus = Some(Focus {
                    page,
                    visual: v,
                    table,
                });
            }
        },
        "show_table" => {
            crate::params::allow(p, &["action", "page", "visual", "on"])?;
            let on = match p.get("on") {
                None | Some(Value::Null) => true,
                Some(Value::Bool(b)) => *b,
                Some(_) => return Err("`on` must be true or false".into()),
            };
            let (page, v) = focusable(report, p)?;
            if on {
                next.focus = Some(Focus {
                    page,
                    visual: v,
                    table: true,
                });
            } else if let Some(f) = next.focus.as_mut().filter(|f| f.page == page && f.visual == v) {
                // off: the visual stays in focus, without its table
                f.table = false;
            }
        }
        "spotlight" => reader::spotlight(&mut next, report, p)?,
        "sort" => reader::sort(&mut next, report, s.model.as_ref(), p)?,
        "tree_toggle" => reader::tree_toggle(&mut next, report, p)?,
        "filter" | "filter_clear" => {
            reader::filter(&mut next, report, s.model.as_ref(), p, action == "filter_clear")?
        }
        "clear_selection" => {
            let page = find_page(report, p.get("page"))?;
            next.selections.remove(&page.name);
        }
        "clear_slicers" => {
            let page = find_page(report, p.get("page"))?;
            next.clear_slicers(page);
        }
        "apply_slicers" => {
            let page = find_page(report, p.get("page"))?;
            next.apply_slicers(&page.name);
        }
        "button" => {
            crate::params::allow(p, &["action", "page", "visual", "destination", "ctrl"])?;
            let (page, v) = find_visual(report, p)?;
            let mut link = visual_link(v).ok_or("the visual has no button action")?;
            // a conditional (fx) destination: the reader sends the value it shows (the snapshot's
            // `interaction.button.destination`); a fixed one is never replaced
            if let Some(d) = p.get("destination").filter(|d| !d.is_null()) {
                if !link.conditional {
                    return Err("`destination` is for a button whose destination is conditional (fx)".into());
                }
                let d = d.as_str().ok_or("`destination` must be a text")?;
                if d.chars().count() > 2000 {
                    return Err("`destination` is too long".into());
                }
                match link.kind.as_str() {
                    "PageNavigation" | "Drillthrough" => link.page = Some(d.to_string()),
                    "Bookmark" => link.bookmark = Some(d.to_string()),
                    "WebUrl" => link.url = Some(d.to_string()).filter(|u| safe_url(u)),
                    _ => {}
                }
            }
            result.insert("button".into(), json!(link.kind));
            match link.kind.as_str() {
                "PageNavigation" => {
                    let target = link.page.ok_or("the button has no destination page")?;
                    let target = find_page(report, Some(&json!(target)))?;
                    next.go_to(report, &target.name);
                }
                "Back" => next.go_back(report),
                "ApplyAllSlicers" => next.apply_slicers(&page.name),
                "ClearAllSlicers" => next.clear_slicers(page),
                "Drillthrough" => {
                    // the page's selected data point is the context (Desktop greys the button without one)
                    let target = link.page.ok_or("the button has no drillthrough page")?;
                    let target = find_page(report, Some(&json!(target)))?;
                    if drill::page_kind(target) != Some("Drillthrough") {
                        return Err(format!(
                            "page `{}` is not a drillthrough page",
                            target.display_name
                        ));
                    }
                    if drill::cross_report(target) {
                        return Err("cross-report drillthrough is a Power BI service feature; the viewer does not follow it".into());
                    }
                    let sel = next
                        .selections
                        .get(&page.name)
                        .filter(|x| x.points.len() == 1)
                        .cloned()
                        .ok_or(
                            "select one data point first: a drillthrough button drills through the selection",
                        )?;
                    let point = sel.points.first().cloned().unwrap_or_default();
                    next.drill_through(report, page, &sel.visual, &point, target)?;
                    result.insert("page".into(), json!(target.name));
                }
                "WebUrl" => {
                    // the front end opens the link; nothing changes here
                    let url = link.url.ok_or("the button has no web URL")?;
                    result.insert("url".into(), json!(url));
                }
                "Bookmark" => {
                    let b = link.bookmark.ok_or("the button has no bookmark")?;
                    result.extend(bookmark::apply(&mut next, report, s.model.as_ref(), &b)?);
                }
                "Qna" => {
                    return Err("Q&A is not available in BIcraft".into());
                }
                other => {
                    return Err(format!("button action `{other}` is not supported by the viewer"));
                }
            }
        }
        other => return Err(format!("unknown action `{other}` (see the command's params)")),
    }
    let changed = !same_state(&next, &*current);
    if changed {
        next.seq = next.seq.saturating_add(1);
    }
    *current = next;
    result.insert("action".into(), json!(action));
    result.insert("changed".into(), json!(changed));
    result.insert("state".into(), current.state_json());
    Ok(Value::Object(result))
}

/// `view.interact {action: "view_as", roles: [...] | null, user?}`: Desktop's View as. Every
/// role must be one of the model's (case-insensitive; an unknown one is an error and nothing
/// changes); `roles: null` (or no role) without a user stops viewing. The model, the report, the
/// revision and the unsaved-changes flags are never changed.
fn view_as_action(s: &Session, current: &mut Interactions, p: &Value) -> Result<Value, String> {
    let model = s.model.as_ref().ok_or("no model is open")?;
    let mut roles: Vec<String> = Vec::new();
    match p.get("roles") {
        None | Some(Value::Null) => {}
        Some(Value::Array(list)) => {
            if list.len() > MAX_VIEW_AS_ROLES {
                return Err(format!("`roles` takes at most {MAX_VIEW_AS_ROLES} roles"));
            }
            for r in list {
                let name = r.as_str().ok_or("`roles` must be a list of role names")?;
                let role = model
                    .roles
                    .iter()
                    .find(|x| x.name == name)
                    .or_else(|| model.roles.iter().find(|x| x.name.eq_ignore_ascii_case(name)))
                    .ok_or_else(|| format!("the model has no role `{name}`"))?;
                if !roles.contains(&role.name) {
                    roles.push(role.name.clone());
                }
            }
        }
        Some(_) => return Err("`roles` must be a list of role names, or null".into()),
    }
    let user = match p.get("user") {
        None | Some(Value::Null) => None,
        Some(Value::String(u)) => {
            let u = u.trim();
            if u.chars().count() > MAX_VIEW_AS_USER {
                return Err(format!("`user` takes at most {MAX_VIEW_AS_USER} characters"));
            }
            (!u.is_empty()).then(|| u.to_string())
        }
        Some(_) => return Err("`user` must be a text, or null".into()),
    };
    let next = (!roles.is_empty() || user.is_some()).then_some(ViewAs { roles, user });
    let changed = next != current.view_as;
    if changed {
        if next.is_none() {
            s.preview.drop_secured();
        }
        current.view_as = next;
        current.seq = current.seq.saturating_add(1);
    }
    Ok(json!({"action": "view_as", "changed": changed, "state": current.state_json()}))
}

/// `view.keep_drill {page?, visual?}`: writes the reader's drill state (displayed levels, drill
/// filters, a matrix's expansion) into the report as Desktop saves it, for every drilled visual
/// or the one named; those visuals then show the saved state and their reader state is dropped.
pub(crate) fn view_keep_drill(s: &mut Session, p: &Value) -> Result<Value, String> {
    crate::params::allow(p, &["page", "visual"])?;
    let report = s.report.as_ref().ok_or("the session has no report")?;
    let only: Option<(String, String)> = match p.get("visual").and_then(Value::as_str) {
        Some(_) => {
            let (page, v) = find_visual(report, p)?;
            Some((page.name.clone(), v.name.clone()))
        }
        None => None,
    };
    let page_only = match (&only, p.get("page")) {
        (None, Some(x)) if !x.is_null() => Some(find_page(report, Some(x))?.name.clone()),
        _ => None,
    };
    let mut keys: Vec<(String, String)> = s
        .interactions
        .drills
        .keys()
        .filter(|k| only.as_ref().is_none_or(|o| o == *k))
        .filter(|k| page_only.as_ref().is_none_or(|pg| *pg == k.0))
        .cloned()
        .collect();
    keys.sort();
    let mut kept = Vec::new();
    if let Some(r) = s.report.as_mut() {
        for k in &keys {
            let Some(d) = s.interactions.drills.get(k) else {
                continue;
            };
            let Some(v) = r
                .pages
                .iter_mut()
                .find(|pg| pg.name == k.0)
                .and_then(|pg| pg.visuals.iter_mut().find(|v| v.name == k.1))
            else {
                continue;
            };
            let before = v.clone();
            drill::apply(v, d);
            desktop_drill_names(v, &k.0, &k.1);
            if *v != before {
                kept.push(json!({"page": k.0, "visual": k.1}));
            }
        }
    }
    for k in &keys {
        s.interactions.drills.remove(k);
    }
    if !keys.is_empty() {
        s.interactions.seq = s.interactions.seq.saturating_add(1);
    }
    if !kept.is_empty() {
        s.report_dirty = true;
    }
    Ok(json!({"kept": kept, "changed": !kept.is_empty()}))
}

/// Gives the `Drill` filters `drill::apply` wrote on visual `visual` of `page` Desktop's kind of
/// names (stable ids), not the viewer's.
fn desktop_drill_names(v: &mut Visual, page: &str, visual: &str) {
    let Some(list) = v
        .filter_config
        .as_mut()
        .and_then(|c| c.get_mut("filters"))
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for f in list.iter_mut() {
        let is_drill = f.get("howCreated").and_then(Value::as_str) == Some("Drill");
        let name = f
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if is_drill
            && let Some(rest) = name.strip_prefix("bicraft-drill-")
            && let Some(o) = f.as_object_mut()
        {
            let id = crate::report_apply::stable_name(&format!("{page}/{visual}/drill/{rest}"));
            o.insert("name".into(), json!(id));
        }
    }
}

/// `view.interactions`: the state as JSON.
pub(crate) fn view_interactions(s: &Session, p: &Value) -> Result<Value, String> {
    crate::params::allow(p, &[])?;
    Ok(s.interactions.state_json())
}

fn same_state(a: &Interactions, b: &Interactions) -> bool {
    a.page == b.page
        && a.back == b.back
        && a.applied == b.applied
        && a.pending == b.pending
        && a.selections == b.selections
        && a.key_influencers == b.key_influencers
        && a.view_as == b.view_as
        && a.trees == b.trees
        && a.hidden == b.hidden
        && a.filters == b.filters
        && a.bookmark == b.bookmark
        && a.focus == b.focus
        && a.spotlight == b.spotlight
        && a.sorts == b.sorts
        && a.nodes == b.nodes
        && drill::same(a, b)
}

/// The page and visual names of `p`'s visual when it can be shown in focus mode (not a group).
fn focusable(r: &Report, p: &Value) -> Result<(String, String), String> {
    let (page, v) = find_visual(r, p)?;
    if v.visual_type.is_none() {
        return Err(format!(
            "`{}` is a visual group; focus mode shows one visual",
            v.name
        ));
    }
    Ok((page.name.clone(), v.name.clone()))
}

impl Interactions {
    /// `v` with its drill state (the displayed levels' `active` flags, drill filters, a matrix's
    /// expansion), as the data report evaluates it.
    fn shown_visual(&self, page: &Page, v: &Visual) -> Visual {
        let mut x = v.clone();
        if let Some(d) = self.drills.get(&(page.name.clone(), v.name.clone())) {
            drill::apply(&mut x, d);
        }
        x
    }

    /// Adds the drill fields to a snapshot: each visual with a drill hierarchy gets `drill`
    /// (its levels, the displayed ones, drill mode, what can be done), `interaction.drillthrough`
    /// (the drillthrough pages a point of it reaches, with the data columns that feed them) and
    /// `interaction.tooltip_page` (its report-page tooltip); `interaction.tooltip` at the top
    /// says which hovered point the tooltip page shows.
    pub(crate) fn annotate_drill(&self, model: Option<&SemanticModel>, r: &Report, snap: &mut Value) {
        if let Some(o) = snap.get_mut("interaction").and_then(Value::as_object_mut) {
            o.insert(
                "tooltip".into(),
                self.tip.as_ref().map_or(Value::Null, drill::tip_json),
            );
        }
        let Some(pages) = snap.pointer_mut("/report/pages").and_then(Value::as_array_mut) else {
            return;
        };
        for p in pages {
            let Some(page_name) = p.get("name").and_then(Value::as_str).map(str::to_string) else {
                continue;
            };
            let Some(page) = r.pages.iter().find(|x| x.name == page_name) else {
                continue;
            };
            let Some(visuals) = p.get_mut("visuals").and_then(Value::as_array_mut) else {
                continue;
            };
            for vj in visuals {
                let Some(name) = vj.get("name").and_then(Value::as_str).map(str::to_string) else {
                    continue;
                };
                let Some(v) = page.visuals.iter().find(|t| t.name == name) else {
                    continue;
                };
                if v.fields.is_empty() {
                    continue;
                }
                let key = (page_name.clone(), name.clone());
                let d = match self.drills.get(&key) {
                    Some(d) => Some(d.clone()),
                    None => drill::hierarchy(model, v, None).map(|h| drill::stored(v, &h)),
                };
                let Some(o) = vj.as_object_mut() else { continue };
                if let Some(d) = &d {
                    o.insert("drill".into(), drill::drill_json(d, false));
                }
                let shown_full = self.shown_visual(page, v);
                let shown = displayed(model, &shown_full);
                let targets: Vec<Value> = drill::drillthrough_targets(r, &shown)
                    .into_iter()
                    .filter(|(t, _)| t.name != page.name)
                    .map(|(t, cols)| {
                        json!({"page": t.name, "label": t.display_name, "columns": cols,
                               "cross_report": drill::cross_report(t)})
                    })
                    .collect();
                let normal = model.and_then(|m| crate::preview::normalized(m, v));
                let tip = drill::tooltip_page(r, normal.as_ref().unwrap_or(v)).map(|t| t.name.clone());
                if targets.is_empty() && tip.is_none() {
                    continue;
                }
                let info = o
                    .entry("interaction".to_string())
                    .or_insert_with(|| Value::Object(Map::new()));
                if let Some(info) = info.as_object_mut() {
                    if !targets.is_empty() {
                        info.insert("drillthrough".into(), Value::Array(targets));
                    }
                    if let Some(t) = tip {
                        info.insert("tooltip_page".into(), json!(t));
                    }
                }
            }
        }
    }

    /// Drills through from `visual` of `page` on `point` to the drillthrough page `target`: its
    /// drillthrough filters take the point's values, "Keep all filters" carries the visual's
    /// filters, and the reader moves there (Back returns).
    fn drill_through(
        &mut self,
        report: &Report,
        page: &Page,
        visual: &str,
        point: &[Pv],
        target: &Page,
    ) -> Result<(), String> {
        let mut values = Vec::new();
        for (name, table, column, agg) in drill::page_fields(target) {
            if agg {
                continue;
            }
            if let Some(x) = point
                .iter()
                .find(|x| x.table.eq_ignore_ascii_case(&table) && x.column.eq_ignore_ascii_case(&column))
            {
                let f = in_filter(
                    &x.table,
                    std::slice::from_ref(&x.column),
                    &[vec![x.value.clone()]],
                    &[x.ty],
                    false,
                )?;
                values.push((name, f));
            }
        }
        if values.is_empty() {
            return Err(format!(
                "the point holds none of the drillthrough fields of page `{}`",
                target.display_name
            ));
        }
        let context = if drill::keep_all(target) {
            match self.effective(report) {
                Some(e) => drill::context(&e.data, &page.name, visual),
                None => drill::context(report, &page.name, visual),
            }
        } else {
            Vec::new()
        };
        self.through.insert(
            target.name.clone(),
            drill::Through {
                source: (page.name.clone(), visual.to_string()),
                values,
                context,
            },
        );
        // the reader is on the source page: Back returns there
        self.page = Some(page.name.clone());
        self.go_to(report, &target.name);
        Ok(())
    }

    fn go_to(&mut self, r: &Report, page: &str) {
        let current = self
            .page
            .clone()
            .or_else(|| r.active_page.clone())
            .or_else(|| r.pages.first().map(|p| p.name.clone()));
        if current.as_deref() == Some(page) {
            return;
        }
        // another page: focus mode is left (as Desktop's Back to report), spotlight too
        self.focus = None;
        self.spotlight = None;
        if let Some(c) = current {
            self.back.push(c);
            if self.back.len() > MAX_BACK {
                self.back.remove(0);
            }
        }
        self.page = Some(page.to_string());
    }

    fn go_back(&mut self, r: &Report) {
        while let Some(p) = self.back.pop() {
            if r.pages.iter().any(|x| x.name == p) {
                self.page = Some(p);
                self.focus = None;
                self.spotlight = None;
                return;
            }
        }
    }

    /// "Clear all slicers": every slicer of the page loses its selection, pending ones too.
    fn clear_slicers(&mut self, page: &Page) {
        for v in page.visuals.iter().filter(|v| filters::is_slicer(v)) {
            let key = (page.name.clone(), v.name.clone());
            self.pending.remove(&key);
            let mut st = cleared();
            st.data.extend(
                RANGE_PROPS
                    .iter()
                    .chain(RELATIVE_PROPS.iter())
                    .map(|k| ((*k).to_string(), None)),
            );
            if stored_state(v).filter.is_none() && !has_data_props(v) {
                self.applied.remove(&key);
            } else {
                self.applied.insert(key, st);
            }
        }
    }

    /// Drops the reader's selection of slicer (`page`, `visual`), applied and pending, so it
    /// shows the stored one (`bookmark.apply` restored it). True when there was one.
    pub(crate) fn forget_slicer(&mut self, page: &str, visual: &str) -> bool {
        let key = (page.to_string(), visual.to_string());
        let had = self.applied.remove(&key).is_some() | self.pending.remove(&key).is_some();
        if had {
            self.seq = self.seq.saturating_add(1);
        }
        had
    }

    /// "Apply all slicers": the page's pending selections become the applied ones.
    fn apply_slicers(&mut self, page: &str) {
        let keys: Vec<(String, String)> = self.pending.keys().filter(|(p, _)| p == page).cloned().collect();
        for k in keys {
            if let Some(st) = self.pending.remove(&k) {
                self.applied.insert(k, st);
            }
        }
    }
}

fn cleared() -> SlicerState {
    SlicerState::default()
}

fn has_data_props(v: &Visual) -> bool {
    general_or_data(v, "data").iter().any(|e| {
        e.get("properties").and_then(Value::as_object).is_some_and(|p| {
            RANGE_PROPS
                .iter()
                .chain(RELATIVE_PROPS.iter())
                .any(|k| p.contains_key(*k))
        })
    })
}

// ---------------------------------------------------------------------------------------------
// Lookups

fn find_page<'a>(r: &'a Report, which: Option<&Value>) -> Result<&'a Page, String> {
    let which = which.ok_or("`page` is required")?;
    if let Some(i) = which.as_u64() {
        return usize::try_from(i)
            .ok()
            .and_then(|i| r.pages.get(i))
            .ok_or_else(|| format!("no page {i}"));
    }
    let name = which.as_str().ok_or("`page` must be a page name or index")?;
    r.pages
        .iter()
        .find(|p| p.name == name)
        .or_else(|| r.pages.iter().find(|p| p.display_name == name))
        .ok_or_else(|| format!("no page `{name}`"))
}

pub(crate) fn find_visual<'a>(r: &'a Report, p: &Value) -> Result<(&'a Page, &'a Visual), String> {
    let page = find_page(r, p.get("page"))?;
    let name = p
        .get("visual")
        .and_then(Value::as_str)
        .ok_or("`visual` is required")?;
    let v = page
        .visuals
        .iter()
        .find(|v| v.name == name)
        .ok_or_else(|| format!("no visual `{name}` on page `{}`", page.display_name))?;
    Ok((page, v))
}

/// The interaction type the page sets from `source` to `target` (`visualInteractions`).
pub(crate) fn interaction_type<'a>(page: &'a Page, source: &str, target: &str) -> Option<&'a str> {
    page.extra
        .get("visualInteractions")
        .and_then(Value::as_array)?
        .iter()
        .find(|i| {
            i.get("source").and_then(Value::as_str) == Some(source)
                && i.get("target").and_then(Value::as_str) == Some(target)
        })?
        .get("type")
        .and_then(Value::as_str)
}

/// Whether a selection in `source` reaches `target`: every other visual with fields, unless the
/// page turns the interaction off (`NoFilter`). With the default interactions off (`off`: the
/// query reduction option, `optimize::interactions_off`) only the visuals a rule names.
fn reaches(page: &Page, source: &str, target: &Visual, off: bool) -> bool {
    target.name != source
        && !target.fields.is_empty()
        && match interaction_type(page, source, &target.name) {
            Some(t) => t != "NoFilter",
            None => !off,
        }
}

/// How a selection reaches a visual.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Filter,
    Highlight,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Mode::Filter => "filter",
            Mode::Highlight => "highlight",
        }
    }
}

/// Built-in visual types that highlight: the ones Desktop 2.160 highlighted in the highlight probe
/// (testsupport/interaction_oracle/highlight_types.json, docs/interaction-oracle.md rule 9). Types
/// the probe did not settle (Q&A, maps) are filtered.
const HIGHLIGHT_TYPES: [&str; 14] = [
    "barChart",
    "clusteredBarChart",
    "hundredPercentStackedBarChart",
    "columnChart",
    "clusteredColumnChart",
    "hundredPercentStackedColumnChart",
    "lineClusteredColumnComboChart",
    "lineStackedColumnComboChart",
    "ribbonChart",
    "pieChart",
    "donutChart",
    "funnel",
    "treemap",
    "decompositionTreeVisual",
];

/// Whether visuals of type `ty` can highlight.
pub(crate) fn supports_highlight(ty: Option<&str>) -> bool {
    ty.is_some_and(|t| HIGHLIGHT_TYPES.iter().any(|h| h.eq_ignore_ascii_case(t)))
}

/// Whether visual `v` can highlight: a built-in type that can, or one of BIcraft's visuals that
/// declares `supportsHighlight` (a measure role; with no field in it, it stays as it is:
/// docs/formats/bicraft-visuals.md "Highlights").
pub(crate) fn can_highlight(v: &Visual) -> bool {
    supports_highlight(v.visual_type.as_deref()) || crate::builtin_visuals::highlights(v)
}

/// How a data-point selection reaches `target` (oracle rules 9 and 14): `DataFilter` filters;
/// `HighlightFilter` highlights a visual that can highlight; no entry follows the report's
/// default (`cross_filter`: the report setting `defaultFilterActionIsDataFilter`), highlight
/// otherwise. A visual that cannot highlight is always filtered.
fn mode_of(page: &Page, source: &str, target: &Visual, cross_filter: bool) -> Mode {
    let explicit = interaction_type(page, source, &target.name);
    if explicit.is_some_and(|t| t.eq_ignore_ascii_case("DataFilter") || t.eq_ignore_ascii_case("Filter")) {
        return Mode::Filter;
    }
    if !can_highlight(target) {
        return Mode::Filter;
    }
    let highlight = explicit.is_some_and(|t| t.eq_ignore_ascii_case("HighlightFilter"));
    if cross_filter && !highlight {
        Mode::Filter
    } else {
        Mode::Highlight
    }
}

/// The report setting "Change default visual interaction from cross highlighting to cross
/// filtering" (`settings.defaultFilterActionIsDataFilter`).
pub(crate) fn cross_filter_default(r: &Report) -> bool {
    r.extra
        .get("settings")
        .and_then(|s| s.get("defaultFilterActionIsDataFilter"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// The report setting "Drilling filters other visuals" (`settings.defaultDrillFilterOtherVisuals`;
/// Desktop writes `true` in new reports, an absent setting reads `false`).
pub(crate) fn drill_filters_default(r: &Report) -> bool {
    r.extra
        .get("settings")
        .and_then(|s| s.get("defaultDrillFilterOtherVisuals"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// What the Edit interactions mode offers for a visual (`interactivity` in the snapshot,
/// docs/view-snapshot.md "Interactions"): whether it takes part (`target`: a visual with fields,
/// so a source or a target), whether it can highlight (`highlight`, the same table as
/// [`mode_of`]), and whether it is a slicer (a slicer's selection only filters or does nothing).
pub(crate) fn interactivity(v: &Visual) -> Value {
    json!({
        "target": v.visual_type.is_some() && !v.fields.is_empty(),
        "highlight": can_highlight(v),
        "slicer": filters::is_slicer(v),
    })
}

/// The page's `visualInteractions` in `page.interactions`' words (`filter`, `highlight`,
/// `none`; another type is kept as written), at most `max`.
pub(crate) fn page_rules(page: &Page, max: usize) -> Value {
    let list = page
        .extra
        .get("visualInteractions")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    Value::Array(
        list.iter()
            .take(max)
            .filter_map(|i| {
                let s = i.get("source")?.as_str()?;
                let t = i.get("target")?.as_str()?;
                let ty = i.get("type")?.as_str()?;
                let word = match ty {
                    "DataFilter" => "filter",
                    "HighlightFilter" => "highlight",
                    "NoFilter" => "none",
                    other => other,
                };
                Some(json!({"source": s, "target": t, "type": word}))
            })
            .collect(),
    )
}

/// A button's action: `visualLink` (`visualContainerObjects`, or `objects` in older files).
pub(crate) struct Link {
    pub kind: String,
    /// `navigationSection`, else `drillthroughSection` (Drillthrough).
    pub page: Option<String>,
    /// `webUrl` (WebUrl), only http, https and mailto links.
    pub url: Option<String>,
    /// `bookmark` (Bookmark).
    pub bookmark: Option<String>,
    /// The destination (page, bookmark or URL) is a conditional (fx) expression, evaluated by
    /// the snapshot (`action.dynamic_destination`).
    pub conditional: bool,
    /// `tooltip`, `enabledTooltip`, `disabledTooltip` (literal texts).
    pub tooltip: Option<String>,
    pub enabled_tooltip: Option<String>,
    pub disabled_tooltip: Option<String>,
}

/// An http, https or mailto link without control characters, at most 2000 bytes.
fn safe_url(u: &str) -> bool {
    let l = u.trim().to_ascii_lowercase();
    (l.starts_with("http://") || l.starts_with("https://") || l.starts_with("mailto:"))
        && u.len() <= 2000
        && !u.chars().any(char::is_control)
}

/// The snapshot's `interaction.button` of a visual with an action: `{kind, page, url?, bookmark?,
/// conditional?, destination?, enabled, tooltip?}`. A conditional destination is the value the
/// snapshot evaluated (`action.dynamic_destination`, `v`), matched to a page or bookmark by name
/// or display name. A drillthrough button is enabled when the page has one selected data point
/// (Desktop's disabled state otherwise); a conditional destination that names nothing disables
/// the button. The tooltip: the conditional one, else the enabled or disabled tooltip, else the
/// plain one; left out when it repeats the button's text (Microsoft Learn, "Create buttons").
fn button_info(r: &Report, sel: Option<&Selection>, link: Link, v: &Value) -> Value {
    let dynamic = |k: &str| {
        v.get("action")
            .and_then(|a| a.get(k))
            .and_then(Value::as_str)
            .map(|s| s.chars().take(2000).collect::<String>())
    };
    let mut b = Map::new();
    b.insert("kind".into(), json!(link.kind));
    let mut enabled = true;
    let mut target = link.page.clone();
    let mut url = link.url.clone();
    let mut bookmark = link.bookmark.clone();
    if link.conditional {
        b.insert("conditional".into(), json!(true));
        let d = dynamic("dynamic_destination").filter(|d| !d.trim().is_empty());
        match link.kind.as_str() {
            "PageNavigation" | "Drillthrough" => {
                target = d.as_deref().and_then(|d| {
                    r.pages
                        .iter()
                        .find(|p| p.name == d)
                        .or_else(|| r.pages.iter().find(|p| p.display_name == d))
                        .map(|p| p.name.clone())
                });
            }
            "Bookmark" => {
                bookmark = d.as_deref().and_then(|d| {
                    crate::bookmarks::bookmark_index(r, d)
                        .ok()
                        .and_then(|i| r.bookmarks.get(i))
                        .map(|x| x.name.clone())
                });
            }
            "WebUrl" => url = d.filter(|u| safe_url(u)),
            _ => {}
        }
        let found = match link.kind.as_str() {
            "PageNavigation" | "Drillthrough" => target.is_some(),
            "Bookmark" => bookmark.is_some(),
            "WebUrl" => url.is_some(),
            _ => true,
        };
        enabled &= found;
        if let Some(t) = target.as_ref().or(bookmark.as_ref()).or(url.as_ref()) {
            b.insert("destination".into(), json!(t));
        }
    }
    if link.kind == "Drillthrough" {
        let one_point = sel.is_some_and(|s| s.points.len() == 1);
        let is_drill_page = target
            .as_deref()
            .and_then(|t| r.pages.iter().find(|p| p.name == t))
            .is_some_and(|p| drill::page_kind(p) == Some("Drillthrough") && !drill::cross_report(p));
        enabled &= one_point && is_drill_page;
    }
    b.insert("page".into(), json!(target));
    if let Some(u) = url {
        b.insert("url".into(), json!(u));
    }
    if let Some(bm) = bookmark {
        b.insert("bookmark".into(), json!(bm));
    }
    b.insert("enabled".into(), json!(enabled));
    let tip = dynamic("dynamic_tooltip")
        .or_else(|| {
            if enabled {
                link.enabled_tooltip.clone()
            } else {
                link.disabled_tooltip.clone()
            }
        })
        .or(link.tooltip.clone())
        .filter(|t| !t.trim().is_empty());
    let text = v.pointer("/button/text").and_then(Value::as_str).unwrap_or("");
    if let Some(t) = tip.filter(|t| t.trim() != text.trim()) {
        b.insert("tooltip".into(), json!(t));
    }
    Value::Object(b)
}

pub(crate) fn visual_link(v: &Visual) -> Option<Link> {
    for key in ["visualContainerObjects", "objects"] {
        let Some(list) = v
            .visual_extra
            .get(key)
            .and_then(|o| o.get("visualLink"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        let Some(props) = list
            .iter()
            .find(|e| e.get("selector").is_none_or(Value::is_null))
            .and_then(|e| e.get("properties"))
        else {
            continue;
        };
        if literal_text(props.get("show")).is_some_and(|s| s == "false") {
            return None;
        }
        let kind = literal_text(props.get("type")).map(|s| unquote(&s))?;
        let page = literal_text(props.get("navigationSection"))
            .or_else(|| literal_text(props.get("drillthroughSection")))
            .map(|s| unquote(&s));
        let url = literal_text(props.get("webUrl"))
            .map(|s| unquote(&s))
            .filter(|u| safe_url(u));
        let bookmark = literal_text(props.get("bookmark")).map(|s| unquote(&s));
        // an expression other than a literal: a conditional (fx) destination
        let conditional = ["navigationSection", "drillthroughSection", "bookmark", "webUrl"]
            .iter()
            .any(|k| {
                props
                    .get(*k)
                    .and_then(|x| x.get("expr"))
                    .is_some_and(|e| e.get("Literal").is_none())
            });
        let text = |k: &str| {
            literal_text(props.get(k))
                .map(|s| unquote(&s))
                .filter(|s| !s.is_empty())
        };
        return Some(Link {
            kind,
            page,
            url,
            bookmark,
            conditional,
            tooltip: text("tooltip"),
            enabled_tooltip: text("enabledTooltip"),
            disabled_tooltip: text("disabledTooltip"),
        });
    }
    None
}

fn literal_text(prop: Option<&Value>) -> Option<String> {
    prop?
        .get("expr")?
        .get("Literal")?
        .get("Value")?
        .as_str()
        .map(str::to_string)
}

fn unquote(s: &str) -> String {
    s.strip_prefix('\'')
        .and_then(|x| x.strip_suffix('\''))
        .map_or_else(|| s.to_string(), |x| x.replace("''", "'"))
}

/// Whether the page has a visible button with that `visualLink` type.
pub(crate) fn has_button(page: &Page, kind: &str) -> bool {
    page.visuals
        .iter()
        .any(|v| visual_link(v).is_some_and(|l| l.kind == kind))
}

/// The sync group a slicer's filter follows (`visual.syncGroup.groupName`, unless
/// `filterChanges` is off).
fn sync_group(v: &Visual) -> Option<String> {
    let g = v.visual_extra.get("syncGroup")?;
    if g.get("filterChanges").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    g.get("groupName")
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
}

/// The slicer's column: its first column field.
fn slicer_field(v: &Visual) -> Option<(String, String)> {
    v.fields.iter().find_map(|b| match &b.field {
        FieldRef::Column { table, column } => Some((table.clone(), column.clone())),
        _ => None,
    })
}

fn general_or_data<'a>(v: &'a Visual, object: &str) -> &'a [Value] {
    v.visual_extra
        .get("objects")
        .and_then(|o| o.get(object))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn data_type(model: Option<&SemanticModel>, table: &str, column: &str) -> DataType {
    model
        .and_then(|m| m.tables.iter().find(|t| t.name.eq_ignore_ascii_case(table)))
        .and_then(|t| t.columns.iter().find(|c| c.name.eq_ignore_ascii_case(column)))
        .map_or(DataType::Unknown, |c| c.data_type)
}

// ---------------------------------------------------------------------------------------------
// Slicers

/// The selection the file stores for `v`, as a starting point.
fn stored_state(v: &Visual) -> SlicerState {
    let sel = filters::selection_json(v);
    let values = sel
        .as_ref()
        .and_then(|s| s.get("selected"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let inverted = sel
        .as_ref()
        .and_then(|s| s.get("inverted"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    SlicerState {
        filter: filters::slicer_filter(v).cloned(),
        data: Vec::new(),
        values,
        inverted,
    }
}

/// Selection options of a slicer: (single select, Ctrl needed to multi-select).
fn selection_options(v: &Visual) -> (bool, bool) {
    let props = general_or_data(v, "selection")
        .iter()
        .find(|e| e.get("selector").is_none_or(Value::is_null))
        .and_then(|e| e.get("properties"));
    let flag = |name: &str| props.and_then(|p| literal_text(p.get(name))).map(|t| t == "true");
    (
        flag("strictSingleSelect").unwrap_or(false),
        flag("singleSelect").unwrap_or(true),
    )
}

/// Value identity for toggling: numbers by value, text without case (as the filters compare).
fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        // a date and its midnight are the same day (`2024-01-31`, `2024-01-31T00:00:00`)
        (Value::String(x), Value::String(y)) => {
            x.to_lowercase() == y.to_lowercase() || iso_serial(x).is_some_and(|d| iso_serial(y) == Some(d))
        }
        _ => a == b,
    }
}

fn slicer_action(
    action: &str,
    p: &Value,
    v: &Visual,
    field: &(String, String),
    current: SlicerState,
    model: Option<&SemanticModel>,
    today: f64,
) -> Result<SlicerState, String> {
    let (table, column) = field;
    let ty = data_type(model, table, column);
    let values_state = |values: Vec<Value>, inverted: bool| -> Result<SlicerState, String> {
        if values.is_empty() {
            return Ok(cleared());
        }
        let filter = in_filter(
            table,
            std::slice::from_ref(column),
            &values.iter().map(|x| vec![x.clone()]).collect::<Vec<_>>(),
            &[ty],
            inverted,
        )?;
        Ok(SlicerState {
            filter: Some(filter),
            data: Vec::new(),
            values,
            inverted,
        })
    };
    match action {
        "slicer_toggle" => {
            let value = p.get("value").ok_or("`value` is required")?.clone();
            let ctrl = p.get("ctrl").and_then(Value::as_bool).unwrap_or(false);
            let (single, ctrl_needed) = selection_options(v);
            let mut values = current.values;
            let mut inverted = current.inverted;
            let at = values.iter().position(|x| same_value(x, &value));
            if single {
                return values_state(vec![value], false);
            }
            if ctrl || !ctrl_needed {
                // add or remove the item (an excluded value becomes included again and back)
                match at {
                    Some(i) => {
                        values.remove(i);
                    }
                    None => {
                        if values.len() >= MAX_VALUES {
                            return Err(format!("a slicer selection holds at most {MAX_VALUES} values"));
                        }
                        values.push(value);
                    }
                }
                if values.is_empty() {
                    inverted = false;
                }
                return values_state(values, inverted);
            }
            // a plain click: that item alone, or nothing when it was the only one selected
            if !inverted && values.len() == 1 && at.is_some() {
                return Ok(cleared());
            }
            values_state(vec![value], false)
        }
        "slicer_set" => {
            let list = p
                .get("values")
                .and_then(Value::as_array)
                .ok_or("`values` must be an array")?;
            if list.len() > MAX_VALUES {
                return Err(format!("a slicer selection holds at most {MAX_VALUES} values"));
            }
            let inverted = p.get("inverted").and_then(Value::as_bool).unwrap_or(false);
            values_state(list.clone(), inverted)
        }
        "slicer_select_all" | "slicer_clear" => {
            let mut st = cleared();
            if action == "slicer_clear" {
                st.data
                    .extend(RANGE_PROPS.iter().map(|k| ((*k).to_string(), None)));
                st.data
                    .extend(RELATIVE_PROPS.iter().map(|k| ((*k).to_string(), None)));
            }
            Ok(st)
        }
        "slicer_text" => {
            let text = p.get("text").and_then(Value::as_str).unwrap_or_default();
            if text.chars().count() > MAX_TEXT {
                return Err(format!("the text is longer than {MAX_TEXT} characters"));
            }
            if text.is_empty() {
                return Ok(cleared());
            }
            let cond = json!({"Contains": {"Left": col_expr("t", column), "Right": literal(&quote(text))}});
            Ok(SlicerState {
                filter: Some(where_filter(&[("t", table)], vec![cond])),
                ..SlicerState::default()
            })
        }
        "slicer_range" => {
            let date = matches!(ty, DataType::DateTime);
            let bound = |k: &str| -> Result<Option<Value>, String> {
                match p.get(k) {
                    None | Some(Value::Null) => Ok(None),
                    Some(x @ Value::Number(_)) if !date => Ok(Some(x.clone())),
                    Some(Value::String(s)) if date && iso_serial(s).is_some() => Ok(Some(json!(s))),
                    Some(other) => Err(format!(
                        "`{k}` must be {} or null, got {other}",
                        if date { "an ISO date" } else { "a number" }
                    )),
                }
            };
            let (from, to) = (bound("from")?, bound("to")?);
            let mut conds = Vec::new();
            let mut data: Vec<(String, Option<Value>)> =
                RANGE_PROPS.iter().map(|k| ((*k).to_string(), None)).collect();
            if let Some(f) = &from {
                let (lit, prop) = range_literal(f, ty, false)?;
                conds.push(cmp(2, column, &lit));
                data.push((prop.into(), Some(json!({"expr": literal(&lit)}))));
            }
            if let Some(t) = &to {
                let (lit, prop) = range_literal(t, ty, true)?;
                // a date range includes its last day: `< next day`, as Desktop writes it
                let kind = if date { 3 } else { 4 };
                conds.push(cmp(kind, column, &lit));
                let shown = if date {
                    date_literal(t.as_str().unwrap_or_default(), 0)?
                } else {
                    lit.clone()
                };
                data.push((prop.into(), Some(json!({"expr": literal(&shown)}))));
            }
            if conds.is_empty() {
                let mut st = cleared();
                st.data = data;
                return Ok(st);
            }
            let cond = and_all(conds);
            Ok(SlicerState {
                filter: Some(where_filter(&[("t", table)], vec![cond])),
                data,
                values: Vec::new(),
                inverted: false,
            })
        }
        "slicer_relative" => {
            let range = p
                .get("range")
                .and_then(Value::as_str)
                .unwrap_or("last")
                .to_ascii_lowercase();
            let period = p
                .get("period")
                .and_then(Value::as_str)
                .ok_or("`period` is required")?
                .to_ascii_lowercase();
            let duration = p.get("duration").and_then(Value::as_f64).unwrap_or(1.0);
            if !(1.0..=10_000.0).contains(&duration) || duration.fract() != 0.0 {
                return Err("`duration` must be a whole number from 1 to 10000".into());
            }
            let (a, b) = filters::relative_days(&range, duration, &period, today)
                .ok_or_else(|| format!("unsupported relative date `{range} {duration} {period}`"))?;
            let from = date_literal(&filters::iso_day(a), 0)?;
            let to = date_literal(&filters::iso_day(b), 1)?;
            let cond = and_all(vec![cmp(2, column, &from), cmp(3, column, &to)]);
            let words = |s: &str| -> String {
                s.split_whitespace()
                    .map(|w| {
                        let mut c = w.chars();
                        c.next()
                            .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                            .unwrap_or_default()
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            let data = vec![
                (
                    "relativeRange".to_string(),
                    Some(json!({"expr": literal(&quote(&words(&range)))})),
                ),
                (
                    "relativeDuration".to_string(),
                    Some(json!({"expr": literal(&format!("{duration}L"))})),
                ),
                (
                    "relativePeriod".to_string(),
                    Some(json!({"expr": literal(&quote(&words(&period)))})),
                ),
            ];
            Ok(SlicerState {
                filter: Some(where_filter(&[("t", table)], vec![cond])),
                data,
                values: Vec::new(),
                inverted: false,
            })
        }
        _ => Err(format!("unknown slicer action `{action}`")),
    }
}

/// Writes a slicer state into the visual: `objects.general[0].properties.filter` and the
/// `objects.data` properties it sets or removes.
fn put_slicer(v: &mut Visual, st: &SlicerState) {
    let objects = v
        .visual_extra
        .entry("objects".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !objects.is_object() {
        *objects = Value::Object(Map::new());
    }
    let Some(objects) = objects.as_object_mut() else {
        return;
    };
    let general = props_entry(objects, "general");
    if let Some(g) = general {
        g.remove("filter");
        if let Some(f) = &st.filter {
            g.insert("filter".into(), json!({"filter": f}));
        }
    }
    // the stored filter may sit in a later entry: only the first one is read
    if let Some(list) = objects.get_mut("general").and_then(Value::as_array_mut) {
        for e in list.iter_mut().skip(1) {
            if let Some(p) = e.get_mut("properties").and_then(Value::as_object_mut) {
                p.remove("filter");
            }
        }
    }
    if !st.data.is_empty()
        && let Some(d) = props_entry(objects, "data")
    {
        for (k, val) in &st.data {
            match val {
                Some(x) => {
                    d.insert(k.clone(), x.clone());
                }
                None => {
                    d.remove(k);
                }
            }
        }
    }
}

/// The `properties` of the selector-less entry of `objects[name]`, created when missing.
fn props_entry<'a>(objects: &'a mut Map<String, Value>, name: &str) -> Option<&'a mut Map<String, Value>> {
    let list = objects
        .entry(name.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !list.is_array() {
        *list = Value::Array(Vec::new());
    }
    let list = list.as_array_mut()?;
    let at = match list
        .iter()
        .position(|e| e.get("selector").is_none_or(Value::is_null))
    {
        Some(i) => i,
        None => {
            list.insert(0, json!({"properties": {}}));
            0
        }
    };
    let entry = list.get_mut(at)?;
    if !entry.get("properties").is_some_and(Value::is_object)
        && let Some(e) = entry.as_object_mut()
    {
        e.insert("properties".into(), Value::Object(Map::new()));
    }
    entry.get_mut("properties")?.as_object_mut()
}

// ---------------------------------------------------------------------------------------------
// Data points

/// The grouping values of a clicked point: `[{c, v}]` with `c` a data column of the visual (at
/// its displayed drill level) bound to a model column; other entries are refused.
pub(crate) fn resolve_point(
    v: &Visual,
    point: &Value,
    model: Option<&SemanticModel>,
) -> Result<Vec<Pv>, String> {
    let list = point
        .as_array()
        .ok_or("`point` must be an array of {c, v} (data column, value)")?;
    if list.is_empty() || list.len() > MAX_POINT_COLUMNS {
        return Err(format!("a point has 1 to {MAX_POINT_COLUMNS} grouping values"));
    }
    let drilled = crate::preview::at_drill_level(v);
    let fv = drilled.as_ref().unwrap_or(v);
    // hierarchy levels read as their columns
    let normal = model.and_then(|m| crate::preview::normalized(m, fv));
    let fv = normal.as_ref().unwrap_or(fv);
    let mut out = Vec::new();
    for e in list {
        let c = e
            .get("c")
            .and_then(Value::as_u64)
            .and_then(|c| usize::try_from(c).ok())
            .ok_or("each point entry needs a data column index `c`")?;
        let value = e.get("v").cloned().unwrap_or(Value::Null);
        if matches!(value, Value::Array(_) | Value::Object(_)) {
            return Err("a point value must be a number, text, boolean or null".into());
        }
        let Some(b) = fv.fields.get(c) else {
            return Err(format!("the visual has no data column {c}"));
        };
        let FieldRef::Column { table, column } = &b.field else {
            return Err(format!("data column {c} is not a grouping column"));
        };
        if out.iter().any(|x: &Pv| x.c == c) {
            continue;
        }
        out.push(Pv {
            c,
            table: table.clone(),
            column: column.clone(),
            ty: data_type(model, table, column),
            value,
        });
    }
    Ok(out)
}

fn same_point(a: &[Pv], b: &[Pv]) -> bool {
    a.len() == b.len()
        && a.iter()
            .all(|x| b.iter().any(|y| x.c == y.c && same_value(&x.value, &y.value)))
}

/// `v` at its displayed drill level, its hierarchy levels read as their columns: what the data
/// columns (a point's `c`) index.
fn displayed(model: Option<&SemanticModel>, v: &Visual) -> Visual {
    let d = crate::preview::at_drill_level(v).unwrap_or_else(|| v.clone());
    model.and_then(|m| crate::preview::normalized(m, &d)).unwrap_or(d)
}

/// Desktop's click rules on data points (module docs).
fn select_point(st: &mut Interactions, page: &str, visual: &str, point: Vec<Pv>, ctrl: bool) {
    let current = st.selections.get(page).filter(|s| s.visual == visual).cloned();
    let mut points = current.map(|s| s.points).unwrap_or_default();
    let at = points.iter().position(|p| same_point(p, &point));
    if ctrl {
        match at {
            Some(i) => {
                points.remove(i);
            }
            None if points.len() < MAX_POINTS => points.push(point),
            None => {}
        }
    } else if at.is_some() && points.len() == 1 {
        points.clear();
    } else {
        points = vec![point];
    }
    if points.is_empty() {
        st.selections.remove(page);
    } else {
        st.selections.insert(
            page.to_string(),
            Selection {
                visual: visual.to_string(),
                points,
            },
        );
    }
}

/// Several points clicked together (a group): with Ctrl the group is added, or removed when all
/// of it is selected; a plain click selects the group alone, or clears the selection when it is
/// exactly the group (the runtime's `clickPlan` for a target of several selection ids).
fn select_points(st: &mut Interactions, page: &str, visual: &str, group: Vec<Vec<Pv>>, ctrl: bool) {
    let current = st.selections.get(page).filter(|s| s.visual == visual).cloned();
    let mut points = current.map(|s| s.points).unwrap_or_default();
    let all_in = group.iter().all(|g| points.iter().any(|p| same_point(p, g)));
    if ctrl {
        if all_in {
            points.retain(|p| !group.iter().any(|g| same_point(p, g)));
        } else {
            for g in group {
                if points.len() >= MAX_POINTS {
                    break;
                }
                if !points.iter().any(|p| same_point(p, &g)) {
                    points.push(g);
                }
            }
        }
    } else if all_in && points.len() == group.len() {
        points.clear();
    } else {
        points = group;
    }
    if points.is_empty() {
        st.selections.remove(page);
    } else {
        st.selections.insert(
            page.to_string(),
            Selection {
                visual: visual.to_string(),
                points,
            },
        );
    }
}

/// Adds the selection as a visual filter of every visual it reaches in `mode` (`all`: the page's
/// visuals, from which the modes are read). Columns are grouped by table (a filter condition reads
/// one table): one `In` over the tuples of each table, exact for one point or one table, a superset
/// of the points otherwise (documented gap).
fn add_selection_filters(
    page: &mut Page,
    sel: &Selection,
    all: &[Visual],
    mode: Option<Mode>,
    cf: bool,
    off: bool,
) {
    let Some(filter) = selection_filter(sel) else {
        return;
    };
    let targets: Vec<String> = all
        .iter()
        .filter(|t| reaches(page, &sel.visual, t, off))
        .filter(|t| mode.is_none_or(|m| mode_of(page, &sel.visual, t, cf) == m))
        .map(|t| t.name.clone())
        .collect();
    for v in page.visuals.iter_mut().filter(|v| targets.contains(&v.name)) {
        let container = json!({"name": SELECTION_FILTER, "filter": filter});
        let config = v.filter_config.get_or_insert_with(|| json!({"filters": []}));
        if !config.get("filters").is_some_and(Value::is_array)
            && let Some(o) = config.as_object_mut()
        {
            o.insert("filters".into(), json!([]));
        }
        if let Some(list) = config.get_mut("filters").and_then(Value::as_array_mut) {
            list.push(container);
        } else {
            v.filter_config = Some(json!({"filters": [container]}));
        }
    }
}

/// The semantic-query filter of a data-point selection (`None` when it has no point).
fn selection_filter(sel: &Selection) -> Option<Value> {
    let first = sel.points.first()?;
    let mut tables: Vec<&str> = Vec::new();
    for x in first {
        if !tables.iter().any(|t| t.eq_ignore_ascii_case(&x.table)) {
            tables.push(&x.table);
        }
    }
    let mut from = Vec::new();
    let mut conds = Vec::new();
    for (i, t) in tables.iter().enumerate() {
        let alias = format!("t{i}");
        let cols: Vec<&Pv> = first.iter().filter(|x| x.table.eq_ignore_ascii_case(t)).collect();
        let mut tuples: Vec<Vec<&Value>> = Vec::new();
        for p in &sel.points {
            let tuple: Option<Vec<&Value>> = cols
                .iter()
                .map(|c| p.iter().find(|x| x.c == c.c).map(|x| &x.value))
                .collect();
            if let Some(tuple) = tuple
                && !tuples
                    .iter()
                    .any(|x| x.iter().zip(&tuple).all(|(a, b)| same_value(a, b)))
            {
                tuples.push(tuple);
            }
        }
        let exprs: Vec<Value> = cols.iter().map(|c| col_expr(&alias, &c.column)).collect();
        let values: Vec<Value> = tuples
            .iter()
            .map(|tp| {
                Value::Array(
                    tp.iter()
                        .zip(&cols)
                        .map(|(x, c)| literal(&value_literal(x, c.ty)))
                        .collect(),
                )
            })
            .collect();
        from.push(json!({"Name": alias, "Entity": t, "Type": 0}));
        conds.push(json!({"Condition": {"In": {"Expressions": exprs, "Values": values}}}));
    }
    Some(json!({"Version": 2, "From": from, "Where": conds}))
}

/// `interaction.selected` of the source visual: the points as `[{c, v}]` lists and the indexes
/// of the data rows they match (a point matches a row when every value it holds equals the
/// row's).
fn selected_json(sel: &Selection, v: &Value) -> Value {
    let rows = v
        .pointer("/data/rows")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let matched: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            sel.points.iter().any(|p| {
                p.iter()
                    .all(|x| r.get(x.c).is_some_and(|y| same_value(&x.value, y)))
            })
        })
        .map(|(i, _)| i)
        .collect();
    let points: Vec<Value> = sel
        .points
        .iter()
        .map(|p| Value::Array(p.iter().map(|x| json!({"c": x.c, "v": x.value})).collect()))
        .collect();
    json!({"points": points, "rows": matched})
}

// ---------------------------------------------------------------------------------------------
// Semantic-query building

fn literal(text: &str) -> Value {
    json!({"Literal": {"Value": text}})
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn col_expr(alias: &str, column: &str) -> Value {
    json!({"Column": {"Expression": {"SourceRef": {"Source": alias}}, "Property": column}})
}

fn cmp(kind: u8, column: &str, lit: &str) -> Value {
    json!({"Comparison": {"ComparisonKind": kind, "Left": col_expr("t", column), "Right": literal(lit)}})
}

fn and_all(mut conds: Vec<Value>) -> Value {
    let first = if conds.is_empty() {
        Value::Null
    } else {
        conds.remove(0)
    };
    conds
        .into_iter()
        .fold(first, |a, b| json!({"And": {"Left": a, "Right": b}}))
}

fn where_filter(from: &[(&str, &str)], conds: Vec<Value>) -> Value {
    let from: Vec<Value> = from
        .iter()
        .map(|(a, t)| json!({"Name": a, "Entity": t, "Type": 0}))
        .collect();
    let wh: Vec<Value> = conds.into_iter().map(|c| json!({"Condition": c})).collect();
    json!({"Version": 2, "From": from, "Where": wh})
}

/// `In` (or `Not(In)`) over one table's columns.
fn in_filter(
    table: &str,
    columns: &[String],
    tuples: &[Vec<Value>],
    types: &[DataType],
    inverted: bool,
) -> Result<Value, String> {
    let exprs: Vec<Value> = columns.iter().map(|c| col_expr("t", c)).collect();
    let values: Vec<Value> = tuples
        .iter()
        .map(|tp| {
            Value::Array(
                tp.iter()
                    .enumerate()
                    .map(|(i, x)| literal(&value_literal(x, types.get(i).copied().unwrap_or_default())))
                    .collect(),
            )
        })
        .collect();
    let cond = json!({"In": {"Expressions": exprs, "Values": values}});
    let cond = if inverted {
        json!({"Not": {"Expression": cond}})
    } else {
        cond
    };
    Ok(where_filter(&[("t", table)], vec![cond]))
}

/// A data value (as the snapshot gives it) as a semantic-query literal of a column of type `ty`.
fn value_literal(v: &Value, ty: DataType) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => {
            let x = n.as_f64().unwrap_or(0.0);
            match ty {
                DataType::String => quote(&n.to_string()),
                DataType::Int64 => format!("{}L", x.trunc()),
                DataType::Decimal => format!("{x}M"),
                _ if n.is_i64() || n.is_u64() => format!("{n}L"),
                _ => format!("{x}D"),
            }
        }
        Value::String(s) => {
            // an unknown type (a column the model no longer has) keeps dates as text
            if matches!(ty, DataType::DateTime) && iso_serial(s).is_some() {
                let full = if s.len() == 10 {
                    format!("{s}T00:00:00")
                } else {
                    s.clone()
                };
                format!("datetime'{full}'")
            } else {
                quote(s)
            }
        }
        other => quote(&other.to_string()),
    }
}

/// A range bound as a literal, and the `objects.data` property Desktop shows it from.
fn range_literal(v: &Value, ty: DataType, upper: bool) -> Result<(String, &'static str), String> {
    if matches!(ty, DataType::DateTime) {
        let s = v.as_str().ok_or("a date bound must be an ISO date")?;
        let lit = date_literal(s, if upper { 1 } else { 0 })?;
        return Ok((lit, if upper { "endDate" } else { "startDate" }));
    }
    let x = v.as_f64().ok_or("a number bound must be a number")?;
    let lit = match ty {
        DataType::Int64 => format!("{x}L"),
        DataType::Decimal => format!("{x}M"),
        _ => format!("{x}D"),
    };
    Ok((lit, if upper { "numericEnd" } else { "numericStart" }))
}

/// `datetime'YYYY-MM-DDT00:00:00'` of the ISO date `s` plus `days`.
fn date_literal(s: &str, days: i64) -> Result<String, String> {
    let serial = iso_serial(s).ok_or_else(|| format!("`{s}` is not an ISO date"))?;
    let day = filters::iso_day(serial.floor() + days as f64);
    Ok(format!("datetime'{day}T00:00:00'"))
}

/// Days since 1899-12-30 of an ISO date (`YYYY-MM-DD`, optionally followed by a time).
pub(crate) fn iso_serial(s: &str) -> Option<f64> {
    let date = s.get(..10)?;
    let rest = s.get(10..).unwrap_or_default();
    if !(rest.is_empty() || rest.starts_with('T')) {
        return None;
    }
    let mut parts = date.split('-');
    let (y, m, d) = (parts.next()?, parts.next()?, parts.next()?);
    if y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return None;
    }
    let (y, m, d) = (
        y.parse::<i64>().ok()?,
        m.parse::<u32>().ok()?,
        d.parse::<u32>().ok()?,
    );
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some(bicraft_locale::datefmt::days_from_civil(y, m, d) as f64 + 25_569.0)
}

/// Joins the highlight report's results to the data report's: each highlighted visual's `data`
/// gets `highlights`, one entry per row of `rows` (the highlighted part of each value column,
/// `null` under the grouping columns and where nothing is highlighted), matched on the grouping
/// values. Desktop computes the same thing in one query (`IGNORE(CALCULATE(m,
/// KEEPFILTERS(<selection>)))` next to `m`, oracle rule 11); bicraft evaluates the visual twice.
pub(crate) fn merge_highlights(
    data: &mut HashMap<(String, String), Value>,
    highlight: &HashMap<(String, String), Value>,
) {
    for (key, h) in highlight {
        let Some(d) = data.get_mut(key) else {
            continue;
        };
        if d.get("status").and_then(Value::as_str) != Some("ok") {
            continue;
        }
        let kinds: Vec<bool> = d
            .get("columns")
            .and_then(Value::as_array)
            .map(|c| {
                c.iter()
                    .map(|x| x.get("kind").and_then(Value::as_str) == Some("column"))
                    .collect()
            })
            .unwrap_or_default();
        // no value column (a BIcraft visual with no field in its measure roles): nothing to
        // highlight, the visual stays as it is, as Desktop 2.159.1231.0 leaves it
        if kinds.iter().all(|g| *g) {
            continue;
        }
        let group_key = |row: &Value| -> String {
            let cells = row.as_array().map(Vec::as_slice).unwrap_or_default();
            let parts: Vec<String> = kinds
                .iter()
                .enumerate()
                .filter(|(_, g)| **g)
                .map(|(i, _)| match cells.get(i) {
                    Some(Value::String(t)) => format!("s{}", t.to_lowercase()),
                    Some(other) => other.to_string(),
                    None => "null".into(),
                })
                .collect();
            parts.join("\u{1}")
        };
        let ok = h.get("status").and_then(Value::as_str) == Some("ok");
        let mut by_key: HashMap<String, &Value> = HashMap::new();
        if ok {
            for r in h.get("rows").and_then(Value::as_array).into_iter().flatten() {
                by_key.entry(group_key(r)).or_insert(r);
            }
        }
        let rows = d
            .get("rows")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let highlights: Vec<Value> = rows
            .iter()
            .map(|r| {
                let hit = by_key.get(&group_key(r));
                Value::Array(
                    kinds
                        .iter()
                        .enumerate()
                        .map(|(i, g)| {
                            if *g {
                                Value::Null
                            } else {
                                hit.and_then(|x| x.get(i)).cloned().unwrap_or(Value::Null)
                            }
                        })
                        .collect(),
                )
            })
            .collect();
        if let Some(o) = d.as_object_mut() {
            o.insert("highlights".into(), Value::Array(highlights));
            if !ok {
                o.insert(
                    "highlights_reason".into(),
                    h.get("reason")
                        .cloned()
                        .unwrap_or(json!("the highlight could not be evaluated")),
                );
            }
        }
    }
}

#[path = "interact_bookmark.rs"]
pub(crate) mod bookmark;
#[path = "interact_drill.rs"]
pub(crate) mod drill;
#[path = "interact_expand.rs"]
pub(crate) mod expand;
#[path = "interact_highlight.rs"]
pub(crate) mod highlight;
#[path = "interact_reader.rs"]
pub(crate) mod reader;

#[cfg(test)]
#[path = "interact_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "interact_filters_tests.rs"]
mod filters_tests;
#[cfg(test)]
#[path = "interact_focus_tests.rs"]
mod focus_tests;
#[cfg(test)]
#[path = "interact_reader_tests.rs"]
mod reader_tests;
