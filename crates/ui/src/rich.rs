//! Rich report content in the Report view (ROADMAP M11.20, docs/desktop.md "Insert"): the Insert
//! tab's Text box, Buttons and Image, the text box editor in place on the canvas with its rich
//! text toolbar, the action editor of buttons, images and shapes (with the button states), the
//! image options and the page's Format window. Every committed change is one engine command
//! (`textbox.add`, `textbox.set`, `button.add`, `image.add`, `image.set`, `visual.set_action`,
//! `visual.format`, `page.format`), so Undo takes it back and the page re-renders.
//!
//! The ribbon's, menus' and palette's buttons reach these through [`intercept`] (called by
//! `ribbon::invoke` once the command is enabled): a command given every value it needs still runs
//! as is (the palette's Shift+Enter keeps the generic form).

use egui::{Color32, Id, Rect, RichText, Stroke, Ui};
use serde_json::{Value, json};

use crate::actions::Call;
use crate::app::{App, Then};
use crate::i18n::Lang;
use crate::ribbon::VisualSel;
use crate::rich_text::{Align, CLOSE, Doc, List, OPEN, Style};
use crate::theme::Tokens;

/// Largest image the window sends (the engine's cap).
pub const MAX_IMAGE_BYTES: usize = 16 << 20;
/// Font families the toolbar offers (Desktop's text box list, the common ones).
pub const FAMILIES: &[&str] = &[
    "Segoe UI",
    "Segoe UI Light",
    "Segoe UI Semibold",
    "Segoe UI Bold",
    "Arial",
    "Calibri",
    "Cambria",
    "Candara",
    "Consolas",
    "Corbel",
    "Courier New",
    "DIN",
    "Georgia",
    "Tahoma",
    "Times New Roman",
    "Trebuchet MS",
    "Verdana",
];
/// Font sizes the toolbar offers (points).
pub const SIZES: &[f32] = &[
    8.0, 9.0, 10.0, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 24.0, 28.0, 32.0, 36.0, 40.0, 48.0, 60.0, 72.0,
];
/// The default size of text box text (points), as the renderer draws a run without one.
const DEFAULT_PT: f32 = 10.0;

/// What a command run from here is waiting for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// A visual was inserted: select it (and edit its text when `edit`).
    Inserted { edit: bool },
    /// A change was committed (the error, if any, goes to the error bar).
    Committed,
}

/// Where a picked image goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageTarget {
    /// `image.add` on the page shown.
    Insert,
    /// `image.set` of this visual.
    Replace(VisualSel),
    /// `page.format`'s background (`true`) or wallpaper image of this page.
    Page { page: String, background: bool },
}

/// The text box being edited in place.
#[derive(Debug, Clone)]
pub struct Editor {
    pub page: String,
    pub visual: String,
    pub doc: Doc,
    /// The document as it was opened (a commit that changes nothing sends nothing).
    pub original: Doc,
    /// The string the text edit works on (the document's text).
    pub text: String,
    /// The style typed characters get (the toolbar's state with no selection).
    pub typing: Style,
    /// The selection or cursor (char indices, any order).
    pub sel: (usize, usize),
    /// The link being typed for the selection.
    pub link: String,
    /// Focus the text on the next frame.
    pub focus: bool,
    /// Put the cursor here on the next frame.
    pub cursor_to: Option<usize>,
}

impl Editor {
    pub fn new(page: &str, visual: &str, doc: Doc) -> Editor {
        let n = doc.text.chars().count();
        Editor {
            page: page.to_string(),
            visual: visual.to_string(),
            text: doc.text.clone(),
            typing: doc.style_at(n),
            original: doc.clone(),
            doc,
            sel: (n, n),
            link: String::new(),
            focus: true,
            cursor_to: Some(n),
        }
    }

    fn range(&self) -> (usize, usize) {
        (self.sel.0.min(self.sel.1), self.sel.0.max(self.sel.1))
    }

    /// Styles the selection, or the typing style when nothing is selected.
    pub fn style(&mut self, f: impl Fn(&mut Style)) {
        let (a, b) = self.range();
        if a == b {
            f(&mut self.typing);
        } else {
            self.doc.apply((a, b), &f);
            // the typing style follows what the selection now shows
            self.typing = self.doc.style_at(b);
        }
        self.focus = true;
    }

    /// The style the toolbar shows: the first selected character's, else the typing style.
    pub fn shown(&self) -> Style {
        let (a, b) = self.range();
        if a == b {
            self.typing.clone()
        } else {
            self.doc.styles.get(a).cloned().unwrap_or_default()
        }
    }

    /// Inserts a dynamic value at the cursor (over the selection).
    pub fn insert_value(&mut self, field: &str) {
        let (a, b) = self.range();
        if a != b {
            let chars: Vec<char> = self.doc.text.chars().collect();
            let keep: String = chars.iter().take(a).chain(chars.iter().skip(b)).collect();
            self.doc.sync(&keep, &self.typing);
        }
        let at = self.doc.insert_value(a, field, &self.typing);
        self.text = self.doc.text.clone();
        self.sel = (at, at);
        self.cursor_to = Some(at);
        self.focus = true;
    }
}

/// The Report view's rich content state (per document, in [`crate::ribbon::DocUi`]).
#[derive(Debug, Default)]
pub struct RichUi {
    pub editor: Option<Editor>,
    /// The Insert › Buttons chooser is open.
    pub buttons: bool,
    /// The workspace image chooser (the browser: no file dialog) for this target.
    pub choose_image: Option<ImageTarget>,
    pub action: Option<ActionForm>,
    pub image: Option<ImageForm>,
    pub page_format: Option<PageForm>,
    /// A web link a button opened, for the window to hand to the browser or the OS.
    pub open_url: Option<String>,
}

/// The action editor's values.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionForm {
    pub sel: VisualSel,
    pub visual_type: String,
    pub kind: String,
    pub destination: String,
    pub bookmark: String,
    pub url: String,
    pub tooltip: String,
    /// The destination is conditional (fx): the value of `field` names it.
    pub fx: bool,
    /// The fx field: `[Measure]` or `Table[Column]`.
    pub field: String,
    /// A drillthrough button's tooltips when it is enabled and disabled.
    pub enabled_tooltip: String,
    pub disabled_tooltip: String,
    /// The button state the style section edits (`default`, `hover`, `press`, `disabled`).
    pub state: String,
    pub text: String,
    pub font_color: [u8; 3],
    pub fill: [u8; 3],
    pub fill_transparency: f32,
    /// The outline (Style › Border) of that state: on, colour, weight.
    pub outline: bool,
    pub outline_color: [u8; 3],
    pub outline_weight: f32,
}

/// The image options' values.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageForm {
    pub sel: VisualSel,
    pub scaling: String,
}

/// The page format window's values (the snapshot's at opening; only what changed is sent).
#[derive(Debug, Clone, PartialEq)]
pub struct PageForm {
    pub page: String,
    pub bg: Option<[u8; 3]>,
    pub bg_transparency: f32,
    pub bg_scaling: String,
    pub wp: Option<[u8; 3]>,
    pub wp_transparency: f32,
    pub wp_scaling: String,
    pub size: String,
    pub width: f32,
    pub height: f32,
    pub valign: String,
    pub start: Box<Option<PageForm>>,
}

/// The kinds of Insert › Buttons with their labels (EN, FR).
pub const BUTTON_KINDS: &[(&str, &str, &str)] = &[
    ("left_arrow", "Left arrow", "Flèche gauche"),
    ("right_arrow", "Right arrow", "Flèche droite"),
    ("reset", "Reset", "Réinitialiser"),
    ("back", "Back", "Retour"),
    ("information", "Information", "Information"),
    ("help", "Help", "Aide"),
    ("qna", "Q&A", "Q&R"),
    ("bookmark", "Bookmark", "Signet"),
    ("blank", "Blank", "Vide"),
    ("page_navigator", "Page navigator", "Navigateur de pages"),
    (
        "bookmark_navigator",
        "Bookmark navigator",
        "Navigateur de signets",
    ),
    (
        "apply_all_slicers",
        "Apply all slicers",
        "Appliquer tous les segments",
    ),
    (
        "clear_all_slicers",
        "Clear all slicers",
        "Effacer tous les segments",
    ),
];

