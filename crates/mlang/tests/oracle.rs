//! Signature corpus for M (M11.40): BIcraft's catalogue of M functions against the signatures
//! Power BI Desktop's M engine reports for the same functions.
//!
//! `oracle/m_signatures.json` holds, for each function of the catalogue, what Desktop's engine
//! answered through the public M library (`Type.FunctionParameters`,
//! `Type.FunctionRequiredParameters`, `Type.FunctionReturn`): the required and total parameter
//! counts, each parameter's name and type and the return type, as BIcraft's type codes (a
//! primitive kind, `?` when nullable; `any?` is M's `any`, `any` is `anynonnull`). It also lists
//! the library functions Desktop has that BIcraft lacks, with their counts. The recording method
//! is in docs/dax-oracle.md "M signatures" and oracle/SOURCE.txt; these tests need neither
//! Windows nor Desktop.

use std::collections::BTreeMap;

use serde_json::Value;

/// Where the catalogue deliberately differs from Desktop's engine, and why. A listed function that
/// matches fails the test, so the list stays exact.
const DEVIATIONS: &[(&str, &str)] = &[
    (
        "Error.Record",
        "the parameters and errorCode arguments are not evaluated yet (docs/gaps.md G108)",
    ),
    (
        "Splitter.SplitTextByDelimiter",
        "the csvStyle argument is not evaluated yet (docs/gaps.md G108)",
    ),
];

fn corpus() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/oracle/m_signatures.json");
    let text = std::fs::read_to_string(path).unwrap();
    serde_json::from_str(&text).unwrap()
}

/// One function as the corpus records it.
#[derive(Debug, PartialEq)]
struct Sig {
    min: usize,
    max: usize,
    params: Vec<String>,
    names: Vec<String>,
    returns: String,
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect()
}

fn recorded(doc: &Value) -> BTreeMap<String, Sig> {
    doc["functions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            let sig = Sig {
                min: f["min"].as_u64().unwrap() as usize,
                max: f["max"].as_u64().unwrap() as usize,
                params: strings(&f["params"]),
                names: strings(&f["names"]),
                returns: f["returns"].as_str().unwrap().to_string(),
            };
            (f["name"].as_str().unwrap().to_string(), sig)
        })
        .collect()
}

/// An M type as the catalogue writes it (`nullable text`, `anynonnull`) as a corpus code.
fn code(ty: &str) -> String {
    match ty {
        "any" => "any?".into(),
        "anynonnull" => "any".into(),
        "null" | "none" => ty.into(),
        t => match t.strip_prefix("nullable ") {
            Some(k) => format!("{k}?"),
            None => t.into(),
        },
    }
}

fn catalogue(f: &bicraft_mlang::Function) -> Sig {
    Sig {
        min: f.required(),
        max: f.params.len(),
        params: f.params.iter().map(|p| code(p.ty)).collect(),
        names: f.params.iter().map(|p| p.name.to_string()).collect(),
        returns: code(f.returns),
    }
}

const KINDS: &[&str] = &[
    "any",
    "null",
    "none",
    "number",
    "text",
    "logical",
    "date",
    "datetime",
    "datetimezone",
    "time",
    "duration",
    "binary",
    "list",
    "record",
    "table",
    "function",
    "type",
];

#[test]
fn the_corpus_is_well_formed() {
    let doc = corpus();
    assert_eq!(doc["engine"]["version"], "2.159.1231.0");
    let all = recorded(&doc);
    assert_eq!(
        all.len(),
        doc["functions"].as_array().unwrap().len(),
        "duplicate names"
    );
    for (name, s) in &all {
        assert!(s.min <= s.max, "{name}");
        assert_eq!(s.params.len(), s.max, "{name}");
        assert_eq!(s.names.len(), s.max, "{name}");
        for c in s.params.iter().chain([&s.returns]) {
            assert!(KINDS.contains(&c.trim_end_matches('?')), "{name}: {c}");
        }
    }
    // no documentation text: only names, counts and type codes
    let text = serde_json::to_string(&doc["functions"]).unwrap();
    assert!(!text.contains("Documentation"));
    for f in doc["library_only"].as_array().unwrap() {
        let name = f["name"].as_str().unwrap();
        assert!(!all.contains_key(name), "{name} is in both lists");
        assert!(f["min"].as_u64().unwrap() <= f["max"].as_u64().unwrap(), "{name}");
    }
}

/// Every catalogue function was recorded, and its counts, parameter names and types and return
/// type are Desktop's, except the listed deviations.
#[test]
fn the_catalogue_matches_desktops_engine() {
    let all = recorded(&corpus());
    let mut agree = 0;
    let mut differ = Vec::new();
    let mut deviating = Vec::new();
    for f in bicraft_mlang::functions() {
        let want = all
            .get(f.name)
            .unwrap_or_else(|| panic!("{} was not recorded", f.name));
        let got = catalogue(f);
        let listed = DEVIATIONS.iter().any(|(n, _)| *n == f.name);
        match (got == *want, listed) {
            (true, false) => agree += 1,
            (false, true) => deviating.push(f.name),
            (true, true) => differ.push(format!("{}: listed as a deviation but matches", f.name)),
            (false, false) => differ.push(format!("{}: catalogue {got:?}, Desktop {want:?}", f.name)),
        }
    }
    assert_eq!(
        all.len(),
        bicraft_mlang::functions().len(),
        "recorded functions not in the catalogue"
    );
    println!(
        "M signatures: {agree} of {} catalogue functions agree with Desktop, {} listed deviations",
        all.len(),
        deviating.len()
    );
    assert!(differ.is_empty(), "{}", differ.join("\n"));
    assert_eq!(deviating.len(), DEVIATIONS.len());
}

/// The deviations differ only by trailing optional parameters BIcraft does not take yet.
#[test]
fn deviations_only_drop_trailing_optional_parameters() {
    let all = recorded(&corpus());
    for (name, _) in DEVIATIONS {
        let f = bicraft_mlang::function(name).unwrap();
        let (got, want) = (catalogue(f), &all[*name]);
        assert_eq!(got.min, want.min, "{name}");
        assert!(got.max < want.max, "{name}");
        assert_eq!(got.params[..], want.params[..got.max], "{name}");
        assert_eq!(got.names[..], want.names[..got.max], "{name}");
        assert_eq!(got.returns, want.returns, "{name}");
    }
}
