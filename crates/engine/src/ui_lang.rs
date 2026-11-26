//! M10.2: the user-interface language of the generated visual text (auto titles, implicit-measure
//! names, "(Blank)", "Total", slicer "All" / "Multiple selections", …), like Power BI Desktop,
//! which writes them in its own UI language (`docs/view-snapshot.md`, "UI language").
//!
//! The phrases below are BIcraft's own wording per language. The English and French texts Desktop
//! also draws on a visual (auto titles, implicit-measure names, "(Blank)", "Total", slicer
//! texts) were compared with what Desktop shows (`docs/view-snapshot.md`); the prompts (empty
//! visual, no active filter, the open button) are BIcraft's own; the German, Spanish, Italian,
//! Dutch, Portuguese and Japanese ones are BIcraft's own translations of the English, not
//! compared with Desktop (`docs/formats/locales.md`, "UI phrases").

use bicraft_report::{FieldRef, Visual};
use serde_json::{Map, Value, json};

/// Longest accepted language tag.
const MAX_TAG: usize = 35;
/// Most fields named in one auto title (values / categories), like the viewer.
const TITLE_VALUES: usize = 3;
const TITLE_CATEGORIES: usize = 2;
/// Longest generated label or title, in characters.
const MAX_LABEL: usize = 400;

/// Languages with a phrase table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Lang {
    #[default]
    En,
    Fr,
    De,
    Es,
    It,
    Nl,
    Pt,
    Ja,
}

/// The phrases of one language. `{v}`, `{c}` and `{0}` are placeholders.
struct Phrases {
    /// Auto title with values and categories: `{v}` values, `{c}` categories.
    title_by: &'static str,
    /// Joins the last two field names in a title or axis caption.
    and: &'static str,
    /// Joins the other field names ("A, B and C").
    list_sep: &'static str,
    blank: &'static str,
    total: &'static str,
    all: &'static str,
    multiple_selections: &'static str,
    select_all: &'static str,
    search: &'static str,
    other: &'static str,
    increase: &'static str,
    decrease: &'static str,
    /// A KPI's goal label.
    goal: &'static str,
    /// The prompt of a visual that binds no field.
    empty_visual: &'static str,
    /// A relative-date slicer without a period.
    no_filters_applied: &'static str,
    /// The live viewer's button that opens the saved file in Power BI Desktop.
    open_in_desktop: &'static str,
    year: &'static str,
    quarter: &'static str,
    month: &'static str,
    day: &'static str,
    /// Implicit measures, per aggregation (`{0}` is the column name).
    sum: &'static str,
    average: &'static str,
    min: &'static str,
    max: &'static str,
    /// `CountNonNull` (Desktop's "Count").
    count: &'static str,
    /// `Count` (Desktop's "Count (Distinct)").
    distinct_count: &'static str,
    median: &'static str,
    std_dev: &'static str,
    variance: &'static str,
}

const EN: Phrases = Phrases {
    title_by: "{v} by {c}",
    and: " and ",
    list_sep: ", ",
    blank: "(Blank)",
    total: "Total",
    all: "All",
    multiple_selections: "Multiple selections",
    select_all: "Select all",
    search: "Search",
    other: "Other",
    increase: "Increase",
    decrease: "Decrease",
    goal: "Goal",
    empty_visual: "Add fields to show data in this visual",
    no_filters_applied: "No active filter",
    open_in_desktop: "Open in Power BI Desktop",
    year: "Year",
    quarter: "Quarter",
    month: "Month",
    day: "Day",
    sum: "Sum of {0}",
    average: "Average of {0}",
    min: "Min of {0}",
    max: "Max of {0}",
    count: "Count of {0}",
    distinct_count: "Count of {0}",
    median: "Median of {0}",
    std_dev: "Standard deviation of {0}",
    variance: "Variance of {0}",
};

const FR: Phrases = Phrases {
    title_by: "{v} par {c}",
    and: " et ",
    list_sep: ", ",
    blank: "(Vide)",
    total: "Total",
    all: "Tout",
    multiple_selections: "Plusieurs sélections",
    select_all: "Sélectionner tout",
    search: "Rechercher",
    other: "Autre",
    increase: "Augmenter",
    decrease: "Diminuer",
    goal: "Objectif",
    empty_visual: "Ajoutez des champs pour afficher des données dans ce visuel",
    no_filters_applied: "Aucun filtre actif",
    open_in_desktop: "Ouvrir dans Power BI Desktop",
    year: "Année",
    quarter: "Trimestre",
    month: "Mois",
    day: "Jour",
    sum: "Somme de {0}",
    average: "Moyenne de {0}",
    min: "Min de {0}",
    max: "Max de {0}",
    count: "Nombre de {0}",
    distinct_count: "Nombre de {0}",
    median: "Médiane de {0}",
    std_dev: "Écart type de {0}",
    variance: "Variance de {0}",
};

