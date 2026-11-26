//! The parameters form of a command (ROADMAP M11.2, docs/method.md decision 7), built from its
//! `form` ([`crate::registry::Param`]): object names get a picker filled from the snapshot's model
//! and report, fields a `Table[Column]` / `[Measure]` picker, enums a dropdown, booleans a
//! checkbox, numbers a checked input, paths the desktop's file dialogs or the browser's workspace
//! files, DAX / M / code a monospace editor, colours a picker, lists and JSON a checked text
//! area. Required fields are marked and checked before Run; the command's own answer or error is
//! shown inline. A command the session's role or the server's policy refuses shows the reason and
//! no fields.
//!
//! The palette and the ribbon open it the same way: [`open`].

use std::sync::atomic::{AtomicU64, Ordering};

use egui::{Color32, Key, RichText, Vec2};
use serde_json::{Map, Value, json};

use crate::App;
use crate::actions::Call;
use crate::app::Then;
use crate::i18n::Lang;
use crate::registry::{self, Cmd, Kind, Obj, Param};
use crate::theme::Tokens;
use crate::widgets;

/// Most height of the answer shown under the fields.
const RESULT_H: f32 = 240.0;
/// Most entries a picker lists.
const MAX_OPTIONS: usize = 2000;

/// One field being filled.
#[derive(Debug, Clone)]
pub struct Input {
    pub field: &'static Param,
    /// The value as typed (every kind but booleans).
    pub text: String,
    /// A boolean's value; `None`: not set (the command's default).
    pub flag: Option<bool>,
}

/// Names a picker fetched with a read command when the form opened (environment profiles live
/// in a file next to the project, not in the snapshot): `env.list` through the document's source,
/// the same for a local file, a server and the browser.
#[derive(Debug, Clone)]
pub struct Lookup {
    pub obj: Obj,
    /// The tag of the run.
    pub tag: u64,
    /// `None` while it runs.
    pub names: Option<Result<Vec<String>, String>>,
}

/// An open form.
#[derive(Debug, Clone)]
pub struct Form {
    pub id: &'static str,
    pub inputs: Vec<Input>,
    /// More parameters as a JSON object (commands that take keys their form does not list, and
    /// prefilled keys outside the form).
    pub extra: Option<String>,
    /// Run was pressed: every problem is shown.
    pub tried: bool,
    /// The tag of the run waiting for its answer.
    pub running: Option<u64>,
    /// The last run's answer.
    pub result: Option<Result<Value, String>>,
    /// The answer read for display (grid, tree, JSON toggle).
    pub view: Option<crate::answer::Answer>,
    /// The parameters help is shown.
    pub help: bool,
    /// The values it was opened with (the selection's, the caller's).
    pub prefill: Value,
    /// Names fetched for its pickers ([`Lookup`]).
    pub lookups: Vec<Lookup>,
    serial: u64,
}

impl Form {
    pub fn spec(&self) -> Option<&'static Cmd> {
        registry::find(self.id)
    }

    /// The input of parameter `name`.
    pub fn input_mut(&mut self, name: &str) -> Option<&mut Input> {
        self.inputs.iter_mut().find(|i| i.field.name == name)
    }

    pub fn input(&self, name: &str) -> Option<&Input> {
        self.inputs.iter().find(|i| i.field.name == name)
    }
}

/// A value as the form's text for `kind`.
fn as_text(kind: &Kind, v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Array(a) if matches!(kind, Kind::List | Kind::Paths) && a.iter().all(Value::is_string) => {
            a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n")
        }
        Value::Array(_) | Value::Object(_) => serde_json::to_string_pretty(v).unwrap_or_default(),
        other => other.to_string(),
    }
}

/// Commands that take keys their form does not list: a JSON field is offered for them.
pub const FREE_KEYS: [&str; 2] = ["view.interact", "visual.format"];

/// Forms opened so far (their egui ids).
static SERIAL: AtomicU64 = AtomicU64::new(0);

/// A parameter's label: its name with spaces (`display_name` → `display name`).
pub fn field_label(name: &str) -> String {
    name.replace('_', " ")
}

