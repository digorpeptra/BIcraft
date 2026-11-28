//! The engine's command registry as the UI reads it (ROADMAP M11, AGENTS.md §9 "every command
//! has its button"): each command's id, labels, place (a ribbon tab and group, a context menu or
//! the palette), icon, shortcut, needs, server effect and parameters form, in
//! [`COMMANDS`].
//!
//! The UI crate does not link the engine (it builds for wasm32 and the browser shell has no
//! engine), so the table is a copy of `bicraft_engine::COMMANDS` in plain data, generated into
//! `registry_table.rs` and kept identical by the test `the_ui_table_is_the_engines_registry`
//! (`BICRAFT_BLESS=1` rewrites it). Both shells compile the same table, so their ribbons are the
//! same.
//!
//! Whether a button is enabled comes from the session: every snapshot carries the reasons the
//! editor's disabled commands give (`commands.disabled`, `docs/view-snapshot.md`), computed by
//! `Session::command_enabled` where the session runs (the desktop's engine, the server), and the
//! UI adds the read-only role on top ([`enabled`]).

use serde_json::Value;

use crate::i18n::Lang;
use crate::icons::Icon;

#[path = "registry_table.rs"]
#[rustfmt::skip]
mod table;

pub use table::COMMANDS;

/// A ribbon tab (the engine's `command_ui::Tab`), in tab-row order. `File` is the Backstage;
/// `Visual`, `Table` and `Measure` are contextual tabs shown with a selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tab {
    File,
    Home,
    Insert,
    Report,
    Model,
    Data,
    Analyze,
    View,
    Optimize,
    Visual,
    Table,
    Measure,
}

impl Tab {
    pub const ALL: [Tab; 12] = [
        Tab::File,
        Tab::Home,
        Tab::Insert,
        Tab::Report,
        Tab::Model,
        Tab::Data,
        Tab::Analyze,
        Tab::View,
        Tab::Optimize,
        Tab::Visual,
        Tab::Table,
        Tab::Measure,
    ];

    /// Shown only with a selection.
    pub fn contextual(self) -> bool {
        matches!(self, Tab::Visual | Tab::Table | Tab::Measure)
    }

    /// The tab row's caption.
    pub fn caption(self, l: Lang) -> &'static str {
        match self {
            Tab::File => l.t("File", "Fichier"),
            Tab::Home => l.t("Home", "Accueil"),
            Tab::Insert => l.t("Insert", "Insertion"),
            Tab::Report => l.t("Report", "Rapport"),
            Tab::Model => l.t("Model", "Modèle"),
            Tab::Data => l.t("Data", "Données"),
            Tab::Analyze => l.t("Analyze", "Analyser"),
            Tab::View => l.t("View", "Affichage"),
            Tab::Optimize => l.t("Optimize", "Optimiser"),
            Tab::Visual => l.t("Visual", "Visuel"),
            Tab::Table => l.t("Table", "Table"),
            Tab::Measure => l.t("Measure", "Mesure"),
        }
    }
}

/// A context menu (right click).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Menu {
    /// A page tab.
    Page,
    /// A visual on the canvas.
    Visual,
    /// A table, column, measure or relationship of the model diagram, or the diagram itself.
    ModelObject,
}

/// Where a command's button lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum At {
    Ribbon { tab: Tab, group: &'static str },
    Menu(Menu),
    Palette,
}

/// An object a name parameter refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Obj {
    Table,
    Column,
    Measure,
    Hierarchy,
    Partition,
    Expression,
    Relationship,
    Role,
    Culture,
    CalculationGroup,
    CalculationItem,
    Page,
    Visual,
    ReportMeasure,
    DataRole,
    VisualType,
    Profile,
}

/// The kind of value a parameter takes (the engine's `command_ui::Kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    Name(Obj),
    Field,
    Dax,
    M,
    Code,
    Path,
    Paths,
    PathOrText,
    Bool,
    Integer,
    Number,
    Enum(&'static [&'static str]),
    Color,
    List,
    Json,
}

/// One parameter of a command's form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Param {
    pub name: &'static str,
    pub kind: Kind,
    pub required: bool,
    /// What the command uses when it is left out, as JSON text (the engine's `Param::default`).
    pub default: Option<&'static str>,
}

