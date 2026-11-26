//! Randomized differential tests of the storage engine (`docs/storage-engine.md`): star schemas
//! generated at random from a seed (a few thousand sales, segments of a few sizes, compressed
//! columns, orphan keys, blanks, negative and whole numbers, Currency, doubles, an inactive and
//! sometimes a bidirectional relationship), queries generated from the shapes of the engine bench
//! by a small grammar, each evaluated by the storage engine and by the reference evaluator (the
//! per-cell path), which must agree cell by cell.
//!
//! `cargo test -p bicraft-engine storage_fuzz` runs [`DEFAULT_CASES`] cases;
//! `BICRAFT_FUZZ_CASES=5000` runs more, `BICRAFT_FUZZ_SEED=…` starts elsewhere. A failure prints
//! the seed, the model and the query, shrunk to the smallest variant that still differs.

use bicraft_vertipaq::CellValue;
use serde_json::{Value, json};

use super::super::store::{Codes, ColData};
use super::storage_tests::{evaluate, same};
use crate::Session;

/// Cases of one run (queries over models; a new model every [`QUERIES_PER_MODEL`]).
const DEFAULT_CASES: usize = 240;
const QUERIES_PER_MODEL: usize = 24;

/// A small deterministic generator (splitmix64).
pub(super) struct Rng(pub(super) u64);

impl Rng {
    pub(super) fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut x = self.0;
        x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        x ^ (x >> 31)
    }
    pub(super) fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[self.below(v.len() as u64) as usize]
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

/// What a generated model is made of.
#[derive(Debug, Clone)]
pub(super) struct Spec {
    pub(super) rows: u64,
    pub(super) products: u64,
    pub(super) stores: u64,
    pub(super) customers: u64,
    pub(super) days: u64,
    pub(super) seg: usize,
    pub(super) both: bool,
    pub(super) orphans: bool,
    pub(super) blanks: bool,
}

pub(super) fn spec(r: &mut Rng) -> Spec {
    Spec {
        rows: 300 + r.below(2700),
        products: 3 + r.below(120),
        stores: 2 + r.below(30),
        customers: 2 + r.below(800),
        days: 40 + r.below(800),
        seg: *r.pick(&[257usize, 2047, 2048, 2049, 4096, 65_536]),
        both: r.chance(30),
        orphans: r.chance(60),
        blanks: r.chance(60),
    }
}

pub(super) const DAY0: f64 = 44_197.0;

pub(super) fn col(values: Vec<CellValue>) -> ColData {
    ColData::from_stored(values).unwrap()
}

pub(super) fn packed(c: ColData, seg: usize) -> ColData {
    ColData {
        codes: Codes::pack(&c, seg),
        ..c
    }
}

