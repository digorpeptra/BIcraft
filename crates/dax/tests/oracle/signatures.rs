//! Signature corpus (M11.40): every function of BIcraft's DAX catalogue, probed on Microsoft's
//! engine with too few, too many and wrongly typed arguments.
//!
//! `signatures.json` holds, for each function, the minimum and maximum argument counts the engine
//! reports, and a list of probe expressions with the engine's verdict on each (accepted or not,
//! with our error class only; the engine's messages are never committed). The verdicts were
//! recorded during development with the maintainers' development oracles, kept outside this
//! repository (docs/dax-oracle.md, "Signature corpus"). These tests replay them against the
//! catalogue and the binder on any platform.
//!
//! The probes come from [`probes`], derived from the catalogue as it stood when they were
//! recorded; `BICRAFT_SIGNATURE_PROBES=<file>` writes the current list for a new recording. The
//! recorded list is the reference and does not change when the catalogue does.

use super::*;

use bicraft_dax::types::{ParamKind, Returns, ScalarType, Signature, ValueType};
use bicraft_dax::{all_functions, lookup_function};

/// Where the catalogue's argument counts deliberately differ from the engine's, and why. A listed
/// function that matches the engine fails the test, so the list stays exact.
const ARITY_DEVIATIONS: &[(&str, &str)] = &[
    // `TRUE(1)` and `FALSE(1)` are syntax errors: the engine reports no count, so its answer
    // reads "no maximum"; the reference documents no argument.
    ("TRUE", "keyword function, no argument"),
    ("FALSE", "keyword function, no argument"),
    // `NOT` is an operator as well; `NOT()` parses as the operator with nothing after it.
    ("NOT", "operator form, one argument"),
    // The engine ignores extra arguments; the reference documents none.
    ("USERNAME", "documented with no argument"),
    // The engine binds RANK's arguments by shape and reports unused ones, not a count.
    ("RANK", "seven documented parameters"),
];

/// Probes on which the binder and the engine disagree, each with the reason it is accepted for
/// now. An exact id must disagree, and a pattern (`FUNCTION/*`) must cover at least one
/// disagreement; otherwise the test fails, so the list stays exact.
const PROBE_DEVIATIONS: &[(&str, &str)] = &[
    // Not modelled yet (docs/gaps.md G19).
    (
        "ADDMISSINGITEMS/*",
        "its show-all, group-by and filter sections are not bound by shape",
    ),
    (
        "SAMPLEAXISWITHLOCALMINMAX/*",
        "the probes' axis is a constant where the engine needs a numeric column; its second section is not modelled",
    ),
    (
        "TOPNSKIP/3-table",
        "its trailing `ties` keeps the order-by section from being bound by shape",
    ),
    (
        "TOPNSKIP/4-text",
        "its trailing `ties` keeps the order-by section from being bound by shape",
    ),
    ("ISAFTER/many", "an unpaired trailing value is not reported"),
    ("ISONORAFTER/many", "an unpaired trailing value is not reported"),
    (
        "RANKX/min",
        "the omitted value evaluates the expression outside the row context; not reported",
    ),
    (
        "ISSUBTOTAL/min",
        "a column read outside SUMMARIZE's row context is not reported",
    ),
    (
        "TEXTCONTAINS/min",
        "the text column is read as a value, which needs a row context; not reported",
    ),
    (
        "TEXTCONTAINS/max",
        "the text column is read as a value, which needs a row context; not reported",
    ),
    (
        "TEXTSIMILARITY/min",
        "the text column is read as a value, which needs a row context; not reported",
    ),
    (
        "TEXTSIMILARITY/max",
        "the text column is read as a value, which needs a row context; not reported",
    ),
    (
        "PATH/min",
        "the engine refuses the same column as child and parent; not modelled",
    ),
    (
        "TOCSV/1-text",
        "the engine fails on a text max_rows when the measure is defined; BIcraft warns (runtime-conversion)",
    ),
    (
        "TOJSON/1-text",
        "the engine fails on a text max_rows when the measure is defined; BIcraft warns (runtime-conversion)",
    ),
    // The engine ignores USERNAME's extra arguments (see ARITY_DEVIATIONS).
    ("USERNAME/many", "documented with no argument"),
    // The probe environment, not the function.
    (
        "EXTERNALMEASURE/*",
        "the engine resolves EXTERNALMEASURE only in a composite model over a remote model",
    ),
    (
        "INFO.CALENDARCOLUMNGROUPS/*",
        "the probing engine instance refuses this rowset as unavailable in its edition",
    ),
    (
        "INFO.CALENDARCOLUMNREFERENCES/*",
        "the probing engine instance refuses this rowset as unavailable in its edition",
    ),
    (
        "INFO.CALENDARS/*",
        "the probing engine instance refuses this rowset as unavailable in its edition",
    ),
    (
        "INFO.DATACOVERAGEDEFINITIONS/*",
        "the probing engine instance refuses this rowset as unavailable in its edition",
    ),
    (
        "INFO.EXCLUDEDARTIFACTS/*",
        "the probing engine instance refuses this rowset as unavailable in its edition",
    ),
    (
        "INFO.USERDEFINEDFUNCTIONS/*",
        "the probing engine instance refuses this rowset as unavailable in its edition",
    ),
    // Each rowset has its own restriction names, which the catalogue does not list; the probes'
    // "ID" is not one of these rowsets'.
    (
        "INFO.CALCDEPENDENCY/max",
        "restriction names are not in the catalogue",
    ),
    ("INFO.CATALOGS/max", "restriction names are not in the catalogue"),
    (
        "INFO.CSDLMETADATA/max",
        "restriction names are not in the catalogue",
    ),
    (
        "INFO.DEPENDENCIES/max",
        "restriction names are not in the catalogue",
    ),
    ("INFO.FUNCTIONS/max", "restriction names are not in the catalogue"),
    ("INFO.MODEL/max", "restriction names are not in the catalogue"),
    (
        "INFO.PROPERTIES/max",
        "restriction names are not in the catalogue",
    ),
    (
        "INFO.STORAGETABLECOLUMNS/max",
        "restriction names are not in the catalogue",
    ),
    (
        "INFO.STORAGETABLECOLUMNSEGMENTS/max",
        "restriction names are not in the catalogue",
    ),
    (
        "INFO.STORAGETABLES/max",
        "restriction names are not in the catalogue",
    ),
];

