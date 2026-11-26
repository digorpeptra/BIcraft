//! The Model view's right-most pane, as in Power BI Desktop (ROADMAP lot 18, docs/gaps.md G61):
//! two tabs at its top,
//!
//! - **Data**: the model's tables with their columns, hierarchies (with their levels) and
//!   measures;
//! - **Model**, the Model explorer: the semantic model's calculation groups with their items (in
//!   ordinal order), cultures, data sources, functions, measures, perspectives, relationships,
//!   roles, shared expressions (queries and parameters) and tables.
//!
//! A search box above the tree keeps the rows whose name holds its text (ignoring case), every
//! branch open. A click on a table, a column, a measure, a calculation group or item, a
//! hierarchy, a relationship, a role, a culture, a shared expression or a data source selects
//! it: the Properties pane (`props_pane.rs`) and the diagram follow; Ctrl+click adds a table, column or
//! measure to the selection (or takes it out), Shift+click selects the rows from the last one
//! picked. Right-click menus run the engine's
//! commands on the node (rename, delete, new calculation group or item, new role, culture,
//! query…); a calculation item dragged onto another takes its place (`calculation_item.set` with
//! the new `ordinal`, one undo step). Nothing here edits a file: every gesture is a command the
//! Model view hands to the app ([`Out`]).

use std::collections::{BTreeSet, HashMap, HashSet};

use egui::{Align2, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::{Value, json};

use crate::app::clip;
use crate::i18n::Lang;
use crate::icons::{self, Icon};
use crate::model_view::{Item, Sel};
use crate::registry;
use crate::theme::{self, Tokens};
use crate::widgets;

/// Height of a row.
const ROW_H: f32 = 22.0;
/// Indent of each level.
const INDENT: f32 = 14.0;
/// Longest name drawn (characters).
const MAX_LABEL: usize = 60;
/// Most rows a node lists (the snapshot caps its lists too).
const MAX_ROWS: usize = 2000;

/// The commands the pane's menus run (the coverage test reads them through `menus::ids`).
pub const COMMANDS: &[&str] = &[
    "calculation_group.add",
    "calculation_group.set",
    "calculation_group.remove",
    "calculation_item.add",
    "calculation_item.set",
    "calculation_item.remove",
    "culture.add",
    "culture.remove",
    "relationship.add",
    "relationship.set",
    "relationship.remove",
    "relationship.autodetect",
    "role.add",
    "role.set_filter",
    "role.remove",
    "role.rename",
    "role.set",
    "role.set_permission",
    "perspective.add",
    "perspective.set",
    "perspective.remove",
    "expression.add",
    "expression.set",
    "expression.rename",
    "expression.remove",
    "function.add",
    "function.set",
    "function.rename",
    "function.remove",
    "tmdl.script",
    "table.rename",
    "table.remove",
    "column.rename",
    "column.remove",
    "measure.rename",
    "measure.remove",
    "diagram.move",
    "diagram.remove_tables",
    "hierarchy.set",
    "hierarchy.remove",
];

/// The pane's tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Data,
    Model,
}

/// The pane's view state (pure UI).
#[derive(Debug, Clone)]
pub struct Explorer {
    pub tab: Tab,
    /// The nodes expanded, by key (`m` the model, `cg` its calculation groups, `t:<table>`…).
    pub open: BTreeSet<String>,
    /// The Data tab's tables, columns and measures in the order last drawn (Shift+click
    /// selects a run of them).
    pub order: Vec<Sel>,
    /// The rows being drawn this frame.
    drawing: Vec<Sel>,
    /// The search box's text: only the rows whose name holds it (ignoring case) are drawn, with
    /// every branch open.
    pub query: String,
    /// `query` in lower case, for this frame.
    needle: String,
}

impl Default for Explorer {
    fn default() -> Explorer {
        Explorer {
            tab: Tab::Data,
            open: ["m".to_string()].into_iter().collect(),
            order: Vec::new(),
            drawing: Vec::new(),
            query: String::new(),
            needle: String::new(),
        }
    }
}

/// Longest search text.
const MAX_QUERY: usize = 200;

/// How a menu item runs its command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Via {
    /// Through the app's invoke: sent when the values are all there, else its form.
    Invoke,
    /// Its form, with these values (the edits, whose values are all optional).
    Form,
}

