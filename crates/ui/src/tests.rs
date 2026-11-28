//! UI-level tests of the shared app, independent of the shells' sources: snapshots are handed
//! over by a fixed source (`fixed`) or set by hand. The shells' own tests (the engine source,
//! the server client, files on disk) are in `apps/bicraft-desktop/src/tests.rs`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bicraft_engine::Session;
use egui::{Event, FontId, Modifiers, PointerButton, Pos2, Vec2, pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use crate::App;
use crate::app::Tab;
use crate::diagram;
use crate::shell::{Caps, Shell};
use crate::source::{Driver, Loaded, Request, Running, Update};

/// A driven source that ignores requests.
struct Idle;

impl Driver for Idle {
    fn request(&self, _r: Request) {}
}

/// A shell with `caps` whose every source sends `loaded` once.
fn fixed(caps: Caps, loaded: Option<Loaded>) -> Shell {
    Shell::new(caps, move |_origin, link| {
        if let Some(l) = &loaded {
            link.send(Update::Snapshot(l.clone()));
        }
        Running::Driven(Box::new(Idle))
    })
}

fn loaded(snap: Value) -> Loaded {
    Loaded {
        snap: Arc::new(snap),
        assets: Arc::new(HashMap::new()),
        seq: 1,
    }
}

fn harness(app: App, size: (f32, f32)) -> Harness<'static, App> {
    Harness::builder()
        .with_size(egui::vec2(size.0, size.1))
        .build_ui_state(
            |ui, app: &mut App| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            app,
        )
}

/// A left click at `pos`.
fn click(h: &mut Harness<'_, App>, pos: Pos2) {
    h.hover_at(pos);
    h.step();
    for pressed in [true, false] {
        h.event_modifiers(
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            },
            Modifiers::NONE,
        );
        h.step();
    }
}

/// A window showing a snapshot set by hand (no source).
fn with_snapshot(snap: Value, size: (f32, f32)) -> Harness<'static, App> {
    let mut app = App::new("en-US", fixed(Caps::DESKTOP, None));
    app.loaded = Some(loaded(snap));
    app.tab = Tab::Model;
    let mut h = harness(app, size);
    h.step();
    h.step();
    h
}

/// A report with a slicer, a card and a column chart over a literal table, and its model: the
/// session's snapshot once every visual has its data.
fn shop_snapshot() -> Loaded {
    let mut s = Session::new();
    let sales =
        r#"#table({"Category", "Amount"}, {{"Wheels", 100}, {"Wheels", 50}, {"Toys", 5}, {"Games", 20}})"#;
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
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])", "format_string": "#,0"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Broken", "expression": "SUM(Sales[Nope])", "force": true}),
        ),
        ("report.new", json!({})),
    ] {
        s.execute(id, &p).unwrap();
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    for (ty, rect, fields) in [
        (
            "slicer",
            [20, 20, 220, 260],
            json!([{"role": "Values", "field": "Sales[Category]"}]),
        ),
        (
            "card",
            [260, 20, 240, 120],
            json!([{"role": "Values", "field": "[Total]"}]),
        ),
        (
            "clusteredColumnChart",
            [260, 160, 600, 400],
            json!([{"role": "Category", "field": "Sales[Category]"}, {"role": "Y", "field": "[Total]"}]),
        ),
    ] {
        s.execute(
            "visual.add",
            &json!({"page": page, "visual_type": ty, "x": rect[0], "y": rect[1], "width": rect[2],
                    "height": rect[3], "fields": fields}),
        )
        .unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let snap = s.snapshot();
        s.background_work();
        if !bicraft_engine::watch::data_pending(&snap) {
            let assets = s.snapshot_assets().into_iter().map(|a| (a.id, a.bytes)).collect();
            return Loaded {
                snap: Arc::new(snap),
                assets: Arc::new(assets),
                seq: 1,
            };
        }
        assert!(Instant::now() < deadline, "the visuals never got their data");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The app of a shell with `caps` showing `loaded` (opened as `shop.pbip`), pages rendered on the
/// test thread.
fn shell_app(caps: Caps, loaded: &Loaded, lang: &str) -> Harness<'static, App> {
    let mut app = App::new(lang, fixed(caps, Some(loaded.clone()))).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1280.0, 800.0));
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() {
            break;
        }
    }
    assert!(h.state().shown.is_some(), "the page was never drawn");
    h.step();
    h
}

