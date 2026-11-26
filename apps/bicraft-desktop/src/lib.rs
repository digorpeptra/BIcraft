//! The BIcraft desktop app (`bicraft-desktop`): the native shell of the BIcraft UI
//! ([`bicraft_ui::App`], docs/webui.md). No web engine: the window is egui/eframe over wgpu.
//! This crate supplies what only a native program has (docs/desktop.md):
//!
//! - the data sources on worker threads: a local file or project in an engine session
//!   ([`local`], re-read when it changes on disk) and the blocking client of a BIcraft server
//!   ([`remote`]);
//! - the Open dialog, the settings file ([`prefs`]), `BICRAFT_SERVER_TOKEN`, the OS language;
//! - Windows integration (`--register`: [`register`], [`lnk`]) and the icon.
//!
//! Everything drawn is the shared UI's, so the desktop and the browser look and behave the same.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use std::path::PathBuf;

use bicraft_engine::RecoveryConfig;
use bicraft_ui::i18n::Lang;
use bicraft_ui::shell::{Caps, PrefsStore, Shell};
use bicraft_ui::source::{self, Origin};

pub mod control;
pub mod lnk;
pub mod local;
pub mod prefs;
pub mod register;
pub mod remote;

pub use bicraft_ui::App;

/// The app icon (256 px PNG), drawn from `assets/logo/bicraft-icon.svg` by `build.rs`.
pub const ICON_PNG: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon-256.png"));

/// File extensions the app opens (also the Open-with registrations of `--register`).
pub const EXTENSIONS: [&str; 4] = ["pbix", "pbit", "pbip", "bim"];

/// Excel workbooks the Open dialog also offers: `file.open` imports their Power Pivot model
/// (not registered for Open with: Excel owns them).
pub const WORKBOOK_EXTENSIONS: [&str; 2] = ["xlsx", "xlsm"];
/// Other files the Open dialog offers: `file.open` reads them into a new model (M11.36).
pub const OTHER_EXTENSIONS: [&str; 6] = ["vpax", "pbids", "pq", "m", "pqt", "json"];

/// The native shell: worker-thread sources, the Open dialog, the settings file at `prefs`
/// (none: kept in memory), the token of `BICRAFT_SERVER_TOKEN` for Connect to server, and the
/// per-user recovery folder (docs/recovery.md) when there is a settings file.
pub fn shell(prefs: Option<PathBuf>) -> Shell {
    let recovery = prefs.as_ref().and_then(|_| RecoveryConfig::default_dir());
    shell_with(prefs, recovery)
}

/// [`shell`] with an explicit recovery folder (`None`: unsaved edits are not journaled).
pub fn shell_with(prefs: Option<PathBuf>, recovery: Option<PathBuf>) -> Shell {
    let journal = recovery.map(RecoveryConfig::new);
    let for_sources = journal.clone();
    let mut s = Shell::new(Caps::DESKTOP, move |origin, link| {
        let journal = for_sources.clone();
        source::spawn(link, move |link| match origin {
            Origin::Local { path, lang } => local::run(&path, &lang, journal, link),
            Origin::New {
                dir,
                name,
                template,
                lang,
            } => local::run_new(&dir, &name, template.as_deref(), &lang, journal, link),
            Origin::Remote(server) => remote::run(&server, link),
            // the browser's workspace tabs; the desktop opens workspace files in its server tab
            Origin::Workspace { file, .. } => link.send(bicraft_ui::source::Update::Error(format!(
                "{file}: open it from the server's tab (Open > Browse)"
            ))),
        })
    });
    s.pick_file = Some(Box::new(pick_file));
    s.pick_save = Some(Box::new(pick_save));
    s.pick_folder = Some(Box::new(pick_folder));
    s.pick_image = Some(Box::new(pick_image));
    s.upload = Some(Box::new(upload));
    s.pick_data = Some(Box::new(pick_data));
    s.now = Some(Box::new(now));
    s.prefs = prefs.map(|p| Box::new(prefs::FileStore(p)) as Box<dyn PrefsStore>);
    s.server_token = bicraft_env::var("BICRAFT_SERVER_TOKEN").unwrap_or_default();
    // the Paste button: the OS clipboard's text (Ctrl+V reads the window's paste event)
    s.read_clipboard = Some(Box::new(|slot: bicraft_ui::canvas::ClipSlot| {
        let text = arboard::Clipboard::new()
            .and_then(|mut c| c.get_text())
            .map_err(|e| e.to_string());
        *slot.borrow_mut() = Some(text);
    }));
    // the start screen lists what earlier sessions left unsaved
    s.recovery_list = journal.map(|cfg| {
        Box::new(move || bicraft_engine::recovery_offers(&cfg)) as bicraft_ui::shell::RecoveryList
    });
    s
}

/// The app in the native shell, in language `lang_tag` (also the session's UI language).
pub fn app(lang_tag: &str, prefs: Option<PathBuf>) -> App {
    App::new(lang_tag, shell(prefs))
}