/// What a gesture in the pane asks for.
#[derive(Debug, Default)]
pub struct Out {
    /// A new selection (`Some(None)`: none).
    pub sel: Option<Option<Sel>>,
    /// A command through the app's invoke (its form when a value is missing).
    pub picked: Option<(&'static str, Value)>,
    /// A command's form with these values.
    pub form: Option<(&'static str, Value)>,
    /// A command sent as it is (a calculation item dropped on another).
    pub run: Option<(&'static str, Value)>,
    /// Ctrl+click: this object joins the selection or leaves it.
    pub toggle: Option<Sel>,
    /// Shift+click: the rows from the last one picked to this one.
    pub range: Option<Sel>,
}

/// What the pane reads.
pub struct Cx<'a> {
    pub model: &'a Value,
    pub sel: Option<&'a Sel>,
    /// Every selected object (`sel` alone, or a multi-selection).
    pub picked: &'a [Sel],
    pub states: &'a HashMap<&'static str, Result<(), String>>,
    pub editable: bool,
    pub l: Lang,
    /// The layout tab shown when it is not the one of all tables: its name, where a table added
    /// to it goes, and the tables it has.
    pub layout: Option<(&'a str, (f64, f64), &'a HashSet<String>)>,
}

impl Cx<'_> {
    /// Whether `x` is selected (alone or with others).
    fn on(&self, x: &Sel) -> bool {
        self.sel == Some(x) || self.picked.contains(x)
    }
}

/// A click on a table, column or measure row: alone, Ctrl (joins or leaves the selection) or
/// Shift (a run of rows); whether it was a plain click.
fn pick(ui: &Ui, out: &mut Out, sel: Sel) -> bool {
    let m = ui.input(|i| i.modifiers);
    if m.command {
        out.toggle = Some(sel);
        false
    } else if m.shift {
        out.range = Some(sel);
        false
    } else {
        out.sel = Some(Some(sel));
        true
    }
}

/// A calculation item being dragged.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ItemDrag {
    group: String,
    name: String,
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn yes(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool) == Some(true)
}

fn strings(v: Option<&Value>) -> Vec<&str> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .take(MAX_ROWS)
        .collect()
}

fn objects<'a>(v: &'a Value, k: &str) -> Vec<(usize, &'a Value)> {
    crate::diagram::objects(v, k).take(MAX_ROWS).collect()
}

/// `tmdl.script`'s objects for every function listed.
fn function_refs(functions: &[(usize, &Value)]) -> Value {
    functions
        .iter()
        .map(|(_, f)| json!({"kind": "function", "name": s(f, "name")}))
        .collect()
}

/// A relationship's text: `From[Column] → To[Column]`.
pub fn rel_text(r: &Value) -> String {
    format!(
        "{}[{}] → {}[{}]",
        s(r, "from_table"),
        s(r, "from_column"),
        s(r, "to_table"),
        s(r, "to_column")
    )
}