/// Everything a frame sends to the GPU: the tessellated meshes (positions, texture coordinates,
/// colours, textures, clip rectangles), the font atlas they sample and the page's image. Two
/// frames with the same values draw the same pixels.
fn frame(h: &Harness<'_, App>) -> (String, Vec<u8>, Vec<u8>) {
    let out = h.output();
    let prims = h.ctx.tessellate(out.shapes.clone(), out.pixels_per_point);
    let mut meshes = String::new();
    for p in &prims {
        meshes.push_str(&format!("{:?}\n", p.clip_rect));
        match &p.primitive {
            egui::epaint::Primitive::Mesh(m) => {
                meshes.push_str(&format!("{:?} {:?}\n", m.texture_id, m.indices));
                for v in &m.vertices {
                    meshes.push_str(&format!("{:?} {:?} {:?}\n", v.pos, v.uv, v.color));
                }
            }
            egui::epaint::Primitive::Callback(_) => meshes.push_str("callback\n"),
        }
    }
    let atlas = h.ctx.fonts(|f| f.image());
    let atlas: Vec<u8> = atlas.pixels.iter().flat_map(|c| c.to_array()).collect();
    let page = match &h.state().shown {
        Some((_, Ok(img))) => img.rgba.clone(),
        _ => Vec::new(),
    };
    (meshes, atlas, page)
}

/// The desktop's and the browser's shells draw the same frames: the same fonts, the same layout,
/// the same page, in every tab, in both languages, light and dark. Their only differences are
/// inside the File menu (closed here) and the title setter (the title is the same).
#[test]
fn the_desktop_and_the_browser_draw_the_same_frames() {
    let snap = shop_snapshot();
    for (lang, dark) in [("en-US", false), ("fr-FR", false), ("en-US", true)] {
        let mut desktop = shell_app(Caps::DESKTOP, &snap, lang);
        let mut browser = shell_app(Caps::BROWSER, &snap, lang);
        for tab in [Tab::Report, Tab::Model, Tab::Validation] {
            for h in [&mut desktop, &mut browser] {
                if dark {
                    h.state_mut().set_theme(crate::theme::Brightness::Dark);
                }
                h.state_mut().tab = tab;
                h.step();
                h.step();
                h.step();
            }
            let (a, b) = (frame(&desktop), frame(&browser));
            assert!(!a.0.is_empty() && !a.1.is_empty());
            assert!(a.0 == b.0, "{lang} {tab:?}: the meshes differ");
            assert!(a.1 == b.1, "{lang} {tab:?}: the font atlases differ");
            assert!(a.2 == b.2, "{lang} {tab:?}: the pages differ");
        }
        assert_eq!(desktop.state().title(), browser.state().title());
    }
}

/// The fonts are the bundled Selawik faces, whichever shell runs: their metrics, not egui's
/// default Ubuntu Light; headings are semibold.
#[test]
fn the_ui_text_is_selawik() {
    let mut h = with_snapshot(json!({}), (400.0, 300.0));
    h.step();
    let fd = crate::fonts::definitions();
    assert_eq!(fd.families[&egui::FontFamily::Proportional][0], "Selawik");
    let w = |family: egui::FontFamily| {
        h.ctx
            .fonts_mut(|f| f.glyph_width(&FontId::new(20.0, family), 'W'))
    };
    let (regular, bold) = (w(egui::FontFamily::Proportional), w(crate::fonts::bold()));
    assert!(regular > 0.0 && bold > regular, "{regular} {bold}");
    let heading = h.ctx.style_of(egui::Theme::Light).text_styles[&egui::TextStyle::Heading]
        .family
        .clone();
    assert_eq!(heading, crate::fonts::semibold());
}