// The six tables below are BIcraft's own translations, made from the meaning of the English
// table; they were not compared with Desktop. Implicit-measure names put the aggregation after
// the column ("Amount (Summe)"), BIcraft's own form.

const DE: Phrases = Phrases {
    title_by: "{v} nach {c}",
    and: " und ",
    list_sep: ", ",
    blank: "(ohne Wert)",
    total: "Gesamtsumme",
    all: "Alle",
    multiple_selections: "Mehrere ausgewählt",
    select_all: "Alles auswählen",
    search: "Suche",
    other: "Sonstige",
    increase: "Anstieg",
    decrease: "Rückgang",
    goal: "Zielwert",
    empty_visual: "Felder hinzufügen, damit dieses Visual Daten zeigt",
    no_filters_applied: "Kein Filter aktiv",
    open_in_desktop: "Mit Power BI Desktop öffnen",
    year: "Jahr",
    quarter: "Quartal",
    month: "Monat",
    day: "Tag",
    sum: "{0} (Summe)",
    average: "{0} (Mittelwert)",
    min: "{0} (Minimum)",
    max: "{0} (Maximum)",
    count: "{0} (Anzahl)",
    distinct_count: "{0} (Anzahl)",
    median: "{0} (Median)",
    std_dev: "{0} (Standardabweichung)",
    variance: "{0} (Varianz)",
};

const ES: Phrases = Phrases {
    title_by: "{v} por {c}",
    and: " y ",
    list_sep: ", ",
    blank: "(sin valor)",
    total: "Total general",
    all: "Todos",
    multiple_selections: "Selección múltiple",
    select_all: "Marcar todo",
    search: "Búsqueda",
    other: "Resto",
    increase: "Subida",
    decrease: "Bajada",
    goal: "Meta",
    empty_visual: "Añada campos para que este objeto visual muestre datos",
    no_filters_applied: "Sin filtro activo",
    open_in_desktop: "Abrir con Power BI Desktop",
    year: "Año",
    quarter: "Trimestre",
    month: "Mes",
    day: "Día",
    sum: "{0} (suma)",
    average: "{0} (media)",
    min: "{0} (mínimo)",
    max: "{0} (máximo)",
    count: "{0} (número)",
    distinct_count: "{0} (número)",
    median: "{0} (mediana)",
    std_dev: "{0} (desviación típica)",
    variance: "{0} (varianza)",
};

const IT: Phrases = Phrases {
    title_by: "{v} per {c}",
    and: " e ",
    list_sep: ", ",
    blank: "(senza valore)",
    total: "Totale complessivo",
    all: "Tutti",
    multiple_selections: "Più valori selezionati",
    select_all: "Seleziona tutti",
    search: "Ricerca",
    other: "Restanti",
    increase: "Crescita",
    decrease: "Calo",
    goal: "Traguardo",
    empty_visual: "Aggiungere campi perché questo oggetto visivo mostri dati",
    no_filters_applied: "Nessun filtro attivo",
    open_in_desktop: "Apri con Power BI Desktop",
    year: "Anno",
    quarter: "Trimestre",
    month: "Mese",
    day: "Giorno",
    sum: "{0} (somma)",
    average: "{0} (media)",
    min: "{0} (minimo)",
    max: "{0} (massimo)",
    count: "{0} (numero)",
    distinct_count: "{0} (numero)",
    median: "{0} (mediana)",
    std_dev: "{0} (scarto quadratico medio)",
    variance: "{0} (varianza)",
};

