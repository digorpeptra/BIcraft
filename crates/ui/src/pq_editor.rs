//! Home › Transform data (ROADMAP M11 lot 12; Power BI Desktop's Power Query editor): a window
//! with the model's queries (`query.list`), the selected query's applied steps
//! (`query.steps`), a preview of the query at the selected step (`query.preview {step}`), the
//! formula bar with that step's M, and a ribbon of transforms that each add one step
//! (`query.apply_step`). Steps are renamed and changed (`query.edit_step`) or removed
//! (`query.remove_step`). The edits stay in the window's drafts, as Desktop's stay in its editor,
//! until Close & Apply writes the changed queries back and reloads their tables (`query.apply`,
//! one undo step). The Navigator's Transform data opens the window on the items picked as new
//! queries.
//!
//! The Advanced editor shows the selected query's whole M; Done checks it (`query.steps`) and
//! takes it as the draft only when it parses. Each preview column header has a type menu
//! (`change_type`) and a filter menu (sorts, text or number filters, and a list of the values
//! with check boxes, `filter_values`), read from the query at the step shown (`query.preview`,
//! the first 1000 rows, as the menu's list may be incomplete). The queries list makes new
//! blank queries, duplicates and references (`query.new`), renames a query with the drafts that
//! read it (`query.rename`) and deletes one after a confirmation, all in the drafts: Close &
//! Apply renames and deletes in the model in the same undo step (`query.apply {renamed,
//! removed}`). Closing with changes asks first.
//!
//! The View tab's data profiling (`query.profile`): Column quality and Column distribution under
//! each column header, Column profile (the first picked column's statistics and value
//! distribution) in a bottom pane, over the top 1000 rows or the entire data set.
//!
//! Every button runs an engine command by id; only the window's state (drafts, the step and
//! columns picked, the open form) is the UI's.

use std::collections::BTreeSet;

use egui::{RichText, Ui};
use serde_json::{Value, json};

use crate::App;
use crate::actions::Call;
use crate::app::{Then, clip};
use crate::get_data::Preview;
use crate::i18n::Lang;
use crate::icons::{self, Icon};
use crate::m_box::{self, MAssist};
use crate::theme::{self, Tokens};
use crate::widgets;

pub const LIST: &str = "query.list";
pub const STEPS: &str = "query.steps";
pub const APPLY_STEP: &str = "query.apply_step";
pub const EDIT_STEP: &str = "query.edit_step";
pub const REMOVE_STEP: &str = "query.remove_step";
pub const APPLY: &str = "query.apply";
pub const PREVIEW: &str = "query.preview";
pub const NEW: &str = "query.new";
pub const RENAME: &str = "query.rename";
pub const FOLDING: &str = "query.folding";
pub const NATIVE: &str = "query.native";
pub const DIAGNOSE: &str = "query.diagnose";
pub const PROFILE: &str = "query.profile";
/// Home › Export results (Desktop's Output data).
pub const EXPORT: &str = "query.export";
/// The id of Export results' answer ([`Then::Output`]).
pub const EXPORT_OUTPUT: &str = "pq_export";
/// Format: the Advanced editor's text or the step's formula laid out (`m.format`).
pub const FORMAT: &str = "m.format";
/// The commands the window runs (the coverage test reads it).
pub const COMMANDS: &[&str] = &[
    LIST,
    STEPS,
    APPLY_STEP,
    EDIT_STEP,
    REMOVE_STEP,
    APPLY,
    PREVIEW,
    NEW,
    RENAME,
    FOLDING,
    NATIVE,
    DIAGNOSE,
    PROFILE,
    EXPORT,
    FORMAT,
    crate::pq_deps::DEPENDENCIES,
];
/// Rows a preview shows.
pub const PREVIEW_ROWS: usize = 100;
/// Rows a filter menu's value list reads.
pub const VALUE_ROWS: usize = 1000;
/// Longest text drawn in a preview cell (characters).
const MAX_CELL: usize = 60;
/// Most queries listed.
const MAX_QUERIES: usize = 2000;

/// One step of the selected query.
#[derive(Debug, Clone, PartialEq)]
pub struct StepRow {
    pub name: String,
    pub expression: String,
    pub function: Option<String>,
    /// The `{kind, …}` that wrote the step, when its M reads back (`query.steps`): the step
    /// settings gear opens its form.
    pub settings: Option<Value>,
}

/// One query of the window.
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub name: String,
    /// `table`, `query` (a shared expression) or `new` (from the Navigator, not loaded yet).
    pub source: String,
    /// The model's text (none for a new query).
    pub original: Option<String>,
    /// The window's text.
    pub draft: String,
    pub steps: Option<Vec<StepRow>>,
    pub parameter: bool,
    /// Its name in the model (none for a new query): Close & Apply renames it when it differs.
    pub model_name: Option<String>,
    /// The connector families it calls (`query.steps`), passed as `allow_connectors`.
    pub connectors: Vec<String>,
    /// Its steps' folding indicators (`query.folding`; none while read).
    pub folding: Option<Result<Vec<Fold>, String>>,
}

/// One step's folding indicator.
#[derive(Debug, Clone, PartialEq)]
pub struct Fold {
    pub name: String,
    /// `folded`, `partial`, `not_folded` or `unknown`.
    pub folding: String,
    /// View native query shows a text.
    pub native: bool,
    pub reason: String,
}

/// Diagnose step: BIcraft's own time for the query at a step and the requests it sent.
#[derive(Debug, Clone, PartialEq)]
pub struct DiagView {
    pub step: String,
    /// The diagnosis, or why it failed; none while it runs.
    pub result: Option<Result<Diagnosis, String>>,
}

/// One step's diagnosis (`query.diagnose`).
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnosis {
    pub elapsed_ms: u64,
    pub rows: Option<u64>,
    pub error: Option<String>,
    /// (kind, connector, rows, ms, text) of each source request.
    pub requests: Vec<(String, String, u64, u64, String)>,
}

/// View native query: a step's native query, read-only.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeView {
    pub step: String,
    /// (the text, `sql` or `url`), or why there is none; none while read.
    pub text: Option<Result<(String, String), String>>,
}

impl Query {
    /// Whether Close & Apply writes it.
    pub fn changed(&self) -> bool {
        self.original.as_deref() != Some(self.draft.as_str())
    }

    /// Whether Close & Apply renames it.
    pub fn renamed(&self) -> bool {
        self.model_name.as_ref().is_some_and(|m| m != &self.name)
    }
}

/// A column's filter menu: its values with their check boxes.
#[derive(Debug, Clone, PartialEq)]
pub struct FilterMenu {
    pub column: String,
    /// The column's data type (the preview's).
    pub ty: String,
    /// The distinct values of the first [`VALUE_ROWS`] rows, ticked or not (none while read).
    pub values: Option<Result<Vec<(Value, bool)>, String>>,
    /// More rows than were read: the list may be incomplete.
    pub incomplete: bool,
    pub search: String,
    /// A date/time column's M type (`date`, `datetime`), as `query.preview` gives it: its values
    /// are written as `#date` / `#datetime` literals.
    pub date_kind: Option<String>,
}

/// The editor ribbon's tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorTab {
    Home,
    Transform,
    AddColumn,
    View,
}

/// One column's quality and distribution (`query.profile`).
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnProfile {
    pub name: String,
    /// Shares of valid, error and empty values (0 to 1).
    pub valid: f64,
    pub error: f64,
    pub empty: f64,
    pub distinct: u64,
    pub unique: u64,
    /// The most frequent values' counts, most frequent first (the small histogram).
    pub bars: Vec<u64>,
}

/// The data profile of the step shown.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub columns: Vec<ColumnProfile>,
    /// Rows profiled.
    pub rows: u64,
    /// The column profile (statistics and value distribution) of the column asked, as
    /// `query.profile` answered it.
    pub column: Option<Value>,
}

impl Profile {
    fn of(v: &Value) -> Self {
        let n = |x: &Value, k: &str| x.get(k).and_then(Value::as_u64).unwrap_or(0);
        let f = |x: &Value, k: &str| x.get(k).and_then(Value::as_f64).unwrap_or(0.0);
        let columns = v
            .get("columns")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|c| ColumnProfile {
                name: s(c, "name"),
                valid: f(c, "valid_share"),
                error: f(c, "error_share"),
                empty: f(c, "empty_share"),
                distinct: n(c, "distinct"),
                unique: n(c, "unique"),
                bars: c
                    .get("distribution")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|d| n(d, "count"))
                    .collect(),
            })
            .collect();
        Self {
            columns,
            rows: n(v, "rows"),
            column: v.get("column").filter(|c| !c.is_null()).cloned(),
        }
    }
}

/// What the window asks before it acts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confirm {
    /// Delete query `n` (its index).
    Delete(usize),
    /// Close with changes not applied.
    Close,
}

/// A transform's values, asked in the window before its step is added.
#[derive(Debug, Clone, PartialEq)]
pub enum Form {
    Filter {
        column: String,
        op: usize,
        value: String,
    },
    Rename {
        column: String,
        new_name: String,
    },
    ChangeType {
        ty: usize,
    },
    Replace {
        find: String,
        replace: String,
    },
    Group {
        name: String,
        op: usize,
        column: String,
    },
    Merge {
        other: String,
        key: String,
        other_key: String,
        join: usize,
    },
    Append {
        other: String,
    },
    Expand {
        column: String,
        columns: String,
    },
    Split {
        delimiter: String,
        parts: String,
    },
    Pivot {
        values: String,
        agg: usize,
    },
    Custom {
        name: String,
        formula: String,
    },
    /// A step `{kind, …}` whose remaining values the fields ask (`crate::pq_tabs`).
    Params {
        title: String,
        step: Value,
        fields: Vec<Field>,
    },
    /// Add column › Conditional column: if … then … else if … else.
    Conditional {
        name: String,
        rules: Vec<CondRule>,
        otherwise: String,
    },
}

/// One value a [`Form::Params`] asks, written as the step's `key`.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub key: &'static str,
    pub en: &'static str,
    pub fr: &'static str,
    pub kind: FieldKind,
    pub value: String,
}

/// How a [`Field`]'s text is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    /// A whole number, 0 or more.
    Count,
    /// A whole number.
    Int,
    Number,
    /// A value of the first picked column's type ([`typed_value`]).
    Value,
}

/// One rule of a conditional column (`op` indexes [`OPS`]).
#[derive(Debug, Clone, PartialEq)]
pub struct CondRule {
    pub column: String,
    pub op: usize,
    pub value: String,
    pub output: String,
}

/// The open window's state (pure view state).
#[derive(Debug, Clone, PartialEq)]
pub struct Editor {
    pub queries: Vec<Query>,
    pub selected: Option<usize>,
    /// The step shown (its name).
    pub step: Option<String>,
    /// The formula bar's text (the step's M).
    pub formula: String,
    pub preview: Option<Result<Preview, String>>,
    /// Columns picked in the preview (header clicks), in click order.
    pub columns: Vec<String>,
    pub form: Option<Form>,
    /// The step whose settings the open form changes (the gear), else the form adds a step.
    pub editing: Option<String>,
    /// The step being renamed and its new name.
    pub rename: Option<String>,
    /// The folder file functions may read (the Navigator's), else the session's.
    pub data_root: Option<String>,
    pub busy: bool,
    pub problem: Option<String>,
    /// Bumped by every run: older answers are dropped.
    pub generation: u64,
    /// New queries to add once the model's are listed (the Navigator's).
    pub pending: Vec<(String, String)>,
    /// The Advanced editor's text, while open.
    pub advanced: Option<String>,
    /// The open column filter menu.
    pub filter: Option<FilterMenu>,
    /// The model's queries deleted in the window (their model names).
    pub removed: Vec<String>,
    /// The selected query's new name, while it is renamed.
    pub renaming: Option<String>,
    pub confirm: Option<Confirm>,
    /// View native query, while open.
    pub native: Option<NativeView>,
    /// Diagnose step, while open.
    pub diagnosis: Option<DiagView>,
    /// The ribbon tab shown.
    pub tab: EditorTab,
    /// View › Column quality, Column distribution and Column profile.
    pub quality: bool,
    pub distribution: bool,
    pub profile: bool,
    /// Profiling over the entire data set (else the top 1000 rows).
    pub full: bool,
    /// The step shown's profile, while a profiling view is on (none while read).
    pub profiled: Option<Result<Profile, String>>,
    /// View › Query dependencies, while open.
    pub deps: Option<crate::pq_deps::Deps>,
    /// The Advanced editor's and the formula bar's completion lists (M11.35).
    pub m_advanced: MAssist,
    pub m_formula: MAssist,
}

impl Editor {
    /// Whether a profiling view is on.
    pub fn profiling(&self) -> bool {
        self.quality || self.distribution || self.profile
    }

    /// Whether Close & Apply would change the model.
    pub fn has_changes(&self) -> bool {
        !self.removed.is_empty() || self.queries.iter().any(|q| q.changed() || q.renamed())
    }
}

/// What an answer of the window's commands is for ([`Then::Pq`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    List(u64),
    Steps(u64, usize),
    Preview(u64, usize, String),
    Edit(u64, usize),
    Apply(u64),
    /// The Advanced editor's text checked for query `n`.
    Advanced(u64, usize),
    /// A filter menu's values for (query, column).
    Values(u64, usize, String),
    New(u64),
    /// Query `n` renamed to the name.
    Rename(u64, usize, String),
    /// Query `n`'s folding indicators.
    Folding(u64, usize),
    /// View native query of (query, step).
    Native(u64, usize, String),
    /// Diagnose step of (query, step).
    Diagnose(u64, usize, String),
    /// The query dependencies graph.
    Deps(u64),
    /// The profile of (query, step).
    Profile(u64, usize, String),
    /// The formatted text for the Advanced editor (`true`) or the formula bar.
    Format(u64, bool),
}

/// The types of Change type: (command value, English, French).
pub const TYPES: [(&str, &str, &str); 12] = [
    ("text", "Text", "Texte"),
    ("whole", "Whole number", "Nombre entier"),
    ("decimal", "Decimal number", "Nombre décimal"),
    ("currency", "Fixed decimal number", "Nombre décimal fixe"),
    ("percentage", "Percentage", "Pourcentage"),
    ("date", "Date", "Date"),
    ("datetime", "Date/Time", "Date/Heure"),
    ("datetimezone", "Date/Time/Zone", "Date/Heure/Fuseau"),
    ("time", "Time", "Heure"),
    ("duration", "Duration", "Durée"),
    ("logical", "True/False", "Vrai/Faux"),
    ("any", "Any", "Quelconque"),
];

/// The comparisons of Filter rows: (command value, English, French).
pub const OPS: [(&str, &str, &str); 12] = [
    ("equals", "equals", "est égal à"),
    ("not_equals", "does not equal", "est différent de"),
    ("greater", "is greater than", "est supérieur à"),
    ("greater_or_equal", "is at least", "est au moins"),
    ("less", "is less than", "est inférieur à"),
    ("less_or_equal", "is at most", "est au plus"),
    ("contains", "contains", "contient"),
    ("not_contains", "does not contain", "ne contient pas"),
    ("begins_with", "begins with", "commence par"),
    ("ends_with", "ends with", "se termine par"),
    ("is_null", "is empty (null)", "est vide (null)"),
    ("is_not_null", "is not empty", "n'est pas vide"),
];

/// The aggregations of Group by and Pivot: (command value, English, French).
pub const AGGS: [(&str, &str, &str); 8] = [
    ("count", "Count rows", "Compter les lignes"),
    (
        "count_distinct",
        "Count distinct rows",
        "Compter les lignes distinctes",
    ),
    ("sum", "Sum", "Somme"),
    ("average", "Average", "Moyenne"),
    ("median", "Median", "Médiane"),
    ("min", "Min", "Min"),
    ("max", "Max", "Max"),
    ("all_rows", "All rows", "Toutes les lignes"),
];

/// The joins of Merge queries: (command value, English, French).
pub const JOINS: [(&str, &str, &str); 6] = [
    (
        "left_outer",
        "Left outer (all of this, matches of the other)",
        "Externe gauche (toutes ces lignes, celles de l'autre qui correspondent)",
    ),
    ("right_outer", "Right outer", "Externe droite"),
    ("full_outer", "Full outer", "Externe entière"),
    (
        "inner",
        "Inner (matching rows only)",
        "Interne (lignes correspondantes seulement)",
    ),
    (
        "left_anti",
        "Left anti (rows of this only)",
        "Anti gauche (lignes de celle-ci seulement)",
    ),
    ("right_anti", "Right anti", "Anti droite"),
];

