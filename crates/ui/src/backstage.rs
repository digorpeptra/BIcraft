//! The File view (Backstage): a full-window view that replaces the ribbon, the page and the status
//! bar until Escape or the back arrow returns to the document; with nothing open it is the start
//! screen. A side menu picks a page (Home, Open, Info, Export); Save and Close act at once. Every
//! action runs an engine command through [`App::run`] ([`crate::actions`]); the state here is UI
//! state only and never saved. Recent files are listed by path, split as text: nothing on disk is
//! read until one is picked.
//!
//! Ported from DeckCraft `crates/ui-egui/src/backstage.rs` and `backstage/files.rs` @ c252a68
//! (MIT OR Apache-2.0, © 2026 ArtCraft Team and the DeckCraft contributors): the side menu
//! (`NAV_W`, `nav_item`, `separator`, the keyboard walk), `split_path`, the recent-file rows with
//! their "Remove from Recent" button, `place_row`, `square_button`, `task`, `heading`, `dim`,
//! `arrow_link`, `search`. Adapted to BIcraft: its pages (Home, Open with Recent / Browse /
//! Servers, Info, Export with Save, Save As, Convert, Render, Compare), the format badges, the
//! dates and pins, the workspace files of the browser, the never-crash rules (no indexing, capped
//! lists), the strings in English and French.

use std::path::PathBuf;

use egui::{Align2, CornerRadius, Key, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::Value;

use crate::App;
use crate::actions::{self, CONVERT_FORMATS};
use crate::app::{Then, clip};
use crate::i18n::Lang;
use crate::icons::{self, Icon};
use crate::prefs::Recent;
use crate::theme::{self, Tokens};
use crate::widgets;

/// Width of the side menu.
pub const NAV_W: f32 = 160.0;
const ITEM_H: f32 = 40.0;
const BACK_H: f32 = 46.0;
/// The widest a page's content grows.
const CONTENT_W: f32 = 1100.0;
/// Most rows a list draws (the search narrows the rest).
pub const MAX_LIST_ROWS: usize = 500;
/// Recent files on Home (the rest are on Open).
const HOME_RECENT: usize = 8;

/// The File tab's commands Info lists as task rows (the others have their own page or task).
pub const INFO_COMMANDS: [&str; 3] = ["model.new", "objects.import", "about.licenses"];

/// Every engine command the File view runs (the registry's File tab; the coverage test reads it):
/// Open and Recent, Save, Save As, New, Info, the Export tasks and [`INFO_COMMANDS`].
pub const IDS: &[&str] = &[
    "file.open",
    "file.save",
    "file.save_as",
    "project.new",
    "file.inspect",
    "file.convert",
    "file.redact",
    "report.render",
    "report.export_pdf",
    "report.print",
    "report.settings",
    "file.diff",
    "model.new",
    "objects.import",
    "about.licenses",
];

/// A page of the File view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Home,
    Open,
    Info,
    Export,
    /// Options and settings: the current file's Report settings (`crate::options_page`).
    Options,
}

impl Page {
    pub fn label(self, l: Lang) -> &'static str {
        match self {
            Page::Home => l.t("Home", "Accueil"),
            Page::Open => l.t("Open", "Ouvrir"),
            Page::Info => l.t("Info", "Informations"),
            Page::Export => l.t("Export", "Exporter"),
            Page::Options => l.t("Options and settings", "Options et paramètres"),
        }
    }

    fn icon(self) -> Icon {
        match self {
            Page::Home => Icon::Home,
            Page::Open => Icon::Folder,
            Page::Info => Icon::Info,
            Page::Export => Icon::Export,
            Page::Options => Icon::Settings,
        }
    }

    /// Pages about the open document (hidden on the start screen).
    pub fn needs_doc(self) -> bool {
        matches!(self, Page::Info | Page::Export | Page::Options)
    }
}

/// A side-menu entry: a page, or an action that runs at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    Page(Page),
    Save,
    SaveAs,
    Close,
}

impl Nav {
    fn label(self, l: Lang) -> &'static str {
        match self {
            Nav::Page(p) => p.label(l),
            Nav::Save => l.t("Save", "Enregistrer"),
            Nav::SaveAs => l.t("Save As", "Enregistrer sous"),
            Nav::Close => l.t("Close", "Fermer"),
        }
    }

    fn icon(self) -> Icon {
        match self {
            Nav::Page(p) => p.icon(),
            Nav::Save => Icon::Save,
            Nav::SaveAs => Icon::Save,
            Nav::Close => Icon::Close,
        }
    }
}

/// The side menu in keyboard order; with nothing open only Home and Open.
pub fn entries(has_doc: bool) -> Vec<Nav> {
    let mut v = vec![Nav::Page(Page::Home), Nav::Page(Page::Open)];
    if has_doc {
        v.extend([
            Nav::Page(Page::Info),
            Nav::Save,
            Nav::SaveAs,
            Nav::Page(Page::Export),
            Nav::Page(Page::Options),
            Nav::Close,
        ]);
    }
    v
}

/// Open: the place listed on the right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Recent,
    /// The browser: the files of the server workspace.
    Browse,
    /// The desktop: the remembered servers.
    Servers,
}

/// Export: the task shown on the right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Task {
    SaveAs,
    Convert,
    Pdf,
    Print,
    Render,
    Data,
    Compare,
    Redact,
}

impl Task {
    pub const ALL: [Task; 8] = [
        Task::SaveAs,
        Task::Convert,
        Task::Pdf,
        Task::Print,
        Task::Render,
        Task::Data,
        Task::Compare,
        Task::Redact,
    ];

    /// The id of its output ([`Then::Output`]).
    pub fn id(self) -> &'static str {
        match self {
            Task::SaveAs => "save_as",
            Task::Convert => "convert",
            Task::Pdf => "pdf",
            Task::Print => "print",
            Task::Render => "render",
            Task::Data => "data_setting",
            Task::Compare => "compare",
            Task::Redact => "redact",
        }
    }

    fn label(self, l: Lang) -> &'static str {
        match self {
            Task::SaveAs => l.t("Save a copy as", "Enregistrer sous"),
            Task::Convert => l.t("Convert to another format", "Convertir dans un autre format"),
            Task::Pdf => l.t("Export to PDF", "Exporter au format PDF"),
            Task::Print => l.t("Print", "Imprimer"),
            Task::Render => l.t("Render the page to PNG", "Rendre la page en PNG"),
            Task::Data => l.t("Export data setting", "Paramètre d'export des données"),
            Task::Compare => l.t("Compare with another file", "Comparer avec un autre fichier"),
            Task::Redact => l.t("Redact a copy for sharing", "Copie anonymisée à partager"),
        }
    }

    fn icon(self) -> Icon {
        match self {
            Task::SaveAs => Icon::Save,
            Task::Convert => Icon::Export,
            Task::Pdf => Icon::Export,
            Task::Print => Icon::Print,
            Task::Render => Icon::Chart,
            Task::Data => Icon::Settings,
            Task::Compare => Icon::Copy,
            Task::Redact => Icon::Lock,
        }
    }
}

/// The File view's state while it is open.
#[derive(Clone, Debug)]
pub struct Backstage {
    pub page: Page,
    pub place: Place,
    pub task: Task,
    /// Keyboard focus in the side menu (an index into [`entries`]).
    pub focus: Option<usize>,
    /// Search text of Home and Open.
    pub query: String,
    /// Convert: the target format (an id of [`CONVERT_FORMATS`]).
    pub format: &'static str,
    /// The name typed for the task's target when the shell has no Save dialog (the browser: a
    /// workspace-relative name).
    pub target: String,
    /// New project: the name being typed (the dialog is open).
    pub new_name: Option<String>,
    /// New from a template: the template picked.
    pub template: Option<PathBuf>,
    /// Export to PDF: which pages (`all`, `current` or `range`), the range typed, hidden pages.
    pub pdf_pages: &'static str,
    pub pdf_range: String,
    pub pdf_hidden: bool,
    /// Export to PDF: an archival file (PDF/A-2b).
    pub pdf_archive: bool,
    /// Redact a copy: replace the stored text values too (`file.redact {data}`).
    pub redact_data: bool,
}

impl Backstage {
    pub fn new(page: Page) -> Backstage {
        Backstage {
            page,
            place: Place::Recent,
            task: Task::SaveAs,
            focus: None,
            query: String::new(),
            format: "pbix",
            target: String::new(),
            new_name: None,
            template: None,
            pdf_pages: "all",
            pdf_range: String::new(),
            pdf_hidden: false,
            pdf_archive: false,
            redact_data: false,
        }
    }