/// Every character the UI's own strings use has a glyph in the app's fonts (no missing-glyph box
/// in either shell): the arrows, `▶`, `●`, `Σ`, `ƒ`, `✔`, `×`…
#[test]
fn every_character_the_ui_uses_has_a_glyph() {
    let sources = [
        include_str!("app.rs"),
        include_str!("backstage.rs"),
        include_str!("page_tabs.rs"),
        include_str!("actions.rs"),
        include_str!("model_view.rs"),
        include_str!("diagram.rs"),
        include_str!("i18n.rs"),
        include_str!("pages.rs"),
        include_str!("registry.rs"),
        include_str!("registry_table.rs"),
        include_str!("ribbon.rs"),
        include_str!("menus.rs"),
        include_str!("form.rs"),
        include_str!("palette.rs"),
        include_str!("panes.rs"),
        include_str!("answer.rs"),
    ];
    let mut chars: Vec<char> = sources
        .iter()
        .flat_map(|s| s.lines())
        .filter(|l| !l.trim_start().starts_with("//"))
        .flat_map(str::chars)
        .filter(|c| !c.is_ascii())
        .collect();
    chars.sort_unstable();
    chars.dedup();
    assert!(chars.contains(&'→') && chars.contains(&'Σ'), "{chars:?}");
    // the cmaps of each family's faces (egui's own `has_glyph` reports the face holding its
    // replacement glyph, Hack, as missing)
    let fd = crate::fonts::definitions();
    for family in [
        egui::FontFamily::Proportional,
        egui::FontFamily::Monospace,
        crate::fonts::semibold(),
        crate::fonts::bold(),
    ] {
        let faces: Vec<skrifa::charmap::Charmap<'_>> = fd.families[&family]
            .iter()
            .map(|k| {
                skrifa::MetadataProvider::charmap(
                    &skrifa::FontRef::new((*fd.font_data[k].font).as_ref()).unwrap(),
                )
            })
            .collect();
        let missing: Vec<char> = chars
            .iter()
            .copied()
            .filter(|c| !faces.iter().any(|f| f.map(*c).is_some()))
            .collect();
        assert!(missing.is_empty(), "{family:?} lacks {missing:?}");
    }
}

/// The browser's Home ribbon has Reload only: no Open, recent files, Connect or Quit, and Ctrl+O
/// does nothing; the desktop's has them all.
#[test]
fn the_home_ribbon_follows_the_shells_capabilities() {
    let snap = loaded(json!({"model": {"name": "M", "tables": []}}));
    for caps in [Caps::DESKTOP, Caps::BROWSER] {
        let picked = Rc::new(RefCell::new(0usize));
        let mut shell = fixed(caps, Some(snap.clone()));
        let p = Rc::clone(&picked);
        shell.pick_file = Some(Box::new(move |_| {
            *p.borrow_mut() += 1;
            None
        }));
        let mut app = App::new("en-US", shell);
        app.open_path(Path::new("m.bim"));
        let mut h = harness(app, (1100.0, 600.0));
        h.step();
        h.step();
        h.get_by_label("Home").click();
        h.step();
        h.step();
        assert_eq!(h.state().ribbon_tab, crate::app::RibbonTab::Home);
        h.get_by_label("Reload");
        for item in [
            "Open…",
            "Connect to server…",
            "Recent files",
            "Recent servers",
            "Quit",
        ] {
            assert_eq!(
                h.query_by_label(item).is_some(),
                caps == Caps::DESKTOP,
                "{item} with {caps:?}"
            );
        }
        h.key_press_modifiers(Modifiers::COMMAND, egui::Key::O);
        h.step();
        assert_eq!(*picked.borrow(), usize::from(caps == Caps::DESKTOP), "{caps:?}");
    }
}

/// A store kept in memory (tests).
#[derive(Clone, Default)]
pub(super) struct MemStore(pub(super) Rc<RefCell<Value>>);

impl crate::shell::PrefsStore for MemStore {
    fn load(&self) -> Value {
        self.0.borrow().clone()
    }
    fn save(&self, v: &Value) -> Result<(), String> {
        *self.0.borrow_mut() = v.clone();
        Ok(())
    }
}