/// Insert › Shapes: the `tileShape` values `shape.add` takes, with their labels (EN after the
/// public theme schema's titles, FR translated from their meaning).
pub const SHAPES: &[(&str, &str, &str)] = &[
    ("rectangle", "Rectangle", "Rectangle"),
    ("rectangleRounded", "Rounded rectangle", "Rectangle arrondi"),
    (
        "rectangleRoundedByPixel",
        "Rounded rectangle (px)",
        "Rectangle arrondi (px)",
    ),
    ("oval", "Oval", "Ovale"),
    ("line", "Line", "Ligne"),
    ("triangleIsoc", "Triangle", "Triangle"),
    ("triangleRight", "Right triangle", "Triangle rectangle"),
    ("arrow", "Arrow", "Flèche"),
    ("arrowChevron", "Chevron arrow", "Flèche chevron"),
    ("arrowPentagon", "Pentagon arrow", "Flèche pentagone"),
    ("pentagon", "Pentagon", "Pentagone"),
    ("hexagon", "Hexagon", "Hexagone"),
    ("octagon", "Octagon", "Octogone"),
    ("parallelogram", "Parallelogram", "Parallélogramme"),
    ("trapezoid", "Trapezoid", "Trapèze"),
    ("pill", "Pill", "Pilule"),
    ("heart", "Heart", "Cœur"),
    ("speechbubbleRectangle", "Speech bubble", "Bulle de dialogue"),
    (
        "tabCutCorner",
        "Snipped tab, top right",
        "Onglet coupé, en haut à droite",
    ),
    (
        "tabCutTopCorners",
        "Snipped tab, both top",
        "Onglet coupé, les deux coins du haut",
    ),
    (
        "tabCutTopCornersByPixel",
        "Snipped tab, both top (px)",
        "Onglet coupé, les deux coins du haut (px)",
    ),
    (
        "tabRoundCorner",
        "Rounded tab, top right",
        "Onglet arrondi, en haut à droite",
    ),
    (
        "tabRoundTopCorners",
        "Rounded tab, both top",
        "Onglet arrondi, les deux coins du haut",
    ),
];

/// Inserts shape `tile` on the page shown (`shape.add`) and selects it.
pub fn insert_shape(app: &mut App, tile: &str) {
    if let Some(page) = shown_page(app) {
        run(
            app,
            "shape.add",
            json!({"page": page, "shape": tile}),
            Step::Inserted { edit: false },
        );
    }
}

/// The action types with their labels (EN, FR).
pub const ACTION_KINDS: &[(&str, &str, &str)] = &[
    ("back", "Back", "Retour"),
    ("bookmark", "Bookmark", "Signet"),
    ("drillthrough", "Drill through", "Extraction"),
    ("page_navigation", "Page navigation", "Navigation de page"),
    ("qna", "Q&A", "Q&R"),
    ("web_url", "Web URL", "URL web"),
    (
        "apply_all_slicers",
        "Apply all slicers",
        "Appliquer tous les segments",
    ),
    (
        "clear_all_slicers",
        "Clear all slicers",
        "Effacer tous les segments",
    ),
];

const SIZES_PAGE: &[(&str, &str, &str)] = &[
    ("16:9", "16:9", "16:9"),
    ("4:3", "4:3", "4:3"),
    ("letter", "Letter", "Lettre"),
    ("tooltip", "Tooltip", "Info-bulle"),
    ("custom", "Custom", "Personnalisé"),
];

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

