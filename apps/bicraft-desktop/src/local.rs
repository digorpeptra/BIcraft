//! A local file or project: an engine session on the worker thread, as `bicraft-cli view` runs it
//! (`apps/bicraft-cli/src/view_cmd.rs`): the snapshot is published again while visuals wait for
//! their data, the files are watched ([`bicraft_engine::watch`]) and re-read when they change,
//! clicks run as `view.interact` on the session (reader state only), and the window's buttons run
//! engine commands on it ([`Request::Command`]: pages, Save, Save As, Export). A command that
//! moves the session to another file (Save As, a new project) moves the watcher with it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::RecvTimeoutError;

use bicraft_engine::watch::{POLL, Watcher, data_pending};
use bicraft_engine::{RecoveryConfig, Session, SnapshotLimits, Source, UiLanguage};
use serde_json::json;

use bicraft_ui::app::with_path;
use bicraft_ui::source::{Link, Loaded, Request, Update};

/// The session's snapshot and images; returns whether visuals still wait for their data.
pub fn publish(session: &Session, seq: &mut u64, link: &Link) -> bool {
    let pending = send(session, session.snapshot(), seq, link);
    // once published: the aggregation jobs the snapshot's visuals asked for
    session.background_work();
    pending
}

/// A file just opened: its pages without their visuals' data first (a snapshot that runs no
/// query; results already cached are kept), so the window draws the report while the next
/// snapshot runs the queries ([`publish`]), instead of a blank page until they are all done.
pub fn publish_opened(session: &Session, seq: &mut u64, link: &Link) -> bool {
    let layout = SnapshotLimits {
        data_budget_ms: 0,
        ..SnapshotLimits::default()
    };
    if send(session, session.snapshot_with(&layout), seq, link) {
        publish(session, seq, link)
    } else {
        false
    }
}

/// Sends `snap` with the session's images; returns whether visuals still wait for their data.
fn send(session: &Session, snap: serde_json::Value, seq: &mut u64, link: &Link) -> bool {
    let assets = session
        .snapshot_assets()
        .into_iter()
        .map(|a| (a.id, a.bytes))
        .collect();
    let pending = data_pending(&snap);
    *seq = seq.saturating_add(1);
    link.send(Update::Snapshot(Loaded {
        snap: Arc::new(snap),
        assets: Arc::new(assets),
        seq: *seq,
    }));
    pending
}

/// Opens `path` in `session`; errors go to the window (the file may be half-written: the next
/// change re-reads it). A file without a report (a `.bim`) is no error: its snapshot has no
/// `report` and the Report tab says so; a report that cannot be read is one.
fn open(session: &mut Session, path: &Path, link: &Link) {
    link.send(Update::Status(format!("{}…", path.display())));
    let shown = path.display().to_string();
    match session.execute("file.open", &json!({"path": path})) {
        Ok(_) => {
            if let Some(why) = &session.report_error {
                link.send(Update::Error(with_path(&shown, why)));
            }
        }
        Err(e) => link.send(Update::Error(with_path(&shown, &e))),
    }
}

/// The file the session has open.
fn session_path(session: &Session) -> Option<PathBuf> {
    session.source.as_ref().map(|s| match s {
        Source::Package { path, .. } | Source::Project { path, .. } | Source::Bim { path } => path.clone(),
    })
}

/// Creates the project `<dir>/<name>.pbip` in `session` and returns the path it has open: blank
/// (`project.new`), or converted from a template report (`file.convert`, then `file.open`).
pub fn create(
    session: &mut Session,
    dir: &Path,
    name: &str,
    template: Option<&Path>,
) -> Result<PathBuf, String> {
    let target = dir.join(format!("{name}.pbip"));
    match template {
        None => {
            session.execute("project.new", &json!({"dir": dir, "name": name}))?;
        }
        Some(t) => {
            session.execute(
                "file.convert",
                &json!({"from": t, "to": target, "format": "pbip-tmdl"}),
            )?;
            session.execute("file.open", &json!({"path": target}))?;
        }
    }
    Ok(session_path(session).unwrap_or(target))
}

