//! BIcraft Scatter+ (docs/formats/chart-pack.md "Scatter+").

use serde_json::{Value, json};

use super::{Def, Object, Prop, PropKind, Role};

const fn prop(name: &'static str, display: &'static str, kind: PropKind, key: &'static str) -> Prop {
    Prop {
        name,
        display,
        kind,
        key,
    }
}

pub(crate) const DEF: Def = Def {
    kind: "scatter",
    guid: "bicraftScatterPlus4E8A2C6F0B3D5A7C9E1F",
    name: "bicraftScatterPlus",
    display_name: "BIcraft Scatter+",
    version: "1.0.4.0",
    description: "A scatter chart that draws every point up to 10,000, switches to a density view when dense, with marginal distributions, a trend line, quadrants and point labels (BIcraft, docs/formats/chart-pack.md)",
    roles: &[
        Role {
            name: "details",
            kind: "Grouping",
            display: "Details",
            required: true,
        },
        Role {
            name: "legend",
            kind: "Grouping",
            display: "Legend",
            required: false,
        },
        Role {
            name: "x",
            kind: "Measure",
            display: "X axis",
            required: true,
        },
        Role {
            name: "y",
            kind: "Measure",
            display: "Y axis",
            required: true,
        },
        Role {
            name: "size",
            kind: "Measure",
            display: "Size",
            required: false,
        },
    ],
    objects: &[
        super::STYLE,
        Object {
            name: "markers",
            display: "Markers",
            props: &[
                prop("size", "Size", PropKind::Numeric, "markerSize"),
                prop("fill", "Fill", PropKind::Bool, "markerFill"),
                prop("border", "Border", PropKind::Bool, "markerBorder"),
                prop("opacity", "Fill opacity (%)", PropKind::Numeric, "fillOpacity"),
            ],
        },
        Object {
            name: "density",
            display: "Density",
            props: &[
                prop(
                    "mode",
                    "Mode",
                    PropKind::Enum(&[("auto", "Automatic"), ("on", "On"), ("off", "Off")]),
                    "density",
                ),
                prop(
                    "threshold",
                    "Points before density",
                    PropKind::Numeric,
                    "densityThreshold",
                ),
                prop("cellSize", "Cell size", PropKind::Numeric, "hexSize"),
            ],
        },
        Object {
            name: "marginals",
            display: "Marginal distributions",
            props: &[prop(
                "kind",
                "Kind",
                PropKind::Enum(&[
                    ("none", "None"),
                    ("histogram", "Histograms"),
                    ("box", "Box strips"),
                ]),
                "marginals",
            )],
        },
        Object {
            name: "trend",
            display: "Trend line",
            props: &[prop("show", "Show", PropKind::Bool, "trendLine")],
        },
        Object {
            name: "quadrants",
            display: "Quadrants",
            props: &[
                prop("show", "Show", PropKind::Bool, "quadrants"),
                prop(
                    "at",
                    "Lines at",
                    PropKind::Enum(&[("average", "Averages"), ("value", "Given values")]),
                    "quadrantAt",
                ),
                prop("x", "X value", PropKind::Numeric, "quadrantX"),
                prop("y", "Y value", PropKind::Numeric, "quadrantY"),
                prop("topRight", "Top right label", PropKind::Text, "labelTopRight"),
                prop("topLeft", "Top left label", PropKind::Text, "labelTopLeft"),
                prop(
                    "bottomLeft",
                    "Bottom left label",
                    PropKind::Text,
                    "labelBottomLeft",
                ),
                prop(
                    "bottomRight",
                    "Bottom right label",
                    PropKind::Text,
                    "labelBottomRight",
                ),
            ],
        },
        Object {
            name: "labels",
            display: "Labels",
            props: &[
                prop(
                    "mode",
                    "Points labelled",
                    PropKind::Enum(&[
                        ("none", "None"),
                        ("top", "Top points"),
                        ("selected", "Selected points"),
                    ]),
                    "labelMode",
                ),
                prop("count", "How many", PropKind::Numeric, "labelCount"),
                prop("fontSize", "Text size", PropKind::Numeric, "fontSize"),
            ],
        },
        Object {
            name: "legend",
            display: "Legend",
            props: &[prop("show", "Show", PropKind::Bool, "showLegend")],
        },
    ],
    visual_ts: include_str!("scatter_visual.ts"),
    layout_ts: ("scatter.ts", include_str!("scatter.ts")),
    categorical,
};

fn categorical() -> Value {
    json!({
        "categories": {
            "select": [{"bind": {"to": "details"}}, {"bind": {"to": "legend"}}],
            "dataReductionAlgorithm": {"top": {"count": bicraft_render::chart_pack::scatter::MAX_POINTS}}
        },
        "values": {"select": [{"bind": {"to": "x"}}, {"bind": {"to": "y"}}, {"bind": {"to": "size"}}]}
    })
}
