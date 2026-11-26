//! The browser shell: the eframe web runner over [`bicraft_ui::App`] with the browser's
//! [`Shell`] (no file dialog, no Connect: the server and the file are the page's; the title is
//! `document.title`), and the data source it drives (no threads in a browser: requests run as
//! futures on the page's event loop).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use bicraft_ui::App;
use bicraft_ui::control::{Job, Ticket};
use bicraft_ui::remote::{Failure, asset_ids};
use bicraft_ui::shell::{Caps, PrefsStore, Shell};
use bicraft_ui::source::{Assets, Driver, Link, Loaded, Origin, Request, Running, Server, Update};
use serde_json::Value;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::{JsFuture, spawn_local};

use crate::protocol::{self, Config, Refetch, Route};

const CANVAS_ID: &str = "bicraft_canvas";
const LOADING_ID: &str = "bicraft_loading";
/// The interaction token's header (`bicraft_view::TOKEN_HEADER`).
const TOKEN_HEADER: &str = "X-Bicraft-Token";

/// Starts the app on the page's canvas.
pub fn start() {
    spawn_local(async {
        let Some(window) = web_sys::window() else {
            return;
        };
        let Some(document) = window.document() else {
            return;
        };
        let loading = document.get_element_by_id(LOADING_ID);
        let Some(canvas) = document
            .get_element_by_id(CANVAS_ID)
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
        else {
            if let Some(el) = &loading {
                el.set_text_content(Some("BIcraft: this page has no canvas."));
            }
            return;
        };
        let lang = window
            .navigator()
            .language()
            .filter(|l| !l.is_empty() && l.len() <= 40)
            .unwrap_or_else(|| "en-US".into());
        let location = window.location();
        let pathname = location.pathname().unwrap_or_default();
        let mut app = App::new(&lang, shell());
        // the UI control channel, when the server relays one (`bicraft view --control`)
        let control = Relay::start(&mut app, &pathname);
        app.open_server(Server {
            url: location.origin().unwrap_or_default(),
            workspace: protocol::workspace_of(&pathname).unwrap_or_else(|| "view".into()),
            token: String::new(),
        });
        // WebGPU where the browser has it, else WebGL2 (wgpu's own fallback); `?webgl` forces
        // WebGL2 (a browser whose WebGPU misbehaves)
        let mut options = eframe::WebOptions::default();
        if protocol::wants_webgl(&location.search().unwrap_or_default())
            && let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup
        {
            create.instance_descriptor.backends = eframe::wgpu::Backends::GL;
        }
        // F5 and Alt+N are the app's: the page stops the browser's reload (Ctrl+F5 and Ctrl+R
        // still reload); Ctrl+N is never handed to a page, so New is Alt+N here
        let on_key = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(|e: web_sys::KeyboardEvent| {
            if protocol::stops_browser_key(
                &e.key(),
                &e.code(),
                e.ctrl_key(),
                e.shift_key(),
                e.alt_key(),
                e.meta_key(),
            ) {
                e.prevent_default();
            }
        });
        let _ = window.add_event_listener_with_callback_and_bool(
            "keydown",
            on_key.as_ref().unchecked_ref(),
            true,
        );
        on_key.forget();
        // the page going (its browser tab or window closed, another page loaded): the document
        // views its tabs hold go on the server now, not after `--view-idle`; a page kept for the
        // back button keeps them (it may come back)
        let on_hide =
            Closure::<dyn FnMut(web_sys::PageTransitionEvent)>::new(|e: web_sys::PageTransitionEvent| {
                let held = HELD.with(|h| h.borrow_mut().page_hidden(e.persisted()));
                for (url, token) in held {
                    close_now(&url, &token);
                }
            });
        let _ = window.add_event_listener_with_callback("pagehide", on_hide.as_ref().unchecked_ref());
        on_hide.forget();
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                options,
                Box::new(move |cc| {
                    if let Some(c) = &control {
                        *c.ctx.borrow_mut() = Some(cc.egui_ctx.clone());
                    }
                    Ok(Box::new(Runner(app, control)))
                }),
            )
            .await;
        if let Some(el) = loading {
            match result {
                Ok(()) => el.remove(),
                Err(e) => el.set_text_content(Some(&format!(
                    "BIcraft could not start ({e:?}). A browser with WebGPU or WebGL2 is required."
                ))),
            }
        }
    });
}

struct Runner(App, Option<Rc<Relay>>);

impl eframe::App for Runner {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.0.logic(ctx);
        if let Some(c) = &self.1 {
            c.send_replies();
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.0.ui(ui);
    }
}

