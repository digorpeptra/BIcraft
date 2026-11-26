//! The live viewer commands (M8, `docs/view-snapshot.md`):
//!
//! - `bicraft mcp --view [--view-port N] [--open]`: the MCP server on stdio, publishing a session
//!   snapshot to the viewer after every tool call, and between calls until every bound visual
//!   has its data.
//! - `bicraft view PATH [--port N] [--open] [--data-root DIR]`: opens a file or project and
//!   re-reads it whenever it changes on disk (whatever tool wrote it), publishing each new state.
//!   `--data-root` lets the tables' Power Query sources read files under DIR (none by default).
//!
//! `view` makes the page a report reader (M8.3, `docs/view-snapshot.md`, "Interactions"):
//! slicer clicks, cross-filtering on data points, page navigation and the slicer buttons go to
//! `POST /api/interact`, are run as `view.interact` on the session and republished; nothing is
//! written to the files. `--read-only` turns it off. (`mcp --view` stays read-only.)
//!
//! The page has an "Open in Power BI Desktop" button (`--no-open-button` removes it): it asks the
//! viewer to open the session's saved file with the OS default application
//! (`bicraft_engine::open_with_default_app`, `docs/view-snapshot.md`). It runs on an explicit
//! click only; bicraft itself never needs Desktop.
//!
//! The viewer listens on 127.0.0.1 only. Its URL goes to stderr (stdout is the MCP channel).
//! Neither command holds anything that needs flushing, so Ctrl+C (the default handler) is a
//! clean exit.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use bicraft_engine::watch::{POLL, Watcher, data_pending};
use bicraft_engine::{Session, UiLanguage};
use bicraft_view::ViewHandle;
use serde_json::json;

use super::{Failure, OK, USAGE_ERROR, usage};

fn parse_port(text: &str, flag: &str) -> Result<u16, Failure> {
    text.parse()
        .map_err(|_| usage(format!("{flag} needs a port number (0-65535), got `{text}`")))
}

fn next<'a>(it: &mut impl Iterator<Item = &'a String>, flag: &str) -> Result<&'a str, Failure> {
    it.next()
        .map(String::as_str)
        .ok_or_else(|| usage(format!("{flag} needs a value")))
}

/// Starts the viewer and announces it on stderr; `open` also launches the browser.
/// `open_button`: the page's "Open in Power BI Desktop" button (`--no-open-button` turns it off).
/// `app`: announce (and open) the browser app at `/app/` (`--ui egui`, docs/webui.md) instead of
/// the HTML page.
/// `control`: the UI control channel's relay to the app page (`--control`), its token written to
/// that file.
fn start_viewer(
    port: u16,
    open: bool,
    open_button: bool,
    interactions: bool,
    app: bool,
    control: Option<&Path>,
) -> Result<ViewHandle, Failure> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let opener = open_button.then(|| -> bicraft_view::Opener {
        Arc::new(|path: &Path| bicraft_engine::open_with_default_app(path))
    });
    let options = bicraft_view::ViewOptions {
        opener,
        interactions,
        control: control.is_some(),
    };
    let handle = bicraft_view::start_with(addr, options).map_err(|e| {
        (
            USAGE_ERROR,
            format!("cannot start the live viewer on {addr}: {e}"),
        )
    })?;
    let mut url = handle.url();
    if app {
        if !bicraft_view::webui::available() {
            return Err((
                USAGE_ERROR,
                "--ui egui: this bicraft was built without the browser app (docs/webui.md)".into(),
            ));
        }
        url = format!("{}/app/", url.trim_end_matches('/'));
    }
    eprintln!("bicraft: live viewer at {url}");
    if let (Some(file), Some(token)) = (control, handle.control_token()) {
        let text = json!({
            "kind": "view",
            "url": format!("{}/api/control", handle.url().trim_end_matches('/')),
            "token": token,
            "pid": std::process::id(),
        });
        bicraft_http_write(file, &text)?;
        eprintln!(
            "bicraft: UI control channel at {}/api/control for the app page (token in {})",
            handle.url().trim_end_matches('/'),
            file.display()
        );
    }
    if open {
        open_browser(&url);
    }
    Ok(handle)
}