fn pick(l: Lang, (_, en, fr): (&'static str, &'static str, &'static str)) -> &'static str {
    l.t(en, fr)
}

/// Opens the window on the model's queries, with `new` queries (name, M) added (the Navigator's).
pub fn open(app: &mut App, new: Vec<(String, String)>, data_root: Option<String>) {
    // a former window's export answer is not shown again
    if exported(app).is_some() {
        app.output = None;
    }
    app.cmd.pq = Some(Editor {
        queries: Vec::new(),
        selected: None,
        step: None,
        formula: String::new(),
        preview: None,
        columns: Vec::new(),
        form: None,
        editing: None,
        rename: None,
        data_root,
        busy: true,
        problem: None,
        generation: 1,
        pending: new,
        advanced: None,
        filter: None,
        removed: Vec::new(),
        renaming: None,
        confirm: None,
        native: None,
        diagnosis: None,
        tab: EditorTab::Home,
        quality: false,
        distribution: false,
        profile: false,
        full: false,
        profiled: None,
        deps: None,
        m_advanced: MAssist::default(),
        m_formula: MAssist::default(),
    });
    app.run(
        Call {
            id: LIST,
            params: json!({}),
        },
        Then::Pq(Step::List(1)),
    );
}

/// Transform data and the window's commands open the window when run without their values (a
/// ribbon or palette click); programmatic calls never do.
pub fn intercept(app: &mut App, id: &str, params: &Value) -> bool {
    let open_it = match id {
        LIST => true,
        STEPS => params.get("name").is_none() && params.get("expression").is_none(),
        APPLY_STEP => params.get("step").is_none(),
        EDIT_STEP | REMOVE_STEP => params.get("step").is_none(),
        APPLY => ["queries", "renamed", "removed"]
            .iter()
            .all(|k| params.get(*k).is_none()),
        NEW => params.get("names").is_none() && params.get("of").is_none(),
        RENAME => params.get("queries").is_none(),
        FOLDING | NATIVE | DIAGNOSE | PROFILE => {
            params.get("name").is_none() && params.get("expression").is_none()
        }
        crate::pq_deps::DEPENDENCIES => params.get("queries").is_none(),
        _ => false,
    };
    if open_it {
        open(app, Vec::new(), None);
        // Query dependencies: the editor's View tab with the diagram open
        if id == crate::pq_deps::DEPENDENCIES {
            if let Some(e) = app.cmd.pq.as_mut() {
                e.tab = EditorTab::View;
            }
            // read once the queries are listed
            let listed = app.cmd.pq.as_ref().is_some_and(|e| !e.queries.is_empty());
            if listed {
                open_dependencies(app);
            } else if let Some(e) = app.cmd.pq.as_mut() {
                e.deps = Some(crate::pq_deps::Deps::new());
            }
        }
        // Column profile: the editor with its View tab's profiling on
        if id == PROFILE
            && let Some(e) = app.cmd.pq.as_mut()
        {
            e.tab = EditorTab::View;
            e.quality = true;
            e.distribution = true;
            e.profile = true;
        }
    }
    open_it
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn step_rows(v: &Value) -> Vec<StepRow> {
    v.get("steps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|x| StepRow {
            name: s(x, "name"),
            expression: s(x, "expression"),
            function: x.get("function").and_then(Value::as_str).map(str::to_string),
            settings: x.get("settings").filter(|v| v.is_object()).cloned(),
        })
        .collect()
}

/// The answer of one of the window's commands.
pub fn answer(app: &mut App, step: Step, r: Result<Value, String>) {
    let Some(e) = app.cmd.pq.as_mut() else { return };
    match step {
        Step::List(g) if g == e.generation => {
            e.busy = false;
            match r {
                Ok(v) => {
                    let mut qs: Vec<Query> = v
                        .get("queries")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .take(MAX_QUERIES)
                        .map(|q| Query {
                            name: s(q, "name"),
                            source: s(q, "source"),
                            original: Some(s(q, "expression")),
                            draft: s(q, "expression"),
                            steps: None,
                            parameter: q.get("parameter").and_then(Value::as_bool).unwrap_or(false),
                            model_name: Some(s(q, "name")),
                            connectors: Vec::new(),
                            folding: None,
                        })
                        .collect();
                    for (name, m) in std::mem::take(&mut e.pending) {
                        qs.push(new_query(name, m));
                    }
                    let first = qs
                        .iter()
                        .position(|q| q.source == "new")
                        .or_else(|| qs.iter().position(|q| !q.parameter))
                        .or(if qs.is_empty() { None } else { Some(0) });
                    e.queries = qs;
                    // the diagram asked before the queries were listed
                    let deps = e.deps.is_some();
                    if let Some(i) = first {
                        select_query(app, i);
                    }
                    if deps {
                        open_dependencies(app);
                    }
                }
                Err(err) => e.problem = Some(clip(&err, 600)),
            }
        }
        Step::Steps(g, qi) if g == e.generation => {
            e.busy = false;
            match r {
                Ok(v) => took_steps(app, qi, &v, None),
                Err(err) => {
                    e.problem = Some(clip(&err, 600));
                    e.preview = Some(Err(clip(&err, 600)));
                }
            }
        }
        Step::Edit(g, qi) if g == e.generation => {
            e.busy = false;
            match r {
                Ok(v) => {
                    if let Some(q) = e.queries.get_mut(qi) {
                        q.draft = s(&v, "expression");
                    }
                    e.form = None;
                    e.editing = None;
                    e.filter = None;
                    e.rename = None;
                    e.problem = None;
                    let step = v
                        .get("step")
                        .and_then(Value::as_str)
                        .or_else(|| v.get("result_step").and_then(Value::as_str))
                        .map(str::to_string);
                    took_steps(app, qi, &v, step);
                }
                Err(err) => e.problem = Some(clip(&err, 600)),
            }
        }
        Step::Preview(g, qi, name) if g == e.generation => {
            if e.selected == Some(qi) && e.step.as_deref() == Some(name.as_str()) {
                e.preview = Some(r.map(|v| crate::get_data::preview_of(&v, PREVIEW_ROWS)));
            }
        }
        Step::Apply(g) if g == e.generation => {
            e.busy = false;
            match r {
                Ok(_) => app.cmd.pq = None,
                Err(err) => e.problem = Some(clip(&err, 800)),
            }
        }
        Step::Advanced(g, qi) if g == e.generation => {
            e.busy = false;
            match r {
                Ok(v) => {
                    if let Some(q) = e.queries.get_mut(qi) {
                        q.draft = s(&v, "expression");
                    }
                    e.advanced = None;
                    e.problem = None;
                    e.filter = None;
                    e.columns.clear();
                    took_steps(app, qi, &v, None);
                }
                // the text stays in the Advanced editor, the draft as it was
                Err(err) => e.problem = Some(clip(&err, 800)),
            }
        }
        Step::Values(g, qi, column) if g == e.generation => {
            let Some(f) = e.filter.as_mut() else { return };
            if e.selected != Some(qi) || f.column != column {
                return;
            }
            match r {
                Ok(v) => {
                    let p = crate::get_data::preview_of(&v, VALUE_ROWS);
                    f.incomplete = p.truncated;
                    f.date_kind = date_kind_of(&v, &column);
                    let literal =
                        f.ty != "dateTime" || matches!(f.date_kind.as_deref(), Some("date" | "datetime"));
                    f.values = Some(Ok(if literal {
                        distinct(&p, &column).into_iter().map(|x| (x, true)).collect()
                    } else {
                        // a time or zoned value has no literal here: no value list
                        Vec::new()
                    }));
                }
                Err(err) => f.values = Some(Err(clip(&err, 600))),
            }
        }
        Step::New(g) if g == e.generation => {
            e.busy = false;
            match r {
                Ok(v) => {
                    e.queries.push(new_query(s(&v, "name"), s(&v, "expression")));
                    let i = e.queries.len().saturating_sub(1);
                    select_query(app, i);
                }
                Err(err) => e.problem = Some(clip(&err, 600)),
            }
        }
        Step::Rename(g, qi, new_name) if g == e.generation => {
            e.busy = false;
            match r {
                Ok(v) => {
                    if let Some(q) = e.queries.get_mut(qi) {
                        q.name.clone_from(&new_name);
                    }
                    let mut reread = false;
                    for x in v.get("queries").and_then(Value::as_array).into_iter().flatten() {
                        let name = s(x, "name");
                        if let Some((i, q)) = e.queries.iter_mut().enumerate().find(|(_, q)| q.name == name) {
                            q.draft = s(x, "expression");
                            q.steps = None;
                            reread |= e.selected == Some(i);
                        }
                    }
                    e.renaming = None;
                    e.problem = None;
                    if reread && let Some(i) = e.selected {
                        select_query(app, i);
                    }
                }
                Err(err) => e.problem = Some(clip(&err, 600)),
            }
        }
        Step::Folding(g, qi) if g == e.generation => {
            if let Some(q) = e.queries.get_mut(qi) {
                q.folding = Some(r.map(|v| folds(&v)).map_err(|err| clip(&err, 600)));
            }
        }
        Step::Diagnose(g, qi, step) if g == e.generation => {
            if e.selected != Some(qi) {
                return;
            }
            let Some(d) = e.diagnosis.as_mut().filter(|d| d.step == step) else {
                return;
            };
            d.result = Some(r.map(|v| diagnosis_of(&v)).map_err(|err| clip(&err, 600)));
        }
        Step::Native(g, qi, step) if g == e.generation => {
            if e.selected != Some(qi) {
                return;
            }
            let Some(n) = e.native.as_mut().filter(|n| n.step == step) else {
                return;
            };
            n.text = Some(match r {
                Ok(v) => match v.get("native").and_then(Value::as_str) {
                    Some(text) => Ok((text.to_string(), s(&v, "kind"))),
                    None => Err(s(&v, "reason")),
                },
                Err(err) => Err(clip(&err, 600)),
            });
        }
        Step::Deps(g) if g == e.generation => {
            if let Some(d) = e.deps.as_mut() {
                d.graph = Some(
                    r.map(|v| crate::pq_deps::Graph::of(&v))
                        .map_err(|err| clip(&err, 600)),
                );
            }
        }
        Step::Profile(g, qi, name)
            if g == e.generation
                && e.selected == Some(qi)
                && e.step.as_deref() == Some(name.as_str())
                && e.profiling() =>
        {
            e.profiled = Some(r.map(|v| Profile::of(&v)).map_err(|err| clip(&err, 600)));
        }
        Step::Format(g, advanced) if g == e.generation => {
            e.busy = false;
            match r {
                Ok(v) => {
                    let text = s(&v, "expression");
                    if advanced {
                        if let Some(a) = e.advanced.as_mut() {
                            *a = text;
                        }
                    } else {
                        e.formula = text;
                    }
                    e.problem = None;
                }
                // the text stays as it was
                Err(err) => e.problem = Some(clip(&err, 800)),
            }
        }
        _ => {}
    }
}

/// Format (Advanced editor, formula bar): the text laid out by `m.format`, nothing applied.
pub fn format_text(app: &mut App, advanced: bool) {
    let Some(e) = app.cmd.pq.as_mut() else { return };
    let text = if advanced {
        e.advanced.clone()
    } else {
        e.step.as_ref().map(|_| e.formula.clone())
    };
    let Some(text) = text.filter(|t| !t.trim().is_empty()) else {
        return;
    };
    e.generation = e.generation.wrapping_add(1);
    e.busy = true;
    e.problem = None;
    let g = e.generation;
    app.run(
        Call {
            id: FORMAT,
            params: json!({"expression": text}),
        },
        Then::Pq(Step::Format(g, advanced)),
    );
}

/// View › Query dependencies: opens the diagram and reads the graph of the window's queries
/// (`query.dependencies`, the drafts as they are).
pub fn open_dependencies(app: &mut App) {
    let Some(e) = app.cmd.pq.as_mut() else { return };
    let queries: Vec<Value> = e
        .queries
        .iter()
        .map(|q| json!({"name": q.name, "expression": q.draft, "load": q.source != "query"}))
        .collect();
    e.deps = Some(crate::pq_deps::Deps::new());
    e.form = None;
    e.filter = None;
    let g = e.generation;
    app.run(
        Call {
            id: crate::pq_deps::DEPENDENCIES,
            params: json!({"queries": queries}),
        },
        Then::Pq(Step::Deps(g)),
    );
}

/// Reads the profile of the step shown (`query.profile`) while a profiling view is on: over the
/// top 1000 rows or the entire data set, with the first picked column's profile when Column
/// profile is on.
pub fn refresh_profile(app: &mut App) {
    let Some(e) = app.cmd.pq.as_mut() else { return };
    e.profiled = None;
    if !e.profiling() {
        return;
    }
    let (Some(qi), Some(step)) = (e.selected, e.step.clone()) else {
        return;
    };
    let Some(q) = e.queries.get(qi) else { return };
    let mut p = json!({"expression": q.draft, "step": step, "full": e.full});
    // a column the step shown has (the preview's, once read)
    let has = |c: &String| match &e.preview {
        Some(Ok(p)) => p.columns.iter().any(|(n, _)| n == c),
        _ => true,
    };
    if e.profile
        && let Some(c) = e.columns.first().filter(|c| has(c))
    {
        p["column"] = json!(c);
    }
    if let Some(r) = &e.data_root {
        p["data_root"] = json!(r);
    }
    let p = with_drafts(e, qi, p);
    let g = e.generation;
    app.run(
        Call {
            id: PROFILE,
            params: p,
        },
        Then::Pq(Step::Profile(g, qi, step)),
    );
}

/// A query new in the window (not in the model yet).
fn new_query(name: String, m: String) -> Query {
    Query {
        name,
        source: "new".into(),
        original: None,
        draft: m,
        steps: None,
        parameter: false,
        model_name: None,
        connectors: Vec::new(),
        folding: None,
    }
}

/// The parameters every evaluation of the selected query's draft takes: the folder file
/// functions may read and the connectors the query calls.
fn eval_params(e: &Editor, q: &Query, mut p: Value) -> Value {
    if let Some(r) = &e.data_root {
        p["data_root"] = json!(r);
    }
    if !q.connectors.is_empty() {
        p["allow_connectors"] = json!(q.connectors);
    }
    p
}

/// `p` with the drafts of the window's other queries that Close & Apply would write
/// (`queries`): a preview of query `qi` reads them as the window has them (a merge with a changed
/// query, a reference to a renamed or new one).
fn with_drafts(e: &Editor, qi: usize, mut p: Value) -> Value {
    let drafts: Vec<Value> = e
        .queries
        .iter()
        .enumerate()
        .filter(|(i, q)| *i != qi && (q.changed() || q.renamed()))
        .map(|(_, q)| json!({"name": q.name, "expression": q.draft}))
        .collect();
    if !drafts.is_empty() {
        p["queries"] = json!(drafts);
    }
    p
}

/// Reads query `qi`'s folding indicators.
fn read_folding(app: &mut App, qi: usize) {
    let Some(e) = app.cmd.pq.as_mut() else { return };
    let Some(q) = e.queries.get_mut(qi) else { return };
    q.folding = None;
    let Some(q) = e.queries.get(qi) else { return };
    let p = eval_params(e, q, json!({"expression": q.draft}));
    let g = e.generation;
    app.run(
        Call {
            id: FOLDING,
            params: p,
        },
        Then::Pq(Step::Folding(g, qi)),
    );
}

/// The folding indicators of a `query.folding` answer.
fn folds(v: &Value) -> Vec<Fold> {
    v.get("steps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|x| Fold {
            name: s(x, "name"),
            folding: s(x, "folding"),
            native: x
                .get("native_available")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            reason: s(x, "reason"),
        })
        .collect()
}

/// The first step's diagnosis of a `query.diagnose` answer.
fn diagnosis_of(v: &Value) -> Diagnosis {
    let st = v.pointer("/steps/0").cloned().unwrap_or(Value::Null);
    let n = |x: &Value, k: &str| x.get(k).and_then(Value::as_u64);
    Diagnosis {
        elapsed_ms: n(&st, "elapsed_ms").unwrap_or(0),
        rows: n(&st, "rows"),
        error: st.get("error").and_then(Value::as_str).map(str::to_string),
        requests: st
            .get("requests")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|r| {
                (
                    s(r, "kind"),
                    s(r, "connector"),
                    n(r, "rows").unwrap_or(0),
                    n(r, "elapsed_ms").unwrap_or(0),
                    s(r, "text"),
                )
            })
            .collect(),
    }
}

/// Diagnose step `step` of the selected query.
pub fn diagnose(app: &mut App, step: &str) {
    let Some(e) = app.cmd.pq.as_mut() else { return };
    let Some(qi) = e.selected else { return };
    e.native = None;
    e.diagnosis = Some(DiagView {
        step: step.to_string(),
        result: None,
    });
    let Some(q) = e.queries.get(qi) else { return };
    let p = eval_params(e, q, json!({"expression": q.draft, "step": step}));
    let g = e.generation;
    app.run(
        Call {
            id: DIAGNOSE,
            params: p,
        },
        Then::Pq(Step::Diagnose(g, qi, step.to_string())),
    );
}

