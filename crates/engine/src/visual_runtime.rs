//! BIcraft's visual runtime (ROADMAP M12.2, docs/community-visuals.md "Runtime"): one TypeScript
//! library, written by us from Microsoft Learn's custom visual pages and the MIT
//! `powerbi-visuals-api` 5.3.0 types, that BIcraft's own custom visuals import with relative
//! imports (`./runtime/…`) and `bicraft_tsbuild` bundles into each of them: tooltips through
//! `host.tooltipService`, selection and cross-filtering through the selection manager, the
//! context menu, the landing page, keyboard focus, high contrast, value formatting with the data
//! view's format strings, EN/FR strings and a format pane model builder. The capabilities those
//! need are merged by [`with_capabilities`].

use std::collections::BTreeMap;

use serde_json::{Value, json};

/// The folder the runtime takes in a visual's sources.
pub const FOLDER: &str = "src/runtime";

/// The runtime's modules: their name in [`FOLDER`] and their text.
pub(crate) const FILES: &[(&str, &str)] = &[
    ("contrast.ts", include_str!("visual_runtime/contrast.ts")),
    ("data.ts", include_str!("visual_runtime/data.ts")),
    ("format.ts", include_str!("visual_runtime/format.ts")),
    ("formatting.ts", include_str!("visual_runtime/formatting.ts")),
    ("index.ts", include_str!("visual_runtime/index.ts")),
    ("interactions.ts", include_str!("visual_runtime/interactions.ts")),
    ("keys.ts", include_str!("visual_runtime/keys.ts")),
    ("landing.ts", include_str!("visual_runtime/landing.ts")),
    ("paint.ts", include_str!("visual_runtime/paint.ts")),
    ("selection.ts", include_str!("visual_runtime/selection.ts")),
    ("strings.ts", include_str!("visual_runtime/strings.ts")),
    ("style.ts", include_str!("visual_runtime/style.ts")),
    ("textsettings.ts", include_str!("visual_runtime/textsettings.ts")),
    ("tooltips.ts", include_str!("visual_runtime/tooltips.ts")),
];

/// The runtime's CSS (landing page, messages, focus ring), appended to a visual's own.
pub(crate) const CSS: &str = include_str!("visual_runtime/runtime.css");

/// Adds the runtime's modules to a visual's sources, under [`FOLDER`].
pub(crate) fn add_sources(sources: &mut BTreeMap<String, String>) {
    for (name, text) in FILES {
        sources.insert(format!("{FOLDER}/{name}"), (*text).to_string());
    }
}

/// What a visual uses of the runtime, for its capabilities.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Uses {
    /// The visual reads `highlights` (another visual's selection highlights its points instead
    /// of filtering them).
    pub highlight: bool,
}

/// `caps` (a visual's `capabilities.json`) with what the runtime needs: `privileges: []` (no
/// web access or downloads), the default and canvas tooltips, `supportsLandingPage` and
/// `supportsEmptyDataView` (the landing page), `supportsKeyboardFocus`, and `supportsHighlight`
/// when the visual uses highlights. Keys the visual already sets are kept.
pub(crate) fn with_capabilities(mut caps: Value, uses: Uses) -> Value {
    if let Value::Object(o) = &mut caps {
        let mut add = |k: &str, v: Value| {
            o.entry(k.to_string()).or_insert(v);
        };
        add("privileges", json!([]));
        add(
            "tooltips",
            json!({"supportedTypes": {"default": true, "canvas": true}}),
        );
        add("supportsLandingPage", json!(true));
        add("supportsEmptyDataView", json!(true));
        add("supportsKeyboardFocus", json!(true));
        if uses.highlight {
            add("supportsHighlight", json!(true));
        }
    }
    caps
}