/// A store whose every call fails (blocked storage).
struct Blocked;

impl crate::shell::PrefsStore for Blocked {
    fn load(&self) -> Value {
        Value::Null
    }
    fn save(&self, _v: &Value) -> Result<(), String> {
        Err("storage is blocked".into())
    }
}

/// Light by default; the title row's toggle switches to dark, and a new window in the same
/// store starts dark; the ribbon's collapsed state and the tab are kept too. A blocked store
/// gives the defaults.
#[test]
fn light_by_default_and_the_dark_toggle_is_kept() {
    use crate::theme::{Brightness, Tokens};
    let store = MemStore::default();
    let mut shell = fixed(Caps::BROWSER, None);
    shell.prefs = Some(Box::new(store.clone()));
    let mut h = harness(App::new("en-US", shell), (1000.0, 600.0));
    h.step();
    h.step();
    assert_eq!(h.state().prefs.theme, Brightness::Light);
    assert!(!h.ctx.global_style().visuals.dark_mode);
    assert_eq!(Tokens::get(&h.ctx), Tokens::light());
    h.get_by_label("Dark mode").click();
    h.step();
    h.step();
    assert_eq!(h.state().prefs.theme, Brightness::Dark);
    assert!(h.ctx.global_style().visuals.dark_mode);
    assert_eq!(h.ctx.global_style().visuals.panel_fill, Tokens::dark().chrome);
    h.get_by_label("Model").click();
    h.step();
    h.get_by_label("Model").click(); // the active tab collapses the ribbon
    h.step();
    h.step();
    assert!(h.state().prefs.ribbon_collapsed);
    assert_eq!(store.0.borrow()["theme"], "dark");

    let mut shell = fixed(Caps::BROWSER, None);
    shell.prefs = Some(Box::new(store.clone()));
    let mut h = harness(App::new("en-US", shell), (1000.0, 600.0));
    h.step();
    h.step();
    h.step();
    assert!(h.ctx.global_style().visuals.dark_mode);
    assert_eq!(h.state().tab, Tab::Model);
    assert!(h.state().prefs.ribbon_collapsed);
    h.get_by_label("Light mode");

    let mut shell = fixed(Caps::BROWSER, None);
    shell.prefs = Some(Box::new(Blocked));
    let mut h = harness(App::new("en-US", shell), (1000.0, 600.0));
    h.step();
    h.step();
    h.get_by_label("Dark mode").click();
    h.step();
    h.step();
    assert!(
        h.ctx.global_style().visuals.dark_mode,
        "a blocked store still toggles"
    );
}

/// The chrome: the title row's file and pills, the tabs, the status bar's page, role-free local
/// file, the last command.
#[test]
fn the_chrome_shows_the_file_and_its_state() {
    let snap = loaded(
        json!({"revision": 7, "source": {"path": "C:/data/Sales.pbix", "format": "pbix"},
        "dirty": {"model": true, "report": false}, "model": {"tables": []},
        "last_command": {"id": "measure.add", "ok": false, "error": "x", "at_ms": 1}}),
    );
    let mut app = App::new("en-US", fixed(Caps::BROWSER, Some(snap)));
    app.open_path(Path::new("Sales.pbix"));
    let mut h = harness(app, (1200.0, 700.0));
    h.step();
    h.step();
    // the title row and the document tab
    assert_eq!(h.query_all_by_label("Sales.pbix").count(), 2);
    h.get_by_label("PBIX");
    h.get_by_label("rev 7");
    h.get_by_label("model");
    h.get_by_label("✕ measure.add");
    h.get_by_label("live");
    for tab in ["Home", "Report", "Model", "Data", "Analyze", "View"] {
        h.get_by_label(tab);
    }
}