/// View native query of step `step` of the selected query.
pub fn view_native(app: &mut App, step: &str) {
    let Some(e) = app.cmd.pq.as_mut() else { return };
    let Some(qi) = e.selected else { return };
    e.diagnosis = None;
    e.native = Some(NativeView {
        step: step.to_string(),
        text: None,
    });
    let Some(q) = e.queries.get(qi) else { return };
    let p = eval_params(e, q, json!({"expression": q.draft, "step": step}));
    let g = e.generation;
    app.run(
        Call {
            id: NATIVE,
            params: p,
        },
        Then::Pq(Step::Native(g, qi, step.to_string())),
    );
}

/// The distinct values of column `name` in a preview, in their order (null first), at most
/// [`VALUE_ROWS`].
fn distinct(p: &Preview, name: &str) -> Vec<Value> {
    let Some(at) = p.columns.iter().position(|(n, _)| n == name) else {
        return Vec::new();
    };
    let mut out: Vec<Value> = Vec::new();
    for row in &p.rows {
        let v = row.get(at).cloned().unwrap_or(Value::Null);
        if !out.contains(&v) {
            out.push(v);
        }
        if out.len() >= VALUE_ROWS {
            break;
        }
    }
    out.sort_by(|a, b| match (a, b) {
        (Value::Null, Value::Null) => std::cmp::Ordering::Equal,
        (Value::Null, _) => std::cmp::Ordering::Less,
        (_, Value::Null) => std::cmp::Ordering::Greater,
        (Value::Number(x), Value::Number(y)) => x
            .as_f64()
            .partial_cmp(&y.as_f64())
            .unwrap_or(std::cmp::Ordering::Equal),
        (Value::String(x), Value::String(y)) => x.to_lowercase().cmp(&y.to_lowercase()),
        _ => a.to_string().cmp(&b.to_string()),
    });
    out
}

/// Whether a filter menu lists the values of a column of type `ty` (text, numbers, true/false:
/// the values M writes as literals).
pub fn lists_values(ty: &str) -> bool {
    matches!(ty, "string" | "int64" | "double" | "decimal" | "boolean")
}

/// The `filter_values` step of a filter menu, or none when every value is ticked (nothing to
/// filter) or none is. The shorter list is written: the values left out when the list is
/// complete and they are fewer, else the values kept.
pub fn filter_step(f: &FilterMenu) -> Option<Value> {
    let Some(Ok(values)) = &f.values else {
        return None;
    };
    let kept: Vec<&Value> = values.iter().filter(|x| x.1).map(|x| &x.0).collect();
    let out: Vec<&Value> = values.iter().filter(|x| !x.1).map(|x| &x.0).collect();
    if kept.is_empty() || out.is_empty() {
        return None;
    }
    let exclude = !f.incomplete && out.len() < kept.len();
    let list: Vec<Value> = if exclude { out } else { kept }
        .into_iter()
        .map(|v| date_value(v, f.date_kind.as_deref()))
        .collect();
    Some(json!({"kind": "filter_values", "column": f.column, "values": list, "exclude": exclude}))
}

/// The `date_kind` `query.preview` gives column `column`.
fn date_kind_of(v: &Value, column: &str) -> Option<String> {
    v.get("columns")?
        .as_array()?
        .iter()
        .find(|c| c.get("name").and_then(Value::as_str) == Some(column))?
        .get("date_kind")?
        .as_str()
        .map(str::to_string)
}

/// A preview value of a date/time column as `query.apply_step` takes it: `{"date": "Y-M-D"}`
/// or `{"datetime": "Y-M-DTh:m:s"}` (the preview writes `2024-01-31` or `2024-01-31T10:30:00`).
pub fn date_value(v: &Value, kind: Option<&str>) -> Value {
    let (Some(text), Some(kind)) = (v.as_str(), kind) else {
        return v.clone();
    };
    match kind {
        "date" => json!({"date": text.get(..10).unwrap_or(text)}),
        "datetime" if text.len() == 10 => json!({"datetime": format!("{text}T00:00:00")}),
        "datetime" => json!({"datetime": text}),
        _ => v.clone(),
    }
}

/// Opens column `column`'s filter menu and reads its values at the step shown.
pub fn open_filter(app: &mut App, column: &str) {
    let Some(e) = app.cmd.pq.as_mut() else { return };
    let (Some(qi), Some(step)) = (e.selected, e.step.clone()) else {
        return;
    };
    let Some(q) = e.queries.get(qi) else { return };
    let ty = column_type(e, column);
    let listed = lists_values(&ty) || ty == "dateTime";
    let draft = q.draft.clone();
    e.form = None;
    e.filter = Some(FilterMenu {
        column: column.to_string(),
        ty,
        values: if listed { None } else { Some(Ok(Vec::new())) },
        incomplete: false,
        search: String::new(),
        date_kind: None,
    });
    if !listed {
        return;
    }
    let g = e.generation;
    let Some(q) = e.queries.get(qi) else { return };
    let p = with_drafts(
        e,
        qi,
        eval_params(
            e,
            q,
            json!({"expression": draft, "step": step, "limit": VALUE_ROWS}),
        ),
    );
    app.run(
        Call {
            id: PREVIEW,
            params: p,
        },
        Then::Pq(Step::Values(g, qi, column.to_string())),
    );
}

/// Home › Export results: every row of the selected query, as the window has it, to a CSV file
/// (`query.export`). On the desktop the Save dialog picks the file; in the browser it is written
/// in the server workspace under the query's name, then downloaded.
fn export_results(app: &mut App) {
    let Some(e) = app.cmd.pq.as_ref() else { return };
    let Some(qi) = e.selected else { return };
    let Some(q) = e.queries.get(qi) else { return };
    let file = format!("{}.csv", crate::export_dialog::stem(&q.name));
    let (out, then) = match &app.shell.pick_save {
        Some(pick) => {
            let title = app.lang.t("Export results", "Exporter les résultats");
            match pick(app.lang, title, &file, &["csv"]) {
                Some(p) => (p.display().to_string(), Then::Output(EXPORT_OUTPUT)),
                None => return,
            }
        }
        None if app.shell.download.is_some() => (file.clone(), Then::Download(EXPORT_OUTPUT, file)),
        None => (file, Then::Output(EXPORT_OUTPUT)),
    };
    let p = with_drafts(
        e,
        qi,
        eval_params(e, q, json!({"expression": q.draft, "out": out})),
    );
    app.run(
        Call {
            id: EXPORT,
            params: p,
        },
        then,
    );
}

/// The answer of Export results, while it is the last output.
fn exported(app: &App) -> Option<Option<Result<Value, String>>> {
    match &app.output {
        Some((id, r)) if *id == EXPORT_OUTPUT => Some(r.clone()),
        _ => None,
    }
}

/// The names of the window's queries (`query.new` and `query.rename` keep them unique).
fn names(e: &Editor) -> Vec<String> {
    e.queries.iter().map(|q| q.name.clone()).collect()
}

/// New source › Blank query, Duplicate or Reference (`kind`) of the selected query.
pub fn new_query_of(app: &mut App, kind: &str) {
    let Some(e) = app.cmd.pq.as_mut() else { return };
    let mut p = json!({"kind": kind, "names": names(e)});
    if kind != "blank" {
        let Some(q) = e.selected.and_then(|i| e.queries.get(i)) else {
            return;
        };
        p["of"] = json!(q.name);
        if kind == "duplicate" {
            p["expression"] = json!(q.draft);
        }
    }
    e.generation = e.generation.wrapping_add(1);
    e.busy = true;
    e.problem = None;
    let g = e.generation;
    app.run(Call { id: NEW, params: p }, Then::Pq(Step::New(g)));
}

/// Renames the selected query to `new_name`, the drafts that read it following.
pub fn rename_query(app: &mut App, new_name: &str) {
    let Some(e) = app.cmd.pq.as_mut() else { return };
    let Some(qi) = e.selected else { return };
    let Some(q) = e.queries.get(qi) else { return };
    let drafts: Vec<Value> = e
        .queries
        .iter()
        .map(|q| json!({"name": q.name, "expression": q.draft}))
        .collect();
    let p = json!({"name": q.name, "new_name": new_name, "queries": drafts});
    e.generation = e.generation.wrapping_add(1);
    e.busy = true;
    e.problem = None;
    let g = e.generation;
    app.run(
        Call {
            id: RENAME,
            params: p,
        },
        Then::Pq(Step::Rename(g, qi, new_name.to_string())),
    );
}

/// Deletes query `qi` from the window (Close & Apply deletes it from the model).
pub fn delete_query(app: &mut App, qi: usize) {
    let Some(e) = app.cmd.pq.as_mut() else { return };
    if qi >= e.queries.len() {
        return;
    }
    let q = e.queries.remove(qi);
    if let Some(m) = q.model_name {
        e.removed.push(m);
    }
    e.confirm = None;
    e.renaming = None;
    e.filter = None;
    e.advanced = None;
    if e.queries.is_empty() {
        e.selected = None;
        e.step = None;
        e.preview = None;
        e.formula.clear();
        e.columns.clear();
        return;
    }
    let next = match e.selected {
        Some(s) if s > qi => s.saturating_sub(1),
        Some(s) if s == qi => qi.min(e.queries.len().saturating_sub(1)),
        Some(s) => s,
        None => 0,
    };
    // older answers name the old indexes
    e.generation = e.generation.wrapping_add(1);
    e.busy = false;
    e.selected = None;
    select_query(app, next);
}

/// Advanced editor › Done: the text checked, taken as the draft when it parses.
pub fn advanced_done(app: &mut App) {
    let Some(e) = app.cmd.pq.as_mut() else { return };
    let (Some(qi), Some(text)) = (e.selected, e.advanced.clone()) else {
        return;
    };
    if e.queries.get(qi).is_some_and(|q| q.draft == text) {
        e.advanced = None;
        e.problem = None;
        return;
    }
    e.generation = e.generation.wrapping_add(1);
    e.busy = true;
    e.problem = None;
    let g = e.generation;
    app.run(
        Call {
            id: STEPS,
            params: json!({"expression": text}),
        },
        Then::Pq(Step::Advanced(g, qi)),
    );
}

/// Steps read for query `qi`: show `step` (default the last one).
fn took_steps(app: &mut App, qi: usize, v: &Value, step: Option<String>) {
    let Some(e) = app.cmd.pq.as_mut() else { return };
    let rows = step_rows(v);
    let step = step
        .filter(|n| rows.iter().any(|r| &r.name == n))
        .or_else(|| v.get("result_step").and_then(Value::as_str).map(str::to_string))
        .or_else(|| rows.last().map(|r| r.name.clone()));
    if let Some(q) = e.queries.get_mut(qi) {
        q.steps = Some(rows);
        q.connectors = v
            .get("connectors")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect();
    }
    if e.selected == Some(qi)
        && let Some(st) = step
    {
        select_step(app, &st);
    }
    read_folding(app, qi);
}

/// Shows query `i`: its steps (read once), then its last step.
pub fn select_query(app: &mut App, i: usize) {
    let Some(e) = app.cmd.pq.as_mut() else { return };
    let Some(q) = e.queries.get(i) else { return };
    e.selected = Some(i);
    e.columns.clear();
    e.form = None;
    e.filter = None;
    e.advanced = None;
    e.renaming = None;
    e.rename = None;
    e.problem = None;
    e.preview = None;
    e.profiled = None;
    e.step = None;
    e.formula.clear();
    if let Some(rows) = &q.steps {
        if let Some(last) = rows.last().map(|r| r.name.clone()) {
            select_step(app, &last);
        }
        return;
    }
    let draft = q.draft.clone();
    e.generation = e.generation.wrapping_add(1);
    e.busy = true;
    let g = e.generation;
    app.run(
        Call {
            id: STEPS,
            params: json!({"expression": draft}),
        },
        Then::Pq(Step::Steps(g, i)),
    );
}

/// Shows step `name` of the selected query: the formula bar and the preview at it.
pub fn select_step(app: &mut App, name: &str) {
    let Some(e) = app.cmd.pq.as_mut() else { return };
    let Some(qi) = e.selected else { return };
    let Some(q) = e.queries.get(qi) else { return };
    let formula = q
        .steps
        .as_ref()
        .and_then(|rows| rows.iter().find(|r| r.name == name))
        .map(|r| r.expression.clone())
        .unwrap_or_default();
    let draft = q.draft.clone();
    e.step = Some(name.to_string());
    e.formula = formula;
    e.preview = None;
    e.rename = None;
    e.filter = None;
    let g = e.generation;
    let Some(q) = e.queries.get(qi) else { return };
    let p = with_drafts(
        e,
        qi,
        eval_params(
            e,
            q,
            json!({"expression": draft, "step": name, "limit": PREVIEW_ROWS}),
        ),
    );
    app.run(
        Call {
            id: PREVIEW,
            params: p,
        },
        Then::Pq(Step::Preview(g, qi, name.to_string())),
    );
    refresh_profile(app);
}

/// The step settings gear: step `step`'s form, filled with the settings that wrote it; OK
/// writes the step anew (`query.edit_step {settings}`).
pub fn step_settings(app: &mut App, step: &str, l: Lang) {
    let Some(e) = app.cmd.pq.as_mut() else { return };
    let settings = e
        .selected
        .and_then(|i| e.queries.get(i))
        .and_then(|q| q.steps.as_ref())
        .and_then(|rows| rows.iter().find(|r| r.name == step))
        .and_then(|r| r.settings.clone());
    let Some((form, columns)) = settings
        .as_ref()
        .and_then(|v| crate::pq_tabs::settings_form(v, l))
    else {
        return;
    };
    e.form = Some(form);
    e.editing = Some(step.to_string());
    if !columns.is_empty() {
        e.columns = columns;
    }
    e.filter = None;
    e.problem = None;
}

/// Runs one of the step commands on the selected query's draft.
fn edit(app: &mut App, id: &'static str, mut params: Value) {
    let Some(e) = app.cmd.pq.as_mut() else { return };
    let Some(qi) = e.selected else { return };
    let Some(q) = e.queries.get(qi) else { return };
    params["expression"] = json!(q.draft);
    e.generation = e.generation.wrapping_add(1);
    e.busy = true;
    e.problem = None;
    let g = e.generation;
    app.run(Call { id, params }, Then::Pq(Step::Edit(g, qi)));
}

/// Adds the step of transform `step` (`{kind, …}`) after the step shown.
pub fn transform(app: &mut App, step: Value) {
    let after = app.cmd.pq.as_ref().and_then(|e| e.step.clone());
    let mut p = json!({"step": step});
    if let Some(a) = after {
        p["after"] = json!(a);
    }
    edit(app, APPLY_STEP, p);
}

/// The `query.apply` call of the changed queries (none when nothing changed).
pub fn apply_call(app: &App) -> Option<Call> {
    let e = app.cmd.pq.as_ref()?;
    let queries: Vec<Value> = e
        .queries
        .iter()
        .filter(|q| q.changed())
        .map(|q| json!({"name": q.name, "expression": q.draft}))
        .collect();
    let renamed: Vec<Value> = e
        .queries
        .iter()
        .filter(|q| q.renamed())
        .filter_map(|q| {
            q.model_name
                .as_ref()
                .map(|m| json!({"name": m, "new_name": q.name}))
        })
        .collect();
    if queries.is_empty() && renamed.is_empty() && e.removed.is_empty() {
        return None;
    }
    let mut p = json!({"queries": queries});
    if !renamed.is_empty() {
        p["renamed"] = json!(renamed);
    }
    if !e.removed.is_empty() {
        p["removed"] = json!(e.removed);
    }
    if let Some(r) = &e.data_root {
        p["data_root"] = json!(r);
    }
    Some(Call { id: APPLY, params: p })
}

/// Close & Apply.
pub fn close_and_apply(app: &mut App) {
    let Some(c) = apply_call(app) else {
        app.cmd.pq = None;
        return;
    };
    let Some(e) = app.cmd.pq.as_mut() else { return };
    e.generation = e.generation.wrapping_add(1);
    e.busy = true;
    e.problem = None;
    let g = e.generation;
    app.run(c, Then::Pq(Step::Apply(g)));
}

/// The data type of preview column `name`.
fn column_type(e: &Editor, name: &str) -> String {
    match &e.preview {
        Some(Ok(p)) => p
            .columns
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, t)| t.clone())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// A value typed in a form, read for a column of type `ty`: a number for numeric columns, true
/// or false for logical ones, else text; nothing typed is null.
pub fn typed_value(text: &str, ty: &str) -> Value {
    if text.is_empty() {
        return Value::Null;
    }
    match ty {
        "int64" | "double" | "decimal" => match text.trim().replace(',', ".").parse::<f64>() {
            Ok(n) if n.is_finite() => json!(n),
            _ => json!(text),
        },
        "boolean" => match text.trim().to_lowercase().as_str() {
            "true" | "vrai" => json!(true),
            "false" | "faux" => json!(false),
            _ => json!(text),
        },
        _ => json!(text),
    }
}