    /// Shows `page`, as a click in the side menu does.
    pub fn go(&mut self, page: Page) {
        if self.page != page {
            self.query.clear();
        }
        self.page = page;
    }
}

/// What a click asked for, applied after drawing.
#[derive(Default)]
struct Outcome {
    close: bool,
    page: Option<Page>,
}

/// Runs a side-menu action. Returns the page to show, or `None` to close the view.
fn act(app: &mut App, bs: &mut Backstage, nav: Nav) -> Option<Page> {
    match nav {
        Nav::Page(p) => Some(p),
        Nav::Save => {
            app.save();
            None
        }
        Nav::SaveAs => {
            bs.task = Task::SaveAs;
            Some(Page::Export)
        }
        Nav::Close => {
            app.ask_close(app.active_doc());
            Some(bs.page)
        }
    }
}

/// The File view's own keys: Escape goes back to the document; Up and Down walk the side menu (a
/// page shows as soon as it has the focus); Enter or Space runs the focused action. True when the
/// key was used.
pub fn key(app: &mut App, key: Key) -> bool {
    let has_doc = app.has_doc();
    let all = entries(has_doc);
    let Some(mut bs) = app.backstage.take() else {
        return false;
    };
    let used = match key {
        Key::Escape if has_doc => {
            app.backstage = None;
            return true;
        }
        Key::ArrowDown | Key::ArrowUp => {
            let n = all.len().max(1);
            let cur = bs
                .focus
                .or_else(|| all.iter().position(|x| *x == Nav::Page(bs.page)));
            let next = match (cur, key == Key::ArrowDown) {
                (None, _) => 0,
                (Some(i), true) => (i + 1) % n,
                (Some(i), false) => (i + n - 1) % n,
            };
            bs.focus = Some(next);
            if let Some(Nav::Page(p)) = all.get(next) {
                bs.go(*p);
            }
            true
        }
        Key::Enter | Key::Space => match bs.focus.and_then(|i| all.get(i)).copied() {
            Some(nav) => {
                match act(app, &mut bs, nav) {
                    None if has_doc => {
                        app.backstage = None;
                        return true;
                    }
                    None => {}
                    Some(p) => bs.go(p),
                }
                true
            }
            None => false,
        },
        _ => false,
    };
    if app.backstage.is_none() {
        app.backstage = Some(bs);
    }
    used
}

/// Draws the File view in place of the ribbon, the document and the status bar.
pub fn show(app: &mut App, ui: &mut Ui) {
    let Some(mut bs) = app.backstage.take() else {
        return;
    };
    app.close_backstage_now = false;
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let has_doc = app.has_doc();
    if !has_doc && bs.page.needs_doc() {
        bs.page = Page::Home;
    }
    let mut out = Outcome::default();
    let all = entries(has_doc);
    egui::Panel::left("bicraft-backstage-nav")
        .exact_size(NAV_W)
        .resizable(false)
        .frame(egui::Frame::NONE.fill(t.chrome))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if has_doc {
                let (r, resp) = ui.allocate_exact_size(vec2(NAV_W, BACK_H), Sense::click());
                let label = l.t("Back", "Retour");
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
                let c = pos2(r.left() + 26.0, r.center().y);
                if resp.hovered() {
                    ui.painter().circle_filled(c, 17.0, t.hover);
                }
                icons::paint(
                    ui.painter(),
                    Rect::from_center_size(c, vec2(22.0, 22.0)),
                    Icon::Back,
                    t.text,
                    false,
                );
                if resp.on_hover_text(l.t("Back (Esc)", "Retour (Échap)")).clicked() {
                    out.close = true;
                }
            } else {
                ui.add_space(BACK_H);
            }
            for (i, nav) in all.iter().enumerate() {
                if i == 2 {
                    separator(ui, &t);
                }
                if nav_item(ui, *nav, &bs, i, &t, l).clicked() {
                    bs.focus = None;
                    match act(app, &mut bs, *nav) {
                        None => out.close = true,
                        Some(p) => out.page = Some(p),
                    }
                }
            }
        });
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(t.card))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("bicraft-backstage-page")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Frame::NONE
                        .inner_margin(egui::Margin {
                            left: 48,
                            right: 36,
                            top: 22,
                            bottom: 32,
                        })
                        .show(ui, |ui| {
                            ui.set_max_width(ui.available_width().min(CONTENT_W));
                            if bs.page != Page::Home {
                                ui.label(
                                    RichText::new(bs.page.label(l))
                                        .font(theme::bold(22.0))
                                        .color(t.text),
                                );
                                ui.add_space(16.0);
                            }
                            if let Some(e) = app.error.clone() {
                                if widgets::error_bar(ui, &e, l.t("Close", "Fermer")) {
                                    app.error = None;
                                }
                                ui.add_space(8.0);
                            }
                            match bs.page {
                                Page::Home => home(app, ui, &mut bs, &mut out),
                                Page::Open => open_page(app, ui, &mut bs),
                                Page::Info => info(app, ui),
                                Page::Export => export(app, ui, &mut bs),
                                Page::Options => crate::options_page::show(app, ui),
                            }
                        });
                });
        });
    new_dialog(app, ui.ctx(), &mut bs);
    if let Some(page) = out.page {
        bs.go(page);
    }
    // a command run from a page may have closed the view itself (a new document); keep that
    if out.close && app.has_doc() {
        app.backstage = None;
    } else if app.backstage.is_none() && !app.close_backstage_now {
        app.backstage = Some(bs);
    }
    app.close_backstage_now = false;
}

// ---------------------------------------------------------------- side menu

fn nav_item(ui: &mut Ui, nav: Nav, bs: &Backstage, index: usize, t: &Tokens, l: Lang) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(NAV_W, ITEM_H), Sense::click());
    let selected = nav == Nav::Page(bs.page);
    let label = nav.label(l);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, label));
    let p = ui.painter();
    let row = rect.shrink2(vec2(6.0, 1.0));
    if resp.hovered() {
        p.rect_filled(row, CornerRadius::same(4), t.hover);
    }
    if bs.focus == Some(index) || selected {
        let ink = if bs.focus == Some(index) { t.text } else { t.border };
        p.rect_stroke(
            row,
            CornerRadius::same(4),
            Stroke::new(1.0, ink),
            StrokeKind::Inside,
        );
    }
    if selected {
        p.rect_filled(
            Rect::from_min_size(pos2(row.left(), row.top() + 8.0), vec2(3.0, row.height() - 16.0)),
            CornerRadius::same(1),
            t.accent,
        );
    }
    let ink = if selected { t.contextual } else { t.text };
    icons::paint(
        p,
        Rect::from_center_size(pos2(rect.left() + 26.0, rect.center().y), vec2(16.0, 16.0)),
        nav.icon(),
        t.text,
        false,
    );
    p.text(
        pos2(rect.left() + 44.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        if selected {
            theme::bold(13.0)
        } else {
            theme::font(13.0)
        },
        ink,
    );
    resp
}

fn separator(ui: &mut Ui, t: &Tokens) {
    let (rect, _) = ui.allocate_exact_size(vec2(NAV_W, 13.0), Sense::hover());
    let y = rect.center().y;
    ui.painter().line_segment(
        [pos2(rect.left() + 20.0, y), pos2(rect.right() - 14.0, y)],
        Stroke::new(1.0, t.separator),
    );
}

// ---------------------------------------------------------------- shared pieces

fn heading(ui: &mut Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(text).font(theme::bold(16.0)).color(t.text));
    ui.add_space(8.0);
}

fn dim(ui: &mut Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(text).font(theme::font(12.5)).color(t.text_dim));
}

/// A link with an arrow (More files →): true when clicked.
fn arrow_link(ui: &mut Ui, text: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    ui.add(
        egui::Label::new(
            RichText::new(format!("{text}  →"))
                .font(theme::font(12.5))
                .color(t.contextual),
        )
        .sense(Sense::click()),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
    .clicked()
}

/// A search box.
fn search(ui: &mut Ui, query: &mut String, hint: &str, width: f32) {
    ui.add(
        egui::TextEdit::singleline(query)
            .hint_text(hint)
            .char_limit(200)
            .desired_width(width),
    );
}

/// File name and folder of a path, split as text (nothing on disk is read).
pub fn split_path(path: &str) -> (String, String) {
    let cut = path.rfind(['/', '\\']);
    match cut {
        Some(i) => {
            let name = path.get(i + 1..).unwrap_or("");
            let folder = path.get(..i).unwrap_or("");
            if name.is_empty() {
                (path.to_string(), String::new())
            } else {
                (name.to_string(), folder.to_string())
            }
        }
        None => (path.to_string(), String::new()),
    }
}

/// The format of a file name, from its extension: `PBIX`, `PBIP`, `PBIT`, `BIM` (`""` otherwise).
pub fn format_of(name: &str) -> &'static str {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "pbix" => "PBIX",
        "pbip" => "PBIP",
        "pbit" => "PBIT",
        "bim" => "BIM",
        "xlsx" => "XLSX",
        "xlsm" => "XLSM",
        "png" => "PNG",
        _ => "",
    }
}