/// Standard base64 (the `data` parameter of `image.add`).
pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3).saturating_mul(4));
    for c in bytes.chunks(3) {
        let b = |i: usize| u32::from(c.get(i).copied().unwrap_or(0));
        let n = (b(0) << 16) | (b(1) << 8) | b(2);
        for i in 0..4usize {
            if i <= c.len() {
                let k = ((n >> (18 - 6 * i as u32)) & 63) as usize;
                out.push(T.get(k).map_or('A', |x| char::from(*x)));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

/// `#RRGGBB` (or `#RGB`) → bytes.
pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().strip_prefix('#')?;
    let h: String = match h.len() {
        3 => h.chars().flat_map(|c| [c, c]).collect(),
        6 | 8 => h.chars().take(6).collect(),
        _ => return None,
    };
    let v = u32::from_str_radix(&h, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// The snapshot's object of a visual on page `page`.
pub fn snap_visual(app: &App, page: &str, visual: &str) -> Option<Value> {
    let snap = &app.loaded.as_ref()?.snap;
    snap.pointer("/report/pages")?
        .as_array()?
        .iter()
        .find(|p| p.get("name").and_then(Value::as_str) == Some(page))?
        .get("visuals")?
        .as_array()?
        .iter()
        .find(|v| v.get("name").and_then(Value::as_str) == Some(visual))
        .cloned()
}

fn snap_page(app: &App, page: &str) -> Option<Value> {
    let snap = &app.loaded.as_ref()?.snap;
    snap.pointer("/report/pages")?
        .as_array()?
        .iter()
        .find(|p| p.get("name").and_then(Value::as_str) == Some(page))
        .cloned()
}

fn shown_page(app: &App) -> Option<String> {
    app.page_names().get(app.active_page()).map(|p| p.0.clone())
}

fn vtype(app: &App, sel: &VisualSel) -> String {
    snap_visual(app, &sel.page, &sel.visual)
        .and_then(|v| v.get("type").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default()
}

/// The model's measures as `[Name]`, for "+ Value".
pub fn measures(app: &App) -> Vec<String> {
    let mut out = Vec::new();
    let Some(l) = app.loaded.as_ref() else { return out };
    for t in l
        .snap
        .pointer("/model/tables")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .take(2000)
    {
        for m in t
            .get("measures")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .take(5000)
        {
            if let Some(n) = m.get("name").and_then(Value::as_str) {
                out.push(format!("[{}]", n.replace(']', "]]")));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn run(app: &mut App, id: &'static str, params: Value, step: Step) {
    app.run(Call { id, params }, Then::Rich(step));
}

// ---------------------------------------------------------------------------------------------
// The buttons' entry point
// ---------------------------------------------------------------------------------------------

/// What a click on command `id` does here instead of the parameters form; `false`: the button's
/// usual path. Called once the command is enabled.
pub fn intercept(app: &mut App, id: &str, params: &Value) -> bool {
    let page = params
        .get("page")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| shown_page(app));
    let sel = app.cmd.visual.clone();
    match id {
        "textbox.add" if params.get("paragraphs").is_none() && params.get("text").is_none() => {
            let Some(page) = page else { return false };
            run(
                app,
                "textbox.add",
                json!({"page": page}),
                Step::Inserted { edit: true },
            );
            true
        }
        "button.add" if params.get("kind").is_none() => {
            app.cmd.rich.buttons = true;
            true
        }
        "image.add" if params.get("data").is_none() && params.get("path").is_none() => {
            pick_image(app, ImageTarget::Insert);
            true
        }
        "textbox.set" if params.get("paragraphs").is_none() && params.get("text").is_none() => {
            let Some(sel) = sel else { return false };
            open_editor(app, &sel);
            true
        }
        "image.set"
            if params.get("data").is_none()
                && params.get("path").is_none()
                && params.get("scaling").is_none() =>
        {
            let Some(sel) = sel else { return false };
            if !vtype(app, &sel).eq_ignore_ascii_case("image") {
                app.status = app
                    .lang
                    .t("Select an image first.", "Sélectionnez d'abord une image.")
                    .into();
                return true;
            }
            let scaling = snap_visual(app, &sel.page, &sel.visual)
                .and_then(|v| {
                    v.pointer("/image/scaling")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "normal".into());
            app.cmd.rich.image = Some(ImageForm { sel, scaling });
            true
        }
        "visual.set_action" if params.get("type").is_none() => {
            let Some(sel) = sel else { return false };
            open_action(app, &sel);
            true
        }
        "page.format" if params.as_object().is_some_and(|o| o.keys().all(|k| k == "page")) => {
            let Some(page) = page else { return false };
            open_page_format(app, &page);
            true
        }
        _ => false,
    }
}

/// Opens the in-place editor on a text box.
pub fn open_editor(app: &mut App, sel: &VisualSel) {
    let v = snap_visual(app, &sel.page, &sel.visual);
    if let Some(v) = &v
        && !v
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|t| t.eq_ignore_ascii_case("textbox"))
    {
        app.status = app
            .lang
            .t(
                "Select a text box first.",
                "Sélectionnez d'abord une zone de texte.",
            )
            .into();
        return;
    }
    let doc = v
        .as_ref()
        .and_then(|v| v.get("text"))
        .map_or_else(Doc::empty, Doc::from_snapshot);
    app.cmd.rich.editor = Some(Editor::new(&sel.page, &sel.visual, doc));
}

fn open_action(app: &mut App, sel: &VisualSel) {
    let v = snap_visual(app, &sel.page, &sel.visual).unwrap_or(Value::Null);
    let ty = v.get("type").and_then(Value::as_str).unwrap_or("").to_string();
    if !["actionButton", "image", "shape", "basicShape"]
        .iter()
        .any(|t| t.eq_ignore_ascii_case(&ty))
    {
        app.status = app
            .lang
            .t(
                "Actions are on buttons, images and shapes.",
                "Les actions sont sur les boutons, images et formes.",
            )
            .into();
        return;
    }
    let a = v.get("action").cloned().unwrap_or(Value::Null);
    let st = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let b = if ty.eq_ignore_ascii_case("shape") {
        // a shape's style: its text and fill (the snapshot's shape names)
        let sh = v.get("shape").cloned().unwrap_or(Value::Null);
        json!({
            "text": sh.pointer("/text/text").cloned().unwrap_or(Value::Null),
            "font_color": sh.pointer("/text/font_color").cloned().unwrap_or(Value::Null),
            "fill": sh.get("fill").cloned().unwrap_or(Value::Null),
            "fill_transparency": sh.get("fill_transparency").cloned().unwrap_or(Value::Null),
            "show_outline": sh.get("show_line").cloned().unwrap_or(Value::Null),
            "outline": sh.get("line").cloned().unwrap_or(Value::Null),
            "outline_weight": sh.get("line_weight").cloned().unwrap_or(Value::Null),
        })
    } else {
        v.get("button").cloned().unwrap_or(Value::Null)
    };
    let color = |k: &str, d: [u8; 3]| b.get(k).and_then(Value::as_str).and_then(parse_hex).unwrap_or(d);
    app.cmd.rich.action = Some(ActionForm {
        sel: sel.clone(),
        visual_type: ty,
        kind: if st("type").is_empty() {
            "none".into()
        } else {
            st("type")
        },
        destination: st("destination"),
        bookmark: st("bookmark"),
        url: st("url"),
        tooltip: st("tooltip"),
        fx: !st("destination_field").is_empty(),
        field: st("destination_field"),
        enabled_tooltip: st("enabled_tooltip"),
        disabled_tooltip: st("disabled_tooltip"),
        state: "default".into(),
        text: b.get("text").and_then(Value::as_str).unwrap_or("").to_string(),
        font_color: color("font_color", [0x25, 0x25, 0x25]),
        fill: color("fill", [0xE6, 0xE6, 0xE6]),
        fill_transparency: b.get("fill_transparency").and_then(Value::as_f64).unwrap_or(0.0) as f32,
        outline: b.get("show_outline").and_then(Value::as_bool).unwrap_or(false),
        outline_color: color("outline", [0x60, 0x5E, 0x5C]),
        outline_weight: b.get("outline_weight").and_then(Value::as_f64).unwrap_or(1.0) as f32,
    });
}

/// The report's bookmarks: (name, display name), in their order.
fn bookmark_list(app: &App) -> Vec<(String, String)> {
    let Some(l) = app.loaded.as_ref() else {
        return Vec::new();
    };
    l.snap
        .pointer("/bookmarks/bookmarks")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .take(500)
        .filter_map(|b| {
            let name = b.get("name")?.as_str()?.to_string();
            let label = b
                .get("display_name")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or(&name)
                .to_string();
            Some((name, label))
        })
        .collect()
}

fn open_page_format(app: &mut App, page: &str) {
    let p = snap_page(app, page).unwrap_or(Value::Null);
    let n = |k: &str, d: f64| p.get(k).and_then(Value::as_f64).unwrap_or(d);
    let wp = p.get("wallpaper").cloned().unwrap_or(Value::Null);
    let mut f = PageForm {
        page: page.to_string(),
        bg: p.get("background").and_then(Value::as_str).and_then(parse_hex),
        bg_transparency: n("background_transparency", 0.0) as f32,
        bg_scaling: p
            .pointer("/background_image/fit")
            .and_then(Value::as_str)
            .unwrap_or("normal")
            .to_string(),
        wp: wp.get("color").and_then(Value::as_str).and_then(parse_hex),
        wp_transparency: wp.get("transparency").and_then(Value::as_f64).unwrap_or(0.0) as f32,
        wp_scaling: wp
            .get("fit")
            .and_then(Value::as_str)
            .unwrap_or("normal")
            .to_string(),
        size: p
            .get("size")
            .and_then(Value::as_str)
            .unwrap_or("16:9")
            .to_string(),
        width: n("width", 1280.0) as f32,
        height: n("height", 720.0) as f32,
        valign: p
            .get("vertical_alignment")
            .and_then(Value::as_str)
            .unwrap_or("top")
            .to_string(),
        start: Box::new(None),
    };
    *f.start = Some(f.clone());
    app.cmd.rich.page_format = Some(f);
}

/// Asks for an image: the shell's dialog (the desktop reads the file and sends its bytes), else
/// the workspace's image files (the browser on a server).
pub fn pick_image(app: &mut App, target: ImageTarget) {
    let l = app.lang;
    if let Some(pick) = &app.shell.pick_image {
        match pick(l) {
            None => {}
            Some(Err(e)) => app.error = Some(format!("✕ {e}")),
            Some(Ok((name, bytes))) => {
                if bytes.len() > MAX_IMAGE_BYTES {
                    app.error = Some(format!(
                        "✕ {name}: {} ({} MB)",
                        l.t("the image is larger than the limit", "l'image dépasse la limite"),
                        MAX_IMAGE_BYTES >> 20
                    ));
                    return;
                }
                send_image(app, target, Some(base64(&bytes)), None, &name);
            }
        }
        return;
    }
    if app.workspace().is_some() {
        if app.files.is_none() {
            app.list_files();
        }
        app.cmd.rich.choose_image = Some(target);
    } else {
        app.status = l
            .t(
                "No file dialog here: an image is inserted from a server workspace's files.",
                "Pas de boîte de dialogue ici : une image s'insère depuis les fichiers d'un espace de travail.",
            )
            .into();
    }
}

/// Sends a picked image (its bytes as `data`, or a workspace `path`) to its target.
pub fn send_image(
    app: &mut App,
    target: ImageTarget,
    data: Option<String>,
    path: Option<String>,
    name: &str,
) {
    match target {
        ImageTarget::Insert => {
            let Some(page) = shown_page(app) else { return };
            let mut p = json!({"page": page, "name": name});
            if let Some(d) = data {
                p["data"] = json!(d);
            }
            if let Some(f) = path {
                p["path"] = json!(f);
            }
            run(app, "image.add", p, Step::Inserted { edit: false });
        }
        ImageTarget::Replace(sel) => {
            let mut p = json!({"page": sel.page, "visual": sel.visual, "name": name});
            if let Some(d) = data {
                p["data"] = json!(d);
            }
            if let Some(f) = path {
                p["path"] = json!(f);
            }
            run(app, "image.set", p, Step::Committed);
        }
        ImageTarget::Page { page, background } => {
            let k = if background { "background" } else { "wallpaper" };
            let mut p = json!({"page": page});
            if let Some(d) = data {
                p[format!("{k}_image")] = json!(d);
            }
            if let Some(f) = path {
                p[format!("{k}_image_path")] = json!(f);
            }
            run(app, "page.format", p, Step::Committed);
        }
    }
}

/// The answer of a command run from here.
pub fn answer(app: &mut App, step: Step, r: Result<Value, String>) {
    match r {
        Err(e) => app.error = Some(format!("✕ {}", crate::app::clip(&e, 600))),
        Ok(v) => {
            if let Step::Inserted { edit } = step
                && let (Some(page), Some(visual)) = (
                    v.get("page").and_then(Value::as_str),
                    v.get("visual").and_then(Value::as_str),
                )
            {
                let sel = VisualSel {
                    page: page.to_string(),
                    visual: visual.to_string(),
                };
                app.cmd.visual = Some(sel.clone());
                if edit {
                    app.cmd.rich.editor = Some(Editor::new(page, visual, Doc::empty()));
                }
            }
        }
    }
}

/// A reader's click on a button was answered: a web link opens.
pub fn interacted(app: &mut App, v: &Value) {
    if let Some(u) = v.get("url").and_then(Value::as_str) {
        let l = u.trim().to_ascii_lowercase();
        if l.starts_with("http://") || l.starts_with("https://") || l.starts_with("mailto:") {
            app.cmd.rich.open_url = Some(u.to_string());
        }
    }
}

/// Whether a plain click on the page should leave a button alone: in the editor (not a
/// reader), a button acts on Ctrl+click only, as in Desktop's editing view; so do the buttons of
/// a page or bookmark navigator (their hits name the navigator: `visual`).
pub fn click_needs_ctrl(app: &App, action: &Value, ctrl: bool) -> bool {
    let from_navigator = matches!(
        action.get("action").and_then(Value::as_str),
        Some("page" | "bookmark" | "bookmark_deselect")
    ) && action.get("visual").is_some();
    (action.get("action").and_then(Value::as_str) == Some("button") || from_navigator)
        && !ctrl
        && app.access.as_deref() != Some("reader")
}

/// Where the pointer is for Desktop's button states (`bicraft_render::states`): the topmost
/// button, shape or navigator under `at` (report pixels) that sets a hover or press look, with
/// the navigator button under it (`hit.item`), pressed or not; `None` elsewhere, so the page is
/// drawn again only over visuals whose look changes.
pub(crate) fn pointer_state(
    app: &App,
    at: Option<egui::Pos2>,
    hit: Option<&bicraft_render::hits::Hit>,
    pressed: bool,
) -> Option<bicraft_render::states::Pointer> {
    let at = at?;
    let (_, _, items) = crate::canvas::page_items(app)?;
    let snap = &app.loaded.as_ref()?.snap;
    let visuals = snap
        .pointer(&format!("/report/pages/{}/visuals", app.active_page()))?
        .as_array()?;
    let (item, v) = items
        .iter()
        .filter(|i| i.pickable && i.rect.contains(at))
        .filter_map(|i| {
            let v = visuals
                .iter()
                .find(|v| v.get("name").and_then(Value::as_str) == Some(i.name.as_str()))?;
            Some((i, v))
        })
        .max_by(|a, b| a.0.z.total_cmp(&b.0.z))?;
    let has = |p: &str| {
        v.pointer(p)
            .and_then(Value::as_object)
            .is_some_and(|o| !o.is_empty())
    };
    let ty = v.get("type").and_then(Value::as_str).unwrap_or("");
    let nav = ty.eq_ignore_ascii_case("pageNavigator") || ty.eq_ignore_ascii_case("bookmarkNavigator");
    let looks = if nav {
        has("/navigator/styles/hover") || has("/navigator/styles/press")
    } else if ty.eq_ignore_ascii_case("actionButton") {
        has("/button/states/hover") || has("/button/states/press")
    } else if ty.eq_ignore_ascii_case("shape") {
        has("/shape/states/hover") || has("/shape/states/press")
    } else {
        false
    };
    if !looks {
        return None;
    }
    let sub = hit.filter(|h| h.visual == item.name).and_then(|h| h.item.clone());
    if nav && sub.is_none() {
        return None;
    }
    Some(bicraft_render::states::Pointer {
        visual: item.name.clone(),
        item: if nav { sub } else { None },
        press: pressed,
    })
}

// ---------------------------------------------------------------------------------------------
// The canvas: the editor in place
// ---------------------------------------------------------------------------------------------

/// The text box editor over the page (`rect` the page on the screen at `zoom`), and a double
/// click on a text box that opens it. Called after the page's own input.
pub fn canvas(app: &mut App, ui: &mut Ui, resp: &egui::Response, rect: Rect, zoom: f32) {
    if let Some(u) = app.cmd.rich.open_url.take() {
        ui.ctx().open_url(egui::OpenUrl::new_tab(u));
    }
    if resp.double_clicked()
        && app.cmd.rich.editor.is_none()
        && app.access.as_deref() != Some("reader")
        && let Some(sel) = app.cmd.visual.clone()
        && vtype(app, &sel).eq_ignore_ascii_case("textbox")
    {
        open_editor(app, &sel);
    }
    let Some(ed) = app.cmd.rich.editor.as_ref() else {
        return;
    };
    let shown = shown_page(app);
    if shown.as_deref() != Some(ed.page.as_str()) {
        commit(app);
        return;
    }
    let sel = VisualSel {
        page: ed.page.clone(),
        visual: ed.visual.clone(),
    };
    // the new text box is not in the snapshot yet: wait for it
    let Some(r) = crate::menus::visual_rect(app, &sel) else {
        return;
    };
    let screen = Rect::from_min_size(rect.min + r.min.to_vec2() * zoom, r.size() * zoom);
    // a click on the page outside the box commits the edit
    if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
        && !screen.contains(p)
    {
        commit(app);
        return;
    }
    editor_ui(app, ui, screen, zoom);
}

/// Sends the edit (`textbox.set`) when it changed anything, and closes the editor.
pub fn commit(app: &mut App) {
    let Some(ed) = app.cmd.rich.editor.take() else {
        return;
    };
    let mut doc = ed.doc.clone();
    doc.sync(&ed.text, &ed.typing);
    if doc == ed.original {
        return;
    }
    run(
        app,
        "textbox.set",
        json!({"page": ed.page, "visual": ed.visual, "paragraphs": doc.to_paragraphs()}),
        Step::Committed,
    );
}

/// Closes the editor without sending anything.
pub fn cancel(app: &mut App) {
    app.cmd.rich.editor = None;
}

fn text_format(st: &Style, zoom: f32, t: &Tokens, token: bool) -> egui::TextFormat {
    let pt = st.size.unwrap_or(DEFAULT_PT).clamp(4.0, 200.0);
    let px = (pt * 96.0 / 72.0 * zoom).clamp(4.0, 400.0);
    let family = if st.bold {
        crate::fonts::bold()
    } else {
        egui::FontFamily::Proportional
    };
    let color = st
        .color
        .as_deref()
        .and_then(parse_hex)
        .map_or(Color32::from_rgb(0x25, 0x25, 0x25), |c| {
            Color32::from_rgb(c[0], c[1], c[2])
        });
    let mut f = egui::TextFormat {
        font_id: egui::FontId::new(px, family),
        color,
        italics: st.italic,
        ..Default::default()
    };
    if st.underline || st.url.is_some() {
        f.underline = Stroke::new((px / 14.0).max(1.0), color);
    }
    if token {
        f.background = t.accent_soft;
    }
    f
}

/// The editor's layout: one section per run of a style, dynamic values on a tint, list
/// paragraphs indented, one alignment when every paragraph has the same.
fn layout(doc: &Doc, text: &str, typing: &Style, zoom: f32, wrap: f32, t: &Tokens) -> egui::text::LayoutJob {
    let mut d = doc.clone();
    d.sync(text, typing);
    let mut job = egui::text::LayoutJob {
        wrap: egui::text::TextWrapping {
            max_width: wrap,
            ..Default::default()
        },
        ..Default::default()
    };
    if let Some(first) = d.paras.first()
        && d.paras.iter().all(|p| p.align == first.align)
    {
        job.halign = match first.align {
            Align::Left => egui::Align::LEFT,
            Align::Center => egui::Align::Center,
            Align::Right => egui::Align::RIGHT,
        };
    }
    let chars: Vec<char> = d.text.chars().collect();
    // which characters belong to a dynamic value token («…» on one line)
    let mut token = vec![false; chars.len()];
    let mut open: Option<usize> = None;
    for (i, c) in chars.iter().enumerate() {
        match c {
            '\n' => open = None,
            c if *c == OPEN => open = Some(i),
            c if *c == CLOSE => {
                if let Some(o) = open.take() {
                    for f in token.iter_mut().take(i.saturating_add(1)).skip(o) {
                        *f = true;
                    }
                }
            }
            _ => {}
        }
    }
    let mut line = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        let st = d.styles.get(i).cloned().unwrap_or_default();
        let tk = token.get(i).copied().unwrap_or(false);
        let line_start = i == 0 || chars.get(i.wrapping_sub(1)) == Some(&'\n');
        let mut piece = String::new();
        let mut j = i;
        while let Some(&ch) = chars.get(j) {
            if j > i && (d.styles.get(j) != Some(&st) || token.get(j).copied().unwrap_or(false) != tk) {
                break;
            }
            piece.push(ch);
            j += 1;
            if ch == '\n' {
                break;
            }
        }
        let lead = if line_start && d.paras.get(line).is_some_and(|p| p.list != List::None) {
            18.0 * zoom
        } else {
            0.0
        };
        job.append(&piece, lead, text_format(&st, zoom, t, tk));
        if piece.ends_with('\n') {
            line += 1;
        }
        i = j.max(i + 1);
    }
    if chars.is_empty() {
        job.append("", 0.0, text_format(typing, zoom, t, false));
    }
    job
}

fn editor_ui(app: &mut App, ui: &mut Ui, screen: Rect, zoom: f32) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let measures = measures(app);
    // the box's own background (not white: white text on a dark box must stay readable)
    let backdrop = app
        .backdrop_at(screen)
        .map(|c| if c.a() < 255 { Color32::WHITE.blend(c) } else { c })
        .unwrap_or(Color32::WHITE);
    let ctx = ui.ctx().clone();
    let id = Id::new("bicraft-textbox-editor");
    let Some(ed) = app.cmd.rich.editor.as_mut() else {
        return;
    };
    let mut done: Option<bool> = None;
    // the text, over the rendered box
    egui::Area::new(id.with("area"))
        .fixed_pos(screen.min)
        .order(egui::Order::Foreground)
        .show(&ctx, |ui| {
            ui.painter().rect_filled(screen, 0.0, backdrop);
            ui.painter().rect_stroke(
                screen.expand(1.0),
                0.0,
                Stroke::new(1.5, t.ring),
                egui::StrokeKind::Outside,
            );
            let (doc, typing) = (ed.doc.clone(), ed.typing.clone());
            let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
                let job = layout(&doc, buf.as_str(), &typing, zoom, wrap, &t);
                ui.fonts_mut(|f| f.layout_job(job))
            };
            let te = egui::TextEdit::multiline(&mut ed.text)
                .id(id)
                .frame(egui::Frame::NONE)
                .margin(egui::Margin::ZERO)
                .desired_width(screen.width().max(10.0))
                .min_size(screen.size())
                .layouter(&mut layouter);
            let mut out = te.show(ui);
            if let Some(c) = ed.cursor_to.take() {
                out.state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(c))));
                out.state.clone().store(ui.ctx(), id);
            }
            if ed.focus {
                out.response.response.request_focus();
                ed.focus = false;
            }
            if let Some(r) = out.cursor_range {
                let s = (r.secondary.index.0, r.primary.index.0);
                if s != ed.sel {
                    let moved = s.0 == s.1;
                    ed.sel = s;
                    if moved && !out.response.response.changed() {
                        ed.typing = ed.doc.style_at(s.1);
                    }
                }
            }
            ed.doc.sync(&ed.text.clone(), &ed.typing.clone());
            // Esc cancels (the text edit drops its focus on Esc itself, so the key decides)
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) && !ui.ctx().any_popup_open() {
                done = Some(false);
            }
        });
    // the toolbar, above the box (below it at the top of the window)
    let above = screen.min.y > 90.0;
    let pos = if above {
        egui::pos2(screen.min.x, screen.min.y - 74.0)
    } else {
        egui::pos2(screen.min.x, screen.max.y + 6.0)
    };
    egui::Area::new(id.with("toolbar"))
        .fixed_pos(pos)
        .order(egui::Order::Foreground)
        .show(&ctx, |ui| {
            crate::widgets::card(&t).show(ui, |ui| {
                toolbar(ui, ed, l, &measures, &mut done);
            });
        });
    match done {
        Some(true) => commit(app),
        Some(false) => cancel(app),
        None => {}
    }
}

fn toolbar(ui: &mut Ui, ed: &mut Editor, l: Lang, measures: &[String], done: &mut Option<bool>) {
    let shown = ed.shown();
    ui.horizontal(|ui| {
        let fam = shown.family.clone().unwrap_or_else(|| "Segoe UI".into());
        egui::ComboBox::from_id_salt("bicraft-tb-family")
            .selected_text(fam.clone())
            .width(130.0)
            .show_ui(ui, |ui| {
                for f in FAMILIES {
                    if ui.selectable_label(fam == *f, *f).clicked() {
                        let v = (*f != "Segoe UI").then(|| (*f).to_string());
                        ed.style(|s| s.family.clone_from(&v));
                    }
                }
            })
            .response
            .on_hover_text(l.t("Font", "Police"));
        let size = shown.size.unwrap_or(DEFAULT_PT);
        egui::ComboBox::from_id_salt("bicraft-tb-size")
            .selected_text(format!("{size}"))
            .width(48.0)
            .show_ui(ui, |ui| {
                for s in SIZES {
                    if ui
                        .selectable_label((size - s).abs() < 0.01, format!("{s}"))
                        .clicked()
                    {
                        let v = (*s != DEFAULT_PT).then_some(*s);
                        ed.style(|st| st.size = v);
                    }
                }
            })
            .response
            .on_hover_text(l.t("Font size", "Taille de police"));
        let toggle = |ui: &mut Ui, label: RichText, on: bool, tip: &str| {
            ui.add(egui::Button::selectable(on, label))
                .on_hover_text(tip)
                .clicked()
        };
        if toggle(
            ui,
            RichText::new("B").strong(),
            shown.bold,
            l.t("Bold (Ctrl+B)", "Gras (Ctrl+B)"),
        ) {
            let v = !shown.bold;
            ed.style(|s| s.bold = v);
        }
        if toggle(
            ui,
            RichText::new("I").italics(),
            shown.italic,
            l.t("Italic (Ctrl+I)", "Italique (Ctrl+I)"),
        ) {
            let v = !shown.italic;
            ed.style(|s| s.italic = v);
        }
        if toggle(
            ui,
            RichText::new("U").underline(),
            shown.underline,
            l.t("Underline (Ctrl+U)", "Souligné (Ctrl+U)"),
        ) {
            let v = !shown.underline;
            ed.style(|s| s.underline = v);
        }
        let mut c = shown
            .color
            .as_deref()
            .and_then(parse_hex)
            .map_or(Color32::from_rgb(0x25, 0x25, 0x25), |c| {
                Color32::from_rgb(c[0], c[1], c[2])
            });
        if ui
            .color_edit_button_srgba(&mut c)
            .on_hover_text(l.t("Font colour", "Couleur de police"))
            .changed()
        {
            let v = Some(hex([c.r(), c.g(), c.b()]));
            ed.style(|s| s.color.clone_from(&v));
        }
    });
    ui.horizontal(|ui| {
        let line = ed.doc.line_of(ed.range().0);
        let para = ed.doc.paras.get(line).copied().unwrap_or_default();
        for (a, label, tip) in [
            (Align::Left, "⇤", l.t("Align left", "Aligner à gauche")),
            (Align::Center, "≡", l.t("Center", "Centrer")),
            (Align::Right, "⇥", l.t("Align right", "Aligner à droite")),
        ] {
            if ui
                .add(egui::Button::selectable(para.align == a, label))
                .on_hover_text(tip)
                .clicked()
            {
                let r = ed.range();
                ed.doc.apply_para(r, |p| p.align = a);
                ed.focus = true;
            }
        }
        for (k, label, tip) in [
            (List::Bullet, "• —", l.t("Bulleted list", "Liste à puces")),
            (List::Number, "1. —", l.t("Numbered list", "Liste numérotée")),
        ] {
            if ui
                .add(egui::Button::selectable(para.list == k, label))
                .on_hover_text(tip)
                .clicked()
            {
                let r = ed.range();
                let v = if para.list == k { List::None } else { k };
                ed.doc.apply_para(r, |p| p.list = v);
                ed.focus = true;
            }
        }
        ui.separator();
        ui.add(
            egui::TextEdit::singleline(&mut ed.link)
                .desired_width(140.0)
                .hint_text(l.t("https://…", "https://…")),
        )
        .on_hover_text(l.t("Link of the selection", "Lien de la sélection"));
        if ui
            .button(l.t("Link", "Lien"))
            .on_hover_text(l.t(
                "Make the selection a link (http, https or mailto); empty removes it",
                "Faire de la sélection un lien (http, https ou mailto) ; vide le retire",
            ))
            .clicked()
        {
            let u = ed.link.trim().to_string();
            let v = (!u.is_empty()).then_some(u);
            ed.style(|s| s.url.clone_from(&v));
        }
        ui.menu_button(l.t("+ Value", "+ Valeur"), |ui| {
            if measures.is_empty() {
                ui.label(
                    RichText::new(l.t("No measure in the model", "Aucune mesure dans le modèle")).weak(),
                );
            }
            egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                for m in measures.iter().take(2000) {
                    if ui.selectable_label(false, m).clicked() {
                        ed.insert_value(m);
                        ui.close();
                    }
                }
            });
        })
        .response
        .on_hover_text(l.t(
            "Insert a measure's value into the text (a dynamic value)",
            "Insérer la valeur d'une mesure dans le texte (valeur dynamique)",
        ));
        ui.separator();
        if ui
            .button(l.t("✔ Done", "✔ Terminé"))
            .on_hover_text(l.t("Keep the text", "Garder le texte"))
            .clicked()
        {
            *done = Some(true);
        }
        if ui
            .button(l.t("× Cancel", "× Annuler"))
            .on_hover_text(l.t("Esc", "Échap"))
            .clicked()
        {
            *done = Some(false);
        }
    });
    // Ctrl+B, Ctrl+I, Ctrl+U
    let keys = ui.input_mut(|i| {
        [egui::Key::B, egui::Key::I, egui::Key::U].map(|k| i.consume_key(egui::Modifiers::COMMAND, k))
    });
    if keys[0] {
        let v = !shown.bold;
        ed.style(|s| s.bold = v);
    }
    if keys[1] {
        let v = !shown.italic;
        ed.style(|s| s.italic = v);
    }
    if keys[2] {
        let v = !shown.underline;
        ed.style(|s| s.underline = v);
    }
}

