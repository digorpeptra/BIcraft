//! The BIcraft Histogram (docs/formats/bicraft-visuals.md "Histogram"): the values of a numeric
//! column, weighted by a frequency measure, counted in bins of equal width.

use serde_json::{Value, json};

use super::{Def, Prop, PropKind, Role};

pub(super) const DEF: Def = Def {
    kind: "histogram",
    guid: "bicraftHistogram4C8E1A7D3F9B2E6A0D5C",
    name: "bicraftHistogram",
    display_name: "BIcraft Histogram",
    version: "1.3.3.0",
    description: "Counts the values of a numeric column in bins (BIcraft, docs/formats/bicraft-visuals.md)",
    roles: &[
        Role {
            name: "values",
            kind: "Grouping",
            display: "Values",
            max: 1,
        },
        Role {
            name: "frequency",
            kind: "Measure",
            display: "Frequency",
            max: 1,
        },
    ],
    required: &["values"],
    one_column: None,
    props: &[
        Prop {
            object: "bins",
            property: "count",
            kind: PropKind::Num {
                min: 1.0,
                max: 200.0,
                int: true,
            },
            display: "Number of bins",
        },
        Prop {
            object: "bins",
            property: "size",
            kind: PropKind::Num {
                min: 0.0,
                max: 1e15,
                int: false,
            },
            display: "Bin size",
        },
        Prop {
            object: "bins",
            property: "cumulative",
            kind: PropKind::Bool,
            display: "Cumulative",
        },
        Prop {
            object: "colors",
            property: "fill",
            kind: PropKind::Color,
            display: "Bars",
        },
        Prop {
            object: "labels",
            property: "show",
            kind: PropKind::Bool,
            display: "Show",
        },
    ],
    mappings,
    layout_ts: include_str!("bvis/histogram.ts"),
    visual_ts: include_str!("bvis/histogram_visual.ts"),
};

/// Each distinct value of the column is a category (up to 30,000), its frequency the measure.
fn mappings() -> Value {
    json!([{
        "conditions": [{"values": {"max": 1}, "frequency": {"max": 1}}],
        "categorical": {
            "categories": {"for": {"in": "values"}, "dataReductionAlgorithm": {"top": {"count": 30000}}},
            "values": {"select": [{"bind": {"to": "frequency"}}]}
        }
    }])
}