/// `2026-11-04` for seconds since 1970 (UTC; `—` when unknown).
pub fn fmt_date(secs: u64) -> String {
    if secs == 0 {
        return "—".to_string();
    }
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let (y, m, d) = bicraft_locale::calendar::civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `12.3 MB` for a byte count.
pub fn fmt_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0usize;
    while v >= 1024.0 && u + 1 < UNITS.len() {
        v /= 1024.0;
        u += 1;
    }
    let unit = UNITS.get(u).copied().unwrap_or("bytes");
    if u == 0 {
        format!("{n} {unit}")
    } else {
        format!("{v:.1} {unit}")
    }
}

/// A file's format badge: the file icon with its format written under it (never colour alone).
fn format_badge(ui: &Ui, c: egui::Pos2, name: &str, t: &Tokens, ink: egui::Color32) {
    let p = ui.painter();
    icons::paint(
        p,
        Rect::from_center_size(c - vec2(0.0, 5.0), vec2(22.0, 22.0)),
        Icon::File,
        ink,
        false,
    );
    let f = format_of(name);
    if !f.is_empty() {
        let r = Rect::from_center_size(c + vec2(0.0, 11.0), vec2(30.0, 12.0));
        p.rect_filled(r, CornerRadius::same(3), t.selected);
        p.text(r.center(), Align2::CENTER_CENTER, f, theme::bold(9.0), t.text);
    }
}

/// A small icon button with its label for the accessibility tree and as tooltip.
fn icon_button(ui: &mut Ui, rect: Rect, icon: Icon, label: &str, t: &Tokens) -> bool {
    let r = ui.allocate_rect(rect, Sense::click());
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    if r.hovered() {
        ui.painter().rect_filled(r.rect, CornerRadius::same(3), t.hover);
    }
    icons::paint(
        ui.painter(),
        Rect::from_center_size(r.rect.center(), vec2(14.0, 14.0)),
        icon,
        t.text_dim,
        false,
    );
    r.on_hover_text(label).clicked()
}

/// The recent files whose name or folder matches `query`.
pub fn matching(recent: &[Recent], query: &str) -> Vec<Recent> {
    let q = query.trim().to_lowercase();
    recent
        .iter()
        .filter(|r| q.is_empty() || r.path.to_lowercase().contains(&q))
        .cloned()
        .collect()
}

/// The recent files of this shell: workspace files of its workspace in the browser, paths on the
/// desktop.
fn shell_recent(app: &App) -> Vec<Recent> {
    let ws = app.shell.workspace.clone();
    app.prefs.recent.iter().filter(|r| r.ws == ws).cloned().collect()
}

/// Recent files matching `query`, at most `limit` rows: format badge, name over folder, the date
/// last opened, pin and "Remove from Recent" buttons. Opening one is the user's explicit choice.
fn recent_list(app: &mut App, ui: &mut Ui, query: &str, limit: usize) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let all = shell_recent(app);
    if all.is_empty() {
        dim(
            ui,
            l.t(
                "Files you open show up here. Use Open to find one.",
                "Les fichiers ouverts apparaissent ici. Utilisez Ouvrir pour en trouver un.",
            ),
        );
        return;
    }
    let list = matching(&all, query);
    if list.is_empty() {
        dim(
            ui,
            &format!(
                "{} “{}”.",
                l.t("No recent file matches", "Aucun fichier récent ne correspond à"),
                clip(query.trim(), 60)
            ),
        );
        return;
    }
    let width = ui.available_width().min(CONTENT_W);
    let total = list.len();
    for r in list.into_iter().take(limit) {
        let (name, folder) = split_path(&r.path);
        let (rect, resp) = ui.allocate_exact_size(vec2(width, 44.0), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
        if resp.hovered() {
            ui.painter().rect_filled(rect, CornerRadius::same(3), t.hover);
        }
        let right = rect.right().min(ui.clip_rect().right() - 6.0);
        {
            let p = ui.painter();
            format_badge(ui, pos2(rect.left() + 20.0, rect.center().y), &name, &t, t.text);
            let text_clip = Rect::from_min_max(
                pos2(rect.left() + 42.0, rect.top()),
                pos2(right - 160.0, rect.bottom()),
            );
            let pc = p.with_clip_rect(text_clip.intersect(ui.clip_rect()));
            pc.text(
                pos2(rect.left() + 44.0, rect.center().y - 8.0),
                Align2::LEFT_CENTER,
                clip(&name, 120),
                theme::font(13.0),
                t.text,
            );
            pc.text(
                pos2(rect.left() + 44.0, rect.center().y + 9.0),
                Align2::LEFT_CENTER,
                clip(&folder, 200),
                theme::font(11.0),
                t.text_dim,
            );
            p.text(
                pos2(right - 72.0, rect.center().y),
                Align2::RIGHT_CENTER,
                fmt_date(r.opened),
                theme::font(11.0),
                t.text_dim,
            );
        }
        let resp = resp.on_hover_text(&r.path);
        let (pin_label, unpin) = if r.pinned {
            (l.t("Unpin", "Désépingler"), true)
        } else {
            (l.t("Pin to the list", "Épingler"), false)
        };
        let pin_rect = Rect::from_min_max(
            pos2(right - 64.0, rect.top() + 9.0),
            pos2(right - 38.0, rect.bottom() - 9.0),
        );
        if unpin {
            ui.painter()
                .rect_filled(pin_rect, CornerRadius::same(3), t.accent_soft);
        }
        if icon_button(ui, pin_rect, Icon::Pin, pin_label, &t) {
            app.prefs.toggle_pin(&r);
            app.save_prefs();
        }
        let x_rect = Rect::from_min_max(
            pos2(right - 32.0, rect.top() + 9.0),
            pos2(right - 6.0, rect.bottom() - 9.0),
        );
        if icon_button(
            ui,
            x_rect,
            Icon::Close,
            l.t("Remove from Recent", "Supprimer de la liste"),
            &t,
        ) {
            app.prefs.remove_recent(&r);
            app.save_prefs();
        } else if resp.clicked() {
            app.open_recent(&r);
        }
    }
    if total > limit && limit >= MAX_LIST_ROWS {
        dim(
            ui,
            &format!(
                "{} {}",
                total - limit,
                l.t("more: refine the search", "de plus : affinez la recherche")
            ),
        );
    }
}

/// A row of a page's own list on the left (Open's places, Export's tasks).
fn place_row(ui: &mut Ui, icon: Icon, label: &str, selected: bool, width: f32) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 44.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, label));
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, CornerRadius::same(4), t.selected);
        p.rect_stroke(
            rect,
            CornerRadius::same(4),
            Stroke::new(1.0, t.border),
            StrokeKind::Inside,
        );
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(4), t.hover);
    }
    icons::paint(
        p,
        Rect::from_center_size(pos2(rect.left() + 22.0, rect.center().y), vec2(22.0, 22.0)),
        icon,
        t.text,
        false,
    );
    let pc = p.with_clip_rect(rect.shrink(2.0));
    pc.text(
        pos2(rect.left() + 44.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        theme::font(13.0),
        t.text,
    );
    resp
}

/// A large square command button.
fn square_button(ui: &mut Ui, icon: Icon, label: &str, enabled: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(
        vec2(96.0, 80.0),
        if enabled { Sense::click() } else { Sense::hover() },
    );
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    let p = ui.painter();
    let hot = resp.hovered() && enabled;
    p.rect_filled(rect, CornerRadius::same(4), if hot { t.hover } else { t.card });
    p.rect_stroke(
        rect,
        CornerRadius::same(4),
        Stroke::new(1.0, if hot { t.ring } else { t.border }),
        StrokeKind::Inside,
    );
    icons::paint(
        p,
        Rect::from_center_size(pos2(rect.center().x, rect.top() + 26.0), vec2(26.0, 26.0)),
        icon,
        t.text,
        !enabled,
    );
    let ink = if enabled { t.text } else { t.text_faint };
    let galley = p.layout(label.to_string(), theme::font(12.0), ink, rect.width() - 8.0);
    p.galley(
        pos2(rect.center().x - galley.size().x / 2.0, rect.top() + 46.0),
        galley,
        ink,
    );
    resp
}