/// Writes the control channel's token file, readable by its owner only.
fn bicraft_http_write(file: &Path, text: &serde_json::Value) -> Result<(), Failure> {
    let body = serde_json::to_string_pretty(text).unwrap_or_default();
    bicraft_http::write_owner_only(file, body.as_bytes())
        .map_err(|e| (USAGE_ERROR, format!("--control-file {}: {e}", file.display())))
}

/// Opens `url` in the default browser; failures are ignored (the URL is on stderr). The child's
/// output is discarded so it can never reach the MCP stream.
fn open_browser(url: &str) {
    let mut cmd = if cfg!(windows) {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", "", url]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    let spawned = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match spawned {
        // Reap it in the background so it does not linger as a zombie.
        Ok(mut child) => {
            let _ = std::thread::Builder::new()
                .name("open-browser".into())
                .spawn(move || child.wait());
        }
        Err(e) => eprintln!("bicraft: cannot open a browser ({e}); open the URL above"),
    }
}

/// Publishes the session's snapshot together with the images it references (served by the viewer
/// at `/api/asset/<id>`). Returns whether visuals still wait for their data: a snapshot computes
/// data within a time budget and leaves the rest `null` for the next snapshot.
fn publish(handle: &ViewHandle, session: &Session) -> bool {
    let images = session.snapshot_assets();
    let mut snap = session.snapshot();
    let pending = data_pending(&snap);
    // BIcraft's own visuals: the renderer's drawing and hit map (the page has no drawing of them)
    let by_id = images.iter().map(|a| (a.id.clone(), a.bytes.clone())).collect();
    let drawings = bicraft_engine::view_drawings::add_drawings(&mut snap, &by_id);
    let assets = images.into_iter().chain(drawings).map(|a| bicraft_view::Asset {
        id: a.id,
        mime: a.mime.to_string(),
        bytes: a.bytes,
    });
    // The file the "Open in Power BI Desktop" button opens: the session's, never the page's.
    let target = session.desktop_target();
    handle.set_open_target(target.path, target.unsaved);
    handle.publish_with_assets(snap, assets);
    // once published: the aggregation jobs the snapshot's visuals asked for
    session.background_work();
    pending
}

/// `--lang TAG`: the UI language of the generated visual text (M10.2).
fn parse_lang(text: &str) -> Result<UiLanguage, Failure> {
    UiLanguage::parse(text).map_err(|e| usage(format!("--lang: {e}")))
}

/// `bicraft mcp --view [--view-port N] [--open] [--lang TAG] [--no-open-button]` (`--view-port`,
/// `--open`, `--lang` and `--no-open-button` imply `--view`).
pub(crate) fn mcp(args: &[String]) -> Result<u8, Failure> {
    let mut port = 0;
    let mut open = false;
    let mut open_button = true;
    let mut lang = UiLanguage::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--view" => {}
            "--view-port" => port = parse_port(next(&mut it, "--view-port")?, "--view-port")?,
            "--open" => open = true,
            "--no-open-button" => open_button = false,
            "--lang" => lang = parse_lang(next(&mut it, "--lang")?)?,
            other => return Err(usage(format!("mcp: unexpected argument `{other}`"))),
        }
    }
    eprintln!("bicraft {}: MCP server on stdio", env!("CARGO_PKG_VERSION"));
    let handle = Arc::new(start_viewer(port, open, open_button, false, false, None)?);
    let mut first = Session::new();
    first.ui_language = lang.clone();
    publish(&handle, &first);
    let mut server = republishing_server(handle);
    server.session().ui_language = lang;
    let stdin = std::io::BufReader::new(std::io::stdin());
    let stdout = std::io::stdout();
    bicraft_mcp::serve_with_idle(
        &mut server,
        stdin,
        stdout.lock(),
        bicraft_mcp::MAX_LINE_BYTES,
        POLL,
    )
    .map_err(|e| (USAGE_ERROR, format!("mcp: {e}")))?;
    Ok(OK)
}

