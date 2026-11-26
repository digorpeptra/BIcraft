//! The native client of a BIcraft server (`bicraft serve`, `docs/web-deployment.md`): the
//! workspace's viewer endpoints compute the snapshots and run the interactions; the app only
//! draws them. The pure half (URLs, the reading of answers, [`Failure`]) is shared with the
//! browser shell in [`bicraft_ui::remote`].
//!
//! ```text
//! POST {url}/w/{ws}/view/api/session          bearer token → {version, view, token, access}
//! GET  {url}/w/{ws}/view/api/state?v=VIEW     the view's snapshot
//! GET  {url}/w/{ws}/view/api/asset/ID?v=VIEW  its images
//! GET  {url}/w/{ws}/view/api/events?v=VIEW    SSE: `event: snapshot` when the view should refetch
//! POST {url}/w/{ws}/view/api/interact?v=VIEW  a `view.interact` action (X-Bicraft-Token)
//! ```
//!
//! The view is private to this window (its slicers and selections never reach anyone else),
//! readers see saved files, editors the editor session (`docs/auth.md`, "Roles"). The token is
//! sent in `Authorization` only, never logged, never written to disk. HTTPS through rustls with
//! the bundled Mozilla roots. A view the server forgot (a restart, an idle eviction) is replaced
//! by a new one transparently.

use std::collections::HashMap;
use std::io::BufRead;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bicraft_ui::remote::{
    COMMAND_HEADERS, Failure, MAX_ANSWER_BYTES, MAX_ASSET_BYTES, MAX_SNAPSHOT_BYTES, asset_ids, classify,
    command_answer, command_body, read_files, valid_workspace, view_base, workspace_base,
};
use bicraft_ui::source::{Assets, Link, Loaded, Request, Server, Update, WsFile};
use serde_json::Value;

/// The client of one workspace's viewer.
pub struct Client {
    agent: ureq::Agent,
    base: String,
    /// `{url}/w/{ws}`: the workspace's MCP endpoint and file routes.
    ws_base: String,
    auth: Option<String>,
    /// The view id and its interaction token.
    view: Option<(String, String)>,
    pub access: String,
}

