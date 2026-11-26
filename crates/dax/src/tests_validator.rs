//! The whole-model [`Validator`] against per-expression [`validate_as`] (gap G5): same
//! diagnostics everywhere, same result types wherever per-expression inference stays within its
//! nesting budget, and results that do not depend on the order objects are validated in.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use bicraft_model::{
    Column, DataType, Measure, Partition, Relationship, Role, SemanticModel, Table, TablePermission,
};

use crate::deps::{ObjectRef, dependencies};
use crate::index::{ModelIndex, Names};
use crate::types::{ScalarType, ValueType};
use crate::{Context, ExprKind, Validation, Validator, validate, validate_as};

fn col(name: &str, data_type: DataType) -> Column {
    Column {
        name: name.into(),
        data_type,
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

fn row_number(name: &str) -> Column {
    let mut c = col(name, DataType::Int64);
    c.extra.insert("type".into(), "rowNumber".into());
    c
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

/// One DAX object of a model.
struct Obj<'m> {
    text: &'m str,
    home: Option<&'m str>,
    kind: ExprKind,
    id: ObjectRef,
}

fn objects(m: &SemanticModel) -> Vec<Obj<'_>> {
    let mut out = Vec::new();
    for t in &m.tables {
        for x in &t.measures {
            out.push(Obj {
                text: &x.expression,
                home: Some(&t.name),
                kind: ExprKind::Measure,
                id: ObjectRef::Measure {
                    table: t.name.clone(),
                    name: x.name.clone(),
                },
            });
        }
        for c in &t.columns {
            if let Some(e) = c.expression.as_deref() {
                out.push(Obj {
                    text: e,
                    home: Some(&t.name),
                    kind: ExprKind::CalculatedColumn,
                    id: ObjectRef::Column {
                        table: t.name.clone(),
                        name: c.name.clone(),
                    },
                });
            }
        }
        if let Some(e) = crate::deps::calculated_table_expression(t) {
            out.push(Obj {
                text: e,
                home: None,
                kind: ExprKind::CalculatedTable,
                id: ObjectRef::Table { name: t.name.clone() },
            });
        }
    }
    for r in &m.roles {
        for tp in &r.table_permissions {
            if let Some(e) = tp.filter_expression.as_deref() {
                out.push(Obj {
                    text: e,
                    home: Some(&tp.table),
                    kind: ExprKind::RowFilter,
                    id: ObjectRef::RowFilter {
                        role: r.name.clone(),
                        table: tp.table.clone(),
                    },
                });
            }
        }
    }
    out
}

/// Objects that reach at most `limit` objects through the dependency graph (themselves
/// included): per-expression inference of those never runs into its nesting budget.
fn small_closures(m: &SemanticModel, limit: usize) -> HashSet<ObjectRef> {
    let g = dependencies(m);
    let mut out = HashSet::new();
    for (i, node) in g.nodes.iter().enumerate() {
        let mut seen = HashSet::from([i]);
        let mut stack = vec![i];
        while let Some(v) = stack.pop() {
            for &w in g.edges.get(v).map_or(&[][..], Vec::as_slice) {
                if seen.insert(w) {
                    stack.push(w);
                }
            }
            if seen.len() > limit {
                break;
            }
        }
        if seen.len() <= limit {
            out.insert(node.clone());
        }
    }
    out
}

fn per_expression(m: &SemanticModel, o: &Obj<'_>) -> Validation {
    validate_as(
        o.text,
        &Context {
            model: Some(m),
            table: o.home,
        },
        o.kind,
    )
}

/// Checks every object of `m` both ways, with the validator walking the objects forwards and
/// backwards. Returns the validator's result types (identical in both orders).
fn compare(m: &SemanticModel, exact_types: &HashSet<ObjectRef>) -> Vec<Option<ValueType>> {
    let objs = objects(m);
    let old: Vec<Validation> = objs.iter().map(|o| per_expression(m, o)).collect();
    let mut orders: Vec<Vec<Option<ValueType>>> = Vec::new();
    for reverse in [false, true] {
        let mut v = Validator::new(m);
        let mut idx: Vec<usize> = (0..objs.len()).collect();
        if reverse {
            idx.reverse();
        }
        let mut types = vec![None; objs.len()];
        for i in idx {
            let (o, old) = (&objs[i], &old[i]);
            let new = v.validate_as(o.text, o.home, o.kind);
            assert_eq!(old.diagnostics, new.diagnostics, "{:?}: {}", o.id, o.text);
            assert_eq!(old.valid, new.valid, "{:?}: {}", o.id, o.text);
            if exact_types.contains(&o.id) {
                assert_eq!(old.result_type, new.result_type, "{:?}: {}", o.id, o.text);
            }
            types[i] = new.result_type;
        }
        // Bare expressions too (`dax.validate`, `newly_broken`).
        for t in &m.tables {
            for x in &t.measures {
                let text = format!("[{}] + COUNTROWS('{}')", x.name, t.name.replace('\'', "''"));
                let old = validate(
                    &text,
                    &Context {
                        model: Some(m),
                        table: Some(&t.name),
                    },
                );
                let new = v.validate(&text, Some(&t.name));
                assert_eq!(old.diagnostics, new.diagnostics, "{text}");
                assert_eq!(old.valid, new.valid, "{text}");
            }
        }
        orders.push(types);
    }
    assert_eq!(
        orders.first(),
        orders.last(),
        "result types depend on validation order"
    );
    orders.pop().unwrap_or_default()
}

fn all_exact(m: &SemanticModel) -> HashSet<ObjectRef> {
    objects(m).into_iter().map(|o| o.id).collect()
}

const TEMPLATES: &[&str] = &[
    "{m} + 1",
    "DIVIDE({m}, 2)",
    "CALCULATE({m}, Fact[Int] > 1)",
    "IF({m} > 0, {m}, BLANK())",
    "VAR x = {m} RETURN x * 2",
    "SUMX(Fact, Fact[Calc] + {m})",
    "MAXX(VALUES(Fact[Name]), {m})",
    "{m} & \"!\"",
    "COALESCE({m}, 0.5)",
    "FORMAT({m}, \"0\")",
];

fn fact(extra_columns: Vec<Column>, measures: Vec<Measure>) -> Table {
    let mut columns = vec![
        col("Int", DataType::Int64),
        col("Amount", DataType::Double),
        col("Price", DataType::Decimal),
        col("Name", DataType::String),
        col("When", DataType::DateTime),
        calc("Calc", "Fact[Amount] * 2"),
    ];
    columns.extend(extra_columns);
    table("Fact", columns, measures)
}

/// `len` measures, each referencing the previous one through a template; listed backwards when
/// `backwards`. With `cycle`, the first one references the last.
fn chain(len: usize, backwards: bool, cycle: bool) -> SemanticModel {
    let mut ms: Vec<Measure> = (0..len)
        .map(|i| {
            let text = match i {
                0 if cycle => format!("[M{}] + 1", len.saturating_sub(1)),
                0 => "SUM(Fact[Int])".to_string(),
                _ => TEMPLATES[i % TEMPLATES.len()].replace("{m}", &format!("[M{}]", i - 1)),
            };
            measure(&format!("M{i}"), &text)
        })
        .collect();
    if backwards {
        ms.reverse();
    }
    SemanticModel {
        name: "Chain".into(),
        tables: vec![fact(Vec::new(), ms)],
        ..Default::default()
    }
}

#[test]
fn short_chains_match_exactly() {
    for len in [1, 2, 5, 9, 15] {
        for backwards in [false, true] {
            for cycle in [false, true] {
                let m = chain(len, backwards, cycle);
                compare(&m, &all_exact(&m));
            }
        }
    }
}

#[test]
fn long_chains_get_exact_types_in_any_order() {
    // `[M(i-1)] + 1` from `1`: every measure is an integer. Per-expression inference gives up
    // 16 measures down and reports the end of the chain as a variant.
    for backwards in [false, true] {
        let ms: Vec<Measure> = (0..120)
            .map(|i| {
                let text = if i == 0 {
                    "1".to_string()
                } else {
                    format!("[M{}] + 1", i - 1)
                };
                measure(&format!("M{i}"), &text)
            })
            .collect();
        let mut ms = ms;
        if backwards {
            ms.reverse();
        }
        let m = SemanticModel {
            tables: vec![fact(Vec::new(), ms)],
            ..Default::default()
        };
        let exact = small_closures(&m, 12);
        let types = compare(&m, &exact);
        let measures = objects(&m).iter().filter(|o| o.kind == ExprKind::Measure).count();
        assert_eq!(measures, 120);
        assert!(
            types
                .iter()
                .take(measures)
                .all(|t| *t == Some(ValueType::Scalar(ScalarType::Integer))),
            "{types:?}"
        );
        let last = objects(&m)
            .into_iter()
            .find(|o| matches!(&o.id, ObjectRef::Measure { name, .. } if name == "M119"))
            .map(|o| per_expression(&m, &o).result_type);
        assert_eq!(last, Some(Some(ValueType::Scalar(ScalarType::Variant))));
    }
    for backwards in [false, true] {
        for cycle in [false, true] {
            let m = chain(100, backwards, cycle);
            compare(&m, &small_closures(&m, 12));
        }
    }
}

/// Names that collide across tables, case variants, quoting, row-number columns, cycles through
/// calculated columns, calculated tables and row filters.
fn awkward_model() -> SemanticModel {
    let sales = table(
        "Sales",
        vec![
            col("Amount", DataType::Double),
            col("Qty", DataType::Int64),
            col("Key", DataType::Int64),
            row_number("RowNumber-1"),
            calc("Net", "Sales[Amount] - [Discount]"),
            calc("Loop A", "Sales[Loop B] + 1"),
            calc("Loop B", "Sales[Loop A] * 2"),
            calc("Uses Loop", "Sales[Loop A] + [Total]"),
            calc("Ctx", "CALCULATE(SUM(Sales[Qty]))"),
            calc("Rel", "RELATED('Prod''s'[Price])"),
        ],
        vec![
            measure("Total", "SUM(Sales[Amount])"),
            measure("total upper", "[TOTAL] * 2"),
            measure("Discount", "[Total] * 0.1"),
            measure("Self", "[Self] + 1"),
            measure("Ping", "[Pong] + [Total]"),
            measure("Pong", "[Ping] - 1"),
            measure("Over Ping", "[Ping] + [Discount]"),
            measure("Dup", "1"),
            measure("Qualified", "Sales[Total] + 'Prod''s'[Dup] + Prod[Dup]"),
            measure("Wrong table", "'Prod''s'[Total]"),
            measure("Unknown", "[Nope] + Nope[Total] + SUM(Sales[Nope])"),
            measure("Hidden row number", "SUM(Sales[RowNumber-1])"),
            measure("Column ref", "SUM([Amount]) + MAX([Price])"),
            measure("Bare column", "Sales[Amount] + [Qty]"),
            measure("Iter", "SUMX(Sales, Sales[Net] + [Discount] + Sales[Uses Loop])"),
            measure(
                "Filters",
                "CALCULATE([Total], Sales[Qty] > [Discount], 'Prod''s'[Price] > 1)",
            ),
            measure("Table", "VALUES(Sales[Key])"),
            measure("Wide", "FILTER(Sales, [Total] > 0)"),
            measure("Broken", "SUM(Sales[Amount]"),
            measure("Calls broken", "[Broken] + 1"),
            measure(
                "Var shadow",
                "VAR Total = 1 VAR t = Sales RETURN Total + COUNTROWS(t)",
            ),
        ],
    );
    let prods = table(
        "Prod's",
        vec![
            col("Key", DataType::Int64),
            col("Price", DataType::Decimal),
            col("Amount", DataType::String),
            calc("Margin", "'Prod''s'[Price] - [Dup]"),
        ],
        vec![
            measure("Dup", "2.5"),
            measure("DUP", "\"shadowed\""),
            measure("Price Sum", "SUM('Prod''s'[Price]) + [Total]"),
        ],
    );
    let prod = table(
        "PROD",
        vec![col("Amount", DataType::Boolean), row_number("Price")],
        vec![measure("Dup", "TRUE()"), measure("Row", "SUM(Prod[Price])")],
    );
    let dup_table = table(
        "sales",
        vec![col("Shadow", DataType::String)],
        vec![measure("In shadow table", "SUM(sales[Shadow]) + [Total]")],
    );
    let mut m = SemanticModel {
        name: "Awkward".into(),
        tables: vec![
            sales,
            prods,
            prod,
            dup_table,
            calc_table("Calc T", "ADDCOLUMNS(VALUES(Sales[Key]), \"T\", [Total])"),
            calc_table("Calc Loop", "FILTER('Calc T', [Ping] > 0)"),
        ],
        relationships: vec![Relationship {
            name: "r".into(),
            from_table: "Sales".into(),
            from_column: "Key".into(),
            to_table: "Prod's".into(),
            to_column: "Key".into(),
            is_active: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    m.roles.push(Role {
        name: "Reader".into(),
        table_permissions: vec![
            TablePermission {
                table: "Sales".into(),
                filter_expression: Some("Sales[Qty] > [Discount]".into()),
                ..Default::default()
            },
            TablePermission {
                table: "Prod's".into(),
                filter_expression: Some("[Price] > 0 && [Self] > 0".into()),
                ..Default::default()
            },
        ],
        ..Default::default()
    });
    m
}

#[test]
fn awkward_names_and_cycles_match_exactly() {
    let m = awkward_model();
    compare(&m, &all_exact(&m));
}

/// A small deterministic generator (no `rand` dependency).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) as usize) % n.max(1)
    }
}

fn random_model(seed: u64, max_measures: usize) -> SemanticModel {
    let mut r = Lcg(seed);
    let tables = 1 + r.next(4);
    let measures = 1 + r.next(max_measures);
    let names: Vec<String> = (0..measures)
        .map(|i| {
            // Some names differ only by case: the first one wins, like the engine's lookups.
            if i > 0 && r.next(12) == 0 {
                format!("m{}", r.next(i))
            } else {
                format!("M{i}")
            }
        })
        .collect();
    let mut model = SemanticModel {
        name: format!("Random {seed}"),
        ..Default::default()
    };
    for t in 0..tables {
        let tname = if t == 0 {
            "Fact".to_string()
        } else {
            format!("T{t}")
        };
        let mut cols = vec![
            col("Int", DataType::Int64),
            col("Amount", DataType::Double),
            col("Name", DataType::String),
            calc("Calc", &format!("'{tname}'[Amount] * 2")),
        ];
        if r.next(3) == 0 {
            let target = names[r.next(names.len())].clone();
            cols.push(calc("Calc M", &format!("'{tname}'[Int] + [{target}]")));
        }
        if r.next(4) == 0 {
            cols.push(calc("Calc Self", &format!("'{tname}'[Calc Other] + 1")));
            cols.push(calc("Calc Other", &format!("'{tname}'[Calc Self] + [Calc]")));
        }
        model.tables.push(table(&tname, cols, Vec::new()));
    }
    for (i, name) in names.iter().enumerate() {
        let refs = r.next(3);
        let mut text = if r.next(5) == 0 {
            "SUM(Fact[Amount])".to_string()
        } else {
            format!("{}", r.next(100))
        };
        for _ in 0..refs {
            // Mostly earlier measures (chains), sometimes any (cycles).
            let target = if i > 0 && r.next(4) != 0 {
                names[r.next(i)].clone()
            } else {
                names[r.next(names.len())].clone()
            };
            let reference = match r.next(5) {
                0 => format!("[{}]", target.to_ascii_lowercase()),
                1 => format!("'T{}'[{target}]", r.next(tables)),
                _ => format!("[{target}]"),
            };
            let template = TEMPLATES[r.next(TEMPLATES.len())];
            let part = template.replace("{m}", &reference);
            text = format!("{text} + ({part})");
        }
        let t = r.next(tables);
        if let Some(tbl) = model.tables.get_mut(t) {
            tbl.measures.push(measure(name, &text));
        }
    }
    if r.next(3) == 0 {
        let target = &names[r.next(names.len())];
        model.roles.push(Role {
            name: "R".into(),
            table_permissions: vec![TablePermission {
                table: "Fact".into(),
                filter_expression: Some(format!("Fact[Int] > [{target}]")),
                ..Default::default()
            }],
            ..Default::default()
        });
    }
    model
}

#[test]
fn random_models_match() {
    let mut exact_compared = 0usize;
    let seeds: u64 = bicraft_env::var("BICRAFT_RANDOM_MODELS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);
    for seed in 0..seeds {
        // Mostly small models; every tenth has long chains through cycles.
        let size = if seed % 10 == 9 { 200 } else { 60 };
        let m = random_model(seed, size);
        let exact = small_closures(&m, 12);
        exact_compared += objects(&m).iter().filter(|o| exact.contains(&o.id)).count();
        compare(&m, &exact);
    }
    assert!(exact_compared > 1000, "{exact_compared}");
}

#[test]
fn index_answers_like_the_model_scans() {
    let m = awkward_model();
    let ix = ModelIndex::new(&m);
    let (scan, index) = (Names::Scan(&m), Names::Index(&ix));
    let mut names: Vec<String> = Vec::new();
    for t in &m.tables {
        names.push(t.name.clone());
        names.extend(t.columns.iter().map(|c| c.name.clone()));
        names.extend(t.measures.iter().map(|x| x.name.clone()));
    }
    names.extend(["", "nope", "ROWNUMBER-1", "price", "prod'S", "DUP"].map(String::from));
    let id = |x: Option<&Table>| x.map(std::ptr::from_ref);
    for a in &names {
        let variants = [a.clone(), a.to_ascii_uppercase(), a.to_ascii_lowercase()];
        for n in &variants {
            assert_eq!(id(scan.table(n)), id(index.table(n)), "{n}");
            assert_eq!(id(ix.table(n)), id(m.table(n)), "{n}");
            let pair = |x: Option<(&Table, &Measure)>| {
                x.map(|(t, x)| (std::ptr::from_ref(t), std::ptr::from_ref(x)))
            };
            assert_eq!(pair(scan.measure(n)), pair(index.measure(n)), "{n}");
            assert_eq!(pair(ix.measure(n)), pair(m.measure(n)), "{n}");
            let cpair =
                |x: Option<(&Table, &Column)>| x.map(|(t, c)| (std::ptr::from_ref(t), std::ptr::from_ref(c)));
            assert_eq!(cpair(scan.any_column(n)), cpair(index.any_column(n)), "{n}");
            for b in &names {
                assert_eq!(cpair(scan.column(b, n)), cpair(index.column(b, n)), "{b} {n}");
                assert_eq!(cpair(ix.column(b, n)), cpair(scan.column(b, n)), "{b} {n}");
                if let (Some(s), Some(i)) = (scan.table_at(b), index.table_at(b)) {
                    let mp = |x: Option<&Measure>| x.map(std::ptr::from_ref);
                    assert_eq!(mp(scan.measure_of(s, n)), mp(index.measure_of(i, n)), "{b} {n}");
                }
            }
        }
    }
}

/// The benchmark's shape: `n` measures over `n / 100` tables of 20 columns, chains of 100, two
/// 3-measure cycles.
fn bench_model(n: usize) -> SemanticModel {
    let tables = (n / 100).max(10);
    let mut m = SemanticModel::default();
    for t in 0..tables {
        let cols = (0..20)
            .map(|c| {
                let dt = [
                    DataType::Int64,
                    DataType::Double,
                    DataType::String,
                    DataType::Decimal,
                ][c % 4];
                col(&format!("Col {c}"), dt)
            })
            .collect();
        m.tables.push(table(&format!("Table {t}"), cols, Vec::new()));
    }
    let cycle_every = n / 2;
    for i in 0..n {
        let t = i % tables;
        let mut text = TEMPLATES[i % TEMPLATES.len()]
            .replace("{m}", &format!("SUM('Table {t}'[Col 1])"))
            .replace("Fact[Int]", &format!("'Table {t}'[Col 0]"))
            .replace("Fact[Calc]", &format!("'Table {t}'[Col 1]"))
            .replace("Fact[Name]", &format!("'Table {t}'[Col 2]"))
            .replace("Fact", &format!("'Table {t}'"));
        if cycle_every > 0 && i % cycle_every < 3 && i + 3 <= n {
            let start = i - i % cycle_every;
            let next = if i % cycle_every == 2 { start } else { i + 1 };
            text = format!("{text} + [Measure {next}]");
        }
        if i % 100 != 0 {
            text = format!("{text} + [Measure {}]", i - 1);
        }
        if let Some(tbl) = m.tables.get_mut(t) {
            tbl.measures.push(measure(&format!("Measure {i}"), &text));
        }
    }
    m
}

/// Long chains and cycles of deeply nested expressions: the nesting budgets cut inference both
/// inside one expression and across objects.
fn hostile_model(n: usize, cycle: bool, nesting: usize) -> SemanticModel {
    let (open, close) = ("(".repeat(nesting), ")".repeat(nesting));
    let ms = (0..n)
        .map(|i| {
            let next = if i + 1 < n {
                format!("[Chain {}]", i + 1)
            } else if cycle {
                "[Chain 0]".to_string()
            } else {
                "SUM(Fact[Int])".to_string()
            };
            measure(&format!("Chain {i}"), &format!("{open}{next} + 1{close}"))
        })
        .collect();
    SemanticModel {
        tables: vec![fact(Vec::new(), ms)],
        ..Default::default()
    }
}

#[test]
fn hostile_chains_match_and_stay_bounded() {
    for (n, cycle, nesting) in [
        (120, false, 60),
        (120, true, 60),
        (60, false, 120),
        (40, true, 127),
    ] {
        let m = hostile_model(n, cycle, nesting);
        compare(&m, &HashSet::new());
    }
    for cycle in [false, true] {
        let m = hostile_model(3_000, cycle, 20);
        let mut v = Validator::new(&m);
        for o in objects(&m) {
            let r = v.validate_as(o.text, o.home, o.kind);
            assert!(matches!(r.result_type, Some(ValueType::Scalar(_))), "{r:?}");
        }
    }
}

#[test]
fn bench_shape_matches() {
    let m = bench_model(400);
    compare(&m, &small_closures(&m, 12));
}

/// Validating every object of a model grows linearly with its size. Ignored by default: run with
/// `cargo test -p bicraft-dax --release -- --ignored --nocapture validator_scales`.
#[test]
#[ignore]
fn validator_scales_linearly() {
    let mut times: HashMap<usize, f64> = HashMap::new();
    for n in [100, 1_000, 10_000] {
        let m = bench_model(n);
        let objs = objects(&m);
        // Best of a few runs, to keep machine noise out of the ratio.
        let mut secs = f64::MAX;
        let mut invalid = 0;
        for _ in 0..5 {
            let start = Instant::now();
            let mut v = Validator::new(&m);
            invalid = objs
                .iter()
                .filter(|o| !v.validate_as(o.text, o.home, o.kind).valid)
                .count();
            secs = secs.min(start.elapsed().as_secs_f64());
        }
        let old = if n <= 1_000 {
            let mut old = f64::MAX;
            for _ in 0..3 {
                let start = Instant::now();
                for o in &objs {
                    let _ = per_expression(&m, o);
                }
                old = old.min(start.elapsed().as_secs_f64());
            }
            format!(", per expression {:.1} ms", old * 1e3)
        } else {
            String::new()
        };
        eprintln!("{n} measures: {:.1} ms{old}, {invalid} invalid", secs * 1e3);
        times.insert(n, secs);
    }
    let (small, large) = (times[&1_000], times[&10_000]);
    assert!(
        large < 30.0 * small.max(1e-3),
        "10k: {large:.3} s, 1k: {small:.3} s"
    );
}