/// Opens the form of command `cmd_id`, its fields filled from `prefill` (an object of parameter
/// values; keys outside the form go to its JSON field). Replaces an open form and closes the
/// palette. An unknown id is an error in the window.
pub fn open(app: &mut App, cmd_id: &str, prefill: Value) {
    let Some(spec) = registry::find(cmd_id) else {
        app.error = Some(format!(
            "{} `{}`",
            app.lang.t("Unknown command", "Commande inconnue"),
            crate::app::clip(cmd_id, 80)
        ));
        return;
    };
    let empty = Map::new();
    let pre = prefill.as_object().unwrap_or(&empty);
    let inputs = spec
        .form
        .iter()
        .map(|f| {
            let v = pre.get(f.name);
            Input {
                field: f,
                text: match (&f.kind, v) {
                    (Kind::Bool, _) | (_, None) => String::new(),
                    (k, Some(v)) => as_text(k, v),
                },
                flag: match (&f.kind, v) {
                    (Kind::Bool, Some(v)) => v.as_bool(),
                    _ => None,
                },
            }
        })
        .collect();
    let extras: Map<String, Value> = pre
        .iter()
        .filter(|(k, _)| !spec.form.iter().any(|f| f.name == k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let extra = if !extras.is_empty() {
        Some(serde_json::to_string_pretty(&Value::Object(extras)).unwrap_or_default())
    } else if FREE_KEYS.contains(&spec.id) {
        Some(String::new())
    } else {
        None
    };
    app.palette = None;
    app.form = Some(Form {
        id: spec.id,
        inputs,
        extra,
        tried: false,
        running: None,
        result: None,
        view: None,
        help: false,
        prefill: prefill.clone(),
        lookups: Vec::new(),
        serial: SERIAL.fetch_add(1, Ordering::Relaxed),
    });
    if spec.form.iter().any(|f| f.kind == Kind::Name(Obj::Profile)) {
        lookup(app, Obj::Profile, pre.get("env_file").cloned());
    }
    // the browser has no file dialog: its path fields list the workspace's files
    let paths = spec
        .form
        .iter()
        .any(|f| matches!(f.kind, Kind::Path | Kind::Paths | Kind::PathOrText));
    if paths && app.shell.pick_file.is_none() && app.workspace().is_some() && app.files.is_none() {
        app.list_files();
    }
}

/// Fetches the names of `obj` for the open form's pickers (only environment profiles: everything
/// else is in the snapshot). Skipped when the read command cannot run.
fn lookup(app: &mut App, obj: Obj, env_file: Option<Value>) {
    let id = match obj {
        Obj::Profile => "env.list",
        _ => return,
    };
    if crate::ribbon::state(app, id).is_err() {
        return;
    }
    let mut params = Map::new();
    if let Some(v) = env_file.filter(|v| !v.is_null()) {
        params.insert("env_file".into(), v);
    }
    let tag = app.next_tag();
    if app.run(
        Call {
            id,
            params: Value::Object(params),
        },
        Then::Lookup(obj),
    ) && let Some(f) = &mut app.form
    {
        f.lookups.push(Lookup {
            obj,
            tag,
            names: None,
        });
    }
}

/// Takes in the answer of a [`Lookup`] (`tag`): the `name`s of its list (`profiles`).
pub fn looked_up(app: &mut App, tag: u64, r: Result<Value, String>) {
    let Some(f) = &mut app.form else { return };
    let Some(l) = f.lookups.iter_mut().find(|l| l.tag == tag) else {
        return;
    };
    l.names = Some(r.map(|v| {
        v.get("profiles")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|p| p.get("name").and_then(Value::as_str).map(str::to_string))
            .take(MAX_OPTIONS)
            .collect()
    }));
}

/// The value the command uses when parameter `f` is left out (its declared default).
pub fn default_of(f: &Param) -> Option<Value> {
    f.default.and_then(|d| serde_json::from_str(d).ok())
}

// --- values ---------------------------------------------------------------------------------

/// The value of one input: `Ok(None)` when it is empty and optional, `Err` with what is wrong.
pub fn value(i: &Input, lang: Lang) -> Result<Option<Value>, String> {
    let f = i.field;
    if let Kind::Bool = f.kind {
        return match (i.flag, f.required) {
            (Some(b), _) => Ok(Some(json!(b))),
            (None, true) => Ok(Some(json!(false))),
            (None, false) => Ok(None),
        };
    }
    let raw = i.text.as_str();
    let t = raw.trim();
    if t.is_empty() {
        return if f.required {
            Err(lang.t("required", "obligatoire").to_string())
        } else {
            Ok(None)
        };
    }
    let v = match &f.kind {
        Kind::Text => json!(raw),
        Kind::Dax | Kind::M | Kind::Code | Kind::PathOrText => json!(raw),
        Kind::Name(_) | Kind::Field | Kind::Path => json!(t),
        Kind::Bool => json!(false),
        Kind::Integer => match t.parse::<i64>() {
            Ok(n) => json!(n),
            Err(_) => return Err(lang.t("a whole number", "un nombre entier").to_string()),
        },
        Kind::Number => match t.replace(',', ".").parse::<f64>() {
            Ok(n) if n.is_finite() => json!(n),
            _ => return Err(lang.t("a number", "un nombre").to_string()),
        },
        Kind::Enum(values) => {
            if values.contains(&t) {
                json!(t)
            } else {
                return Err(format!(
                    "{} {}",
                    lang.t("one of", "une valeur parmi"),
                    values.join(", ")
                ));
            }
        }
        Kind::Color => {
            if color(t).is_some() {
                json!(t)
            } else {
                return Err(lang
                    .t("a colour as #RRGGBB", "une couleur au format #RRVVBB")
                    .to_string());
            }
        }
        Kind::List => {
            if t.starts_with('[') {
                match serde_json::from_str::<Value>(t) {
                    Ok(v @ Value::Array(_)) => v,
                    _ => return Err(lang.t("a JSON list", "une liste JSON").to_string()),
                }
            } else {
                Value::Array(
                    t.split(['\n', ','])
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(|s| json!(s))
                        .collect(),
                )
            }
        }
        Kind::Paths => {
            if t.starts_with('[') || t.starts_with('{') {
                serde_json::from_str::<Value>(t)
                    .map_err(|e| format!("{}: {e}", lang.t("not valid JSON", "JSON non valide")))?
            } else {
                let lines: Vec<&str> = t.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
                match lines.as_slice() {
                    [one] => json!(one),
                    many => json!(many),
                }
            }
        }
        Kind::Json => match serde_json::from_str::<Value>(t) {
            Ok(v) => v,
            Err(e) if t.starts_with('{') || t.starts_with('[') => {
                return Err(format!("{}: {e}", lang.t("not valid JSON", "JSON non valide")));
            }
            // a bare word: the shapes that take a name or a keyword
            Err(_) => json!(t),
        },
    };
    Ok(Some(v))
}

/// `#RRGGBB` (or `#RRGGBBAA`) as a colour.
pub fn color(s: &str) -> Option<Color32> {
    let h = s.strip_prefix('#')?;
    if !(h.len() == 6 || h.len() == 8) || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let b = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok());
    Some(Color32::from_rgb(b(0)?, b(2)?, b(4)?))
}