/// What the session needs before the button is enabled (the engine's `Need`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    File,
    SavedReportFile,
    Project,
    Refreshable,
    Model,
    Report,
    NoReport,
    Data,
    Undo,
    Redo,
}

/// What a command does, for read-only callers (the engine's `sandbox::Effect`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Mutating,
    ReadOnly,
    Opens,
}

/// One engine command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cmd {
    pub id: &'static str,
    /// The engine's label (a sentence: the palette's and the tooltip's text).
    pub label: &'static str,
    /// The engine's parameters doc.
    pub params: &'static str,
    pub at: At,
    /// The button's English text.
    pub button: &'static str,
    /// A kebab-case icon name ([`Icon::from_name`]).
    pub icon: &'static str,
    pub shortcut: Option<&'static str>,
    pub needs: &'static [Need],
    pub form: &'static [Param],
    pub effect: Effect,
}

impl Cmd {
    /// The button's text in `l`.
    pub fn text(&self, l: Lang) -> &'static str {
        match l {
            Lang::En => self.button,
            Lang::Fr => fr(self.id).unwrap_or(self.button),
        }
    }

    /// The drawing of its icon (a generic one for a name the UI does not draw).
    pub fn icon(&self) -> Icon {
        Icon::from_name(self.icon).unwrap_or(Icon::More)
    }

    /// Whether running it needs values the caller has not given: a required parameter missing
    /// from `params`.
    pub fn needs_values(&self, params: &Value) -> bool {
        self.form
            .iter()
            .any(|p| p.required && params.get(p.name).is_none_or(Value::is_null))
    }

    /// Its shortcut in a shell: a browser that keeps the registry's chord for itself gets
    /// [`BROWSER_CHORDS`]' instead (`browser`: [`crate::shell::Caps::reserved_keys`]).
    pub fn shortcut_for(&self, browser: bool) -> Option<&'static str> {
        let k = self.shortcut?;
        if browser && let Some((_, b)) = BROWSER_CHORDS.iter().find(|(d, _)| *d == k) {
            return Some(b);
        }
        Some(k)
    }

    /// The tooltip: what it does, its shortcut (the browser's chord in a browser), and why it is
    /// disabled.
    pub fn tooltip(&self, l: Lang, state: &Result<(), String>, browser: bool) -> String {
        let mut s = self.label.to_string();
        if let Some(k) = self.shortcut_for(browser) {
            s.push_str(&format!(" ({k})"));
        }
        s.push_str(&format!("\n{}", self.id));
        if let Err(why) = state {
            s.push_str(&format!("\n{} {why}", l.t("Unavailable:", "Indisponible :")));
        }
        s
    }
}

/// The registry's chords a browser never hands to a page (Ctrl+N opens a browser window), with
/// the chord the app takes instead there (docs/webui.md, "Keyboard in the browser"). F5 is not
/// here: the browser shell stops the browser's reload, so F5 stays F5.
pub const BROWSER_CHORDS: &[(&str, &str)] = &[("Ctrl+N", "Alt+N")];

/// The command `id`.
pub fn find(id: &str) -> Option<&'static Cmd> {
    COMMANDS.iter().find(|c| c.id == id)
}

/// The commands of ribbon tab `tab`, grouped in table order: (group, commands).
pub fn ribbon_groups(tab: Tab) -> Vec<(&'static str, Vec<&'static Cmd>)> {
    let mut out: Vec<(&'static str, Vec<&'static Cmd>)> = Vec::new();
    for c in COMMANDS {
        if let At::Ribbon { tab: t, group } = c.at
            && t == tab
        {
            match out.iter_mut().find(|(g, _)| *g == group) {
                Some((_, v)) => v.push(c),
                None => out.push((group, vec![c])),
            }
        }
    }
    out
}

/// The commands of context menu `menu`, in table order.
pub fn menu_commands(menu: Menu) -> Vec<&'static Cmd> {
    COMMANDS.iter().filter(|c| c.at == At::Menu(menu)).collect()
}

/// What the UI knows of the session to decide whether a command can run.
#[derive(Debug, Clone, Copy)]
pub struct Context<'a> {
    /// The active document's snapshot.
    pub snap: Option<&'a Value>,
    /// A document is open (its source runs).
    pub open: bool,
    /// The caller is a read-only reader of a server workspace.
    pub reader: bool,
}