/// Whether a deviation entry covers a probe id: the same id, or `PREFIX*`.
fn covers(entry: &str, id: &str) -> bool {
    match entry.strip_suffix('*') {
        Some(prefix) => id.starts_with(prefix),
        None => entry == id,
    }
}

// ------------------------------------------------------------------------------------------------
// Probe generation (authoring aid; the recorded list in signatures.json is the reference).

/// One probe: a measure on `Sales` (or a query, for the query-only `INFO.*` functions).
struct Probe {
    id: String,
    function: &'static str,
    role: &'static str,
    expression: String,
}

/// The canonical (well-typed) argument for a parameter kind, in the test model.
fn canonical(kind: ParamKind, index: usize) -> String {
    match kind {
        ParamKind::Scalar(Some(ScalarType::String)) => "\"a\"".into(),
        ParamKind::Scalar(Some(ScalarType::Boolean)) => "TRUE()".into(),
        ParamKind::Scalar(Some(ScalarType::DateTime)) => "DATE(2020, 1, 1)".into(),
        ParamKind::Scalar(_) | ParamKind::Any | ParamKind::Calendar => "1".into(),
        ParamKind::Table | ParamKind::TableRef => "'Sales'".into(),
        // A relation without duplicate rows, holding the columns the clauses below name.
        ParamKind::Relation => RELATION.into(),
        ParamKind::ColumnRef
        | ParamKind::TableOrColumnRef
        | ParamKind::TableOrColumn
        | ParamKind::RowExpression => "'Sales'[Quantity]".into(),
        ParamKind::Filter => "'Sales'[Quantity] > 1".into(),
        ParamKind::NewColumnName => format!("\"N{index}\""),
        ParamKind::Keyword(set) | ParamKind::KeywordOrScalar(set) => {
            set.first().copied().unwrap_or("ASC").into()
        }
        ParamKind::Dates => "'Date'[Date]".into(),
        ParamKind::MeasureRef => "[Total Sales]".into(),
        ParamKind::OrderBy => "ORDERBY('Sales'[SalesKey])".into(),
        ParamKind::PartitionBy => "PARTITIONBY('Sales'[Channel])".into(),
        ParamKind::MatchBy => "MATCHBY('Sales'[SalesKey])".into(),
    }
}

