//! Caps, sandboxing, laziness and fuzz-style tests of the M evaluator.

use super::*;
use CellValue::{Int, Null, Real, Text};

fn load(s: &str) -> Result<TableData, Error> {
    load_m(s, &Limits::default())
}

fn vals(t: &TableData, i: usize) -> Vec<CellValue> {
    t.columns()[i].values.clone()
}

fn table1(expr: &str) -> String {
    format!("#table({{\"A\"}}, {{{{{expr}}}}})")
}

#[test]
fn recursion_and_step_caps() {
    // Unbounded recursion hits the evaluation depth cap, not the stack.
    let r = load(&table1(
        "let f = (n) => if n = 0 then 0 else 1 + @f(n - 1) in f(1000000)",
    ));
    assert!(matches!(r, Err(Error::LimitExceeded(_))), "{r:?}");
    let r = load(&table1("let f = (n) => @f(n) in f(1)"));
    assert!(matches!(r, Err(Error::LimitExceeded(_))), "{r:?}");
    // Bounded recursion is fine.
    let t = load(&table1(
        "let f = (n) => if n <= 1 then 1 else n * @f(n - 1) in f(10)",
    ))
    .unwrap();
    assert_eq!(vals(&t, 0), vec![Int(3_628_800)]);
    // Work and time caps.
    let few_steps = Limits {
        max_steps: 10_000,
        ..Limits::default()
    };
    let r = load_m(&table1("List.Sum(List.Numbers(1, 100000))"), &few_steps);
    assert!(matches!(r, Err(Error::LimitExceeded(_))), "{r:?}");
    let r = load_m(
        &table1("List.Count(List.Generate(() => 0, each true, each _ + 1))"),
        &Limits {
            max_rows: 10_000,
            ..Limits::default()
        },
    );
    assert!(matches!(r, Err(Error::LimitExceeded(_))), "{r:?}");
    let quick = Limits {
        max_millis: 1,
        ..Limits::default()
    };
    let r = load_m(
        &table1("List.Accumulate(List.Numbers(1, 5000000), 0, (s, x) => s + x)"),
        &quick,
    );
    assert!(matches!(r, Err(Error::LimitExceeded(_))), "{r:?}");
    // Size caps.
    let r = load(&table1("Text.Length(Text.Repeat(\"x\", 1000000000))"));
    assert!(matches!(r, Err(Error::LimitExceeded(_))), "{r:?}");
    let small = Limits {
        max_rows: 100,
        ..Limits::default()
    };
    let r = load_m("Table.Repeat(#table({\"A\"}, {{1}}), 1000)", &small);
    assert!(matches!(r, Err(Error::LimitExceeded(_))), "{r:?}");
    let r = load_m(&table1("List.Count({1..1000000})"), &small);
    assert!(matches!(r, Err(Error::LimitExceeded(_))), "{r:?}");
}

#[test]
fn errors_are_lazy_and_catchable() {
    // An unused failing step does not fail the query.
    let t = load("let bad = error \"x\", ok = #table({\"A\"}, {{1}}) in ok").unwrap();
    assert_eq!(vals(&t, 0), vec![Int(1)]);
    // A record field error is raised only when the field is read.
    let t = load(&table1("[a = 1, b = error \"x\"][a]")).unwrap();
    assert_eq!(vals(&t, 0), vec![Int(1)]);
    let t = load(&table1("try [a = 1, b = error \"x\"][b] otherwise 7")).unwrap();
    assert_eq!(vals(&t, 0), vec![Int(7)]);
    let r = load("error [Reason = \"Custom.Error\", Message = \"boom\"]");
    assert!(
        matches!(&r, Err(Error::Evaluation { reason, message }) if reason == "Custom.Error" && message == "boom"),
        "{r:?}"
    );
    // Cell errors load as blanks (as in Desktop), or fail the load in strict mode.
    let e = "Table.TransformColumnTypes(#table({\"A\"}, {{\"1\"}, {\"x\"}}), {{\"A\", Int64.Type}})";
    let t = load(e).unwrap();
    assert_eq!(vals(&t, 0), vec![Int(1), Null]);
    let strict = SourceOptions {
        strict_errors: true,
        ..SourceOptions::default()
    };
    assert!(matches!(
        load_m_with(e, &strict),
        Err(Error::Value { row: 2, .. })
    ));
    // `try` does not hide unsupported functions, external sources or caps.
    let r = load(&table1("try Web.Contents(\"http://x\") otherwise 1"));
    assert!(matches!(r, Err(Error::ExternalSource(_))), "{r:?}");
}

