// Ported from DashCraft (MIT OR Apache-2.0, used here under MIT),
// crates/pbi-m/src/service/format.rs@0958fd82b04656ee1797101a2b4df8d114dfebd1 (its tests)
// Copyright (c) 2026 Jack Sweeney and the DashCraft contributors. Licensed under the MIT License.
// See crates/data/LICENSE-MIT-dashcraft and THIRD_PARTY_NOTICES.md at the repository root.
// Adapted to bicraft's grammar (no section documents) and extended with hostile inputs.

//! The M formatter (ROADMAP M11.35): layout, idempotence, comments, literals kept verbatim,
//! hostile input.

use super::*;

fn fmt(src: &str) -> String {
    fmt_with(src, &FormatOptions::default())
}

fn fmt_with(src: &str, opts: &FormatOptions) -> String {
    let limits = Limits::default();
    let out = format(src, opts, &limits).unwrap_or_else(|e| panic!("{e} for {src:?}"));
    let again = format(&out, opts, &limits).unwrap_or_else(|e| panic!("{e} for {out:?}"));
    assert_eq!(again, out, "not idempotent for {src:?}");
    out
}

fn narrow(width: usize) -> FormatOptions {
    FormatOptions {
        max_width: width,
        ..FormatOptions::default()
    }
}

#[test]
fn queries_get_one_step_per_line() {
    let src = r#"let Source = Csv.Document(File.Contents("x.csv"),[Delimiter=",", Columns=3]), #"Promoted Headers" = Table.PromoteHeaders(Source, [PromoteAllScalars=true]),#"Changed Type" = Table.TransformColumnTypes(#"Promoted Headers",{{"a", type text}, {"b", Int64.Type}}) in #"Changed Type""#;
    assert_eq!(
        fmt(src),
        r#"let
    Source = Csv.Document(File.Contents("x.csv"), [Delimiter = ",", Columns = 3]),
    #"Promoted Headers" = Table.PromoteHeaders(Source, [PromoteAllScalars = true]),
    #"Changed Type" = Table.TransformColumnTypes(
        #"Promoted Headers",
        {{"a", type text}, {"b", Int64.Type}}
    )
in
    #"Changed Type""#
    );
    assert_eq!(
        fmt_with(src, &narrow(60)),
        r#"let
    Source = Csv.Document(
        File.Contents("x.csv"),
        [Delimiter = ",", Columns = 3]
    ),
    #"Promoted Headers" = Table.PromoteHeaders(
        Source,
        [PromoteAllScalars = true]
    ),
    #"Changed Type" = Table.TransformColumnTypes(
        #"Promoted Headers",
        {{"a", type text}, {"b", Int64.Type}}
    )
in
    #"Changed Type""#
    );
    assert_eq!(fmt("let a=1 in a"), "let\n    a = 1\nin\n    a");
    assert_eq!(
        fmt("let a = let b = 1 in b in a"),
        "let\n    a =\n        let\n            b = 1\n        in\n            b\nin\n    a"
    );
}

#[test]
fn comments_and_blank_lines_stay() {
    let src = "// Orders\nlet\n    Source = 1, // first\n\n\n    /* the\n       second */\n    Next = Source  +  1\n    // before in\nin\n    Next /* done */\n// end";
    assert_eq!(
        fmt(src),
        "// Orders\nlet\n    Source = 1, // first\n\n    /* the\n       second */\n    Next = Source + 1\n    // before in\nin\n    Next /* done */\n// end"
    );
    let src = "let\n    a = 1,\n\n    b = 2\nin\n    b";
    assert_eq!(fmt(src), src);
    assert_eq!(fmt("f(a, // x\n b)"), "f(\n    a, // x\n    b\n)");
    // Comment markers inside text literals and quoted identifiers are not comments.
    assert_eq!(
        fmt("let #\"a // b\" = \"/* not */ // text\" in #\"a // b\""),
        "let\n    #\"a // b\" = \"/* not */ // text\"\nin\n    #\"a // b\""
    );
}