/// The model of `sp`, its data made from `seed`.
pub(super) fn model(sp: &Spec, seed: u64) -> Session {
    let mut s = Session::new();
    let mut steps: Vec<(&str, Value)> = vec![("model.new", json!({"name": "Fuzz", "culture": "en-US"}))];
    let tables: [(&str, &[(&str, &str)]); 4] = [
        (
            "Produit",
            &[
                ("ProduitKey", "int64"),
                ("Categorie", "string"),
                ("Prix", "decimal"),
            ],
        ),
        ("Magasin", &[("MagasinKey", "int64"), ("Region", "string")]),
        (
            "Calendrier",
            &[("Date", "dateTime"), ("Annee", "int64"), ("Mois", "int64")],
        ),
        (
            "Ventes",
            &[
                ("Date", "dateTime"),
                ("Livraison", "dateTime"),
                ("ProduitKey", "int64"),
                ("MagasinKey", "int64"),
                ("ClientKey", "int64"),
                ("Quantite", "int64"),
                ("Montant", "decimal"),
                ("Poids", "double"),
            ],
        ),
    ];
    for (t, cols) in tables {
        steps.push((
            "table.add",
            json!({"name": t, "partition": {"source_type": "m", "expression": "#table({}, {})"}}),
        ));
        for (c, ty) in cols {
            steps.push(("column.add", json!({"table": t, "name": c, "data_type": ty})));
        }
    }
    steps.push((
        "table.set",
        json!({"name": "Calendrier", "data_category": "Time", "key_column": "Date"}),
    ));
    steps.push((
        "relationship.add",
        json!({"from": "Ventes[Date]", "to": "Calendrier[Date]"}),
    ));
    steps.push((
        "relationship.add",
        json!({"from": "Ventes[Livraison]", "to": "Calendrier[Date]", "active": false}),
    ));
    steps.push((
        "relationship.add",
        json!({"from": "Ventes[ProduitKey]", "to": "Produit[ProduitKey]"}),
    ));
    steps.push((
        "relationship.add",
        json!({"from": "Ventes[MagasinKey]", "to": "Magasin[MagasinKey]",
            "cross_filter": if sp.both { "both" } else { "single" }}),
    ));
    for (name, dax) in [
        ("CA", "SUM(Ventes[Montant])"),
        ("Qte", "SUM(Ventes[Quantite])"),
        ("Poids total", "SUM(Ventes[Poids])"),
        ("Nb clients", "DISTINCTCOUNT(Ventes[ClientKey])"),
        ("Lignes", "COUNTROWS(Ventes)"),
        ("CA tous produits", "CALCULATE([CA], ALL(Produit))"),
        ("Part CA", "DIVIDE([CA], [CA tous produits])"),
        (
            "CA livré",
            "CALCULATE([CA], USERELATIONSHIP(Ventes[Livraison], Calendrier[Date]))",
        ),
        ("CA YTD", "TOTALYTD([CA], Calendrier[Date])"),
        (
            "Rang",
            "IF(HASONEVALUE(Produit[Categorie]), RANKX(ALL(Produit[Categorie]), [CA]))",
        ),
    ] {
        steps.push((
            "measure.add",
            json!({"table": "Ventes", "name": name, "expression": dax}),
        ));
    }
    for (id, p) in steps {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"));
    }
    let mut r = Rng(seed);
    let cats = 1 + sp.products / 4;
    let (mut a, mut b, mut c) = (Vec::new(), Vec::new(), Vec::new());
    for k in 1..=sp.products {
        a.push(CellValue::Int(k as i64));
        b.push(if sp.blanks && k % 11 == 0 {
            CellValue::Null
        } else {
            CellValue::Text(format!("C{:02}", 1 + r.below(cats)))
        });
        c.push(CellValue::Decimal((50 + r.below(50_000)) as i64 * 100));
    }
    super::super::inject_table(
        &s,
        "Produit",
        sp.products as usize,
        vec![
            ("ProduitKey".into(), col(a)),
            ("Categorie".into(), col(b)),
            ("Prix".into(), col(c)),
        ],
    );
    let (mut a, mut b) = (Vec::new(), Vec::new());
    for k in 1..=sp.stores {
        a.push(CellValue::Int(k as i64));
        b.push(CellValue::Text(format!("R{}", 1 + r.below(4))));
    }
    super::super::inject_table(
        &s,
        "Magasin",
        sp.stores as usize,
        vec![("MagasinKey".into(), col(a)), ("Region".into(), col(b))],
    );
    let (mut a, mut b, mut c) = (Vec::new(), Vec::new(), Vec::new());
    for d in 0..sp.days {
        let (y, m, _) = super::ymd(DAY0 as i64 + d as i64);
        a.push(CellValue::DateTime(DAY0 + d as f64));
        b.push(CellValue::Int(y));
        c.push(CellValue::Int(m));
    }
    super::super::inject_table(
        &s,
        "Calendrier",
        sp.days as usize,
        vec![
            ("Date".into(), col(a)),
            ("Annee".into(), col(b)),
            ("Mois".into(), col(c)),
        ],
    );
    let mut v: [Vec<CellValue>; 8] = Default::default();
    // sorted by date some of the time (segments then hold a few dates: segment skipping)
    let sorted = r.chance(40);
    for k in 0..sp.rows {
        let day = if sorted {
            k * sp.days / sp.rows
        } else {
            r.below(sp.days)
        };
        let mut product = 1 + r.below(sp.products);
        if sp.orphans && r.chance(3) {
            product = sp.products + 1 + r.below(3);
        }
        let blank = |r: &mut Rng| sp.blanks && r.chance(4);
        v[0].push(CellValue::DateTime(DAY0 + day as f64));
        v[1].push(if blank(&mut r) {
            CellValue::Null
        } else {
            CellValue::DateTime(DAY0 + (day + r.below(10)).min(sp.days - 1) as f64)
        });
        v[2].push(CellValue::Int(product as i64));
        v[3].push(CellValue::Int(1 + r.below(sp.stores) as i64));
        v[4].push(if blank(&mut r) {
            CellValue::Null
        } else {
            CellValue::Int(1 + r.below(sp.customers) as i64)
        });
        // whole numbers, some negative (a value encoding with a negative base)
        v[5].push(if blank(&mut r) {
            CellValue::Null
        } else {
            CellValue::Int(r.below(13) as i64 - 3)
        });
        v[6].push(if blank(&mut r) {
            CellValue::Null
        } else {
            CellValue::Decimal(r.below(2_000_000) as i64 - 100_000)
        });
        v[7].push(if blank(&mut r) {
            CellValue::Null
        } else {
            CellValue::Real((r.below(1 << 40) as f64) / 7.0 - 1.0e10)
        });
    }
    let names = [
        "Date",
        "Livraison",
        "ProduitKey",
        "MagasinKey",
        "ClientKey",
        "Quantite",
        "Montant",
        "Poids",
    ];
    let cols = names
        .iter()
        .zip(v)
        .map(|(n, vals)| (n.to_string(), packed(col(vals), sp.seg)))
        .collect();
    super::super::inject_table(&s, "Ventes", sp.rows as usize, cols);
    s
}

