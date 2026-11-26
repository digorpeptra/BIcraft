//! Home › Get data (G58; Power BI Desktop's Get data and Navigator): a dialog that first shows
//! the sources BIcraft's Power Query evaluator reads (an Excel workbook, a text or CSV file, a
//! JSON file, a folder, a SQL Server database; Enter data; a blank query through `table.import`'s
//! form), then, once a file is picked (or a server and database typed) and Connect runs
//! `source.navigate`, the Navigator: the source's items with a
//! check box each, a preview of the one clicked (`query.preview` on its query), and Load, which
//! runs `source.load` with the checked items (one undo step). Transform data opens the checked
//! items in the Power Query editor (`crate::pq_editor`) instead, as new queries.
//!
//! Web, OData feed, SharePoint folder and SharePoint list take an http(s) address and, optionally,
//! a credential (anonymous, basic, Web API key, bearer token) and the level (URL prefix) it
//! applies to: Connect first runs `source.credentials {kind: web, …}` when one is chosen (the
//! secrets are emptied once sent), then `source.navigate {kind: web | odata | sharepoint_folder |
//! sharepoint_list, url}`; previews and Load pass the answer's `allow_connectors` (`web`,
//! `odata`, `sharepoint`).
//!
//! A SQL Server database (lot 11) takes a server, a database, an optional SQL statement and a
//! SQL Server login: Connect first runs `source.credentials` with the login (held by the session
//! in memory only; the password field is emptied once sent and never kept in [`App::sent`]),
//! then `source.navigate`; previews and Load pass the answer's `allow_connectors`.
//!
//! The ribbon's Get data button, the palette's `source.navigate` and `source.load` without
//! their values open the dialog ([`intercept`]); programmatic calls never do.

use std::collections::BTreeSet;

use egui::{RichText, Ui};
use serde_json::{Value, json};

use crate::App;
use crate::actions::Call;
use crate::app::{Then, clip};
use crate::i18n::Lang;
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::widgets;

pub const NAVIGATE: &str = "source.navigate";
pub const LOAD: &str = "source.load";
pub const CREDENTIALS: &str = "source.credentials";
/// The commands the dialog runs (the coverage test reads it).
pub const COMMANDS: &[&str] = &[
    NAVIGATE,
    LOAD,
    CREDENTIALS,
    "query.preview",
    "table.import",
    "table.enter_data",
];
/// Rows a preview shows.
const PREVIEW_ROWS: usize = 50;
/// Most items listed.
const MAX_LISTED: usize = 1000;
/// Longest text drawn in a preview cell (characters).
const MAX_CELL: usize = 60;

/// A source of the gallery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Excel,
    Csv,
    Json,
    Folder,
    Sql,
    Web,
    OData,
    SharePointFolder,
    SharePointList,
}

impl Source {
    pub const ALL: [Source; 9] = [
        Source::Excel,
        Source::Csv,
        Source::Json,
        Source::Folder,
        Source::Sql,
        Source::Web,
        Source::OData,
        Source::SharePointFolder,
        Source::SharePointList,
    ];

    /// Whether the source is read from a URL.
    fn is_url(self) -> bool {
        matches!(
            self,
            Source::Web | Source::OData | Source::SharePointFolder | Source::SharePointList
        )
    }

    /// The `kind` of `source.navigate`.
    pub fn kind(self) -> &'static str {
        match self {
            Source::Excel => "excel",
            Source::Csv => "csv",
            Source::Json => "json",
            Source::Folder => "folder",
            Source::Sql => "sqlserver",
            Source::Web => "web",
            Source::OData => "odata",
            Source::SharePointFolder => "sharepoint_folder",
            Source::SharePointList => "sharepoint_list",
        }
    }

    pub fn text(self, l: Lang) -> &'static str {
        match self {
            Source::Excel => l.t("Excel workbook", "Classeur Excel"),
            Source::Csv => l.t("Text/CSV", "Texte/CSV"),
            Source::Json => "JSON",
            Source::Folder => l.t("Folder", "Dossier"),
            Source::Sql => l.t("SQL Server database", "Base de données SQL Server"),
            Source::Web => "Web",
            Source::OData => l.t("OData feed", "Flux OData"),
            Source::SharePointFolder => l.t("SharePoint folder", "Dossier SharePoint"),
            Source::SharePointList => l.t("SharePoint list", "Liste SharePoint"),
        }
    }

    fn about(self, l: Lang) -> &'static str {
        match self {
            Source::Excel => l.t(
                "The sheets, tables and named ranges of an .xlsx or .xlsm file.",
                "Les feuilles, tableaux et plages nommées d'un fichier .xlsx ou .xlsm.",
            ),
            Source::Csv => l.t(
                "A delimited text file (.csv, .tsv, .txt): its delimiter, headers and types are detected.",
                "Un fichier texte délimité (.csv, .tsv, .txt) : son séparateur, ses en-têtes et ses types sont détectés.",
            ),
            Source::Json => l.t(
                "A JSON file: its list of records, or each list and record it holds.",
                "Un fichier JSON : sa liste d'enregistrements, ou chaque liste et enregistrement qu'il contient.",
            ),
            Source::Folder => l.t(
                "The list of a folder's files, and its CSV files appended into one table.",
                "La liste des fichiers d'un dossier, et ses fichiers CSV ajoutés en une table.",
            ),
            Source::Sql => l.t(
                "The tables and views of a SQL Server or Azure SQL database, or the result of a SQL statement. The login is kept in memory for this session only, never in the file.",
                "Les tables et vues d'une base SQL Server ou Azure SQL, ou le résultat d'une instruction SQL. L'identifiant n'est gardé qu'en mémoire, pour cette session, jamais dans le fichier.",
            ),
            Source::Web => l.t(
                "A JSON document or a delimited text file at an http or https address. Web pages (HTML) are not read.",
                "Un document JSON ou un fichier texte délimité à une adresse http ou https. Les pages web (HTML) ne sont pas lues.",
            ),
            Source::OData => l.t(
                "The entity sets of an OData service (JSON, OData v4); column choices, filters, sorts and first rows are sent to the service.",
                "Les ensembles d'entités d'un service OData (JSON, OData v4) ; les choix de colonnes, filtres, tris et premières lignes sont transmis au service.",
            ),
            Source::SharePointFolder => l.t(
                "Every file of a SharePoint site's document libraries, subfolders included, and its CSV files appended. Type the site's root address.",
                "Tous les fichiers des bibliothèques de documents d'un site SharePoint, sous-dossiers compris, et ses fichiers CSV ajoutés. Saisissez l'adresse racine du site.",
            ),
            Source::SharePointList => l.t(
                "The lists of a SharePoint site and their items. Type the site's root address, not a list's.",
                "Les listes d'un site SharePoint et leurs éléments. Saisissez l'adresse racine du site, pas celle d'une liste.",
            ),
        }
    }

    fn icon(self) -> Icon {
        match self {
            Source::Folder => Icon::Folder,
            Source::Sql => Icon::Server,
            Source::Web => Icon::Language,
            Source::OData => Icon::Link,
            Source::SharePointFolder => Icon::Folder,
            Source::Json => Icon::File,
            _ => Icon::Table,
        }
    }

    fn extensions(self) -> Option<&'static [&'static str]> {
        match self {
            Source::Excel => Some(&["xlsx", "xlsm"]),
            Source::Csv => Some(&["csv", "tsv", "txt", "prn"]),
            Source::Json => Some(&["json"]),
            Source::Folder
            | Source::Sql
            | Source::Web
            | Source::OData
            | Source::SharePointFolder
            | Source::SharePointList => None,
        }
    }
}