#[test]
fn conditions_functions_and_errors() {
    assert_eq!(fmt("if a then 1 else 2"), "if a then 1 else 2");
    assert_eq!(
        fmt_with(
            "if a > 1 then \"big\" else if a > 0 then \"small\" else \"none\"",
            &narrow(30)
        ),
        "if a > 1 then\n    \"big\"\nelse if a > 0 then\n    \"small\"\nelse\n    \"none\""
    );
    assert_eq!(
        fmt("(x as number, optional y as nullable text) as number => x+1"),
        "(x as number, optional y as nullable text) as number => x + 1"
    );
    assert_eq!(
        fmt("(k) => let t = Text.Trim(k) in t"),
        "(k) =>\n    let\n        t = Text.Trim(k)\n    in\n        t"
    );
    assert_eq!(
        fmt_with(
            "(c as binary) as table => Table.PromoteHeaders(Csv.Document(c))",
            &narrow(42)
        ),
        "(c as binary) as table =>\n    Table.PromoteHeaders(Csv.Document(c))"
    );
    assert_eq!(
        fmt("Table.AddColumn(Source,\"x\",each [a]*[b])"),
        "Table.AddColumn(Source, \"x\", each [a] * [b])"
    );
    assert_eq!(
        fmt("try Number.From(x) otherwise null"),
        "try Number.From(x) otherwise null"
    );
    assert_eq!(
        fmt_with("try Number.From(x) catch (e) => e[Message]", &narrow(20)),
        "try Number.From(x)\n    catch (e) => e[Message]"
    );
    assert_eq!(fmt("error \"bad\""), "error \"bad\"");
    assert_eq!(fmt("not (a and b) or -x <> - -1"), "not (a and b) or -x <> --1");
    assert_eq!(
        fmt_with("aaaa & bbbb & cccc * dddd * eeee", &narrow(12)),
        "aaaa\n    & bbbb\n    & cccc\n        * dddd\n        * eeee"
    );
    assert_eq!(fmt("a??b??c"), "a ?? b ?? c");
}

#[test]
fn records_lists_access_and_types() {
    assert_eq!(
        fmt("Source{[Name=\"orders.csv\"]}[Content]"),
        "Source{[Name = \"orders.csv\"]}[Content]"
    );
    assert_eq!(
        fmt("[Order   Date] & x[Ship  Date]? & [[a],[b c]]"),
        "[Order   Date] & x[Ship  Date]? & [[a], [b c]]"
    );
    assert_eq!(fmt("{1..10, a .. .5, {}}"), "{1..10, a.. .5, {}}");
    assert_eq!(
        fmt("type table [A=text, optional B=nullable number]"),
        "type table [A = text, optional B = nullable number]"
    );
    assert_eq!(
        fmt("type [a = {text}, ...] meta [Documentation.Name=\"x\"]"),
        "type [a = {text}, ...] meta [Documentation.Name = \"x\"]"
    );
    assert_eq!(
        fmt("type function (x as text, optional y as number) as text"),
        "type function (x as text, optional y as number) as text"
    );
    assert_eq!(fmt("x as nullable text is text"), "x as nullable text is text");
    assert_eq!(fmt("#table({\"a\"},{{1},{2}})"), "#table({\"a\"}, {{1}, {2}})");
    assert_eq!(fmt("@f(x) + ..."), "@f(x) + ...");
    assert_eq!(
        fmt(
            "\"https://example.com\" meta [IsParameterQuery=true, Type=\"Text\", IsParameterQueryRequired=true]"
        ),
        "\"https://example.com\" meta [IsParameterQuery = true, Type = \"Text\", IsParameterQueryRequired = true]"
    );
    // A multi-line text literal keeps its line breaks and breaks its group.
    assert_eq!(fmt("f(\"a\nb\", 1)"), "f(\n    \"a\nb\",\n    1\n)");
    // Escapes in texts and quoted identifiers stay as written.
    assert_eq!(
        fmt("#\"x\"\"y#(tab)\"   &\"a\"\"#(cr,lf)b\""),
        "#\"x\"\"y#(tab)\" & \"a\"\"#(cr,lf)b\""
    );
    assert_eq!(fmt("type table T"), "type table T");
    assert_eq!(fmt("x{0}?[a]?"), "x{0}?[a]?");
}

