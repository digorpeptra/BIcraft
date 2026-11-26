//! The BIcraft Dumbbell and slope chart (docs/formats/bicraft-visuals.md "Dumbbell and slope
//! chart"): two measures per category, as dumbbells or, with `style.slope`, as a slope chart.

use serde_json::{Value, json};

use super::{Def, Prop, PropKind, Role};

pub(super) const DEF: Def = Def {
    kind: "dumbbell",
    guid: "bicraftDumbbell6F1A8C3E9D2B7F4A0C5E",
    name: "bicraftDumbbell",
    display_name: "BIcraft Dumbbell",
    version: "1.3.4.0",
    description: "Two values per category as dumbbells or a slope chart (BIcraft, docs/formats/bicraft-visuals.md)",
    roles: &[
        Role {
            name: "category",
            kind: "Grouping",
            display: "Category",
            max: 1,
        },
        Role {
            name: "start",
            kind: "Measure",
            display: "Start",
            max: 1,
        },
        Role {
            name: "end",
            kind: "Measure",
            display: "End",
            max: 1,
        },
    ],
    required: &["category", "start", "end"],
    one_column: None,
    props: &[
        Prop {
            object: "style",
            property: "slope",
            kind: PropKind::Bool,
            display: "Slope chart",
        },
        Prop {
            object: "colors",
            property: "start",
            kind: PropKind::Color,
            display: "Start",
        },
        Prop {
            object: "colors",
            property: "end",
            kind: PropKind::Color,
            display: "End",
        },
        Prop {
            object: "colors",
            property: "line",
            kind: PropKind::Color,
            display: "Line",
        },
        Prop {
            object: "labels",
            property: "show",
            kind: PropKind::Bool,
            display: "Show",
        },
    ],
    mappings,
    layout_ts: include_str!("bvis/dumbbell.ts"),
    visual_ts: include_str!("bvis/dumbbell_visual.ts"),
};

/// One row per category value (up to 200), the two measures bound.
fn mappings() -> Value {
    json!([{
        "conditions": [{"category": {"max": 1}, "start": {"max": 1}, "end": {"max": 1}}],
        "categorical": {
            "categories": {"for": {"in": "category"}, "dataReductionAlgorithm": {"top": {"count": 200}}},
            "values": {"select": [{"bind": {"to": "start"}}, {"bind": {"to": "end"}}]}
        }
    }])
}