fn agent(timeout: Option<Duration>) -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(timeout)
        .user_agent(concat!("bicraft-desktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent()
}

impl Client {
    pub fn new(server: &Server) -> Client {
        let token = server.token.trim();
        Client {
            agent: agent(Some(Duration::from_secs(120))),
            base: view_base(&server.url, &server.workspace),
            ws_base: workspace_base(&server.url, &server.workspace),
            auth: (!token.is_empty()).then(|| format!("Bearer {token}")),
            view: None,
            access: String::new(),
        }
    }

    fn get(&self, url: &str) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        let r = self.agent.get(url);
        match &self.auth {
            Some(a) => r.header("Authorization", a),
            None => r,
        }
    }

    fn post(&self, url: &str) -> ureq::RequestBuilder<ureq::typestate::WithBody> {
        let r = self.agent.post(url);
        match &self.auth {
            Some(a) => r.header("Authorization", a),
            None => r,
        }
    }

    fn check(
        resp: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
        limit: u64,
        on_view: bool,
    ) -> Result<Vec<u8>, Failure> {
        let mut resp = resp.map_err(|e| Failure::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        let body = resp
            .body_mut()
            .with_config()
            .limit(limit)
            .read_to_vec()
            .map_err(|e| Failure::Network(e.to_string()))?;
        classify(status, &body, on_view).map(|()| body)
    }

    /// Runs an engine command on the workspace's session through its MCP endpoint
    /// (`run_command`): editors only, under the server's policy (docs/server-mode.md).
    pub fn command(&self, id: &str, params: &Value) -> Result<Value, String> {
        let mut r = self.post(&format!("{}/mcp", self.ws_base));
        for (k, v) in COMMAND_HEADERS {
            r = r.header(k, v);
        }
        let mut resp = r
            .send(command_body(id, params))
            .map_err(|e| Failure::Network(e.to_string()).to_string())?;
        let status = resp.status().as_u16();
        let body = resp
            .body_mut()
            .with_config()
            .limit(MAX_ANSWER_BYTES)
            .read_to_vec()
            .map_err(|e| Failure::Network(e.to_string()).to_string())?;
        command_answer(status, &body)
    }

    /// The workspace's files (`GET /w/<id>/files/`).
    pub fn files(&self) -> Result<Vec<WsFile>, String> {
        let url = format!("{}/files/", self.ws_base);
        let body = Self::check(self.get(&url).call(), 32 << 20, false).map_err(|e| e.to_string())?;
        read_files(&body)
    }

    /// Opens a new private view.
    pub fn open_view(&mut self) -> Result<(), Failure> {
        let url = format!("{}/api/session", self.base);
        let body = Self::check(self.post(&url).send_empty(), 1 << 20, false)?;
        let v: Value = serde_json::from_slice(&body).map_err(|e| Failure::Protocol(e.to_string()))?;
        let hex32 = |s: &str| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit());
        match (
            v.get("view").and_then(Value::as_str),
            v.get("token").and_then(Value::as_str),
        ) {
            (Some(view), Some(token)) if hex32(view) && hex32(token) => {
                self.view = Some((view.to_string(), token.to_string()));
                self.access = v
                    .get("access")
                    .and_then(Value::as_str)
                    .unwrap_or("reader")
                    .chars()
                    .take(20)
                    .collect();
                Ok(())
            }
            _ => Err(Failure::Protocol(
                "no view in the answer (is this a BIcraft server with the viewer on?)".into(),
            )),
        }
    }

    pub fn view_id(&self) -> Option<&str> {
        self.view.as_ref().map(|(v, _)| v.as_str())
    }

    /// Runs `f` with the view, opening a new one when the server forgot it.
    fn with_view<T>(&mut self, f: impl Fn(&Client, &str, &str) -> Result<T, Failure>) -> Result<T, Failure> {
        if self.view.is_none() {
            self.open_view()?;
        }
        let (v, t) = self.view.clone().unwrap_or_default();
        match f(self, &v, &t) {
            Err(Failure::ViewGone) => {
                self.open_view()?;
                let (v, t) = self.view.clone().unwrap_or_default();
                f(self, &v, &t)
            }
            r => r,
        }
    }

    /// The view's snapshot.
    pub fn state(&mut self) -> Result<Value, Failure> {
        let body = self.with_view(|c, v, _| {
            let url = format!("{}/api/state?v={v}", c.base);
            Self::check(c.get(&url).call(), MAX_SNAPSHOT_BYTES, true)
        })?;
        serde_json::from_slice(&body).map_err(|e| Failure::Protocol(e.to_string()))
    }

    /// An image of the view's snapshot.
    pub fn asset(&self, id: &str) -> Result<Vec<u8>, Failure> {
        let v = self.view_id().unwrap_or_default();
        let url = format!("{}/api/asset/{id}?v={v}", self.base);
        Self::check(self.get(&url).call(), MAX_ASSET_BYTES, true)
    }

    /// Runs a `view.interact` action on the view.
    pub fn interact(&mut self, action: &Value) -> Result<Value, Failure> {
        let body = action.to_string();
        let out = self.with_view(|c, v, t| {
            let url = format!("{}/api/interact?v={v}", c.base);
            let r = c
                .post(&url)
                .header("X-Bicraft-Token", t)
                .header("Content-Type", "application/json")
                .send(body.as_bytes());
            match Self::check(r, 1 << 20, true) {
                // a refused action is a 400 with `{ok: false, error}`
                Err(Failure::Status(400, m)) => Ok(Err(m)),
                Err(e) => Err(e),
                Ok(b) => Ok(Ok(b)),
            }
        })?;
        match out {
            Err(m) => Err(Failure::Status(400, m)),
            Ok(b) => {
                let v: Value = serde_json::from_slice(&b).map_err(|e| Failure::Protocol(e.to_string()))?;
                Ok(v.get("result").cloned().unwrap_or(Value::Null))
            }
        }
    }

    fn events_url(&self) -> Option<String> {
        self.view_id().map(|v| format!("{}/api/events?v={v}", self.base))
    }
}

/// The view's snapshot with its images (kept from the previous snapshot when unchanged).
fn load(client: &mut Client, old: &Assets, seq: &mut u64) -> Result<Loaded, Failure> {
    let snap = client.state()?;
    let mut assets: Assets = HashMap::new();
    for id in asset_ids(&snap) {
        if let Some(b) = old.get(&id) {
            assets.insert(id, Arc::clone(b));
        } else if let Ok(b) = client.asset(&id) {
            // an image that cannot be read is drawn as missing, like the viewer's broken image
            assets.insert(id, Arc::from(b));
        }
    }
    *seq = seq.saturating_add(1);
    Ok(Loaded {
        snap: Arc::new(snap),
        assets: Arc::new(assets),
        seq: *seq,
    })
}

/// The change feed: reads the view's SSE stream and asks the worker to refetch on every
/// `snapshot` event; reconnects (with a short back-off) when the stream ends.
fn feed(url: Arc<Mutex<Option<String>>>, auth: Option<String>, tx: Sender<Request>, stop: Arc<AtomicBool>) {
    // no overall deadline: the stream lives as long as the view; keepalives come every few seconds
    let agent = agent(None);
    let mut backoff = Duration::from_millis(500);
    while !stop.load(Ordering::SeqCst) {
        let current = url.lock().map(|u| u.clone()).unwrap_or(None);
        let Some(u) = current else {
            std::thread::sleep(Duration::from_millis(200));
            continue;
        };
        let mut req = agent.get(&u).header("Accept", "text/event-stream");
        if let Some(a) = &auth {
            req = req.header("Authorization", a);
        }
        match req.call() {
            Ok(resp) if resp.status().as_u16() == 200 => {
                backoff = Duration::from_millis(500);
                let reader = std::io::BufReader::new(resp.into_body().into_reader());
                for line in reader.lines() {
                    if stop.load(Ordering::SeqCst) {
                        return;
                    }
                    let Ok(line) = line else { break };
                    if line.trim() == "event: snapshot" && tx.send(Request::Changed).is_err() {
                        return;
                    }
                }
                // the stream ended (the server restarted or dropped the view): the worker
                // refetches, which opens a new view if needed
                let _ = tx.send(Request::Changed);
            }
            Ok(resp) if resp.status().as_u16() == 404 => {
                // the view is gone: the worker's next fetch replaces it
                let _ = tx.send(Request::Changed);
            }
            _ => {}
        }
        std::thread::sleep(backoff);
        backoff = (backoff * 2).min(Duration::from_secs(5));
    }
}