// ---------------------------------------------------------------------------------------------
// The UI control channel (docs/webui.md, "Control channel")

/// The page's end of the control channel's relay: requests come over `api/control/events`, run
/// in the window as the desktop's do (`bicraft_ui::control`), and their replies go back to
/// `api/control/reply` with the page token. Only on a page whose server relays them (the
/// `bicraft-control-token` slot).
struct Relay {
    jobs: std::sync::mpsc::SyncSender<Job>,
    token: String,
    root: String,
    cfg: Config,
    waiting: RefCell<Vec<(u64, Ticket)>>,
    ctx: RefCell<Option<egui::Context>>,
    events: RefCell<Option<web_sys::EventSource>>,
    listener: RefCell<Option<Closure<dyn FnMut(web_sys::Event)>>>,
}

impl Relay {
    fn start(app: &mut App, pathname: &str) -> Option<Rc<Relay>> {
        let cfg = Config::from_meta(meta);
        let token = cfg.control_token.clone()?;
        let (tx, rx) = std::sync::mpsc::sync_channel(bicraft_ui::control::MAX_QUEUED);
        app.control.attach(rx);
        let relay = Rc::new(Relay {
            jobs: tx,
            token,
            root: protocol::api_root(pathname),
            cfg,
            waiting: RefCell::new(Vec::new()),
            ctx: RefCell::new(None),
            events: RefCell::new(None),
            listener: RefCell::new(None),
        });
        let es =
            web_sys::EventSource::new(&protocol::url(&relay.root, &Route::ControlEvents, &relay.cfg)).ok()?;
        let w = Rc::downgrade(&relay);
        let on_control = Closure::<dyn FnMut(web_sys::Event)>::new(move |e: web_sys::Event| {
            let Some(r) = w.upgrade() else { return };
            let data = e
                .dyn_ref::<web_sys::MessageEvent>()
                .and_then(|m| m.data().as_string())
                .unwrap_or_default();
            r.take(&data);
        });
        let _ = es.add_event_listener_with_callback("control", on_control.as_ref().unchecked_ref());
        *relay.events.borrow_mut() = Some(es);
        *relay.listener.borrow_mut() = Some(on_control);
        Some(relay)
    }

    /// One relayed request: to the window, or refused at once.
    fn take(self: &Rc<Self>, data: &str) {
        match protocol::control_event(data) {
            Ok((seq, request, doc)) => {
                let id = request.id.clone();
                let (job, ticket) = Job::new(request);
                let job = job.in_doc(doc);
                if self.jobs.try_send(job).is_err() {
                    self.post(
                        seq,
                        bicraft_ui::control::err(&id, "busy: the window has too many requests queued"),
                    );
                    return;
                }
                self.waiting.borrow_mut().push((seq, ticket));
                if let Some(ctx) = self.ctx.borrow().as_ref() {
                    ctx.request_repaint();
                }
            }
            Err(Some((seq, reply))) => self.post(seq, reply),
            Err(None) => {}
        }
    }

    /// Posts the replies the window has given (after each frame's logic).
    fn send_replies(self: &Rc<Self>) {
        let done: Vec<(u64, Value)> = {
            let mut w = self.waiting.borrow_mut();
            let mut out = Vec::new();
            w.retain(|(seq, t)| match t.try_take() {
                Some(v) => {
                    out.push((*seq, v));
                    false
                }
                None => true,
            });
            out
        };
        for (seq, v) in done {
            self.post(seq, v);
        }
    }

    fn post(self: &Rc<Self>, seq: u64, reply: Value) {
        let url = protocol::url(&self.root, &Route::ControlReply, &self.cfg);
        let token = self.token.clone();
        spawn_local(async move {
            let body = protocol::control_reply(seq, &reply);
            let _ = fetch(&url, Some((&token, &body)), protocol::limit(&Route::ControlReply)).await;
        });
    }
}
// ---------------------------------------------------------------------------------------------
// The data source

/// The content of `<meta name=…>`.
fn meta(name: &str) -> Option<String> {
    let document = web_sys::window()?.document()?;
    document
        .query_selector(&format!("meta[name=\"{name}\"]"))
        .ok()
        .flatten()?
        .get_attribute("content")
}

struct Browser {
    link: Link,
    /// The view's id and token: the page's, or a document view's once the server opened it.
    cfg: RefCell<Config>,
    /// A document view (a workspace file's tab, docs/webui.md, "Document tabs"): its own
    /// session on the server, its commands at its own MCP endpoint.
    doc: bool,
    root: String,
    host: String,
    st: RefCell<State>,
}