/// The parameters the form gives, or the problems: (input index, what is wrong); the JSON field
/// is index `inputs.len()`.
pub fn params(form: &Form, lang: Lang) -> Result<Value, Vec<(usize, String)>> {
    let mut out = Map::new();
    let mut errs = Vec::new();
    if let Some(extra) = &form.extra
        && !extra.trim().is_empty()
    {
        match serde_json::from_str::<Value>(extra) {
            Ok(Value::Object(o)) => out.extend(o),
            _ => errs.push((
                form.inputs.len(),
                lang.t("a JSON object", "un objet JSON").to_string(),
            )),
        }
    }
    for (n, i) in form.inputs.iter().enumerate() {
        match value(i, lang) {
            Ok(Some(v)) => {
                out.insert(i.field.name.to_string(), v);
            }
            Ok(None) => {}
            Err(e) => errs.push((n, e)),
        }
    }
    if errs.is_empty() {
        Ok(Value::Object(out))
    } else {
        Err(errs)
    }
}

/// Runs the open form's command when its values are valid (else every problem is shown).
pub fn run(app: &mut App) {
    let l = app.lang;
    let Some(f) = &mut app.form else { return };
    let id = f.id;
    let p = match params(f, l) {
        Ok(p) => p,
        Err(_) => {
            f.tried = true;
            return;
        }
    };
    if crate::ribbon::state(app, id).is_err() {
        return;
    }
    let tag = app.next_tag();
    if app.run(Call { id, params: p }, Then::Form) {
        if let Some(f) = &mut app.form {
            f.running = Some(tag);
            f.result = None;
            f.view = None;
        }
        app.prefs.add_command(id);
        app.save_prefs();
    }
}

/// Takes in the answer of a form's run (`tag`); a form closed meanwhile sends its error to the
/// error bar.
pub fn answer(app: &mut App, tag: u64, r: Result<Value, String>) {
    match &mut app.form {
        Some(f) if f.running == Some(tag) => {
            f.running = None;
            f.view = r.as_ref().ok().map(crate::answer::Answer::new);
            f.result = Some(r);
        }
        _ => {
            if let Err(e) = r {
                app.error = Some(format!("✕ {}", crate::app::clip(&e, 600)));
            }
        }
    }
}

// --- pickers --------------------------------------------------------------------------------