/// One generated query: its parts, so that it can be shrunk.
#[derive(Debug, Clone)]
pub(super) struct Query {
    shape: u8,
    groups: Vec<&'static str>,
    filters: Vec<String>,
    items: Vec<String>,
    /// The first grouping column rolled up (`ROLLUPADDISSUBTOTAL`).
    rollup: bool,
}

const GROUPS: &[&str] = &[
    "Produit[Categorie]",
    "Magasin[Region]",
    "Calendrier[Annee]",
    "Calendrier[Mois]",
    "Ventes[Quantite]",
    "Ventes[ClientKey]",
    "Magasin[MagasinKey]",
];

fn measure(r: &mut Rng, depth: u32) -> String {
    let cols = [
        "Ventes[Montant]",
        "Ventes[Quantite]",
        "Ventes[Poids]",
        "Ventes[ClientKey]",
        "Ventes[Date]",
    ];
    let base = match r.below(17) {
        0 => "[CA]".to_string(),
        1 => "[Qte]".to_string(),
        2 => "[Poids total]".to_string(),
        3 => "[Nb clients]".to_string(),
        4 => "[Lignes]".to_string(),
        5 => "[Part CA]".to_string(),
        6 => "[CA livré]".to_string(),
        7 => "[Rang]".to_string(),
        8 => format!(
            "{}({})",
            r.pick(&[
                "SUM",
                "AVERAGE",
                "MIN",
                "MAX",
                "COUNT",
                "COUNTBLANK",
                "DISTINCTCOUNT"
            ]),
            r.pick(&cols)
        ),
        9 => format!(
            "{}(FILTER(Ventes, Ventes[Quantite] {} {}), {})",
            r.pick(&["SUMX", "AVERAGEX", "MINX", "MAXX", "COUNTX"]),
            r.pick(&[">", ">=", "<", "=", "<>"]),
            r.below(10) as i64 - 3,
            r.pick(&["Ventes[Montant]", "Ventes[Quantite]", "Ventes[Poids]"])
        ),
        10 => "SUMX(Ventes, Ventes[Quantite] * RELATED(Produit[Prix]))".to_string(),
        11 => "SUMX(VALUES(Magasin[Region]), [CA])".to_string(),
        12 => "[CA YTD]".to_string(),
        13 => "COUNTROWS(VALUES(Ventes[ClientKey]))".to_string(),
        14 => "DIVIDE([CA], [Qte])".to_string(),
        // a grouping of a fact column counted (without its rows when a filter wraps it)
        15 => {
            let grouping = format!(
                "SUMMARIZECOLUMNS({}, \"x\", {})",
                r.pick(&[
                    "Ventes[ClientKey]",
                    "Ventes[Quantite]",
                    "Ventes[Montant]",
                    "Ventes[Date]"
                ]),
                r.pick(&[
                    "SUM(Ventes[Montant])",
                    "SUM(Ventes[Quantite])",
                    "SUM(Ventes[Poids])",
                    "[CA]",
                    "[Lignes]",
                    "COUNTROWS(Ventes)"
                ])
            );
            if r.chance(50) {
                format!(
                    "COUNTROWS(CALCULATETABLE({grouping}, {}))",
                    r.pick(&[
                        "Produit[Categorie] = \"C02\"",
                        "Calendrier[Annee] = 2022",
                        "Ventes[Quantite] > 2",
                        "USERELATIONSHIP(Ventes[Livraison], Calendrier[Date])"
                    ])
                )
            } else {
                format!("COUNTROWS({grouping})")
            }
        }
        _ => "AVERAGEX(VALUES(Magasin[MagasinKey]), [Qte])".to_string(),
    };
    if depth == 0 || !r.chance(40) {
        return base;
    }
    let filter = match r.below(8) {
        0 => format!("Produit[Categorie] = \"C{:02}\"", 1 + r.below(4)),
        1 => "ALL(Produit)".to_string(),
        2 => "ALLEXCEPT(Produit, Produit[Categorie])".to_string(),
        3 => format!(
            "KEEPFILTERS(Magasin[Region] IN {{\"R1\", \"R{}\"}})",
            2 + r.below(3)
        ),
        4 => format!("Ventes[Quantite] >= {}", r.below(8) as i64 - 2),
        5 => format!("TREATAS({{{}}}, Calendrier[Annee])", 2021 + r.below(3)),
        6 => "USERELATIONSHIP(Ventes[Livraison], Calendrier[Date])".to_string(),
        _ => "ALL(Calendrier)".to_string(),
    };
    format!("CALCULATE({}, {filter})", measure(r, depth - 1))
}

