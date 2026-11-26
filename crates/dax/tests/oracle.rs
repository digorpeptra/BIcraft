//! Engine oracle suite (M6.3): how often does bicraft's DAX validator agree with the real
//! Analysis Services engine?
//!
//! `oracle/corpus.json` holds DAX expressions, each with the role it plays in the synthetic model
//! `oracle/model.bim` (measure, calculated column, calculated table, row-level security filter or
//! query). `oracle/verdicts.json` holds the verdicts the engine of Power BI Desktop gave on them,
//! recorded with the maintainers' development oracles (kept outside this repository). This test
//! needs neither Windows nor the engine: it replays the recorded verdicts against bicraft's own,
//! prints agreement figures and the disagreements, and fails when agreement drops below
//! [`AGREEMENT_FLOOR`].
//!
//! Set `BICRAFT_ORACLE_REPORT=<file>` to also write the disagreements as a Markdown table.
//! See docs/dax-oracle.md.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use bicraft_dax::{Context, ExprKind, Severity, Validation, Validator, deps, validate_as, validate_query};
use bicraft_model::{
    Column, DataType, Measure, Partition, Relationship, Role, SemanticModel, Table, TablePermission,
    Variation,
};

#[path = "oracle/signatures.rs"]
mod signatures;

/// Minimum share of corpus entries on which bicraft and the engine must agree (definition
/// verdicts). Measured 2026-10-13: 916/917 = 99.9%; the floor sits a little below. Raise it as the
/// validator improves.
const AGREEMENT_FLOOR: f64 = 0.99;

/// Most false rejects (valid DAX that bicraft refuses) tolerated. They are worse than false
/// accepts: they block legitimate edits. Measured 2026-10-13: 0.
const MAX_FALSE_REJECTS: usize = 0;

// ------------------------------------------------------------------------------------------------
// A small JSON reader: the dax crate has no JSON dependency and this test must not add one.

#[derive(Debug, Clone, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    fn str(&self, key: &str) -> Option<&str> {
        match self.get(key) {
            Some(Json::Str(s)) => Some(s),
            _ => None,
        }
    }
    fn bool(&self, key: &str) -> Option<bool> {
        match self.get(key) {
            Some(Json::Bool(b)) => Some(*b),
            _ => None,
        }
    }
    fn arr(&self, key: &str) -> &[Json] {
        match self.get(key) {
            Some(Json::Arr(a)) => a,
            _ => &[],
        }
    }
}

struct Reader<'a> {
    s: &'a [u8],
    i: usize,
}