// ---------------------------------------------------------------------------------------------
// Windows: buttons, images, actions, page format
// ---------------------------------------------------------------------------------------------

/// The windows of this module (once per frame).
pub fn windows(app: &mut App, ctx: &egui::Context) {
    buttons_window(app, ctx);
    choose_image_window(app, ctx);
    image_window(app, ctx);
    action_window(app, ctx);
    page_window(app, ctx);
}

fn buttons_window(app: &mut App, ctx: &egui::Context) {
    if !app.cmd.rich.buttons {
        return;
    }
    let l = app.lang;
    let mut open = true;
    let mut picked = None;
    egui::Window::new(l.t("Insert a button", "Insérer un bouton"))
        .id(Id::new("bicraft-insert-buttons"))
        .open(&mut open)
        .collapsible(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, -20.0))
        .resizable(false)
        .show(ctx, |ui| {
            egui::Grid::new("bicraft-buttons-grid")
                .num_columns(2)
                .show(ui, |ui| {
                    for (i, (k, en, fr)) in BUTTON_KINDS.iter().enumerate() {
                        if ui.button(l.t(en, fr)).clicked() {
                            picked = Some(*k);
                        }
                        if i % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });
        });
    if let Some(k) = picked {
        open = false;
        if let Some(page) = shown_page(app) {
            run(
                app,
                "button.add",
                json!({"page": page, "kind": k}),
                Step::Inserted { edit: false },
            );
        }
    }
    app.cmd.rich.buttons = open;
}