/// The values a picker offers for `field` (value, label), from the snapshot and the form's other
/// values (the columns of the chosen table, the visuals of the chosen page).
pub fn options(app: &App, form: &Form, field: &Param) -> Vec<(String, String)> {
    let Some(snap) = app.loaded.as_ref().map(|l| l.snap.as_ref()) else {
        return Vec::new();
    };
    let arr = |v: Option<&Value>| -> Vec<Value> { v.and_then(Value::as_array).cloned().unwrap_or_default() };
    let name = |v: &Value| v.get("name").and_then(Value::as_str).unwrap_or("").to_string();
    let tables = arr(snap.pointer("/model/tables"));
    let pages = arr(snap.pointer("/report/pages"));
    let chosen = |k: &str| {
        form.input(k)
            .map(|i| i.text.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let mut out: Vec<(String, String)> = Vec::new();
    match &field.kind {
        Kind::Name(obj) => match obj {
            Obj::Table => out = tables.iter().map(|t| (name(t), name(t))).collect(),
            Obj::Column => {
                let only = chosen("table");
                for t in &tables {
                    let tn = name(t);
                    if only.as_ref().is_some_and(|o| *o != tn) {
                        continue;
                    }
                    for c in arr(t.get("columns")) {
                        let cn = name(&c);
                        out.push((cn.clone(), format!("{tn}[{cn}]")));
                    }
                }
            }
            Obj::Measure => {
                for t in &tables {
                    for m in arr(t.get("measures")) {
                        let mn = name(&m);
                        out.push((mn.clone(), format!("[{mn}] · {}", name(t))));
                    }
                }
            }
            Obj::Page => {
                out = pages
                    .iter()
                    .map(|p| {
                        let label = p
                            .get("display_name")
                            .and_then(Value::as_str)
                            .map_or_else(|| name(p), str::to_string);
                        (name(p), label)
                    })
                    .collect();
            }
            Obj::Visual => {
                let page =
                    chosen("page").or_else(|| app.page_names().get(app.active_page()).map(|p| p.0.clone()));
                for p in &pages {
                    if page.as_ref().is_some_and(|x| *x != name(p)) {
                        continue;
                    }
                    for v in arr(p.get("visuals")) {
                        let ty = v.get("type").and_then(Value::as_str).unwrap_or("group");
                        let title = v.get("title").and_then(Value::as_str).unwrap_or("");
                        let label = if title.is_empty() {
                            ty.to_string()
                        } else {
                            format!("{ty} · {title}")
                        };
                        out.push((name(&v), label));
                    }
                }
            }
            Obj::DataRole | Obj::VisualType => {
                let mut seen: Vec<String> = Vec::new();
                for p in &pages {
                    for v in arr(p.get("visuals")) {
                        let found: Vec<String> = if *obj == Obj::DataRole {
                            arr(v.get("fields"))
                                .iter()
                                .filter_map(|f| f.get("role").and_then(Value::as_str).map(str::to_string))
                                .collect()
                        } else {
                            v.get("type")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                                .into_iter()
                                .collect()
                        };
                        seen.extend(found);
                    }
                }
                let common: &[&str] = if *obj == Obj::DataRole {
                    &["Category", "Values", "Y", "Legend", "Rows", "Columns", "Tooltips"]
                } else {
                    &[
                        "card",
                        "clusteredColumnChart",
                        "clusteredBarChart",
                        "lineChart",
                        "pieChart",
                        "tableEx",
                        "pivotTable",
                        "slicer",
                        "textbox",
                    ]
                };
                seen.extend(common.iter().map(|s| s.to_string()));
                seen.sort();
                seen.dedup();
                out = seen.into_iter().map(|s| (s.clone(), s)).collect();
            }
            Obj::Hierarchy | Obj::Partition => {
                let key = if *obj == Obj::Hierarchy {
                    "hierarchies"
                } else {
                    "partitions"
                };
                let only = chosen("table");
                for t in &tables {
                    let tn = name(t);
                    if only.as_ref().is_some_and(|o| *o != tn) {
                        continue;
                    }
                    for h in strings(t.get(key)) {
                        out.push((h.clone(), format!("{h} · {tn}")));
                    }
                }
            }
            Obj::CalculationGroup => {
                out = tables
                    .iter()
                    .filter(|t| t.get("calculation_items").is_some())
                    .map(|t| (name(t), name(t)))
                    .collect();
            }
            Obj::CalculationItem => {
                let only = chosen("group");
                for t in tables.iter().filter(|t| t.get("calculation_items").is_some()) {
                    let tn = name(t);
                    if only.as_ref().is_some_and(|o| *o != tn) {
                        continue;
                    }
                    for i in strings(t.get("calculation_items")) {
                        out.push((i.clone(), format!("{i} · {tn}")));
                    }
                }
            }
            Obj::Expression | Obj::Role | Obj::Culture => {
                let key = match obj {
                    Obj::Expression => "/model/expressions",
                    Obj::Role => "/model/roles",
                    _ => "/model/cultures",
                };
                out = strings(snap.pointer(key))
                    .into_iter()
                    .map(|s| (s.clone(), s))
                    .collect();
            }
            Obj::Relationship => {
                for r in arr(snap.pointer("/model/relationships")) {
                    let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                    let label = format!(
                        "{}[{}] → {}[{}]",
                        s("from_table"),
                        s("from_column"),
                        s("to_table"),
                        s("to_column")
                    );
                    out.push((name(&r), label));
                }
            }
            Obj::ReportMeasure => {
                let only = chosen("table");
                for m in arr(snap.pointer("/report/measures")) {
                    let tn = m.get("table").and_then(Value::as_str).unwrap_or("").to_string();
                    if only.as_ref().is_some_and(|o| *o != tn) {
                        continue;
                    }
                    out.push((name(&m), format!("[{}] · {tn}", name(&m))));
                }
            }
            Obj::Profile => {
                if let Some(Ok(names)) = form
                    .lookups
                    .iter()
                    .find(|l| l.obj == Obj::Profile)
                    .and_then(|l| l.names.as_ref())
                {
                    out = names.iter().map(|n| (n.clone(), n.clone())).collect();
                }
            }
        },
        Kind::Field => {
            for t in &tables {
                let tn = name(t);
                for c in arr(t.get("columns")) {
                    let f = format!("{tn}[{}]", name(&c));
                    out.push((f.clone(), f));
                }
                for m in arr(t.get("measures")) {
                    let f = format!("[{}]", name(&m));
                    out.push((f.clone(), format!("{f} · {tn}")));
                }
            }
        }
        _ => {}
    }
    out.retain(|(v, _)| !v.is_empty());
    out.truncate(MAX_OPTIONS);
    out
}

/// The strings of a JSON list (others skipped).
fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

/// How a path parameter is picked on the desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathMode {
    Open,
    Save,
    Folder,
}

/// Whether path parameter `name` of command `id` names a file to read, one to write or a folder.
pub fn path_mode(id: &str, name: &str) -> PathMode {
    if name == "dir" || name.ends_with("_dir") || name == "data_root" || name.contains("folder") {
        return PathMode::Folder;
    }
    let writes = ["out", "to", "output", "target"].contains(&name)
        || (name == "path" && ["save", "export", "render"].iter().any(|w| id.contains(w)));
    if writes { PathMode::Save } else { PathMode::Open }
}

// --- drawing --------------------------------------------------------------------------------

/// What a frame of the form asks for, done once it is drawn.
enum Act {
    Run,
    Close,
    ListFiles,
}

/// Draws the open form (a window over the page; Ctrl+Enter runs, Escape closes).
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut form) = app.form.take() else {
        return;
    };
    let Some(spec) = form.spec() else { return };
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let state = crate::ribbon::state(app, form.id);
    let mut acts: Vec<Act> = Vec::new();
    let (esc, ctrl_enter) = ctx.input(|i| {
        (
            i.key_pressed(Key::Escape),
            i.modifiers.command && i.key_pressed(Key::Enter),
        )
    });
    if esc {
        acts.push(Act::Close);
    }
    if ctrl_enter && state.is_ok() {
        acts.push(Act::Run);
    }
    let errs = match params(&form, l) {
        Ok(_) => Vec::new(),
        Err(e) => e,
    };
    let title = spec.text(l).to_string();
    egui::Window::new(&title)
        .id(egui::Id::new(("bicraft-form", form.serial)))
        .collapsible(false)
        .resizable(false)
        .default_width(500.0)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::new(0.0, -20.0))
        .show(ctx, |ui| {
            ui.set_min_width(460.0);
            ui.label(
                RichText::new(format!("{} · {}", spec.place_path(l), spec.id))
                    .small()
                    .color(t.text_dim),
            );
            if spec.label != spec.button {
                ui.label(RichText::new(spec.label).color(t.text));
            }
            ui.add_space(6.0);
            if let Err(why) = &state {
                unavailable(ui, why, &t, l);
            } else {
                egui::ScrollArea::vertical()
                    .max_height(380.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| fields(app, ui, &mut form, &errs, &t, &mut acts));
                if !spec.params.is_empty() {
                    form.help =
                        widgets::section(ui, l.t("Parameters help", "Aide des paramètres"), form.help);
                    if form.help {
                        egui::ScrollArea::vertical()
                            .id_salt("help")
                            .max_height(140.0)
                            .show(ui, |ui| {
                                ui.label(RichText::new(spec.params).small().color(t.text_dim));
                            });
                    }
                }
            }
            result(ui, &mut form, &t, l);
            ui.add_space(8.0);
            let w = ui.available_width();
            ui.allocate_ui_with_layout(
                Vec2::new(w, 28.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if state.is_ok() {
                        let can = form.running.is_none();
                        let reason = l.t("Running…", "Exécution…");
                        if widgets::primary(ui, l.t("Run", "Exécuter"), can, reason).clicked() {
                            acts.push(Act::Run);
                        }
                    }
                    let close = if form.result.is_some() || state.is_err() {
                        l.t("Close", "Fermer")
                    } else {
                        l.t("Cancel", "Annuler")
                    };
                    if ui
                        .add(egui::Button::new(close).min_size(Vec2::new(72.0, 24.0)))
                        .clicked()
                    {
                        acts.push(Act::Close);
                    }
                    if form.tried && !errs.is_empty() {
                        ui.label(
                            RichText::new(format!(
                                "✕ {}",
                                l.t("Some values need fixing", "Des valeurs sont à corriger")
                            ))
                            .color(t.err_text),
                        );
                    }
                },
            );
        });
    app.form = Some(form);
    for a in acts {
        match a {
            Act::Run => run(app),
            Act::Close => {
                app.form = None;
                return;
            }
            Act::ListFiles => app.list_files(),
        }
    }
}