pub(super) fn query(r: &mut Rng) -> Query {
    let shape = r.below(4) as u8;
    let n = if shape == 1 { 0 } else { 1 + r.below(2) as usize };
    let mut groups = Vec::new();
    while groups.len() < n {
        let g = *r.pick(GROUPS);
        if !groups.contains(&g) {
            groups.push(g);
        }
    }
    let mut filters = Vec::new();
    if shape == 0 && r.chance(40) {
        filters.push(match r.below(3) {
            0 => format!(
                "TREATAS({{\"C01\", \"C0{}\"}}, Produit[Categorie])",
                2 + r.below(3)
            ),
            1 => format!("TREATAS({{{}}}, Calendrier[Annee])", 2021 + r.below(3)),
            _ => format!(
                "FILTER(KEEPFILTERS(VALUES(Ventes[Quantite])), Ventes[Quantite] > {})",
                r.below(6) as i64 - 2
            ),
        });
    }
    let items = (0..1 + r.below(3)).map(|_| measure(r, 2)).collect();
    let rollup = shape == 0 && r.chance(25);
    Query {
        shape,
        groups,
        filters,
        items,
        rollup,
    }
}

impl Query {
    pub(super) fn text(&self) -> String {
        let items: Vec<String> = self
            .items
            .iter()
            .enumerate()
            .map(|(i, m)| format!("\"v{i}\", {m}"))
            .collect();
        let order = |g: &[&str]| {
            if g.is_empty() {
                String::new()
            } else {
                format!(" ORDER BY {}", g.join(", "))
            }
        };
        match self.shape {
            1 => format!("EVALUATE ROW({})", items.join(", ")),
            2 if !self.groups.is_empty() => format!(
                "EVALUATE ADDCOLUMNS(VALUES({}), {}){}",
                self.groups[0],
                items.join(", "),
                order(&self.groups[..1])
            ),
            3 if !self.groups.is_empty() => format!(
                "EVALUATE TOPN(5, SUMMARIZECOLUMNS({}, \"v0\", {}), [v0], DESC, {}, ASC) ORDER BY [v0] DESC, {}",
                self.groups[0], self.items[0], self.groups[0], self.groups[0]
            ),
            _ => {
                let mut args: Vec<String> = self.groups.iter().map(|g| g.to_string()).collect();
                let mut ord = order(&self.groups);
                if self.rollup
                    && let Some(first) = args.first_mut()
                {
                    *first = format!("ROLLUPADDISSUBTOTAL({first}, \"T\")");
                    ord = format!(" ORDER BY [T], {}", self.groups.join(", "));
                }
                args.extend(self.filters.iter().cloned());
                args.extend(items);
                format!("EVALUATE SUMMARIZECOLUMNS({}){ord}", args.join(", "))
            }
        }
    }