/// The French of every data role, format card and format option name of BIcraft's visuals,
/// by its English (each translated from the meaning of the English, as the format pane's own
/// strings in the visuals' TypeScript).
const FR: &[(&str, &str)] = &[
    (
        "Areas as the counts (two sets)",
        "Surfaces selon les nombres (deux ensembles)",
    ),
    ("Animation", "Animation"),
    ("At or above the threshold", "Au seuil ou au-dessus"),
    ("BIcraft colors", "Couleurs BIcraft"),
    ("Bands", "Plages"),
    ("Bars", "Barres"),
    ("Below the threshold", "Sous le seuil"),
    ("Bin size", "Largeur d'un intervalle"),
    ("Bins", "Intervalles"),
    ("Bold", "Affirmé"),
    ("Boxes", "Boîtes"),
    ("By category", "Par catégorie"),
    ("By rule", "Par règle"),
    ("By sign", "Par signe"),
    ("Card background", "Fond en carte"),
    ("Category", "Catégorie"),
    ("Collapsed or expanded nodes", "Nœuds réduits ou développés"),
    ("Color", "Couleur"),
    ("Color by", "Couleur selon"),
    ("Colors", "Couleurs"),
    ("Columns", "Colonnes"),
    ("Compact tree", "Arbre compact"),
    ("Compare with", "Comparer à"),
    ("Connectors", "Connecteurs"),
    ("Counts", "Nombres"),
    ("Cumulative", "Cumulé"),
    ("Data labels", "Étiquettes de données"),
    ("Date", "Date"),
    ("Dates", "Dates"),
    ("Day without data", "Jour sans donnée"),
    ("Decrease", "Baisse"),
    ("Destination", "Destination"),
    ("End", "Fin"),
    ("First band", "Première plage"),
    ("First band (% of target)", "Première plage (% de la cible)"),
    ("First set", "Premier ensemble"),
    ("First threshold", "Premier seuil"),
    ("Fourth band", "Quatrième plage"),
    ("Frequency", "Fréquence"),
    ("Gauge", "Jauge"),
    ("Gradient", "Dégradé"),
    ("Group (subtotals)", "Groupe (sous-totaux)"),
    ("Highest value", "Valeur la plus haute"),
    ("Horizontal", "Horizontal"),
    ("Id", "Id"),
    ("Increase", "Hausse"),
    ("Item", "Élément"),
    ("KPI card", "Carte d'indicateur"),
    ("Label", "Étiquette"),
    ("Labels", "Étiquettes"),
    ("Left to right", "De gauche à droite"),
    ("Legend", "Légende"),
    ("Line", "Ligne"),
    ("Linear bullet", "Barre linéaire"),
    ("Links", "Liens"),
    ("Lower is better", "Plus bas est mieux"),
    ("Lowest value", "Valeur la plus basse"),
    ("Maximum", "Maximum"),
    ("Minimal", "Sobre"),
    ("Minimum", "Minimum"),
    ("Modern", "Moderne"),
    ("Node 1", "Nœud 1"),
    ("Node 2", "Nœud 2"),
    ("Node 3", "Nœud 3"),
    ("Node 4", "Nœud 4"),
    ("Node 5", "Nœud 5"),
    ("Node 6", "Nœud 6"),
    ("Node 7", "Nœud 7"),
    ("Node 8", "Nœud 8"),
    ("Node 9", "Nœud 9"),
    ("Node 10", "Nœud 10"),
    ("Node 11", "Nœud 11"),
    ("Node 12", "Nœud 12"),
    ("Node colors", "Couleurs des nœuds"),
    ("Nodes", "Nœuds"),
    ("None", "Aucune"),
    ("Number of bins", "Nombre d'intervalles"),
    ("One color", "Une couleur"),
    ("Opacity (%)", "Opacité (%)"),
    ("Only the months with data", "Seulement les mois avec des données"),
    ("Open levels (0: all)", "Niveaux ouverts (0 : tous)"),
    ("Options", "Options"),
    ("Orientation", "Orientation"),
    ("Parent id", "Id du parent"),
    ("Percent", "Pourcentage"),
    ("Preset", "Préréglage"),
    ("Previous value", "Valeur précédente"),
    ("Radial arc", "Arc radial"),
    ("Range", "Échelle"),
    ("Report theme", "Thème du rapport"),
    ("Rounded corners", "Coins arrondis"),
    ("Rule threshold", "Seuil de la règle"),
    ("Sample", "Échantillon"),
    ("Second band", "Deuxième plage"),
    ("Second band (% of target)", "Deuxième plage (% de la cible)"),
    ("Second set", "Deuxième ensemble"),
    ("Second threshold", "Deuxième seuil"),
    ("Series", "Série"),
    ("Series 1", "Série 1"),
    ("Series 2", "Série 2"),
    ("Series 3", "Série 3"),
    ("Series 4", "Série 4"),
    ("Series 5", "Série 5"),
    ("Series 6", "Série 6"),
    ("Set", "Ensemble"),
    ("Sets", "Ensembles"),
    ("Shadow", "Ombre"),
    ("Show", "Afficher"),
    ("Show outliers", "Afficher les valeurs aberrantes"),
    ("Show the mean", "Afficher la moyenne"),
    ("Show the total", "Afficher le total"),
    ("Show the trend", "Afficher la tendance"),
    ("Show values", "Afficher les valeurs"),
    ("Slope chart", "Graphique de pente"),
    ("Source", "Source"),
    ("Spacing", "Espacement"),
    ("Spec", "Spécification"),
    ("Spec (JSON)", "Spécification (JSON)"),
    ("Stack the series", "Empiler les séries"),
    ("Start", "Début"),
    ("Style", "Style"),
    (
        "Subtotal every N steps (0: none)",
        "Sous-total toutes les N étapes (0 : aucun)",
    ),
    ("Subtotal flag", "Indicateur de sous-total"),
    ("Target", "Cible"),
    ("Target color", "Couleur de la cible"),
    ("Text color", "Couleur du texte"),
    ("Text size", "Taille du texte"),
    ("Third band", "Troisième plage"),
    ("Third band (% of target)", "Troisième plage (% de la cible)"),
    ("Third set", "Troisième ensemble"),
    ("Third threshold", "Troisième seuil"),
    ("Total", "Total"),
    ("Tree", "Arbre"),
    ("Trend", "Tendance"),
    ("Value", "Valeur"),
    ("Value and percent", "Valeur et pourcentage"),
    ("Value chips", "Pastilles de valeur"),
    ("Value in its band's color", "Valeur dans la couleur de sa plage"),
    ("Values", "Valeurs"),
    ("Vertical", "Vertical"),
    ("Waterfall", "Cascade"),
    ("Weeks start on Sunday", "Semaines commençant le dimanche"),
    ("Weight", "Poids"),
    ("Widest first", "Les plus larges d'abord"),
    ("Width", "Largeur"),
    ("Width axis", "Axe des largeurs"),
    // BIcraft Scatter+ and Line / Area+
    ("Above", "Au-dessus"),
    ("Above a value", "Au-dessus d'une valeur"),
    ("All points", "Tous les points"),
    ("Area", "Aire"),
    ("Automatic", "Automatique"),
    ("Averages", "Moyennes"),
    ("Axes", "Axes"),
    ("Axis", "Axe"),
    ("Axis type", "Type d'axe"),
    ("Band between values", "Bande entre les valeurs"),
    ("Below", "En dessous"),
    ("Below a value", "En dessous d'une valeur"),
    ("Border", "Bordure"),
    ("Bottom left label", "Étiquette en bas à gauche"),
    ("Bottom right label", "Étiquette en bas à droite"),
    ("Box strips", "Bandes à boîtes"),
    ("Categories", "Catégories"),
    ("Cell size", "Taille des cellules"),
    ("Color rule", "Règle de couleur"),
    ("Density", "Densité"),
    ("Details", "Détails"),
    ("Emphasized series", "Série mise en avant"),
    ("Fill", "Remplissage"),
    ("Fill opacity (%)", "Opacité du remplissage (%)"),
    ("First value above", "Première valeur au-dessus"),
    ("First value below", "Première valeur en dessous"),
    ("Forecast", "Prévision"),
    (
        "From date (yyyy-mm-dd or today)",
        "À partir du (aaaa-mm-jj ou today)",
    ),
    ("Given values", "Valeurs données"),
    ("Histograms", "Histogrammes"),
    ("How many", "Combien"),
    ("Kind", "Type"),
    ("Last point", "Dernier point"),
    ("Lines", "Lignes"),
    ("Lines at", "Lignes à"),
    ("Lowest and highest", "Plus bas et plus haut"),
    ("Marginal distributions", "Distributions marginales"),
    ("Markers", "Marqueurs"),
    ("Mode", "Mode"),
    ("None", "Aucun"),
    ("Off", "Désactivé"),
    ("On", "Activé"),
    ("Points before density", "Points avant la densité"),
    ("Points labelled", "Points étiquetés"),
    ("Quadrants", "Quadrants"),
    ("Second value", "Deuxième valeur"),
    ("Selected points", "Points sélectionnés"),
    ("Series names at line ends", "Noms des séries en bout de ligne"),
    ("Shade", "Ombrer"),
    ("Size", "Taille"),
    ("Start at zero", "Commencer à zéro"),
    ("Threshold", "Seuil"),
    ("Top left label", "Étiquette en haut à gauche"),
    ("Top points", "Points les plus hauts"),
    ("Top right label", "Étiquette en haut à droite"),
    ("Trend line", "Courbe de tendance"),
    ("X axis", "Axe X"),
    ("X value", "Valeur X"),
    ("Y axis", "Axe Y"),
    ("Y value", "Valeur Y"),
    // the text settings (`textsettings.ts`)
    ("Bold text", "Texte en gras"),
    ("Bold title", "Titre en gras"),
    ("Display units", "Unités d'affichage"),
    ("Font", "Police"),
    ("Italic text", "Texte en italique"),
    ("Italic title", "Titre en italique"),
    ("Title", "Titre"),
    ("Title color", "Couleur du titre"),
    ("Title font", "Police du titre"),
    ("Title text size", "Taille du texte du titre"),
    ("Underlined text", "Texte souligné"),
    ("Underlined title", "Titre souligné"),
    ("Value decimal places", "Décimales des valeurs"),
];