/// Whether command `id` can run now (`Ok`), or why not. The snapshot's `commands.disabled`
/// (the session's own answer, `Session::command_enabled` for an editor) decides; a snapshot
/// without it (an older server) falls back on the needs the snapshot shows. A reader is refused
/// what changes the session, as the server would.
pub fn enabled(id: &str, cx: Context<'_>) -> Result<(), String> {
    let Some(cmd) = find(id) else {
        return Err(format!("unknown command `{id}`"));
    };
    if !cx.open {
        return Err("open a file first".to_string());
    }
    if cx.reader {
        match cmd.effect {
            Effect::Mutating => return Err("read-only access: this command changes the session".into()),
            Effect::Opens if dirty(cx.snap) => {
                return Err("read-only access: the open file has unsaved changes".into());
            }
            _ => {}
        }
    }
    let Some(snap) = cx.snap else {
        return Err("the file is still opening".into());
    };
    if let Some(map) = snap.pointer("/commands/disabled").and_then(Value::as_object) {
        return match map.get(id) {
            Some(why) => Err(why
                .as_str()
                .unwrap_or("not available")
                .chars()
                .take(300)
                .collect()),
            None => Ok(()),
        };
    }
    match cmd.needs.iter().find(|n| !met(**n, snap)) {
        Some(n) => Err(reason(*n).to_string()),
        None => Ok(()),
    }
}

fn dirty(snap: Option<&Value>) -> bool {
    snap.is_some_and(|s| {
        ["model", "report", "queries"]
            .iter()
            .any(|k| s.pointer(&format!("/dirty/{k}")).and_then(Value::as_bool) == Some(true))
    })
}

/// Whether the snapshot shows `n` (unknown ones count as met: the engine decides).
fn met(n: Need, snap: &Value) -> bool {
    let kind = snap
        .pointer("/source/kind")
        .and_then(Value::as_str)
        .unwrap_or("none");
    match n {
        Need::File => kind != "none",
        Need::SavedReportFile => matches!(kind, "package" | "project"),
        Need::Project => kind == "project",
        Need::Refreshable => {
            kind == "project" || snap.pointer("/source/format").and_then(Value::as_str) == Some("pbix")
        }
        Need::Model => snap.get("model").is_some_and(Value::is_object),
        Need::Report => snap.get("report").is_some_and(Value::is_object),
        Need::NoReport => !snap.get("report").is_some_and(Value::is_object),
        Need::Data | Need::Undo | Need::Redo => true,
    }
}

/// The engine's reason for a missing need (`Need::reason`).
fn reason(n: Need) -> &'static str {
    match n {
        Need::File => "open a file first",
        Need::SavedReportFile => "needs a saved PBIX, PBIT or PBIP file",
        Need::Project => "needs an open PBIP project",
        Need::Refreshable => "needs an open PBIP project or PBIX file",
        Need::Model => "needs a model: open a file or start a new model",
        Need::Report => "needs a report",
        Need::NoReport => "the session already has a report",
        Need::Data => "needs stored data (a PBIX DataModel or a PBIP project's cache.abf)",
        Need::Undo => "nothing to undo",
        Need::Redo => "nothing to redo",
    }
}

