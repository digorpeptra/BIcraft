//! BIcraft Calendar Heatmap (docs/formats/chart-pack.md "Calendar heatmap").

use serde_json::{Value, json};

use super::{Def, Object, Prop, PropKind, Role};

pub(crate) const DEF: Def = Def {
    kind: "calendar",
    guid: "bicraftCalendarHeatmap1E3B5D7F9A2C4E6B8D0F",
    name: "bicraftCalendarHeatmap",
    display_name: "BIcraft Calendar Heatmap",
    version: "1.3.4.0",
    description: "A calendar heatmap: one cell per day, weeks as columns and weekdays as rows, coloured by the day's value (BIcraft, docs/formats/chart-pack.md)",
    roles: &[
        Role {
            name: "date",
            kind: "Grouping",
            display: "Date",
            required: true,
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
                Prop {
                    name: "low",
                    display: "Lowest value",
                    kind: PropKind::Fill,
                    key: "low",
                },
                Prop {
                    name: "high",
                    display: "Highest value",
                    kind: PropKind::Fill,
                    key: "high",
                },
                Prop {
                    name: "empty",
                    display: "Day without data",
                    kind: PropKind::Fill,
                    key: "empty",
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
            name: "range",
            display: "Dates",
            props: &[
                Prop {
                    name: "months",
                    display: "Only the months with data",
                    kind: PropKind::Bool,
                    key: "monthsOnly",
                },
                Prop {
                    name: "sundayFirst",
                    display: "Weeks start on Sunday",
                    kind: PropKind::Bool,
                    key: "sundayFirst",
                },
            ],
        },
    ],
    visual_ts: include_str!("calendar_visual.ts"),
    layout_ts: ("calendar.ts", include_str!("calendar.ts")),
    categorical,
};

fn categorical() -> Value {
    json!({
        "categories": {
            "for": {"in": "date"},
            "dataReductionAlgorithm": {"top": {"count": bicraft_render::chart_pack::calendar::MAX_ROWS}}
        },
        "values": {"select": [{"bind": {"to": "value"}}]}
    })
}