/// [`app`] with an explicit recovery folder.
pub fn app_with(lang_tag: &str, prefs: Option<PathBuf>, recovery: Option<PathBuf>) -> App {
    App::new(lang_tag, shell_with(prefs, recovery))
}

/// The language tag of the operating system's user interface (`fr-FR`), when it says.
pub fn os_tag() -> Option<String> {
    sys_locale::get_locale().filter(|t| !t.is_empty() && t.len() <= 40)
}

/// The native Open dialog.
pub fn pick_file(lang: Lang) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title(lang.t("Open a Power BI file", "Ouvrir un fichier Power BI"))
        .add_filter(
            lang.t(
                "Power BI files and Excel workbooks",
                "Fichiers Power BI et classeurs Excel",
            ),
            &[EXTENSIONS.as_slice(), WORKBOOK_EXTENSIONS.as_slice()].concat(),
        )
        .add_filter(lang.t("Power BI files", "Fichiers Power BI"), &EXTENSIONS)
        .add_filter("PBIX", &["pbix"])
        .add_filter("PBIT", &["pbit"])
        .add_filter("PBIP", &["pbip"])
        .add_filter("BIM", &["bim"])
        .add_filter(
            lang.t(
                "Excel workbooks with a data model",
                "Classeurs Excel avec un modèle de données",
            ),
            &WORKBOOK_EXTENSIONS,
        )
        .add_filter(
            lang.t(
                "Other sources (VPAX, PBIDS, Power Query, Dataflow, CDM)",
                "Autres sources (VPAX, PBIDS, Power Query, Dataflow, CDM)",
            ),
            &OTHER_EXTENSIONS,
        )
        .pick_file()
}

/// The native Save dialog: `title`, the suggested `name`, the extensions offered.
pub fn pick_save(_lang: Lang, title: &str, name: &str, exts: &[&str]) -> Option<PathBuf> {
    let mut d = rfd::FileDialog::new().set_title(title).set_file_name(name);
    for e in exts {
        d = d.add_filter(e.to_uppercase(), &[*e]);
    }
    d.save_file()
}

/// The native dialog of Get data: a file of `exts`, or a folder for `None`.
pub fn pick_data(_lang: Lang, title: &str, exts: Option<&[&str]>) -> Option<PathBuf> {
    let d = rfd::FileDialog::new().set_title(title);
    match exts {
        Some(exts) => d.add_filter(exts.join(", "), exts).pick_file(),
        None => d.pick_folder(),
    }
}

/// The native folder dialog (New project).
pub fn pick_folder(lang: Lang) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title(lang.t("Folder of the new project", "Dossier du nouveau projet"))
        .pick_folder()
}

/// The native image dialog (Insert › Image): the file's name and bytes, at most
/// [`bicraft_ui::rich::MAX_IMAGE_BYTES`] (checked before reading). `None`: cancelled.
pub fn pick_image(lang: Lang) -> Option<Result<(String, Vec<u8>), String>> {
    let path = rfd::FileDialog::new()
        .set_title(lang.t("Insert an image", "Insérer une image"))
        .add_filter(
            lang.t("Images", "Images"),
            &["png", "jpg", "jpeg", "gif", "svg", "webp"],
        )
        .pick_file()?;
    Some(read_image(&path))
}

/// The native dialog of a file upload (Import a visual from a file): `title`, files of `exts`,
/// at most `max` bytes (checked before reading); the result is in the slot when it returns.
pub fn upload(_lang: Lang, title: &str, exts: &[&str], max: usize, slot: bicraft_ui::shell::UploadSlot) {
    let picked = match rfd::FileDialog::new()
        .set_title(title)
        .add_filter(exts.join(", "), exts)
        .pick_file()
    {
        None => bicraft_ui::shell::Picked::Cancelled,
        Some(path) => read_upload(&path, max),
    };
    *slot.borrow_mut() = Some(picked);
}

/// A file's name and bytes for an upload, refused past `max` bytes before it is read.
pub fn read_upload(path: &std::path::Path, max: usize) -> bicraft_ui::shell::Picked {
    use bicraft_ui::shell::Picked;
    let len = match std::fs::metadata(path) {
        Ok(m) => m.len(),
        Err(e) => return Picked::Failed(format!("{}: {e}", path.display())),
    };
    if len > max as u64 {
        return Picked::Failed(format!("{}: {len} bytes; the limit is {max}", path.display()));
    }
    match std::fs::read(path) {
        Ok(bytes) => Picked::File {
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            bytes,
        },
        Err(e) => Picked::Failed(format!("{}: {e}", path.display())),
    }
}

/// An image file's name and bytes, refused past the size cap before it is read.
pub fn read_image(path: &std::path::Path) -> Result<(String, Vec<u8>), String> {
    let len = std::fs::metadata(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .len();
    let cap = bicraft_ui::rich::MAX_IMAGE_BYTES;
    if len > cap as u64 {
        return Err(format!("{}: {len} bytes; the limit is {cap}", path.display()));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    Ok((name, bytes))
}

/// Seconds since 1970.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests;
