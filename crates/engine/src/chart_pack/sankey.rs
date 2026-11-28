//! BIcraft Sankey (docs/formats/chart-pack.md "Sankey").

use serde_json::{Value, json};

use super::{Def, Object, Prop, PropKind, Role};

pub(crate) const DEF: Def = Def {
    kind: "sankey",
    guid: "bicraftSankey7B3D5F1A9C2E4B6D8F0A1C3E",
    name: "bicraftSankey",
    display_name: "BIcraft Sankey",
    version: "1.3.8.0",
    description: "A Sankey diagram: weighted links from sources to destinations as ribbons between nodes in columns (BIcraft, docs/formats/chart-pack.md)",
    roles: &[
        Role {
            name: "source",
            kind: "Grouping",
            display: "Source",
            required: true,
        },
        Role {
            name: "destination",
            kind: "Grouping",
            display: "Destination",
            required: true,
        },
        Role {
            name: "weight",
            kind: "Measure",
            display: "Weight",
            required: true,
        },
    ],
    objects: &[
        super::STYLE,
        Object {
            name: "nodes",
            display: "Nodes",
            props: &[
                Prop {
                    name: "width",
                    display: "Width",
                    kind: PropKind::Numeric,
                    key: "nodeWidth",
                },
                Prop {
                    name: "padding",
                    display: "Spacing",
                    kind: PropKind::Numeric,
                    key: "nodePadding",
                },
            ],
        },
        Object {
            name: "colors",
            display: "Node colors",
            props: &[
                node_color("node1", "Node 1", "nodeColor1"),
                node_color("node2", "Node 2", "nodeColor2"),
                node_color("node3", "Node 3", "nodeColor3"),
                node_color("node4", "Node 4", "nodeColor4"),
                node_color("node5", "Node 5", "nodeColor5"),
                node_color("node6", "Node 6", "nodeColor6"),
                node_color("node7", "Node 7", "nodeColor7"),
                node_color("node8", "Node 8", "nodeColor8"),
                node_color("node9", "Node 9", "nodeColor9"),
                node_color("node10", "Node 10", "nodeColor10"),
                node_color("node11", "Node 11", "nodeColor11"),
                node_color("node12", "Node 12", "nodeColor12"),
            ],
        },
        Object {
            name: "links",
            display: "Links",
            props: &[
                Prop {
                    name: "colorBy",
                    display: "Color by",
                    kind: PropKind::Enum(&[
                        ("source", "Source"),
                        ("destination", "Destination"),
                        ("fixed", "One color"),
                    ]),
                    key: "linkColor",
                },
                Prop {
                    name: "fill",
                    display: "Color",
                    kind: PropKind::Fill,
                    key: "linkFill",
                },
                Prop {
                    name: "opacity",
                    display: "Opacity (%)",
                    kind: PropKind::Numeric,
                    key: "linkOpacity",
                },
            ],
        },
        Object {
            name: "labels",
            display: "Labels",
            props: &[
                Prop {
                    name: "show",
                    display: "Show",
                    kind: PropKind::Bool,
                    key: "showLabels",
                },
                Prop {
                    name: "fontSize",
                    display: "Text size",
                    kind: PropKind::Numeric,
                    key: "fontSize",
                },
            ],
        },
    ],
    visual_ts: include_str!("sankey_visual.ts"),
    layout_ts: ("sankey.ts", include_str!("sankey.ts")),
    categorical,
};

fn categorical() -> Value {
    json!({
        "categories": {
            "select": [{"bind": {"to": "source"}}, {"bind": {"to": "destination"}}],
            "dataReductionAlgorithm": {"top": {"count": bicraft_render::chart_pack::sankey::MAX_ROWS}}
        },
        "values": {"select": [{"bind": {"to": "weight"}}]}
    })
}

/// A node colour of the format pane: `colors.<name>`, the layout's `<key>` (none set: the look's
/// palette colour of that node, by its order seen).
const fn node_color(name: &'static str, display: &'static str, key: &'static str) -> Prop {
    Prop {
        name,
        display,
        kind: PropKind::Fill,
        key,
    }
}
