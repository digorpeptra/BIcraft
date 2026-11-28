//! BIcraft Gauge / KPI+ (docs/formats/chart-pack.md "Gauge / KPI+").

use serde_json::{Value, json};

use super::{Def, Object, Prop, PropKind, Role};

const fn measure(name: &'static str, display: &'static str) -> Role {
    Role {
        name,
        kind: "Measure",
        display,
        required: false,
    }
}

const fn numeric(name: &'static str, display: &'static str) -> Prop {
    Prop {
        name,
        display,
        kind: PropKind::Numeric,
        key: name,
    }
}

const fn fill(name: &'static str, display: &'static str, key: &'static str) -> Prop {
    Prop {
        name,
        display,
        kind: PropKind::Fill,
        key,
    }
}

pub(crate) const DEF: Def = Def {
    kind: "gauge",
    guid: "bicraftGaugeKpi9E1A3C5B7D2F4E6A8C0B1D3F",
    name: "bicraftGaugeKpi",
    display_name: "BIcraft Gauge / KPI+",
    version: "1.0.4.0",
    description: "A gauge or KPI: one value against free colour bands, a target and up to three thresholds, as a radial arc, a linear bullet or a KPI card with its variance and a trend sparkline (BIcraft, docs/formats/chart-pack.md)",
    roles: &[
        Role {
            name: "trend",
            kind: "Grouping",
            display: "Trend",
            required: false,
        },
        Role {
            name: "value",
            kind: "Measure",
            display: "Value",
            required: true,
        },
        measure("target", "Target"),
        measure("min", "Minimum"),
        measure("max", "Maximum"),
        measure("threshold1", "First threshold"),
        measure("threshold2", "Second threshold"),
        measure("threshold3", "Third threshold"),
        measure("previous", "Previous value"),
    ],
    objects: &[
        super::STYLE,
        Object {
            name: "gauge",
            display: "Gauge",
            props: &[
                Prop {
                    name: "style",
                    display: "Style",
                    kind: PropKind::Enum(&[
                        ("arc", "Radial arc"),
                        ("bullet", "Linear bullet"),
                        ("kpi", "KPI card"),
                    ]),
                    key: "gaugeStyle",
                },
                Prop {
                    name: "compareTo",
                    display: "Compare with",
                    kind: PropKind::Enum(&[("target", "Target"), ("previous", "Previous value")]),
                    key: "compareTo",
                },
                Prop {
                    name: "lowerIsBetter",
                    display: "Lower is better",
                    kind: PropKind::Bool,
                    key: "lowerIsBetter",
                },
                Prop {
                    name: "showTrend",
                    display: "Show the trend",
                    kind: PropKind::Bool,
                    key: "showTrend",
                },
            ],
        },
        Object {
            name: "range",
            display: "Range",
            props: &[
                numeric("min", "Minimum"),
                numeric("max", "Maximum"),
                numeric("target", "Target"),
                numeric("threshold1", "First threshold"),
                numeric("threshold2", "Second threshold"),
                numeric("threshold3", "Third threshold"),
            ],
        },
        Object {
            name: "colors",
            display: "Colors",
            props: &[
                fill("value", "Value", "valueFill"),
                Prop {
                    name: "valueByBand",
                    display: "Value in its band's color",
                    kind: PropKind::Bool,
                    key: "valueByBand",
                },
                fill("band1", "First band", "band1"),
                fill("band2", "Second band", "band2"),
                fill("band3", "Third band", "band3"),
                fill("band4", "Fourth band", "band4"),
                fill("target", "Target color", "targetFill"),
            ],
        },
    ],
    visual_ts: include_str!("gauge_visual.ts"),
    layout_ts: ("gauge.ts", include_str!("gauge.ts")),
    categorical,
};

fn categorical() -> Value {
    json!({
        "categories": {
            "select": [{"bind": {"to": "trend"}}],
            "dataReductionAlgorithm": {"top": {"count": bicraft_render::chart_pack::gauge::MAX_ROWS}}
        },
        "values": {"select": [
            {"bind": {"to": "value"}}, {"bind": {"to": "target"}}, {"bind": {"to": "min"}},
            {"bind": {"to": "max"}}, {"bind": {"to": "threshold1"}}, {"bind": {"to": "threshold2"}},
            {"bind": {"to": "threshold3"}}, {"bind": {"to": "previous"}}
        ]}
    })
}
