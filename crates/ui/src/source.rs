//! Where the snapshots come from. The shell starts the source of an [`Origin`]
//! ([`crate::shell::Shell::start_source`]): natively a worker thread (a local file in an engine
//! session, `apps/bicraft-desktop/src/local.rs`, or the blocking client of a BIcraft server,
//! `apps/bicraft-desktop/src/remote.rs`); in the browser a *driven* source, answered as futures on
//! the page's event loop (`apps/bicraft-web/src/web.rs`). Every source speaks the same messages,
//! so the window never blocks on a file read, an evaluation or the network, and everything drawn
//! is shared by both shells.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
#[cfg(not(target_arch = "wasm32"))]
use std::thread::JoinHandle;

use serde_json::Value;

/// Snapshot images by id (`docs/view-snapshot.md`, "Images").
pub type Assets = HashMap<String, Arc<[u8]>>;

/// One snapshot of the open report, as the viewer receives it.
#[derive(Clone)]
pub struct Loaded {
    pub snap: Arc<Value>,
    pub assets: Arc<Assets>,
    /// Increases with every snapshot of this source (render cache key).
    pub seq: u64,
}

/// A remote server: base URL (`https://bi.example.com`), workspace id and bearer token (an API
/// token or an OIDC access token; empty on a server without authentication).
#[derive(Clone, PartialEq, Eq)]
pub struct Server {
    pub url: String,
    pub workspace: String,
    pub token: String,
}

impl std::fmt::Debug for Server {
    // never prints the token
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server")
            .field("url", &self.url)
            .field("workspace", &self.workspace)
            .field("token", &if self.token.is_empty() { "" } else { "<set>" })
            .finish()
    }
}

/// What to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// A file or project; `lang` is the session's UI language (`--lang`).
    Local {
        path: PathBuf,
        lang: String,
    },
    /// A new PBIP project `<dir>/<name>.pbip`, created in a fresh session and then open like a
    /// [`Origin::Local`] one: blank (`project.new`), or from a template report (`file.convert` of a
    /// `.pbit` into the project).
    New {
        dir: PathBuf,
        name: String,
        template: Option<PathBuf>,
        lang: String,
    },
    Remote(Server),
    /// A file of the page's server workspace (the browser's document tabs): a document view of
    /// its own on the server (`POST …/api/session {file}`), with its own session, snapshot,
    /// event stream and commands.
    Workspace {
        ws: String,
        file: String,
    },
}

impl Origin {
    /// The name shown in the title bar.
    pub fn label(&self) -> String {
        match self {
            Origin::Local { path, .. } => path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string()),
            Origin::New { name, .. } => format!("{name}.pbip"),
            Origin::Remote(s) => format!("{} · {}", s.workspace, host_of(&s.url)),
            Origin::Workspace { file, .. } => file.rsplit('/').next().unwrap_or(file).to_string(),
        }
    }
}

/// The host part of a URL, for labels.
pub fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split('/').next().unwrap_or(rest).to_string()
}

/// The command that stops the running DAX queries (raises [`Link::cancel`] as it is sent).
pub const CANCEL: &str = "dax.cancel";

/// From the window to the worker.
#[derive(Debug)]
pub enum Request {
    /// A `view.interact` action (`docs/view-snapshot.md`, "Interactions").
    Interact(Value),
    /// Run an engine command (`crates/engine/src/commands.rs`) on the source's session; the
    /// answer is [`Update::Ran`] with the same `tag`, then the next snapshot.
    Command {
        tag: u64,
        id: String,
        params: Value,
    },
    /// List the files of the source's server workspace (`GET /w/<id>/files/`): [`Update::Files`].
    /// Local files have no workspace (the shell's Open dialog browses the disk).
    ListFiles,
    /// Read the source again.
    Reload,
    /// The remote change feed saw a change (internal).
    Changed,
    Stop,
}

/// From the worker to the window.
#[derive(Debug)]
pub enum Update {
    /// Work in progress (a status-bar message).
    Status(String),
    Snapshot(Loaded),
    /// An error to show; the last snapshot stays.
    Error(String),
    /// A server answered: the caller's access (`reader`, `editor`).
    Connected {
        access: String,
    },
    /// The answer to an interaction.
    Interacted(Result<Value, String>),
    /// The answer to [`Request::Command`] `tag`.
    Ran {
        tag: u64,
        result: Result<Value, String>,
    },
    /// The answer to [`Request::ListFiles`].
    Files(Result<Vec<WsFile>, String>),
    /// The server's cap on document views per user (the snapshot's `view.limits`): the window
    /// opens at most that many workspace tabs beside its first one, [`crate::shell::MAX_DOCS`]
    /// tabs at most in all.
    Limits {
        views_per_user: usize,
    },
    /// The source could not start (the server refused the view: a cap, its memory, a busy
    /// workspace): the tab closes and the tab strip says why.
    Refused(String),
}

