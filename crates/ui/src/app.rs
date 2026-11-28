//! The window, in DeckCraft's chrome (docs/design.md): a title row, the command registry's tabs
//! over a ribbon card (`crate::ribbon`), the page on its canvas with the page tabs under it, a status bar.
//! Every click on the page goes through the renderer's hit map ([`bicraft_render::hits`]) to
//! the `view.interact` action the live viewer would send; the source runs it and sends the next
//! snapshot. The same code runs in every shell; what a shell cannot do ([`Caps`]) is not shown.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bicraft_render::hits::{HitAction, hit_at};
use egui::{Color32, CornerRadius, Pos2, Rect, RichText, Sense, TextureHandle, TextureOptions, Vec2, pos2};
use serde_json::{Value, json};

use crate::actions::{self, Call};
use crate::backstage::{self, Backstage};
use crate::i18n::Lang;
use crate::icons::Icon;
use crate::keyboard::Section;
use crate::model_view::ModelView;
use crate::page_tabs::{self, PageTabs};
use crate::pages::{Key, Pages, Rendered};
use crate::prefs::{Prefs, Recent};
use crate::shell::{Caps, Shell};
use crate::source::{Loaded, Notify, Origin, Request, Server, Update, Worker, WsFile};
use crate::theme::{self, Brightness, Tokens};
use crate::widgets;

/// Height of the title row (DeckCraft's `TITLE_H`).
pub const TITLE_H: f32 = 38.0;
/// Height of the status bar.
pub const STATUS_H: f32 = 28.0;

/// Most rows a Model or Validation list shows.
const MAX_ROWS: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Report,
    /// The Data view (Desktop's table view, [`crate::data_view`]).
    Data,
    Model,
    /// The DAX query view ([`crate::dax_query`]).
    Query,
    /// The TMDL view ([`crate::tmdl_view`]).
    Tmdl,
    Validation,
}

impl Tab {
    /// The preference value (`report`, `model`, `validation`).
    pub fn name(self) -> &'static str {
        match self {
            Tab::Report => "report",
            Tab::Data => "data",
            Tab::Model => "model",
            Tab::Query => "query",
            Tab::Tmdl => "tmdl",
            Tab::Validation => "validation",
        }
    }

    pub fn from_name(s: &str) -> Tab {
        match s {
            "data" => Tab::Data,
            "model" => Tab::Model,
            "query" => Tab::Query,
            "tmdl" => Tab::Tmdl,
            "validation" => Tab::Validation,
            _ => Tab::Report,
        }
    }
}

/// The ribbon's tab: the registry's tabs (`crate::ribbon`); Report and Model show their view.
pub use crate::registry::Tab as RibbonTab;

/// A popup the viewer opens on a slicer (dropdown items, range inputs, relative date, text).
#[derive(Debug, Clone)]
pub struct Popup {
    pub spec: Value,
    pub at: Pos2,
    pub from: String,
    pub to: String,
    pub text: String,
    pub range: String,
    pub duration: String,
    pub period: String,
    /// Opened this frame: the click that opened it does not close it.
    pub fresh: bool,
}

impl Popup {
    fn new(spec: Value, at: Pos2) -> Popup {
        let s = |k: &str| match spec.get(k) {
            Some(Value::String(t)) => t.chars().take(10).collect(),
            Some(Value::Number(n)) => n.to_string(),
            _ => String::new(),
        };
        let (from, to) = (s("from"), s("to"));
        let text = spec.get("text").and_then(Value::as_str).unwrap_or("").to_string();
        let range = spec
            .get("range")
            .and_then(Value::as_str)
            .filter(|r| ["last", "next", "this"].contains(r))
            .unwrap_or("last")
            .to_string();
        let duration = spec
            .get("duration")
            .and_then(Value::as_f64)
            .filter(|d| d.is_finite() && *d >= 1.0)
            .map_or_else(|| "1".to_string(), |d| format!("{}", d.round() as i64));
        let period = spec
            .get("period")
            .and_then(Value::as_str)
            .filter(|p| PERIODS.contains(p))
            .unwrap_or("months")
            .to_string();
        Popup {
            spec,
            at,
            from,
            to,
            text,
            range,
            duration,
            period,
            fresh: true,
        }
    }

    /// The action `{action, page, visual, …extra}` of this popup's slicer.
    fn action(&self, action: &str, extra: Value) -> Value {
        let mut a = json!({"action": action, "page": self.spec["page"], "visual": self.spec["visual"]});
        if let (Some(o), Value::Object(e)) = (a.as_object_mut(), extra) {
            o.extend(e);
        }
        a
    }
}

/// The relative-date periods `slicer_relative` takes (the viewer's list).
pub const PERIODS: [&str; 7] = [
    "days",
    "weeks",
    "calendar weeks",
    "months",
    "calendar months",
    "years",
    "calendar years",
];

/// File > Connect to server.
#[derive(Debug, Clone, Default)]
pub struct ConnectForm {
    pub url: String,
    pub workspace: String,
    pub token: String,
}

/// Where the page was drawn last: its screen rectangle and points per report pixel.
#[derive(Debug, Clone, Copy)]
pub struct Placed {
    pub rect: Rect,
    pub zoom: f32,
}

/// How the canvas scales the page: the page's `displayOption` (View › Page view) or the status
/// bar's zoom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewZoom {
    /// The whole page in the window (`FitToPage`, the default).
    FitPage,
    /// The page's width in the window, scrolled down (`FitToWidth`).
    FitWidth,
    /// One report pixel per point, scrolled (`ActualSize`).
    Actual,
    /// The zoom slider's percentage ([`ZOOM_MIN`] to [`ZOOM_MAX`]), scrolled.
    Percent(u16),
}

/// The zoom slider's range and step (percent).
pub const ZOOM_MIN: u16 = 10;
pub const ZOOM_MAX: u16 = 400;
pub const ZOOM_STEP: u16 = 10;
/// Largest side of the zoomed page on the screen (points): a larger zoom is held there.
const MAX_ZOOMED: f32 = 8000.0;

impl ViewZoom {
    /// The mode a page's `displayOption` names (`FitToPage` for anything else).
    pub fn of_page(display_option: Option<&str>) -> ViewZoom {
        match display_option {
            Some("FitToWidth") => ViewZoom::FitWidth,
            Some("ActualSize") => ViewZoom::Actual,
            _ => ViewZoom::FitPage,
        }
    }
}