#[derive(Default)]
struct State {
    seq: u64,
    assets: Arc<Assets>,
    refetch: Refetch,
    /// The newest snapshot an event carried inline (fetched otherwise).
    inline: Option<Value>,
    connected: bool,
    stopped: bool,
    events: Option<web_sys::EventSource>,
    /// The event stream's listeners, kept alive with it.
    listeners: Vec<Closure<dyn FnMut(web_sys::Event)>>,
    failures: u32,
}

struct BrowserDriver(Rc<Browser>);

impl Driver for BrowserDriver {
    fn request(&self, r: Request) {
        let b = Rc::clone(&self.0);
        match r {
            Request::Interact(action) => spawn_local(async move { b.interact(action).await }),
            Request::Command { tag, id, params } => {
                spawn_local(async move { b.command(tag, id, params).await })
            }
            Request::ListFiles => spawn_local(async move { b.files().await }),
            Request::Reload | Request::Changed => b.changed(None),
            Request::Stop => b.stop(),
        }
    }
}

/// The `localStorage` key of the UI preferences.
const PREFS_KEY: &str = "bicraft.ui";

/// The browser's preferences: the UI preferences only ([`bicraft_ui::prefs::ui_only`]: theme,
/// zoom, ribbon, tab), in `localStorage` (docs/webui.md, "Browser storage"). Every access is
/// wrapped: storage that is blocked, absent or full, or a damaged value, gives the defaults.
struct LocalStore;

impl PrefsStore for LocalStore {
    fn load(&self) -> Value {
        let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) else {
            return Value::Null;
        };
        let text = s.get_item(PREFS_KEY).ok().flatten().unwrap_or_default();
        if text.len() > 64 * 1024 {
            return Value::Null;
        }
        serde_json::from_str::<Value>(&text)
            .map(|v| bicraft_ui::prefs::ui_only(&v))
            .unwrap_or(Value::Null)
    }

    fn save(&self, v: &Value) -> Result<(), String> {
        // never an error for the user: a browser that refuses storage keeps the defaults
        if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
            let _ = s.set_item(PREFS_KEY, &bicraft_ui::prefs::ui_only(v).to_string());
        }
        Ok(())
    }
}

/// The browser's shell: driven sources, the UI preferences in `localStorage`, `document.title`
/// as the title.
fn shell() -> Shell {
    let mut s = Shell::new(Caps::BROWSER, |origin, link| {
        Running::Driven(start_source(origin, link))
    });
    s.set_title = Some(Box::new(|_ctx: &egui::Context, title: &str| {
        if let Some(d) = web_sys::window().and_then(|w| w.document()) {
            d.set_title(title);
        }
    }));
    s.prefs = Some(Box::new(LocalStore));
    // the system's high contrast (Windows contrast themes force colours; others ask for more)
    s.system_high_contrast = web_sys::window().is_some_and(|w| {
        ["(forced-colors: active)", "(prefers-contrast: more)"]
            .iter()
            .any(|q| w.match_media(q).ok().flatten().is_some_and(|m| m.matches()))
    });
    // each workspace file opens in a tab of its own, a document view on the server with its own
    // session (docs/webui.md, "Document tabs"); Open > Browse lists the workspace's files on
    // `bicraft serve`
    s.ws_tabs = true;
    s.workspace = web_sys::window()
        .and_then(|w| w.location().pathname().ok())
        .and_then(|p| protocol::workspace_of(&p));
    // exports written in the workspace go to the user through its file route
    if let Some(ws) = s.workspace.clone() {
        let ws2 = ws.clone();
        s.download = Some(Box::new(move |rel: &str| download(&ws, rel)));
        // File › Export › Print: the PDF report.print wrote, printed by the browser
        s.print = Some(Box::new(move |rel: &str| print_pdf(&ws2, rel)));
    }
    // Import a visual from a file: the browser's file input, the file read in the page
    s.upload = Some(Box::new(upload));
    s.now = Some(Box::new(|| {
        let ms = js_sys::Date::now();
        if ms.is_finite() && ms > 0.0 {
            (ms / 1000.0) as u64
        } else {
            0
        }
    }));
    // Paste (the button and the menu): the async clipboard API; refused or unavailable, the
    // app uses its in-app clipboard (Ctrl+V reads the page's paste event instead)
    s.read_clipboard = Some(Box::new(|slot: bicraft_ui::canvas::ClipSlot| {
        let Some(w) = web_sys::window() else {
            *slot.borrow_mut() = Some(Err("no window".into()));
            return;
        };
        let promise = w.navigator().clipboard().read_text();
        spawn_local(async move {
            let r = match JsFuture::from(promise).await {
                Ok(v) => v
                    .as_string()
                    .ok_or_else(|| "the clipboard holds no text".to_string()),
                Err(e) => Err(e
                    .as_string()
                    .unwrap_or_else(|| "the clipboard cannot be read".into())),
            };
            *slot.borrow_mut() = Some(r);
        });
    }));
    s
}

