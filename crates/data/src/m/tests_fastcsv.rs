//! The CSV fast path (`fastcsv.rs`) against the evaluator: generated CSV files of every shape
//! the fast path takes (quotes, embedded line breaks, empty fields, short and long records,
//! cultures, dates, decimals, currency, BOM, CRLF, lone CR, code pages, multi-byte
//! delimiters), loaded both ways, must give the same table or the same error. Regions and
//! chunks are tiny in tests (`REGION`, `MIN_CHUNK`), so these small files cross many region
//! and chunk boundaries.

use super::*;
use crate::tempdir::TempDir;

/// Tiny deterministic PRNG (xorshift64*).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len())]
    }
}

const CELLS: &[&str] = &[
    "1",
    "-12",
    "007",
    "123456789012345678",
    "9999999999999999999",
    "1.5",
    "-0.25",
    "1,5",
    "1 234,5",
    "1,234.5",
    "$2.675",
    "-$1.5",
    "2,675 €",
    "1e3",
    "12.34567",
    "0.1",
    "",
    "",
    "",
    " 7 ",
    "+3",
    "abc",
    "2026-01-05",
    "2026-02-30",
    "05/01/2026",
    "1/5/2026 12:00",
    "2026-01-05T10:30:00",
    "Jan 5, 2026",
    "true",
    "FALSE",
    "x",
    "é",
    "€",
    "10%",
    "(2.5)",
    "NaN",
    "12:30",
    "1.02:03:04",
    "5.",
    ".5",
    "-",
];

const TYPES: &[&str] = &[
    "Int64.Type",
    "type number",
    "Currency.Type",
    "type date",
    "type datetime",
    "type text",
    "type logical",
    "Percentage.Type",
    "Int32.Type",
    "type time",
    "type duration",
    "Decimal.Type",
    "Double.Type",
    "type any",
];

fn field(rng: &mut Rng, delim: &str) -> String {
    let base = match rng.below(10) {
        0 => format!("a{delim}b"),
        1 => "line\nbreak".into(),
        2 => "cr\r\nlf".into(),
        3 => "say \"hi\"".into(),
        _ => rng.pick(CELLS).to_string(),
    };
    match rng.below(6) {
        0 => format!("\"{}\"", base.replace('"', "\"\"")),
        // Text after a closing quote, and a quote inside an unquoted field.
        1 => format!("\"{}\"x", base.replace('"', "\"\"")),
        2 if !base.contains(['\n', '\r']) && !base.contains(delim) => format!("{base}\"q"),
        _ if base.contains(['\n', '\r', '"']) || base.contains(delim) => {
            format!("\"{}\"", base.replace('"', "\"\""))
        }
        _ => base,
    }
}

fn csv_text(rng: &mut Rng, cols: usize, rows: usize, delim: &str) -> (String, Vec<String>) {
    let mut s = String::new();
    let eol = ["\n", "\r\n", "\r"];
    let line_end = eol[rng.below(2)];
    let names: Vec<String> = (0..cols)
        .map(|i| match rng.below(8) {
            0 => String::new(),
            1 => "Dup".into(),
            _ => format!("C{i}"),
        })
        .collect();
    s.push_str(&names.join(delim));
    s.push_str(line_end);
    for _ in 0..rows {
        let n = match rng.below(20) {
            0 => 0,
            1 => cols + 1,
            2 => cols.saturating_sub(1),
            _ => cols,
        };
        let fields: Vec<String> = (0..n).map(|_| field(rng, delim)).collect();
        s.push_str(&fields.join(delim));
        s.push_str(if rng.below(30) == 0 {
            eol[rng.below(3)]
        } else {
            line_end
        });
    }
    if rng.below(3) == 0 {
        // No line break after the last record.
        while s.ends_with(['\n', '\r']) {
            s.pop();
        }
    }
    (s, names)
}

struct Case {
    expr: String,
    opts: SourceOptions,
}