/// A ribbon group's caption in `l`.
pub fn group_text(group: &'static str, l: Lang) -> &'static str {
    match l {
        Lang::En => group,
        Lang::Fr => group_fr(group).unwrap_or(group),
    }
}

/// A ribbon group's French caption.
pub fn group_fr(group: &str) -> Option<&'static str> {
    Some(match group {
        "File" => "Fichier",
        "Info" => "Informations",
        "Export" => "Exporter",
        "Compare" => "Comparer",
        "Import" => "Importer",
        "About" => "À propos",
        "Share" => "Partager",
        "Data" => "Données",
        "Calculations" => "Calculs",
        "Parameters" => "Paramètres",
        "Groups" => "Groupes",
        "Queries" => "Requêtes",
        "Undo" => "Annuler",
        "Report" => "Rapport",
        "Visuals" => "Visuels",
        "Report measures" => "Mesures du rapport",
        "Spec" => "Spécification",
        "Pages" => "Pages",
        "Properties" => "Propriétés",
        "Relationships" => "Relations",
        "Calculation groups" => "Groupes de calcul",
        "Security" => "Sécurité",
        "Layouts" => "Dispositions",
        "Translations" => "Traductions",
        "Stored data" => "Données stockées",
        "Partitions" => "Partitions",
        "Sources" => "Sources",
        "Environments" => "Environnements",
        "Storage" => "Stockage",
        "DAX" => "DAX",
        "Validate" => "Valider",
        "Usage" => "Utilisation",
        "Performance" => "Performances",
        "Pause visuals" => "Suspendre les visuels",
        "Optimization presets" => "Préréglages d'optimisation",
        "Tests" => "Tests",
        "Language" => "Langue",
        "Preview" => "Aperçu",
        "Page view" => "Affichage de la page",
        "Themes" => "Thèmes",
        "Mobile layout" => "Disposition mobile",
        "Type" => "Type",
        "Arrange" => "Organiser",
        "Format" => "Mise en forme",
        "Filters" => "Filtres",
        "Analytics" => "Analytique",
        "Interactions" => "Interactions",
        "Bookmarks" => "Signets",
        "Columns" => "Colonnes",
        "Hierarchies" => "Hiérarchies",
        "Structure" => "Structure",
        "Clipboard" => "Presse-papiers",
        "Elements" => "Éléments",
        "Sparklines" => "Graphiques sparkline",
        _ => return None,
    })
}