/// What a text setting holds (`textsettings.ts`'s kinds).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextKind {
    Bool,
    Color,
    Size,
    Family,
    Units,
    Decimals,
}

/// The text settings' format properties (`textsettings.ts` `TEXT_PROPS`, with the property
/// names Power BI's own charts use): object, property, the text element and setting of the Spec
/// layout's `text` it feeds (docs/formats/spec-visual.md "Text"), its kind and its name in the
/// format pane.
pub(crate) const TEXT_PROPS: &[(&str, &str, &str, &str, TextKind, &str)] = {
    use TextKind::{Bool, Color, Decimals, Family, Size, Units};
    &[
        ("labels", "show", "labels", "show", Bool, "Show"),
        ("labels", "color", "labels", "color", Color, "Color"),
        ("labels", "fontFamily", "labels", "family", Family, "Font"),
        ("labels", "fontSize", "labels", "size", Size, "Text size"),
        ("labels", "bold", "labels", "bold", Bool, "Bold text"),
        ("labels", "italic", "labels", "italic", Bool, "Italic text"),
        (
            "labels",
            "underline",
            "labels",
            "underline",
            Bool,
            "Underlined text",
        ),
        (
            "labels",
            "labelDisplayUnits",
            "labels",
            "units",
            Units,
            "Display units",
        ),
        (
            "labels",
            "labelPrecision",
            "labels",
            "decimals",
            Decimals,
            "Value decimal places",
        ),
        ("categoryAxis", "show", "x", "show", Bool, "Show"),
        ("categoryAxis", "labelColor", "x", "color", Color, "Color"),
        ("categoryAxis", "fontFamily", "x", "family", Family, "Font"),
        ("categoryAxis", "fontSize", "x", "size", Size, "Text size"),
        ("categoryAxis", "bold", "x", "bold", Bool, "Bold text"),
        ("categoryAxis", "italic", "x", "italic", Bool, "Italic text"),
        (
            "categoryAxis",
            "underline",
            "x",
            "underline",
            Bool,
            "Underlined text",
        ),
        ("categoryAxis", "showAxisTitle", "xTitle", "show", Bool, "Title"),
        (
            "categoryAxis",
            "titleColor",
            "xTitle",
            "color",
            Color,
            "Title color",
        ),
        (
            "categoryAxis",
            "titleFontFamily",
            "xTitle",
            "family",
            Family,
            "Title font",
        ),
        (
            "categoryAxis",
            "titleFontSize",
            "xTitle",
            "size",
            Size,
            "Title text size",
        ),
        ("categoryAxis", "titleBold", "xTitle", "bold", Bool, "Bold title"),
        (
            "categoryAxis",
            "titleItalic",
            "xTitle",
            "italic",
            Bool,
            "Italic title",
        ),
        (
            "categoryAxis",
            "titleUnderline",
            "xTitle",
            "underline",
            Bool,
            "Underlined title",
        ),
        ("valueAxis", "show", "y", "show", Bool, "Show"),
        ("valueAxis", "labelColor", "y", "color", Color, "Color"),
        ("valueAxis", "fontFamily", "y", "family", Family, "Font"),
        ("valueAxis", "fontSize", "y", "size", Size, "Text size"),
        ("valueAxis", "bold", "y", "bold", Bool, "Bold text"),
        ("valueAxis", "italic", "y", "italic", Bool, "Italic text"),
        (
            "valueAxis",
            "underline",
            "y",
            "underline",
            Bool,
            "Underlined text",
        ),
        (
            "valueAxis",
            "labelDisplayUnits",
            "y",
            "units",
            Units,
            "Display units",
        ),
        (
            "valueAxis",
            "labelPrecision",
            "y",
            "decimals",
            Decimals,
            "Value decimal places",
        ),
        ("valueAxis", "showAxisTitle", "yTitle", "show", Bool, "Title"),
        ("valueAxis", "titleColor", "yTitle", "color", Color, "Title color"),
        (
            "valueAxis",
            "titleFontFamily",
            "yTitle",
            "family",
            Family,
            "Title font",
        ),
        (
            "valueAxis",
            "titleFontSize",
            "yTitle",
            "size",
            Size,
            "Title text size",
        ),
        ("valueAxis", "titleBold", "yTitle", "bold", Bool, "Bold title"),
        (
            "valueAxis",
            "titleItalic",
            "yTitle",
            "italic",
            Bool,
            "Italic title",
        ),
        (
            "valueAxis",
            "titleUnderline",
            "yTitle",
            "underline",
            Bool,
            "Underlined title",
        ),
        ("legend", "show", "legend", "show", Bool, "Show"),
        ("legend", "labelColor", "legend", "color", Color, "Color"),
        ("legend", "fontFamily", "legend", "family", Family, "Font"),
        ("legend", "fontSize", "legend", "size", Size, "Text size"),
        ("legend", "bold", "legend", "bold", Bool, "Bold text"),
        ("legend", "italic", "legend", "italic", Bool, "Italic text"),
        (
            "legend",
            "underline",
            "legend",
            "underline",
            Bool,
            "Underlined text",
        ),
    ]
};