/// What to do with a command's answer ([`App::run`]).
#[derive(Debug, Clone, PartialEq)]
pub enum Then {
    /// Nothing more (an error goes to the error bar).
    Nothing,
    /// Keep it as the Info page's `file.inspect`.
    Inspect,
    /// Keep it as the output of a Backstage Export task (its id).
    Output(&'static str),
    /// As [`Then::Output`], then the shell's download of the workspace file written (the
    /// browser's exports, [`crate::shell::Shell::download`]).
    Download(&'static str, String),
    /// As [`Then::Output`], then the shell's print of the workspace PDF written (the browser's
    /// Print, [`crate::shell::Shell::print`]).
    Print(&'static str, String),
    /// The page spec of Duplicate's first step: apply it as a new page.
    Duplicate { key: String, name: String },
    /// Show the page the command created (`page.add`).
    ShowPage,
    /// Show the answer of a read command run from a button (its id).
    Show(&'static str),
    /// The answer of the open parameters form ([`crate::form`]).
    Form,
    /// The names a picker of the open form asked for ([`crate::form::looked_up`]).
    Lookup(crate::registry::Obj),
    /// The answer of a `recovery.*` command or an automatic save ([`crate::recovery::answer`]).
    Recovery(&'static str),
    /// The values of a column for the Filters pane (field text, the snapshot revision asked at).
    FilterValues(String, u64),
    /// Select the visual a command created (`visual.add`: its `page` and `visual`).
    SelectVisual,
    /// The answer of one of the formula bar's commands ([`crate::formula::answer`]).
    Formula(crate::formula::Step),
    /// `visual.copy`'s clip: to the clipboards ([`crate::canvas::copied`]).
    Clipboard,
    /// Select what `visual.paste` or `visual.duplicate` made ([`crate::canvas::pasted`]).
    SelectPasted,
    /// The answer of one of the Data view's commands ([`crate::data_view::answer`]).
    Data(crate::data_view::Step),
    /// The answer of a Get data, Navigator or Enter data command ([`crate::get_data::answer`]).
    GetData(crate::get_data::Step),
    /// The answer of a Data source settings command ([`crate::source_settings::answer`]).
    SourceSettings(crate::source_settings::Step),
    /// The answer of one of the Power Query editor's commands ([`crate::pq_editor::answer`]).
    Pq(crate::pq_editor::Step),
    /// The answer of a rich content command (`crate::rich`: insert, edit, format).
    Rich(crate::rich::Step),
    /// The selected visual's own formatting for the Format visual tab
    /// ([`crate::format_pane::props_answer`]), read at that snapshot revision.
    FormatProps {
        page: String,
        visual: String,
        revision: u64,
    },
    /// The answer of an edit of the Format visual tab ([`crate::format_pane::edit_answer`]).
    FormatEdit { page: String, visual: String },
    /// The values of the column the Data colours card lists colours for
    /// ([`crate::format_pane::values_answer`]): its field text, the revision asked at.
    FormatValues { field: String, revision: u64 },
    /// The answer of a Performance analyzer command ([`crate::perf_pane::answer`]).
    Perf(&'static str),
    /// The answer of the visual calculations window's Apply ([`crate::viscalc_dialog::answer`]).
    VisualCalc,
    /// The visual calculations window's live preview of the name and expression in its key
    /// ([`crate::viscalc_dialog::preview_answer`]).
    VisualCalcPreview(String),
    /// The answer of Autodetect relationships ([`crate::autodetect::answer`]).
    Autodetect,
    /// The answers of Manage relationships and its Create / Edit window
    /// ([`crate::relationships_dialog::answer`]).
    Rels(crate::relationships_dialog::Step),
    /// The answers of Manage security roles' Check and Save ([`crate::roles_dialog::answer`]).
    Roles(crate::roles_dialog::Step),
    /// The values of the column the Groups dialog groups ([`crate::group_dialog::values_answer`]).
    GroupValues,
    /// The values of the Quick measures dialog's filter column ([`crate::quick_dialog::values_answer`]).
    QuickValues(String),
    /// The answer of the DAX query view's Run or Update model ([`crate::dax_query::answer`]).
    DaxQuery(crate::dax_query::Step),
    /// The answer of the TMDL view's Script or Apply ([`crate::tmdl_view::answer`]).
    Tmdl(crate::tmdl_view::Step),
}

/// A question asked before an action that cannot be undone from the window.
#[derive(Debug, Clone, PartialEq)]
pub enum Confirm {
    DeletePage {
        name: String,
        label: String,
    },
    /// Close the document at this index although it has unsaved edits.
    CloseDoc(usize),
}

/// What the window keeps of one open document: its source and what it shows. The active
/// document's lives in [`App`]'s own fields (so every view reads `app.loaded`…); the others wait
/// here, their sources still running, and swap in when their tab is picked.
#[derive(Default)]
pub struct Doc {
    origin: Option<Origin>,
    worker: Option<Worker>,
    pending: Option<Origin>,
    loaded: Option<Loaded>,
    error: Option<String>,
    status: String,
    access: Option<String>,
    shown: Option<Rendered>,
    texture: Option<(Key, TextureHandle)>,
    placed: Option<Placed>,
    popup: Option<Popup>,
    model_view: ModelView,
    last_action: Option<Value>,
    answers: u64,
    calls: Vec<(u64, &'static str, Then)>,
    info: Option<Result<Value, String>>,
    info_rev: Option<u64>,
    output: Option<(&'static str, Option<Result<Value, String>>)>,
    files: Option<Result<Vec<WsFile>, String>>,
    cmd: crate::ribbon::DocUi,
    refused: bool,
    autosave: crate::recovery::Clock,
}

impl Doc {
    /// The tab's label (the file name) and whether it has unsaved edits.
    fn tab_label(&self) -> (String, bool) {
        doc_label(self.origin.as_ref(), self.loaded.as_ref())
    }
}

/// A document's file name and its unsaved state, from its snapshot (else its origin).
fn doc_label(origin: Option<&Origin>, loaded: Option<&Loaded>) -> (String, bool) {
    let path = loaded
        .and_then(|l| l.snap.pointer("/source/path").and_then(Value::as_str))
        .unwrap_or("");
    let base = path.rsplit(['/', '\\']).next().unwrap_or("");
    let name = if base.is_empty() {
        origin.map(Origin::label).unwrap_or_else(|| "BIcraft".into())
    } else {
        base.to_string()
    };
    let dirty = loaded.is_some_and(|l| {
        ["model", "report", "queries"]
            .iter()
            .any(|k| l.snap.pointer(&format!("/dirty/{k}")).and_then(Value::as_bool) == Some(true))
    });
    (name, dirty)
}

/// Height of the document tab strip.
pub const DOC_TABS_H: f32 = 30.0;
/// Most commands kept in [`App::sent`].
const MAX_SENT: usize = 500;

pub struct App {
    pub lang: Lang,
    /// The session's UI language for the text inside visuals (`--lang`).
    pub ui_tag: String,
    pub origin: Option<Origin>,
    worker: Option<Worker>,
    pending: Option<Origin>,
    pub loaded: Option<Loaded>,
    pub error: Option<String>,
    pub status: String,
    /// A server's answer: `reader` or `editor`.
    pub access: Option<String>,
    /// The commands sent and not answered yet: (tag, id, what to do with the answer).
    calls: Vec<(u64, &'static str, Then)>,
    /// The Info page's `file.inspect` and the revision it was read at.
    pub info: Option<Result<Value, String>>,
    info_rev: Option<u64>,
    /// The last Export task's output (`None` inside: still running).
    pub output: Option<(&'static str, Option<Result<Value, String>>)>,
    /// The server workspace's files (Open > Browse).
    pub files: Option<Result<Vec<WsFile>, String>>,
    /// The server refused the active document's source (its tab closes, [`App::strip_note`]
    /// says why).
    refused: bool,
    /// A message of the document tab strip: why a tab could not open (the server's limit or
    /// 503 answer). Cleared by a click on it.
    pub strip_note: Option<String>,
    /// The other open documents; `docs[active]` is an empty stand-in for the active one, whose
    /// state is in the fields above.
    docs: Vec<Doc>,
    active: usize,
    /// Snapshot and command counters shared by every document (render-cache keys, answer tags).
    seq: u64,
    tag: u64,
    /// Every command sent, oldest first (at most 500; tests and the status bar).
    pub sent: Vec<Call>,
    /// The File view, when it is open.
    pub backstage: Option<Backstage>,
    /// Set by a Backstage action that returns to the document.
    pub(crate) close_backstage_now: bool,
    /// The page tabs' state (renaming, dragging).
    pub page_tabs: PageTabs,
    /// A confirmation being asked.
    pub confirm: Option<Confirm>,
    pub tab: Tab,
    /// The ribbon's tab.
    pub ribbon_tab: RibbonTab,
    /// The active document's command state: the visual selected, a read command's answer.
    pub cmd: crate::ribbon::DocUi,
    /// The parameters form being filled ([`crate::form::open`]).
    pub form: Option<crate::form::Form>,
    /// The command palette, when it is open (Ctrl+Shift+P, [`crate::palette`]).
    pub palette: Option<crate::palette::Palette>,
    /// The Report view's Build and Data panes' view state ([`crate::panes`]).
    pub panes: crate::panes::PanesUi,
    /// The Filters pane's view state.
    pub filters: crate::filters_pane::FiltersUi,
    /// The Selection pane's view state ([`crate::selection_pane`]).
    pub selection: crate::selection_pane::SelectionUi,
    /// The Bookmarks pane's view state ([`crate::bookmarks_pane`]).
    pub bookmarks: crate::bookmarks_pane::BookmarksUi,
    /// The Sync slicers pane's view state ([`crate::sync_pane`]).
    pub sync: crate::sync_pane::SyncUi,
    pub prefs: Prefs,
    /// The shell's services.
    pub shell: Shell,
    fonts: bool,
    pub(crate) pages: Option<Pages>,
    sync_render: bool,
    texture: Option<(Key, TextureHandle)>,
    /// The page on screen (rendered image and hit map).
    pub shown: Option<Rendered>,
    pub placed: Option<Placed>,
    /// Where the reader's pointer is over a button, shape or navigator button (hovered or
    /// pressed): the page is drawn with it in that state (`bicraft_render::states`).
    pub pointer: Option<bicraft_render::states::Pointer>,
    /// The reader's view of each Gantt, by visual name: the lines scrolled and the time line's
    /// zoom (view state, never in the file; [`crate::gantt_ui`]).
    pub gantt: crate::pages::GanttViews,
    /// The status bar's zoom (fit page, fit width, actual size, a percentage): a view setting
    /// of this window over the page's View › Page view; `None` follows the page.
    pub view_zoom: Option<ViewZoom>,
    pub popup: Option<Popup>,
    /// The Model tab's diagram (zoom, pan, selection).
    pub model_view: ModelView,
    pub connect: Option<ConnectForm>,
    title: String,
    /// The last action sent and the answers received (tests, status bar).
    pub last_action: Option<Value>,
    pub answers: u64,
    /// The theme applied to egui's style.
    applied: Option<Brightness>,
    /// The UI control channel's requests and waits (opt-in, [`crate::control`]).
    pub control: crate::control::Control,
    /// The last command answers (tag, result), newest last (the control channel's replies).
    pub(crate) answered: Vec<(u64, Result<Value, String>)>,
    /// Auto-recovery and autosave ([`crate::recovery`]).
    /// The DAX formula bar ([`crate::formula`]).
    pub formula: crate::formula::FormulaBar,
    pub recovery: crate::recovery::RecoveryUi,
    /// The active document's autosave clock ([`crate::recovery::tick`]).
    pub(crate) autosave: crate::recovery::Clock,
    /// The reader's drill on the canvas: header icons, menus, tooltip pages ([`crate::drill`]).
    pub drill: crate::drill::DrillUi,
    /// Canvas editing: the drag in progress and the in-app clipboard, shared by the documents
    /// ([`crate::canvas`]).
    pub canvas: crate::canvas::CanvasUi,
    /// The page drawn in focus mode ([`crate::focus`]), built once per snapshot.
    pub(crate) focus_view: crate::focus::FocusCache,
    /// View › Mobile layout and the phone canvas drawn ([`crate::mobile`]).
    pub mobile: crate::mobile::MobileUi,
    /// Keyboard navigation: the sections Ctrl+F6 cycles, the dialogs' focus, the focus ring
    /// ([`crate::keyboard`]).
    pub keyboard: crate::keyboard::KeyboardUi,
}

impl App {
    /// A window in language `lang_tag` (also the session's UI language) in `shell`.
    pub fn new(lang_tag: &str, shell: Shell) -> App {
        let stored = shell.prefs.as_ref().map(|s| s.load());
        let mut prefs = stored.as_ref().map(Prefs::from_json).unwrap_or_default();
        // the operating system's high contrast, until the user picks a theme
        if shell.system_high_contrast && stored.as_ref().and_then(|v| v.get("theme")).is_none() {
            prefs.theme = Brightness::HighContrast;
            prefs.report_contrast = true;
        }
        App {
            lang: Lang::from_tag(lang_tag),
            ui_tag: lang_tag.to_string(),
            origin: None,
            worker: None,
            pending: None,
            loaded: None,
            error: None,
            status: String::new(),
            access: None,
            calls: Vec::new(),
            info: None,
            info_rev: None,
            output: None,
            files: None,
            refused: false,
            strip_note: None,
            docs: Vec::new(),
            active: 0,
            seq: 0,
            tag: 0,
            sent: Vec::new(),
            backstage: None,
            close_backstage_now: false,
            page_tabs: PageTabs::default(),
            confirm: None,
            tab: Tab::from_name(&prefs.tab),
            ribbon_tab: crate::ribbon::tab_of(Tab::from_name(&prefs.tab)),
            cmd: crate::ribbon::DocUi::default(),
            form: None,
            palette: None,
            panes: crate::panes::PanesUi::default(),
            filters: crate::filters_pane::FiltersUi::default(),
            selection: crate::selection_pane::SelectionUi::default(),
            bookmarks: crate::bookmarks_pane::BookmarksUi::default(),
            sync: crate::sync_pane::SyncUi::default(),
            prefs,
            shell,
            fonts: false,
            pages: None,
            sync_render: false,
            texture: None,
            shown: None,
            placed: None,
            pointer: None,
            gantt: crate::pages::GanttViews::new(),
            view_zoom: None,
            popup: None,
            model_view: ModelView::default(),
            connect: None,
            title: String::new(),
            last_action: None,
            answers: 0,
            applied: None,
            control: crate::control::Control::default(),
            answered: Vec::new(),
            recovery: crate::recovery::RecoveryUi::default(),
            autosave: crate::recovery::Clock::default(),
            formula: crate::formula::FormulaBar::default(),
            drill: crate::drill::DrillUi::default(),
            canvas: crate::canvas::CanvasUi::default(),
            focus_view: crate::focus::FocusCache::default(),
            mobile: crate::mobile::MobileUi::default(),
            keyboard: crate::keyboard::KeyboardUi::default(),
        }
    }

    /// Renders pages on the window's thread with the bundled fonts only (tests: deterministic,
    /// the same on every machine).
    pub fn with_sync_render(mut self) -> App {
        self.sync_render = true;
        self
    }

    /// Whether pages render on the window's thread (tests: no waiting).
    pub fn sync_rendering(&self) -> bool {
        self.sync_render
    }

    /// Opens a file or project (the worker starts on the next frame).
    pub fn open_path(&mut self, path: &Path) {
        let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        let now = self.now();
        self.prefs.add_recent(&path, now);
        self.save_prefs();
        self.open_origin(Origin::Local {
            path,
            lang: self.ui_tag.clone(),
        });
    }

    /// Connects to a server.
    pub fn open_server(&mut self, server: Server) {
        self.prefs.add_server(&server.url, &server.workspace);
        self.save_prefs();
        self.open_origin(Origin::Remote(server));
    }

    /// The clock of the shell (seconds since 1970; 0 without one).
    pub fn now(&self) -> u64 {
        self.shell.now.as_ref().map_or(0, |f| f())
    }

    // --- documents ------------------------------------------------------------------------

    /// Opens `o` in a new document tab (the worker starts on the next frame): its tab if it is
    /// already open; the current document when the shell keeps one only (the browser); refused
    /// with a message beyond [`Shell::max_docs`].
    pub fn open_origin(&mut self, o: Origin) {
        self.backstage = None;
        self.close_backstage_now = true;
        if self.origin.as_ref() == Some(&o) && self.pending.is_none() {
            return;
        }
        if let Some(i) = self
            .docs
            .iter()
            .position(|d| d.origin.as_ref() == Some(&o) || d.pending.as_ref() == Some(&o))
        {
            self.switch_doc(i);
            return;
        }
        if self.docs.is_empty() {
            self.docs.push(Doc::default());
            self.active = 0;
        }
        let empty = self.origin.is_none() && self.pending.is_none();
        let max = self.shell.max_docs.max(1);
        if empty || max == 1 {
            self.pending = Some(o);
            return;
        }
        if self.docs.len() >= max {
            self.error = Some(match self.lang {
                Lang::En => format!("At most {max} files open at once: close one first."),
                Lang::Fr => format!("{max} fichiers ouverts au plus : fermez-en un d'abord."),
            });
            return;
        }
        let cur = self.take_doc();
        if let Some(slot) = self.docs.get_mut(self.active) {
            *slot = cur;
        }
        self.docs.push(Doc::default());
        self.active = self.docs.len().saturating_sub(1);
        self.pending = Some(o);
    }

    /// The open documents' tab labels and unsaved states, in tab order, and the active one.
    pub fn doc_tabs(&self) -> (Vec<(String, bool)>, usize) {
        let tabs = self
            .docs
            .iter()
            .enumerate()
            .map(|(i, d)| {
                if i == self.active {
                    doc_label(
                        self.origin.as_ref().or(self.pending.as_ref()),
                        self.loaded.as_ref(),
                    )
                } else {
                    d.tab_label()
                }
            })
            .collect();
        (tabs, self.active)
    }

    /// The document tab of server workspace file `file` (the browser's document tabs: a
    /// document view each), open or opening.
    pub fn workspace_doc(&self, file: &str) -> Option<usize> {
        let is = |o: Option<&Origin>| matches!(o, Some(Origin::Workspace { file: f, .. }) if f == file);
        (0..self.docs.len()).find(|&i| {
            if i == self.active {
                is(self.origin.as_ref()) || is(self.pending.as_ref())
            } else {
                self.docs
                    .get(i)
                    .is_some_and(|d| is(d.origin.as_ref()) || is(d.pending.as_ref()))
            }
        })
    }

    /// The index of the active document.
    pub fn active_doc(&self) -> usize {
        self.active
    }

    /// A document is open (or opening).
    pub fn has_doc(&self) -> bool {
        self.origin.is_some() || self.pending.is_some()
    }

    /// Exchanges the active document's fields with `d`'s.
    fn swap_doc(&mut self, d: &mut Doc) {
        use std::mem::swap;
        swap(&mut self.origin, &mut d.origin);
        swap(&mut self.worker, &mut d.worker);
        swap(&mut self.pending, &mut d.pending);
        swap(&mut self.loaded, &mut d.loaded);
        swap(&mut self.error, &mut d.error);
        swap(&mut self.status, &mut d.status);
        swap(&mut self.access, &mut d.access);
        swap(&mut self.shown, &mut d.shown);
        swap(&mut self.texture, &mut d.texture);
        swap(&mut self.placed, &mut d.placed);
        swap(&mut self.popup, &mut d.popup);
        swap(&mut self.model_view, &mut d.model_view);
        swap(&mut self.last_action, &mut d.last_action);
        swap(&mut self.answers, &mut d.answers);
        swap(&mut self.calls, &mut d.calls);
        swap(&mut self.info, &mut d.info);
        swap(&mut self.info_rev, &mut d.info_rev);
        swap(&mut self.output, &mut d.output);
        swap(&mut self.files, &mut d.files);
        swap(&mut self.cmd, &mut d.cmd);
        swap(&mut self.refused, &mut d.refused);
        swap(&mut self.autosave, &mut d.autosave);
    }

    /// The active document's state, leaving the fields empty.
    fn take_doc(&mut self) -> Doc {
        let mut d = Doc::default();
        self.swap_doc(&mut d);
        d
    }

    /// Makes document `i` the active one.
    pub fn switch_doc(&mut self, i: usize) {
        if i == self.active || i >= self.docs.len() {
            return;
        }
        let cur = self.take_doc();
        if let Some(slot) = self.docs.get_mut(self.active) {
            *slot = cur;
        }
        if let Some(slot) = self.docs.get_mut(i) {
            let mut d = std::mem::take(slot);
            self.swap_doc(&mut d);
        }
        self.active = i;
        self.page_tabs = PageTabs::default();
        self.confirm = None;
    }

    /// Closes document `i`, asking first when it has unsaved edits.
    pub fn ask_close(&mut self, i: usize) {
        let (tabs, _) = self.doc_tabs();
        match tabs.get(i) {
            Some((_, true)) => self.confirm = Some(Confirm::CloseDoc(i)),
            Some(_) => self.close_doc(i),
            None => {}
        }
    }

    /// Closes document `i` (its source stops); the next one becomes active.
    pub fn close_doc(&mut self, i: usize) {
        if i >= self.docs.len() {
            return;
        }
        if i == self.active {
            drop(self.take_doc());
            self.docs.remove(i);
            if self.docs.is_empty() {
                self.active = 0;
            } else {
                self.active = i.min(self.docs.len() - 1);
                if let Some(slot) = self.docs.get_mut(self.active) {
                    let mut d = std::mem::take(slot);
                    self.swap_doc(&mut d);
                }
            }
        } else {
            self.docs.remove(i);
            if i < self.active {
                self.active -= 1;
            }
        }
        self.page_tabs = PageTabs::default();
        if let Some(Confirm::CloseDoc(_)) = self.confirm {
            self.confirm = None;
        }
    }

    // --- commands -------------------------------------------------------------------------

    /// The tag the next command gets.
    pub fn next_tag(&self) -> u64 {
        self.tag.saturating_add(1)
    }

    /// Runs an engine command on the active document's source; `then` says what to do with the
    /// answer. False when no document is open.
    pub fn run(&mut self, c: Call, then: Then) -> bool {
        if self.worker.is_none() {
            self.error = Some(
                self.lang
                    .t("Open a file first.", "Ouvrez d'abord un fichier.")
                    .to_string(),
            );
            return false;
        }
        // the DAX query tabs go to the session first, so the save writes them
        if matches!(c.id, "file.save" | "file.save_as" | "project.new") {
            crate::dax_query::flush(self);
        }
        let Some(w) = &self.worker else {
            return false;
        };
        self.tag = self.tag.saturating_add(1);
        let tag = self.tag;
        w.send(Request::Command {
            tag,
            id: c.id.to_string(),
            params: c.params.clone(),
        });
        if let Then::Output(task) | Then::Download(task, _) | Then::Print(task, _) = &then {
            self.output = Some((*task, None));
        }
        if self.calls.len() >= 256 {
            self.calls.remove(0);
        }
        self.calls.push((tag, c.id, then));
        if self.sent.len() >= MAX_SENT {
            self.sent.remove(0);
        }
        let mut c = c;
        // a password, key or token goes to the session, never into this history
        if c.id == crate::get_data::CREDENTIALS {
            for k in ["password", "key", "token"] {
                if let Some(pw) = c.params.get_mut(k) {
                    *pw = Value::Null;
                }
            }
        }
        self.sent.push(c);
        true
    }

    /// Saves the open file (`file.save`); a file never saved goes to Save As.
    pub fn save(&mut self) {
        if self.source_path().is_empty() {
            let mut bs = Backstage::new(backstage::Page::Export);
            bs.task = backstage::Task::SaveAs;
            self.backstage = Some(bs);
            return;
        }
        self.run(actions::file_save(), Then::Nothing);
    }

    /// Reads `file.inspect` for the Info page once per revision.
    pub fn inspect_if_needed(&mut self) {
        let rev = self
            .loaded
            .as_ref()
            .and_then(|l| l.snap.get("revision").and_then(Value::as_u64));
        if self.worker.is_some() && rev.is_some() && self.info_rev != rev {
            self.info_rev = rev;
            self.run(actions::file_inspect(), Then::Inspect);
        }
    }

    /// The server workspace of this window: the browser page's, or the server the desktop is
    /// connected to.
    pub fn workspace(&self) -> Option<String> {
        self.shell.workspace.clone().or_else(|| match &self.origin {
            // the desktop connected to a server (the browser page names its own, or has none)
            Some(Origin::Remote(s)) if self.shell.caps.connect => Some(s.workspace.clone()),
            _ => None,
        })
    }

    /// Asks the source for the server workspace's files (Open > Browse).
    pub fn list_files(&mut self) {
        self.files = None;
        if let Some(w) = &self.worker {
            w.send(Request::ListFiles);
        } else {
            self.files = Some(Err(self
                .lang
                .t("Not connected.", "Pas de connexion.")
                .to_string()));
        }
    }

    /// Opens a workspace file, kept in Recent: in a tab of its own in the browser (a document
    /// view on the server, beside the other tabs), in the server tab's session on the desktop.
    pub fn open_ws_file(&mut self, rel: &str) {
        if self.shell.ws_tabs
            && let Some(ws) = self.shell.workspace.clone()
        {
            let now = self.now();
            self.prefs.add_ws_recent(&ws, rel, now);
            self.save_prefs();
            self.open_origin(Origin::Workspace {
                ws,
                file: rel.to_string(),
            });
            return;
        }
        if self.run(actions::file_open(rel), Then::Nothing) {
            if let Some(ws) = self.shell.workspace.clone() {
                let now = self.now();
                self.prefs.add_ws_recent(&ws, rel, now);
                self.save_prefs();
            }
            self.close_backstage_now = true;
            self.backstage = None;
        }
    }

    /// Opens a recent file: a path on the desktop, a workspace file in the browser.
    pub fn open_recent(&mut self, r: &Recent) {
        match &r.ws {
            None => self.open_path(Path::new(&r.path)),
            Some(ws) if self.shell.workspace.as_ref() == Some(ws) => self.open_ws_file(&r.path),
            Some(ws) => {
                self.error = Some(format!(
                    "{} {ws}",
                    self.lang.t(
                        "This file belongs to another workspace:",
                        "Ce fichier appartient à un autre espace de travail :"
                    )
                ));
            }
        }
    }

    /// New project `name`: in a folder the user picks, opened in a new tab (from `template`, a
    /// `.pbit`, when given); without a folder dialog (the browser), at the workspace's root in the
    /// open session. False when nothing was started.
    pub fn new_project(&mut self, name: &str, template: Option<PathBuf>) -> bool {
        if let Some(pick) = &self.shell.pick_folder {
            let Some(dir) = pick(self.lang) else {
                return false;
            };
            let now = self.now();
            self.prefs.add_recent(&dir.join(format!("{name}.pbip")), now);
            self.save_prefs();
            self.open_origin(Origin::New {
                dir,
                name: name.to_string(),
                template,
                lang: self.ui_tag.clone(),
            });
            return true;
        }
        self.run(actions::project_new(".", name), Then::Nothing)
    }

    /// The shell's Open dialog.
    pub fn pick_file(&self) -> Option<PathBuf> {
        self.shell.pick_file.as_ref().and_then(|f| f(self.lang))
    }

    /// Takes in one update of the active document's source.
    fn handle(&mut self, u: Update) {
        match u {
            Update::Status(s) => self.status = s,
            Update::Snapshot(mut l) => {
                // one counter for every document: the page cache never mixes two files
                self.seq = self.seq.saturating_add(1);
                l.seq = self.seq;
                self.loaded = Some(l);
            }
            Update::Error(e) => self.error = Some(e),
            Update::Connected { access } => {
                self.access = Some(access);
                self.error = None;
            }
            Update::Interacted(r) => {
                self.answers = self.answers.saturating_add(1);
                if let Ok(v) = &r {
                    crate::rich::interacted(self, v);
                }
                if let Err(e) = r {
                    // the viewer's refused-action message: `✕ reason`
                    self.error = Some(format!("✕ {e}"));
                }
            }
            Update::Files(r) => self.files = Some(r),
            Update::Limits { views_per_user } => {
                // the first tab follows the workspace's session; each other one is a view
                let cap = views_per_user.saturating_add(1).clamp(1, crate::shell::MAX_DOCS);
                self.shell.max_docs = cap;
            }
            Update::Refused(why) => {
                self.refused = true;
                self.strip_note = Some(clip(&why, 300));
            }
            Update::Ran { tag, result } => {
                let Some(pos) = self.calls.iter().position(|c| c.0 == tag) else {
                    return;
                };
                let (_, id, then) = self.calls.remove(pos);
                self.answers = self.answers.saturating_add(1);
                if self.control.attached() {
                    if self.answered.len() >= 8 {
                        drop(self.answered.remove(0));
                    }
                    self.answered.push((tag, result.clone()));
                }
                match (then, result) {
                    (Then::Output(task), r) => self.output = Some((task, Some(r))),
                    (Then::Download(task, file), r) => {
                        if r.is_ok()
                            && let Some(d) = &self.shell.download
                        {
                            d(&file);
                        }
                        self.output = Some((task, Some(r)));
                    }
                    (Then::Print(task, file), r) => {
                        if r.is_ok()
                            && let Some(p) = &self.shell.print
                        {
                            p(&file);
                        }
                        self.output = Some((task, Some(r)));
                    }
                    (Then::Inspect, r) => self.info = Some(r),
                    (Then::Show(id), r) => {
                        self.cmd.view = None;
                        self.cmd.result = Some((id, Some(r)));
                    }
                    (Then::Form, r) => crate::form::answer(self, tag, r),
                    (Then::Lookup(_), r) => crate::form::looked_up(self, tag, r),
                    (Then::Recovery(what), r) => crate::recovery::answer(self, what, r),
                    (Then::FilterValues(field, rev), r) => {
                        crate::filters_pane::values_answer(self, &field, rev, r)
                    }
                    (Then::Formula(step), r) => crate::formula::answer(self, step, r),
                    (Then::Data(step), r) => crate::data_view::answer(self, step, r),
                    (Then::Rich(step), r) => crate::rich::answer(self, step, r),
                    (Then::Perf(id), r) => crate::perf_pane::answer(self, id, r),
                    (Then::VisualCalc, r) => crate::viscalc_dialog::answer(self, r),
                    (Then::VisualCalcPreview(asked), r) => {
                        crate::viscalc_dialog::preview_answer(self, &asked, r)
                    }
                    (Then::Autodetect, r) => crate::autodetect::answer(self, r),
                    (Then::Rels(step), r) => crate::relationships_dialog::answer(self, step, r),
                    (Then::Roles(step), r) => crate::roles_dialog::answer(self, step, r),
                    (Then::DaxQuery(step), r) => crate::dax_query::answer(self, step, r),
                    (Then::Tmdl(step), r) => crate::tmdl_view::answer(self, step, r),
                    (Then::GroupValues, r) => crate::group_dialog::values_answer(self, r),
                    (Then::QuickValues(column), r) => crate::quick_dialog::values_answer(self, column, r),
                    (Then::GetData(step), r) => crate::get_data::answer(self, step, r),
                    (Then::SourceSettings(step), r) => crate::source_settings::answer(self, step, r),
                    (Then::Pq(step), r) => crate::pq_editor::answer(self, step, r),
                    (
                        Then::FormatProps {
                            page,
                            visual,
                            revision,
                        },
                        r,
                    ) => crate::format_pane::props_answer(self, page, visual, revision, r),
                    (Then::FormatEdit { page, visual }, r) => {
                        crate::format_pane::edit_answer(self, &page, &visual, r)
                    }
                    (Then::FormatValues { field, revision }, r) => {
                        crate::format_pane::values_answer(self, field, revision, r)
                    }
                    (_, Err(e)) => self.error = Some(format!("✕ {id}: {}", clip(&e, 600))),
                    (Then::Nothing, Ok(_)) => {}
                    (Then::Clipboard, Ok(v)) => crate::canvas::copied(self, &v),
                    (Then::SelectPasted, Ok(v)) => crate::canvas::pasted(self, &v),
                    (Then::SelectVisual, Ok(v)) => {
                        if let (Some(page), Some(visual)) = (
                            v.get("page").and_then(Value::as_str),
                            v.get("visual").and_then(Value::as_str),
                        ) {
                            self.cmd.visual = Some(crate::ribbon::VisualSel {
                                page: page.to_string(),
                                visual: visual.to_string(),
                            });
                        }
                    }
                    (Then::Duplicate { key, name }, Ok(spec)) => {
                        match actions::duplicate_from_spec(&spec, &key, &name) {
                            Ok(c) => {
                                self.run(c, Then::Nothing);
                            }
                            Err(e) => self.error = Some(format!("✕ {e}")),
                        }
                    }
                    (Then::ShowPage, Ok(v)) => {
                        let name = ["name", "page"]
                            .iter()
                            .find_map(|k| v.get(*k).and_then(Value::as_str))
                            .or_else(|| v.as_str())
                            .map(str::to_string);
                        if let Some(name) = name {
                            self.interact(json!({"action": "page", "page": name}));
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn save_prefs(&mut self) {
        if let Some(store) = &self.shell.prefs
            && let Err(e) = store.save(&self.prefs.to_json())
        {
            self.status = e;
        }
    }

    /// What the shell can do.
    pub fn caps(&self) -> Caps {
        self.shell.caps
    }

    /// Sends a `view.interact` action to the source (a matrix's expand and collapse while editing
    /// go to `visual.expand` instead, [`crate::drill::route`]).
    pub fn interact(&mut self, action: Value) {
        if crate::drill::route(self, &action) {
            return;
        }
        if let Some(w) = &self.worker {
            self.last_action = Some(action.clone());
            w.send(Request::Interact(action));
        }
    }

    /// Starts the active document's pending source.
    fn start_pending(&mut self, ctx: &egui::Context) {
        if let Some(o) = self.pending.take() {
            let c = ctx.clone();
            let notify: Notify = Arc::new(move || c.request_repaint());
            self.worker = Some(Worker::start(o.clone(), notify, &self.shell.start_source));
            self.origin = Some(o);
            self.loaded = None;
            self.shown = None;
            self.texture = None;
            self.error = None;
            self.access = None;
            self.popup = None;
            self.model_view = ModelView::default();
            self.calls.clear();
            self.info = None;
            self.info_rev = None;
            self.output = None;
            self.files = None;
            self.cmd = crate::ribbon::DocUi::default();
            self.filters = crate::filters_pane::FiltersUi::default();
            self.selection = crate::selection_pane::SelectionUi::default();
            self.refused = false;
            self.autosave = crate::recovery::Clock::default();
            if self.docs.is_empty() {
                self.docs.push(Doc::default());
                self.active = 0;
            }
        }
    }

    /// Starts a pending source and takes in what the sources and the renderer sent.
    pub fn logic(&mut self, ctx: &egui::Context) {
        if !self.fonts {
            crate::fonts::install(ctx);
            self.fonts = true;
        }
        if self.pages.is_none() {
            self.pages = Some(if self.sync_render {
                Pages::synchronous(crate::pages::font_book())
            } else {
                let c = ctx.clone();
                let notify: Notify = Arc::new(move || c.request_repaint());
                Pages::threaded(notify)
            });
        }
        self.start_pending(ctx);
        // a document opened while another was opening: start its source too
        for i in 0..self.docs.len() {
            if i != self.active
                && self.docs.get(i).is_some_and(|d| d.pending.is_some())
                && let Some(slot) = self.docs.get_mut(i)
            {
                let mut d = std::mem::take(slot);
                self.swap_doc(&mut d);
                self.start_pending(ctx);
                self.swap_doc(&mut d);
                if let Some(slot) = self.docs.get_mut(i) {
                    *slot = d;
                }
            }
        }
        let updates = self.worker.as_ref().map(Worker::poll).unwrap_or_default();
        for u in updates {
            self.handle(u);
        }
        // the other documents' sources keep running: take in what they sent, each in turn
        for i in 0..self.docs.len() {
            if i == self.active {
                continue;
            }
            let ups = self
                .docs
                .get(i)
                .and_then(|d| d.worker.as_ref())
                .map(Worker::poll)
                .unwrap_or_default();
            if ups.is_empty() {
                continue;
            }
            if let Some(slot) = self.docs.get_mut(i) {
                let mut d = std::mem::take(slot);
                self.swap_doc(&mut d);
                for u in ups {
                    self.handle(u);
                }
                self.swap_doc(&mut d);
                if let Some(slot) = self.docs.get_mut(i) {
                    *slot = d;
                }
            }
        }
        // a tab whose source the server refused closes (the tab strip says why)
        for i in (0..self.docs.len()).rev() {
            let refused = if i == self.active {
                self.refused
            } else {
                self.docs.get(i).is_some_and(|d| d.refused)
            };
            if refused {
                self.close_doc(i);
            }
        }
        // the control channel's requests run here, between frames, as the UI's own clicks
        crate::control::pump(self, ctx);
        // autosave: every open document, not only the one shown
        crate::recovery::tick(self, ctx);
        for i in 0..self.docs.len() {
            if i == self.active {
                continue;
            }
            if let Some(slot) = self.docs.get_mut(i) {
                let mut d = std::mem::take(slot);
                self.swap_doc(&mut d);
                crate::recovery::tick(self, ctx);
                self.swap_doc(&mut d);
                if let Some(slot) = self.docs.get_mut(i) {
                    *slot = d;
                }
            }
        }
        let title = self.window_title();
        if title != self.title {
            match &self.shell.set_title {
                Some(set) => set(ctx, &title),
                None => ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone())),
            }
            self.title = title;
        }
        if !self.shell.caps.open_files {
            return;
        }
        // files dropped on the window
        let dropped: Option<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .find(|p| !p.as_os_str().is_empty())
        });
        if let Some(p) = dropped {
            self.open_path(&p);
        }
    }

    /// The window's title (tests).
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The title the viewer's page gives its tab: `● file · rev N` (`●` when the model or the
    /// report has unsaved edits), the origin's name until the first snapshot, `BIcraft` when
    /// nothing is open.
    pub fn window_title(&self) -> String {
        let Some(o) = &self.origin else {
            return "BIcraft".to_string();
        };
        let Some(l) = &self.loaded else {
            return clip(&o.label(), 200);
        };
        let snap = &l.snap;
        let path = snap.pointer("/source/path").and_then(Value::as_str).unwrap_or("");
        let base = path.rsplit(['/', '\\']).next().unwrap_or("");
        let base = if base.is_empty() {
            o.label()
        } else {
            base.to_string()
        };
        let dirty = ["model", "report", "queries"]
            .iter()
            .any(|k| snap.pointer(&format!("/dirty/{k}")).and_then(Value::as_bool) == Some(true));
        let rev = snap.get("revision").and_then(Value::as_u64).unwrap_or(0);
        format!(
            "{}{} · rev {}",
            if dirty { "● " } else { "" },
            clip(&base, 200),
            rev
        )
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        // the fonts `logic` installed apply from the next pass: until then the app's families
        // (semibold, bold) do not exist and egui would refuse to lay text out in them
        if !crate::fonts::installed(ui.ctx()) {
            ui.ctx().request_repaint();
            return;
        }
        // the keyboard's sections, dialogs and focus ring (crate::keyboard)
        crate::keyboard::begin(self, ui.ctx());
        self.window_ui(ui);
        crate::keyboard::finish(self, ui.ctx());
    }

    /// The window's chrome, its view and its dialogs.
    fn window_ui(&mut self, ui: &mut egui::Ui) {
        if self.applied != Some(self.prefs.theme) {
            theme::apply(ui.ctx(), &Tokens::of(self.prefs.theme));
            self.applied = Some(self.prefs.theme);
            // the new style applies from the next pass: this one would lay the window out in
            // egui's default spacing, and what it sizes stays (a pane grows to fit its content
            // and keeps that width), so the picture would depend on what was on screen at that
            // frame. Draw nothing and run the next pass at once.
            ui.ctx().request_discard("the theme applies from the next pass");
            ui.ctx().request_repaint();
            return;
        }
        self.shortcuts(ui);
        self.title_row(ui);
        let t = Tokens::of(self.prefs.theme);
        if !self.docs.is_empty() {
            let r = egui::Panel::top("bicraft-doc-tabs")
                .exact_size(DOC_TABS_H)
                .frame(egui::Frame::NONE.fill(t.chrome))
                .show(ui, |ui| self.doc_tab_strip(ui));
            crate::keyboard::section(self, ui, Section::Ribbon, r.response.rect);
        }
        // the File view: opened from its tab, and the start screen of a shell that opens files
        let caps = self.shell.caps;
        let start = !self.has_doc() && self.loaded.is_none() && (caps.open_files || caps.connect);
        if start && self.backstage.is_none() {
            self.backstage = Some(Backstage::new(backstage::Page::Home));
        }
        if self.backstage.is_some() {
            backstage::show(self, ui);
            self.popup = None;
            self.connect_ui(ui.ctx());
            self.confirm_ui(ui.ctx());
            crate::recovery::prompt(self, ui.ctx());
            crate::form::show(self, ui.ctx());
            crate::palette::show(self, ui.ctx());
            return;
        }
        self.ribbon(ui);
        crate::view_as::banner(self, ui);
        crate::optimize_ui::banner(self, ui);
        // the DAX query and TMDL views have their own editor: no formula bar there
        if !matches!(self.tab, Tab::Query | Tab::Tmdl) {
            crate::formula::show(self, ui);
        }
        self.status_bar(ui);
        let report_shown = self.tab == Tab::Report
            && self
                .loaded
                .as_ref()
                .is_some_and(|l| l.snap.get("report").is_some_and(Value::is_object));
        if report_shown {
            crate::panes::show(self, ui);
        }
        if self.tab == Tab::Report
            && self
                .loaded
                .as_ref()
                .is_some_and(|l| l.snap.get("report").is_some_and(Value::is_object))
        {
            let r = egui::Panel::bottom("bicraft-page-tabs")
                .exact_size(page_tabs::TABS_H)
                .frame(
                    egui::Frame::NONE
                        .fill(t.chrome)
                        .inner_margin(egui::Margin::symmetric(8, 0)),
                )
                .show(ui, |ui| page_tabs::show(self, ui));
            crate::keyboard::section(self, ui, Section::PageTabs, r.response.rect);
        }
        let main = egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(t.canvas))
            .show(ui, |ui| {
                if let Some(e) = self.error.clone() {
                    let l = self.lang;
                    egui::Frame::NONE
                        .inner_margin(egui::Margin::symmetric(10, 8))
                        .show(ui, |ui| {
                            if widgets::error_bar(ui, &e, l.t("Close", "Fermer")) {
                                self.error = None;
                            }
                        });
                }
                if self.origin.is_none() && self.loaded.is_none() {
                    self.start_screen(ui);
                    return;
                }
                match self.tab {
                    Tab::Report => self.report(ui),
                    Tab::Data => crate::data_view::show(self, ui),
                    Tab::Model => self.model(ui),
                    Tab::Query => crate::dax_query::show(self, ui),
                    Tab::Tmdl => crate::tmdl_view::show(self, ui),
                    Tab::Validation => self.validation(ui),
                }
            });
        crate::keyboard::section(self, ui, Section::Main, main.response.rect);
        crate::panes::finish(self, ui.ctx());
        self.popup_ui(ui.ctx());
        crate::rich::windows(self, ui.ctx());
        crate::param_dialog::show(self, ui.ctx());
        crate::viscalc_dialog::show(self, ui.ctx());
        crate::group_dialog::show(self, ui.ctx());
        crate::quick_dialog::show(self, ui.ctx());
        crate::view_as::show(self, ui.ctx());
        crate::relationships_dialog::show(self, ui.ctx());
        crate::autodetect::show(self, ui.ctx());
        crate::roles_dialog::show(self, ui.ctx());
        crate::perspective_dialog::show(self, ui.ctx());
        crate::theme_dialog::show(self, ui.ctx());
        crate::custom_visuals::show(self, ui.ctx());
        crate::storage_ui::show(self, ui.ctx());
        crate::refresh_policy_dialog::show(self, ui.ctx());
        crate::get_data::show(self, ui.ctx());
        crate::source_settings::show(self, ui.ctx());
        crate::enter_data::show(self, ui.ctx());
        crate::pq_editor::show(self, ui.ctx());
        crate::form::show(self, ui.ctx());
        crate::ribbon::result_ui(self, ui.ctx());
        self.connect_ui(ui.ctx());
        self.confirm_ui(ui.ctx());
        crate::recovery::prompt(self, ui.ctx());
        crate::palette::show(self, ui.ctx());
    }

    /// The document tabs under the title row: the file name, an amber dot with unsaved edits, ×
    /// to close (a middle click too), `+` for the File view; Ctrl+Tab and Ctrl+Shift+Tab switch.
    fn doc_tab_strip(&mut self, ui: &mut egui::Ui) {
        let l = self.lang;
        let t = Tokens::of(self.prefs.theme);
        let (tabs, active) = self.doc_tabs();
        let mut switch = None;
        let mut close = None;
        let mut plus = false;
        let rect = ui.max_rect();
        let mut row = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    pos2(rect.min.x + 8.0, rect.min.y + 2.0),
                    pos2(rect.max.x - 8.0, rect.max.y),
                ))
                .layout(egui::Layout::left_to_right(egui::Align::Max)),
        );
        row.spacing_mut().item_spacing.x = 2.0;
        egui::ScrollArea::horizontal()
            .id_salt("bicraft-doc-tabs")
            .max_width(
                (row.available_width() - 34.0 - if self.strip_note.is_some() { 360.0 } else { 0.0 })
                    .max(60.0),
            )
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(&mut row, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (i, (name, dirty)) in tabs.iter().enumerate() {
                        let on = i == active;
                        let label = clip(name, 40);
                        let font = if on { theme::bold(12.0) } else { theme::font(12.0) };
                        let tw = ui.fonts_mut(|f| {
                            f.layout_no_wrap(label.clone(), theme::bold(12.0), t.text)
                                .size()
                                .x
                        });
                        let w = tw + 24.0 + 22.0 + if *dirty { 12.0 } else { 0.0 };
                        let (r, resp) =
                            ui.allocate_exact_size(Vec2::new(w, DOC_TABS_H - 4.0), Sense::click());
                        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, name));
                        let p = ui.painter();
                        let shape = CornerRadius {
                            nw: 6,
                            ne: 6,
                            sw: 0,
                            se: 0,
                        };
                        if on {
                            p.rect_filled(r, shape, t.card);
                            p.rect_filled(
                                Rect::from_min_size(r.min, Vec2::new(r.width(), 2.0)),
                                CornerRadius::same(1),
                                t.accent,
                            );
                        } else if resp.hovered() {
                            p.rect_filled(r, shape, t.hover);
                        }
                        let mut x = r.min.x + 12.0;
                        if *dirty {
                            p.circle_filled(pos2(x + 3.0, r.center().y), 3.5, t.accent);
                            x += 12.0;
                        }
                        p.text(
                            pos2(x, r.center().y),
                            egui::Align2::LEFT_CENTER,
                            &label,
                            font,
                            t.text,
                        );
                        let resp = resp.on_hover_text(if *dirty {
                            format!(
                                "{name} · {}",
                                l.t("unsaved changes", "modifications non enregistrées")
                            )
                        } else {
                            name.clone()
                        });
                        let xr =
                            Rect::from_center_size(pos2(r.max.x - 14.0, r.center().y), Vec2::splat(18.0));
                        let x_resp =
                            ui.interact(xr, ui.scope_id().with(("bicraft-doc-close", i)), Sense::click());
                        let close_label = format!("{} {name}", l.t("Close", "Fermer"));
                        x_resp.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::Role::Button, true, &close_label)
                        });
                        if x_resp.hovered() {
                            ui.painter().rect_filled(xr, CornerRadius::same(4), t.pressed);
                        }
                        if on || resp.hovered() || x_resp.hovered() {
                            crate::icons::paint(ui.painter(), xr.shrink(4.0), Icon::Close, t.text_dim, false);
                        }
                        if x_resp.clicked() || resp.middle_clicked() {
                            close = Some(i);
                        } else if resp.clicked() && !on {
                            switch = Some(i);
                        }
                    }
                });
            });
        let (r, resp) = row.allocate_exact_size(Vec2::new(26.0, DOC_TABS_H - 4.0), Sense::click());
        let label = l.t("Open or create a file", "Ouvrir ou créer un fichier");
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
        if resp.hovered() {
            row.painter().rect_filled(r, CornerRadius::same(4), t.hover);
        }
        crate::icons::paint(
            row.painter(),
            Rect::from_center_size(r.center(), Vec2::splat(14.0)),
            Icon::Plus,
            t.text,
            false,
        );
        if resp.on_hover_text(label).clicked() {
            plus = true;
        }
        // why a tab could not open (the server's limit or 503 answer); a click dismisses it
        if let Some(note) = self.strip_note.clone() {
            row.add_space(8.0);
            let text = RichText::new(format!("⚠ {}", clip(&note, 160)))
                .font(theme::font(12.0))
                .color(t.text);
            let resp = row
                .add(egui::Label::new(text).sense(Sense::click()).truncate())
                .on_hover_text(format!(
                    "{note}\n{}",
                    l.t("Click to dismiss.", "Cliquez pour fermer.")
                ));
            if resp.clicked() {
                self.strip_note = None;
            }
        }
        if let Some(i) = close {
            self.ask_close(i);
        } else if let Some(i) = switch {
            self.switch_doc(i);
        }
        if plus {
            self.backstage = Some(Backstage::new(backstage::Page::Home));
        }
    }