/// A square button with its title and description beside it.
fn task_row(ui: &mut Ui, icon: Icon, button: &str, title: &str, detail: &str, enabled: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let mut clicked = false;
    ui.horizontal_top(|ui| {
        clicked = square_button(ui, icon, button, enabled).clicked();
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.add_space(4.0);
            ui.label(RichText::new(title).font(theme::font(15.0)).color(t.text));
            if !detail.is_empty() {
                dim(ui, detail);
            }
        });
    });
    ui.add_space(14.0);
    clicked
}

/// A tile of Home's New row: a blank page drawn in code over its name.
fn new_tile(ui: &mut Ui, label: &str, template: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(176.0, 134.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(4), t.hover);
    }
    let thumb = Rect::from_min_size(rect.min + vec2(8.0, 10.0), vec2(160.0, 90.0));
    p.rect_filled(thumb, CornerRadius::same(2), egui::Color32::WHITE);
    p.rect_stroke(
        thumb,
        CornerRadius::same(2),
        Stroke::new(1.0, t.border),
        StrokeKind::Inside,
    );
    if template {
        // a page with a few visuals sketched in the brand colours
        for (i, c) in [icons::TEAL, icons::AMBER, theme::DARK_AMBER].iter().enumerate() {
            let x = thumb.left() + 20.0 + i as f32 * 22.0;
            let h = 20.0 + i as f32 * 14.0;
            p.rect_filled(
                Rect::from_min_max(
                    pos2(x, thumb.bottom() - 12.0 - h),
                    pos2(x + 14.0, thumb.bottom() - 12.0),
                ),
                CornerRadius::same(1),
                *c,
            );
        }
        p.rect_stroke(
            Rect::from_min_size(thumb.min + vec2(96.0, 14.0), vec2(48.0, 28.0)),
            CornerRadius::same(2),
            Stroke::new(1.0, egui::Color32::from_gray(190)),
            StrokeKind::Inside,
        );
    } else {
        icons::paint(
            p,
            Rect::from_center_size(thumb.center(), vec2(24.0, 24.0)),
            Icon::Plus,
            egui::Color32::from_gray(120),
            false,
        );
    }
    if resp.hovered() {
        p.rect_stroke(
            thumb,
            CornerRadius::same(2),
            Stroke::new(2.0, t.ring),
            StrokeKind::Outside,
        );
    }
    let galley = p.layout(label.to_string(), theme::font(12.0), t.text, rect.width() - 8.0);
    p.galley(
        pos2(rect.center().x - galley.size().x / 2.0, thumb.bottom() + 8.0),
        galley,
        t.text,
    );
    resp
}

// ---------------------------------------------------------------- pages

/// Home: the BIcraft mark, New (a blank PBIP project, or one from a template report), Open and
/// Connect, and the recent files with a search box. The same page is the start screen.
fn home(app: &mut App, ui: &mut Ui, bs: &mut Backstage, out: &mut Outcome) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let caps = app.shell.caps;
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
        theme::paint_logo(ui.painter(), r);
        ui.add_space(6.0);
        ui.label(RichText::new("BIcraft").font(theme::bold(26.0)).color(t.text));
    });
    ui.add_space(4.0);
    dim(
        ui,
        l.t(
            "Power BI files, read and checked in pure Rust.",
            "Les fichiers Power BI, lus et vérifiés en Rust pur.",
        ),
    );
    ui.add_space(18.0);
    heading(ui, l.t("New", "Nouveau"));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
        if new_tile(ui, l.t("Blank report", "Rapport vide"), false).clicked() {
            bs.template = None;
            bs.new_name = Some(l.t("Report", "Rapport").to_string());
        }
        // a template report (.pbit) needs the Open dialog to pick it
        if caps.open_files
            && app.shell.pick_file.is_some()
            && new_tile(
                ui,
                l.t("From a template (.pbit)…", "À partir d'un modèle (.pbit)…"),
                true,
            )
            .clicked()
            && let Some(p) = app.pick_file()
        {
            let name = split_path(&p.display().to_string()).0;
            let stem = name
                .rsplit_once('.')
                .map_or(name.as_str(), |(s, _)| s)
                .to_string();
            bs.template = Some(p);
            bs.new_name = Some(stem);
        }
    });
    ui.add_space(14.0);
    if caps.open_files || caps.connect {
        ui.horizontal(|ui| {
            if caps.open_files
                && widgets::primary(ui, l.t("Open a file…", "Ouvrir un fichier…"), true, "").clicked()
            {
                app.pick_and_open();
            }
            if caps.connect
                && ui
                    .button(l.t("Connect to server…", "Se connecter à un serveur…"))
                    .clicked()
            {
                app.connect = Some(app.connect_form(None));
            }
        });
        ui.add_space(4.0);
        ui.label(
            RichText::new(l.t(
                ".pbix, .pbit, .pbip (or a project folder) and .bim; or drop a file here.",
                ".pbix, .pbit, .pbip (ou un dossier de projet) et .bim ; ou déposez un fichier ici.",
            ))
            .small()
            .color(t.text_dim),
        );
        ui.add_space(18.0);
    }
    crate::recovery::home_section(app, ui);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(l.t("Recent", "Récents"))
                .font(theme::bold(16.0))
                .color(t.text),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            search(
                ui,
                &mut bs.query,
                l.t("Search for a file", "Rechercher un fichier"),
                280.0,
            );
        });
    });
    ui.add_space(6.0);
    let q = bs.query.clone();
    recent_list(app, ui, &q, HOME_RECENT);
    if shell_recent(app).len() > HOME_RECENT {
        ui.add_space(6.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            if arrow_link(ui, l.t("More files", "Plus de fichiers")) {
                out.page = Some(Page::Open);
            }
        });
    }
}

/// Open: Recent, Browse and Servers on the left; the chosen place on the right. The desktop's
/// Browse is the Open dialog; the browser's lists the files of the server workspace.
fn open_page(app: &mut App, ui: &mut Ui, bs: &mut Backstage) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let caps = app.shell.caps;
    let workspace = app.workspace();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(230.0);
            if place_row(
                ui,
                Icon::File,
                l.t("Recent", "Récents"),
                bs.place == Place::Recent,
                220.0,
            )
            .clicked()
            {
                bs.place = Place::Recent;
            }
            let browse_tip = if workspace.is_some() {
                l.t(
                    "The files of the server workspace",
                    "Les fichiers de l'espace de travail du serveur",
                )
            } else {
                l.t(
                    "Open a file from a folder (Ctrl+O)",
                    "Ouvrir un fichier d'un dossier (Ctrl+O)",
                )
            };
            if (caps.open_files || workspace.is_some())
                && place_row(
                    ui,
                    Icon::Folder,
                    l.t("Browse", "Parcourir"),
                    bs.place == Place::Browse,
                    220.0,
                )
                .on_hover_text(browse_tip)
                .clicked()
            {
                if workspace.is_some() {
                    bs.place = Place::Browse;
                    app.list_files();
                } else {
                    app.pick_and_open();
                }
            }
            if caps.connect
                && place_row(
                    ui,
                    Icon::Server,
                    l.t("Servers", "Serveurs"),
                    bs.place == Place::Servers,
                    220.0,
                )
                .clicked()
            {
                bs.place = Place::Servers;
            }
        });
        let h = ui.available_height().max(320.0);
        let (r, _) = ui.allocate_exact_size(vec2(1.0, h), Sense::hover());
        ui.painter()
            .line_segment([r.center_top(), r.center_bottom()], Stroke::new(1.0, t.separator));
        ui.add_space(24.0);
        ui.vertical(|ui| match bs.place {
            Place::Recent => {
                search(
                    ui,
                    &mut bs.query,
                    l.t("Search for a file", "Rechercher un fichier"),
                    380.0,
                );
                ui.add_space(12.0);
                heading(ui, l.t("Recent files", "Fichiers récents"));
                let q = bs.query.clone();
                recent_list(app, ui, &q, MAX_LIST_ROWS);
            }
            Place::Browse => browse(app, ui, bs),
            Place::Servers => servers(app, ui),
        });
    });
}

