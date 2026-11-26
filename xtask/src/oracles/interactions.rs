//! `oracle-gen interactions`: the interaction oracle (docs/interaction-oracle.md). What does Power
//! BI Desktop show, visual by visual, for a given interaction state: slicer selections of every
//! mode, slicers filtering each other, cross-filtering and cross-highlighting from a click on a
//! bar, a column, a pie slice, a line point or a table row, Ctrl+click, the page's
//! `visualInteractions` (DataFilter / HighlightFilter / NoFilter), one-way and two-way
//! relationships, an inactive relationship, CALCULATE / ALL / KEEPFILTERS / ALLSELECTED under a
//! selection, slicers synced across pages, the Clear and Apply filters buttons.
//!
//! One scenario = one page of the generated report (so that every page starts from the state
//! its files describe); a scenario has steps, each step a list of UI actions followed by a
//! reading of every visual of the page. A step without actions is a *file state* (the selection
//! comes from the PBIR: `objects.general.filter`, `visualInteractions`, `syncGroup`).
//!
//! ```text
//! cargo xtask oracle-gen interactions DIR          -> DIR/model.bim, cmds1.json, cmds2.json, plan.json
//! bicraft run --file DIR/cmds1.json               -> DIR/proj/Interactions.pbip
//! cargo xtask oracle-gen interactions DIR patch    -> slicer selections, interactions, sync groups
//! bicraft run --file DIR/cmds2.json               -> DIR/interactions.pbix
//! xtask/interaction-oracle/record.ps1 -Dir DIR     -> DIR/raw/ (Desktop: trace, readings, results)
//! cargo xtask oracle-gen interactions DIR collect  -> testsupport/interaction_oracle/corpus.json
//! ```
//! The model is made of DAX calculated tables only (no data source); culture en-US.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Map, Value, json};

use super::{Cmds, join, read_json, write_json};

// ---------------------------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------------------------

/// The products: key, name, category, colour.
pub const PRODUCTS: [(i64, &str, &str, &str); 6] = [
    (1, "Bike", "Wheels", "Red"),
    (2, "Ball", "Toys", "Blue"),
    (3, "Car", "Wheels", "Red"),
    (4, "Kite", "Toys", "Green"),
    (5, "Helmet", "Gear", "Blue"),
    (6, "Gloves", "Gear", "Red"),
];
/// The stores: key, name, region.
pub const STORES: [(i64, &str, &str); 4] = [
    (1, "Paris", "North"),
    (2, "Lyon", "South"),
    (3, "Nice", "South"),
    (4, "Lille", "North"),
];

/// `Sales`: 240 rows. Each product is sold in two stores only (product p in stores p and p + 1,
/// modulo 4) (so that a product selection
/// leaves some stores without sales; Kite: Lille and Paris), dates over 2025-01-01 .. 2026-11-30, ship dates 0 to 19
/// days later (inside the `Date` table).
const SALES: &str = "SELECTCOLUMNS(GENERATESERIES(1, 240), \"SalesKey\", [Value], \
\"ProductKey\", MOD([Value] * 5, 6) + 1, \
\"StoreKey\", MOD(MOD([Value] * 5, 6) + MOD(INT([Value] / 7), 2), 4) + 1, \
\"Date\", DATE(2025, 1, 1) + MOD([Value] * 37, 700), \
\"ShipDate\", DATE(2025, 1, 1) + MOD([Value] * 37, 700) + MOD([Value], 20), \
\"Qty\", MOD([Value], 5) + 1, \
\"Amount\", (MOD([Value] * 13, 41) + 10) * (MOD([Value], 5) + 1))";

const DATES: &str = "ADDCOLUMNS(CALENDAR(DATE(2025, 1, 1), DATE(2026, 12, 31)), \"Year\", YEAR([Date]), \
\"Month\", MONTH([Date]), \"Quarter\", \"Q\" & QUARTER([Date]))";

/// The measures (all on `Sales`).
pub const MEASURES: [(&str, &str); 10] = [
    ("Total Amount", "SUM(Sales[Amount])"),
    ("Units", "SUM(Sales[Qty])"),
    ("Lines", "COUNTROWS(Sales)"),
    ("All Products", "CALCULATE([Total Amount], ALL(Product))"),
    (
        "Red Keep",
        "CALCULATE([Total Amount], KEEPFILTERS(Product[Color] = \"Red\"))",
    ),
    (
        "Red Replace",
        "CALCULATE([Total Amount], Product[Color] = \"Red\")",
    ),
    (
        "Share Selected",
        "DIVIDE([Total Amount], CALCULATE([Total Amount], ALLSELECTED(Product)))",
    ),
    (
        "Shipped Amount",
        "CALCULATE([Total Amount], USERELATIONSHIP(Sales[ShipDate], 'Date'[Date]))",
    ),
    ("Store Count", "COUNTROWS(Store)"),
    ("Product Count", "COUNTROWS(Product)"),
];

fn ccol(name: &str, ty: &str) -> Value {
    json!({"type": "calculatedTableColumn", "name": name, "dataType": ty, "isNameInferred": true,
           "isDataTypeInferred": true, "sourceColumn": format!("[{name}]")})
}