impl Explorer {
    /// The pane: its tabs, then the tab's tree.
    pub fn ui(&mut self, ui: &mut Ui, cx: &Cx<'_>, out: &mut Out) {
        let l = cx.l;
        self.drawing.clear();
        ui.horizontal(|ui| {
            for (tab, text) in [
                (Tab::Data, l.t("Data", "Données")),
                (Tab::Model, l.t("Model", "Modèle")),
            ] {
                if widgets::tab(ui, text, self.tab == tab, false).clicked() {
                    self.tab = tab;
                }
            }
        });
        ui.add_space(4.0);
        let name = l.t("Search the model", "Rechercher dans le modèle");
        ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .char_limit(MAX_QUERY)
                .hint_text(l.t("Search", "Rechercher"))
                .desired_width(ui.available_width()),
        )
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, name));
        self.needle = self.query.trim().to_lowercase();
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .id_salt("bicraft-model-explorer-tree")
            .auto_shrink([false, false])
            .show(ui, |ui| match self.tab {
                Tab::Data => {
                    for (_, t) in objects(cx.model, "tables") {
                        self.table_ui(ui, cx, out, t, 0);
                    }
                    self.order = std::mem::take(&mut self.drawing);
                }
                Tab::Model => self.model_ui(ui, cx, out),
            });
    }

    fn toggle(&mut self, key: &str) {
        if !self.open.remove(key) {
            self.open.insert(key.to_string());
        }
    }

    /// Whether a row named `text` matches the search (every row without one).
    fn hit(&self, text: &str) -> bool {
        self.needle.is_empty() || text.to_lowercase().contains(&self.needle)
    }

    /// Whether branch `key` shows its children: opened, or a search is on.
    fn is_open(&self, key: &str) -> bool {
        self.open.contains(key) || !self.needle.is_empty()
    }

    /// A leaf row for a Model explorer object the diagram does not draw: a click selects it (the
    /// Properties pane shows it), its menu runs `menu`. Nothing when it does not match the search.
    #[allow(clippy::too_many_arguments)]
    fn item_row(
        &self,
        ui: &mut Ui,
        cx: &Cx<'_>,
        out: &mut Out,
        depth: usize,
        icon: Icon,
        text: &str,
        item: Item,
        drag: bool,
        menu: &[(&'static str, Value, Via)],
    ) -> Option<egui::Response> {
        if !self.hit(text) {
            return None;
        }
        let sel = Sel::Item(item);
        let resp = row(
            ui,
            cx.l,
            depth,
            icon,
            text,
            None,
            cx.sel == Some(&sel),
            false,
            drag,
        );
        if resp.clicked() {
            out.sel = Some(Some(sel));
        }
        menu_on(&resp, cx, out, menu);
        Some(resp)
    }

    /// A branch: its row (a click opens or closes it), its menu; `body` draws its children.
    #[allow(clippy::too_many_arguments)]
    fn branch(
        &mut self,
        ui: &mut Ui,
        cx: &Cx<'_>,
        out: &mut Out,
        key: &str,
        depth: usize,
        icon: Icon,
        text: &str,
        menu: &[(&'static str, Value, Via)],
        sel: Option<Sel>,
        body: impl FnOnce(&mut Self, &mut Ui, &mut Out),
    ) {
        let open = self.is_open(key);
        let selected = sel.as_ref().is_some_and(|x| cx.on(x));
        let r = row(ui, cx.l, depth, icon, text, Some(open), selected, false, false);
        if r.clicked() {
            self.toggle(key);
            if let Some(sel) = sel {
                out.sel = Some(Some(sel));
            }
        }
        menu_on(&r, cx, out, menu);
        if open {
            body(self, ui, out);
        }
    }

    fn model_ui(&mut self, ui: &mut Ui, cx: &Cx<'_>, out: &mut Out) {
        let l = cx.l;
        let m = cx.model;
        let tables = objects(m, "tables");
        let name = s(m, "name");
        let title = if name.is_empty() {
            l.t("Semantic model", "Modèle sémantique").to_string()
        } else {
            name.to_string()
        };
        self.branch(
            ui,
            cx,
            out,
            "m",
            0,
            Icon::Model,
            &title,
            &[("tmdl.script", json!({}), Via::Invoke)],
            None,
            |me, ui, out| {
                // calculation groups: the tables that list calculation items
                let groups: Vec<&Value> = tables
                    .iter()
                    .map(|x| x.1)
                    .filter(|t| t.get("calculation_items").is_some_and(Value::is_array))
                    .collect();
                let text = count_text(l.t("Calculation groups", "Groupes de calcul"), groups.len());
                let menu = [("calculation_group.add", json!({}), Via::Invoke)];
                me.branch(
                    ui,
                    cx,
                    out,
                    "cg",
                    1,
                    Icon::Calculator,
                    &text,
                    &menu,
                    None,
                    |me, ui, out| {
                        for g in &groups {
                            me.group_ui(ui, cx, out, g);
                        }
                    },
                );
                // cultures
                let cultures = strings(m.get("cultures"));
                let text = count_text(l.t("Cultures", "Cultures"), cultures.len());
                let menu = [("culture.add", json!({}), Via::Invoke)];
                me.branch(
                    ui,
                    cx,
                    out,
                    "cul",
                    1,
                    Icon::Language,
                    &text,
                    &menu,
                    None,
                    |me, ui, out| {
                        for c in &cultures {
                            me.item_row(
                                ui,
                                cx,
                                out,
                                2,
                                Icon::Language,
                                c,
                                Item::Culture(c.to_string()),
                                false,
                                &[("culture.remove", json!({"name": c}), Via::Invoke)],
                            );
                        }
                    },
                );
                // data sources: the model's legacy and structured sources (TMSL dataSources)
                let sources = objects(m, "data_sources");
                let text = count_text(l.t("Data sources", "Sources de données"), sources.len());
                me.branch(
                    ui,
                    cx,
                    out,
                    "ds",
                    1,
                    Icon::Server,
                    &text,
                    &[],
                    None,
                    |me, ui, out| {
                        for (_, d) in &sources {
                            let name = s(d, "name");
                            me.item_row(
                                ui,
                                cx,
                                out,
                                2,
                                Icon::Server,
                                name,
                                Item::DataSource(name.to_string()),
                                false,
                                &[],
                            );
                        }
                    },
                );
                // functions: the DAX user-defined functions
                let functions = objects(m, "functions");
                let text = count_text(l.t("Functions", "Fonctions"), functions.len());
                let mut menu = vec![("function.add", json!({}), Via::Form)];
                if !functions.is_empty() {
                    menu.push((
                        "tmdl.script",
                        json!({"objects": function_refs(&functions)}),
                        Via::Invoke,
                    ));
                }
                me.branch(
                    ui,
                    cx,
                    out,
                    "fun",
                    1,
                    Icon::Calculator,
                    &text,
                    &menu,
                    None,
                    |me, ui, out| {
                        for (_, f) in &functions {
                            let name = s(f, "name");
                            if !me.hit(name) {
                                continue;
                            }
                            let resp = row(
                                ui,
                                cx.l,
                                2,
                                Icon::Calculator,
                                name,
                                None,
                                false,
                                yes(f, "hidden"),
                                false,
                            );
                            let edit = json!({
                                "name": name,
                                "expression": s(f, "expression"),
                                "description": s(f, "description"),
                            });
                            if resp.double_clicked() && cx.editable {
                                out.form = Some(("function.set", edit.clone()));
                            }
                            menu_on(
                                &resp,
                                cx,
                                out,
                                &[
                                    ("function.set", edit, Via::Form),
                                    ("function.rename", json!({"name": name}), Via::Invoke),
                                    ("function.remove", json!({"name": name}), Via::Invoke),
                                    (
                                        "tmdl.script",
                                        json!({"objects": [{"kind": "function", "name": name}]}),
                                        Via::Invoke,
                                    ),
                                ],
                            );
                        }
                    },
                );
                // measures, every table's
                let measures: Vec<(&str, &Value)> = tables
                    .iter()
                    .flat_map(|(_, t)| {
                        objects(t, "measures")
                            .into_iter()
                            .map(move |(_, x)| (s(t, "name"), x))
                    })
                    .take(MAX_ROWS)
                    .collect();
                let text = count_text(l.t("Measures", "Mesures"), measures.len());
                me.branch(
                    ui,
                    cx,
                    out,
                    "mea",
                    1,
                    Icon::Measure,
                    &text,
                    &[],
                    None,
                    |me, ui, out| {
                        for (table, x) in &measures {
                            if me.hit(s(x, "name")) {
                                measure_row(ui, cx, out, table, x, 2);
                            }
                        }
                    },
                );
                // perspectives
                let perspectives = strings(m.get("perspectives"));
                let text = count_text(l.t("Perspectives", "Perspectives"), perspectives.len());
                let menu = [
                    ("perspective.add", json!({}), Via::Invoke),
                    ("perspective.set", json!({}), Via::Invoke),
                ];
                me.branch(
                    ui,
                    cx,
                    out,
                    "per",
                    1,
                    Icon::Eye,
                    &text,
                    &menu,
                    None,
                    |me, ui, out| {
                        for p in perspectives.iter().filter(|p| me.hit(p)) {
                            let resp = row(ui, cx.l, 2, Icon::Eye, p, None, false, false, false);
                            menu_on(
                                &resp,
                                cx,
                                out,
                                &[
                                    ("perspective.set", json!({"name": p}), Via::Invoke),
                                    ("perspective.remove", json!({"name": p}), Via::Invoke),
                                ],
                            );
                        }
                    },
                );
                // relationships
                let rels = objects(m, "relationships");
                let text = count_text(l.t("Relationships", "Relations"), rels.len());
                let menu = [
                    ("relationship.add", json!({}), Via::Invoke),
                    ("relationship.autodetect", json!({}), Via::Invoke),
                ];
                me.branch(
                    ui,
                    cx,
                    out,
                    "rel",
                    1,
                    Icon::Relationship,
                    &text,
                    &menu,
                    None,
                    |me, ui, out| {
                        for (i, r) in rels.iter().filter(|(_, r)| me.hit(&rel_text(r))) {
                            let sel = Sel::Rel(*i);
                            let on = cx.sel == Some(&sel);
                            let dim = r.get("active").and_then(Value::as_bool) == Some(false);
                            let resp = row(
                                ui,
                                cx.l,
                                2,
                                Icon::Relationship,
                                &rel_text(r),
                                None,
                                on,
                                dim,
                                false,
                            );
                            if resp.clicked() {
                                out.sel = Some(Some(sel));
                            }
                            if resp.double_clicked() && cx.editable {
                                out.form = Some(("relationship.set", crate::model_view::rel_values(r)));
                            }
                            let id = crate::model_view::rel_ref(r);
                            menu_on(
                                &resp,
                                cx,
                                out,
                                &[
                                    ("relationship.set", crate::model_view::rel_values(r), Via::Form),
                                    ("relationship.remove", id, Via::Invoke),
                                ],
                            );
                        }
                    },
                );
                // roles
                let roles = strings(m.get("roles"));
                let text = count_text(l.t("Roles", "Rôles"), roles.len());
                let menu = [
                    ("role.add", json!({}), Via::Invoke),
                    ("role.set_permission", json!({}), Via::Invoke),
                ];
                me.branch(
                    ui,
                    cx,
                    out,
                    "rol",
                    1,
                    Icon::Lock,
                    &text,
                    &menu,
                    None,
                    |me, ui, out| {
                        for r in &roles {
                            me.item_row(
                                ui,
                                cx,
                                out,
                                2,
                                Icon::Lock,
                                r,
                                Item::Role(r.to_string()),
                                false,
                                &[
                                    ("role.set_filter", json!({"role": r}), Via::Form),
                                    ("role.set_permission", json!({"role": r}), Via::Invoke),
                                    ("role.set", json!({"name": r}), Via::Form),
                                    ("role.rename", json!({"name": r}), Via::Form),
                                    ("role.remove", json!({"name": r}), Via::Invoke),
                                ],
                            );
                        }
                    },
                );
                // shared expressions: the queries and parameters of the model
                let exprs = strings(m.get("expressions"));
                let text = count_text(l.t("Shared expressions", "Expressions partagées"), exprs.len());
                let menu = [("expression.add", json!({}), Via::Invoke)];
                me.branch(
                    ui,
                    cx,
                    out,
                    "exp",
                    1,
                    Icon::File,
                    &text,
                    &menu,
                    None,
                    |me, ui, out| {
                        for e in &exprs {
                            me.item_row(
                                ui,
                                cx,
                                out,
                                2,
                                Icon::File,
                                e,
                                Item::Expression(e.to_string()),
                                false,
                                &[
                                    ("expression.set", json!({"name": e}), Via::Form),
                                    ("expression.rename", json!({"name": e}), Via::Invoke),
                                    ("expression.remove", json!({"name": e}), Via::Invoke),
                                ],
                            );
                        }
                    },
                );
                // tables
                let text = count_text(l.t("Tables", "Tables"), tables.len());
                me.branch(
                    ui,
                    cx,
                    out,
                    "tab",
                    1,
                    Icon::Table,
                    &text,
                    &[],
                    None,
                    |me, ui, out| {
                        for (_, t) in &tables {
                            me.table_ui(ui, cx, out, t, 2);
                        }
                    },
                );
            },
        );
    }

    /// A calculation group and its items (drag an item onto another to move it there).
    fn group_ui(&mut self, ui: &mut Ui, cx: &Cx<'_>, out: &mut Out, g: &Value) {
        let group = s(g, "name").to_string();
        let items: Vec<String> = strings(g.get("calculation_items"))
            .into_iter()
            .map(str::to_string)
            .collect();
        let menu = [
            ("calculation_item.add", json!({"group": group}), Via::Invoke),
            ("calculation_group.set", json!({"name": group}), Via::Form),
            ("calculation_group.remove", json!({"name": group}), Via::Invoke),
        ];
        let key = format!("cg:{group}");
        let sel = Some(Sel::Table(group.clone()));
        self.branch(
            ui,
            cx,
            out,
            &key,
            2,
            Icon::Calculator,
            &group,
            &menu,
            sel,
            |me, ui, out| {
                for (k, name) in items.iter().enumerate() {
                    let item = Item::CalcItem {
                        group: group.clone(),
                        name: name.clone(),
                    };
                    let menu = [
                        (
                            "calculation_item.set",
                            json!({"group": group, "name": name}),
                            Via::Form,
                        ),
                        (
                            "calculation_item.remove",
                            json!({"group": group, "name": name}),
                            Via::Invoke,
                        ),
                    ];
                    let Some(resp) =
                        me.item_row(ui, cx, out, 3, Icon::Calculator, name, item, cx.editable, &menu)
                    else {
                        continue;
                    };
                    resp.dnd_set_drag_payload(ItemDrag {
                        group: group.clone(),
                        name: name.clone(),
                    });
                    // the drop target: a line above the row the item goes to
                    if let Some(d) = resp.dnd_hover_payload::<ItemDrag>()
                        && d.group == group
                        && d.name != *name
                    {
                        let t = Tokens::get(ui.ctx());
                        let y = resp.rect.top();
                        ui.painter().line_segment(
                            [pos2(resp.rect.left(), y), pos2(resp.rect.right(), y)],
                            Stroke::new(2.0, t.ring),
                        );
                    }
                    if let Some(d) = resp.dnd_release_payload::<ItemDrag>()
                        && d.group == group
                        && d.name != *name
                        && cx.editable
                    {
                        out.run = Some((
                            "calculation_item.set",
                            json!({"group": group, "name": d.name, "ordinal": k}),
                        ));
                    }
                }
            },
        );
    }

    /// A table with its columns and measures.
    fn table_ui(&mut self, ui: &mut Ui, cx: &Cx<'_>, out: &mut Out, t: &Value, depth: usize) {
        let name = s(t, "name").to_string();
        let menu = [
            ("table.rename", json!({"name": name}), Via::Invoke),
            ("table.remove", json!({"name": name}), Via::Invoke),
            (
                "tmdl.script",
                json!({"objects": [{"kind": "table", "name": name}]}),
                Via::Invoke,
            ),
        ];
        // the search: the table when its name or one of its fields holds the text
        let all = self.hit(&name);
        let fields = || {
            ["columns", "measures"]
                .into_iter()
                .flat_map(|k| objects(t, k).into_iter().map(|(_, x)| s(x, "name")))
                .chain(strings(t.get("hierarchies")))
        };
        if !all && !fields().any(|n| self.hit(n)) {
            return;
        }
        // a layout other than the one of all tables: Add to layout, or Add related tables and
        // Remove from layout for a table it shows
        let mut add = None;
        let mut related = None;
        let mut menu = menu.to_vec();
        if let Some((layout, (x, y), has)) = cx.layout {
            if !has.contains(&name) {
                add = Some((
                    "diagram.move",
                    json!({"table": name, "x": x, "y": y, "diagram": layout}),
                ));
            } else {
                let mut others: Vec<&str> = Vec::new();
                for (_, r) in objects(cx.model, "relationships") {
                    let (a, b) = (s(r, "from_table"), s(r, "to_table"));
                    let other = if a == name {
                        b
                    } else if b == name {
                        a
                    } else {
                        continue;
                    };
                    if !other.is_empty() && !has.contains(other) && !others.contains(&other) {
                        others.push(other);
                    }
                }
                if !others.is_empty() {
                    let places: Vec<Value> = others
                        .iter()
                        .enumerate()
                        .map(|(k, o)| json!({"table": o, "x": x + k as f64 * RELATED_GAP, "y": y}))
                        .collect();
                    related = Some(("diagram.move", json!({"tables": places, "diagram": layout})));
                }
                menu.insert(
                    0,
                    (
                        "diagram.remove_tables",
                        json!({"tables": [name], "diagram": layout}),
                        Via::Invoke,
                    ),
                );
            }
        }
        let key = format!("t:{name}");
        let open = self.is_open(&key);
        let sel = Sel::Table(name.clone());
        let selected = cx.on(&sel);
        let r = row(
            ui,
            cx.l,
            depth,
            Icon::Table,
            &name,
            Some(open),
            selected,
            yes(t, "hidden"),
            false,
        );
        self.drawing.push(sel.clone());
        // a plain click opens or closes it too
        if r.clicked() && pick(ui, out, sel) {
            self.toggle(&key);
        }
        let l = cx.l;
        crate::keyboard::context_menu(&r, |ui| {
            ui.set_min_width(200.0);
            for (text, cmd) in [
                (l.t("Add to layout", "Ajouter à la disposition"), &add),
                (
                    l.t("Add related tables", "Ajouter les tables associées"),
                    &related,
                ),
            ] {
                if let Some((id, p)) = cmd {
                    let st = state(cx, id);
                    if crate::menus::item(ui, text, &st, l) {
                        out.run = Some((id, p.clone()));
                        ui.close();
                    }
                }
            }
            items(ui, cx, out, &menu);
        });
        if !open {
            return;
        }
        for (_, c) in objects(t, "columns") {
            let cname = s(c, "name");
            if !all && !self.hit(cname) {
                continue;
            }
            let sel = Sel::Column {
                table: name.clone(),
                name: cname.to_string(),
            };
            let icon = if yes(c, "calculated") {
                Icon::Calculator
            } else {
                Icon::Column
            };
            let resp = row(
                ui,
                cx.l,
                depth + 1,
                icon,
                cname,
                None,
                cx.on(&sel),
                yes(c, "hidden"),
                false,
            );
            self.drawing.push(sel.clone());
            if resp.clicked() {
                pick(ui, out, sel);
            }
            menu_on(
                &resp,
                cx,
                out,
                &[
                    (
                        "column.rename",
                        json!({"table": name, "name": cname}),
                        Via::Invoke,
                    ),
                    (
                        "column.remove",
                        json!({"table": name, "name": cname}),
                        Via::Invoke,
                    ),
                ],
            );
        }
        // hierarchies, each with its levels
        for (_, h) in objects(t, "hierarchy_levels") {
            let hname = s(h, "name");
            if !all && !self.hit(hname) {
                continue;
            }
            let item = Item::Hierarchy {
                table: name.clone(),
                name: hname.to_string(),
            };
            let sel = Sel::Item(item.clone());
            let hkey = format!("h:{name}:{hname}");
            let hopen = self.open.contains(&hkey);
            let resp = row(
                ui,
                cx.l,
                depth + 1,
                Icon::Hierarchy,
                hname,
                Some(hopen),
                cx.sel == Some(&sel),
                yes(h, "hidden"),
                false,
            );
            if resp.clicked() {
                self.toggle(&hkey);
                out.sel = Some(Some(sel));
            }
            menu_on(
                &resp,
                cx,
                out,
                &[
                    ("hierarchy.set", json!({"table": name, "name": hname}), Via::Form),
                    (
                        "hierarchy.remove",
                        json!({"table": name, "name": hname}),
                        Via::Invoke,
                    ),
                ],
            );
            if hopen {
                for (_, level) in objects(h, "levels") {
                    row(
                        ui,
                        cx.l,
                        depth + 2,
                        Icon::Column,
                        s(level, "name"),
                        None,
                        false,
                        false,
                        false,
                    );
                }
            }
        }
        for (_, x) in objects(t, "measures") {
            if !all && !self.hit(s(x, "name")) {
                continue;
            }
            self.drawing.push(Sel::Measure {
                table: name.clone(),
                name: s(x, "name").to_string(),
            });
            measure_row(ui, cx, out, &name, x, depth + 1);
        }
    }
}

/// How far apart Add related tables puts the cards (diagram pixels).
const RELATED_GAP: f64 = 280.0;

/// A section's text with its count: `Roles (2)`.
fn count_text(text: &str, n: usize) -> String {
    format!("{text} ({n})")
}

fn measure_row(ui: &mut Ui, cx: &Cx<'_>, out: &mut Out, table: &str, x: &Value, depth: usize) {
    let name = s(x, "name");
    let sel = Sel::Measure {
        table: table.to_string(),
        name: name.to_string(),
    };
    let resp = row(
        ui,
        cx.l,
        depth,
        Icon::Measure,
        name,
        None,
        cx.on(&sel),
        yes(x, "hidden"),
        false,
    );
    if resp.clicked() {
        pick(ui, out, sel);
    }
    menu_on(
        &resp,
        cx,
        out,
        &[
            (
                "measure.rename",
                json!({"table": table, "name": name}),
                Via::Invoke,
            ),
            (
                "measure.remove",
                json!({"table": table, "name": name}),
                Via::Invoke,
            ),
            (
                "tmdl.script",
                json!({"objects": [{"kind": "measure", "table": table, "name": name}]}),
                Via::Invoke,
            ),
        ],
    );
}

fn state(cx: &Cx<'_>, id: &str) -> Result<(), String> {
    cx.states
        .get(id)
        .cloned()
        .unwrap_or_else(|| Err("not available".into()))
}

/// The right-click menu of a row (nothing when it has no items).
fn menu_on(r: &egui::Response, cx: &Cx<'_>, out: &mut Out, menu: &[(&'static str, Value, Via)]) {
    if menu.is_empty() {
        return;
    }
    crate::keyboard::context_menu(r, |ui| {
        ui.set_min_width(200.0);
        items(ui, cx, out, menu);
    });
}

/// The menu's items: the registry's button text, greyed with the reason when unavailable.
fn items(ui: &mut Ui, cx: &Cx<'_>, out: &mut Out, menu: &[(&'static str, Value, Via)]) {
    for (id, p, via) in menu {
        let Some(c) = registry::find(id) else { continue };
        let st = state(cx, id);
        if crate::menus::item(ui, c.text(cx.l), &st, cx.l) {
            match via {
                Via::Invoke => out.picked = Some((c.id, p.clone())),
                Via::Form => out.form = Some((c.id, p.clone())),
            }
            ui.close();
        }
    }
}

