//! L1: the catalog of Power BI visual types and their formatting options (M2.5).
//!
//! For every visual type: its formatting objects (`visual.objects` entries such as `legend`,
//! `categoryAxis`, `dataPoint`, and the container objects of `visualContainerObjects` such as
//! `title`, `background`), each object's properties with their type, allowed values, range and
//! default, and whether the object takes per-series / per-category selectors.
//!
//! FROZEN API (the engine and other M2.5 work build on it; keep the signatures):
//! [`visual_types`], [`visual`], [`object`], [`property`], [`check_value`], and the types below.
//! The catalog data itself (where it comes from, how it is embedded) is owned by this crate.
//!
//! # Where the data comes from
//!
//! - Visual types, `visual.objects` objects, property types, enumerated values, ranges, display
//!   names and descriptions: Microsoft's published report theme JSON schema
//!   ([`THEME_SCHEMA_VERSION`], vendored unmodified under `schemas/`, MIT), parsed once on first
//!   use.
//! - Container objects (`visualContainerObjects`) and their properties: the public PBIR
//!   `visualConfiguration` schema, typed from the theme schema's `commonCards`.
//! - Data roles with their kinds, drill-down roles and accepted field counts
//!   ([`VisualType::data_roles`], [`VisualType::role_conditions`]): public documentation and
//!   black-box tests against Power BI Desktop [`ROLES_DESKTOP_VERSION`], written up with their
//!   evidence in `docs/formats/visual-roles-bb.md` (`roles_bb.rs`).
//! - Which properties take field and rule expressions (every literal-typed one: Desktop keeps a
//!   measure on all of them when it saves), measured with the reproducibility oracle.
//! - Which objects take selectors and the 0–100 range of transparencies: our own reading of the
//!   files Desktop writes.
//!
//! No defaults are published, so [`Property::default`] is `None`.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

mod build;
mod check;
pub mod icons;
mod roles_bb;
mod themes;

pub use themes::{BUILTIN_THEMES, BuiltinTheme, builtin_theme};

use std::sync::OnceLock;

/// The Power BI Desktop release the data roles were observed with (black-box tests,
/// `docs/formats/visual-roles-bb.md`).
pub const ROLES_DESKTOP_VERSION: &str = "2.159.883.0";

use serde::Serialize;
use serde_json::Value;

/// The version of the vendored report theme schema the catalog is built from.
pub const THEME_SCHEMA_VERSION: &str = bicraft_versions::REPORT_THEME_SCHEMA_VERSION;

/// The vendored report theme JSON schema itself (Microsoft, MIT), for callers that validate
/// theme files against it.
pub const THEME_SCHEMA: &str = build::THEME_SCHEMA;

/// The type of a formatting property's value.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PropType {
    Bool,
    /// A number; `integer` means Desktop stores it with an `L` suffix.
    Number {
        min: Option<f64>,
        max: Option<f64>,
        integer: bool,
    },
    Text,
    /// A color (`{"solid": {"color": ...}}`), possibly theme-based.
    Color,
    /// One of a fixed set of string values.
    Enum {
        values: Vec<String>,
    },
    /// A font family.
    Font,
    /// An image reference.
    Image,
    /// A field or measure reference (e.g. conditional formatting by field value).
    Field,
    /// Anything the catalog does not type more precisely (kept, written verbatim).
    Other {
        description: String,
    },
}

/// One formatting property of an object.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Property {
    pub name: String,
    pub display_name: Option<String>,
    pub ty: PropType,
    pub default: Option<Value>,
    pub description: Option<String>,
    /// Whether conditional formatting (rules / gradient / field value) is allowed on it.
    pub conditional: bool,
}

/// One formatting object of a visual type.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Object {
    pub name: String,
    pub display_name: Option<String>,
    /// `true` for `visualContainerObjects` (title, background, border...), `false` for
    /// `visual.objects`.
    pub container: bool,
    /// Whether entries may carry a selector (per series / per category / per measure).
    pub selectors: bool,
    pub properties: Vec<Property>,
}

/// A visual type and its objects.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VisualType {
    pub name: String,
    pub display_name: Option<String>,
    /// Data role names (Category, Y, Legend, Values, Rows...).
    pub roles: Vec<String>,
    pub objects: Vec<Object>,
    /// Whether the data roles below are known (every field well established by black-box tests,
    /// see `docs/formats/visual-roles-bb.md`). `true` with no roles: the type takes no data fields.
    pub roles_known: bool,
    /// The data roles with their kind and field limits (empty when not known).
    pub data_roles: Vec<DataRole>,
    /// The combinations of field counts Desktop accepts: the bound fields must satisfy at least
    /// one. Empty: no limits beyond each role's own.
    pub role_conditions: Vec<RoleCondition>,
    /// Groups of roles of which at least one must hold a field, or Desktop draws an empty visual
    /// (black-box render tests).
    pub required_roles: Vec<Vec<String>>,
}

/// What a data role accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleKind {
    /// Columns (grouping): a measure cannot go there.
    Grouping,
    /// Measures, or columns that Desktop aggregates.
    Measure,
    GroupingOrMeasure,
}