/// Wrong-kind arguments for a parameter kind: (tag, argument).
fn wrong(kind: ParamKind) -> Vec<(&'static str, &'static str)> {
    match kind {
        ParamKind::Scalar(Some(ScalarType::Integer | ScalarType::Decimal | ScalarType::Currency)) => {
            vec![("table", "'Product'"), ("text", "\"a\"")]
        }
        ParamKind::Scalar(Some(ScalarType::DateTime)) => vec![("table", "'Product'"), ("text", "\"a\"")],
        ParamKind::Scalar(_) => vec![("table", "'Product'")],
        ParamKind::Table | ParamKind::Relation => vec![("scalar", "1")],
        ParamKind::TableRef => vec![("scalar", "1"), ("expr", "FILTER('Sales', TRUE())")],
        ParamKind::ColumnRef => vec![
            ("scalar", "1"),
            ("table", "'Sales'"),
            ("measure", "[Total Sales]"),
        ],
        ParamKind::TableOrColumnRef | ParamKind::TableOrColumn => vec![("scalar", "1")],
        ParamKind::RowExpression => vec![("table", "'Product'")],
        ParamKind::Filter => vec![("text", "\"a\"")],
        ParamKind::NewColumnName => vec![("scalar", "1")],
        ParamKind::Keyword(_) => vec![("scalar", "1"), ("text", "\"a\"")],
        ParamKind::KeywordOrScalar(_) => vec![("text", "\"a\"")],
        ParamKind::Dates | ParamKind::MeasureRef => vec![("scalar", "1")],
        ParamKind::OrderBy | ParamKind::PartitionBy | ParamKind::MatchBy => vec![("scalar", "1")],
        ParamKind::Any | ParamKind::Calendar => Vec::new(),
    }
}

const RELATION: &str = "ALL('Sales'[SalesKey], 'Sales'[Channel], 'Sales'[Quantity])";

const MODIFIERS: &[&str] = &[
    "KEEPFILTERS",
    "REMOVEFILTERS",
    "ALLCROSSFILTERED",
    "CROSSFILTER",
    "USERELATIONSHIP",
];

fn returns_table(sig: &Signature) -> bool {
    match sig.returns {
        Returns::Fixed(ValueType::Table) => true,
        Returns::SameAsArg(i) => sig
            .params
            .get(usize::from(i))
            .is_some_and(|p| matches!(p.kind, ParamKind::Table | ParamKind::TableRef)),
        _ => false,
    }
}

/// Places a call where the function can stand: the role and the probe's text.
fn place(sig: &Signature, call: &str) -> (&'static str, String) {
    let n = sig.name;
    if bicraft_dax::is_query_only_info(n) {
        return ("query", format!("EVALUATE {call}"));
    }
    let expr = if MODIFIERS.contains(&n) {
        format!("CALCULATE(1, {call})")
    } else if n == "ORDERBY" {
        format!("COUNTROWS(INDEX(1, {RELATION}, {call}))")
    } else if n == "PARTITIONBY" {
        format!("COUNTROWS(INDEX(1, {RELATION}, ORDERBY('Sales'[SalesKey]), , {call}))")
    } else if n == "MATCHBY" {
        format!("COUNTROWS(INDEX(1, {RELATION}, ORDERBY('Sales'[SalesKey]), , , {call}))")
    } else if matches!(n, "ROLLUP" | "ROLLUPGROUP") {
        format!("COUNTROWS(SUMMARIZE('Sales', {call}))")
    } else if returns_table(sig) {
        format!("COUNTROWS({call})")
    } else {
        call.to_string()
    };
    ("measure", expr)
}

/// The parameters of the shortest well-formed call: every required one (a mandatory repeating
/// unit once), then optional ones in order up to the signature's minimum.
fn min_params(sig: &Signature) -> Vec<usize> {
    let mut keep: Vec<usize> = (0..sig.params.len())
        .filter(|&i| sig.params.get(i).is_some_and(|p| !p.optional))
        .collect();
    let min = usize::from(sig.min_args.unwrap_or(0));
    let mut i = 0;
    while keep.len() < min && i < sig.params.len() {
        if !keep.contains(&i) {
            keep.push(i);
            keep.sort_unstable();
        }
        i += 1;
    }
    keep
}

fn call(name: &str, args: &[String]) -> String {
    format!("{name}({})", args.join(", "))
}