/// The browser's Browse: the server workspace's files (Power BI files first), with a search box.
fn browse(app: &mut App, ui: &mut Ui, bs: &mut Backstage) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    ui.horizontal(|ui| {
        search(
            ui,
            &mut bs.query,
            l.t("Search for a file", "Rechercher un fichier"),
            320.0,
        );
        if ui.button(l.t("Refresh", "Actualiser")).clicked() {
            app.list_files();
        }
    });
    ui.add_space(12.0);
    heading(ui, l.t("Workspace files", "Fichiers de l'espace de travail"));
    let files = match &app.files {
        None => {
            ui.spinner();
            return;
        }
        Some(Err(e)) => {
            ui.label(RichText::new(format!("✕ {}", clip(e, 300))).color(t.err));
            return;
        }
        Some(Ok(f)) => f.clone(),
    };
    let q = bs.query.trim().to_lowercase();
    let mut list: Vec<_> = files
        .into_iter()
        .filter(|f| !format_of(&f.path).is_empty() && format_of(&f.path) != "PNG")
        .filter(|f| q.is_empty() || f.path.to_lowercase().contains(&q))
        .collect();
    list.sort_by_key(|f| f.path.to_lowercase());
    if list.is_empty() {
        dim(ui, l.t("No Power BI file here.", "Aucun fichier Power BI ici."));
        return;
    }
    let width = ui.available_width().min(CONTENT_W);
    let total = list.len();
    for f in list.into_iter().take(MAX_LIST_ROWS) {
        let (name, folder) = split_path(&f.path);
        let (rect, resp) = ui.allocate_exact_size(vec2(width, 44.0), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
        if resp.hovered() {
            ui.painter().rect_filled(rect, CornerRadius::same(3), t.hover);
        }
        format_badge(ui, pos2(rect.left() + 20.0, rect.center().y), &name, &t, t.text);
        let p = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        p.text(
            pos2(rect.left() + 44.0, rect.center().y - 8.0),
            Align2::LEFT_CENTER,
            clip(&name, 120),
            theme::font(13.0),
            t.text,
        );
        p.text(
            pos2(rect.left() + 44.0, rect.center().y + 9.0),
            Align2::LEFT_CENTER,
            clip(&folder, 200),
            theme::font(11.0),
            t.text_dim,
        );
        let size = f.size.map(fmt_bytes).unwrap_or_default();
        let date = f.modified.get(..10).unwrap_or("").to_string();
        p.text(
            pos2(rect.right() - 8.0, rect.center().y),
            Align2::RIGHT_CENTER,
            format!("{date}   {size}"),
            theme::font(11.0),
            t.text_dim,
        );
        if resp.on_hover_text(&f.path).clicked() {
            app.open_ws_file(&f.path);
        }
    }
    if total > MAX_LIST_ROWS {
        dim(
            ui,
            &format!(
                "{} {}",
                total - MAX_LIST_ROWS,
                l.t("more: refine the search", "de plus : affinez la recherche")
            ),
        );
    }
}

/// The desktop's Servers: the remembered servers and Connect.
fn servers(app: &mut App, ui: &mut Ui) {
    let l = app.lang;
    if widgets::primary(
        ui,
        l.t("Connect to server…", "Se connecter à un serveur…"),
        true,
        "",
    )
    .clicked()
    {
        app.connect = Some(app.connect_form(None));
    }
    ui.add_space(12.0);
    heading(ui, l.t("Recent servers", "Serveurs récents"));
    if app.prefs.servers.is_empty() {
        dim(
            ui,
            l.t(
                "Servers you connect to show up here.",
                "Les serveurs auxquels vous vous connectez apparaissent ici.",
            ),
        );
    }
    let width = ui.available_width().min(CONTENT_W);
    for (u, w) in app.prefs.servers.clone() {
        let label = format!("{w} · {}", crate::source::host_of(&u));
        if place_row(ui, Icon::Server, &label, false, width)
            .on_hover_text(&u)
            .clicked()
        {
            app.connect = Some(app.connect_form(Some((u, w))));
        }
    }
}

/// The sum of the part sizes `file.inspect` lists for a package (uncompressed).
fn package_size(inspect: &Value) -> Option<u64> {
    let parts = inspect.pointer("/details/inventory/parts")?.as_array()?;
    Some(
        parts
            .iter()
            .filter_map(|p| p.get("bytes").and_then(Value::as_u64))
            .fold(0u64, u64::saturating_add),
    )
}

/// Info: the format, size, revision, counts, validation summary and unsaved state of the open
/// document, from its snapshot and `file.inspect`.
fn info(app: &mut App, ui: &mut Ui) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    app.inspect_if_needed();
    let Some(loaded) = app.loaded.clone() else {
        ui.spinner();
        return;
    };
    let snap = &loaded.snap;
    let path = snap.pointer("/source/path").and_then(Value::as_str).unwrap_or("");
    let (name, folder) = split_path(path);
    let fmt = snap
        .pointer("/source/format")
        .and_then(Value::as_str)
        .map(|f| f.split('-').next().unwrap_or(f).to_uppercase())
        .unwrap_or_else(|| "—".into());
    let tables = snap.pointer("/model/tables").and_then(Value::as_array);
    let count = |k: &str| -> usize {
        tables
            .into_iter()
            .flatten()
            .map(|tb| tb.get(k).and_then(Value::as_array).map_or(0, Vec::len))
            .fold(0usize, usize::saturating_add)
    };
    let pages = snap.pointer("/report/pages").and_then(Value::as_array);
    let visuals: usize = pages
        .into_iter()
        .flatten()
        .map(|p| p.get("visuals").and_then(Value::as_array).map_or(0, Vec::len))
        .fold(0usize, usize::saturating_add);
    let dirty = |k: &str| snap.pointer(&format!("/dirty/{k}")).and_then(Value::as_bool) == Some(true);
    let size = app
        .info
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .and_then(package_size)
        .map(fmt_bytes)
        .unwrap_or_else(|| "—".into());
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(420.0);
            ui.label(
                RichText::new(clip(if name.is_empty() { "—" } else { &name }, 80)).font(theme::bold(18.0)),
            );
            dim(ui, &clip(&folder, 160));
            ui.add_space(14.0);
            let v = snap.get("validation").cloned().unwrap_or(Value::Null);
            let n = |p: &str| v.pointer(p).and_then(Value::as_u64).unwrap_or(0);
            let (bad, e, w) = (
                n("/model/invalid_count"),
                n("/report/error_count"),
                n("/report/warning_count"),
            );
            let (mark, color, text) = if bad > 0 || e > 0 {
                (
                    "✕",
                    t.err,
                    l.t("This file has problems", "Ce fichier a des problèmes"),
                )
            } else if w > 0 {
                (
                    "⚠",
                    t.warn,
                    l.t("This file has warnings", "Ce fichier a des avertissements"),
                )
            } else {
                ("✓", t.ok, l.t("No problem found", "Aucun problème trouvé"))
            };
            if task_row(
                ui,
                Icon::Validate,
                l.t("Check the file", "Vérifier le fichier"),
                &format!("{mark} {text}"),
                &format!(
                    "{bad} {} · {e} {} · {w} {}",
                    l.t("invalid model objects", "objets du modèle non valides"),
                    l.t("report errors", "erreurs du rapport"),
                    l.t("warnings", "avertissements")
                ),
                true,
            ) {
                app.show_tab(crate::app::Tab::Validation);
                app.close_backstage_now = true;
            }
            let _ = color;
            let unsaved = dirty("model") || dirty("report");
            if task_row(
                ui,
                Icon::Save,
                l.t("Save", "Enregistrer"),
                if unsaved {
                    l.t("● Unsaved changes", "● Modifications non enregistrées")
                } else {
                    l.t(
                        "All changes are saved",
                        "Toutes les modifications sont enregistrées",
                    )
                },
                match (dirty("model"), dirty("report")) {
                    (true, true) => l.t(
                        "The model and the report have edits.",
                        "Le modèle et le rapport ont des modifications.",
                    ),
                    (true, false) => l.t("The model has edits.", "Le modèle a des modifications."),
                    (false, true) => l.t("The report has edits.", "Le rapport a des modifications."),
                    _ => "",
                },
                unsaved,
            ) {
                app.save();
            }
            // the File tab's other commands (the registry's File > File, Import, About)
            for id in INFO_COMMANDS {
                let Some(c) = crate::registry::find(id) else {
                    continue;
                };
                let st = crate::ribbon::state(app, id);
                let detail = match &st {
                    Ok(()) => c.label.to_string(),
                    Err(why) => format!("{} {why}", l.t("Unavailable:", "Indisponible :")),
                };
                if task_row(
                    ui,
                    c.icon(),
                    c.text(l),
                    c.text(l),
                    &clip(&detail, 120),
                    st.is_ok(),
                ) {
                    let p = crate::ribbon::prefill(app, c, None);
                    crate::ribbon::invoke(app, id, p);
                    app.close_backstage_now = true;
                }
            }
            crate::recovery::info_section(app, ui);
        });
        ui.add_space(24.0);
        ui.vertical(|ui| {
            heading(ui, l.t("Properties", "Propriétés"));
            egui::Grid::new("bicraft-info-props")
                .num_columns(2)
                .spacing(vec2(24.0, 6.0))
                .show(ui, |ui| {
                    let rows: Vec<(&str, String)> = vec![
                        (l.t("Format", "Format"), fmt.clone()),
                        (l.t("Size", "Taille"), size.clone()),
                        (
                            l.t("Revision", "Révision"),
                            snap.get("revision")
                                .and_then(Value::as_u64)
                                .unwrap_or(0)
                                .to_string(),
                        ),
                        (l.t("Tables", "Tables"), tables.map_or(0, Vec::len).to_string()),
                        (l.t("Columns", "Colonnes"), count("columns").to_string()),
                        (l.t("Measures", "Mesures"), count("measures").to_string()),
                        (l.t("Pages", "Pages"), pages.map_or(0, Vec::len).to_string()),
                        (l.t("Visuals", "Visuels"), visuals.to_string()),
                    ];
                    for (k, v) in rows {
                        ui.label(RichText::new(k).color(t.text_dim));
                        ui.label(v);
                        ui.end_row();
                    }
                });
            if let Some(Err(e)) = &app.info {
                ui.add_space(8.0);
                dim(ui, &clip(e, 300));
            }
        });
    });
}