fn query(path: &str, opts: &str, promote: bool, types: &[(String, &str)], culture: Option<&str>) -> String {
    let p = path.replace('"', "\"\"");
    let mut steps = format!("let\n    Source = Csv.Document(File.Contents(\"{p}\"){opts}),\n");
    let mut last = "Source".to_string();
    if promote {
        steps.push_str(
            "    #\"Promoted Headers\" = Table.PromoteHeaders(Source, [PromoteAllScalars = true]),\n",
        );
        last = "#\"Promoted Headers\"".into();
    }
    if !types.is_empty() {
        let t: Vec<String> = types.iter().map(|(c, ty)| format!("{{\"{c}\", {ty}}}")).collect();
        let c = culture.map(|c| format!(", \"{c}\"")).unwrap_or_default();
        steps.push_str(&format!(
            "    #\"Changed Type\" = Table.TransformColumnTypes({last}, {{{}}}{c}),\n",
            t.join(", ")
        ));
        last = "#\"Changed Type\"".into();
    }
    let steps = steps.trim_end_matches(",\n").to_string();
    format!("{steps}\nin\n    {last}")
}

/// Loads both ways; the fast path must take the query (unless `may_fall_back`). Returns
/// whether it did.
fn compare(case: &Case, may_fall_back: bool) -> bool {
    let main = parser::parse(&case.expr, &case.opts.limits).unwrap();
    let mut interp = eval::Interp::new(&case.opts, Vec::new());
    let fast = fastcsv::try_load(&main, &mut interp, &case.opts);
    drop(interp);
    let slow = evaluate_with(&case.expr, &case.opts, false);
    let Some(fast) = fast else {
        assert!(may_fall_back, "fast path not taken:\n{}", case.expr);
        return false;
    };
    match (&fast, &slow) {
        (Ok(f), Ok(s)) => {
            if f != s {
                let names = |t: &TableData| {
                    t.columns()
                        .iter()
                        .map(|c| (c.name.clone(), c.data_type))
                        .collect::<Vec<_>>()
                };
                assert_eq!(names(f), names(s), "{}", case.expr);
                assert_eq!(f.row_count(), s.row_count(), "{}", case.expr);
                for (a, b) in f.columns().iter().zip(s.columns()) {
                    for (r, (x, y)) in a.values.iter().zip(&b.values).enumerate() {
                        let nan = matches!((x, y), (CellValue::Real(p), CellValue::Real(q)) if p.is_nan() && q.is_nan());
                        assert!(
                            nan || x == y,
                            "column {} row {}: {x:?} != {y:?}\n{}",
                            a.name,
                            r + 1,
                            case.expr
                        );
                    }
                }
            }
            for i in 0..f.columns().len() {
                assert_eq!(f.date_kind(i), s.date_kind(i), "{}", case.expr);
            }
        }
        (Err(f), Err(s)) => assert_eq!(f, s, "{}", case.expr),
        _ => panic!("fast {fast:?}\nslow {slow:?}\n{}", case.expr),
    }
    true
}

fn options(dir: &TempDir, culture: &str) -> SourceOptions {
    SourceOptions {
        file_roots: vec![dir.to_path_buf()],
        culture: Some(culture.into()),
        ..SourceOptions::default()
    }
}