/// Largest file the browser uploads: its bytes go to the server in base64 inside one MCP
/// request, whose body `bicraft serve` and `bicraft view` cap at 8 MB by default.
const MAX_BROWSER_UPLOAD: usize = 5 << 20;

/// A file upload through a file input (`<input type=file>`): the chosen file is read in the page
/// and put in the slot; past the size limit, the slot says why. A dialog the user closes leaves
/// the slot empty (browsers do not always say so); the next upload replaces it.
fn upload(
    lang: bicraft_ui::i18n::Lang,
    _title: &str,
    exts: &[&str],
    max: usize,
    slot: bicraft_ui::shell::UploadSlot,
) {
    use bicraft_ui::shell::Picked;
    let fail = |slot: &bicraft_ui::shell::UploadSlot, why: String| {
        *slot.borrow_mut() = Some(Picked::Failed(why));
    };
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        fail(&slot, "no document".into());
        return;
    };
    let input = match doc
        .create_element("input")
        .ok()
        .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
    {
        Some(i) => i,
        None => {
            fail(&slot, "the page cannot open a file input".into());
            return;
        }
    };
    input.set_type("file");
    let accept: Vec<String> = exts.iter().map(|e| format!(".{e}")).collect();
    input.set_accept(&accept.join(","));
    let cap = max.min(MAX_BROWSER_UPLOAD);
    let picked = input.clone();
    let on_change = Closure::once_into_js(move || {
        let Some(file) = picked.files().and_then(|f| f.get(0)) else {
            *slot.borrow_mut() = Some(Picked::Cancelled);
            return;
        };
        let name = file.name();
        let size = file.size();
        if !size.is_finite() || size > cap as f64 {
            *slot.borrow_mut() = Some(Picked::Failed(format!(
                "{name}: {} ({} MB)",
                lang.t(
                    "the file is larger than the browser's upload limit",
                    "le fichier dépasse la limite d'envoi du navigateur"
                ),
                cap >> 20
            )));
            return;
        }
        let promise = file.array_buffer();
        spawn_local(async move {
            let got = match JsFuture::from(promise).await {
                Ok(buf) => Picked::File {
                    name,
                    bytes: js_sys::Uint8Array::new(&buf).to_vec(),
                },
                Err(e) => Picked::Failed(
                    e.as_string()
                        .unwrap_or_else(|| format!("{name}: the file cannot be read")),
                ),
            };
            *slot.borrow_mut() = Some(got);
        });
    });
    input.set_onchange(Some(on_change.unchecked_ref()));
    input.click();
}

/// A same-origin request with explicit headers (the MCP endpoint, the file routes).
async fn fetch_with(
    url: &str,
    method: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
    limit: u64,
) -> Result<(u16, Vec<u8>), Failure> {
    let net = |e: JsValue| Failure::Network(e.as_string().unwrap_or_else(|| format!("{e:?}")));
    let window = web_sys::window().ok_or_else(|| Failure::Network("no window".into()))?;
    let init = web_sys::RequestInit::new();
    init.set_cache(web_sys::RequestCache::NoStore);
    init.set_credentials(web_sys::RequestCredentials::SameOrigin);
    init.set_method(method);
    if let Some(b) = body {
        init.set_body(&JsValue::from_str(b));
    }
    let request = web_sys::Request::new_with_str_and_init(url, &init).map_err(net)?;
    for (k, v) in headers {
        request.headers().set(k, v).map_err(net)?;
    }
    let resp: web_sys::Response = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(net)?
        .dyn_into()
        .map_err(net)?;
    let status = resp.status();
    let buf = JsFuture::from(resp.array_buffer().map_err(net)?)
        .await
        .map_err(net)?;
    let bytes = js_sys::Uint8Array::new(&buf);
    if u64::from(bytes.length()) > limit {
        return Err(Failure::Protocol(format!("answer larger than {limit} bytes")));
    }
    Ok((status, bytes.to_vec()))
}

