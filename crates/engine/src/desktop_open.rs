//! Opening the session's saved file with the operating system's default application (Power BI
//! Desktop on a machine that has it): the `file.open_in_desktop` command and the live viewer's
//! "Open in Power BI Desktop" button (`docs/view-snapshot.md`, "Open in Power BI Desktop").
//!
//! A development convenience run only on an explicit request. bicraft never needs Desktop: the
//! file is handed to the OS (`explorer.exe` on Windows, `open` on macOS, `xdg-open` elsewhere),
//! which starts whatever application is registered for `.pbix` / `.pbit` / `.pbip` files.
//!
//! Only the session's own file is ever opened, and only when it is a `.pbix`, `.pbit` or `.pbip`
//! file that exists: no path comes from the caller, and nothing else (no program, no URL) can be
//! launched.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::params::{allow, opt_bool};
use crate::{Session, Source};

/// The only file types handed to the OS.
pub const DESKTOP_EXTENSIONS: &[&str] = &["pbix", "pbit", "pbip"];
/// Most directory entries looked at when searching a project folder for its `.pbip` file.
const MAX_PROJECT_ENTRIES: usize = 4096;
/// How long `open` / `xdg-open` may take to report that no application handles the file.
const HANDLER_WAIT: Duration = Duration::from_secs(5);

/// Opens a file with the default application. Injected in tests (and by front ends that want to
/// log or confirm) so nothing is launched.
pub type Opener = Arc<dyn Fn(&Path) -> Result<(), String> + Send + Sync>;

/// [`Session::external_opener`]: an [`Opener`] with a `Debug` impl.
#[derive(Clone)]
pub struct ExternalOpener(pub Opener);

impl fmt::Debug for ExternalOpener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ExternalOpener(..)")
    }
}

/// What "open in Power BI Desktop" would open for a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopTarget {
    /// The absolute path of the saved `.pbix` / `.pbit` file, or of the project's `.pbip` file.
    /// `None` when there is nothing Desktop can open (see `reason`).
    pub path: Option<PathBuf>,
    /// Why `path` is `None`.
    pub reason: Option<String>,
    /// The session has model or report edits that are not in that file yet.
    pub unsaved: bool,
}

impl DesktopTarget {
    fn none(reason: impl Into<String>, unsaved: bool) -> Self {
        Self {
            path: None,
            reason: Some(reason.into()),
            unsaved,
        }
    }

    /// The snapshot's `source.desktop` object: `{path, reason, unsaved}`.
    pub fn to_json(&self) -> Value {
        json!({
            "path": self.path.as_ref().map(|p| p.display().to_string()),
            "reason": self.reason,
            "unsaved": self.unsaved,
        })
    }
}

impl Session {
    /// The file "open in Power BI Desktop" opens: the saved PBIX/PBIT file, or the `.pbip` file
    /// of a project (also when the session opened its folder or an item folder). Edits that are
    /// not saved are not in it (`unsaved`).
    pub fn desktop_target(&self) -> DesktopTarget {
        let unsaved = self.dirty || self.report_dirty;
        if self.server_config().is_some() {
            // no desktop on a server: the viewer's open route and file.open_in_desktop launch nothing
            return DesktopTarget::none(
                "server mode: opening in Power BI Desktop is not available (download the saved file instead)",
                unsaved,
            );
        }
        let Some(source) = &self.source else {
            return DesktopTarget::none(
                "nothing is saved yet: save the session as a .pbix or .pbip file first (file.save_as)",
                unsaved,
            );
        };
        let found = match source {
            Source::Package { path, .. } => Ok(path.clone()),
            Source::Project { path, project } => pbip_file(path, project),
            Source::Bim { .. } => Err(
                "Power BI Desktop does not open a standalone model.bim: save it as a .pbix or .pbip file first (file.save_as)"
                    .to_string(),
            ),
        };
        match found.and_then(|p| openable(&p)) {
            Ok(path) => DesktopTarget {
                path: Some(path),
                reason: None,
                unsaved,
            },
            Err(reason) => DesktopTarget::none(reason, unsaved),
        }
    }
}

/// The `.pbip` file of a project opened as `path` (the `.pbip` file itself, the project folder or
/// an item folder): the one named like its items, else the first one in the folder.
fn pbip_file(path: &Path, project: &bicraft_pbip::Project) -> Result<PathBuf, String> {
    if has_desktop_extension(path, "pbip") && path.is_file() {
        return Ok(path.to_path_buf());
    }
    let stem = [&project.semantic_model, &project.report]
        .into_iter()
        .flatten()
        .find_map(|item| {
            let name = item.file_name()?.to_str()?;
            [".SemanticModel", ".Report"].iter().find_map(|suffix| {
                let cut = name.len().checked_sub(suffix.len())?;
                let (stem, end) = (name.get(..cut)?, name.get(cut..)?);
                (!stem.is_empty() && end.eq_ignore_ascii_case(suffix)).then(|| stem.to_string())
            })
        });
    if let Some(stem) = &stem {
        let named = project.root.join(format!("{stem}.pbip"));
        if named.is_file() {
            return Ok(named);
        }
    }
    let entries = std::fs::read_dir(&project.root)
        .map_err(|e| format!("cannot list the project folder {}: {e}", project.root.display()))?;
    let mut found: Vec<PathBuf> = entries
        .take(MAX_PROJECT_ENTRIES)
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| has_desktop_extension(p, "pbip") && p.is_file())
        .collect();
    found.sort();
    found.into_iter().next().ok_or_else(|| {
        format!(
            "the project folder {} has no .pbip file for Power BI Desktop to open",
            project.root.display()
        )
    })
}

