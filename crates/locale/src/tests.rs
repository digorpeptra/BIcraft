use super::*;

#[test]
fn lookup_falls_back_to_language_then_en_us() {
    assert_eq!(Locale::get("fr-FR").name, "fr-FR");
    assert_eq!(Locale::get("FR_fr").name, "fr-FR");
    assert_eq!(Locale::get("fr").name, "fr-FR");
    assert_eq!(Locale::get("fr-MC").name, "fr-FR");
    assert_eq!(Locale::get("de-LI").name, "de-DE");
    assert_eq!(Locale::get("xx-YY").name, "en-US");
    assert_eq!(Locale::get(&"x".repeat(10_000)).name, "en-US");
    assert!(Locale::get("").is_invariant());
    assert!(Locale::get("Invariant").is_invariant());
    assert_eq!(Locale::find("xx"), None);
    assert_eq!(Locale::en_us().name, "en-US");
    assert!(Locale::all().len() >= 30);
}

#[test]
fn every_culture_is_consistent() {
    for l in Locale::all().iter().chain([Locale::invariant()]) {
        assert!(!l.decimal.is_ascii_digit(), "{}", l.name);
        assert_ne!(l.decimal, l.group, "{}", l.name);
        for p in [
            l.short_date,
            l.long_date,
            l.long_time,
            l.short_time,
            l.vb.medium_date,
            l.vb.medium_time,
        ] {
            assert!(!p.is_empty(), "{}", l.name);
        }
        assert!(
            l.months.iter().chain(l.days.iter()).all(|n| !n.is_empty()),
            "{}",
            l.name
        );
        assert!(l.first_day < 7 && l.vb.first_day < 7, "{}", l.name);
    }
}

#[test]
fn required_cultures_are_present() {
    for c in [
        "en-US", "en-GB", "fr-FR", "de-DE", "es-ES", "it-IT", "nl-NL", "pt-BR", "ja-JP",
    ] {
        assert_eq!(Locale::get(c).name, c);
    }
}

#[test]
fn format_examples() {
    use vb::{Value, format};
    let fr = Locale::get("fr-FR");
    let de = Locale::get("de-DE");
    let us = Locale::get("en-US");
    let f = |v, s, l| format(v, s, l).unwrap_or_else(|| "<none>".into());
    assert_eq!(f(Value::Number(1234.5), "Currency", us), "$1,234.50");
    assert_eq!(f(Value::Number(-1234.5), "Currency", us), "($1,234.50)");
    assert_eq!(f(Value::Number(1234.5), "Currency", fr), "1\u{202f}234,50 €");
    assert_eq!(f(Value::Number(1234.5), "#,0.00", de), "1.234,50");
    assert_eq!(f(Value::Number(0.256), "0.0%", fr), "25,6%");
    // 2024-03-05 14:07:09
    let x = 45_356.0 + (14.0 * 3600.0 + 7.0 * 60.0 + 9.0) / 86_400.0;
    assert_eq!(f(Value::Date(x), "Long Date", fr), "mardi 5 mars 2024");
    assert_eq!(f(Value::Date(x), "dd/mm/yyyy", de), "05.03.2024");
    assert_eq!(f(Value::Date(x), "", us), "3/5/2024 2:07:09 PM");
    assert_eq!(f(Value::Date(x), "", fr), "05/03/2024 14:07:09");
    assert_eq!(f(Value::Date(x), "mmmm", Locale::get("ja-JP")), "March");
    assert_eq!(f(Value::Blank, "0.00", us), "");
    assert_eq!(f(Value::Text("abc"), "0.00", us), "abc");
    assert_eq!(f(Value::Text("1,5"), "0.00", fr), "1,50");
}

/// Scaling commas (right before the decimal point or the end of the numeric part) divide by
/// 1000 each, also when a literal follows; letters that are no format character are literal.
#[test]
fn scaling_commas_before_a_literal_suffix() {
    use vb::{Value, format};
    let us = Locale::get("en-US");
    let fr = Locale::get("fr-FR");
    let f = |v, s, l| format(v, s, l).unwrap_or_else(|| "<none>".into());
    assert_eq!(f(Value::Number(1_234_567.0), "#,##0,, \"M\"", us), "1 M");
    assert_eq!(f(Value::Number(1234.0), "0,K", us), "1K");
    assert_eq!(f(Value::Number(1_234_567.0), "#,0,K", us), "1,235K");
    assert_eq!(f(Value::Number(12_345.0), "#,0,.0 k€", fr), "12,3 k€");
    assert_eq!(f(Value::Number(-2500.0), "0,.0k;(0,.0k)", us), "(2.5k)");
    // date, time and string characters stay refused unquoted: their meaning there is unknown
    assert_eq!(f(Value::Number(1_234_567.0), "#,##0,, M", us), "<none>");
}