/// One item of the Navigator.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub name: String,
    pub kind: String,
    pub m: String,
    pub hidden: bool,
    pub error: Option<String>,
}

/// A preview read once.
#[derive(Debug, Clone, PartialEq)]
pub struct Preview {
    pub columns: Vec<(String, String)>,
    pub rows: Vec<Vec<Value>>,
    pub truncated: bool,
}

/// The Navigator: what `source.navigate` answered.
#[derive(Debug, Clone, PartialEq)]
pub struct Nav {
    pub path: String,
    pub data_root: String,
    pub items: Vec<Item>,
    pub checked: BTreeSet<usize>,
    pub selected: Option<usize>,
    pub preview: Option<Result<Preview, String>>,
    pub search: String,
    /// The connectors the source's queries need (`allow_connectors` of the answer).
    pub connectors: Vec<String>,
}

/// A password typed in the dialog: never printed, emptied once sent.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(pub(crate) String);

impl Secret {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_empty() {
            "Secret(empty)"
        } else {
            "Secret(…)"
        })
    }
}

/// A SQL Server database's connection, typed in the gallery.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SqlForm {
    pub server: String,
    pub database: String,
    /// A SQL statement (Desktop's advanced option); empty: the database's tables and views.
    pub query: String,
    /// A SQL Server login; empty: the session's credentials for the server, else the
    /// environment's.
    pub user: String,
    pub password: Secret,
}

/// How a web source authenticates (`source.credentials {kind: web, authentication}`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WebMethod {
    /// Nothing is stored: the credential the session already holds for the URL, else anonymous.
    #[default]
    Anonymous,
    Basic,
    Key,
    Bearer,
}

impl WebMethod {
    pub const ALL: [WebMethod; 4] = [
        WebMethod::Anonymous,
        WebMethod::Basic,
        WebMethod::Key,
        WebMethod::Bearer,
    ];

    pub fn name(self) -> &'static str {
        match self {
            WebMethod::Anonymous => "anonymous",
            WebMethod::Basic => "basic",
            WebMethod::Key => "key",
            WebMethod::Bearer => "bearer",
        }
    }

    pub fn from_name(n: &str) -> Self {
        Self::ALL.into_iter().find(|m| m.name() == n).unwrap_or_default()
    }

    pub fn text(self, l: Lang) -> &'static str {
        match self {
            WebMethod::Anonymous => l.t("Anonymous", "Anonyme"),
            WebMethod::Basic => l.t("Basic", "De base"),
            WebMethod::Key => l.t("Web API key", "Clé d'API web"),
            WebMethod::Bearer => l.t(
                "Organizational account (access token)",
                "Compte professionnel (jeton d'accès)",
            ),
        }
    }
}

/// A web source's credential, typed in the dialog.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WebAuth {
    pub method: WebMethod,
    pub user: String,
    pub password: Secret,
    pub key: Secret,
    /// The header a key is sent in (empty: the query's ApiKeyName).
    pub header: String,
    pub token: Secret,
    /// The URL prefix the credential applies to; empty: the address typed.
    pub level: String,
}

impl WebAuth {
    /// `source.credentials`'s parameters for `url`, the secrets taken out of the form.
    pub fn take_params(&mut self, url: &str) -> Value {
        let level = if self.level.trim().is_empty() {
            url.to_string()
        } else {
            self.level.trim().to_string()
        };
        let mut p = json!({"kind": "web", "url": level, "authentication": self.method.name()});
        match self.method {
            WebMethod::Anonymous => {}
            WebMethod::Basic => {
                p["user"] = json!(self.user.trim());
                p["password"] = json!(std::mem::take(&mut self.password).0);
            }
            WebMethod::Key => {
                p["key"] = json!(std::mem::take(&mut self.key).0);
                if !self.header.trim().is_empty() {
                    p["header"] = json!(self.header.trim());
                }
            }
            WebMethod::Bearer => p["token"] = json!(std::mem::take(&mut self.token).0),
        }
        p
    }
}