impl Reader<'_> {
    fn parse(text: &str) -> Result<Json, String> {
        let mut r = Reader {
            s: text.as_bytes(),
            i: 0,
        };
        let v = r.value(0)?;
        r.ws();
        if r.i != r.s.len() {
            return Err(format!("trailing data at byte {}", r.i));
        }
        Ok(v)
    }

    fn ws(&mut self) {
        while matches!(self.s.get(self.i), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.i += 1;
        }
    }

    fn eat(&mut self, b: u8) -> Result<(), String> {
        self.ws();
        if self.s.get(self.i) == Some(&b) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected `{}` at byte {}", b as char, self.i))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, String> {
        if depth > 64 {
            return Err("nesting too deep".into());
        }
        self.ws();
        match self.s.get(self.i) {
            Some(b'{') => {
                self.i += 1;
                let mut fields = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(Json::Obj(fields));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.eat(b':')?;
                    fields.push((k, self.value(depth + 1)?));
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Json::Obj(fields));
                        }
                        _ => return Err(format!("expected `,` or `}}` at byte {}", self.i)),
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(Json::Arr(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Json::Arr(items));
                        }
                        _ => return Err(format!("expected `,` or `]` at byte {}", self.i)),
                    }
                }
            }
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => self.word("true", Json::Bool(true)),
            Some(b'f') => self.word("false", Json::Bool(false)),
            Some(b'n') => self.word("null", Json::Null),
            _ => {
                let start = self.i;
                while matches!(
                    self.s.get(self.i),
                    Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
                ) {
                    self.i += 1;
                }
                std::str::from_utf8(&self.s[start..self.i])
                    .ok()
                    .and_then(|t| t.parse().ok())
                    .map(Json::Num)
                    .ok_or_else(|| format!("bad value at byte {start}"))
            }
        }
    }

    fn word(&mut self, w: &str, v: Json) -> Result<Json, String> {
        if self.s[self.i..].starts_with(w.as_bytes()) {
            self.i += w.len();
            Ok(v)
        } else {
            Err(format!("bad literal at byte {}", self.i))
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let h = self
            .s
            .get(self.i..self.i + 4)
            .and_then(|b| std::str::from_utf8(b).ok())
            .and_then(|t| u32::from_str_radix(t, 16).ok())
            .ok_or_else(|| format!("bad \\u escape at byte {}", self.i))?;
        self.i += 4;
        Ok(h)
    }

    fn string(&mut self) -> Result<String, String> {
        if self.s.get(self.i) != Some(&b'"') {
            return Err(format!("expected string at byte {}", self.i));
        }
        self.i += 1;
        let mut out = Vec::new();
        loop {
            match self.s.get(self.i) {
                None => return Err("unterminated string".into()),
                Some(b'"') => {
                    self.i += 1;
                    return String::from_utf8(out).map_err(|e| e.to_string());
                }
                Some(b'\\') => {
                    self.i += 1;
                    let c = self.s.get(self.i).copied();
                    self.i += 1;
                    let ch = match c {
                        Some(b'"') => '"',
                        Some(b'\\') => '\\',
                        Some(b'/') => '/',
                        Some(b'n') => '\n',
                        Some(b'r') => '\r',
                        Some(b't') => '\t',
                        Some(b'b') => '\u{8}',
                        Some(b'f') => '\u{c}',
                        Some(b'u') => {
                            let hi = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&hi) {
                                if self.s.get(self.i..self.i + 2) != Some(b"\\u") {
                                    return Err("lone surrogate".into());
                                }
                                self.i += 2;
                                let lo = self.hex4()?;
                                0x10000 + ((hi - 0xD800) << 10) + (lo.wrapping_sub(0xDC00) & 0x3FF)
                            } else {
                                hi
                            };
                            char::from_u32(code).ok_or("bad code point")?
                        }
                        _ => return Err(format!("bad escape at byte {}", self.i)),
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                Some(&b) => {
                    out.push(b);
                    self.i += 1;
                }
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------
// Fixture loading.

fn oracle_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("oracle")
}

fn load(name: &str) -> Json {
    let path = oracle_dir().join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Reader::parse(text.trim_start_matches('\u{feff}')).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The test model, read from the TMSL `model.bim` the harness deploys to the engine.
fn load_model() -> SemanticModel {
    let bim = load("model.bim");
    let model = bim.get("model").unwrap_or(&Json::Null);
    let tables = model
        .arr("tables")
        .iter()
        .map(|t| Table {
            name: t.str("name").unwrap_or_default().into(),
            columns: t
                .arr("columns")
                .iter()
                .map(|c| {
                    let mut column = Column {
                        name: c.str("name").unwrap_or_default().into(),
                        data_type: DataType::from_tmsl(c.str("dataType").unwrap_or_default()),
                        expression: if c.str("type") == Some("calculated") {
                            c.str("expression").map(str::to_string)
                        } else {
                            None
                        },
                        source_column: c.str("sourceColumn").map(str::to_string),
                        ..Default::default()
                    };
                    // Auto date/time variations (`'Sales'[OrderDate].[Year]`).
                    let variations: Vec<Variation> = c
                        .arr("variations")
                        .iter()
                        .map(|v| {
                            let pair = |key: &str, second: &str| {
                                let o = v.get(key)?;
                                Some((o.str("table")?.to_string(), o.str(second)?.to_string()))
                            };
                            Variation {
                                name: v.str("name").unwrap_or_default().into(),
                                relationship: v.str("relationship").map(str::to_string),
                                default_hierarchy: pair("defaultHierarchy", "hierarchy"),
                                default_column: pair("defaultColumn", "column"),
                                is_default: v.bool("isDefault") == Some(true),
                            }
                        })
                        .collect();
                    column.set_variations(&variations);
                    column
                })
                .collect(),
            measures: t
                .arr("measures")
                .iter()
                .map(|m| Measure {
                    name: m.str("name").unwrap_or_default().into(),
                    expression: m.str("expression").unwrap_or_default().into(),
                    ..Default::default()
                })
                .collect(),
            partitions: t
                .arr("partitions")
                .iter()
                .map(|p| {
                    let src = p.get("source").unwrap_or(&Json::Null);
                    Partition {
                        name: p.str("name").unwrap_or_default().into(),
                        source_type: src.str("type").unwrap_or_default().into(),
                        expression: src.str("expression").map(str::to_string),
                        ..Default::default()
                    }
                })
                .collect(),
            ..Default::default()
        })
        .collect();
    let relationships = model
        .arr("relationships")
        .iter()
        .map(|r| Relationship {
            name: r.str("name").unwrap_or_default().into(),
            from_table: r.str("fromTable").unwrap_or_default().into(),
            from_column: r.str("fromColumn").unwrap_or_default().into(),
            to_table: r.str("toTable").unwrap_or_default().into(),
            to_column: r.str("toColumn").unwrap_or_default().into(),
            is_active: true,
            ..Default::default()
        })
        .collect();
    SemanticModel {
        name: bim.str("name").unwrap_or_default().into(),
        compatibility_level: Some(1567),
        tables,
        relationships,
        ..Default::default()
    }
}

struct Entry {
    id: String,
    category: String,
    role: String,
    table: Option<String>,
    expression: String,
    with: Vec<Json>,
}

fn load_corpus() -> Vec<Entry> {
    let Json::Arr(items) = load("corpus.json") else {
        panic!("corpus.json must be an array");
    };
    items
        .iter()
        .map(|e| Entry {
            id: e.str("id").unwrap_or_default().into(),
            category: e.str("category").unwrap_or_default().into(),
            role: e.str("role").unwrap_or_default().into(),
            table: e.str("table").map(str::to_string),
            expression: e.str("expression").unwrap_or_default().into(),
            with: e.arr("with").to_vec(),
        })
        .collect()
}

#[derive(Debug, Clone)]
struct Verdict {
    valid: bool,
    evaluates: Option<bool>,
    class: String,
}

// ------------------------------------------------------------------------------------------------
// Running bicraft on one entry, the way the harness places it in the model.

fn add_measure(m: &mut SemanticModel, table: &str, name: &str, expr: &str) {
    if let Some(t) = m.table_mut(table) {
        t.measures.push(Measure {
            name: name.into(),
            expression: expr.into(),
            ..Default::default()
        });
    }
}

fn add_column(m: &mut SemanticModel, table: &str, name: &str, expr: &str) {
    if let Some(t) = m.table_mut(table) {
        t.columns.push(Column {
            name: name.into(),
            expression: Some(expr.into()),
            ..Default::default()
        });
    }
}

#[allow(clippy::needless_update)] // `Context` may gain fields; keep their defaults.
fn context<'a>(model: &'a SemanticModel, table: Option<&'a str>) -> Context<'a> {
    Context {
        model: Some(model),
        table,
        ..Default::default()
    }
}

/// bicraft's verdict, the codes of its error diagnostics and those of its run-time warnings
/// (`runtime-*`, G20).
fn bicraft_full(base: &SemanticModel, e: &Entry) -> (bool, Vec<&'static str>, Vec<&'static str>) {
    let table = e.table.as_deref().unwrap_or("Sales");
    if e.role == "query" {
        let v = validate_query(&e.expression, &context(base, None));
        let (valid, codes) = summarize(&v);
        return (valid, codes, runtime_warnings(&v));
    }
    let m = probe_model(base, e, table);
    let (kind, home) = match e.role.as_str() {
        "measure" => (ExprKind::Measure, Some(table)),
        "column" => (ExprKind::CalculatedColumn, Some(table)),
        "rls" => (ExprKind::RowFilter, Some(table)),
        _ => (ExprKind::CalculatedTable, None),
    };
    let v = validate_as(&e.expression, &context(&m, home), kind);
    let warnings = runtime_warnings(&v);
    let (mut valid, mut codes) = summarize(&v);
    // Circular references are a model-level property, as they are for the engine.
    for (_, d) in deps::cycle_diagnostics(&m) {
        if d.severity == Severity::Error {
            valid = false;
            if !codes.contains(&d.code) {
                codes.push(d.code);
            }
        }
    }
    (valid, codes, warnings)
}

fn runtime_warnings(v: &Validation) -> Vec<&'static str> {
    let mut codes: Vec<&'static str> = Vec::new();
    for d in &v.diagnostics {
        if d.severity == Severity::Warning && d.code.starts_with("runtime-") && !codes.contains(&d.code) {
            codes.push(d.code);
        }
    }
    codes
}

/// The oracle model with the entry (and the objects it comes `with`) added.
fn probe_model(base: &SemanticModel, e: &Entry, table: &str) -> SemanticModel {
    let mut m = base.clone();
    match e.role.as_str() {
        "measure" => add_measure(&mut m, table, "Probe", &e.expression),
        "column" => add_column(&mut m, table, "Probe", &e.expression),
        "table" => m.tables.push(Table {
            name: "Probe Table".into(),
            partitions: vec![Partition {
                name: "Probe Table".into(),
                source_type: "calculated".into(),
                expression: Some(e.expression.clone()),
                ..Default::default()
            }],
            ..Default::default()
        }),
        "rls" => m.roles.push(Role {
            name: "Probe Role".into(),
            model_permission: Some("read".into()),
            table_permissions: vec![TablePermission {
                table: table.into(),
                filter_expression: Some(e.expression.clone()),
                ..Default::default()
            }],
            ..Default::default()
        }),
        other => panic!("{}: unknown role {other}", e.id),
    }
    for w in &e.with {
        let (t, n, x) = (
            w.str("table").unwrap_or_default(),
            w.str("name").unwrap_or_default(),
            w.str("expression").unwrap_or_default(),
        );
        if w.str("kind") == Some("measure") {
            add_measure(&mut m, t, n, x);
        } else {
            add_column(&mut m, t, n, x);
        }
    }
    m
}

fn summarize(v: &Validation) -> (bool, Vec<&'static str>) {
    let mut codes: Vec<&'static str> = Vec::new();
    for d in v.diagnostics.iter().filter(|d| d.severity == Severity::Error) {
        if !codes.contains(&d.code) {
            codes.push(d.code);
        }
    }
    (v.valid, codes)
}

// ------------------------------------------------------------------------------------------------

#[derive(Default, Clone, Copy)]
struct Tally {
    total: usize,
    agree: usize,
    false_accept: usize,
    false_reject: usize,
}

impl Tally {
    fn add(&mut self, ours: bool, engine: bool) {
        self.total += 1;
        match (ours, engine) {
            (a, b) if a == b => self.agree += 1,
            (true, false) => self.false_accept += 1,
            _ => self.false_reject += 1,
        }
    }
    fn rate(&self) -> f64 {
        if self.total == 0 {
            1.0
        } else {
            self.agree as f64 / self.total as f64
        }
    }
    fn line(&self, name: &str) -> String {
        format!(
            "{name:<20} {:>4} {:>6.1}%  false accepts {:>3}  false rejects {:>3}",
            self.total,
            100.0 * self.rate(),
            self.false_accept,
            self.false_reject
        )
    }
}

#[test]
fn corpus_is_well_formed() {
    let corpus = load_corpus();
    assert!(corpus.len() >= 300, "corpus has {} entries", corpus.len());
    let mut ids = std::collections::HashSet::new();
    for e in &corpus {
        assert!(ids.insert(e.id.clone()), "duplicate id {}", e.id);
        assert!(
            ["measure", "column", "table", "rls", "query"].contains(&e.role.as_str()),
            "{}: role {}",
            e.id,
            e.role
        );
        if matches!(e.role.as_str(), "measure" | "column" | "rls") {
            assert!(e.table.is_some(), "{}: needs a table", e.id);
        }
    }
    // The model the harness deploys must itself be valid for bicraft.
    let m = load_model();
    for t in &m.tables {
        for ms in &t.measures {
            let v = validate_as(&ms.expression, &context(&m, Some(&t.name)), ExprKind::Measure);
            assert!(v.valid, "{}[{}]: {:?}", t.name, ms.name, v.diagnostics);
        }
    }
}

#[test]
// `MAX_FALSE_REJECTS` is 0 today; the comparison stays so the knob can be loosened.
#[allow(clippy::absurd_extreme_comparisons)]
fn agreement_with_engine() {
    let corpus = load_corpus();
    let verdicts_doc = load("verdicts.json");
    let mut verdicts = BTreeMap::new();
    for v in verdicts_doc.arr("entries") {
        let (Some(id), Some(valid)) = (v.str("id"), v.bool("valid")) else {
            continue;
        };
        verdicts.insert(
            id.to_string(),
            Verdict {
                valid,
                evaluates: v.bool("evaluates"),
                class: v.str("class").unwrap_or_default().into(),
            },
        );
    }
    if verdicts.is_empty() {
        eprintln!(
            "note: oracle/verdicts.json has no recorded verdicts; they come from the maintainers' development oracles (docs/dax-oracle.md)"
        );
        return;
    }
    let base = load_model();

    let mut overall = Tally::default();
    let mut with_eval = Tally::default();
    let mut by_cat: BTreeMap<String, Tally> = BTreeMap::new();
    let mut by_role: BTreeMap<String, Tally> = BTreeMap::new();
    let mut by_class: BTreeMap<String, Tally> = BTreeMap::new();
    let mut missing = Vec::new();
    let mut disagreements = Vec::new();
    // G20: run-time errors (definition valid, evaluation fails) bicraft flags with a warning, and
    // warnings on entries that evaluate fine.
    let (mut runtime_total, mut runtime_flagged) = (0usize, 0usize);
    let mut runtime_missed: Vec<&Entry> = Vec::new();
    let mut false_warnings: Vec<(&Entry, Vec<&'static str>)> = Vec::new();
    for e in &corpus {
        let Some(v) = verdicts.get(&e.id) else {
            missing.push(e.id.clone());
            continue;
        };
        let (ours, codes, warnings) = bicraft_full(&base, e);
        if v.valid && v.evaluates == Some(false) {
            runtime_total += 1;
            if warnings.is_empty() && ours {
                runtime_missed.push(e);
            } else {
                runtime_flagged += 1;
            }
        }
        // A query that runs may still define objects that would fail if it used them: its
        // verdict says nothing about those, so queries are left out here.
        if v.valid && v.evaluates == Some(true) && !warnings.is_empty() && e.role != "query" {
            false_warnings.push((e, warnings.clone()));
        }
        overall.add(ours, v.valid);
        with_eval.add(ours, v.valid && v.evaluates != Some(false));
        by_cat.entry(e.category.clone()).or_default().add(ours, v.valid);
        by_role.entry(e.role.clone()).or_default().add(ours, v.valid);
        if !v.valid {
            by_class.entry(v.class.clone()).or_default().add(ours, v.valid);
        }
        if ours != v.valid {
            disagreements.push((e, v.clone(), codes));
        }
    }

    let mut out = String::new();
    let _ = writeln!(out, "DAX oracle: bicraft vs the Analysis Services engine");
    let _ = writeln!(out, "{}", overall.line("definitions"));
    let _ = writeln!(out, "{}", with_eval.line("incl. evaluation"));
    let _ = writeln!(
        out,
        "run-time errors flagged (warning or error): {runtime_flagged} of {runtime_total};          run-time warnings on entries that evaluate: {}",
        false_warnings.len()
    );
    let _ = writeln!(out, "-- by role");
    for (k, t) in &by_role {
        let _ = writeln!(out, "{}", t.line(k));
    }
    let _ = writeln!(out, "-- by category");
    for (k, t) in &by_cat {
        let _ = writeln!(out, "{}", t.line(k));
    }
    let _ = writeln!(
        out,
        "-- engine rejections by error class (agree = bicraft rejects too)"
    );
    for (k, t) in &by_class {
        let _ = writeln!(out, "{}", t.line(k));
    }
    for e in &runtime_missed {
        let _ = writeln!(
            out,
            "run-time error not flagged: {:<24} {}",
            e.id,
            e.expression.replace('\n', " ")
        );
    }
    for (e, w) in &false_warnings {
        let _ = writeln!(
            out,
            "warning on an entry that evaluates: {:<24} {:<30} {}",
            e.id,
            w.join(","),
            e.expression.replace('\n', " ")
        );
    }
    let _ = writeln!(out, "-- disagreements ({})", disagreements.len());
    for (e, v, codes) in &disagreements {
        let kind = if v.valid { "false reject" } else { "false accept" };
        let _ = writeln!(
            out,
            "{:<24} {kind:<13} engine={:<22} bicraft={:<30} {}",
            e.id,
            v.class,
            codes.join(","),
            e.expression.replace('\n', " ")
        );
    }
    println!("{out}");

    if let Ok(path) = bicraft_env::var("BICRAFT_ORACLE_REPORT") {
        let mut md = String::from(
            "| Id | Role | Kind | Engine class | bicraft errors | Expression |\n|---|---|---|---|---|---|\n",
        );
        for (e, v, codes) in &disagreements {
            let kind = if v.valid { "false reject" } else { "false accept" };
            let role = match &e.table {
                Some(t) if e.role != "table" && e.role != "query" => format!("{} ({t})", e.role),
                _ => e.role.clone(),
            };
            let _ = writeln!(
                md,
                "| {} | {role} | {kind} | {} | {} | `{}` |",
                e.id,
                v.class,
                if codes.is_empty() {
                    "-".into()
                } else {
                    codes.join(", ")
                },
                e.expression
                    .replace('\n', " ")
                    .replace('|', "\\|")
                    .replace('`', "'")
            );
        }
        std::fs::write(&path, md).unwrap_or_else(|err| panic!("{path}: {err}"));
    }

    assert!(
        missing.is_empty(),
        "corpus entries without a recorded verdict (re-run the harness): {missing:?}"
    );
    assert!(
        overall.false_reject <= MAX_FALSE_REJECTS,
        "{} false rejects (at most {MAX_FALSE_REJECTS} allowed)",
        overall.false_reject
    );
    // G20: a run-time warning must point at a real run-time failure.
    assert!(
        false_warnings.is_empty(),
        "{} run-time warnings on entries the engine evaluates without error",
        false_warnings.len()
    );
    assert!(
        overall.rate() >= AGREEMENT_FLOOR,
        "agreement {:.1}% fell below the floor {:.1}%",
        100.0 * overall.rate(),
        100.0 * AGREEMENT_FLOOR
    );
}

/// Every DAX object of the model: (expression, home table, kind).
fn dax_objects(m: &SemanticModel) -> Vec<(&str, Option<&str>, ExprKind)> {
    let mut out = Vec::new();
    for t in &m.tables {
        for x in &t.measures {
            out.push((x.expression.as_str(), Some(t.name.as_str()), ExprKind::Measure));
        }
        for c in &t.columns {
            if let Some(e) = c.expression.as_deref() {
                out.push((e, Some(t.name.as_str()), ExprKind::CalculatedColumn));
            }
        }
        if let Some(e) = deps::calculated_table_expression(t) {
            out.push((e, None, ExprKind::CalculatedTable));
        }
    }
    for r in &m.roles {
        for tp in &r.table_permissions {
            if let Some(e) = tp.filter_expression.as_deref() {
                out.push((e, Some(tp.table.as_str()), ExprKind::RowFilter));
            }
        }
    }
    out
}

/// The whole-model [`Validator`] (shared name index and type cache) gives exactly what
/// per-expression `validate_as` gives, on the oracle model with every corpus entry placed in it.
#[test]
fn validator_matches_per_expression_validation() {
    let base = load_model();
    let mut compared = 0usize;
    for e in load_corpus().iter().filter(|e| e.role != "query") {
        let m = probe_model(&base, e, e.table.as_deref().unwrap_or("Sales"));
        let objects = dax_objects(&m);
        let mut validator = Validator::new(&m);
        for &(text, home, kind) in objects.iter().rev() {
            let old = validate_as(text, &context(&m, home), kind);
            let new = validator.validate_as(text, home, kind);
            assert_eq!(old.diagnostics, new.diagnostics, "{}: {text}", e.id);
            assert_eq!(old.valid, new.valid, "{}: {text}", e.id);
            assert_eq!(old.result_type, new.result_type, "{}: {text}", e.id);
            compared += 1;
        }
    }
    assert!(compared > 1000, "only {compared} objects compared");
}