    /// Simpler variants: one part less.
    fn shrinks(&self) -> Vec<Query> {
        let mut out = Vec::new();
        for i in 0..self.items.len() {
            if self.items.len() > 1 {
                let mut q = self.clone();
                q.items.remove(i);
                out.push(q);
            }
            // a CALCULATE replaced by its expression
            if let Some(inner) = self.items[i].strip_prefix("CALCULATE(")
                && let Some(comma) = inner.rfind(", ")
            {
                let mut q = self.clone();
                q.items[i] = inner[..comma].to_string();
                out.push(q);
            }
        }
        for i in 0..self.groups.len() {
            if self.groups.len() > 1 {
                let mut q = self.clone();
                q.groups.remove(i);
                out.push(q);
            }
        }
        for i in 0..self.filters.len() {
            let mut q = self.clone();
            q.filters.remove(i);
            out.push(q);
        }
        if self.rollup {
            let mut q = self.clone();
            q.rollup = false;
            out.push(q);
        }
        out
    }
}

thread_local! {
    /// (both answered, the reference refused, both refused) since the start, and a few errors.
    static TALLY: std::cell::RefCell<(usize, usize, usize, Vec<String>)> = const {
        std::cell::RefCell::new((0, 0, 0, Vec::new()))
    };
}

/// `None`: the engines agree; else what differs.
fn differs(s: &mut Session, q: &str) -> Option<String> {
    let reference = evaluate(s, q, false);
    let got = evaluate(s, q, true);
    TALLY.with(|t| {
        let mut t = t.borrow_mut();
        match (&reference, &got) {
            (Ok(_), Ok(_)) => t.0 += 1,
            (Err(e), Ok(_)) => {
                t.1 += 1;
                if t.3.len() < 6 {
                    t.3.push(format!("reference refused: {e}"));
                }
            }
            (Err(e), Err(_)) => {
                t.2 += 1;
                if t.3.len() < 6 {
                    t.3.push(format!("both refused {q}: {e}"));
                }
            }
            _ => {}
        }
    });
    // again, from the caches the first run filled
    let warm = evaluate(s, q, true);
    if let (Ok(a), Ok(b)) = (&got, &warm)
        && a != b
    {
        return Some(format!("storage cold {got:?}\n  storage warm {warm:?}"));
    }
    match (&reference, &got) {
        (Ok(a), Ok(b)) if same(a, b) => None,
        (Err(_), Ok(_)) => None,
        (Err(a), Err(b)) if a == b => None,
        _ => Some(format!("reference {reference:?}\n  storage   {got:?}")),
    }
}

fn shrink(s: &mut Session, mut q: Query) -> (Query, String) {
    let mut why = differs(s, &q.text()).unwrap_or_default();
    'outer: loop {
        for c in q.shrinks() {
            if let Some(w) = differs(s, &c.text()) {
                q = c;
                why = w;
                continue 'outer;
            }
        }
        return (q, why);
    }
}

