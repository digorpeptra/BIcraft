//! File › Options and settings (the File view's Options page): the current file's Report settings,
//! as in Desktop's Options › Current file › Report settings. Each control runs
//! `report.settings` with the one value it changes; the page reads the values back from the
//! snapshot's `report.settings` (docs/view-snapshot.md), so it always shows the file's state.

use egui::{RichText, Ui};
use serde_json::{Value, json};

use crate::app::{App, Then};
use crate::i18n::Lang;
use crate::theme::{self, Tokens};

/// The command every control of the page runs.
pub const COMMAND: &str = "report.settings";

/// The id of the page's output ([`Then::Output`]).
const OUTPUT: &str = "options";

/// One boolean of `report.settings`: its parameter, its label, and whether the checkbox shows the
/// opposite of the value (Desktop's "Don't allow…" for persistent filters).
struct Flag {
    param: &'static str,
    label: (&'static str, &'static str),
    inverted: bool,
}

const FILTERS: [Flag; 1] = [Flag {
    param: "persistent_filters",
    label: (
        "Don't allow end users to save filters on this file in the Power BI service",
        "Ne pas autoriser les utilisateurs finaux à enregistrer les filtres de ce fichier dans le service Power BI",
    ),
    inverted: true,
}];

const VISUALS: [Flag; 3] = [
    Flag {
        param: "personalize_visuals",
        label: ("Personalize visuals", "Personnaliser les visuels"),
        inverted: false,
    },
    Flag {
        param: "drill_filters_other_visuals",
        label: (
            "Drilling filters other visuals",
            "L'exploration filtre les autres visuels",
        ),
        inverted: false,
    },
    Flag {
        param: "cross_report_drillthrough",
        label: ("Cross-report drillthrough", "Extraction entre rapports"),
        inverted: false,
    },
];

const FILTERING: [Flag; 2] = [
    Flag {
        param: "allow_change_filter_types",
        label: (
            "Allow users to change filter types",
            "Autoriser les utilisateurs à changer les types de filtre",
        ),
        inverted: false,
    },
    Flag {
        param: "filter_pane_visible",
        label: (
            "Show the Filters pane to report readers",
            "Afficher le volet Filtres aux lecteurs du rapport",
        ),
        inverted: false,
    },
];

/// The snapshot's `report.settings`.
fn settings(app: &App) -> Value {
    app.loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/report/settings"))
        .cloned()
        .unwrap_or(Value::Null)
}

fn section(ui: &mut Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(10.0);
    ui.label(RichText::new(text).font(theme::bold(14.0)).color(t.text));
    ui.add_space(4.0);
}

fn dim(ui: &mut Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(text).font(theme::font(12.5)).color(t.text_dim));
}

/// The checkboxes of `flags`; the parameters of the one clicked.
fn flags(ui: &mut Ui, l: Lang, st: &Value, flags: &[Flag], enabled: bool) -> Option<Value> {
    let mut out = None;
    for f in flags {
        let value = st.get(f.param).and_then(Value::as_bool).unwrap_or(false);
        let mut checked = value != f.inverted;
        let r = ui.add_enabled(
            enabled,
            egui::Checkbox::new(&mut checked, l.t(f.label.0, f.label.1)),
        );
        if r.changed() {
            out = Some(json!({ f.param: checked != f.inverted }));
        }
    }
    out
}

/// The Options page.
pub fn show(app: &mut App, ui: &mut Ui) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let Some(_) = app.loaded.as_ref() else {
        ui.spinner();
        return;
    };
    if app
        .loaded
        .as_ref()
        .and_then(|x| x.snap.get("report"))
        .is_none_or(Value::is_null)
    {
        dim(
            ui,
            l.t("This file has no report.", "Ce fichier n'a pas de rapport."),
        );
        return;
    }
    ui.label(
        RichText::new(l.t("Report settings", "Paramètres du rapport"))
            .font(theme::bold(16.0))
            .color(t.text),
    );
    dim(
        ui,
        l.t(
            "Settings of the current file, saved with its report.",
            "Paramètres du fichier actif, enregistrés avec son rapport.",
        ),
    );
    let can = crate::ribbon::state(app, COMMAND);
    let on = can.is_ok();
    let st = settings(app);
    let mut run: Option<Value> = None;
    section(ui, l.t("Persistent filters", "Filtres persistants"));
    run = run.or(flags(ui, l, &st, &FILTERS, on));
    section(ui, l.t("Visual options", "Options des visuels"));
    // the default interaction is a choice of two, shown as Desktop's checkbox
    let mut cross = st.get("default_interaction").and_then(Value::as_str) == Some("filter");
    let r = ui.add_enabled(
        on,
        egui::Checkbox::new(
            &mut cross,
            l.t(
                "Change default visual interaction from cross highlighting to cross filtering",
                "Remplacer l'interaction par défaut des visuels (sélection croisée) par le filtrage croisé",
            ),
        ),
    );
    if r.changed() {
        run = Some(json!({"default_interaction": if cross { "filter" } else { "highlight" }}));
    }
    run = run.or(flags(ui, l, &st, &VISUALS, on));
    section(ui, l.t("Filtering experience", "Expérience de filtrage"));
    run = run.or(flags(ui, l, &st, &FILTERING, on));
    if let Err(why) = &can {
        ui.add_space(6.0);
        dim(ui, &format!("{} {why}", l.t("Unavailable:", "Indisponible :")));
    }
    if let Some(p) = run {
        app.run(crate::actions::report_settings(p), Then::Output(OUTPUT));
    }
    // the Optimize ribbon's Customize: the query reduction options
    crate::optimize_ui::query_reduction_section(app, ui);
}