/// What a click asks for (applied after drawing).
enum Act {
    Query(usize),
    Step(String),
    Run(Value),
    Form(Form),
    SubmitForm,
    CancelForm,
    Formula,
    Rename(String),
    Remove(String),
    InsertAfter,
    CloseApply,
    Discard,
    /// Close: asks first when there are changes.
    AskClose,
    Advanced,
    AdvancedDone,
    /// Format the Advanced editor's text (`true`) or the formula.
    Format(bool),
    Filter(String),
    FilterOk,
    /// Blank query, duplicate or reference.
    NewQuery(&'static str),
    /// Shows the selected query's rename field.
    StartRename,
    RenameQuery(String),
    AskDelete(usize),
    Delete(usize),
    /// View native query of the step.
    Native(String),
    CloseNative,
    /// Diagnose step.
    Diagnose(String),
    CloseDiagnosis,
    Tab(EditorTab),
    /// View › Column quality, Column distribution, Column profile, entire data set.
    Toggle(Toggle),
    /// The picked columns changed: the column profile follows.
    Reprofile,
    /// View › Query dependencies.
    Dependencies,
    /// The step settings gear of a step.
    Settings(String),
    /// Home › Export results: the selected query's rows to a CSV file.
    Export,
}

impl Act {
    fn of(a: crate::pq_tabs::TabAct) -> Self {
        match a {
            crate::pq_tabs::TabAct::Run(v) => Act::Run(v),
            crate::pq_tabs::TabAct::Form(f) => Act::Form(f),
        }
    }
}

/// The View tab's profiling switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggle {
    Quality,
    Distribution,
    Profile,
    Full,
}

/// Turns one of the View tab's profiling switches on or off, and reads the profile again.
pub fn toggle(app: &mut App, which: Toggle) {
    let Some(e) = app.cmd.pq.as_mut() else { return };
    match which {
        Toggle::Quality => e.quality = !e.quality,
        Toggle::Distribution => e.distribution = !e.distribution,
        Toggle::Profile => e.profile = !e.profile,
        Toggle::Full => e.full = !e.full,
    }
    refresh_profile(app);
}

/// The step `{kind, …}` of the open form, or why it cannot run.
fn form_step(e: &Editor, f: &Form) -> Result<Value, String> {
    let cols = &e.columns;
    let first = cols.first().cloned().unwrap_or_default();
    Ok(match f {
        Form::Filter { column, op, value } => {
            let (op, _, _) = OPS.get(*op).copied().unwrap_or(OPS[0]);
            let v = typed_value(value, &column_type(e, column));
            json!({"kind": "filter_rows", "column": column, "op": op, "value": v})
        }
        Form::Rename { column, new_name } => {
            json!({"kind": "rename_columns", "renames": [{"column": column, "new_name": new_name}]})
        }
        Form::ChangeType { ty } => {
            let (ty, _, _) = TYPES.get(*ty).copied().unwrap_or(TYPES[0]);
            let c: Vec<Value> = cols.iter().map(|c| json!({"column": c, "type": ty})).collect();
            json!({"kind": "change_type", "columns": c})
        }
        Form::Replace { find, replace } => {
            let ty = column_type(e, &first);
            json!({"kind": "replace_values", "columns": cols, "find": typed_value(find, &ty),
                "replace": typed_value(replace, &ty)})
        }
        Form::Group { name, op, column } => {
            let (op, _, _) = AGGS.get(*op).copied().unwrap_or(AGGS[0]);
            let mut a = json!({"name": name, "op": op});
            if !column.is_empty() {
                a["column"] = json!(column);
            }
            json!({"kind": "group_by", "keys": cols, "aggregations": [a]})
        }
        Form::Merge {
            other,
            key,
            other_key,
            join,
        } => {
            let (join, _, _) = JOINS.get(*join).copied().unwrap_or(JOINS[0]);
            json!({"kind": "merge", "other": other, "keys": [key], "other_keys": [other_key],
                "join": join, "column": other})
        }
        Form::Append { other } => json!({"kind": "append", "others": [other]}),
        Form::Expand { column, columns } => {
            let names: Vec<&str> = columns
                .split(',')
                .map(str::trim)
                .filter(|x| !x.is_empty())
                .collect();
            json!({"kind": "expand", "column": column, "columns": names, "prefix": true})
        }
        Form::Split { delimiter, parts } => {
            let n: u64 = parts.trim().parse().map_err(|_| "parts".to_string())?;
            json!({"kind": "split_column", "column": first, "delimiter": delimiter, "parts": n})
        }
        Form::Pivot { values, agg } => {
            let agg = if *agg == 0 {
                "none"
            } else {
                AGGS.get(agg.saturating_sub(1)).map_or("none", |a| a.0)
            };
            json!({"kind": "pivot", "column": first, "values": values, "aggregate": agg})
        }
        Form::Custom { name, formula } => json!({"kind": "add_column", "name": name, "formula": formula}),
        Form::Params { step, fields, .. } => {
            let mut step = step.clone();
            let ty = column_type(e, &first);
            for f in fields {
                let t = f.value.trim();
                let v = match f.kind {
                    FieldKind::Text => json!(f.value),
                    FieldKind::Count => json!(t.parse::<u64>().map_err(|_| f.en.to_string())?),
                    FieldKind::Int => json!(t.parse::<i64>().map_err(|_| f.en.to_string())?),
                    FieldKind::Number => match t.replace(',', ".").parse::<f64>() {
                        Ok(n) if n.is_finite() => json!(n),
                        _ => return Err(f.en.to_string()),
                    },
                    FieldKind::Value => typed_value(&f.value, &ty),
                };
                step[f.key] = v;
            }
            step
        }
        Form::Conditional {
            name,
            rules,
            otherwise,
        } => {
            let mut out = Vec::with_capacity(rules.len());
            for r in rules {
                let (op, _, _) = OPS.get(r.op).copied().unwrap_or(OPS[0]);
                let ty = column_type(e, &r.column);
                let mut rule = json!({"column": r.column, "op": op, "output": typed_value(&r.output, "")});
                if !op.starts_with("is_") {
                    rule["value"] = typed_value(&r.value, &ty);
                }
                out.push(rule);
            }
            json!({"kind": "conditional_column", "name": name, "rules": out,
                "otherwise": typed_value(otherwise, "")})
        }
    })
}

/// Draws the open window.
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut e) = app.cmd.pq.clone() else { return };
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let mut open = true;
    let mut acts = Vec::new();
    let export = exported(app);
    let area = ctx.content_rect();
    let size = egui::vec2(
        (area.width() - 40.0).clamp(640.0, 1400.0),
        (area.height() - 40.0).clamp(420.0, 900.0),
    );
    egui::Window::new(l.t("Power Query editor", "Éditeur Power Query"))
        .id(egui::Id::new("bicraft-pq-editor"))
        .collapsible(false)
        .resizable(false)
        .fixed_size(size)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            // a narrow window scrolls the ribbon sideways rather than growing past the screen
            editor_tabs(ui, &e, l, &mut acts);
            egui::ScrollArea::horizontal()
                .id_salt("bicraft-pq-ribbon")
                .show(ui, |ui| match e.tab {
                    EditorTab::Home => ribbon(ui, &e, l, &mut acts),
                    EditorTab::Transform => {
                        let mut more = Vec::new();
                        crate::pq_tabs::transform_ribbon(ui, &e, l, &mut more);
                        acts.extend(more.into_iter().map(Act::of));
                    }
                    EditorTab::AddColumn => {
                        let mut more = Vec::new();
                        crate::pq_tabs::add_column_ribbon(ui, &e, l, &mut more);
                        acts.extend(more.into_iter().map(Act::of));
                    }
                    EditorTab::View => view_ribbon(ui, &e, l, &mut acts),
                });
            ui.separator();
            if let Some(c) = e.confirm.clone() {
                confirm_ui(ui, &mut e, &c, l, &t, &mut acts);
            }
            if e.advanced.is_none() {
                formula_bar(ui, &mut e, l, &mut acts);
            }
            if let Some(f) = e.form.clone() {
                let mut f = f;
                form_ui(ui, &e, &mut f, l, &t, &mut acts);
                e.form = Some(f);
            }
            if e.filter.is_some() {
                filter_ui(ui, &mut e, l, &t, &mut acts);
            }
            if let Some(p) = &e.problem {
                ui.label(RichText::new(format!("⚠ {p}")).color(t.warn));
            }
            match &export {
                Some(None) => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(l.t("Exporting…", "Export en cours…"));
                    });
                }
                Some(Some(Ok(v))) => {
                    let rows = v.get("rows").and_then(Value::as_u64).unwrap_or(0);
                    let path = v.get("path").and_then(Value::as_str).unwrap_or("");
                    ui.label(
                        RichText::new(format!(
                            "✓ {} {} ({rows} {})",
                            l.t("Written:", "Écrit :"),
                            clip(path, 200),
                            l.t("rows", "lignes")
                        ))
                        .color(t.ok),
                    );
                }
                Some(Some(Err(err))) => {
                    ui.label(RichText::new(format!("✕ {}", clip(err, 400))).color(t.err));
                }
                None => {}
            }
            if let Some(n) = e.native.clone() {
                native_ui(ui, &n, l, &t, &mut acts);
            }
            if let Some(d) = e.diagnosis.clone() {
                diagnosis_ui(ui, &d, l, &t, &mut acts);
            }
            if e.advanced.is_some() {
                advanced_ui(ui, &mut e, l, &t, &mut acts);
                return;
            }
            if let Some(d) = e.deps.as_mut() {
                if crate::pq_deps::show(ui, d, l, &t) == Some(crate::pq_deps::DepsAct::Close) {
                    e.deps = None;
                }
                return;
            }
            let h = ui.available_height() - 4.0;
            ui.horizontal_top(|ui| {
                queries_pane(ui, &mut e, l, &t, h, &mut acts);
                rule(ui, &t, h);
                let w = (ui.available_width() - 250.0).max(200.0);
                ui.vertical(|ui| {
                    ui.set_width(w);
                    ui.set_height(h);
                    preview_ui(ui, &mut e, l, &t, &mut acts);
                });
                rule(ui, &t, h);
                steps_pane(ui, &mut e, l, &t, &mut acts);
            });
        });
    if !open && e.has_changes() {
        // the window's X: asks first
        open = true;
        e.confirm = Some(Confirm::Close);
    }
    app.cmd.pq = if open { Some(e) } else { None };
    for a in acts {
        match a {
            Act::Query(i) => select_query(app, i),
            Act::Step(n) => select_step(app, &n),
            Act::Run(step) => transform(app, step),
            Act::Form(f) => {
                if let Some(e) = app.cmd.pq.as_mut() {
                    e.form = Some(f);
                    e.editing = None;
                    e.filter = None;
                    e.problem = None;
                }
            }
            Act::SubmitForm => {
                let r = app
                    .cmd
                    .pq
                    .as_ref()
                    .and_then(|e| e.form.as_ref().map(|f| form_step(e, f)));
                let editing = app.cmd.pq.as_ref().and_then(|e| e.editing.clone());
                match r {
                    Some(Ok(settings)) if editing.is_some() => {
                        edit(app, EDIT_STEP, json!({"step": editing, "settings": settings}));
                    }
                    Some(Ok(step)) => transform(app, step),
                    Some(Err(why)) => {
                        if let Some(e) = app.cmd.pq.as_mut() {
                            e.problem = Some(format!(
                                "{} `{why}`",
                                l.t("Check the value of", "Vérifiez la valeur de")
                            ));
                        }
                    }
                    None => {}
                }
            }
            Act::CancelForm => {
                if let Some(e) = app.cmd.pq.as_mut() {
                    e.form = None;
                    e.editing = None;
                }
            }
            Act::Settings(step) => step_settings(app, &step, l),
            Act::Formula => {
                let Some(e) = app.cmd.pq.as_ref() else { continue };
                let Some(step) = e.step.clone() else { continue };
                let formula = e.formula.clone();
                edit(app, EDIT_STEP, json!({"step": step, "formula": formula}));
            }
            Act::Rename(new_name) => {
                let Some(step) = app.cmd.pq.as_ref().and_then(|e| e.step.clone()) else {
                    continue;
                };
                edit(app, EDIT_STEP, json!({"step": step, "new_name": new_name}));
            }
            Act::Remove(step) => edit(app, REMOVE_STEP, json!({"step": step})),
            Act::InsertAfter => transform(app, json!({"kind": "custom"})),
            Act::CloseApply => close_and_apply(app),
            Act::Discard => app.cmd.pq = None,
            Act::AskClose => {
                if let Some(e) = app.cmd.pq.as_mut() {
                    if e.has_changes() {
                        e.confirm = Some(Confirm::Close);
                    } else {
                        app.cmd.pq = None;
                    }
                }
            }
            Act::Advanced => {
                if let Some(e) = app.cmd.pq.as_mut()
                    && let Some(q) = e.selected.and_then(|i| e.queries.get(i))
                {
                    e.advanced = Some(q.draft.clone());
                    e.form = None;
                    e.filter = None;
                    e.problem = None;
                }
            }
            Act::AdvancedDone => advanced_done(app),
            Act::Format(advanced) => format_text(app, advanced),
            Act::Filter(c) => open_filter(app, &c),
            Act::FilterOk => {
                let step = app
                    .cmd
                    .pq
                    .as_ref()
                    .and_then(|e| e.filter.as_ref())
                    .and_then(filter_step);
                match step {
                    Some(step) => transform(app, step),
                    None => {
                        if let Some(e) = app.cmd.pq.as_mut() {
                            e.filter = None;
                        }
                    }
                }
            }
            Act::NewQuery(kind) => new_query_of(app, kind),
            Act::StartRename => {
                if let Some(e) = app.cmd.pq.as_mut() {
                    e.renaming = e.selected.and_then(|i| e.queries.get(i)).map(|q| q.name.clone());
                }
            }
            Act::RenameQuery(n) => rename_query(app, &n),
            Act::AskDelete(i) => {
                if let Some(e) = app.cmd.pq.as_mut() {
                    e.confirm = Some(Confirm::Delete(i));
                }
            }
            Act::Delete(i) => delete_query(app, i),
            Act::Native(step) => view_native(app, &step),
            Act::Diagnose(step) => diagnose(app, &step),
            Act::CloseDiagnosis => {
                if let Some(e) = app.cmd.pq.as_mut() {
                    e.diagnosis = None;
                }
            }
            Act::CloseNative => {
                if let Some(e) = app.cmd.pq.as_mut() {
                    e.native = None;
                }
            }
            Act::Tab(tab) => {
                if let Some(e) = app.cmd.pq.as_mut() {
                    e.tab = tab;
                }
            }
            Act::Toggle(which) => toggle(app, which),
            Act::Dependencies => open_dependencies(app),
            Act::Export => export_results(app),
            Act::Reprofile => {
                if app.cmd.pq.as_ref().is_some_and(|e| e.profile) {
                    refresh_profile(app);
                }
            }
        }
    }
}

/// A question of the window (delete a query, close with changes).
fn confirm_ui(ui: &mut Ui, e: &mut Editor, c: &Confirm, l: Lang, t: &Tokens, acts: &mut Vec<Act>) {
    egui::Frame::new()
        .fill(t.card)
        .stroke(egui::Stroke::new(1.0, t.warn))
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| match c {
                Confirm::Delete(i) => {
                    let name = e.queries.get(*i).map(|q| q.name.clone()).unwrap_or_default();
                    ui.label(
                        RichText::new(format!(
                            "⚠ {} « {} » ? {}",
                            l.t("Delete query", "Supprimer la requête"),
                            clip(&name, 60),
                            l.t(
                                "Close & Apply deletes it from the model; queries that read it stop working.",
                                "Fermer et appliquer la supprime du modèle ; les requêtes qui la lisent ne fonctionnent plus."
                            )
                        ))
                        .color(t.text),
                    );
                    if ui.button(l.t("Delete", "Supprimer")).clicked() {
                        acts.push(Act::Delete(*i));
                    }
                    if ui.button(l.t("Keep it", "La garder")).clicked() {
                        e.confirm = None;
                    }
                }
                Confirm::Close => {
                    ui.label(
                        RichText::new(format!(
                            "⚠ {}",
                            l.t(
                                "The queries have changes that are not applied. Apply them to the model?",
                                "Les requêtes ont des modifications non appliquées. Les appliquer au modèle ?"
                            )
                        ))
                        .color(t.text),
                    );
                    if widgets::primary(ui, l.t("Apply changes", "Appliquer les modifications"), !e.busy, "")
                        .clicked()
                        && !e.busy
                    {
                        e.confirm = None;
                        acts.push(Act::CloseApply);
                    }
                    if ui.button(l.t("Discard", "Abandonner")).clicked() {
                        acts.push(Act::Discard);
                    }
                    if ui.button(l.t("Keep editing", "Continuer l'édition")).clicked() {
                        e.confirm = None;
                    }
                }
            });
        });
}

