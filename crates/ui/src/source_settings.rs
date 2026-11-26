//! Home › Data source settings (Power Query's *Data source settings*, Microsoft Learn, *Privacy
//! levels in Power Query*): the button runs `source.settings`; the dialog lists the model's data
//! sources and those the session holds a setting for, each with its credential kind and privacy
//! level, both as an icon and a word. Edit Permissions opens a source's credential (web sources:
//! anonymous, basic, Web API key, bearer token; SQL Server: a login) and privacy level; OK runs
//! `source.credentials` and `source.privacy`, Clear Permissions runs `source.credentials {clear}`
//! and `source.privacy {clear}`, then the list is read again. Secrets are emptied from the form
//! once sent. Privacy levels are recorded, not enforced (the dialog says so).

use egui::{RichText, Ui};
use serde_json::{Value, json};

use crate::App;
use crate::actions::Call;
use crate::app::{Then, clip};
use crate::get_data::{Secret, WebAuth, WebMethod};
use crate::i18n::Lang;
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::widgets;

pub const LIST: &str = "source.settings";
pub const CREDENTIALS: &str = "source.credentials";
pub const PRIVACY: &str = "source.privacy";
/// The commands the dialog runs (the coverage test reads it).
pub const COMMANDS: &[&str] = &[LIST, CREDENTIALS, PRIVACY];
/// Most sources listed.
const MAX_LISTED: usize = 500;
/// The privacy levels, as `source.privacy` names them.
pub const LEVELS: [&str; 4] = ["none", "public", "organizational", "private"];

/// A data source of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub kind: String,
    pub source: String,
    pub functions: Vec<String>,
    pub credential: Option<String>,
    pub privacy: Option<String>,
}

/// Edit Permissions of one source.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Edit {
    pub row: usize,
    pub auth: WebAuth,
    /// A SQL Server login.
    pub sql_user: String,
    pub sql_password: Secret,
    /// The level picked (`None`: not set).
    pub privacy: Option<&'static str>,
}

/// The dialog while it is open (pure view state).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dialog {
    /// `None` while the list is read.
    pub rows: Option<Vec<Row>>,
    pub selected: Option<usize>,
    pub edit: Option<Edit>,
    pub error: Option<String>,
    /// Commands sent whose answers are awaited.
    pub pending: usize,
}

/// What an answer is for ([`Then::SourceSettings`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    List,
    Saved,
}

/// The button (or the palette) opens the dialog.
pub fn intercept(app: &mut App, id: &str, _: &Value) -> bool {
    if id != LIST {
        return false;
    }
    open(app);
    true
}

pub fn open(app: &mut App) {
    app.cmd.source_settings = Some(Dialog::default());
    list(app);
}

fn list(app: &mut App) {
    if let Some(d) = app.cmd.source_settings.as_mut() {
        d.pending = d.pending.saturating_add(1);
    }
    app.run(
        Call {
            id: LIST,
            params: json!({}),
        },
        Then::SourceSettings(Step::List),
    );
}

fn text(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}

/// The answer of one of the dialog's commands.
pub fn answer(app: &mut App, step: Step, r: Result<Value, String>) {
    let Some(d) = app.cmd.source_settings.as_mut() else {
        return;
    };
    d.pending = d.pending.saturating_sub(1);
    match (step, r) {
        (_, Err(e)) => {
            d.error = Some(clip(&e, 600));
            if d.rows.is_none() {
                d.rows = Some(Vec::new());
            }
        }
        (Step::List, Ok(v)) => {
            let rows: Vec<Row> = v
                .get("sources")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(MAX_LISTED)
                .map(|s| Row {
                    kind: text(s, "kind").unwrap_or_default(),
                    source: text(s, "source").unwrap_or_default(),
                    functions: s
                        .get("functions")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect(),
                    credential: text(s, "credential"),
                    privacy: text(s, "privacy"),
                })
                .collect();
            if d.selected.is_some_and(|i| i >= rows.len()) {
                d.selected = None;
            }
            d.rows = Some(rows);
        }
        (Step::Saved, Ok(_)) => {
            d.error = None;
            list(app);
        }
    }
}