fn unavailable(ui: &mut egui::Ui, why: &str, t: &Tokens, l: Lang) {
    egui::Frame::new()
        .fill(t.err_soft)
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::new(16.0, 16.0), egui::Sense::hover());
                crate::icons::paint(ui.painter(), r, crate::icons::Icon::Warning, t.err, false);
                ui.label(
                    RichText::new(format!("{} {why}", l.t("Unavailable:", "Indisponible :")))
                        .color(t.err_text),
                );
            });
        });
}

/// The last run's answer: running, done with its result, or the command's error.
fn result(ui: &mut egui::Ui, form: &mut Form, t: &Tokens, l: Lang) {
    if form.running.is_some() {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(l.t("Running…", "Exécution…"));
        });
        return;
    }
    match &form.result {
        None => {}
        Some(Ok(v)) => {
            ui.add_space(6.0);
            ui.label(RichText::new(format!("✓ {}", l.t("Done", "Terminé"))).color(t.ok));
            let view = form.view.get_or_insert_with(|| crate::answer::Answer::new(v));
            if !view.is_empty() {
                crate::answer::show(
                    ui,
                    v,
                    view,
                    egui::Id::new(("form-answer", form.serial)),
                    RESULT_H,
                    l,
                );
            }
        }
        Some(Err(e)) => {
            ui.add_space(6.0);
            egui::Frame::new()
                .fill(t.err_soft)
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(egui::Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(format!(
                            "✕ {} {}",
                            l.t("Error:", "Erreur :"),
                            crate::app::clip(e, 2000)
                        ))
                        .color(t.err_text),
                    );
                });
        }
    }
}