/// The Advanced editor: the selected query's whole M.
/// The names the M editors complete beyond the text's own: the window's queries, the preview's
/// columns and, for the formula bar, the steps before the one shown.
fn m_names(e: &Editor, formula: bool) -> m_box::Names {
    let query = e.selected.and_then(|i| e.queries.get(i));
    let steps = match (formula, query.and_then(|q| q.steps.as_ref())) {
        (true, Some(rows)) => rows
            .iter()
            .map(|r| r.name.clone())
            .take_while(|n| Some(n) != e.step.as_ref())
            .collect(),
        _ => Vec::new(),
    };
    let columns = match &e.preview {
        Some(Ok(p)) => p.columns.iter().map(|c| c.0.clone()).collect(),
        _ => Vec::new(),
    };
    m_box::Names {
        queries: e.queries.iter().map(|q| q.name.clone()).collect(),
        steps,
        columns,
    }
}

fn advanced_ui(ui: &mut Ui, e: &mut Editor, l: Lang, t: &Tokens, acts: &mut Vec<Act>) {
    let qname = e
        .selected
        .and_then(|i| e.queries.get(i))
        .map(|q| q.name.clone())
        .unwrap_or_default();
    let busy = e.busy;
    let names = m_names(e, false);
    let assist = &mut e.m_advanced;
    let Some(text) = e.advanced.as_mut() else { return };
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!(
                "{} · {}",
                l.t("Advanced editor", "Éditeur avancé"),
                clip(&qname, 60)
            ))
            .font(theme::bold(14.0)),
        );
        if busy {
            ui.spinner();
        }
    });
    ui.label(
        RichText::new(l.t(
            "The whole query in M. Done checks it: a query that does not parse is not taken.",
            "Toute la requête en M. Terminé la vérifie : une requête qui ne s'analyse pas n'est pas reprise.",
        ))
        .color(t.text_dim),
    );
    let label = l.t("Query M", "M de la requête");
    let h = (ui.available_height() - 40.0).max(120.0);
    egui::ScrollArea::vertical()
        .id_salt("bicraft-pq-advanced")
        .max_height(h)
        .show(ui, |ui| {
            m_box::show(
                ui,
                text,
                assist,
                egui::Id::new("bicraft-pq-advanced-text"),
                label,
                &names,
                18,
                f32::INFINITY,
                !busy,
                l,
            );
        });
    let can_format = !busy && !text.trim().is_empty();
    ui.horizontal(|ui| {
        if widgets::primary(ui, l.t("Done", "Terminé"), !busy, "").clicked() && !busy {
            acts.push(Act::AdvancedDone);
        }
        if ui
            .add_enabled(can_format, egui::Button::new(l.t("Format", "Mettre en forme")))
            .on_hover_text(l.t(
                "Lay the M out: one step per line, even spacing; comments are kept",
                "Mettre le M en forme : une étape par ligne, espacement régulier ; les commentaires restent",
            ))
            .clicked()
        {
            acts.push(Act::Format(true));
        }
        if ui.button(l.t("Cancel", "Annuler")).clicked() {
            e.advanced = None;
            e.problem = None;
        }
    });
}

/// The comparisons a column's Text or Number filters offer (indexes into [`OPS`]).
fn filter_ops(ty: &str) -> &'static [usize] {
    match ty {
        "string" => &[0, 1, 8, 9, 6, 7],
        "int64" | "double" | "decimal" => &[0, 1, 2, 3, 4, 5],
        _ => &[0, 1],
    }
}

/// A column's filter menu: sorts, text or number filters, the value list.
fn filter_ui(ui: &mut Ui, e: &mut Editor, l: Lang, t: &Tokens, acts: &mut Vec<Act>) {
    let busy = e.busy;
    let Some(f) = e.filter.as_mut() else { return };
    let column = f.column.clone();
    let mut close = false;
    egui::Frame::new()
        .fill(t.card)
        .stroke(egui::Stroke::new(1.0, t.card_border))
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(format!("{} {}", l.t("Filter", "Filtrer"), clip(&column, 60)))
                        .font(theme::bold(13.0)),
                );
                if ui
                    .add_enabled(!busy, egui::Button::new(l.t("Sort ascending", "Tri croissant")))
                    .clicked()
                {
                    acts.push(Act::Run(
                        json!({"kind": "sort", "columns": [{"column": column, "descending": false}]}),
                    ));
                }
                if ui
                    .add_enabled(!busy, egui::Button::new(l.t("Sort descending", "Tri décroissant")))
                    .clicked()
                {
                    acts.push(Act::Run(
                        json!({"kind": "sort", "columns": [{"column": column, "descending": true}]}),
                    ));
                }
                let numeric = matches!(f.ty.as_str(), "int64" | "double" | "decimal");
                let title = if numeric {
                    l.t("Number filters", "Filtres numériques")
                } else if f.ty == "string" {
                    l.t("Text filters", "Filtres de texte")
                } else {
                    l.t("Filters", "Filtres")
                };
                ui.menu_button(title, |ui| {
                    for &i in filter_ops(&f.ty) {
                        let Some(op) = OPS.get(i) else { continue };
                        if ui.button(format!("{}…", pick(l, *op))).clicked() {
                            acts.push(Act::Form(Form::Filter {
                                column: column.clone(),
                                op: i,
                                value: String::new(),
                            }));
                            close = true;
                            ui.close();
                        }
                    }
                });
            });
            ui.add_space(4.0);
            match &mut f.values {
                None => {
                    ui.spinner();
                }
                Some(Err(err)) => {
                    ui.label(RichText::new(format!("⚠ {}", clip(err, 600))).color(t.warn));
                }
                Some(Ok(values)) if values.is_empty() => {
                    ui.label(
                        RichText::new(l.t(
                            "The value list is offered for text, number, true/false, date and date/time columns.",
                            "La liste des valeurs est proposée pour les colonnes de texte, de nombres, vrai/faux, date et date/heure.",
                        ))
                        .color(t.text_dim),
                    );
                }
                Some(Ok(values)) => {
                    let r = ui.label(l.t("Search", "Rechercher"));
                    ui.add(
                        egui::TextEdit::singleline(&mut f.search)
                            .char_limit(256)
                            .desired_width(200.0),
                    )
                    .labelled_by(r.id);
                    let q = f.search.to_lowercase();
                    let mut all = values.iter().all(|x| x.1);
                    if ui
                        .checkbox(&mut all, l.t("(Select all)", "(Sélectionner tout)"))
                        .changed()
                    {
                        for x in values.iter_mut() {
                            x.1 = all;
                        }
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("bicraft-pq-filter-values")
                        .max_height(160.0)
                        .show(ui, |ui| {
                            for (v, on) in values.iter_mut() {
                                let text = if v.is_null() {
                                    "(null)".to_string()
                                } else {
                                    cell(v, l)
                                };
                                if !q.is_empty() && !text.to_lowercase().contains(&q) {
                                    continue;
                                }
                                ui.checkbox(on, text);
                            }
                        });
                    if f.incomplete {
                        ui.label(
                            RichText::new(format!(
                                "⚠ {}",
                                l.t(
                                    "The list may be incomplete (the first 1000 rows).",
                                    "La liste est peut-être incomplète (les 1000 premières lignes)."
                                )
                            ))
                            .color(t.text_dim),
                        );
                    }
                }
            }
            ui.horizontal(|ui| {
                let ticked = matches!(&f.values, Some(Ok(v)) if v.iter().any(|x| x.1));
                if widgets::primary(ui, l.t("OK", "OK"), !busy && ticked, "").clicked() && !busy && ticked {
                    acts.push(Act::FilterOk);
                }
                if ui.button(l.t("Cancel", "Annuler")).clicked() {
                    close = true;
                }
            });
        });
    if close {
        e.filter = None;
    }
}

/// A vertical rule of a fixed height between two columns.
fn rule(ui: &mut Ui, t: &Tokens, height: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(9.0, height.max(10.0)), egui::Sense::hover());
    ui.painter().vline(
        rect.center().x,
        rect.y_range(),
        egui::Stroke::new(1.0, t.separator),
    );
}

/// A transform button with its icon, big or small as in the main ribbon, enabled when the step it
/// needs is shown (and columns are picked); its tooltip says what it does, or why it cannot run.
fn tbutton(ui: &mut Ui, (icon, big): (Icon, bool), label: &str, tip: &str, ok: bool, why: &str) -> bool {
    let r = if big {
        widgets::big_button(ui, icon, label, ok)
    } else {
        widgets::small_button(ui, icon, label, ok, false)
    };
    let tip = if ok { tip } else { why };
    let r = if tip.is_empty() { r } else { r.on_hover_text(tip) };
    r.clicked() && ok
}

const BIG: bool = true;
const SMALL: bool = false;

/// The editor's ribbon, drawn like the main window's: groups of big and small icon buttons.
fn ribbon(ui: &mut Ui, e: &Editor, l: Lang, acts: &mut Vec<Act>) {
    let ready = !e.busy && e.step.is_some();
    let cols = e.columns.len();
    let first = e.columns.first().cloned().unwrap_or_default();
    let pick1 = l.t(
        "Click a column header first.",
        "Cliquez d'abord sur un en-tête de colonne.",
    );
    let other_queries: Vec<String> = e
        .queries
        .iter()
        .enumerate()
        .filter(|(i, q)| Some(*i) != e.selected && !q.parameter)
        .map(|(_, q)| q.name.clone())
        .collect();
    let changed = e.has_changes();
    let ready = ready && e.advanced.is_none();
    let col_ok = ready && cols > 0;
    let one_other = other_queries.first().cloned().unwrap_or_default();
    let no_other = l.t(
        "The model has no other query.",
        "Le modèle n'a pas d'autre requête.",
    );
    ui.horizontal(|ui| {
        ui.set_height(widgets::RIBBON_H);
        // Close & Apply, Discard changes, Advanced editor
        widgets::group(ui, |ui| {
            let apply = if changed {
                l.t("Close &\nApply", "Fermer et\nappliquer")
            } else {
                l.t("Close", "Fermer")
            };
            if tbutton(ui, (Icon::Check, BIG), apply, "", !e.busy, "") {
                acts.push(Act::CloseApply);
            }
            widgets::rows(ui, |ui| {
                if changed
                    && tbutton(
                        ui,
                        (Icon::Undo, SMALL),
                        l.t("Discard changes", "Abandonner"),
                        l.t(
                            "Close without applying the changes",
                            "Fermer sans appliquer les modifications",
                        ),
                        true,
                        "",
                    )
                {
                    acts.push(Act::AskClose);
                }
                if tbutton(
                    ui,
                    (Icon::Edit, SMALL),
                    l.t("Advanced editor", "Éditeur avancé"),
                    l.t(
                        "View and edit the whole query in M",
                        "Afficher et modifier toute la requête en M",
                    ),
                    !e.busy && e.selected.is_some() && e.advanced.is_none(),
                    l.t("Pick a query.", "Choisissez une requête."),
                ) {
                    acts.push(Act::Advanced);
                }
                if tbutton(
                    ui,
                    (Icon::Export, SMALL),
                    l.t("Export results", "Exporter les résultats"),
                    l.t(
                        "Write every row of the query to a CSV file",
                        "Écrit toutes les lignes de la requête dans un fichier CSV",
                    ),
                    !e.busy && e.selected.is_some() && e.advanced.is_none(),
                    l.t("Pick a query.", "Choisissez une requête."),
                ) {
                    acts.push(Act::Export);
                }
            });
        });
        // Manage columns
        widgets::group(ui, |ui| {
            widgets::rows(ui, |ui| {
                if tbutton(
                    ui,
                    (Icon::Delete, SMALL),
                    l.t("Remove columns", "Supprimer les colonnes"),
                    l.t("Remove the columns picked", "Supprime les colonnes choisies"),
                    col_ok,
                    pick1,
                ) {
                    acts.push(Act::Run(json!({"kind": "remove_columns", "columns": e.columns})));
                }
                if tbutton(
                    ui,
                    (Icon::Column, SMALL),
                    l.t("Keep columns", "Garder les colonnes"),
                    l.t("Remove the other columns", "Supprime les autres colonnes"),
                    col_ok,
                    pick1,
                ) {
                    acts.push(Act::Run(json!({"kind": "keep_columns", "columns": e.columns})));
                }
                if tbutton(
                    ui,
                    (Icon::Rename, SMALL),
                    l.t("Rename column", "Renommer la colonne"),
                    "",
                    col_ok,
                    pick1,
                ) {
                    acts.push(Act::Form(Form::Rename {
                        column: first.clone(),
                        new_name: first.clone(),
                    }));
                }
            });
            widgets::rows(ui, |ui| {
                if tbutton(
                    ui,
                    (Icon::Format, SMALL),
                    l.t("Change type", "Modifier le type"),
                    "",
                    col_ok,
                    pick1,
                ) {
                    acts.push(Act::Form(Form::ChangeType { ty: 0 }));
                }
            });
            if tbutton(
                ui,
                (Icon::Table, BIG),
                l.t("Use first row\nas headers", "Première ligne\nen en-têtes"),
                "",
                ready,
                "",
            ) {
                acts.push(Act::Run(json!({"kind": "promote_headers"})));
            }
        });
        // Reduce and sort rows
        widgets::group(ui, |ui| {
            widgets::rows(ui, |ui| {
                if tbutton(
                    ui,
                    (Icon::Filter, SMALL),
                    l.t("Filter rows", "Filtrer les lignes"),
                    "",
                    col_ok,
                    pick1,
                ) {
                    acts.push(Act::Form(Form::Filter {
                        column: first.clone(),
                        op: 0,
                        value: String::new(),
                    }));
                }
                if tbutton(
                    ui,
                    (Icon::Sort, SMALL),
                    l.t("Sort ascending", "Tri croissant"),
                    "",
                    col_ok,
                    pick1,
                ) {
                    acts.push(Act::Run(
                        json!({"kind": "sort", "columns": [{"column": first, "descending": false}]}),
                    ));
                }
                if tbutton(
                    ui,
                    (Icon::Sort, SMALL),
                    l.t("Sort descending", "Tri décroissant"),
                    "",
                    col_ok,
                    pick1,
                ) {
                    acts.push(Act::Run(
                        json!({"kind": "sort", "columns": [{"column": first, "descending": true}]}),
                    ));
                }
            });
            widgets::rows(ui, |ui| {
                if tbutton(
                    ui,
                    (Icon::Minus, SMALL),
                    l.t("Remove duplicates", "Supprimer les doublons"),
                    l.t(
                        "Over the columns picked, else whole rows",
                        "Sur les colonnes choisies, sinon les lignes entières",
                    ),
                    ready,
                    "",
                ) {
                    acts.push(Act::Run(
                        json!({"kind": "remove_duplicates", "columns": e.columns}),
                    ));
                }
                if tbutton(
                    ui,
                    (Icon::Edit, SMALL),
                    l.t("Replace values", "Remplacer les valeurs"),
                    "",
                    col_ok,
                    pick1,
                ) {
                    acts.push(Act::Form(Form::Replace {
                        find: String::new(),
                        replace: String::new(),
                    }));
                }
            });
            widgets::rows(ui, |ui| {
                let mut more = Vec::new();
                crate::pq_tabs::reduce_rows(ui, e, ready, l, &mut more);
                acts.extend(more.into_iter().map(Act::of));
            });
        });
        // Combine
        widgets::group(ui, |ui| {
            if tbutton(
                ui,
                (Icon::Group, BIG),
                l.t("Group by", "Regrouper par"),
                l.t(
                    "Group by the columns picked",
                    "Regroupe selon les colonnes choisies",
                ),
                ready,
                "",
            ) {
                acts.push(Act::Form(Form::Group {
                    name: l.t("Count", "Nombre").into(),
                    op: 0,
                    column: String::new(),
                }));
            }
            widgets::rows(ui, |ui| {
                if tbutton(
                    ui,
                    (Icon::Link, SMALL),
                    l.t("Merge queries", "Fusionner des requêtes"),
                    "",
                    col_ok && !other_queries.is_empty(),
                    if cols == 0 { pick1 } else { no_other },
                ) {
                    acts.push(Act::Form(Form::Merge {
                        other: one_other.clone(),
                        key: first.clone(),
                        other_key: first.clone(),
                        join: 0,
                    }));
                }
                if tbutton(
                    ui,
                    (Icon::Layers, SMALL),
                    l.t("Expand column", "Développer la colonne"),
                    l.t(
                        "A merged table column into its columns",
                        "Une colonne de table fusionnée en ses colonnes",
                    ),
                    col_ok,
                    pick1,
                ) {
                    acts.push(Act::Form(Form::Expand {
                        column: first.clone(),
                        columns: String::new(),
                    }));
                }
                if tbutton(
                    ui,
                    (Icon::Plus, SMALL),
                    l.t("Append queries", "Ajouter des requêtes"),
                    "",
                    ready && !other_queries.is_empty(),
                    no_other,
                ) {
                    acts.push(Act::Form(Form::Append {
                        other: one_other.clone(),
                    }));
                }
            });
        });
        // Transform
        widgets::group(ui, |ui| {
            widgets::rows(ui, |ui| {
                if tbutton(
                    ui,
                    (Icon::Ungroup, SMALL),
                    l.t("Split column", "Fractionner la colonne"),
                    "",
                    col_ok,
                    pick1,
                ) {
                    acts.push(Act::Form(Form::Split {
                        delimiter: ",".into(),
                        parts: "2".into(),
                    }));
                }
                if tbutton(
                    ui,
                    (Icon::Move, SMALL),
                    l.t("Unpivot other columns", "Dépivoter les autres"),
                    l.t(
                        "Turn the columns not picked into attribute and value rows",
                        "Dépivote les colonnes non choisies en lignes attribut et valeur",
                    ),
                    col_ok,
                    pick1,
                ) {
                    acts.push(Act::Run(
                        json!({"kind": "unpivot", "columns": e.columns, "other": true}),
                    ));
                }
                if tbutton(
                    ui,
                    (Icon::Hierarchy, SMALL),
                    l.t("Pivot column", "Pivoter la colonne"),
                    "",
                    col_ok,
                    pick1,
                ) {
                    acts.push(Act::Form(Form::Pivot {
                        values: String::new(),
                        agg: 3,
                    }));
                }
            });
            if tbutton(
                ui,
                (Icon::Calculator, BIG),
                l.t("Custom\ncolumn", "Colonne\npersonnalisée"),
                "",
                ready,
                "",
            ) {
                acts.push(Act::Form(Form::Custom {
                    name: l.t("Custom", "Personnalisée").into(),
                    formula: String::new(),
                }));
            }
        });
    });
}

