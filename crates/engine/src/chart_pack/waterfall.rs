//! BIcraft Waterfall+ (docs/formats/chart-pack.md "Waterfall+").

use serde_json::{Value, json};

use super::{Def, Object, Prop, PropKind, Role};

pub(crate) const DEF: Def = Def {
    kind: "waterfall",
    guid: "bicraftWaterfallPlus3C5E7A9B1D2F4A6C8E0B",
    name: "bicraftWaterfallPlus",
    display_name: "BIcraft Waterfall+",
    version: "1.0.4.0",
    description: "A waterfall: a running total step by step with subtotals, a total and a target, vertical or horizontal, coloured by sign, category or rule; several measures become the steps (BIcraft, docs/formats/chart-pack.md)",
    roles: &[
        Role {
            name: "category",
            kind: "Grouping",
            display: "Category",
            required: false,
        },
        Role {
            name: "group",
            kind: "Grouping",
            display: "Group (subtotals)",
            required: false,
        },
        Role {
            name: "subtotal",
            kind: "Grouping",
            display: "Subtotal flag",
            required: false,
        },
        Role {
            name: "target",
            kind: "Measure",
            display: "Target",
            required: false,
        },
        Role {
            name: "values",
            kind: "Measure",
            display: "Values",
            required: true,
        },
    ],
    objects: &[
        super::STYLE,
        Object {
            name: "waterfall",
            display: "Waterfall",
            props: &[
                Prop {
                    name: "orientation",
                    display: "Orientation",
                    kind: PropKind::Enum(&[("vertical", "Vertical"), ("horizontal", "Horizontal")]),
                    key: "orientation",
                },
                Prop {
                    name: "showTotal",
                    display: "Show the total",
                    kind: PropKind::Bool,
                    key: "showTotal",
                },
                Prop {
                    name: "subtotalEvery",
                    display: "Subtotal every N steps (0: none)",
                    kind: PropKind::Numeric,
                    key: "subtotalEvery",
                },
                Prop {
                    name: "connectors",
                    display: "Connectors",
                    kind: PropKind::Bool,
                    key: "showConnectors",
                },
            ],
        },
        Object {
            name: "colors",
            display: "Colors",
            props: &[
                Prop {
                    name: "colorBy",
                    display: "Color by",
                    kind: PropKind::Enum(&[
                        ("sign", "By sign"),
                        ("category", "By category"),
                        ("rule", "By rule"),
                    ]),
                    key: "colorBy",
                },
                Prop {
                    name: "increase",
                    display: "Increase",
                    kind: PropKind::Fill,
                    key: "increaseFill",
                },
                Prop {
                    name: "decrease",
                    display: "Decrease",
                    kind: PropKind::Fill,
                    key: "decreaseFill",
                },
                Prop {
                    name: "total",
                    display: "Total",
                    kind: PropKind::Fill,
                    key: "totalFill",
                },
                Prop {
                    name: "ruleValue",
                    display: "Rule threshold",
                    kind: PropKind::Numeric,
                    key: "ruleValue",
                },
                Prop {
                    name: "ruleAbove",
                    display: "At or above the threshold",
                    kind: PropKind::Fill,
                    key: "ruleAbove",
                },
                Prop {
                    name: "ruleBelow",
                    display: "Below the threshold",
                    kind: PropKind::Fill,
                    key: "ruleBelow",
                },
            ],
        },
        Object {
            name: "labels",
            display: "Labels",
            props: &[
                Prop {
                    name: "mode",
                    display: "Show",
                    kind: PropKind::Enum(&[
                        ("value", "Value"),
                        ("percent", "Percent"),
                        ("both", "Value and percent"),
                        ("none", "None"),
                    ]),
                    key: "labelMode",
                },
                Prop {
                    name: "fontSize",
                    display: "Text size",
                    kind: PropKind::Numeric,
                    key: "fontSize",
                },
            ],
        },
        Object {
            name: "target",
            display: "Target",
            props: &[Prop {
                name: "fill",
                display: "Target color",
                kind: PropKind::Fill,
                key: "targetFill",
            }],
        },
    ],
    visual_ts: include_str!("waterfall_visual.ts"),
    layout_ts: ("waterfall.ts", include_str!("waterfall.ts")),
    categorical,
};

fn categorical() -> Value {
    json!({
        "categories": {
            "select": [{"bind": {"to": "category"}}, {"bind": {"to": "group"}}, {"bind": {"to": "subtotal"}}],
            "dataReductionAlgorithm": {"top": {"count": bicraft_render::chart_pack::waterfall::MAX_ROWS}}
        },
        "values": {"select": [{"bind": {"to": "values"}}, {"bind": {"to": "target"}}]}
    })
}
