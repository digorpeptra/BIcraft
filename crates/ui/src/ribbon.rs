//! The ribbon built from the engine's command registry (ROADMAP M11, docs/design.md §5): the tab
//! row's tabs (Home, Report, Model, Data, Analyze, View, then the contextual Visual, Table and
//! Measure tabs a selection shows), every ribbon-placed command as a button of its tab and group
//! (label in the UI's language, icon, tooltip with the reason when it is greyed), the registry's
//! shortcuts, and what a click does: the File view's own flows, the parameters form when the
//! command needs values ([`crate::form::open`]), else the command itself on the active
//! document's source.
//!
//! The view's own controls (zoom, fit, previous and next page, the Validation view) stay pure UI
//! state, in groups before the commands'. The File tab is the Backstage (`backstage.rs`).

use egui::{RichText, Ui};
use serde_json::{Map, Value, json};

use crate::actions::{self, Call};
use crate::app::{App, Tab as View, Then};
use crate::backstage::{self, Backstage};
use crate::i18n::Lang;
use crate::model_view::{Item, Sel};
use crate::registry::{self, At, Cmd, Context, Effect, Kind, Obj, Tab};
use crate::widgets;

/// The fixed tabs after File, in tab-row order.
pub const TABS: [Tab; 7] = [
    Tab::Home,
    Tab::Insert,
    Tab::Report,
    Tab::Model,
    Tab::Data,
    Tab::Analyze,
    Tab::View,
];

/// The View tab's command groups shown before its view groups, in this order (Desktop's View
/// ribbon starts with Themes and Page view).
const VIEW_FIRST: [&str; 2] = ["Themes", "Page view"];

/// A visual selected on the canvas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualSel {
    pub page: String,
    pub visual: String,
}