/// The page's own tab (the page's view, which follows the workspace's session), or a workspace
/// file's tab: a document view the server opens for it first (`POST …/api/session {file}`).
fn start_source(origin: Origin, link: Link) -> Box<dyn Driver> {
    let window = web_sys::window();
    let location = window.as_ref().map(web_sys::Window::location);
    let pathname = location
        .as_ref()
        .and_then(|l| l.pathname().ok())
        .unwrap_or_default();
    let host = location.as_ref().and_then(|l| l.host().ok()).unwrap_or_default();
    let file = match origin {
        Origin::Workspace { file, .. } => Some(file),
        _ => None,
    };
    let b = Rc::new(Browser {
        link,
        cfg: RefCell::new(if file.is_some() {
            Config::default()
        } else {
            Config::from_meta(meta)
        }),
        doc: file.is_some(),
        root: protocol::api_root(&pathname),
        host,
        st: RefCell::new(State::default()),
    });
    b.link.send(Update::Status(format!("{}…", b.host)));
    match file {
        Some(file) => {
            let d = Rc::clone(&b);
            spawn_local(async move { d.open_doc(file).await });
        }
        None => {
            b.changed(None);
            Browser::connect(&b);
        }
    }
    Box::new(BrowserDriver(b))
}

/// A `fetch` on this origin: the status and the body (at most `limit` bytes).
async fn fetch(url: &str, post: Option<(&str, &str)>, limit: u64) -> Result<(u16, Vec<u8>), Failure> {
    let net = |e: JsValue| Failure::Network(e.as_string().unwrap_or_else(|| format!("{e:?}")));
    let window = web_sys::window().ok_or_else(|| Failure::Network("no window".into()))?;
    let init = web_sys::RequestInit::new();
    init.set_cache(web_sys::RequestCache::NoStore);
    init.set_credentials(web_sys::RequestCredentials::SameOrigin);
    if let Some((_, body)) = post {
        init.set_method("POST");
        init.set_body(&JsValue::from_str(body));
    }
    let request = web_sys::Request::new_with_str_and_init(url, &init).map_err(net)?;
    if let Some((token, _)) = post {
        let h = request.headers();
        h.set("Content-Type", "application/json").map_err(net)?;
        if !token.is_empty() {
            h.set(TOKEN_HEADER, token).map_err(net)?;
        }
    }
    let resp: web_sys::Response = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(net)?
        .dyn_into()
        .map_err(net)?;
    let status = resp.status();
    let buf = JsFuture::from(resp.array_buffer().map_err(net)?)
        .await
        .map_err(net)?;
    let bytes = js_sys::Uint8Array::new(&buf);
    if u64::from(bytes.length()) > limit {
        return Err(Failure::Protocol(format!("answer larger than {limit} bytes")));
    }
    Ok((status, bytes.to_vec()))
}

impl Browser {
    fn url(&self, route: &Route<'_>) -> String {
        protocol::url(&self.root, route, &self.cfg.borrow())
    }

    /// Asks the server for this tab's document view on `file`, then reads it as the page's view
    /// is read. A refusal (the server's caps, its memory, a busy workspace) closes the tab with
    /// the message in the tab strip.
    async fn open_doc(self: Rc<Self>, file: String) {
        // the page's own view names the page that holds this document view (the control channel)
        let url = protocol::session_url(&self.root, Config::from_meta(meta).view.as_deref());
        let body = protocol::session_body(&file);
        let r = match fetch_with(
            &url,
            "POST",
            &[("Content-Type", "application/json")],
            Some(&body),
            1 << 20,
        )
        .await
        {
            Ok((status, bytes)) => protocol::read_session(status, &bytes),
            Err(e) => Err(protocol::Refusal::Error(protocol::wording(&e))),
        };
        if self.st.borrow().stopped {
            // the tab closed meanwhile: its view goes too
            if let Ok(d) = &r
                && let Some(url) = protocol::close_url(&self.root, &d.cfg)
            {
                close_now(&url, d.cfg.interact_token.as_deref().unwrap_or_default());
            }
            return;
        }
        match r {
            Ok(d) => {
                HELD.with(|h| h.borrow_mut().hold(&self.root, &d.cfg));
                *self.cfg.borrow_mut() = d.cfg;
                self.link.send(Update::Connected { access: d.access });
                self.changed(None);
                Browser::connect(&self);
            }
            Err(protocol::Refusal::Limit(why)) => self.link.send(Update::Refused(why)),
            Err(protocol::Refusal::Error(why)) => {
                self.link.send(Update::Error(format!("{}: {why}", self.host)));
            }
        }
    }