/// The MCP server of `mcp --view`: publishes after every tool call, then, while visuals still wait
/// for their data (a snapshot computes for a bounded time), keeps republishing between calls.
fn republishing_server(handle: Arc<ViewHandle>) -> bicraft_mcp::Server {
    let left = Arc::new(AtomicBool::new(false));
    let (h2, l2) = (Arc::clone(&handle), Arc::clone(&left));
    bicraft_mcp::Server::new()
        .with_observer(move |s| l2.store(publish(&h2, s), Ordering::SeqCst))
        .with_idle(move |s| {
            if !left.load(Ordering::SeqCst) {
                return false;
            }
            let more = publish(&handle, s);
            left.store(more, Ordering::SeqCst);
            more
        })
}

/// `bicraft view PATH [--port N] [--open] [--data-root DIR] [--lang TAG] [--culture NAME]
/// [--as-of YYYY-MM-DD] [--cache-abf CACHE.abf] [--no-open-button] [--read-only] [--ui html|egui]
/// [--control [--control-file F]]`.
pub(crate) fn view(args: &[String]) -> Result<u8, Failure> {
    let mut path = None;
    let mut port = 0;
    let mut open = false;
    let mut open_button = true;
    let mut interactions = true;
    let mut app = false;
    let mut data_root = None;
    let mut cache_abf = None;
    let mut lang = UiLanguage::default();
    let mut culture = None;
    let mut as_of = None;
    let mut control = false;
    let mut control_file = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--port" => port = parse_port(next(&mut it, "--port")?, "--port")?,
            "--open" => open = true,
            "--no-open-button" => open_button = false,
            "--read-only" => interactions = false,
            "--control" => control = true,
            "--control-file" => control_file = Some(PathBuf::from(next(&mut it, "--control-file")?)),
            "--ui" => {
                app = match next(&mut it, "--ui")? {
                    "html" => false,
                    "egui" => true,
                    other => return Err(usage(format!("--ui: `html` or `egui`, not `{other}`"))),
                }
            }
            "--lang" => lang = parse_lang(next(&mut it, "--lang")?)?,
            "--culture" => culture = Some(next(&mut it, "--culture")?.to_string()),
            "--as-of" => as_of = Some(next(&mut it, "--as-of")?.to_string()),
            "--data-root" => data_root = Some(next(&mut it, "--data-root")?),
            "--cache-abf" => cache_abf = Some(next(&mut it, "--cache-abf")?),
            other if other.starts_with("--") => return Err(usage(format!("unknown option `{other}`"))),
            other => {
                if path.replace(other).is_some() {
                    return Err(usage("view takes one PATH"));
                }
            }
        }
    }
    let path = PathBuf::from(path.ok_or_else(|| usage("view needs a PATH"))?);
    if !path.exists() {
        return Err((USAGE_ERROR, format!("{}: no such file or folder", path.display())));
    }
    if let Some(d) = data_root
        && !Path::new(d).is_dir()
    {
        return Err((USAGE_ERROR, format!("--data-root {d}: no such folder")));
    }
    if let Some(c) = cache_abf
        && !Path::new(c).is_file()
    {
        return Err((USAGE_ERROR, format!("--cache-abf {c}: no such file")));
    }
    let mut session = Session::new();
    session.ui_language = lang;
    // `--culture`: captions in that model culture (M10.4); kept across re-reads.
    session.display_culture = culture;
    // `--as-of`: the date TODAY()/NOW() return (reproducible previews).
    if let Some(d) = &as_of {
        session
            .execute("view.set_as_of", &json!({"as_of": d}))
            .map_err(|e| usage(format!("--as-of: {e}")))?;
    }
    // `data_root`: the only folder the tables' Power Query sources may read (none by default).
    // `cache_abf`: a PBIP's data read from that file (read-only) instead of its .pbi/cache.abf.
    let params = json!({"path": path, "data_root": data_root, "cache_abf": cache_abf});
    if let Err(e) = session.execute("file.open", &params) {
        // Keep serving: the file may be half-written; the next change re-reads it.
        eprintln!("bicraft: {e} (watching for changes)");
    }
    // The state as opened is revision 0; every re-read bumps it.
    session.revision = 0;
    let control_file = match (control, control_file) {
        (false, None) => None,
        (false, Some(_)) => return Err(usage("--control-file goes with --control")),
        (true, f) => {
            if !bicraft_view::webui::available() {
                return Err((
                    USAGE_ERROR,
                    "--control: this bicraft was built without the browser app that takes the requests (docs/webui.md)".into(),
                ));
            }
            Some(
                f.or_else(|| bicraft_env::var_os("BICRAFT_CONTROL_FILE").map(PathBuf::from))
                    .or_else(|| crate::ui_cmd::default_file(crate::ui_cmd::Kind::View))
                    .ok_or_else(|| usage("--control: no place for the token file: give --control-file"))?,
            )
        }
    };
    let handle = start_viewer(
        port,
        open,
        open_button,
        interactions,
        app,
        control_file.as_deref(),
    )?;
    // republished until every visual has its data (each snapshot computes for a bounded time)
    let mut data_left = publish(&handle, &session);
    eprintln!("bicraft: watching {} (Ctrl+C to stop)", path.display());

    let mut watcher = Watcher::new(&path, &session);
    loop {
        // waits for the next poll, answering the page's reader actions meanwhile
        if let Some(left) = serve_interactions(&handle, &mut session, POLL) {
            data_left = left;
            continue;
        }
        if !watcher.poll() {
            if data_left {
                data_left = publish(&handle, &session);
            }
            continue;
        }
        match session.execute("file.open", &params) {
            Ok(_) => eprintln!(
                "bicraft: re-read {} (revision {})",
                path.display(),
                session.revision
            ),
            Err(e) => eprintln!("bicraft: cannot re-read {}: {e}", path.display()),
        }
        data_left = publish(&handle, &session);
        watcher.reloaded(&path, &session);
    }
}