const NL: Phrases = Phrases {
    title_by: "{v} per {c}",
    and: " en ",
    list_sep: ", ",
    blank: "(zonder waarde)",
    total: "Eindtotaal",
    all: "Alles",
    multiple_selections: "Meer dan één gekozen",
    select_all: "Alles aanvinken",
    search: "Zoek",
    other: "Rest",
    increase: "Stijging",
    decrease: "Daling",
    goal: "Streefwaarde",
    empty_visual: "Voeg velden toe zodat deze visual gegevens toont",
    no_filters_applied: "Geen filter actief",
    open_in_desktop: "Met Power BI Desktop openen",
    year: "Jaar",
    quarter: "Kwartaal",
    month: "Maand",
    day: "Dag",
    sum: "{0} (som)",
    average: "{0} (gemiddelde)",
    min: "{0} (minimum)",
    max: "{0} (maximum)",
    count: "{0} (aantal)",
    distinct_count: "{0} (aantal)",
    median: "{0} (mediaan)",
    std_dev: "{0} (standaarddeviatie)",
    variance: "{0} (variantie)",
};

const PT: Phrases = Phrases {
    title_by: "{v} por {c}",
    and: " e ",
    list_sep: ", ",
    blank: "(sem valor)",
    total: "Total geral",
    all: "Todos",
    multiple_selections: "Seleção múltipla",
    select_all: "Marcar todos",
    search: "Busca",
    other: "Restantes",
    increase: "Alta",
    decrease: "Queda",
    goal: "Alvo",
    empty_visual: "Adicione campos para que este visual mostre dados",
    no_filters_applied: "Sem filtro ativo",
    open_in_desktop: "Abrir com o Power BI Desktop",
    year: "Ano",
    quarter: "Trimestre",
    month: "Mês",
    day: "Dia",
    sum: "{0} (soma)",
    average: "{0} (média)",
    min: "{0} (mínimo)",
    max: "{0} (máximo)",
    count: "{0} (quantidade)",
    distinct_count: "{0} (quantidade)",
    median: "{0} (mediana)",
    std_dev: "{0} (desvio-padrão)",
    variance: "{0} (variância)",
};

const JA: Phrases = Phrases {
    title_by: "{c}ごとの{v}",
    and: "と",
    list_sep: "、",
    blank: "(値なし)",
    total: "総計",
    all: "全部",
    multiple_selections: "複数の値を選択中",
    select_all: "全部を選択",
    search: "探す",
    other: "残り",
    increase: "上昇",
    decrease: "下降",
    goal: "目標値",
    empty_visual: "フィールドを追加すると、このビジュアルにデータが表示されます",
    no_filters_applied: "有効なフィルターなし",
    open_in_desktop: "Power BI Desktop で開く",
    year: "年",
    quarter: "四半期",
    month: "月",
    day: "日",
    sum: "{0} (合計)",
    average: "{0} (平均)",
    min: "{0} (最小)",
    max: "{0} (最大)",
    count: "{0} (件数)",
    distinct_count: "{0} (件数)",
    median: "{0} (中央値)",
    std_dev: "{0} (標準偏差)",
    variance: "{0} (分散)",
};

impl Lang {
    fn from_primary(p: &str) -> Option<Self> {
        Some(match p {
            "en" => Self::En,
            "fr" => Self::Fr,
            "de" => Self::De,
            "es" => Self::Es,
            "it" => Self::It,
            "nl" => Self::Nl,
            "pt" => Self::Pt,
            "ja" => Self::Ja,
            _ => return None,
        })
    }

    fn phrases(self) -> &'static Phrases {
        match self {
            Self::En => &EN,
            Self::Fr => &FR,
            Self::De => &DE,
            Self::Es => &ES,
            Self::It => &IT,
            Self::Nl => &NL,
            Self::Pt => &PT,
            Self::Ja => &JA,
        }
    }

    fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Fr => "fr",
            Self::De => "de",
            Self::Es => "es",
            Self::It => "it",
            Self::Nl => "nl",
            Self::Pt => "pt",
            Self::Ja => "ja",
        }
    }
}

/// Primary languages with a phrase table.
pub const SUPPORTED_LANGUAGES: &[&str] = &["en", "fr", "de", "es", "it", "nl", "pt", "ja"];

/// The UI language of the generated visual text: a BCP 47 tag (`fr-FR`) and the phrase table it
/// selects. Languages without a table use English (`fallback`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiLanguage {
    tag: String,
    lang: Lang,
    fallback: bool,
}

impl Default for UiLanguage {
    fn default() -> Self {
        Self {
            tag: "en-US".into(),
            lang: Lang::En,
            fallback: false,
        }
    }
}