#[test]
fn generated_files_match_the_evaluator() {
    let dir = TempDir::new("fastcsv-diff");
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let delims = [",", ";", "\t", "||", "§"];
    let mut taken = 0;
    for case in 0..160 {
        let delim = delims[rng.below(delims.len())];
        let cols = 1 + rng.below(5);
        let rows = rng.below(if case % 10 == 0 { 400 } else { 40 });
        let (mut text, header) = csv_text(&mut rng, cols, rows, delim);
        let bom = rng.below(4) == 0;
        if bom {
            text.insert(0, '\u{feff}');
        }
        let path = dir.join(format!("f{case}.csv"));
        std::fs::write(&path, &text).unwrap();
        let quote = [
            "",
            ", QuoteStyle = QuoteStyle.Csv",
            ", QuoteStyle = QuoteStyle.None",
        ][rng.below(3)];
        let columns = match rng.below(4) {
            0 => format!(", Columns = {}", cols + rng.below(3) - 1),
            _ => String::new(),
        };
        let encoding = [", Encoding = 65001", ", Encoding = 1252", ""][rng.below(3)];
        let opts = format!(
            ", [Delimiter = \"{}\"{columns}{encoding}{quote}]",
            delim.replace('\t', "#(tab)")
        );
        let promote = rng.below(4) != 0;
        let names: Vec<String> = if promote {
            header.iter().filter(|n| n.starts_with('C')).cloned().collect()
        } else {
            (1..=cols).map(|i| format!("Column{i}")).collect()
        };
        let mut types: Vec<(String, &str)> = Vec::new();
        for n in &names {
            if rng.below(5) != 0 {
                types.push((n.clone(), rng.pick(TYPES)));
            }
        }
        let culture = [None, Some("en-US"), Some("fr-FR"), Some("de-DE"), Some("")][rng.below(5)];
        let model_culture = ["en-US", "fr-FR"][rng.below(2)];
        let mut o = options(&dir, model_culture);
        o.strict_errors = rng.below(5) == 0;
        let case = Case {
            expr: query(&path.display().to_string(), &opts, promote, &types, culture),
            opts: o,
        };
        // A header name may be missing (empty or duplicate names) and a long record with
        // ExtraValues defaults: the evaluator decides those.
        if compare(&case, true) {
            taken += 1;
        }
    }
    eprintln!("fast path taken in {taken} of 160 cases");
    assert!(taken >= 140, "fast path taken in {taken} of 160 cases");
}

#[test]
fn the_bench_shape_takes_the_fast_path() {
    let dir = TempDir::new("fastcsv-bench-shape");
    let mut s = String::from("Date,ProduitKey,MagasinKey,ClientKey,Quantite,Montant\n");
    for k in 0..3000u64 {
        s.push_str(&format!(
            "2024-{:02}-{:02},{},{},{},{},{}.{:02}\n",
            1 + k % 12,
            1 + k % 28,
            k % 1000,
            k % 200,
            k * 7919 % 100_000,
            1 + k % 10,
            k * 31 % 5000,
            k % 100
        ));
    }
    let path = dir.join("Ventes.csv");
    std::fs::write(&path, &s).unwrap();
    let types: Vec<(String, &str)> = vec![
        ("Date".into(), "type date"),
        ("ProduitKey".into(), "Int64.Type"),
        ("MagasinKey".into(), "Int64.Type"),
        ("ClientKey".into(), "Int64.Type"),
        ("Quantite".into(), "Int64.Type"),
        ("Montant".into(), "Currency.Type"),
    ];
    for (opts, culture) in [
        (
            ", [Delimiter = \",\", Encoding = 65001, QuoteStyle = QuoteStyle.None]",
            Some("en-US"),
        ),
        (
            ", [Delimiter = \",\", Columns = 6, Encoding = 1252, QuoteStyle = QuoteStyle.None]",
            None,
        ),
        ("", None),
    ] {
        let case = Case {
            expr: query(&path.display().to_string(), opts, true, &types, culture),
            opts: options(&dir, "en-US"),
        };
        compare(&case, false);
    }
    // Nested calls, no let.
    let p = path.display().to_string().replace('"', "\"\"");
    let case = Case {
        expr: format!(
            "Table.TransformColumnTypes(Table.PromoteHeaders(Csv.Document(File.Contents(\"{p}\"))), {{{{\"Quantite\", Int64.Type}}}})"
        ),
        opts: options(&dir, "en-US"),
    };
    compare(&case, false);
}

