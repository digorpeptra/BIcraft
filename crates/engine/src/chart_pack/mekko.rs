//! BIcraft Marimekko (docs/formats/chart-pack.md "Marimekko").

use serde_json::{Value, json};

use super::{Def, Object, Prop, PropKind, Role};

pub(crate) const DEF: Def = Def {
    kind: "mekko",
    guid: "bicraftMarimekko5D7F9B1E3A6C8E0B2D4F6A",
    name: "bicraftMarimekko",
    display_name: "BIcraft Marimekko",
    version: "1.3.5.0",
    description: "A Marimekko chart: 100 % stacked columns whose widths are the categories' shares of the total (BIcraft, docs/formats/chart-pack.md)",
    roles: &[
        Role {
            name: "category",
            kind: "Grouping",
            display: "Category",
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
    ],
    objects: &[
        super::STYLE,
        Object {
            name: "colors",
            display: "Colors",
            props: &[
                series_color("series1", "Series 1", "color1"),
                series_color("series2", "Series 2", "color2"),
                series_color("series3", "Series 3", "color3"),
                series_color("series4", "Series 4", "color4"),
                series_color("series5", "Series 5", "color5"),
                series_color("series6", "Series 6", "color6"),
            ],
        },
        Object {
            name: "labels",
            display: "Data labels",
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
        Object {
            name: "legend",
            display: "Legend",
            props: &[Prop {
                name: "show",
                display: "Show",
                kind: PropKind::Bool,
                key: "showLegend",
            }],
        },
        Object {
            name: "columns",
            display: "Columns",
            props: &[
                Prop {
                    name: "sort",
                    display: "Widest first",
                    kind: PropKind::Bool,
                    key: "sortColumns",
                },
                Prop {
                    name: "widthAxis",
                    display: "Width axis",
                    kind: PropKind::Bool,
                    key: "widthAxis",
                },
            ],
        },
    ],
    visual_ts: include_str!("mekko_visual.ts"),
    layout_ts: ("mekko.ts", include_str!("mekko.ts")),
    categorical,
};

fn categorical() -> Value {
    json!({
        "categories": {
            "select": [{"bind": {"to": "category"}}, {"bind": {"to": "series"}}],
            "dataReductionAlgorithm": {"top": {"count": bicraft_render::chart_pack::mekko::MAX_ROWS}}
        },
        "values": {"select": [{"bind": {"to": "value"}}]}
    })
}

/// A series colour of the format pane: `colors.<name>`, the layout's `<key>` (none set: the
/// look's palette colour of that series).
const fn series_color(name: &'static str, display: &'static str, key: &'static str) -> Prop {
    Prop {
        name,
        display,
        kind: PropKind::Fill,
        key,
    }
}
