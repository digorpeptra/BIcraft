//! Custom visuals in the Visualizations pane (docs/formats/pbiviz.md): the gallery's cells of the
//! report's own custom visuals with the icon of their package, and its "…" menu, as in Desktop:
//! Import a visual from a file (`visual.import_custom`, the file read by the shell: the desktop's
//! dialog, the browser's file upload) and Remove a custom visual (`visual.remove_custom`, in a
//! window that lists them). BIcraft never runs a custom visual's code: the page draws it as a
//! placeholder with its icon and name (`bicraft_render`).

use std::collections::HashMap;

use egui::{TextureHandle, TextureId, vec2};
use serde_json::{Value, json};

use crate::App;
use crate::actions::Call;
use crate::app::Then;
use crate::i18n::Lang;
use crate::shell::{Picked, UploadSlot};
use crate::theme::Tokens;

pub const IMPORT: &str = "visual.import_custom";
pub const REMOVE: &str = "visual.remove_custom";
/// Largest `.pbiviz` sent to the engine (it reads up to 32 MB).
pub const MAX_PBIVIZ_BYTES: usize = 32 << 20;
/// Largest icon decoded for the gallery (pixels a side; a package's icon is 20×20).
const MAX_ICON_SIDE: u32 = 256;

/// The custom visuals' UI state of a document.
#[derive(Default)]
pub struct CvUi {
    /// An upload the shell is doing (Import a visual from a file).
    pending: Option<UploadSlot>,
    /// The Remove a custom visual window, while it is open.
    pub remove: Option<RemoveDialog>,
    /// The gallery's icon textures by asset id (`None`: the bytes do not decode).
    icons: HashMap<String, Option<TextureHandle>>,
}

impl CvUi {
    /// Whether the icon of asset `id` is decoded as a texture (the gallery drew it).
    pub fn has_icon(&self, id: &str) -> bool {
        self.icons.get(id).is_some_and(Option::is_some)
    }
}

impl std::fmt::Debug for CvUi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CvUi")
            .field("pending", &self.pending.is_some())
            .field("remove", &self.remove)
            .field("icons", &self.icons.len())
            .finish()
    }
}

/// The Remove a custom visual window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoveDialog {
    /// The visual whose removal waits for a confirmation (visuals of the report use it).
    pub confirm: Option<String>,
}

/// An embedded custom visual of the snapshot (`report.custom_visuals`), or one of BIcraft's
/// visuals (`report.bicraft_visuals`).
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// What the gallery's click sends as `visual_type`: a custom visual's guid, a BIcraft
    /// visual's name.
    pub name: String,
    /// The visual type its visuals carry (the guid).
    pub visual_type: String,
    /// Its display name, else its guid.
    pub label: String,
    /// Its icon's asset id.
    pub icon: Option<String>,
    /// One of BIcraft's visuals (listed whether the report embeds it or not).
    pub bicraft: bool,
}

/// The report's embedded custom visuals, as the snapshot lists them.
pub fn entries(app: &App) -> Vec<Entry> {
    let Some(l) = app.loaded.as_ref() else {
        return Vec::new();
    };
    l.snap
        .pointer("/report/custom_visuals")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .take(200)
        .filter_map(|c| {
            let name = c.get("name").and_then(Value::as_str)?.to_string();
            let label = c
                .get("display_name")
                .and_then(Value::as_str)
                .filter(|d| !d.trim().is_empty())
                .map_or_else(|| name.clone(), str::to_string);
            let icon = c.pointer("/icon/id").and_then(Value::as_str).map(str::to_string);
            Some(Entry {
                visual_type: name.clone(),
                name,
                label,
                icon,
                bicraft: false,
            })
        })
        .collect()
}

/// The Visualizations pane's cells after the built-in types: BIcraft's visuals (embedded or
/// not, built in by default), then the report's other custom visuals.
pub fn gallery_entries(app: &App) -> Vec<Entry> {
    let mut out: Vec<Entry> = app
        .loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/report/bicraft_visuals"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .take(200)
        .filter_map(|c| {
            let name = c.get("name").and_then(Value::as_str)?.to_string();
            let visual_type = c.get("guid").and_then(Value::as_str)?.to_string();
            let label = c
                .get("display_name")
                .and_then(Value::as_str)
                .filter(|d| !d.trim().is_empty())
                .map_or_else(|| name.clone(), str::to_string);
            let icon = c.pointer("/icon/id").and_then(Value::as_str).map(str::to_string);
            Some(Entry {
                name,
                visual_type,
                label,
                icon,
                bicraft: true,
            })
        })
        .collect();
    let others: Vec<Entry> = entries(app)
        .into_iter()
        .filter(|e| {
            !out.iter()
                .any(|b| b.visual_type.eq_ignore_ascii_case(&e.visual_type))
        })
        .collect();
    out.extend(others);
    out
}