/// One data role of a visual type.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DataRole {
    pub name: String,
    pub kind: RoleKind,
    /// The fewest fields every accepted combination needs (usually 0).
    pub min: u32,
    /// The most fields any accepted combination allows; `None`: no limit (always for drill
    /// roles, whose fields form a hierarchy and are counted per drill level).
    pub max: Option<u32>,
    /// A drill-down role (`drilldown.roles`).
    pub drill: bool,
}

/// A bound on the number of fields in one role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoleBound {
    pub role: String,
    pub min: u32,
    /// `None`: no maximum.
    pub max: Option<u32>,
    /// In this combination the role takes only measures (`Measure`: a measure or a summarized
    /// column) or only columns that are not summarized (`Grouping`); `None`: either (the
    /// a condition's own kind constraint).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<RoleKind>,
    /// Several fields in this role must come from one table or from tables related along one
    /// chain of relationships.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub related: bool,
}

/// One accepted combination (a `dataViewMappings[].conditions[]` entry): every bound holds.
/// Roles it does not name are unbounded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoleCondition {
    pub bounds: Vec<RoleBound>,
}

static CATALOG: OnceLock<Vec<VisualType>> = OnceLock::new();

/// The catalog, sorted by visual type name (built on first use).
fn catalog() -> &'static [VisualType] {
    CATALOG.get_or_init(build::build)
}

/// Every visual type in the catalog, by name.
pub fn visual_types() -> Vec<&'static str> {
    catalog().iter().map(|v| v.name.as_str()).collect()
}

/// A visual type, matched case-sensitively (Desktop's names: `clusteredColumnChart`...).
pub fn visual(name: &str) -> Option<&'static VisualType> {
    let all = catalog();
    all.binary_search_by(|v| v.name.as_str().cmp(name))
        .ok()
        .and_then(|i| all.get(i))
}

/// An object of a visual type (container objects included). When a visual object and a container
/// object share a name (`general`, `border`...), the visual object comes first; see
/// [`object_in`].
pub fn object(visual_type: &str, object: &str) -> Option<&'static Object> {
    visual(visual_type)?.objects.iter().find(|o| o.name == object)
}

/// The object `object` of a visual type among its container objects (`container: true`) or its
/// `visual.objects` (`false`).
pub fn object_in(visual_type: &str, object: &str, container: bool) -> Option<&'static Object> {
    visual(visual_type)?
        .objects
        .iter()
        .find(|o| o.name == object && o.container == container)
}

/// A property of an object of a visual type (the visual object's, else the container object's of
/// that name).
pub fn property(visual_type: &str, object_name: &str, property: &str) -> Option<&'static Property> {
    visual(visual_type)?
        .objects
        .iter()
        .filter(|o| o.name == object_name)
        .find_map(|o| o.properties.iter().find(|p| p.name == property))
}

/// A data role of a visual type, matched case-sensitively; `None` for an unknown type or role.
pub fn data_role(visual_type: &str, role: &str) -> Option<&'static DataRole> {
    visual(visual_type)?.data_roles.iter().find(|r| r.name == role)
}

/// The container objects (`visualContainerObjects`) and their property names, the same for every
/// visual type (public PBIR `visualConfiguration` schema).
pub fn container_objects() -> &'static [(&'static str, &'static [&'static str])] {
    build::CONTAINER_OBJECTS
}

/// Checks a PBIR property value (the `{"expr": ...}` / `{"solid": ...}` form written in
/// `objects`) against the property's type. `Err` explains what is expected.
///
/// Literals are checked against the type (booleans, numbers with their range and wholeness,
/// quoted text, enumerated values, `'#RRGGBB'` colors, `ThemeDataColor` ids). Field and rule
/// expressions (conditional formatting) are accepted on properties marked `conditional`, and
/// refused elsewhere. `Other` properties accept any value.
pub fn check_value(property: &Property, value: &Value) -> Result<(), String> {
    check::check_value(property, value)
}

/// The candidate closest to `word` by edit distance, for "did you mean" hints: an exact match
/// ignoring case, else one at most a third of the word's length away (at least 2 edits allowed).
pub fn suggest<'a>(word: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    const MAX_LEN: usize = 200;
    let w: Vec<char> = word.to_lowercase().chars().collect();
    if w.is_empty() || w.len() > MAX_LEN {
        return None;
    }
    let limit = (w.len() / 3).max(2);
    let mut best: Option<(usize, &str)> = None;
    for c in candidates {
        let cl: Vec<char> = c.to_lowercase().chars().collect();
        if cl.len() > MAX_LEN {
            continue;
        }
        let d = distance(&w, &cl);
        if d <= limit && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, c));
        }
    }
    best.map(|(_, c)| c)
}

/// Levenshtein distance between two short strings.
fn distance(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len().saturating_add(1)];
    for (i, ca) in a.iter().enumerate() {
        if let Some(first) = cur.first_mut() {
            *first = i.saturating_add(1);
        }
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            let sub = prev.get(j).copied().unwrap_or(usize::MAX).saturating_add(cost);
            let del = prev.get(j + 1).copied().unwrap_or(usize::MAX).saturating_add(1);
            let ins = cur.get(j).copied().unwrap_or(usize::MAX).saturating_add(1);
            if let Some(slot) = cur.get_mut(j + 1) {
                *slot = sub.min(del).min(ins);
            }
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev.last().copied().unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests_bb;
