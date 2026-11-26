//! BIcraft Org Chart (docs/formats/chart-pack.md "Organization chart").

use serde_json::{Value, json};

use super::{Def, Object, Prop, PropKind, Role};

pub(crate) const DEF: Def = Def {
    kind: "org",
    guid: "bicraftOrgChart3C8E1A6F2D4B9E7A5C1F0D2B",
    name: "bicraftOrgChart",
    display_name: "BIcraft Org Chart",
    version: "1.3.5.0",
    description: "An organization or hierarchy chart from id and parent id columns, drawn as a tidy tree with collapsible nodes (BIcraft, docs/formats/chart-pack.md)",
    roles: &[
        Role {
            name: "id",
            kind: "Grouping",
            display: "Id",
            required: true,
        },
        Role {
            name: "parent",
            kind: "Grouping",
            display: "Parent id",
            required: true,
        },
        Role {
            name: "label",
            kind: "Grouping",
            display: "Label",
            required: false,
        },
        Role {
            name: "value",
            kind: "Measure",
            display: "Value",
            required: false,
        },
    ],
    objects: &[
        super::STYLE,
        Object {
            name: "nodes",
            display: "Nodes",
            props: &[
                Prop {
                    name: "fill",
                    display: "Color",
                    kind: PropKind::Fill,
                    key: "color",
                },
                Prop {
                    name: "fontColor",
                    display: "Text color",
                    kind: PropKind::Fill,
                    key: "textColor",
                },
                Prop {
                    name: "fontSize",
                    display: "Text size",
                    kind: PropKind::Numeric,
                    key: "fontSize",
                },
                Prop {
                    name: "showValues",
                    display: "Show values",
                    kind: PropKind::Bool,
                    key: "showValues",
                },
            ],
        },
        Object {
            name: "tree",
            display: "Tree",
            props: &[
                Prop {
                    name: "levels",
                    display: "Open levels (0: all)",
                    kind: PropKind::Numeric,
                    key: "levels",
                },
                Prop {
                    name: "toggled",
                    display: "Collapsed or expanded nodes",
                    kind: PropKind::TextList,
                    key: "toggled",
                },
                Prop {
                    name: "compact",
                    display: "Compact tree",
                    kind: PropKind::Bool,
                    key: "compact",
                },
                Prop {
                    name: "leftToRight",
                    display: "Left to right",
                    kind: PropKind::Bool,
                    key: "leftToRight",
                },
            ],
        },
    ],
    visual_ts: include_str!("org_visual.ts"),
    layout_ts: ("org.ts", include_str!("org.ts")),
    categorical,
};

fn categorical() -> Value {
    json!({
        "categories": {
            "select": [{"bind": {"to": "id"}}, {"bind": {"to": "parent"}}, {"bind": {"to": "label"}}],
            "dataReductionAlgorithm": {"top": {"count": bicraft_render::chart_pack::org::MAX_NODES}}
        },
        "values": {"select": [{"bind": {"to": "value"}}]}
    })
}