/// The editor ribbon's tab strip (Home, View).
fn editor_tabs(ui: &mut Ui, e: &Editor, l: Lang, acts: &mut Vec<Act>) {
    ui.horizontal(|ui| {
        for (tab, label) in [
            (EditorTab::Home, l.t("Home", "Accueil")),
            (EditorTab::Transform, l.t("Transform", "Transformer")),
            (EditorTab::AddColumn, l.t("Add column", "Ajouter une colonne")),
            (EditorTab::View, l.t("View", "Affichage")),
        ] {
            let r = ui.selectable_label(e.tab == tab, RichText::new(label).font(theme::bold(13.0)));
            if r.clicked() && e.tab != tab {
                acts.push(Act::Tab(tab));
            }
        }
    });
}

/// The View tab: the data profiling switches (Microsoft Learn, "Using the data profiling tools")
/// and the Advanced editor.
fn view_ribbon(ui: &mut Ui, e: &Editor, l: Lang, acts: &mut Vec<Act>) {
    let ready = !e.busy && e.step.is_some() && e.advanced.is_none();
    let why = l.t("Pick a query.", "Choisissez une requête.");
    ui.horizontal(|ui| {
        ui.set_height(widgets::RIBBON_H);
        // Data preview
        widgets::group(ui, |ui| {
            widgets::rows(ui, |ui| {
                for (which, on, label, tip) in [
                    (
                        Toggle::Quality,
                        e.quality,
                        l.t("Column quality", "Qualité de colonne"),
                        l.t(
                            "Valid, error and empty values under each column header",
                            "Valeurs valides, en erreur et vides sous chaque en-tête de colonne",
                        ),
                    ),
                    (
                        Toggle::Distribution,
                        e.distribution,
                        l.t("Column distribution", "Distribution de colonne"),
                        l.t(
                            "Distinct and unique values, with a small histogram",
                            "Valeurs distinctes et uniques, avec un petit histogramme",
                        ),
                    ),
                    (
                        Toggle::Profile,
                        e.profile,
                        l.t("Column profile", "Profil de colonne"),
                        l.t(
                            "Statistics and value distribution of the column picked, in a bottom pane",
                            "Statistiques et distribution des valeurs de la colonne choisie, dans un volet en bas",
                        ),
                    ),
                ] {
                    let mut v = on;
                    let r = ui.add_enabled(ready, egui::Checkbox::new(&mut v, label));
                    let r = r.on_hover_text(tip).on_disabled_hover_text(why);
                    if r.clicked() && ready {
                        acts.push(Act::Toggle(which));
                    }
                }
            });
            widgets::rows(ui, |ui| {
                let label = l.t(
                    "Profile the entire data set",
                    "Profiler l'ensemble du jeu de données",
                );
                let mut v = e.full;
                let r = ui
                    .add_enabled(ready, egui::Checkbox::new(&mut v, label))
                    .on_disabled_hover_text(why)
                    .on_hover_text(if ready {
                        l.t(
                            "Off: the top 1000 rows are profiled",
                            "Désactivé : les 1000 premières lignes sont profilées",
                        )
                    } else {
                        why
                    });
                if r.clicked() && ready {
                    acts.push(Act::Toggle(Toggle::Full));
                }
            });
        });
        // Dependencies
        widgets::group(ui, |ui| {
            if tbutton(
                ui,
                (Icon::Model, BIG),
                l.t("Query\ndependencies", "Dépendances\nde requête"),
                l.t(
                    "A diagram of the queries, their sources and the model tables they load into",
                    "Un diagramme des requêtes, de leurs sources et des tables du modèle qu'elles chargent",
                ),
                !e.busy && e.deps.is_none() && e.advanced.is_none(),
                "",
            ) {
                acts.push(Act::Dependencies);
            }
        });
        // Advanced
        widgets::group(ui, |ui| {
            if tbutton(
                ui,
                (Icon::Edit, BIG),
                l.t("Advanced\neditor", "Éditeur\navancé"),
                l.t(
                    "View and edit the whole query in M",
                    "Afficher et modifier toute la requête en M",
                ),
                !e.busy && e.selected.is_some() && e.advanced.is_none(),
                why,
            ) {
                acts.push(Act::Advanced);
            }
        });
    });
}

fn formula_bar(ui: &mut Ui, e: &mut Editor, l: Lang, acts: &mut Vec<Act>) {
    let has = e.step.is_some();
    let names = m_names(e, true);
    ui.horizontal(|ui| {
        let r = ui.label(RichText::new("fx").font(theme::bold(13.0)));
        let label = l.t("Formula of the step", "Formule de l'étape");
        let w = (ui.available_width() - 270.0).max(120.0);
        let enabled = has && !e.busy;
        m_box::show(
            ui,
            &mut e.formula,
            &mut e.m_formula,
            egui::Id::new("bicraft-pq-formula-text"),
            label,
            &names,
            2,
            w,
            enabled,
            l,
        )
        .labelled_by(r.id);
        let current = e
            .selected
            .and_then(|i| e.queries.get(i))
            .and_then(|q| q.steps.as_ref())
            .and_then(|rows| rows.iter().find(|x| Some(&x.name) == e.step.as_ref()))
            .map(|x| x.expression.as_str());
        let ok = has && !e.busy && current != Some(e.formula.as_str());
        if ui
            .add_enabled(
                ok,
                egui::Button::new(l.t("Apply formula", "Appliquer la formule")),
            )
            .clicked()
        {
            acts.push(Act::Formula);
        }
        let can_format = has && !e.busy && !e.formula.trim().is_empty();
        if ui
            .add_enabled(can_format, egui::Button::new(l.t("Format", "Mettre en forme")))
            .clicked()
        {
            acts.push(Act::Format(false));
        }
        if e.busy {
            ui.spinner();
        }
    });
}

fn combo(ui: &mut Ui, id: &str, label: &str, items: &[&str], at: &mut usize) {
    let r = ui.label(label);
    let shown = items.get(*at).copied().unwrap_or_default();
    egui::ComboBox::from_id_salt(id)
        .selected_text(shown)
        .width(220.0)
        .show_ui(ui, |ui| {
            for (i, it) in items.iter().enumerate() {
                ui.selectable_value(at, i, *it);
            }
        })
        .response
        .labelled_by(r.id);
}

fn names_combo(ui: &mut Ui, id: &str, label: &str, names: &[String], at: &mut String) {
    let r = ui.label(label);
    egui::ComboBox::from_id_salt(id)
        .selected_text(clip(at, 40))
        .width(200.0)
        .show_ui(ui, |ui| {
            for n in names {
                ui.selectable_value(at, n.clone(), clip(n, 60));
            }
        })
        .response
        .labelled_by(r.id);
}

fn field(ui: &mut Ui, label: &str, value: &mut String, width: f32) {
    let r = ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .char_limit(4096)
            .desired_width(width),
    )
    .labelled_by(r.id);
}

fn form_ui(ui: &mut Ui, e: &Editor, f: &mut Form, l: Lang, t: &Tokens, acts: &mut Vec<Act>) {
    let columns: Vec<String> = match &e.preview {
        Some(Ok(p)) => p.columns.iter().map(|(n, _)| n.clone()).collect(),
        _ => Vec::new(),
    };
    let others: Vec<String> = e
        .queries
        .iter()
        .enumerate()
        .filter(|(i, q)| Some(*i) != e.selected && !q.parameter)
        .map(|(_, q)| q.name.clone())
        .collect();
    let picked = clip(&e.columns.join(", "), 80);
    egui::Frame::new()
        .fill(t.card)
        .stroke(egui::Stroke::new(1.0, t.card_border))
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                match f {
                    Form::Filter { column, op, value } => {
                        ui.label(
                            RichText::new(l.t("Filter rows", "Filtrer les lignes")).font(theme::bold(13.0)),
                        );
                        names_combo(ui, "pq-filter-col", l.t("Column", "Colonne"), &columns, column);
                        let ops: Vec<&str> = OPS.iter().map(|o| pick(l, *o)).collect();
                        combo(
                            ui,
                            "pq-filter-op",
                            l.t("Keep rows where", "Garder les lignes où"),
                            &ops,
                            op,
                        );
                        if OPS.get(*op).is_some_and(|o| !o.0.starts_with("is_")) {
                            field(ui, l.t("Value", "Valeur"), value, 160.0);
                        }
                    }
                    Form::Rename { column, new_name } => {
                        ui.label(
                            RichText::new(l.t("Rename column", "Renommer la colonne"))
                                .font(theme::bold(13.0)),
                        );
                        ui.label(clip(column, 60));
                        field(ui, l.t("New name", "Nouveau nom"), new_name, 200.0);
                    }
                    Form::ChangeType { ty } => {
                        ui.label(
                            RichText::new(l.t("Change type", "Modifier le type")).font(theme::bold(13.0)),
                        );
                        ui.label(picked.clone());
                        let types: Vec<&str> = TYPES.iter().map(|x| pick(l, *x)).collect();
                        combo(ui, "pq-type", l.t("Type", "Type"), &types, ty);
                    }
                    Form::Replace { find, replace } => {
                        ui.label(
                            RichText::new(l.t("Replace values", "Remplacer les valeurs"))
                                .font(theme::bold(13.0)),
                        );
                        ui.label(picked.clone());
                        field(ui, l.t("Value to find", "Valeur à rechercher"), find, 140.0);
                        field(ui, l.t("Replace with", "Remplacer par"), replace, 140.0);
                    }
                    Form::Group { name, op, column } => {
                        ui.label(RichText::new(l.t("Group by", "Regrouper par")).font(theme::bold(13.0)));
                        ui.label(if picked.is_empty() {
                            l.t("(all rows)", "(toutes les lignes)").to_string()
                        } else {
                            picked.clone()
                        });
                        field(ui, l.t("New column", "Nouvelle colonne"), name, 120.0);
                        let aggs: Vec<&str> = AGGS.iter().map(|x| pick(l, *x)).collect();
                        combo(ui, "pq-group-op", l.t("Operation", "Opération"), &aggs, op);
                        if AGGS
                            .get(*op)
                            .is_some_and(|a| !matches!(a.0, "count" | "count_distinct" | "all_rows"))
                        {
                            names_combo(ui, "pq-group-col", l.t("Column", "Colonne"), &columns, column);
                        }
                    }
                    Form::Merge {
                        other,
                        key,
                        other_key,
                        join,
                    } => {
                        ui.label(
                            RichText::new(l.t("Merge queries", "Fusionner des requêtes"))
                                .font(theme::bold(13.0)),
                        );
                        names_combo(ui, "pq-merge-key", l.t("Column", "Colonne"), &columns, key);
                        names_combo(
                            ui,
                            "pq-merge-other",
                            l.t("With query", "Avec la requête"),
                            &others,
                            other,
                        );
                        field(ui, l.t("Its column", "Sa colonne"), other_key, 120.0);
                        let joins: Vec<&str> = JOINS.iter().map(|x| pick(l, *x)).collect();
                        combo(
                            ui,
                            "pq-merge-join",
                            l.t("Join kind", "Type de jointure"),
                            &joins,
                            join,
                        );
                    }
                    Form::Append { other } => {
                        ui.label(
                            RichText::new(l.t("Append queries", "Ajouter des requêtes"))
                                .font(theme::bold(13.0)),
                        );
                        names_combo(
                            ui,
                            "pq-append-other",
                            l.t("Query to append", "Requête à ajouter"),
                            &others,
                            other,
                        );
                    }
                    Form::Expand { column, columns } => {
                        ui.label(
                            RichText::new(l.t("Expand column", "Développer la colonne"))
                                .font(theme::bold(13.0)),
                        );
                        ui.label(clip(column, 60));
                        field(
                            ui,
                            l.t(
                                "Columns (comma-separated)",
                                "Colonnes (séparées par des virgules)",
                            ),
                            columns,
                            220.0,
                        );
                    }
                    Form::Split { delimiter, parts } => {
                        ui.label(
                            RichText::new(l.t("Split column", "Fractionner la colonne"))
                                .font(theme::bold(13.0)),
                        );
                        ui.label(picked.clone());
                        field(ui, l.t("Delimiter", "Séparateur"), delimiter, 50.0);
                        field(ui, l.t("Number of columns", "Nombre de colonnes"), parts, 50.0);
                    }
                    Form::Pivot { values, agg } => {
                        ui.label(
                            RichText::new(l.t("Pivot column", "Pivoter la colonne")).font(theme::bold(13.0)),
                        );
                        ui.label(picked.clone());
                        names_combo(
                            ui,
                            "pq-pivot-values",
                            l.t("Values column", "Colonne de valeurs"),
                            &columns,
                            values,
                        );
                        let mut aggs: Vec<&str> = vec![l.t("Don't aggregate", "Ne pas agréger")];
                        aggs.extend(AGGS.iter().filter(|a| a.0 != "all_rows").map(|x| pick(l, *x)));
                        combo(ui, "pq-pivot-agg", l.t("Aggregate", "Agréger"), &aggs, agg);
                    }
                    Form::Custom { name, formula } => {
                        ui.label(
                            RichText::new(l.t("Custom column", "Colonne personnalisée"))
                                .font(theme::bold(13.0)),
                        );
                        field(
                            ui,
                            l.t("New column name", "Nom de la nouvelle colonne"),
                            name,
                            140.0,
                        );
                        let r = ui.label(l.t("Formula (M, each row)", "Formule (M, pour chaque ligne)"));
                        ui.add(
                            egui::TextEdit::singleline(formula)
                                .code_editor()
                                .char_limit(1 << 16)
                                .desired_width(320.0)
                                .hint_text("[Qty] * 2"),
                        )
                        .labelled_by(r.id);
                    }
                    Form::Params { title, fields, .. } => {
                        ui.label(RichText::new(clip(title, 60)).font(theme::bold(13.0)));
                        if !picked.is_empty() {
                            ui.label(picked.clone());
                        }
                        for x in fields.iter_mut() {
                            let w = if x.kind == FieldKind::Text { 140.0 } else { 70.0 };
                            field(ui, l.t(x.en, x.fr), &mut x.value, w);
                        }
                    }
                    Form::Conditional {
                        name,
                        rules,
                        otherwise,
                    } => {
                        ui.label(
                            RichText::new(l.t("Conditional column", "Colonne conditionnelle"))
                                .font(theme::bold(13.0)),
                        );
                        field(
                            ui,
                            l.t("New column name", "Nom de la nouvelle colonne"),
                            name,
                            120.0,
                        );
                        let ops: Vec<&str> = OPS.iter().map(|o| pick(l, *o)).collect();
                        let n = rules.len();
                        let mut drop = None;
                        for (i, r) in rules.iter_mut().enumerate() {
                            ui.end_row();
                            ui.label(if i == 0 {
                                l.t("If", "Si")
                            } else {
                                l.t("Else if", "Sinon si")
                            });
                            names_combo(
                                ui,
                                &format!("pq-cond-col-{i}"),
                                l.t("Column", "Colonne"),
                                &columns,
                                &mut r.column,
                            );
                            combo(
                                ui,
                                &format!("pq-cond-op-{i}"),
                                l.t("Operator", "Opérateur"),
                                &ops,
                                &mut r.op,
                            );
                            if OPS.get(r.op).is_some_and(|o| !o.0.starts_with("is_")) {
                                field(ui, l.t("Value", "Valeur"), &mut r.value, 90.0);
                            }
                            field(ui, l.t("Output", "Sortie"), &mut r.output, 90.0);
                            if n > 1 && ui.button(l.t("Remove clause", "Supprimer la clause")).clicked() {
                                drop = Some(i);
                            }
                        }
                        if let Some(i) = drop
                            && i < rules.len()
                        {
                            rules.remove(i);
                        }
                        ui.end_row();
                        if rules.len() < 100 && ui.button(l.t("Add clause", "Ajouter une clause")).clicked() {
                            rules.push(CondRule {
                                column: columns.first().cloned().unwrap_or_default(),
                                op: 0,
                                value: String::new(),
                                output: String::new(),
                            });
                        }
                        field(ui, l.t("Else", "Sinon"), otherwise, 90.0);
                    }
                }
                if widgets::primary(ui, l.t("OK", "OK"), !e.busy, "").clicked() && !e.busy {
                    acts.push(Act::SubmitForm);
                }
                if ui.button(l.t("Cancel", "Annuler")).clicked() {
                    acts.push(Act::CancelForm);
                }
            });
        });
}