#[test]
fn value_parses_in_the_culture() {
    use parse::value;
    let fr = Locale::get("fr-FR");
    let us = Locale::get("en-US");
    let br = Locale::get("pt-BR");
    assert_eq!(value("1 234,5", fr), Some(1234.5));
    assert_eq!(value("1\u{202f}234,50 €", fr), Some(1234.5));
    assert_eq!(value("2.675", fr), None);
    assert_eq!(value("$2.675", us), Some(2.675));
    assert_eq!(value("(5)", us), Some(-5.0));
    assert_eq!(value("5-", us), Some(-5.0));
    assert_eq!(value("1.234,5", us), Some(1.2345));
    assert_eq!(value("R$ 1.234,50", br), Some(1234.5));
    assert_eq!(value("12%", us), None);
    assert_eq!(value("", us), None);
    assert_eq!(value("1E3", us), Some(1000.0));
    assert_eq!(value(&"9".repeat(1000), us), None);
}

#[test]
fn display_units_and_layout() {
    use display::{Unit, number, percent, with_unit};
    let fr = Locale::get("fr-FR");
    let es = Locale::get("es-ES");
    let us = Locale::get("en-US");
    assert_eq!(with_unit(1_234_567.0, Unit::Millions, 2, us), "1.23M");
    assert_eq!(with_unit(300_000.0, Unit::Millions, 1, fr), "0,3\u{a0}M");
    assert_eq!(with_unit(2e9, Unit::Billions, 0, us), "2bn");
    assert_eq!(number(1234.0, 0, 0, true, es), "1234");
    assert_eq!(number(12345.0, 0, 0, true, es), "12.345");
    assert_eq!(number(-0.004, 0, 2, true, us), "0");
    assert_eq!(percent(0.256, 1, 1, fr), "25,6\u{a0}%");
    assert_eq!(Unit::auto(-2500.0), Unit::Thousands);
    assert_eq!(Unit::from_property(1e6), Some(Unit::Millions));
    assert_eq!(Unit::from_property(0.0), None);
}

#[test]
fn dotnet_patterns() {
    use datefmt::{DateParts, format, format_pattern};
    let p = DateParts::from_serial(45_356.5).unwrap_or_else(|| panic!("serial"));
    let ru = Locale::get("ru-RU");
    assert_eq!(format(&p, "d MMMM", ru), "5 марта");
    // Power Query (Desktop 2.158) capitalizes the nominative.
    assert_eq!(format(&p, "MMMM", ru), "Март");
    assert_eq!(format(&p, "M", ru), "5 марта");
    assert_eq!(format(&p, "Y", Locale::get("es-ES")), "marzo de 2024");
    assert_eq!(format(&p, "D", Locale::get("fr-FR")), "mardi 5 mars 2024");
    assert_eq!(format(&p, "d", Locale::get("en-US")), "3/5/2024");
    assert_eq!(format(&p, "T", Locale::get("en-US")), "12:00:00 PM");
    assert_eq!(
        format_pattern(&p, "yyyy'-'MM'-'dd", &Locale::invariant().names()),
        "2024-03-05"
    );
    assert_eq!(DateParts::from_serial(f64::NAN), None);
    assert_eq!(DateParts::from_serial(1e300), None);
    let jan1 = DateParts::from_serial(45_292.0).unwrap_or_else(|| panic!("serial"));
    assert_eq!((jan1.year, jan1.month, jan1.day, jan1.weekday), (2024, 1, 1, 1));
}

/// Hostile format strings, values and texts never panic and stay bounded.
#[test]
fn hostile_input_is_bounded() {
    use datefmt::DateParts;
    let pieces = [
        "0", "#", ",", ".", "%", "E+", "e-", ";", "\"", "\\", "d", "m", "y", "h", "n", "s", "w", "q", "c",
        "t", "AM/PM", "a/p", "AMPM", "/", ":", "'", "dddddd", "ttttt", "yyyyy", "mmmmm", "x", "€",
        "\u{202f}",
    ];
    let values = [
        0.0,
        -0.0,
        1e300,
        -1e300,
        f64::NAN,
        f64::INFINITY,
        5e-324,
        45_356.75,
        -657_434.0,
        2_958_465.99,
    ];
    let mut seed = 12_345u64;
    let mut next = |n: usize| {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        usize::try_from(seed >> 33).unwrap_or(0) % n
    };
    for round in 0..3000 {
        let mut f = String::new();
        for _ in 0..next(12) {
            f.push_str(pieces.get(next(pieces.len())).copied().unwrap_or(""));
        }
        let loc = Locale::all()
            .get(round % Locale::all().len())
            .unwrap_or_else(|| Locale::en_us());
        let x = values.get(next(values.len())).copied().unwrap_or(0.0);
        for v in [
            vb::Value::Number(x),
            vb::Value::Date(x),
            vb::Value::Text(&f),
            vb::Value::Bool(round % 2 == 0),
        ] {
            if let Some(s) = vb::format(v, &f, loc) {
                assert!(s.len() < 20_000, "{f:?}");
            }
        }
        let _ = parse::value(&f, loc);
        let p = DateParts::from_days_ms(i64::MAX, i64::MIN);
        assert!(datefmt::format_pattern(&p, &f, &loc.names()).len() < 20_000);
    }
    assert!(vb::format(vb::Value::Number(1.0), &"0".repeat(5000), Locale::en_us()).is_none());
    let long = "dddd".repeat(250);
    assert!(vb::format(vb::Value::Date(45_356.0), &long, Locale::en_us()).is_some_and(|s| s.len() < 20_000));
}