#[test]
fn options() {
    let limits = Limits::default();
    let two = FormatOptions {
        indent_width: 2,
        ..FormatOptions::default()
    };
    assert_eq!(fmt_with("let a=1 in a", &two), "let\n  a = 1\nin\n  a");
    let tabs = FormatOptions {
        use_tabs: true,
        ..FormatOptions::default()
    };
    assert_eq!(fmt_with("let a=1 in a", &tabs), "let\n\ta = 1\nin\n\ta");
    let wide = FormatOptions {
        indent_width: 1000,
        ..FormatOptions::default()
    };
    assert_eq!(
        fmt_with("let a=1 in a", &wide),
        format!("let\n{0}a = 1\nin\n{0}a", " ".repeat(MAX_INDENT_WIDTH))
    );
    // Width 0 breaks every group and stays stable.
    fmt_with("let a = f(1, [b = {2, 3}]) in a", &narrow(0));
    assert_eq!(
        format("", &FormatOptions::default(), &limits).ok().as_deref(),
        Some("")
    );
    assert_eq!(
        format(" \n\t", &FormatOptions::default(), &limits)
            .ok()
            .as_deref(),
        Some("")
    );
}

#[test]
fn blank_invalid_and_hostile_text() {
    let limits = Limits::default();
    let opts = FormatOptions::default();
    for bad in ["let a = in a", "\"open", "f(", "/* open", "let", "[a=1", "1 2"] {
        assert!(
            matches!(
                format(bad, &opts, &limits),
                Err(FormatError::Invalid(Error::Syntax { .. }))
            ),
            "{bad}"
        );
    }
    // A BOM and CR LF line breaks go; text literal contents stay.
    assert_eq!(
        fmt("\u{feff}let\r\n  a = \"x\r\ny\"\r\nin\r\n  a"),
        "let\n    a = \"x\r\ny\"\nin\n    a"
    );
    // Deep nesting is refused by the parser's depth cap, not by a stack overflow.
    let deep = format!("{}1{}", "(".repeat(10_000), ")".repeat(10_000));
    assert!(matches!(
        format(&deep, &opts, &limits),
        Err(FormatError::Invalid(Error::LimitExceeded(_)))
    ));
    // Long flat chains and lists format without deep recursion.
    let long = vec!["a"; 20_000].join(" & ");
    let out = fmt(&long);
    assert!(out.lines().count() > 1000);
    let list = format!("{{{}}}", vec!["1"; 20_000].join(","));
    fmt(&list);
    // Over the size cap.
    let small = Limits {
        max_bytes: 10,
        ..Limits::default()
    };
    assert!(matches!(
        format("let a = 1 in a", &opts, &small),
        Err(FormatError::Invalid(Error::LimitExceeded(_)))
    ));
    // Arbitrary byte soup never panics.
    let soup = [
        "let", "in", "(", ")", "[", "]", "{", "}", ",", "=", "\"t\"", "//c\n", "/*b*/", "x", "1", "each",
        "..", "?", "type", "as", "number",
    ];
    let mut seed = 7u64;
    for _ in 0..2000 {
        let mut s = String::new();
        for _ in 0..12 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            s.push_str(soup[(seed >> 33) as usize % soup.len()]);
            s.push(' ');
        }
        if let Ok(out) = format(&s, &opts, &limits) {
            assert_eq!(
                format(&out, &opts, &limits).ok().as_deref(),
                Some(out.as_str()),
                "{s}"
            );
        }
    }
}