/// A button's French text, by command id (BIcraft's own wording, AGENTS.md §9).
pub fn fr(id: &str) -> Option<&'static str> {
    Some(match id {
        "file.open" => "Ouvrir",
        "file.save" => "Enregistrer",
        "file.save_as" => "Enregistrer sous",
        "project.new" => "Nouveau projet",
        "model.new" => "Nouveau modèle",
        "file.inspect" => "Infos du fichier",
        "file.convert" => "Convertir un fichier",
        "file.redact" => "Copie anonymisée",
        "report.render" => "Exporter en PNG",
        "report.export_pdf" => "Exporter au format PDF",
        "report.print" => "Imprimer",
        "visual.export_data" => "Exporter les données",
        "file.diff" => "Comparer des fichiers",
        "objects.import" => "Importer d'un fichier",
        "about.licenses" => "Licences",
        "file.open_in_desktop" => "Ouvrir dans Desktop",
        "source.navigate" => "Obtenir des données",
        "source.load" => "Charger les éléments du navigateur",
        "source.credentials" => "Identifiants de la source de données",
        "source.privacy" => "Niveau de confidentialité de la source",
        "source.settings" => "Paramètres de source de données",
        "table.enter_data" => "Entrer des données",
        "table.entered_data" => "Modifier les données",
        "table.import" => "Importer une table",
        "measure.add" => "Nouvelle mesure",
        "measure.quick" => "Mesure rapide",
        "model.refresh" => "Actualiser",
        "edit.undo" => "Annuler",
        "edit.redo" => "Rétablir",
        "edit.history" => "Historique",
        "report.new" => "Nouveau rapport",
        "report.custom_visuals" => "Visuels personnalisés",
        "visual.import_custom" => "Importer un visuel",
        "visual.remove_custom" => "Supprimer un visuel personnalisé",
        "visual.scaffold" => "Nouveau projet de visuel",
        "visual.build" => "Compiler un visuel",
        "visual.pack" => "Empaqueter un visuel",
        "visual.export_bicraft_visual" => "Exporter un visuel BIcraft",
        "visual.set_spec" => "Spécification de graphique",
        "visual.spec_template" => "Modèle de spécification de graphique",
        "visual.set_bicraft_visual" => "Graphique BIcraft",
        "visual.add" => "Nouveau visuel",
        "report_measure.list" => "Mesures du rapport",
        "report_measure.add" => "Nouvelle mesure du rapport",
        "report_measure.set" => "Modifier la mesure du rapport",
        "report_measure.remove" => "Supprimer la mesure du rapport",
        "report.apply" => "Appliquer une spécification",
        "report.check" => "Comparer à la spécification",
        "report.extract" => "Extraire la spécification",
        "page.add" => "Nouvelle page",
        "page.arrange" => "Disposer les visuels",
        "model.set" => "Propriétés du modèle",
        "relationship.list" => "Gérer les relations",
        "relationship.add" => "Nouvelle relation",
        "relationship.set" => "Modifier la relation",
        "relationship.autodetect" => "Détecter les relations",
        "role.set_permission" => "Gérer les rôles",
        "perspective.add" => "Nouvelle perspective",
        "perspective.set" => "Gérer les perspectives",
        "perspective.remove" => "Supprimer la perspective",
        "diagram.move" => "Déplacer la table dans le diagramme",
        "diagram.add" => "Nouvelle disposition",
        "diagram.rename" => "Renommer la disposition",
        "diagram.remove" => "Supprimer la disposition",
        "diagram.remove_tables" => "Retirer de la disposition",
        "table.add" => "Nouvelle table",
        "table.add_refresh_info" => "Table de l'actualisation",
        "parameter.what_if" => "Plage numérique",
        "parameter.fields" => "Champs",
        "parameter.fix_field" => "Corriger un champ",
        "column.group" => "Groupes de données",
        "visual.group_points" => "Grouper les données",
        "calculation_group.add" => "Nouveau groupe de calcul",
        "calculation_group.set" => "Propriétés du groupe de calcul",
        "calculation_item.add" => "Nouvel élément de calcul",
        "calculation_item.set" => "Modifier l'élément de calcul",
        "role.add" => "Nouveau rôle",
        "role.set_filter" => "Filtre du rôle",
        "culture.add" => "Nouvelle culture",
        "translation.set" => "Traduire",
        "translation.list" => "Traductions",
        "file.set_cache_abf" => "Fichier de cache des données",
        "m.validate" => "Vérifier le M",
        "m.format" => "Mettre en forme le M",
        "m.complete" => "Suggestions M",
        "m.signature" => "Aide à la signature M",
        "m.hover" => "Infos sur un nom M",
        "expression.add" => "Nouvelle requête",
        "expression.set" => "Modifier la requête",
        "function.add" => "Nouvelle fonction",
        "function.set" => "Modifier la fonction",
        "function.rename" => "Renommer la fonction",
        "function.remove" => "Supprimer la fonction",
        "tmdl.script" => "Scripter en TMDL",
        "tmdl.apply" => "Appliquer le script TMDL",
        "partition.set" => "Modifier la partition",
        "partition.add" => "Nouvelle partition",
        "partition.remove" => "Supprimer la partition",
        "sources.list" => "Sources de données",
        "sources.rebase" => "Changer les sources",
        "env.apply" => "Appliquer un environnement",
        "env.list" => "Profils d'environnement",
        "env.check" => "Vérifier l'environnement",
        "model.storage_stats" => "Statistiques de stockage",
        "model.export_vpax" => "Exporter en VPAX",
        "dax.validate" => "Vérifier le DAX",
        "dax.validate_query" => "Vérifier la requête",
        "dax.evaluate" => "Exécuter une requête DAX",
        "dax.apply_define" => "Mettre à jour le modèle",
        "dax.format" => "Mettre en forme la requête",
        "dax.define_query" => "Requête de définition des mesures",
        "dax.cancel" => "Annuler la requête",
        "dax.queries" => "Onglets de requêtes DAX",
        "dax.set_queries" => "Garder les onglets de requêtes DAX",
        "model.validate" => "Valider le modèle",
        "model.analyze" => "Bonnes pratiques",
        "report.validate" => "Valider le rapport",
        "report.repair" => "Réparer le rapport",
        "report.rebind" => "Relier le champ",
        "report.remove_reference" => "Retirer la référence",
        "model.usage" => "Où est-ce utilisé",
        "model.unused" => "Objets inutilisés",
        "model.remove_unused" => "Supprimer l'inutilisé",
        "dax.clear_cache" => "Vider le cache",
        "dax.aggregations" => "Agrégations",
        "dax.memory" => "Budget mémoire",
        "perf.results" => "Analyseur de performances",
        "perf.start" => "Démarrer l'enregistrement",
        "perf.refresh" => "Actualiser les visuels",
        "perf.stop" => "Arrêter l'enregistrement",
        "perf.clear" => "Effacer l'enregistrement",
        "perf.render" => "Enregistrer les temps de rendu",
        "perf.export" => "Exporter l'enregistrement",
        "project.test" => "Exécuter les tests",
        "view.set_language" => "Langue du texte des visuels",
        "view.set_culture" => "Afficher les traductions",
        "view.set_as_of" => "Date de référence",
        "theme.get" => "Thème actuel",
        "theme.import" => "Rechercher des thèmes",
        "theme.export" => "Enregistrer le thème actuel",
        "theme.remove" => "Réinitialiser le thème",
        "theme.set_colors" => "Couleurs du thème",
        "theme.set_text_classes" => "Classes de texte",
        "theme.set_visual_style" => "Styles des visuels",
        "theme.customize" => "Personnaliser le thème actuel",
        "visual.set_position" => "Position et taille",
        "visual.place" => "Aligner sur la grille",
        "visual.order" => "Ordre des calques",
        "visual.set_hidden" => "Afficher ou masquer",
        "visual.rename_group" => "Renommer le groupe",
        "visual.tab_order" => "Ordre de tabulation",
        "visual.mobile" => "Disposition mobile",
        "visual.mobile_format" => "Mise en forme mobile",
        "visual.mobile_auto" => "Création automatique",
        "visual.mobile_remove_all" => "Tout retirer",
        "visual.bind" => "Ajouter un champ",
        "visual.unbind" => "Retirer un champ",
        "visual.fix_query_refs" => "Corriger les queryRefs",
        "visual.move_field" => "Déplacer un champ",
        "visual.set_aggregation" => "Synthétiser",
        "visual.set_type" => "Changer de type",
        "visual.small_multiples" => "Multiples",
        "visual.set_title" => "Titre",
        "visual.format" => "Mise en forme",
        "visual.axis" => "Axes",
        "visual.tooltip_page" => "Page d'info-bulle",
        "visual.series" => "Format des séries",
        "visual.conditional" => "Mise en forme conditionnelle",
        "visual.filter" => "Filtrer",
        "visual.clear_filters" => "Effacer les filtres",
        "page.filter" => "Filtre de page",
        "page.clear_filters" => "Effacer les filtres de page",
        "report.filter" => "Filtre de rapport",
        "report.clear_filters" => "Effacer les filtres de rapport",
        "filter.order" => "Ordre des cartes de filtre",
        "visual.rename_field" => "Renommer pour ce visuel",
        "visual.show_value_as" => "Afficher la valeur comme",
        "visual.show_items_no_data" => "Afficher les éléments sans données",
        "visual.set_data_format" => "Format des données",
        "visual.add_sparkline" => "Ajouter un graphique sparkline",
        "visual.edit_sparkline" => "Modifier le graphique sparkline",
        "visual.calculation.add" => "Nouveau calcul",
        "visual.calculation.set" => "Modifier le calcul visuel",
        "visual.calculation.remove" => "Supprimer le calcul visuel",
        "textbox.add" => "Zone de texte",
        "textbox.set" => "Modifier le texte",
        "image.add" => "Image",
        "image.set" => "Changer l'image",
        "shape_map.set_map" => "Ajouter un type de carte",
        "button.add" => "Boutons",
        "shape.add" => "Formes",
        "visual.set_action" => "Action",
        "page.format" => "Format de la page",
        "page.format_filter_pane" => "Mettre en forme le volet Filtres",
        "visual.bind_hierarchy" => "Ajouter une hiérarchie",
        "visual.analytics" => "Lignes d'analyse",
        "page.interactions" => "Modifier les interactions",
        "report.settings" => "Paramètres d'interaction",
        "report.pause_visuals" => "Suspendre les visuels",
        "report.refresh_visuals" => "Actualiser tous les visuels",
        "report.query_reduction" => "Préréglages d'optimisation",
        "visual.refresh" => "Actualiser le visuel",
        "visual.drill_filter" => "Appliquer les filtres d'exploration à",
        "column.remove_variation" => "Retirer les variantes de date",
        "column.add" => "Nouvelle colonne",
        "column.set" => "Propriétés de la colonne",
        "table.set_data" => "Remplacer les données",
        "table.set" => "Propriétés de la table",
        "model.set_properties" => "Définir les propriétés de la sélection",
        "table.set_storage_mode" => "Mode de stockage",
        "table.set_refresh_policy" => "Actualisation incrémentielle",
        "table.remove_refresh_policy" => "Supprimer l'actualisation incrémentielle",
        "hierarchy.add" => "Nouvelle hiérarchie",
        "hierarchy.set" => "Modifier la hiérarchie",
        "measure.set" => "Propriétés de la mesure",
        "measure.move" => "Table d'accueil",
        "page.remove" => "Supprimer la page",
        "page.rename" => "Renommer la page",
        "page.move" => "Déplacer la page",
        "page.set_active" => "Ouvrir sur cette page",
        "page.set_display_option" => "Affichage de la page",
        "page.set_type" => "Type de page",
        "visual.remove" => "Supprimer le visuel",
        "visual.set_sort" => "Trier",
        "table.remove" => "Supprimer la table",
        "table.rename" => "Renommer la table",
        "expression.rename" => "Renommer la requête",
        "expression.remove" => "Supprimer la requête",
        "column.rename" => "Renommer la colonne",
        "column.remove" => "Supprimer la colonne",
        "measure.rename" => "Renommer la mesure",
        "measure.remove" => "Supprimer la mesure",
        "hierarchy.remove" => "Supprimer la hiérarchie",
        "calculation_group.remove" => "Supprimer le groupe de calcul",
        "calculation_item.remove" => "Supprimer l'élément de calcul",
        "relationship.remove" => "Supprimer la relation",
        "role.remove" => "Supprimer le rôle",
        "role.rename" => "Renommer le rôle",
        "role.set" => "Propriétés du rôle",
        "role.check_filter" => "Vérifier le filtre du rôle",
        "culture.remove" => "Supprimer la culture",
        "session.revision" => "Révision de la session",
        "file.textconv" => "Forme texte d'un fichier",
        "model.set_annotation" => "Annotation du modèle",
        "table.set_annotation" => "Annotation de la table",
        "column.set_annotation" => "Annotation de la colonne",
        "measure.set_annotation" => "Annotation de la mesure",
        "model.summary" => "Résumé du modèle",
        "model.get" => "Propriétés du modèle (lecture)",
        "table.get" => "Propriétés de la table (lecture)",
        "column.get" => "Propriétés de la colonne (lecture)",
        "column.values" => "Valeurs de la colonne",
        "table.rows" => "Lignes de la table",
        "query.preview" => "Aperçu de la requête",
        "query.profile" => "Profil de colonne",
        "query.export" => "Exporter les résultats de la requête",
        "query.dependencies" => "Dépendances de requête",
        "query.list" => "Transformer les données",
        "query.steps" => "Étapes appliquées",
        "query.apply_step" => "Ajouter une étape",
        "query.edit_step" => "Modifier une étape",
        "query.remove_step" => "Supprimer une étape Power Query",
        "query.new" => "Requête vide",
        "query.rename" => "Renommer la requête (brouillon)",
        "query.apply" => "Fermer et appliquer (Power Query)",
        "query.folding" => "Indicateurs de pliage des requêtes (Power Query)",
        "query.native" => "Afficher la requête native",
        "query.diagnose" => "Diagnostics de requête",
        "visual.records" => "Voir les enregistrements",
        "visual.analyze" => "Analyser ce point",
        "visual.explain_anomaly" => "Expliquer l'anomalie",
        "measure.get" => "Propriétés de la mesure (lecture)",
        "dax.functions" => "Fonctions DAX",
        "table.set_partition" => "Modifier la partition (table.set_partition)",
        "report.summary" => "Résumé du rapport",
        "view.interact" => "Action de lecture",
        "view.interactions" => "État de lecture",
        "view.keep_drill" => "Conserver l'exploration",
        "visual.expand" => "Développer ou réduire",
        "visual.toggle_node" => "Réduire ou développer un nœud d'organigramme",
        "visual.props" => "Propriétés du visuel (lecture)",
        "visual.options" => "Options de mise en forme",
        "theme.set" => "Thèmes",
        "recovery.list" => "Modifications récupérables",
        "recovery.apply" => "Récupérer les modifications",
        "recovery.discard" => "Abandonner les modifications récupérables",
        "visual.copy" => "Copier",
        "visual.paste" => "Coller",
        "visual.copy_format" => "Copier la mise en forme",
        "visual.cut" => "Couper",
        "visual.duplicate" => "Dupliquer",
        "visual.move" => "Déplacer les visuels",
        "visual.align" => "Aligner",
        "visual.group" => "Grouper",
        "bookmark.list" => "Signets",
        "bookmark.capture" => "Ajouter un signet",
        "bookmark.update" => "Mettre à jour le signet",
        "bookmark.set" => "Modifier le signet",
        "bookmark.remove" => "Supprimer le signet",
        "bookmark.group" => "Grouper les signets",
        "bookmark.ungroup" => "Dissocier les signets",
        "bookmark.rename_group" => "Renommer le groupe de signets",
        "bookmark.apply" => "Afficher le signet",
        "slicer.sync_list" => "Segments synchronisés",
        "slicer.set_sync" => "Synchroniser les segments",
        "visual.ungroup" => "Dissocier",
        _ => return None,
    })
}