/// A credential kind in the window's language.
pub fn credential_text(c: Option<&str>, l: Lang) -> &'static str {
    match c {
        None => l.t("Not set", "Non défini"),
        Some("anonymous") => l.t("Anonymous", "Anonyme"),
        Some("basic") => l.t("Basic", "De base"),
        Some("key") => l.t("Web API key", "Clé d'API web"),
        Some("bearer") => l.t("Access token", "Jeton d'accès"),
        Some("database") => l.t("Database", "Base de données"),
        Some(_) => l.t("Other", "Autre"),
    }
}

/// A privacy level in the window's language.
pub fn privacy_text(p: Option<&str>, l: Lang) -> &'static str {
    match p {
        None => l.t("Not set", "Non défini"),
        Some("none") => l.t("None", "Aucun"),
        Some("public") => l.t("Public", "Public"),
        Some("organizational") => l.t("Organizational", "Organisationnel"),
        Some("private") => l.t("Private", "Privé"),
        Some(_) => l.t("Other", "Autre"),
    }
}

fn kind_text(kind: &str, l: Lang) -> &'static str {
    match kind {
        "web" => "Web",
        "sqlserver" => "SQL Server",
        _ => l.t("File", "Fichier"),
    }
}

fn kind_icon(kind: &str) -> Icon {
    match kind {
        "web" => Icon::Language,
        "sqlserver" => Icon::Server,
        _ => Icon::File,
    }
}

enum Act {
    Edit(usize),
    Clear(usize),
    Ok,
    Cancel,
}

/// Draws the open dialog.
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut d) = app.cmd.source_settings.clone() else {
        return;
    };
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let mut open = true;
    let mut close = false;
    let mut acts = Vec::new();
    egui::Window::new(l.t("Data source settings", "Paramètres de source de données"))
        .id(egui::Id::new("bicraft-source-settings"))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width(640.0);
            body(ui, &mut d, l, &t, &mut acts, &mut close);
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    app.cmd.source_settings = (open && !close).then_some(d);
    for a in acts {
        match a {
            Act::Edit(i) => start_edit(app, i),
            Act::Clear(i) => clear(app, i),
            Act::Ok => save(app),
            Act::Cancel => {
                if let Some(d) = app.cmd.source_settings.as_mut() {
                    d.edit = None;
                }
            }
        }
    }
}

fn body(ui: &mut Ui, d: &mut Dialog, l: Lang, t: &Tokens, acts: &mut Vec<Act>, close: &mut bool) {
    ui.label(RichText::new(l.t(
        "Manage the credentials and privacy levels of the data sources this file uses. They are kept in memory for this session only, never in the file.",
        "Gérez les informations d'identification et les niveaux de confidentialité des sources de données de ce fichier. Ils ne sont gardés qu'en mémoire, pour cette session, jamais dans le fichier.",
    ))
    .color(t.text_dim));
    ui.add_space(6.0);
    let Some(rows) = d.rows.clone() else {
        ui.spinner();
        return;
    };
    if rows.is_empty() {
        ui.label(l.t(
            "The model names no data source with a literal address, server or path.",
            "Le modèle ne nomme aucune source de données par une adresse, un serveur ou un chemin littéral.",
        ));
    }
    egui::ScrollArea::vertical()
        .id_salt("bicraft-source-settings-list")
        .max_height(200.0)
        .show(ui, |ui| {
            egui::Grid::new("bicraft-source-settings-grid")
                .num_columns(3)
                .striped(true)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    ui.label(RichText::new(l.t("Data source", "Source de données")).font(theme::bold(13.0)));
                    ui.label(RichText::new(l.t("Credentials", "Identification")).font(theme::bold(13.0)));
                    ui.label(
                        RichText::new(l.t("Privacy level", "Niveau de confidentialité"))
                            .font(theme::bold(13.0)),
                    );
                    ui.end_row();
                    for (i, r) in rows.iter().enumerate() {
                        let on = d.selected == Some(i);
                        let resp = ui
                            .horizontal(|ui| {
                                let (rect, _) =
                                    ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                                icons::paint(ui.painter(), rect, kind_icon(&r.kind), t.text, false);
                                ui.selectable_label(
                                    on,
                                    format!("{} · {}", kind_text(&r.kind, l), clip(&r.source, 56)),
                                )
                            })
                            .inner
                            .on_hover_text(if r.functions.is_empty() {
                                r.source.clone()
                            } else {
                                format!("{}\n{}", r.source, r.functions.join(", "))
                            });
                        if resp.clicked() {
                            d.selected = Some(i);
                        }
                        if resp.double_clicked() {
                            acts.push(Act::Edit(i));
                        }
                        ui.horizontal(|ui| {
                            let (rect, _) =
                                ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                            if r.credential.is_some() {
                                icons::paint(ui.painter(), rect, Icon::Lock, t.text, false);
                            }
                            ui.label(credential_text(r.credential.as_deref(), l));
                        });
                        ui.label(privacy_text(r.privacy.as_deref(), l));
                        ui.end_row();
                    }
                });
        });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let sel = d.selected.filter(|_| d.edit.is_none());
        if widgets::primary(
            ui,
            l.t("Edit Permissions…", "Modifier les autorisations…"),
            sel.is_some(),
            "",
        )
        .clicked()
            && let Some(i) = sel
        {
            acts.push(Act::Edit(i));
        }
        let clearable = sel
            .and_then(|i| rows.get(i))
            .is_some_and(|r| r.credential.is_some() || r.privacy.is_some());
        if ui
            .add_enabled(
                clearable,
                egui::Button::new(l.t("Clear Permissions", "Effacer les autorisations")),
            )
            .clicked()
            && let Some(i) = sel
        {
            acts.push(Act::Clear(i));
        }
        if d.pending > 0 {
            ui.spinner();
        }
    });
    if let Some(e) = d.edit.as_mut()
        && let Some(r) = rows.get(e.row)
    {
        ui.separator();
        edit_ui(ui, e, r, l, t, acts);
    }
    if let Some(e) = &d.error {
        ui.add_space(4.0);
        ui.label(RichText::new(format!("⚠ {e}")).color(t.warn));
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button(l.t("Close", "Fermer")).clicked() {
            *close = true;
        }
    });
}

