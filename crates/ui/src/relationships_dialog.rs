//! Modeling › Manage relationships (Power BI Desktop's dialog, as Microsoft Learn's "Create and
//! manage relationships in Power BI Desktop" describes it): the list of the model's relationships
//! with their active check box, from and to columns, cardinality and cross filter direction;
//! New, Autodetect, Edit and Delete. The Create / Edit relationship window picks a table for each
//! end, shows a preview of its first rows (`table.rows`, the Data view's read) and takes the column
//! from a click on the preview's header; cardinality, cross filter direction, "Make this
//! relationship active", "Assume referential integrity" and "Apply security filter in both
//! directions" below. Each change runs `relationship.add` / `relationship.set` with `check: true`
//! (nothing changes): the review's errors and warnings show at once, and a change of columns takes
//! the cardinality, direction and activity the data suggests. OK runs the same command without
//! `check` (one undo step).
//!
//! Every button runs an engine command: `relationship.list` (the list, read again at each new
//! snapshot revision), `relationship.add`, `relationship.set`, `relationship.remove`,
//! `relationship.autodetect` (its own dialog, [`crate::autodetect`]).

use egui::{RichText, Ui};
use serde_json::{Map, Value, json};

use crate::App;
use crate::actions::Call;
use crate::app::Then;
use crate::i18n::Lang;
use crate::theme::{self, Tokens};
use crate::widgets;

/// Rows each preview shows.
const PREVIEW_ROWS: usize = 5;
/// Most relationships the list shows.
const MAX_LISTED: usize = 2_000;
/// Characters of a preview cell.
const CELL_CHARS: usize = 24;

/// What an answer is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// `relationship.list`.
    List,
    /// `table.rows` of the table picked for end `side` (0 or 1).
    Preview { side: usize, table: String },
    /// A review (`check: true`), by its sequence number.
    Check(u64),
    /// OK (`relationship.add` / `relationship.set`).
    Apply,
    /// A row's Active box or Delete.
    Change,
}

/// One relationship of the list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    pub from_table: String,
    pub from_column: String,
    pub to_table: String,
    pub to_column: String,
    pub active: bool,
    pub from_card: String,
    pub to_card: String,
    pub both: bool,
    pub security_both: bool,
    pub rely: bool,
}

impl Row {
    /// How the `relationship.*` commands name it.
    fn reference(&self) -> Value {
        if self.name.is_empty() {
            json!({
                "from": format!("{}[{}]", self.from_table, self.from_column),
                "to": format!("{}[{}]", self.to_table, self.to_column),
            })
        } else {
            json!({"name": self.name})
        }
    }
    fn card(&self) -> Card {
        match (self.from_card.as_str(), self.to_card.as_str()) {
            ("one", "one") => Card::OneToOne,
            ("many", "many") => Card::ManyToMany,
            ("one", "many") => Card::OneToMany,
            _ => Card::ManyToOne,
        }
    }
}

/// The Manage relationships list while it is open.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manage {
    /// `None` while the list is read.
    pub rows: Option<Vec<Row>>,
    /// The selected row.
    pub sel: Option<usize>,
    /// The snapshot revision the list was read at.
    pub rev: Option<u64>,
    pub error: Option<String>,
}

/// The cardinality picker's four choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Card {
    #[default]
    ManyToOne,
    OneToOne,
    OneToMany,
    ManyToMany,
}

impl Card {
    const ALL: [Card; 4] = [Card::ManyToOne, Card::OneToOne, Card::OneToMany, Card::ManyToMany];
    fn label(self, l: Lang) -> &'static str {
        match self {
            Card::ManyToOne => l.t("Many to one (*:1)", "Plusieurs à un (*:1)"),
            Card::OneToOne => l.t("One to one (1:1)", "Un à un (1:1)"),
            Card::OneToMany => l.t("One to many (1:*)", "Un à plusieurs (1:*)"),
            Card::ManyToMany => l.t("Many to many (*:*)", "Plusieurs à plusieurs (*:*)"),
        }
    }
    fn of(code: &str) -> Option<Card> {
        match code {
            "many_to_one" => Some(Card::ManyToOne),
            "one_to_one" => Some(Card::OneToOne),
            "one_to_many" => Some(Card::OneToMany),
            "many_to_many" => Some(Card::ManyToMany),
            _ => None,
        }
    }
}