fn is_image_file(p: &str) -> bool {
    let p = p.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".svg", ".webp"]
        .iter()
        .any(|e| p.ends_with(e))
}

fn choose_image_window(app: &mut App, ctx: &egui::Context) {
    let Some(target) = app.cmd.rich.choose_image.clone() else {
        return;
    };
    let l = app.lang;
    let mut open = true;
    let mut picked: Option<String> = None;
    let mut relist = false;
    egui::Window::new(l.t("Choose an image", "Choisir une image"))
        .id(Id::new("bicraft-choose-image"))
        .open(&mut open)
        .collapsible(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, -20.0))
        .show(ctx, |ui| {
            ui.label(
                RichText::new(l.t(
                    "Images among the workspace's files (upload one to the workspace first).",
                    "Images parmi les fichiers de l'espace (téléversez-en une d'abord).",
                ))
                .weak(),
            );
            match &app.files {
                None => {
                    ui.spinner();
                }
                Some(Err(e)) => {
                    ui.label(crate::app::clip(e, 200));
                    relist = ui
                        .button(l.t("List the files again", "Relister les fichiers"))
                        .clicked();
                }
                Some(Ok(files)) => {
                    let imgs: Vec<&str> = files
                        .iter()
                        .map(|f| f.path.as_str())
                        .filter(|p| is_image_file(p))
                        .take(2000)
                        .collect();
                    if imgs.is_empty() {
                        ui.label(l.t("No image file in the workspace.", "Aucune image dans l'espace."));
                    }
                    egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                        for p in imgs {
                            if ui.selectable_label(false, p).clicked() {
                                picked = Some(p.to_string());
                            }
                        }
                    });
                }
            }
        });
    if relist {
        app.list_files();
    }
    if let Some(p) = picked {
        open = false;
        let name = p.rsplit(['/', '\\']).next().unwrap_or(&p).to_string();
        send_image(app, target, None, Some(p), &name);
    }
    if !open {
        app.cmd.rich.choose_image = None;
    }
}

