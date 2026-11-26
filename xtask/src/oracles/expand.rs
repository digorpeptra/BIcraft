//! `oracle-gen expand`: expanded matrices of the render oracle (set `expand`): every level
//! expanded ("Expand all down one level", Desktop writes the expanded levels without
//! `isCollapsed` and the last one with it), single expanded members (`root.children` with
//! `isToggled`), nested and inverted members, expanded column levels, row subtotals at the
//! bottom, per-level `subTotals` entries (`perRowLevel` + `selector.metadata`) and custom
//! subtotal labels. The `expansionStates` and `subTotals` shapes are the ones Desktop 2.158
//! saved after the same actions in its UI (docs/formats/visual-options.md, "Matrix expansion").
//!
//! ```text
//! cargo xtask oracle-gen expand DIR          -> DIR/cmds1.json, DIR/cmds2.json, DIR/layout.json
//! bicraft run --file DIR/cmds1.json         -> DIR/proj/Synth.pbip
//! cargo xtask oracle-gen expand DIR patch    -> writes expansionStates and objects
//! bicraft run --file DIR/cmds2.json         -> DIR/expand.pbix
//! ```
//! Model culture fr-FR (render with `{"ui_language": "fr-FR"}`).

use serde_json::{Value, json};

use super::{Cmds, Vis, fields, grids, two_step_gen, two_step_patch};

const R: &str = "Sales[Region]";
const CA: &str = "Sales[Category]";
const CH: &str = "Sales[Channel]";

fn lit(v: &str) -> Value {
    json!({"expr": {"Literal": {"Value": v}}})
}

fn text(v: &str) -> Value {
    json!({"Literal": {"Value": format!("'{v}'")}})
}

