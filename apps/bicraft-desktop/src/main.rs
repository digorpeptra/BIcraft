//! `bicraft-desktop`: the BIcraft desktop app (docs/desktop.md).
//!
//! ```text
//! bicraft-desktop [FILE] [--lang TAG]                       a .pbix, .pbit, .pbip (or project folder), .bim
//! bicraft-desktop --server URL --workspace ID [--lang TAG]  a BIcraft server (token: BICRAFT_SERVER_TOKEN)
//! bicraft-desktop --register | --unregister                 Windows: per-user Start menu entry and Open with
//! bicraft-desktop [FILE] --control PORT [--control-file F]  the UI control channel (docs/desktop.md)
//! bicraft-desktop [FILE] --render OUT.png [--size WxH] [--scale N] [--set JSON]
//!                                                           the window drawn with no window
//! bicraft-desktop --help | --version
//! ```
//!
//! A Windows GUI program (no console window): `--register`, `--help` and `--version` print to
//! standard output when it is redirected (`bicraft-desktop --register | more`) and answer with
//! their exit code.
#![cfg_attr(windows, windows_subsystem = "windows")]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, OnceLock};

use bicraft_ui::source::Server;

const USAGE: &str = "\
Usage: bicraft-desktop [FILE] [--lang TAG]
       bicraft-desktop --server URL --workspace ID [--lang TAG]
       bicraft-desktop [FILE] --render OUT.png [--size WxH] [--scale N] [--set JSON]
       bicraft-desktop --register | --unregister

  FILE              a .pbix, .pbit, .pbip (or a project folder) or .bim to open
  --lang TAG        the UI language (en, fr…; default: the system's), also the language of the
                    text Power BI generates in visuals (auto titles, (Blank), Total)
  --server URL      a BIcraft server (bicraft serve); the token comes from BICRAFT_SERVER_TOKEN
  --workspace ID    the workspace on that server
  --control PORT    the UI control channel for agents on 127.0.0.1:PORT (0: a free port; also
                    BICRAFT_CONTROL_PORT), its token in an owner-only file (docs/desktop.md)
  --control-file F  that file (also BICRAFT_CONTROL_FILE; default: control.json next to the
                    settings file)
  --render OUT.png  draws the window with no window, no GPU and no display, once the file is
                    open, its data computed and its page drawn, then quits (docs/desktop.md)
  --size WxH        the window's size in points for --render (default 1366x860)
  --scale N         pixels per point for --render (default 1)
  --set JSON        what to show first for --render: the control channel's ui.set params
                    ('{\"page\": \"Overview\", \"theme\": \"dark\", \"view\": \"model\"}')
  --register        Windows: Start menu shortcut and Open with entries, for this user only
  --unregister      Windows: removes them
  --help, -h        this help
  --version, -V     the version
";

#[derive(Debug, PartialEq)]
enum Startup {
    Help,
    Version,
    Register,
    Unregister,
    Run {
        file: Option<PathBuf>,
        server: Option<(String, String)>,
        lang: Option<String>,
        /// The control channel's port (`--control`) and token file.
        control: Option<u16>,
        control_file: Option<PathBuf>,
    },
    /// `--render OUT`: the window drawn offscreen.
    Render {
        file: Option<PathBuf>,
        lang: Option<String>,
        out: PathBuf,
        opts: bicraft_desktop::headless::Options,
    },
}

fn startup(args: impl IntoIterator<Item = String>) -> Result<Startup, String> {
    let mut file = None;
    let mut url = None;
    let mut workspace = None;
    let mut lang = None;
    let mut control = None;
    let mut control_file = None;
    let mut render = None;
    let mut opts = bicraft_desktop::headless::Options::default();
    let mut render_opt = false;
    let mut args = args.into_iter();
    while let Some(a) = args.next() {
        let mut value = |flag: &str| args.next().ok_or_else(|| format!("{flag} needs a value"));
        match a.as_str() {
            "--help" | "-h" => return Ok(Startup::Help),
            "--version" | "-V" => return Ok(Startup::Version),
            "--register" => return Ok(Startup::Register),
            "--unregister" => return Ok(Startup::Unregister),
            "--lang" => lang = Some(value("--lang")?),
            "--server" => url = Some(value("--server")?),
            "--workspace" => workspace = Some(value("--workspace")?),
            "--control" => control = Some(port(&value("--control")?)?),
            "--control-file" => control_file = Some(PathBuf::from(value("--control-file")?)),
            "--render" => render = Some(PathBuf::from(value("--render")?)),
            "--size" => {
                opts.size = bicraft_desktop::headless::size(&value("--size")?)?;
                render_opt = true;
            }
            "--scale" => {
                let v = value("--scale")?;
                opts.scale = v
                    .trim()
                    .parse()
                    .map_err(|_| format!("--scale needs a number (1, 2), not `{v}`"))?;
                render_opt = true;
            }
            "--set" => {
                let v = value("--set")?;
                let set: serde_json::Value =
                    serde_json::from_str(&v).map_err(|e| format!("--set needs a JSON object: {e}"))?;
                if !set.is_object() {
                    return Err("--set needs a JSON object (ui.set's params)".into());
                }
                opts.set = Some(set);
                render_opt = true;
            }
            s if s.starts_with("--") => return Err(format!("unknown option {s}")),
            _ => {
                if file.replace(PathBuf::from(&a)).is_some() {
                    return Err("bicraft-desktop opens one FILE".into());
                }
            }
        }
    }
    let server = match (url, workspace) {
        (Some(u), Some(w)) => Some((u, w)),
        (None, None) => None,
        _ => return Err("--server and --workspace go together".into()),
    };
    if server.is_some() && file.is_some() {
        return Err("give a FILE or a --server, not both".into());
    }
    if let Some(out) = render {
        if server.is_some() || control.is_some() || control_file.is_some() {
            return Err("--render draws a FILE: no --server or --control".into());
        }
        return Ok(Startup::Render {
            file,
            lang,
            out,
            opts,
        });
    }
    if render_opt {
        return Err("--size, --scale and --set go with --render".into());
    }
    Ok(Startup::Run {
        file,
        server,
        lang,
        control,
        control_file,
    })
}

fn port(s: &str) -> Result<u16, String> {
    s.trim()
        .parse()
        .map_err(|_| format!("--control needs a port number (0-65535), not `{s}`"))
}

fn main() -> ExitCode {
    let start = match startup(std::env::args().skip(1)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("bicraft-desktop: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let (file, server, lang, control, control_file) = match start {
        Startup::Help => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Startup::Version => {
            println!("bicraft-desktop {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Startup::Register => {
            let exe = std::env::current_exe().and_then(std::path::absolute);
            let r = exe
                .map_err(|e| e.to_string())
                .and_then(|exe| bicraft_desktop::register::register(&exe));
            return report(r, "registered");
        }
        Startup::Unregister => return report(bicraft_desktop::register::unregister(), "removed"),
        Startup::Render {
            file,
            lang,
            out,
            opts,
        } => return render(file, lang, &out, &opts),
        Startup::Run {
            file,
            server,
            lang,
            control,
            control_file,
        } => (file, server, lang, control, control_file),
    };
    // the control channel: the flag, else BICRAFT_CONTROL_PORT
    let control = match control.map(Ok).or_else(|| {
        bicraft_env::var("BICRAFT_CONTROL_PORT")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(|s| port(&s))
    }) {
        None => None,
        Some(Ok(p)) => Some(p),
        Some(Err(e)) => {
            eprintln!("bicraft-desktop: BICRAFT_CONTROL_PORT: {e}");
            return ExitCode::from(2);
        }
    };
    let control_file = control_file
        .or_else(|| bicraft_env::var_os("BICRAFT_CONTROL_FILE").map(PathBuf::from))
        .or_else(bicraft_desktop::control::default_file);
    let lang = lang
        .or_else(bicraft_desktop::os_tag)
        .unwrap_or_else(|| "en-US".into());
    // M's local clock (`DateTime.LocalNow()`): the machine's time zone, read at each
    // evaluation, as in Desktop (docs/formats/data-sources.md, "Clock")
    bicraft_engine::set_local_clock(bicraft_engine::LocalClock::Zone(std::sync::Arc::new(
        bicraft_locale::calendar::machine_offset_minutes,
    )));
    let mut app = bicraft_desktop::app(&lang, bicraft_desktop::prefs::default_path());
    if let Some(f) = file {
        app.open_path(&f);
    } else if let Some((url, workspace)) = server {
        app.open_server(Server {
            url,
            workspace,
            token: bicraft_env::var("BICRAFT_SERVER_TOKEN").unwrap_or_default(),
        });
    }
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("BIcraft")
        .with_app_id(bicraft_desktop::register::APP_ID)
        .with_inner_size([1366.0, 860.0])
        .with_min_inner_size([640.0, 420.0])
        .with_drag_and_drop(true);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(bicraft_desktop::ICON_PNG) {
        viewport = viewport.with_icon(icon);
    }
    let mut options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    graphics(&mut options);
    // the control channel listens before the window opens; requests wait for its first frame
    let window: Arc<OnceLock<egui::Context>> = Arc::new(OnceLock::new());
    let _control = match control {
        None => None,
        Some(p) => match start_control(&mut app, p, control_file.as_deref(), &window) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("bicraft-desktop: control channel: {e}");
                return ExitCode::FAILURE;
            }
        },
    };
    let r = eframe::run_native(
        "BIcraft",
        options,
        Box::new(move |cc| {
            let _ = window.set(cc.egui_ctx.clone());
            Ok(Box::new(App(app)))
        }),
    );
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("bicraft-desktop: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Windows: DirectX 12 only, with the FXC shader compiler that ships with Windows, unless
/// `WGPU_BACKEND` / `WGPU_DX12_COMPILER` say otherwise (GL drivers and a stray dxcompiler.dll can
/// fail before the first frame; DeckCraft hit both).
fn graphics(options: &mut eframe::NativeOptions) {
    #[cfg(windows)]
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {
        use eframe::wgpu::{Backends, Dx12Compiler};
        create.instance_descriptor.backends = Backends::from_env().unwrap_or(Backends::DX12);
        create.instance_descriptor.backend_options.dx12.shader_compiler =
            Dx12Compiler::from_env().unwrap_or(Dx12Compiler::Fxc);
    }
    #[cfg(not(windows))]
    let _ = options;
}

/// Starts the control channel on `port`, attached to the app, its token written to `file`; the
/// address and the file go to stderr (never the token).
fn start_control(
    app: &mut bicraft_ui::App,
    port: u16,
    file: Option<&std::path::Path>,
    window: &Arc<OnceLock<egui::Context>>,
) -> Result<bicraft_desktop::control::Server, String> {
    use bicraft_desktop::control;
    let file = file.ok_or("no place for the token file: give --control-file")?;
    let (tx, rx) = std::sync::mpsc::sync_channel(bicraft_ui::control::MAX_QUEUED);
    let w = Arc::clone(window);
    let notify: control::Notify = Arc::new(move || {
        if let Some(ctx) = w.get() {
            ctx.request_repaint();
        }
    });
    let server = control::start(port, tx, notify, control::Limits::default())
        .map_err(|e| format!("cannot listen on 127.0.0.1:{port}: {e}"))?;
    server
        .write_file(file)
        .map_err(|e| format!("{}: {e}", file.display()))?;
    app.control.attach(rx);
    eprintln!(
        "bicraft-desktop: control channel on {} (token in {})",
        server.addr(),
        file.display()
    );
    Ok(server)
}

/// `--render`: the app with no settings file and no recovery journal, its pages drawn with the
/// bundled fonts on the window's thread (the same pixels on every machine), drawn offscreen.
fn render(
    file: Option<PathBuf>,
    lang: Option<String>,
    out: &std::path::Path,
    opts: &bicraft_desktop::headless::Options,
) -> ExitCode {
    let lang = lang.unwrap_or_else(|| "en-US".into());
    let mut app = bicraft_desktop::app_with(&lang, None, None).with_sync_render();
    if let Some(f) = &file {
        app.open_path(f);
    }
    let r = bicraft_desktop::headless::render(&mut app, opts).and_then(|(png, [w, h])| {
        std::fs::write(out, &png).map_err(|e| format!("{}: {e}", out.display()))?;
        Ok(format!("{} ({w}×{h} pixels, {} bytes)", out.display(), png.len()))
    });
    match r {
        Ok(done) => {
            println!("rendered: {done}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("bicraft-desktop: --render: {e}");
            ExitCode::FAILURE
        }
    }
}

fn report(r: Result<Vec<String>, String>, verb: &str) -> ExitCode {
    match r {
        Ok(done) => {
            if done.is_empty() {
                println!("bicraft-desktop: nothing to do");
            }
            for d in done {
                println!("{verb}: {d}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("bicraft-desktop: {e}");
            ExitCode::FAILURE
        }
    }
}

struct App(bicraft_ui::App);

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.0.logic(ctx);
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.0.ui(ui);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Result<Startup, String> {
        startup(a.iter().map(|s| s.to_string()))
    }

    #[test]
    fn arguments() {
        assert_eq!(args(&["-h"]), Ok(Startup::Help));
        assert_eq!(args(&["x.pbix", "--version"]), Ok(Startup::Version));
        assert_eq!(args(&["--register"]), Ok(Startup::Register));
        assert_eq!(
            args(&["x.pbix", "--lang", "fr"]),
            Ok(Startup::Run {
                file: Some(PathBuf::from("x.pbix")),
                server: None,
                lang: Some("fr".into()),
                control: None,
                control_file: None,
            })
        );
        assert!(matches!(
            args(&["x.pbix", "--control", "0", "--control-file", "c.json"]),
            Ok(Startup::Run {
                control: Some(0),
                control_file: Some(_),
                ..
            })
        ));
        assert!(args(&["--control"]).is_err());
        assert!(args(&["--control", "x"]).is_err());
        assert!(args(&["--control", "70000"]).is_err());
        assert!(matches!(
            args(&["--server", "https://h", "--workspace", "a"]),
            Ok(Startup::Run { server: Some(_), .. })
        ));
        assert!(args(&["--server", "https://h"]).is_err());
        assert!(args(&["a.pbix", "b.pbix"]).is_err());
        assert!(args(&["--lang"]).is_err());
        assert!(args(&["--nope"]).is_err());
        assert!(args(&["a.pbix", "--server", "u", "--workspace", "w"]).is_err());
        let r = args(&[
            "a.pbix",
            "--render",
            "o.png",
            "--size",
            "800x600",
            "--scale",
            "2",
            "--set",
            r#"{"page": 1}"#,
        ]);
        let Ok(Startup::Render { out, opts, .. }) = r else {
            panic!("{r:?}")
        };
        assert_eq!(out, PathBuf::from("o.png"));
        assert_eq!((opts.size, opts.scale), ([800.0, 600.0], 2.0));
        assert!(opts.set.is_some());
        assert!(args(&["a.pbix", "--size", "800x600"]).is_err());
        assert!(args(&["a.pbix", "--render", "o.png", "--size", "800"]).is_err());
        assert!(args(&["a.pbix", "--render", "o.png", "--set", "[1]"]).is_err());
        assert!(args(&["a.pbix", "--render", "o.png", "--control", "0"]).is_err());
    }
}