/// One document's command UI state (swapped with the document, as its other fields).
#[derive(Debug, Default)]
pub struct DocUi {
    /// The visual selected on the canvas (the Visual tab, the visual's menu, Delete).
    pub visual: Option<VisualSel>,
    /// The other visuals selected with it, on its page (Ctrl+click, a rubber band, Ctrl+A;
    /// `crate::canvas`).
    pub more: Vec<String>,
    /// A visual to select once its page is shown (Best practices' Go to visual,
    /// `crate::analyze_ui`).
    pub reveal: Option<VisualSel>,
    /// The answer of the last read command run from a button: (id, `None` while it runs).
    pub result: Option<(&'static str, Option<Result<Value, String>>)>,
    /// That answer read for display (built on its first frame; dropped with a new run).
    pub view: Option<crate::answer::Answer>,
    /// The Edit interactions mode (`interactions.rs`): the selected visual is the source.
    pub edit_interactions: bool,
    /// The Export data dialog of a visual, while it is open.
    pub export_data: Option<crate::export_dialog::ExportData>,
    /// The Data view's state ([`crate::data_view`]).
    pub data: crate::data_view::DataView,
    /// The data point a right click on the page landed on (its `select` point), for See records.
    pub point: Option<Value>,
    /// All the points of the mark under that right click when it stands for several (a
    /// histogram bin, a Venn set), for See records.
    pub points: Option<Value>,
    /// The text box editor and the Insert, Action and Page format windows (`crate::rich`).
    pub rich: crate::rich::RichUi,
    /// The visual calculations edit window, while it is open (`crate::viscalc_dialog`).
    pub viscalc: Option<crate::viscalc_dialog::Editor>,
    /// The New parameter dialog, while it is open (`crate::param_dialog`).
    pub parameter: Option<crate::param_dialog::ParamDialog>,
    /// The Groups dialog (New group, Edit groups), while it is open (`crate::group_dialog`).
    pub group: Option<crate::group_dialog::GroupDialog>,
    /// The Quick measures dialog, while it is open (`crate::quick_dialog`).
    pub quick: Option<crate::quick_dialog::QuickDialog>,
    /// The View as dialog, while it is open ([`crate::view_as`]).
    pub view_as: Option<crate::view_as::Dialog>,
    /// The Autodetect relationships dialog, while it is open ([`crate::autodetect`]).
    pub autodetect: Option<crate::autodetect::Dialog>,
    /// The Manage relationships list, while it is open ([`crate::relationships_dialog`]).
    pub rels: Option<crate::relationships_dialog::Manage>,
    /// The Create / Edit relationship window, while it is open.
    pub rel_edit: Option<crate::relationships_dialog::Editor>,
    /// The Manage roles dialog, while it is open ([`crate::roles_dialog`]).
    pub roles: Option<crate::roles_dialog::Dialog>,
    /// The Manage perspectives dialog, while it is open ([`crate::perspective_dialog`]).
    pub perspective: Option<crate::perspective_dialog::Dialog>,
    /// The Customize theme dialog, while it is open ([`crate::theme_dialog`]).
    pub theme: Option<crate::theme_dialog::Dialog>,
    /// The custom visuals' upload, Remove window and gallery icons ([`crate::custom_visuals`]).
    pub custom_visuals: crate::custom_visuals::CvUi,
    /// The storage mode confirmation (switching to Import), while it is open ([`crate::storage_ui`]).
    pub storage: Option<crate::storage_ui::Confirm>,
    /// The Incremental refresh dialog, while it is open ([`crate::refresh_policy_dialog`]).
    pub refresh_policy: Option<crate::refresh_policy_dialog::Dialog>,
    /// The Performance analyzer pane (`crate::perf_pane`).
    pub perf: crate::perf_pane::PerfUi,
    /// Get data's gallery and Navigator, while open (`crate::get_data`).
    pub get_data: Option<crate::get_data::GetData>,
    /// Data source settings, while open (`crate::source_settings`).
    pub source_settings: Option<crate::source_settings::Dialog>,
    /// The Enter data / Edit data grid, while open (`crate::enter_data`).
    pub enter_data: Option<crate::enter_data::Grid>,
    /// The Power Query editor (Transform data), while open (`crate::pq_editor`).
    pub pq: Option<crate::pq_editor::Editor>,
    /// The DAX query view's tabs (`crate::dax_query`): pure view state, kept per document.
    pub dax_query: crate::dax_query::QueryView,
    /// The TMDL view's script tabs (`crate::tmdl_view`): pure view state, kept per document.
    pub tmdl: crate::tmdl_view::TmdlView,
}

/// The tabs the tab row shows now: the fixed ones, then the contextual ones the selection asks
/// for (a visual in the Report view; a table or column, or a measure, in the Model view).
pub fn tabs(app: &App) -> Vec<Tab> {
    let mut v = TABS.to_vec();
    v.extend(contextual(app));
    v
}

/// The contextual tabs of the selection.
pub fn contextual(app: &App) -> Vec<Tab> {
    match app.tab {
        // the mode keeps its tab (and its button) without a selection
        View::Report if app.cmd.visual.is_some() || app.cmd.edit_interactions => vec![Tab::Visual],
        // the Data view's Column tools
        View::Data if app.cmd.data.column.is_some() && app.cmd.data.stat.is_none() => vec![Tab::Table],
        View::Model => match app.model_view.sel {
            Some(Sel::Table(_) | Sel::Column { .. }) => vec![Tab::Table],
            Some(Sel::Measure { .. }) => vec![Tab::Measure],
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// The view a ribbon tab shows (none: the tab keeps the current view).
pub fn view_of(tab: Tab) -> Option<View> {
    match tab {
        Tab::Report | Tab::Insert | Tab::Visual => Some(View::Report),
        Tab::Data => Some(View::Data),
        Tab::Model | Tab::Measure => Some(View::Model),
        // the Table tab is the Data view's Column tools there, the Model view's elsewhere: it
        // keeps the view
        _ => None,
    }
}

/// The ribbon tab of a view (the Validation view is Analyze's).
pub fn tab_of(view: View) -> Tab {
    match view {
        View::Report => Tab::Report,
        View::Data => Tab::Data,
        View::Model => Tab::Model,
        // the DAX commands (Run DAX query, Update model with changes) are Analyze's
        View::Query | View::Validation => Tab::Analyze,
        // Script TMDL and Apply are the Modeling tab's
        View::Tmdl => Tab::Model,
    }
}

/// Whether command `id` can run now on the active document (`Ok`), or why not in the window's
/// language. The one answer the ribbon, the menus, the palette and the form all use.
pub fn state(app: &App, id: &str) -> Result<(), String> {
    registry::enabled(
        id,
        Context {
            snap: app.loaded.as_ref().map(|l| l.snap.as_ref()),
            open: app.has_doc(),
            reader: app.access.as_deref() == Some("reader"),
        },
    )
    .map_err(|why| registry::reason_text(&why, app.lang))
}

/// The ribbon card's content for `tab`: the view's own groups, then the commands' groups.
pub fn show(app: &mut App, ui: &mut Ui, tab: Tab) {
    match tab {
        Tab::Home => app.home_ribbon(ui),
        Tab::Report => app.report_ribbon(ui),
        Tab::Model => app.model_ribbon(ui),
        Tab::View => {
            // Desktop's order: Themes, Page view, Mobile layout, Page options, then the rest
            for first in VIEW_FIRST {
                command_groups_where(app, ui, tab, |g| g == first);
            }
            crate::mobile::view_group(app, ui);
            crate::canvas::view_group(app, ui);
            command_groups_where(app, ui, tab, |g| !VIEW_FIRST.contains(&g));
            return;
        }
        Tab::Table if app.tab == View::Data => crate::data_view::column_tools(app, ui),
        Tab::Analyze => {
            let l = app.lang;
            widgets::group(ui, |ui| {
                let on = app.tab == View::Validation;
                let label = l.t("Show validation", "Afficher la validation");
                if widgets::big_button(ui, crate::icons::Icon::Validate, label, true)
                    .on_hover_text(l.t(
                        "Every problem of the model and the report",
                        "Tous les problèmes du modèle et du rapport",
                    ))
                    .clicked()
                    && !on
                {
                    app.show_tab(View::Validation);
                }
            });
        }
        _ => {}
    }
    command_groups(app, ui, tab);
    if tab == Tab::Analyze {
        app.validation_ribbon(ui);
    }
    // View as (Desktop's Modeling › Security): the ribbon's last group
    if tab == Tab::Model {
        crate::view_as::ribbon(app, ui);
    }
}

/// The buttons of every group of `tab` that the registry places there: one or two big buttons,
/// or columns of three small ones.
pub fn command_groups(app: &mut App, ui: &mut Ui, tab: Tab) {
    command_groups_where(app, ui, tab, |_| true);
}

/// The groups of `tab` whose name `keep` accepts, in the registry's order.
fn command_groups_where(app: &mut App, ui: &mut Ui, tab: Tab, keep: impl Fn(&str) -> bool) {
    for (group, cmds) in registry::ribbon_groups(tab) {
        if !keep(group) {
            continue;
        }
        widgets::group(ui, |ui| {
            // Model › Parameters: one New parameter button with a menu, as in Desktop
            if tab == Tab::Model && group == crate::param_dialog::GROUP {
                crate::param_dialog::ribbon_button(app, ui, &cmds);
            } else if tab == Tab::Home && group == "Data" {
                // Home › Data: Get data and Enter data big, as in Desktop; the others small
                let (big, small): (Vec<_>, Vec<_>) = cmds
                    .iter()
                    .partition(|c| matches!(c.id, "source.navigate" | "table.enter_data"));
                for c in big {
                    button(app, ui, c, true);
                }
                widgets::rows(ui, |ui| {
                    for c in small {
                        button(app, ui, c, false);
                    }
                });
            } else if tab == Tab::View && group == "Themes" {
                // View › Themes: the gallery big, as in Desktop; the other theme commands small
                let (big, small): (Vec<_>, Vec<_>) =
                    cmds.iter().partition(|c| c.id == crate::themes_ui::COMMAND);
                for c in big {
                    crate::themes_ui::button(app, ui, c);
                }
                for chunk in small.chunks(3) {
                    widgets::rows(ui, |ui| {
                        for c in chunk {
                            button(app, ui, c, false);
                        }
                    });
                }
            } else if cmds.len() <= 2 {
                for c in &cmds {
                    button(app, ui, c, true);
                }
            } else {
                for chunk in cmds.chunks(3) {
                    widgets::rows(ui, |ui| {
                        for c in chunk {
                            button(app, ui, c, false);
                        }
                    });
                }
            }
        });
    }
}

/// One command's button: its label and icon, greyed with the reason when it cannot run.
fn button(app: &mut App, ui: &mut Ui, c: &'static Cmd, big: bool) {
    // the Optimize group's toggle and menu (Pause visuals, Optimization presets)
    if crate::optimize_ui::button(app, ui, c, big) {
        return;
    }
    let st = state(app, c.id);
    let label = c.text(app.lang);
    // Edit interactions is a toggle, as in Desktop: the mode's icons run the command (the
    // palette still opens its form)
    if c.id == crate::interactions::COMMAND {
        let on = crate::interactions::on(app);
        let r = widgets::toggle_button(ui, c.icon(), label, st.is_ok() || on, on, big);
        let tipped = crate::key_tips::button(app, ui, c.id, &r);
        if r.on_hover_text(c.tooltip(app.lang, &st, app.caps().reserved_keys))
            .clicked()
            || (tipped && (st.is_ok() || on))
        {
            crate::interactions::toggle(app);
        }
        return;
    }
    // Format painter is a toggle, as in Desktop: on with the selected visual, the next click on
    // a visual applies its formatting (crate::canvas::painter_toggle)
    if c.id == crate::canvas::PAINTER {
        let on = app.canvas.painter.is_some();
        let ready = (st.is_ok() && app.cmd.visual.is_some()) || on;
        let r = widgets::toggle_button(ui, c.icon(), label, ready, on, big);
        let tipped = crate::key_tips::button(app, ui, c.id, &r);
        let tip = if st.is_ok() && !on && app.cmd.visual.is_none() {
            app.lang
                .t(
                    "Select a visual first: its formatting is copied to the next visual you click.",
                    "Sélectionnez d'abord un visuel : sa mise en forme est copiée sur le prochain visuel cliqué.",
                )
                .to_string()
        } else {
            c.tooltip(app.lang, &st, app.caps().reserved_keys)
        };
        if r.on_hover_text(tip).clicked() || (tipped && ready) {
            crate::canvas::painter_toggle(app);
        }
        return;
    }
    let r = if big {
        widgets::big_button(ui, c.icon(), label, st.is_ok())
    } else {
        widgets::small_button(ui, c.icon(), label, st.is_ok(), false)
    };
    // its key tip (crate::key_tips): a menu button's opens its menu, the others click
    let tipped = crate::key_tips::button(app, ui, c.id, &r);
    if tipped && st.is_err() && menu_button(c.id) {
        // greyed: the command says why in the status bar, as the palette does
        let p = prefill(app, c, None);
        invoke(app, c.id, p);
        return;
    }
    // Insert › Shapes opens the menu of shapes, as Desktop's dropdown
    if c.id == "shape.add" {
        let r = r.on_hover_text(c.tooltip(app.lang, &st, app.caps().reserved_keys));
        if st.is_ok() {
            let mut picked = None;
            crate::key_tips::menu(app, &r, tipped).show(|ui| {
                egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                    for (tile, en, fr) in crate::rich::SHAPES {
                        if ui.button(app.lang.t(en, fr)).clicked() {
                            picked = Some(*tile);
                            ui.close();
                        }
                    }
                });
            });
            if let Some(tile) = picked {
                crate::rich::insert_shape(app, tile);
            }
        }
        return;
    }
    // Align and Layer order open a menu of their `to` values (Desktop's Arrange dropdowns)
    if crate::canvas::MENU_BUTTONS.contains(&c.id) {
        let r = r.on_hover_text(c.tooltip(app.lang, &st, app.caps().reserved_keys));
        if st.is_ok() {
            let mut picked = None;
            crate::key_tips::menu(app, &r, tipped).show(|ui| {
                for (to, en, fr) in crate::canvas::menu_items(c.id) {
                    if ui.button(app.lang.t(en, fr)).clicked() {
                        picked = Some(*to);
                        ui.close();
                    }
                }
            });
            if let Some(to) = picked {
                crate::canvas::run_menu_item(app, c, to);
            }
        }
        return;
    }
    if r.on_hover_text(c.tooltip(app.lang, &st, app.caps().reserved_keys))
        .clicked()
        || tipped
    {
        let p = prefill(app, c, None);
        invoke(app, c.id, p);
    }
}

/// A button that opens a menu (its key tip opens it): Shapes, Align, Layer order.
fn menu_button(id: &str) -> bool {
    id == "shape.add" || crate::canvas::MENU_BUTTONS.contains(&id)
}

/// The parameters the selection gives command `c`: the page shown (or `page`, a page tab's), the
/// visual selected, the model object selected, by the kind of each parameter of its form.
pub fn prefill(app: &App, c: &Cmd, page: Option<&str>) -> Value {
    let mut out = Map::new();
    let active = app.page_names().get(app.active_page()).map(|p| p.0.clone());
    let vis = app.cmd.visual.as_ref();
    let sel = if app.tab == View::Model {
        app.model_view.sel.as_ref()
    } else {
        None
    };
    let table = match sel {
        Some(Sel::Table(t)) => Some(t.clone()),
        Some(
            Sel::Column { table, .. } | Sel::Measure { table, .. } | Sel::Item(Item::Hierarchy { table, .. }),
        ) => Some(table.clone()),
        _ => None,
    };
    // the Data view's table and column
    let (table, data_column) = if app.tab == View::Data && app.cmd.data.stat.is_none() {
        (app.cmd.data.table.clone(), app.cmd.data.column.clone())
    } else {
        (table, None)
    };
    for p in c.form {
        let v = match p.kind {
            Kind::Name(Obj::Page) => page
                .map(str::to_string)
                .or_else(|| vis.map(|v| v.page.clone()))
                .or_else(|| active.clone()),
            Kind::Name(Obj::Visual) => vis.map(|v| v.visual.clone()),
            Kind::Name(Obj::Table) => table.clone(),
            Kind::Name(Obj::CalculationGroup) => match sel {
                Some(Sel::Table(t) | Sel::Item(Item::CalcItem { group: t, .. })) => Some(t.clone()),
                _ => None,
            },
            // the Model explorer's objects
            Kind::Name(o) if matches!(sel, Some(Sel::Item(_))) => match (o, sel) {
                (Obj::CalculationItem, Some(Sel::Item(Item::CalcItem { name, .. })))
                | (Obj::Hierarchy, Some(Sel::Item(Item::Hierarchy { name, .. })))
                | (Obj::Role, Some(Sel::Item(Item::Role(name))))
                | (Obj::Culture, Some(Sel::Item(Item::Culture(name))))
                | (Obj::Expression, Some(Sel::Item(Item::Expression(name)))) => Some(name.clone()),
                _ => None,
            },
            Kind::Name(Obj::Column) => match sel {
                Some(Sel::Column { name, .. }) => Some(name.clone()),
                _ => data_column.clone(),
            },
            Kind::Name(Obj::Measure) => match sel {
                Some(Sel::Measure { name, .. }) => Some(name.clone()),
                _ => None,
            },
            Kind::Field if matches!(p.name, "from" | "to") => match sel {
                Some(Sel::Rel(i)) => rel_end(app, *i, p.name),
                _ => None,
            },
            _ => None,
        };
        if let Some(v) = v {
            out.insert(p.name.to_string(), json!(v));
        }
    }
    // the commands that take `visual` and `visuals` get the whole selection
    let sel = crate::canvas::selection(app);
    if !sel.is_empty()
        && c.form.iter().any(|p| p.name == "visuals" && p.kind == Kind::List)
        && c.form.iter().any(|p| p.kind == Kind::Name(Obj::Visual))
    {
        out.insert("visuals".into(), json!(sel));
    }
    // the title's form starts from the visual's title text (free text, edited in place)
    if c.id == "visual.set_title"
        && let Some(v) = vis
        && let Some(title) = crate::rich::snap_visual(app, &v.page, &v.visual)
            .and_then(|x| x.get("title").and_then(Value::as_str).map(str::to_string))
    {
        out.insert("title".into(), json!(title));
    }
    // a new page: the next free name at the end, as the page tabs' `+`
    if c.id == "page.add" {
        let names: Vec<String> = app.page_names().into_iter().map(|p| p.1).collect();
        let prefix = app.lang.t("Page", "Page");
        out.insert(
            "display_name".into(),
            json!(actions::new_page_name(prefix, &names)),
        );
        out.insert("index".into(), json!(names.len()));
    }
    Value::Object(out)
}

/// `Table[Column]` of one end of relationship `i` in the snapshot.
fn rel_end(app: &App, i: usize, end: &str) -> Option<String> {
    let r = app
        .loaded
        .as_ref()?
        .snap
        .pointer(&format!("/model/relationships/{i}"))?
        .clone();
    let t = r.get(format!("{end}_table"))?.as_str()?;
    let c = r.get(format!("{end}_column"))?.as_str()?;
    Some(format!("{t}[{c}]"))
}

/// Whether a click on `c` with `params` opens the parameters form: a required value is missing,
/// or the selection gave the required ones but the form has more (`column.set {table, name}`
/// would change nothing). A command with optional parameters only runs at once.
pub fn opens_form(c: &Cmd, params: &Value) -> bool {
    if c.needs_values(params) {
        return true;
    }
    let has_required = c.form.iter().any(|p| p.required);
    let left = c.form.iter().any(|p| params.get(p.name).is_none());
    // Group names its group itself (Group 1, Group 2…), as Desktop's button
    has_required && left && !matches!(c.id, "page.add" | "visual.group")
}

/// Runs command `id` as its button does: the File view's flows for the file commands, the form
/// when values are needed, else the command on the active document (a read command's answer is
/// shown). A command that cannot run says why in the status bar.
pub fn invoke(app: &mut App, id: &str, params: Value) {
    let l = app.lang;
    let Some(c) = registry::find(id) else {
        app.error = Some(format!("✕ unknown command `{id}`"));
        return;
    };
    let export = |app: &mut App, task: backstage::Task| {
        let mut bs = Backstage::new(backstage::Page::Export);
        bs.task = task;
        app.backstage = Some(bs);
    };
    match c.id {
        "file.open" => {
            if app.caps().open_files {
                app.pick_and_open();
            } else {
                app.backstage = Some(Backstage::new(backstage::Page::Open));
            }
            return;
        }
        "model.new" if !app.has_doc() => {
            app.backstage = Some(Backstage::new(backstage::Page::Home));
            return;
        }
        "project.new" => {
            app.backstage = Some(Backstage::new(backstage::Page::Home));
            return;
        }
        "file.inspect" if app.has_doc() => {
            app.backstage = Some(Backstage::new(backstage::Page::Info));
            return;
        }
        "file.save" if app.has_doc() => {
            app.save();
            return;
        }
        "file.save_as" if app.has_doc() => return export(app, backstage::Task::SaveAs),
        "file.convert" if app.has_doc() => return export(app, backstage::Task::Convert),
        "file.redact" if app.has_doc() => return export(app, backstage::Task::Redact),
        "report.render" if app.has_doc() => return export(app, backstage::Task::Render),
        "file.diff" if app.has_doc() => return export(app, backstage::Task::Compare),
        // the canvas's clipboard (crate::canvas): the selection, the clipboards
        "visual.copy" if app.has_doc() => return crate::canvas::copy(app),
        "visual.cut" if app.has_doc() => return crate::canvas::cut(app),
        "visual.paste" if params.get("clip").is_none() && app.has_doc() => {
            return crate::canvas::paste_command(app);
        }
        "visual.duplicate" if app.has_doc() => return crate::canvas::duplicate(app),
        // Format painter without targets: the toggle (the palette's entry too)
        crate::canvas::PAINTER if params.get("to").is_none() && app.has_doc() => {
            return crate::canvas::painter_toggle(app);
        }
        "report.export_pdf" if app.has_doc() => return export(app, backstage::Task::Pdf),
        "report.print" if app.has_doc() => return export(app, backstage::Task::Print),
        // the dialog of the selected visual (Desktop's Export data), when no file is given
        "visual.export_data" if app.has_doc() && params.get("out").is_none() => {
            if let (Some(page), Some(visual)) = (
                params.get("page").and_then(Value::as_str),
                params.get("visual").and_then(Value::as_str),
            ) {
                app.cmd.visual = Some(VisualSel {
                    page: page.to_string(),
                    visual: visual.to_string(),
                });
            }
            crate::export_dialog::open(app);
            return;
        }
        _ => {}
    }
    if let Err(why) = state(app, c.id) {
        app.status = format!("{}: {why}", c.text(l));
        return;
    }
    // Format visual opens the Build pane's Format visual tab, as Desktop's Format button does
    if crate::format_pane::intercept(app, c.id, &params) {
        return;
    }
    // the Insert tab, the text editor, the action and page format windows (`rich.rs`)
    if crate::rich::intercept(app, c.id, &params) {
        return;
    }
    // the visual calculations edit window (`viscalc_dialog.rs`): New calculation, an edit
    if crate::viscalc_dialog::intercept(app, c.id, &params) {
        return;
    }
    // New parameter's Numeric range and Fields dialogs (`param_dialog.rs`)
    if crate::param_dialog::intercept(app, c.id, &params)
        || crate::autodetect::intercept(app, c.id, &params)
        || crate::relationships_dialog::intercept(app, c.id, &params)
        || crate::roles_dialog::intercept(app, c.id, &params)
        || crate::perspective_dialog::intercept(app, c.id, &params)
        || crate::theme_dialog::intercept(app, c.id, &params)
        || crate::group_dialog::intercept(app, c.id, &params)
        || crate::quick_dialog::intercept(app, c.id, &params)
        || crate::custom_visuals::intercept(app, c.id, &params)
        || crate::refresh_policy_dialog::intercept(app, c.id, &params)
        || crate::source_settings::intercept(app, c.id, &params)
    {
        return;
    }
    // theme.set without a theme opens its form (the gallery passes `builtin`)
    if crate::themes_ui::intercept(app, c.id, &params) {
        return;
    }
    // the Performance analyzer runs in its pane (`perf_pane.rs`)
    if crate::perf_pane::intercept(app, c.id, &params) {
        return;
    }
    // Get data's gallery and Navigator, the Enter data and Edit data grid
    if crate::get_data::intercept(app, c.id, &params)
        || crate::enter_data::intercept(app, c.id, &params)
        || crate::pq_editor::intercept(app, c.id, &params)
    {
        return;
    }
    // Run DAX query and Update model with changes open the DAX query view and act on its tab
    if crate::dax_query::intercept(app, c.id, &params) {
        return;
    }
    // Script TMDL fills the TMDL view; Apply without a script applies its tab
    if crate::tmdl_view::intercept(app, c.id, &params) {
        return;
    }
    // the Data view's New column is written in the formula bar, as in Desktop (its type is the
    // expression's)
    if c.id == "column.add" && app.tab == View::Data && params.get("expression").is_none() {
        let table = params.get("table").and_then(Value::as_str).map(str::to_string);
        crate::formula::new_column(app, table);
        return;
    }
    // the Data view's reads draw in its grid (a table's rows, a query preview, See records)
    if matches!(c.id, "table.rows" | "query.preview" | "visual.records")
        && crate::data_view::invoke(app, c.id, &params)
    {
        return;
    }
    // a new measure is written in the formula bar, as in Desktop (the palette's Shift+Enter
    // still opens the form)
    if c.id == "measure.add" && params.get("expression").is_none() {
        let table = params.get("table").and_then(Value::as_str).map(str::to_string);
        crate::formula::new_measure(app, table);
        return;
    }
    if opens_form(c, &params) {
        crate::form::open(app, c.id, params);
        return;
    }
    send(app, c, params);
}

/// Sends command `c` with `params` as they are to the active document (what a click does once
/// the values are there): a new page is shown, a read command's answer goes to the answer window.
/// The control channel's `command` with every required value comes here too.
pub fn send(app: &mut App, c: &'static Cmd, params: Value) {
    let then = if c.id == "page.add" {
        Then::ShowPage
    } else if c.effect == Effect::ReadOnly || c.id == "model.refresh" {
        // a refresh shows its progress (the spinner) and its per-table results in the answer
        // window, as a read command's answer
        app.cmd.result = Some((c.id, None));
        app.cmd.view = None;
        Then::Show(c.id)
    } else {
        Then::Nothing
    };
    app.run(Call { id: c.id, params }, then);
}

/// The registry's shortcuts (Ctrl+Z, Ctrl+M, Delete, F5…), none while text is typed or a dialog,
/// the palette or a form is open. Ctrl+S stays the app's, and Ctrl+O too where the shell has an
/// Open dialog (the browser's Ctrl+O is `file.open`'s File view). Delete is ignored while a
/// control has the focus (a page tab deletes its page) and needs a selected visual. F5 is
/// `model.refresh` on a PBIP project or a PBIX file, a reload of the file otherwise.
pub fn shortcuts(app: &mut App, ui: &Ui) {
    let dialog = app.confirm.is_some()
        || app.connect.is_some()
        || app.form.is_some()
        || app.palette.is_some()
        || app.cmd.parameter.is_some()
        || app.cmd.group.is_some()
        || app.cmd.quick.is_some()
        || app.cmd.get_data.is_some()
        || app.cmd.enter_data.is_some()
        || app.cmd.pq.is_some();
    if ui.ctx().egui_wants_keyboard_input() || dialog {
        return;
    }
    let focused = ui.ctx().memory(|m| m.focused().is_some());
    let open_dialog = app.caps().open_files;
    let browser = app.caps().reserved_keys;
    if ui.input(|i| i.key_pressed(egui::Key::F5) && i.modifiers.is_none())
        && state(app, "model.refresh").is_err()
    {
        ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F5));
        app.reload();
        return;
    }
    let mut keys: Vec<(&'static Cmd, (egui::Modifiers, egui::Key))> = registry::COMMANDS
        .iter()
        .filter(|c| c.id != "file.save" && (c.id != "file.open" || !open_dialog))
        // Ctrl+C, Ctrl+X and Ctrl+V come as the shells' copy, cut and paste events (crate::canvas::keys)
        .filter(|c| !matches!(c.id, "visual.copy" | "visual.cut" | "visual.paste"))
        .filter(|c| c.id != "visual.duplicate" || (app.cmd.visual.is_some() && app.tab == View::Report))
        // Delete is the page tabs' too: it deletes a visual only when one is selected
        .filter(|c| c.id != "visual.remove" || (app.cmd.visual.is_some() && !focused))
        .filter_map(|c| c.shortcut_for(browser).and_then(chord).map(|k| (c, k)))
        .collect();
    // Ctrl+Shift+S before Ctrl+S
    keys.sort_by_key(|(_, (m, _))| std::cmp::Reverse(u8::from(m.shift) + u8::from(m.alt)));
    let hit = keys
        .into_iter()
        .find(|(_, (m, k))| ui.input_mut(|i| i.consume_key(*m, *k)));
    if let Some((c, _)) = hit {
        let p = prefill(app, c, None);
        invoke(app, c.id, p);
    }
}

/// A chord as egui reads it: `Ctrl+Shift+S`, `F5`, `Delete`.
pub fn chord(s: &str) -> Option<(egui::Modifiers, egui::Key)> {
    let mut m = egui::Modifiers::NONE;
    let mut key = None;
    for part in s.split('+') {
        match part {
            "Ctrl" => m |= egui::Modifiers::COMMAND,
            "Shift" => m |= egui::Modifiers::SHIFT,
            "Alt" => m |= egui::Modifiers::ALT,
            k => key = egui::Key::from_name(k),
        }
    }
    key.map(|k| (m, k))
}

/// The answer of a read command run from a button, in a window (DeckCraft's dialog frame): its
/// grid or key/value tree with the JSON toggle (`answer.rs`), Close; an error on `err_soft`.
pub fn result_ui(app: &mut App, ctx: &egui::Context) {
    let l = app.lang;
    let Some((id, answer)) = app.cmd.result.take() else {
        return;
    };
    let t = crate::theme::Tokens::of(app.prefs.theme);
    let title = registry::find(id).map_or(id, |c| c.text(l));
    let mut open = true;
    let mut close = false;
    let mut act = None;
    let view = &mut app.cmd.view;
    egui::Window::new(title)
        .id(egui::Id::unique("bicraft-command-result"))
        .collapsible(false)
        .resizable(true)
        .default_size([640.0, 440.0])
        .anchor(egui::Align2::CENTER_CENTER, [0.0, -40.0])
        .open(&mut open)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(id).monospace().color(t.text_dim));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(l.t("Close", "Fermer")).clicked() {
                        close = true;
                    }
                });
            });
            match &answer {
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        if id == "model.refresh" {
                            ui.label(l.t(
                                "Refreshing: the file is written once every table has loaded",
                                "Actualisation : le fichier est écrit une fois toutes les tables chargées",
                            ));
                        }
                    });
                }
                Some(Ok(v)) => {
                    // a refresh: one line, then its per-table results as a grid
                    let v = if id == "model.refresh" {
                        ui.label(RichText::new(refresh_summary(v, l)).color(t.ok));
                        v.get("tables").unwrap_or(v)
                    } else {
                        v
                    };
                    // Best practices: the report findings first, with Go to and Fix
                    if id == "model.analyze" {
                        act = crate::analyze_ui::show(ui, v, l, &t);
                    }
                    let a = view.get_or_insert_with(|| crate::answer::Answer::new(v));
                    if a.is_empty() {
                        ui.label(RichText::new(format!("✓ {}", l.t("Done", "Terminé"))).color(t.ok));
                    } else {
                        let h = (ui.available_height() - 40.0).max(160.0);
                        crate::answer::show(ui, v, a, egui::Id::unique("cmd-answer"), h, l);
                    }
                }
                Some(Err(e)) => {
                    egui::Frame::new()
                        .fill(t.err_soft)
                        .corner_radius(egui::CornerRadius::same(8))
                        .inner_margin(egui::Margin::symmetric(10, 6))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(format!("✕ {}", crate::app::clip(e, 4000))).color(t.err_text),
                            );
                        });
                    if ui.button(l.t("Copy", "Copier")).clicked() {
                        ui.ctx().copy_text(e.clone());
                    }
                }
            }
        });
    if !open || close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.cmd.view = None;
    } else {
        app.cmd.result = Some((id, answer));
    }
    if let Some(act) = act {
        crate::analyze_ui::apply(app, act);
    }
}