fn edit_ui(ui: &mut Ui, e: &mut Edit, r: &Row, l: Lang, t: &Tokens, acts: &mut Vec<Act>) {
    ui.label(RichText::new(clip(&r.source, 100)).font(theme::bold(14.0)));
    ui.add_space(4.0);
    ui.label(RichText::new(l.t("Credentials", "Informations d'identification")).font(theme::bold(13.0)));
    match r.kind.as_str() {
        "web" => crate::get_data::auth_fields(ui, &mut e.auth, &r.source, "bicraft-source-settings", l, t),
        "sqlserver" => {
            ui.horizontal(|ui| {
                let lb = ui.add_sized(
                    [110.0, 20.0],
                    egui::Label::new(l.t("User name", "Nom d'utilisateur")),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut e.sql_user)
                        .char_limit(128)
                        .desired_width(330.0),
                )
                .labelled_by(lb.id);
            });
            ui.horizontal(|ui| {
                let lb = ui.add_sized([110.0, 20.0], egui::Label::new(l.t("Password", "Mot de passe")));
                ui.add(
                    egui::TextEdit::singleline(&mut e.sql_password.0)
                        .password(true)
                        .char_limit(128)
                        .desired_width(330.0),
                )
                .labelled_by(lb.id);
            });
            ui.label(
                RichText::new(l.t(
                    "Empty: the credential stays as it is.",
                    "Vide : l'identifiant reste tel quel.",
                ))
                .color(t.text_dim),
            );
        }
        _ => {
            ui.label(
                RichText::new(l.t(
                    "Files are read with this session's own access to them.",
                    "Les fichiers sont lus avec les droits de cette session.",
                ))
                .color(t.text_dim),
            );
        }
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let lb = ui.add_sized(
            [110.0, 20.0],
            egui::Label::new(l.t("Privacy level", "Niveau de confidentialité")),
        );
        egui::ComboBox::from_id_salt("bicraft-source-settings-privacy")
            .selected_text(privacy_text(e.privacy, l))
            .width(330.0)
            .show_ui(ui, |ui| {
                for lv in LEVELS {
                    ui.selectable_value(&mut e.privacy, Some(lv), privacy_text(Some(lv), l));
                }
            })
            .response
            .labelled_by(lb.id);
    });
    ui.label(RichText::new(l.t(
        "Recorded and shown: BIcraft does not apply privacy levels when a query combines sources.",
        "Enregistré et affiché : BIcraft n'applique pas les niveaux de confidentialité quand une requête combine des sources.",
    ))
    .color(t.text_dim));
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if widgets::primary(ui, "OK", true, "").clicked() {
            acts.push(Act::Ok);
        }
        if ui.button(l.t("Cancel", "Annuler")).clicked() {
            acts.push(Act::Cancel);
        }
    });
}