#[test]
fn storage_fuzz_matches_the_reference_evaluator() {
    let cases: usize = bicraft_env::var("BICRAFT_FUZZ_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CASES);
    let start: u64 = bicraft_env::var("BICRAFT_FUZZ_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0x5EED_0001);
    let mut failures = Vec::new();
    let mut done = 0usize;
    let mut m = 0u64;
    while done < cases {
        let seed = start.wrapping_add(m);
        m += 1;
        let mut r = Rng(seed);
        let sp = spec(&mut r);
        let mut s = model(&sp, seed);
        for k in 0..QUERIES_PER_MODEL.min(cases - done) {
            let q = query(&mut r);
            done += 1;
            if differs(&mut s, &q.text()).is_some() {
                let (q, why) = shrink(&mut s, q);
                failures.push(format!(
                    "seed {seed} (query {k}) model {sp:?}\n  {}\n  {why}",
                    q.text()
                ));
                if failures.len() >= 5 {
                    break;
                }
            }
        }
        if failures.len() >= 5 {
            break;
        }
    }
    TALLY.with(|t| {
        let t = t.borrow();
        println!(
            "storage fuzz: {done} queries, {} answered by both, {} refused by the reference only, {} by both",
            t.0, t.1, t.2
        );
        for e in &t.3 {
            println!("  {e}");
        }
        // the generated queries must mostly be answered, else the test tests nothing
        assert!(t.0 * 10 >= done * 8, "too few queries answered by both engines");
    });
    assert!(
        failures.is_empty(),
        "{} of {done} generated queries differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The queries the determinism and cache tests run (doubles, Currency, groupings, iterators).
fn fixed_queries() -> Vec<String> {
    [
        "EVALUATE ROW(\"p\", [Poids total], \"c\", [CA], \"a\", AVERAGE(Ventes[Poids]), \"x\", SUMX(Ventes, Ventes[Poids]))",
        "EVALUATE SUMMARIZECOLUMNS(Magasin[MagasinKey], \"p\", [Poids total], \"c\", [CA], \"q\", [Qte]) ORDER BY Magasin[MagasinKey]",
        "EVALUATE SUMMARIZECOLUMNS(Ventes[ClientKey], \"p\", [Poids total], \"n\", [Lignes]) ORDER BY Ventes[ClientKey]",
        "EVALUATE SUMMARIZECOLUMNS(Produit[Categorie], \"p\", AVERAGEX(FILTER(Ventes, Ventes[Quantite] > 2), Ventes[Poids]), \
         \"r\", [Rang], \"d\", [Nb clients]) ORDER BY Produit[Categorie]",
        "EVALUATE SUMMARIZECOLUMNS(Calendrier[Annee], \"y\", [CA YTD], \"l\", [CA livré]) ORDER BY Calendrier[Annee]",
    ]
    .iter()
    .map(|q| q.to_string())
    .collect()
}

pub(super) fn big_spec() -> Spec {
    Spec {
        rows: 300_000,
        products: 80,
        stores: 20,
        customers: 30_000,
        days: 700,
        seg: 65_536,
        both: false,
        orphans: true,
        blanks: true,
    }
}

fn run(s: &mut Session, q: &str) -> String {
    s.execute("dax.clear_cache", &json!({})).unwrap();
    let v = evaluate(s, q, true).unwrap_or_else(|e| panic!("{q}: {e}"));
    v["results"].to_string()
}

#[test]
fn storage_results_are_the_same_bits_on_any_number_of_threads() {
    // 300 k rows: several morsels, several workers; a column of doubles with many values (its
    // sums are compensated, merged in row order)
    let mut s = model(&big_spec(), 7);
    for q in fixed_queries() {
        bicraft_scan::set_threads(Some(1));
        let one = run(&mut s, &q);
        for n in [2usize, 8, 20] {
            bicraft_scan::set_threads(Some(n));
            assert_eq!(run(&mut s, &q), one, "{n} threads: {q}");
        }
        bicraft_scan::set_threads(None);
    }
}

#[test]
fn storage_caches_follow_the_model_and_the_data() {
    let sp = Spec {
        rows: 2_000,
        ..big_spec()
    };
    let mut s = model(&sp, 11);
    let q = "EVALUATE SUMMARIZECOLUMNS(Magasin[Region], \"c\", [CA], \"x\", SUMX(FILTER(Ventes, Ventes[Quantite] > 1), Ventes[Montant])) \
             ORDER BY Magasin[Region]";
    let check = |s: &mut Session, what: &str| {
        let got = evaluate(s, q, true).unwrap();
        let want = evaluate(s, q, false).unwrap();
        assert!(same(&got, &want), "{what}: {got} / {want}");
        got
    };
    let first = check(&mut s, "first");
    // warm: the same
    assert_eq!(check(&mut s, "warm"), first);
    // a model edit: [CA] now sums the quantities
    s.execute(
        "measure.set",
        &json!({"name": "CA", "expression": "SUM(Ventes[Quantite])"}),
    )
    .unwrap();
    let edited = check(&mut s, "after measure.set");
    assert_ne!(edited, first);
    // new data for the fact
    let mut r = Rng(99);
    let rows = 1_500u64;
    let mut v: [Vec<CellValue>; 8] = Default::default();
    for _ in 0..rows {
        v[0].push(CellValue::DateTime(DAY0 + r.below(sp.days) as f64));
        v[1].push(CellValue::Null);
        v[2].push(CellValue::Int(1 + r.below(sp.products) as i64));
        v[3].push(CellValue::Int(1 + r.below(sp.stores) as i64));
        v[4].push(CellValue::Int(1 + r.below(50) as i64));
        v[5].push(CellValue::Int(r.below(5) as i64));
        v[6].push(CellValue::Decimal(r.below(10_000) as i64 * 100));
        v[7].push(CellValue::Real(r.below(1000) as f64 / 3.0));
    }
    let names = [
        "Date",
        "Livraison",
        "ProduitKey",
        "MagasinKey",
        "ClientKey",
        "Quantite",
        "Montant",
        "Poids",
    ];
    let cols = names
        .iter()
        .zip(v)
        .map(|(n, vals)| (n.to_string(), packed(col(vals), 512)))
        .collect();
    super::super::inject_table(&s, "Ventes", rows as usize, cols);
    let changed = check(&mut s, "after new data");
    assert_ne!(changed, edited);
    // dax.clear_cache: the same results, recomputed
    s.execute("dax.clear_cache", &json!({})).unwrap();
    assert_eq!(check(&mut s, "after dax.clear_cache"), changed);
    // another session on other data has nothing of this one's
    let mut t = model(&sp, 13);
    let a = evaluate(&mut t, q, true).unwrap();
    let b = evaluate(&mut t, q, false).unwrap();
    assert!(same(&a, &b), "a new session: {a} / {b}");
}

/// The results kept between queries (iterators, counted groupings, RANKX's values) follow the
/// clock and ALLSELECTED: an expression that reads them is recomputed, never served stale.
#[test]
fn storage_caches_follow_the_clock_and_allselected() {
    let sp = Spec {
        rows: 2_000,
        ..big_spec()
    };
    let mut s = model(&sp, 17);
    let q = "EVALUATE SUMMARIZECOLUMNS(Produit[Categorie], \
             \"x\", SUMX(FILTER(Ventes, Ventes[Date] <= TODAY()), Ventes[Montant]), \
             \"n\", COUNTROWS(CALCULATETABLE(SUMMARIZECOLUMNS(Ventes[ClientKey], \"m\", [CA]), Calendrier[Date] <= TODAY())), \
             \"r\", RANKX(ALL(Produit[Categorie]), CALCULATE([CA], Calendrier[Date] <= TODAY()))) \
             ORDER BY Produit[Categorie]";
    let check = |s: &mut Session, q: &str, what: &str| {
        let got = evaluate(s, q, true).unwrap();
        let want = evaluate(s, q, false).unwrap();
        assert!(same(&got, &want), "{what}: {got} / {want}");
        got
    };
    s.execute("view.set_as_of", &json!({"as_of": "2021-03-01"}))
        .unwrap();
    let march = check(&mut s, q, "2021-03-01");
    assert_eq!(check(&mut s, q, "2021-03-01 warm"), march);
    s.execute("view.set_as_of", &json!({"as_of": "2021-09-01"}))
        .unwrap();
    let september = check(&mut s, q, "2021-09-01");
    assert_ne!(september, march, "the clock moved: the results must too");
    // ALLSELECTED in the ranked expression: two outer selections, two answers
    let q2 = |regions: &str| {
        format!(
            "EVALUATE SUMMARIZECOLUMNS(Produit[Categorie], TREATAS({{{regions}}}, Magasin[Region]), \
             \"r\", RANKX(ALL(Produit[Categorie]), CALCULATE([CA], ALLSELECTED(Magasin))), \
             \"s\", SUMX(VALUES(Produit[Categorie]), CALCULATE([CA], ALLSELECTED(Magasin)))) ORDER BY Produit[Categorie]"
        )
    };
    let one = check(&mut s, &q2("\"R1\""), "ALLSELECTED R1");
    let two = check(&mut s, &q2("\"R2\", \"R3\""), "ALLSELECTED R2, R3");
    assert_ne!(one, two);
    assert_eq!(check(&mut s, &q2("\"R1\""), "ALLSELECTED R1 again"), one);
}

#[test]
fn storage_currency_sums_are_exact_and_overflows_fail_as_the_reference() {
    // Currency to the cent over many rows, and sums past the Currency range
    let sp = Spec {
        rows: 3_000,
        ..big_spec()
    };
    let mut s = model(&sp, 21);
    let rows = 3_000u64;
    let mut v: [Vec<CellValue>; 8] = Default::default();
    for k in 0..rows {
        v[0].push(CellValue::DateTime(DAY0 + (k % sp.days) as f64));
        v[1].push(CellValue::Null);
        v[2].push(CellValue::Int(1 + (k % sp.products) as i64));
        v[3].push(CellValue::Int(1 + (k % sp.stores) as i64));
        v[4].push(CellValue::Int(1));
        v[5].push(CellValue::Int(1));
        // 0.01 steps, and a few values near the Currency limit
        v[6].push(CellValue::Decimal(if k % 1000 == 0 {
            900_000_000_000_000_000
        } else {
            100 + k as i64
        }));
        v[7].push(CellValue::Real(0.1));
    }
    let names = [
        "Date",
        "Livraison",
        "ProduitKey",
        "MagasinKey",
        "ClientKey",
        "Quantite",
        "Montant",
        "Poids",
    ];
    let cols = names
        .iter()
        .zip(v)
        .map(|(n, vals)| (n.to_string(), packed(col(vals), 700)))
        .collect();
    super::super::inject_table(&s, "Ventes", rows as usize, cols);
    for q in [
        "EVALUATE ROW(\"c\", [CA])",
        "EVALUATE ROW(\"c\", CALCULATE([CA], Ventes[Montant] < 1000000))",
        "EVALUATE SUMMARIZECOLUMNS(Magasin[MagasinKey], \"c\", CALCULATE([CA], Ventes[Montant] < 1000000), \
         \"a\", AVERAGE(Ventes[Montant]), \"p\", [Poids total]) ORDER BY Magasin[MagasinKey]",
        "EVALUATE ROW(\"x\", SUMX(Ventes, Ventes[Montant]))",
    ] {
        let got = evaluate(&mut s, q, true);
        let want = evaluate(&mut s, q, false);
        match (&got, &want) {
            (Ok(a), Ok(b)) => assert!(same(a, b), "{q}: {a} / {b}"),
            (Err(a), Err(b)) => assert_eq!(a, b, "{q}"),
            _ => panic!("{q}: storage {got:?}, reference {want:?}"),
        }
    }
}