/// The text settings' format objects for `capabilities.json` (names localized by
/// [`localize`]).
pub(crate) fn text_objects() -> serde_json::Map<String, Value> {
    let mut objects = serde_json::Map::new();
    for (object, property, _, _, kind, display) in TEXT_PROPS {
        let o = objects.entry((*object).to_string()).or_insert_with(|| {
            let name = match *object {
                "labels" => "Data labels",
                "categoryAxis" => "X axis",
                "valueAxis" => "Y axis",
                _ => "Legend",
            };
            json!({"displayName": name, "properties": {}})
        });
        let ty = match kind {
            TextKind::Bool => json!({"bool": true}),
            TextKind::Color => json!({"fill": {"solid": {"color": true}}}),
            TextKind::Size => json!({"formatting": {"fontSize": true}}),
            TextKind::Family => json!({"formatting": {"fontFamily": true}}),
            TextKind::Units => json!({"formatting": {"labelDisplayUnits": true}}),
            TextKind::Decimals => json!({"integer": true}),
        };
        if let Some(props) = o.get_mut("properties").and_then(Value::as_object_mut) {
            props.insert((*property).into(), json!({"displayName": display, "type": ty}));
        }
    }
    objects
}

/// A text setting's value as the Spec layout takes it (`textsettings.ts` `valueOf`), from its
/// PBIR form; none when it is not one the layout can draw.
fn text_value(v: &Value, kind: TextKind) -> Option<Value> {
    use crate::visual_props::Lit;
    let lit = |e: &Value| {
        e.get("expr")?
            .get("Literal")?
            .get("Value")?
            .as_str()
            .map(crate::visual_props::parse_lit)
    };
    match kind {
        TextKind::Color => {
            // a fill ({solid: {color}}), as Desktop writes it, or a plain text literal
            let at = v.get("solid").and_then(|x| x.get("color")).unwrap_or(v);
            let Lit::Text(c) = lit(at)? else {
                return None;
            };
            let ok = c.len() == 7 && c.starts_with('#') && c.bytes().skip(1).all(|b| b.is_ascii_hexdigit());
            ok.then(|| json!(c.to_ascii_uppercase()))
        }
        TextKind::Bool => match lit(v)? {
            Lit::Bool(b) => Some(json!(b)),
            _ => None,
        },
        TextKind::Family => match lit(v)? {
            Lit::Text(t) if t.chars().count() <= 200 && !t.chars().any(char::is_control) => Some(json!(t)),
            _ => None,
        },
        TextKind::Size | TextKind::Units | TextKind::Decimals => {
            let Lit::Num(n, _) = lit(v)? else {
                return None;
            };
            match kind {
                TextKind::Size => (n.is_finite() && (6.0..=60.0).contains(&n)).then(|| json!(n)),
                TextKind::Units => bicraft_render::spec::UNITS.contains(&n).then(|| json!(n)),
                _ => (n.is_finite() && (0.0..=6.0).contains(&n)).then(|| json!(n.floor() as u32)),
            }
        }
    }
}

