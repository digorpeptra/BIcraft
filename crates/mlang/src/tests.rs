use super::*;

const QUERY: &str = "let\n    Source = #table({\"Qty\"}, {{1}}),\n    #\"Kept Rows\" = Table.SelectRows(Source, each [Qty] > 0),\n    R = [a = 1, b = 2]\nin\n    #\"Kept Rows\"";

fn labels(c: &Completion) -> Vec<&str> {
    c.items.iter().map(|i| i.label.as_str()).collect()
}

/// The text with `|` marking the cursor: (text, byte offset).
fn at(marked: &str) -> (String, usize) {
    let c = marked.find('|').unwrap_or(marked.len());
    (marked.replacen('|', "", 1), c)
}

#[test]
fn the_catalogue_reads_every_line() {
    let lines = catalog::FUNCTIONS
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count();
    assert_eq!(functions().len(), lines);
    let f = function("Text.PadStart").unwrap();
    assert_eq!(f.signature(), "Text.PadStart(text, count, optional character)");
    assert_eq!(f.required(), 2);
    assert!(!f.description.is_empty());
    assert_eq!(
        function("DateTime.LocalNow").unwrap().signature(),
        "DateTime.LocalNow()"
    );
    assert!(function("text.padstart").is_none(), "M names are case-sensitive");
    assert_eq!(
        constant("Order.Descending").unwrap().description,
        "How a sort orders values"
    );
    assert!(constant("Int64.Type").is_some());
    // names are unique and every optional parameter comes after the required ones
    for w in functions().windows(2) {
        assert!(w[0].name < w[1].name, "{} twice", w[0].name);
    }
    for f in functions() {
        let first_optional = f.params.iter().position(|p| p.optional).unwrap_or(f.params.len());
        assert!(
            f.params.iter().skip(first_optional).all(|p| p.optional),
            "{}",
            f.name
        );
        assert!(f.description.ends_with('.'), "{}", f.name);
    }
}

#[test]
fn identifiers_are_quoted_when_they_must_be() {
    assert_eq!(identifier("Source"), "Source");
    assert_eq!(identifier("Kept Rows"), "#\"Kept Rows\"");
    assert_eq!(identifier("each"), "#\"each\"");
    assert_eq!(identifier("say \"hi\""), "#\"say \"\"hi\"\"\"");
}

#[test]
fn bindings_are_the_lets_steps_only() {
    assert_eq!(bindings(QUERY), vec!["Source", "Kept Rows", "R"]);
    assert_eq!(
        bindings("let a = 1, f = (x, y) => let z = x in z, b = 2 in b"),
        vec!["a", "f", "z", "b"]
    );
    assert!(bindings("[a = 1]").is_empty());
}

#[test]
fn completion_offers_functions_steps_queries_and_keywords() {
    let queries = vec!["Cities".to_string(), "Sales Data".to_string()];
    let scope = Scope {
        queries: &queries,
        ..Scope::default()
    };
    // a dotted prefix: that namespace's functions
    let (t, c) = at("Table.Sel|");
    let got = complete(&t, c, &scope, false).unwrap();
    assert_eq!(got.replace, 0..9);
    let l = labels(&got);
    assert_eq!(l.first().copied(), Some("Table.SelectColumns"));
    assert!(l.contains(&"Table.SelectRows"));
    let f = got.items.iter().find(|i| i.label == "Table.SelectRows").unwrap();
    assert_eq!(f.insert, "Table.SelectRows(");
    assert_eq!(f.detail, "Table.SelectRows(table, condition)");
    assert_eq!(f.kind, ItemKind::Function);
    // `Table.` alone lists the namespace
    let (t, c) = at("Table.|");
    assert!(labels(&complete(&t, c, &scope, false).unwrap()).contains(&"Table.AddColumn"));
    // a member's name without its namespace, after the names that start with it
    let (t, c) = at("SelectR|");
    let l = labels(&complete(&t, c, &scope, false).unwrap()).join(",");
    assert!(l.contains("Table.SelectRows"), "{l}");
    // steps of the text and the model's queries come first, quoted when needed
    let text = format!("{}, x = Ci", QUERY.trim_end_matches("in\n    #\"Kept Rows\""));
    let got = complete(&text, text.len(), &scope, false).unwrap();
    assert_eq!(got.items[0].label, "Cities");
    assert_eq!(got.items[0].kind, ItemKind::Query);
    let (t, c) = at("let Source = 1, x = K|");
    let got = complete(&t, c, &Scope::default(), false);
    assert!(got.is_none() || !labels(got.as_ref().unwrap()).contains(&"Source"));
    let mut text = QUERY.to_string();
    text.push_str(" & Ke");
    let got = complete(&text, text.len(), &scope, false).unwrap();
    assert_eq!(got.items[0].label, "Kept Rows");
    assert_eq!(got.items[0].insert, "#\"Kept Rows\"");
    let (t, c) = at("Sal|");
    let got = complete(&t, c, &scope, false).unwrap();
    assert_eq!(got.items[0].insert, "#\"Sales Data\"");
    // keywords and constants
    let (t, c) = at("ea|");
    let got = complete(&t, c, &scope, false).unwrap();
    assert_eq!(got.items[0].label, "each");
    assert_eq!(got.items[0].insert, "each ");
    let (t, c) = at("Order.D|");
    assert_eq!(
        complete(&t, c, &scope, false).unwrap().items[0].label,
        "Order.Descending"
    );
    // `#` names
    let (t, c) = at("#da|");
    let l = labels(&complete(&t, c, &scope, false).unwrap()).join(",");
    assert!(l.starts_with("#date"), "{l}");
    // quoted names being typed
    let mut text = QUERY.to_string();
    text.push_str(" & #\"Ke");
    let got = complete(&text, text.len(), &scope, false).unwrap();
    assert_eq!(labels(&got), vec!["Kept Rows"]);
    assert_eq!(got.replace.start, QUERY.len() + 3);
}