fn probes() -> Vec<Probe> {
    let mut out = Vec::new();
    for sig in all_functions() {
        let n = sig.name;
        let mut push = |id: String, call: String| {
            let (role, expression) = place(sig, &call);
            out.push(Probe {
                id,
                function: n,
                role,
                expression,
            });
        };
        // Count probes: the engine names its minimum and maximum argument counts.
        push(format!("{n}/none"), call(n, &[]));
        push(format!("{n}/many"), call(n, &vec!["1".to_string(); 40]));

        let every: Vec<String> = sig
            .params
            .iter()
            .enumerate()
            .map(|(i, p)| match p.name {
                "reset" => "NONE".to_string(),
                "object" | "reference" => "'Sales'[Quantity]".to_string(),
                "restriction_name" => "\"ID\"".to_string(),
                _ => canonical(p.kind, i),
            })
            .collect();
        // A window function's `reset` is probed on its own: with ORDERBY or PARTITIONBY it is
        // refused outside visual calculations, which would hide every other kind probe.
        let has_reset = sig.params.last().is_some_and(|p| p.name == "reset");
        let mut full = every.clone();
        if has_reset {
            full.pop();
        }
        let short: Vec<String> = min_params(sig)
            .iter()
            .filter_map(|&i| full.get(i).cloned())
            .collect();
        if !short.is_empty() {
            push(format!("{n}/min"), call(n, &short));
            if short.len() > 1 {
                let mut below = short.clone();
                below.pop();
                push(format!("{n}/below"), call(n, &below));
            }
        }
        if full.len() > short.len() {
            push(format!("{n}/max"), call(n, &full));
        }
        if has_reset {
            push(format!("{n}/reset"), call(n, &every));
        }
        let finite = lookup_function(n).and_then(|a| a.max).is_some();
        if finite && !every.is_empty() {
            let mut above = every.clone();
            above.push("1".into());
            push(format!("{n}/above"), call(n, &above));
        }
        // Kind probes: one wrongly typed argument in the full call.
        for (i, p) in sig.params.iter().enumerate().take(full.len()) {
            for (tag, arg) in wrong(p.kind) {
                let mut args = full.clone();
                if let Some(slot) = args.get_mut(i) {
                    *slot = arg.to_string();
                }
                push(format!("{n}/{i}-{tag}"), call(n, &args));
            }
        }
    }
    out
}

fn json_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

// ------------------------------------------------------------------------------------------------
// The recorded corpus.

fn count(v: Option<&Json>) -> Option<u8> {
    match v {
        Some(Json::Num(n)) if n.fract() == 0.0 && (0.0..=255.0).contains(n) => Some(*n as u8),
        _ => None,
    }
}

/// (name, engine minimum, engine maximum: `None` when 40 arguments raised no count error).
fn recorded_counts(doc: &Json) -> Vec<(String, u8, Option<u8>)> {
    doc.arr("functions")
        .iter()
        .map(|f| {
            let name = f.str("name").unwrap_or_default().to_string();
            let min = count(f.get("min")).unwrap_or_else(|| panic!("{name}: min"));
            (name, min, count(f.get("max")))
        })
        .collect()
}

#[test]
fn signature_corpus_is_well_formed() {
    let doc = load("signatures.json");
    for key in ["product", "version", "analysis_services"] {
        assert!(
            doc.get("engine")
                .and_then(|e| e.str(key))
                .is_some_and(|s| !s.is_empty()),
            "engine.{key}"
        );
    }
    assert!(
        doc.str("recorded").is_some_and(|s| s.len() == 10),
        "recorded date"
    );
    assert!(doc.str("method").is_some_and(|s| !s.is_empty()), "method");
    let mut ids: Vec<&str> = Vec::new();
    for p in doc.arr("probes") {
        let id = p.str("id").unwrap_or_default();
        assert!(id.contains('/'), "probe id {id}");
        assert!(matches!(p.str("role"), Some("measure" | "query")), "{id}: role");
        assert!(
            p.str("expression").is_some_and(|e| !e.is_empty()),
            "{id}: expression"
        );
        assert!(p.bool("valid").is_some(), "{id}: valid");
        assert!(p.str("class").is_some(), "{id}: class");
        ids.push(id);
    }
    let n = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), n, "duplicate probe ids");
}