/// How many visuals of the report are of type `name`.
fn users(app: &App, name: &str) -> usize {
    let Some(l) = app.loaded.as_ref() else { return 0 };
    l.snap
        .pointer("/report/pages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|p| p.get("visuals").and_then(Value::as_array).into_iter().flatten())
        .filter(|v| {
            v.get("type")
                .and_then(Value::as_str)
                .is_some_and(|t| t.eq_ignore_ascii_case(name))
        })
        .count()
}

/// The texture of a gallery icon (asset `id`), decoded once.
pub fn icon(app: &mut App, ctx: &egui::Context, id: &str) -> Option<TextureId> {
    if let Some(t) = app.cmd.custom_visuals.icons.get(id) {
        return t.as_ref().map(TextureHandle::id);
    }
    let bytes = app.loaded.as_ref().and_then(|l| l.assets.get(id).cloned())?;
    let tex = tiny_skia::Pixmap::decode_png(&bytes)
        .ok()
        .filter(|pm| pm.width() <= MAX_ICON_SIDE && pm.height() <= MAX_ICON_SIDE)
        .map(|pm| {
            let size = [pm.width() as usize, pm.height() as usize];
            let img = egui::ColorImage::from_rgba_premultiplied(size, pm.data());
            ctx.load_texture(format!("bicraft-cv-{id}"), img, egui::TextureOptions::LINEAR)
        });
    let out = tex.as_ref().map(TextureHandle::id);
    if app.cmd.custom_visuals.icons.len() < 512 {
        app.cmd.custom_visuals.icons.insert(id.to_string(), tex);
    }
    out
}

/// Import a visual from a file: the shell's upload, then `visual.import_custom {data}`.
pub fn import(app: &mut App) {
    let l = app.lang;
    if let Err(why) = crate::ribbon::state(app, IMPORT) {
        app.status = format!("{} {why}", l.t("Unavailable:", "Indisponible :"));
        return;
    }
    let Some(upload) = &app.shell.upload else {
        // no dialog here: the parameters form takes a path
        crate::form::open(app, IMPORT, json!({}));
        return;
    };
    let slot: UploadSlot = std::rc::Rc::new(std::cell::RefCell::new(None));
    upload(
        l,
        l.t(
            "Import a visual from a file",
            "Importer un visuel à partir d'un fichier",
        ),
        &["pbiviz"],
        MAX_PBIVIZ_BYTES,
        slot.clone(),
    );
    app.cmd.custom_visuals.pending = Some(slot);
    poll(app);
}

/// Takes a finished upload: its bytes go to `visual.import_custom`.
fn poll(app: &mut App) {
    let Some(slot) = &app.cmd.custom_visuals.pending else {
        return;
    };
    let got = slot.borrow_mut().take();
    let Some(got) = got else { return };
    app.cmd.custom_visuals.pending = None;
    let l = app.lang;
    match got {
        Picked::Cancelled => {}
        Picked::Failed(e) => app.error = Some(format!("✕ {e}")),
        Picked::File { name, bytes } if bytes.len() > MAX_PBIVIZ_BYTES => {
            app.error = Some(format!(
                "✕ {name}: {} ({} MB)",
                l.t(
                    "the file is larger than the limit",
                    "le fichier dépasse la limite"
                ),
                MAX_PBIVIZ_BYTES >> 20
            ));
        }
        Picked::File { bytes, .. } => {
            app.run(
                Call {
                    id: IMPORT,
                    params: json!({"data": crate::rich::base64(&bytes)}),
                },
                Then::Nothing,
            );
        }
    }
}

/// The buttons (the ribbon, the palette) without their values: the upload, the Remove window.
pub fn intercept(app: &mut App, id: &str, params: &Value) -> bool {
    match id {
        IMPORT if params.get("path").is_none() && params.get("data").is_none() => {
            import(app);
            true
        }
        REMOVE if params.get("name").is_none() => {
            app.cmd.custom_visuals.remove = Some(RemoveDialog::default());
            true
        }
        _ => false,
    }
}

/// The gallery's "…" menu: Import a visual from a file, Remove a custom visual.
pub fn more_menu(app: &mut App, ui: &mut egui::Ui) {
    let l = app.lang;
    let import_state = crate::ribbon::state(app, IMPORT);
    let r = ui.add_enabled(
        import_state.is_ok(),
        egui::Button::new(l.t(
            "Import a visual from a file",
            "Importer un visuel à partir d'un fichier",
        )),
    );
    if r.clicked() {
        ui.close();
        crate::ribbon::invoke(app, IMPORT, json!({}));
    }
    let any = !entries(app).is_empty();
    let remove_ok = any && crate::ribbon::state(app, REMOVE).is_ok();
    let r = ui.add_enabled(
        remove_ok,
        egui::Button::new(l.t("Remove a custom visual", "Supprimer un visuel personnalisé")),
    );
    let r = if any {
        r
    } else {
        r.on_disabled_hover_text(l.t(
            "The report has no custom visual.",
            "Le rapport n'a aucun visuel personnalisé.",
        ))
    };
    if r.clicked() {
        ui.close();
        crate::ribbon::invoke(app, REMOVE, json!({}));
    }
}

/// Draws the Remove a custom visual window and takes a finished upload; every frame.
pub fn show(app: &mut App, ctx: &egui::Context) {
    poll(app);
    if app.cmd.custom_visuals.pending.is_some() {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
    let Some(mut d) = app.cmd.custom_visuals.remove.clone() else {
        return;
    };
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let list = entries(app);
    let counts: Vec<usize> = list.iter().map(|e| users(app, &e.name)).collect();
    let mut open = true;
    let mut close = false;
    let mut remove: Option<(String, bool)> = None;
    egui::Window::new(l.t("Remove a custom visual", "Supprimer un visuel personnalisé"))
        .id(egui::Id::unique("bicraft-remove-custom-visual"))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width(420.0);
            if list.is_empty() {
                ui.label(egui::RichText::new(no_visual(l)).color(t.text_dim));
            }
            for (e, n) in list.iter().zip(&counts) {
                ui.horizontal(|ui| {
                    let tex = e.icon.as_deref().and_then(|id| icon_of(app, ui.ctx(), id));
                    match tex {
                        Some(tex) => {
                            ui.add(egui::Image::new((tex, vec2(20.0, 20.0))));
                        }
                        None => {
                            ui.add_space(24.0);
                        }
                    }
                    ui.label(&e.label);
                    if *n > 0 {
                        ui.label(egui::RichText::new(used_text(l, *n)).small().color(t.text_dim));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = format!("{} {}", l.t("Remove", "Supprimer"), e.label);
                        let b = ui.button(l.t("Remove", "Supprimer"));
                        b.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
                        if b.clicked() {
                            if *n > 0 {
                                d.confirm = Some(e.name.clone());
                            } else {
                                remove = Some((e.name.clone(), false));
                            }
                        }
                    });
                });
            }
            if let Some(name) = d.confirm.clone() {
                let n = list
                    .iter()
                    .zip(&counts)
                    .find(|(e, _)| e.name == name)
                    .map_or(0, |(_, n)| *n);
                ui.separator();
                ui.label(confirm_text(l, n));
                ui.horizontal(|ui| {
                    if ui
                        .button(l.t("Remove it, keep the visuals", "Le supprimer, garder les visuels"))
                        .clicked()
                    {
                        remove = Some((name.clone(), true));
                    }
                    if ui.button(l.t("Cancel", "Annuler")).clicked() {
                        d.confirm = None;
                    }
                });
            }
            ui.add_space(4.0);
            if ui.button(l.t("Close", "Fermer")).clicked() {
                close = true;
            }
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    if let Some((name, force)) = remove {
        d.confirm = None;
        let mut p = json!({"name": name});
        if force {
            p["force"] = json!(true);
        }
        app.run(
            Call {
                id: REMOVE,
                params: p,
            },
            Then::Nothing,
        );
    }
    app.cmd.custom_visuals.remove = (open && !close).then_some(d);
}

/// [`icon`] for a borrowed app (the window's rows): the cached texture only.
fn icon_of(app: &App, _ctx: &egui::Context, id: &str) -> Option<TextureId> {
    app.cmd
        .custom_visuals
        .icons
        .get(id)
        .and_then(|t| t.as_ref().map(TextureHandle::id))
}

fn no_visual(l: Lang) -> &'static str {
    l.t(
        "The report has no custom visual. Import one from the Visualizations pane's … menu.",
        "Le rapport n'a aucun visuel personnalisé. Importez-en un depuis le menu … du volet Visualisations.",
    )
}

fn used_text(l: Lang, n: usize) -> String {
    match (l, n) {
        (Lang::Fr, 1) => "utilisé par 1 visuel".into(),
        (Lang::Fr, n) => format!("utilisé par {n} visuels"),
        (_, 1) => "used by 1 visual".into(),
        (_, n) => format!("used by {n} visuals"),
    }
}

fn confirm_text(l: Lang, n: usize) -> String {
    match l {
        Lang::Fr => format!(
            "{n} visuel(s) du rapport l'utilisent : ils restent sur leurs pages, dessinés comme des espaces réservés."
        ),
        _ => format!("{n} visual(s) of the report use it: they stay on their pages, drawn as placeholders."),
    }
}