    /// Something changed (an event, a reload, an answered click): fetch the snapshot, once for a
    /// burst.
    fn changed(self: &Rc<Self>, inline: Option<Value>) {
        let start = {
            let mut st = self.st.borrow_mut();
            if st.stopped {
                return;
            }
            if inline.is_some() {
                st.inline = inline;
            }
            st.refetch.want()
        };
        if start {
            let b = Rc::clone(self);
            spawn_local(async move { b.refresh_loop().await });
        }
    }

    async fn refresh_loop(self: Rc<Self>) {
        loop {
            let inline = self.st.borrow_mut().inline.take();
            let r = match inline {
                Some(snap) => Ok(snap),
                None => {
                    let route = Route::State;
                    match fetch(&self.url(&route), None, protocol::limit(&route)).await {
                        Ok((status, body)) => protocol::read_state(status, &body),
                        Err(e) => Err(e),
                    }
                }
            };
            match r {
                Ok(snap) => self.publish(snap).await,
                Err(e) => {
                    self.st.borrow_mut().connected = false;
                    self.link
                        .send(Update::Error(format!("{}: {}", self.host, protocol::wording(&e))));
                }
            }
            let again = {
                let mut st = self.st.borrow_mut();
                !st.stopped && st.refetch.done()
            };
            if !again {
                return;
            }
        }
    }

    /// Fetches the images the snapshot needs (keeping the ones already here) and hands it over.
    async fn publish(&self, snap: Value) {
        let old = Arc::clone(&self.st.borrow().assets);
        let mut assets: Assets = HashMap::new();
        for id in asset_ids(&snap) {
            if let Some(b) = old.get(&id) {
                assets.insert(id, Arc::clone(b));
                continue;
            }
            let route = Route::Asset(&id);
            // an image that cannot be read is drawn as missing, like the viewer's broken image
            if let Ok((200, bytes)) = fetch(&self.url(&route), None, protocol::limit(&route)).await {
                assets.insert(id.clone(), Arc::from(bytes));
            }
        }
        let assets = Arc::new(assets);
        let (seq, first) = {
            let mut st = self.st.borrow_mut();
            st.seq = st.seq.saturating_add(1);
            st.assets = Arc::clone(&assets);
            let first = !st.connected;
            st.connected = true;
            (st.seq, first)
        };
        if first && let Some(access) = protocol::role(&snap) {
            self.link.send(Update::Connected { access });
        }
        if first && let Some(views_per_user) = protocol::views_per_user(&snap) {
            self.link.send(Update::Limits { views_per_user });
        }
        self.link.send(Update::Snapshot(Loaded {
            snap: Arc::new(snap),
            assets,
            seq,
        }));
        self.link.send(Update::Status(String::new()));
    }

    async fn interact(self: Rc<Self>, action: Value) {
        let token = self.cfg.borrow().interact_token.clone();
        let Some(token) = token else {
            self.link.send(Update::Interacted(Err(
                "this viewer is read-only (started with --read-only)".into(),
            )));
            return;
        };
        let route = Route::Interact;
        let body = action.to_string();
        let r = match fetch(&self.url(&route), Some((&token, &body)), protocol::limit(&route)).await {
            Ok((status, bytes)) => protocol::read_interaction(status, &bytes),
            Err(e) => Err(protocol::wording(&e)),
        };
        if r.is_ok() {
            self.changed(None);
        }
        self.link.send(Update::Interacted(r));
    }

    /// `/w/<id>` of the page's workspace (`bicraft serve` only).
    fn workspace_root(&self) -> Option<String> {
        self.workspace().map(|ws| format!("/w/{ws}"))
    }

    /// The page's workspace id (`bicraft serve` only).
    fn workspace(&self) -> Option<String> {
        let path = web_sys::window()?.location().pathname().ok()?;
        protocol::workspace_of(&path)
    }

