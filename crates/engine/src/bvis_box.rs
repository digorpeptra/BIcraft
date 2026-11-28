//! The BIcraft Box plot (docs/formats/bicraft-visuals.md "Box plot"): the distribution of a
//! measure over the items of a column, one box per category.

use serde_json::{Value, json};

use super::{Def, Prop, PropKind, Role};

pub(super) const DEF: Def = Def {
    kind: "box_plot",
    guid: "bicraftBoxPlot2E7B4D9A1F6C3E8B5D0A",
    name: "bicraftBoxPlot",
    display_name: "BIcraft Box Plot",
    version: "1.3.3.0",
    description: "Quartiles, whiskers at 1.5 IQR and outliers of a measure over a column's items (BIcraft, docs/formats/bicraft-visuals.md)",
    roles: &[
        Role {
            name: "category",
            kind: "Grouping",
            display: "Category",
            max: 1,
        },
        Role {
            name: "sample",
            kind: "Grouping",
            display: "Sample",
            max: 1,
        },
        Role {
            name: "value",
            kind: "Measure",
            display: "Value",
            max: 1,
        },
    ],
    required: &["sample", "value"],
    one_column: Some("sample"),
    props: &[
        Prop {
            object: "box",
            property: "mean",
            kind: PropKind::Bool,
            display: "Show the mean",
        },
        Prop {
            object: "box",
            property: "outliers",
            kind: PropKind::Bool,
            display: "Show outliers",
        },
        Prop {
            object: "box",
            property: "horizontal",
            kind: PropKind::Bool,
            display: "Horizontal",
        },
        Prop {
            object: "colors",
            property: "fill",
            kind: PropKind::Color,
            display: "Boxes",
        },
    ],
    mappings,
    layout_ts: include_str!("bvis/box_plot.ts"),
    visual_ts: include_str!("bvis/box_plot_visual.ts"),
};

/// One row per category and sample item (up to 30,000), the measure bound.
fn mappings() -> Value {
    json!([{
        "conditions": [{"category": {"max": 1}, "sample": {"max": 1}, "value": {"max": 1}}],
        "categorical": {
            "categories": {
                "select": [{"bind": {"to": "category"}}, {"bind": {"to": "sample"}}],
                "dataReductionAlgorithm": {"top": {"count": 30000}}
            },
            "values": {"select": [{"bind": {"to": "value"}}]}
        }
    }])
}