/// The text settings the format pane holds on `v` (its entries without a selector), as the Spec
/// layout's `text`: `{part: {key: value}}`.
pub(crate) fn text_of(v: &bicraft_report::Visual) -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    let objects = v.visual_extra.get("objects");
    for (object, property, part, key, kind, _) in TEXT_PROPS {
        let Some(x) = objects
            .and_then(|o| o.get(*object))
            .and_then(Value::as_array)
            .and_then(|l| l.iter().find(|e| e.get("selector").is_none_or(Value::is_null)))
            .and_then(|e| e.get("properties"))
            .and_then(|p| p.get(*property))
            .and_then(|x| text_value(x, *kind))
        else {
            continue;
        };
        if let Some(o) = out
            .entry((*part).to_string())
            .or_insert_with(|| json!({}))
            .as_object_mut()
        {
            o.insert((*key).into(), x);
        }
    }
    out
}

/// The locales of BIcraft's visuals' strings.
pub(crate) const LOCALES: [&str; 2] = ["en-US", "fr-FR"];

/// Gives every data role, format object and format property of `caps` (one of BIcraft's
/// visuals' `capabilities.json`) a `displayNameKey` (`Role_<role>`, `Object_<object>`,
/// `Object_<object>_<property>`, and each item of a drop-down list
/// `Object_<object>_<property>_<value>`; Microsoft Learn's "Add the locale in Power BI for Power
/// BI visuals") and returns the visual's `stringResources`: its `displayName`s in English and
/// French by key ([`LOCALES`]). A name without its French is an error, so no visual ships half
/// translated.
pub(crate) fn localize(caps: &mut Value) -> Result<BTreeMap<String, Value>, String> {
    let mut en = serde_json::Map::new();
    let mut fr = serde_json::Map::new();
    let mut add = |o: &mut serde_json::Map<String, Value>, key: String| -> Result<(), String> {
        let Some(name) = o.get("displayName").and_then(Value::as_str).map(str::to_string) else {
            return Ok(());
        };
        let french = FR
            .iter()
            .find(|(e, _)| *e == name)
            .map(|(_, f)| *f)
            .ok_or_else(|| format!("internal: `{name}` has no French"))?;
        o.insert("displayNameKey".into(), json!(key));
        en.insert(key.clone(), json!(name));
        fr.insert(key, json!(french));
        Ok(())
    };
    if let Some(roles) = caps.get_mut("dataRoles").and_then(Value::as_array_mut) {
        for r in roles.iter_mut() {
            if let Some(o) = r.as_object_mut() {
                let key = format!("Role_{}", o.get("name").and_then(Value::as_str).unwrap_or(""));
                add(o, key)?;
            }
        }
    }
    if let Some(objects) = caps.get_mut("objects").and_then(Value::as_object_mut) {
        for (name, obj) in objects.iter_mut() {
            let Some(o) = obj.as_object_mut() else {
                continue;
            };
            add(o, format!("Object_{name}"))?;
            if let Some(props) = o.get_mut("properties").and_then(Value::as_object_mut) {
                for (prop, p) in props.iter_mut() {
                    if let Some(po) = p.as_object_mut() {
                        add(po, format!("Object_{name}_{prop}"))?;
                        let items = po
                            .get_mut("type")
                            .and_then(|t| t.get_mut("enumeration"))
                            .and_then(Value::as_array_mut);
                        for item in items.into_iter().flatten() {
                            if let Some(io) = item.as_object_mut() {
                                let v = io.get("value").and_then(Value::as_str).unwrap_or("");
                                let key = format!("Object_{name}_{prop}_{v}");
                                add(io, key)?;
                            }
                        }
                    }
                }
            }
        }
    }
    let mut out = BTreeMap::new();
    let [en_tag, fr_tag] = LOCALES;
    out.insert(en_tag.to_string(), Value::Object(en));
    out.insert(fr_tag.to_string(), Value::Object(fr));
    Ok(out)
}