pub fn run(server: &Server, link: &Link) {
    let mut client = Client::new(server);
    if !valid_workspace(&server.workspace) {
        link.send(Update::Error(format!(
            "`{}` is not a workspace id (letters, digits, -, _ and . only)",
            server.workspace
        )));
        return;
    }
    let host = bicraft_ui::source::host_of(&server.url);
    link.send(Update::Status(format!("{host}…")));
    let events = Arc::new(Mutex::new(None::<String>));
    let stop = Arc::new(AtomicBool::new(false));
    let mut seq = 0u64;
    let mut assets: Arc<Assets> = Arc::new(HashMap::new());
    let mut connected = false;
    // the first fetch opens the view; the feed starts once there is one
    let mut refresh = |client: &mut Client, assets: &mut Arc<Assets>, connected: &mut bool| match load(
        client, assets, &mut seq,
    ) {
        Ok(l) => {
            if !*connected {
                *connected = true;
                link.send(Update::Connected {
                    access: client.access.clone(),
                });
            }
            if let Ok(mut u) = events.lock() {
                *u = client.events_url();
            }
            *assets = Arc::clone(&l.assets);
            link.send(Update::Snapshot(l));
            link.send(Update::Status(String::new()));
            true
        }
        Err(e) => {
            // the next success says so again (the window clears the error)
            *connected = false;
            link.send(Update::Error(format!("{host}: {e}")));
            false
        }
    };
    refresh(&mut client, &mut assets, &mut connected);
    {
        let (events, stop, tx) = (Arc::clone(&events), Arc::clone(&stop), link.requests_tx.clone());
        let auth = client.auth.clone();
        let _ = std::thread::Builder::new()
            .name("bicraft-feed".into())
            .spawn(move || feed(events, auth, tx, stop));
    }
    loop {
        match link.requests.recv_timeout(Duration::from_secs(30)) {
            Ok(Request::Interact(action)) => {
                let r = client.interact(&action).map_err(|e| e.to_string());
                // the next snapshot first, then the answer (as the local source does)
                if r.is_ok() {
                    refresh(&mut client, &mut assets, &mut connected);
                }
                link.send(Update::Interacted(r));
            }
            Ok(Request::Changed) => {
                // coalesce a burst of events into one fetch
                let mut answers = Vec::new();
                let mut ran = Vec::new();
                while let Ok(r) = link.requests.try_recv() {
                    match r {
                        Request::Changed => {}
                        Request::Stop => {
                            stop.store(true, Ordering::SeqCst);
                            return;
                        }
                        Request::Interact(a) => answers.push(client.interact(&a).map_err(|e| e.to_string())),
                        Request::Command { tag, id, params } => ran.push(Update::Ran {
                            tag,
                            result: client.command(&id, &params),
                        }),
                        Request::ListFiles => ran.push(Update::Files(client.files())),
                        Request::Reload => {}
                    }
                }
                refresh(&mut client, &mut assets, &mut connected);
                for r in answers {
                    link.send(Update::Interacted(r));
                }
                for u in ran {
                    link.send(u);
                }
            }
            Ok(Request::Command { tag, id, params }) => {
                let result = client.command(&id, &params);
                if result.is_ok() {
                    refresh(&mut client, &mut assets, &mut connected);
                }
                link.send(Update::Ran { tag, result });
            }
            Ok(Request::ListFiles) => link.send(Update::Files(client.files())),
            Ok(Request::Reload) => {
                refresh(&mut client, &mut assets, &mut connected);
            }
            Ok(Request::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            // without a feed (a proxy that buffers SSE), still notice changes now and then
            Err(RecvTimeoutError::Timeout) => {
                refresh(&mut client, &mut assets, &mut connected);
            }
        }
    }
    stop.store(true, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unreachable_server_is_a_network_failure() {
        let server = Server {
            url: "http://127.0.0.1:9".into(),
            workspace: "a".into(),
            token: "t".into(),
        };
        let mut c = Client::new(&server);
        assert!(matches!(c.open_view(), Err(Failure::Network(_))));
    }
}