/// The levels a credential may apply to: the URL's origin, then each longer path, up to the
/// URL itself (Desktop's "Select which level to apply these settings to").
pub fn levels(url: &str) -> Vec<String> {
    let url = url.trim();
    let path = url.split(['?', '#']).next().unwrap_or(url).trim_end_matches('/');
    let Some(i) = path.find("://") else {
        return Vec::new();
    };
    let start = i + 3;
    let mut out = Vec::new();
    let rest = path.get(start..).unwrap_or("");
    let host_end = rest.find('/').map_or(path.len(), |j| start + j);
    out.push(path.get(..host_end).unwrap_or(path).to_string());
    let mut at = host_end;
    while let Some(j) = path.get(at + 1..).and_then(|r| r.find('/')) {
        at = at + 1 + j;
        out.push(path.get(..at).unwrap_or(path).to_string());
    }
    if out.last().map(String::as_str) != Some(path) {
        out.push(path.to_string());
    }
    out.truncate(32);
    out
}

/// The open dialog's state (pure view state).
#[derive(Debug, Clone, PartialEq)]
pub struct GetData {
    pub source: Source,
    pub path: String,
    /// A text file's delimiter; empty: detected.
    pub delimiter: String,
    pub sql: SqlForm,
    /// The address of a Web, OData or SharePoint source.
    pub url: String,
    /// The credential of a Web, OData or SharePoint source.
    pub auth: WebAuth,
    pub nav: Option<Nav>,
    /// A command of the dialog is running.
    pub busy: bool,
    pub problem: Option<String>,
    /// Bumped by every run: older answers are dropped.
    pub generation: u64,
}

/// What an answer of the dialog's commands is for ([`Then::GetData`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// A SQL Server login stored; `source.navigate` follows.
    Credentials(u64),
    Navigate(u64),
    Preview(u64, usize),
    Load(u64),
    /// Edit data's read of the table (`crate::enter_data`).
    Read(u64),
    /// The grid's Load (`crate::enter_data`).
    Enter(u64),
}

/// Opens the gallery.
pub fn open(app: &mut App) {
    open_on(app, Source::Excel);
}

fn open_on(app: &mut App, source: Source) {
    app.cmd.get_data = Some(GetData {
        source,
        path: String::new(),
        delimiter: String::new(),
        sql: SqlForm::default(),
        url: String::new(),
        auth: WebAuth::default(),
        nav: None,
        busy: false,
        problem: None,
        generation: 0,
    });
}