/// One end's table, column and preview.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Side {
    pub table: String,
    pub column: Option<String>,
    /// The table the preview was asked for.
    pub asked: String,
    /// The preview's rows by column name order of `columns` (`None` while read).
    pub preview: Option<Preview>,
}

/// A table's first rows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Preview {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    /// Why there is no preview (DirectQuery, no data).
    pub note: Option<String>,
}

/// The review of the relationship as it stands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Verdict {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// The Create / Edit relationship window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Editor {
    /// The relationship edited (`None`: a new one).
    pub old: Option<Row>,
    pub sides: [Side; 2],
    pub card: Card,
    /// Cross filter in both directions.
    pub both: bool,
    pub active: bool,
    /// A new relationship's activity is the engine's (active when it can be) until the box is
    /// clicked: `active` is then left out of the calls.
    pub active_auto: bool,
    pub rely: bool,
    pub security_both: bool,
    /// The last review asked, its sequence number and whether its suggestion is taken.
    pub asked: String,
    pub seq: u64,
    pub adopt: bool,
    pub verdict: Option<Verdict>,
    pub error: Option<String>,
}

impl Editor {
    fn new_one() -> Self {
        Self {
            active: true,
            active_auto: true,
            ..Self::default()
        }
    }

    fn of(r: &Row) -> Self {
        let side = |t: &str, c: &str| Side {
            table: t.to_string(),
            column: Some(c.to_string()),
            ..Side::default()
        };
        Self {
            old: Some(r.clone()),
            sides: [
                side(&r.from_table, &r.from_column),
                side(&r.to_table, &r.to_column),
            ],
            card: r.card(),
            both: r.both,
            active: r.active,
            rely: r.rely,
            security_both: r.security_both,
            ..Self::default()
        }
    }

    /// The ends as the commands take them: the "many" side first (one-to-many swaps them).
    fn ends(&self) -> Option<(String, String, &'static str, &'static str)> {
        let refs: Vec<String> = self
            .sides
            .iter()
            .map(|s| s.column.as_ref().map(|c| format!("{}[{c}]", s.table)))
            .collect::<Option<Vec<_>>>()?;
        let (a, b) = (refs.first()?.clone(), refs.get(1)?.clone());
        Some(match self.card {
            Card::ManyToOne => (a, b, "many", "one"),
            Card::OneToMany => (b, a, "many", "one"),
            Card::OneToOne => (a, b, "one", "one"),
            Card::ManyToMany => (a, b, "many", "many"),
        })
    }

    /// The command and parameters OK runs (`check` adds `check: true`).
    pub fn call(&self, check: bool) -> Option<Call> {
        let (from, to, fc, tc) = self.ends()?;
        let both = self.both || self.card == Card::OneToOne;
        let mut p = Map::new();
        let id = match &self.old {
            None => {
                p.insert("from".into(), json!(from));
                p.insert("to".into(), json!(to));
                if self.security_both && both {
                    p.insert("security_filtering".into(), json!("both"));
                }
                if self.rely {
                    p.insert("assume_referential_integrity".into(), json!(true));
                }
                "relationship.add"
            }
            Some(old) => {
                if let Value::Object(r) = old.reference() {
                    p.extend(r);
                }
                p.insert("new_from".into(), json!(from));
                p.insert("new_to".into(), json!(to));
                let sec = self.security_both && both;
                if sec != old.security_both {
                    p.insert(
                        "security_filtering".into(),
                        json!(if sec { "both" } else { "single" }),
                    );
                }
                if self.rely != old.rely {
                    p.insert("assume_referential_integrity".into(), json!(self.rely));
                }
                "relationship.set"
            }
        };
        p.insert("from_cardinality".into(), json!(fc));
        p.insert("to_cardinality".into(), json!(tc));
        p.insert("cross_filter".into(), json!(if both { "both" } else { "single" }));
        if !(self.old.is_none() && self.active_auto) {
            p.insert("active".into(), json!(self.active));
        }
        if check {
            p.insert("check".into(), json!(true));
        }
        Some(Call {
            id,
            params: Value::Object(p),
        })
    }
}