/// The fields, one row each: the name (with `*` when required), the widget, the problem.
fn fields(
    app: &App,
    ui: &mut egui::Ui,
    form: &mut Form,
    errs: &[(usize, String)],
    t: &Tokens,
    acts: &mut Vec<Act>,
) {
    let l = app.lang;
    let id = form.id;
    let serial = form.serial;
    let n = form.inputs.len();
    let tried = form.tried;
    if n == 0 && form.extra.is_none() {
        ui.label(
            RichText::new(l.t(
                "This command takes no values.",
                "Cette commande ne prend aucune valeur.",
            ))
            .color(t.text_dim),
        );
    }
    for k in 0..n {
        let opts = match form.inputs.get(k) {
            Some(i) => options(app, form, i.field),
            None => continue,
        };
        let Some(input) = form.inputs.get_mut(k) else {
            continue;
        };
        let f = input.field;
        // a checkbox carries its own name
        let label = if f.kind == Kind::Bool {
            if f.required {
                "*".to_string()
            } else {
                String::new()
            }
        } else {
            format!("{}{}", field_label(f.name), if f.required { " *" } else { "" })
        };
        let salt = (serial, k);
        ui.horizontal_top(|ui| {
            let lab = ui.allocate_ui_with_layout(
                Vec2::new(130.0, 22.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_width(130.0);
                    let r = ui.label(&label);
                    if f.required {
                        r.on_hover_text(l.t("Required", "Obligatoire"))
                    } else {
                        r
                    }
                },
            );
            let lab_id = lab.inner.id;
            let pending = match f.kind {
                Kind::Name(o) => form
                    .lookups
                    .iter()
                    .find(|x| x.obj == o)
                    .and_then(|x| match &x.names {
                        None => Some(l.t("Listing…", "Liste en cours…").to_string()),
                        Some(Err(e)) => Some(crate::app::clip(e, 160)),
                        Some(Ok(_)) => None,
                    }),
                _ => None,
            };
            ui.vertical(|ui| {
                widget(app, ui, input, &opts, id, salt, lab_id, acts);
                if let Some(p) = &pending {
                    ui.label(RichText::new(p).small().color(t.text_dim));
                }
                let typed = !input.text.trim().is_empty();
                if let Some((_, e)) = errs.iter().find(|(i, _)| *i == k)
                    && (tried || typed)
                {
                    ui.label(RichText::new(format!("✕ {e}")).small().color(t.err_text));
                }
            });
        });
        ui.add_space(2.0);
    }
    if let Some(extra) = &mut form.extra {
        ui.horizontal_top(|ui| {
            let lab = ui.allocate_ui_with_layout(
                Vec2::new(130.0, 22.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_width(130.0);
                    ui.label(l.t("more (JSON)", "autres (JSON)"))
                },
            );
            ui.vertical(|ui| {
                ui.add(
                    egui::TextEdit::multiline(extra)
                        .code_editor()
                        .desired_rows(3)
                        .desired_width(300.0)
                        .hint_text("{\"key\": \"value\"}"),
                )
                .labelled_by(lab.inner.id);
                if let Some((_, e)) = errs.iter().find(|(i, _)| *i == n) {
                    ui.label(RichText::new(format!("✕ {e}")).small().color(t.err_text));
                }
            });
        });
    }
}