impl Menu {
    /// The menu's caption.
    pub fn caption(self, l: Lang) -> &'static str {
        match self {
            Menu::Page => l.t("Page tab", "Onglet de page"),
            Menu::Visual => l.t("Visual", "Visuel"),
            Menu::ModelObject => l.t("Model object", "Objet du modèle"),
        }
    }
}

impl Cmd {
    /// Where its button is, as the palette shows it: `Model › Relationships`, `Page tab (menu)`,
    /// `Palette`.
    pub fn place_path(&self, l: Lang) -> String {
        match self.at {
            At::Ribbon { tab, group } => format!("{} › {}", tab.caption(l), group_text(group, l)),
            At::Menu(m) => format!("{} ({})", m.caption(l), l.t("menu", "menu")),
            At::Palette => l.t("Palette", "Palette").to_string(),
        }
    }

    /// Order of its place in the palette's list: the ribbon tabs in tab-row order, the context
    /// menus, then the palette-only commands.
    pub fn place_rank(&self) -> usize {
        match self.at {
            At::Ribbon { tab, .. } => Tab::ALL.iter().position(|t| *t == tab).unwrap_or(0),
            At::Menu(Menu::Page) => 20,
            At::Menu(Menu::Visual) => 21,
            At::Menu(Menu::ModelObject) => 22,
            At::Palette => 40,
        }
    }
}

