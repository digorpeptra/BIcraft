//! System tables (`H$`, `R$`, `U$`): synthetic checks.
//!
//! The expected arrays come from a synthetic model of ours (calculated tables only) that the
//! Analysis Services engine of Power BI Desktop 2.158.1304.0 processed: they are the values the
//! engine wrote into its own `H$` / `R$` / `U$` files (see
//! `docs/formats/vertipaq-system-tables.md`, "Oracle"). No real data is involved.

use bicraft_vertipaq::system::{
    AttributeHierarchyOptions, Collation, SystemTableError, UserHierarchyLevel, build_attribute_hierarchy,
    build_attribute_hierarchy_with, build_relationship_index, build_user_hierarchy, encode_column,
};
use bicraft_vertipaq::{CellValue, Limits, decode_data_ids, parse_idfmeta};

fn text(s: &str) -> CellValue {
    CellValue::Text(s.to_string())
}

fn ints(values: &[i64]) -> Vec<CellValue> {
    values.iter().map(|&v| CellValue::Int(v)).collect()
}

fn fr() -> Collation {
    Collation::for_culture("fr-FR")
}

fn stored(h: &bicraft_vertipaq::AttributeHierarchy, name: &str) -> Vec<u32> {
    h.table.column(name).map(|c| c.values.clone()).unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------
// Collation

/// The order the engine gave a fr-FR string column (minus three pairs the engine considers
/// equal and orders arbitrarily: æ/ae, ĳ/ij, œ/oe).
const FR_ORDER: &[&str] = &[
    "", "'", "-", "–", "—", " ", "  ", " a", "!", "\"", "#", "#1", "$", "%", "&", "(", ")", "*", ",", ".",
    "/", ":", ";", "?", "@", "[", "\\", "]", "^", "_", "`", "{", "|", "}", "~", "¡", "¿", "’", "£", "¥", "€",
    "+", "+1", "<", "=", ">", "«", "»", "§", "°", "µ", "…", "01", "½", "1", "-1", "1.5", "10", "2", "²", "9",
    "Ⅳ", "a", "á", "à", "â", "ä", "å", "a ", "a b", "a!b", "a,b", "a.b", "ab", "'ab", "-ab", "a'b", "a-b",
    "a--b", "ab-", "abc", "abd", "ao", "ÄÖ", "ay", "az", "b", "c", "ç", "co op", "co_op", "coop", "co-op",
    "coöp", "cote", "côte", "coté", "côté", "d", "đ", "ð", "e", "é", "è", "ê", "ë", "f", "fi", "ﬁ", "i", "ı",
    "İ", "î", "ï", "IV", "ł", "n", "ñ", "ŋ", "o", "ô", "ö", "ø", "resume", "résumé", "ss", "ß", "t", "þ",
    "u", "ù", "û", "ü", "x²", "y", "ÿ", "z", "α", "β", "Ω", "д", "я", "😀", "中", "文", "日本",
];

#[test]
fn fr_fr_string_order_matches_the_engine() {
    let c = fr();
    let mut shuffled: Vec<&str> = FR_ORDER.iter().rev().copied().collect();
    shuffled.rotate_left(17);
    shuffled.sort_by(|a, b| c.compare(a, b));
    assert_eq!(shuffled, FR_ORDER);
}

fn assert_order(c: Collation, expected: &[&str]) {
    let mut v: Vec<&str> = expected.iter().rev().copied().collect();
    v.sort_by(|a, b| c.compare(a, b));
    assert_eq!(v, expected, "{c:?}");
}

#[test]
fn accents_forward_in_en_us_backward_in_fr_fr() {
    let words = ["cote", "côte", "coté", "côté", "cotes", "côtes"];
    assert_order(fr(), &words);
    assert_order(
        Collation::for_culture("en-US"),
        &["cote", "coté", "côte", "côté", "cotes", "côtes"],
    );
    assert_order(fr(), &["ea", "éa", "eb", "éb", "ee", "ée", "eé", "éé"]);
    assert_order(
        Collation::for_culture("en-US"),
        &["ea", "éa", "eb", "éb", "ee", "eé", "ée", "éé"],
    );
}

#[test]
fn hyphens_and_apostrophes_only_break_ties() {
    assert_order(
        fr(),
        &[
            "",
            "'",
            "-",
            "''",
            "'-",
            "-'",
            "--",
            "a",
            "a-",
            "co op",
            "coo",
            "co-o",
            "coo-",
            "coop",
            "'coop",
            "-coop",
            "c-o-o-p",
            "co'op",
            "co-op",
            "co--op",
            "co\u{2010}op",
            "co\u{2011}op",
            "co\u{2013}op",
            "co\u{2014}op",
            "coo-p",
            "coop'",
            "coop-",
            "cooq",
        ],
    );
}

#[test]
fn diacritics_follow_the_engine_order() {
    assert_order(
        fr(),
        &[
            "a", "á", "à", "â", "ä", "ă", "ā", "ã", "å", "ą", "b", "c", "ć", "ċ", "ĉ", "č", "ç", "d", "ď",
            "đ", "ð", "e", "é", "è", "ė", "ê", "ë", "ě", "ĕ", "ē", "ę", "g", "ġ", "ĝ", "ğ", "ģ", "i", "ı",
            "í", "ì", "î", "ï", "ĭ", "ī", "ĩ", "į", "l", "ĺ", "ŀ", "ľ", "ļ", "ł", "n", "ń", "ň", "ñ", "ņ",
            "ŋ", "o", "ó", "ò", "ô", "ö", "ŏ", "ō", "õ", "ő", "ø", "ơ", "s", "ś", "ŝ", "š", "ş", "u", "ú",
            "ù", "û", "ü", "ŭ", "ū", "ũ", "ů", "ų", "ű", "ư", "z", "ź", "ż", "ž",
        ],
    );
    // Letters with a stroke sort as accented letters, ŋ as its own letter after n.
    assert_order(
        fr(),
        &[
            "d", "đa", "ða", "db", "l", "ła", "lb", "n", "nb", "nz", "ŋa", "o", "øa", "ob",
        ],
    );
}

#[test]
fn digits_symbols_and_scripts() {
    assert_order(
        fr(),
        &[
            "0", "00", "0a", "¼", "½", "1", "１", "¹", "١", "१", "10", "11", "1a", "2", "²", "2a", "3", "³",
            "¾", "4", "9", "Ⅳ", "a0", "a1", "a9", "aa",
        ],
    );
    assert_order(
        fr(),
        &[
            " ", "\u{3000}", "\u{a0}", "!", "！", "\"", "#", "$", "%", "&", "(", ")", "*", ",", "，", "、",
            ".", "。", "/", ":", ";", "?", "@", "[", "\\", "]", "^", "_", "`", "{", "|", "}", "~", "¡", "¿",
            "‘", "’", "“", "”", "‹", "›", "¢", "£", "¥", "€", "+", "<", "=", ">", "±", "«", "»", "×", "÷",
            "§", "©", "¬", "®", "°", "µ", "¶", "·", "…", "†", "‡", "•", "‰",
        ],
    );
    assert_order(
        fr(),
        &[
            "a", "ａ", "z", "α", "ω", "а", "я", "ա", "ა", "ｱ", "ア", "あ", "א", "ا", "अ", "ก", "😀", "가",
            "一",
        ],
    );
}

#[test]
fn equivalence_is_case_insensitive_only() {
    let c = fr();
    assert!(c.equivalent("RÉSUMÉ", "résumé"));
    assert!(c.equivalent("Ω", "ω"));
    assert!(!c.equivalent("resume", "résumé"));
    assert!(!c.equivalent("a", "ａ"));
    assert!(!c.equivalent("ア", "あ"));
    assert!(!c.equivalent("co-op", "coop"));
    assert!(!c.equivalent("", " "));
    let o = Collation::Ordinal;
    assert!(!o.equivalent("A", "a"));
    assert!(o.compare("B", "a").is_lt());
}

// ---------------------------------------------------------------------------------------------
// H$

#[test]
fn h_value_encoded_int_with_blank() {
    let entries = [
        (2, CellValue::Null),
        (3, CellValue::Int(-3)),
        (6, CellValue::Int(0)),
        (11, CellValue::Int(5)),
        (13, CellValue::Int(7)),
        (18, CellValue::Int(12)),
    ];
    let options = AttributeHierarchyOptions {
        sort_by: None,
        id_to_pos: false,
    };
    let h = build_attribute_hierarchy_with(&entries, &fr(), &options).unwrap();
    assert_eq!(stored(&h, "POS_TO_ID"), [2, 3, 6, 11, 13, 18, 0, 0]);
    assert!(h.table.column("ID_TO_POS").is_none());
    assert_eq!(h.materialization_type, 1);
    assert_eq!(h.distinct_data_count, 6);
    assert_eq!(h.table.records_per_segment, 6);
    assert_eq!(h.min_value, Some(CellValue::Int(-3)));
    assert_eq!(h.max_value, Some(CellValue::Int(12)));
}

#[test]
fn h_reals_dates_bools_decimals() {
    let reals = [
        (2, CellValue::Null),
        (3, CellValue::Real(1.5)),
        (4, CellValue::Real(-2.25)),
        (5, CellValue::Real(0.0)),
        (6, CellValue::Real(1e300)),
        (7, CellValue::Real(3.0)),
        (8, CellValue::Real(-1e-300)),
    ];
    let h = build_attribute_hierarchy(&reals, &fr()).unwrap();
    assert_eq!(stored(&h, "POS_TO_ID"), [2, 4, 8, 5, 3, 7, 6, 0, 0]);
    assert_eq!(stored(&h, "ID_TO_POS"), [0, 7, 0, 4, 1, 3, 6, 5, 2]);

    let dates = [
        (2, CellValue::Null),
        (3, CellValue::DateTime(45322.0)),
        (4, CellValue::DateTime(36525.572916666664)),
        (5, CellValue::DateTime(45321.0)),
        (6, CellValue::DateTime(2.0)),
        (7, CellValue::DateTime(73216.0)),
    ];
    let h = build_attribute_hierarchy(&dates, &fr()).unwrap();
    assert_eq!(stored(&h, "POS_TO_ID"), [2, 6, 4, 5, 3, 7, 0, 0]);
    assert_eq!(stored(&h, "ID_TO_POS"), [0, 6, 0, 4, 2, 3, 1, 5]);

    let no_id_to_pos = AttributeHierarchyOptions {
        sort_by: None,
        id_to_pos: false,
    };
    let bools = [
        (2, CellValue::Null),
        (3, CellValue::Bool(false)),
        (4, CellValue::Bool(true)),
    ];
    let h = build_attribute_hierarchy_with(&bools, &fr(), &no_id_to_pos).unwrap();
    assert_eq!(stored(&h, "POS_TO_ID"), [2, 3, 4, 0, 0]);
    let swapped = [(3, CellValue::Bool(true)), (4, CellValue::Bool(false))];
    let h = build_attribute_hierarchy(&swapped, &fr()).unwrap();
    assert_eq!(stored(&h, "POS_TO_ID"), [4, 3, 0, 0, 0]);

    let decimals = [
        (2, CellValue::Null),
        (3, CellValue::Decimal(-45_000)),
        (453, CellValue::Decimal(0)),
        (578, CellValue::Decimal(12_500)),
        (10_452, CellValue::Decimal(999_900)),
        (10_000_453, CellValue::Decimal(1_000_000_000)),
    ];
    let h = build_attribute_hierarchy_with(&decimals, &fr(), &no_id_to_pos).unwrap();
    assert_eq!(
        stored(&h, "POS_TO_ID"),
        [2, 3, 453, 578, 10_452, 10_000_453, 0, 0]
    );
}

#[test]
fn h_strings_and_case_insensitive_keys() {
    let names = [
        (3, text("three")),
        (4, text("one")),
        (5, text("five")),
        (6, text("two")),
        (7, text("four")),
    ];
    let h = build_attribute_hierarchy(&names, &fr()).unwrap();
    assert_eq!(stored(&h, "POS_TO_ID"), [5, 7, 4, 3, 6, 0, 0, 0]);
    assert_eq!(stored(&h, "ID_TO_POS"), [0, 5, 0, 3, 2, 0, 4, 1]);
    assert_eq!(h.string_value_max_length, 5);
    assert_eq!(
        (h.min_value, h.max_value),
        (Some(text("five")), Some(text("two")))
    );

    let codes = [
        (2, CellValue::Null),
        (3, text("ABC")),
        (4, text("zzz")),
        (5, text("déf")),
        (6, text("DEF")),
        (7, text("x")),
    ];
    let h = build_attribute_hierarchy(&codes, &fr()).unwrap();
    assert_eq!(stored(&h, "POS_TO_ID"), [2, 3, 6, 5, 7, 4, 0, 0]);
    assert_eq!(stored(&h, "ID_TO_POS"), [0, 6, 0, 1, 5, 3, 2, 4]);
}

/// Distinct (data id, value) pairs of a column.
type Entries = Vec<(u32, CellValue)>;

fn geo_city() -> (Entries, Entries) {
    let cities = [
        "Lille",
        "Arras",
        "Nice",
        "Arles",
        "Namur",
        "Bruxelles",
        "Gand",
        "Séville",
        "Köln",
        "Bonn",
    ];
    let order = [3, 4, 1, 2, 5, 6, 7, 9, 10, 0];
    let mut entries = vec![(2, CellValue::Null)];
    let mut sort_by = vec![(2, CellValue::Int(8))];
    for (k, (city, o)) in cities.iter().zip(order).enumerate() {
        let id = 3 + k as u32;
        entries.push((id, text(city)));
        sort_by.push((id, CellValue::Int(o)));
    }
    (entries, sort_by)
}

#[test]
fn h_sort_by_column_moves_blank_too() {
    let (entries, sort_by) = geo_city();
    let options = AttributeHierarchyOptions {
        sort_by: Some(&sort_by),
        id_to_pos: true,
    };
    let h = build_attribute_hierarchy_with(&entries, &fr(), &options).unwrap();
    assert_eq!(
        stored(&h, "POS_TO_ID"),
        [12, 5, 6, 3, 4, 7, 8, 9, 2, 10, 11, 0, 0]
    );
    assert_eq!(stored(&h, "ID_TO_POS"), [0, 11, 8, 3, 4, 1, 2, 5, 6, 7, 9, 10, 0]);
    assert_eq!(
        (h.min_value, h.max_value),
        (Some(text("Bonn")), Some(text("Köln")))
    );
    assert_eq!(h.string_value_max_length, 9);
}

#[test]
fn h_files_have_the_engine_layout() {
    let countries = [
        (2, CellValue::Null),
        (3, text("France")),
        (4, text("Belgique")),
        (5, text("Espagne")),
        (6, text("Allemagne")),
    ];
    let h = build_attribute_hierarchy(&countries, &fr()).unwrap();
    assert_eq!(stored(&h, "POS_TO_ID"), [2, 6, 4, 5, 3, 0, 0]);
    assert_eq!(stored(&h, "ID_TO_POS"), [0, 5, 0, 4, 2, 3, 1]);
    assert_eq!((h.table.record_count(), h.table.segment_count()), (7, 2));
    for (index, column) in h.table.columns.iter().enumerate() {
        let files = h.table.encode(index).unwrap();
        // Two segments (5 + 2 records), 32-bit packing, one spare value per block.
        assert_eq!(
            (files.idf.len(), files.idfmeta.len()),
            (56, 316),
            "{}",
            column.name
        );
        let meta = parse_idfmeta(&files.idfmeta).unwrap();
        assert_eq!(meta, files.meta);
        let records: Vec<u64> = meta.segments.iter().map(|s| s.records).collect();
        assert_eq!(records, [5, 2]);
        assert!(
            meta.segments
                .iter()
                .all(|s| s.compression_class == 703_062 && s.first_run_value == 0)
        );
        assert_eq!(
            decode_data_ids(&files.idf, &meta, &Limits::default()).unwrap(),
            column.values
        );
    }
}

#[test]
fn h_rejects_bad_input() {
    let c = fr();
    assert_eq!(
        build_attribute_hierarchy(&[(0, CellValue::Int(1))], &c),
        Err(SystemTableError::ReservedDataId(0))
    );
    assert_eq!(
        build_attribute_hierarchy(&[(3, CellValue::Null)], &c),
        Err(SystemTableError::BlankMismatch(3))
    );
    let empty = build_attribute_hierarchy(&[], &c).unwrap();
    assert_eq!(stored(&empty, "POS_TO_ID"), [0, 0, 0]);
    assert_eq!(empty.table.records_per_segment, 1);
}

// ---------------------------------------------------------------------------------------------
// R$

#[test]
fn r_many_to_one_with_missing_keys_and_blank() {
    let from = [
        (2, CellValue::Null),
        (3, CellValue::Int(2)),
        (4, CellValue::Int(7)),
        (5, CellValue::Int(1)),
        (6, CellValue::Int(5)),
        (7, CellValue::Int(3)),
        (8, CellValue::Int(9)),
    ];
    let to = ints(&[3, 1, 5, 2, 4]);
    let r = build_relationship_index(&from, &to, &fr()).unwrap();
    let index = &r.table.columns[0];
    assert_eq!(index.values, [0, 0, 0, 4, 0, 2, 3, 1, 0]);
    assert_eq!(r.index, [-1, -1, -1, 3, -1, 1, 2, 0, -1]);
    assert_eq!((index.width, r.unmatched, r.record_count()), (3, 2, 9));
    let files = r.table.encode(0).unwrap();
    assert_eq!(files.idf.len(), 16);
    let meta = parse_idfmeta(&files.idfmeta).unwrap();
    assert_eq!(meta.segments[0].compression_class, 703_033);
    assert_eq!(meta.segments[0].first_run_value, u32::MAX);
    assert_eq!(
        decode_data_ids(&files.idf, &meta, &Limits::default()).unwrap(),
        index.values
    );
}

#[test]
fn r_string_keys_match_case_insensitively() {
    let from = [
        (2, CellValue::Null),
        (3, text("ABC")),
        (4, text("zzz")),
        (5, text("déf")),
        (6, text("DEF")),
        (7, text("x")),
    ];
    let to = [text("abc"), text("Déf"), text("x")];
    let r = build_relationship_index(&from, &to, &fr()).unwrap();
    assert_eq!(r.table.columns[0].values, [0, 0, 0, 1, 0, 2, 0, 3]);
    assert_eq!(r.table.columns[0].width, 2);
    // Under an ordinal collation only exact matches join.
    let r = build_relationship_index(&from, &to, &Collation::Ordinal).unwrap();
    assert_eq!(r.table.columns[0].values, [0, 0, 0, 0, 0, 0, 0, 3]);
}

#[test]
fn r_one_to_one_has_an_index_per_direction() {
    let a_rows = ints(&[4, 1, 2, 3]);
    let b_rows = ints(&[2, 5, 3, 4]);
    let a_ids = [
        (3, CellValue::Int(4)),
        (4, CellValue::Int(1)),
        (5, CellValue::Int(2)),
        (6, CellValue::Int(3)),
    ];
    let b_ids = [
        (3, CellValue::Int(2)),
        (4, CellValue::Int(5)),
        (5, CellValue::Int(3)),
        (6, CellValue::Int(4)),
    ];
    let a_to_b = build_relationship_index(&a_ids, &b_rows, &fr()).unwrap();
    let b_to_a = build_relationship_index(&b_ids, &a_rows, &fr()).unwrap();
    assert_eq!(a_to_b.table.columns[0].values, [0, 0, 0, 4, 0, 1, 3]);
    assert_eq!(b_to_a.table.columns[0].values, [0, 0, 0, 3, 0, 4, 1]);
}

#[test]
fn r_numbers_match_across_storage_types() {
    let from = [
        (3, CellValue::Decimal(20_000)),
        (4, CellValue::Real(2.5)),
        (5, CellValue::DateTime(45_000.0)),
    ];
    let to = [
        CellValue::Int(2),
        CellValue::Decimal(25_000),
        CellValue::Null,
        CellValue::DateTime(45_000.0),
    ];
    let r = build_relationship_index(&from, &to, &fr()).unwrap();
    assert_eq!(r.index, [-1, -1, -1, 0, 1, 3]);
    assert_eq!(r.unmatched, 0);
    // A blank key on the one side never matches a blank on the many side.
    let r = build_relationship_index(&[(2, CellValue::Null)], &[CellValue::Null], &fr()).unwrap();
    assert_eq!(r.index, [-1, -1, -1]);
}

// ---------------------------------------------------------------------------------------------
// U$

#[test]
fn u_three_levels_with_blanks_and_sort_by() {
    let countries = [
        (2, CellValue::Null),
        (3, text("France")),
        (4, text("Belgique")),
        (5, text("Espagne")),
        (6, text("Allemagne")),
    ];
    let regions = [
        (2, CellValue::Null),
        (3, text("Nord")),
        (4, text("Sud")),
        (5, text("Wallonie")),
        (6, text("Flandre")),
        (7, text("Ouest")),
    ];
    let (cities, city_order) = geo_city();
    let c = fr();
    let country_h = build_attribute_hierarchy(&countries, &c).unwrap();
    let region_h = build_attribute_hierarchy(&regions, &c).unwrap();
    let city_options = AttributeHierarchyOptions {
        sort_by: Some(&city_order),
        id_to_pos: true,
    };
    let city_h = build_attribute_hierarchy_with(&cities, &c, &city_options).unwrap();
    let country_ids = [3, 3, 3, 3, 4, 4, 4, 2, 5, 3, 6, 6];
    let region_ids = [3, 3, 4, 4, 5, 2, 6, 2, 4, 3, 7, 7];
    let city_ids = [3, 4, 5, 6, 7, 8, 9, 2, 10, 3, 11, 12];
    let u = build_user_hierarchy(&[
        UserHierarchyLevel {
            data_ids: &country_ids,
            hierarchy: &country_h,
        },
        UserHierarchyLevel {
            data_ids: &region_ids,
            hierarchy: &region_h,
        },
        UserHierarchyLevel {
            data_ids: &city_ids,
            hierarchy: &city_h,
        },
    ])
    .unwrap();
    let col = |n: &str| u.table.column(n).unwrap().values.clone();
    assert_eq!(
        col("MULTI_LEVEL_ID"),
        [
            2, 6, 4, 5, 3, 2, 7, 2, 6, 5, 4, 3, 4, 2, 12, 11, 8, 9, 7, 10, 3, 4, 5, 6
        ]
    );
    assert_eq!(
        col("PARENT_POS"),
        [
            0, 0, 0, 0, 0, 1, 2, 3, 3, 3, 4, 5, 5, 6, 7, 7, 8, 9, 10, 11, 12, 12, 13, 13
        ]
    );
    assert_eq!(
        col("FIRST_CHILD_POS"),
        [
            6, 7, 8, 11, 12, 14, 15, 17, 18, 19, 20, 21, 23, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
        ]
    );
    assert_eq!(
        col("CHILD_COUNT"),
        [
            2, 2, 4, 2, 3, 2, 3, 2, 2, 2, 2, 3, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
        ]
    );
    assert_eq!(
        u.level_definition(&["Country (77)", "Region (78)", "City (79)"]),
        "$Country (77)$0$Region (78)$5$City (79)$13$"
    );
    let files = encode_column(&u.table.columns[0], u.table.records_per_segment).unwrap();
    assert_eq!((files.idf.len(), files.idfmeta.len()), (112, 176));
    let parent = encode_column(&u.table.columns[1], u.table.records_per_segment).unwrap();
    assert_eq!(parent.meta.segments[0].first_run_value, u32::MAX);
}

#[test]
fn u_rejects_inconsistent_levels() {
    let h = build_attribute_hierarchy(&[(3, CellValue::Int(1))], &fr()).unwrap();
    assert_eq!(build_user_hierarchy(&[]), Err(SystemTableError::NoLevels));
    assert_eq!(
        build_user_hierarchy(&[
            UserHierarchyLevel {
                data_ids: &[3, 3],
                hierarchy: &h
            },
            UserHierarchyLevel {
                data_ids: &[3],
                hierarchy: &h
            },
        ]),
        Err(SystemTableError::LevelLengthMismatch)
    );
    assert_eq!(
        build_user_hierarchy(&[UserHierarchyLevel {
            data_ids: &[4],
            hierarchy: &h
        }]),
        Err(SystemTableError::UnknownMember(4))
    );
}