/// Get data and Load open the dialog when run without their values (a ribbon or palette click);
/// the palette's Data source credentials without a server opens it on SQL Server database.
pub fn intercept(app: &mut App, id: &str, params: &Value) -> bool {
    let source = match id {
        NAVIGATE if params.get("path").is_none() && params.get("server").is_none() => Source::Excel,
        LOAD if params.get("tables").is_none() => Source::Excel,
        CREDENTIALS if params.get("server").is_none() && params.get("url").is_none() => Source::Sql,
        _ => return false,
    };
    open_on(app, source);
    true
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// The answer of one of the dialog's commands.
pub fn answer(app: &mut App, step: Step, r: Result<Value, String>) {
    if matches!(step, Step::Read(_) | Step::Enter(_)) {
        crate::enter_data::answer(app, step, r);
        return;
    }
    let Some(d) = app.cmd.get_data.as_mut() else {
        return;
    };
    match step {
        Step::Credentials(g) if g == d.generation => match r {
            Ok(_) => navigate(app),
            Err(e) => {
                d.busy = false;
                d.problem = Some(clip(&e, 600));
            }
        },
        Step::Navigate(g) if g == d.generation => {
            d.busy = false;
            match r {
                Ok(v) => {
                    let items: Vec<Item> = v
                        .get("items")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .take(MAX_LISTED)
                        .map(|i| Item {
                            name: s(i, "name"),
                            kind: s(i, "kind"),
                            m: s(i, "m"),
                            hidden: i.get("hidden").and_then(Value::as_bool).unwrap_or(false),
                            error: i.get("error").and_then(Value::as_str).map(str::to_string),
                        })
                        .collect();
                    // a source of one item comes checked and shown, as Desktop's text files
                    let one = items.len() == 1 && items.first().is_some_and(|i| i.error.is_none());
                    let path = match (v.get("server"), v.get("url")) {
                        (Some(_), _) => format!("{} › {}", s(&v, "server"), s(&v, "database")),
                        (None, Some(_)) => s(&v, "url"),
                        _ => s(&v, "path"),
                    };
                    let connectors = v
                        .get("allow_connectors")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect();
                    d.nav = Some(Nav {
                        path,
                        connectors,
                        data_root: s(&v, "data_root"),
                        checked: if one { BTreeSet::from([0]) } else { BTreeSet::new() },
                        selected: None,
                        items,
                        preview: None,
                        search: String::new(),
                    });
                    d.problem = None;
                    if one {
                        select(app, 0);
                    }
                }
                Err(e) => d.problem = Some(clip(&e, 600)),
            }
        }
        Step::Preview(g, i) if g == d.generation => {
            if let Some(n) = d.nav.as_mut()
                && n.selected == Some(i)
            {
                n.preview = Some(r.map(|v| preview_of(&v, PREVIEW_ROWS)));
            }
        }
        Step::Load(g) if g == d.generation => {
            d.busy = false;
            match r {
                Ok(_) => app.cmd.get_data = None,
                Err(e) => d.problem = Some(clip(&e, 600)),
            }
        }
        _ => {}
    }
}

/// A `query.preview` answer, its first `max` rows.
pub fn preview_of(v: &Value, max: usize) -> Preview {
    let columns = v
        .get("columns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|c| (s(c, "name"), s(c, "data_type")))
        .collect();
    let rows = v
        .get("rows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(max)
        .map(|r| r.as_array().cloned().unwrap_or_default())
        .collect();
    Preview {
        columns,
        rows,
        truncated: v.get("truncated").and_then(Value::as_bool).unwrap_or(false),
    }
}

/// Connect: `source.navigate` on the path typed or picked.
pub fn connect(app: &mut App) {
    let Some(d) = app.cmd.get_data.as_mut() else {
        return;
    };
    if d.source == Source::Sql {
        connect_sql(app);
        return;
    }
    if d.source.is_url() {
        connect_url(app);
        return;
    }
    let path = d.path.trim().to_string();
    if path.is_empty() {
        d.problem = Some(
            app.lang
                .t(
                    "Type or pick the path of the source.",
                    "Saisissez ou choisissez le chemin de la source.",
                )
                .into(),
        );
        return;
    }
    d.generation = d.generation.wrapping_add(1);
    d.busy = true;
    d.problem = None;
    let mut p = json!({"path": path, "kind": d.source.kind()});
    if d.source == Source::Csv && !d.delimiter.is_empty() {
        p["delimiter"] = json!(d.delimiter);
    }
    let g = d.generation;
    app.run(
        Call {
            id: NAVIGATE,
            params: p,
        },
        Then::GetData(Step::Navigate(g)),
    );
}

/// Connect on a Web or OData source: `source.navigate {kind, url}`.
fn connect_url(app: &mut App) {
    let l = app.lang;
    let Some(d) = app.cmd.get_data.as_mut() else {
        return;
    };
    let url = d.url.trim().to_string();
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        d.problem = Some(
            l.t(
                "Type an address that starts with http:// or https://.",
                "Saisissez une adresse qui commence par http:// ou https://.",
            )
            .into(),
        );
        return;
    }
    if d.auth.method == WebMethod::Anonymous {
        navigate(app);
        return;
    }
    d.generation = d.generation.wrapping_add(1);
    d.busy = true;
    d.problem = None;
    let g = d.generation;
    let p = d.auth.take_params(&url);
    app.run(
        Call {
            id: CREDENTIALS,
            params: p,
        },
        Then::GetData(Step::Credentials(g)),
    );
}

/// Connect on a SQL Server database: `source.credentials` when a login is typed (the password
/// field is emptied at once), then `source.navigate`.
fn connect_sql(app: &mut App) {
    let l = app.lang;
    let Some(d) = app.cmd.get_data.as_mut() else {
        return;
    };
    let server = d.sql.server.trim().to_string();
    if server.is_empty() || d.sql.database.trim().is_empty() {
        d.problem = Some(
            l.t(
                "Type the server and the database.",
                "Saisissez le serveur et la base de données.",
            )
            .into(),
        );
        return;
    }
    let user = d.sql.user.trim().to_string();
    if user.is_empty() {
        navigate(app);
        return;
    }
    let password = std::mem::take(&mut d.sql.password).0;
    d.generation = d.generation.wrapping_add(1);
    d.busy = true;
    d.problem = None;
    let g = d.generation;
    app.run(
        Call {
            id: CREDENTIALS,
            params: json!({"server": server, "user": user, "password": password}),
        },
        Then::GetData(Step::Credentials(g)),
    );
}

/// `source.navigate` on the SQL Server database or the address typed.
fn navigate(app: &mut App) {
    let Some(d) = app.cmd.get_data.as_mut() else {
        return;
    };
    d.generation = d.generation.wrapping_add(1);
    d.busy = true;
    d.problem = None;
    let p = if d.source.is_url() {
        json!({"kind": d.source.kind(), "url": d.url.trim()})
    } else {
        let mut p = json!({
            "kind": Source::Sql.kind(),
            "server": d.sql.server.trim(),
            "database": d.sql.database.trim(),
        });
        if !d.sql.query.trim().is_empty() {
            p["query"] = json!(d.sql.query.trim());
        }
        p
    };
    let g = d.generation;
    app.run(
        Call {
            id: NAVIGATE,
            params: p,
        },
        Then::GetData(Step::Navigate(g)),
    );
}

/// The parameters a source's queries run with: its data root, the connectors it needs.
fn source_params(p: &mut Value, n: &Nav) {
    if !n.data_root.is_empty() || n.connectors.is_empty() {
        p["data_root"] = json!(n.data_root);
    }
    if !n.connectors.is_empty() {
        p["allow_connectors"] = json!(n.connectors);
    }
}

/// Shows item `i`: its preview (`query.preview` on its query).
pub fn select(app: &mut App, i: usize) {
    let Some(d) = app.cmd.get_data.as_mut() else {
        return;
    };
    let g = d.generation;
    let Some(n) = d.nav.as_mut() else { return };
    let Some(item) = n.items.get(i) else { return };
    n.selected = Some(i);
    if let Some(e) = &item.error {
        n.preview = Some(Err(e.clone()));
        return;
    }
    n.preview = None;
    let mut p = json!({"expression": item.m, "limit": PREVIEW_ROWS});
    source_params(&mut p, n);
    app.run(
        Call {
            id: "query.preview",
            params: p,
        },
        Then::GetData(Step::Preview(g, i)),
    );
}

/// The snapshot's table names, lower case.
fn taken(app: &App) -> Vec<String> {
    app.loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/model/tables"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| t.get("name").and_then(Value::as_str))
        .map(str::to_lowercase)
        .collect()
}

/// The table names of the checked items: the item's name, else `Name (2)`, `Name (3)`… the first
/// one no table and no other checked item has.
pub fn table_names(items: &[&Item], taken: &[String]) -> Vec<String> {
    let mut used: Vec<String> = taken.to_vec();
    let mut out = Vec::with_capacity(items.len());
    for it in items {
        let base = if it.name.trim().is_empty() {
            "Query".to_string()
        } else {
            it.name.trim().to_string()
        };
        let name = std::iter::once(base.clone())
            .chain((2..10_000).map(|i| format!("{base} ({i})")))
            .find(|n| !used.contains(&n.to_lowercase()))
            .unwrap_or(base);
        used.push(name.to_lowercase());
        out.push(name);
    }
    out
}

/// The `source.load` call of the checked items.
pub fn load_call(app: &App) -> Option<Call> {
    let n = app.cmd.get_data.as_ref()?.nav.as_ref()?;
    let items: Vec<&Item> = n.checked.iter().filter_map(|i| n.items.get(*i)).collect();
    if items.is_empty() {
        return None;
    }
    let names = table_names(&items, &taken(app));
    let tables: Vec<Value> = items
        .iter()
        .zip(names)
        .map(|(it, name)| json!({"name": name, "m": it.m}))
        .collect();
    let mut params = json!({"tables": tables});
    source_params(&mut params, n);
    Some(Call { id: LOAD, params })
}

fn load(app: &mut App) {
    let Some(c) = load_call(app) else { return };
    let Some(d) = app.cmd.get_data.as_mut() else {
        return;
    };
    d.generation = d.generation.wrapping_add(1);
    d.busy = true;
    d.problem = None;
    let g = d.generation;
    app.run(c, Then::GetData(Step::Load(g)));
}

/// Transform data: the checked items open in the Power Query editor as new queries (named as
/// Load would name their tables); Close & Apply there loads them.
fn transform(app: &mut App) {
    let Some(c) = load_call(app) else { return };
    let new: Vec<(String, String)> = c.params["tables"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| (s(t, "name"), s(t, "m")))
        .collect();
    let root = c.params["data_root"].as_str().map(str::to_string);
    app.cmd.get_data = None;
    crate::pq_editor::open(app, new, root);
}

/// What a click in the dialog asks for (applied after drawing).
enum Act {
    Connect,
    Browse,
    Select(usize),
    Load,
    Transform,
    Back,
    EnterData,
    Blank,
}

/// Draws the open dialog.
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut d) = app.cmd.get_data.clone() else {
        return;
    };
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let mut open = true;
    let mut close = false;
    let mut acts = Vec::new();
    let title = if d.nav.is_some() {
        l.t("Navigator", "Navigateur")
    } else {
        l.t("Get data", "Obtenir des données")
    };
    let browse = app.shell.pick_data.is_some();
    let files: Vec<String> = match (&app.files, app.workspace()) {
        (Some(Ok(f)), Some(_)) => f.iter().map(|f| f.path.clone()).take(MAX_LISTED).collect(),
        _ => Vec::new(),
    };
    egui::Window::new(title)
        .id(egui::Id::new("bicraft-get-data"))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width(760.0);
            match d.nav.is_some() {
                false => gallery(ui, &mut d, l, &t, browse, &files, &mut acts, &mut close),
                true => navigator(ui, &mut d, l, &t, &mut acts, &mut close),
            }
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    app.cmd.get_data = (open && !close).then_some(d);
    for a in acts {
        match a {
            Act::Connect => connect(app),
            Act::Browse => pick(app),
            Act::Select(i) => select(app, i),
            Act::Load => load(app),
            Act::Transform => transform(app),
            Act::Back => {
                if let Some(d) = app.cmd.get_data.as_mut() {
                    d.nav = None;
                    d.problem = None;
                    d.generation = d.generation.wrapping_add(1);
                }
            }
            Act::EnterData => {
                app.cmd.get_data = None;
                crate::enter_data::open(app);
            }
            Act::Blank => {
                app.cmd.get_data = None;
                crate::form::open(app, "table.import", json!({}));
            }
        }
    }
}