#[test]
fn completion_stays_quiet_where_it_should() {
    let scope = Scope::default();
    for marked in [
        "\"Table.Sel|",
        "// Table.Sel|",
        "/* Table.Sel|",
        "\"abc\"|",
        "12|",
        "x + |",
        "",
    ] {
        let (t, c) = at(marked);
        assert!(complete(&t, c, &scope, false).is_none(), "{marked:?}");
    }
    // Ctrl+Space lists everything at an empty place
    let (t, c) = at("x + |");
    let got = complete(&t, c, &scope, true).unwrap();
    assert_eq!(got.replace, 4..4);
    assert!(got.items.len() > 50);
    // a name typed in full that is the only match needs no list
    let (t, c) = at("Table.RowCount|");
    assert!(complete(&t, c, &scope, false).is_none());
    // after a block comment, completion is back
    let (t, c) = at("/* note */ Table.RowC|");
    assert_eq!(
        labels(&complete(&t, c, &scope, false).unwrap()),
        vec!["Table.RowCount"]
    );
}

#[test]
fn columns_come_inside_brackets() {
    let columns = vec!["Qty".to_string(), "Unit Price".to_string(), "City".to_string()];
    let scope = Scope {
        columns: &columns,
        ..Scope::default()
    };
    let (t, c) = at("Table.SelectRows(Source, each [|");
    let got = complete(&t, c, &scope, false).unwrap();
    assert_eq!(labels(&got), vec!["City", "Qty", "Unit Price"]);
    assert_eq!(got.replace, c..c);
    assert_eq!(got.items[0].insert, "City]");
    let (t, c) = at("each [Un|]");
    let got = complete(&t, c, &scope, false).unwrap();
    assert_eq!(labels(&got), vec!["Unit Price"]);
    assert_eq!(got.items[0].insert, "Unit Price", "the ] is there already");
    assert_eq!(got.replace, 6..8);
    let (t, c) = at("each [Unit P|");
    assert_eq!(
        labels(&complete(&t, c, &scope, false).unwrap()),
        vec!["Unit Price"]
    );
    // a record's fields are not columns
    let (t, c) = at("[a = 1, Qt|");
    let got = complete(&t, c, &scope, false);
    assert!(got.is_none_or(|g| !g.items.iter().any(|i| i.kind == ItemKind::Column)));
    // no column known: nothing inside brackets
    let (t, c) = at("each [Q|");
    assert!(complete(&t, c, &Scope::default(), false).is_none());
}

