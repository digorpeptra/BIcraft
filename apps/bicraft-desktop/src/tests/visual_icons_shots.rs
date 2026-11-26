//! Screenshots of the Visualizations pane's gallery with BIcraft's fourteen visuals, each with its own
//! drawing (ROADMAP M12.4), next to the built-in types, drawn with wgpu (`--features gpu-tests`),
//! saved into `BICRAFT_DESKTOP_SHOTS` when set: the whole window and the gallery cropped (and
//! enlarged three times), in light and dark, with a BIcraft Venn selected (the selected state).

use std::path::PathBuf;

use bicraft_engine::Session;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::{Temp, build_shop, ready, wait};
use crate::App;

/// The cells' labels, in the registry's order.
const LABELS: [&str; 12] = [
    "BIcraft Spec",
    "BIcraft Histogram",
    "BIcraft Bullet",
    "BIcraft Box Plot",
    "BIcraft Dumbbell",
    "BIcraft Org Chart",
    "BIcraft Venn",
    "BIcraft Marimekko",
    "BIcraft Calendar Heatmap",
    "BIcraft Sankey",
    "BIcraft Scatter+",
    "BIcraft Line / Area+",
];

const PPP: f32 = 2.0;

fn save(dir: &Option<PathBuf>, name: &str, pm: &tiny_skia::Pixmap) {
    let Some(dir) = dir else { return };
    std::fs::create_dir_all(dir).unwrap();
    pm.save_png(dir.join(format!("{name}.png"))).unwrap();
}

#[test]
fn visual_icon_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("visual-icons-shots");
    // the shop with a BIcraft Venn on its first page
    let mut s = Session::new();
    let names = build_shop(&mut s);
    let venn = s
        .execute(
            "visual.add",
            &json!({"page": names.page, "visual_type": "bicraftVenn", "x": 880, "y": 160, "width": 300, "height": 260}),
        )
        .unwrap()["visual"]
        .as_str()
        .unwrap()
        .to_string();
    let path = dir.path().join("icons.pbip");
    s.execute("file.save_as", &json!({"path": path})).unwrap();
    for (suffix, b) in [
        ("light", bicraft_ui::theme::Brightness::Light),
        ("dark", bicraft_ui::theme::Brightness::Dark),
    ] {
        let mut app = crate::app("en-US", None).with_sync_render();
        app.set_theme(b);
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1440.0, 1000.0))
            .with_pixels_per_point(PPP)
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
        wait(&mut h, "the first snapshot", ready);
        h.state_mut().prefs.panes.build_width = 300.0;
        h.state_mut().cmd.visual = Some(bicraft_ui::ribbon::VisualSel {
            page: names.page.clone(),
            visual: venn.clone(),
        });
        for _ in 0..10 {
            h.step();
        }
        wait(&mut h, "visual.props", |a| a.calls_pending() == 0);
        // each of the ten cells is there, once, with its label
        let rects: Vec<egui::Rect> = LABELS
            .iter()
            .map(|l| {
                let mut all = h.get_all_by_label(l);
                let r = all.next().unwrap_or_else(|| panic!("no cell {l}")).rect();
                assert!(all.next().is_none(), "{l}: one cell");
                r
            })
            .collect();
        // the pointer away from the gallery (no tooltip over the cells)
        h.event(egui::Event::PointerMoved(egui::pos2(700.0, 900.0)));
        for _ in 0..6 {
            h.step();
        }
        let img = h.render().unwrap();
        let size = tiny_skia::IntSize::from_wh(img.width(), img.height()).unwrap();
        let full = tiny_skia::Pixmap::from_vec(img.as_raw().clone(), size).unwrap();
        save(&shots, &format!("visual-icons-{suffix}-window"), &full);
        // the gallery: the ten cells, the "…" cell and two rows of built-in types above them
        let more = h.get_by_label("More visuals").rect();
        let mut area = rects.iter().fold(more, |a, r| a.union(*r));
        area.min.y -= 64.0;
        let area = area.expand(6.0);
        let (x0, y0) = (
            (area.min.x * PPP).max(0.0) as i32,
            (area.min.y * PPP).max(0.0) as i32,
        );
        let (w, h_px) = ((area.width() * PPP) as u32, (area.height() * PPP) as u32);
        let crop = full
            .clone_rect(tiny_skia::IntRect::from_xywh(x0, y0, w, h_px).unwrap())
            .unwrap();
        save(&shots, &format!("visual-icons-{suffix}-gallery"), &crop);
        let mut big = tiny_skia::Pixmap::new(w * 3, h_px * 3).unwrap();
        big.draw_pixmap(
            0,
            0,
            crop.as_ref(),
            &tiny_skia::PixmapPaint {
                quality: tiny_skia::FilterQuality::Nearest,
                ..Default::default()
            },
            tiny_skia::Transform::from_scale(3.0, 3.0),
            None,
        );
        save(&shots, &format!("visual-icons-{suffix}-gallery-x3"), &big);
        // the ten cells' pixels differ from one another
        let cell = |r: egui::Rect| {
            let r = r.shrink(3.0);
            full.clone_rect(
                tiny_skia::IntRect::from_xywh(
                    (r.min.x * PPP) as i32,
                    (r.min.y * PPP) as i32,
                    (r.width() * PPP) as u32,
                    (r.height() * PPP) as u32,
                )
                .unwrap(),
            )
            .unwrap()
            .data()
            .to_vec()
        };
        let cells: Vec<Vec<u8>> = rects.iter().map(|r| cell(*r)).collect();
        for (i, a) in cells.iter().enumerate() {
            for (j, b) in cells.iter().enumerate().skip(i + 1) {
                assert_ne!(a, b, "{} and {} look the same", LABELS[i], LABELS[j]);
            }
        }
    }
}