/// Browse…: the shell's data dialog.
fn pick(app: &mut App) {
    let l = app.lang;
    let Some(d) = app.cmd.get_data.as_ref() else {
        return;
    };
    let source = d.source;
    let picked = app
        .shell
        .pick_data
        .as_ref()
        .and_then(|p| p(l, source.text(l), source.extensions()));
    if let (Some(p), Some(d)) = (picked, app.cmd.get_data.as_mut()) {
        d.path = p.display().to_string();
    }
}

/// A vertical rule of a fixed height between two columns (`ui.separator()` in a row takes the
/// whole available height, and an auto-sized window then grows every frame).
fn rule(ui: &mut Ui, t: &Tokens, height: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(9.0, height), egui::Sense::hover());
    ui.painter().vline(
        rect.center().x,
        rect.y_range(),
        egui::Stroke::new(1.0, t.separator),
    );
}

fn labelled_text(ui: &mut Ui, label: &str, value: &mut String, width: f32) -> egui::Response {
    ui.horizontal(|ui| {
        let r = ui.add_sized([110.0, 20.0], egui::Label::new(label));
        ui.add(
            egui::TextEdit::singleline(value)
                .char_limit(4096)
                .desired_width(width),
        )
        .labelled_by(r.id)
    })
    .inner
}