#[test]
fn encodings_and_line_endings() {
    let dir = TempDir::new("fastcsv-enc");
    let body = "Nom;Prix;Quand\r\nVélo;1,5;05/01/2026\r\n\"Cas\r\nque\";-2,25;2026-12-31\rCafé;3;\r\n";
    let mut files: Vec<(String, Vec<u8>, &str)> = vec![
        ("utf8.csv".into(), body.as_bytes().to_vec(), ", Encoding = 65001"),
        (
            "bom.csv".into(),
            [&[0xEF, 0xBB, 0xBF][..], body.as_bytes()].concat(),
            ", Encoding = 1252",
        ),
        // Invalid UTF-8 (a lone 0xE9): decoded with replacement characters.
        ("bad.csv".into(), b"A;B\r\nv\xE9lo;1\r\n".to_vec(), ""),
        // Windows-1252 é.
        (
            "cp.csv".into(),
            b"A;B\r\nv\xE9lo;1\r\n".to_vec(),
            ", Encoding = 1252",
        ),
        ("ascii.csv".into(), b"A;B\nx;1\n".to_vec(), ", Encoding = 20127"),
    ];
    let mut le = vec![0xFF, 0xFE];
    le.extend(body.encode_utf16().flat_map(u16::to_le_bytes));
    files.push(("utf16.csv".into(), le, ", Encoding = 1200"));
    // A multi-byte character across the tiny regions.
    let long: String = (0..2000).map(|i| format!("é€{i};{i}\n")).collect();
    files.push(("long.csv".into(), format!("A;B\n{long}").into_bytes(), ""));
    for (name, bytes, enc) in files {
        let path = dir.join(&name);
        std::fs::write(&path, &bytes).unwrap();
        for quote in ["QuoteStyle.Csv", "QuoteStyle.None"] {
            let opts = format!(", [Delimiter = \";\"{enc}, QuoteStyle = {quote}]");
            let promoted = query(&path.display().to_string(), &opts, true, &[], None);
            let case = Case {
                expr: promoted,
                opts: options(&dir, "fr-FR"),
            };
            compare(&case, false);
        }
    }
}

#[test]
fn limits_are_the_evaluators() {
    let dir = TempDir::new("fastcsv-limits");
    let mut s = String::from("A,B\n");
    for i in 0..500 {
        s.push_str(&format!("{i},\"{}\"\n", "x".repeat(i % 50)));
    }
    let path = dir.join("t.csv");
    std::fs::write(&path, &s).unwrap();
    let expr = query("t.csv", "", true, &[("A".into(), "Int64.Type")], None);
    for limits in [
        Limits {
            max_rows: 300,
            ..Limits::default()
        },
        Limits {
            max_rows: 500,
            ..Limits::default()
        },
        Limits {
            max_rows: 501,
            ..Limits::default()
        },
        Limits {
            max_steps: 900,
            ..Limits::default()
        },
        Limits {
            max_cell_bytes: 40,
            ..Limits::default()
        },
    ] {
        let mut o = options(&dir, "en-US");
        o.limits = limits;
        compare(
            &Case {
                expr: expr.clone(),
                opts: o,
            },
            false,
        );
    }
    // Text after the quoted part counts toward the cell limit.
    std::fs::write(&path, format!("A\n\"{}\"{}\n", "y".repeat(30), "z".repeat(20))).unwrap();
    let mut o = options(&dir, "en-US");
    o.limits.max_cell_bytes = 40;
    let expr = query("t.csv", "", true, &[], None);
    compare(&Case { expr, opts: o }, false);
}

#[test]
fn files_over_the_old_cap_and_the_budget() {
    let dir = TempDir::new("fastcsv-cap");
    let mut s = String::from("A,B\n");
    for i in 0..20_000 {
        s.push_str(&format!("{i},{}\n", i * 3));
    }
    let path = dir.join("big.csv");
    std::fs::write(&path, &s).unwrap();
    let expr = query(
        &path.display().to_string(),
        "",
        true,
        &[("A".into(), "Int64.Type"), ("B".into(), "Int64.Type")],
        None,
    );
    // The evaluator's file cap no longer applies to this shape...
    let mut o = options(&dir, "en-US");
    o.limits.max_bytes = 1000;
    let t = load_m_with(&expr, &o).unwrap();
    assert_eq!(t.row_count(), 20_000);
    assert_eq!(t.columns()[1].values[19_999], CellValue::Int(59_997));
    // ...the streamed-file cap and the memory budget do, refused cleanly.
    o.limits.max_file_bytes = 1000;
    let e = load_m_with(&expr, &o).unwrap_err();
    assert!(
        matches!(&e, Error::LimitExceeded(m) if m.contains("larger than 1000 bytes")),
        "{e}"
    );
    let mut o = options(&dir, "en-US");
    o.limits.max_load_bytes = 100_000;
    let e = load_m_with(&expr, &o).unwrap_err();
    assert!(
        matches!(&e, Error::LimitExceeded(m) if m.contains("load budget")),
        "{e}"
    );
    // Other shapes keep the evaluator's cap.
    let mut o = options(&dir, "en-US");
    o.limits.max_bytes = 1000;
    let p = path.display().to_string().replace('"', "\"\"");
    let other = format!("Table.FirstN(Csv.Document(File.Contents(\"{p}\")), 2)");
    assert!(matches!(load_m_with(&other, &o), Err(Error::LimitExceeded(_))));
}