/// New query, Duplicate, Reference, Rename and Delete of the queries list (and of a query's
/// context menu).
fn query_tools(ui: &mut Ui, e: &Editor, i: Option<usize>, l: Lang, acts: &mut Vec<Act>, menu: bool) {
    let free = !e.busy;
    let picked = free && i.is_some();
    let mut item = |ui: &mut Ui, icon: Icon, label: &str, ok: bool, act: Act| {
        if widgets::small_button(ui, icon, label, ok, false).clicked() && ok {
            if let (Some(i), false) = (i, e.selected == i) {
                acts.push(Act::Query(i));
            }
            acts.push(act);
            if menu {
                ui.close();
            }
        }
    };
    item(
        ui,
        Icon::Plus,
        l.t("New query", "Nouvelle requête"),
        free,
        Act::NewQuery("blank"),
    );
    item(
        ui,
        Icon::Duplicate,
        l.t("Duplicate", "Dupliquer"),
        picked,
        Act::NewQuery("duplicate"),
    );
    item(
        ui,
        Icon::Link,
        l.t("Reference", "Référence"),
        picked,
        Act::NewQuery("reference"),
    );
    item(
        ui,
        Icon::Rename,
        l.t("Rename query", "Renommer la requête"),
        picked,
        Act::StartRename,
    );
    item(
        ui,
        Icon::Delete,
        l.t("Delete query", "Supprimer la requête"),
        picked,
        Act::AskDelete(i.unwrap_or_default()),
    );
}

fn queries_pane(ui: &mut Ui, e: &mut Editor, l: Lang, t: &Tokens, h: f32, acts: &mut Vec<Act>) {
    ui.vertical(|ui| {
        ui.set_width(200.0);
        ui.set_height(h);
        ui.label(
            RichText::new(format!("{} [{}]", l.t("Queries", "Requêtes"), e.queries.len()))
                .font(theme::bold(13.0)),
        );
        ui.horizontal_wrapped(|ui| query_tools(ui, e, e.selected, l, acts, false));
        let busy = e.busy;
        if let Some(new_name) = e.renaming.as_mut() {
            let r = ui.label(l.t("New query name", "Nouveau nom de la requête"));
            ui.add(
                egui::TextEdit::singleline(new_name)
                    .char_limit(256)
                    .desired_width(180.0),
            )
            .labelled_by(r.id);
            let name = new_name.trim().to_string();
            let current = e.selected.and_then(|i| e.queries.get(i)).map(|q| q.name.clone());
            let mut cancel = false;
            ui.horizontal(|ui| {
                let ok = !busy && !name.is_empty() && current.as_deref() != Some(name.as_str());
                if ui
                    .add_enabled(ok, egui::Button::new(l.t("Rename", "Renommer")))
                    .clicked()
                    && ok
                {
                    acts.push(Act::RenameQuery(name.clone()));
                }
                if ui.button(l.t("Cancel", "Annuler")).clicked() {
                    cancel = true;
                }
            });
            if cancel {
                e.renaming = None;
            }
        }
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("bicraft-pq-queries")
            .max_height(h - 24.0)
            .show(ui, |ui| {
                if e.queries.is_empty() && !e.busy {
                    ui.label(
                        RichText::new(l.t(
                            "The model has no Power Query (M) query.",
                            "Le modèle n'a aucune requête Power Query (M).",
                        ))
                        .color(t.text_dim),
                    );
                }
                for (i, q) in e.queries.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                        let icon = if q.source == "query" {
                            Icon::List
                        } else {
                            Icon::Table
                        };
                        icons::paint(ui.painter(), rect, icon, t.text, false);
                        let mut label = clip(&q.name, 40);
                        if q.changed() || q.renamed() {
                            label.push_str(" *");
                        }
                        let mut text = RichText::new(label);
                        if q.parameter {
                            text = text.color(t.text_dim);
                        }
                        let tip = match (q.source.as_str(), q.changed()) {
                            ("new", _) => l.t(
                                "New query: Close & Apply loads it",
                                "Nouvelle requête : Fermer et appliquer la charge",
                            ),
                            (_, true) => l.t(
                                "Changed: Close & Apply writes it",
                                "Modifiée : Fermer et appliquer l'écrit",
                            ),
                            ("query", _) => l.t("Shared query", "Requête partagée"),
                            _ => l.t("Table query", "Requête de table"),
                        };
                        let r = ui
                            .selectable_label(e.selected == Some(i), text)
                            .on_hover_text(tip);
                        if r.clicked() {
                            acts.push(Act::Query(i));
                        }
                        crate::keyboard::context_menu(&r, |ui| query_tools(ui, e, Some(i), l, acts, true));
                    });
                }
            });
    });
}

fn steps_pane(ui: &mut Ui, e: &mut Editor, l: Lang, t: &Tokens, acts: &mut Vec<Act>) {
    ui.vertical(|ui| {
        ui.set_width(230.0);
        ui.label(RichText::new(l.t("Applied steps", "Étapes appliquées")).font(theme::bold(13.0)));
        let rows: Vec<StepRow> = e
            .selected
            .and_then(|i| e.queries.get(i))
            .and_then(|q| q.steps.clone())
            .unwrap_or_default();
        let folding = e
            .selected
            .and_then(|i| e.queries.get(i))
            .and_then(|q| q.folding.clone());
        egui::ScrollArea::vertical()
            .id_salt("bicraft-pq-steps")
            .max_height(320.0)
            .show(ui, |ui| {
                for r in &rows {
                    let on = e.step.as_deref() == Some(r.name.as_str());
                    let tip = r.function.clone().unwrap_or_default();
                    let fold = match &folding {
                        Some(Ok(f)) => f.iter().find(|x| x.name == r.name).cloned(),
                        _ => None,
                    };
                    let resp = ui
                        .horizontal(|ui| {
                            let resp = ui.selectable_label(on, clip(&r.name, 32));
                            if let Some(f) = &fold {
                                // a column of indicators at the pane's right edge
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    fold_badge(ui, f, l, t)
                                });
                            }
                            resp
                        })
                        .inner;
                    let resp = if tip.is_empty() {
                        resp
                    } else {
                        resp.on_hover_text(tip)
                    };
                    if resp.clicked() && !on {
                        acts.push(Act::Step(r.name.clone()));
                    }
                    crate::keyboard::context_menu(&resp, |ui| {
                        step_menu(ui, &r.name, fold.as_ref(), l, t, acts)
                    });
                }
            });
        if let Some(Err(err)) = &folding {
            ui.label(
                RichText::new(format!(
                    "{} {}",
                    l.t("Folding indicators:", "Indicateurs de pliage :"),
                    clip(err, 200)
                ))
                .color(t.text_dim),
            );
        }
        let Some(step) = e.step.clone() else { return };
        ui.separator();
        let busy = e.busy;
        match e.rename.as_mut() {
            Some(new_name) => {
                let r = ui.label(l.t("New step name", "Nouveau nom d'étape"));
                ui.add(
                    egui::TextEdit::singleline(new_name)
                        .char_limit(256)
                        .desired_width(200.0),
                )
                .labelled_by(r.id);
                let name = new_name.clone();
                ui.horizontal(|ui| {
                    let ok = !busy && !name.trim().is_empty() && name != step;
                    if ui
                        .add_enabled(ok, egui::Button::new(l.t("Rename", "Renommer")))
                        .clicked()
                    {
                        acts.push(Act::Rename(name.trim().to_string()));
                    }
                    if ui.button(l.t("Cancel", "Annuler")).clicked() {
                        e.rename = None;
                    }
                });
            }
            None => {
                if widgets::small_button(
                    ui,
                    Icon::Rename,
                    l.t("Rename step", "Renommer l'étape"),
                    !busy,
                    false,
                )
                .clicked()
                    && !busy
                {
                    e.rename = Some(step.clone());
                }
            }
        }
        let many = rows.len() > 1;
        let del = widgets::small_button(
            ui,
            Icon::Delete,
            l.t("Delete step", "Supprimer l'étape"),
            !busy && many,
            false,
        );
        let del = if many {
            del
        } else {
            del.on_hover_text(l.t(
                "A query keeps at least one step.",
                "Une requête garde au moins une étape.",
            ))
        };
        if del.clicked() && !busy && many {
            acts.push(Act::Remove(step.clone()));
        }
        let gear = !busy
            && rows
                .iter()
                .find(|r| r.name == step)
                .and_then(|r| r.settings.as_ref())
                .is_some_and(|v| crate::pq_tabs::settings_form(v, l).is_some());
        if widgets::small_button(
            ui,
            Icon::Settings,
            l.t("Step settings", "Paramètres de l'étape"),
            gear,
            false,
        )
        .on_hover_text(if gear {
            l.t(
                "Change the values this step was written with",
                "Modifier les valeurs avec lesquelles cette étape a été écrite",
            )
        } else {
            l.t(
                "This step has no settings: change its formula.",
                "Cette étape n'a pas de paramètres : modifiez sa formule.",
            )
        })
        .clicked()
            && gear
        {
            acts.push(Act::Settings(step.clone()));
        }
        if widgets::small_button(
            ui,
            Icon::Plus,
            l.t("Insert step after", "Insérer une étape après"),
            !busy,
            false,
        )
        .clicked()
            && !busy
        {
            acts.push(Act::InsertAfter);
        }
        ui.add_space(6.0);
        ui.label(
            RichText::new(l.t(
                "Each transform adds a step after the step shown.",
                "Chaque transformation ajoute une étape après l'étape affichée.",
            ))
            .color(t.text_dim),
        );
    });
}

/// A folding indicator's icon, colour and words (English, French): never the colour alone.
fn fold_look(folding: &str, t: &Tokens) -> (Icon, egui::Color32, &'static str, &'static str) {
    match folding {
        "folded" => (Icon::Check, t.ok, "Folded", "Plié"),
        "partial" => (Icon::Layers, t.warn, "Partially folded", "Partiellement plié"),
        "not_folded" => (Icon::Close, t.text_dim, "Not folded", "Non plié"),
        _ => (Icon::Help, t.text_dim, "Unknown", "Inconnu"),
    }
}

/// The folding indicator of a step, laid out right to left: a word, then its icon before it,
/// with why on hover.
fn fold_badge(ui: &mut Ui, f: &Fold, l: Lang, t: &Tokens) {
    let (icon, ink, en, fr) = fold_look(&f.folding, t);
    let word = ui.label(RichText::new(l.t(en, fr)).font(theme::font(11.0)).color(ink));
    if !f.reason.is_empty() {
        word.on_hover_text(&f.reason);
    }
    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
    icons::paint(ui.painter(), rect, icon, ink, false);
}

/// A step's context menu: View native query, greyed out with why when the step does not fold.
fn step_menu(ui: &mut Ui, step: &str, fold: Option<&Fold>, l: Lang, t: &Tokens, acts: &mut Vec<Act>) {
    let ok = fold.is_some_and(|f| f.native);
    let label = l.t("View native query", "Afficher la requête native");
    let b = ui.add_enabled(ok, egui::Button::new(label));
    if b.clicked() {
        acts.push(Act::Native(step.to_string()));
        ui.close();
    }
    if ui.button(l.t("Diagnose step", "Diagnostiquer l'étape")).clicked() {
        acts.push(Act::Diagnose(step.to_string()));
        ui.close();
    }
    if !ok {
        let why = match fold {
            None => l.t(
                "The folding indicators are being read.",
                "Les indicateurs de pliage sont en cours de lecture.",
            ),
            Some(f) if f.folding == "folded" => l.t(
                "This step is a navigation table: no query is sent yet.",
                "Cette étape est une table de navigation : aucune requête n'est encore envoyée.",
            ),
            Some(f) if f.folding == "unknown" => l.t(
                "The folding of this step is unknown.",
                "Le pliage de cette étape est inconnu.",
            ),
            Some(_) => l.t(
                "This step does not fold: BIcraft runs it, not the source.",
                "Cette étape n'est pas pliée : BIcraft l'exécute, pas la source.",
            ),
        };
        ui.label(RichText::new(why).font(theme::font(11.0)).color(t.text_dim));
    }
}

/// View native query: the step's query as the source would be sent it, read-only.
fn native_ui(ui: &mut Ui, n: &NativeView, l: Lang, t: &Tokens, acts: &mut Vec<Act>) {
    widgets::card(t).show(ui, |ui| {
        ui.label(
            RichText::new(format!(
                "{} — {}",
                l.t("Native query", "Requête native"),
                clip(&n.step, 60)
            ))
            .font(theme::bold(13.0)),
        );
        match &n.text {
            None => {
                ui.label(RichText::new(l.t("Reading…", "Lecture…")).color(t.text_dim));
            }
            Some(Ok((text, kind))) => {
                let what = if kind == "url" {
                    l.t(
                        "The URL BIcraft sends to the source for this step (read-only):",
                        "L'URL que BIcraft envoie à la source pour cette étape (lecture seule) :",
                    )
                } else {
                    l.t(
                        "The query BIcraft sends to the source for this step (read-only):",
                        "La requête que BIcraft envoie à la source pour cette étape (lecture seule) :",
                    )
                };
                let r = ui.label(RichText::new(what).color(t.text_dim));
                let mut shown = text.as_str();
                egui::ScrollArea::vertical()
                    .id_salt("bicraft-pq-native")
                    .max_height(160.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut shown)
                                .font(egui::TextStyle::Monospace)
                                .desired_width(f32::INFINITY)
                                .desired_rows(3),
                        )
                        .labelled_by(r.id);
                    });
            }
            Some(Err(why)) => {
                ui.label(RichText::new(format!("⚠ {}", clip(why, 400))).color(t.warn));
            }
        }
        if ui
            .button(l.t("Close native query", "Fermer la requête native"))
            .clicked()
        {
            acts.push(Act::CloseNative);
        }
    });
}

/// Diagnose step: BIcraft's own time, the rows, and each request sent to a source.
fn diagnosis_ui(ui: &mut Ui, d: &DiagView, l: Lang, t: &Tokens, acts: &mut Vec<Act>) {
    widgets::card(t).show(ui, |ui| {
        ui.label(
            RichText::new(format!(
                "{} — {}",
                l.t("Diagnostics", "Diagnostics"),
                clip(&d.step, 60)
            ))
            .font(theme::bold(13.0)),
        );
        match &d.result {
            None => {
                ui.label(
                    RichText::new(l.t("Running the query…", "Exécution de la requête…")).color(t.text_dim),
                );
            }
            Some(Err(why)) => {
                ui.label(RichText::new(format!("⚠ {}", clip(why, 400))).color(t.warn));
            }
            Some(Ok(x)) => {
                let rows = x.rows.map_or_else(|| "—".to_string(), |r| r.to_string());
                ui.label(format!(
                    "{} {} ms · {} {rows} · {} {}",
                    l.t("BIcraft's time:", "Temps de BIcraft :"),
                    x.elapsed_ms,
                    l.t("rows:", "lignes :"),
                    l.t("source requests:", "requêtes à la source :"),
                    x.requests.len()
                ));
                if let Some(err) = &x.error {
                    ui.label(RichText::new(format!("⚠ {}", clip(err, 400))).color(t.warn));
                }
                egui::ScrollArea::vertical()
                    .id_salt("bicraft-pq-diagnosis")
                    .max_height(140.0)
                    .show(ui, |ui| {
                        for (kind, connector, rows, ms, text) in &x.requests {
                            let what = if kind == "list" {
                                l.t("list", "liste")
                            } else {
                                l.t("query", "requête")
                            };
                            ui.label(
                                RichText::new(format!(
                                    "{connector} · {what} · {rows} {} · {ms} ms — {}",
                                    l.t("rows", "lignes"),
                                    clip(text, 300)
                                ))
                                .monospace(),
                            );
                        }
                    });
            }
        }
        if ui
            .button(l.t("Close diagnostics", "Fermer les diagnostics"))
            .clicked()
        {
            acts.push(Act::CloseDiagnosis);
        }
    });
}