#[allow(clippy::too_many_arguments)]
fn gallery(
    ui: &mut Ui,
    d: &mut GetData,
    l: Lang,
    t: &Tokens,
    browse: bool,
    files: &[String],
    acts: &mut Vec<Act>,
    close: &mut bool,
) {
    ui.horizontal_top(|ui| {
        // the sources
        ui.vertical(|ui| {
            ui.set_width(220.0);
            ui.label(RichText::new(l.t("Sources", "Sources")).font(theme::bold(13.0)));
            for src in Source::ALL {
                let on = d.source == src;
                let r = ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                    icons::paint(ui.painter(), rect, src.icon(), t.text, false);
                    ui.selectable_label(on, src.text(l))
                });
                if r.inner.clicked() && !on {
                    d.source = src;
                    d.problem = None;
                }
            }
            ui.separator();
            ui.label(RichText::new(l.t("Other", "Autres")).font(theme::bold(13.0)));
            if ui
                .selectable_label(false, l.t("Enter data", "Entrer des données"))
                .on_hover_text(l.t("Type or paste a table", "Saisissez ou collez une table"))
                .clicked()
            {
                acts.push(Act::EnterData);
            }
            if ui
                .selectable_label(false, l.t("Blank query", "Requête vide"))
                .on_hover_text(l.t(
                    "A table from Power Query (M), CSV text or DAX (Import table)",
                    "Une table depuis du Power Query (M), du texte CSV ou du DAX (Importer une table)",
                ))
                .clicked()
            {
                acts.push(Act::Blank);
            }
        });
        rule(ui, t, 300.0);
        // the source picked
        ui.vertical(|ui| {
            ui.label(RichText::new(d.source.text(l)).font(theme::bold(15.0)));
            ui.label(RichText::new(d.source.about(l)).color(t.text_dim));
            ui.add_space(8.0);
            if d.source == Source::Sql {
                sql_fields(ui, &mut d.sql, l, t, acts);
            } else if d.source.is_url() {
                url_fields(ui, d, l, t, acts);
            } else {
                file_fields(ui, d, l, browse, files, acts);
            }
            if let Some(p) = &d.problem {
                ui.add_space(4.0);
                ui.label(RichText::new(format!("⚠ {p}")).color(t.warn));
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if d.busy {
                    ui.spinner();
                }
                let ready = match d.source {
                    Source::Sql => !d.sql.server.trim().is_empty() && !d.sql.database.trim().is_empty(),
                    Source::Web | Source::OData | Source::SharePointFolder | Source::SharePointList => {
                        !d.url.trim().is_empty()
                    }
                    _ => !d.path.trim().is_empty(),
                };
                let ok = !d.busy && ready;
                if widgets::primary(ui, l.t("Connect", "Se connecter"), ok, "").clicked() && ok {
                    acts.push(Act::Connect);
                }
                if ui.button(l.t("Cancel", "Annuler")).clicked() {
                    *close = true;
                }
            });
        });
    });
}

/// A SQL Server database: server, database, an optional SQL statement, a login.
fn sql_fields(ui: &mut Ui, f: &mut SqlForm, l: Lang, t: &Tokens, acts: &mut Vec<Act>) {
    let enter = |ui: &Ui, r: &egui::Response| r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let r = labelled_text(ui, l.t("Server", "Serveur"), &mut f.server, 330.0).on_hover_text(l.t(
        "host, host,port or host\\instance",
        "hôte, hôte,port ou hôte\\instance",
    ));
    if enter(ui, &r) {
        acts.push(Act::Connect);
    }
    let r = labelled_text(ui, l.t("Database", "Base de données"), &mut f.database, 330.0);
    if enter(ui, &r) {
        acts.push(Act::Connect);
    }
    ui.horizontal_top(|ui| {
        let r = ui.add_sized(
            [110.0, 20.0],
            egui::Label::new(l.t("SQL statement", "Instruction SQL")),
        );
        ui.add(
            egui::TextEdit::multiline(&mut f.query)
                .char_limit(256 * 1024)
                .desired_rows(3)
                .desired_width(330.0)
                .hint_text(l.t("Optional", "Facultatif")),
        )
        .labelled_by(r.id);
    });
    ui.add_space(6.0);
    ui.label(RichText::new(l.t("SQL Server login", "Identifiant SQL Server")).font(theme::bold(13.0)));
    labelled_text(ui, l.t("User name", "Nom d'utilisateur"), &mut f.user, 200.0);
    let r = ui
        .horizontal(|ui| {
            let r = ui.add_sized([110.0, 20.0], egui::Label::new(l.t("Password", "Mot de passe")));
            ui.add(
                egui::TextEdit::singleline(&mut f.password.0)
                    .password(true)
                    .char_limit(128)
                    .desired_width(200.0),
            )
            .labelled_by(r.id)
        })
        .inner;
    if enter(ui, &r) {
        acts.push(Act::Connect);
    }
    ui.label(
        RichText::new(l.t(
            "Kept in memory for this session only, never in the file. Empty: the login entered before for this server, else BICRAFT_SQLSERVER_USER and BICRAFT_SQLSERVER_PASSWORD. Windows authentication is not supported.",
            "Gardé en mémoire pour cette session seulement, jamais dans le fichier. Vide : l'identifiant déjà saisi pour ce serveur, sinon BICRAFT_SQLSERVER_USER et BICRAFT_SQLSERVER_PASSWORD. L'authentification Windows n'est pas prise en charge.",
        ))
        .color(t.text_dim),
    );
}

/// A Web, OData or SharePoint source: its address (Desktop's From Web, Basic), then the
/// credential and the level it applies to (Desktop's Access Web content).
fn url_fields(ui: &mut Ui, d: &mut GetData, l: Lang, t: &Tokens, acts: &mut Vec<Act>) {
    let label = match d.source {
        Source::OData => l.t("Feed URL", "URL du flux"),
        Source::SharePointFolder | Source::SharePointList => l.t("Site URL", "URL du site"),
        _ => "URL",
    };
    let r = labelled_text(ui, label, &mut d.url, 330.0);
    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        acts.push(Act::Connect);
    }
    ui.add_space(6.0);
    ui.label(RichText::new(l.t("Credentials", "Informations d'identification")).font(theme::bold(13.0)));
    let url = d.url.clone();
    auth_fields(ui, &mut d.auth, &url, "bicraft-get-data", l, t);
}