#[test]
fn catalogue_arity_matches_the_engine() {
    let doc = load("signatures.json");
    let counts = recorded_counts(&doc);
    let mut problems = Vec::new();
    for (name, min, max) in &counts {
        let Some(a) = lookup_function(name) else {
            problems.push(format!("{name}: probed but not in the catalogue"));
            continue;
        };
        let deviates = ARITY_DEVIATIONS.iter().any(|(d, _)| d == name);
        let agrees = (a.min, a.max) == (*min, *max);
        if !agrees && !deviates {
            problems.push(format!(
                "{name}: catalogue {}..{:?}, engine {min}..{max:?}",
                a.min, a.max
            ));
        }
        if agrees && deviates {
            problems.push(format!("{name}: listed as a deviation but matches the engine"));
        }
    }
    for s in all_functions() {
        if !counts.iter().any(|(n, ..)| n == s.name) {
            problems.push(format!("{}: in the catalogue but not probed", s.name));
        }
    }
    for (d, _) in ARITY_DEVIATIONS {
        if !counts.iter().any(|(n, ..)| n == d) {
            problems.push(format!("{d}: deviation for a function not probed"));
        }
    }
    println!(
        "signature corpus: {} functions, {} agree on argument counts, {} justified deviations",
        counts.len(),
        counts.len() - ARITY_DEVIATIONS.len(),
        ARITY_DEVIATIONS.len()
    );
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn binder_matches_the_engine_on_signature_probes() {
    let doc = load("signatures.json");
    let base = load_model();
    let mut tally = Tally::default();
    let mut problems = Vec::new();
    let mut agreeing_functions: BTreeMap<String, bool> = BTreeMap::new();
    let mut used: Vec<usize> = Vec::new();
    for p in doc.arr("probes") {
        let id = p.str("id").unwrap_or_default();
        let engine_valid = p.bool("valid").unwrap_or(false);
        let entry = Entry {
            id: id.to_string(),
            category: "signatures".into(),
            role: p.str("role").unwrap_or("measure").to_string(),
            table: Some("Sales".into()),
            expression: p.str("expression").unwrap_or_default().to_string(),
            with: Vec::new(),
        };
        let (ours, codes, warnings) = bicraft_full(&base, &entry);
        tally.add(ours, engine_valid);
        let mut disagreement = None;
        if ours != engine_valid {
            disagreement = Some(format!(
                "{id}: bicraft {}, engine {} ({}); codes {codes:?}  {}",
                if ours { "valid" } else { "invalid" },
                if engine_valid { "valid" } else { "invalid" },
                p.str("class").unwrap_or_default(),
                entry.expression
            ));
        } else if p.bool("evaluates") == Some(true) && !warnings.is_empty() {
            disagreement = Some(format!(
                "{id}: evaluates on the engine, bicraft warns {warnings:?}  {}",
                entry.expression
            ));
        }
        let function = id.split('/').next().unwrap_or_default().to_string();
        let ok = agreeing_functions.entry(function).or_insert(true);
        let listed: Vec<usize> = (0..PROBE_DEVIATIONS.len())
            .filter(|&k| PROBE_DEVIATIONS.get(k).is_some_and(|(d, _)| covers(d, id)))
            .collect();
        match disagreement {
            Some(d) => {
                *ok = false;
                if listed.is_empty() {
                    problems.push(d);
                }
                used.extend(listed);
            }
            None => {
                // An exact entry must disagree; a pattern may also cover agreeing probes.
                for k in listed {
                    if let Some((d, _)) = PROBE_DEVIATIONS.get(k)
                        && !d.ends_with('*')
                    {
                        problems.push(format!("{id}: listed as a deviation but agrees"));
                    }
                }
            }
        }
    }
    for (k, (d, _)) in PROBE_DEVIATIONS.iter().enumerate() {
        if !used.contains(&k) {
            problems.push(format!("{d}: deviation that covers no disagreement"));
        }
    }
    println!("{}", tally.line("signature probes"));
    println!(
        "justified deviations: {} entries covering {} probes",
        PROBE_DEVIATIONS.len(),
        doc.arr("probes")
            .iter()
            .filter(|p| PROBE_DEVIATIONS
                .iter()
                .any(|(d, _)| covers(d, p.str("id").unwrap_or_default())))
            .count()
    );
    println!(
        "functions whose probes all agree: {} / {}",
        agreeing_functions.values().filter(|ok| **ok).count(),
        agreeing_functions.len()
    );
    assert!(
        problems.is_empty(),
        "{} disagreements:\n{}",
        problems.len(),
        problems.join("\n")
    );
}

/// Writes the probe list for a new recording when `BICRAFT_SIGNATURE_PROBES` names a file.
#[test]
fn write_signature_probes_on_request() {
    let Some(path) = std::env::var_os("BICRAFT_SIGNATURE_PROBES") else {
        return;
    };
    let probes = probes();
    let mut text = String::from("[\n");
    for (k, p) in probes.iter().enumerate() {
        let sep = if k + 1 == probes.len() { "" } else { "," };
        let _ = writeln!(
            text,
            "  {{\"id\": {}, \"function\": {}, \"role\": {}, \"expression\": {}}}{sep}",
            json_str(&p.id),
            json_str(p.function),
            json_str(p.role),
            json_str(&p.expression)
        );
    }
    text.push_str("]\n");
    std::fs::write(&path, text).unwrap_or_else(|e| panic!("{}: {e}", PathBuf::from(&path).display()));
    println!("{} probes written", probes.len());
}