fn ctable(name: &str, cols: &[(&str, &str)], expr: &str) -> Value {
    let columns: Vec<Value> = cols.iter().map(|(n, t)| ccol(n, t)).collect();
    json!({"name": name, "columns": columns,
           "partitions": [{"name": name, "source": {"type": "calculated", "expression": expr}}]})
}

fn rows_literal<T>(rows: &[T], f: impl Fn(&T) -> String) -> String {
    let r: Vec<String> = rows.iter().map(|x| format!("{{{}}}", f(x))).collect();
    format!("{{{}}}", r.join(", "))
}

/// The TMSL model (calculated tables; Desktop computes them itself on open).
pub fn model() -> Value {
    let product = ctable(
        "Product",
        &[
            ("ProductKey", "int64"),
            ("Product", "string"),
            ("Category", "string"),
            ("Color", "string"),
        ],
        &format!(
            "DATATABLE(\"ProductKey\", INTEGER, \"Product\", STRING, \"Category\", STRING, \"Color\", STRING, {})",
            rows_literal(&PRODUCTS, |(k, n, c, o)| format!(
                "{k}, \"{n}\", \"{c}\", \"{o}\""
            ))
        ),
    );
    let store = ctable(
        "Store",
        &[("StoreKey", "int64"), ("Store", "string"), ("Region", "string")],
        &format!(
            "DATATABLE(\"StoreKey\", INTEGER, \"Store\", STRING, \"Region\", STRING, {})",
            rows_literal(&STORES, |(k, n, r)| format!("{k}, \"{n}\", \"{r}\""))
        ),
    );
    let mut date = ctable(
        "Date",
        &[
            ("Date", "dateTime"),
            ("Year", "int64"),
            ("Month", "int64"),
            ("Quarter", "string"),
        ],
        DATES,
    );
    date["dataCategory"] = json!("Time");
    if let Some(c) = date["columns"].get_mut(0) {
        c["isKey"] = json!(true);
        c["formatString"] = json!("yyyy-MM-dd");
    }
    let mut sales = ctable(
        "Sales",
        &[
            ("SalesKey", "int64"),
            ("ProductKey", "int64"),
            ("StoreKey", "int64"),
            ("Date", "dateTime"),
            ("ShipDate", "dateTime"),
            ("Qty", "int64"),
            ("Amount", "int64"),
        ],
        SALES,
    );
    let measures: Vec<Value> = MEASURES
        .iter()
        .map(|(n, e)| json!({"name": n, "expression": e}))
        .collect();
    sales["measures"] = Value::Array(measures);
    let ids: Vec<String> = scenarios().iter().map(|s| format!("{{\"{}\"}}", s.id)).collect();
    let tag = ctable(
        "Tag",
        &[("Page", "string")],
        &format!("DATATABLE(\"Page\", STRING, {{{}}})", ids.join(", ")),
    );
    json!({
        "name": "Interactions",
        "compatibilityLevel": 1567,
        "model": {
            "culture": "en-US",
            "tables": [product, store, date, sales, tag],
            "relationships": [
                {"name": "Sales_Product", "fromTable": "Sales", "fromColumn": "ProductKey",
                 "toTable": "Product", "toColumn": "ProductKey"},
                {"name": "Sales_Store", "fromTable": "Sales", "fromColumn": "StoreKey",
                 "toTable": "Store", "toColumn": "StoreKey",
                 "crossFilteringBehavior": "bothDirections"},
                {"name": "Sales_Date", "fromTable": "Sales", "fromColumn": "Date",
                 "toTable": "Date", "toColumn": "Date"},
                {"name": "Sales_ShipDate", "fromTable": "Sales", "fromColumn": "ShipDate",
                 "toTable": "Date", "toColumn": "Date", "isActive": false}
            ],
            "annotations": [{"name": "__PBI_TimeIntelligenceEnabled", "value": "0"}]
        }
    })
}

// ---------------------------------------------------------------------------------------------
// Semantic-query helpers (the filters Desktop writes)
// ---------------------------------------------------------------------------------------------

/// Source alias of a table in a filter's `From`.
fn alias(entity: &str) -> &'static str {
    match entity {
        "Product" => "p",
        "Store" => "s",
        "Date" => "d",
        _ => "f",
    }
}

fn qcol(entity: &str, prop: &str) -> Value {
    json!({"Column": {"Expression": {"SourceRef": {"Source": alias(entity)}}, "Property": prop}})
}

fn lit(v: &str) -> Value {
    json!({"Literal": {"Value": v}})
}

fn e(v: &str) -> Value {
    json!({"expr": lit(v)})
}

/// A slicer's `general.filter` on `entity` with `cond`.
fn filter(entity: &str, cond: Value) -> Value {
    json!({"Version": 2, "From": [{"Name": alias(entity), "Entity": entity, "Type": 0}],
           "Where": [{"Condition": cond}]})
}

fn in_text(entity: &str, prop: &str, vals: &[&str]) -> Value {
    let vals: Vec<Value> = vals.iter().map(|v| json!([lit(&format!("'{v}'"))])).collect();
    json!({"In": {"Expressions": [qcol(entity, prop)], "Values": vals}})
}

fn not(c: Value) -> Value {
    json!({"Not": {"Expression": c}})
}