/// The credential of a web source: its kind, its fields, the level it applies to. Shared with
/// Data source settings (`crate::source_settings`).
pub fn auth_fields(ui: &mut Ui, a: &mut WebAuth, url: &str, salt: &str, l: Lang, t: &Tokens) {
    ui.horizontal(|ui| {
        let r = ui.add_sized(
            [110.0, 20.0],
            egui::Label::new(l.t("Authentication", "Authentification")),
        );
        egui::ComboBox::from_id_salt(format!("{salt}-auth"))
            .selected_text(a.method.text(l))
            .width(330.0)
            .show_ui(ui, |ui| {
                for m in WebMethod::ALL {
                    ui.selectable_value(&mut a.method, m, m.text(l));
                }
            })
            .response
            .labelled_by(r.id);
    });
    let secret = |ui: &mut Ui, label: &str, s: &mut Secret| {
        ui.horizontal(|ui| {
            let r = ui.add_sized([110.0, 20.0], egui::Label::new(label));
            ui.add(
                egui::TextEdit::singleline(&mut s.0)
                    .password(true)
                    .char_limit(16 * 1024)
                    .desired_width(330.0),
            )
            .labelled_by(r.id);
        });
    };
    match a.method {
        WebMethod::Anonymous => {}
        WebMethod::Basic => {
            labelled_text(ui, l.t("User name", "Nom d'utilisateur"), &mut a.user, 330.0);
            secret(ui, l.t("Password", "Mot de passe"), &mut a.password);
        }
        WebMethod::Key => {
            secret(ui, l.t("Key", "Clé"), &mut a.key);
            labelled_text(ui, l.t("Header", "En-tête"), &mut a.header, 330.0).on_hover_text(l.t(
                "Empty: the key goes in the query parameter the query's ApiKeyName names",
                "Vide : la clé va dans le paramètre de requête que nomme ApiKeyName",
            ));
        }
        WebMethod::Bearer => secret(ui, l.t("Access token", "Jeton d'accès"), &mut a.token),
    }
    if a.method != WebMethod::Anonymous {
        let levels = levels(url);
        if !levels.is_empty() {
            ui.horizontal(|ui| {
                let r = ui.add_sized([110.0, 20.0], egui::Label::new(l.t("Apply to", "Appliquer à")));
                let shown = if a.level.is_empty() {
                    levels.last().cloned().unwrap_or_default()
                } else {
                    a.level.clone()
                };
                egui::ComboBox::from_id_salt(format!("{salt}-level"))
                    .selected_text(clip(&shown, 60))
                    .width(330.0)
                    .show_ui(ui, |ui| {
                        for lv in &levels {
                            if ui.selectable_label(shown == *lv, clip(lv, 80)).clicked() {
                                a.level = lv.clone();
                            }
                        }
                    })
                    .response
                    .labelled_by(r.id);
            });
        }
    }
    ui.label(
        RichText::new(match a.method {
            WebMethod::Anonymous => l.t(
                "Anonymous: no credential is stored; a credential set before for this address is used.",
                "Anonyme : aucun identifiant n'est enregistré ; celui déjà défini pour cette adresse est utilisé.",
            ),
            WebMethod::Bearer => l.t(
                "Paste an access token obtained elsewhere: BIcraft does not run the sign-in. Kept in memory for this session only, never in the file.",
                "Collez un jeton d'accès obtenu ailleurs : BIcraft ne lance pas la connexion. Gardé en mémoire pour cette session seulement, jamais dans le fichier.",
            ),
            _ => l.t(
                "Kept in memory for this session only, never in the file. Windows authentication is not supported.",
                "Gardé en mémoire pour cette session seulement, jamais dans le fichier. L'authentification Windows n'est pas prise en charge.",
            ),
        })
        .color(t.text_dim),
    );
}

/// A file or a folder: its path, Browse… or the workspace's files, a text file's delimiter.
fn file_fields(ui: &mut Ui, d: &mut GetData, l: Lang, browse: bool, files: &[String], acts: &mut Vec<Act>) {
    let label = if d.source == Source::Folder {
        l.t("Folder path", "Chemin du dossier")
    } else {
        l.t("File path", "Chemin du fichier")
    };
    let r = labelled_text(ui, label, &mut d.path, 330.0);
    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        acts.push(Act::Connect);
    }
    if browse {
        ui.horizontal(|ui| {
            ui.add_space(114.0);
            if ui.button(l.t("Browse…", "Parcourir…")).clicked() {
                acts.push(Act::Browse);
            }
        });
    } else if !files.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(114.0);
            egui::ComboBox::from_id_salt("bicraft-get-data-files")
                .selected_text(l.t("Workspace files…", "Fichiers de l'espace…"))
                .width(330.0)
                .show_ui(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                        for f in files {
                            if ui.selectable_label(false, f).clicked() {
                                d.path = f.clone();
                            }
                        }
                    });
                });
        });
    }
    if d.source == Source::Csv {
        labelled_text(ui, l.t("Delimiter", "Séparateur"), &mut d.delimiter, 60.0).on_hover_text(l.t(
            "Empty: found from the first line. One character, or tab",
            "Vide : trouvé dans la première ligne. Un caractère, ou tab",
        ));
    }
}