fn scaling_combo(ui: &mut Ui, l: Lang, salt: &str, v: &mut String) {
    egui::ComboBox::from_id_salt(salt)
        .selected_text(match v.as_str() {
            "fit" => l.t("Fit", "Ajuster"),
            "fill" => l.t("Fill", "Remplir"),
            _ => l.t("Normal", "Normal"),
        })
        .show_ui(ui, |ui| {
            for (k, en, fr) in [
                ("normal", "Normal", "Normal"),
                ("fit", "Fit", "Ajuster"),
                ("fill", "Fill", "Remplir"),
            ] {
                ui.selectable_value(v, k.to_string(), l.t(en, fr));
            }
        });
}

fn image_window(app: &mut App, ctx: &egui::Context) {
    let Some(mut f) = app.cmd.rich.image.clone() else {
        return;
    };
    let l = app.lang;
    let mut open = true;
    let (mut apply, mut replace) = (false, false);
    egui::Window::new(l.t("Image", "Image"))
        .id(Id::new("bicraft-image-options"))
        .open(&mut open)
        .collapsible(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, -20.0))
        .resizable(false)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(l.t("Scaling", "Mise à l'échelle"));
                scaling_combo(ui, l, "bicraft-image-scaling", &mut f.scaling);
            });
            ui.horizontal(|ui| {
                replace = ui.button(l.t("Replace image…", "Remplacer l'image…")).clicked();
                apply = ui
                    .button(l.t("Apply scaling", "Appliquer la mise à l'échelle"))
                    .clicked();
            });
        });
    if replace {
        pick_image(app, ImageTarget::Replace(f.sel.clone()));
    }
    if apply {
        run(
            app,
            "image.set",
            json!({"page": f.sel.page, "visual": f.sel.visual, "scaling": f.scaling}),
            Step::Committed,
        );
        open = false;
    }
    app.cmd.rich.image = open.then_some(f);
}

fn pages(app: &App, kind: Option<&str>) -> Vec<(String, String)> {
    let Some(l) = app.loaded.as_ref() else {
        return Vec::new();
    };
    l.snap
        .pointer("/report/pages")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .take(2000)
        .filter(|p| kind.is_none_or(|k| p.get("page_type").and_then(Value::as_str) == Some(k)))
        .filter_map(|p| {
            Some((
                p.get("name")?.as_str()?.to_string(),
                p.get("display_name")?.as_str()?.to_string(),
            ))
        })
        .collect()
}

fn color_row(ui: &mut Ui, label: &str, c: &mut [u8; 3]) {
    ui.label(label);
    ui.horizontal(|ui| {
        let mut col = Color32::from_rgb(c[0], c[1], c[2]);
        if ui.color_edit_button_srgba(&mut col).changed() {
            *c = [col.r(), col.g(), col.b()];
        }
        ui.label(hex(*c));
    });
    ui.end_row();
}