/// `✓` and `✕` (the viewer's marks) come from the bundled symbol face, in every family.
#[test]
fn check_and_cross_are_the_symbol_faces() {
    let fd = crate::fonts::definitions();
    let face = skrifa::MetadataProvider::charmap(&skrifa::FontRef::new(crate::fonts::SYMBOLS_TTF).unwrap());
    for c in ['✓', '✕', '✖', '⚠', '●', '▶'] {
        assert!(face.map(c).is_some(), "the subset lacks {c}");
    }
    for family in [
        egui::FontFamily::Proportional,
        crate::fonts::semibold(),
        crate::fonts::bold(),
    ] {
        let faces = &fd.families[&family];
        let first = faces
            .iter()
            .find(|k| {
                skrifa::FontRef::new((*fd.font_data[*k].font).as_ref())
                    .is_ok_and(|f| skrifa::MetadataProvider::charmap(&f).map('✓').is_some())
            })
            .unwrap();
        assert_eq!(first, "NotoSansSymbols2-subset", "{family:?}");
    }
}

/// The pages' fonts are the bundled faces in every shell (the render thread too), so a page the
/// desktop draws equals the browser's: no system face is in the book.
#[test]
fn pages_use_the_bundled_fonts_everywhere() {
    let book = crate::pages::font_book();
    for f in [
        "arial",
        "segoe ui symbol",
        "calibri",
        "dejavu sans",
        "liberation sans",
    ] {
        assert!(!book.has_family(f), "{f} comes from the system");
    }
    let snap = shop_snapshot();
    let key = crate::pages::Key::new(snap.seq, 0, 1.0);
    let sync = crate::pages::render(&snap.snap, &snap.assets, &crate::pages::font_book(), key).unwrap();
    // the render thread (what the shells run)
    let mut pages = crate::pages::Pages::threaded(Arc::new(|| {}));
    let deadline = Instant::now() + Duration::from_secs(60);
    let threaded = loop {
        if let Some((k, Ok(img))) = pages.get(&snap, key)
            && k == key
        {
            break img;
        }
        assert!(Instant::now() < deadline, "the render thread never answered");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!((threaded.width, threaded.height), (sync.width, sync.height));
    assert!(
        threaded.rgba == sync.rgba,
        "the render thread draws what the browser draws"
    );
}

/// The window's pages draw subpixel text below the high-DPI threshold only, and it moves no
/// text or hit region; above it they are the grayscale pages `bicraft-cli render` draws.
#[test]
fn pages_draw_subpixel_text_below_high_dpi_only() {
    use crate::pages::{Key, LCD_TEXT_BELOW, font_book, lcd_text, render};
    assert!(lcd_text(0.6) && lcd_text(1.0) && lcd_text(1.5));
    assert!(!lcd_text(LCD_TEXT_BELOW) && !lcd_text(2.0) && !lcd_text(f32::NAN));
    let snap = shop_snapshot();
    let fonts = font_book();
    let plain = |scale: f32| {
        let opts = bicraft_render::RenderOptions {
            scale,
            hits: true,
            raw: true,
            system_fonts: false,
            ..bicraft_render::RenderOptions::default()
        };
        bicraft_render::render_page(&snap.snap, 0, &*snap.assets, &fonts, &opts).unwrap()
    };
    for scale in [0.6f32, 1.0, 1.5, 2.0] {
        let page = render(&snap.snap, &snap.assets, &fonts, Key::new(snap.seq, 0, scale)).unwrap();
        let gray = plain(scale);
        assert_eq!(page.hits, gray.hits, "scale {scale}: hit map");
        assert_eq!(
            page.rgba == gray.rgba,
            scale >= LCD_TEXT_BELOW,
            "scale {scale}: subpixel text"
        );
    }
}

/// The browser shell sets the title through its own setter (`document.title`), like the viewer's
/// page: `● file · rev N`.
#[test]
fn the_title_is_the_viewers() {
    let titles = Rc::new(RefCell::new(Vec::<String>::new()));
    let snap = loaded(json!({"revision": 7, "source": {"path": "C:\\data\\Sales.pbix"},
        "dirty": {"model": true, "report": false}, "model": {"tables": []}}));
    let mut shell = fixed(Caps::BROWSER, Some(snap));
    let t = Rc::clone(&titles);
    shell.set_title = Some(Box::new(move |_, title| t.borrow_mut().push(title.to_string())));
    let mut app = App::new("en-US", shell);
    assert_eq!(app.window_title(), "BIcraft");
    app.open_path(Path::new("Sales.pbix"));
    let mut h = harness(app, (800.0, 600.0));
    h.step();
    h.step();
    assert_eq!(h.state().title(), "● Sales.pbix · rev 7");
    assert_eq!(
        titles.borrow().last().map(String::as_str),
        Some("● Sales.pbix · rev 7")
    );
}

/// A snapshot without a report (a `.bim`): no error, no page list, the viewer's empty state in
/// the Report tab; the Model tab draws the model.
#[test]
fn a_model_only_snapshot_has_a_neutral_report_tab() {
    let snap = loaded(json!({"report": null, "model": {"name": "M", "tables": [
        {"name": "T", "columns": [{"name": "a", "data_type": "int64"}], "measures": []}], "relationships": []}}));
    let mut app = App::new("fr-FR", fixed(Caps::BROWSER, Some(snap)));
    app.open_path(Path::new("m.bim"));
    let mut h = harness(app, (900.0, 600.0));
    h.step();
    h.step();
    h.get_by_label("Pas de rapport dans ce fichier");
    assert!(h.state().error.is_none());
    assert!(h.state().page_names().is_empty());
    h.state_mut().tab = Tab::Model;
    h.step();
    h.step();
    assert!(h.state().model_view.diagram().is_some());
}

#[test]
fn huge_and_damaged_models_never_panic() {
    // 400 tables (300 drawn), 5,000 measures, 2,000 relationships
    let tables: Vec<Value> = (0..400)
        .map(|i| {
            let measures: Vec<Value> = (0..if i < 100 { 50 } else { 0 })
                .map(|k| json!({"name": format!("M{k}"), "expression": format!("SUM(T{i}[a]) * {k}"), "valid": k % 7 != 0}))
                .collect();
            json!({"name": format!("T{i}"), "hidden": i % 9 == 0, "columns": [
                {"name": "k", "data_type": "int64"}, {"name": "a", "data_type": "double", "calculated": true},
                {"name": "b", "data_type": "string", "hidden": true}], "measures": measures})
        })
        .collect();
    let rels: Vec<Value> = (1..2001)
        .map(|i| {
            json!({"from_table": format!("T{}", i % 400), "from_column": "a", "to_table": format!("T{}", (i * 7) % 400),
                   "to_column": "k", "active": i % 3 != 0, "cross_filter": if i % 5 == 0 { "both" } else { "single" },
                   "from_cardinality": "many", "to_cardinality": "one"})
        })
        .collect();
    let n_meas: usize = tables
        .iter()
        .map(|t| t["measures"].as_array().map_or(0, Vec::len))
        .sum();
    assert_eq!(n_meas, 5000);
    let mut h = with_snapshot(
        json!({"model": {"name": "Huge", "tables": tables, "relationships": rels, "truncated": true}}),
        (1400.0, 900.0),
    );
    let d = h.state().model_view.diagram().unwrap();
    assert_eq!(d.cards.len(), diagram::MAX_TABLES);
    assert!(d.links.len() <= diagram::MAX_RELS);
    h.get_by_label("drawing the first 300 tables");
    // zoom all the way in and out, sweep the pointer, fit, click
    let c = h.state().model_view.canvas.center();
    for z in [8.0, 8.0, 8.0, 0.1, 0.1, 0.1, 0.1, 2.0] {
        h.hover_at(c);
        h.event(Event::Zoom(z));
        h.step();
        let zoom = h.state().model_view.zoom;
        assert!((crate::model_view::MIN_ZOOM..=crate::model_view::MAX_ZOOM).contains(&zoom));
    }
    for i in 0..20 {
        h.hover_at(c + Vec2::new(i as f32 * 25.0 - 250.0, i as f32 * 15.0 - 150.0));
        h.step();
    }
    h.get_by_label("Fit").click();
    h.step();
    click(&mut h, c);
    h.step();

    // damaged snapshots: shown as a message or an empty diagram, never a panic
    for snap in [
        json!({}),
        json!({"model": null}),
        json!({"model": {"tables": "x", "relationships": {}}}),
        json!({"model": {"tables": [null, 3, {"name": null, "columns": [1, {"name": 2}], "measures": "m"}],
                         "relationships": [{"from_table": 1}, null]}}),
        json!({"model": {"name": "Empty", "tables": [], "relationships": []}}),
    ] {
        let mut h = with_snapshot(snap, (800.0, 600.0));
        click(&mut h, pos2(400.0, 300.0));
        h.step();
    }
}

#[test]
fn show_all_expands_a_card() {
    let mut cols: Vec<Value> = (0..30)
        .map(|i| json!({"name": format!("C{i}"), "data_type": "int64"}))
        .collect();
    cols.push(json!({"name": "K", "data_type": "string"}));
    let m = json!({"name": "Wide", "tables": [{"name": "Wide", "columns": cols, "measures": []}], "relationships": []});
    let mut h = with_snapshot(json!({"model": m}), (1000.0, 1200.0));
    let more = {
        let d = h.state().model_view.diagram().unwrap();
        assert_eq!(d.cards[0].columns.len(), diagram::ROWS_COLLAPSED);
        d.cards[0].more.unwrap()
    };
    let p = h.state().model_view.screen_pos(more.center());
    click(&mut h, p);
    h.step();
    let d = h.state().model_view.diagram().unwrap();
    assert_eq!(d.cards[0].columns.len(), 31);
    assert!(h.state().model_view.expanded.contains("Wide"));
}

/// An open error names its file once: a message that already has the path is not prefixed again.
#[test]
fn open_errors_name_the_file_once() {
    let p = "C:/data/shop.pbix";
    assert_eq!(
        crate::app::with_path(p, "cannot read"),
        "C:/data/shop.pbix: cannot read"
    );
    assert_eq!(
        crate::app::with_path(p, "C:/data/shop.pbix: not a ZIP file"),
        "C:/data/shop.pbix: not a ZIP file"
    );
    assert_eq!(
        crate::app::with_path(p, "cannot read C:/data/shop.pbix/Report: denied"),
        "cannot read C:/data/shop.pbix/Report: denied"
    );
}

mod analytics_ui_tests;
mod analyze_ui_tests;
mod answer_ui_tests;
mod backstage_tests;
mod bookmarks_ui_tests;
mod buttons_ui_tests;
mod canvas_ui_tests;
mod control_ui_tests;
mod custom_visuals_ui_tests;
mod data_view_tests;
mod dax_query_ui_tests;
mod decomp_ui_tests;
mod drill_ui_tests;
mod export_tests;
mod filter_order_ui_tests;
mod filter_pane_format_ui_tests;
mod filters_ui_tests;
mod focus_ui_tests;
mod format_ui_tests;
mod formula_ui_tests;
mod gantt_ui_tests;
mod get_data_ui_tests;
mod group_ui_tests;
mod interactions_ui_tests;
mod keyboard_ui_tests;
mod maps_ui_tests;
mod matrix_ui_tests;
mod mobile_ui_tests;
mod model_explorer_ui_tests;
mod model_sec_ui_tests;
mod navigator_ui_tests;
mod optimize_ui_tests;
mod own_visuals_ui_tests;
mod palette_ui_tests;
mod panes_ui_tests;
mod param_ui_tests;
mod perf_ui_tests;
mod pq_editor_ui_tests;
mod pq_folding_ui_tests;
mod pq_tabs_ui_tests;
mod quick_ui_tests;
mod reader_filters_ui_tests;
mod recovery_tests;
mod refresh_policy_ui_tests;
mod refresh_ui_tests;
mod relationships_ui_tests;
mod report_settings_ui_tests;
mod ribbon_tests;
mod rich_ui_tests;
mod roles_ui_tests;
mod selection_ui_tests;
mod sharepoint_ui_tests;
mod speed_ui_tests;
mod storage_ui_tests;
mod sync_ui_tests;
mod themes_ui_tests;
mod tmdl_view_ui_tests;
mod view_as_ui_tests;
mod viscalc_ui_tests;