/// Runs a local file or project.
pub fn run(path: &Path, lang: &str, journal: Option<RecoveryConfig>, link: &Link) {
    serve(path.to_path_buf(), None, lang, journal, link);
}

/// Runs a new project `<dir>/<name>.pbip` (blank, or from `template`).
pub fn run_new(
    dir: &Path,
    name: &str,
    template: Option<&Path>,
    lang: &str,
    journal: Option<RecoveryConfig>,
    link: &Link,
) {
    serve(
        dir.join(format!("{name}.pbip")),
        Some((
            dir.to_path_buf(),
            name.to_string(),
            template.map(Path::to_path_buf),
        )),
        lang,
        journal,
        link,
    );
}

/// A project to create: folder, name, template.
type NewProject = (PathBuf, String, Option<PathBuf>);

fn serve(
    mut path: PathBuf,
    new: Option<NewProject>,
    lang: &str,
    journal: Option<RecoveryConfig>,
    link: &Link,
) {
    let mut session = Session::new();
    session.ui_language = UiLanguage::parse(lang).unwrap_or_default();
    // unsaved edits are journaled in the per-user recovery folder (docs/recovery.md)
    session.set_recovery(journal);
    // the window's Cancel stops a running DAX query before the queue reaches `dax.cancel`
    session.set_cancel(Some(Arc::clone(&link.cancel)));
    match &new {
        Some((dir, name, template)) => {
            link.send(Update::Status(format!("{}…", path.display())));
            match create(&mut session, dir, name, template.as_deref()) {
                Ok(p) => path = p,
                Err(e) => link.send(Update::Error(with_path(&path.display().to_string(), &e))),
            }
        }
        None => {
            if !path.exists() {
                link.send(Update::Error(format!(
                    "{}: no such file or folder",
                    path.display()
                )));
            }
            open(&mut session, &path, link);
        }
    }
    session.revision = 0;
    let mut seq = 0u64;
    let mut left = publish_opened(&session, &mut seq, link);
    link.send(Update::Status(String::new()));
    let mut watcher = Watcher::new(&path, &session);
    loop {
        match link.requests.recv_timeout(POLL) {
            Ok(Request::Interact(action)) => {
                let r = session.execute("view.interact", &action);
                link.cancel.store(false, std::sync::atomic::Ordering::SeqCst);
                if r.is_ok() {
                    left = publish(&session, &mut seq, link);
                }
                link.send(Update::Interacted(r));
            }
            Ok(Request::Command { tag, id, params }) => {
                let before = session.revision;
                let result = session.execute(&id, &params);
                // a Cancel was for this command (or came after it): the next ones run in full
                link.cancel.store(false, std::sync::atomic::Ordering::SeqCst);
                // the session may have moved to another file (Save As, a new project, Open)
                if let Some(p) = session_path(&session) {
                    path = p;
                }
                if result.is_ok() && (id.starts_with("file.") || id.starts_with("project.")) {
                    watcher = Watcher::new(&path, &session);
                }
                // read-only commands change nothing: no new snapshot, no new render
                if session.revision != before {
                    left = publish(&session, &mut seq, link);
                }
                link.send(Update::Ran { tag, result });
            }
            Ok(Request::ListFiles) => link.send(Update::Files(Err(
                "a local file has no server workspace: use Open".into(),
            ))),
            Ok(Request::Reload) => {
                open(&mut session, &path, link);
                left = publish_opened(&session, &mut seq, link);
                watcher.reloaded(&path, &session);
                link.send(Update::Status(String::new()));
            }
            Ok(Request::Changed) => {}
            Ok(Request::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                if watcher.poll() {
                    open(&mut session, &path, link);
                    left = publish_opened(&session, &mut seq, link);
                    watcher.reloaded(&path, &session);
                    link.send(Update::Status(String::new()));
                } else if left {
                    left = publish(&session, &mut seq, link);
                }
            }
        }
    }
}