    /// An engine command on the workspace's session through its MCP endpoint (`run_command`), with
    /// the browser's own credentials (the auth proxy's); `bicraft view` has none.
    async fn command(self: Rc<Self>, tag: u64, id: String, params: Value) {
        let result = match self.workspace() {
            None => Err(
                "this page runs no commands (bicraft view): use the desktop app, the CLI or bicraft serve"
                    .to_string(),
            ),
            Some(ws) => {
                // a document view's commands run on its own session, with its token
                let (vid, token) = {
                    let cfg = self.cfg.borrow();
                    (cfg.view.clone(), cfg.interact_token.clone().unwrap_or_default())
                };
                let url = protocol::mcp_url(&ws, if self.doc { vid.as_deref() } else { None });
                let body = bicraft_ui::remote::command_body(&id, &params);
                let mut headers: Vec<(&str, &str)> = bicraft_ui::remote::COMMAND_HEADERS.to_vec();
                if self.doc {
                    headers.push((TOKEN_HEADER, &token));
                }
                match fetch_with(
                    &url,
                    "POST",
                    &headers,
                    Some(&body),
                    bicraft_ui::remote::MAX_ANSWER_BYTES,
                )
                .await
                {
                    Ok((status, bytes)) => bicraft_ui::remote::command_answer(status, &bytes),
                    Err(e) => Err(protocol::wording(&e)),
                }
            }
        };
        if result.is_ok() {
            self.changed(None);
        }
        self.link.send(Update::Ran { tag, result });
    }

    /// The workspace's files (`GET /w/<id>/files/`).
    async fn files(self: Rc<Self>) {
        let r = match self.workspace_root() {
            None => Err("this page has no server workspace (bicraft view)".to_string()),
            Some(root) => {
                let url = format!("{root}/files/");
                match fetch_with(&url, "GET", &[("Accept", "application/json")], None, 32 << 20).await {
                    Ok((status, bytes)) => bicraft_ui::remote::classify(status, &bytes, false)
                        .map_err(|e| protocol::wording(&e))
                        .and_then(|()| bicraft_ui::remote::read_files(&bytes)),
                    Err(e) => Err(protocol::wording(&e)),
                }
            }
        };
        self.link.send(Update::Files(r));
    }

    /// Opens the event stream; on an error it is closed and opened again after a back-off.
    fn connect(b: &Rc<Self>) {
        if b.st.borrow().stopped {
            return;
        }
        let Ok(es) = web_sys::EventSource::new(&b.url(&Route::Events)) else {
            Browser::retry(b);
            return;
        };
        let on_snapshot = {
            let w = Rc::downgrade(b);
            Closure::<dyn FnMut(web_sys::Event)>::new(move |e: web_sys::Event| {
                let Some(b) = w.upgrade() else { return };
                let data = e
                    .dyn_ref::<web_sys::MessageEvent>()
                    .and_then(|m| m.data().as_string())
                    .unwrap_or_default();
                b.changed(protocol::inline_snapshot(&data));
            })
        };
        let on_open = {
            let w = Rc::downgrade(b);
            Closure::<dyn FnMut(web_sys::Event)>::new(move |_e: web_sys::Event| {
                if let Some(b) = w.upgrade() {
                    b.st.borrow_mut().failures = 0;
                }
            })
        };
        let on_error = {
            let w = Rc::downgrade(b);
            Closure::<dyn FnMut(web_sys::Event)>::new(move |_e: web_sys::Event| {
                if let Some(b) = w.upgrade() {
                    Browser::retry(&b);
                }
            })
        };
        let _ = es.add_event_listener_with_callback("snapshot", on_snapshot.as_ref().unchecked_ref());
        let _ = es.add_event_listener_with_callback("open", on_open.as_ref().unchecked_ref());
        let _ = es.add_event_listener_with_callback("error", on_error.as_ref().unchecked_ref());
        let mut st = b.st.borrow_mut();
        st.events = Some(es);
        st.listeners = vec![on_snapshot, on_open, on_error];
    }

    /// Closes the stream and opens it again later (0.5 s doubling to 15 s); the snapshot is
    /// fetched again on reconnection, since events may have been missed.
    fn retry(b: &Rc<Self>) {
        let wait = {
            let mut st = b.st.borrow_mut();
            if let Some(es) = st.events.take() {
                es.close();
            }
            // the listeners stay alive until the next connect replaces them (one may be running)
            let wait = protocol::backoff_ms(st.failures);
            st.failures = st.failures.saturating_add(1);
            wait
        };
        if b.st.borrow().stopped {
            return;
        }
        let w = Rc::downgrade(b);
        let again = Closure::once_into_js(move || {
            if let Some(b) = w.upgrade() {
                Browser::connect(&b);
                b.changed(None);
            }
        });
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                again.unchecked_ref(),
                i32::try_from(wait).unwrap_or(15_000),
            );
        }
    }

    fn stop(&self) {
        {
            let mut st = self.st.borrow_mut();
            st.stopped = true;
            if let Some(es) = st.events.take() {
                es.close();
            }
            st.listeners.clear();
        }
        // a closed tab's document view goes on the server (its session with it)
        if self.doc {
            let held = HELD.with(|h| h.borrow_mut().release(&self.root, &self.cfg.borrow()));
            if let Some((url, token)) = held {
                close_now(&url, &token);
            }
        }
    }
}