impl UiLanguage {
    /// Parses a language tag (`fr`, `fr-FR`, `pt_BR`, case-insensitive). A well-formed tag of a
    /// language without a phrase table is accepted and uses English.
    pub fn parse(tag: &str) -> Result<Self, String> {
        let t = tag.trim();
        let bad = || {
            format!(
                "`{}` is not a language tag (e.g. en-US, fr-FR, de, ja-JP)",
                t.chars().take(40).collect::<String>()
            )
        };
        if t.is_empty() || t.len() > MAX_TAG {
            return Err(bad());
        }
        let parts: Vec<&str> = t.split(['-', '_']).collect();
        let primary = parts.first().copied().unwrap_or("");
        if !(2..=3).contains(&primary.len()) || !primary.chars().all(|c| c.is_ascii_alphabetic()) {
            return Err(bad());
        }
        let rest_ok = parts
            .iter()
            .skip(1)
            .all(|p| (1..=8).contains(&p.len()) && p.chars().all(|c| c.is_ascii_alphanumeric()));
        if !rest_ok {
            return Err(bad());
        }
        let primary = primary.to_ascii_lowercase();
        let normalized: Vec<String> = std::iter::once(primary.clone())
            .chain(parts.iter().skip(1).map(|p| match p.len() {
                2 => p.to_ascii_uppercase(),
                4 => {
                    let mut s = p.to_ascii_lowercase();
                    if let Some(f) = s.get_mut(..1) {
                        f.make_ascii_uppercase();
                    }
                    s
                }
                _ => p.to_ascii_lowercase(),
            }))
            .collect();
        let (lang, fallback) = match Lang::from_primary(&primary) {
            Some(l) => (l, false),
            None => (Lang::En, true),
        };
        Ok(Self {
            tag: normalized.join("-"),
            lang,
            fallback,
        })
    }

    /// The normalized tag (`fr-FR`).
    pub fn tag(&self) -> &str {
        &self.tag
    }

    fn p(&self) -> &'static Phrases {
        self.lang.phrases()
    }

    /// The snapshot's `ui` object: `{language, strings_language, fallback?, strings}`.
    pub(crate) fn snapshot_json(&self) -> Value {
        let p = self.p();
        let mut o = Map::new();
        o.insert("language".into(), json!(self.tag));
        o.insert("strings_language".into(), json!(self.lang.code()));
        if self.fallback {
            o.insert("fallback".into(), json!(true));
        }
        o.insert(
            "strings".into(),
            json!({
                "title_by": p.title_by,
                "and": p.and,
                "list_separator": p.list_sep,
                "blank": p.blank,
                "total": p.total,
                "all": p.all,
                "multiple_selections": p.multiple_selections,
                "select_all": p.select_all,
                "search": p.search,
                "other": p.other,
                "increase": p.increase,
                "decrease": p.decrease,
                "goal": p.goal,
                "empty_visual": p.empty_visual,
                "no_filters_applied": p.no_filters_applied,
                "open_in_desktop": p.open_in_desktop,
                "year": p.year,
                "quarter": p.quarter,
                "month": p.month,
                "day": p.day,
                "aggregations": {
                    "sum": p.sum,
                    "avg": p.average,
                    "min": p.min,
                    "max": p.max,
                    "countnonnull": p.count,
                    "count": p.distinct_count,
                    "median": p.median,
                    "standarddeviation": p.std_dev,
                    "variance": p.variance,
                },
            }),
        );
        Value::Object(o)
    }

    /// The implicit-measure name Desktop shows for `function` over `column` (`Sum of Amount`);
    /// `None` for an unknown aggregation.
    pub(crate) fn aggregation(&self, function: &str, column: &str) -> Option<String> {
        let p = self.p();
        let template = match function.to_ascii_lowercase().as_str() {
            "sum" => p.sum,
            "avg" | "average" => p.average,
            "min" => p.min,
            "max" => p.max,
            "countnonnull" => p.count,
            "count" | "distinctcount" => p.distinct_count,
            "median" => p.median,
            "standarddeviation" | "stdev" | "stddev" => p.std_dev,
            "variance" | "var" => p.variance,
            _ => return None,
        };
        Some(template.replace("{0}", column))
    }

    /// Joins names like Desktop: "A and B", "A, B and C".
    pub(crate) fn join_and(&self, names: &[String]) -> String {
        let p = self.p();
        match names.split_last() {
            None => String::new(),
            Some((last, [])) => last.clone(),
            Some((last, rest)) => format!("{}{}{last}", rest.join(p.list_sep), p.and),
        }
    }

    /// Desktop's title for a visual without one: `values by categories`, or the categories alone
    /// (comma-separated) when there are no values. Empty when the visual binds nothing.
    #[cfg(test)]
    pub(crate) fn auto_title(&self, values: &[String], categories: &[String]) -> String {
        self.auto_title_with(values, categories, TITLE_VALUES)
    }

    /// [`Self::auto_title`] naming at most `max_values` values (a gauge names its value, minimum,
    /// maximum and target: four).
    fn auto_title_with(&self, values: &[String], categories: &[String], max_values: usize) -> String {
        let cats: Vec<String> = categories.iter().take(TITLE_CATEGORIES).cloned().collect();
        let vals: Vec<String> = values.iter().take(max_values).cloned().collect();
        let t = if vals.is_empty() {
            categories.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
        } else if cats.is_empty() {
            self.join_and(&vals)
        } else {
            self.p()
                .title_by
                .replace("{v}", &self.join_and(&vals))
                .replace("{c}", &self.join_and(&cats))
        };
        cap(&t)
    }
}