/// One input's widget.
#[allow(clippy::too_many_arguments)]
fn widget(
    app: &App,
    ui: &mut egui::Ui,
    input: &mut Input,
    opts: &[(String, String)],
    id: &str,
    salt: (u64, usize),
    label: egui::Id,
    acts: &mut Vec<Act>,
) {
    let l = app.lang;
    let f = input.field;
    let w = 300.0;
    match &f.kind {
        Kind::Bool => {
            // left out: the command's default, drawn as it (a dash when it has none: unchanged)
            let default = default_of(f).and_then(|v| v.as_bool());
            let unset = input.flag.is_none() && !f.required;
            let mut b = input.flag.or(default).unwrap_or(false);
            ui.horizontal(|ui| {
                let r = ui.add(
                    egui::Checkbox::new(&mut b, field_label(f.name))
                        .indeterminate(unset && default.is_none()),
                );
                if r.changed() {
                    input.flag = Some(b);
                }
                if unset {
                    let note = match default {
                        Some(_) => l.t("(default)", "(par défaut)"),
                        None => l.t("(not set)", "(non défini)"),
                    };
                    ui.label(RichText::new(note).small().weak());
                } else if !f.required
                    && ui
                        .small_button(l.t("Reset", "Réinitialiser"))
                        .on_hover_text(l.t(
                            "Leave it out: the command's default",
                            "Ne pas l'envoyer : la valeur par défaut de la commande",
                        ))
                        .clicked()
                {
                    input.flag = None;
                }
            });
        }
        Kind::Enum(values) => {
            let default = match default_of(f) {
                Some(Value::String(d)) => format!("{} {d}", l.t("(default)", "(par défaut)")),
                _ => l.t("(default)", "(par défaut)").to_string(),
            };
            let shown = if input.text.is_empty() {
                default.clone()
            } else {
                input.text.clone()
            };
            egui::ComboBox::from_id_salt(("enum", salt))
                .selected_text(shown)
                .width(w)
                .show_ui(ui, |ui| {
                    if !f.required {
                        ui.selectable_value(&mut input.text, String::new(), default);
                    }
                    for v in values.iter() {
                        ui.selectable_value(&mut input.text, (*v).to_string(), *v);
                    }
                })
                .response
                .labelled_by(label);
        }
        Kind::Name(Obj::Page | Obj::Visual) if !opts.is_empty() => {
            let shown = opts
                .iter()
                .find(|(v, _)| *v == input.text)
                .map(|(_, lab)| lab.clone())
                .unwrap_or_else(|| {
                    if input.text.is_empty() {
                        l.t("(choose)", "(choisir)").to_string()
                    } else {
                        input.text.clone()
                    }
                });
            egui::ComboBox::from_id_salt(("name", salt))
                .selected_text(crate::app::clip(&shown, 60))
                .width(w)
                .show_ui(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                        if !f.required {
                            ui.selectable_value(&mut input.text, String::new(), l.t("(none)", "(aucun)"));
                        }
                        for (v, lab) in opts {
                            ui.selectable_value(&mut input.text, v.clone(), crate::app::clip(lab, 80));
                        }
                    });
                })
                .response
                .labelled_by(label);
        }
        Kind::Dax | Kind::M | Kind::Code => {
            ui.add(
                egui::TextEdit::multiline(&mut input.text)
                    .code_editor()
                    .desired_rows(4)
                    .desired_width(w),
            )
            .labelled_by(label);
        }
        Kind::Json | Kind::List | Kind::Paths | Kind::PathOrText => {
            let hint = match f.kind {
                Kind::List => l.t("one per line, or a JSON list", "un par ligne, ou une liste JSON"),
                Kind::Json => "{ }",
                _ => "",
            };
            ui.add(
                egui::TextEdit::multiline(&mut input.text)
                    .code_editor()
                    .desired_rows(3)
                    .desired_width(w)
                    .hint_text(hint),
            )
            .labelled_by(label);
            if matches!(f.kind, Kind::Paths | Kind::PathOrText) {
                browse(app, ui, input, id, salt, acts);
            }
        }
        Kind::Color => {
            ui.horizontal(|ui| {
                let mut c = color(input.text.trim()).unwrap_or(Color32::BLACK);
                if ui.color_edit_button_srgba(&mut c).changed() {
                    input.text = format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b());
                }
                ui.add(
                    egui::TextEdit::singleline(&mut input.text)
                        .desired_width(w - 50.0)
                        .hint_text("#118DFF"),
                )
                .labelled_by(label);
            });
        }
        _ => {
            let declared = default_of(f).map(|d| match d {
                Value::String(s) => s,
                other => other.to_string(),
            });
            let hint = match (&declared, f.kind) {
                (Some(d), _) => d.as_str(),
                (None, Kind::Integer) => "0",
                (None, Kind::Number) => "0.0",
                (None, Kind::Field) => "Table[Column]",
                _ => "",
            };
            ui.add(
                egui::TextEdit::singleline(&mut input.text)
                    .desired_width(w)
                    .hint_text(hint),
            )
            .labelled_by(label);
            if !opts.is_empty() {
                egui::ComboBox::from_id_salt(("pick", salt))
                    .selected_text(l.t("Pick…", "Choisir…"))
                    .width(w)
                    .show_ui(ui, |ui| {
                        egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                            for (v, lab) in opts {
                                ui.selectable_value(&mut input.text, v.clone(), crate::app::clip(lab, 80));
                            }
                        });
                    });
            }
            if matches!(f.kind, Kind::Path) {
                browse(app, ui, input, id, salt, acts);
            }
        }
    }
}

