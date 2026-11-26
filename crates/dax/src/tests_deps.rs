//! Dependency graph and circular-reference detection (M6.5).

use bicraft_model::{
    Column, DataType, Measure, Partition, Relationship, Role, SemanticModel, Table, TablePermission,
};

use crate::Severity;
use crate::deps::{ObjectRef, cycle_diagnostics, dependencies, expression_dependencies, find_cycles};

fn col(name: &str) -> Column {
    Column {
        name: name.into(),
        data_type: DataType::Double,
        ..Default::default()
    }
}

fn calc(name: &str, expression: &str) -> Column {
    Column {
        name: name.into(),
        expression: Some(expression.into()),
        ..Default::default()
    }
}

fn measure(name: &str, expression: &str) -> Measure {
    Measure {
        name: name.into(),
        expression: expression.into(),
        ..Default::default()
    }
}

fn table(name: &str, columns: Vec<Column>, measures: Vec<Measure>) -> Table {
    Table {
        name: name.into(),
        columns,
        measures,
        ..Default::default()
    }
}

fn calc_table(name: &str, expression: &str) -> Table {
    Table {
        name: name.into(),
        partitions: vec![Partition {
            name: name.into(),
            source_type: "calculated".into(),
            expression: Some(expression.into()),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn model(tables: Vec<Table>) -> SemanticModel {
    SemanticModel {
        name: "Deps".into(),
        tables,
        ..Default::default()
    }
}

fn m(table: &str, name: &str) -> ObjectRef {
    ObjectRef::Measure {
        table: table.into(),
        name: name.into(),
    }
}

fn c(table: &str, name: &str) -> ObjectRef {
    ObjectRef::Column {
        table: table.into(),
        name: name.into(),
    }
}

fn t(name: &str) -> ObjectRef {
    ObjectRef::Table { name: name.into() }
}

fn base() -> SemanticModel {
    model(vec![
        table(
            "Sales",
            vec![
                col("Amount"),
                col("Qty"),
                calc("Line", "Sales[Amount] * [Qty] + [Total]"),
            ],
            vec![
                measure("Total", "SUM(Sales[Amount])"),
                measure("Double", "[Total] * 2"),
                measure(
                    "Rows",
                    "VAR Sales = 1 RETURN COUNTROWS(Product) + Sales + 'Sales'",
                ),
                measure("Ratio", "DIVIDE([Total], CALCULATE([Total], ALL(Sales)))"),
            ],
        ),
        table(
            "Product",
            vec![col("Key"), calc("Sold", "SUMX(RELATEDTABLE(Sales), Sales[Qty])")],
            vec![],
        ),
        calc_table("Top", "TOPN(10, Product, [Total])"),
    ])
}

#[test]
fn collects_direct_dependencies() {
    let model = base();
    let g = dependencies(&model);
    assert_eq!(g.dependencies_of(&m("Sales", "Total")), [&c("Sales", "Amount")]);
    assert_eq!(g.dependencies_of(&m("Sales", "Double")), [&m("Sales", "Total")]);
    // Variables shadow tables: `Sales` and `'Sales'` in `Rows` are the variable.
    assert_eq!(g.dependencies_of(&m("Sales", "Rows")), [&t("Product")]);
    assert_eq!(
        g.dependencies_of(&m("Sales", "Ratio")),
        [&t("Sales"), &m("Sales", "Total")]
    );
    // Calculated columns: unqualified names are columns of the home table first.
    assert_eq!(
        g.dependencies_of(&c("Sales", "Line")),
        [&c("Sales", "Amount"), &c("Sales", "Qty"), &m("Sales", "Total")]
    );
    assert_eq!(
        g.dependencies_of(&c("Product", "Sold")),
        [&t("Sales"), &c("Sales", "Qty")]
    );
    // Calculated tables.
    assert_eq!(
        g.dependencies_of(&t("Top")),
        [&m("Sales", "Total"), &t("Product")]
    );
    // Data columns and plain tables depend on nothing.
    assert!(g.dependencies_of(&c("Sales", "Amount")).is_empty());
    assert!(g.dependencies_of(&t("Sales")).is_empty());
    // Reverse edges, case-insensitively.
    // Edges come in model order.
    let users = g.dependents_of(&m("SALES", "total"));
    assert_eq!(users.len(), 4, "{users:?}");
    assert!(find_cycles(&model).is_empty());
    assert!(cycle_diagnostics(&model).is_empty());
}

#[test]
fn row_filters_are_nodes() {
    let mut model = base();
    model.roles.push(Role {
        name: "Reader".into(),
        table_permissions: vec![TablePermission {
            table: "Sales".into(),
            filter_expression: Some("[Amount] > 0 && [Total] > 0".into()),
            ..Default::default()
        }],
        ..Default::default()
    });
    let g = dependencies(&model);
    let rf = ObjectRef::RowFilter {
        role: "Reader".into(),
        table: "Sales".into(),
    };
    assert_eq!(
        g.dependencies_of(&rf),
        [&c("Sales", "Amount"), &m("Sales", "Total")]
    );
    assert_eq!(rf.to_string(), "row filter of role Reader on 'Sales'");
}

#[test]
fn expression_dependencies_resolve_like_the_binder() {
    let model = base();
    let deps = expression_dependencies(
        &model,
        "[Total] + SUM('Sales'[Qty]) + COUNTROWS(Product)",
        None,
        false,
    );
    assert_eq!(deps, [m("Sales", "Total"), c("Sales", "Qty"), t("Product")]);
    // Declared columns and table-variable columns are not model objects.
    let deps = expression_dependencies(
        &model,
        "VAR v = ADDCOLUMNS(Product, \"Total2\", 1) RETURN SUMX(v, [Total2])",
        None,
        false,
    );
    assert_eq!(deps, [t("Product")]);
    assert!(expression_dependencies(&model, "SUM(", None, false).is_empty());
}

#[test]
fn self_reference_is_a_cycle() {
    let model = model(vec![table("T", vec![col("A")], vec![measure("M", "[M] + 1")])]);
    assert_eq!(find_cycles(&model), [vec![m("T", "M")]]);
    let d = cycle_diagnostics(&model);
    assert_eq!(d.len(), 1);
    let (obj, diag) = &d[0];
    assert_eq!(obj, &m("T", "M"));
    assert_eq!(diag.code, "circular-dependency");
    assert_eq!(diag.severity, Severity::Error);
    assert_eq!(diag.message, "circular dependency: [M] -> [M]");
}

#[test]
fn mutual_recursion_is_listed_in_dependency_order() {
    let model = model(vec![table(
        "T",
        vec![col("A")],
        vec![
            measure("A1", "[B1] + 1"),
            measure("B1", "[C1] * 2"),
            measure("C1", "IF([A1] > 0, 1)"),
            measure("Outside", "[A1]"),
        ],
    )]);
    let cycles = find_cycles(&model);
    assert_eq!(cycles, [vec![m("T", "A1"), m("T", "B1"), m("T", "C1")]]);
    let d = cycle_diagnostics(&model);
    let messages: Vec<_> = d
        .iter()
        .map(|(o, d)| (o.to_string(), d.message.clone()))
        .collect();
    assert_eq!(
        messages,
        [
            (
                "[A1]".to_string(),
                "circular dependency: [A1] -> [B1] -> [C1] -> [A1]".to_string()
            ),
            (
                "[B1]".to_string(),
                "circular dependency: [B1] -> [C1] -> [A1] -> [B1]".to_string()
            ),
            (
                "[C1]".to_string(),
                "circular dependency: [C1] -> [A1] -> [B1] -> [C1]".to_string()
            ),
        ]
    );
    // `Outside` depends on the cycle but is not part of it.
    assert!(d.iter().all(|(o, _)| *o != m("T", "Outside")));
}

#[test]
fn calculated_column_cycles() {
    let model = model(vec![table(
        "T",
        vec![
            col("A"),
            calc("X", "[Y] + T[A]"),
            calc("Y", "T[X] * 2"),
            calc("Z", "[X]"),
        ],
        vec![],
    )]);
    assert_eq!(find_cycles(&model), [vec![c("T", "X"), c("T", "Y")]]);

    // Through a measure: column -> measure -> column.
    let model = model_with_measure_cycle();
    assert_eq!(find_cycles(&model), [vec![c("T", "C"), m("T", "M")]]);
    let d = cycle_diagnostics(&model);
    assert_eq!(d[0].1.message, "circular dependency: 'T'[C] -> [M] -> 'T'[C]");

    // A calculated column may count the rows of its own table.
    let model = self::model(vec![table(
        "T",
        vec![col("A"), calc("N", "COUNTROWS(T)")],
        vec![],
    )]);
    assert!(find_cycles(&model).is_empty());
}

fn model_with_measure_cycle() -> SemanticModel {
    model(vec![table(
        "T",
        vec![col("A"), calc("C", "[M]")],
        vec![measure("M", "SUM(T[C])")],
    )])
}

#[test]
fn calculated_table_cycles() {
    let model = model(vec![
        table("T", vec![col("A")], vec![measure("M", "COUNTROWS(CT)")]),
        calc_table("CT", "FILTER(T, [M] > 0)"),
    ]);
    assert_eq!(find_cycles(&model), [vec![m("T", "M"), t("CT")]]);

    // Through a column of the calculated table.
    let mut model = model;
    if let Some(ct) = model.tables.get_mut(1) {
        ct.columns.push(col("V"));
    }
    if let Some(mm) = model.tables.get_mut(0).and_then(|t| t.measures.get_mut(0)) {
        mm.expression = "SUM(CT[V])".into();
    }
    let cycles = find_cycles(&model);
    assert_eq!(cycles.len(), 1);
    assert_eq!(cycles[0].len(), 3, "{cycles:?}");
}

#[test]
fn complex_components_are_reported_once_per_object() {
    // A <-> B and B <-> C: one component, not a simple cycle.
    let model = model(vec![table(
        "T",
        vec![],
        vec![
            measure("A", "[B]"),
            measure("B", "[A] + [C]"),
            measure("C", "[B]"),
        ],
    )]);
    let cycles = find_cycles(&model);
    assert_eq!(cycles.len(), 1);
    assert_eq!(cycles[0].len(), 3);
    let d = cycle_diagnostics(&model);
    assert_eq!(d.len(), 3);
    assert!(
        d[0].1
            .message
            .contains("part of a dependency cycle between [A], [B], [C]"),
        "{}",
        d[0].1.message
    );
}

#[test]
fn huge_cycles_do_not_exhaust_the_stack() {
    let n = 50_000;
    let measures = (0..n)
        .map(|i| measure(&format!("M{i}"), &format!("[M{}] + 1", (i + 1) % n)))
        .collect();
    let model = model(vec![table("T", vec![], measures)]);
    let cycles = find_cycles(&model);
    assert_eq!(cycles.len(), 1);
    assert_eq!(cycles[0].len(), n);
    assert_eq!(cycles[0].first(), Some(&m("T", "M0")));
    assert_eq!(cycles[0].get(1), Some(&m("T", "M1")));
}

#[test]
fn hostile_expressions_have_no_dependencies() {
    let deep = format!("{}[Total]{}", "(".repeat(100_000), ")".repeat(100_000));
    let model = model(vec![table("T", vec![], vec![measure("Total", &deep)])]);
    assert!(dependencies(&model).dependencies_of(&m("T", "Total")).is_empty());
    assert!(find_cycles(&model).is_empty());
}

#[test]
fn context_transition_columns_depend_on_each_other() {
    let two = |a: &str, b: &str| {
        model(vec![table(
            "Sales",
            vec![col("Qty"), calc("A", a), calc("B", b)],
            vec![measure("Total", "SUM(Sales[Qty])")],
        )])
    };
    // Two columns with a context transition: the engine's classic circular dependency.
    let m2 = two("CALCULATE(SUM(Sales[Qty]))", "CALCULATE(SUM(Sales[Qty]))");
    assert_eq!(find_cycles(&m2), [vec![c("Sales", "A"), c("Sales", "B")]]);
    // A measure reference is an implicit CALCULATE.
    let m2 = two("[Total]", "CALCULATE(COUNTROWS(Sales))");
    assert_eq!(find_cycles(&m2).len(), 1);
    // A column that depends on the transitioning one is excluded from its filter.
    assert!(find_cycles(&two("CALCULATE(SUM(Sales[Qty]))", "[A] * 2")).is_empty());
    assert!(find_cycles(&two("Sales[Qty] * 2", "CALCULATE(SUM(Sales[Qty]))")).is_empty());
    // A table on the one side of a relationship (or with a key column) is filtered by its key.
    let mut keyed = two("CALCULATE(SUM(Sales[Qty]))", "CALCULATE(SUM(Sales[Qty]))");
    keyed.tables.push(table("Fact", vec![col("Key")], vec![]));
    keyed.relationships.push(Relationship {
        from_table: "Fact".into(),
        from_column: "Key".into(),
        to_table: "Sales".into(),
        to_column: "Qty".into(),
        ..Default::default()
    });
    assert!(find_cycles(&keyed).is_empty());
    let mut keyed = two("CALCULATE(SUM(Sales[Qty]))", "CALCULATE(SUM(Sales[Qty]))");
    if let Some(q) = keyed.tables.first_mut().and_then(|t| t.columns.first_mut()) {
        q.extra.insert("isKey".into(), true.into());
    }
    assert!(find_cycles(&keyed).is_empty());
}