fn cmp(entity: &str, prop: &str, kind: i64, v: &str) -> Value {
    json!({"Comparison": {"ComparisonKind": kind, "Left": qcol(entity, prop), "Right": lit(v)}})
}

fn and(l: Value, r: Value) -> Value {
    json!({"And": {"Left": l, "Right": r}})
}

/// `objects.general.filter` = `f` (plus the other `general` properties of `more`).
fn sel(f: Value, more: Value) -> Value {
    let mut p = Map::new();
    p.insert("filter".into(), json!({"filter": f}));
    if let Value::Object(m) = more {
        p.extend(m);
    }
    json!({"general": [{"properties": p}]})
}

fn mode(m: &str) -> Value {
    json!({"data": [{"properties": {"mode": e(&format!("'{m}'"))}}]})
}

fn merge(a: Value, b: Value) -> Value {
    let mut m = match a {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    if let Value::Object(b) = b {
        for (k, v) in b {
            m.insert(k, v);
        }
    }
    Value::Object(m)
}

// ---------------------------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------------------------

/// One visual of a scenario page.
#[derive(Clone)]
pub struct V {
    /// Short name, unique on its page (the PBIR name is `<page id>_<key>`).
    pub key: &'static str,
    pub ty: &'static str,
    pub rect: (i64, i64, i64, i64),
    /// `(role, field)` pairs (`report.apply` field syntax).
    pub fields: Vec<(&'static str, &'static str)>,
    /// Visual `objects` written into the PBIR; `$visual` keys go to the `visual` object
    /// (`syncGroup`), `$container` keys to the container (`isHidden`), `$vco` to
    /// `visualContainerObjects`.
    pub objects: Value,
}

/// One UI action of a step.
#[derive(Clone)]
pub enum Act {
    /// Click the data point (bar, column, slice, line point, table row) labelled `point`.
    Click {
        visual: &'static str,
        point: &'static str,
        ctrl: bool,
    },
    /// Click a slicer item.
    Item {
        visual: &'static str,
        item: &'static str,
        ctrl: bool,
    },
    /// Ctrl+click a button (Desktop's editing view activates buttons with Ctrl).
    Button { visual: &'static str },
    /// Go to another scenario page (by id), then come back is another action.
    Page { page: &'static str },
}

pub struct Step {
    pub id: &'static str,
    pub acts: Vec<Act>,
    /// What the step is meant to establish (docs/interaction-oracle.md).
    pub note: &'static str,
}

pub struct Scenario {
    /// Page id and display name (`I01` …).
    pub id: &'static str,
    pub title: &'static str,
    pub visuals: Vec<V>,
    /// `(source key, target key, type)`: the page's `visualInteractions`.
    pub interactions: Vec<(&'static str, &'static str, &'static str)>,
    pub steps: Vec<Step>,
    /// Hidden page (only reached through `Act::Page`).
    pub hidden: bool,
}

fn labels_on() -> Value {
    json!({"labels": [{"properties": {"show": e("true")}}]})
}

/// The probe visuals every scenario page carries (right of the slicer column).
fn probes() -> Vec<V> {
    vec![
        V {
            key: "amt",
            ty: "card",
            rect: (270, 10, 190, 100),
            fields: vec![("Values", "[Total Amount]")],
            objects: json!({}),
        },
        V {
            key: "stores",
            ty: "card",
            rect: (470, 10, 190, 100),
            fields: vec![("Values", "[Store Count]")],
            objects: json!({}),
        },
        V {
            key: "prods",
            ty: "card",
            rect: (670, 10, 190, 100),
            fields: vec![("Values", "[Product Count]")],
            objects: json!({}),
        },
        V {
            key: "bar",
            ty: "clusteredBarChart",
            rect: (270, 120, 320, 250),
            fields: vec![("Category", "Product[Category]"), ("Y", "[Total Amount]")],
            objects: labels_on(),
        },
        V {
            key: "col",
            ty: "clusteredColumnChart",
            rect: (600, 120, 330, 250),
            fields: vec![("Category", "Store[Store]"), ("Y", "[Total Amount]")],
            objects: labels_on(),
        },
        V {
            key: "pie",
            ty: "pieChart",
            rect: (940, 120, 330, 250),
            fields: vec![("Category", "Product[Color]"), ("Y", "[Total Amount]")],
            objects: labels_on(),
        },
        V {
            key: "tbl",
            ty: "tableEx",
            rect: (270, 380, 450, 330),
            fields: vec![
                ("Values", "Product[Product]"),
                ("Values", "[Total Amount]"),
                ("Values", "[All Products]"),
                ("Values", "[Red Keep]"),
                ("Values", "[Red Replace]"),
            ],
            objects: json!({}),
        },
        V {
            key: "tbl2",
            ty: "tableEx",
            rect: (730, 380, 280, 330),
            fields: vec![
                ("Values", "Product[Product]"),
                ("Values", "[Share Selected]"),
                ("Values", "[Shipped Amount]"),
            ],
            objects: json!({}),
        },
        V {
            key: "line",
            ty: "lineChart",
            rect: (1020, 380, 250, 330),
            fields: vec![("Category", "Date[Month]"), ("Y", "[Total Amount]")],
            objects: merge(
                labels_on(),
                json!({"lineStyles": [{"properties": {"showMarker": e("true")}}]}),
            ),
        },
    ]
}

/// A slicer in the left column, slot 0..2.
fn slicer(key: &'static str, ty: &'static str, slot: i64, field: &'static str, objects: Value) -> V {
    V {
        key,
        ty,
        rect: (10, 10 + 235 * slot, 250, 225),
        fields: vec![("Values", field)],
        objects,
    }
}

fn with_probes(mut v: Vec<V>) -> Vec<V> {
    v.extend(probes());
    v
}

fn file_step(note: &'static str) -> Vec<Step> {
    vec![Step {
        id: "file",
        acts: vec![],
        note,
    }]
}

fn click(visual: &'static str, point: &'static str) -> Act {
    Act::Click {
        visual,
        point,
        ctrl: false,
    }
}

fn ctrl_click(visual: &'static str, point: &'static str) -> Act {
    Act::Click {
        visual,
        point,
        ctrl: true,
    }
}

fn item(visual: &'static str, it: &'static str, ctrl: bool) -> Act {
    Act::Item {
        visual,
        item: it,
        ctrl,
    }
}

fn step(id: &'static str, acts: Vec<Act>, note: &'static str) -> Step {
    Step { id, acts, note }
}

fn sc(id: &'static str, title: &'static str, visuals: Vec<V>, steps: Vec<Step>) -> Scenario {
    Scenario {
        id,
        title,
        visuals,
        interactions: vec![],
        steps,
        hidden: false,
    }
}

const SYNC: &str = "syncCategory";

fn sync_group() -> Value {
    json!({"$visual": {"syncGroup": {"groupName": SYNC, "fieldChanges": true, "filterChanges": true}}})
}

/// Every scenario, in page order.
pub fn scenarios() -> Vec<Scenario> {
    let cat = "Product[Category]";
    let toys = || {
        sel(
            filter("Product", in_text("Product", "Category", &["Toys"])),
            json!({}),
        )
    };
    let mut out = vec![
        sc(
            "I01",
            "baseline",
            probes(),
            file_step("no selection: the reference values of every probe"),
        ),
        sc(
            "I02",
            "list one",
            with_probes(vec![slicer("sl", "slicer", 0, cat, toys())]),
            file_step("list slicer, one item from the file"),
        ),
        sc(
            "I03",
            "list two",
            with_probes(vec![slicer(
                "sl",
                "slicer",
                0,
                cat,
                sel(
                    filter("Product", in_text("Product", "Category", &["Toys", "Gear"])),
                    json!({}),
                ),
            )]),
            file_step("list slicer, two items (OR within one slicer)"),
        ),
        sc(
            "I04",
            "dropdown",
            with_probes(vec![slicer(
                "sl",
                "slicer",
                0,
                "Product[Color]",
                merge(
                    sel(
                        filter("Product", in_text("Product", "Color", &["Red"])),
                        json!({}),
                    ),
                    mode("Dropdown"),
                ),
            )]),
            file_step("dropdown slicer"),
        ),
        sc(
            "I05",
            "tiles",
            with_probes(vec![
                slicer(
                    "sl",
                    "slicer",
                    0,
                    "Store[Region]",
                    sel(
                        filter("Store", in_text("Store", "Region", &["North"])),
                        json!({"orientation": e("1D")}),
                    ),
                ),
                slicer(
                    "btn",
                    "advancedSlicerVisual",
                    1,
                    cat,
                    sel(
                        filter("Product", in_text("Product", "Category", &["Wheels"])),
                        json!({}),
                    ),
                ),
            ]),
            file_step("horizontal (tile) slicer on a two-way dimension, and a button slicer (new)"),
        ),
        sc(
            "I06",
            "between",
            with_probes(vec![slicer(
                "sl",
                "slicer",
                0,
                "Sales[Qty]",
                merge(
                    sel(
                        filter(
                            "Sales",
                            and(cmp("Sales", "Qty", 2, "2L"), cmp("Sales", "Qty", 4, "4L")),
                        ),
                        json!({}),
                    ),
                    json!({"data": [{"properties": {"mode": e("'Between'"),
                        "numericStart": e("2L"), "numericEnd": e("4L")}}]}),
                ),
            )]),
            file_step("range slicer on a fact column (2 <= Qty <= 4)"),
        ),
        sc(
            "I07",
            "relative data",
            with_probes(vec![slicer(
                "sl",
                "slicer",
                0,
                "Date[Date]",
                json!({"data": [{"properties": {"mode": e("'Relative'"),
                    "relativeRange": e("'Last'"), "relativeDuration": e("6D"),
                    "relativePeriod": e("'Months'")}}]}),
            )]),
            file_step(
                "relative date slicer written as its settings only (no general.filter): does Desktop filter?",
            ),
        ),
        sc(
            "I08",
            "relative filter",
            with_probes(vec![slicer(
                "sl",
                "slicer",
                0,
                "Date[Date]",
                merge(
                    sel(filter("Date", relative_last_months("Date", "Date", 6)), json!({})),
                    json!({"data": [{"properties": {"mode": e("'Relative'"),
                        "relativeRange": e("'Last'"), "relativeDuration": e("6D"),
                        "relativePeriod": e("'Months'")}}]}),
                ),
            )]),
            file_step("relative date slicer with its filter (last 6 months, today included)"),
        ),
        sc(
            "I09",
            "text",
            with_probes(vec![slicer(
                "sl",
                "textSlicer",
                0,
                "Product[Product]",
                sel(
                    filter(
                        "Product",
                        json!({"Contains": {"Left": qcol("Product", "Product"), "Right": lit("'a'")}}),
                    ),
                    json!({}),
                ),
            )]),
            file_step("text slicer: Product contains 'a' (Ball, Car)"),
        ),
        sc(
            "I10",
            "same table",
            with_probes(vec![
                slicer("cat", "slicer", 0, cat, toys()),
                slicer("prod", "slicer", 1, "Product[Product]", json!({})),
            ]),
            file_step("two slicers of one table: the second lists only the items the first leaves"),
        ),
        sc(
            "I11",
            "two way",
            with_probes(vec![
                slicer(
                    "prod",
                    "slicer",
                    0,
                    "Product[Product]",
                    sel(
                        filter("Product", in_text("Product", "Product", &["Kite"])),
                        json!({}),
                    ),
                ),
                slicer("store", "slicer", 1, "Store[Store]", json!({})),
                slicer("month", "slicer", 2, "Date[Month]", json!({})),
            ]),
            file_step(
                "product slicer: a slicer on a two-way dimension (Store) and one on a one-way dimension (Date)",
            ),
        ),
        sc(
            "I12",
            "one way",
            with_probes(vec![
                slicer(
                    "store",
                    "slicer",
                    0,
                    "Store[Store]",
                    sel(filter("Store", in_text("Store", "Store", &["Paris"])), json!({})),
                ),
                slicer("prod", "slicer", 1, "Product[Product]", json!({})),
            ]),
            file_step("store slicer (two-way relationship): the product slicer and the Product Count card"),
        ),
        Scenario {
            interactions: vec![
                ("sl", "amt", "NoFilter"),
                ("sl", "bar", "NoFilter"),
                ("sl", "pie", "DataFilter"),
            ],
            ..sc(
                "I13",
                "slicer nofilter",
                with_probes(vec![slicer("sl", "slicer", 0, cat, toys())]),
                file_step("visualInteractions NoFilter from a slicer to two visuals"),
            )
        },
        sc(
            "I14",
            "inverted",
            with_probes(vec![slicer(
                "sl",
                "slicer",
                0,
                cat,
                merge(
                    sel(
                        filter("Product", not(in_text("Product", "Category", &["Toys"]))),
                        json!({}),
                    ),
                    json!({"selection": [{"properties": {"selectAllCheckboxEnabled": e("true")}}]}),
                ),
            )]),
            file_step("inverted selection: select all, then Toys unticked (Not In)"),
        ),
        sc(
            "I15",
            "months inactive",
            with_probes(vec![slicer(
                "sl",
                "slicer",
                0,
                "Date[Month]",
                merge(
                    sel(
                        filter(
                            "Date",
                            and(cmp("Date", "Month", 2, "3L"), cmp("Date", "Month", 4, "6L")),
                        ),
                        json!({}),
                    ),
                    json!({"data": [{"properties": {"mode": e("'Between'"),
                        "numericStart": e("3L"), "numericEnd": e("6L")}}]}),
                ),
            )]),
            file_step("month range on the date table: Shipped Amount follows the inactive relationship"),
        ),
        // ---- sync groups: the two copies are read from their files before the source page
        // changes the group's selection ----
        sc(
            "I16",
            "sync hidden",
            with_probes(vec![slicer(
                "sl",
                "slicer",
                0,
                cat,
                merge(
                    merge(toys(), sync_group()),
                    json!({"$container": {"isHidden": true}}),
                ),
            )]),
            file_step("hidden synced copy holding Toys in the file"),
        ),
        sc(
            "I17",
            "sync nofilter",
            with_probes(vec![slicer("sl", "slicer", 0, cat, sync_group())]),
            file_step("synced copy whose file holds no filter while the other copies hold Toys"),
        ),
        sc(
            "I18",
            "sync source",
            with_probes(vec![slicer("sl", "slicer", 0, cat, merge(toys(), sync_group()))]),
            vec![
                step("file", vec![], "synced slicer with Toys from the file"),
                step(
                    "gear",
                    vec![item("sl", "Gear", false)],
                    "click Gear on the synced slicer",
                ),
                step(
                    "on_I16",
                    vec![Act::Page { page: "I16" }],
                    "page I16 after Gear was clicked on I18 (hidden synced copy)",
                ),
                step(
                    "on_I17",
                    vec![Act::Page { page: "I17" }],
                    "page I17 after Gear was clicked on I18 (copy without filter in its file)",
                ),
            ],
        ),
        // ---- clicks ----
        sc(
            "I19",
            "click bar",
            probes(),
            vec![
                step(
                    "toys",
                    vec![click("bar", "Toys")],
                    "click a bar: default interactions (charts highlight, card and table filter)",
                ),
                step(
                    "gear_ctrl",
                    vec![ctrl_click("bar", "Gear")],
                    "Ctrl+click a second bar: both selected",
                ),
                step(
                    "gear_plain",
                    vec![click("bar", "Gear")],
                    "plain click on one of two selected bars",
                ),
                step(
                    "gear_again",
                    vec![click("bar", "Gear")],
                    "click the only selected bar again",
                ),
            ],
        ),
        sc(
            "I20",
            "click column",
            probes(),
            vec![step(
                "paris",
                vec![click("col", "Paris")],
                "click a column of the two-way dimension (Store)",
            )],
        ),
        sc(
            "I21",
            "click pie",
            probes(),
            vec![
                step("red", vec![click("pie", "Red")], "click a pie slice"),
                step(
                    "blue_ctrl",
                    vec![ctrl_click("pie", "Blue")],
                    "Ctrl+click a second slice",
                ),
            ],
        ),
        sc(
            "I22",
            "click row",
            probes(),
            vec![step(
                "ball",
                vec![click("tbl", "Ball")],
                "click a table row: tables cross-filter",
            )],
        ),
        sc(
            "I23",
            "click point",
            probes(),
            vec![step(
                "m3",
                vec![click("line", "3")],
                "click a line point (Month 3)",
            )],
        ),
        Scenario {
            interactions: vec![
                ("bar", "col", "NoFilter"),
                ("bar", "pie", "DataFilter"),
                ("bar", "tbl", "HighlightFilter"),
                ("bar", "line", "HighlightFilter"),
            ],
            ..sc(
                "I24",
                "interactions",
                probes(),
                vec![step(
                    "toys",
                    vec![click("bar", "Toys")],
                    "click a bar with visualInteractions NoFilter / DataFilter / HighlightFilter",
                )],
            )
        },
        sc(
            "I25",
            "slicer and click",
            with_probes(vec![slicer(
                "sl",
                "slicer",
                0,
                "Store[Store]",
                sel(
                    filter("Store", in_text("Store", "Store", &["Paris", "Lyon"])),
                    json!({}),
                ),
            )]),
            vec![step(
                "toys",
                vec![click("bar", "Toys")],
                "a click on top of a slicer selection",
            )],
        ),
        sc(
            "I26",
            "slicer clicks",
            with_probes(vec![slicer("sl", "slicer", 0, cat, json!({}))]),
            vec![
                step(
                    "toys",
                    vec![item("sl", "Toys", false)],
                    "click a list slicer item",
                ),
                step(
                    "gear_ctrl",
                    vec![item("sl", "Gear", true)],
                    "Ctrl+click a second item",
                ),
                step(
                    "wheels_plain",
                    vec![item("sl", "Wheels", false)],
                    "plain click on a third item (multi-select by default?)",
                ),
            ],
        ),
        sc(
            "I27",
            "clear",
            with_probes(vec![
                slicer("cat", "slicer", 0, cat, toys()),
                slicer(
                    "color",
                    "slicer",
                    1,
                    "Product[Color]",
                    sel(
                        filter("Product", in_text("Product", "Color", &["Red", "Blue"])),
                        json!({}),
                    ),
                ),
                button("clear", 2, "ClearAllSlicers"),
            ]),
            vec![
                step("file", vec![], "two slicers selected in the file"),
                step(
                    "cleared",
                    vec![click("bar", "Toys"), Act::Button { visual: "clear" }],
                    "a bar clicked, then the Clear all slicers button",
                ),
            ],
        ),
        sc(
            "I28",
            "apply",
            with_probes(vec![
                slicer("cat", "slicer", 0, cat, json!({})),
                slicer("color", "slicer", 1, "Product[Color]", json!({})),
                button("apply", 2, "ApplyAllSlicers"),
            ]),
            vec![
                step(
                    "pending",
                    vec![item("cat", "Toys", false), item("color", "Red", false)],
                    "two slicer items clicked on a page with an Apply all slicers button",
                ),
                step(
                    "applied",
                    vec![Act::Button { visual: "apply" }],
                    "the Apply all slicers button",
                ),
            ],
        ),
    ];
    out.insert(0, sc("I00", "start", vec![], vec![]));
    out
}

/// A button in the left column, slot `slot` (top of the slot).
fn button(key: &'static str, slot: i64, action: &str) -> V {
    V {
        key,
        ty: "actionButton",
        rect: (10, 10 + 235 * slot, 200, 50),
        fields: vec![],
        objects: json!({
            "text": [{"properties": {"show": e("true")}},
                     {"properties": {"show": e("true"), "text": e(&format!("'{action}'"))},
                      "selector": {"id": "default"}}],
            "$vco": {"visualLink": [{"properties": {"show": e("true"),
                                                    "type": e(&format!("'{action}'"))}}]}
        }),
    }
}

/// `Between(col, DateSpan(DateAdd(DateAdd(Now, 1, day), -n, month), day), DateSpan(Now, day))`:
/// the relative-date filter of "last n months" (today included).
fn relative_last_months(entity: &str, prop: &str, n: i64) -> Value {
    let now = json!({"Now": {}});
    let lower = json!({"DateSpan": {"Expression": {"DateAdd": {"Expression": {"DateAdd": {
        "Expression": now, "Amount": 1, "TimeUnit": 0}}, "Amount": -n, "TimeUnit": 2}},
        "TimeUnit": 0}});
    let upper = json!({"DateSpan": {"Expression": {"Now": {}}, "TimeUnit": 0}});
    json!({"Between": {"Expression": qcol(entity, prop), "LowerBound": lower, "UpperBound": upper}})
}

/// The page filter `Tag[Page] = <page id>` (a disconnected table: it changes no value, only the
/// query text, which keeps Desktop from answering a page's queries from another page's cache).
fn tag_filter(page: &str) -> Value {
    json!({
        "name": format!("tag{}", page.to_ascii_lowercase()),
        "field": {"Column": {"Expression": {"SourceRef": {"Entity": "Tag"}}, "Property": "Page"}},
        "type": "Categorical",
        "filter": {"Version": 2, "From": [{"Name": "t", "Entity": "Tag", "Type": 0}],
                   "Where": [{"Condition": {"In": {
                       "Expressions": [{"Column": {"Expression": {"SourceRef": {"Source": "t"}},
                                                   "Property": "Page"}}],
                       "Values": [[lit(&format!("'{page}'"))]]}}}]},
        "howCreated": "User"
    })
}

/// The PBIR name of a scenario visual.
pub fn visual_name(page: &str, key: &str) -> String {
    format!("{}_{key}", page.to_ascii_lowercase())
}

// ---------------------------------------------------------------------------------------------
// Generation
// ---------------------------------------------------------------------------------------------

fn fields_json(v: &V) -> Value {
    Value::Array(
        v.fields
            .iter()
            .map(|(r, f)| json!({"role": r, "field": f}))
            .collect(),
    )
}

fn apply_spec(scs: &[Scenario]) -> Value {
    let pages: Vec<Value> = scs
        .iter()
        .map(|s| {
            let visuals: Vec<Value> = s
                .visuals
                .iter()
                .map(|v| {
                    let (x, y, w, h) = v.rect;
                    json!({"name": visual_name(s.id, v.key), "type": v.ty, "x": x, "y": y,
                           "width": w, "height": h, "fields": fields_json(v)})
                })
                .collect();
            json!({"key": s.id, "display_name": s.id, "width": 1280, "height": 720,
                   "visuals": visuals})
        })
        .collect();
    json!({"pages": pages})
}

fn act_json(page: &str, a: &Act) -> Value {
    match a {
        Act::Click { visual, point, ctrl } => {
            json!({"do": "click", "visual": visual_name(page, visual), "point": point, "ctrl": ctrl})
        }
        Act::Item { visual, item, ctrl } => {
            json!({"do": "item", "visual": visual_name(page, visual), "item": item, "ctrl": ctrl})
        }
        Act::Button { visual } => json!({"do": "button", "visual": visual_name(page, visual)}),
        Act::Page { page: p } => json!({"do": "page", "page": p}),
    }
}

/// The scenario description shared by the driver (plan.json) and the corpus.
pub fn plan() -> Value {
    let scs = scenarios();
    let mut pages = Vec::new();
    for s in &scs {
        let visuals: Vec<Value> = s
            .visuals
            .iter()
            .map(|v| {
                let (x, y, w, h) = v.rect;
                let mut o = json!({"name": visual_name(s.id, v.key), "key": v.key, "type": v.ty,
                    "x": x, "y": y, "width": w, "height": h,
                    "fields": v.fields.iter().map(|(r, f)| json!([r, f])).collect::<Vec<_>>()});
                if let Some(g) = v.objects.get("general").and_then(|g| g.get(0))
                    && let Some(f) = g.pointer("/properties/filter/filter")
                {
                    o["selection"] = f.clone();
                }
                if let Some(m) = v.objects.pointer("/data/0/properties/mode/expr/Literal/Value") {
                    o["mode"] = m.clone();
                }
                if v.objects.pointer("/$visual/syncGroup").is_some() {
                    o["sync_group"] = json!(SYNC);
                }
                if v.objects.pointer("/$container/isHidden") == Some(&json!(true)) {
                    o["hidden"] = json!(true);
                }
                if let Some(t) = v
                    .objects
                    .pointer("/$vco/visualLink/0/properties/type/expr/Literal/Value")
                {
                    o["action"] = t.clone();
                }
                o
            })
            .collect();
        let inter: Vec<Value> = s
            .interactions
            .iter()
            .map(|(a, b, t)| {
                json!({"source": visual_name(s.id, a), "target": visual_name(s.id, b), "type": t})
            })
            .collect();
        let steps: Vec<Value> = s
            .steps
            .iter()
            .map(|st| {
                json!({"id": format!("{}.{}", s.id, st.id), "note": st.note,
                       "actions": st.acts.iter().map(|a| act_json(s.id, a)).collect::<Vec<_>>()})
            })
            .collect();
        pages.push(json!({"page": s.id, "title": s.title, "hidden": s.hidden,
                          "visuals": visuals, "interactions": inter, "steps": steps}));
    }
    json!({"version": 1, "pbix": "interactions.pbix", "width": 1280, "height": 720,
           "scenarios": pages})
}

/// `bicraft run` commands: model (from model.bim), report, PBIP project.
pub fn cmds1(dir: &str) -> Value {
    let scs = scenarios();
    let mut c = Cmds::default();
    c.c("file.open", json!({"path": join(dir, &["model.bim"])}));
    c.c("report.new", json!({}));
    c.c("report.apply", json!({"spec": apply_spec(&scs), "prune": "all"}));
    c.c(
        "project.new",
        json!({"dir": join(dir, &["proj"]), "name": "Interactions"}),
    );
    c.value()
}

pub fn cmds2(dir: &str) -> Value {
    json!([
        {"command": "file.open", "params": {"path": join(dir, &["proj", "Interactions.pbip"])}},
        {"command": "file.save_as", "params": {"path": join(dir, &["interactions.pbix"]), "overwrite": true}}
    ])
}

fn generate(dir: &str) -> Result<(), String> {
    let d = Path::new(dir);
    std::fs::create_dir_all(d).map_err(|e| format!("{dir}: {e}"))?;
    write_json(&d.join("model.bim"), &model(), 1)?;
    write_json(&d.join("cmds1.json"), &cmds1(dir), 1)?;
    write_json(&d.join("cmds2.json"), &cmds2(dir), 1)?;
    write_json(&d.join("plan.json"), &plan(), 1)?;
    let scs = scenarios();
    let steps: usize = scs.iter().map(|s| s.steps.len()).sum();
    println!(
        "{} scenario pages, {steps} steps; next: bicraft run --file {}",
        scs.len(),
        join(dir, &["cmds1.json"])
    );
    Ok(())
}

/// Writes the slicer objects, sync groups, hidden flags, button links and the pages'
/// `visualInteractions` into the generated project (`report_dir` = `…/Interactions.Report`).
pub fn patch_report(report_dir: &Path) -> Result<usize, String> {
    let scs = scenarios();
    let mut by_name: BTreeMap<String, &V> = BTreeMap::new();
    for s in &scs {
        for v in &s.visuals {
            by_name.insert(visual_name(s.id, v.key), v);
        }
    }
    let pages_dir = report_dir.join("definition").join("pages");
    let mut patched = 0usize;
    // the start page (no visuals) is the active one: every scenario page is first queried while
    // the trace runs
    let meta = pages_dir.join("pages.json");
    let mut pm = read_json(&meta)?;
    pm["activePageName"] = json!(super::stable_name("I00"));
    write_json(&meta, &pm, 2)?;
    for p in sorted_dirs(&pages_dir)? {
        let page_file = p.join("page.json");
        if page_file.is_file() {
            let mut pj = read_json(&page_file)?;
            let shown = pj
                .get("displayName")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if let Some(s) = scs.iter().find(|s| s.id == shown) {
                let mut changed = true;
                pj["filterConfig"] = json!({"filters": [tag_filter(s.id)]});
                if !s.interactions.is_empty() {
                    let inter: Vec<Value> = s
                        .interactions
                        .iter()
                        .map(|(a, b, t)| {
                            json!({"source": visual_name(s.id, a), "target": visual_name(s.id, b), "type": t})
                        })
                        .collect();
                    pj["visualInteractions"] = Value::Array(inter);
                    changed = true;
                }
                if s.hidden {
                    pj["visibility"] = json!("HiddenInViewMode");
                    changed = true;
                }
                if changed {
                    write_json(&page_file, &pj, 2)?;
                    patched += 1;
                }
            }
        }
        let vdir = p.join("visuals");
        if !vdir.is_dir() {
            continue;
        }
        for vd in sorted_dirs(&vdir)? {
            let f = vd.join("visual.json");
            if !f.is_file() {
                continue;
            }
            let mut j = read_json(&f)?;
            let name = j
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let Some(v) = by_name.get(&name) else {
                continue;
            };
            let Some(objs) = v.objects.as_object().filter(|o| !o.is_empty()) else {
                continue;
            };
            let mut objs = objs.clone();
            if let Some(Value::Object(keys)) = objs.remove("$container")
                && let Some(o) = j.as_object_mut()
            {
                o.extend(keys);
            }
            let Some(visual) = j.get_mut("visual").and_then(Value::as_object_mut) else {
                return Err(format!("{}: no `visual`", f.display()));
            };
            if let Some(Value::Object(keys)) = objs.remove("$visual") {
                visual.extend(keys);
            }
            if let Some(Value::Object(vco)) = objs.remove("$vco") {
                let t = visual
                    .entry("visualContainerObjects")
                    .or_insert_with(|| Value::Object(Map::new()));
                if let Some(t) = t.as_object_mut() {
                    t.extend(vco);
                }
            }
            if !objs.is_empty() {
                let t = visual
                    .entry("objects")
                    .or_insert_with(|| Value::Object(Map::new()));
                if let Some(t) = t.as_object_mut() {
                    t.extend(objs);
                }
            }
            write_json(&f, &j, 2)?;
            patched += 1;
        }
    }
    Ok(patched)
}

fn sorted_dirs(dir: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    let mut v: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    v.sort();
    Ok(v)
}

/// `cargo xtask oracle-gen interactions …`.
pub fn cmd(args: &[&str]) -> Result<(), String> {
    match args {
        [dir] => generate(dir),
        [dir, "patch"] => {
            let r = Path::new(dir).join("proj").join("Interactions.Report");
            let n = patch_report(&r)?;
            println!("patched {n} files");
            Ok(())
        }
        [dir, "collect"] => super::interactions_collect::collect(Path::new(dir), None),
        [dir, "collect", out] => super::interactions_collect::collect(Path::new(dir), Some(Path::new(out))),
        _ => Err("usage: oracle-gen interactions DIR [patch | collect [OUT.json]]".into()),
    }
}