/// The runtime's browser-free helpers (`index.ts`) bundled into one script whose value is the
/// module's exports (BIcraft's tests run it under Node).
pub fn helpers_script() -> Result<String, String> {
    let mut files = BTreeMap::new();
    add_sources(&mut files);
    bicraft_tsbuild::bundle(&files, &format!("{FOLDER}/index.ts")).map(|o| o.js)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every module of the runtime compiles with BIcraft's bundler: `index.ts` exports from
    /// each of them.
    #[test]
    fn the_runtime_compiles() {
        let mut files = BTreeMap::new();
        add_sources(&mut files);
        let index = bicraft_tsbuild::bundle(&files, "src/runtime/index.ts").unwrap();
        assert!(index.js.contains("formatValue"), "{}", index.js.len());
        for (name, _) in FILES {
            let m = format!("{FOLDER}/{name}");
            assert!(index.modules.contains(&m), "{m}: {:?}", index.modules);
        }
    }

    #[test]
    fn capabilities_keep_what_the_visual_sets() {
        let caps = with_capabilities(
            json!({"dataRoles": [], "supportsKeyboardFocus": false}),
            Uses { highlight: true },
        );
        assert_eq!(caps["supportsKeyboardFocus"], json!(false));
        assert_eq!(caps["supportsLandingPage"], json!(true));
        assert_eq!(caps["supportsHighlight"], json!(true));
        assert_eq!(caps["privileges"], json!([]));
        let plain = with_capabilities(json!({}), Uses::default());
        assert!(plain.get("supportsHighlight").is_none());
    }
}