#[test]
fn shared_queries_and_parameters() {
    let opts = SourceOptions {
        shared: vec![
            (
                "Rate".into(),
                "1.5 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]".into(),
            ),
            (
                "Base".into(),
                "#table(type table [A = number], {{2}, {4}})".into(),
            ),
        ],
        ..SourceOptions::default()
    };
    let t = load_m_with(
        "let S = Base, T = Table.AddColumn(S, \"B\", each [A] * Rate, type number) in T",
        &opts,
    )
    .unwrap();
    assert_eq!(vals(&t, 1), vec![Real(3.0), Real(6.0)]);
}

/// A guarded temp folder holding `root/inner/`; returns the guard and `root`.
fn temp_dir(tag: &str) -> (crate::tempdir::TempDir, std::path::PathBuf) {
    let tmp = crate::tempdir::TempDir::new(&format!("mq-{tag}"));
    let root = tmp.join("root");
    std::fs::create_dir_all(root.join("inner")).unwrap();
    (tmp, root)
}

fn csv_query(path: &str) -> String {
    format!("Csv.Document(File.Contents(\"{}\"))", path.replace('"', "\"\""))
}

#[test]
fn file_access_is_sandboxed() {
    let (tmp, root) = temp_dir("sandbox");
    std::fs::write(root.join("inner").join("a.csv"), "x,y\r\n1,2\r\n").unwrap();
    // A sibling of the root, inside the same guarded folder.
    let outside = tmp.join("outside.csv");
    std::fs::write(&outside, "secret\n").unwrap();
    let inside = root.join("inner").join("a.csv");
    let inside = inside.to_str().unwrap();
    // No root: file access is disabled.
    let r = load(&csv_query(inside));
    assert!(
        matches!(&r, Err(Error::UnsupportedSource(m)) if m.contains("File.Contents")),
        "{r:?}"
    );
    let opts = SourceOptions {
        file_roots: vec![root.clone()],
        ..SourceOptions::default()
    };
    let t = load_m_with(&csv_query(inside), &opts).unwrap();
    assert_eq!(vals(&t, 0), vec![Text("x".into()), Text("1".into())]);
    // Relative paths resolve against the root.
    let t = load_m_with(&csv_query("inner/a.csv"), &opts).unwrap();
    assert_eq!(t.row_count(), 2);
    // Outside the root, directly or through `..`: refused, and not catchable.
    for p in [
        outside.to_str().unwrap().to_string(),
        "../x.csv".into(),
        "inner/../../x.csv".into(),
    ] {
        let r = load_m_with(&format!("try {} otherwise null", csv_query(&p)), &opts);
        assert!(matches!(r, Err(Error::UnsupportedSource(_))), "{p}: {r:?}");
    }
    // A missing file inside the root is an ordinary (catchable) data source error.
    let r = load_m_with(&csv_query("inner/missing.csv"), &opts);
    assert!(
        matches!(&r, Err(Error::Evaluation { reason, .. }) if reason == "DataSource.Error"),
        "{r:?}"
    );
    let t = load_m_with(
        &format!(
            "try {} otherwise #table({{\"A\"}}, {{}})",
            csv_query("inner/missing.csv")
        ),
        &opts,
    )
    .unwrap();
    assert_eq!(t.row_count(), 0);
    // Size cap.
    let tiny = SourceOptions {
        limits: Limits {
            max_bytes: 4,
            ..Limits::default()
        },
        ..opts
    };
    let r = load_m_with(&csv_query("inner/a.csv"), &tiny);
    assert!(matches!(r, Err(Error::LimitExceeded(_))), "{r:?}");
}