fn action_window(app: &mut App, ctx: &egui::Context) {
    let Some(mut f) = app.cmd.rich.action.clone() else {
        return;
    };
    let l = app.lang;
    let all_pages = pages(app, None);
    let drill_pages = pages(app, Some("drillthrough"));
    let bookmarks = bookmark_list(app);
    let mut open = true;
    let (mut apply, mut style) = (false, false);
    egui::Window::new(l.t("Action", "Action"))
        .id(Id::new("bicraft-action-editor"))
        .open(&mut open)
        .collapsible(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, -20.0))
        .resizable(false)
        .show(ctx, |ui| {
            egui::Grid::new("bicraft-action-grid")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label(l.t("Type", "Type"));
                    let label = |k: &str| {
                        ACTION_KINDS
                            .iter()
                            .find(|x| x.0 == k)
                            .map_or(l.t("None", "Aucune"), |x| l.t(x.1, x.2))
                    };
                    egui::ComboBox::from_id_salt("bicraft-action-type")
                        .selected_text(label(&f.kind))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut f.kind, "none".to_string(), l.t("None", "Aucune"));
                            for (k, en, fr) in ACTION_KINDS {
                                ui.selectable_value(&mut f.kind, (*k).to_string(), l.t(en, fr));
                            }
                        });
                    ui.end_row();
                    // fx: the destination comes from a field's value (Microsoft Learn, "Set the
                    // page navigation destination conditionally")
                    let takes_fx = matches!(
                        f.kind.as_str(),
                        "page_navigation" | "drillthrough" | "bookmark" | "web_url"
                    );
                    if takes_fx {
                        ui.label("");
                        ui.checkbox(
                            &mut f.fx,
                            l.t(
                                "fx  Conditional destination (based on a field)",
                                "fx  Destination conditionnelle (selon un champ)",
                            ),
                        );
                        ui.end_row();
                    }
                    match f.kind.as_str() {
                        _ if takes_fx && f.fx => {
                            ui.label(l.t("Based on field", "Selon le champ"));
                            ui.add(
                                egui::TextEdit::singleline(&mut f.field)
                                    .hint_text("[Measure] / Table[Column]"),
                            );
                            ui.end_row();
                            ui.label("");
                            ui.label(
                                RichText::new(l.t(
                                    "Its value names the page, bookmark or URL (exact match).",
                                    "Sa valeur nomme la page, le signet ou l'URL (correspondance exacte).",
                                ))
                                .small()
                                .weak(),
                            );
                            ui.end_row();
                        }
                        "page_navigation" | "drillthrough" => {
                            ui.label(l.t("Destination", "Destination"));
                            let list = if f.kind == "drillthrough" {
                                &drill_pages
                            } else {
                                &all_pages
                            };
                            let shown =
                                list.iter().find(|(n, _)| *n == f.destination).map_or(
                                    l.t("Choose a page", "Choisir une page").to_string(),
                                    |(_, d)| d.clone(),
                                );
                            egui::ComboBox::from_id_salt("bicraft-action-dest")
                                .selected_text(shown)
                                .show_ui(ui, |ui| {
                                    if list.is_empty() {
                                        ui.label(l.t("No drillthrough page", "Aucune page d'extraction"));
                                    }
                                    for (n, d) in list {
                                        ui.selectable_value(&mut f.destination, n.clone(), d);
                                    }
                                });
                            ui.end_row();
                        }
                        "bookmark" => {
                            ui.label(l.t("Bookmark", "Signet"));
                            let shown = bookmarks.iter().find(|(n, _)| *n == f.bookmark).map_or(
                                if f.bookmark.is_empty() {
                                    l.t("Choose a bookmark", "Choisir un signet").to_string()
                                } else {
                                    f.bookmark.clone()
                                },
                                |(_, d)| d.clone(),
                            );
                            egui::ComboBox::from_id_salt("bicraft-action-bookmark")
                                .selected_text(shown)
                                .show_ui(ui, |ui| {
                                    if bookmarks.is_empty() {
                                        ui.label(l.t("No bookmark", "Aucun signet"));
                                    }
                                    for (n, d) in &bookmarks {
                                        ui.selectable_value(&mut f.bookmark, n.clone(), d);
                                    }
                                });
                            ui.end_row();
                        }
                        "web_url" => {
                            ui.label(l.t("Web URL", "URL web"));
                            ui.text_edit_singleline(&mut f.url);
                            ui.end_row();
                        }
                        _ => {}
                    }
                    ui.label(l.t("Tooltip", "Info-bulle"));
                    ui.text_edit_singleline(&mut f.tooltip);
                    ui.end_row();
                    if f.kind == "drillthrough" {
                        ui.label(l.t("Enabled tooltip", "Info-bulle si actif"));
                        ui.text_edit_singleline(&mut f.enabled_tooltip);
                        ui.end_row();
                        ui.label(l.t("Disabled tooltip", "Info-bulle si désactivé"));
                        ui.text_edit_singleline(&mut f.disabled_tooltip);
                        ui.end_row();
                    }
                });
            apply = ui.button(l.t("Apply action", "Appliquer l'action")).clicked();
            let shape = f.visual_type.eq_ignore_ascii_case("shape");
            if f.visual_type.eq_ignore_ascii_case("actionButton") || shape {
                ui.separator();
                ui.label(
                    RichText::new(if shape {
                        l.t("Shape style", "Style de la forme")
                    } else {
                        l.t("Button style", "Style du bouton")
                    })
                    .strong(),
                );
                egui::Grid::new("bicraft-button-style")
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label(l.t("State", "État"));
                        let states = [
                            ("default", "Default", "Par défaut"),
                            ("hover", "On hover", "Au survol"),
                            ("press", "On press", "Au clic"),
                            ("disabled", "Disabled", "Désactivé"),
                        ];
                        let shown = states
                            .iter()
                            .find(|(k, ..)| *k == f.state)
                            .map_or(f.state.as_str(), |(_, en, fr)| l.t(en, fr))
                            .to_string();
                        egui::ComboBox::from_id_salt("bicraft-button-state")
                            .selected_text(shown)
                            .show_ui(ui, |ui| {
                                for (k, en, fr) in states {
                                    // a shape has no disabled state (no action that can fail)
                                    if shape && k == "disabled" {
                                        continue;
                                    }
                                    ui.selectable_value(&mut f.state, k.to_string(), l.t(en, fr));
                                }
                            });
                        ui.end_row();
                        ui.label(l.t("Text", "Texte"));
                        ui.text_edit_singleline(&mut f.text);
                        ui.end_row();
                        color_row(ui, l.t("Font colour", "Couleur de police"), &mut f.font_color);
                        color_row(ui, l.t("Fill", "Remplissage"), &mut f.fill);
                        ui.label(l.t("Fill transparency", "Transparence du remplissage"));
                        ui.add(egui::Slider::new(&mut f.fill_transparency, 0.0..=100.0).suffix(" %"));
                        ui.end_row();
                        ui.label(l.t("Border", "Bordure"));
                        ui.checkbox(&mut f.outline, l.t("Show", "Afficher"));
                        ui.end_row();
                        if f.outline {
                            color_row(
                                ui,
                                l.t("Border colour", "Couleur de la bordure"),
                                &mut f.outline_color,
                            );
                            ui.label(l.t("Border width", "Épaisseur de la bordure"));
                            ui.add(egui::Slider::new(&mut f.outline_weight, 0.0..=20.0).suffix(" px"));
                            ui.end_row();
                        }
                    });
                style = ui.button(l.t("Apply style", "Appliquer le style")).clicked();
            }
        });
    if apply {
        let mut p = json!({"page": f.sel.page, "visual": f.sel.visual, "type": f.kind});
        let takes_fx = matches!(
            f.kind.as_str(),
            "page_navigation" | "drillthrough" | "bookmark" | "web_url"
        );
        match f.kind.as_str() {
            _ if takes_fx && f.fx => {
                if !f.field.trim().is_empty() {
                    p["destination_field"] = json!(f.field.trim());
                }
            }
            "page_navigation" | "drillthrough" if !f.destination.is_empty() => {
                p["destination"] = json!(f.destination)
            }
            "bookmark" if !f.bookmark.trim().is_empty() => p["bookmark"] = json!(f.bookmark.trim()),
            "web_url" if !f.url.trim().is_empty() => p["url"] = json!(f.url.trim()),
            _ => {}
        }
        if !f.tooltip.is_empty() {
            p["tooltip"] = json!(f.tooltip);
        }
        if f.kind == "drillthrough" {
            for (k, t) in [
                ("enabled_tooltip", &f.enabled_tooltip),
                ("disabled_tooltip", &f.disabled_tooltip),
            ] {
                if !t.is_empty() {
                    p[k] = json!(t);
                }
            }
        }
        run(app, "visual.set_action", p, Step::Committed);
    }
    if style {
        let st = f.state.clone();
        let o = |object: &str, property: &str, value: Value| json!({"object": object, "property": property, "value": value, "state": st});
        let mut objects = vec![
            o("text", "show", json!(true)),
            o("text", "text", json!(f.text)),
            o("text", "fontColor", json!({"color": hex(f.font_color)})),
            o("fill", "show", json!(true)),
            o("fill", "fillColor", json!({"color": hex(f.fill)})),
            o("fill", "transparency", json!(f.fill_transparency.round())),
            o("outline", "show", json!(f.outline)),
        ];
        if f.outline {
            objects.push(o("outline", "lineColor", json!({"color": hex(f.outline_color)})));
            objects.push(o("outline", "weight", json!(f.outline_weight.round())));
        }
        run(
            app,
            "visual.format",
            json!({"page": f.sel.page, "visual": f.sel.visual, "objects": objects}),
            Step::Committed,
        );
    }
    app.cmd.rich.action = open.then_some(f);
}