/// A path's Browse: the desktop's Open, Save or folder dialog; the browser's workspace files.
fn browse(
    app: &App,
    ui: &mut egui::Ui,
    input: &mut Input,
    id: &str,
    salt: (u64, usize),
    acts: &mut Vec<Act>,
) {
    let l = app.lang;
    let mode = path_mode(id, input.field.name);
    let multi = matches!(input.field.kind, Kind::Paths);
    let mut put = |p: String| {
        if multi && !input.text.trim().is_empty() {
            input.text.push('\n');
            input.text.push_str(&p);
        } else {
            input.text = p;
        }
    };
    if app.shell.pick_file.is_some() {
        if ui.button(l.t("Browse…", "Parcourir…")).clicked() {
            let picked = match mode {
                PathMode::Open => app.shell.pick_file.as_ref().and_then(|p| p(l)),
                PathMode::Save => app
                    .shell
                    .pick_save
                    .as_ref()
                    .and_then(|p| p(l, l.t("Save as", "Enregistrer sous"), "", &[])),
                PathMode::Folder => app.shell.pick_folder.as_ref().and_then(|p| p(l)),
            };
            if let Some(p) = picked {
                put(p.display().to_string());
            }
        }
        return;
    }
    if app.workspace().is_none() {
        return;
    }
    match &app.files {
        Some(Ok(files)) => {
            egui::ComboBox::from_id_salt(("files", salt))
                .selected_text(l.t("Workspace files…", "Fichiers de l'espace…"))
                .width(300.0)
                .show_ui(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                        for f in files.iter().take(MAX_OPTIONS) {
                            if ui.selectable_label(false, &f.path).clicked() {
                                put(f.path.clone());
                            }
                        }
                    });
                });
        }
        Some(Err(e)) => {
            ui.label(RichText::new(crate::app::clip(e, 200)).small().weak());
            if ui
                .button(l.t("List the files again", "Relister les fichiers"))
                .clicked()
            {
                acts.push(Act::ListFiles);
            }
        }
        None => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(l.t("Listing the files…", "Liste des fichiers…")).small());
            });
        }
    }
}

#[cfg(test)]
#[path = "form_tests.rs"]
mod tests;