fn cap(s: &str) -> String {
    s.chars().take(MAX_LABEL).collect()
}

/// Role classes for the auto title, like the viewer's `roleClass`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoleClass {
    Value,
    Category,
    Other,
}

fn role_class(role: &str) -> RoleClass {
    let r: String = role
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_')
        .flat_map(char::to_lowercase)
        .collect();
    match r.as_str() {
        "category" | "axis" | "row" | "rows" | "group" | "details" | "location" | "xaxis" | "columns"
        | "column" | "legend" | "series" => RoleClass::Category,
        "y" | "y2" | "value" | "values" | "size" | "indicator" | "data" | "measure" | "measures"
        | "field" | "fields" | "linevalues" | "secondaryy" | "minvalue" | "maxvalue" | "targetvalue"
        | "goal" => RoleClass::Value,
        // a KPI's trend axis: "Total and Goal by Month"
        "trendline" => RoleClass::Category,
        _ => RoleClass::Other,
    }
}

/// Order of a value role in an auto title: the value first, then a gauge's minimum, maximum and
/// target (a KPI's goal).
fn value_rank(role: &str) -> u8 {
    match role.to_ascii_lowercase().as_str() {
        "minvalue" => 1,
        "maxvalue" => 2,
        "targetvalue" | "goal" => 3,
        _ => 0,
    }
}

/// The name each field of the visual is renamed to (its projection's `displayName`), in
/// `Visual::fields` order.
fn projection_names(v: &Visual) -> Vec<Option<String>> {
    v.fields
        .iter()
        .map(|b| b.display_name.as_deref().filter(|s| !s.is_empty()).map(cap))
        .collect()
}

/// The level name of a hierarchy-level field (`{"HierarchyLevel": {…, "Level": "Year"}}`).
fn hierarchy_level(raw: &Value) -> Option<String> {
    raw.get("HierarchyLevel")?.get("Level")?.as_str().map(cap)
}

/// The caption of a bound field: the visual's rename, else the measure / column name, the
/// implicit-measure name for an aggregation, or a hierarchy level's name.
fn field_label(lang: &UiLanguage, f: &FieldRef, rename: Option<&String>) -> Option<String> {
    if let Some(r) = rename {
        return Some(r.clone());
    }
    match f {
        FieldRef::Column { column, .. } => Some(cap(column)),
        FieldRef::Measure { measure, .. } => Some(cap(measure)),
        FieldRef::Aggregation { column, function, .. } => Some(cap(&lang
            .aggregation(function, column)
            .unwrap_or_else(|| format!("{function}({column})")))),
        FieldRef::Other { raw } => hierarchy_level(raw)
            .or_else(|| bicraft_report::query::visual_calc(f).map(|c| c.name))
            .or_else(|| {
                // a sparkline: "Sum of Amount by Date", in the UI language
                let s = bicraft_report::query::sparkline(f)?;
                let y = field_label(lang, &s.measure, None)?;
                let x: Vec<String> = s
                    .groupings
                    .iter()
                    .filter_map(|g| field_label(lang, g, None))
                    .collect();
                Some(lang.auto_title_with(&[y], &x, 1))
            }),
    }
}

/// The localized label of every field (`None` when unknown), in `Visual::fields` order.
pub(crate) fn field_labels(lang: &UiLanguage, v: &Visual) -> Vec<Option<String>> {
    let renames = projection_names(v);
    v.fields
        .iter()
        .enumerate()
        .map(|(i, b)| field_label(lang, &b.field, renames.get(i).and_then(Option::as_ref)))
        .collect()
}