#[test]
fn hostile_inputs_never_panic() {
    let dir = TempDir::new("fastcsv-hostile");
    let mut rng = Rng(7);
    let pieces: &[&[u8]] = &[
        b",",
        b";",
        b"\"",
        b"\"\"",
        b"\r",
        b"\n",
        b"\r\n",
        b"1",
        b"a",
        b"\xC3\xA9",
        b"\xFF",
        b"\xE2\x82",
        b"\xEF\xBB\xBF",
        b"\0",
        b"2026-01-05",
        b"1,5",
    ];
    for case in 0..120 {
        let n = rng.below(400);
        let mut bytes = Vec::new();
        for _ in 0..n {
            bytes.extend_from_slice(pieces[rng.below(pieces.len())]);
        }
        let path = dir.join(format!("h{case}.bin"));
        std::fs::write(&path, &bytes).unwrap();
        let quote = ["QuoteStyle.Csv", "QuoteStyle.None"][case % 2];
        let opts = format!(", [Delimiter = \",\", QuoteStyle = {quote}]");
        let expr = query(
            &path.display().to_string(),
            &opts,
            case % 3 != 0,
            &[("Column1".into(), "Int64.Type")],
            None,
        );
        compare(
            &Case {
                expr,
                opts: options(&dir, "en-US"),
            },
            true,
        );
    }
    // A huge line, an unterminated quote over the whole file, binary junk.
    let huge = format!("A,B\n1,{}\n", "q".repeat(20_000));
    let open = format!("A,B\n1,\"{}\n2,3\n", "w\n".repeat(3000));
    let junk: Vec<u8> = (0..20_000u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    for (i, bytes) in [huge.into_bytes(), open.into_bytes(), junk]
        .into_iter()
        .enumerate()
    {
        let path = dir.join(format!("x{i}.csv"));
        std::fs::write(&path, &bytes).unwrap();
        for quote in ["QuoteStyle.Csv", "QuoteStyle.None"] {
            let opts = format!(", [Delimiter = \",\", QuoteStyle = {quote}]");
            let expr = query(&path.display().to_string(), &opts, true, &[], None);
            compare(
                &Case {
                    expr,
                    opts: options(&dir, "en-US"),
                },
                true,
            );
        }
    }
}

#[test]
fn other_shapes_fall_back() {
    let dir = TempDir::new("fastcsv-shapes");
    let path = dir.join("t.csv");
    std::fs::write(&path, "A,B\n1,2\n").unwrap();
    let p = path.display().to_string().replace('"', "\"\"");
    for expr in [
        // A binding shadows a library function.
        format!(
            "let\n    #\"Csv.Document\" = (x) => #table({{\"A\"}}, {{{{1}}}}),\n    S = Csv.Document(File.Contents(\"{p}\"))\nin\n    S"
        ),
        format!("Table.FirstN(Csv.Document(File.Contents(\"{p}\")), 1)"),
        format!("Csv.Document(File.Contents(\"{p}\"), [Columns = {{\"X\", \"Y\"}}])"),
        format!("Csv.Document(File.Contents(\"{p}\"), [ExtraValues = ExtraValues.List])"),
        format!(
            "Table.TransformColumnTypes(Table.PromoteHeaders(Csv.Document(File.Contents(\"{p}\"))), {{{{\"Missing\", Int64.Type}}}})"
        ),
        format!(
            "Table.TransformColumnTypes(Table.PromoteHeaders(Csv.Document(File.Contents(\"{p}\"))), {{{{\"A\", type datetimezone}}}})"
        ),
        format!("Csv.Document(File.Contents(\"{p}\"), [Encoding = 437])"),
        "Csv.Document(File.Contents(\"missing.csv\"))".to_string(),
    ] {
        compare(
            &Case {
                expr,
                opts: options(&dir, "en-US"),
            },
            true,
        );
    }
}