fn navigator(ui: &mut Ui, d: &mut GetData, l: Lang, t: &Tokens, acts: &mut Vec<Act>, close: &mut bool) {
    let busy = d.busy;
    let Some(n) = d.nav.as_mut() else { return };
    ui.label(RichText::new(clip(&n.path, 120)).color(t.text_dim));
    ui.add_space(4.0);
    ui.horizontal_top(|ui| {
        // the items
        ui.vertical(|ui| {
            ui.set_width(250.0);
            labelled_text(ui, l.t("Search", "Rechercher"), &mut n.search, 120.0);
            let needle = n.search.trim().to_lowercase();
            egui::ScrollArea::vertical()
                .id_salt("bicraft-get-data-items")
                .max_height(340.0)
                .show(ui, |ui| {
                    if n.items.is_empty() {
                        ui.label(
                            RichText::new(l.t("Nothing to load here.", "Rien à charger ici."))
                                .color(t.text_dim),
                        );
                    }
                    for (i, it) in n.items.iter().enumerate() {
                        if !needle.is_empty() && !it.name.to_lowercase().contains(&needle) {
                            continue;
                        }
                        ui.horizontal(|ui| {
                            let mut on = n.checked.contains(&i);
                            let ok = it.error.is_none();
                            let check = ui.add_enabled(ok, egui::Checkbox::without_text(&mut on));
                            let label = format!("{} {}", l.t("Load", "Charger"), it.name);
                            check.widget_info(|| {
                                egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ok, on, &label)
                            });
                            if check.changed() {
                                if on {
                                    n.checked.insert(i);
                                } else {
                                    n.checked.remove(&i);
                                }
                            }
                            let kind = kind_text(&it.kind, l);
                            let mut text = RichText::new(format!("{}  ({kind})", clip(&it.name, 60)));
                            if it.hidden || !ok {
                                text = text.color(t.text_dim);
                            }
                            let r = ui.selectable_label(n.selected == Some(i), text);
                            let r = match &it.error {
                                Some(e) => r.on_hover_text(format!("⚠ {}", clip(e, 300))),
                                None => r,
                            };
                            if r.clicked() {
                                acts.push(Act::Select(i));
                            }
                        });
                    }
                });
        });
        rule(ui, t, 360.0);
        // the preview
        ui.vertical(|ui| {
            ui.set_width(470.0);
            ui.set_min_height(340.0);
            match (n.selected.and_then(|i| n.items.get(i)), &n.preview) {
                (None, _) => {
                    ui.label(
                        RichText::new(l.t(
                            "Click an item to preview it.",
                            "Cliquez sur un élément pour en voir un aperçu.",
                        ))
                        .color(t.text_dim),
                    );
                }
                (Some(it), p) => {
                    ui.label(RichText::new(clip(&it.name, 80)).font(theme::bold(14.0)));
                    match p {
                        None => {
                            ui.spinner();
                        }
                        Some(Err(e)) => {
                            ui.label(RichText::new(format!("⚠ {}", clip(e, 600))).color(t.warn));
                        }
                        Some(Ok(p)) => preview_grid(ui, p, l, t),
                    }
                }
            }
        });
    });
    if let Some(p) = &d.problem {
        ui.label(RichText::new(format!("⚠ {p}")).color(t.warn));
    }
    ui.add_space(8.0);
    let checked = d.nav.as_ref().map_or(0, |n| n.checked.len());
    ui.horizontal(|ui| {
        if busy {
            ui.spinner();
        }
        let ok = !busy && checked > 0;
        let why = l.t("Tick the items to load.", "Cochez les éléments à charger.");
        if widgets::primary(ui, l.t("Load", "Charger"), ok, if ok { "" } else { why }).clicked() && ok {
            acts.push(Act::Load);
        }
        let why = l.t(
            "Tick the items to transform.",
            "Cochez les éléments à transformer.",
        );
        let r = ui.add_enabled(
            ok,
            egui::Button::new(l.t("Transform data", "Transformer les données")),
        );
        let r = if ok {
            r.on_hover_text(l.t(
                "Open the items in the Power Query editor; Close & Apply loads them",
                "Ouvre les éléments dans l'éditeur Power Query ; Fermer et appliquer les charge",
            ))
        } else {
            r.on_disabled_hover_text(why)
        };
        if r.clicked() && ok {
            acts.push(Act::Transform);
        }
        if ui.button(l.t("Back", "Retour")).clicked() {
            acts.push(Act::Back);
        }
        if ui.button(l.t("Cancel", "Annuler")).clicked() {
            *close = true;
        }
        if checked > 0 {
            ui.label(
                RichText::new(format!("{checked} {}", l.t("selected", "sélectionné(s)"))).color(t.text_dim),
            );
        }
    });
}

fn kind_text(kind: &str, l: Lang) -> &'static str {
    match kind {
        "View" => l.t("view", "vue"),
        "Query" => l.t("query", "requête"),
        "Sheet" => l.t("sheet", "feuille"),
        "Table" => l.t("table", "tableau"),
        "DefinedName" => l.t("named range", "plage nommée"),
        "File" => l.t("file", "fichier"),
        "List" => l.t("list", "liste"),
        "Record" => l.t("record", "enregistrement"),
        "Folder" => l.t("file list", "liste de fichiers"),
        "Combined" => l.t("files appended", "fichiers ajoutés"),
        _ => l.t("item", "élément"),
    }
}

/// A cell of a preview as text.
fn cell(v: &Value, l: Lang) -> String {
    match v {
        Value::Null => String::new(),
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

fn preview_grid(ui: &mut Ui, p: &Preview, l: Lang, t: &Tokens) {
    egui::ScrollArea::both()
        .id_salt("bicraft-get-data-preview")
        .max_height(300.0)
        .show(ui, |ui| {
            egui::Grid::new("bicraft-get-data-grid")
                .striped(true)
                .spacing(egui::vec2(12.0, 3.0))
                .show(ui, |ui| {
                    for (name, ty) in &p.columns {
                        ui.label(RichText::new(clip(name, MAX_CELL)).font(theme::bold(12.0)))
                            .on_hover_text(ty);
                    }
                    ui.end_row();
                    for r in &p.rows {
                        for v in r {
                            ui.label(RichText::new(cell(v, l)).color(t.text));
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
}