    /// The confirmation dialog (DeckCraft's dialog: primary then Cancel, Enter, Escape).
    fn confirm_ui(&mut self, ctx: &egui::Context) {
        let l = self.lang;
        let Some(c) = self.confirm.clone() else { return };
        let (title, text, ok) = match &c {
            Confirm::DeletePage { label, .. } => (
                l.t("Delete page", "Supprimer la page"),
                format!(
                    "{} “{}” {}",
                    l.t("Delete", "Supprimer"),
                    clip(label, 80),
                    l.t("and its visuals?", "et ses visuels ?")
                ),
                l.t("Delete", "Supprimer"),
            ),
            Confirm::CloseDoc(i) => {
                let name = self.doc_tabs().0.get(*i).map(|t| t.0.clone()).unwrap_or_default();
                (
                    l.t("Unsaved changes", "Modifications non enregistrées"),
                    format!(
                        "{} “{}” {}",
                        l.t("Close", "Fermer"),
                        clip(&name, 80),
                        l.t("without saving?", "sans enregistrer ?")
                    ),
                    l.t("Close without saving", "Fermer sans enregistrer"),
                )
            }
        };
        let mut go = false;
        let mut cancel = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::new(0.0, -40.0))
            .show(ctx, |ui| {
                ui.set_min_width(340.0);
                ui.label(text);
                ui.add_space(10.0);
                let w = ui.available_width();
                ui.allocate_ui_with_layout(
                    Vec2::new(w, 28.0),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if widgets::primary(ui, ok, true, "").clicked() || enter {
                            go = true;
                        }
                        if ui
                            .add(egui::Button::new(l.t("Cancel", "Annuler")).min_size(Vec2::new(72.0, 24.0)))
                            .clicked()
                        {
                            cancel = true;
                        }
                    },
                );
            });
        if go {
            self.confirm = None;
            match c {
                Confirm::DeletePage { name, .. } => {
                    self.run(actions::page_remove(&name), Then::Nothing);
                }
                Confirm::CloseDoc(i) => {
                    // closed without saving: its recovery journal goes too (the command runs on
                    // that document's source, so a background tab is brought forward for it)
                    let back = self.active;
                    self.switch_doc(i);
                    crate::recovery::discard_current(self);
                    self.close_doc(i);
                    if back != i {
                        self.switch_doc(if back > i { back - 1 } else { back });
                    }
                }
            }
        } else if cancel {
            self.confirm = None;
        }
    }

    /// Switches the view (and the ribbon) to `tab`, kept in the preferences.
    pub fn show_tab(&mut self, tab: Tab) {
        self.tab = tab;
        self.ribbon_tab = crate::ribbon::tab_of(tab);
        let name = tab.name().to_string();
        if self.prefs.tab != name {
            self.prefs.tab = name;
            self.save_prefs();
        }
    }

    /// Light, dark or high contrast, kept in the preferences. The report pages follow: in high
    /// contrast with the high-contrast theme, as Desktop draws reports when the system is in
    /// high contrast (View › High-contrast colours changes it afterwards).
    pub fn set_theme(&mut self, b: Brightness) {
        if self.prefs.theme != b {
            self.prefs.theme = b;
            self.prefs.report_contrast = b == Brightness::HighContrast;
            self.save_prefs();
        }
    }

    /// The title row: the mark and quick access, the document and its pills, the actions.
    fn title_row(&mut self, ui: &mut egui::Ui) {
        let l = self.lang;
        let t = Tokens::of(self.prefs.theme);
        let caps = self.shell.caps;
        let r = egui::Panel::top("bicraft-title")
            .exact_size(TITLE_H)
            .frame(egui::Frame::NONE.fill(t.chrome))
            .show(ui, |ui| {
                let rect = ui.max_rect();
                let mut left = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(Rect::from_min_max(
                            pos2(rect.min.x + 10.0, rect.min.y),
                            pos2(rect.center().x, rect.max.y),
                        ))
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                left.spacing_mut().item_spacing.x = 2.0;
                let (r, _) = left.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                theme::paint_logo(left.painter(), r);
                left.add_space(8.0);
                if widgets::icon_toggle(
                    &mut left,
                    Icon::Reload,
                    l.t("Reload the file", "Recharger le fichier"),
                    self.worker.is_some(),
                    false,
                )
                .clicked()
                {
                    self.reload();
                }
                // the document and its pills, centred
                let (name, pills) = self.header();
                let font = theme::font(13.0);
                let name_w = ui.fonts_mut(|f| f.layout_no_wrap(name.clone(), font.clone(), t.text).size().x);
                let pills_w: f32 = pills
                    .iter()
                    .map(|(s, _)| {
                        ui.fonts_mut(|f| f.layout_no_wrap(s.clone(), theme::font(11.0), t.text).size().x)
                            + 34.0
                    })
                    .sum();
                let w = (name_w + pills_w + 8.0).min(rect.width() * 0.5);
                let mut mid = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(Rect::from_center_size(
                            rect.center(),
                            Vec2::new(w + 4.0, rect.height()),
                        ))
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                mid.set_clip_rect(mid.max_rect());
                mid.spacing_mut().item_spacing.x = 6.0;
                let tip = self.source_path();
                let r = mid.label(RichText::new(&name).font(font).color(t.text));
                if !tip.is_empty() {
                    r.on_hover_text(tip);
                }
                for (s, dot) in pills {
                    widgets::pill(&mut mid, &s, dot, true);
                }
                // the actions, right to left
                let mut right = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(Rect::from_min_max(
                            pos2(rect.center().x, rect.min.y),
                            pos2(rect.max.x - 10.0, rect.max.y),
                        ))
                        .layout(egui::Layout::right_to_left(egui::Align::Center)),
                );
                right.spacing_mut().item_spacing.x = 6.0;
                // right to left (design.md §5): the primary pill, search, the theme
                if caps.open_in_desktop {
                    let _ = widgets::primary(
                        &mut right,
                        l.t("Open in Power BI Desktop", "Ouvrir dans Power BI Desktop"),
                        false,
                        l.t("Not available here yet", "Pas encore disponible ici"),
                    );
                }
                let search = format!(
                    "{} (Ctrl+Shift+P)",
                    l.t("Search commands", "Rechercher une commande")
                );
                if widgets::icon_toggle(&mut right, Icon::Search, &search, true, self.palette.is_some())
                    .clicked()
                {
                    crate::palette::toggle(self);
                }
                let dark = self.prefs.theme != Brightness::Light;
                let (icon, tip) = if dark {
                    (Icon::Sun, l.t("Light mode", "Mode clair"))
                } else {
                    (Icon::Moon, l.t("Dark mode", "Mode sombre"))
                };
                if widgets::icon_toggle(&mut right, icon, tip, true, false).clicked() {
                    self.set_theme(self.prefs.theme.toggled());
                }
            });
        crate::keyboard::section(self, ui, Section::Ribbon, r.response.rect);
    }

    /// The document's name and its pills (format, revision, unsaved parts).
    fn header(&self) -> (String, Vec<(String, Option<Color32>)>) {
        let t = Tokens::of(self.prefs.theme);
        let Some(o) = &self.origin else {
            return ("BIcraft".to_string(), Vec::new());
        };
        let Some(l) = &self.loaded else {
            return (clip(&o.label(), 80), Vec::new());
        };
        let snap = &l.snap;
        let path = self.source_path();
        let base = path.rsplit(['/', '\\']).next().unwrap_or("");
        let name = if base.is_empty() {
            o.label()
        } else {
            base.to_string()
        };
        let mut pills = Vec::new();
        if let Some(f) = snap
            .pointer("/source/format")
            .and_then(Value::as_str)
            .filter(|f| !f.is_empty())
        {
            let f = f.split('-').next().unwrap_or(f).to_uppercase();
            pills.push((clip(&f, 12), None));
        }
        if let Some(rev) = snap.get("revision").and_then(Value::as_u64) {
            pills.push((format!("rev {rev}"), None));
        }
        for k in ["model", "report", "queries"] {
            if snap.pointer(&format!("/dirty/{k}")).and_then(Value::as_bool) == Some(true) {
                let label = match (k, self.lang) {
                    ("model", Lang::Fr) => "modèle",
                    ("report", Lang::Fr) => "rapport",
                    ("queries", Lang::Fr) => "requêtes",
                    (k, _) => k,
                };
                pills.push((label.to_string(), Some(t.accent)));
            }
        }
        (clip(&name, 80), pills)
    }

    pub(crate) fn source_path(&self) -> String {
        self.loaded
            .as_ref()
            .and_then(|l| {
                l.snap
                    .pointer("/source/path")
                    .and_then(Value::as_str)
                    .map(|s| clip(s, 400))
            })
            .unwrap_or_default()
    }

    pub(crate) fn reload(&mut self) {
        if let Some(w) = &self.worker {
            w.send(Request::Reload);
        }
    }

    /// A click on ribbon tab `rt` of the tab row: File opens the File view; Report, Model and
    /// their contextual tabs show their view (the control channel's `ui.set` too).
    pub fn select_ribbon_tab(&mut self, rt: RibbonTab) {
        if rt == RibbonTab::File {
            self.backstage = Some(Backstage::new(backstage::Page::Home));
            return;
        }
        if let Some(v) = crate::ribbon::view_of(rt) {
            self.show_tab(v);
        }
        self.ribbon_tab = rt;
    }

    /// Whether the command sent with `tag` waits for its answer (the control channel).
    pub fn waiting_for(&self, tag: u64) -> bool {
        self.calls.iter().any(|c| c.0 == tag)
    }

    /// Commands of the active document waiting for their answer.
    pub fn calls_pending(&self) -> usize {
        self.calls.len()
    }

    /// A document's source is about to start (the next frame starts it).
    pub fn opening(&self) -> bool {
        self.pending.is_some()
    }

    /// The tab row and the ribbon card.
    fn ribbon(&mut self, ui: &mut egui::Ui) {
        // the view may change elsewhere (the status bar, the shell): the ribbon follows it
        // and a contextual tab goes with its selection
        let tabs = crate::ribbon::tabs(self);
        if crate::ribbon::view_of(self.ribbon_tab).is_some_and(|v| v != self.tab)
            || !tabs.contains(&self.ribbon_tab)
        {
            self.ribbon_tab = crate::ribbon::tab_of(self.tab);
        }
        let l = self.lang;
        let t = Tokens::of(self.prefs.theme);
        let collapsed = self.prefs.ribbon_collapsed;
        let h = 30.0 + if collapsed { 2.0 } else { widgets::RIBBON_H + 14.0 };
        let r = egui::Panel::top("bicraft-ribbon")
            .exact_size(h)
            .frame(egui::Frame::NONE.fill(t.chrome))
            .show(ui, |ui| {
                let rect = ui.max_rect();
                let mut row = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(Rect::from_min_size(
                            pos2(rect.min.x + 12.0, rect.min.y + 2.0),
                            Vec2::new(rect.width() - 24.0, 24.0),
                        ))
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                row.spacing_mut().item_spacing.x = 2.0;
                // the File view (Backstage), in the accent text as Office's File tab
                let file = widgets::tab(&mut row, l.t("File", "Fichier"), false, true);
                crate::key_tips::place_tab(self, &row, RibbonTab::File, file.rect);
                if file
                    .on_hover_text(l.t(
                        "Open, save, export (Alt+F)",
                        "Ouvrir, enregistrer, exporter (Alt+F)",
                    ))
                    .clicked()
                {
                    self.backstage = Some(Backstage::new(backstage::Page::Home));
                }
                let mut toggle = false;
                for rt in tabs {
                    let active = self.ribbon_tab == rt;
                    let r = widgets::tab(&mut row, rt.caption(l), active, rt.contextual());
                    crate::key_tips::place_tab(self, &row, rt, r.rect);
                    if r.clicked() {
                        if active {
                            toggle = true;
                        } else {
                            self.select_ribbon_tab(rt);
                            if self.prefs.ribbon_collapsed {
                                toggle = true;
                            }
                        }
                    }
                    if r.double_clicked() {
                        toggle = true;
                    }
                }
                if toggle {
                    self.prefs.ribbon_collapsed = !self.prefs.ribbon_collapsed;
                    self.save_prefs();
                }
                if collapsed {
                    return;
                }
                let card = Rect::from_min_max(
                    pos2(rect.min.x + 8.0, rect.min.y + 30.0),
                    pos2(rect.max.x - 8.0, rect.min.y + 30.0 + widgets::RIBBON_H + 8.0),
                );
                ui.painter().rect_filled(
                    card.translate(Vec2::new(0.0, 1.0)),
                    CornerRadius::same(10),
                    t.card_border,
                );
                ui.painter().rect_filled(card, CornerRadius::same(10), t.card);
                let inner = Rect::from_min_size(
                    pos2(card.min.x + 10.0, card.min.y + 4.0),
                    Vec2::new(card.width() - 20.0, widgets::RIBBON_H),
                );
                let mut cui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(inner)
                        .layout(egui::Layout::left_to_right(egui::Align::Min)),
                );
                cui.set_clip_rect(inner.expand2(Vec2::new(0.0, 3.0)));
                cui.spacing_mut().item_spacing.x = 1.0;
                let tab = self.ribbon_tab;
                egui::ScrollArea::horizontal()
                    .id_salt("bicraft-ribbon-scroll")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(&mut cui, |ui| {
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = 1.0;
                            crate::ribbon::show(self, ui, tab);
                        });
                    });
            });
        crate::keyboard::section(self, ui, Section::Ribbon, r.response.rect);
    }

    /// Home: the file (what the shell can do), the display.
    pub(crate) fn home_ribbon(&mut self, ui: &mut egui::Ui) {
        let l = self.lang;
        let caps = self.shell.caps;
        widgets::group(ui, |ui| {
            if caps.open_files
                && widgets::big_button(ui, Icon::Folder, l.t("Open…", "Ouvrir…"), true).clicked()
            {
                self.pick_and_open();
            }
            if caps.connect
                && widgets::big_button(
                    ui,
                    Icon::Server,
                    l.t("Connect to server…", "Se connecter à un serveur…"),
                    true,
                )
                .clicked()
            {
                self.connect = Some(self.connect_form(None));
            }
            if widgets::big_button(
                ui,
                Icon::Reload,
                l.t("Reload", "Recharger"),
                self.worker.is_some(),
            )
            .clicked()
            {
                self.reload();
            }
            if caps.open_files || caps.connect || caps.quit {
                widgets::rows(ui, |ui| {
                    if caps.open_files {
                        let recent = self.prefs.recent_paths();
                        let r = widgets::small_button(
                            ui,
                            Icon::File,
                            l.t("Recent files", "Fichiers récents"),
                            true,
                            false,
                        );
                        egui::Popup::menu(&r).show(|ui| {
                            if recent.is_empty() {
                                ui.label(l.t("(none)", "(aucun)"));
                            }
                            for p in recent {
                                if ui.button(clip(&p.display().to_string(), 120)).clicked() {
                                    ui.close();
                                    self.open_path(&p);
                                }
                            }
                        });
                    }
                    if caps.connect {
                        let servers = self.prefs.servers.clone();
                        let r = widgets::small_button(
                            ui,
                            Icon::Server,
                            l.t("Recent servers", "Serveurs récents"),
                            true,
                            false,
                        );
                        egui::Popup::menu(&r).show(|ui| {
                            if servers.is_empty() {
                                ui.label(l.t("(none)", "(aucun)"));
                            }
                            for (u, w) in servers {
                                if ui
                                    .button(format!("{w} · {}", crate::source::host_of(&u)))
                                    .clicked()
                                {
                                    ui.close();
                                    self.connect = Some(self.connect_form(Some((u, w))));
                                }
                            }
                        });
                    }
                    if caps.quit
                        && widgets::small_button(ui, Icon::Close, l.t("Quit", "Quitter"), true, false)
                            .clicked()
                    {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            }
        });
        widgets::group(ui, |ui| {
            // the app's theme: light, dark or high contrast (white on black, 7:1 text)
            let now = self.prefs.theme;
            for (b, icon, label) in [
                (Brightness::Light, Icon::Sun, l.t("Light", "Clair")),
                (Brightness::Dark, Icon::Moon, l.t("Dark", "Sombre")),
                (
                    Brightness::HighContrast,
                    Icon::Contrast,
                    l.t("High contrast", "Contraste élevé"),
                ),
            ] {
                if widgets::toggle_button(ui, icon, label, true, now == b, true).clicked() {
                    self.set_theme(b);
                }
            }
        });
    }

    /// Report: the pages (the zoom is in the status bar, Page view in the View tab).
    pub(crate) fn report_ribbon(&mut self, ui: &mut egui::Ui) {
        let l = self.lang;
        let names = self.page_names();
        let visible: Vec<usize> = names
            .iter()
            .enumerate()
            .filter(|(_, (_, _, hidden))| !hidden)
            .map(|(i, _)| i)
            .collect();
        let active = self.active_page();
        let pos = visible.iter().position(|&i| i == active);
        widgets::group(ui, |ui| {
            let prev = pos
                .and_then(|p| p.checked_sub(1))
                .and_then(|p| visible.get(p).copied());
            let next = pos.and_then(|p| visible.get(p + 1).copied());
            widgets::rows(ui, |ui| {
                if widgets::small_button(
                    ui,
                    Icon::ChevronLeft,
                    l.t("Previous page", "Page précédente"),
                    prev.is_some(),
                    false,
                )
                .clicked()
                    && let Some((name, _, _)) = prev.and_then(|i| names.get(i))
                {
                    self.interact(json!({"action": "page", "page": name}));
                }
                if widgets::small_button(
                    ui,
                    Icon::ChevronRight,
                    l.t("Next page", "Page suivante"),
                    next.is_some(),
                    false,
                )
                .clicked()
                    && let Some((name, _, _)) = next.and_then(|i| names.get(i))
                {
                    self.interact(json!({"action": "page", "page": name}));
                }
            });
        });
    }

    /// Model: the diagram's zoom.
    pub(crate) fn model_ribbon(&mut self, ui: &mut egui::Ui) {
        let l = self.lang;
        let mv = &mut self.model_view;
        let center = mv.canvas.center();
        widgets::group(ui, |ui| {
            if widgets::big_button(
                ui,
                Icon::FitSmall,
                l.t("Fit to window", "Ajuster à la fenêtre"),
                true,
            )
            .clicked()
            {
                mv.fit();
            }
            widgets::rows(ui, |ui| {
                if widgets::small_button(ui, Icon::ZoomIn, l.t("Zoom in", "Zoom avant"), true, false)
                    .clicked()
                {
                    mv.zoom_about(center, 1.25);
                }
                if widgets::small_button(ui, Icon::ZoomOut, l.t("Zoom out", "Zoom arrière"), true, false)
                    .clicked()
                {
                    mv.zoom_about(center, 1.0 / 1.25);
                }
                if widgets::small_button(ui, Icon::Search, l.t("Actual size", "Taille réelle"), true, false)
                    .clicked()
                {
                    mv.zoom = 1.0;
                    mv.pan = Vec2::ZERO;
                }
            });
        });
    }

    /// Validation: the counts.
    pub(crate) fn validation_ribbon(&mut self, ui: &mut egui::Ui) {
        let l = self.lang;
        let t = Tokens::of(self.prefs.theme);
        let v = self
            .loaded
            .as_ref()
            .and_then(|x| x.snap.get("validation").cloned())
            .unwrap_or(Value::Null);
        let n = |p: &str| v.pointer(p).and_then(Value::as_u64).unwrap_or(0);
        widgets::group(ui, |ui| {
            widgets::rows(ui, |ui| {
                ui.add_space(4.0);
                let bad = n("/model/invalid_count");
                widgets::pill(
                    ui,
                    &format!(
                        "{} {bad}",
                        l.t("Invalid model objects:", "Objets du modèle non valides :")
                    ),
                    Some(if bad > 0 { t.err } else { t.ok }),
                    true,
                );
                ui.add_space(4.0);
                let (e, w) = (n("/report/error_count"), n("/report/warning_count"));
                widgets::pill(
                    ui,
                    &format!(
                        "{} {e} · {} {w}",
                        l.t("Report errors:", "Erreurs du rapport :"),
                        l.t("warnings:", "avertissements :")
                    ),
                    Some(if e > 0 {
                        t.err
                    } else if w > 0 {
                        t.warn
                    } else {
                        t.ok
                    }),
                    true,
                );
            });
        });
    }

    /// The status bar: the page, the status, the role, the connection, the last command, the
    /// view buttons.
    /// The status bar's zoom, right to left: the percentage, zoom in, the slider, zoom out, then
    /// Actual size, Fit width and Fit page. A view setting of this window: the file keeps the
    /// page's View › Page view.
    fn zoom_controls(&mut self, ui: &mut egui::Ui) {
        let l = self.lang;
        let t = Tokens::of(self.prefs.theme);
        let now = self.placed.map_or(100.0, |p| p.zoom * 100.0);
        let pct = (now.round() as u16).clamp(ZOOM_MIN, ZOOM_MAX);
        ui.label(
            RichText::new(format!("{} %", now.round() as u32))
                .font(theme::font(12.0))
                .color(t.text_dim),
        );
        // the next step up or down from the zoom shown
        let up = (pct / ZOOM_STEP + 1).saturating_mul(ZOOM_STEP).min(ZOOM_MAX);
        let down = (pct.div_ceil(ZOOM_STEP).saturating_sub(1))
            .saturating_mul(ZOOM_STEP)
            .max(ZOOM_MIN);
        if widgets::status_icon(
            ui,
            Icon::ZoomIn,
            l.t("Zoom in on the page", "Zoom avant sur la page"),
            false,
        )
        .clicked()
        {
            self.view_zoom = Some(ViewZoom::Percent(up));
        }
        let mut v = pct;
        ui.spacing_mut().slider_width = 96.0;
        let label = l.t("Page zoom", "Zoom de la page");
        let r = ui
            .add(egui::Slider::new(&mut v, ZOOM_MIN..=ZOOM_MAX).show_value(false))
            .on_hover_text(l.t("Page zoom (10 to 400 %)", "Zoom de la page (10 à 400 %)"));
        r.widget_info(|| egui::WidgetInfo::slider(true, f64::from(v), label));
        if r.changed() {
            self.view_zoom = Some(ViewZoom::Percent(v));
        }
        if widgets::status_icon(
            ui,
            Icon::ZoomOut,
            l.t("Zoom out of the page", "Zoom arrière sur la page"),
            false,
        )
        .clicked()
        {
            self.view_zoom = Some(ViewZoom::Percent(down));
        }
        ui.add_space(4.0);
        let mode = self.view_zoom.unwrap_or_else(|| self.page_zoom());
        for (z, icon, tip) in [
            (
                ViewZoom::Actual,
                Icon::Search,
                l.t("Actual size", "Taille réelle"),
            ),
            (
                ViewZoom::FitWidth,
                Icon::FitWidth,
                l.t("Fit width", "Ajuster à la largeur"),
            ),
            (
                ViewZoom::FitPage,
                Icon::FitSmall,
                l.t("Fit page", "Ajuster à la page"),
            ),
        ] {
            if widgets::status_icon(ui, icon, tip, mode == z).clicked() {
                self.view_zoom = Some(z);
            }
        }
    }

    /// The active page's View › Page view (its `displayOption` in the snapshot).
    pub fn page_zoom(&self) -> ViewZoom {
        let page = self.active_page();
        ViewZoom::of_page(self.loaded.as_ref().and_then(|l| {
            l.snap
                .pointer(&format!("/report/pages/{page}/display_option"))
                .and_then(Value::as_str)
        }))
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let l = self.lang;
        let t = Tokens::of(self.prefs.theme);
        let r = egui::Panel::bottom("bicraft-status")
            .exact_size(STATUS_H)
            .frame(
                egui::Frame::NONE
                    .fill(t.chrome)
                    .inner_margin(egui::Margin::symmetric(14, 0)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let names = self.page_names();
                    if self.tab == Tab::Report && !names.is_empty() {
                        let n = names.len();
                        let i = self.active_page().saturating_add(1).min(n);
                        ui.label(
                            RichText::new(match l {
                                Lang::En => format!("Page {i} of {n}"),
                                Lang::Fr => format!("Page {i} sur {n}"),
                            })
                            .font(theme::font(12.0))
                            .color(t.text_dim),
                        );
                    }
                    if let Some(s) = crate::canvas::status(self) {
                        ui.label(RichText::new(s).font(theme::font(12.0)).color(t.text));
                    }
                    if self.tab == Tab::Data
                        && let Some(s) = crate::data_view::status(self)
                    {
                        ui.label(
                            RichText::new(clip(&s, 160))
                                .font(theme::font(12.0))
                                .color(t.text_dim),
                        );
                    }
                    // the status message is a polite live region, drawn even when empty so a
                    // screen reader knows it before it speaks (crate::keyboard announces where
                    // the focus went through it)
                    let status = ui.label(
                        RichText::new(clip(&self.status, 160))
                            .font(theme::font(12.0))
                            .color(t.text_dim),
                    );
                    crate::keyboard::live_region(ui.ctx(), status.id, &clip(&self.status, 160));
                    if self.pages.as_ref().is_some_and(Pages::busy) {
                        ui.spinner();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for (tab, icon, tip) in [
                            (
                                Tab::Validation,
                                Icon::Validate,
                                l.t("Validation view", "Vue validation"),
                            ),
                            (Tab::Tmdl, Icon::File, l.t("TMDL view", "Vue TMDL")),
                            (
                                Tab::Query,
                                Icon::Measure,
                                l.t("DAX query view", "Vue de requête DAX"),
                            ),
                            (Tab::Model, Icon::Model, l.t("Model view", "Vue modèle")),
                            (Tab::Data, Icon::Table, l.t("Data view", "Vue données")),
                            (Tab::Report, Icon::Page, l.t("Report view", "Vue rapport")),
                        ] {
                            if widgets::status_icon(ui, icon, tip, self.tab == tab).clicked() {
                                self.show_tab(tab);
                            }
                        }
                        ui.add_space(8.0);
                        if self.tab == Tab::Report && self.placed.is_some() {
                            self.zoom_controls(ui);
                            ui.add_space(8.0);
                        }
                        if let Some(a) = &self.access {
                            let editor = a == "editor";
                            let role = if editor {
                                l.t("editor", "éditeur")
                            } else {
                                l.t("reader", "lecteur")
                            };
                            widgets::pill(ui, role, editor.then_some(t.accent), true);
                        }
                        if let Some((label, dot)) = self.connection() {
                            widgets::pill(ui, &label, Some(dot), false);
                        }
                        if let Some((label, dot)) = self.last_command() {
                            widgets::pill(ui, &label, Some(dot), false);
                        }
                        if let Some((label, dot, tip)) = crate::recovery::status_pill(self) {
                            widgets::pill(ui, &label, Some(dot), false).on_hover_text(tip);
                        }
                    });
                });
            });
        crate::keyboard::section(self, ui, Section::Status, r.response.rect);
    }

    /// The connection pill: live (a snapshot and no error) or connecting.
    fn connection(&self) -> Option<(String, Color32)> {
        let t = Tokens::of(self.prefs.theme);
        let l = self.lang;
        self.origin.as_ref()?;
        Some(if self.loaded.is_some() && self.error.is_none() {
            (l.t("live", "en direct").to_string(), t.live)
        } else if self.error.is_some() {
            (l.t("offline", "hors ligne").to_string(), t.err)
        } else {
            (l.t("connecting", "connexion").to_string(), t.warn)
        })
    }

    /// The last command pill: `✓ id` or `✕ id`, as the viewer's.
    fn last_command(&self) -> Option<(String, Color32)> {
        let t = Tokens::of(self.prefs.theme);
        let lc = self.loaded.as_ref()?.snap.get("last_command")?;
        let id = lc.get("id").and_then(Value::as_str).filter(|s| !s.is_empty())?;
        let ok = lc.get("ok").and_then(Value::as_bool).unwrap_or(true);
        Some((
            format!("{} {}", if ok { "✓" } else { "✕" }, clip(id, 60)),
            if ok { t.ok } else { t.err },
        ))
    }

    /// Nothing open: the mark, Open and Connect, recent files and servers (DeckCraft's start
    /// screen).
    fn start_screen(&mut self, ui: &mut egui::Ui) {
        let l = self.lang;
        let t = Tokens::of(self.prefs.theme);
        let caps = self.shell.caps;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui::Frame::NONE
                .inner_margin(egui::Margin { left: 40, top: 28, right: 40, bottom: 28 })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
                        theme::paint_logo(ui.painter(), r);
                        ui.add_space(6.0);
                        ui.label(RichText::new("BIcraft").font(theme::bold(26.0)).color(t.text));
                    });
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(l.t(
                            "Power BI files, read and checked in pure Rust.",
                            "Les fichiers Power BI, lus et vérifiés en Rust pur.",
                        ))
                        .color(t.text_dim),
                    );
                    ui.add_space(18.0);
                    if !caps.open_files && !caps.connect {
                        ui.label(RichText::new(l.t("Nothing open", "Rien d'ouvert")).font(theme::bold(16.0)));
                        return;
                    }
                    ui.horizontal(|ui| {
                        if caps.open_files
                            && widgets::primary(ui, l.t("Open a file…", "Ouvrir un fichier…"), true, "").clicked()
                        {
                            self.pick_and_open();
                        }
                        if caps.connect && ui.button(l.t("Connect to server…", "Se connecter à un serveur…")).clicked() {
                            self.connect = Some(self.connect_form(None));
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
                    if caps.open_files && !self.prefs.recent_paths().is_empty() {
                        ui.add_space(18.0);
                        ui.label(RichText::new(l.t("Recent files", "Fichiers récents")).font(theme::bold(16.0)));
                        ui.add_space(4.0);
                        for p in self.prefs.recent_paths() {
                            if ui.link(clip(&p.display().to_string(), 140)).clicked() {
                                self.open_path(&p);
                            }
                        }
                    }
                    if caps.connect && !self.prefs.servers.is_empty() {
                        ui.add_space(18.0);
                        ui.label(RichText::new(l.t("Recent servers", "Serveurs récents")).font(theme::bold(16.0)));
                        ui.add_space(4.0);
                        for (u, w) in self.prefs.servers.clone() {
                            if ui.link(format!("{w} · {}", crate::source::host_of(&u))).clicked() {
                                self.connect = Some(self.connect_form(Some((u, w))));
                            }
                        }
                    }
                });
        });
    }

    fn shortcuts(&mut self, ui: &egui::Ui) {
        use egui::Key as K;
        // the command palette's key, and nothing else while it or a form is open
        if crate::palette::keys(self, ui) {
            return;
        }
        let typing = ui.ctx().egui_wants_keyboard_input();
        let (save, file, next_doc, prev_doc) = ui.input(|i| {
            (
                i.modifiers.command && !i.modifiers.shift && i.key_pressed(K::S),
                i.modifiers.alt && !i.modifiers.command && i.key_pressed(K::F),
                i.modifiers.ctrl && !i.modifiers.shift && i.key_pressed(K::Tab),
                i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(K::Tab),
            )
        });
        if save && self.has_doc() {
            self.save();
        }
        if file && self.backstage.is_none() {
            self.backstage = Some(Backstage::new(backstage::Page::Home));
            return;
        }
        let n = self.docs.len();
        if n > 1 && (next_doc || prev_doc) {
            let i = if next_doc {
                (self.active + 1) % n
            } else {
                (self.active + n - 1) % n
            };
            self.switch_doc(i);
        }
        if self.backstage.is_some() {
            let dialog = self.confirm.is_some()
                || self.connect.is_some()
                || self.backstage.as_ref().is_some_and(|b| b.new_name.is_some());
            if dialog {
                return;
            }
            for k in [K::Escape, K::ArrowDown, K::ArrowUp, K::Enter, K::Space] {
                let arrows = !matches!(k, K::Escape);
                if (!arrows || !typing) && ui.input(|i| i.key_pressed(k)) && backstage::key(self, k) {
                    break;
                }
            }
            return;
        }
        // the DAX query view's Run (Ctrl+Enter, F5), even while its editor has the focus
        if crate::dax_query::keys(self, ui) {
            return;
        }
        // the TMDL view's Apply (Ctrl+Enter)
        if crate::tmdl_view::keys(self, ui) {
            return;
        }
        // the canvas's copy, paste, Ctrl+A and arrows (crate::canvas)
        crate::canvas::keys(self, ui);
        // the registry's chords (F5 is `model.refresh`, Delete a selected visual's)
        crate::ribbon::shortcuts(self, ui);
        let (open, esc) = ui.input(|i| {
            (
                i.modifiers.command && i.key_pressed(egui::Key::O),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if open && self.shell.caps.open_files {
            self.pick_and_open();
        }
        // Escape in a rename box drops the rename only; the formula bar's editor takes its own
        // Escape leaves the Edit interactions mode first (the source stays selected)
        if esc
            && self.cmd.edit_interactions
            && self.panes.renaming.is_none()
            && !crate::formula::has_focus(ui.ctx())
        {
            crate::interactions::leave(self);
            return;
        }
        // Escape leaves focus mode first (the visual stays selected)
        if esc
            && crate::focus::active(self)
            && self.tab == Tab::Report
            && self.panes.renaming.is_none()
            && !crate::formula::has_focus(ui.ctx())
        {
            crate::focus::exit(self);
            return;
        }
        // then the spotlight
        if esc
            && crate::focus::spotlight(self).is_some()
            && self.tab == Tab::Report
            && self.panes.renaming.is_none()
            && !crate::formula::has_focus(ui.ctx())
        {
            crate::focus::end_spotlight(self);
            return;
        }
        if esc && self.panes.renaming.is_none() && !crate::formula::has_focus(ui.ctx()) {
            self.popup = None;
            self.cmd.visual = None;
            if self.tab == Tab::Model {
                self.model_view.sel = None;
            }
        }
    }

    pub(crate) fn pick_and_open(&mut self) {
        let picked = self.shell.pick_file.as_ref().and_then(|f| f(self.lang));
        if let Some(p) = picked {
            self.open_path(&p);
        }
    }

    pub(crate) fn connect_form(&self, server: Option<(String, String)>) -> ConnectForm {
        let (url, workspace) = server
            .or_else(|| self.prefs.servers.first().cloned())
            .unwrap_or_default();
        ConnectForm {
            url,
            workspace,
            token: self.shell.server_token.clone(),
        }
    }

    /// The Connect dialog (DeckCraft's dialog: centred, label column, primary then Cancel at the
    /// right, Enter connects, Escape cancels).
    fn connect_ui(&mut self, ctx: &egui::Context) {
        let l = self.lang;
        let t = Tokens::of(self.prefs.theme);
        let Some(form) = &mut self.connect else { return };
        let mut go = false;
        let mut cancel = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        egui::Window::new(l.t("Connect to a BIcraft server", "Se connecter à un serveur BIcraft"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::new(0.0, -40.0))
            .show(ctx, |ui| {
                ui.set_min_width(340.0);
                let field = |ui: &mut egui::Ui, label: &str, edit: egui::TextEdit<'_>| {
                    ui.horizontal(|ui| {
                        ui.allocate_ui_with_layout(
                            Vec2::new(120.0, 22.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.set_min_width(120.0);
                                ui.label(label);
                            },
                        );
                        ui.add(edit.desired_width(220.0));
                    });
                };
                field(
                    ui,
                    l.t("Server URL", "URL du serveur"),
                    egui::TextEdit::singleline(&mut form.url).hint_text("https://bi.example.com"),
                );
                field(
                    ui,
                    l.t("Workspace", "Espace de travail"),
                    egui::TextEdit::singleline(&mut form.workspace),
                );
                field(
                    ui,
                    l.t("Token", "Jeton"),
                    egui::TextEdit::singleline(&mut form.token).password(true),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(l.t(
                        "An API token or an OIDC access token; kept in memory only.",
                        "Un jeton d'API ou un jeton d'accès OIDC ; gardé en mémoire uniquement.",
                    ))
                    .small()
                    .color(t.text_dim),
                );
                ui.add_space(8.0);
                let w = ui.available_width();
                ui.allocate_ui_with_layout(
                    Vec2::new(w, 28.0),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        let ok = !form.url.trim().is_empty()
                            && crate::remote::valid_workspace(form.workspace.trim());
                        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let reason = l.t(
                            "A server URL and a workspace name are needed",
                            "Il faut l'URL du serveur et le nom de l'espace de travail",
                        );
                        if widgets::primary(ui, l.t("Connect", "Se connecter"), ok, reason).clicked()
                            || (ok && enter)
                        {
                            go = true;
                        }
                        if ui
                            .add(egui::Button::new(l.t("Cancel", "Annuler")).min_size(Vec2::new(72.0, 24.0)))
                            .clicked()
                        {
                            cancel = true;
                        }
                    },
                );
            });
        if go && let Some(f) = self.connect.take() {
            self.open_server(Server {
                url: f.url.trim().to_string(),
                workspace: f.workspace.trim().to_string(),
                token: f.token.trim().to_string(),
            });
        } else if cancel {
            self.connect = None;
        }
    }

    // --- report ---------------------------------------------------------------------------

    /// The snapshot's pages: (name, label, hidden).
    pub fn page_names(&self) -> Vec<(String, String, bool)> {
        let Some(l) = &self.loaded else { return Vec::new() };
        l.snap
            .pointer("/report/pages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(1000)
            .map(|p| {
                let name = p.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                let label = p
                    .get("display_name")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&name)
                    .chars()
                    .take(120)
                    .collect();
                let hidden = p.get("hidden").and_then(Value::as_bool).unwrap_or(false);
                (name, label, hidden)
            })
            .collect()
    }

    /// The index of the page shown: the reader's page (`report.active_page`), else the first.
    pub fn active_page(&self) -> usize {
        let Some(l) = &self.loaded else { return 0 };
        let active = l
            .snap
            .pointer("/report/active_page")
            .and_then(Value::as_str)
            .unwrap_or("");
        self.page_names()
            .iter()
            .position(|(n, _, _)| n == active)
            .unwrap_or(0)
    }

    fn report(&mut self, ui: &mut egui::Ui) {
        let l = self.lang;
        let Some(loaded) = self.loaded.clone() else {
            if self.origin.is_some() {
                ui.centered_and_justified(|ui| ui.spinner());
            } else {
                empty(ui, l.t("Nothing open", "Rien d'ouvert"), "");
            }
            return;
        };
        if !loaded.snap.get("report").is_some_and(Value::is_object) {
            // a model-only file (.bim): the Model and Validation tabs are what it is for
            empty(
                ui,
                l.t("No report in this file", "Pas de rapport dans ce fichier"),
                l.t(
                    "Its model is in the Model and Validation tabs.",
                    "Son modèle est dans les onglets Modèle et Validation.",
                ),
            );
            return;
        }
        let names = self.page_names();
        if names.is_empty() {
            let truncated = loaded.snap.pointer("/report/truncated").and_then(Value::as_bool) == Some(true);
            empty(
                ui,
                l.t("The report has no pages yet", "Le rapport n'a pas encore de page"),
                if truncated {
                    l.t(
                        "The page list was truncated.",
                        "La liste des pages a été tronquée.",
                    )
                } else {
                    ""
                },
            );
            return;
        }
        let page = self.active_page();
        // focus mode: the derived page (crate::focus), drawn and hit as the page, under a Back
        // to report bar; the canvas's helpers read it while the page is drawn
        let focused = crate::focus::loaded(self);
        if focused.is_some() {
            crate::focus::bar(self, ui);
        }
        // the phone canvas (View › Mobile layout, or a narrow window): a derived page too
        // (crate::mobile)
        let phone = crate::mobile::loaded(self, ui.available_width());
        let focused = focused.or(phone.clone());
        let loaded = focused.clone().unwrap_or(loaded);
        let pv = loaded
            .snap
            .pointer(&format!("/report/pages/{page}"))
            .cloned()
            .unwrap_or(Value::Null);
        let num = |k: &str, d: f64| {
            pv.get(k)
                .and_then(Value::as_f64)
                .filter(|x| x.is_finite() && *x > 0.0)
                .unwrap_or(d)
        };
        let (pw, ph) = (
            num("width", 1280.0).min(20_000.0) as f32,
            num("height", 720.0).min(20_000.0) as f32,
        );
        let avail = ui.available_size();
        let mode = self
            .view_zoom
            .unwrap_or_else(|| ViewZoom::of_page(pv.get("display_option").and_then(Value::as_str)));
        let ppp = ui.ctx().pixels_per_point();
        let render_at = |this: &mut App, ui: &mut egui::Ui, zoom: f32| {
            let real = focused.as_ref().map(|f| this.loaded.replace(f.clone()));
            let size = Vec2::new(pw * zoom, ph * zoom);
            let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
            this.draw_page(ui, &loaded, page, rect, zoom * ppp);
            if crate::mobile::drawn(this) {
                crate::mobile::bezel(ui, rect);
            }
            this.placed = Some(Placed { rect, zoom });
            this.page_input(ui, &resp, rect, zoom);
            crate::point_keys::paint(this, ui, rect, zoom);
            if real.is_none() {
                crate::rich::canvas(this, ui, &resp, rect, zoom);
            }
            if let Some(r) = real {
                this.loaded = r;
            }
        };
        if phone.is_some() {
            // the phone canvas: portrait, scrolled down when its visuals go further
            // its rendered image kept within MAX_ZOOMED pixels high (the edited canvas is 3600)
            let zoom = crate::mobile::zoom(self, avail).min(MAX_ZOOMED / (ph * ppp).max(1.0));
            egui::ScrollArea::vertical()
                .id_salt("bicraft-page-phone")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(16.0);
                        render_at(self, ui, zoom);
                        ui.add_space(16.0);
                    });
                });
        } else if let Some(zoom) = match mode {
            ViewZoom::Actual => Some(1.0),
            // the slider's zoom, the page's image kept within MAX_ZOOMED pixels a side
            ViewZoom::Percent(p) => Some((f32::from(p) / 100.0).min(MAX_ZOOMED / pw.max(ph))),
            _ => None,
        } {
            egui::ScrollArea::both()
                .id_salt("bicraft-page-actual")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // centred while it fits, scrolled from its corner when it does not
                    let pad = ((avail.x - pw * zoom) / 2.0).max(16.0);
                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        ui.add_space(pad);
                        render_at(self, ui, zoom);
                        ui.add_space(16.0);
                    });
                    ui.add_space(16.0);
                });
        } else if mode == ViewZoom::FitWidth {
            let zoom = ((avail.x - 40.0).max(50.0) / pw).clamp(0.05, 8.0);
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(16.0);
                    render_at(self, ui, zoom);
                    ui.add_space(16.0);
                });
            });
        } else {
            let zoom = ((avail.x - 32.0).max(50.0) / pw)
                .min((avail.y - 32.0).max(50.0) / ph)
                .clamp(0.05, 8.0);
            ui.vertical_centered(|ui| {
                // Format › Canvas settings › Vertical alignment: Top puts the page at the top
                let top = pv.get("vertical_alignment").and_then(Value::as_str) == Some("top");
                let pad = if top {
                    16.0
                } else {
                    ((avail.y - ph * zoom) / 2.0).max(0.0)
                };
                ui.add_space(pad.min(avail.y));
                render_at(self, ui, zoom);
            });
        }
    }

    /// The texture of the page image on screen and the snapshot it was rendered from, when it is
    /// the page shown itself (not a focus mode's or the phone layout's view of it).
    pub(crate) fn page_texture(&self) -> Option<(egui::TextureId, u64)> {
        let (k, t) = self.texture.as_ref()?;
        (k.page == self.active_page() && k.view == 0).then(|| (t.id(), k.seq))
    }

    /// Draws page `page` into `rect`, rendered at `scale` device pixels per report pixel.
    /// The rendered image lands on whole device pixels at its own size ([`texel_rect`]): a page
    /// placed at a fractional pixel, or stretched by a pixel, is resampled and reads blurred.
    /// The colour the page shows behind `r` (screen points): the rendered page's pixels sampled
    /// just inside `r`'s corners, the most frequent one (the box's own fill, or the page's under a
    /// transparent box). `None` before a page is drawn.
    pub(crate) fn backdrop_at(&self, r: Rect) -> Option<Color32> {
        let placed = self.placed.as_ref()?;
        let img = self.shown.as_ref()?.1.as_ref().ok()?;
        let (w, h) = (img.width as usize, img.height as usize);
        if w == 0 || h == 0 || placed.rect.width() <= 0.0 || placed.rect.height() <= 0.0 {
            return None;
        }
        let (sx, sy) = (w as f32 / placed.rect.width(), h as f32 / placed.rect.height());
        let inset = 4.0_f32.min(r.width() / 4.0).min(r.height() / 4.0).max(0.0);
        let inner = r.shrink(inset);
        let mut seen: Vec<(Color32, u32)> = Vec::new();
        for p in [
            inner.left_top(),
            inner.right_top(),
            inner.left_bottom(),
            inner.right_bottom(),
        ] {
            let x = ((p.x - placed.rect.min.x) * sx).floor();
            let y = ((p.y - placed.rect.min.y) * sy).floor();
            if !(x >= 0.0 && y >= 0.0 && (x as usize) < w && (y as usize) < h) {
                continue;
            }
            let i = ((y as usize) * w + x as usize) * 4;
            let Some(px) = img.rgba.get(i..i + 4) else {
                continue;
            };
            let c = Color32::from_rgba_premultiplied(px[0], px[1], px[2], px[3]);
            match seen.iter_mut().find(|s| s.0 == c) {
                Some(s) => s.1 += 1,
                None => seen.push((c, 1)),
            }
        }
        seen.into_iter().max_by_key(|s| s.1).map(|s| s.0)
    }

    fn draw_page(&mut self, ui: &egui::Ui, loaded: &Loaded, page: usize, rect: Rect, scale: f32) {
        // DeckCraft's slide shadow under the page
        let shadow = Tokens::of(self.prefs.theme).shadow;
        ui.painter().rect_filled(
            rect.translate(Vec2::new(0.0, 1.5)).expand(1.0),
            CornerRadius::same(2),
            shadow,
        );
        let view = crate::focus::current(self)
            .map(|f| f.view_key())
            .or_else(|| crate::mobile::view_key(self))
            .unwrap_or(0);
        let pointer = self.pointer.clone();
        let key = Key::new(loaded.seq, page, scale)
            .with_view(view)
            .with_contrast(self.prefs.report_contrast)
            .with_pointer(pointer.as_ref())
            .with_gantt(&self.gantt)
            .with_grid(crate::canvas::grid_under(
                self,
                scale / ui.ctx().pixels_per_point(),
            ));
        let gantt = self.gantt.clone();
        let got = self
            .pages
            .as_mut()
            .and_then(|p| p.get_at(loaded, key, pointer.as_ref(), &gantt));
        if let Some(r) = got {
            self.shown = Some(r);
        }
        let Some((k, r)) = self.shown.clone() else {
            ui.painter().rect_filled(rect, 0.0, Color32::WHITE);
            return;
        };
        match r {
            Ok(img) => {
                if self.texture.as_ref().map(|t| t.0) != Some(k) {
                    let ci = egui::ColorImage::from_rgba_premultiplied(
                        [img.width as usize, img.height as usize],
                        &img.rgba,
                    );
                    match &mut self.texture {
                        Some((tk, t)) => {
                            t.set(ci, TextureOptions::LINEAR);
                            *tk = k;
                        }
                        None => {
                            let t = ui.ctx().load_texture("bicraft-page", ci, TextureOptions::LINEAR);
                            self.texture = Some((k, t));
                        }
                    }
                }
                if let Some((_, t)) = &self.texture {
                    let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                    let at = if k == key {
                        texel_rect(rect, [img.width, img.height], ui.ctx().pixels_per_point())
                    } else {
                        rect
                    };
                    ui.painter().image(t.id(), at, uv, Color32::WHITE);
                }
                // a black page on the black high-contrast canvas: its edge outlined
                if self.prefs.theme == Brightness::HighContrast {
                    ui.painter().rect_stroke(
                        rect,
                        0.0,
                        egui::Stroke::new(1.0, Tokens::high_contrast().border),
                        egui::StrokeKind::Outside,
                    );
                }
            }
            Err(e) => {
                ui.painter().rect_filled(rect, 0.0, Color32::WHITE);
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    e,
                    egui::FontId::proportional(14.0),
                    Color32::DARK_RED,
                );
            }
        }
    }

    /// The hit under a screen point: report coordinates through the placement.
    pub(crate) fn hit_at_screen(&self, pos: Pos2) -> Option<bicraft_render::hits::Hit> {
        let placed = self.placed?;
        let (_, Ok(img)) = self.shown.as_ref()? else {
            return None;
        };
        let x = (pos.x - placed.rect.min.x) / placed.zoom;
        let y = (pos.y - placed.rect.min.y) / placed.zoom;
        hit_at(&img.hits, x, y)
            .filter(|h| h.visual != crate::focus::TABLE_VISUAL)
            .cloned()
    }

    /// Opens a hit map popup (`spec`) at screen point `at`.
    pub(crate) fn open_popup(&mut self, spec: Value, at: Pos2) {
        self.popup = Some(Popup::new(spec, at));
    }

    /// A click at a report position (report pixels): what the viewer would do. Returns the
    /// action sent, if any.
    pub fn click_report(&mut self, x: f32, y: f32, ctrl: bool, screen: Pos2) -> Option<Value> {
        let (_, Ok(img)) = self.shown.as_ref()? else {
            return None;
        };
        // the data table of Show as a table is not clickable (crate::focus)
        let hit = hit_at(&img.hits, x, y)
            .filter(|h| h.visual != crate::focus::TABLE_VISUAL)
            .cloned()?;
        match &hit.action {
            HitAction::Send { .. } => {
                let a = hit.action_for(ctrl)?;
                // editing: a button acts on Ctrl+click only (Desktop's editing view); a plain
                // click selects it
                if crate::rich::click_needs_ctrl(self, &a, ctrl) {
                    self.popup = None;
                    return None;
                }
                self.popup = None;
                self.interact(a.clone());
                Some(a)
            }
            HitAction::Popup(spec) => {
                self.popup = Some(Popup::new(spec.clone(), screen));
                None
            }
            HitAction::None | HitAction::Menu(_) => {
                self.popup = None;
                None
            }
        }
    }

    fn page_input(&mut self, ui: &egui::Ui, resp: &egui::Response, rect: Rect, zoom: f32) {
        crate::menus::on_page(self, ui, resp, rect, zoom);
        // drill icons over the hovered visual, report-page tooltips (drill.rs)
        let on_icon = crate::drill::header(self, ui, rect, zoom);
        // a matrix's +/- buttons as labelled widgets (the accessibility tree), clickable
        let on_icon = crate::drill::expand_buttons(self, ui, rect, zoom) || on_icon;
        // a decomposition tree's "+", remove crosses and scroll arrows, likewise (G34)
        let on_icon = crate::decomp_ui::buttons(self, ui, rect, zoom) || on_icon;
        // a what-if parameter's single-value slider as a slider widget (value_slider.rs)
        let on_icon = crate::value_slider::overlay(self, ui, rect, zoom) || on_icon;
        // paused visual queries: each waiting visual's pending-changes strip (optimize_ui.rs)
        let on_icon = crate::optimize_ui::pending_strips(self, ui, rect, zoom) || on_icon;
        // query reduction's Apply button on each slicer
        let on_icon = crate::optimize_ui::slicer_apply(self, ui, rect, zoom) || on_icon;
        // a Gantt's expand/collapse icons, scroll bar and time line zoom (G99)
        let on_icon = crate::gantt_ui::controls(self, ui, rect, zoom) || on_icon;
        let hovered = resp
            .hover_pos()
            .and_then(|p| self.hit_at_screen(p))
            .filter(|_| !on_icon);
        // paused: no report-page tooltip (it would need its queries), and a waiting visual
        // shows no tooltip at all
        let paused = crate::optimize_ui::paused(self);
        let page_tip = !paused && crate::drill::tooltip(self, ui, resp, hovered.as_ref());
        let hovered_waits = hovered
            .as_ref()
            .is_some_and(|h| crate::optimize_ui::visual_pending(self, &h.visual));
        // a button, shape or navigator button under the pointer is drawn in its hover or press
        // state (Desktop's button states); the page is drawn again when that changes
        let pressed = ui.input(|i| i.pointer.primary_down());
        let at = resp
            .hover_pos()
            .map(|p| Pos2::new((p.x - rect.min.x) / zoom, (p.y - rect.min.y) / zoom));
        let pointer = crate::rich::pointer_state(self, at, hovered.as_ref(), pressed);
        if pointer != self.pointer {
            self.pointer = pointer;
            ui.ctx().request_repaint();
        }
        if let Some(h) = hovered {
            if !h.tip.is_empty() && !page_tip && !hovered_waits {
                let tip = h.tip.clone();
                let _ = resp.clone().on_hover_ui_at_pointer(|ui| {
                    egui::Grid::new("bicraft-tip").num_columns(2).show(ui, |ui| {
                        for (k, v) in tip.iter().take(12) {
                            ui.label(RichText::new(clip(k, 80)).weak());
                            ui.label(clip(v, 120));
                            ui.end_row();
                        }
                    });
                });
            }
            // a region that only carries a right-click menu or the app's controls is not a link
            if matches!(h.action, HitAction::Send { .. } | HitAction::Popup(_)) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        }
        if resp.clicked()
            && !on_icon
            && let Some(pos) = resp.interact_pointer_pos()
        {
            // a click outside the visual in spotlight ends it (and does nothing else)
            if let Some((_, lit)) = crate::focus::spotlight(self)
                && self.hit_at_screen(pos).is_none_or(|h| h.visual != lit)
            {
                crate::focus::end_spotlight(self);
                return;
            }
            let ctrl = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
            let (x, y) = ((pos.x - rect.min.x) / zoom, (pos.y - rect.min.y) / zoom);
            self.click_report(x, y, ctrl, pos);
        }
    }

    fn popup_ui(&mut self, ctx: &egui::Context) {
        let l = self.lang;
        let Some(p) = &mut self.popup else { return };
        let mut send: Option<Value> = None;
        let mut close = false;
        let ctrl = ctx.input(|i| i.modifiers.command || i.modifiers.ctrl);
        let fresh_now = p.fresh;
        let area = egui::Area::new(egui::Id::unique("bicraft-popup"))
            .fixed_pos(p.at + Vec2::new(0.0, 8.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    let kind = p
                        .spec
                        .get("kind")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    match kind.as_str() {
                        "dropdown" => {
                            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                                if let Some(sa) = p.spec.get("select_all").filter(|s| s.is_object()) {
                                    let mut on = sa.get("on").and_then(Value::as_bool).unwrap_or(false);
                                    let label =
                                        sa.get("label").and_then(Value::as_str).unwrap_or("Select all");
                                    if ui.checkbox(&mut on, clip(label, 120)).clicked() {
                                        send = Some(p.action("slicer_select_all", json!({})));
                                    }
                                }
                                let items = p
                                    .spec
                                    .get("items")
                                    .and_then(Value::as_array)
                                    .cloned()
                                    .unwrap_or_default();
                                for it in items.iter().take(500) {
                                    let mut on = it.get("on").and_then(Value::as_bool).unwrap_or(false);
                                    let label = it.get("label").and_then(Value::as_str).unwrap_or("");
                                    if ui.checkbox(&mut on, clip(label, 120)).clicked() {
                                        let v = it.get("value").cloned().unwrap_or(Value::Null);
                                        send = Some(
                                            p.action("slicer_toggle", json!({"value": v, "ctrl": ctrl})),
                                        );
                                    }
                                }
                            });
                        }
                        "range" => {
                            let is_date = p.spec.get("is_date").and_then(Value::as_bool).unwrap_or(false);
                            let hint = if is_date { "YYYY-MM-DD" } else { "" };
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut p.from)
                                        .desired_width(110.0)
                                        .hint_text(hint),
                                );
                                ui.add(
                                    egui::TextEdit::singleline(&mut p.to)
                                        .desired_width(110.0)
                                        .hint_text(hint),
                                );
                            });
                            ui.horizontal(|ui| {
                                if ui.button(l.t("Apply", "Appliquer")).clicked() {
                                    let read = |t: &str| -> Value {
                                        let t = t.trim();
                                        if t.is_empty() {
                                            Value::Null
                                        } else if is_date {
                                            Value::String(t.to_string())
                                        } else {
                                            t.replace(',', ".")
                                                .parse::<f64>()
                                                .ok()
                                                .map_or(Value::String(t.to_string()), |x| json!(x))
                                        }
                                    };
                                    send = Some(p.action(
                                        "slicer_range",
                                        json!({"from": read(&p.from), "to": read(&p.to)}),
                                    ));
                                }
                                if ui.button(l.t("Clear", "Effacer")).clicked() {
                                    send = Some(p.action("slicer_clear", json!({})));
                                }
                            });
                        }
                        "relative" => {
                            ui.horizontal(|ui| {
                                egui::ComboBox::from_id_salt("bicraft-rel-range")
                                    .selected_text(p.range.clone())
                                    .show_ui(ui, |ui| {
                                        for r in ["last", "next", "this"] {
                                            ui.selectable_value(&mut p.range, r.to_string(), r);
                                        }
                                    });
                                ui.add(egui::TextEdit::singleline(&mut p.duration).desired_width(40.0));
                                egui::ComboBox::from_id_salt("bicraft-rel-period")
                                    .selected_text(p.period.clone())
                                    .show_ui(ui, |ui| {
                                        for r in PERIODS {
                                            ui.selectable_value(&mut p.period, r.to_string(), r);
                                        }
                                    });
                            });
                            ui.horizontal(|ui| {
                                if ui.button(l.t("Apply", "Appliquer")).clicked() {
                                    let d = p
                                        .duration
                                        .trim()
                                        .parse::<f64>()
                                        .ok()
                                        .filter(|d| d.is_finite())
                                        .map_or(1, |d| d.round().clamp(1.0, 1e6) as i64);
                                    send = Some(p.action(
                                        "slicer_relative",
                                        json!({"range": p.range, "duration": d, "period": p.period}),
                                    ));
                                }
                                if ui.button(l.t("Clear", "Effacer")).clicked() {
                                    send = Some(p.action("slicer_clear", json!({})));
                                }
                            });
                        }
                        "text" => {
                            let r = ui.add(egui::TextEdit::singleline(&mut p.text).char_limit(1000));
                            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            ui.horizontal(|ui| {
                                if ui.button(l.t("Apply", "Appliquer")).clicked() || enter {
                                    send = Some(p.action("slicer_text", json!({"text": p.text})));
                                }
                                if ui.button(l.t("Clear", "Effacer")).clicked() {
                                    send = Some(p.action("slicer_text", json!({"text": ""})));
                                }
                            });
                        }
                        "decomp_expand" => {
                            send = crate::decomp_ui::popup(ui, &p.spec, fresh_now, l);
                            // Escape closes it, as a menu
                            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                close = true;
                            }
                        }
                        _ => close = true,
                    }
                });
            });
        // a click elsewhere closes it, as in the viewer
        let fresh = std::mem::replace(&mut p.fresh, false);
        if !fresh && area.response.clicked_elsewhere() && send.is_none() {
            let on_page = ctx.input(|i| i.pointer.any_pressed());
            if on_page {
                close = true;
            }
        }
        if let Some(a) = send {
            self.popup = None;
            self.interact(a);
        } else if close {
            self.popup = None;
        }
    }

    // --- model and validation ------------------------------------------------------------

    fn model(&mut self, ui: &mut egui::Ui) {
        self.model_view.menu_states = crate::menus::model_states(self);
        // the diagram's drags and the Properties pane edit through `diagram.move` and `*.set`
        self.model_view.editable = crate::ribbon::state(self, "diagram.move").is_ok();
        self.model_view.ui(ui, self.loaded.as_ref(), self.lang);
        // its Data and Properties panes (drawn in that order) are sections of their own (Ctrl+F6)
        for (i, r) in std::mem::take(&mut self.model_view.panes).into_iter().enumerate() {
            let pane = if i == 0 {
                crate::keyboard::Region::Data
            } else {
                crate::keyboard::Region::Properties
            };
            crate::keyboard::pane(self, ui, pane, r);
        }
        // a gesture's or a property's command, with all its values: sent as it is
        if let Some((id, p)) = self.model_view.run.take()
            && let Some(c) = crate::registry::find(id)
        {
            match crate::ribbon::state(self, id) {
                Ok(()) => {
                    self.error = None;
                    // a DirectQuery table switched to Import asks first, as in Desktop
                    if !crate::storage_ui::intercept(self, id, &p) {
                        crate::ribbon::send(self, c, p);
                    }
                }
                Err(why) => {
                    self.status = format!("{}: {why}", c.text(self.lang));
                    self.model_view.drop_moved();
                }
            }
        }
        // a failed move: the card goes back
        if self.error.is_some() {
            self.model_view.drop_moved();
        }
        if let Some((id, p)) = self.model_view.form.take() {
            // a double click on a relationship line opens the Edit relationship window
            if id == "relationship.set" {
                crate::relationships_dialog::edit(self, &p);
            } else {
                crate::form::open(self, id, p);
            }
        }
        // a model object's menu item: its command, with the object as parameters
        if let Some((id, picked)) = self.model_view.picked.take()
            && let Some(c) = crate::registry::find(id)
        {
            let mut p = crate::ribbon::prefill(self, c, None);
            // the object picked in the item's submenu (a hierarchy, a query, a role…)
            if let (Some(p), Some(o)) = (p.as_object_mut(), picked.as_object()) {
                p.extend(o.clone());
            }
            crate::ribbon::invoke(self, id, p);
        }
    }

    /// The Validation tab: a card for the model and one for the report, rows with their status
    /// icon (docs/design.md §9).
    fn validation(&mut self, ui: &mut egui::Ui) {
        let l = self.lang;
        let t = Tokens::of(self.prefs.theme);
        let Some(loaded) = &self.loaded else {
            empty(ui, l.t("Nothing open", "Rien d'ouvert"), "");
            return;
        };
        let v = loaded.snap.get("validation").cloned().unwrap_or(Value::Null);
        let vm = v.get("model").cloned().unwrap_or(Value::Null);
        let vr = v.get("report").cloned().unwrap_or(Value::Null);
        let n = |x: &Value, k: &str| x.get(k).and_then(Value::as_u64).unwrap_or(0);
        // the viewer's status pills: `✓ valid`, `✕ invalid`
        let status = |ui: &mut egui::Ui, text: String, color: Color32| {
            ui.label(RichText::new(text).font(theme::bold(13.0)).color(color));
        };
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::NONE
                    .inner_margin(egui::Margin::same(16))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        widgets::card(&t).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                RichText::new(l.t("Model checks", "Contrôles du modèle"))
                                    .font(theme::bold(16.0)),
                            );
                            ui.add_space(4.0);
                            if !vm.is_object() {
                                status(ui, l.t("Not validated.", "Non validé.").into(), t.text_dim);
                            } else if vm.get("valid").and_then(Value::as_bool) != Some(false) {
                                status(ui, l.t("✓ valid", "✓ valide").into(), t.ok);
                            } else {
                                status(
                                    ui,
                                    format!(
                                        "✕ {} {}",
                                        n(&vm, "invalid_count"),
                                        l.t("invalid objects", "objets non valides")
                                    ),
                                    t.err,
                                );
                            }
                            for o in vm
                                .get("objects")
                                .and_then(Value::as_array)
                                .into_iter()
                                .flatten()
                                .take(MAX_ROWS)
                            {
                                let s = |k: &str| o.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                                let what = [s("table"), s("name")]
                                    .iter()
                                    .filter(|x| !x.is_empty())
                                    .cloned()
                                    .collect::<Vec<_>>()
                                    .join(" / ");
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new(format!("{} {}", s("kind"), clip(&what, 160)))
                                        .font(theme::bold(13.0))
                                        .color(t.text),
                                );
                                for d in o
                                    .get("diagnostics")
                                    .and_then(Value::as_array)
                                    .into_iter()
                                    .flatten()
                                    .take(50)
                                {
                                    ui.label(
                                        RichText::new(format!("    {}", clip(&issue_text(d), 400)))
                                            .color(t.text_dim),
                                    );
                                }
                            }
                        });
                        ui.add_space(12.0);
                        widgets::card(&t).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                RichText::new(l.t("Report checks", "Contrôles du rapport"))
                                    .font(theme::bold(16.0)),
                            );
                            ui.add_space(4.0);
                            if !vr.is_object() {
                                status(ui, l.t("Not validated.", "Non validé.").into(), t.text_dim);
                            } else {
                                let (e, w) = (n(&vr, "error_count"), n(&vr, "warning_count"));
                                let (mark, color) = if e > 0 {
                                    ("✕", t.err)
                                } else if w > 0 {
                                    ("⚠", t.warn)
                                } else {
                                    ("✓", t.ok)
                                };
                                status(
                                    ui,
                                    format!(
                                        "{mark} {e} {}, {w} {}",
                                        l.t("errors", "erreurs"),
                                        l.t("warnings", "avertissements")
                                    ),
                                    color,
                                );
                            }
                            for i in vr
                                .get("issues")
                                .and_then(Value::as_array)
                                .into_iter()
                                .flatten()
                                .take(MAX_ROWS)
                            {
                                let sev = i.get("severity").and_then(Value::as_str).unwrap_or("");
                                let (mark, c) = match sev {
                                    "warning" | "warn" => ("⚠", t.warn),
                                    _ => ("✖", t.err),
                                };
                                ui.label(
                                    RichText::new(format!("{mark} {}", clip(&issue_text(i), 400))).color(c),
                                );
                            }
                        });
                    });
            });
    }
}