/// A cell of a preview as text.
fn cell(v: &Value, l: Lang) -> String {
    match v {
        Value::Null => "null".into(),
        Value::String(s) => clip(s, MAX_CELL),
        Value::Bool(b) => if *b {
            l.t("True", "Vrai")
        } else {
            l.t("False", "Faux")
        }
        .to_string(),
        other => clip(&other.to_string(), MAX_CELL),
    }
}

/// A column header's type badge, as the editor marks its columns.
fn type_badge(ty: &str) -> &'static str {
    match ty {
        "string" => "ABC",
        "int64" => "123",
        "double" | "decimal" => "1.2",
        "dateTime" => "DATE",
        "boolean" => "T/F",
        _ => "ABC123",
    }
}

fn preview_ui(ui: &mut Ui, e: &mut Editor, l: Lang, t: &Tokens, acts: &mut Vec<Act>) {
    let busy = e.busy;
    let filtering = e.filter.as_ref().map(|f| f.column.clone());
    let Some(name) = e.step.clone() else {
        if e.busy {
            ui.spinner();
        } else {
            ui.label(RichText::new(l.t("Pick a query.", "Choisissez une requête.")).color(t.text_dim));
        }
        return;
    };
    let qname = e
        .selected
        .and_then(|i| e.queries.get(i))
        .map(|q| q.name.clone())
        .unwrap_or_default();
    ui.label(RichText::new(format!("{} › {}", clip(&qname, 60), clip(&name, 60))).font(theme::bold(14.0)));
    let picked: BTreeSet<String> = e.columns.iter().cloned().collect();
    let mut toggle: Option<(String, bool)> = None;
    match &e.preview {
        None => {
            ui.spinner();
        }
        Some(Err(err)) => {
            ui.label(RichText::new(format!("⚠ {}", clip(err, 800))).color(t.warn));
        }
        Some(Ok(p)) => {
            let add = ui.input(|i| i.modifiers.command || i.modifiers.shift);
            // the column profile's pane and the profiling status line below the grid
            let below = if e.profile { PROFILE_H + 30.0 } else { 0.0 }
                + if e.profiling() { 26.0 } else { 0.0 }
                + if p.truncated { 20.0 } else { 0.0 };
            let profiled = match &e.profiled {
                Some(Ok(pr)) => Some(pr),
                _ => None,
            };
            egui::ScrollArea::both()
                .id_salt("bicraft-pq-preview")
                .auto_shrink([false, false])
                .max_height((ui.available_height() - below).max(80.0))
                .show(ui, |ui| {
                    egui::Grid::new("bicraft-pq-grid")
                        .striped(true)
                        .spacing(egui::vec2(12.0, 3.0))
                        .show(ui, |ui| {
                            for (n, ty) in &p.columns {
                                let on = picked.contains(n);
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 2.0;
                                    let label =
                                        format!("{} {n}", l.t("Change type of", "Modifier le type de"));
                                    let r = ui.add_enabled_ui(!busy, |ui| {
                                        ui.menu_button(RichText::new(type_badge(ty)).small(), |ui| {
                                            for x in TYPES {
                                                if ui.button(pick(l, x)).clicked() {
                                                    acts.push(Act::Run(json!({"kind": "change_type",
                                                        "columns": [{"column": n, "type": x.0}]})));
                                                    ui.close();
                                                }
                                            }
                                        })
                                    });
                                    r.inner
                                        .response
                                        .on_hover_text(format!("{ty} · {label}"))
                                        .widget_info(|| {
                                            egui::WidgetInfo::labeled(egui::WidgetType::Button, !busy, &label)
                                        });
                                    let r = ui
                                        .selectable_label(
                                            on,
                                            RichText::new(clip(n, MAX_CELL)).font(theme::bold(12.0)),
                                        )
                                        .on_hover_text(format!(
                                            "{ty} · {}",
                                            l.t(
                                                "click to pick, Ctrl+click to add",
                                                "cliquez pour choisir, Ctrl+clic pour ajouter"
                                            )
                                        ));
                                    if r.clicked() {
                                        toggle = Some((n.clone(), add));
                                    }
                                    let label = format!("{} {n}", l.t("Filter", "Filtrer"));
                                    let open = filtering.as_deref() == Some(n.as_str());
                                    let r = widgets::small_button(ui, Icon::ChevronDown, "", !busy, open)
                                        .on_hover_text(&label);
                                    r.widget_info(|| {
                                        egui::WidgetInfo::labeled(egui::WidgetType::Button, !busy, &label)
                                    });
                                    if r.clicked() && !busy {
                                        acts.push(Act::Filter(n.clone()));
                                    }
                                });
                            }
                            ui.end_row();
                            let of =
                                |n: &str| profiled.and_then(|pr| pr.columns.iter().find(|c| c.name == n));
                            if e.quality {
                                for (n, _) in &p.columns {
                                    quality_cell(ui, n, of(n), l, t);
                                }
                                ui.end_row();
                            }
                            if e.distribution {
                                for (n, _) in &p.columns {
                                    distribution_cell(ui, n, of(n), l, t);
                                }
                                ui.end_row();
                            }
                            for row in &p.rows {
                                for v in row {
                                    let text = RichText::new(cell(v, l));
                                    ui.label(if v.is_null() {
                                        text.color(t.text_faint)
                                    } else {
                                        text.color(t.text)
                                    });
                                }
                                ui.end_row();
                            }
                        });
                });
            if p.truncated {
                ui.label(
                    RichText::new(format!(
                        "{} {PREVIEW_ROWS} {}",
                        l.t("The first", "Les"),
                        l.t("rows.", "premières lignes.")
                    ))
                    .color(t.text_dim),
                );
            }
            if e.profiling() {
                profiling_status(ui, e, l, t, acts);
            }
            if e.profile {
                profile_pane(ui, e, l, t);
            }
        }
    }
    if let Some((n, add)) = toggle {
        let first = e.columns.first().cloned();
        if add {
            if let Some(i) = e.columns.iter().position(|c| c == &n) {
                e.columns.remove(i);
            } else {
                e.columns.push(n);
            }
        } else if e.columns == [n.clone()] {
            e.columns.clear();
        } else {
            e.columns = vec![n];
        }
        if e.columns.first() != first.as_ref() {
            acts.push(Act::Reprofile);
        }
    }
}

/// Height of the column profile's pane.
const PROFILE_H: f32 = 190.0;
/// Width of a column's quality and distribution cells.
const CELL_W: f32 = 132.0;

/// A share as the editor shows it: whole percents, `<1%` for a few.
fn pct(x: f64, l: Lang) -> String {
    let n = (x * 100.0).round();
    let n = if x > 0.0 && n < 1.0 {
        "<1".to_string()
    } else {
        format!("{n:.0}")
    };
    match l {
        Lang::Fr => format!("{n} %"),
        Lang::En => format!("{n}%"),
    }
}

/// Column quality under a column header: a bar of the valid, error and empty shares and the
/// three shares with their words (never the colour alone).
fn quality_cell(ui: &mut Ui, name: &str, c: Option<&ColumnProfile>, l: Lang, t: &Tokens) {
    ui.vertical(|ui| {
        ui.set_width(CELL_W);
        ui.spacing_mut().item_spacing.y = 0.0;
        let Some(c) = c else {
            ui.spinner();
            return;
        };
        let (rect, _) = ui.allocate_exact_size(egui::vec2(CELL_W, 4.0), egui::Sense::hover());
        let p = ui.painter();
        let mut x = rect.min.x;
        for (share, col) in [(c.valid, t.ok), (c.error, t.err), (c.empty, t.text_faint)] {
            let w = (share as f32).clamp(0.0, 1.0) * rect.width();
            if w > 0.0 {
                p.rect_filled(
                    egui::Rect::from_min_size(egui::pos2(x, rect.min.y), egui::vec2(w, rect.height())),
                    0.0,
                    col,
                );
            }
            x += w;
        }
        ui.add_space(2.0);
        let words = [
            (l.t("Valid", "Valide"), c.valid, t.ok),
            (l.t("Error", "Erreur"), c.error, t.err),
            (l.t("Empty", "Vide"), c.empty, t.text_faint),
        ];
        let summary: Vec<String> = words
            .iter()
            .map(|(w, x, _)| format!("{w} {}", pct(*x, l)))
            .collect();
        for (word, share, col) in words {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let (dot, _) = ui.allocate_exact_size(egui::vec2(7.0, 7.0), egui::Sense::hover());
                ui.painter().rect_filled(dot, 1.0, col);
                ui.label(
                    RichText::new(format!("{word} {}", pct(share, l)))
                        .small()
                        .color(t.text_dim),
                );
            });
        }
        let label = format!(
            "{} {name}: {}",
            l.t("Quality of", "Qualité de"),
            summary.join(", ")
        );
        ui.allocate_response(egui::vec2(0.0, 0.0), egui::Sense::hover())
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &label));
    });
}

/// Column distribution under a column header: the most frequent values' bars and the distinct
/// and unique counts.
fn distribution_cell(ui: &mut Ui, name: &str, c: Option<&ColumnProfile>, l: Lang, t: &Tokens) {
    ui.vertical(|ui| {
        ui.set_width(CELL_W);
        ui.spacing_mut().item_spacing.y = 1.0;
        let Some(c) = c else {
            ui.spinner();
            return;
        };
        let (rect, _) = ui.allocate_exact_size(egui::vec2(CELL_W, 30.0), egui::Sense::hover());
        let p = ui.painter();
        let top = c.bars.iter().copied().max().unwrap_or(0).max(1);
        let n = c.bars.len().max(1);
        let bw = rect.width() / n as f32;
        for (i, k) in c.bars.iter().enumerate() {
            let h = (*k as f32 / top as f32) * rect.height();
            let x = rect.min.x + i as f32 * bw;
            p.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(x + bw * 0.1, rect.max.y - h.max(1.0)),
                    egui::pos2(x + bw * 0.9, rect.max.y),
                ),
                0.0,
                t.accent,
            );
        }
        let text = format!(
            "{} {}, {} {}",
            c.distinct,
            l.t("distinct", "distinctes"),
            c.unique,
            l.t("unique", "uniques")
        );
        let label = format!("{} {name}: {text}", l.t("Distribution of", "Distribution de"));
        ui.label(RichText::new(&text).small().color(t.text_dim))
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &label));
    });
}

/// The status line under the preview: what profiling reads, a click switching it.
fn profiling_status(ui: &mut Ui, e: &Editor, l: Lang, t: &Tokens, acts: &mut Vec<Act>) {
    ui.horizontal(|ui| {
        let text = if e.full {
            l.t(
                "Column profiling based on the entire data set",
                "Profilage de colonne basé sur l'ensemble du jeu de données",
            )
        } else {
            l.t(
                "Column profiling based on top 1000 rows",
                "Profilage de colonne basé sur les 1000 premières lignes",
            )
        };
        let r = ui
            .add_enabled(
                !e.busy,
                egui::Button::new(RichText::new(text).small()).frame(false),
            )
            .on_hover_text(l.t(
                "Click to switch between the top 1000 rows and the entire data set",
                "Cliquez pour basculer entre les 1000 premières lignes et l'ensemble du jeu de données",
            ));
        if r.clicked() && !e.busy {
            acts.push(Act::Toggle(Toggle::Full));
        }
        match &e.profiled {
            None => {
                ui.spinner();
            }
            Some(Ok(p)) => {
                ui.label(
                    RichText::new(format!(
                        "· {} {}",
                        p.rows,
                        l.t("rows profiled", "lignes profilées")
                    ))
                    .small()
                    .color(t.text_dim),
                );
            }
            Some(Err(err)) => {
                ui.label(
                    RichText::new(format!("⚠ {}", clip(err, 300)))
                        .small()
                        .color(t.warn),
                );
            }
        }
    });
}

/// The column profile's pane: the picked column's statistics and value distribution.
fn profile_pane(ui: &mut Ui, e: &Editor, l: Lang, t: &Tokens) {
    ui.separator();
    let col = match &e.profiled {
        Some(Ok(p)) => p.column.clone(),
        _ => None,
    };
    let Some(col) = col else {
        ui.allocate_ui(egui::vec2(ui.available_width(), PROFILE_H), |ui| {
            let text = if e.columns.is_empty() {
                l.t(
                    "Click a column header to see its profile.",
                    "Cliquez sur un en-tête de colonne pour voir son profil.",
                )
            } else {
                l.t("Reading the profile…", "Lecture du profil…")
            };
            ui.label(RichText::new(text).color(t.text_dim));
        });
        return;
    };
    let name = s(&col, "name");
    let stats = col.get("statistics").cloned().unwrap_or(Value::Null);
    let rows: Vec<(&str, &str)> = vec![
        ("count", l.t("Count", "Nombre")),
        ("errors", l.t("Error", "Erreur")),
        ("empty", l.t("Empty", "Vide")),
        ("distinct", l.t("Distinct", "Distinct")),
        ("unique", l.t("Unique", "Unique")),
        ("empty_string", l.t("Empty string", "Texte vide")),
        ("min", l.t("Min", "Min")),
        ("max", l.t("Max", "Max")),
        ("average", l.t("Average", "Moyenne")),
        ("standard_deviation", l.t("Standard deviation", "Écart type")),
        ("zero", l.t("Zero", "Zéro")),
        ("even", l.t("Even", "Pair")),
        ("odd", l.t("Odd", "Impair")),
    ];
    ui.horizontal_top(|ui| {
        ui.set_height(PROFILE_H);
        ui.vertical(|ui| {
            ui.set_width(280.0);
            ui.label(
                RichText::new(format!(
                    "{} · {}",
                    l.t("Column statistics", "Statistiques de colonne"),
                    clip(&name, 40)
                ))
                .font(theme::bold(13.0)),
            );
            egui::ScrollArea::vertical()
                .id_salt("bicraft-pq-stats")
                .max_height(PROFILE_H - 24.0)
                .show(ui, |ui| {
                    egui::Grid::new("bicraft-pq-stats-grid")
                        .striped(true)
                        .spacing(egui::vec2(16.0, 2.0))
                        .show(ui, |ui| {
                            for (key, word) in rows {
                                let Some(v) = stats.get(key) else { continue };
                                ui.label(RichText::new(word).color(t.text_dim));
                                let shown = match v {
                                    Value::Number(n) => match n.as_f64() {
                                        Some(x) if x.fract() != 0.0 => format!("{x:.4}")
                                            .trim_end_matches('0')
                                            .trim_end_matches('.')
                                            .to_string(),
                                        Some(x) if x.abs() < 1e15 => format!("{x:.0}"),
                                        _ => n.to_string(),
                                    },
                                    other => cell(other, l),
                                };
                                ui.label(RichText::new(shown).color(t.text));
                                ui.end_row();
                            }
                        });
                });
        });
        rule(ui, t, PROFILE_H);
        ui.vertical(|ui| {
            ui.label(
                RichText::new(l.t("Value distribution", "Distribution des valeurs")).font(theme::bold(13.0)),
            );
            let values: Vec<(String, u64)> = col
                .get("values")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(40)
                .map(|x| {
                    (
                        cell(x.get("value").unwrap_or(&Value::Null), l),
                        x.get("count").and_then(Value::as_u64).unwrap_or(0),
                    )
                })
                .collect();
            let top = values.iter().map(|v| v.1).max().unwrap_or(0).max(1);
            let w = (ui.available_width() - 200.0).max(80.0);
            egui::ScrollArea::vertical()
                .id_salt("bicraft-pq-values")
                .max_height(PROFILE_H - 24.0)
                .show(ui, |ui| {
                    for (v, k) in &values {
                        ui.horizontal(|ui| {
                            ui.allocate_ui(egui::vec2(140.0, 16.0), |ui| {
                                ui.set_width(140.0);
                                ui.add(egui::Label::new(RichText::new(clip(v, 24)).small()).truncate());
                            });
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 12.0), egui::Sense::hover());
                            let bw = (*k as f32 / top as f32) * rect.width();
                            ui.painter().rect_filled(
                                egui::Rect::from_min_size(rect.min, egui::vec2(bw.max(1.0), rect.height())),
                                2.0,
                                t.accent,
                            );
                            ui.label(RichText::new(k.to_string()).small().color(t.text_dim));
                        });
                    }
                });
        });
    });
}