/// A reason of [`enabled`] (the engine's words) in `l`: the needs' reasons and the role and
/// policy prefixes are translated, anything else stays as the engine wrote it.
pub fn reason_text(en: &str, l: Lang) -> String {
    if l == Lang::En {
        return en.to_string();
    }
    for (e, f) in REASONS_FR {
        if en == *e {
            return (*f).to_string();
        }
        if let Some(rest) = e.strip_suffix("{}").and_then(|p| en.strip_prefix(p)) {
            return f.replace("{}", rest);
        }
    }
    en.to_string()
}

const REASONS_FR: &[(&str, &str)] = &[
    ("open a file first", "ouvrez d'abord un fichier"),
    (
        "needs a saved PBIX, PBIT or PBIP file",
        "il faut un fichier PBIX, PBIT ou PBIP enregistré",
    ),
    ("needs an open PBIP project", "il faut un projet PBIP ouvert"),
    (
        "needs an open PBIP project or PBIX file",
        "il faut un projet PBIP ou un fichier PBIX ouvert",
    ),
    (
        "needs a model: open a file or start a new model",
        "il faut un modèle : ouvrez un fichier ou créez un modèle",
    ),
    ("needs a report", "il faut un rapport"),
    ("the session already has a report", "la session a déjà un rapport"),
    (
        "needs stored data (a PBIX DataModel or a PBIP project's cache.abf)",
        "il faut des données stockées (le DataModel d'un PBIX ou le cache.abf d'un projet PBIP)",
    ),
    ("nothing to undo", "rien à annuler"),
    ("nothing to redo", "rien à rétablir"),
    ("the file is still opening", "le fichier est en cours d'ouverture"),
    (
        "read-only access: this command changes the session",
        "accès en lecture seule : cette commande modifie la session",
    ),
    (
        "read-only access: the open file has unsaved changes",
        "accès en lecture seule : le fichier ouvert a des modifications non enregistrées",
    ),
    (
        "not available on a server: {}",
        "indisponible sur un serveur : {}",
    ),
    ("unknown command {}", "commande inconnue {}"),
];

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