/// One row: a chevron for a branch (`open`), the icon, the name; selected rows are lit, hidden
/// or inactive objects dimmed. Its accessible label is its text (a generic name when empty).
#[allow(clippy::too_many_arguments)]
fn row(
    ui: &mut Ui,
    l: Lang,
    depth: usize,
    icon: Icon,
    text: &str,
    open: Option<bool>,
    selected: bool,
    dim: bool,
    drag: bool,
) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let sense = if drag {
        Sense::click_and_drag()
    } else {
        Sense::click()
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(60.0), ROW_H), sense);
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, CornerRadius::same(4), t.selected);
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(4), t.hover);
    }
    let x = rect.min.x + depth as f32 * INDENT;
    let c = pos2(x + 7.0, rect.center().y);
    let ink = if dim { t.text_faint } else { t.text };
    match open {
        Some(true) => widgets::chevron(p, c, 3.5, ink),
        Some(false) => {
            p.line_segment(
                [pos2(c.x - 2.0, c.y - 4.0), pos2(c.x + 2.0, c.y)],
                Stroke::new(1.2, ink),
            );
            p.line_segment(
                [pos2(c.x + 2.0, c.y), pos2(c.x - 2.0, c.y + 4.0)],
                Stroke::new(1.2, ink),
            );
        }
        None => {}
    }
    let ir = Rect::from_center_size(pos2(x + 24.0, rect.center().y), vec2(16.0, 16.0));
    icons::paint(p, ir, icon, ink, dim);
    p.text(
        pos2(x + 36.0, rect.center().y),
        Align2::LEFT_CENTER,
        clip(text, MAX_LABEL),
        theme::font(13.0),
        ink,
    );
    // a nameless object (a damaged snapshot) still has an accessible name
    let label = if text.trim().is_empty() {
        l.t("Object without a name", "Objet sans nom").to_string()
    } else {
        text.to_string()
    };
    resp.widget_info(|| match open {
        Some(open) => egui::WidgetInfo::selected(egui::Role::Button, true, open, &label),
        None => egui::WidgetInfo::selected(egui::Role::Button, true, selected, &label),
    });
    if drag && resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    resp
}
