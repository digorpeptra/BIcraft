//! Auto-recovery in the desktop shell (docs/recovery.md): edits journaled by the window's own
//! engine source, a crash (the session dropped without a save), the start screen listing the
//! file, and the prompt recovering the edits after the file is opened again.

use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::{Temp, harness, shop, wait};

#[test]
fn a_crash_leaves_edits_the_next_window_recovers() {
    let dir = Temp::new("recovery");
    let (path, _) = shop(dir.path());
    let rec = dir.path().join("recovery");
    // the first window edits the project, then goes without a save
    {
        let mut app = crate::app_with("en-US", None, Some(rec.clone()));
        app.open_path(&path);
        let mut h = harness(app, (1280.0, 800.0));
        wait(&mut h, "the file", |a| a.loaded.is_some());
        let table = h.state().loaded.as_ref().unwrap().snap["model"]["tables"][0]["name"]
            .as_str()
            .unwrap()
            .to_string();
        h.state_mut().run(
            bicraft_ui::actions::Call {
                id: "measure.add",
                params: json!({"table": table, "name": "Kept", "expression": "1"}),
            },
            bicraft_ui::app::Then::Nothing,
        );
        wait(&mut h, "the edit", |a| {
            a.loaded
                .as_ref()
                .is_some_and(|l| l.snap.pointer("/recovery/edits") == Some(&json!(1)))
        });
        h.get_by_label("Recovery: 1 edit");
    }
    // the window's source thread lets its session go when the window does
    let cfg = bicraft_engine::RecoveryConfig::new(&rec);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while bicraft_engine::recovery_offers(&cfg).is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "the journal was never released"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    // the next start: File > Home lists the file; opening it shows the prompt
    let app = crate::app_with("en-US", None, Some(rec.clone()));
    let mut h = harness(app, (1280.0, 800.0));
    wait(&mut h, "the start screen", |a| a.backstage.is_some());
    h.get_by_label("Recover unsaved changes");
    h.get_by_label("shop.pbip");
    h.get_all_by_label("Open")
        .max_by(|a, b| a.rect().min.x.total_cmp(&b.rect().min.x))
        .unwrap()
        .click();
    wait(&mut h, "the offer", |a| {
        a.loaded.as_ref().is_some_and(|l| {
            l.snap
                .pointer("/recovery/offers")
                .and_then(Value::as_array)
                .is_some_and(|o| o.len() == 1)
        })
    });
    h.step();
    h.get_by_label("Recover").click();
    wait(&mut h, "the recovered edit", |a| {
        a.loaded
            .as_ref()
            .is_some_and(|l| l.snap.to_string().contains("\"Kept\""))
    });
    assert_eq!(h.state().status, "Recovered 1 edit");
    assert!(bicraft_engine::recovery_offers(&cfg).is_empty());
}
