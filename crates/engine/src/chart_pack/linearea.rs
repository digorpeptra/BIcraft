//! BIcraft Line / Area+ (docs/formats/chart-pack.md "Line / Area+").

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
    kind: "linearea",
    guid: "bicraftLineAreaPlus9D2B6F4A8C0E3B5D7F1A",
    name: "bicraftLineAreaPlus",
    display_name: "BIcraft Line / Area+",
    version: "1.0.4.0",
    description: "Lines or areas on a continuous date axis with a band between two measures, threshold shading, labels on the last or extreme points, colours by rule, a dashed forecast and series named at the line ends (BIcraft, docs/formats/chart-pack.md)",
    roles: &[
        Role {
            name: "axis",
            kind: "Grouping",
            display: "Axis",
            required: true,
        },
        Role {
            name: "series",
            kind: "Grouping",
            display: "Series",
            required: false,
        },
        Role {
            name: "value",
            kind: "Measure",
            display: "Value",
            required: true,
        },
        Role {
            name: "value2",
            kind: "Measure",
            display: "Second value",
            required: false,
        },
    ],
    objects: &[
        super::STYLE,
        Object {
            name: "lines",
            display: "Lines",
            props: &[
                prop("color", "Color", PropKind::Fill, "lineColor"),
                prop("width", "Width", PropKind::Numeric, "lineWidth"),
                prop("markers", "Markers", PropKind::Bool, "markers"),
                prop("emphasize", "Emphasized series", PropKind::Text, "emphasize"),
            ],
        },
        Object {
            name: "area",
            display: "Area",
            props: &[
                prop("show", "Show", PropKind::Bool, "area"),
                prop("fill", "Color", PropKind::Fill, "areaFill"),
                prop("opacity", "Opacity (%)", PropKind::Numeric, "areaOpacity"),
                prop("stacked", "Stack the series", PropKind::Bool, "stacked"),
            ],
        },
        Object {
            name: "band",
            display: "Band between values",
            props: &[
                prop("show", "Show", PropKind::Bool, "band"),
                prop("up", "First value above", PropKind::Fill, "bandUp"),
                prop("down", "First value below", PropKind::Fill, "bandDown"),
                prop("opacity", "Opacity (%)", PropKind::Numeric, "bandOpacity"),
            ],
        },
        Object {
            name: "threshold",
            display: "Threshold",
            props: &[
                prop("show", "Show", PropKind::Bool, "threshold"),
                prop("value", "Value", PropKind::Numeric, "thresholdValue"),
                prop(
                    "side",
                    "Shade",
                    PropKind::Enum(&[("above", "Above"), ("below", "Below")]),
                    "thresholdSide",
                ),
                prop("color", "Color", PropKind::Fill, "thresholdColor"),
            ],
        },
        Object {
            name: "rule",
            display: "Color rule",
            props: &[
                prop(
                    "mode",
                    "Mode",
                    PropKind::Enum(&[
                        ("none", "None"),
                        ("above", "Above a value"),
                        ("below", "Below a value"),
                    ]),
                    "colorRule",
                ),
                prop("value", "Value", PropKind::Numeric, "ruleValue"),
                prop("color", "Color", PropKind::Fill, "ruleColor"),
            ],
        },
        Object {
            name: "forecast",
            display: "Forecast",
            props: &[prop(
                "from",
                "From date (yyyy-mm-dd or today)",
                PropKind::Text,
                "forecastFrom",
            )],
        },
        Object {
            name: "labels",
            display: "Labels",
            props: &[
                prop(
                    "mode",
                    "Mode",
                    PropKind::Enum(&[
                        ("last", "Last point"),
                        ("minmax", "Lowest and highest"),
                        ("all", "All points"),
                        ("none", "None"),
                    ]),
                    "labelMode",
                ),
                prop("fontSize", "Text size", PropKind::Numeric, "fontSize"),
                prop(
                    "endLabels",
                    "Series names at line ends",
                    PropKind::Bool,
                    "endLabels",
                ),
            ],
        },
        Object {
            name: "axes",
            display: "Axes",
            props: &[
                prop(
                    "type",
                    "Axis type",
                    PropKind::Enum(&[("auto", "Automatic"), ("category", "Categories")]),
                    "axisType",
                ),
                prop("zero", "Start at zero", PropKind::Bool, "yZero"),
            ],
        },
        Object {
            name: "legend",
            display: "Legend",
            props: &[prop("show", "Show", PropKind::Bool, "showLegend")],
        },
    ],
    visual_ts: include_str!("linearea_visual.ts"),
    layout_ts: ("linearea.ts", include_str!("linearea.ts")),
    categorical,
};

fn categorical() -> Value {
    json!({
        "categories": {
            "select": [{"bind": {"to": "axis"}}, {"bind": {"to": "series"}}],
            "dataReductionAlgorithm": {"top": {"count": bicraft_render::chart_pack::linearea::MAX_ROWS}}
        },
        "values": {"select": [{"bind": {"to": "value"}}, {"bind": {"to": "value2"}}]}
    })
}