/// An issue or diagnostic as one line: severity, message, then what it is about.
pub(crate) fn issue_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Object(o) => {
            let s = |k: &str| o.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let mut parts = Vec::new();
            if !s("severity").is_empty() {
                parts.push(format!("[{}]", s("severity")));
            }
            parts.push(s("message"));
            for k in ["page", "visual", "table", "object", "code"] {
                if !s(k).is_empty() {
                    parts.push(format!("({k}: {})", s(k)));
                }
            }
            parts.join(" ")
        }
        other => other.to_string(),
    }
}

/// The viewer's empty state: a semibold title and a muted line, centred.
pub fn empty(ui: &mut egui::Ui, title: &str, sub: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() / 3.0).clamp(0.0, 240.0));
        ui.label(RichText::new(title).font(crate::fonts::semibold_px(16.0)));
        if !sub.is_empty() {
            ui.add_space(6.0);
            ui.label(RichText::new(sub).weak());
        }
    });
}

/// `msg` about the file at `path`, naming it once: prefixed with `path: ` unless the message
/// already contains it (the engine's open errors do).
pub fn with_path(path: &str, msg: &str) -> String {
    if path.is_empty() || msg.contains(path) {
        msg.to_string()
    } else {
        format!("{path}: {msg}")
    }
}

