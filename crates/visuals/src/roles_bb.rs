//! The data roles of the built-in visual types, established from public documentation and
//! black-box tests against Power BI Desktop (`docs/formats/visual-roles-bb.md`, sources in
//! `docs/formats/visual-roles-bb.sources.md`).
//!
//! Every fact here was observed: a field dragged into a field well of Desktop is saved under
//! that well's role name; the well shows whether it takes several fields and how that changes
//! when other wells are filled; a measure dropped on a well is kept or refused; a text column is
//! kept as a column or summarized. Facts not established are left out, so the checks built on
//! them stay permissive: a type whose wells were not all identified is not checked at all.

use crate::{DataRole, RoleBound, RoleCondition, RoleKind};

/// What a role takes, as observed: a measure dropped on its well is refused (`G`), or kept and a
/// text column dropped on it is summarized (`M`), or both kept as they are (`GM`).
#[derive(Clone, Copy)]
enum K {
    G,
    M,
    GM,
}

/// One role: name, kind, most fields (`None`: the well takes several, no limit observed), and
/// whether several fields form a drill-down hierarchy (Desktop gives the role's fields an
/// `active` flag when it saves, bb:wells-A/R; the chart axes also show drill buttons, bb:wells-C).
type R = (&'static str, K, Option<u32>, bool);

/// One bound of an accepted combination.
#[derive(Clone, Copy)]
struct Bd {
    role: &'static str,
    min: u32,
    max: Option<u32>,
    /// In this combination the role takes only measures or summarized columns (`M`), or only
    /// columns that are not summarized (`G`).
    kind: Option<K>,
    /// Several fields of the role must be related along one chain of relationships.
    related: bool,
}

const fn b(role: &'static str, min: u32, max: Option<u32>) -> Bd {
    Bd {
        role,
        min,
        max,
        kind: None,
        related: false,
    }
}

const fn bk(role: &'static str, min: u32, max: Option<u32>, kind: K) -> Bd {
    Bd {
        role,
        min,
        max,
        kind: Some(kind),
        related: false,
    }
}

const fn related(role: &'static str, min: u32) -> Bd {
    Bd {
        role,
        min,
        max: None,
        kind: None,
        related: true,
    }
}

/// One accepted combination: every bound holds.
type C = &'static [Bd];

struct Spec {
    ty: &'static str,
    roles: &'static [R],
    conditions: &'static [C],
    required: &'static [&'static [&'static str]],
    /// Every well of the type was identified and its kind and limits observed: the engine may
    /// check bindings against it. `false`: only the role names are given.
    complete: bool,
}

const N: Option<u32> = None;
const ONE: Option<u32> = Some(1);

/// The wells of the column, bar, area and ribbon charts (bb:wells-A/B/C): a drill axis, values,
/// one legend field, small multiples, tooltips.
const BARS: &[R] = &[
    ("Category", K::G, N, true),
    ("Y", K::M, N, false),
    ("Series", K::G, ONE, false),
    ("Rows", K::G, N, false),
    ("Tooltips", K::M, N, false),
];

/// With a legend field, the values well takes one field (bb:wells-C, bb:pilot-sery2).
const LEGEND_ONE_Y: &[C] = &[&[b("Series", 0, Some(0))], &[b("Series", 1, ONE), b("Y", 0, ONE)]];

/// Line and area charts: like [`BARS`] plus a secondary values well.
const LINES: &[R] = &[
    ("Category", K::G, N, true),
    ("Y", K::M, N, false),
    ("Y2", K::M, N, false),
    ("Series", K::G, ONE, false),
    ("Rows", K::G, N, false),
    ("Tooltips", K::M, N, false),
];

/// With a legend field, one value in all: in `Y` or in `Y2` (bb:wells-A refused the legend with
/// both filled, bb:wells-R refused `Y` with the legend and `Y2` filled, bb:wells-C).
const LINES_COND: &[C] = &[
    &[b("Series", 0, Some(0))],
    &[b("Series", 1, ONE), b("Y", 0, ONE), b("Y2", 0, Some(0))],
    &[b("Series", 1, ONE), b("Y", 0, Some(0)), b("Y2", 0, ONE)],
];

/// The combo charts: the legend applies to the columns (`Y`) only.
const COMBO: &[R] = LINES;

/// Pie and donut: the legend is the drill axis, details split the slices.
const PIES: &[R] = &[
    ("Category", K::G, N, true),
    ("Y", K::M, N, false),
    ("Series", K::G, ONE, false),
    ("Tooltips", K::M, N, false),
];

/// With details (`Category`), X and Y must be summarized, one field each (bb:kinds-s09, s10, s13,
/// s14 show an error; s01, s07 draw); without details raw columns draw (bb:kinds-s03, s04, s05,
/// s11). Several X columns with a raw Y column also fail (bb:kinds-s12) but X counts as one
/// field per drill level, so that case is not caught (permissive).
/// A play axis asks the same (render `pair X+Play` with a raw X column: "remove the play axis or
/// summarize the axes").
const SCATTER_COND: &[C] = &[
    &[b("Category", 0, Some(0)), b("Play", 0, Some(0))],
    &[b("Category", 1, N), bk("X", 0, ONE, K::M), bk("Y", 0, ONE, K::M)],
    &[b("Play", 1, ONE), bk("X", 0, ONE, K::M), bk("Y", 0, ONE, K::M)],
];

/// A waterfall breakdown without a category draws an empty chart (render `pair Breakdown+Y`).
const WATERFALL_COND: &[C] = &[
    &[b("Breakdown", 0, Some(0))],
    &[b("Breakdown", 1, ONE), b("Category", 1, N)],
];

/// Several fields of a slicer must be related along one chain of active relationships
/// (bb:kinds-r01..r12, l01, l02, l05, l06).
const SLICER_COND: &[C] = &[&[related("Values", 0)]];

/// The list slicer also needs a `Values` field: with tooltips only it shows a "not enough
/// columns" error (render `single Tooltips`).
const LIST_SLICER_COND: &[C] = &[&[related("Values", 1)]];

/// The key influencers need a field in `Target` and one in `ExplainBy` (render `single Target`,
/// `single ExplainBy`: a "no field" message), and `Details` only with a summarized `Target`
/// (render `pair Target+Details`, `all one` with a raw column: an error).
const KEY_DRIVERS_COND: &[C] = &[
    &[
        b("Target", 1, ONE),
        b("ExplainBy", 1, N),
        b("Details", 0, Some(0)),
    ],
    &[bk("Target", 1, ONE, K::M), b("ExplainBy", 1, N)],
];

/// Groups of roles of which one must hold a field, or the visual draws nothing: render tests, each
/// role alone (only the roles listed draw data).
type Q = &'static [&'static [&'static str]];
const REQ_ADVANCED_SLICER: Q = &[&["Values"]];
const REQ_LINES: Q = &[&["Y", "Y2"]];
const REQ_BARS: Q = &[&["Y"]];
const REQ_CARD_VISUAL: Q = &[&["Data", "Rows"]];
const REQ_DECOMPOSITION: Q = &[&["Analyze"]];
const REQ_PIES: Q = &[&["Y"]];
const REQ_FILLED_MAP: Q = &[];
const REQ_FUNNEL: Q = &[&["Y"]];
/// The Gantt needs its task row (Learn: only the row label is required; bb:gantt-R: no other
/// well takes a field before it).
const REQ_GANTT: Q = &[&["Task"]];
const REQ_GAUGE: Q = &[&["Y"]];
const REQ_KEY_DRIVERS: Q = &[];
const REQ_KPI: Q = &[&["Indicator"], &["TrendLine"]];
const REQ_COMBO: Q = &[&["Y", "Y2"]];
const REQ_LIST_SLICER: Q = &[&["Values"]];
const REQ_MAP: Q = &[];
const REQ_PIVOT: Q = &[];
const REQ_SCATTER: Q = &[];
const REQ_SHAPE_MAP: Q = &[];
const REQ_TREEMAP: Q = &[&["Values"]];
const REQ_WATERFALL: Q = &[&["Y"]];

/// A type with no field well whose behaviour with fields was not established: not checked.
macro_rules! unchecked {
    ($ty:literal) => {
        Spec {
            ty: $ty,
            roles: &[],
            conditions: &[],
            required: &[],
            complete: false,
        }
    };
}

macro_rules! spec {
    ($ty:literal, $roles:expr, $conditions:expr, $required:expr) => {
        Spec {
            ty: $ty,
            roles: $roles,
            conditions: $conditions,
            required: $required,
            complete: true,
        }
    };
}

/// Sorted by type name.
static SPECS: &[Spec] = &[
    unchecked!("actionButton"),
    spec!(
        "advancedSlicerVisual",
        &[
            ("Values", K::G, ONE, false),
            ("Label", K::M, ONE, false),
            ("Tooltips", K::M, N, false)
        ],
        &[],
        REQ_ADVANCED_SLICER
    ),
    unchecked!("aiNarratives"),
    spec!("areaChart", LINES, LINES_COND, REQ_LINES),
    spec!(
        "azureMap",
        &[
            ("Category", K::G, N, true),
            ("Y", K::M, ONE, false),
            ("X", K::M, ONE, false),
            ("Series", K::G, ONE, false),
            ("Size", K::M, ONE, false),
            ("Tooltips", K::M, N, false),
            ("PathID", K::G, ONE, false),
            ("PointOrder", K::G, ONE, false),
        ],
        &[],
        &[]
    ),
    spec!("barChart", BARS, LEGEND_ONE_Y, REQ_BARS),
    unchecked!("bookmarkNavigator"),
    spec!("card", &[("Values", K::M, ONE, false)], &[], &[]),
    spec!(
        "cardVisual",
        &[
            ("Data", K::M, N, false),
            ("Rows", K::G, N, false),
            ("Tooltips", K::M, N, false)
        ],
        &[],
        REQ_CARD_VISUAL
    ),
    spec!("clusteredBarChart", BARS, LEGEND_ONE_Y, REQ_BARS),
    spec!("clusteredColumnChart", BARS, LEGEND_ONE_Y, REQ_BARS),
    spec!("columnChart", BARS, LEGEND_ONE_Y, REQ_BARS),
    spec!(
        "decompositionTreeVisual",
        &[
            ("Analyze", K::M, ONE, false),
            ("ExplainBy", K::G, N, false),
            ("Tooltips", K::M, N, false)
        ],
        &[],
        REQ_DECOMPOSITION
    ),
    spec!("donutChart", PIES, LEGEND_ONE_Y, REQ_PIES),
    spec!(
        "filledMap",
        &[
            ("Category", K::G, N, true),
            ("Series", K::G, ONE, false),
            ("Y", K::M, ONE, false),
            ("X", K::M, ONE, false),
            ("Tooltips", K::M, N, false),
        ],
        &[],
        REQ_FILLED_MAP
    ),
    spec!(
        "funnel",
        &[
            ("Category", K::G, N, true),
            ("Y", K::M, N, false),
            ("Tooltips", K::M, N, false)
        ],
        &[
            &[b("Category", 0, Some(0))],
            &[b("Category", 1, N), b("Y", 0, ONE)]
        ],
        REQ_FUNNEL
    ),
    // Desktop 2.159.1231.0 (bb:gantt-A/B/C/R): `StartDate` and `EndDate` keep a date measure
    // and summarize a date column (earliest, latest), refusing text columns and numeric
    // measures; `Category` takes up to three levels with no drill flag; the other wells take a
    // field only once `Task` has one.
    spec!(
        "ganttChart",
        &[
            ("Task", K::G, ONE, false),
            ("StartDate", K::M, ONE, false),
            ("EndDate", K::M, ONE, false),
            ("Progress", K::M, ONE, false),
            ("Category", K::G, Some(3), false),
            ("Columns", K::M, N, false),
            ("Tooltips", K::M, N, false),
        ],
        &[],
        REQ_GANTT
    ),
    spec!(
        "gauge",
        &[
            ("Y", K::M, ONE, false),
            ("MinValue", K::M, ONE, false),
            ("MaxValue", K::M, ONE, false),
            ("TargetValue", K::M, ONE, false),
            ("Tooltips", K::M, N, false),
        ],
        &[],
        REQ_GAUGE
    ),
    spec!("hundredPercentStackedAreaChart", BARS, LEGEND_ONE_Y, REQ_BARS),
    spec!("hundredPercentStackedBarChart", BARS, LEGEND_ONE_Y, REQ_BARS),
    spec!("hundredPercentStackedColumnChart", BARS, LEGEND_ONE_Y, REQ_BARS),
    unchecked!("image"),
    spec!(
        "keyDriversVisual",
        &[
            ("Target", K::GM, ONE, false),
            ("ExplainBy", K::GM, N, false),
            ("Details", K::G, N, false)
        ],
        KEY_DRIVERS_COND,
        REQ_KEY_DRIVERS
    ),
    spec!(
        "kpi",
        &[
            ("Indicator", K::M, ONE, false),
            ("TrendLine", K::G, ONE, false),
            ("Goal", K::M, Some(2), false)
        ],
        &[],
        REQ_KPI
    ),
    spec!("lineChart", LINES, LINES_COND, REQ_LINES),
    spec!("lineClusteredColumnComboChart", COMBO, LEGEND_ONE_Y, REQ_COMBO),
    spec!("lineStackedColumnComboChart", COMBO, LEGEND_ONE_Y, REQ_COMBO),
    spec!(
        "listSlicer",
        &[("Values", K::G, N, true), ("Tooltips", K::M, N, false)],
        LIST_SLICER_COND,
        REQ_LIST_SLICER
    ),
    spec!(
        "map",
        &[
            ("Category", K::G, N, true),
            ("Series", K::G, ONE, false),
            ("Y", K::M, ONE, false),
            ("X", K::M, ONE, false),
            ("Size", K::M, ONE, false),
            ("Tooltips", K::M, N, false),
        ],
        &[],
        REQ_MAP
    ),
    spec!("multiRowCard", &[("Values", K::GM, N, false)], &[], &[]),
    unchecked!("pageNavigator"),
    spec!("pieChart", PIES, LEGEND_ONE_Y, REQ_PIES),
    spec!(
        "pivotTable",
        &[
            ("Rows", K::G, N, true),
            ("Columns", K::G, N, true),
            ("Values", K::M, N, false)
        ],
        &[],
        REQ_PIVOT
    ),
    spec!("pythonVisual", &[("Values", K::GM, N, false)], &[], &[]),
    unchecked!("qnaVisual"),
    spec!("rdlVisual", &[("parameterValues", K::GM, N, false)], &[], &[]),
    spec!("ribbonChart", BARS, LEGEND_ONE_Y, REQ_BARS),
    spec!(
        "scatterChart",
        &[
            ("Category", K::G, N, true),
            ("X", K::GM, N, true),
            ("Y", K::M, ONE, false),
            ("Series", K::G, ONE, false),
            ("Size", K::M, ONE, false),
            ("Play", K::G, ONE, false),
            ("Tooltips", K::M, N, false),
        ],
        SCATTER_COND,
        REQ_SCATTER
    ),
    // No well, and a field makes Desktop show an error (bb:kinds-t03).
    spec!("scorecard", &[], &[], &[]),
    spec!("scriptVisual", &[("Values", K::GM, N, false)], &[], &[]),
    unchecked!("shape"),
    spec!(
        "shapeMap",
        &[
            ("Category", K::G, ONE, false),
            ("Series", K::G, ONE, false),
            ("Value", K::M, ONE, false),
            ("Tooltips", K::M, N, false),
        ],
        &[],
        REQ_SHAPE_MAP
    ),
    spec!("slicer", &[("Values", K::G, N, true)], SLICER_COND, &[]),
    spec!("stackedAreaChart", BARS, LEGEND_ONE_Y, REQ_BARS),
    spec!("tableEx", &[("Values", K::GM, N, false)], &[], &[]),
    unchecked!("textbox"),
    spec!("textSlicer", &[("Values", K::G, ONE, false)], &[], &[]),
    spec!(
        "treemap",
        &[
            ("Group", K::G, N, true),
            ("Details", K::G, ONE, false),
            ("Values", K::M, N, false),
            ("Tooltips", K::M, N, false),
        ],
        &[
            &[b("Details", 0, Some(0))],
            &[b("Details", 1, ONE), b("Values", 0, ONE)]
        ],
        REQ_TREEMAP
    ),
    spec!(
        "waterfallChart",
        &[
            ("Category", K::G, N, true),
            ("Breakdown", K::G, ONE, false),
            ("Y", K::M, ONE, false),
            ("Tooltips", K::M, N, false),
        ],
        WATERFALL_COND,
        REQ_WATERFALL
    ),
];

/// The known roles of a visual type: `None` when they were not established.
pub(crate) struct Known {
    pub data_roles: Vec<DataRole>,
    pub conditions: Vec<RoleCondition>,
    pub required: Vec<Vec<String>>,
}

fn kind(k: K) -> RoleKind {
    match k {
        K::G => RoleKind::Grouping,
        K::M => RoleKind::Measure,
        K::GM => RoleKind::GroupingOrMeasure,
    }
}

/// The role names observed for a visual type, in the order of its field wells (empty: none
/// known).
pub(crate) fn role_names(visual_type: &str) -> Vec<String> {
    SPECS
        .iter()
        .find(|s| s.ty == visual_type)
        .map(|s| s.roles.iter().map(|r| r.0.to_string()).collect())
        .unwrap_or_default()
}

/// The roles of a visual type whose wells were all established (`None` otherwise: no checks).
pub(crate) fn roles_of(visual_type: &str) -> Option<Known> {
    let s = SPECS.iter().find(|s| s.ty == visual_type && s.complete)?;
    let data_roles = s
        .roles
        .iter()
        .map(|&(name, k, max, drill)| DataRole {
            name: name.to_string(),
            kind: kind(k),
            min: 0,
            max: if drill { None } else { max },
            drill,
        })
        .collect();
    let conditions = s
        .conditions
        .iter()
        .map(|c| RoleCondition {
            bounds: c
                .iter()
                .map(|bd| RoleBound {
                    role: bd.role.to_string(),
                    min: bd.min,
                    max: bd.max,
                    kind: bd.kind.map(kind),
                    related: bd.related,
                })
                .collect(),
        })
        .collect();
    let required = s
        .required
        .iter()
        .map(|g| g.iter().map(|r| (*r).to_string()).collect())
        .collect();
    Some(Known {
        data_roles,
        conditions,
        required,
    })
}
