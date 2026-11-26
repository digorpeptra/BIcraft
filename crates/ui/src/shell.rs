//! What a shell gives the app. The UI is one ([`crate::App`]); the native window
//! (`apps/bicraft-desktop`) and the browser page (`apps/bicraft-web`) only differ by the services
//! they can offer: a file dialog, recent files, a server connection, a settings file, the window
//! title. The app asks [`Caps`] instead of showing commands that would do nothing (the browser has
//! no file dialog, and its server is the page's own).

use std::path::PathBuf;

use serde_json::Value;

use crate::i18n::Lang;
use crate::source::{Link, Origin, Running, StartSource};

/// What the shell can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    /// File > Open (Ctrl+O), Recent files, files dropped on the window.
    pub open_files: bool,
    /// File > Connect to server, Recent servers.
    pub connect: bool,
    /// File > Quit.
    pub quit: bool,
    /// "Open in Power BI Desktop" (phase 2, docs/webui.md).
    pub open_in_desktop: bool,
    /// Download of the saved file (phase 2, docs/webui.md).
    pub download: bool,
    /// The host keeps some chords for itself (a browser's Ctrl+N): the app shows and takes
    /// [`crate::registry::BROWSER_CHORDS`] instead, the one keyboard difference of the shells.
    pub reserved_keys: bool,
}

impl Caps {
    /// The native window.
    pub const DESKTOP: Caps = Caps {
        open_files: true,
        connect: true,
        quit: true,
        open_in_desktop: false,
        download: false,
        reserved_keys: false,
    };
    /// The browser page: the server and the file are the page's.
    pub const BROWSER: Caps = Caps {
        open_files: false,
        connect: false,
        quit: false,
        open_in_desktop: false,
        download: false,
        reserved_keys: true,
    };
}

/// Where the preferences ([`crate::prefs::Prefs`]) are kept.
pub trait PrefsStore {
    /// The stored value (`Null` when there is none or it cannot be read).
    fn load(&self) -> Value;
    fn save(&self, v: &Value) -> Result<(), String>;
}

/// Sets the window's title (the browser: `document.title`).
pub type SetTitle = Box<dyn Fn(&egui::Context, &str)>;

/// A file dialog.
pub type PickFile = Box<dyn Fn(Lang) -> Option<PathBuf>>;

/// A Save dialog: the title, the suggested file name and the extensions offered (`["pbix"]`).
pub type PickSave = Box<dyn Fn(Lang, &str, &str, &[&str]) -> Option<PathBuf>>;

/// An image dialog (Insert › Image): the file's name and bytes, read by the shell (at most
/// `rich::MAX_IMAGE_BYTES`); `None` when cancelled, `Some(Err)` when the file cannot be read.
pub type PickImage = Box<dyn Fn(Lang) -> Option<Result<(String, Vec<u8>), String>>>;

/// What a file upload ([`Shell::upload`]) gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picked {
    /// The user closed the dialog.
    Cancelled,
    /// The file's name and bytes.
    File { name: String, bytes: Vec<u8> },
    /// Why the file could not be read (too large, unreadable).
    Failed(String),
}

/// Where a file upload puts what it gave, now or later.
pub type UploadSlot = std::rc::Rc<std::cell::RefCell<Option<Picked>>>;

/// A file upload (Visualizations pane › … › Import a visual from a file): the title, the
/// extensions offered (`["pbiviz"]`) and the largest file accepted; the shell reads the file and
/// puts it in the slot, now (the desktop's dialog) or once read (the browser's file input).
pub type Upload = Box<dyn Fn(Lang, &str, &[&str], usize, UploadSlot)>;

/// A data source dialog (Get data): the title, then the extensions of a file (`["csv"]`) or `None`
/// for a folder.
pub type PickData = Box<dyn Fn(Lang, &str, Option<&[&str]>) -> Option<PathBuf>>;

/// A folder dialog (New project).
pub type PickFolder = Box<dyn Fn(Lang) -> Option<PathBuf>>;

/// The clock: seconds since 1970 (Recent's "last opened").
pub type Now = Box<dyn Fn() -> u64>;

/// The recoverable journals left by earlier sessions (`recovery.list {all: true}`'s rows,
/// docs/recovery.md): the desktop reads its recovery folder; the browser asks its server.
pub type RecoveryList = Box<dyn Fn() -> Vec<Value>>;