/// At most `n` characters, with an ellipsis.
pub fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(n.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

/// Where an image of `px` device pixels drawn for `rect` goes so that each of its pixels covers
/// exactly one device pixel: the top-left corner on the pixel grid nearest `rect`'s, and the
/// image's own size. Linear filtering then samples texel centres and the page stays sharp.
pub(crate) fn texel_rect(rect: Rect, px: [u32; 2], ppp: f32) -> Rect {
    let ppp = if ppp.is_finite() && ppp > 0.0 { ppp } else { 1.0 };
    if !rect.min.x.is_finite() || !rect.min.y.is_finite() || px[0] == 0 || px[1] == 0 {
        return rect;
    }
    let min = pos2((rect.min.x * ppp).round() / ppp, (rect.min.y * ppp).round() / ppp);
    Rect::from_min_size(min, Vec2::new(px[0] as f32 / ppp, px[1] as f32 / ppp))
}

#[cfg(test)]
mod texel_rect_tests {
    use super::*;

    #[test]
    fn a_page_lands_on_whole_device_pixels_at_its_own_size() {
        // a page fitted at a fractional position, rendered 1313 x 739 pixels, at 1.25 px/pt
        let rect = Rect::from_min_size(pos2(148.3, 216.7), Vec2::new(1050.1, 591.4));
        let r = texel_rect(rect, [1313, 739], 1.25);
        for v in [r.min.x, r.min.y, r.width(), r.height()] {
            let d = v * 1.25;
            assert!((d - d.round()).abs() < 1e-3, "{v} is not on the pixel grid");
        }
        assert!((r.width() * 1.25 - 1313.0).abs() < 1e-3);
        assert!((r.min.x - rect.min.x).abs() <= 0.5 / 1.25 + 1e-4);
        // nonsense in: the rect is kept
        assert_eq!(texel_rect(rect, [0, 10], 1.0), rect);
        assert_eq!(texel_rect(rect, [10, 10], f32::NAN).width(), 10.0);
    }
}