/// The target of an Export task: the Save dialog on the desktop; a workspace-relative name typed
/// in the browser (`bs.target`).
fn target(app: &App, bs: &mut Backstage, ui: &mut Ui, default: &str, exts: &[&str]) -> Option<String> {
    let l = app.lang;
    if let Some(pick) = &app.shell.pick_save {
        let go = widgets::primary(ui, l.t("Choose where…", "Choisir où…"), true, "").clicked();
        return if go {
            pick(l, l.t("Save as", "Enregistrer sous"), default, exts).map(|p| p.display().to_string())
        } else {
            None
        };
    }
    if bs.target.is_empty() {
        bs.target = default.to_string();
    }
    let mut go = false;
    ui.horizontal(|ui| {
        ui.label(l.t("File name", "Nom du fichier"));
        let r = ui.add(
            egui::TextEdit::singleline(&mut bs.target)
                .char_limit(300)
                .desired_width(300.0),
        );
        if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            go = true;
        }
        let ok = crate::remote::safe_relative(bs.target.trim());
        if widgets::primary(
            ui,
            l.t("Run", "Lancer"),
            ok,
            l.t(
                "A workspace-relative name",
                "Un nom relatif à l'espace de travail",
            ),
        )
        .clicked()
        {
            go = true;
        }
    });
    let t = bs.target.trim().to_string();
    (go && crate::remote::safe_relative(&t)).then_some(t)
}

/// The suggested name of an export target: the open file's stem with `ext`.
fn default_name(path: &str, ext: &str, suffix: &str) -> String {
    let (name, _) = split_path(path);
    let stem = name.rsplit_once('.').map_or(name.as_str(), |(s, _)| s);
    let stem = if stem.is_empty() { "report" } else { stem };
    format!("{stem}{suffix}.{ext}")
}

/// Export: Save a copy as, Convert (PBIX, PBIT, PBIP), Render the page to PNG, Compare with
/// another file; each runs its engine command and shows its output.
fn export(app: &mut App, ui: &mut Ui, bs: &mut Backstage) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let path = app.source_path();
    let dirty = app.loaded.as_ref().is_some_and(|x| {
        ["model", "report", "queries"]
            .iter()
            .any(|k| x.snap.pointer(&format!("/dirty/{k}")).and_then(Value::as_bool) == Some(true))
    });
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(270.0);
            for task in Task::ALL {
                if place_row(ui, task.icon(), task.label(l), bs.task == task, 260.0).clicked() && bs.task != task {
                    bs.task = task;
                    bs.target.clear();
                }
            }
        });
        let h = ui.available_height().max(320.0);
        let (r, _) = ui.allocate_exact_size(vec2(1.0, h), Sense::hover());
        ui.painter()
            .line_segment([r.center_top(), r.center_bottom()], Stroke::new(1.0, t.separator));
        ui.add_space(24.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(bs.task.label(l)).font(theme::bold(16.0)));
            ui.add_space(8.0);
            match bs.task {
                Task::SaveAs => {
                    dim(ui, l.t(
                        "Saves the model and the report, unsaved edits included, to a new file that becomes the open one.",
                        "Enregistre le modèle et le rapport, modifications comprises, dans un nouveau fichier qui devient le fichier ouvert.",
                    ));
                    ui.add_space(8.0);
                    let ext = path.rsplit_once('.').map_or("pbip", |(_, e)| e).to_ascii_lowercase();
                    let ext = if ["pbix", "pbit", "pbip", "bim"].contains(&ext.as_str()) { ext } else { "pbip".into() };
                    if let Some(to) = target(app, bs, ui, &default_name(&path, &ext, " (copy)"), &["pbix", "pbit", "pbip", "bim"]) {
                        app.run(actions::file_save_as(&to), Then::Output(Task::SaveAs.id()));
                    }
                }
                Task::Convert => {
                    dim(ui, l.t(
                        "Writes the saved file in another format; the open file is not touched.",
                        "Écrit le fichier enregistré dans un autre format ; le fichier ouvert n'est pas modifié.",
                    ));
                    if dirty {
                        ui.label(RichText::new(l.t(
                            "⚠ Unsaved edits are not in the saved file: save first.",
                            "⚠ Les modifications non enregistrées ne sont pas dans le fichier : enregistrez d'abord.",
                        )).color(t.warn));
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        for (id, _, label) in CONVERT_FORMATS {
                            if ui.selectable_label(bs.format == id, label).clicked() && bs.format != id {
                                bs.format = id;
                                bs.target.clear();
                            }
                        }
                    });
                    ui.add_space(6.0);
                    let ext = CONVERT_FORMATS.iter().find(|f| f.0 == bs.format).map_or("pbix", |f| f.1);
                    let can = !path.is_empty();
                    if !can {
                        dim(ui, l.t("Save the file first.", "Enregistrez d'abord le fichier."));
                    } else if let Some(to) = target(app, bs, ui, &default_name(&path, ext, ""), &[ext]) {
                        let fmt = bs.format;
                        app.run(actions::file_convert(&path, &to, fmt), Then::Output(Task::Convert.id()));
                    }
                }
                Task::Redact => redact(app, ui, bs, &path, dirty),
                Task::Render => {
                    let names = app.page_names();
                    let page = names.get(app.active_page()).cloned();
                    match page {
                        None => dim(ui, l.t("This file has no report page.", "Ce fichier n'a pas de page de rapport.")),
                        Some((name, label, _)) => {
                            dim(ui, &format!("{} “{}”.", l.t("Draws the page shown:", "Dessine la page affichée :"), clip(&label, 80)));
                            ui.add_space(8.0);
                            let safe: String = label
                                .chars()
                                .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' })
                                .take(60)
                                .collect();
                            let def = format!("{}.png", if safe.trim().is_empty() { "page" } else { safe.trim() });
                            if let Some(to) = target(app, bs, ui, &def, &["png"]) {
                                let then = exported(app, Task::Render, &to);
                                app.run(actions::report_render(&name, &to), then);
                            }
                        }
                    }
                }
                Task::Pdf => export_pdf(app, ui, bs, &path),
                Task::Print => print(app, ui, bs, &path),
                Task::Data => data_setting(app, ui),
                Task::Compare => {
                    dim(ui, l.t(
                        "Compares the saved file with another one through the model: tables, measures, pages, visuals.",
                        "Compare le fichier enregistré avec un autre à travers le modèle : tables, mesures, pages, visuels.",
                    ));
                    ui.add_space(8.0);
                    let other = if app.shell.pick_file.is_some() {
                        if widgets::primary(ui, l.t("Choose a file…", "Choisir un fichier…"), !path.is_empty(), "").clicked() {
                            app.pick_file().map(|p| p.display().to_string())
                        } else {
                            None
                        }
                    } else {
                        target(app, bs, ui, "", &[])
                    };
                    if let Some(b) = other
                        && !path.is_empty()
                    {
                        app.run(actions::file_diff(&path, &b), Then::Output(Task::Compare.id()));
                    }
                }
            }
            ui.add_space(14.0);
            output(app, ui, bs.task);
        });
    });
}

