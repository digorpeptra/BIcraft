//! The BIcraft Bullet chart (docs/formats/bicraft-visuals.md "Bullet chart"): a value against its
//! target over qualitative bands, one horizontal bullet per category.

use serde_json::{Value, json};

use super::{Def, Prop, PropKind, Role};

const PCT: PropKind = PropKind::Num {
    min: 0.0,
    max: 1000.0,
    int: false,
};

pub(super) const DEF: Def = Def {
    kind: "bullet",
    guid: "bicraftBullet9A3D6F1C8E2B5A7D4F0E",
    name: "bicraftBullet",
    display_name: "BIcraft Bullet",
    version: "1.3.3.0",
    description: "A value against its target over qualitative bands (BIcraft, docs/formats/bicraft-visuals.md)",
    roles: &[
        Role {
            name: "category",
            kind: "Grouping",
            display: "Category",
            max: 1,
        },
        Role {
            name: "value",
            kind: "Measure",
            display: "Value",
            max: 1,
        },
        Role {
            name: "target",
            kind: "Measure",
            display: "Target",
            max: 1,
        },
        Role {
            name: "bands",
            kind: "Measure",
            display: "Bands",
            max: 4,
        },
        Role {
            name: "maximum",
            kind: "Measure",
            display: "Maximum",
            max: 1,
        },
    ],
    required: &["value"],
    one_column: None,
    props: &[
        Prop {
            object: "bands",
            property: "first",
            kind: PCT,
            display: "First band (% of target)",
        },
        Prop {
            object: "bands",
            property: "second",
            kind: PCT,
            display: "Second band (% of target)",
        },
        Prop {
            object: "bands",
            property: "third",
            kind: PCT,
            display: "Third band (% of target)",
        },
        Prop {
            object: "colors",
            property: "value",
            kind: PropKind::Color,
            display: "Value",
        },
        Prop {
            object: "colors",
            property: "target",
            kind: PropKind::Color,
            display: "Target",
        },
        Prop {
            object: "labels",
            property: "show",
            kind: PropKind::Bool,
            display: "Show",
        },
        Prop {
            object: "style",
            property: "vertical",
            kind: PropKind::Bool,
            display: "Vertical",
        },
    ],
    mappings,
    layout_ts: include_str!("bvis/bullet.ts"),
    visual_ts: include_str!("bvis/bullet_visual.ts"),
};

/// One bullet per category value (up to 100), each measure role bound in order.
fn mappings() -> Value {
    json!([{
        "conditions": [{"category": {"max": 1}, "value": {"max": 1}, "target": {"max": 1}, "bands": {"max": 4}, "maximum": {"max": 1}}],
        "categorical": {
            "categories": {"for": {"in": "category"}, "dataReductionAlgorithm": {"top": {"count": 100}}},
            "values": {"select": [
                {"bind": {"to": "value"}}, {"bind": {"to": "target"}}, {"bind": {"to": "bands"}}, {"bind": {"to": "maximum"}}
            ]}
        }
    }])
}