fn start_edit(app: &mut App, i: usize) {
    let Some(d) = app.cmd.source_settings.as_mut() else {
        return;
    };
    let Some(r) = d.rows.as_ref().and_then(|rows| rows.get(i)) else {
        return;
    };
    d.selected = Some(i);
    d.edit = Some(Edit {
        row: i,
        auth: WebAuth {
            method: WebMethod::from_name(r.credential.as_deref().unwrap_or("anonymous")),
            ..WebAuth::default()
        },
        privacy: LEVELS.into_iter().find(|lv| r.privacy.as_deref() == Some(*lv)),
        ..Edit::default()
    });
}

/// The `kind` the commands take for a row.
fn command_kind(kind: &str) -> &str {
    match kind {
        "web" | "sqlserver" => kind,
        _ => "file",
    }
}

fn send(app: &mut App, id: &'static str, params: Value) {
    if let Some(d) = app.cmd.source_settings.as_mut() {
        d.pending = d.pending.saturating_add(1);
    }
    app.run(Call { id, params }, Then::SourceSettings(Step::Saved));
}

/// OK: the credential (when one is typed or the kind changed) and the privacy level (when it
/// changed).
fn save(app: &mut App) {
    let Some(d) = app.cmd.source_settings.as_mut() else {
        return;
    };
    let Some(mut e) = d.edit.take() else { return };
    let Some(r) = d.rows.as_ref().and_then(|rows| rows.get(e.row)).cloned() else {
        return;
    };
    let mut calls = Vec::new();
    match r.kind.as_str() {
        "web" => {
            let was = WebMethod::from_name(r.credential.as_deref().unwrap_or("anonymous"));
            let typed = !e.auth.password.is_empty() || !e.auth.key.is_empty() || !e.auth.token.is_empty();
            if typed
                || e.auth.method != was
                || (r.credential.is_none() && e.auth.method == WebMethod::Anonymous)
            {
                calls.push((CREDENTIALS, e.auth.take_params(&r.source)));
            }
        }
        "sqlserver" if !e.sql_user.trim().is_empty() => {
            calls.push((
                CREDENTIALS,
                json!({
                    "kind": "sqlserver",
                    "server": r.source,
                    "user": e.sql_user.trim(),
                    "password": std::mem::take(&mut e.sql_password).0,
                }),
            ));
        }
        _ => {}
    }
    if e.privacy
        != r.privacy
            .as_deref()
            .and_then(|p| LEVELS.into_iter().find(|lv| *lv == p))
    {
        match e.privacy {
            Some(lv) => calls.push((
                PRIVACY,
                json!({"kind": command_kind(&r.kind), "source": r.source, "level": lv}),
            )),
            None => calls.push((
                PRIVACY,
                json!({"kind": command_kind(&r.kind), "source": r.source, "clear": true}),
            )),
        }
    }
    for (id, p) in calls {
        send(app, id, p);
    }
}

/// Clear Permissions: the source's credential and privacy level.
fn clear(app: &mut App, i: usize) {
    let Some(r) = app
        .cmd
        .source_settings
        .as_ref()
        .and_then(|d| d.rows.as_ref())
        .and_then(|rows| rows.get(i))
        .cloned()
    else {
        return;
    };
    if let Some(d) = app.cmd.source_settings.as_mut() {
        d.edit = None;
    }
    if r.credential.is_some() {
        let p = match r.kind.as_str() {
            "sqlserver" => json!({"kind": "sqlserver", "server": r.source, "clear": true}),
            _ => json!({"kind": "web", "url": r.source, "clear": true}),
        };
        send(app, CREDENTIALS, p);
    }
    if r.privacy.is_some() {
        send(
            app,
            PRIVACY,
            json!({"kind": command_kind(&r.kind), "source": r.source, "clear": true}),
        );
    }
}