/// Redact a copy for sharing (`file.redact`): the saved file written with its connection
/// strings, credentials, servers, databases, paths and URLs (and, on request, its stored text
/// values) replaced by placeholders; the open file is not touched.
fn redact(app: &mut App, ui: &mut Ui, bs: &mut Backstage, path: &str, dirty: bool) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    dim(ui, l.t(
        "Writes a copy of the saved file for a bug report: connection strings, credentials, server and database names, file paths and URLs become placeholders (the same value always gets the same one). Every byte of the copy is checked; the open file is not touched.",
        "Écrit une copie du fichier enregistré pour un rapport de bogue : chaînes de connexion, identifiants, noms de serveurs et de bases, chemins de fichiers et URL deviennent des valeurs de remplacement (une même valeur reçoit toujours la même). Chaque octet de la copie est vérifié ; le fichier ouvert n'est pas modifié.",
    ));
    if dirty {
        ui.label(
            RichText::new(l.t(
                "⚠ Unsaved edits are not in the saved file: save first.",
                "⚠ Les modifications non enregistrées ne sont pas dans le fichier : enregistrez d'abord.",
            ))
            .color(t.warn),
        );
    }
    ui.add_space(8.0);
    ui.checkbox(
        &mut bs.redact_data,
        l.t(
            "Replace the stored text values too (PBIX)",
            "Remplacer aussi les valeurs texte stockées (PBIX)",
        ),
    );
    ui.add_space(6.0);
    let ext = path
        .rsplit_once('.')
        .map_or("pbip", |(_, e)| e)
        .to_ascii_lowercase();
    let ext = if ["pbix", "pbit", "pbip"].contains(&ext.as_str()) {
        ext
    } else {
        "pbip".into()
    };
    if path.is_empty() {
        dim(ui, l.t("Save the file first.", "Enregistrez d'abord le fichier."));
    } else if let Some(to) = target(
        app,
        bs,
        ui,
        &default_name(path, &ext, " (redacted)"),
        &[ext.as_str()],
    ) {
        let then = exported(app, Task::Redact, &to);
        app.run(actions::file_redact(path, &to, bs.redact_data), then);
    }
}

/// How an export's answer is kept: its task's output, then (the browser) the download of the
/// workspace file `to` it wrote.
fn exported(app: &App, task: Task, to: &str) -> Then {
    if app.shell.download.is_some() {
        Then::Download(task.id(), to.to_string())
    } else {
        Then::Output(task.id())
    }
}

/// Export to PDF: every page (hidden ones on request), the page shown, or a range; one PDF page
/// per report page (`report.export_pdf`).
fn export_pdf(app: &mut App, ui: &mut Ui, bs: &mut Backstage, path: &str) {
    let l = app.lang;
    let Some(pages) = pdf_pages(
        app,
        ui,
        bs,
        l.t(
            "Writes the report's pages to one PDF file, a PDF page per report page: shapes and charts as vectors, text you can select and search, with its fonts embedded, a bookmark per page and a structure for screen readers.",
            "Écrit les pages du rapport dans un fichier PDF, une page PDF par page du rapport : formes et graphiques vectoriels, texte sélectionnable et cherchable, polices incorporées, un signet par page et une structure pour les lecteurs d'écran.",
        ),
    ) else {
        return;
    };
    ui.checkbox(
        &mut bs.pdf_archive,
        l.t(
            "Archival PDF (PDF/A-2b): metadata and sRGB colours for long-term keeping",
            "PDF d'archivage (PDF/A-2b) : métadonnées et couleurs sRGB pour la conservation à long terme",
        ),
    );
    ui.add_space(8.0);
    let ext = "pdf";
    if let Some(to) = target(app, bs, ui, &default_name(path, ext, ""), &[ext]) {
        let then = exported(app, Task::Pdf, &to);
        app.run(
            actions::report_export_pdf(&to, pages, bs.pdf_hidden, bs.pdf_archive),
            then,
        );
    }
}

/// Print: the pages picked as Export to PDF picks them, written to a PDF that the operating
/// system's PDF application opens to print (the desktop), or that the browser prints
/// (`report.print`).
fn print(app: &mut App, ui: &mut Ui, bs: &mut Backstage, path: &str) {
    let l = app.lang;
    let browser = app.shell.print.is_some();
    let what = if browser {
        l.t(
            "Writes the pages to a PDF in the workspace, then opens your browser's print dialog on it.",
            "Écrit les pages dans un PDF de l'espace de travail, puis ouvre dessus la boîte d'impression de votre navigateur.",
        )
    } else {
        l.t(
            "Writes the pages to a PDF, then opens it in your PDF application, where you print it.",
            "Écrit les pages dans un PDF, puis l'ouvre dans votre application PDF, où vous l'imprimez.",
        )
    };
    let Some(pages) = pdf_pages(app, ui, bs, what) else {
        return;
    };
    if widgets::primary(ui, l.t("Print…", "Imprimer…"), true, "").clicked() {
        if browser {
            let to = default_name(path, "pdf", "-print");
            let then = Then::Print(Task::Print.id(), to.clone());
            app.run(actions::report_print(pages, bs.pdf_hidden, Some(&to)), then);
        } else {
            app.run(
                actions::report_print(pages, bs.pdf_hidden, None),
                Then::Output(Task::Print.id()),
            );
        }
    }
}

/// The page choice of Export to PDF and Print, under `what` the task does: every page (hidden
/// ones on request), the page shown, or a range, as `report.export_pdf` takes `pages`; `None`
/// while there is nothing to export (no page, a range that is not one).
fn pdf_pages(app: &App, ui: &mut Ui, bs: &mut Backstage, what: &str) -> Option<Value> {
    let l = app.lang;
    let names = app.page_names();
    if names.is_empty() {
        dim(
            ui,
            l.t(
                "This file has no report page.",
                "Ce fichier n'a pas de page de rapport.",
            ),
        );
        return None;
    }
    dim(ui, what);
    ui.add_space(8.0);
    let current = names
        .get(app.active_page())
        .map(|p| p.1.clone())
        .unwrap_or_default();
    ui.radio_value(&mut bs.pdf_pages, "all", l.t("All pages", "Toutes les pages"));
    ui.radio_value(
        &mut bs.pdf_pages,
        "current",
        format!(
            "{} “{}”",
            l.t("Current page only:", "Page actuelle seulement :"),
            clip(&current, 60)
        ),
    );
    ui.horizontal(|ui| {
        ui.radio_value(&mut bs.pdf_pages, "range", l.t("Pages:", "Pages :"));
        let r = ui.add(
            egui::TextEdit::singleline(&mut bs.pdf_range)
                .char_limit(60)
                .desired_width(120.0)
                .hint_text("1-3, 5"),
        );
        if r.changed() {
            bs.pdf_pages = "range";
        }
        dim(ui, &format!("{} {}", l.t("of", "sur"), names.len()));
    });
    let hidden = names.iter().filter(|p| p.2).count();
    if hidden > 0 {
        ui.checkbox(
            &mut bs.pdf_hidden,
            format!(
                "{} ({hidden})",
                l.t("Include hidden pages", "Inclure les pages masquées")
            ),
        );
    }
    ui.add_space(8.0);
    let range = bs.pdf_range.trim().to_string();
    let range_ok = !range.is_empty()
        && range
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '-' | ',' | ' '));
    if bs.pdf_pages == "range" && !range_ok {
        dim(
            ui,
            l.t(
                "Type page numbers, as 1-3, 5.",
                "Tapez des numéros de page, comme 1-3, 5.",
            ),
        );
        return None;
    }
    Some(match bs.pdf_pages {
        "current" => names
            .get(app.active_page())
            .map_or(Value::from("current"), |p| Value::from(p.0.clone())),
        // a single number is a 1-based page, as a print dialog takes it
        "range" if !range.contains(['-', ',']) => Value::from(format!("{range}-{range}")),
        "range" => Value::from(range),
        _ => Value::from("all"),
    })
}

/// The report's Export data setting (`report.settings {export_data}`): what a visual's Export data
/// may write.
fn data_setting(app: &mut App, ui: &mut Ui) {
    let l = app.lang;
    dim(ui, l.t(
        "A visual's data is exported from its More options (⋯) menu or right-click: Export data. This setting of the report says what may be exported.",
        "Les données d'un visuel s'exportent depuis son menu Plus d'options (⋯) ou le clic droit : Exporter les données. Ce paramètre du rapport dit ce qui peut l'être.",
    ));
    ui.add_space(8.0);
    let mode = crate::export_dialog::mode(app);
    let can = crate::ribbon::state(app, "report.settings");
    let mut pick: Option<&str> = None;
    for (id, en, fr) in [
        (
            "AllowSummarized",
            "Allow summarized data only",
            "Autoriser les données résumées seulement",
        ),
        (
            "AllowSummarizedAndUnderlying",
            "Allow summarized and underlying data",
            "Autoriser les données résumées et sous-jacentes",
        ),
        (
            "None",
            "Don't allow exporting data",
            "Ne pas autoriser l'export des données",
        ),
    ] {
        let r = ui.add_enabled(can.is_ok(), egui::RadioButton::new(mode == id, l.t(en, fr)));
        if r.clicked() && mode != id {
            pick = Some(id);
        }
    }
    if let Err(why) = &can {
        dim(ui, &format!("{} {why}", l.t("Unavailable:", "Indisponible :")));
    }
    if let Some(m) = pick {
        app.run(actions::report_export_setting(m), Then::Output(Task::Data.id()));
    }
}

