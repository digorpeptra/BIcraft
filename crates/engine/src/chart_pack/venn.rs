//! BIcraft Venn (docs/formats/chart-pack.md "Venn diagram").

use serde_json::{Value, json};

use super::{Def, Object, Prop, PropKind, Role};

pub(crate) const DEF: Def = Def {
    kind: "venn",
    guid: "bicraftVenn9A2E4C6B8D1F3A5E7C9B0D2F",
    name: "bicraftVenn",
    display_name: "BIcraft Venn",
    version: "1.3.4.0",
    description: "A Venn diagram of two or three sets from an item and set membership table, with the count of items in each region (BIcraft, docs/formats/chart-pack.md)",
    roles: &[
        Role {
            name: "item",
            kind: "Grouping",
            display: "Item",
            required: true,
        },
        Role {
            name: "set",
            kind: "Grouping",
            display: "Set",
            required: true,
        },
    ],
    objects: &[
        super::STYLE,
        Object {
            name: "sets",
            display: "Sets",
            props: &[
                Prop {
                    name: "color1",
                    display: "First set",
                    kind: PropKind::Fill,
                    key: "color1",
                },
                Prop {
                    name: "color2",
                    display: "Second set",
                    kind: PropKind::Fill,
                    key: "color2",
                },
                Prop {
                    name: "color3",
                    display: "Third set",
                    kind: PropKind::Fill,
                    key: "color3",
                },
                Prop {
                    name: "proportional",
                    display: "Areas as the counts (two sets)",
                    kind: PropKind::Bool,
                    key: "proportional",
                },
            ],
        },
        Object {
            name: "labels",
            display: "Counts",
            props: &[
                Prop {
                    name: "show",
                    display: "Show",
                    kind: PropKind::Bool,
                    key: "showCounts",
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
    visual_ts: include_str!("venn_visual.ts"),
    layout_ts: ("venn.ts", include_str!("venn.ts")),
    categorical,
};

fn categorical() -> Value {
    json!({
        "categories": {
            "select": [{"bind": {"to": "item"}}, {"bind": {"to": "set"}}],
            "dataReductionAlgorithm": {"top": {"count": bicraft_render::chart_pack::venn::MAX_ROWS}}
        }
    })
}