/// Reads the system clipboard into a slot ([`Shell::read_clipboard`]).
pub type ReadClipboard = Box<dyn Fn(crate::canvas::ClipSlot)>;
/// Hands a file of the server workspace to the user as a download (the browser: its file route,
/// `GET /w/<id>/files/<path>`): the exports (PDF, data, PNG) the commands wrote there.
pub type Download = Box<dyn Fn(&str)>;

/// Most documents open at once in one window (each holds a source: an engine session on the
/// desktop, with its model and data). Opening one more is refused with a message.
pub const MAX_DOCS: usize = 8;

/// The services of a shell.
pub struct Shell {
    pub caps: Caps,
    /// Starts the data source of an origin.
    pub start_source: StartSource,
    /// The Open dialog (with [`Caps::open_files`]).
    pub pick_file: Option<PickFile>,
    /// The Save dialog (Save As, Export); none: the Backstage asks for a workspace-relative name.
    pub pick_save: Option<PickSave>,
    /// The folder dialog (New); none: new projects go to the workspace's root.
    pub pick_folder: Option<PickFolder>,
    /// The image dialog (Insert › Image); none: images come from the workspace's files.
    pub pick_image: Option<PickImage>,
    /// A file upload (a custom visual's `.pbiviz`): the desktop's dialog, the browser's file
    /// input; none: the command takes a typed path.
    pub upload: Option<Upload>,
    /// The data file dialog (Get data): a file of the extensions given, or a folder for `None`;
    /// none: Get data takes a typed path or the workspace's files.
    pub pick_data: Option<PickData>,
    /// The clock; none: dates are unknown (0).
    pub now: Option<Now>,
    /// The server workspace the page belongs to (the browser on `bicraft serve`): Open > Browse
    /// lists its files and Recent keeps workspace-relative names.
    pub workspace: Option<String>,
    /// Most documents open at once ([`MAX_DOCS`], the same in every shell).
    pub max_docs: usize,
    /// A workspace file opens in a document tab of its own ([`crate::source::Origin::Workspace`]),
    /// whose source is a private view on the server with a session of its own on that file (the
    /// browser on `bicraft serve`, docs/webui.md, "Document tabs"): the tabs work side by side,
    /// as the desktop's. Off: a workspace file opens in the server tab's own session (the desktop
    /// connected to a server).
    pub ws_tabs: bool,
    /// The preferences' store; none: kept in memory only.
    pub prefs: Option<Box<dyn PrefsStore>>,
    /// Sets the title; none: egui's viewport title.
    pub set_title: Option<SetTitle>,
    /// The token Connect to server starts with (the desktop: `BICRAFT_SERVER_TOKEN`).
    pub server_token: String,
    /// Lists the recoverable journals without an open document (the desktop's start screen);
    /// none: the File view asks the active document's session.
    pub recovery_list: Option<RecoveryList>,
    /// Reads the system clipboard's text for the Paste button and menu item: puts the text
    /// (or why it could not) in the slot, now or later (the browser's async clipboard API).
    /// None: Paste uses the in-app clipboard ([`crate::canvas`]); Ctrl+V always reads the
    /// shell's paste event.
    pub read_clipboard: Option<ReadClipboard>,
    /// Downloads a workspace file once an export wrote it; none: the file stays where it was
    /// written (the desktop writes where the Save dialog said).
    pub download: Option<Download>,
    /// Prints a workspace PDF once `report.print` wrote it (the browser: the file fetched and
    /// printed by the browser's own print); none: the engine hands the PDF to the operating
    /// system's PDF application (the desktop).
    pub print: Option<Download>,
    /// The operating system asks for high contrast (the browser: `forced-colors: active` or
    /// `prefers-contrast: more`): a window without a theme preference starts in the
    /// high-contrast theme with the report pages in high contrast. The desktop leaves it off
    /// (eframe exposes only light and dark; View › High contrast instead).
    pub system_high_contrast: bool,
}

impl Shell {
    /// A shell with `caps` whose sources start with `start`, and nothing else.
    pub fn new(caps: Caps, start: impl Fn(Origin, Link) -> Running + 'static) -> Shell {
        Shell {
            caps,
            start_source: Box::new(start),
            pick_file: None,
            pick_save: None,
            pick_folder: None,
            pick_image: None,
            upload: None,
            pick_data: None,
            now: None,
            workspace: None,
            max_docs: MAX_DOCS,
            ws_tabs: false,
            prefs: None,
            set_title: None,
            server_token: String::new(),
            recovery_list: None,
            read_clipboard: None,
            download: None,
            print: None,
            system_high_contrast: false,
        }
    }
}