fn has_desktop_extension(path: &Path, ext: &str) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// `path` made absolute, when it is an existing `.pbix`, `.pbit` or `.pbip` file.
pub fn openable(path: &Path) -> Result<PathBuf, String> {
    if !DESKTOP_EXTENSIONS
        .iter()
        .any(|ext| has_desktop_extension(path, ext))
    {
        return Err(format!(
            "{}: only .pbix, .pbit and .pbip files are opened in Power BI Desktop",
            path.display()
        ));
    }
    if !path.is_file() {
        return Err(format!(
            "{}: the file does not exist (any more); save the session first",
            path.display()
        ));
    }
    std::path::absolute(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Opens `path` (an existing `.pbix`, `.pbit` or `.pbip` file; anything else is refused) with the
/// operating system's default application, without a shell: `explorer.exe` on Windows, `open` on
/// macOS, `xdg-open` elsewhere. Returns once the handler is started; on macOS and Linux, an error
/// when the handler reports within a few seconds that it could not open the file.
pub fn open_with_default_app(path: &Path) -> Result<(), String> {
    launch(&openable(path)?)
}

/// Opens `path`, an existing `.pdf` file (anything else is refused), with the operating
/// system's default application, as [`open_with_default_app`] does: `report.print`'s PDF, for
/// the PDF viewer's print.
pub fn open_pdf_with_default_app(path: &Path) -> Result<(), String> {
    if !has_desktop_extension(path, "pdf") {
        return Err(format!(
            "{}: only a .pdf file is opened for printing",
            path.display()
        ));
    }
    if !path.is_file() {
        return Err(format!("{}: the file does not exist", path.display()));
    }
    let path = std::path::absolute(path).map_err(|e| format!("{}: {e}", path.display()))?;
    launch(&path)
}

/// Hands `path` (absolute, checked by the caller) to the OS's file opener.
fn launch(path: &Path) -> Result<(), String> {
    let path = path.to_path_buf();
    let (mut cmd, handler) = handler_command(&path);
    let spawned = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!(
                "no `{handler}` on this system to open {} (install a desktop environment's file opener, or open the file yourself)",
                path.display()
            ));
        }
        Err(e) => return Err(format!("cannot start `{handler}`: {e}")),
    };
    if cfg!(windows) {
        // explorer.exe hands the file to the registered application and exits with an
        // unrelated status; reap it in the background.
        reap(child);
        return Ok(());
    }
    let start = Instant::now();
    while start.elapsed() < HANDLER_WAIT {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                return Err(format!(
                    "`{handler}` could not open {} ({status}): is an application registered for .{} files?",
                    path.display(),
                    path.extension().and_then(|e| e.to_str()).unwrap_or_default()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(format!("`{handler}`: {e}")),
        }
    }
    // Still running (some handlers stay in the foreground): treat as started.
    reap(child);
    Ok(())
}

fn reap(mut child: std::process::Child) {
    let _ = std::thread::Builder::new()
        .name("open-in-desktop".into())
        .spawn(move || child.wait());
}

#[cfg(windows)]
fn handler_command(path: &Path) -> (Command, &'static str) {
    use std::os::windows::process::CommandExt as _;
    // The full path, so no `explorer.exe` planted in the working directory or on PATH runs.
    let explorer = std::env::var_os("SystemRoot")
        .map(|root| PathBuf::from(root).join("explorer.exe"))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("explorer.exe"));
    let mut cmd = Command::new(explorer);
    // Always quoted (Windows paths cannot contain `"`): explorer.exe splits an unquoted argument
    // at commas.
    let mut quoted = std::ffi::OsString::from("\"");
    quoted.push(path.as_os_str());
    quoted.push("\"");
    cmd.raw_arg(quoted);
    (cmd, "explorer.exe")
}

#[cfg(not(windows))]
fn handler_command(path: &Path) -> (Command, &'static str) {
    let handler = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let mut cmd = Command::new(handler);
    // `path` is absolute, so it cannot be taken for an option.
    cmd.arg(path);
    (cmd, handler)
}

/// `file.open_in_desktop {dry_run?}`.
pub(crate) fn file_open_in_desktop(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["dry_run"])?;
    let dry_run = opt_bool(p, "dry_run")?.unwrap_or(false);
    let target = s.desktop_target();
    let Some(path) = &target.path else {
        return Err(target
            .reason
            .unwrap_or_else(|| "nothing to open in Power BI Desktop".into()));
    };
    if !dry_run {
        match &s.external_opener {
            Some(ExternalOpener(open)) => open(path)?,
            None => open_with_default_app(path)?,
        }
    }
    let mut out = json!({
        "path": path.display().to_string(),
        "opened": !dry_run,
        "unsaved": target.unsaved,
    });
    if target.unsaved {
        out["warning"] = json!(
            "the session has unsaved edits; the file opened in Power BI Desktop does not contain them (file.save first)"
        );
    }
    Ok(out)
}

#[cfg(test)]
#[path = "desktop_open_tests.rs"]
mod tests;