fn m_text(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

fn texts(t: &TableData, i: usize) -> Vec<String> {
    vals(t, i)
        .into_iter()
        .map(|v| match v {
            Text(s) => s,
            other => format!("{other:?}"),
        })
        .collect()
}

#[test]
fn folder_files_combine_and_laziness() {
    let (tmp, root) = temp_dir("folder");
    std::fs::write(root.join("b.csv"), "k,v\r\nb,2\r\n").unwrap();
    std::fs::write(root.join("inner").join("a.csv"), "k,v\r\na,1\r\n").unwrap();
    std::fs::create_dir_all(root.join("inner").join("deep")).unwrap();
    std::fs::write(root.join("inner").join("deep").join("C.CSV"), "k,v\r\nc,3\r\n").unwrap();
    // Too large for the cap below, but never read: it is filtered out first.
    std::fs::write(root.join("big.txt"), vec![b'x'; 4096]).unwrap();
    std::fs::write(tmp.join("outside.csv"), "k,v\r\nz,9\r\n").unwrap();
    let opts = SourceOptions {
        file_roots: vec![root.clone()],
        limits: Limits {
            max_bytes: 1024,
            ..Limits::default()
        },
        ..SourceOptions::default()
    };
    let dir = m_text(root.to_str().unwrap());
    let listing = format!(
        "let S = Folder.Files({dir}) in Table.SelectColumns(S, {{\"Name\", \"Extension\", \"Folder Path\"}})"
    );
    let t = load_m_with(&listing, &opts).unwrap();
    // Files of a folder (by name) before its subfolders; extensions in lower case.
    assert_eq!(texts(&t, 0), ["b.csv", "big.txt", "a.csv", "C.CSV"]);
    assert_eq!(texts(&t, 1), [".csv", ".txt", ".csv", ".csv"]);
    let sep = std::path::MAIN_SEPARATOR;
    let base = root.to_str().unwrap();
    assert_eq!(
        texts(&t, 2),
        [
            format!("{base}{sep}"),
            format!("{base}{sep}"),
            format!("{base}{sep}inner{sep}"),
            format!("{base}{sep}inner{sep}deep{sep}"),
        ]
    );
    // Desktop's "Combine files" shape.
    let combine = format!(
        "let S = Folder.Files({dir}),
            F = Table.SelectRows(S, each Text.Lower([Extension]) = \".csv\" and [Attributes]?[Hidden]? <> true),
            A = Table.AddColumn(F, \"T\", each Table.PromoteHeaders(Csv.Document([Content]))),
            C = Table.Combine(A[T])
        in C"
    );
    let t = load_m_with(&combine, &opts).unwrap();
    assert_eq!(texts(&t, 0), ["b", "a", "c"]);
    assert_eq!(texts(&t, 1), ["2", "1", "3"]);
    // Reading the large file hits the cap (not catchable).
    let read_big = format!(
        "let S = Folder.Files({dir}), B = Table.SelectRows(S, each [Name] = \"big.txt\"){{0}}[Content] in \
         #table({{\"N\"}}, {{{{try Binary.Length(B) otherwise -1}}}})"
    );
    let r = load_m_with(&read_big, &opts);
    assert!(matches!(r, Err(Error::LimitExceeded(_))), "{r:?}");
    // Relative paths resolve against the first root; outside the roots is refused.
    let t = load_m_with("Table.SelectColumns(Folder.Files(\"inner\"), {\"Name\"})", &opts).unwrap();
    assert_eq!(texts(&t, 0), ["a.csv", "C.CSV"]);
    for p in [tmp.to_str().unwrap(), "..", "inner/../.."] {
        let r = load_m_with(&format!("try Folder.Files({}) otherwise null", m_text(p)), &opts);
        assert!(matches!(r, Err(Error::UnsupportedSource(_))), "{p}: {r:?}");
    }
    // A missing folder inside the root is a catchable data source error.
    let t = load_m_with(
        "try Folder.Files(\"missing\") otherwise #table({\"A\"}, {})",
        &opts,
    )
    .unwrap();
    assert_eq!(t.row_count(), 0);
    // The allow-list: a second root opens the outside folder.
    let both = SourceOptions {
        file_roots: vec![root.clone(), tmp.to_path_buf()],
        ..opts.clone()
    };
    let t = load_m_with(
        &format!(
            "Table.PromoteHeaders(Csv.Document(File.Contents({})))",
            m_text(tmp.join("outside.csv").to_str().unwrap())
        ),
        &both,
    )
    .unwrap();
    assert_eq!(texts(&t, 0), ["z"]);
}

#[test]
fn folder_contents_nests_folders() {
    let (_tmp, root) = temp_dir("contents");
    std::fs::write(root.join("x.json"), "[1, 2]").unwrap();
    std::fs::write(root.join("inner").join("a.csv"), "1\r\n").unwrap();
    let opts = SourceOptions {
        file_roots: vec![root.clone()],
        ..SourceOptions::default()
    };
    let dir = m_text(root.to_str().unwrap());
    let q = format!(
        "let S = Folder.Contents({dir}),
            K = Table.AddColumn(S, \"Kind\", each [Attributes][Kind]),
            N = Table.AddColumn(K, \"Inside\", each if [Kind] = \"Folder\" then Text.Combine([Content][Name], \",\") else Text.From(Binary.Length([Content])))
        in Table.SelectColumns(N, {{\"Name\", \"Extension\", \"Kind\", \"Inside\"}})"
    );
    let t = load_m_with(&q, &opts).unwrap();
    assert_eq!(texts(&t, 0), ["inner", "x.json"]);
    assert_eq!(texts(&t, 1), ["", ".json"]);
    assert_eq!(texts(&t, 2), ["Folder", "JSON File"]);
    assert_eq!(texts(&t, 3), ["a.csv", "6"]);
}

#[cfg(unix)]
#[test]
fn folder_listing_skips_links_out_of_the_root() {
    let (tmp, root) = temp_dir("links");
    std::fs::write(tmp.join("secret.csv"), "s\n").unwrap();
    std::fs::write(root.join("ok.csv"), "o\n").unwrap();
    std::os::unix::fs::symlink(tmp.join("secret.csv"), root.join("link.csv")).unwrap();
    std::os::unix::fs::symlink(&*tmp, root.join("inner").join("loop")).unwrap();
    let opts = SourceOptions {
        file_roots: vec![root.clone()],
        ..SourceOptions::default()
    };
    let q = format!(
        "Table.SelectColumns(Folder.Files({}), {{\"Name\"}})",
        m_text(root.to_str().unwrap())
    );
    let t = load_m_with(&q, &opts).unwrap();
    assert_eq!(texts(&t, 0), ["ok.csv"]);
}

fn base64(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for chunk in b.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, x)| acc | (u32::from(*x) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                s.push(char::from(T[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                s.push('=');
            }
        }
    }
    s
}

#[test]
fn decompression_is_capped() {
    // 1 MiB of zeros, raw-deflated, must not inflate past `max_bytes`.
    let zeros = vec![0u8; 1 << 20];
    let packed = miniz_oxide::deflate::compress_to_vec(&zeros, 6);
    let e = table1(&format!(
        "Binary.Length(Binary.Decompress(Binary.FromText(\"{}\"), Compression.Deflate))",
        base64(&packed)
    ));
    let t = load(&e).unwrap();
    assert_eq!(vals(&t, 0), vec![Int(1 << 20)]);
    let limits = Limits {
        max_bytes: 64 * 1024,
        ..Limits::default()
    };
    assert!(matches!(load_m(&e, &limits), Err(Error::LimitExceeded(_))));
}

/// A tiny deterministic generator for fuzz-style tests.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
}

#[test]
fn random_token_soup_never_panics() {
    const TOKENS: &[&str] = &[
        "let",
        "in",
        "each",
        "_",
        "if",
        "then",
        "else",
        "try",
        "otherwise",
        "error",
        "(",
        ")",
        "[",
        "]",
        "{",
        "}",
        ",",
        "=",
        "=>",
        "+",
        "-",
        "*",
        "/",
        "&",
        "<",
        ">=",
        "..",
        "??",
        "?",
        "@",
        "1",
        "0",
        "-1",
        "2.5",
        "\"t\"",
        "null",
        "true",
        "#table",
        "#date",
        "#duration",
        "#\"q\"",
        "A",
        "x",
        "Table.FromRows",
        "Table.AddColumn",
        "Table.SelectRows",
        "Table.Group",
        "List.Numbers",
        "List.Generate",
        "Text.Repeat",
        "Table.Repeat",
        "Number.From",
        "Text.From",
        "type",
        "table",
        "text",
        "number",
        "meta",
        "and",
        "or",
        "not",
        "as",
        "is",
        "Binary.FromText",
        "Json.Document",
        "Csv.Document",
        "Table.TransformColumnTypes",
        "Int64.Type",
        "Table.NestedJoin",
        "Table.ExpandTableColumn",
        "Table.Sort",
        "Table.PromoteHeaders",
        "[A]",
        "{1..3}",
    ];
    let limits = Limits {
        max_steps: 200_000,
        max_rows: 10_000,
        max_cell_bytes: 1 << 16,
        ..Limits::default()
    };
    let mut rng = Lcg(42);
    for _ in 0..1500 {
        let n = 1 + (rng.next() % 30) as usize;
        let src: Vec<&str> = (0..n)
            .map(|_| TOKENS[(rng.next() % TOKENS.len() as u64) as usize])
            .collect();
        let _ = load_m(&src.join(" "), &limits);
    }
    for _ in 0..300 {
        let n = (rng.next() % 64) as usize;
        let bytes: Vec<u8> = (0..n).map(|_| (rng.next() % 256) as u8).collect();
        let _ = load_m(&String::from_utf8_lossy(&bytes), &limits);
    }
}

#[test]
fn extreme_values_never_overflow() {
    for e in [
        "#datetime(9999,12,31,23,59,59) + #duration(1000000000,0,0,0)",
        "#date(1,1,1) - #duration(1,0,0,0)",
        "#duration(1e300,0,0,0)",
        "#duration(100000000000000,0,0,0) + #duration(100000000000000,0,0,0) * 1000",
        "Date.AddDays(#date(9999,12,31), 9007199254740991)",
        "Date.AddMonths(#date(2000,1,1), -9007199254740991)",
        "Date.AddYears(#datetimezone(9999,1,1,0,0,0,14,0), 1)",
        "DateTimeZone.SwitchZone(#datetimezone(9999,12,31,23,0,0,-14,0), 14)",
        "DateTimeZone.SwitchZone(#datetimezone(2000,1,1,0,0,0,0,0), 99999999)",
        "DateTime.AddZone(#datetime(2000,1,1,0,0,0), -9007199254740991)",
        "Number.From(#datetimezone(1,1,1,0,0,0,14,0))",
        "DateTime.From(1e300)",
        "Date.From(-1e18)",
        "List.Dates(#date(9999,12,1), 100, #duration(1,0,0,0))",
        "List.DateTimes(#datetime(2000,1,1,0,0,0), 3, #duration(1e14,0,0,0))",
        "#time(23,59,59) + #duration(-1e14,0,0,0)",
        "Duration.TotalDays(#duration(0,0,0,-9e15))",
        "Text.From(#duration(-1e14,0,0,0))",
        "Number.Round(1e308, 15)",
        "Number.Round(1e-308, -15)",
        "Int64.From(1e300)",
        "Text.PadStart(\"x\", 9007199254740991)",
        "Text.Middle(\"abc\", 9007199254740991, 9007199254740991)",
        "List.Range({1,2,3}, 9007199254740991, 9007199254740991)",
        "Table.Range(#table({\"A\"}, {{1}}), 9007199254740991, 9007199254740991)",
        "Table.AddIndexColumn(#table({\"A\"}, {{1}}), \"I\", 1e308, 1e308)",
        "Splitter.SplitTextByPositions({9007199254740991, 0})(\"abc\")",
        "Splitter.SplitTextByLengths({9007199254740991, 9007199254740991})(\"abc\")",
        "Number.ToText(1e308, \"N30\")",
        "Number.ToText(-1e-308, \"E30\")",
        "Number.ToText(1e18, \"X\")",
        "Date.ToText(#date(9999,12,31), \"yyyyyyyyyyyyyyyyyyyyyyyy fffffffffffffff zzzzz\")",
        "Date.FromText(\"99999999999999999999-01-01\")",
        "Time.FromText(\"99999999999999999:00\")",
        "Duration.FromText(\"99999999999999999.23:59:59\")",
        "Date.FromText(\"1/1/1\", [Format=\"fffffffffffffffffff\"])",
        "Character.FromNumber(1e300)",
        "Binary.FromList({1e300})",
        "List.Numbers(0, 9007199254740991)",
        "List.Repeat({1}, 9007199254740991)",
        "Table.Repeat(#table({\"A\"}, {{1}}), 9007199254740991)",
        "{1..9007199254740991}",
        "{-9007199254740991..9007199254740991}",
    ] {
        let _ = load_m(
            &table1(&format!("try {e} otherwise null")),
            &Limits {
                max_steps: 1_000_000,
                max_rows: 100_000,
                ..Limits::default()
            },
        );
    }
}

#[test]
fn hostile_decimals() {
    let text = |e: &str| -> String {
        let t = load(&format!("#table(type table [A = text], {{{{{e}}}}})")).unwrap();
        match vals(&t, 0).first() {
            Some(Text(s)) => s.clone(),
            other => format!("{other:?}"),
        }
    };
    // Desktop never returns from these; bicraft answers.
    assert_eq!(
        text("Text.From(Number.Round(79228162514264337593543950335))"),
        "79228162514264337593543950335"
    );
    assert_eq!(text("Text.From(Number.Round(2.675, -1000))"), "0");
    assert_eq!(text("Text.From(Number.Round(2.675, 1000))"), "2.675");
    assert_eq!(
        text("Text.From(Number.RoundUp(79228162514264337593543950335, 28))"),
        "79228162514264337593543950335"
    );
    // Long literals, extreme exponents and precisions.
    let long = format!("0.{}", "9".repeat(300));
    assert_eq!(text(&format!("Text.From({long})")), "1");
    assert_eq!(text("Text.From(1e-100000)"), "0");
    assert_eq!(text("Text.From(1e100000)"), "\u{221e}");
    let t = text("Number.ToText(0.1234567890123456789, \"E999999999\")");
    assert!(t.len() < 200, "{t}");
    let t = text("Number.ToText(0.1234567890123456789, \"F999999999\")");
    assert!(t.len() < 200, "{t}");
    assert_eq!(
        text(
            "Text.From(Value.Divide(79228162514264337593543950335, 0.0000000000000000000000000001, Precision.Decimal))"
        ),
        "\u{221e}"
    );
    assert_eq!(
        text(
            "Text.From(List.Sum({79228162514264337593543950335, 79228162514264337593543950335, -1}, Precision.Decimal))"
        ),
        "\u{221e}"
    );
}

/// `Folder.Files` and `Folder.Contents` over a synthetic tree, compared with what Power BI
/// Desktop 2.158 returned for the same tree (dates aside: bicraft reads them in UTC). Desktop
/// also listed a hidden file the same way, with `Hidden = true` (not recreated here).
#[test]
fn folder_sources_match_desktop() {
    let (_tmp, root) = temp_dir("desktop-folder");
    let dir = root.join("folder");
    for (path, text) in [
        ("a.csv", "Region,Qty\r\nNorth,1\r\nSouth,2\r\n"),
        ("b.CSV", "Region,Qty\r\nEast,3\r\n"),
        ("A2.csv", "Region,Qty\r\nUpper,6\r\n"),
        ("noext", "Region,Qty\r\nnodot,7\r\n"),
        ("notes.txt", "notes\r\n"),
        ("sub/c.csv", "Region,Qty\r\nWest,4\r\n"),
        ("sub/deep/d.csv", "Region,Qty\r\nDeep,5\r\n"),
        ("Zeta/z.json", "[1]"),
    ] {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    let opts = SourceOptions {
        file_roots: vec![root.clone()],
        ..SourceOptions::default()
    };
    let d = m_text(dir.to_str().unwrap());
    let files = format!(
        "let S = Folder.Files({d}), I = Table.AddIndexColumn(S, \"I\", 0, 1),
            A = Table.AddColumn(I, \"Out\", each [Name = [Name], Extension = [Extension], FolderPath = [Folder Path],
                Attr = Text.Combine(Record.FieldNames([Attributes]), \",\"), Kind = [Attributes][Kind],
                CT = [Attributes][#\"Content Type\"], Size = Text.From([Attributes][Size]),
                Hidden = Text.From([Attributes]?[Hidden]?), DMType = if Value.Is([Date modified], type datetime) then \"dt\" else \"other\"])
        in Table.FromRecords(A[Out])"
    );
    let t = load_m_with(&files, &opts).unwrap();
    let sep = std::path::MAIN_SEPARATOR;
    let base = format!("{}{sep}", dir.to_str().unwrap());
    let attrs = "Content Type,Kind,Size,ReadOnly,Hidden,System,Directory,Archive,Device,Normal,Temporary,\
                 SparseFile,ReparsePoint,Compressed,Offline,NotContentIndexed,Encrypted,ChangeTime,SymbolicLink,MountPoint";
    // Desktop's rows (h.csv left out).
    let want = [
        ("a.csv", ".csv", "", "CSV File", "text/csv", "30"),
        ("A2.csv", ".csv", "", "CSV File", "text/csv", "21"),
        ("b.CSV", ".csv", "", "CSV File", "text/csv", "20"),
        ("noext", "", "", "Text File", "text/plain", "21"),
        ("notes.txt", ".txt", "", "Text File", "text/plain", "7"),
        ("c.csv", ".csv", "sub", "CSV File", "text/csv", "20"),
        ("d.csv", ".csv", "sub/deep", "CSV File", "text/csv", "20"),
        ("z.json", ".json", "Zeta", "JSON File", "application/json", "3"),
    ];
    let got: Vec<Vec<String>> = (0..t.row_count())
        .map(|r| (0..t.columns().len()).map(|c| texts(&t, c)[r].clone()).collect())
        .collect();
    assert_eq!(got.len(), want.len());
    for (g, (name, ext, rel, kind, ct, size)) in got.iter().zip(want) {
        let folder = if rel.is_empty() {
            base.clone()
        } else {
            format!("{base}{}{sep}", rel.replace('/', &sep.to_string()))
        };
        assert_eq!(
            g,
            &[name, ext, &folder, attrs, kind, ct, size, "false", "dt"]
                .map(String::from)
                .to_vec()
        );
    }
    // Folder.Contents: files and folders together, by name.
    let contents = format!(
        "let S = Folder.Contents({d}),
            A = Table.AddColumn(S, \"Out\", each [Name = [Name], Extension = [Extension], Kind = [Attributes][Kind],
                Inside = if [Content] is table then Text.Combine([Content][Name], \",\") else Text.From(Binary.Length([Content]))])
        in Table.FromRecords(A[Out])"
    );
    let t = load_m_with(&contents, &opts).unwrap();
    assert_eq!(
        texts(&t, 0),
        ["a.csv", "A2.csv", "b.CSV", "noext", "notes.txt", "sub", "Zeta"]
    );
    assert_eq!(texts(&t, 1), [".csv", ".csv", ".csv", "", ".txt", "", ""]);
    assert_eq!(
        texts(&t, 2),
        [
            "CSV File",
            "CSV File",
            "CSV File",
            "Text File",
            "Text File",
            "Folder",
            "Folder"
        ]
    );
    assert_eq!(
        texts(&t, 3),
        ["30", "21", "20", "21", "7", "c.csv,deep", "z.json"]
    );
    // A forward-slash path lists Windows folder paths with backslashes.
    if cfg!(windows) {
        let slash = dir.join("sub").to_str().unwrap().replace('\\', "/");
        let t = load_m_with(
            &format!(
                "Table.SelectColumns(Folder.Files({}), {{\"Folder Path\"}})",
                m_text(&slash)
            ),
            &opts,
        )
        .unwrap();
        assert_eq!(
            texts(&t, 0),
            [format!("{base}sub\\"), format!("{base}sub\\deep\\")]
        );
    }
}

/// A shared expression (a model query or parameter) seen from a record initializer, as in
/// `Table.FromRecords({[A = 1], [A = Threshold]})`: its own value, whatever its body's type or
/// meta; a `let` variable or a field of the same record shadows it, another record's field does
/// not; another shared expression built on it is evaluated on its own. Pins the behaviour after a
/// report of 9 instead of 6 (that figure was the session revision counter of a Data view test,
/// not an M value).
#[test]
fn shared_expressions_in_record_initializers() {
    let run = |shared: &[(&str, &str)], q: &str| -> Vec<Vec<CellValue>> {
        let opts = SourceOptions {
            shared: shared.iter().map(|(n, e)| ((*n).into(), (*e).into())).collect(),
            ..SourceOptions::default()
        };
        let t = load_m_with(q, &opts).unwrap_or_else(|e| panic!("{q}: {e}"));
        (0..t.columns().len()).map(|i| vals(&t, i)).collect()
    };
    let q = "Table.FromRecords({[A = 1], [A = Threshold]})";
    let param = "6 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]";
    for body in ["6", param, "(6)", "let x = 6 in x"] {
        assert_eq!(run(&[("Threshold", body)], q), [vec![Int(1), Int(6)]], "{body}");
    }
    for n in [0, 3, 7, 9, 42] {
        let body = n.to_string();
        assert_eq!(run(&[("Threshold", &body)], q), [vec![Int(1), Int(n)]], "{body}");
    }
    // a text parameter stays text
    let text = "\"6\" meta [IsParameterQuery=true, Type=\"Text\", IsParameterQueryRequired=true]";
    for body in ["\"6\"", text] {
        assert_eq!(
            run(&[("Threshold", body)], q),
            [vec![Text("1".into()), Text("6".into())]],
            "{body}"
        );
    }
    let six = [("Threshold", param)];
    // shadowed by a let variable and by a field of the same record
    assert_eq!(
        run(
            &six,
            "let Threshold = 3 in Table.FromRecords({[A = 1], [A = Threshold]})"
        ),
        [vec![Int(1), Int(3)]]
    );
    assert_eq!(
        run(&six, "Table.FromRecords({[Threshold = 3, A = Threshold]})"),
        [vec![Int(3)], vec![Int(3)]]
    );
    // a field of another record is not in scope
    assert_eq!(
        run(
            &six,
            "let r = [Threshold = 3, X = 0] in Table.FromRecords({[A = r[X]], [A = Threshold]})"
        ),
        [vec![Int(0), Int(6)]]
    );
    // another shared expression built on it, listed before or after it
    let limit = ("Limit", "Threshold + 3");
    for shared in [[six[0], limit], [limit, six[0]]] {
        assert_eq!(
            run(
                &shared,
                "Table.FromRecords({[A = Threshold, B = Limit], [A = Limit, B = Threshold]})"
            ),
            [vec![Int(6), Int(9)], vec![Int(9), Int(6)]]
        );
    }
}