#[test]
fn the_signature_follows_the_argument_being_typed() {
    let (t, c) = at("Table.AddColumn(Source, \"x\", |");
    let s = signature_at(&t, c).unwrap();
    assert_eq!(s.function.name, "Table.AddColumn");
    assert_eq!((s.argument, s.active), (2, Some(2)));
    // commas inside lists, records and strings do not count
    let (t, c) = at("Table.SelectColumns(Source, {\"a\", \"b\", |");
    let s = signature_at(&t, c).unwrap();
    assert_eq!((s.function.name, s.active), ("Table.SelectColumns", Some(1)));
    let (t, c) = at("Text.Contains(\"a, b\", |");
    assert_eq!(signature_at(&t, c).unwrap().active, Some(1));
    // the innermost known call; an unknown one is looked through
    let (t, c) = at("Table.AddColumn(t, \"n\", each Text.Upper(|");
    assert_eq!(signature_at(&t, c).unwrap().function.name, "Text.Upper");
    let (t, c) = at("Table.AddColumn(t, \"n\", each MyFn(1, |");
    let s = signature_at(&t, c).unwrap();
    assert_eq!((s.function.name, s.active), ("Table.AddColumn", Some(2)));
    // a closed call is left
    let (t, c) = at("Text.Upper(\"a\") & |");
    assert!(signature_at(&t, c).is_none());
    // past the last parameter
    let (t, c) = at("Text.Upper(\"a\", null, |");
    assert_eq!(signature_at(&t, c).unwrap().active, None);
    let (t, c) = at("// Text.Upper(|");
    assert!(signature_at(&t, c).is_none());
    let (t, c) = at("#date(2024, |");
    assert_eq!(signature_at(&t, c).unwrap().active, Some(1));
}

#[test]
fn hover_names_what_is_under_the_cursor() {
    let queries = vec!["Cities".to_string()];
    let columns = vec!["Qty".to_string(), "Unit Price".to_string()];
    let scope = Scope {
        queries: &queries,
        columns: &columns,
        ..Scope::default()
    };
    let p = QUERY.find("SelectRows").unwrap();
    let h = hover(QUERY, p, &scope).unwrap();
    assert_eq!(h.name, "Table.SelectRows");
    assert_eq!(h.kind, ItemKind::Function);
    assert_eq!(h.signature.as_deref(), Some("Table.SelectRows(table, condition)"));
    assert_eq!(h.range, p - 6..p + 10);
    // just after the name counts too
    assert_eq!(hover(QUERY, p + 10, &scope).unwrap().name, "Table.SelectRows");
    let p = QUERY.find("(Source").unwrap() + 2;
    assert_eq!(hover(QUERY, p, &scope).unwrap().kind, ItemKind::Step);
    let p = QUERY.rfind("#\"Kept").unwrap() + 4;
    let h = hover(QUERY, p, &scope).unwrap();
    assert_eq!((h.name.as_str(), h.kind), ("Kept Rows", ItemKind::Step));
    let p = QUERY.find("[Qty]").unwrap() + 2;
    let h = hover(QUERY, p, &scope).unwrap();
    assert_eq!((h.name.as_str(), h.kind), ("Qty", ItemKind::Column));
    let t = "each [Unit Price] * 2";
    let h = hover(t, 8, &scope).unwrap();
    assert_eq!((h.name.as_str(), h.range.clone()), ("Unit Price", 6..16));
    assert_eq!(hover("Cities", 2, &scope).unwrap().kind, ItemKind::Query);
    let h = hover("each _", 1, &scope).unwrap();
    assert_eq!(h.kind, ItemKind::Keyword);
    assert!(!h.description.is_empty());
    assert_eq!(
        hover("Order.Ascending", 3, &scope).unwrap().kind,
        ItemKind::Constant
    );
    assert_eq!(hover("#date(1, 2, 3)", 2, &scope).unwrap().name, "#date");
    for (t, c) in [
        ("\"Table.SelectRows\"", 3),
        ("Unknown.Thing", 2),
        ("1 + 2", 1),
        ("", 0),
    ] {
        assert!(hover(t, c, &scope).is_none(), "{t:?}");
    }
}

#[test]
fn any_text_and_any_cursor_answer_without_a_panic() {
    let columns = vec!["A".to_string()];
    let scope = Scope {
        columns: &columns,
        ..Scope::default()
    };
    let samples = [
        QUERY,
        "#",
        "#\"",
        "\"",
        "/*",
        "//",
        "[",
        "((((",
        "))))",
        "]]]",
        "é#\"ü\"\u{feff}.",
        "Table.\u{1F600}Sel",
        "let in let in , = = => ...",
        "0x1F.5e+",
    ];
    for s in samples {
        for c in 0..=s.len() + 2 {
            let _ = complete(s, c, &scope, true);
            let _ = complete(s, c, &scope, false);
            let _ = signature_at(s, c);
            let _ = hover(s, c, &scope);
        }
        let _ = bindings(s);
    }
    assert_eq!(byte_of_char("aé b", 3), 4);
    assert_eq!(char_of_byte("aé b", 4), 3);
    assert_eq!(char_of_byte("aé", 2), 2);
    assert_eq!(byte_of_char("ab", 9), 2);
}