/// Waits up to `wait` for a reader action from the page; runs every one that arrives as
/// `view.interact` (republishing before answering, so the page's next snapshot is already sent),
/// then drains the queue. Returns, when one ran, whether visuals still wait for their data (see
/// [`publish`]). Without interactions it just sleeps.
fn serve_interactions(handle: &ViewHandle, session: &mut Session, wait: Duration) -> Option<bool> {
    if !handle.interactions_enabled() {
        std::thread::sleep(wait);
        return None;
    }
    let mut left = None;
    let mut next = handle.next_interaction(wait);
    while let Some(i) = next {
        let result = session.execute("view.interact", &i.action);
        if result.is_ok() || left.is_none() {
            left = Some(publish(handle, session));
        }
        i.respond(result);
        next = handle.next_interaction(Duration::ZERO);
    }
    left
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn ports_are_checked() {
        assert_eq!(parse_port("8080", "--port").unwrap(), 8080);
        assert!(parse_port("70000", "--port").is_err());
        assert!(parse_port("x", "--port").is_err());
    }

    #[test]
    fn lang_tags_are_checked() {
        assert_eq!(parse_lang("fr_fr").unwrap().tag(), "fr-FR");
        assert!(parse_lang("not a tag").is_err());
    }

    #[test]
    fn mcp_view_republishes_between_calls_only_while_data_is_pending() {
        let handle = bicraft_view::start(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).unwrap();
        let mut server = republishing_server(Arc::new(handle));
        // armed from the start; nothing published by a tool call yet, so nothing is pending
        assert!(server.idle_pending());
        server.run_idle();
        assert!(!server.idle_pending());
        // a tool call re-arms it; a model without visuals has no data left to compute
        let call = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                          "params": {"name": "new_model", "arguments": {"name": "M"}}});
        assert!(server.handle_line(&call.to_string()).is_some());
        assert!(server.idle_pending());
        server.run_idle();
        assert!(!server.idle_pending(), "stops once every visual has its data");
    }

    /// A raw HTTP request to the viewer; returns (status, body).
    fn http(addr: SocketAddr, request: String) -> (u16, String) {
        use std::io::{Read, Write};
        let mut s = std::net::TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        s.write_all(request.as_bytes()).unwrap();
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out);
        let text = String::from_utf8_lossy(&out).into_owned();
        let status = text.split(' ').nth(1).and_then(|x| x.parse().ok()).unwrap_or(0);
        let body = text
            .split_once("\r\n\r\n")
            .map(|(_, b)| b.to_string())
            .unwrap_or_default();
        (status, body)
    }

    fn card_value(addr: SocketAddr, card: &str) -> serde_json::Value {
        let request = format!(
            "GET /api/state HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n",
            addr.port()
        );
        let (status, body) = http(addr, request);
        assert_eq!(status, 200);
        let snap: serde_json::Value = serde_json::from_str(&body).unwrap();
        let pages = snap
            .pointer("/report/pages")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let v = pages
            .iter()
            .flat_map(|p| p["visuals"].as_array().cloned().unwrap_or_default())
            .find(|v| v["name"] == card)
            .unwrap();
        v["data"]["rows"][0][0].clone()
    }

    #[test]
    fn a_slicer_click_on_the_page_recomputes_the_visuals() {
        // a session with a literal table, a slicer and a card, served like `bicraft view`
        let mut s = Session::new();
        let sales = r#"#table({"Category", "Amount"}, {{"Wheels", 100}, {"Wheels", 50}, {"Toys", 5}})"#;
        for (id, p) in [
            ("model.new", json!({"name": "Shop"})),
            (
                "table.add",
                json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
            ),
            (
                "column.add",
                json!({"table": "Sales", "name": "Category", "data_type": "string"}),
            ),
            (
                "column.add",
                json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
            ),
            (
                "measure.add",
                json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
            ),
            ("report.new", json!({})),
            ("page.add", json!({"display_name": "Main"})),
        ] {
            s.execute(id, &p).unwrap();
        }
        let add = |s: &mut Session, ty: &str, field: &str| -> String {
            let r = s
                .execute(
                    "visual.add",
                    &json!({"page": "Main", "visual_type": ty, "x": 0, "y": 0,
                    "width": 100, "height": 100, "fields": [{"role": "Values", "field": field}]}),
                )
                .unwrap();
            r["visual"].as_str().unwrap().to_string()
        };
        let slicer = add(&mut s, "slicer", "Sales[Category]");
        let card = add(&mut s, "card", "[Total]");
        let page = s
            .report
            .as_ref()
            .unwrap()
            .pages
            .iter()
            .find(|p| p.display_name == "Main")
            .unwrap()
            .name
            .clone();
        let stored = s.report.clone();

        let handle = start_viewer(0, false, false, true, false, None).unwrap();
        let addr = handle.addr();
        publish(&handle, &s);
        assert_eq!(card_value(addr, &card), json!(155));
        let token = handle.interact_token().unwrap().to_string();
        let body =
            json!({"action": "slicer_toggle", "page": page, "visual": slicer, "value": "Toys"}).to_string();
        let request = format!(
            "POST /api/interact HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\n\
             Sec-Fetch-Site: same-origin\r\nX-Bicraft-Token: {token}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\n\r\n{body}",
            body.len(),
            port = addr.port()
        );
        let client = std::thread::spawn(move || http(addr, request));
        let deadline = Instant::now() + Duration::from_secs(20);
        while serve_interactions(&handle, &mut s, POLL).is_none() {
            assert!(Instant::now() < deadline, "no interaction arrived");
        }
        let (status, answer) = client.join().unwrap();
        assert_eq!(status, 200, "{answer}");
        let answer: serde_json::Value = serde_json::from_str(&answer).unwrap();
        assert_eq!(answer["result"]["changed"], true);
        // the card shows the new value; nothing reached the report
        assert_eq!(card_value(addr, &card), json!(5));
        assert_eq!(s.report, stored);
    }
}