/// The visual's auto title from its labels (`None` when it binds nothing titled).
pub(crate) fn visual_auto_title(lang: &UiLanguage, v: &Visual, labels: &[Option<String>]) -> Option<String> {
    if let Some(b) = v.visual_type.as_deref().and_then(crate::builtin_visuals::find) {
        return bicraft_visual_title(lang, v, labels, &b.roles());
    }
    let mut values = Vec::new();
    let mut cats = Vec::new();
    for (b, l) in v.fields.iter().zip(labels).take(40) {
        let Some(l) = l else { continue };
        // a sparkline's label already names its measure and axis
        if bicraft_report::query::sparkline(&b.field).is_some() {
            continue;
        }
        match role_class(&b.role) {
            RoleClass::Value => values.push((value_rank(&b.role), l.clone())),
            RoleClass::Category => cats.push(l.clone()),
            RoleClass::Other => {}
        }
    }
    // Desktop names a gauge's value, minimum, maximum and target in that order ("Total, Low,
    // High and Goal"), and a KPI's indicator before its goal ("Total and Goal by Month").
    values.sort_by_key(|(rank, _)| *rank);
    let values: Vec<String> = values.into_iter().map(|(_, l)| l).collect();
    let max_values = if v.visual_type.as_deref() == Some("gauge") {
        TITLE_VALUES + 1
    } else {
        TITLE_VALUES
    };
    let t = lang.auto_title_with(&values, &cats, max_values);
    (!t.is_empty()).then_some(t)
}

/// The auto title of one of BIcraft's visuals (`crate::builtin_visuals`), as Power BI Desktop
/// 2.159.1231.0 titles them (2026-11-24): the fields of its measure roles by those of its grouping
/// roles, each list in the order of the visual's roles ("Total and Goal by Dept", "Total by Id,
/// Parent and Name"), or the grouping fields alone ("Id and Skill").
fn bicraft_visual_title(
    lang: &UiLanguage,
    v: &Visual,
    labels: &[Option<String>],
    roles: &[(&'static str, bicraft_visuals::RoleKind, usize)],
) -> Option<String> {
    let mut values = Vec::new();
    let mut cats = Vec::new();
    for (i, (b, l)) in v.fields.iter().zip(labels).enumerate().take(40) {
        let Some(l) = l else { continue };
        let Some(at) = roles.iter().position(|r| r.0.eq_ignore_ascii_case(&b.role)) else {
            continue;
        };
        match roles.get(at).map(|r| r.1) {
            Some(bicraft_visuals::RoleKind::Grouping) => cats.push((at, i, l.clone())),
            _ => values.push((at, i, l.clone())),
        }
    }
    values.sort();
    cats.sort();
    let values: Vec<String> = values.into_iter().take(TITLE_VALUES).map(|x| x.2).collect();
    let cats: Vec<String> = cats.into_iter().map(|x| x.2).collect();
    let t = if values.is_empty() {
        lang.join_and(&cats)
    } else if cats.is_empty() {
        lang.join_and(&values)
    } else {
        lang.p()
            .title_by
            .replace("{v}", &lang.join_and(&values))
            .replace("{c}", &lang.join_and(&cats))
    };
    let t = cap(&t);
    (!t.is_empty()).then_some(t)
}

/// `{ui_language: "fr-FR"}` → the language (`Ok(None)` when absent).
pub(crate) fn param(p: &Value) -> Result<Option<UiLanguage>, String> {
    match p.get("ui_language") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => UiLanguage::parse(s).map(Some),
        Some(_) => Err("`ui_language` must be a language tag string (e.g. \"fr-FR\")".into()),
    }
}

/// `view.set_language {ui_language}`: the language of the generated visual text in the viewer
/// snapshot and `report.render`.
pub(crate) fn view_set_language(s: &mut crate::Session, p: &Value) -> Result<Value, String> {
    crate::params::allow(p, &["ui_language"])?;
    let lang = param(p)?.ok_or("`ui_language` is required (e.g. \"fr-FR\")")?;
    let previous = std::mem::replace(&mut s.ui_language, lang);
    Ok(json!({
        "ui_language": s.ui_language.tag(),
        "previous": previous.tag(),
        "strings_language": s.ui_language.lang.code(),
        "fallback": s.ui_language.fallback,
        "supported": SUPPORTED_LANGUAGES,
    }))
}

#[cfg(test)]
#[path = "ui_lang_tests.rs"]
mod tests;