/// The last output of an Export task: what was written, the warnings, the diff's text.
fn output(app: &App, ui: &mut Ui, task: Task) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let Some((id, r)) = &app.output else { return };
    if *id != task.id() {
        return;
    }
    widgets::card(&t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        match r {
            None => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(l.t("Running…", "En cours…"));
                });
            }
            Some(Err(e)) => {
                ui.label(RichText::new(format!("✕ {}", clip(e, 600))).color(t.err));
            }
            Some(Ok(v)) => {
                let written = ["path", "to", "out"]
                    .iter()
                    .find_map(|k| v.get(*k).and_then(Value::as_str))
                    .map(str::to_string)
                    .or_else(|| {
                        v.get("pages")
                            .and_then(Value::as_array)
                            .and_then(|a| a.first())
                            .and_then(|p| p.get("out").or_else(|| p.get("path")))
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    });
                if let Some(text) = v.get("text").and_then(Value::as_str) {
                    let identical = v.get("identical").and_then(Value::as_bool) == Some(true);
                    ui.label(
                        RichText::new(if identical {
                            l.t(
                                "✓ The two files are the same.",
                                "✓ Les deux fichiers sont identiques.",
                            )
                        } else {
                            l.t("Differences:", "Différences :")
                        })
                        .color(if identical { t.ok } else { t.text }),
                    );
                    egui::ScrollArea::vertical()
                        .id_salt("bicraft-diff")
                        .max_height(360.0)
                        .show(ui, |ui| {
                            for line in text.lines().take(2000) {
                                ui.label(RichText::new(clip(line, 300)).monospace());
                            }
                        });
                } else {
                    ui.label(
                        RichText::new(match &written {
                            Some(p) => format!("✓ {} {}", l.t("Written:", "Écrit :"), clip(p, 300)),
                            None => l.t("✓ Done.", "✓ Terminé.").to_string(),
                        })
                        .color(t.ok),
                    );
                    if v.get("opened").and_then(Value::as_bool) == Some(true) {
                        ui.label(l.t(
                            "Opened in your PDF application: print it from there.",
                            "Ouvert dans votre application PDF : imprimez-le depuis celle-ci.",
                        ));
                    }
                    // Redact a copy: what was replaced, and the check of the written bytes
                    if let Some(n) = v.get("redacted").and_then(Value::as_u64) {
                        let data = v.get("data_values").and_then(Value::as_u64).unwrap_or(0);
                        ui.label(format!(
                            "{n} {} · {data} {} · {}",
                            l.t("values replaced", "valeurs remplacées"),
                            l.t("data values", "valeurs de données"),
                            l.t(
                                "no original value left in the copy",
                                "aucune valeur d'origine dans la copie"
                            ),
                        ));
                        for n in v
                            .get("notes")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .take(20)
                        {
                            if let Some(s) = n.as_str() {
                                dim(ui, &clip(s, 300));
                            }
                        }
                    }
                }
                for w in v
                    .get("warnings")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .take(50)
                {
                    let s = w.as_str().map_or_else(|| w.to_string(), str::to_string);
                    ui.label(RichText::new(format!("⚠ {}", clip(&s, 300))).color(t.warn));
                }
            }
        }
    });
}

/// New project: the name, then (desktop) the folder dialog; the project opens in a new tab.
fn new_dialog(app: &mut App, ctx: &egui::Context, bs: &mut Backstage) {
    let l = app.lang;
    let Some(name) = &mut bs.new_name else { return };
    let mut go = false;
    let mut cancel = ctx.input(|i| i.key_pressed(Key::Escape));
    let title = if bs.template.is_some() {
        l.t(
            "New project from a template",
            "Nouveau projet à partir d'un modèle",
        )
    } else {
        l.t("New PBIP project", "Nouveau projet PBIP")
    };
    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, -40.0))
        .show(ctx, |ui| {
            ui.set_min_width(340.0);
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    vec2(120.0, 22.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_min_width(120.0);
                        ui.label(l.t("Project name", "Nom du projet"));
                    },
                );
                ui.add(
                    egui::TextEdit::singleline(name)
                        .char_limit(100)
                        .desired_width(220.0),
                );
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new(if app.shell.pick_folder.is_some() {
                    l.t(
                        "Next, choose the folder it goes in.",
                        "Ensuite, choisissez son dossier.",
                    )
                } else {
                    l.t(
                        "Created at the root of the workspace.",
                        "Créé à la racine de l'espace de travail.",
                    )
                })
                .small(),
            );
            ui.add_space(8.0);
            let w = ui.available_width();
            ui.allocate_ui_with_layout(
                vec2(w, 28.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    let ok = valid_project_name(name);
                    let enter = ui.input(|i| i.key_pressed(Key::Enter));
                    if widgets::primary(
                        ui,
                        l.t("Create", "Créer"),
                        ok,
                        l.t(
                            "A name without / \\ : * ? \" < > |",
                            "Un nom sans / \\ : * ? \" < > |",
                        ),
                    )
                    .clicked()
                        || (ok && enter)
                    {
                        go = true;
                    }
                    if ui
                        .add(egui::Button::new(l.t("Cancel", "Annuler")).min_size(vec2(72.0, 24.0)))
                        .clicked()
                    {
                        cancel = true;
                    }
                },
            );
        });
    if go {
        let name = name.trim().to_string();
        let template = bs.template.take();
        bs.new_name = None;
        if app.new_project(&name, template) {
            app.close_backstage_now = true;
        }
    } else if cancel {
        bs.new_name = None;
        bs.template = None;
    }
}

/// A project name Desktop can use as a folder name.
pub fn valid_project_name(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && s.chars().count() <= 100
        && !s.chars().any(|c| c.is_control() || "/\\:*?\"<>|".contains(c))
        && s != "."
        && s != ".."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_formats_dates_and_names() {
        assert_eq!(
            split_path(r"C:\x\Sales.pbix"),
            ("Sales.pbix".into(), r"C:\x".into())
        );
        assert_eq!(split_path("a/b/Shop.pbip"), ("Shop.pbip".into(), "a/b".into()));
        assert_eq!(split_path("Shop.pbip"), ("Shop.pbip".into(), String::new()));
        assert_eq!(split_path("a/"), ("a/".into(), String::new()));
        assert_eq!(split_path(""), (String::new(), String::new()));
        assert_eq!(format_of("x.PBIX"), "PBIX");
        assert_eq!(format_of("model.bim"), "BIM");
        assert_eq!(format_of("noext"), "");
        assert_eq!(fmt_date(0), "—");
        assert_eq!(fmt_date(1_762_214_400), "2025-11-04");
        assert_eq!(fmt_date(951_782_400), "2000-02-29");
        // year boundaries, a leap day's last second, a century that is not leap (G37)
        assert_eq!(fmt_date(946_684_799), "1999-12-31");
        assert_eq!(fmt_date(946_684_800), "2000-01-01");
        assert_eq!(fmt_date(1_709_251_199), "2024-02-29");
        assert_eq!(fmt_date(4_107_542_399), "2100-02-28");
        assert_eq!(fmt_date(4_107_542_400), "2100-03-01");
        assert!(!fmt_date(u64::MAX).is_empty());
        assert_eq!(fmt_bytes(10), "10 bytes");
        assert_eq!(fmt_bytes(1536), "1.5 KB");
        assert!(fmt_bytes(u64::MAX).ends_with("TB"));
        assert!(valid_project_name("Sales 2026"));
        for bad in ["", " ", "a/b", "a:b", "..", "a\u{7}b"] {
            assert!(!valid_project_name(bad), "{bad:?}");
        }
        assert_eq!(default_name("C:/x/Sales.pbix", "pbip", ""), "Sales.pbip");
        assert_eq!(default_name("", "png", ""), "report.png");
        let recent = vec![
            Recent {
                path: "a/Sales.pbix".into(),
                ..Recent::default()
            },
            Recent {
                path: "b/Shop.pbip".into(),
                ..Recent::default()
            },
        ];
        assert_eq!(matching(&recent, "shop").len(), 1);
        assert_eq!(matching(&recent, " ").len(), 2);
        assert_eq!(matching(&recent, "nope").len(), 0);
        assert_eq!(entries(false).len(), 2);
        assert!(entries(true).contains(&Nav::Close));
    }
}