fn text(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn row_of(v: &Value) -> Row {
    let lower = |k: &str| text(v, k).to_ascii_lowercase();
    Row {
        name: text(v, "name"),
        from_table: text(v, "from_table"),
        from_column: text(v, "from_column"),
        to_table: text(v, "to_table"),
        to_column: text(v, "to_column"),
        active: v.get("is_active").and_then(Value::as_bool) != Some(false),
        from_card: lower("from_cardinality"),
        to_card: lower("to_cardinality"),
        both: lower("cross_filter") == "both",
        security_both: lower("security_filtering_behavior") == "bothdirections",
        rely: v.get("rely_on_referential_integrity").and_then(Value::as_bool) == Some(true),
    }
}

/// The snapshot's model.
fn model(app: &App) -> Option<&Value> {
    app.loaded.as_ref().and_then(|l| l.snap.get("model"))
}

fn revision(app: &App) -> Option<u64> {
    app.loaded
        .as_ref()
        .and_then(|l| l.snap.get("revision").and_then(Value::as_u64))
}

/// The model's tables: (name, columns, queried live).
fn tables(app: &App) -> Vec<(String, Vec<String>, bool)> {
    let Some(ts) = model(app).and_then(|m| m.get("tables")).and_then(Value::as_array) else {
        return Vec::new();
    };
    ts.iter()
        .filter_map(|t| {
            let name = t.get("name").and_then(Value::as_str)?.to_string();
            let columns = t
                .get("columns")
                .and_then(Value::as_array)
                .map(|cs| {
                    cs.iter()
                        .filter_map(|c| c.get("name").and_then(Value::as_str))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let live = matches!(
                t.get("storage_mode").and_then(Value::as_str),
                Some("directQuery" | "dual")
            );
            Some((name, columns, live))
        })
        .collect()
}

/// Manage relationships, New relationship and Edit relationship without the values to write open
/// the dialogs.
pub fn intercept(app: &mut App, id: &str, params: &Value) -> bool {
    let given = |k: &str| params.get(k).is_some_and(|v| !v.is_null() && v != "");
    match id {
        "relationship.list" if !given("table") => {
            open(app);
            true
        }
        "relationship.add" if !(given("from") && given("to")) => {
            app.cmd.rel_edit = Some(Editor::new_one());
            true
        }
        "relationship.set" => {
            const CHANGES: [&str; 10] = [
                "new_name",
                "new_from",
                "new_to",
                "active",
                "cross_filter",
                "from_cardinality",
                "to_cardinality",
                "security_filtering",
                "assume_referential_integrity",
                "check",
            ];
            if CHANGES.iter().any(|k| given(k)) {
                return false;
            }
            edit(app, params);
            true
        }
        _ => false,
    }
}

/// Opens the Manage relationships list.
pub fn open(app: &mut App) {
    app.cmd.rels = Some(Manage::default());
    list(app);
}

fn list(app: &mut App) {
    let rev = revision(app);
    if let Some(m) = app.cmd.rels.as_mut() {
        m.rev = rev;
    }
    app.run(
        Call {
            id: "relationship.list",
            params: json!({}),
        },
        Then::Rels(Step::List),
    );
}

/// Opens the Edit relationship window on the relationship `r` names (`{name}` or `{from, to}`),
/// as a double click on a relationship line does. Unknown: the Manage list opens instead.
pub fn edit(app: &mut App, r: &Value) {
    let Some(rels) = model(app)
        .and_then(|m| m.get("relationships"))
        .and_then(Value::as_array)
    else {
        open(app);
        return;
    };
    let name = text(r, "name");
    let (from, to) = (text(r, "from"), text(r, "to"));
    let found = rels.iter().find(|x| {
        if name.is_empty() {
            format!("{}[{}]", text(x, "from_table"), text(x, "from_column")) == from
                && format!("{}[{}]", text(x, "to_table"), text(x, "to_column")) == to
        } else {
            text(x, "name") == name
        }
    });
    match found {
        Some(x) => {
            // the snapshot names the flags as the list does
            let mut row = row_of(x);
            row.active = x.get("active").and_then(Value::as_bool) != Some(false);
            row.security_both = text(x, "security_filtering").eq_ignore_ascii_case("both");
            row.both = text(x, "cross_filter") == "both";
            row.rely = x.get("assume_referential_integrity").and_then(Value::as_bool) == Some(true);
            app.cmd.rel_edit = Some(Editor::of(&row));
        }
        None => open(app),
    }
}

/// The answers of the dialogs' commands.
pub fn answer(app: &mut App, step: Step, r: Result<Value, String>) {
    let l = app.lang;
    match step {
        Step::List => {
            let Some(m) = app.cmd.rels.as_mut() else {
                return;
            };
            match r {
                Ok(v) => {
                    let rows: Vec<Row> = v
                        .get("relationships")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .take(MAX_LISTED)
                        .map(row_of)
                        .collect();
                    if m.sel.is_some_and(|i| i >= rows.len()) {
                        m.sel = None;
                    }
                    m.rows = Some(rows);
                }
                Err(e) => {
                    m.error = Some(crate::app::clip(&e, 600));
                    m.rows.get_or_insert_with(Vec::new);
                }
            }
        }
        Step::Change => {
            if let Err(e) = r {
                match app.cmd.rels.as_mut() {
                    Some(m) => m.error = Some(crate::app::clip(&e, 600)),
                    None => app.error = Some(e),
                }
            }
        }
        Step::Preview { side, table } => {
            let Some(s) = app.cmd.rel_edit.as_mut().and_then(|e| e.sides.get_mut(side)) else {
                return;
            };
            if s.asked != table {
                return;
            }
            s.preview = Some(match r {
                Ok(v) => preview_of(&v, l),
                Err(e) => Preview {
                    note: Some(crate::app::clip(&e, 200)),
                    ..Preview::default()
                },
            });
        }
        Step::Check(seq) => {
            let Some(e) = app.cmd.rel_edit.as_mut() else {
                return;
            };
            if seq != e.seq {
                return;
            }
            match r {
                Ok(v) => {
                    let msgs = |k: &str| -> Vec<String> {
                        v.get(k)
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .map(|f| crate::app::clip(&text(f, "message"), 400))
                            .collect()
                    };
                    if std::mem::take(&mut e.adopt) {
                        if let Some(c) = v
                            .pointer("/suggested/cardinality")
                            .and_then(Value::as_str)
                            .and_then(Card::of)
                        {
                            e.card = c;
                        }
                        if let Some(c) = v.pointer("/suggested/cross_filter").and_then(Value::as_str) {
                            e.both = c == "both";
                        }
                        if let Some(a) = v.get("can_be_active").and_then(Value::as_bool)
                            && !e.active_auto
                        {
                            e.active = a;
                        }
                    }
                    if e.active_auto
                        && let Some(a) = v.get("active").and_then(Value::as_bool)
                    {
                        e.active = a;
                    }
                    e.verdict = Some(Verdict {
                        errors: msgs("errors"),
                        warnings: msgs("warnings"),
                    });
                }
                Err(err) => {
                    e.verdict = Some(Verdict {
                        errors: vec![crate::app::clip(&err, 400)],
                        warnings: Vec::new(),
                    })
                }
            }
        }
        Step::Apply => match r {
            Ok(_) => {
                app.cmd.rel_edit = None;
                app.status = l.t("Relationship saved", "Relation enregistrée").to_string();
            }
            Err(err) => {
                if let Some(e) = app.cmd.rel_edit.as_mut() {
                    e.error = Some(crate::app::clip(&err, 600));
                } else {
                    app.error = Some(err);
                }
            }
        },
    }
}

fn cell(v: &Value) -> String {
    let s = match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if s.chars().count() > CELL_CHARS {
        let mut t: String = s.chars().take(CELL_CHARS.saturating_sub(1)).collect();
        t.push('…');
        t
    } else {
        s
    }
}

fn preview_of(v: &Value, l: Lang) -> Preview {
    let columns: Vec<String> = v
        .get("columns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|c| text(c, "name"))
        .collect();
    let rows: Vec<Vec<String>> = v
        .get("rows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(PREVIEW_ROWS)
        .map(|r| {
            r.as_array()
                .map(|r| r.iter().map(cell).collect())
                .unwrap_or_default()
        })
        .collect();
    let note = (v.get("available") == Some(&Value::Bool(false))).then(|| {
        let why = text(v, "reason");
        if why.is_empty() {
            l.t(
                "No preview: the session holds no rows of this table.",
                "Pas d'aperçu : la session ne contient aucune ligne de cette table.",
            )
            .to_string()
        } else {
            crate::app::clip(&why, 200)
        }
    });
    Preview { columns, rows, note }
}

/// Draws the open dialogs, reads the list again at a new revision, asks the previews and the
/// reviews the window needs.
pub fn show(app: &mut App, ctx: &egui::Context) {
    if app.cmd.rels.is_none() && app.cmd.rel_edit.is_none() {
        return;
    }
    let l = app.lang;
    let all = tables(app);
    // the list follows every change (its own, Autodetect's, undo)
    if app.cmd.rels.as_ref().is_some_and(|m| m.rev != revision(app)) {
        list(app);
    }
    let mut calls: Vec<(Call, Step)> = Vec::new();
    if let Some(mut m) = app.cmd.rels.clone() {
        let mut open = true;
        let mut close = false;
        let mut new = false;
        let mut autodetect = false;
        let mut edit_row = None;
        egui::Window::new(l.t("Manage relationships", "Gérer les relations"))
            .id(egui::Id::unique("bicraft-manage-relationships"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.set_width(720.0);
                manage_body(
                    ui,
                    &mut m,
                    l,
                    &mut calls,
                    &mut new,
                    &mut autodetect,
                    &mut edit_row,
                    &mut close,
                );
            });
        let editing = app.cmd.rel_edit.is_some();
        if !editing && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
        app.cmd.rels = (open && !close).then_some(m);
        if new {
            app.cmd.rel_edit = Some(Editor::new_one());
        }
        if let Some(r) = edit_row {
            app.cmd.rel_edit = Some(Editor::of(&r));
        }
        if autodetect {
            crate::ribbon::invoke(app, crate::autodetect::COMMAND, json!({}));
        }
    }
    if let Some(mut e) = app.cmd.rel_edit.clone() {
        let mut open = true;
        let mut close = false;
        let mut ok = None;
        let title = if e.old.is_some() {
            l.t("Edit relationship", "Modifier la relation")
        } else {
            l.t("New relationship", "Nouvelle relation")
        };
        egui::Window::new(title)
            .id(egui::Id::unique("bicraft-edit-relationship"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.set_width(720.0);
                ok = editor_body(ui, &mut e, &all, l, &mut close);
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
        // the previews of the tables picked
        for (i, s) in e.sides.iter_mut().enumerate() {
            if !s.table.is_empty() && s.asked != s.table {
                s.asked = s.table.clone();
                s.preview = None;
                calls.push((
                    Call {
                        id: "table.rows",
                        params: json!({"table": s.table, "limit": PREVIEW_ROWS}),
                    },
                    Step::Preview {
                        side: i,
                        table: s.table.clone(),
                    },
                ));
            }
        }
        // the review of the relationship as it stands
        match e.call(true) {
            Some(c) => {
                let key = c.params.to_string();
                if key != e.asked {
                    e.asked = key;
                    e.seq = e.seq.saturating_add(1);
                    calls.push((c, Step::Check(e.seq)));
                }
            }
            None => {
                e.asked.clear();
                e.verdict = None;
            }
        }
        if let Some(c) = ok {
            e.error = None;
            calls.push((c, Step::Apply));
        }
        app.cmd.rel_edit = (open && !close).then_some(e);
    }
    for (c, step) in calls {
        app.run(c, Then::Rels(step));
    }
}

#[allow(clippy::too_many_arguments)]
fn manage_body(
    ui: &mut Ui,
    m: &mut Manage,
    l: Lang,
    calls: &mut Vec<(Call, Step)>,
    new: &mut bool,
    autodetect: &mut bool,
    edit_row: &mut Option<Row>,
    close: &mut bool,
) {
    let t = Tokens::get(ui.ctx());
    let Some(rows) = m.rows.clone() else {
        ui.label(l.t("Reading the relationships…", "Lecture des relations…"));
        return;
    };
    let one = |c: &str| if c == "one" { "1" } else { "*" };
    egui::ScrollArea::vertical()
        .id_salt("bicraft-relationships-list")
        .max_height(300.0)
        .show(ui, |ui| {
            if rows.is_empty() {
                ui.label(
                    RichText::new(l.t(
                        "No relationship yet: New creates one, Autodetect looks for them.",
                        "Aucune relation pour l'instant : Nouvelle en crée une, Détecter les cherche.",
                    ))
                    .color(t.text_dim),
                );
                return;
            }
            egui::Grid::new("bicraft-relationships-grid")
                .num_columns(5)
                .striped(true)
                .spacing(egui::vec2(14.0, 6.0))
                .show(ui, |ui| {
                    for h in [
                        l.t("Active", "Active"),
                        l.t("From: Table (Column)", "De : Table (Colonne)"),
                        l.t("To: Table (Column)", "Vers : Table (Colonne)"),
                        l.t("Cardinality", "Cardinalité"),
                        l.t("Cross filter", "Filtrage croisé"),
                    ] {
                        ui.label(RichText::new(h).font(theme::bold(12.0)));
                    }
                    ui.end_row();
                    for (i, r) in rows.iter().enumerate() {
                        let from = format!("{} ({})", r.from_table, r.from_column);
                        let to = format!("{} ({})", r.to_table, r.to_column);
                        let mut a = r.active;
                        let label = format!("{} {from} → {to}", l.t("Active:", "Active :"));
                        let resp = ui.checkbox(&mut a, "");
                        resp.widget_info(|| {
                            egui::WidgetInfo::selected(egui::Role::CheckBox, true, a, &label)
                        });
                        if resp.changed() {
                            let mut p = r.reference();
                            if let Value::Object(o) = &mut p {
                                o.insert("active".into(), json!(a));
                            }
                            m.error = None;
                            calls.push((
                                Call {
                                    id: "relationship.set",
                                    params: p,
                                },
                                Step::Change,
                            ));
                        }
                        let s = ui.selectable_label(m.sel == Some(i), &from);
                        if s.clicked() {
                            m.sel = Some(i);
                        }
                        if s.double_clicked() {
                            *edit_row = Some(r.clone());
                        }
                        ui.label(&to);
                        ui.label(format!("{}:{}", one(&r.from_card), one(&r.to_card)));
                        ui.label(if r.both {
                            l.t("Both", "Les deux")
                        } else {
                            l.t("Single", "Unique")
                        });
                        ui.end_row();
                    }
                });
        });
    if let Some(e) = &m.error {
        ui.label(RichText::new(format!("⚠ {e}")).color(t.warn));
    }
    ui.add_space(8.0);
    let selected = m.sel.and_then(|i| rows.get(i)).cloned();
    let why = l.t("Select a relationship first", "Sélectionnez d'abord une relation");
    ui.horizontal(|ui| {
        if ui.button(l.t("New…", "Nouvelle…")).clicked() {
            *new = true;
        }
        if ui.button(l.t("Autodetect…", "Détecter…")).clicked() {
            *autodetect = true;
        }
        let r = ui
            .add_enabled(selected.is_some(), egui::Button::new(l.t("Edit…", "Modifier…")))
            .on_disabled_hover_text(why);
        if r.clicked() {
            *edit_row = selected.clone();
        }
        let r = ui
            .add_enabled(selected.is_some(), egui::Button::new(l.t("Delete", "Supprimer")))
            .on_disabled_hover_text(why);
        if r.clicked()
            && let Some(row) = &selected
        {
            m.sel = None;
            m.error = None;
            calls.push((
                Call {
                    id: "relationship.remove",
                    params: row.reference(),
                },
                Step::Change,
            ));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::primary(ui, l.t("Close", "Fermer"), true, "").clicked() {
                *close = true;
            }
        });
    });
}

/// A labelled combo box of choices; returns the index picked.
fn combo(
    ui: &mut Ui,
    id: &str,
    label: &str,
    current: &str,
    choices: &[String],
    enabled: bool,
) -> Option<usize> {
    let mut picked = None;
    ui.add_enabled_ui(enabled, |ui| {
        let r = egui::ComboBox::from_id_salt(id)
            .selected_text(current)
            .width(260.0)
            .show_ui(ui, |ui| {
                for (i, c) in choices.iter().enumerate() {
                    if ui.selectable_label(c == current, c).clicked() {
                        picked = Some(i);
                    }
                }
            });
        r.response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, enabled, label));
    });
    picked
}

fn editor_body(
    ui: &mut Ui,
    e: &mut Editor,
    all: &[(String, Vec<String>, bool)],
    l: Lang,
    close: &mut bool,
) -> Option<Call> {
    let t = Tokens::get(ui.ctx());
    ui.label(
        RichText::new(l.t(
            "Pick a table for each end, then click the column in its preview.",
            "Choisissez une table pour chaque extrémité, puis cliquez sur la colonne dans son aperçu.",
        ))
        .color(t.text_dim),
    );
    let names: Vec<String> = all.iter().map(|(n, _, _)| n.clone()).collect();
    let mut changed_columns = false;
    for side in 0..2 {
        ui.add_space(6.0);
        let label = if side == 0 {
            l.t("First table", "Première table")
        } else {
            l.t("Second table", "Deuxième table")
        };
        let Some(s) = e.sides.get_mut(side) else {
            continue;
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(label).font(theme::bold(13.0)));
            let shown = if s.table.is_empty() {
                l.t("Select a table", "Sélectionnez une table").to_string()
            } else {
                s.table.clone()
            };
            if let Some(i) = combo(
                ui,
                &format!("bicraft-rel-table-{side}"),
                label,
                &shown,
                &names,
                true,
            ) && let Some(n) = names.get(i)
                && *n != s.table
            {
                s.table = n.clone();
                s.column = None;
                changed_columns = true;
            }
        });
        let columns = all
            .iter()
            .find(|(n, _, _)| *n == s.table)
            .map(|(_, c, _)| c.clone())
            .unwrap_or_default();
        if !s.table.is_empty() && preview_grid(ui, side, s, &columns, l) {
            changed_columns = true;
        }
    }
    if changed_columns {
        e.adopt = true;
    }
    ui.add_space(8.0);
    let live = |side: usize| {
        e.sides
            .get(side)
            .and_then(|s| all.iter().find(|(n, _, _)| *n == s.table))
            .is_some_and(|(_, _, live)| *live)
    };
    let rely_ok = live(0) && live(1) && e.card != Card::ManyToMany;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(l.t("Cardinality", "Cardinalité")).font(theme::bold(12.0)));
            let labels: Vec<String> = Card::ALL.iter().map(|c| c.label(l).to_string()).collect();
            if let Some(i) = combo(
                ui,
                "bicraft-rel-card",
                l.t("Cardinality", "Cardinalité"),
                e.card.label(l),
                &labels,
                true,
            ) && let Some(c) = Card::ALL.get(i)
            {
                e.card = *c;
                if *c == Card::OneToOne {
                    e.both = true;
                }
            }
        });
        ui.add_space(16.0);
        ui.vertical(|ui| {
            ui.label(
                RichText::new(l.t("Cross-filter direction", "Direction du filtrage croisé"))
                    .font(theme::bold(12.0)),
            );
            let labels = vec![
                l.t("Single", "Unique").to_string(),
                l.t("Both", "Les deux").to_string(),
            ];
            let cur = if e.both || e.card == Card::OneToOne { 1 } else { 0 };
            let shown = labels.get(cur).cloned().unwrap_or_default();
            if let Some(i) = combo(
                ui,
                "bicraft-rel-cross",
                l.t("Cross-filter direction", "Direction du filtrage croisé"),
                &shown,
                &labels,
                e.card != Card::OneToOne,
            ) {
                e.both = i == 1;
            }
        });
    });
    ui.add_space(6.0);
    if ui
        .checkbox(
            &mut e.active,
            l.t("Make this relationship active", "Rendre cette relation active"),
        )
        .changed()
    {
        e.active_auto = false;
    }
    ui.add_enabled_ui(rely_ok, |ui| {
        let r = ui.checkbox(
            &mut e.rely,
            l.t(
                "Assume referential integrity",
                "Supposer l'intégrité référentielle",
            ),
        );
        if !rely_ok {
            r.on_disabled_hover_text(l.t(
                "For DirectQuery or Dual tables, not many-to-many",
                "Pour des tables DirectQuery ou Double, hors plusieurs à plusieurs",
            ));
        }
    });
    if !rely_ok {
        e.rely = false;
    }
    let both = e.both || e.card == Card::OneToOne;
    ui.add_enabled_ui(both, |ui| {
        let r = ui.checkbox(
            &mut e.security_both,
            l.t(
                "Apply security filter in both directions",
                "Appliquer le filtre de sécurité dans les deux sens",
            ),
        );
        if !both {
            r.on_disabled_hover_text(l.t(
                "Needs cross filtering in both directions",
                "Nécessite un filtrage croisé dans les deux sens",
            ));
        }
    });
    if !both {
        e.security_both = false;
    }
    // the review
    ui.add_space(6.0);
    let ready = e.ends().is_some();
    match (&e.verdict, ready) {
        (_, false) => {
            ui.label(
                RichText::new(l.t(
                    "Select a column in each table.",
                    "Sélectionnez une colonne dans chaque table.",
                ))
                .color(t.text_dim),
            );
        }
        (None, true) => {
            ui.label(
                RichText::new(l.t("Checking the relationship…", "Vérification de la relation…"))
                    .color(t.text_dim),
            );
        }
        (Some(v), true) => {
            for m in &v.errors {
                ui.label(RichText::new(format!("✕ {} {m}", l.t("Error:", "Erreur :"))).color(t.err_text));
            }
            for m in &v.warnings {
                ui.label(
                    RichText::new(format!("⚠ {} {m}", l.t("Warning:", "Avertissement :"))).color(t.warn),
                );
            }
        }
    }
    if let Some(err) = &e.error {
        ui.label(RichText::new(format!("✕ {err}")).color(t.err_text));
    }
    ui.add_space(8.0);
    let mut go = None;
    ui.horizontal(|ui| {
        let valid = e.verdict.as_ref().is_some_and(|v| v.errors.is_empty());
        let call = e.call(false).filter(|_| valid);
        let why = if ready {
            l.t("Fix the errors first", "Corrigez d'abord les erreurs")
        } else {
            l.t(
                "Select a column in each table",
                "Sélectionnez une colonne dans chaque table",
            )
        };
        if widgets::primary(ui, l.t("OK", "OK"), call.is_some(), why).clicked() && call.is_some() {
            go = call;
        }
        if ui.button(l.t("Cancel", "Annuler")).clicked() {
            *close = true;
        }
    });
    go
}