/// A file of a server workspace, as `GET /w/<id>/files/` lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct WsFile {
    /// Workspace-relative, `/`-separated.
    pub path: String,
    pub size: Option<u64>,
    /// RFC 3339, as the server writes it.
    pub modified: String,
}

impl std::fmt::Debug for Loaded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Loaded").field("seq", &self.seq).finish()
    }
}

/// Wakes the window up (egui's `request_repaint`).
pub type Notify = Arc<dyn Fn() + Send + Sync>;

/// The worker's side of the channels.
pub struct Link {
    pub requests: Receiver<Request>,
    /// For the worker's own helpers (the remote change feed).
    pub requests_tx: Sender<Request>,
    /// Raised by the window, out of turn, when it sends `dax.cancel`: a worker that runs the
    /// engine on its thread hands it to the session (`Session::set_cancel`) so the running query
    /// stops before the command's turn comes, and lowers it after each command.
    pub cancel: Arc<AtomicBool>,
    updates: Sender<Update>,
    notify: Notify,
}

impl Link {
    pub fn send(&self, u: Update) {
        let _ = self.updates.send(u);
        (self.notify)();
    }
}

/// A data source the shell drives itself instead of a worker thread: the browser shell
/// (`apps/bicraft-web`, docs/webui.md), where there are no threads and no blocking sockets. It is
/// handed each request as the window sends it and answers through its [`Link`] (whose `requests`
/// receiver it does not need).
pub trait Driver {
    fn request(&self, r: Request);
}

/// A started source.
pub enum Running {
    /// A worker thread reading [`Link::requests`] (native shells).
    #[cfg(not(target_arch = "wasm32"))]
    Thread(JoinHandle<()>),
    /// A source the shell drives.
    Driven(Box<dyn Driver>),
    /// The source could not start (it said why through its link).
    Failed,
}

/// Starts the source of an origin (the shell's).
pub type StartSource = Box<dyn Fn(Origin, Link) -> Running>;

/// Runs `run` on a worker thread named `bicraft-source`; a panic in it (the engine, the client)
/// becomes an error in the window instead of taking it down.
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn(link: Link, run: impl FnOnce(&Link) + Send + 'static) -> Running {
    let spawned = std::thread::Builder::new()
        .name("bicraft-source".into())
        .spawn(move || {
            let updates = link.updates.clone();
            let notify = Arc::clone(&link.notify);
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&link)));
            if r.is_err() {
                let _ = updates.send(Update::Error("internal error in the data source".into()));
                notify();
            }
        });
    match spawned {
        Ok(h) => Running::Thread(h),
        Err(_) => Running::Failed,
    }
}

/// A running source.
pub struct Worker {
    tx: Sender<Request>,
    cancel: Arc<AtomicBool>,
    rx: Receiver<Update>,
    running: Running,
}

impl Worker {
    /// Starts the source of `origin` with the shell's `start`.
    pub fn start(origin: Origin, notify: Notify, start: &StartSource) -> Worker {
        let (tx, requests) = channel();
        let (updates, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let link = Link {
            requests,
            requests_tx: tx.clone(),
            cancel: Arc::clone(&cancel),
            updates,
            notify,
        };
        let running = start(origin, link);
        Worker {
            tx,
            cancel,
            rx,
            running,
        }
    }

    pub fn send(&self, r: Request) {
        if let Request::Command { id, .. } = &r
            && id == CANCEL
        {
            self.cancel.store(true, Ordering::SeqCst);
        }
        match &self.running {
            Running::Driven(d) => d.request(r),
            _ => {
                let _ = self.tx.send(r);
            }
        }
    }

    /// The updates received since the last call.
    pub fn poll(&self) -> Vec<Update> {
        self.rx.try_iter().collect()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if let Running::Driven(d) = &self.running {
            d.request(Request::Stop);
        }
        // a worker thread stuck in a long evaluation or a network call ends on its own (detached)
        let _ = self.tx.send(Request::Stop);
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_secrets() {
        let o = Origin::Local {
            path: PathBuf::from("x").join("Sales.pbix"),
            lang: "en".into(),
        };
        assert_eq!(o.label(), "Sales.pbix");
        let s = Server {
            url: "https://bi.example.com/".into(),
            workspace: "sales".into(),
            token: "bct_secret".into(),
        };
        assert_eq!(Origin::Remote(s.clone()).label(), "sales · bi.example.com");
        assert!(!format!("{s:?}").contains("secret"));
        assert_eq!(host_of("http://127.0.0.1:8080/w"), "127.0.0.1:8080");
    }
}