thread_local! {
    /// The document views this page's tabs hold, released when the page goes (`pagehide`).
    static HELD: RefCell<protocol::Held> = RefCell::new(protocol::Held::default());
}

/// `POST …/api/close?v=<view>` with the view's token: sent now (a `keepalive` request, so it
/// outlives a page that is going away), its answer awaited in the background and ignored.
fn close_now(url: &str, token: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let init = web_sys::RequestInit::new();
    init.set_cache(web_sys::RequestCache::NoStore);
    init.set_credentials(web_sys::RequestCredentials::SameOrigin);
    init.set_method("POST");
    // `keepalive` has no setter in web-sys yet
    let _ = js_sys::Reflect::set(init.as_ref(), &JsValue::from_str("keepalive"), &JsValue::TRUE);
    let Ok(request) = web_sys::Request::new_with_str_and_init(url, &init) else {
        return;
    };
    if request.headers().set(TOKEN_HEADER, token).is_err() {
        return;
    }
    let sent = JsFuture::from(window.fetch_with_request(&request));
    spawn_local(async move {
        let _ = sent.await;
    });
}

/// Most bytes of a PDF fetched for printing.
const MAX_PRINT_BYTES: u64 = 512 << 20;
/// The id of the hidden frame a PDF is printed from (one at a time: the last replaces it).
const PRINT_FRAME: &str = "bicraft-print-frame";

/// Prints workspace PDF `rel`: fetched (`GET /w/<ws>/files/<rel>`), shown as a `blob:` URL of
/// type `application/pdf` in a hidden frame, and printed by the browser's print of that frame
/// once it loads. Anything that fails falls back to the download of the file.
fn print_pdf(ws: &str, rel: &str) {
    let Some(url) = protocol::file_url(ws, rel) else {
        return;
    };
    let (ws, rel) = (ws.to_string(), rel.to_string());
    spawn_local(async move {
        let printed = match fetch_with(&url, "GET", &[], None, MAX_PRINT_BYTES).await {
            Ok((200, bytes)) => print_bytes(&bytes).is_some(),
            _ => false,
        };
        if !printed {
            download(&ws, &rel);
        }
    });
}

/// The hidden frame for `bytes` (a PDF); its load prints it.
fn print_bytes(bytes: &[u8]) -> Option<()> {
    let doc = web_sys::window()?.document()?;
    if let Some(old) = doc.get_element_by_id(PRINT_FRAME) {
        if let Some(src) = old.get_attribute("src") {
            let _ = web_sys::Url::revoke_object_url(&src);
        }
        old.remove();
    }
    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(bytes));
    let bag = web_sys::BlobPropertyBag::new();
    bag.set_type("application/pdf");
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &bag).ok()?;
    let src = web_sys::Url::create_object_url_with_blob(&blob).ok()?;
    let frame: web_sys::HtmlIFrameElement = doc.create_element("iframe").ok()?.dyn_into().ok()?;
    frame.set_id(PRINT_FRAME);
    // off screen but laid out (a frame that is not displayed does not print)
    frame
        .set_attribute(
            "style",
            "position:fixed;right:0;bottom:0;width:1px;height:1px;border:0;opacity:0",
        )
        .ok()?;
    let target = frame.clone();
    let on_load = Closure::once_into_js(move || {
        if let Some(w) = target.content_window() {
            let _ = w.print();
        }
    });
    frame.set_onload(Some(on_load.unchecked_ref()));
    frame.set_src(&src);
    doc.body()?.append_child(&frame).ok()?;
    Some(())
}

/// Downloads workspace file `rel` (`GET /w/<ws>/files/<rel>`, which answers it as an
/// attachment): a link with `download`, clicked, then removed.
fn download(ws: &str, rel: &str) {
    let Some(url) = protocol::file_url(ws, rel) else {
        return;
    };
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Some(a) = doc
        .create_element("a")
        .ok()
        .and_then(|e| e.dyn_into::<web_sys::HtmlAnchorElement>().ok())
    else {
        return;
    };
    a.set_href(&url);
    a.set_download(rel.rsplit('/').next().unwrap_or(rel));
    if let Some(body) = doc.body() {
        let _ = body.append_child(&a);
        a.click();
        let _ = body.remove_child(&a);
    }
}