/// Every command id the UI's ribbon tabs show a button for (the coverage test reads it): the
/// groups each tab of [`TABS`] and the contextual tabs draws.
pub fn ribbon_ids() -> Vec<&'static str> {
    TABS.iter()
        .chain([Tab::Visual, Tab::Table, Tab::Measure].iter())
        .flat_map(|t| registry::ribbon_groups(*t))
        .flat_map(|(_, cmds)| cmds.into_iter().map(|c| c.id))
        .collect()
}

/// Whether a command is placed on a ribbon tab the UI draws (File is the Backstage's).
pub fn on_a_drawn_tab(c: &Cmd) -> bool {
    matches!(c.at, At::Ribbon { tab, .. } if tab != Tab::File)
}

/// The line above a refresh's per-table grid: tables refreshed, kept, time, where.
pub fn refresh_summary(v: &Value, l: Lang) -> String {
    let n = v.get("tables").and_then(Value::as_array).map_or(0, Vec::len);
    let kept = v.get("kept").and_then(Value::as_u64).unwrap_or(0);
    let ms = v.get("elapsed_ms").and_then(Value::as_u64).unwrap_or(0);
    let secs = format!("{:.1}", ms as f64 / 1000.0);
    let skipped = v.get("skipped").and_then(Value::as_array).map_or(0, Vec::len);
    let mut s = match l {
        Lang::En => format!("✓ {n} tables refreshed, {kept} kept, in {secs} s"),
        Lang::Fr => format!(
            "✓ {n} tables actualisées, {kept} conservées, en {} s",
            secs.replace('.', ",")
        ),
    };
    if skipped > 0 {
        s.push_str(&match l {
            Lang::En => format!(" ({skipped} skipped)"),
            Lang::Fr => format!(" ({skipped} ignorées)"),
        });
    }
    s
}