fn page_window(app: &mut App, ctx: &egui::Context) {
    let Some(mut f) = app.cmd.rich.page_format.clone() else {
        return;
    };
    let l = app.lang;
    let mut open = true;
    let mut apply = false;
    let mut pick: Option<bool> = None;
    let mut remove: Option<bool> = None;
    egui::Window::new(l.t("Page format", "Format de la page"))
        .id(Id::new("bicraft-page-format"))
        .open(&mut open)
        .collapsible(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, -20.0))
        .resizable(false)
        .show(ctx, |ui| {
            for background in [true, false] {
                let (title, color, tr, sc) = if background {
                    (
                        l.t("Canvas background", "Arrière-plan du canevas"),
                        &mut f.bg,
                        &mut f.bg_transparency,
                        &mut f.bg_scaling,
                    )
                } else {
                    (
                        l.t("Wallpaper", "Papier peint"),
                        &mut f.wp,
                        &mut f.wp_transparency,
                        &mut f.wp_scaling,
                    )
                };
                ui.label(RichText::new(title).strong());
                egui::Grid::new(("bicraft-page-layer", background))
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label(l.t("Colour", "Couleur"));
                        ui.horizontal(|ui| {
                            let mut c = color.map_or(Color32::WHITE, |c| Color32::from_rgb(c[0], c[1], c[2]));
                            if ui.color_edit_button_srgba(&mut c).changed() {
                                *color = Some([c.r(), c.g(), c.b()]);
                            }
                            ui.label(color.map_or_else(|| l.t("default", "par défaut").to_string(), hex));
                            if color.is_some() && ui.small_button(l.t("Default", "Par défaut")).clicked() {
                                *color = None;
                            }
                        });
                        ui.end_row();
                        ui.label(l.t("Transparency", "Transparence"));
                        ui.add(egui::Slider::new(tr, 0.0..=100.0).suffix(" %"));
                        ui.end_row();
                        ui.label(l.t("Image", "Image"));
                        ui.horizontal(|ui| {
                            if ui.button(l.t("Browse…", "Parcourir…")).clicked() {
                                pick = Some(background);
                            }
                            if ui.button(l.t("Remove", "Retirer")).clicked() {
                                remove = Some(background);
                            }
                        });
                        ui.end_row();
                        ui.label(l.t("Image fit", "Ajustement de l'image"));
                        scaling_combo(
                            ui,
                            l,
                            if background {
                                "bicraft-bg-fit"
                            } else {
                                "bicraft-wp-fit"
                            },
                            sc,
                        );
                        ui.end_row();
                    });
                ui.separator();
            }
            ui.label(RichText::new(l.t("Canvas settings", "Paramètres du canevas")).strong());
            egui::Grid::new("bicraft-page-size")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label(l.t("Type", "Type"));
                    egui::ComboBox::from_id_salt("bicraft-page-size-type")
                        .selected_text(
                            SIZES_PAGE
                                .iter()
                                .find(|x| x.0 == f.size)
                                .map_or("16:9", |x| l.t(x.1, x.2)),
                        )
                        .show_ui(ui, |ui| {
                            for (k, en, fr) in SIZES_PAGE {
                                ui.selectable_value(&mut f.size, (*k).to_string(), l.t(en, fr));
                            }
                        });
                    ui.end_row();
                    if f.size == "custom" {
                        ui.label(l.t("Width (px)", "Largeur (px)"));
                        ui.add(egui::DragValue::new(&mut f.width).range(1.0..=20_000.0));
                        ui.end_row();
                        ui.label(l.t("Height (px)", "Hauteur (px)"));
                        ui.add(egui::DragValue::new(&mut f.height).range(1.0..=20_000.0));
                        ui.end_row();
                    }
                    ui.label(l.t("Vertical alignment", "Alignement vertical"));
                    egui::ComboBox::from_id_salt("bicraft-page-valign")
                        .selected_text(if f.valign == "middle" {
                            l.t("Middle", "Milieu")
                        } else {
                            l.t("Top", "Haut")
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut f.valign, "top".to_string(), l.t("Top", "Haut"));
                            ui.selectable_value(&mut f.valign, "middle".to_string(), l.t("Middle", "Milieu"));
                        });
                    ui.end_row();
                });
            apply = ui
                .button(l.t("Apply page format", "Appliquer le format"))
                .clicked();
        });
    let page = f.page.clone();
    if let Some(background) = pick {
        pick_image(
            app,
            ImageTarget::Page {
                page: page.clone(),
                background,
            },
        );
    }
    if let Some(background) = remove {
        let k = if background {
            "background_image"
        } else {
            "wallpaper_image"
        };
        let mut p = json!({"page": page});
        p[k] = Value::Null;
        run(app, "page.format", p, Step::Committed);
    }
    if apply {
        if let Some(p) = page_changes(&f) {
            run(app, "page.format", p, Step::Committed);
        }
        open = false;
    }
    app.cmd.rich.page_format = open.then_some(f);
}

/// `page.format`'s parameters for what the window changed (`None`: nothing).
pub fn page_changes(f: &PageForm) -> Option<Value> {
    let start = f.start.as_ref().clone().unwrap_or_else(|| f.clone());
    let mut p = serde_json::Map::new();
    if f.bg != start.bg {
        p.insert(
            "background_color".into(),
            f.bg.map_or(Value::Null, |c| json!(hex(c))),
        );
    }
    if (f.bg_transparency - start.bg_transparency).abs() > 0.01 {
        p.insert("background_transparency".into(), json!(f.bg_transparency.round()));
    }
    if f.bg_scaling != start.bg_scaling {
        p.insert("background_scaling".into(), json!(f.bg_scaling));
    }
    if f.wp != start.wp {
        p.insert(
            "wallpaper_color".into(),
            f.wp.map_or(Value::Null, |c| json!(hex(c))),
        );
    }
    if (f.wp_transparency - start.wp_transparency).abs() > 0.01 {
        p.insert("wallpaper_transparency".into(), json!(f.wp_transparency.round()));
    }
    if f.wp_scaling != start.wp_scaling {
        p.insert("wallpaper_scaling".into(), json!(f.wp_scaling));
    }
    if f.size == "custom" {
        if f.size != start.size
            || (f.width - start.width).abs() > 0.01
            || (f.height - start.height).abs() > 0.01
        {
            p.insert("size".into(), json!("custom"));
            p.insert("width".into(), json!(f.width.round()));
            p.insert("height".into(), json!(f.height.round()));
        }
    } else if f.size != start.size {
        p.insert("size".into(), json!(f.size));
    }
    if f.valign != start.valign {
        p.insert("vertical_alignment".into(), json!(f.valign));
    }
    if p.is_empty() {
        return None;
    }
    p.insert("page".into(), json!(f.page));
    Some(Value::Object(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(parse_hex("#118dff"), Some([0x11, 0x8d, 0xff]));
        assert_eq!(parse_hex("#fff"), Some([255, 255, 255]));
        assert_eq!(parse_hex("red"), None);
    }

    #[test]
    fn page_form_sends_only_what_changed() {
        let mut f = PageForm {
            page: "p".into(),
            bg: None,
            bg_transparency: 0.0,
            bg_scaling: "normal".into(),
            wp: None,
            wp_transparency: 0.0,
            wp_scaling: "normal".into(),
            size: "16:9".into(),
            width: 1280.0,
            height: 720.0,
            valign: "top".into(),
            start: Box::new(None),
        };
        *f.start = Some(f.clone());
        assert_eq!(page_changes(&f), None);
        f.bg = Some([0xff, 0, 0]);
        f.size = "4:3".into();
        assert_eq!(
            page_changes(&f),
            Some(json!({"page": "p", "background_color": "#FF0000", "size": "4:3"}))
        );
    }
}