fn key(col: &str) -> Value {
    json!({"Column": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": col}})
}

/// One level of an expansion state: `Sales.<col>`, collapsed or not, with its identity key
/// when members of it are toggled.
fn level(col: &str, collapsed: bool, keyed: bool) -> Value {
    let mut l = json!({"queryRefs": [format!("Sales.{col}")]});
    if collapsed {
        l["isCollapsed"] = json!(true);
    }
    if keyed {
        l["identityKeys"] = json!([key(col)]);
    }
    l["isPinned"] = json!(true);
    l
}

fn member(v: &str, children: Vec<Value>) -> Value {
    let mut m = json!({"identityValues": [text(v)], "isToggled": true});
    if !children.is_empty() {
        m["children"] = Value::Array(children);
    }
    m
}

fn state(role: &str, levels: Vec<Value>, children: Vec<Value>) -> Value {
    let root = if children.is_empty() {
        json!({})
    } else {
        json!({"children": children})
    };
    json!({"roles": [role], "levels": levels, "root": root})
}

/// `{"$visual": {"expansionStates": states}, "$active": roles, "subTotals": subtotals?}`: the
/// expanded roles' projections are marked `active`, as Desktop marks them when it expands.
fn objs(states: Vec<Value>, subtotals: Option<Value>) -> Value {
    let roles: Vec<Value> = states
        .iter()
        .filter_map(|s| s.get("roles")?.get(0).cloned())
        .collect();
    let mut o = json!({"$visual": {"expansionStates": states}, "$active": roles});
    if let Some(s) = subtotals {
        o["subTotals"] = s;
    }
    o
}

/// The rows expanded one level (Region open, Category collapsed).
fn rows_open() -> Value {
    state(
        "Rows",
        vec![level("Region", false, false), level("Category", true, false)],
        vec![],
    )
}

const G6: [(i64, i64); 6] = [(20, 20), (440, 20), (860, 20), (20, 370), (440, 370), (860, 370)];

fn visuals() -> Vec<Vis> {
    let mut out = Vec::new();
    let mut v = |page: &'static str, name: &'static str, slot: usize, f: &[(&str, &str)], objects: Value| {
        let (x, y) = G6.get(slot).copied().unwrap_or((20, 20));
        out.push(Vis {
            page,
            name,
            ty: "pivotTable",
            x,
            y,
            w: 400,
            h: 330,
            fields: fields(f),
            objects,
        });
    };
    let rc = [("Rows", R), ("Rows", CA), ("Columns", CH), ("Values", "[Total]")];
    let rc2 = [
        ("Rows", R),
        ("Rows", CA),
        ("Values", "[Total]"),
        ("Values", "[Margin %]"),
    ];
    let cols2 = [
        ("Rows", R),
        ("Columns", CH),
        ("Columns", CA),
        ("Values", "[Total]"),
    ];
    let per_row = |sel: &str, prop: &str, value: Value| {
        json!([{"properties": {"perRowLevel": lit("true")}},
               {"properties": {prop: value}, "selector": {"metadata": format!("Sales.{sel}")}}])
    };
    let bottom = || json!({"rowSubtotalsPosition": lit("'Bottom'")});

    v("ExpandA", "x_all", 0, &rc, objs(vec![rows_open()], None));
    v(
        "ExpandA",
        "x_one",
        1,
        &rc2,
        objs(
            vec![state(
                "Rows",
                vec![level("Region", true, true), level("Category", true, false)],
                vec![member("South", vec![])],
            )],
            None,
        ),
    );
    let mut sub = per_row("Category", "levelSubtotalLabel", lit("'Sous-cat'"));
    if let Some(p) = sub.get_mut(0).and_then(|e| e.get_mut("properties")) {
        p["rowSubtotalsPosition"] = lit("'Bottom'");
    }
    v("ExpandA", "x_bottom", 2, &rc, objs(vec![rows_open()], Some(sub)));
    v(
        "ExpandA",
        "x_lvloff",
        3,
        &rc,
        objs(
            vec![rows_open()],
            Some(per_row("Region", "levelSubtotalEnabled", lit("false"))),
        ),
    );
    v(
        "ExpandA",
        "x_catoff",
        4,
        &rc,
        objs(
            vec![rows_open()],
            Some(per_row("Category", "levelSubtotalEnabled", lit("false"))),
        ),
    );
    let mut lbl = bottom();
    lbl["rowSubtotalsLabel"] = lit("'Sous-total'");
    v(
        "ExpandA",
        "x_rowlbl",
        5,
        &rc,
        objs(vec![rows_open()], Some(json!([{"properties": lbl}]))),
    );

    let cols_open = || {
        state(
            "Columns",
            vec![level("Channel", false, false), level("Category", true, false)],
            vec![],
        )
    };
    v("ExpandB", "x_cols", 0, &cols2, objs(vec![cols_open()], None));
    v(
        "ExpandB",
        "x_collbl",
        1,
        &cols2,
        objs(
            vec![cols_open()],
            Some(json!([{"properties": {"columnSubtotalsLabel": lit("'SousCol'")}}])),
        ),
    );
    v(
        "ExpandB",
        "x_two",
        2,
        &[
            ("Rows", R),
            ("Rows", CA),
            ("Columns", CH),
            ("Columns", "Sales[Quarter]"),
            ("Values", "[Total]"),
        ],
        // (a field on both axes makes Desktop fail the query: Quarter, not Category)
        objs(
            vec![
                rows_open(),
                state(
                    "Columns",
                    vec![level("Channel", false, false), level("Quarter", true, false)],
                    vec![],
                ),
            ],
            None,
        ),
    );
    v(
        "ExpandB",
        "x_three",
        3,
        &[("Rows", R), ("Rows", CA), ("Rows", CH), ("Values", "[Total]")],
        objs(
            vec![state(
                "Rows",
                vec![
                    level("Region", true, true),
                    level("Category", true, true),
                    level("Channel", true, false),
                ],
                vec![member("South", vec![member("Bikes", vec![])])],
            )],
            None,
        ),
    );
    v(
        "ExpandB",
        "x_inverse",
        4,
        &rc2,
        objs(
            vec![state(
                "Rows",
                vec![level("Region", false, true), level("Category", true, false)],
                vec![member("East", vec![])],
            )],
            None,
        ),
    );
    v(
        "ExpandB",
        "x_colone",
        5,
        &cols2,
        objs(
            vec![state(
                "Columns",
                vec![level("Channel", true, true), level("Category", true, false)],
                vec![member("Online", vec![])],
            )],
            None,
        ),
    );
    // controls: expansion without `active` flags (Desktop shows the top level), `active`
    // flags without expansion, and a hierarchy drilled to its second level
    v(
        "ExpandC",
        "x_noactive",
        0,
        &rc,
        json!({"$visual": {"expansionStates": [rows_open()]}}),
    );
    v("ExpandC", "x_activeonly", 1, &rc, json!({"$active": ["Rows"]}));
    v("ExpandC", "x_drill", 2, &rc, json!({"$activeAt": {"Rows": 1}}));
    out
}

pub fn run(dir: &str, patch: bool) -> Result<(), String> {
    let vis = visuals();
    if patch {
        return two_step_patch(dir, &vis, false);
    }
    let mut c = Cmds::default();
    c.c("model.new", json!({"name": "Synth", "culture": "fr-FR"}));
    c.c(
        "table.import",
        json!({"name": "Sales", "csv": grids::sales_csv(), "types": {"Amount": "double", "Cost": "double"}}),
    );
    c.c(
        "measure.add",
        json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])",
               "format_string": "#,0"}),
    );
    c.c(
        "measure.add",
        json!({"table": "Sales", "name": "Margin %",
               "expression": "DIVIDE(SUM(Sales[Amount])-SUM(Sales[Cost]),SUM(Sales[Amount]))",
               "format_string": "0.0%"}),
    );
    c.c("report.new", json!({}));
    two_step_gen(dir, c, &vis, "expand.pbix")
}