/// A table's preview: the header's buttons pick the column. Returns whether the column changed.
fn preview_grid(ui: &mut Ui, side: usize, s: &mut Side, columns: &[String], l: Lang) -> bool {
    let t = Tokens::get(ui.ctx());
    let mut changed = false;
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.set_min_width(700.0);
            egui::ScrollArea::horizontal()
                .id_salt(("bicraft-rel-preview", side))
                .show(ui, |ui| {
                    egui::Grid::new(("bicraft-rel-preview-grid", side))
                        .striped(true)
                        .spacing(egui::vec2(12.0, 3.0))
                        .show(ui, |ui| {
                            for c in columns {
                                let on = s.column.as_deref() == Some(c.as_str());
                                let label = format!("{}[{c}]", s.table);
                                let r = ui.selectable_label(on, RichText::new(c).font(theme::bold(12.0)));
                                r.widget_info(|| {
                                    egui::WidgetInfo::selected(egui::Role::Button, true, on, &label)
                                });
                                if r.clicked() && !on {
                                    s.column = Some(c.clone());
                                    changed = true;
                                }
                            }
                            ui.end_row();
                            if let Some(p) = &s.preview {
                                for row in &p.rows {
                                    for c in columns {
                                        let v = p
                                            .columns
                                            .iter()
                                            .position(|x| x == c)
                                            .and_then(|i| row.get(i))
                                            .cloned()
                                            .unwrap_or_default();
                                        let on = s.column.as_deref() == Some(c.as_str());
                                        let text =
                                            RichText::new(v).color(if on { t.text } else { t.text_dim });
                                        ui.label(text);
                                    }
                                    ui.end_row();
                                }
                            }
                        });
                });
            match &s.preview {
                None => {
                    ui.label(
                        RichText::new(l.t("Reading the first rows…", "Lecture des premières lignes…"))
                            .color(t.text_dim),
                    );
                }
                Some(p) => {
                    if let Some(n) = &p.note {
                        ui.label(RichText::new(n).color(t.text_dim));
                    }
                }
            }
        });
    changed
}
